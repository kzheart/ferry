//! 把 canonical 会话树落成 Cursor 的原生两层结构。
//!
//! 一次写入产出四类键（`docs/cursor-migration-target.md` §4）：
//! - `agentKv:blob:<sha256hex>`：上下文层，模型真正读到的消息；
//! - `composerData:<cid>`：会话体，`conversationState` 指回上面那串摘要；
//! - `bubbleId:<cid>:<bid>`：展示层，UI 里看到的每一条；
//! - `composerHeaders` 一行 + `ItemTable` 两个门控键。
//!
//! 顺序：blob 先落，`composerData` 后落——`conversationState` 引用的 blob 必须已经
//! 在库里。整批写在一个事务里，任何一步失败都不留半条会话。
//!
//! **只 INSERT 本次生成的键**：除 `ItemTable` 那两个门控键（Cursor 自己也在写，
//! §4.3 要求刷新）之外，从不改写 Cursor 已有的行。因此不需要（也不可能——本机
//! 1.9 GB）备份整库，回滚由 [`delete_composer_tree`] 精确删掉同一批会话键完成。

use std::path::PathBuf;

use rusqlite::Connection;
use serde_json::{Map, Value};

use crate::adapters::shared::migration::{Fidelity, RenderDecision};
use crate::adapters::shared::narration::narrate;
use crate::adapters::shared::scanner::{clip_text_default, parse_iso8601_ms};
use crate::errors::{DomainError, DomainResult};
use crate::model::{BlockKind, Message, Session, Timestamp, ToolCall, ToolResultStatus};

use super::context::{self, Blob, SHELL_TOOL_NAME};
use super::native_schema::CAPABILITY_TOOL;
use super::workspace::Workspace;
use super::{clock, ids, protobuf, reader, store, workspace};

/// user bubble 的 `type`。
const BUBBLE_USER: i64 = 1;
/// assistant bubble 的 `type`。
const BUBBLE_ASSISTANT: i64 = 2;
/// 工具 bubble 的 `toolFormerData.tool` / header 的 `toolFormerTool`。
const TOOL_FORMER_SHELL: i64 = 15;
/// 工具 header 的 `toolCallCase`。
const TOOL_CALL_CASE_SHELL: &str = "shellToolCall";
/// `composerData._v`：本机同时存在 16 与 17，新写一律用 17。
const COMPOSER_DATA_VERSION: i64 = 17;
/// bubble 的 `_v`，恒为 3。
const BUBBLE_VERSION: i64 = 3;
/// 空 `conversationState` 的字面量，bubble 上恒为它。
const EMPTY_STATE: &str = protobuf::SENTINEL;
/// `textPreview` 的截断长度。
const TEXT_PREVIEW_LIMIT: usize = 200;

/// 写入结果：根会话的新 composerId + 落库位置。
#[derive(Debug)]
pub struct WriteOutcome {
    pub session_id: String,
    pub dest: PathBuf,
}

/// 工具判定回调：迁移目标把 `evaluate_tool` 注进来，plan/preview/write 三路同源。
pub type ToolDecider<'a> =
    &'a dyn Fn(&ToolCall, &Session, &Message) -> DomainResult<RenderDecision>;

fn sqlite_error(error: &rusqlite::Error) -> DomainError {
    DomainError::session_store_unavailable("cursor", &format!("写入失败: {error}"))
}

// ---------------------------------------------------------------------------
// 原生记录模板
// ---------------------------------------------------------------------------

/// 从库里已有的记录采样出的「字段全集」。
///
/// Cursor 每条记录带 60+ 个字段，绝大多数是恒空的 UI 状态。Ferry 显式构造真正有
/// 语义的那十几个，其余键从真实记录里补齐——但**只补中性值**（null/false/0/""/
/// []/{}，含"每一层都空"的嵌套容器），绝不把别的会话的内容抄进来。库里没有样本
/// 时就只写显式字段。
#[derive(Debug, Default)]
struct Templates {
    composer_data: Map<String, Value>,
    user_bubble: Map<String, Value>,
    assistant_bubble: Map<String, Value>,
    /// 真实 user bubble 的 `richText` 是 JSON 字符串还是对象。
    rich_text_as_string: bool,
    /// 采样并深度清空后的 `context` 形状；没有样本时是内置骨架。
    context: Value,
}

/// 中性 = 不携带任何内容。
///
/// 对象**逐层递归**判断：真实 `context` 是 `{"fileSelections":[],...}` 这种「非空
/// 对象、所有叶子为空」的形态，按浅层判空会被当成"有内容"而漏补，Cursor 打开会话时
/// 直接 `Cannot read properties of undefined (reading 'fileSelections')`。
/// 数组则必须**真的为空**才算中性：`[{}]` 虽然不含内容，补过去却凭空多出一个元素。
fn is_neutral(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(entries) => entries.values().all(is_neutral),
        Value::Bool(true) => false,
    }
}

/// 同类型的空值：只保留"它是个什么"，不保留任何内容。
fn empty_like(value: &Value) -> Value {
    match value {
        Value::Object(_) => Value::Object(Map::new()),
        Value::Array(_) => Value::Array(Vec::new()),
        Value::String(_) => Value::from(""),
        Value::Number(_) => Value::from(0),
        Value::Bool(_) => Value::Bool(false),
        Value::Null => Value::Null,
    }
}

/// 由采样到的 `context` 得出要写入的 `context`。
///
/// 三档，越靠前越准：
/// 1. 采样到的本来就是**每层都空**的（Cursor 里大量从未发过消息的草稿就是这样）：
///    直接用，形状与本机 Cursor 版本完全一致；
/// 2. 采样到的带内容：只借它的**顶层键名**补齐骨架里没有的项，空值按类型给，
///    **绝不递归**——嵌套层里躺着的是别的会话的内容，连 map 的键本身都是（
///    `mentions.fileSelections` 以文件 uuid 作键）；
/// 3. 没有采样：内置骨架。
fn context_from(sample: Option<&Value>) -> Value {
    let mut context = default_context();
    let Some(entries) = sample.and_then(Value::as_object) else {
        return context;
    };
    if is_neutral(sample.expect("已确认存在")) {
        return sample.expect("已确认存在").clone();
    }
    let target = context.as_object_mut().expect("骨架是对象");
    for (key, value) in entries {
        if !target.contains_key(key) {
            target.insert(key.clone(), empty_like(value));
        }
    }
    context
}

/// 用模板补齐 target 里缺席的中性字段。
fn fill_neutral(target: &mut Map<String, Value>, template: &Map<String, Value>) {
    for (key, value) in template {
        if !target.contains_key(key) && is_neutral(value) {
            target.insert(key.clone(), value.clone());
        }
    }
}

/// `composerData.context` 的内置骨架（键集取自 Cursor 3.16.17 自己写的记录）。
///
/// **这个键不能缺席**：Cursor 打开会话时直接读 `context.fileSelections`，少了它
/// 会话在列表里点不开，控制台报 `Cannot read properties of undefined`。库里有样本
/// 时以样本的形状为准（跟随本机 Cursor 版本），这里是全新 profile 的兜底。
fn default_context() -> Value {
    /// `context` 顶层的键，全是数组；顺序照抄真实记录。
    const LISTS: &[&str] = &[
        "composers",
        "selectedCommits",
        "selectedPullRequests",
        "selectedImages",
        "selectedDocuments",
        "selectedVideos",
        "folderSelections",
        "fileSelections",
        "selections",
        "terminalSelections",
        "selectedDocs",
        "externalLinks",
        "cursorRules",
        "cursorCommands",
        "gitPRDiffSelections",
        "subagentSelections",
        "browserSelections",
        "extraContext",
    ];
    /// `context.mentions` 的键与容器类型（`true` = 数组，其余是对象）。
    const MENTIONS: &[(&str, bool)] = &[
        ("composers", false),
        ("selectedCommits", false),
        ("selectedPullRequests", false),
        ("gitDiff", true),
        ("gitDiffFromBranchToMain", true),
        ("selectedImages", false),
        ("selectedDocuments", false),
        ("selectedVideos", false),
        ("folderSelections", false),
        ("fileSelections", false),
        ("terminalFiles", false),
        ("selections", false),
        ("terminalSelections", false),
        ("selectedDocs", false),
        ("externalLinks", false),
        ("diffHistory", true),
        ("cursorRules", false),
        ("cursorCommands", false),
        ("uiElementSelections", true),
        ("consoleLogs", true),
        ("ideEditorsState", true),
        ("gitPRDiffSelections", false),
        ("subagentSelections", false),
        ("browserSelections", false),
    ];
    let mut context = Map::new();
    for key in LISTS {
        context.insert((*key).into(), Value::Array(Vec::new()));
    }
    let mut mentions = Map::new();
    for (key, is_list) in MENTIONS {
        let empty = if *is_list {
            Value::Array(Vec::new())
        } else {
            Value::Object(Map::new())
        };
        mentions.insert((*key).into(), empty);
    }
    context.insert("mentions".into(), Value::Object(mentions));
    Value::Object(context)
}

