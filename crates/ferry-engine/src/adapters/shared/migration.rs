//! 格式无关的迁移能力基类与会话树装配。
//!
//! Python 的 `MigrationTargetBase` 是「基类 + 类属性可覆写」；Rust 用
//! [`MigrationTargetBase`] trait 的**默认方法**表达同一件事：
//! - 类属性（`tool_fidelity` / `tool_result_statuses` / `tool_result_native_blocks`
//!   / `tool_result_projected_blocks` / `preserves_tool_result_attachments`）
//!   → 同名的默认方法，adapter 覆写即可；
//! - 可覆写行为（`preview_tool` / `classify_tool_call` / `plan`）→ 默认方法，
//!   方法体统一放在同名的 `default_*` 自由函数里。Rust 没有 `super()`，
//!   覆写后想复用基类逻辑就直接调那个自由函数
//!   （grok 的 `plan` 追加 compaction 统计走的就是 `default_plan(self, session)`）;
//! - `evaluate_tool` 是 plan / preview / writer 三路**唯一**的判定入口，
//!   adapter 不应覆写它，只覆写它调用的 `preview_tool`。
//!
//! 实现了 [`MigrationTargetBase`] 的类型自动获得 `contracts::MigrationTarget`
//! （见文件末尾的 blanket impl），**不要**再手写一份 `impl MigrationTarget`。

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::adapters::contracts::{ScanCache, SessionBrowser};
use crate::errors::{DomainError, DomainResult};
use crate::events::Event;
use crate::loss::{outcome as loss_outcome, Outcome};
use crate::model::{
    tool_result_text, AgentEdge, Block, BlockKind, Message, Session, ToolCall, ToolResultBlockKind,
    ToolResultStatus,
};
use crate::tool_ops::{annotation_inputs, has_valid_tool_input, CanonicalOp};

use super::dialect::{OpBinding, ToolDialect};
use super::narration::narrate;
use super::writing::python_json_dumps_indented;

/// preview 里 summary 的字符上限。
const SUMMARY_LIMIT: usize = 180;
/// preview 里 detail 的字符上限。
const DETAIL_LIMIT: usize = 2500;

/// 一次工具调用在目标端的保真度档位。
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Fidelity {
    /// 原生等价，零损耗。
    #[default]
    Exact,
    /// 形态变了但语义保住（如 workdir 内联进命令）。
    Transformed,
    /// 有字段被丢弃。
    Lossy,
    /// 降级成历史叙述文本。
    Narrated,
    /// 整体丢弃。
    Dropped,
}

impl Fidelity {
    /// 5 个档位。Python 侧 `Fidelity.VALUES` 是 frozenset，迭代序不确定，
    /// 因此 JSON 里这几个 key 的先后**不是** wire 契约。
    pub const VALUES: [Fidelity; 5] = [
        Fidelity::Exact,
        Fidelity::Transformed,
        Fidelity::Lossy,
        Fidelity::Narrated,
        Fidelity::Dropped,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Transformed => "transformed",
            Self::Lossy => "lossy",
            Self::Narrated => "narrated",
            Self::Dropped => "dropped",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::VALUES
            .into_iter()
            .find(|fidelity| fidelity.as_str() == value)
    }

    /// 派生规则：exact → native，dropped → dropped，其余 → degraded。
    pub fn outcome(self) -> &'static str {
        match self {
            Self::Exact => "native",
            Self::Dropped => "dropped",
            _ => "degraded",
        }
    }
}

/// `tool_fidelity` 表的取值。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ToolVerdict {
    /// 目标端有原生形态。
    Native,
    /// 只能降级渲染。
    #[default]
    Degrade,
    /// 直接丢弃。
    Drop,
}

impl ToolVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Degrade => "degrade",
            Self::Drop => "drop",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "native" => Some(Self::Native),
            "degrade" => Some(Self::Degrade),
            "drop" => Some(Self::Drop),
            _ => None,
        }
    }
}

/// `preview_tool` 的返回值：目标端可见的工具块 + 判定用的旁路声明。
///
/// Python 把这些声明塞在同一个 dict 的 `conversion` / `_fidelity` /
/// `_consumed_fields` / `_ignored_fields` / `_reason_codes` 键里，`evaluate_tool`
/// 再逐个 `pop` 出来；Rust 直接拆成结构体字段，[`RenderedTool::block`] 就是
/// pop 完之后剩下的、真正进 preview 输出的那部分。
#[derive(Clone, Debug, Default)]
pub struct RenderedTool {
    /// 目标端块本体：`{"kind","name","input","output"}`（键序即 JSON 键序）。
    pub block: Map<String, Value>,
    /// Python 的 `conversion` 键；只有 `"transformed"` 参与判定。
    pub conversion: Option<String>,
    /// Python 的 `_fidelity`：显式指定后压过所有推导。
    pub fidelity: Option<Fidelity>,
    /// Python 的 `_consumed_fields`；留空则由 `输入字段 - ignored` 推导。
    pub consumed_fields: BTreeSet<String>,
    /// Python 的 `_ignored_fields`。
    pub ignored_fields: BTreeSet<String>,
    /// Python 的 `_reason_codes`。
    pub reason_codes: Vec<String>,
}

impl RenderedTool {
    /// 构造一个目标端工具块。
    pub fn tool(name: &str, input: Value, output: &str) -> Self {
        let mut block = Map::new();
        block.insert("kind".into(), Value::from("tool"));
        block.insert("name".into(), Value::from(name));
        block.insert("input".into(), input);
        block.insert("output".into(), Value::from(output));
        Self {
            block,
            ..Self::default()
        }
    }

    pub fn conversion(mut self, conversion: &str) -> Self {
        self.conversion = Some(conversion.to_string());
        self
    }

    pub fn fidelity(mut self, fidelity: Fidelity) -> Self {
        self.fidelity = Some(fidelity);
        self
    }

    pub fn consumed_fields<I: IntoIterator<Item = String>>(mut self, fields: I) -> Self {
        self.consumed_fields = fields.into_iter().collect();
        self
    }

    pub fn ignored_fields<I: IntoIterator<Item = String>>(mut self, fields: I) -> Self {
        self.ignored_fields = fields.into_iter().collect();
        self
    }

    pub fn reason_codes(mut self, codes: &[&str]) -> Self {
        self.reason_codes = codes.iter().map(|code| (*code).to_string()).collect();
        self
    }
}

/// 一次具体工具调用在目标端的唯一迁移判定。
#[derive(Clone, Debug, Default)]
pub struct RenderDecision {
    pub fidelity: Fidelity,
    pub rendered: Option<Map<String, Value>>,
    pub reason_codes: Vec<String>,
    pub consumed_fields: BTreeSet<String>,
    pub ignored_fields: BTreeSet<String>,
    /// writer 侧的目标记录（Python 的 `Any`），不进 [`RenderDecision::to_value`]。
    pub target_records: Option<Value>,
}

impl RenderDecision {
    pub fn new(fidelity: Fidelity) -> Self {
        Self {
            fidelity,
            ..Self::default()
        }
    }

    pub fn rendered(mut self, rendered: Map<String, Value>) -> Self {
        self.rendered = Some(rendered);
        self
    }

    pub fn reason_codes(mut self, codes: &[&str]) -> Self {
        self.reason_codes = codes.iter().map(|code| (*code).to_string()).collect();
        self
    }

    pub fn consumed_fields<I: IntoIterator<Item = String>>(mut self, fields: I) -> Self {
        self.consumed_fields = fields.into_iter().collect();
        self
    }

    pub fn ignored_fields<I: IntoIterator<Item = String>>(mut self, fields: I) -> Self {
        self.ignored_fields = fields.into_iter().collect();
        self
    }

    pub fn target_records(mut self, records: Value) -> Self {
        self.target_records = Some(records);
        self
    }

    /// 不变量：忽略了字段就必须给出 reason code。
    ///
    /// Python 在 `__post_init__` 抛 `ValueError`（经 RPC 兜底成
    /// `internal.unexpected`）；「未知保真度」那条在 Rust 里由类型系统保证。
    pub fn validated(self) -> DomainResult<Self> {
        if !self.ignored_fields.is_empty() && self.reason_codes.is_empty() {
            return Err(DomainError::internal("忽略工具字段时必须给出 reason code"));
        }
        Ok(self)
    }

    pub fn outcome(&self) -> &'static str {
        self.fidelity.outcome()
    }

    pub fn reason_code(&self) -> Option<&str> {
        self.reason_codes.first().map(String::as_str)
    }

    /// 对齐 `to_dict()`：键序即 JSON 键序。
    pub fn to_value(&self) -> Map<String, Value> {
        let mut value = Map::new();
        value.insert("fidelity".into(), Value::from(self.fidelity.as_str()));
        value.insert("outcome".into(), Value::from(self.outcome()));
        value.insert(
            "rendered".into(),
            self.rendered.clone().map_or(Value::Null, Value::Object),
        );
        value.insert("reason_codes".into(), string_list(&self.reason_codes));
        value.insert(
            "reason_code".into(),
            self.reason_code().map_or(Value::Null, Value::from),
        );
        value.insert("consumed_fields".into(), sorted_list(&self.consumed_fields));
        value.insert("ignored_fields".into(), sorted_list(&self.ignored_fields));
        value
    }
}

// ---------------------------------------------------------------------------
// 会话树装配
// ---------------------------------------------------------------------------

/// 前序遍历扫描元数据树。
fn walk_meta<'a>(nodes: &'a [Value], out: &mut Vec<&'a Map<String, Value>>) {
    for node in nodes {
        let Some(entries) = node.as_object() else {
            continue;
        };
        out.push(entries);
        if let Some(children) = entries.get("children").and_then(Value::as_array) {
            walk_meta(children, out);
        }
    }
}

