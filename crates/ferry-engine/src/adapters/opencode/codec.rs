//! OpenCode 导出负载的唯一轮次解析与编辑编解码。
//!
//! 语义事实源：`engine/adapters/opencode/codec.py`。
//!
//! 轮次定义：含可见内容（text / tool / 可见 reasoning）的用户消息到下一条之前。

use serde_json::{Map, Value};

use crate::adapters::shared::codec::{NativeEditCodec, TurnIndex, TurnSpan};
use crate::errors::{DomainError, DomainResult};
use crate::events::{event, Event};

use super::editor::OpenCodeData;

/// 与 `sessions::reasoning::visible_text` 同语义（`adapters` 不得依赖 `sessions`）。
fn visible_reasoning(text: Option<&Value>) -> bool {
    text.and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

/// 这条原生消息是否参与轮次划分。
fn visible(message: &Value) -> bool {
    message
        .get("parts")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .any(|part| match part.get("type").and_then(Value::as_str) {
            Some("text") | Some("tool") => true,
            Some("reasoning") => visible_reasoning(part.get("text")),
            _ => false,
        })
}

fn info_of(message: &Value) -> Map<String, Value> {
    message
        .get("info")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(Map::new)
}

fn messages_of(payload: &Value) -> &[Value] {
    payload
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// OpenCode 原生 payload 的轮次索引。
pub struct OpenCodeTurnIndex;

impl TurnIndex for OpenCodeTurnIndex {
    /// 官方 export payload。
    type Document = Value;
    /// `(数组下标, 消息)`。
    type VisibleMessage = usize;

    fn visible_messages(&self, document: &Self::Document) -> Vec<usize> {
        messages_of(document)
            .iter()
            .enumerate()
            .filter(|(_, message)| visible(message))
            .map(|(index, _)| index)
            .collect()
    }

    fn turns(&self, document: &Self::Document) -> Vec<TurnSpan> {
        let messages = messages_of(document);
        let users: Vec<usize> = self
            .visible_messages(document)
            .into_iter()
            .filter(|index| info_of(&messages[*index]).get("role") == Some(&Value::from("user")))
            .collect();
        users
            .iter()
            .enumerate()
            .map(|(ordinal, start)| {
                let end = users.get(ordinal + 1).copied().unwrap_or(messages.len());
                let locator = info_of(&messages[*start])
                    .get("id")
                    .filter(|value| !value.is_null())
                    .map(crate::adapters::shared::dialect::python_str)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| format!("message:{start}"));
                TurnSpan::new(ordinal + 1, locator, *start, end)
            })
            .collect()
    }
}

/// OpenCode 的写侧编解码。
pub struct OpenCodeEditCodec;

impl NativeEditCodec for OpenCodeEditCodec {
    type Document = OpenCodeData;
    type Reply = Value;
    type Change = Event;

    /// OpenCode 官方 API 只能整块替换 part，无法安全重排回复。
    fn replace_reply(
        &self,
        _document: &mut Self::Document,
        _span: &TurnSpan,
        _reply: &Self::Reply,
    ) -> DomainResult<Vec<Event>> {
        Err(DomainError::operation_unsupported(
            "opencode",
            "replace-assistant-reply",
            Some("inplace"),
        ))
    }

    /// 删除整轮：连同该轮派生的子会话一起从会话树上摘掉。
    ///
    /// 注意：`OpenCodeBackend::apply_ops` 会**先**拒绝 `delete-turn`（官方 API
    /// 不支持原地删消息），这里保留实现是为了与 Python 的 codec 保持同构。
    fn delete_turn(
        &self,
        document: &mut Self::Document,
        span: &TurnSpan,
    ) -> DomainResult<Vec<Event>> {
        let messages = messages_of(&document.data).to_vec();
        let user_id = messages
            .get(span.start)
            .map(info_of)
            .and_then(|info| info.get("id").cloned())
            .unwrap_or(Value::Null);
        let mut remove_ids: Vec<Value> = vec![user_id.clone()];
        for message in &messages {
            let info = info_of(message);
            if visible(message) && info.get("parentID") == Some(&user_id) {
                remove_ids.push(info.get("id").cloned().unwrap_or(Value::Null));
            }
        }
        let mut removed_children: Vec<String> = Vec::new();
        for message in &messages {
            if !remove_ids.contains(&info_of(message).get("id").cloned().unwrap_or(Value::Null)) {
                continue;
            }
            for part in message
                .get("parts")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                if part.get("tool") != Some(&Value::from("task")) {
                    continue;
                }
                let state = part
                    .get("state")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_else(Map::new);
                let metadata = state
                    .get("metadata")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_else(Map::new);
                if let Some(child) = metadata.get("sessionId").and_then(Value::as_str) {
                    removed_children.push(child.to_string());
                }
            }
        }

        let kept: Vec<Value> = messages
            .into_iter()
            .filter(|message| {
                !remove_ids.contains(&info_of(message).get("id").cloned().unwrap_or(Value::Null))
            })
            .collect();
        if let Some(entries) = document.data.as_object_mut() {
            entries.insert("messages".into(), Value::Array(kept));
        }
        if !removed_children.is_empty() {
            document
                .tree
                .children
                .retain(|child| !removed_children.contains(&child.source_id));
            document
                .tree
                .agent_edges
                .retain(|edge| !removed_children.contains(&edge.child_session_id));
        }

        let mut params = Map::new();
        params.insert("turn".into(), Value::from(span.ordinal));
        Ok(vec![event("edit.turn_deleted", params)])
    }

    /// 改写一条可见消息的正文：首个 text part 承载新正文，其余清空。
    fn rewrite_message(
        &self,
        document: &mut Self::Document,
        locator: &str,
        text: &str,
    ) -> DomainResult<Vec<Event>> {
        let visible_indexes = OpenCodeTurnIndex.visible_messages(&document.data);
        let messages = messages_of(&document.data);
        let mut target = visible_indexes
            .iter()
            .copied()
            .find(|index| info_of(&messages[*index]).get("id") == Some(&Value::from(locator)));
        if target.is_none() {
            if let Some(rest) = locator.strip_prefix("index:") {
                if let Ok(wanted) = rest.parse::<i64>() {
                    // Python 的负下标语义：`visible[-1]` 是最后一条。
                    let resolved = if wanted < 0 {
                        visible_indexes.len() as i64 + wanted
                    } else {
                        wanted
                    };
                    if resolved >= 0 {
                        target = visible_indexes.get(resolved as usize).copied();
                    }
                }
            }
        }
        let Some(index) = target else {
            let mut params = Map::new();
            params.insert("locator".into(), Value::from(locator));
            return Err(DomainError::locator_stale(
                Some("OpenCode 消息定位符已失效，请刷新会话"),
                params,
            ));
        };

        let role = info_of(&messages[index]).get("role").map_or(
            "None".to_string(),
            crate::adapters::shared::dialect::python_str,
        );
        if role != "user" && role != "assistant" {
            return Err(DomainError::operation_unsupported(
                "opencode",
                "rewrite",
                Some(&role),
            ));
        }

        let message = document
            .data
            .get_mut("messages")
            .and_then(Value::as_array_mut)
            .and_then(|messages| messages.get_mut(index))
            .ok_or_else(|| DomainError::internal("OpenCode 消息数组结构异常"))?;
        let parts = message
            .get_mut("parts")
            .and_then(Value::as_array_mut)
            .map(Vec::as_mut_slice)
            .unwrap_or_default();
        let text_indexes: Vec<usize> = parts
            .iter()
            .enumerate()
            .filter(|(_, part)| part.get("type") == Some(&Value::from("text")))
            .map(|(index, _)| index)
            .collect();
        if text_indexes.is_empty() {
            return Err(DomainError::operation_unsupported(
                "opencode",
                "rewrite",
                Some("no-text"),
            ));
        }
        for (position, part_index) in text_indexes.into_iter().enumerate() {
            let value = if position == 0 { text } else { "" };
            if let Some(entries) = parts[part_index].as_object_mut() {
                entries.insert("text".into(), Value::from(value));
            }
        }

        let mut params = Map::new();
        params.insert("count".into(), Value::from(1));
        Ok(vec![event("edit.message_rewritten", params)])
    }
}