fn sample_values(connection: &Connection, pattern: &str, limit: usize) -> Vec<Value> {
    let Ok(mut statement) = connection.prepare(&format!(
        "SELECT value FROM cursorDiskKV WHERE key GLOB '{pattern}' LIMIT {limit}"
    )) else {
        return Vec::new();
    };
    let Ok(mut rows) = statement.query([]) else {
        return Vec::new();
    };
    let mut samples = Vec::new();
    while let Ok(Some(row)) = rows.next() {
        let Ok(text) = row.get_ref(0).map(store::text_cell) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            samples.push(value);
        }
    }
    samples
}

impl Templates {
    fn load(connection: &Connection) -> Self {
        let mut templates = Self {
            context: default_context(),
            ..Self::default()
        };
        // composerData：先要 `context` 齐全的样本（Ferry 早期写的记录没有这个键），
        // 同等条件下取 `_v` 最大的，避免抄到旧版形态。
        let mut best_score = (false, -1i64);
        let mut best_context: Option<Value> = None;
        let mut neutral_context: Option<Value> = None;
        for sample in sample_values(connection, "composerData:*", 16) {
            let Some(entries) = sample.as_object() else {
                continue;
            };
            let context = entries.get("context").filter(|value| value.is_object());
            if let Some(context) = context.filter(|value| is_neutral(value)) {
                // 空态草稿：本机 Cursor 亲手写的、不带任何内容的 context，形状最准。
                neutral_context.get_or_insert_with(|| context.clone());
            }
            let score = (
                context.is_some(),
                entries.get("_v").and_then(Value::as_i64).unwrap_or(0),
            );
            if score > best_score {
                best_score = score;
                templates.composer_data = entries.clone();
                best_context = context.cloned();
            }
        }
        templates.context = neutral_context.unwrap_or_else(|| context_from(best_context.as_ref()));
        for sample in sample_values(connection, "bubbleId:*", 32) {
            let Some(entries) = sample.as_object() else {
                continue;
            };
            match entries.get("type").and_then(Value::as_i64) {
                Some(BUBBLE_USER) if templates.user_bubble.is_empty() => {
                    templates.rich_text_as_string = entries
                        .get("richText")
                        .is_none_or(|value| !value.is_object());
                    templates.user_bubble = entries.clone();
                }
                // 纯文本 assistant：带 capabilityType 的是工具/思考，形态不同。
                Some(BUBBLE_ASSISTANT)
                    if templates.assistant_bubble.is_empty()
                        && !entries.contains_key("capabilityType") =>
                {
                    templates.assistant_bubble = entries.clone();
                }
                _ => {}
            }
        }
        if templates.user_bubble.is_empty() {
            // 没有样本时按 composerData 的形态推断：`richText` 在那里是字符串。
            templates.rich_text_as_string = true;
        }
        templates
    }

    fn bubble(&self, kind: i64) -> &Map<String, Value> {
        if kind == BUBBLE_USER {
            &self.user_bubble
        } else {
            &self.assistant_bubble
        }
    }
}

// ---------------------------------------------------------------------------
// 编译：canonical 会话 → 一条 Cursor 会话的全部记录
// ---------------------------------------------------------------------------

/// 子代理会话回指父会话的信息。
#[derive(Clone, Debug)]
struct ParentLink {
    composer_id: String,
    agent_type: Option<String>,
    tool_call_id: Option<String>,
}

/// 一条编译完成、尚未落库的 Cursor 会话。
struct Composer {
    id: String,
    parent: Option<ParentLink>,
    title: String,
    created_ms: i64,
    updated_ms: i64,
    /// `(bubbleId, bubble 本体)`，顺序即会话顺序。
    bubbles: Vec<(String, Value)>,
    /// `fullConversationHeadersOnly`，与 `bubbles` 一一对应。
    headers: Vec<Value>,
    blobs: Vec<Blob>,
    digests: Vec<[u8; 32]>,
    children: Vec<String>,
}

fn message_ms(message: &Message) -> Option<i64> {
    match message.created_at.as_ref()? {
        Timestamp::Millis(millis) => Some(*millis),
        Timestamp::Text(text) => parse_iso8601_ms(text),
    }
}

fn text_of(message: &Message) -> String {
    let parts: Vec<&str> = message
        .blocks
        .iter()
        .filter(|block| block.kind == BlockKind::Text && !block.text.is_empty())
        .map(|block| block.text.as_str())
        .collect();
    parts.join("\n")
}

fn take_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// 会话标题；源端没有标题时取首条 user 消息的开头。
///
/// 空标题在 Cursor 的会话列表里显示成 "New Agent"，迁进去的会话彼此无法区分。
/// 截断口径与 scanner 的标题一致（压平空白 + 80 字符 + 省略号）。
fn title_of(node: &Session) -> String {
    if !node.title.trim().is_empty() {
        return node.title.clone();
    }
    node.messages
        .iter()
        .filter(|message| message.role != "assistant")
        .map(text_of)
        .find(|text| !text.trim().is_empty())
        .map(|text| clip_text_default(&text))
        .unwrap_or_default()
}

/// `contentHeightHint`：渲染前的行高估算，Cursor 量完真实高度后自会覆盖。
fn height_hint(text: &str) -> i64 {
    let wrapped = text.chars().count() / 80;
    let lines = text.lines().count().max(1) + wrapped;
    (24 + 20 * lines as i64).min(2_000)
}

/// 展示层 bubble 的公共骨架。
fn base_bubble(
    kind: i64,
    bubble_id: &str,
    text: &str,
    created: &str,
    templates: &Templates,
) -> Map<String, Value> {
    let mut bubble = Map::new();
    bubble.insert("_v".into(), Value::from(BUBBLE_VERSION));
    bubble.insert("bubbleId".into(), Value::from(bubble_id));
    bubble.insert("type".into(), Value::from(kind));
    bubble.insert("text".into(), Value::from(text));
    bubble.insert("createdAt".into(), Value::from(created));
    bubble.insert("conversationState".into(), Value::from(EMPTY_STATE));
    fill_neutral(&mut bubble, templates.bubble(kind));
    bubble
}

/// user bubble 的 `richText`：ProseMirror doc，必须与 `text` 同步。
fn rich_text(text: &str, as_string: bool) -> Value {
    let mut leaf = Map::new();
    leaf.insert("type".into(), Value::from("text"));
    leaf.insert("text".into(), Value::from(text));
    let mut paragraph = Map::new();
    paragraph.insert("type".into(), Value::from("paragraph"));
    if !text.is_empty() {
        paragraph.insert("content".into(), Value::Array(vec![Value::Object(leaf)]));
    }
    let mut document = Map::new();
    document.insert("type".into(), Value::from("doc"));
    document.insert(
        "content".into(),
        Value::Array(vec![Value::Object(paragraph)]),
    );
    let document = Value::Object(document);
    if as_string {
        Value::from(serde_json::to_string(&document).expect("richText 可序列化"))
    } else {
        document
    }
}

/// 纯文本 bubble 的 header 元素。
fn text_header(bubble_id: &str, kind: i64, text: &str, created: &str) -> Value {
    let mut grouping = Map::new();
    grouping.insert("isRenderable".into(), Value::Bool(true));
    grouping.insert("hasText".into(), Value::Bool(!text.is_empty()));
    grouping.insert(
        "textPreview".into(),
        Value::from(take_chars(text, TEXT_PREVIEW_LIMIT)),
    );
    grouping.insert("toolDisplayComputed".into(), Value::Bool(true));
    if kind == BUBBLE_ASSISTANT {
        grouping.insert(
            "isShortPlainText".into(),
            Value::Bool(text.chars().count() <= TEXT_PREVIEW_LIMIT),
        );
        grouping.insert(
            "isKeptFinalAiVisibleOutsideWorkedForGroup".into(),
            Value::Bool(true),
        );
    }
    let mut header = Map::new();
    header.insert("bubbleId".into(), Value::from(bubble_id));
    header.insert("type".into(), Value::from(kind));
    header.insert("grouping".into(), Value::Object(grouping));
    header.insert("contentHeightHint".into(), Value::from(height_hint(text)));
    header.insert("createdAt".into(), Value::from(created));
    Value::Object(header)
}