/// 读取会话并按 scanner 元数据装配整棵父子树。
pub fn assemble_tree(
    browser: &dyn SessionBrowser,
    reference: &str,
    cache: &dyn ScanCache,
) -> DomainResult<Session> {
    let path = browser.resolve_ref(reference)?;
    let mut session = browser.read(&path)?;
    if !session.children.is_empty() {
        return Ok(session);
    }
    let roots = browser.scan(cache)?;
    let root_values: Vec<Value> = roots.into_iter().map(Value::Object).collect();
    let mut flattened = Vec::new();
    walk_meta(&root_values, &mut flattened);
    let target = flattened.into_iter().find(|node| {
        node.get("id").and_then(Value::as_str) == Some(session.source_id.as_str())
            || node
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty() && value == path)
    });
    let Some(target) = target else {
        return Ok(session);
    };
    let root_id = target
        .get("root_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .or_else(|| target.get("id").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    attach(browser, &mut session, target, &root_id)?;
    Ok(session)
}

fn attach(
    browser: &dyn SessionBrowser,
    current: &mut Session,
    meta: &Map<String, Value>,
    root_id: &str,
) -> DomainResult<()> {
    current.source_id = meta
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    current.root_id = Some(root_id.to_string());
    current.parent_id = meta
        .get("parent_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    if current.title.is_empty() {
        current.title = meta
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    if current.cwd.is_empty() {
        current.cwd = meta
            .get("dir")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    let existing = std::mem::take(&mut current.children);
    let child_metas: Vec<Map<String, Value>> = meta
        .get("children")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_object().cloned())
                .collect()
        })
        .unwrap_or_default();
    let mut children = Vec::with_capacity(child_metas.len());
    for child_meta in child_metas {
        let child_id = child_meta.get("id").and_then(Value::as_str).unwrap_or("");
        let mut child = match existing
            .iter()
            .find(|candidate| candidate.source_id == child_id)
        {
            Some(found) => found.clone(),
            None => {
                let locator = child_meta
                    .get("path")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(child_id);
                browser.read(locator)?
            }
        };
        attach(browser, &mut child, &child_meta, root_id)?;
        children.push(child);
    }
    current.children = children;
    Ok(())
}

/// 三级匹配定位工具调用对应的子 Agent 边：source_call_id → agent_id → spawn 消息。
pub fn linked_agent_edge<'a>(
    session: &'a Session,
    tool: &ToolCall,
    message: Option<&Message>,
    allow_message: bool,
) -> Option<&'a AgentEdge> {
    if let Some(call_id) = tool.source_call_id.as_deref().filter(|id| !id.is_empty()) {
        if let Some(edge) = session
            .agent_edges
            .iter()
            .find(|edge| edge.source_call_id.as_deref() == Some(call_id))
        {
            return Some(edge);
        }
    }
    if let Some(agent_id) = tool.agent_id.as_deref().filter(|id| !id.is_empty()) {
        if let Some(edge) = session.agent_edges.iter().find(|edge| {
            edge.agent_id.as_deref() == Some(agent_id) || edge.child_session_id == agent_id
        }) {
            return Some(edge);
        }
    }
    if allow_message {
        let source_id = message
            .and_then(|message| message.source_id.as_deref())
            .filter(|id| !id.is_empty())?;
        let matches: Vec<&AgentEdge> = session
            .agent_edges
            .iter()
            .filter(|edge| edge.spawn_message_id.as_deref() == Some(source_id))
            .collect();
        if matches.len() == 1 {
            return Some(matches[0]);
        }
    }
    None
}

/// 任何提供 browser 能力的插件都可以此作为迁移来源。
pub struct TreeMigrationSource {
    browser: Arc<dyn SessionBrowser>,
}

impl TreeMigrationSource {
    pub fn new(browser: Arc<dyn SessionBrowser>) -> Self {
        Self { browser }
    }

    /// 带扫描缓存的完整装配。
    pub fn export_tree_with_cache(
        &self,
        reference: &str,
        cache: &dyn ScanCache,
    ) -> DomainResult<Session> {
        assemble_tree(self.browser.as_ref(), reference, cache)
    }
}

impl crate::adapters::contracts::MigrationSource for TreeMigrationSource {
    /// 无缓存版本：只读单会话，不做扫描装配。
    ///
    /// Python 的 `export_tree(ref, cache=None)` 若真走到装配分支会用 `None`
    /// 调 `browser.scan`（必然崩），实际链路走的是 `sessions.read_tree`，
    /// 这里取"不装配"这个不会崩的分支；需要装配请用
    /// [`TreeMigrationSource::export_tree_with_cache`]。
    fn export_tree(&self, reference: &str) -> DomainResult<Session> {
        let path = self.browser.resolve_ref(reference)?;
        self.browser.read(&path)
    }
}

// ---------------------------------------------------------------------------
// 迁移目标基类
// ---------------------------------------------------------------------------

/// 迁移目标基类：`write` 由实现方提供，plan / classify / preview 有默认策略。
///
/// 实现方设置 [`MigrationTargetBase::dialect`] 后，preview 的名称与参数映射、
/// 丢失字段计算全部由方言声明推导，只保留真正个性化的判断（agent 边校验、
/// 兜底策略）。
pub trait MigrationTargetBase: Send + Sync {
    /// 目标 Agent id。
    fn tool(&self) -> &str;

    /// 目标端的工具方言；`None` 时 `preview_tool` 走"按 classify 判定"的老路。
    fn dialect(&self) -> Option<&ToolDialect> {
        None
    }

    /// 类级 `tool_fidelity` 表：规范操作 → 判定。缺省一律 `degrade`。
    fn tool_fidelity(&self, _op: &str) -> ToolVerdict {
        ToolVerdict::Degrade
    }

    /// 目标端能原生表达的工具结果状态。
    fn tool_result_statuses(&self) -> &[ToolResultStatus] {
        &[ToolResultStatus::Success, ToolResultStatus::Error]
    }

    /// 目标端能原样保留的结果块类型。
    fn tool_result_native_blocks(&self) -> &[ToolResultBlockKind] {
        &[ToolResultBlockKind::Text]
    }

    /// 目标端只能投影成文本的结果块类型（会记 `tool_result_block_degraded`）。
    fn tool_result_projected_blocks(&self) -> &[ToolResultBlockKind] {
        &[ToolResultBlockKind::Json]
    }

    /// 目标端是否保留工具结果附件。
    fn preserves_tool_result_attachments(&self) -> bool {
        false
    }

    /// 入参非法 → degrade；否则查 `tool_fidelity`。
    fn classify_tool_call(&self, tool_call: &ToolCall) -> ToolVerdict {
        default_classify_tool_call(self, tool_call)
    }

    /// 返回目标端可见的工具块；`None` 表示会降级成历史叙述。
    fn preview_tool(
        &self,
        tool: &ToolCall,
        session: &Session,
        message: Option<&Message>,
    ) -> Option<RenderedTool> {
        default_preview_tool(self, tool, session, message)
    }

    /// 按方言渲染目标端形态；`None` 表示方言没有该操作的原生映射。
    fn dialect_preview(&self, tool: &ToolCall) -> Option<RenderedTool> {
        default_dialect_preview(self, tool)
    }

    /// plan / preview / writer 三路共用的唯一调用级判定入口。
    fn evaluate_tool(
        &self,
        tool: &ToolCall,
        session: &Session,
        message: Option<&Message>,
    ) -> DomainResult<RenderDecision> {
        default_evaluate_tool(self, tool, session, message)
    }

    /// 结果侧的降级判定；顺序逐条对齐 Python，不可重排。
    fn with_result_fidelity(
        &self,
        tool: &ToolCall,
        decision: RenderDecision,
    ) -> DomainResult<RenderDecision> {
        default_with_result_fidelity(self, tool, decision)
    }

    /// 预演统计原生映射/降级/丢弃，与 write 的分发逻辑一致。
    fn plan(&self, session: &Session) -> DomainResult<Map<String, Value>> {
        default_plan(self, session)
    }

    /// 构建写入前可展示的目标会话语义，不修改 session 或目标存储。
    fn preview(&self, session: &Session, cwd: Option<&str>) -> DomainResult<Map<String, Value>> {
        default_preview(self, session, cwd)
    }

    /// 实际写入目标存储；没有默认实现。
    fn write(&self, session: &Session, cwd: &str) -> DomainResult<Map<String, Value>>;
}

/// 任何 [`MigrationTargetBase`] 自动满足 `contracts::MigrationTarget`。
impl<T: MigrationTargetBase> crate::adapters::contracts::MigrationTarget for T {
    fn plan(&self, session: &Session) -> DomainResult<Map<String, Value>> {
        MigrationTargetBase::plan(self, session)
    }

    fn preview(&self, session: &Session, cwd: Option<&str>) -> DomainResult<Map<String, Value>> {
        MigrationTargetBase::preview(self, session, cwd)
    }

    fn write(&self, session: &Session, cwd: &str) -> DomainResult<Map<String, Value>> {
        MigrationTargetBase::write(self, session, cwd)
    }

    fn classify_tool_call(&self, tool_call: &ToolCall) -> String {
        MigrationTargetBase::classify_tool_call(self, tool_call)
            .as_str()
            .to_string()
    }
}

/// [`MigrationTargetBase::classify_tool_call`] 的默认实现体。
pub fn default_classify_tool_call<T: MigrationTargetBase + ?Sized>(
    target: &T,
    tool_call: &ToolCall,
) -> ToolVerdict {
    if !has_valid_tool_input(tool_call.op.as_deref(), &tool_call.input) {
        return ToolVerdict::Degrade;
    }
    tool_call
        .op
        .as_deref()
        .map_or(ToolVerdict::Degrade, |op| target.tool_fidelity(op))
}

