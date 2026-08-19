//! 按文件顺序应用 Grok 0.2.106 的 rewind 标记。
//!
//! 语义事实源：`engine/adapters/grok/rewind.py`。
//!
//! rewind 标记是「把历史截回第 N 个用户 run 的起点」，它出现在流里的位置决定
//! 截断范围，因此必须顺序重放而不是最后统一处理。

use serde_json::Value;

/// 一个 envelope 的 `params.update`；缺席返回 `None`（等价 Python 的 `or {}`）。
fn update(envelope: &Value) -> Option<&Value> {
    envelope.get("params")?.get("update")
}

fn field<'a>(value: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    value?.get(key)
}

/// 用户 run 的起点位置。
///
/// 区分两种流：带 `promptIndex` 的（indexed，同一 index 的连续 chunk 只算一个
/// 起点）与不带的（unindexed，连续的用户 chunk 只算一个起点，且一旦见过
/// indexed 就不再承认 unindexed）。`_meta.hostTurn == true` 是 Grok 自己注入的
/// 宿主轮次，不是用户 prompt。
fn user_run_starts(envelopes: &[Value]) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut saw_index = false;
    let mut active_key: Option<i64> = None;
    let mut in_unindexed_run = false;
    for (position, envelope) in envelopes.iter().enumerate() {
        let update = update(envelope);
        let nested = field(update, "_meta");
        let is_user = envelope.get("method").and_then(Value::as_str) == Some("session/update")
            && field(update, "sessionUpdate").and_then(Value::as_str) == Some("user_message_chunk")
            && field(nested, "hostTurn") != Some(&Value::Bool(true));
        if !is_user {
            in_unindexed_run = false;
            continue;
        }
        // Python 的 `isinstance(prompt_index, int)` 排除 bool，也排除 float。
        let prompt_index = field(nested, "promptIndex")
            .filter(|value| !value.is_boolean())
            .and_then(Value::as_i64);
        match prompt_index {
            Some(index) => {
                saw_index = true;
                if active_key != Some(index) {
                    starts.push(position);
                    active_key = Some(index);
                }
                in_unindexed_run = false;
            }
            None if !saw_index && !in_unindexed_run => {
                starts.push(position);
                active_key = None;
                in_unindexed_run = true;
            }
            None => {}
        }
    }
    starts
}

/// 顺序重放 rewind 标记，返回仍然可见的 envelope 流。
pub fn filter_rewind_updates(envelopes: &[Value]) -> Vec<Value> {
    let mut visible: Vec<Value> = Vec::new();
    for envelope in envelopes {
        let is_marker = envelope.get("method").and_then(Value::as_str)
            == Some("_x.ai/session/update")
            && field(update(envelope), "sessionUpdate").and_then(Value::as_str)
                == Some("rewind_marker");
        if !is_marker {
            visible.push(envelope.clone());
            continue;
        }
        let target = field(update(envelope), "target_prompt_index")
            .filter(|value| !value.is_boolean())
            .and_then(Value::as_i64);
        let starts = user_run_starts(&visible);
        match target {
            // 截回第 target 个用户 run 的起点。
            Some(target) if target >= 0 && (target as usize) < starts.len() => {
                visible.truncate(starts[target as usize]);
            }
            // 正好指向「下一个还没出现的 run」：等价于什么都不截，标记本身丢弃。
            Some(target) if target == starts.len() as i64 => {}
            // 无法解释的标记原样保留，交给 aggregate 记成 unknown。
            _ => visible.push(envelope.clone()),
        }
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(index: Option<i64>, text: &str) -> Value {
        let mut meta = json!({});
        if let Some(index) = index {
            meta["promptIndex"] = json!(index);
        }
        json!({"method": "session/update", "params": {"update": {
            "sessionUpdate": "user_message_chunk",
            "content": {"type": "text", "text": text},
            "_meta": meta,
        }}})
    }

    fn assistant(text: &str) -> Value {
        json!({"method": "session/update", "params": {
            "update": {"sessionUpdate": "agent_message_chunk",
                       "content": {"type": "text", "text": text}}}})
    }

    fn marker(target: Value) -> Value {
        json!({"method": "_x.ai/session/update", "params": {"update": {
            "sessionUpdate": "rewind_marker", "target_prompt_index": target,
        }}})
    }

    fn texts(envelopes: &[Value]) -> Vec<String> {
        envelopes
            .iter()
            .map(|envelope| {
                envelope["params"]["update"]["content"]["text"]
                    .as_str()
                    .unwrap_or("<marker>")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn indexed_runs_are_truncated_at_the_target_prompt() {
        let stream = vec![
            user(Some(0), "first"),
            assistant("a0"),
            user(Some(1), "dead"),
            assistant("dead-a"),
            marker(json!(1)),
            assistant("live"),
        ];
        assert_eq!(
            texts(&filter_rewind_updates(&stream)),
            ["first", "a0", "live"]
        );
    }

    #[test]
    fn repeated_chunks_of_one_prompt_count_as_a_single_run() {
        let stream = vec![
            user(Some(0), "a"),
            user(Some(0), "b"),
            user(Some(1), "c"),
            marker(json!(1)),
        ];
        assert_eq!(texts(&filter_rewind_updates(&stream)), ["a", "b"]);
    }

    #[test]
    fn unindexed_runs_collapse_and_lose_to_indexed_ones() {
        // 连续的 unindexed 用户 chunk 只算一个起点。
        let stream = vec![
            user(None, "a"),
            user(None, "b"),
            assistant("x"),
            user(None, "c"),
            marker(json!(1)),
        ];
        assert_eq!(texts(&filter_rewind_updates(&stream)), ["a", "b", "x"]);

        // 一旦出现 indexed，后续 unindexed 不再产生新起点。
        let stream = vec![user(Some(0), "a"), user(None, "b"), marker(json!(1))];
        assert_eq!(texts(&filter_rewind_updates(&stream)), ["a", "b"]);
    }

    #[test]
    fn host_turns_are_not_user_runs() {
        let mut host = user(Some(1), "host");
        host["params"]["update"]["_meta"]["hostTurn"] = json!(true);
        let stream = vec![user(Some(0), "a"), host, marker(json!(1))];
        // 只有一个用户 run，target=1 == len(starts) → 什么都不截、标记丢弃。
        assert_eq!(texts(&filter_rewind_updates(&stream)), ["a", "host"]);
    }

    #[test]
    fn uninterpretable_markers_survive_into_the_stream() {
        let stream = vec![user(Some(0), "a"), marker(json!("bad"))];
        let visible = filter_rewind_updates(&stream);
        assert_eq!(visible.len(), 2);
        assert_eq!(
            visible[1]["params"]["update"]["sessionUpdate"],
            json!("rewind_marker")
        );
        // 超界的 target 同样保留。
        let stream = vec![user(Some(0), "a"), marker(json!(7))];
        assert_eq!(filter_rewind_updates(&stream).len(), 2);
    }

    #[test]
    fn a_zero_target_clears_the_whole_stream() {
        let stream = vec![user(Some(0), "a"), assistant("b"), marker(json!(0))];
        assert!(filter_rewind_updates(&stream).is_empty());
    }
}
