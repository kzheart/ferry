//! 把当前 Grok 的 ACP `session/update` 信封流聚合成 prompt 轮次。
//!
//! 语义事实源：`engine/adapters/grok/updates.py`。
//!
//! 一条 prompt 由「用户输入 + 助手块序列 + 工具调用表」构成；信封流里同一个
//! prompt 的记录可能被 chunk 打散、可能只带 `promptIndex` 而没有 `promptId`，
//! 也可能出现归属不明的工具事件，这里逐条对齐 Python 的兜底顺序。

use std::collections::{BTreeSet, HashMap};

use serde_json::{Map, Value};

use crate::adapters::shared::dialect::python_str;
use crate::jsonutil::canonical_json;

/// 终态：一旦落到这两个状态，后续非终态更新不再覆盖它。
const TERMINAL_TOOL_STATUSES: [&str; 2] = ["completed", "failed"];

/// 助手块：文本/思考累积成段，工具块只记 call id。
#[derive(Clone, Debug, PartialEq)]
pub enum PromptBlock {
    Text(String),
    Thinking(String),
    Tool(String),
}

/// 一次工具调用在信封流里的聚合结果。
#[derive(Clone, Debug)]
pub struct PromptTool {
    pub id: String,
    pub name: String,
    pub input: Value,
    pub output: Option<Value>,
    pub status: String,
}

/// 一个 prompt 轮次。
#[derive(Clone, Debug)]
pub struct Prompt {
    pub id: String,
    pub user: Vec<String>,
    pub blocks: Vec<PromptBlock>,
    pub tools: Vec<PromptTool>,
    pub unknown: Vec<Value>,
    pub compaction: Option<Value>,
}

impl Prompt {
    fn new(id: String) -> Self {
        Self {
            id,
            user: Vec::new(),
            blocks: Vec::new(),
            tools: Vec::new(),
            unknown: Vec::new(),
            compaction: None,
        }
    }

    pub fn tool(&self, call_id: &str) -> Option<&PromptTool> {
        self.tools.iter().find(|tool| tool.id == call_id)
    }

    fn tool_index(&mut self, call_id: &str) -> Option<usize> {
        self.tools.iter().position(|tool| tool.id == call_id)
    }
}

/// `_text(content)`：字符串原样、对象取 `text`、数组逐项拼接、其余为空串。
fn text_of(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Object(entries)) => match entries.get("text") {
            Some(value) if truthy(value) => python_str(value),
            _ => String::new(),
        },
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| text_of(Some(item)))
            .collect::<Vec<_>>()
            .concat(),
        _ => String::new(),
    }
}

/// Python `bool(value)` 的 JSON 等价。
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|float| float != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(entries) => !entries.is_empty(),
    }
}

/// 一串候选里第一个真值（Python 的 `a or b or c`）。
fn first_truthy(candidates: [Option<&Value>; 4]) -> Option<&Value> {
    candidates.into_iter().flatten().find(|value| truthy(value))
}

fn field<'a>(value: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    value?.get(key)
}

/// `(update, params._meta, update._meta)` 三段视图。
struct Parts<'a> {
    update: Option<&'a Value>,
    meta: Option<&'a Value>,
    nested: Option<&'a Value>,
}

fn parts(envelope: &Value) -> Parts<'_> {
    let params = envelope.get("params");
    let update = field(params, "update");
    Parts {
        update,
        meta: field(params, "_meta"),
        nested: field(update, "_meta"),
    }
}

/// prompt 身份：`(promptId, promptIndex)`。
///
/// `meta.get("promptIndex", nested.get("promptIndex"))` 只在键**缺席**时回落到
/// 嵌套 meta；键存在但为 null 时结果就是 null。
fn prompt_identity(parts: &Parts<'_>) -> (Option<String>, Option<Value>) {
    let prompt_id = first_truthy([
        field(parts.meta, "promptId"),
        field(parts.update, "prompt_id"),
        None,
        None,
    ])
    .map(python_str);
    let prompt_index = match field(parts.meta, "promptIndex") {
        Some(value) => Some(value.clone()),
        None => field(parts.nested, "promptIndex").cloned(),
    }
    .filter(|value| !value.is_null());
    (prompt_id, prompt_index)
}