/// [`MigrationTargetBase::preview_tool`] 的默认实现体。
pub fn default_preview_tool<T: MigrationTargetBase + ?Sized>(
    target: &T,
    tool: &ToolCall,
    _session: &Session,
    _message: Option<&Message>,
) -> Option<RenderedTool> {
    if target.dialect().is_some() {
        if !has_valid_tool_input(tool.op.as_deref(), &tool.input) {
            return None;
        }
        return target.dialect_preview(tool);
    }
    if target.classify_tool_call(tool) != ToolVerdict::Native {
        return None;
    }
    let output = tool_result_text(tool.result.as_ref());
    Some(RenderedTool::tool(&tool.name, tool.input.clone(), &output).conversion("native"))
}

/// [`MigrationTargetBase::dialect_preview`] 的默认实现体。
pub fn default_dialect_preview<T: MigrationTargetBase + ?Sized>(
    target: &T,
    tool: &ToolCall,
) -> Option<RenderedTool> {
    let dialect = target.dialect()?;
    let value = &tool.input;
    let output = tool_result_text(tool.result.as_ref());
    if tool.op.as_deref() == Some(CanonicalOp::TOOL_INVOKE) {
        let entries = value.as_object()?;
        let namespace = entries.get("namespace").and_then(Value::as_str)?;
        if namespace != dialect.namespace() && namespace != "mcp" {
            return None;
        }
        let name = entries.get("name").and_then(Value::as_str).unwrap_or("");
        let input = entries.get("input").cloned().unwrap_or(Value::Null);
        return Some(
            RenderedTool::tool(name, input, &output)
                .conversion("native")
                .fidelity(Fidelity::Exact)
                .consumed_fields(entries.keys().cloned()),
        );
    }
    let op = tool.op.as_deref()?;
    let (name, native) = dialect.render(op, value)?;
    let mut supported = dialect.supported_fields(op);
    supported.extend(
        annotation_inputs(op)
            .iter()
            .map(|field| (*field).to_string()),
    );
    let fields = input_field_names(tool);
    let ignored: BTreeSet<String> = fields.difference(&supported).cloned().collect();
    let consumed: BTreeSet<String> = fields.difference(&ignored).cloned().collect();
    let mut rendered = RenderedTool::tool(name, Value::Object(native.clone()), &output)
        .conversion("native")
        .consumed_fields(consumed)
        .ignored_fields(ignored.clone());
    if !ignored.is_empty() {
        rendered = rendered.reason_codes(&["unsupported_tool_fields"]);
    }
    if let Some(hook) = dialect
        .binding_for(op)
        .and_then(OpBinding::render_flags_hook)
    {
        let canonical = value.as_object().cloned().unwrap_or_default();
        let flags = hook(&canonical, &native);
        // Python 是 `result.update(flags)`：hook 返回空 dict 时什么都不改，
        // 返回非空时 `_fidelity` 与 `_reason_codes` 一起覆盖。
        if flags.fidelity.is_some() || !flags.reason_codes.is_empty() {
            if let Some(fidelity) = flags.fidelity {
                rendered.fidelity = Some(fidelity);
            }
            rendered.reason_codes = flags.reason_codes;
        }
    }
    Some(rendered)
}

/// 没有原生形态时的判定。
///
/// `ignored_fields` 的语义是**真的没能进目标端的字段**，它会原样出现在
/// `plan.degrade_details` 与 preview 差异卡上，所以必须与实际写入一致：
/// - `narrated`：writer 会写一段 narration（`narration.rs` 的模板里含工具名、入参
///   JSON 与结果），字段一个都没丢，丢的是"这是一次工具调用"这个结构——那已经由
///   `fidelity` 与 reason code 表达，再列一遍字段就是谎报；
/// - `dropped`：整条调用不写入目标端，入参确实全部消失，如实列出。
fn unrendered_decision(
    tool: &ToolCall,
    fidelity: Fidelity,
    reason: &str,
) -> DomainResult<RenderDecision> {
    let decision = RenderDecision::new(fidelity).reason_codes(&[reason]);
    if fidelity == Fidelity::Dropped {
        return decision.ignored_fields(input_field_names(tool)).validated();
    }
    decision.validated()
}

/// [`MigrationTargetBase::evaluate_tool`] 的默认实现体。
pub fn default_evaluate_tool<T: MigrationTargetBase + ?Sized>(
    target: &T,
    tool: &ToolCall,
    session: &Session,
    message: Option<&Message>,
) -> DomainResult<RenderDecision> {
    if !has_valid_tool_input(tool.op.as_deref(), &tool.input) {
        return unrendered_decision(tool, Fidelity::Narrated, "invalid_tool_input");
    }
    let verdict = target.classify_tool_call(tool);
    let Some(rendered) = target.preview_tool(tool, session, message) else {
        let fidelity = if verdict == ToolVerdict::Drop {
            Fidelity::Dropped
        } else {
            Fidelity::Narrated
        };
        let reason = if fidelity == Fidelity::Dropped {
            "tool_unsupported"
        } else {
            "tool_to_history"
        };
        return unrendered_decision(tool, fidelity, reason);
    };

    let RenderedTool {
        block,
        conversion,
        fidelity: explicit,
        mut consumed_fields,
        ignored_fields,
        mut reason_codes,
    } = rendered;
    if consumed_fields.is_empty() && tool.input.is_object() {
        consumed_fields = input_field_names(tool)
            .difference(&ignored_fields)
            .cloned()
            .collect();
    }
    let fidelity = if let Some(explicit) = explicit {
        explicit
    } else if conversion.as_deref() == Some("transformed") || verdict == ToolVerdict::Degrade {
        Fidelity::Transformed
    } else if !ignored_fields.is_empty() {
        Fidelity::Lossy
    } else {
        Fidelity::Exact
    };
    if fidelity != Fidelity::Exact && reason_codes.is_empty() {
        reason_codes = vec![match fidelity {
            Fidelity::Transformed => "tool_transformed",
            Fidelity::Lossy => "tool_fields_ignored",
            _ => "tool_to_history",
        }
        .to_string()];
    }
    let decision = RenderDecision {
        fidelity,
        rendered: Some(block),
        reason_codes,
        consumed_fields,
        ignored_fields,
        target_records: None,
    }
    .validated()?;
    target.with_result_fidelity(tool, decision)
}

/// [`MigrationTargetBase::with_result_fidelity`] 的默认实现体。
pub fn default_with_result_fidelity<T: MigrationTargetBase + ?Sized>(
    target: &T,
    tool: &ToolCall,
    decision: RenderDecision,
) -> DomainResult<RenderDecision> {
    let Some(result) = tool.result.as_ref() else {
        return Ok(decision);
    };
    if decision.rendered.is_none() {
        return Ok(decision);
    }
    if result.status == ToolResultStatus::Unknown {
        return unrendered_decision(tool, Fidelity::Narrated, "unknown_result_status");
    }
    if !target.tool_result_statuses().contains(&result.status) {
        return unrendered_decision(tool, Fidelity::Narrated, "unsupported_result_status");
    }
    let mut reasons = decision.reason_codes.clone();
    let mut fidelity = decision.fidelity;
    let native = target.tool_result_native_blocks();
    let projected_kinds = target.tool_result_projected_blocks();
    let projected = result
        .blocks
        .iter()
        .any(|block| projected_kinds.contains(&block.kind));
    let dropped = result
        .blocks
        .iter()
        .any(|block| !native.contains(&block.kind) && !projected_kinds.contains(&block.kind));
    if projected {
        reasons.push("tool_result_block_degraded".into());
        if fidelity == Fidelity::Exact {
            fidelity = Fidelity::Transformed;
        }
    }
    if dropped {
        reasons.push("tool_result_block_dropped".into());
        fidelity = downgrade_to_lossy(fidelity);
    }
    if !result.attachments.is_empty() && !target.preserves_tool_result_attachments() {
        reasons.push("tool_result_attachments_dropped".into());
        fidelity = downgrade_to_lossy(fidelity);
    }
    if result.truncated == Some(true) {
        reasons.push("tool_result_truncated".into());
        fidelity = downgrade_to_lossy(fidelity);
    }
    if fidelity == decision.fidelity && reasons.is_empty() {
        return Ok(decision);
    }
    RenderDecision {
        fidelity,
        rendered: decision.rendered,
        reason_codes: dedup_in_order(reasons),
        consumed_fields: decision.consumed_fields,
        ignored_fields: decision.ignored_fields,
        target_records: decision.target_records,
    }
    .validated()
}