/// 工具 bubble 的 header 元素：没有 `hasText` / `textPreview` / `contentHeightHint`。
fn tool_header(bubble_id: &str, tool_call_id: &str, ok: bool, created: &str) -> Value {
    let mut grouping = Map::new();
    grouping.insert("isRenderable".into(), Value::Bool(true));
    grouping.insert("capabilityType".into(), Value::from(CAPABILITY_TOOL));
    grouping.insert("toolFormerTool".into(), Value::from(TOOL_FORMER_SHELL));
    grouping.insert(
        "toolFormerStatus".into(),
        Value::from(if ok { "completed" } else { "error" }),
    );
    grouping.insert(
        "shellStatus".into(),
        Value::from(if ok { "success" } else { "error" }),
    );
    grouping.insert("toolCallId".into(), Value::from(tool_call_id));
    grouping.insert("toolCallCase".into(), Value::from(TOOL_CALL_CASE_SHELL));
    grouping.insert("toolDisplayComputed".into(), Value::Bool(true));
    let mut header = Map::new();
    header.insert("bubbleId".into(), Value::from(bubble_id));
    header.insert("type".into(), Value::from(BUBBLE_ASSISTANT));
    header.insert("grouping".into(), Value::Object(grouping));
    header.insert("createdAt".into(), Value::from(created));
    Value::Object(header)
}

/// 一次编译中的累加器。
struct Builder<'a> {
    templates: &'a Templates,
    bubbles: Vec<(String, Value)>,
    headers: Vec<Value>,
    blobs: Vec<Blob>,
    digests: Vec<[u8; 32]>,
    tool_index: i64,
}

impl<'a> Builder<'a> {
    fn new(templates: &'a Templates) -> Self {
        Self {
            templates,
            bubbles: Vec::new(),
            headers: Vec::new(),
            blobs: Vec::new(),
            digests: Vec::new(),
            tool_index: 0,
        }
    }

    fn push_blob(&mut self, message: Value) {
        let blob = context::blob(&message);
        self.digests.push(blob.digest);
        self.blobs.push(blob);
    }

    fn push_user(&mut self, text: &str, created_ms: i64) {
        let created = clock::iso_utc_millis(created_ms);
        let bubble_id = ids::uuid4();
        let mut bubble = base_bubble(BUBBLE_USER, &bubble_id, text, &created, self.templates);
        bubble.insert(
            "richText".into(),
            rich_text(text, self.templates.rich_text_as_string),
        );
        self.headers
            .push(text_header(&bubble_id, BUBBLE_USER, text, &created));
        self.bubbles.push((bubble_id, Value::Object(bubble)));
        self.push_blob(context::user_message(
            text,
            &clock::local_timestamp_label(created_ms),
        ));
    }

    fn push_assistant_text(&mut self, text: &str, created_ms: i64) {
        let created = clock::iso_utc_millis(created_ms);
        let bubble_id = ids::uuid4();
        let bubble = base_bubble(BUBBLE_ASSISTANT, &bubble_id, text, &created, self.templates);
        self.headers
            .push(text_header(&bubble_id, BUBBLE_ASSISTANT, text, &created));
        self.bubbles.push((bubble_id, Value::Object(bubble)));
        self.push_blob(context::assistant_text(text));
    }

    /// 原生终端调用：展示层一条工具 bubble，上下文层一对 tool-call / tool-result。
    fn push_shell_tool(&mut self, rendered: &Map<String, Value>, tool: &ToolCall, created_ms: i64) {
        let created = clock::iso_utc_millis(created_ms);
        let bubble_id = ids::uuid4();
        let call_id = ids::tool_call_id();
        let name = rendered
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("run_terminal_command_v2")
            .to_string();
        let params = rendered
            .get("input")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let output = rendered
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let status = tool
            .result
            .as_ref()
            .map_or(ToolResultStatus::Success, |result| result.status);
        let ok = status != ToolResultStatus::Error;
        let exit_code = tool
            .result
            .as_ref()
            .and_then(|result| result.exit_code)
            .filter(|code| *code != 0);

        let mut result_payload = Map::new();
        result_payload.insert("output".into(), Value::from(output.as_str()));
        result_payload.insert("rejected".into(), Value::Bool(false));
        result_payload.insert("notInterrupted".into(), Value::Bool(true));
        if let Some(code) = exit_code {
            result_payload.insert("exitCode".into(), Value::from(code));
        }
        let mut additional = Map::new();
        additional.insert(
            "status".into(),
            Value::from(if ok { "success" } else { "error" }),
        );
        additional.insert("startedAtMs".into(), Value::from(created_ms));

        let mut former = Map::new();
        former.insert("tool".into(), Value::from(TOOL_FORMER_SHELL));
        former.insert("toolIndex".into(), Value::from(self.tool_index));
        former.insert("modelCallId".into(), Value::from(""));
        former.insert("toolCallId".into(), Value::from(call_id.as_str()));
        former.insert(
            "status".into(),
            Value::from(if ok { "completed" } else { "error" }),
        );
        former.insert("rawArgs".into(), Value::from(""));
        former.insert("name".into(), Value::from(name.as_str()));
        former.insert(
            "params".into(),
            Value::from(serde_json::to_string(&params).expect("入参可序列化")),
        );
        former.insert(
            "result".into(),
            Value::from(
                serde_json::to_string(&Value::Object(result_payload)).expect("结果可序列化"),
            ),
        );
        former.insert("additionalData".into(), Value::Object(additional));
        self.tool_index += 1;

        let mut bubble = base_bubble(BUBBLE_ASSISTANT, &bubble_id, "", &created, self.templates);
        bubble.insert("capabilityType".into(), Value::from(CAPABILITY_TOOL));
        bubble.insert("toolFormerData".into(), Value::Object(former));
        self.headers
            .push(tool_header(&bubble_id, &call_id, ok, &created));
        self.bubbles.push((bubble_id, Value::Object(bubble)));

        // 上下文层的 Shell 参数：只保留模型需要的三项。
        let native = params.as_object().cloned().unwrap_or_default();
        let mut args = Map::new();
        args.insert(
            "command".into(),
            native.get("command").cloned().unwrap_or(Value::from("")),
        );
        for (native_key, arg_key) in [("cwd", "cwd"), ("commandDescription", "description")] {
            match native.get(native_key) {
                Some(Value::String(value)) if !value.is_empty() => {
                    args.insert(arg_key.into(), Value::from(value.as_str()));
                }
                _ => {}
            }
        }
        let command = args
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.push_blob(context::assistant_tool_call(
            &call_id,
            SHELL_TOOL_NAME,
            &Value::Object(args),
        ));
        self.push_blob(context::tool_result(
            &call_id,
            SHELL_TOOL_NAME,
            &command,
            &output,
        ));
    }
}

/// 把一个节点编译成一条 Cursor 会话。
fn compile(
    node: &Session,
    id: String,
    parent: Option<ParentLink>,
    children: Vec<String>,
    templates: &Templates,
    decide: ToolDecider<'_>,
) -> DomainResult<Composer> {
    let mut builder = Builder::new(templates);
    let mut clock_ms = node
        .messages
        .iter()
        .find_map(message_ms)
        .unwrap_or_else(clock::now_ms);
    let created_ms = clock_ms;

    for message in &node.messages {
        let base_ms = message_ms(message).unwrap_or(clock_ms);
        // 同一条消息可能展开成多条 bubble；毫秒逐条递增，保证顺序稳定可读。
        let mut step = 0i64;
        let mut next_ms = || {
            let value = base_ms + step;
            step += 1;
            value
        };
        if message.role != "assistant" {
            builder.push_user(&text_of(message), next_ms());
        } else {
            for block in &message.blocks {
                match (block.kind, block.tool.as_ref()) {
                    (BlockKind::Text, _) if !block.text.is_empty() => {
                        builder.push_assistant_text(&block.text, next_ms());
                    }
                    (BlockKind::Tool, Some(tool)) => {
                        let decision = decide(tool, node, message)?;
                        if decision.fidelity == Fidelity::Dropped {
                            continue;
                        }
                        match decision.rendered.as_ref() {
                            Some(rendered) => {
                                builder.push_shell_tool(rendered, tool, next_ms());
                            }
                            // 无法原生表达的调用：两层都落成历史叙述文本。
                            None => builder.push_assistant_text(&narrate(tool), next_ms()),
                        }
                    }
                    // thinking 与 image 在 Cursor 迁入端没有等价落位，整体丢弃。
                    _ => {}
                }
            }
        }
        clock_ms = base_ms + step.max(1);
    }

    Ok(Composer {
        id,
        parent,
        title: title_of(node),
        created_ms,
        updated_ms: clock_ms,
        bubbles: builder.bubbles,
        headers: builder.headers,
        blobs: builder.blobs,
        digests: builder.digests,
        children,
    })
}