fn is_tool_event(parts: &Parts<'_>) -> bool {
    let update_type = field(parts.meta, "updateType").and_then(Value::as_str);
    let session_update = field(parts.update, "sessionUpdate").and_then(Value::as_str);
    matches!(update_type, Some("ToolCall" | "ToolCallUpdate"))
        || matches!(session_update, Some("tool_call" | "tool_call_update"))
}

fn call_id(parts: &Parts<'_>) -> String {
    let update_params = field(parts.meta, "updateParams");
    first_truthy([
        field(update_params, "toolCallId"),
        field(parts.update, "toolCallId"),
        field(parts.meta, "toolCallId"),
        None,
    ])
    .map(python_str)
    .unwrap_or_default()
}

/// 工具名的四级兜底：`_meta["x.ai/tool"].name` → `title` → `updateParams.kind`
/// → `kind` → 字面量 `"tool"`。
fn tool_name(parts: &Parts<'_>) -> String {
    let update_params = field(parts.meta, "updateParams");
    let tool_meta = field(parts.nested, "x.ai/tool");
    first_truthy([
        field(tool_meta, "name"),
        field(parts.update, "title"),
        field(update_params, "kind"),
        field(parts.update, "kind"),
    ])
    .map(python_str)
    .unwrap_or_else(|| "tool".to_string())
}

/// 把 prompt_index 折成可比较的 map key。
fn index_key(prompt_index: Option<&Value>) -> String {
    prompt_index
        .and_then(|value| canonical_json(value).ok())
        .unwrap_or_else(|| "null".to_string())
}