/// [`MigrationTargetBase::plan`] 的默认实现体。
pub fn default_plan<T: MigrationTargetBase + ?Sized>(
    target: &T,
    session: &Session,
) -> DomainResult<Map<String, Value>> {
    let mut native = 0i64;
    let mut degrade = 0i64;
    let mut counts = [0i64; 5];
    let mut details: Vec<Value> = Vec::new();
    let mut dropped: Vec<Value> = Vec::new();
    for node in session.walk() {
        for loss in &node.loss {
            match loss_outcome(loss) {
                Some(Outcome::Degraded) => {
                    degrade += 1;
                    details.push(event_value(loss));
                }
                Some(Outcome::Dropped) => dropped.push(event_value(loss)),
                None => {}
            }
        }
        for message in &node.messages {
            for block in &message.blocks {
                match (block.kind, block.tool.as_ref()) {
                    (BlockKind::Text, _) => {
                        native += 1;
                        counts[fidelity_index(Fidelity::Exact)] += 1;
                    }
                    (BlockKind::Tool, Some(tool)) => {
                        let decision = target.evaluate_tool(tool, node, Some(message))?;
                        counts[fidelity_index(decision.fidelity)] += 1;
                        match decision.outcome() {
                            "native" => native += 1,
                            "degraded" => {
                                degrade += 1;
                                let mut params = Map::new();
                                params.insert("tool_name".into(), Value::from(tool.name.as_str()));
                                params.insert(
                                    "fidelity".into(),
                                    Value::from(decision.fidelity.as_str()),
                                );
                                params.insert(
                                    "reason_codes".into(),
                                    string_list(&decision.reason_codes),
                                );
                                params.insert(
                                    "ignored_fields".into(),
                                    sorted_list(&decision.ignored_fields),
                                );
                                details.push(event_value(&Event::new(
                                    "migration.tool_degraded",
                                    params,
                                )));
                            }
                            _ => {
                                let mut params = Map::new();
                                params.insert("tool_name".into(), Value::from(tool.name.as_str()));
                                dropped.push(event_value(&Event::new(
                                    "migration.tool_dropped",
                                    params,
                                )));
                            }
                        }
                    }
                    (BlockKind::Image | BlockKind::Thinking, _) => {
                        counts[fidelity_index(Fidelity::Dropped)] += 1;
                        let mut params = Map::new();
                        params.insert("kind".into(), Value::from(block_kind_name(block.kind)));
                        dropped.push(event_value(&Event::new(
                            "migration.content_dropped",
                            params,
                        )));
                    }
                    _ => {}
                }
            }
        }
    }
    let mut result = Map::new();
    result.insert("native".into(), Value::from(native));
    result.insert("degrade".into(), Value::from(degrade));
    result.insert("drop".into(), Value::from(dropped.len() as i64));
    for fidelity in Fidelity::VALUES {
        result.insert(
            fidelity.as_str().into(),
            Value::from(counts[fidelity_index(fidelity)]),
        );
    }
    result.insert("degrade_details".into(), Value::Array(details));
    result.insert("drop_details".into(), Value::Array(dropped));
    Ok(result)
}