// ---------------------------------------------------------------------------
// 落库
// ---------------------------------------------------------------------------

fn head_value(composer: &Composer, workspace: &Workspace) -> Value {
    let mut head = Map::new();
    head.insert("type".into(), Value::from("head"));
    head.insert("composerId".into(), Value::from(composer.id.as_str()));
    head.insert("name".into(), Value::from(composer.title.as_str()));
    head.insert("createdAt".into(), Value::from(composer.created_ms));
    head.insert("lastUpdatedAt".into(), Value::from(composer.updated_ms));
    head.insert("unifiedMode".into(), Value::from("chat"));
    head.insert("forceMode".into(), Value::from("chat"));
    head.insert("hasUnreadMessages".into(), Value::Bool(false));
    head.insert("totalLinesAdded".into(), Value::from(0));
    head.insert("totalLinesRemoved".into(), Value::from(0));
    head.insert("hasBlockingPendingActions".into(), Value::Bool(false));
    head.insert("isDraft".into(), Value::Bool(false));
    head.insert("isWorktree".into(), Value::Bool(false));
    head.insert("worktreeStartedReadOnly".into(), Value::Bool(false));
    head.insert("isSpec".into(), Value::Bool(false));
    head.insert("isProject".into(), Value::Bool(false));
    head.insert("isBestOfNSubcomposer".into(), Value::Bool(false));
    head.insert(
        "numSubComposers".into(),
        Value::from(composer.children.len() as i64),
    );
    head.insert("referencedPlans".into(), Value::Array(Vec::new()));
    head.insert("trackedGitRepos".into(), Value::Array(Vec::new()));
    head.insert("workspaceIdentifier".into(), workspace.identifier());
    if let Some(parent) = composer.parent.as_ref() {
        let mut info = Map::new();
        info.insert(
            "parentComposerId".into(),
            Value::from(parent.composer_id.as_str()),
        );
        info.insert(
            "subagentTypeName".into(),
            Value::from(parent.agent_type.as_deref().unwrap_or("general")),
        );
        info.insert(
            "toolCallId".into(),
            Value::from(parent.tool_call_id.as_deref().unwrap_or("")),
        );
        head.insert("subagentInfo".into(), Value::Object(info));
    }
    Value::Object(head)
}

fn model_config() -> Value {
    let mut selected = Map::new();
    selected.insert("modelId".into(), Value::from("default"));
    selected.insert("parameters".into(), Value::Array(Vec::new()));
    let mut config = Map::new();
    config.insert("modelName".into(), Value::from("default"));
    config.insert("maxMode".into(), Value::Bool(false));
    config.insert(
        "selectedModels".into(),
        Value::Array(vec![Value::Object(selected)]),
    );
    Value::Object(config)
}

fn composer_data(
    composer: &Composer,
    workspace: &Workspace,
    conversation_state: &str,
    templates: &Templates,
) -> Value {
    let mut data = Map::new();
    data.insert("_v".into(), Value::from(COMPOSER_DATA_VERSION));
    data.insert("composerId".into(), Value::from(composer.id.as_str()));
    data.insert("createdAt".into(), Value::from(composer.created_ms));
    data.insert("lastUpdatedAt".into(), Value::from(composer.updated_ms));
    data.insert("name".into(), Value::from(composer.title.as_str()));
    data.insert("status".into(), Value::from("completed"));
    data.insert("hasLoaded".into(), Value::Bool(true));
    data.insert("unifiedMode".into(), Value::from("chat"));
    data.insert("forceMode".into(), Value::from("chat"));
    data.insert("isAgentic".into(), Value::Bool(false));
    data.insert("conversationState".into(), Value::from(conversation_state));
    data.insert(
        "fullConversationHeadersOnly".into(),
        Value::Array(composer.headers.clone()),
    );
    data.insert("conversationMap".into(), Value::Object(Map::new()));
    // context 是显式字段而不是靠模板补齐：Cursor 打开会话时无条件读
    // `context.fileSelections`，缺席即整条会话点不开（见 default_context 的说明）。
    data.insert("context".into(), templates.context.clone());
    data.insert("todos".into(), Value::Array(Vec::new()));
    data.insert("text".into(), Value::from(""));
    data.insert("richText".into(), Value::from(""));
    data.insert(
        "blobEncryptionKey".into(),
        Value::from(ids::blob_encryption_key()),
    );
    data.insert("modelConfig".into(), model_config());
    data.insert("workspaceIdentifier".into(), workspace.identifier());
    data.insert(
        "subagentComposerIds".into(),
        Value::Array(
            composer
                .children
                .iter()
                .map(|id| Value::from(id.as_str()))
                .collect(),
        ),
    );
    fill_neutral(&mut data, &templates.composer_data);
    Value::Object(data)
}

fn put_kv_text(connection: &Connection, key: &str, value: &str) -> DomainResult<()> {
    connection
        .execute(
            "INSERT OR REPLACE INTO cursorDiskKV (key, value) VALUES (?, ?)",
            rusqlite::params![key, value],
        )
        .map_err(|error| sqlite_error(&error))?;
    Ok(())
}

fn put_kv_bytes(connection: &Connection, key: &str, value: &[u8]) -> DomainResult<()> {
    connection
        .execute(
            "INSERT OR REPLACE INTO cursorDiskKV (key, value) VALUES (?, ?)",
            rusqlite::params![key, value],
        )
        .map_err(|error| sqlite_error(&error))?;
    Ok(())
}

fn table_exists(connection: &Connection, table: &str) -> bool {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?",
            [table],
            |_| Ok(()),
        )
        .is_ok()
}

/// `ItemTable` 的两个门控键（§4.3）。
///
/// `version` 的原生形态是**裸文本**（`<epoch ms>-1`，Phase 5 在真实库上确认），
/// 库里已有该键且写成 JSON 字符串时跟随现状，避免在同一个 profile 里两种形态并存。
fn write_gates(connection: &Connection, now_ms: i64) -> DomainResult<()> {
    if !table_exists(connection, "ItemTable") {
        return Ok(());
    }
    let existing: Option<String> = connection
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'composer.composerHeaders.version'",
            [],
            |row| row.get_ref(0).map(store::text_cell),
        )
        .ok();
    let stamp = format!("{now_ms}-1");
    let version = match existing {
        Some(previous) if previous.starts_with('"') => format!("\"{stamp}\""),
        _ => stamp,
    };
    for (key, value) in [
        ("composer.composerHeaders.tableGateEnabled", "true"),
        ("composer.composerHeaders.version", version.as_str()),
    ] {
        connection
            .execute(
                "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
                rusqlite::params![key, value],
            )
            .map_err(|error| sqlite_error(&error))?;
    }
    Ok(())
}