/// 聚合信封流。返回的顺序即 prompt 首次出现的顺序。
pub fn aggregate_updates(envelopes: &[Value]) -> Vec<Prompt> {
    // 第一遍：按 promptIndex 反查 promptId，供缺 promptId 的记录归位。
    let mut index_prompt_ids: HashMap<String, BTreeSet<String>> = HashMap::new();
    for envelope in envelopes {
        let parts = parts(envelope);
        let (prompt_id, prompt_index) = prompt_identity(&parts);
        if let (Some(prompt_id), Some(prompt_index)) = (prompt_id, prompt_index) {
            index_prompt_ids
                .entry(index_key(Some(&prompt_index)))
                .or_default()
                .insert(prompt_id);
        }
    }

    let prompt_key = |parts: &Parts<'_>| -> Option<String> {
        let (prompt_id, prompt_index) = prompt_identity(parts);
        if let Some(prompt_id) = prompt_id {
            return Some(prompt_id);
        }
        let candidates = prompt_index
            .as_ref()
            .and_then(|index| index_prompt_ids.get(&index_key(Some(index))));
        if let Some(candidates) = candidates {
            if candidates.len() == 1 {
                return candidates.iter().next().cloned();
            }
        }
        prompt_index.map(|index| format!("prompt:{}", python_str(&index)))
    };

    // 第二遍：工具调用的归属集合；跨多个 prompt 的调用无法确定归属。
    let mut call_owners: HashMap<String, BTreeSet<String>> = HashMap::new();
    for envelope in envelopes {
        let parts = parts(envelope);
        if !is_tool_event(&parts) {
            continue;
        }
        let call = call_id(&parts);
        if let (false, Some(key)) = (call.is_empty(), prompt_key(&parts)) {
            call_owners.entry(call).or_default().insert(key);
        }
    }

    /// 按 id 取 prompt 槽位，没有就按首次出现顺序新建。
    fn ensure_prompt(
        prompts: &mut Vec<Prompt>,
        order: &mut HashMap<String, usize>,
        prompt_id: &str,
    ) -> usize {
        *order.entry(prompt_id.to_string()).or_insert_with(|| {
            prompts.push(Prompt::new(prompt_id.to_string()));
            prompts.len() - 1
        })
    }

    let mut prompts: Vec<Prompt> = Vec::new();
    let mut order: HashMap<String, usize> = HashMap::new();

    for envelope in envelopes {
        let parts = parts(envelope);
        let update_type = field(parts.meta, "updateType")
            .and_then(Value::as_str)
            .map(str::to_string);
        let kind = field(parts.update, "kind")
            .and_then(Value::as_str)
            .map(str::to_string);
        let session_update = field(parts.update, "sessionUpdate")
            .and_then(Value::as_str)
            .map(str::to_string);
        let is_tool = is_tool_event(&parts);
        let mut prompt_id = prompt_key(&parts);
        let call = if is_tool {
            call_id(&parts)
        } else {
            String::new()
        };
        let owners = call_owners.get(&call);
        if is_tool && prompt_id.is_none() {
            if let Some(owners) = owners {
                if owners.len() == 1 {
                    prompt_id = owners.iter().next().cloned();
                }
            }
        }
        // 归属不明或跨多个 prompt 的工具事件统一进 unassigned 桶。
        if is_tool && (prompt_id.is_none() || owners.is_some_and(|owners| owners.len() > 1)) {
            let index = ensure_prompt(&mut prompts, &mut order, "prompt:unassigned");
            prompts[index].unknown.push(envelope.clone());
            continue;
        }
        let prompt_id = prompt_id.unwrap_or_else(|| format!("prompt:{}", prompts.len()));
        let index = ensure_prompt(&mut prompts, &mut order, &prompt_id);

        let is_user = matches!(update_type.as_deref(), Some("UserMessage" | "Prompt"))
            || session_update.as_deref() == Some("user_message_chunk")
            || matches!(kind.as_deref(), Some("user_message" | "prompt"));
        let is_agent_text = update_type.as_deref() == Some("AgentMessageChunk")
            || session_update.as_deref() == Some("agent_message_chunk");
        let is_thought = update_type.as_deref() == Some("AgentThoughtChunk")
            || session_update.as_deref() == Some("agent_thought_chunk");
        let is_compaction =
            kind.as_deref() == Some("compaction") || update_type.as_deref() == Some("Compaction");

        if is_user {
            let text = text_of(field(parts.update, "content"));
            prompts[index].user.push(text);
        } else if is_agent_text {
            let text = text_of(field(parts.update, "content"));
            match prompts[index].blocks.last_mut() {
                Some(PromptBlock::Text(existing)) => existing.push_str(&text),
                _ => prompts[index].blocks.push(PromptBlock::Text(text)),
            }
        } else if is_thought {
            let text = text_of(field(parts.update, "content"));
            match prompts[index].blocks.last_mut() {
                Some(PromptBlock::Thinking(existing)) => existing.push_str(&text),
                _ => prompts[index].blocks.push(PromptBlock::Thinking(text)),
            }
        } else if is_tool {
            if call.is_empty() {
                prompts[index].unknown.push(envelope.clone());
                continue;
            }
            let name = tool_name(&parts);
            let position = match prompts[index].tool_index(&call) {
                Some(position) => {
                    // 首次登记时若只拿到兜底名，后续更新可以把真名补上。
                    if prompts[index].tools[position].name == "tool" && name != "tool" {
                        prompts[index].tools[position].name = name;
                    }
                    position
                }
                None => {
                    prompts[index].tools.push(PromptTool {
                        id: call.clone(),
                        name,
                        input: Value::Object(Map::new()),
                        output: None,
                        status: "unknown".to_string(),
                    });
                    prompts[index].blocks.push(PromptBlock::Tool(call.clone()));
                    prompts[index].tools.len() - 1
                }
            };
            if let Some(raw_input) = field(parts.update, "rawInput") {
                prompts[index].tools[position].input = raw_input.clone();
            }
            if let Some(raw_output) = field(parts.update, "rawOutput") {
                prompts[index].tools[position].output = Some(raw_output.clone());
            }
            let content_text = text_of(field(parts.update, "content"));
            if !content_text.is_empty() {
                prompts[index].tools[position].output = Some(Value::from(content_text));
            }
            let status = field(field(parts.meta, "updateParams"), "status")
                .filter(|value| truthy(value))
                .map(|value| python_str(value).to_lowercase());
            if let Some(status) = status {
                let current = &prompts[index].tools[position].status;
                if !TERMINAL_TOOL_STATUSES.contains(&current.as_str())
                    || TERMINAL_TOOL_STATUSES.contains(&status.as_str())
                {
                    prompts[index].tools[position].status = status;
                }
            }
        } else if is_compaction {
            prompts[index].compaction = parts.update.cloned();
        } else {
            prompts[index].unknown.push(envelope.clone());
        }
    }
    prompts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn aggregate(envelopes: Vec<Value>) -> Vec<Prompt> {
        aggregate_updates(&envelopes)
    }

    #[test]
    fn chunks_of_one_prompt_merge_into_a_single_text_block() {
        let prompts = aggregate(vec![
            json!({"method": "session/update", "params": {
                "update": {"kind": "user_message", "content": {"type": "text", "text": "hi"}},
                "_meta": {"promptId": "p1", "promptIndex": 0, "updateType": "UserMessage"}}}),
            json!({"method": "session/update", "params": {
                "update": {"content": {"type": "text", "text": "he"}},
                "_meta": {"promptId": "p1", "updateType": "AgentMessageChunk"}}}),
            json!({"method": "session/update", "params": {
                "update": {"content": {"type": "text", "text": "llo"}},
                "_meta": {"promptId": "p1", "updateType": "AgentMessageChunk"}}}),
        ]);
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].user, ["hi"]);
        assert_eq!(prompts[0].blocks, [PromptBlock::Text("hello".into())]);
    }

    #[test]
    fn a_missing_prompt_id_is_recovered_from_a_unique_prompt_index() {
        let prompts = aggregate(vec![
            json!({"method": "session/update", "params": {
                "update": {"content": {"type": "text", "text": "a"}},
                "_meta": {"promptId": "p9", "promptIndex": 3,
                          "updateType": "AgentMessageChunk"}}}),
            json!({"method": "session/update", "params": {
                "update": {"content": {"type": "text", "text": "b"}},
                "_meta": {"promptIndex": 3, "updateType": "AgentMessageChunk"}}}),
        ]);
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].id, "p9");
        assert_eq!(prompts[0].blocks, [PromptBlock::Text("ab".into())]);
    }

    #[test]
    fn an_ambiguous_prompt_index_falls_back_to_a_synthetic_key() {
        let prompts = aggregate(vec![
            json!({"method": "session/update", "params": {
                "update": {"content": {"type": "text", "text": "a"}},
                "_meta": {"promptId": "p1", "promptIndex": 0,
                          "updateType": "AgentMessageChunk"}}}),
            json!({"method": "session/update", "params": {
                "update": {"content": {"type": "text", "text": "b"}},
                "_meta": {"promptId": "p2", "promptIndex": 0,
                          "updateType": "AgentMessageChunk"}}}),
            json!({"method": "session/update", "params": {
                "update": {"content": {"type": "text", "text": "c"}},
                "_meta": {"promptIndex": 0, "updateType": "AgentMessageChunk"}}}),
        ]);
        let ids: Vec<&str> = prompts.iter().map(|prompt| prompt.id.as_str()).collect();
        assert_eq!(ids, ["p1", "p2", "prompt:0"]);
    }

    #[test]
    fn tool_calls_accumulate_input_output_and_protect_terminal_status() {
        let prompts = aggregate(vec![
            json!({"method": "session/update", "params": {
                "update": {"kind": "read", "rawInput": {"path": "/a"}},
                "_meta": {"promptId": "p1", "updateType": "ToolCall",
                          "updateParams": {"kind": "read", "status": "Pending",
                                           "toolCallId": "t1"}}}}),
            json!({"method": "session/update", "params": {
                "update": {"kind": "read", "rawOutput": {"content": "x"}},
                "_meta": {"promptId": "p1", "updateType": "ToolCallUpdate",
                          "updateParams": {"kind": "read", "status": "Completed",
                                           "toolCallId": "t1"}}}}),
            // 终态之后的非终态更新不得回退状态。
            json!({"method": "session/update", "params": {
                "update": {"kind": "read"},
                "_meta": {"promptId": "p1", "updateType": "ToolCallUpdate",
                          "updateParams": {"status": "Running", "toolCallId": "t1"}}}}),
        ]);
        let tool = prompts[0].tool("t1").unwrap();
        assert_eq!(tool.name, "read");
        assert_eq!(tool.input, json!({"path": "/a"}));
        assert_eq!(tool.output, Some(json!({"content": "x"})));
        assert_eq!(tool.status, "completed");
        assert_eq!(prompts[0].blocks, [PromptBlock::Tool("t1".into())]);
    }

    #[test]
    fn the_tool_name_falls_back_through_four_sources() {
        let name = |envelope: Value| {
            let prompts = aggregate(vec![envelope]);
            prompts[0].tool("t1").unwrap().name.clone()
        };
        let build = |update: Value, meta: Value| {
            json!({"method": "session/update",
                   "params": {"update": update, "_meta": meta}})
        };
        assert_eq!(
            name(build(
                json!({"sessionUpdate": "tool_call", "toolCallId": "t1",
                       "_meta": {"x.ai/tool": {"name": "grep"}}, "title": "T", "kind": "k"}),
                json!({"promptId": "p"})
            )),
            "grep"
        );
        assert_eq!(
            name(build(
                json!({"sessionUpdate": "tool_call", "toolCallId": "t1",
                       "title": "T", "kind": "k"}),
                json!({"promptId": "p"})
            )),
            "T"
        );
        assert_eq!(
            name(build(
                json!({"sessionUpdate": "tool_call", "toolCallId": "t1", "kind": "k"}),
                json!({"promptId": "p", "updateParams": {"kind": "uk"}})
            )),
            "uk"
        );
        assert_eq!(
            name(build(
                json!({"sessionUpdate": "tool_call", "toolCallId": "t1", "kind": "k"}),
                json!({"promptId": "p"})
            )),
            "k"
        );
        assert_eq!(
            name(build(
                json!({"sessionUpdate": "tool_call", "toolCallId": "t1"}),
                json!({"promptId": "p"})
            )),
            "tool"
        );
    }

    #[test]
    fn a_tool_call_owned_by_two_prompts_goes_to_the_unassigned_bucket() {
        let prompts = aggregate(vec![
            json!({"method": "session/update", "params": {
                "update": {"sessionUpdate": "tool_call", "toolCallId": "t1"},
                "_meta": {"promptId": "p1"}}}),
            json!({"method": "session/update", "params": {
                "update": {"sessionUpdate": "tool_call_update", "toolCallId": "t1"},
                "_meta": {"promptId": "p2"}}}),
        ]);
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].id, "prompt:unassigned");
        assert_eq!(prompts[0].unknown.len(), 2);
    }

    #[test]
    fn compaction_and_unknown_updates_are_kept_apart() {
        let prompts = aggregate(vec![
            json!({"method": "session/update", "params": {
                "update": {"kind": "compaction", "summary": "s", "tokensBefore": 10},
                "_meta": {"promptId": "p1"}}}),
            json!({"method": "session/update", "params": {
                "update": {"kind": "mystery"}, "_meta": {"promptId": "p1"}}}),
        ]);
        assert_eq!(
            prompts[0].compaction.as_ref().unwrap()["summary"],
            json!("s")
        );
        assert_eq!(prompts[0].unknown.len(), 1);
    }

    #[test]
    fn thought_chunks_accumulate_separately_from_text() {
        let prompts = aggregate(vec![
            json!({"method": "session/update", "params": {
                "update": {"sessionUpdate": "agent_thought_chunk",
                           "content": {"type": "text", "text": "th"}},
                "_meta": {"promptId": "p1"}}}),
            json!({"method": "session/update", "params": {
                "update": {"sessionUpdate": "agent_thought_chunk",
                           "content": {"type": "text", "text": "ink"}},
                "_meta": {"promptId": "p1"}}}),
            json!({"method": "session/update", "params": {
                "update": {"sessionUpdate": "agent_message_chunk",
                           "content": [{"type": "text", "text": "a"},
                                       {"type": "text", "text": "b"}]},
                "_meta": {"promptId": "p1"}}}),
        ]);
        assert_eq!(
            prompts[0].blocks,
            [
                PromptBlock::Thinking("think".into()),
                PromptBlock::Text("ab".into())
            ]
        );
    }

    #[test]
    fn envelopes_without_any_identity_get_positional_keys() {
        let prompts = aggregate(vec![
            json!({"method": "session/update", "params": {
                "update": {"sessionUpdate": "agent_message_chunk",
                           "content": {"type": "text", "text": "a"}}}}),
            json!({"method": "session/update", "params": {
                "update": {"sessionUpdate": "user_message_chunk",
                           "content": {"type": "text", "text": "b"}}}}),
        ]);
        // 第一条建 prompt:0；第二条在 order 已有 1 项时建 prompt:1。
        let ids: Vec<&str> = prompts.iter().map(|prompt| prompt.id.as_str()).collect();
        assert_eq!(ids, ["prompt:0", "prompt:1"]);
    }
}