/// [`MigrationTargetBase::preview`] 的默认实现体。schema_version 恒为 3。
pub fn default_preview<T: MigrationTargetBase + ?Sized>(
    target: &T,
    session: &Session,
    _cwd: Option<&str>,
) -> DomainResult<Map<String, Value>> {
    let mut differences: Vec<Map<String, Value>> = Vec::new();
    let root = preview_node(target, session, "0", 0, &mut differences)?;

    let degraded = count_kind(&differences, "degraded");
    let dropped = count_kind(&differences, "dropped");

    let mut exact = 0i64;
    for node in session.walk() {
        for message in &node.messages {
            for block in &message.blocks {
                match (block.kind, block.tool.as_ref()) {
                    (BlockKind::Text, _) => exact += 1,
                    (BlockKind::Tool, Some(tool)) => {
                        let fidelity = target.evaluate_tool(tool, node, Some(message))?.fidelity;
                        if fidelity == Fidelity::Exact {
                            exact += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let mut counts = Map::new();
    counts.insert("total".into(), Value::from(degraded + dropped));
    counts.insert("degraded".into(), Value::from(degraded));
    counts.insert("dropped".into(), Value::from(dropped));
    for fidelity in Fidelity::VALUES {
        let total = differences
            .iter()
            .filter(|item| item.get("fidelity").and_then(Value::as_str) == Some(fidelity.as_str()))
            .count() as i64;
        counts.insert(fidelity.as_str().into(), Value::from(total));
    }
    counts.insert(Fidelity::Exact.as_str().into(), Value::from(exact));

    let items: Vec<Value> = differences.into_iter().map(Value::Object).collect();
    let mut difference_block = Map::new();
    difference_block.insert("counts".into(), Value::Object(counts));
    difference_block.insert("items".into(), Value::Array(items));

    let mut preview = Map::new();
    preview.insert("schema_version".into(), Value::from(3));
    preview.insert("target_tool".into(), Value::from(target.tool()));
    preview.insert("root".into(), Value::Object(root));
    preview.insert("read_only".into(), Value::Bool(true));
    preview.insert("differences".into(), Value::Object(difference_block));
    Ok(preview)
}

fn count_kind(differences: &[Map<String, Value>], kind: &str) -> i64 {
    differences
        .iter()
        .filter(|item| item.get("kind").and_then(Value::as_str) == Some(kind))
        .count() as i64
}

/// preview 的单节点递归；`differences` 是整棵树共享的累加器。
fn preview_node<T: MigrationTargetBase + ?Sized>(
    target: &T,
    value: &Session,
    path: &str,
    depth: usize,
    differences: &mut Vec<Map<String, Value>>,
) -> DomainResult<Map<String, Value>> {
    let node_key = format!("n:{path}");
    let mut messages: Vec<Value> = Vec::new();
    let mut node_differences: Vec<PendingDifference> = Vec::new();
    let mut visible_rounds: BTreeSet<String> = BTreeSet::new();
    let mut round_index = 0i64;

    for (message_index, message) in value.messages.iter().enumerate() {
        if message.role == "user" {
            round_index += 1;
        } else if round_index == 0 {
            round_index = 1;
        }
        let message_key = format!("{node_key}/m:{message_index}");
        let round_key = format!("{node_key}/r:{round_index}");
        let mut blocks: Vec<Value> = Vec::new();
        for (block_index, block) in message.blocks.iter().enumerate() {
            let block_key = format!("{message_key}/b:{block_index}");
            match (block.kind, block.tool.as_ref()) {
                (BlockKind::Text, _) if !block.text.is_empty() => {
                    let mut entry = Map::new();
                    entry.insert("key".into(), Value::from(block_key.as_str()));
                    entry.insert("kind".into(), Value::from("text"));
                    entry.insert("text".into(), Value::from(block.text.as_str()));
                    blocks.push(Value::Object(entry));
                }
                (BlockKind::Tool, Some(tool)) => {
                    let decision = target.evaluate_tool(tool, value, Some(message))?;
                    let outcome = decision.outcome();
                    let rendered = match decision.rendered.clone() {
                        Some(mut rendered) => {
                            rendered.remove("conversion");
                            rendered.insert("key".into(), Value::from(block_key.as_str()));
                            blocks.push(Value::Object(rendered.clone()));
                            rendered
                        }
                        None => {
                            let mut rendered = Map::new();
                            rendered.insert("key".into(), Value::from(block_key.as_str()));
                            rendered.insert("kind".into(), Value::from("text"));
                            rendered.insert("text".into(), Value::from(narrate(tool)));
                            if outcome == "degraded" {
                                blocks.push(Value::Object(rendered.clone()));
                            }
                            rendered
                        }
                    };
                    if outcome == "native" {
                        continue;
                    }
                    let target_snapshot = if outcome == "dropped" {
                        None
                    } else if rendered.get("kind").and_then(Value::as_str) == Some("tool") {
                        Some(tool_snapshot(
                            non_empty_str(rendered.get("name")).unwrap_or("history"),
                            rendered.get("input").cloned().unwrap_or(Value::Null),
                            rendered
                                .get("output")
                                .cloned()
                                .unwrap_or_else(|| Value::from("")),
                        ))
                    } else {
                        Some(snapshot(
                            rendered
                                .get("text")
                                .cloned()
                                .unwrap_or_else(|| Value::from("")),
                            rendered.get("kind").and_then(Value::as_str).unwrap_or(""),
                            non_empty_str(rendered.get("name")).unwrap_or("history"),
                        ))
                    };
                    node_differences.push(PendingDifference {
                        diff_id: format!("{block_key}/difference"),
                        kind: outcome.to_string(),
                        fidelity: Some(decision.fidelity),
                        reason_code: decision.reason_code().map(str::to_string),
                        reason_codes: Some(decision.reason_codes.clone()),
                        consumed_fields: sorted_list(&decision.consumed_fields),
                        ignored_fields: sorted_list(&decision.ignored_fields),
                        scope: "block",
                        node_key: node_key.clone(),
                        node_path: path.to_string(),
                        message_key: Some(message_key.clone()),
                        message_index: Some(message_index),
                        block_index: Some(block_index),
                        round_index: Some(round_index),
                        role: Some(message.role.clone()),
                        source: Some(tool_source(tool)),
                        target: target_snapshot,
                        raw_event: None,
                        round_key: Some(round_key.clone()),
                    });
                }
                (BlockKind::Image | BlockKind::Thinking, _) => {
                    let (source, reason_code) = image_or_thinking_source(block);
                    node_differences.push(PendingDifference {
                        diff_id: format!("{block_key}/difference"),
                        kind: "dropped".to_string(),
                        fidelity: Some(Fidelity::Dropped),
                        reason_code: Some(reason_code.to_string()),
                        reason_codes: None,
                        consumed_fields: Value::Array(Vec::new()),
                        ignored_fields: Value::Array(Vec::new()),
                        scope: "block",
                        node_key: node_key.clone(),
                        node_path: path.to_string(),
                        message_key: Some(message_key.clone()),
                        message_index: Some(message_index),
                        block_index: Some(block_index),
                        round_index: Some(round_index),
                        role: Some(message.role.clone()),
                        source: Some(source),
                        target: None,
                        raw_event: None,
                        round_key: Some(round_key.clone()),
                    });
                }
                _ => {}
            }
        }
        if !blocks.is_empty() {
            visible_rounds.insert(round_key.clone());
            let mut entry = Map::new();
            entry.insert("key".into(), Value::from(message_key.as_str()));
            entry.insert("round_index".into(), Value::from(round_index));
            entry.insert(
                "role".into(),
                Value::from(if message.role == "user" || message.role == "assistant" {
                    message.role.as_str()
                } else {
                    "user"
                }),
            );
            entry.insert(
                "created_at".into(),
                serde_json::to_value(&message.created_at).unwrap_or(Value::Null),
            );
            entry.insert("blocks".into(), Value::Array(blocks));
            messages.push(Value::Object(entry));
        }
    }

    for mut pending in node_differences {
        let anchor = pending
            .round_key
            .take()
            .filter(|key| visible_rounds.contains(key));
        differences.push(pending.into_map(value, anchor));
    }

    for (loss_index, loss) in value.loss.iter().enumerate() {
        let Some(outcome) = loss_outcome(loss) else {
            continue;
        };
        let label = if loss.code.is_empty() {
            "source loss"
        } else {
            loss.code.as_str()
        };
        let reason_code = if loss.code.is_empty() {
            "source_loss"
        } else {
            loss.code.as_str()
        };
        let pending = PendingDifference {
            diff_id: format!("{node_key}/loss:{loss_index}"),
            kind: outcome.as_str().to_string(),
            fidelity: Some(if outcome == Outcome::Degraded {
                Fidelity::Narrated
            } else {
                Fidelity::Dropped
            }),
            reason_code: Some(reason_code.to_string()),
            reason_codes: None,
            consumed_fields: Value::Array(Vec::new()),
            ignored_fields: Value::Array(Vec::new()),
            scope: "node",
            node_key: node_key.clone(),
            node_path: path.to_string(),
            message_key: None,
            message_index: None,
            block_index: None,
            round_index: None,
            role: None,
            source: Some(snapshot(Value::Object(loss.params.clone()), "event", label)),
            target: None,
            raw_event: Some(event_value(loss)),
            round_key: None,
        };
        differences.push(pending.into_map(value, None));
    }

    let mut children = Vec::with_capacity(value.children.len());
    for (index, child) in value.children.iter().enumerate() {
        children.push(Value::Object(preview_node(
            target,
            child,
            &format!("{path}.{index}"),
            depth + 1,
            differences,
        )?));
    }

    let mut node = Map::new();
    node.insert("id".into(), Value::from(value.source_id.as_str()));
    node.insert("title".into(), Value::from(value.title.as_str()));
    node.insert("cwd".into(), Value::from(value.cwd.as_str()));
    node.insert("key".into(), Value::from(node_key.as_str()));
    node.insert("path".into(), Value::from(path));
    node.insert(
        "agent_path".into(),
        value.agent_path.as_deref().map_or(Value::Null, Value::from),
    );
    node.insert("depth".into(), Value::from(depth as i64));
    node.insert("messages".into(), Value::Array(messages));
    node.insert("children".into(), Value::Array(children));
    Ok(node)
}

/// preview 差异项的中间形态：Python 里是传给 `add_difference` 的 kwargs。
struct PendingDifference {
    diff_id: String,
    kind: String,
    fidelity: Option<Fidelity>,
    reason_code: Option<String>,
    reason_codes: Option<Vec<String>>,
    consumed_fields: Value,
    ignored_fields: Value,
    scope: &'static str,
    node_key: String,
    node_path: String,
    message_key: Option<String>,
    message_index: Option<usize>,
    block_index: Option<usize>,
    round_index: Option<i64>,
    role: Option<String>,
    source: Option<Map<String, Value>>,
    target: Option<Map<String, Value>>,
    raw_event: Option<Value>,
    round_key: Option<String>,
}

impl PendingDifference {
    /// 键序逐条对齐 Python 的 `add_difference`。
    fn into_map(self, value: &Session, anchor_id: Option<String>) -> Map<String, Value> {
        let reason_codes = self.reason_codes.unwrap_or_else(|| {
            self.reason_code
                .as_ref()
                .map(|code| vec![code.clone()])
                .unwrap_or_default()
        });
        let mut item = Map::new();
        item.insert("id".into(), Value::from(self.diff_id));
        item.insert("kind".into(), Value::from(self.kind));
        item.insert(
            "fidelity".into(),
            self.fidelity
                .map_or(Value::Null, |fidelity| Value::from(fidelity.as_str())),
        );
        item.insert(
            "reason_code".into(),
            self.reason_code.map_or(Value::Null, Value::from),
        );
        item.insert("reason_codes".into(), string_list(&reason_codes));
        item.insert("consumed_fields".into(), self.consumed_fields);
        item.insert("ignored_fields".into(), self.ignored_fields);
        item.insert("scope".into(), Value::from(self.scope));
        item.insert("node_key".into(), Value::from(self.node_key));
        item.insert("node_id".into(), Value::from(value.source_id.as_str()));
        item.insert("node_title".into(), Value::from(value.title.as_str()));
        item.insert("node_path".into(), Value::from(self.node_path));
        item.insert(
            "round_index".into(),
            self.round_index.map_or(Value::Null, Value::from),
        );
        item.insert(
            "message_key".into(),
            self.message_key.map_or(Value::Null, Value::from),
        );
        item.insert(
            "message_index".into(),
            self.message_index
                .map_or(Value::Null, |index| Value::from(index as i64)),
        );
        item.insert(
            "block_index".into(),
            self.block_index
                .map_or(Value::Null, |index| Value::from(index as i64)),
        );
        item.insert("role".into(), self.role.map_or(Value::Null, Value::from));
        item.insert(
            "anchor_id".into(),
            anchor_id.map_or(Value::Null, Value::from),
        );
        item.insert(
            "source".into(),
            self.source.map_or(Value::Null, Value::Object),
        );
        item.insert(
            "target".into(),
            self.target.map_or(Value::Null, Value::Object),
        );
        item.insert("event".into(), self.raw_event.unwrap_or(Value::Null));
        item
    }
}

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

fn string_list(values: &[String]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|item| Value::from(item.as_str()))
            .collect(),
    )
}

fn sorted_list(values: &BTreeSet<String>) -> Value {
    Value::Array(
        values
            .iter()
            .map(|item| Value::from(item.as_str()))
            .collect(),
    )
}

fn input_field_names(tool: &ToolCall) -> BTreeSet<String> {
    tool.input
        .as_object()
        .map(|entries| entries.keys().cloned().collect())
        .unwrap_or_default()
}

fn fidelity_index(fidelity: Fidelity) -> usize {
    Fidelity::VALUES
        .iter()
        .position(|value| *value == fidelity)
        .expect("Fidelity::VALUES 覆盖全部档位")
}

fn downgrade_to_lossy(fidelity: Fidelity) -> Fidelity {
    if matches!(fidelity, Fidelity::Exact | Fidelity::Transformed) {
        Fidelity::Lossy
    } else {
        fidelity
    }
}

fn dedup_in_order(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn event_value(event: &Event) -> Value {
    serde_json::to_value(event).unwrap_or(Value::Null)
}

fn block_kind_name(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::Text => "text",
        BlockKind::Thinking => "thinking",
        BlockKind::Tool => "tool",
        BlockKind::Image => "image",
    }
}

fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

/// 取前 n 个字符（Python 切片按 code point）。
fn take_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn char_count(text: &str) -> usize {
    text.chars().count()
}

/// 差异卡的一侧快照。字符串原样，其余走 `json.dumps(indent=2)`。
pub fn snapshot(value: Value, kind: &str, label: &str) -> Map<String, Value> {
    let text = match &value {
        Value::String(text) => text.clone(),
        other => python_json_dumps_indented(other, 2),
    };
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut summary = take_chars(&compact, SUMMARY_LIMIT);
    if char_count(&compact) > SUMMARY_LIMIT {
        summary.push('…');
    }
    let mut item = Map::new();
    item.insert("kind".into(), Value::from(kind));
    item.insert("label".into(), Value::from(label));
    item.insert("summary".into(), Value::from(summary));
    item.insert("detail".into(), Value::from(clip(&text)));
    item.insert(
        "truncated".into(),
        Value::Bool(char_count(&text) > DETAIL_LIMIT),
    );
    item.insert("char_count".into(), Value::from(char_count(&text) as i64));
    item
}

fn as_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => python_json_dumps_indented(other, 2),
    }
}

fn clip(text: &str) -> String {
    let mut clipped = take_chars(text, DETAIL_LIMIT);
    if char_count(text) > DETAIL_LIMIT {
        clipped.push_str("\n…");
    }
    clipped
}

/// 工具调用快照：detail 保留整体载荷，parts 供 UI 做逐项对照。
pub fn tool_snapshot(label: &str, tool_input: Value, output: Value) -> Map<String, Value> {
    let mut payload = Map::new();
    payload.insert("input".into(), tool_input.clone());
    payload.insert("output".into(), output.clone());
    let mut value = snapshot(Value::Object(payload), "tool", label);
    let mut parts = Map::new();
    parts.insert("input".into(), Value::from(clip(&as_text(&tool_input))));
    parts.insert("output".into(), Value::from(clip(&as_text(&output))));
    value.insert("parts".into(), Value::Object(parts));
    value
}

fn tool_source(tool: &ToolCall) -> Map<String, Value> {
    tool_snapshot(
        &tool.name,
        tool.input.clone(),
        Value::from(tool_result_text(tool.result.as_ref())),
    )
}

fn image_or_thinking_source(block: &Block) -> (Map<String, Value>, &'static str) {
    if block.kind == BlockKind::Thinking {
        return (
            snapshot(Value::from(block.text.as_str()), "thinking", "thinking"),
            "unsupported_thinking",
        );
    }
    let image = block.image.as_ref();
    let mut metadata = Map::new();
    metadata.insert(
        "id".into(),
        Value::from(image.map(|image| image.id.as_str()).unwrap_or("")),
    );
    metadata.insert(
        "mime_type".into(),
        Value::from(image.map(|image| image.mime_type.as_str()).unwrap_or("")),
    );
    metadata.insert(
        "filename".into(),
        image
            .and_then(|image| image.filename.as_deref())
            .map_or(Value::Null, Value::from),
    );
    let label = non_empty_str(metadata.get("filename"))
        .or_else(|| non_empty_str(metadata.get("mime_type")))
        .unwrap_or("image")
        .to_string();
    (
        snapshot(Value::Object(metadata), "image", &label),
        "unsupported_image",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::contracts::ScanRow;
    use crate::adapters::shared::dialect::{
        inline_workdir, workdir_inline_flags, FieldMap, OpBinding,
    };
    use crate::model::{Block, ImageAsset, ToolResult, ToolResultBlock};
    use crate::tool_ops::CanonicalOp;
    use serde_json::json;
    use std::sync::LazyLock;

    /// 与 `scratchpad/oracle.py` 里的 Python 方言逐字段同构：
    /// 期望值全部由 `python3 oracle.py` 跑真实引擎产出后硬编码。
    static PROBE_DIALECT: LazyLock<ToolDialect> = LazyLock::new(|| {
        ToolDialect::new(
            "probe",
            "probe",
            vec![
                OpBinding::new(
                    CanonicalOp::SHELL_EXEC,
                    "Bash",
                    vec![
                        FieldMap::new("command").read_default("").write_default(""),
                        FieldMap::new("timeout_ms").native("timeout"),
                        FieldMap::new("description"),
                    ],
                )
                .encode_post(inline_workdir, ["workdir"])
                .render_flags(workdir_inline_flags),
                OpBinding::new(
                    CanonicalOp::FS_READ,
                    "Read",
                    vec![
                        FieldMap::new("file_path")
                            .read_default("")
                            .write_default(""),
                        FieldMap::new("offset"),
                    ],
                ),
            ],
        )
    });

    struct Probe;

    impl MigrationTargetBase for Probe {
        fn tool(&self) -> &str {
            "probe"
        }

        fn dialect(&self) -> Option<&ToolDialect> {
            Some(&PROBE_DIALECT)
        }

        fn tool_fidelity(&self, op: &str) -> ToolVerdict {
            match op {
                CanonicalOp::SHELL_EXEC | CanonicalOp::FS_READ => ToolVerdict::Native,
                CanonicalOp::WEB_FETCH => ToolVerdict::Drop,
                _ => ToolVerdict::Degrade,
            }
        }

        fn write(&self, _session: &Session, _cwd: &str) -> DomainResult<Map<String, Value>> {
            Err(DomainError::internal("测试目标不写盘"))
        }
    }

    fn call(name: &str, op: &str, input: Value, result: Option<ToolResult>) -> ToolCall {
        let mut tool = ToolCall::new(name, Some(op.to_string()), input);
        tool.result = result;
        tool
    }

    fn text_result(text: &str) -> ToolResult {
        crate::model::text_tool_result(text, ToolResultStatus::Success)
    }

    fn session() -> Session {
        let mut value = Session::new("src", "s1", "/w");
        value.title = "T".into();
        value
    }

    fn decide(tool: &ToolCall) -> RenderDecision {
        Probe.evaluate_tool(tool, &session(), None).unwrap()
    }

    fn codes(decision: &RenderDecision) -> Vec<&str> {
        decision.reason_codes.iter().map(String::as_str).collect()
    }

    fn fields(values: &BTreeSet<String>) -> Vec<&str> {
        values.iter().map(String::as_str).collect()
    }

    #[test]
    fn exact_native_calls_consume_every_field() {
        let decision = decide(&call(
            "Bash",
            CanonicalOp::SHELL_EXEC,
            json!({"command": "ls"}),
            Some(text_result("out")),
        ));
        assert_eq!(decision.fidelity, Fidelity::Exact);
        assert_eq!(decision.outcome(), "native");
        assert!(codes(&decision).is_empty());
        assert_eq!(fields(&decision.consumed_fields), ["command"]);
        assert_eq!(
            decision.rendered.as_ref().unwrap()["input"],
            json!({"command": "ls"})
        );
    }

    #[test]
    fn workdir_inlining_is_transformed_not_lossy() {
        let decision = decide(&call(
            "Bash",
            CanonicalOp::SHELL_EXEC,
            json!({"command": "ls", "workdir": "/a b"}),
            Some(text_result("out")),
        ));
        assert_eq!(decision.fidelity, Fidelity::Transformed);
        assert_eq!(codes(&decision), ["workdir_inlined"]);
        assert_eq!(fields(&decision.consumed_fields), ["command", "workdir"]);
        assert!(decision.ignored_fields.is_empty());
        assert_eq!(
            decision.rendered.as_ref().unwrap()["input"],
            json!({"command": "cd '/a b' && ls"})
        );
    }

    #[test]
    fn unsupported_fields_make_the_call_lossy() {
        let decision = decide(&call(
            "Read",
            CanonicalOp::FS_READ,
            json!({"file_path": "/a", "limit": 5}),
            Some(text_result("out")),
        ));
        assert_eq!(decision.fidelity, Fidelity::Lossy);
        assert_eq!(codes(&decision), ["unsupported_tool_fields"]);
        assert_eq!(fields(&decision.ignored_fields), ["limit"]);
        assert_eq!(fields(&decision.consumed_fields), ["file_path"]);
    }

    #[test]
    fn invalid_input_short_circuits_to_narration() {
        let decision = decide(&call(
            "Bash",
            CanonicalOp::SHELL_EXEC,
            json!({"command": ""}),
            Some(text_result("out")),
        ));
        assert_eq!(decision.fidelity, Fidelity::Narrated);
        assert_eq!(codes(&decision), ["invalid_tool_input"]);
        assert!(decision.rendered.is_none());
        // 降级成叙述文本时入参会原样写进 narration，不能记成"被忽略的字段"。
        assert!(decision.ignored_fields.is_empty());
    }

    #[test]
    fn drop_verdict_and_degrade_verdict_split_the_no_render_path() {
        let dropped = decide(&call(
            "WebFetch",
            CanonicalOp::WEB_FETCH,
            json!({"url": "https://x"}),
            Some(text_result("out")),
        ));
        assert_eq!(dropped.fidelity, Fidelity::Dropped);
        assert_eq!(dropped.outcome(), "dropped");
        assert_eq!(codes(&dropped), ["tool_unsupported"]);
        assert_eq!(fields(&dropped.ignored_fields), ["url"]);

        let narrated = decide(&call(
            "Glob",
            CanonicalOp::FS_GLOB,
            json!({"pattern": "*.rs"}),
            Some(text_result("out")),
        ));
        assert_eq!(narrated.fidelity, Fidelity::Narrated);
        assert_eq!(codes(&narrated), ["tool_to_history"]);
        // dropped 的入参真的没进目标端，narrated 的还在 narration 里。
        assert!(narrated.ignored_fields.is_empty());
    }

    #[test]
    fn narrated_decisions_do_not_claim_fields_that_narration_keeps() {
        // 报告与实际写入必须一致：narration 模板会把入参 JSON 原样写进目标会话，
        // 所以 ignored_fields 必须为空——否则 plan/preview 会谎报字段丢失。
        let tool = call(
            "Glob",
            CanonicalOp::FS_GLOB,
            json!({"pattern": "*.rs"}),
            Some(text_result("out")),
        );
        let decision = decide(&tool);
        assert_eq!(decision.fidelity, Fidelity::Narrated);
        assert!(decision.ignored_fields.is_empty());
        assert!(decision.consumed_fields.is_empty());
        assert!(narrate(&tool).contains("\"pattern\": \"*.rs\""));
    }

    #[test]
    fn result_status_checks_run_before_block_checks() {
        let mut unknown = ToolResult::new(ToolResultStatus::Unknown);
        unknown.blocks.push(ToolResultBlock::text("out"));
        let decision = decide(&call(
            "Bash",
            CanonicalOp::SHELL_EXEC,
            json!({"command": "ls"}),
            Some(unknown),
        ));
        assert_eq!(decision.fidelity, Fidelity::Narrated);
        assert_eq!(codes(&decision), ["unknown_result_status"]);
        // 状态判定会丢掉整份 rendered，改走 narration；入参不算被忽略。
        assert!(decision.rendered.is_none());
        assert!(decision.ignored_fields.is_empty());

        let interrupted = ToolResult::new(ToolResultStatus::Interrupted);
        let decision = decide(&call(
            "Bash",
            CanonicalOp::SHELL_EXEC,
            json!({"command": "ls"}),
            Some(interrupted),
        ));
        assert_eq!(decision.fidelity, Fidelity::Narrated);
        assert_eq!(codes(&decision), ["unsupported_result_status"]);
    }

    #[test]
    fn projected_blocks_degrade_exact_to_transformed() {
        let mut result = ToolResult::new(ToolResultStatus::Success);
        let mut block = ToolResultBlock::new(ToolResultBlockKind::Json);
        block.data = json!({"a": 1});
        result.blocks.push(block);
        let decision = decide(&call(
            "Bash",
            CanonicalOp::SHELL_EXEC,
            json!({"command": "ls"}),
            Some(result),
        ));
        assert_eq!(decision.fidelity, Fidelity::Transformed);
        assert_eq!(codes(&decision), ["tool_result_block_degraded"]);
        assert_eq!(
            decision.rendered.as_ref().unwrap()["output"],
            json!("{\"a\":1}")
        );
    }

    #[test]
    fn unknown_blocks_attachments_and_truncation_all_degrade_to_lossy() {
        let mut with_image = ToolResult::new(ToolResultStatus::Success);
        with_image
            .blocks
            .push(ToolResultBlock::new(ToolResultBlockKind::Image));
        let decision = decide(&call(
            "Bash",
            CanonicalOp::SHELL_EXEC,
            json!({"command": "ls"}),
            Some(with_image),
        ));
        assert_eq!(decision.fidelity, Fidelity::Lossy);
        assert_eq!(codes(&decision), ["tool_result_block_dropped"]);

        let mut with_attachments = text_result("out");
        with_attachments.attachments.push(json!({"a": 1}));
        let decision = decide(&call(
            "Bash",
            CanonicalOp::SHELL_EXEC,
            json!({"command": "ls"}),
            Some(with_attachments),
        ));
        assert_eq!(decision.fidelity, Fidelity::Lossy);
        assert_eq!(codes(&decision), ["tool_result_attachments_dropped"]);

        let mut truncated = text_result("out");
        truncated.truncated = Some(true);
        let decision = decide(&call(
            "Bash",
            CanonicalOp::SHELL_EXEC,
            json!({"command": "ls"}),
            Some(truncated),
        ));
        assert_eq!(decision.fidelity, Fidelity::Lossy);
        assert_eq!(codes(&decision), ["tool_result_truncated"]);
    }

    /// 判定顺序是 wire 面：宿主按 reason code 的先后渲染，顺序不可重排。
    #[test]
    fn stacked_degradations_keep_the_declared_reason_order() {
        let mut result = ToolResult::new(ToolResultStatus::Success);
        let mut json_block = ToolResultBlock::new(ToolResultBlockKind::Json);
        json_block.data = json!({"a": 1});
        result.blocks.push(json_block);
        result
            .blocks
            .push(ToolResultBlock::new(ToolResultBlockKind::File));
        result.attachments.push(json!({"x": 1}));
        result.truncated = Some(true);
        let decision = decide(&call(
            "Bash",
            CanonicalOp::SHELL_EXEC,
            json!({"command": "ls", "workdir": "/a"}),
            Some(result),
        ));
        assert_eq!(decision.fidelity, Fidelity::Lossy);
        assert_eq!(
            codes(&decision),
            [
                "workdir_inlined",
                "tool_result_block_degraded",
                "tool_result_block_dropped",
                "tool_result_attachments_dropped",
                "tool_result_truncated",
            ]
        );
        assert_eq!(
            decision.rendered.as_ref().unwrap()["input"],
            json!({"command": "cd /a && ls"})
        );
    }

    #[test]
    fn decisions_without_reason_codes_cannot_ignore_fields() {
        let error = RenderDecision::new(Fidelity::Lossy)
            .ignored_fields(["x".to_string()])
            .validated()
            .unwrap_err();
        assert_eq!(error.code, "internal.unexpected");
        assert_eq!(error.message(), "忽略工具字段时必须给出 reason code");
    }

    #[test]
    fn to_value_shape_matches_to_dict() {
        let decision = decide(&call(
            "Read",
            CanonicalOp::FS_READ,
            json!({"file_path": "/a", "limit": 5}),
            Some(text_result("out")),
        ));
        let value = decision.to_value();
        let keys: Vec<&str> = value.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "fidelity",
                "outcome",
                "rendered",
                "reason_codes",
                "reason_code",
                "consumed_fields",
                "ignored_fields"
            ]
        );
        assert_eq!(value["reason_code"], json!("unsupported_tool_fields"));
        assert_eq!(value["ignored_fields"], json!(["limit"]));
    }

    // --- plan / preview 整体形状 -------------------------------------------

    fn tree() -> Session {
        crate::loss::declare(&[("probe.custom_degraded", Outcome::Degraded)]);
        let mut root = Session::new("src", "root", "/w");
        root.title = "Root".into();
        let mut params = Map::new();
        params.insert("detail".into(), Value::from("x"));
        root.lose("probe.custom_degraded", params.clone());
        root.lose("never.declared", params);

        let mut first = Message::new("user");
        first.blocks.push(Block::text("hi"));
        first.source_id = Some("m0".into());
        first.created_at = Some(crate::model::Timestamp::Millis(1_700_000_000_000));

        let mut second = Message::new("assistant");
        second.source_id = Some("m1".into());
        second.created_at = Some(crate::model::Timestamp::Millis(1_700_000_000_001));
        for tool in [
            call(
                "Bash",
                CanonicalOp::SHELL_EXEC,
                json!({"command": "ls", "workdir": "/a b"}),
                Some(text_result("out")),
            ),
            call(
                "Glob",
                CanonicalOp::FS_GLOB,
                json!({"pattern": "*"}),
                Some(text_result("g")),
            ),
            call(
                "WebFetch",
                CanonicalOp::WEB_FETCH,
                json!({"url": "https://x"}),
                Some(text_result("w")),
            ),
        ] {
            let mut block = Block::new(BlockKind::Tool);
            block.tool = Some(tool);
            second.blocks.push(block);
        }
        let mut thinking = Block::new(BlockKind::Thinking);
        thinking.text = "secret".into();
        second.blocks.push(thinking);
        let mut image = Block::new(BlockKind::Image);
        image.image = Some(ImageAsset {
            id: "i1".into(),
            mime_type: "image/png".into(),
            data: "QQ==".into(),
            filename: Some("a.png".into()),
        });
        second.blocks.push(image);
        root.messages = vec![first, second];

        let mut child = Session::new("src", "c1", "/w");
        child.title = "Child".into();
        let mut child_message = Message::new("assistant");
        child_message.source_id = Some("c-m0".into());
        child_message.blocks.push(Block::text("child text"));
        child.messages = vec![child_message];
        root.children = vec![child];
        root
    }

    #[test]
    fn plan_counts_match_python() {
        let plan = Probe.plan(&tree()).unwrap();
        assert_eq!(plan["native"], json!(2));
        assert_eq!(plan["degrade"], json!(3));
        assert_eq!(plan["drop"], json!(3));
        assert_eq!(plan["exact"], json!(2));
        assert_eq!(plan["transformed"], json!(1));
        assert_eq!(plan["lossy"], json!(0));
        assert_eq!(plan["narrated"], json!(1));
        assert_eq!(plan["dropped"], json!(3));

        let degraded = plan["degrade_details"].as_array().unwrap();
        let degraded_codes: Vec<&str> = degraded
            .iter()
            .map(|item| item["code"].as_str().unwrap())
            .collect();
        assert_eq!(
            degraded_codes,
            [
                "probe.custom_degraded",
                "migration.tool_degraded",
                "migration.tool_degraded"
            ]
        );
        assert_eq!(
            degraded[1]["params"],
            json!({"tool_name": "Bash", "fidelity": "transformed",
                   "reason_codes": ["workdir_inlined"], "ignored_fields": []})
        );
        let dropped = plan["drop_details"].as_array().unwrap();
        let dropped_codes: Vec<&str> = dropped
            .iter()
            .map(|item| item["code"].as_str().unwrap())
            .collect();
        assert_eq!(
            dropped_codes,
            [
                "migration.tool_dropped",
                "migration.content_dropped",
                "migration.content_dropped"
            ]
        );
        assert_eq!(dropped[1]["params"], json!({"kind": "thinking"}));
        assert_eq!(dropped[2]["params"], json!({"kind": "image"}));
        // 未声明的 loss code 不计入任何一侧。
        assert_eq!(degraded.len() + dropped.len(), 6);
    }

    #[test]
    fn preview_schema_v3_shape_matches_python() {
        let preview = Probe.preview(&tree(), None).unwrap();
        assert_eq!(preview["schema_version"], json!(3));
        assert_eq!(preview["target_tool"], json!("probe"));
        assert_eq!(preview["read_only"], json!(true));

        let root = &preview["root"];
        assert_eq!(root["key"], json!("n:0"));
        assert_eq!(root["path"], json!("0"));
        assert_eq!(root["depth"], json!(0));
        assert_eq!(root["agent_path"], Value::Null);
        assert_eq!(root["children"][0]["key"], json!("n:0.0"));
        assert_eq!(root["children"][0]["path"], json!("0.0"));
        assert_eq!(root["children"][0]["depth"], json!(1));
        assert_eq!(
            root["children"][0]["messages"][0]["created_at"],
            Value::Null
        );

        let messages = root["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["key"], json!("n:0/m:0"));
        assert_eq!(messages[0]["round_index"], json!(1));
        assert_eq!(messages[0]["created_at"], json!(1_700_000_000_000i64));
        assert_eq!(messages[0]["blocks"][0]["key"], json!("n:0/m:0/b:0"));
        // assistant 轮不推进 round_index。
        assert_eq!(messages[1]["round_index"], json!(1));

        // 可见块：原生 Bash 渲染 + Glob 的历史叙述；WebFetch/thinking/image 不进块。
        let blocks = messages[1]["blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["kind"], json!("tool"));
        assert_eq!(blocks[0]["key"], json!("n:0/m:1/b:0"));
        assert_eq!(blocks[0]["input"], json!({"command": "cd '/a b' && ls"}));
        assert!(blocks[0].get("conversion").is_none());
        assert_eq!(
            blocks[1]["text"],
            json!(
                "[History: tool Glob was previously invoked]\n\
                   Input: {\"pattern\": \"*\"}\nResult:\ng"
            )
        );

        let counts = &preview["differences"]["counts"];
        assert_eq!(counts["total"], json!(6));
        assert_eq!(counts["degraded"], json!(3));
        assert_eq!(counts["dropped"], json!(3));
        assert_eq!(counts["exact"], json!(2));
        assert_eq!(counts["transformed"], json!(1));
        assert_eq!(counts["narrated"], json!(2));
        assert_eq!(counts["lossy"], json!(0));

        let items = preview["differences"]["items"].as_array().unwrap();
        let ids: Vec<&str> = items
            .iter()
            .map(|item| item["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "n:0/m:1/b:0/difference",
                "n:0/m:1/b:1/difference",
                "n:0/m:1/b:2/difference",
                "n:0/m:1/b:3/difference",
                "n:0/m:1/b:4/difference",
                "n:0/loss:0",
            ]
        );

        let first = &items[0];
        let keys: Vec<&str> = first
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "id",
                "kind",
                "fidelity",
                "reason_code",
                "reason_codes",
                "consumed_fields",
                "ignored_fields",
                "scope",
                "node_key",
                "node_id",
                "node_title",
                "node_path",
                "round_index",
                "message_key",
                "message_index",
                "block_index",
                "role",
                "anchor_id",
                "source",
                "target",
                "event",
            ]
        );
        assert_eq!(first["anchor_id"], json!("n:0/r:1"));
        assert_eq!(first["node_id"], json!("root"));
        assert_eq!(first["node_title"], json!("Root"));
        assert_eq!(first["scope"], json!("block"));
        assert_eq!(
            first["source"]["summary"],
            json!("{ \"input\": { \"command\": \"ls\", \"workdir\": \"/a b\" }, \"output\": \"out\" }")
        );
        assert_eq!(first["source"]["char_count"], json!(82));
        assert_eq!(
            first["source"]["parts"]["input"],
            json!("{\n  \"command\": \"ls\",\n  \"workdir\": \"/a b\"\n}")
        );
        assert_eq!(first["target"]["char_count"], json!(72));

        // 降级成叙述的目标侧是 text 快照，没有 parts。
        assert_eq!(items[1]["target"]["kind"], json!("text"));
        assert_eq!(items[1]["target"]["label"], json!("history"));
        assert_eq!(items[1]["target"]["char_count"], json!(77));
        assert!(items[1]["target"].get("parts").is_none());
        // 丢弃项没有目标侧。
        assert_eq!(items[2]["target"], Value::Null);
        assert_eq!(items[3]["reason_code"], json!("unsupported_thinking"));
        assert_eq!(items[3]["source"]["kind"], json!("thinking"));
        assert_eq!(items[4]["reason_code"], json!("unsupported_image"));
        assert_eq!(items[4]["source"]["label"], json!("a.png"));
        assert_eq!(
            items[4]["source"]["detail"],
            json!("{\n  \"id\": \"i1\",\n  \"mime_type\": \"image/png\",\n  \"filename\": \"a.png\"\n}")
        );

        let loss = &items[5];
        assert_eq!(loss["scope"], json!("node"));
        assert_eq!(loss["kind"], json!("degraded"));
        assert_eq!(loss["fidelity"], json!("narrated"));
        assert_eq!(loss["reason_code"], json!("probe.custom_degraded"));
        assert_eq!(loss["anchor_id"], Value::Null);
        assert_eq!(loss["round_index"], Value::Null);
        assert_eq!(loss["event"]["code"], json!("probe.custom_degraded"));
        assert_eq!(loss["source"]["kind"], json!("event"));
    }

    #[test]
    fn snapshot_truncates_summary_at_180_and_detail_at_2500() {
        let long = "x".repeat(3000);
        let value = snapshot(Value::from(long.as_str()), "text", "label");
        assert_eq!(value["char_count"], json!(3000));
        assert_eq!(value["truncated"], json!(true));
        let summary = value["summary"].as_str().unwrap();
        assert_eq!(summary.chars().count(), 181);
        assert!(summary.ends_with('…'));
        let detail = value["detail"].as_str().unwrap();
        assert_eq!(detail.chars().count(), 2502);
        assert!(detail.ends_with("\n…"));

        // 恰好等于上限时不加省略号。
        let exact = snapshot(Value::from("y".repeat(180).as_str()), "text", "label");
        assert_eq!(exact["summary"].as_str().unwrap().chars().count(), 180);
        assert_eq!(exact["truncated"], json!(false));
    }

    // --- assemble_tree ------------------------------------------------------

    struct StubBrowser {
        rows: Vec<ScanRow>,
        sessions: std::collections::BTreeMap<String, Session>,
    }

    impl SessionBrowser for StubBrowser {
        fn scan(&self, _cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
            Ok(self.rows.clone())
        }

        fn read(&self, reference: &str) -> DomainResult<Session> {
            self.sessions
                .get(reference)
                .cloned()
                .ok_or_else(|| DomainError::internal(format!("no session: {reference}")))
        }

        fn read_agent(&self, reference: &str) -> DomainResult<Session> {
            self.read(reference)
        }

        fn resolve_ref(&self, reference: &str) -> DomainResult<String> {
            Ok(reference.to_string())
        }

        fn fingerprint(&self, _reference: &str) -> DomainResult<Value> {
            Ok(Value::Null)
        }

        fn agent_fingerprint(&self, _reference: &str) -> DomainResult<Value> {
            Ok(Value::Null)
        }

        fn canonicalize(
            &self,
            _row: &ScanRow,
        ) -> Option<crate::adapters::contracts::NativeSessionReference> {
            None
        }

        fn validate_read_scope(
            &self,
            _reference: &crate::adapters::contracts::NativeSessionReference,
        ) -> DomainResult<()> {
            Ok(())
        }
    }

    struct NullCache;

    impl ScanCache for NullCache {
        fn get(
            &self,
            _path: &std::path::Path,
            _stat: &crate::jsonutil::FileStat,
        ) -> Option<Option<ScanRow>> {
            None
        }

        fn put(
            &self,
            _path: &std::path::Path,
            _stat: &crate::jsonutil::FileStat,
            _meta: Option<ScanRow>,
        ) {
        }

        fn get_digest(
            &self,
            _path: &std::path::Path,
            _stat: &crate::jsonutil::FileStat,
        ) -> Option<String> {
            None
        }

        fn put_digest(
            &self,
            _path: &std::path::Path,
            _stat: &crate::jsonutil::FileStat,
            _digest: &str,
        ) {
        }

        fn flush(&self) {}
    }

    #[test]
    fn assemble_tree_attaches_children_from_scan_metadata() {
        let mut root = Session::new("src", "root", "");
        root.title = String::new();
        let mut child = Session::new("src", "kid", "");
        child.title = "Kid".into();
        let browser = StubBrowser {
            rows: vec![json!({
                "id": "root", "title": "Scanned", "dir": "/scanned",
                "children": [{"id": "kid", "path": "/p/kid.jsonl", "title": "K"}],
            })
            .as_object()
            .cloned()
            .unwrap()],
            sessions: [
                ("/p/root.jsonl".to_string(), root),
                ("/p/kid.jsonl".to_string(), child),
            ]
            .into_iter()
            .collect(),
        };
        let assembled = assemble_tree(&browser, "/p/root.jsonl", &NullCache).unwrap();
        // 标题与 cwd 为空时才从扫描元数据回填。
        assert_eq!(assembled.title, "Scanned");
        assert_eq!(assembled.cwd, "/scanned");
        assert_eq!(assembled.root_id.as_deref(), Some("root"));
        assert_eq!(assembled.parent_id, None);
        assert_eq!(assembled.children.len(), 1);
        assert_eq!(assembled.children[0].source_id, "kid");
        assert_eq!(assembled.children[0].root_id.as_deref(), Some("root"));
        assert_eq!(assembled.children[0].parent_id, None);
        // 子会话自带标题时不被扫描元数据覆盖。
        assert_eq!(assembled.children[0].title, "Kid");
    }

    #[test]
    fn assemble_tree_keeps_sessions_that_already_carry_children() {
        let mut root = Session::new("src", "root", "/w");
        root.children.push(Session::new("src", "kid", "/w"));
        let browser = StubBrowser {
            rows: Vec::new(),
            sessions: [("/p/root.jsonl".to_string(), root)].into_iter().collect(),
        };
        let assembled = assemble_tree(&browser, "/p/root.jsonl", &NullCache).unwrap();
        assert_eq!(assembled.children.len(), 1);
        // scan 没被调用过：rows 是空的，若走了装配分支 children 会被清空。
        assert_eq!(assembled.children[0].source_id, "kid");
    }

    #[test]
    fn linked_agent_edge_matches_in_three_stages() {
        let mut value = Session::new("src", "s1", "/w");
        let mut by_call = AgentEdge::new("s1", "child-a");
        by_call.source_call_id = Some("call-1".into());
        let mut by_agent = AgentEdge::new("s1", "child-b");
        by_agent.agent_id = Some("agent-1".into());
        let mut by_message = AgentEdge::new("s1", "child-c");
        by_message.spawn_message_id = Some("m1".into());
        value.agent_edges = vec![by_call, by_agent, by_message];

        let mut tool = ToolCall::new("Agent", Some(CanonicalOp::AGENT_SPAWN.into()), json!({}));
        tool.source_call_id = Some("call-1".into());
        assert_eq!(
            linked_agent_edge(&value, &tool, None, false)
                .unwrap()
                .child_session_id,
            "child-a"
        );

        tool.source_call_id = None;
        tool.agent_id = Some("agent-1".into());
        assert_eq!(
            linked_agent_edge(&value, &tool, None, false)
                .unwrap()
                .child_session_id,
            "child-b"
        );

        // child_session_id 也参与第二级匹配。
        tool.agent_id = Some("child-c".into());
        assert_eq!(
            linked_agent_edge(&value, &tool, None, false)
                .unwrap()
                .child_session_id,
            "child-c"
        );

        tool.agent_id = None;
        let mut message = Message::new("assistant");
        message.source_id = Some("m1".into());
        assert!(linked_agent_edge(&value, &tool, Some(&message), false).is_none());
        assert_eq!(
            linked_agent_edge(&value, &tool, Some(&message), true)
                .unwrap()
                .child_session_id,
            "child-c"
        );

        // 消息级匹配必须唯一，命中两条就放弃。
        let mut duplicate = AgentEdge::new("s1", "child-d");
        duplicate.spawn_message_id = Some("m1".into());
        value.agent_edges.push(duplicate);
        assert!(linked_agent_edge(&value, &tool, Some(&message), true).is_none());
    }
}
