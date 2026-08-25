use serde_json::{json, Map, Value};

#[derive(Debug, PartialEq)]
pub(super) struct ToolRequestRoute {
    pub(super) method: &'static str,
    pub(super) params: Value,
    pub(super) requires_approval: bool,
}

fn has_exact_keys(args: &Map<String, Value>, required: &[&str], optional: &[&str]) -> bool {
    required.iter().all(|key| args.contains_key(*key))
        && args
            .keys()
            .all(|key| required.contains(&key.as_str()) || optional.contains(&key.as_str()))
}

fn execution_intent(args: &Map<String, Value>) -> Option<bool> {
    match args.get("intent").and_then(Value::as_str) {
        Some("preview") => Some(false),
        Some("execute") => Some(true),
        _ => None,
    }
}

pub(super) fn resolve_tool_request(
    name: &str,
    args: &Map<String, Value>,
) -> Option<ToolRequestRoute> {
    let read = |method| ToolRequestRoute {
        method,
        params: Value::Object(args.clone()),
        requires_approval: false,
    };
    Some(match name {
        "session_search" => read("content_search"),
        "session_read" => read("session_read"),
        "usage" => read("usage_stats"),
        "agent_prompt" => {
            if !has_exact_keys(args, &["tool", "ref", "prompt"], &["model", "timeout_sec"]) {
                return None;
            }
            ToolRequestRoute {
                method: "agent_prompt",
                params: Value::Object(args.clone()),
                requires_approval: false,
            }
        }
        "migrate" => {
            if !has_exact_keys(
                args,
                &["source_tool", "ref", "target_tool", "intent"],
                &["max_turn"],
            ) {
                return None;
            }
            let execute = execution_intent(args)?;
            let mut input = Map::new();
            input.insert("kind".to_owned(), Value::String("migration".to_owned()));
            for key in ["source_tool", "ref", "target_tool"] {
                input.insert(key.to_owned(), args.get(key)?.clone());
            }
            if let Some(max_turn) = args.get("max_turn") {
                input.insert("max_turn".to_owned(), max_turn.clone());
            }
            ToolRequestRoute {
                method: "operation.plan",
                params: json!({"input": Value::Object(input)}),
                requires_approval: execute,
            }
        }
        "session_edit" => match (args.contains_key("ops"), args.contains_key("patch")) {
            (true, false) => {
                if !has_exact_keys(args, &["tool", "ref", "ops", "intent"], &[]) {
                    return None;
                }
                let execute = execution_intent(args)?;
                ToolRequestRoute {
                    method: "operation.plan",
                    params: json!({"input": {
                        "kind": "edit",
                        "tool": args.get("tool")?,
                        "ref": args.get("ref")?,
                        "ops": args.get("ops")?,
                    }}),
                    requires_approval: execute,
                }
            }
            (false, true) => {
                if !has_exact_keys(args, &["tool", "ref", "patch"], &[]) {
                    return None;
                }
                ToolRequestRoute {
                    method: "operation.plan",
                    params: json!({"input": {
                        "kind": "metadata",
                        "tool": args.get("tool")?,
                        "ref": args.get("ref")?,
                        "patch": args.get("patch")?,
                    }}),
                    requires_approval: true,
                }
            }
            _ => return None,
        },
        _ => return None,
    })
}