/// 进程内共享的单例（对齐 Python 的模块级 `TURN_INDEX` / `CODEC`）。
pub const TURN_INDEX: OpenCodeTurnIndex = OpenCodeTurnIndex;
/// 见 [`TURN_INDEX`]。
pub const CODEC: OpenCodeEditCodec = OpenCodeEditCodec;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Session;
    use serde_json::json;

    fn payload() -> Value {
        json!({
            "info": {"id": "s", "directory": "/tmp"},
            "messages": [
                {"info": {"id": "u1", "role": "user"},
                 "parts": [{"id": "p1", "type": "text", "text": "first"}]},
                {"info": {"id": "a1", "role": "assistant", "parentID": "u1"},
                 "parts": [{"id": "p2", "type": "text", "text": "answer"}]},
                {"info": {"id": "sys", "role": "assistant"}, "parts": []},
                {"info": {"id": "u2", "role": "user"},
                 "parts": [{"id": "p3", "type": "reasoning", "text": "   "},
                           {"id": "p4", "type": "text", "text": "second"}]}
            ]
        })
    }

    fn document(payload: Value) -> OpenCodeData {
        OpenCodeData {
            data: payload.clone(),
            original: payload,
            tree: Session::new("opencode", "s", "/tmp"),
        }
    }

    #[test]
    fn turns_start_at_visible_user_messages() {
        let payload = payload();
        let spans = TURN_INDEX.turns(&payload);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0], TurnSpan::new(1, "u1", 0, 3));
        assert_eq!(spans[1], TurnSpan::new(2, "u2", 3, 4));
        // 无可见 part 的消息不参与划分。
        assert_eq!(TURN_INDEX.visible_messages(&payload), [0, 1, 3]);
    }

    #[test]
    fn a_user_message_without_an_id_falls_back_to_its_index() {
        let payload = json!({"messages": [{"info": {"role": "user"},
                                           "parts": [{"type": "text", "text": "x"}]}]});
        assert_eq!(TURN_INDEX.turns(&payload)[0].locator, "message:0");
    }

    #[test]
    fn rewrite_puts_the_new_text_in_the_first_part_and_blanks_the_rest() {
        let mut document = document(json!({
            "messages": [{"info": {"id": "u1", "role": "user"},
                          "parts": [{"type": "text", "text": "a"},
                                    {"type": "tool", "tool": "bash"},
                                    {"type": "text", "text": "b"}]}]
        }));
        let changes = CODEC
            .rewrite_message(&mut document, "u1", "新正文")
            .unwrap();
        assert_eq!(changes[0].code, "edit.message_rewritten");
        assert_eq!(changes[0].params["count"], json!(1));
        let parts = document.data["messages"][0]["parts"].as_array().unwrap();
        assert_eq!(parts[0]["text"], json!("新正文"));
        assert_eq!(parts[2]["text"], json!(""));
        // 工具 part 原样保留。
        assert_eq!(parts[1]["tool"], json!("bash"));
    }

    #[test]
    fn rewrite_supports_index_locators_and_reports_stale_ones() {
        let mut document = document(payload());
        CODEC
            .rewrite_message(&mut document, "index:1", "改过")
            .unwrap();
        assert_eq!(
            document.data["messages"][1]["parts"][0]["text"],
            json!("改过")
        );

        let error = CODEC
            .rewrite_message(&mut document, "gone", "x")
            .unwrap_err();
        assert_eq!(error.code, "session.locator_stale");
        assert_eq!(error.message(), "OpenCode 消息定位符已失效，请刷新会话");
        assert_eq!(error.params()["locator"], json!("gone"));
    }

    #[test]
    fn rewrite_refuses_non_conversational_roles_and_text_free_messages() {
        let mut document = document(json!({
            "messages": [{"info": {"id": "t", "role": "tool"},
                          "parts": [{"type": "text", "text": "x"}]},
                         {"info": {"id": "u", "role": "user"},
                          "parts": [{"type": "tool", "tool": "bash"}]}]
        }));
        let error = CODEC.rewrite_message(&mut document, "t", "x").unwrap_err();
        assert_eq!(error.code, "edit.operation_unsupported");
        assert_eq!(error.params()["mode"], json!("tool"));

        let error = CODEC.rewrite_message(&mut document, "u", "x").unwrap_err();
        assert_eq!(error.params()["mode"], json!("no-text"));
    }

    #[test]
    fn delete_turn_removes_the_user_its_replies_and_the_spawned_children() {
        let mut document = document(json!({
            "messages": [
                {"info": {"id": "u1", "role": "user"},
                 "parts": [{"type": "text", "text": "go"}]},
                {"info": {"id": "a1", "role": "assistant", "parentID": "u1"},
                 "parts": [{"type": "tool", "tool": "task",
                            "state": {"metadata": {"sessionId": "child"}}}]},
                {"info": {"id": "u2", "role": "user"},
                 "parts": [{"type": "text", "text": "next"}]}
            ]
        }));
        document.tree.children = vec![Session::new("opencode", "child", "/tmp")];
        document.tree.agent_edges = vec![crate::model::AgentEdge::new("s", "child")];

        let span = TURN_INDEX.turns(&document.data)[0].clone();
        let changes = CODEC.delete_turn(&mut document, &span).unwrap();
        assert_eq!(changes[0].code, "edit.turn_deleted");
        assert_eq!(changes[0].params["turn"], json!(1));
        let ids: Vec<&str> = document.data["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["info"]["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["u2"]);
        assert!(document.tree.children.is_empty());
        assert!(document.tree.agent_edges.is_empty());
    }

    #[test]
    fn replace_reply_is_unsupported() {
        let mut document = document(payload());
        let span = TurnSpan::new(1, "u1", 0, 1);
        let error = CODEC
            .replace_reply(&mut document, &span, &Value::Null)
            .unwrap_err();
        assert_eq!(error.code, "edit.operation_unsupported");
        assert_eq!(error.params()["mode"], json!("inplace"));
    }
}