/// 落一条会话的全部记录：blob → bubble → composerData → header 行。
fn insert_composer(
    connection: &Connection,
    composer: &Composer,
    workspace: &Workspace,
    templates: &Templates,
    now_ms: i64,
) -> DomainResult<()> {
    // blob 必须先落：composerData 的 conversationState 只存摘要，引用不能悬空。
    for blob in &composer.blobs {
        put_kv_bytes(connection, &blob.key, &blob.bytes)?;
    }
    for (bubble_id, bubble) in &composer.bubbles {
        put_kv_text(
            connection,
            &format!("bubbleId:{}:{bubble_id}", composer.id),
            &bubble.to_string(),
        )?;
    }
    let state = protobuf::encode_sentinel(&protobuf::ConversationState {
        digests: &composer.digests,
        workspace_uri: &workspace.uri(),
        timestamp_ms: now_ms,
        timezone: &clock::timezone_name(),
    });
    put_kv_text(
        connection,
        &format!("composerData:{}", composer.id),
        &composer_data(composer, workspace, &state, templates).to_string(),
    )?;
    connection
        .execute(
            "INSERT OR REPLACE INTO composerHeaders (composerId, workspaceId, createdAt, \
             lastUpdatedAt, isArchived, isSubagent, recency, checkpointAt, value) \
             VALUES (?, ?, ?, ?, 0, ?, ?, NULL, ?)",
            rusqlite::params![
                composer.id,
                workspace.id,
                composer.created_ms,
                composer.updated_ms,
                i64::from(composer.parent.is_some()),
                // recency 是 Cursor 侧边栏的排序键：用写入时刻，迁入的会话排最前。
                now_ms,
                head_value(composer, workspace).to_string(),
            ],
        )
        .map_err(|error| sqlite_error(&error))?;
    Ok(())
}

/// 把 canonical 会话树写进 Cursor。
pub fn write(session: &Session, cwd: &str, decide: ToolDecider<'_>) -> DomainResult<WriteOutcome> {
    store::ensure_offline()?;
    let mut connection = store::open_writable()?;
    let workspace = workspace::resolve(&connection, cwd)?;
    let templates = Templates::load(&connection);

    let nodes: Vec<&Session> = session.walk();
    let ids: Vec<String> = nodes.iter().map(|_| ids::uuid4()).collect();
    let position = |target: &Session| {
        nodes
            .iter()
            .position(|candidate| std::ptr::eq(*candidate, target))
    };

    let mut composers: Vec<Composer> = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let children: Vec<String> = node
            .children
            .iter()
            .filter_map(|child| position(child).map(|slot| ids[slot].clone()))
            .collect();
        let parent = nodes.iter().enumerate().find_map(|(slot, candidate)| {
            candidate
                .children
                .iter()
                .find(|child| position(child) == Some(index))
                .map(|child| {
                    let edge = candidate
                        .agent_edges
                        .iter()
                        .find(|edge| edge.child_session_id == child.source_id);
                    ParentLink {
                        composer_id: ids[slot].clone(),
                        agent_type: edge
                            .and_then(|edge| edge.agent_type.clone())
                            .or_else(|| child.agent_type.clone()),
                        tool_call_id: edge.and_then(|edge| edge.source_call_id.clone()),
                    }
                })
        });
        composers.push(compile(
            node,
            ids[index].clone(),
            parent,
            children,
            &templates,
            decide,
        )?);
    }

    let now = clock::now_ms();
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| sqlite_error(&error))?;
    for composer in &composers {
        insert_composer(&transaction, composer, &workspace, &templates, now)?;
    }
    write_gates(&transaction, now)?;
    transaction.commit().map_err(|error| sqlite_error(&error))?;

    Ok(WriteOutcome {
        session_id: ids[0].clone(),
        dest: store::database_path(),
    })
}

/// 删除一条会话及其整棵子代理子树（迁移回滚与失败清理）。
///
/// 不删 `agentKv:blob:`：blob 是内容寻址的，同样内容的消息会被别的会话共用，
/// 删掉可能悄悄掏空另一条会话的上下文。孤儿 blob 对 Cursor 无害。
pub fn delete_composer_tree(session_id: &str) -> DomainResult<()> {
    let mut connection = store::open_writable()?;
    let children = reader::child_map(&connection);
    let mut pending = vec![session_id.to_string()];
    let mut targets: Vec<String> = Vec::new();
    while let Some(current) = pending.pop() {
        if targets.contains(&current) {
            continue;
        }
        if let Some(next) = children.get(&current) {
            pending.extend(next.iter().cloned());
        }
        targets.push(current);
    }

    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| sqlite_error(&error))?;
    for id in &targets {
        for bubble_id in bubble_ids(&transaction, id) {
            transaction
                .execute(
                    "DELETE FROM cursorDiskKV WHERE key = ?",
                    [format!("bubbleId:{id}:{bubble_id}")],
                )
                .map_err(|error| sqlite_error(&error))?;
        }
        transaction
            .execute(
                "DELETE FROM cursorDiskKV WHERE key = ?",
                [format!("composerData:{id}")],
            )
            .map_err(|error| sqlite_error(&error))?;
        transaction
            .execute("DELETE FROM composerHeaders WHERE composerId = ?", [id])
            .map_err(|error| sqlite_error(&error))?;
    }
    transaction.commit().map_err(|error| sqlite_error(&error))?;
    Ok(())
}

