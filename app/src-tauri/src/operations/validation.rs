//! Operation 输入的共享安全校验原语。

use serde_json::Value;

pub(crate) fn is_known_agent(agent: &str) -> bool {
    crate::contracts::agents::AGENT_IDS.contains(&agent)
}

pub(crate) fn validate_opaque_ref(reference: &str, label: &str) -> Result<(), String> {
    if !crate::contracts::session_ref::is_opaque_session_ref(reference) {
        return Err(format!("{label} ref 不是有效 opaque ref"));
    }
    Ok(())
}

pub(crate) fn validate_bounded_json(
    value: &Value,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), String> {
    *nodes += 1;
    if depth > 8 || *nodes > 2_000 {
        return Err("Operation JSON 结构过深或项目过多".to_owned());
    }
    match value {
        Value::Object(fields) => {
            if fields.keys().any(|key| key.len() > 128) {
                return Err("Operation JSON key 过长".to_owned());
            }
            for child in fields.values() {
                validate_bounded_json(child, depth + 1, nodes)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                validate_bounded_json(child, depth + 1, nodes)?;
            }
        }
        Value::String(value) if value.chars().count() > 20_000 => {
            return Err("Operation JSON 字符串过长".to_owned());
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    Ok(())
}

pub(crate) fn validate_reply(reply: &Value) -> Result<(), String> {
    let fields = reply
        .as_object()
        .filter(|fields| fields.len() == 1 && fields.contains_key("items"))
        .ok_or_else(|| "Operation reply 必须且只能包含 items".to_owned())?;
    let items = fields["items"]
        .as_array()
        .filter(|items| !items.is_empty() && items.len() <= 100)
        .ok_or_else(|| "Operation reply.items 必须包含 1 到 100 项".to_owned())?;
    for item in items {
        let item_fields = item
            .as_object()
            .ok_or_else(|| "Operation reply item 必须是 object".to_owned())?;
        match item_fields.get("kind").and_then(Value::as_str) {
            Some("text") => {
                if item_fields.len() != 2 || !item_fields.contains_key("text") {
                    return Err("Operation reply text item 参数无效".to_owned());
                }
                let text = item_fields["text"]
                    .as_str()
                    .ok_or_else(|| "Operation reply text 必须是字符串".to_owned())?;
                if text.is_empty() || text.chars().count() > 20_000 {
                    return Err("Operation reply text 长度无效".to_owned());
                }
            }
            Some("tool") => {
                if item_fields.len() != 4
                    || !item_fields.contains_key("name")
                    || !item_fields.contains_key("input")
                    || !item_fields.contains_key("output")
                {
                    return Err("Operation reply tool item 参数无效".to_owned());
                }
                let name = item_fields["name"]
                    .as_str()
                    .ok_or_else(|| "Operation reply tool name 必须是字符串".to_owned())?;
                let output = item_fields["output"]
                    .as_str()
                    .ok_or_else(|| "Operation reply tool output 必须是字符串".to_owned())?;
                if name.is_empty()
                    || name.chars().count() > 256
                    || name.chars().any(char::is_control)
                    || output.chars().count() > 20_000
                {
                    return Err("Operation reply tool name/output 长度无效".to_owned());
                }
                let tool_input = &item_fields["input"];
                if !tool_input.is_object() && !tool_input.is_string() {
                    return Err("Operation reply tool input 必须是 object 或字符串".to_owned());
                }
                let mut nodes = 0;
                validate_bounded_json(tool_input, 0, &mut nodes)?;
            }
            _ => return Err("Operation reply item kind 不受支持".to_owned()),
        }
    }
    Ok(())
}