pub(super) fn is_mutating_tool(name: &str, args: &Map<String, Value>) -> bool {
    resolve_tool_request(name, args)
        .map(|route| route.requires_approval)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn tool_gateway_is_an_exact_allowlist() {
        let read = resolve_tool_request("session_read", &map(json!({"ref": "fsr_a"}))).unwrap();
        assert_eq!(read.method, "session_read");
        assert!(!read.requires_approval);

        let migrate_preview = resolve_tool_request(
            "migrate",
            &map(json!({
                "source_tool": "claude",
                "ref": "fsr_a",
                "target_tool": "codex",
                "max_turn": 3,
                "intent": "preview",
            })),
        )
        .unwrap();
        assert_eq!(migrate_preview.method, "operation.plan");
        assert!(!migrate_preview.requires_approval);
        assert_eq!(
            migrate_preview.params,
            json!({"input": {
                "kind": "migration",
                "source_tool": "claude",
                "ref": "fsr_a",
                "target_tool": "codex",
                "max_turn": 3,
            }})
        );

        let migrate_execute = resolve_tool_request(
            "migrate",
            &map(json!({
                "source_tool": "claude",
                "ref": "fsr_a",
                "target_tool": "codex",
                "intent": "execute",
            })),
        )
        .unwrap();
        assert!(migrate_execute.requires_approval);
        assert!(migrate_execute.params.pointer("/input/intent").is_none());

        let edit_preview = resolve_tool_request(
            "session_edit",
            &map(json!({
                "tool": "claude",
                "ref": "fsr_a",
                "ops": [{"op": "delete-turn", "turn": 1}],
                "intent": "preview",
            })),
        )
        .unwrap();
        assert_eq!(edit_preview.method, "operation.plan");
        assert!(!edit_preview.requires_approval);
        assert_eq!(
            edit_preview.params,
            json!({"input": {
                "kind": "edit",
                "tool": "claude",
                "ref": "fsr_a",
                "ops": [{"op": "delete-turn", "turn": 1}],
            }})
        );

        let metadata = resolve_tool_request(
            "session_edit",
            &map(json!({
                "tool": "claude",
                "ref": "fsr_a",
                "patch": {"pinned": true},
            })),
        )
        .unwrap();
        assert_eq!(metadata.method, "operation.plan");
        assert!(metadata.requires_approval);

        let prompt = resolve_tool_request(
            "agent_prompt",
            &map(json!({
                "tool": "grok",
                "ref": "fsr_a",
                "prompt": "继续实现",
                "model": "grok-code-fast-1",
                "timeout_sec": 360,
            })),
        )
        .unwrap();
        assert_eq!(prompt.method, "agent_prompt");
        assert!(!prompt.requires_approval);
        assert_eq!(
            prompt.params,
            json!({
                "tool": "grok",
                "ref": "fsr_a",
                "prompt": "继续实现",
                "model": "grok-code-fast-1",
                "timeout_sec": 360,
            })
        );

        assert_eq!(
            resolve_tool_request(
                "session_edit",
                &map(json!({
                    "tool": "claude",
                    "ref": "fsr_a",
                    "ops": [],
                    "patch": {"pinned": true},
                    "intent": "execute",
                })),
            ),
            None
        );
        assert_eq!(
            resolve_tool_request(
                "agent_prompt",
                &map(json!({
                    "tool": "codex",
                    "ref": "fsr_a",
                    "prompt": "继续",
                    "session_id": "bypass",
                })),
            ),
            None
        );
        assert_eq!(
            resolve_tool_request(
                "agent_prompt",
                &map(json!({"tool": "codex", "ref": "fsr_a"})),
            ),
            None
        );
        assert_eq!(
            resolve_tool_request(
                "migrate",
                &map(json!({
                    "source_tool": "claude",
                    "ref": "fsr_a",
                    "target_tool": "codex",
                    "intent": "execute",
                    "method": "operation.apply",
                })),
            ),
            None
        );
        assert_eq!(
            resolve_tool_request(
                "session_cleanup",
                &map(json!({
                    "intent": "inventory",
                    "scope": {"agents": ["claude"]},
                })),
            ),
            None
        );
        assert_eq!(
            resolve_tool_request("operation.apply", &map(json!({}))),
            None
        );
        assert_eq!(resolve_tool_request("shell", &map(json!({}))), None);
    }

    #[test]
    fn preview_never_requires_approval_and_execute_always_does() {
        for name in ["migrate", "session_edit"] {
            let base = if name == "migrate" {
                json!({"source_tool": "claude", "ref": "fsr_a",
                       "target_tool": "codex"})
            } else {
                json!({"tool": "claude", "ref": "fsr_a",
                       "ops": [{"op": "delete-turn", "turn": 1}]})
            };
            let mut preview = base.as_object().unwrap().clone();
            preview.insert("intent".to_owned(), Value::String("preview".to_owned()));
            assert!(!is_mutating_tool(name, &preview));
            let mut execute = base.as_object().unwrap().clone();
            execute.insert("intent".to_owned(), Value::String("execute".to_owned()));
            assert!(is_mutating_tool(name, &execute));
        }
    }
}