/// 一条会话的存活 bubble id 清单（唯一权威来源是 composerData）。
fn bubble_ids(connection: &Connection, session_id: &str) -> Vec<String> {
    let Ok(Some(raw)) = store::disk_kv(connection, &format!("composerData:{session_id}")) else {
        return Vec::new();
    };
    let Ok(data) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };
    data.get("fullConversationHeadersOnly")
        .and_then(Value::as_array)
        .map(|headers| {
            headers
                .iter()
                .filter_map(|header| header.get("bubbleId").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::cursor::migration::CursorMigrationTarget;
    use crate::adapters::cursor::store::tests::{exclusive, materialize};
    use crate::adapters::shared::dialect::register_dialect;
    use crate::adapters::shared::migration::MigrationTargetBase;
    use crate::model::{text_tool_result, Block, ToolResult};
    use serde_json::json;
    use sha2::{Digest, Sha256};

    /// 一份「Cursor 已经在 /w 里聊过」的库：迁入需要它来认工作区哈希。
    fn seeded(root: &std::path::Path) -> std::path::PathBuf {
        let database = root.join("state.vscdb");
        materialize(
            &database,
            &json!({"sessions": [{"id": "existing", "header": {
                "createdAt": 1,
                "workspaceIdentifier": {"id": "3d6aae0c", "uri": {
                    "$mid": 1, "scheme": "file", "fsPath": "/w", "path": "/w",
                    "external": "file:///w"}}},
                "composerData": {"_v": 17, "fullConversationHeadersOnly": []}}]}),
        );
        store::set_database_path_override(Some(database.clone()));
        register_dialect("cursor", &super::super::dialect::DIALECT);
        database
    }

    fn user(text: &str) -> Message {
        let mut message = Message::new("user");
        message.blocks = vec![Block::text(text)];
        message.created_at = Some(Timestamp::Millis(1_780_000_000_000));
        message
    }

    fn assistant(blocks: Vec<Block>) -> Message {
        let mut message = Message::new("assistant");
        message.blocks = blocks;
        message.created_at = Some(Timestamp::Millis(1_780_000_001_000));
        message
    }

    fn tool_block(call: ToolCall) -> Block {
        let mut block = Block::new(BlockKind::Tool);
        block.tool = Some(call);
        block
    }

    fn shell(command: &str, output: &str) -> ToolCall {
        let mut call = ToolCall::new(
            "Bash",
            Some(crate::tool_ops::CanonicalOp::SHELL_EXEC.into()),
            json!({"command": command, "description": "list"}),
        );
        call.result = Some(text_tool_result(output, ToolResultStatus::Success));
        call
    }

    fn write_session(session: &Session) -> WriteOutcome {
        let target = CursorMigrationTarget;
        let decide = |tool: &ToolCall, node: &Session, message: &Message| {
            target.evaluate_tool(tool, node, Some(message))
        };
        write(session, "/w", &decide).expect("迁入必须成功")
    }

    fn composer_data(connection: &Connection, id: &str) -> Value {
        serde_json::from_str(
            &store::disk_kv(connection, &format!("composerData:{id}"))
                .unwrap()
                .expect("composerData 必须落库"),
        )
        .unwrap()
    }

    #[test]
    fn a_text_conversation_lands_in_both_layers() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        let database = seeded(root.path());

        let mut session = Session::new("claude", "src-1", "/w");
        session.title = "迁入标题".into();
        session.messages = vec![user("看看 README"), assistant(vec![Block::text("读完了")])];
        let outcome = write_session(&session);
        assert_eq!(outcome.dest, database);

        let connection = store::open_readonly(&database).unwrap();
        // 展示层：两条 bubble + 顺序权威的 headers。
        let data = composer_data(&connection, &outcome.session_id);
        let headers = data["fullConversationHeadersOnly"].as_array().unwrap();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0]["type"], json!(1));
        assert_eq!(headers[1]["type"], json!(2));
        assert_eq!(headers[0]["grouping"]["textPreview"], json!("看看 README"));
        let first = &store::disk_kv(
            &connection,
            &format!(
                "bubbleId:{}:{}",
                outcome.session_id,
                headers[0]["bubbleId"].as_str().unwrap()
            ),
        )
        .unwrap()
        .unwrap();
        let bubble: Value = serde_json::from_str(first).unwrap();
        assert_eq!(bubble["text"], json!("看看 README"));
        assert_eq!(bubble["conversationState"], json!("~"));
        // richText 必须与 text 同步。
        let rich: Value = serde_json::from_str(bubble["richText"].as_str().unwrap()).unwrap();
        assert_eq!(
            rich["content"][0]["content"][0]["text"],
            json!("看看 README")
        );

        // 上下文层：conversationState 的摘要逐条指向已落库的 blob。
        let state = data["conversationState"].as_str().unwrap();
        let digests = protobuf::decode_digests(state).expect("状态可解码");
        assert_eq!(digests.len(), 2);
        for digest in &digests {
            let key = format!("agentKv:blob:{}", ids::hex_lower(digest));
            let stored = store::disk_kv(&connection, &key)
                .unwrap()
                .expect("blob 必须先落库");
            let actual: [u8; 32] = Sha256::digest(stored.as_bytes()).into();
            assert_eq!(&actual, digest, "键必须是 value 原始字节的 sha256");
        }
        let user_blob: Value = serde_json::from_str(
            &store::disk_kv(
                &connection,
                &format!("agentKv:blob:{}", ids::hex_lower(&digests[0])),
            )
            .unwrap()
            .unwrap(),
        )
        .unwrap();
        assert_eq!(user_blob["role"], json!("user"));
        assert!(user_blob["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("<user_query>\n看看 README\n</user_query>"));
        // 绝不构造 system / user_info blob。
        assert!(!digests.is_empty());
        let roles: Vec<String> = digests
            .iter()
            .map(|digest| {
                let raw = store::disk_kv(
                    &connection,
                    &format!("agentKv:blob:{}", ids::hex_lower(digest)),
                )
                .unwrap()
                .unwrap();
                serde_json::from_str::<Value>(&raw).unwrap()["role"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(roles, ["user", "assistant"]);

        // header 行落在正确的工作区上。
        let (workspace_id, recency): (String, i64) = connection
            .query_row(
                "SELECT workspaceId, recency FROM composerHeaders WHERE composerId = ?",
                [&outcome.session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(workspace_id, "3d6aae0c");
        assert!(recency > 0);
        drop(connection);

        // 读回来必须是同一段对话。
        let restored = reader::read(&outcome.session_id).unwrap();
        assert_eq!(restored.title, "迁入标题");
        assert_eq!(restored.cwd, "/w");
        assert_eq!(restored.messages.len(), 2);
        assert_eq!(restored.messages[0].blocks[0].text, "看看 README");
        assert_eq!(restored.messages[1].blocks[0].text, "读完了");
        store::set_database_path_override(None);
    }

    #[test]
    fn a_shell_call_becomes_a_native_tool_card_and_a_paired_blob() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        let database = seeded(root.path());

        let mut session = Session::new("claude", "src-1", "/w");
        session.messages = vec![
            user("跑一下"),
            assistant(vec![tool_block(shell("ls /tmp", "a\nb"))]),
        ];
        let outcome = write_session(&session);

        let connection = store::open_readonly(&database).unwrap();
        let data = composer_data(&connection, &outcome.session_id);
        let headers = data["fullConversationHeadersOnly"].as_array().unwrap();
        assert_eq!(headers.len(), 2);
        let grouping = &headers[1]["grouping"];
        assert_eq!(grouping["capabilityType"], json!(15));
        assert_eq!(grouping["toolFormerTool"], json!(15));
        assert_eq!(grouping["toolCallCase"], json!("shellToolCall"));
        assert_eq!(grouping["toolFormerStatus"], json!("completed"));
        // 工具 header 没有文本类字段。
        assert!(headers[1].get("contentHeightHint").is_none());

        let bubble: Value = serde_json::from_str(
            &store::disk_kv(
                &connection,
                &format!(
                    "bubbleId:{}:{}",
                    outcome.session_id,
                    headers[1]["bubbleId"].as_str().unwrap()
                ),
            )
            .unwrap()
            .unwrap(),
        )
        .unwrap();
        let former = &bubble["toolFormerData"];
        assert_eq!(bubble["capabilityType"], json!(15));
        assert_eq!(former["name"], json!("run_terminal_command_v2"));
        assert_eq!(former["status"], json!("completed"));
        // params / result 是双重编码的 JSON 字符串。
        let params: Value = serde_json::from_str(former["params"].as_str().unwrap()).unwrap();
        assert_eq!(params["command"], json!("ls /tmp"));
        assert_eq!(params["options"]["timeout"], json!(30000));
        let result: Value = serde_json::from_str(former["result"].as_str().unwrap()).unwrap();
        assert_eq!(result["output"], json!("a\nb"));
        // toolCallBinary 不需要落。
        assert!(bubble.get("toolCallBinary").is_none());

        // 上下文层：user + tool-call + tool-result，id 必须配对。
        let digests =
            protobuf::decode_digests(data["conversationState"].as_str().unwrap()).unwrap();
        assert_eq!(digests.len(), 3);
        let blob_at = |index: usize| -> Value {
            serde_json::from_str(
                &store::disk_kv(
                    &connection,
                    &format!("agentKv:blob:{}", ids::hex_lower(&digests[index])),
                )
                .unwrap()
                .unwrap(),
            )
            .unwrap()
        };
        let call = blob_at(1);
        let outcome_blob = blob_at(2);
        assert_eq!(call["content"][0]["type"], json!("tool-call"));
        assert_eq!(call["content"][0]["toolName"], json!("Shell"));
        assert_eq!(call["content"][0]["args"]["command"], json!("ls /tmp"));
        assert_eq!(outcome_blob["role"], json!("tool"));
        assert_eq!(
            outcome_blob["content"][0]["toolCallId"],
            call["content"][0]["toolCallId"]
        );
        assert_eq!(
            former["toolCallId"], call["content"][0]["toolCallId"],
            "展示层与上下文层必须共用同一个 toolCallId"
        );
        assert_eq!(outcome_blob["content"][0]["result"], json!("a\nb"));
        drop(connection);
        store::set_database_path_override(None);
    }

    #[test]
    fn an_unmappable_tool_degrades_to_history_text_in_both_layers() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        let database = seeded(root.path());

        let mut call = ToolCall::new(
            "Read",
            Some(crate::tool_ops::CanonicalOp::FS_READ.into()),
            json!({"file_path": "/w/README.md"}),
        );
        call.result = Some(text_tool_result("# hi", ToolResultStatus::Success));
        let mut session = Session::new("claude", "src-1", "/w");
        session.messages = vec![user("读一下"), assistant(vec![tool_block(call)])];
        let outcome = write_session(&session);

        let connection = store::open_readonly(&database).unwrap();
        let data = composer_data(&connection, &outcome.session_id);
        let headers = data["fullConversationHeadersOnly"].as_array().unwrap();
        // 降级后是一条普通 assistant 文本，不是工具卡片。
        assert!(headers[1]["grouping"].get("capabilityType").is_none());
        let bubble: Value = serde_json::from_str(
            &store::disk_kv(
                &connection,
                &format!(
                    "bubbleId:{}:{}",
                    outcome.session_id,
                    headers[1]["bubbleId"].as_str().unwrap()
                ),
            )
            .unwrap()
            .unwrap(),
        )
        .unwrap();
        assert!(bubble["text"]
            .as_str()
            .unwrap()
            .starts_with("[History: tool Read"));
        assert!(bubble.get("toolFormerData").is_none());

        // 上下文层同样是一条 assistant 文本，两层叙述一致。
        let digests =
            protobuf::decode_digests(data["conversationState"].as_str().unwrap()).unwrap();
        assert_eq!(digests.len(), 2);
        let blob: Value = serde_json::from_str(
            &store::disk_kv(
                &connection,
                &format!("agentKv:blob:{}", ids::hex_lower(&digests[1])),
            )
            .unwrap()
            .unwrap(),
        )
        .unwrap();
        assert_eq!(blob["role"], json!("assistant"));
        assert_eq!(blob["content"][0]["text"], bubble["text"]);
        drop(connection);
        store::set_database_path_override(None);
    }

    #[test]
    fn a_subagent_tree_keeps_its_topology_and_rolls_back_whole() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        let database = seeded(root.path());

        let mut child = Session::new("claude", "child-1", "/w");
        child.title = "explore".into();
        child.parent_id = Some("src-1".into());
        child.agent_type = Some("explore".into());
        child.messages = vec![user("子任务"), assistant(vec![Block::text("子结果")])];
        let mut session = Session::new("claude", "src-1", "/w");
        session.title = "父会话".into();
        session.messages = vec![user("派个活"), assistant(vec![Block::text("好")])];
        let mut edge = crate::model::AgentEdge::new("src-1", "child-1");
        edge.agent_type = Some("explore".into());
        edge.source_call_id = Some("call-1".into());
        session.agent_edges = vec![edge];
        session.children = vec![child];

        let outcome = write_session(&session);
        let restored = reader::read(&outcome.session_id).unwrap();
        assert_eq!(restored.children.len(), 1);
        assert_eq!(restored.children[0].title, "explore");
        assert_eq!(
            restored.children[0].parent_id.as_deref(),
            Some(outcome.session_id.as_str())
        );
        assert_eq!(restored.children[0].agent_type.as_deref(), Some("explore"));
        assert_eq!(
            restored.root_id.as_deref(),
            Some(outcome.session_id.as_str())
        );

        let connection = store::open_readonly(&database).unwrap();
        let child_id = restored.children[0].source_id.clone();
        let (subagent,): (i64,) = connection
            .query_row(
                "SELECT isSubagent FROM composerHeaders WHERE composerId = ?",
                [&child_id],
                |row| Ok((row.get(0)?,)),
            )
            .unwrap();
        assert_eq!(subagent, 1);
        let head: Value = serde_json::from_str(
            &connection
                .query_row(
                    "SELECT value FROM composerHeaders WHERE composerId = ?",
                    [&child_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            head["subagentInfo"]["parentComposerId"],
            json!(outcome.session_id)
        );
        assert_eq!(head["subagentInfo"]["toolCallId"], json!("call-1"));
        // 父会话记得自己的子代理。
        let data = composer_data(&connection, &outcome.session_id);
        assert_eq!(data["subagentComposerIds"], json!([child_id]));
        drop(connection);

        // 回滚：父子一起消失，邻居会话不受影响。
        delete_composer_tree(&outcome.session_id).unwrap();
        let connection = store::open_readonly(&database).unwrap();
        let remaining: i64 = connection
            .query_row("SELECT COUNT(*) FROM composerHeaders", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
        assert!(store::disk_kv(&connection, "composerData:existing")
            .unwrap()
            .is_some());
        drop(connection);
        store::set_database_path_override(None);
    }

    #[test]
    fn an_unknown_workspace_refuses_to_write() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        seeded(root.path());
        let target = CursorMigrationTarget;
        let decide = |tool: &ToolCall, node: &Session, message: &Message| {
            target.evaluate_tool(tool, node, Some(message))
        };
        let mut session = Session::new("claude", "src-1", "/nowhere");
        session.messages = vec![user("hi")];
        let error = write(&session, "/nowhere", &decide).unwrap_err();
        store::set_database_path_override(None);
        assert_eq!(error.code, "session.store_unavailable");
        assert!(error.message().contains("/nowhere"));
    }

    #[test]
    fn dropped_content_never_reaches_either_layer() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        let database = seeded(root.path());

        let mut thinking = Block::new(BlockKind::Thinking);
        thinking.text = "内心戏".into();
        let mut session = Session::new("claude", "src-1", "/w");
        session.messages = vec![user("在吗"), assistant(vec![thinking, Block::text("在")])];
        let outcome = write_session(&session);

        let connection = store::open_readonly(&database).unwrap();
        let data = composer_data(&connection, &outcome.session_id);
        assert_eq!(
            data["fullConversationHeadersOnly"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let digests =
            protobuf::decode_digests(data["conversationState"].as_str().unwrap()).unwrap();
        assert_eq!(digests.len(), 2);
        drop(connection);
        store::set_database_path_override(None);
    }

    /// 一条「Cursor 自己写的」真实形态 composerData：`context` 的叶子全空，但对象本身
    /// 非空，且带着别的会话的真实内容（文件路径、草稿正文）。
    fn native_sample(cwd: &str) -> Value {
        json!({
            "_v": 17,
            "composerId": "existing",
            "name": "别人的会话",
            "text": "别人的草稿正文",
            "fullConversationHeadersOnly": [],
            // 全空叶子的嵌套对象：递归判空之前会被当成"有内容"而漏补。
            "capabilities": {"todo": {}, "plans": []},
            "context": {
                "composers": [], "selectedCommits": [], "selectedPullRequests": [],
                "selectedImages": [], "selectedDocuments": [], "selectedVideos": [],
                "folderSelections": [],
                // 真实内容：绝不能被抄进新会话。
                "fileSelections": [{"uri": {"$mid": 1, "path": format!("{cwd}/SECRET.md"),
                                            "scheme": "file"},
                                    "uuid": "5d0369d2-35db-4fb3-8df0-147860d4b4a7"}],
                "selections": [], "terminalSelections": [], "selectedDocs": [],
                "externalLinks": [], "cursorRules": [], "cursorCommands": [],
                "gitPRDiffSelections": [], "subagentSelections": [],
                "browserSelections": [], "extraContext": [],
                "mentions": {"composers": {}, "gitDiff": [], "fileSelections": {},
                             "diffHistory": [], "consoleLogs": []}
            }
        })
    }

    /// 在 `seeded` 的库里补一条真实形态的既有会话记录。
    fn seed_native_sample(database: &std::path::Path, cwd: &str) {
        let connection = Connection::open(database).unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO cursorDiskKV (key, value) VALUES (?, ?)",
                rusqlite::params!["composerData:existing", native_sample(cwd).to_string()],
            )
            .unwrap();
    }

    #[test]
    fn context_is_written_with_the_full_native_shape_and_no_borrowed_content() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        let database = seeded(root.path());
        seed_native_sample(&database, "/w");

        let mut session = Session::new("claude", "src-1", "/w");
        session.title = "标题".into();
        session.messages = vec![user("hi"), assistant(vec![Block::text("ok")])];
        let outcome = write_session(&session);

        let connection = store::open_readonly(&database).unwrap();
        let raw = store::disk_kv(&connection, &format!("composerData:{}", outcome.session_id))
            .unwrap()
            .unwrap();
        let data: Value = serde_json::from_str(&raw).unwrap();

        // 缺这个键 Cursor 打开会话时会 Cannot read properties of undefined。
        let context = data["context"].as_object().expect("context 必须存在");
        for key in [
            "composers",
            "selectedCommits",
            "selectedPullRequests",
            "selectedImages",
            "selectedDocuments",
            "selectedVideos",
            "folderSelections",
            "fileSelections",
            "selections",
            "terminalSelections",
            "selectedDocs",
            "externalLinks",
            "cursorRules",
            "cursorCommands",
            "gitPRDiffSelections",
            "subagentSelections",
            "browserSelections",
            "extraContext",
            "mentions",
        ] {
            assert!(context.contains_key(key), "context 少了 {key}");
        }
        assert_eq!(context["fileSelections"], json!([]));
        assert_eq!(context["mentions"]["fileSelections"], json!({}));
        assert_eq!(context["mentions"]["gitDiff"], json!([]));
        // 采样只取形状：别的会话的路径与草稿一个字都不能出现。
        assert!(!raw.contains("SECRET.md"), "抄到了别的会话的文件选择");
        assert!(!raw.contains("别人的草稿正文"));
        assert!(!raw.contains("别人的会话"));
        // 每层都空的嵌套对象要能补齐（浅层判空会漏掉它）。
        assert_eq!(data["capabilities"], json!({"todo": {}, "plans": []}));
        drop(connection);
        store::set_database_path_override(None);
    }

    #[test]
    fn context_falls_back_to_the_builtin_skeleton_without_a_sample() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        // seeded 里的既有会话没有 context 键：全新 profile 的等价情形。
        let database = seeded(root.path());
        let mut session = Session::new("claude", "src-1", "/w");
        session.title = "标题".into();
        session.messages = vec![user("hi")];
        let outcome = write_session(&session);

        let connection = store::open_readonly(&database).unwrap();
        let data: Value = serde_json::from_str(
            &store::disk_kv(&connection, &format!("composerData:{}", outcome.session_id))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(data["context"]["fileSelections"], json!([]));
        assert_eq!(data["context"]["mentions"]["ideEditorsState"], json!([]));
        assert_eq!(data["context"], default_context());
        drop(connection);
        store::set_database_path_override(None);
    }

    #[test]
    fn an_untitled_session_falls_back_to_its_first_user_message() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        let database = seeded(root.path());

        let mut session = Session::new("claude", "src-1", "/w");
        // 源端没有标题：Cursor 列表会显示成 "New Agent"，必须兜底。
        session.title = String::new();
        session.messages = vec![
            assistant(vec![Block::text("先说话的是助手")]),
            user("帮我看看 README 里的构建步骤"),
            assistant(vec![Block::text("好")]),
        ];
        let outcome = write_session(&session);

        let connection = store::open_readonly(&database).unwrap();
        let head: Value = serde_json::from_str(
            &connection
                .query_row(
                    "SELECT value FROM composerHeaders WHERE composerId = ?",
                    [&outcome.session_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(head["name"], json!("帮我看看 README 里的构建步骤"));
        let data: Value = serde_json::from_str(
            &store::disk_kv(&connection, &format!("composerData:{}", outcome.session_id))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(data["name"], json!("帮我看看 README 里的构建步骤"));
        drop(connection);
        store::set_database_path_override(None);

        // 超长首问按 scanner 的口径压平空白并截到 80 字符。
        let mut long = Session::new("claude", "src-2", "/w");
        long.messages = vec![user(&format!("行首\n{}", "x".repeat(200)))];
        let title = title_of(&long);
        assert_eq!(title.chars().count(), 81);
        assert!(title.starts_with("行首 xxx"));
        assert!(title.ends_with('…'));
        // 没有任何 user 消息时不硬造标题。
        let mut silent = Session::new("claude", "src-3", "/w");
        silent.messages = vec![assistant(vec![Block::text("只有助手")])];
        assert_eq!(title_of(&silent), "");
    }

    #[test]
    fn the_header_gate_keys_follow_the_native_value_form() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        let database = seeded(root.path());
        let mut session = Session::new("claude", "src-1", "/w");
        session.title = "标题".into();
        session.messages = vec![user("hi")];
        write_session(&session);

        let connection = store::open_readonly(&database).unwrap();
        let gate = |key: &str| -> String {
            connection
                .query_row("SELECT value FROM ItemTable WHERE key = ?", [key], |row| {
                    row.get_ref(0).map(store::text_cell)
                })
                .unwrap()
        };
        assert_eq!(gate("composer.composerHeaders.tableGateEnabled"), "true");
        // 原生形态是裸文本 `<epoch ms>-1`，不是 JSON 字符串。
        let version = gate("composer.composerHeaders.version");
        assert!(!version.starts_with('"'), "{version}");
        assert!(version.ends_with("-1"), "{version}");
        assert!(version.trim_end_matches("-1").parse::<i64>().unwrap() > 0);
        drop(connection);

        // 库里已经是 JSON 字符串形态时跟随现状，不在同一个 profile 里混两种写法。
        let writable = Connection::open(&database).unwrap();
        writable
            .execute(
                "INSERT OR REPLACE INTO ItemTable (key, value) VALUES \
                 ('composer.composerHeaders.version', '\"1-1\"')",
                [],
            )
            .unwrap();
        drop(writable);
        write_session(&session);
        let connection = store::open_readonly(&database).unwrap();
        assert!(gate_of(&connection).starts_with('"'));
        drop(connection);
        store::set_database_path_override(None);
    }

    fn gate_of(connection: &Connection) -> String {
        connection
            .query_row(
                "SELECT value FROM ItemTable WHERE key = 'composer.composerHeaders.version'",
                [],
                |row| row.get_ref(0).map(store::text_cell),
            )
            .unwrap()
    }

    #[test]
    fn templates_only_donate_neutral_fields() {
        let mut target = Map::new();
        target.insert("text".into(), Value::from("keep"));
        let mut template = Map::new();
        template.insert("text".into(), Value::from("other session"));
        template.insert("supportsChecks".into(), Value::Bool(false));
        template.insert("attachedFiles".into(), Value::Array(Vec::new()));
        template.insert("tokenCount".into(), Value::from(0));
        template.insert("secretNote".into(), Value::from("别的会话的内容"));
        template.insert("isEnabled".into(), Value::Bool(true));
        // 每层都空的嵌套容器算中性；数组里有元素就不算。
        template.insert("nestedEmpty".into(), json!({"a": [], "b": {"c": []}}));
        template.insert("nestedFilled".into(), json!({"a": [{"x": 1}]}));
        template.insert("listOfEmpties".into(), json!([{}]));
        fill_neutral(&mut target, &template);
        assert_eq!(target["nestedEmpty"], json!({"a": [], "b": {"c": []}}));
        assert!(!target.contains_key("nestedFilled"));
        assert!(!target.contains_key("listOfEmpties"));
        assert_eq!(target["text"], Value::from("keep"));
        assert_eq!(target["supportsChecks"], Value::Bool(false));
        assert_eq!(target["attachedFiles"], Value::Array(Vec::new()));
        assert_eq!(target["tokenCount"], Value::from(0));
        // 有内容的字段一律不抄。
        assert!(!target.contains_key("secretNote"));
        assert!(!target.contains_key("isEnabled"));
    }

    #[test]
    fn a_context_sample_with_content_only_donates_its_top_level_keys() {
        // 带内容的样本：只借顶层键名，嵌套层一律不看——`mentions.fileSelections`
        // 的键本身就是别的会话的文件 uuid。
        let context = context_from(Some(&json!({
            "fileSelections": [{"uri": {"path": "/secret"}}],
            "mentions": {"fileSelections": {"7f3a-uuid": {"path": "/secret"}}},
            "brandNewKey": {"nested": [{"leaked": true}]},
            "brandNewList": [1, 2, 3],
        })));
        // 骨架里已有的键不被样本影响。
        assert_eq!(context["fileSelections"], json!([]));
        assert_eq!(context["mentions"]["fileSelections"], json!({}));
        // 新版本新增的键按类型补空，不带任何嵌套内容。
        assert_eq!(context["brandNewKey"], json!({}));
        assert_eq!(context["brandNewList"], json!([]));
        let text = context.to_string();
        assert!(!text.contains("secret") && !text.contains("uuid") && !text.contains("leaked"));

        // 每层都空的样本原样采纳：那是本机 Cursor 自己写的空态。
        let native_empty = json!({"fileSelections": [], "mentions": {"gitDiff": []}});
        assert_eq!(context_from(Some(&native_empty)), native_empty);
        // 没有样本就用内置骨架。
        assert_eq!(context_from(None), default_context());
    }

    #[test]
    fn results_without_a_status_still_write_a_completed_card() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        let database = seeded(root.path());
        let mut call = shell("false", "boom");
        let mut failed = ToolResult::new(ToolResultStatus::Error);
        failed.blocks = vec![crate::model::ToolResultBlock::text("boom")];
        failed.exit_code = Some(1);
        call.result = Some(failed);
        let mut session = Session::new("claude", "src-1", "/w");
        session.messages = vec![user("跑"), assistant(vec![tool_block(call)])];
        let outcome = write_session(&session);

        let connection = store::open_readonly(&database).unwrap();
        let data = composer_data(&connection, &outcome.session_id);
        let headers = data["fullConversationHeadersOnly"].as_array().unwrap();
        assert_eq!(headers[1]["grouping"]["toolFormerStatus"], json!("error"));
        assert_eq!(headers[1]["grouping"]["shellStatus"], json!("error"));
        let bubble: Value = serde_json::from_str(
            &store::disk_kv(
                &connection,
                &format!(
                    "bubbleId:{}:{}",
                    outcome.session_id,
                    headers[1]["bubbleId"].as_str().unwrap()
                ),
            )
            .unwrap()
            .unwrap(),
        )
        .unwrap();
        let result: Value =
            serde_json::from_str(bubble["toolFormerData"]["result"].as_str().unwrap()).unwrap();
        assert_eq!(result["exitCode"], json!(1));
        drop(connection);
        store::set_database_path_override(None);
    }
}
