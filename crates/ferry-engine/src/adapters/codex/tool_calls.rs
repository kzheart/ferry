//! Codex 当前工具调用联合类型的输入解析。
//!
//! 两种子类型的入参形态完全不同：
//! - `function_call.arguments` 是 JSON 字符串（`spawn_agent` 特判 → 方言归一 →
//!   兜底 `tool.invoke`）；
//! - `custom_tool_call.input` 是任意 JS 源码，必须先用手写词法扫描器把
//!   `tools.<name>(<argument>)` 调用挑出来。

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use crate::adapters::shared::dialect::python_str;
use crate::loss::Outcome;
use crate::model::{Session, ToolCall};
use crate::tool_ops::CanonicalOp;

use super::dialect::{decode_shell, DIALECT};

/// `apply_patch` 是 Codex 私有的工具形态：解析不出补丁时降级叙述，不丢内容。
///
/// Rust 没有 import 副作用，由 `adapter::build()` 显式调用 `loss::declare`。
pub const LOSS_OUTCOMES: &[(&str, Outcome)] =
    &[("migration.apply_patch_unparsed", Outcome::Degraded)];

static PATCH_HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\*\*\* (Add|Update|Delete) File: ([^\r\n]+)$").expect("补丁头正则是常量")
});

static PATCH_MOVE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\*\*\* Move to: ([^\r\n]+)$").expect("Move 正则是常量"));

static HUNK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^@@").expect("hunk 正则是常量"));

const BEGIN_PATCH: &str = "*** Begin Patch";
const END_PATCH: &str = "*** End Patch";

// ---------------------------------------------------------------------------
// JS 词法扫描器
// ---------------------------------------------------------------------------
//
// Python 侧按**字符**索引，这里统一先转成 `Vec<char>` 再扫描，避免 UTF-8 字节
// 偏移与 Python 语义漂移。

fn is_quote(character: char) -> bool {
    matches!(character, '"' | '\'' | '`')
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$'
}

fn starts_with(source: &[char], index: usize, needle: &str) -> bool {
    for (cursor, expected) in (index..).zip(needle.chars()) {
        if source.get(cursor) != Some(&expected) {
            return false;
        }
    }
    true
}

fn find_from(source: &[char], index: usize, needle: &str) -> Option<usize> {
    (index..source.len()).find(|position| starts_with(source, *position, needle))
}

/// 跳过一个 JS 字符串字面量，返回闭合引号之后的位置。
fn skip_js_string(source: &[char], start: usize) -> usize {
    let quote = source[start];
    let mut index = start + 1;
    while index < source.len() {
        if source[index] == '\\' {
            index += 2;
            continue;
        }
        if source[index] == quote {
            return index + 1;
        }
        index += 1;
    }
    source.len()
}

/// 跳过一段 JS 注释；不是注释起点时原样返回 `start`。
fn skip_js_comment(source: &[char], start: usize) -> usize {
    if starts_with(source, start, "//") {
        return match find_from(source, start + 2, "\n") {
            Some(newline) => newline + 1,
            None => source.len(),
        };
    }
    if starts_with(source, start, "/*") {
        return match find_from(source, start + 2, "*/") {
            Some(end) => end + 2,
            None => source.len(),
        };
    }
    start
}

/// 从左括号开始取配平的实参文本，返回 `(实参, 右括号之后的位置)`。
fn balanced_js_argument(source: &[char], open_paren: usize) -> Option<(String, usize)> {
    let mut depth = 1usize;
    let mut index = open_paren + 1;
    while index < source.len() {
        let character = source[index];
        if is_quote(character) {
            index = skip_js_string(source, index);
            continue;
        }
        if starts_with(source, index, "//") || starts_with(source, index, "/*") {
            index = skip_js_comment(source, index);
            continue;
        }
        if character == '(' {
            depth += 1;
        } else if character == ')' {
            depth -= 1;
            if depth == 0 {
                return Some((source[open_paren + 1..index].iter().collect(), index + 1));
            }
        }
        index += 1;
    }
    None
}

/// 扫描 `tools.<name>(<argument>)` 调用；字符串与注释内的同名文本不计入。
pub fn scan_tool_invocations(source: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = source.chars().collect();
    let mut calls = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        let character = chars[index];
        if is_quote(character) {
            index = skip_js_string(&chars, index);
            continue;
        }
        if starts_with(&chars, index, "//") || starts_with(&chars, index, "/*") {
            index = skip_js_comment(&chars, index);
            continue;
        }
        if starts_with(&chars, index, "tools.") && (index == 0 || !is_word(chars[index - 1])) {
            let name_start = index + "tools.".len();
            let mut name_end = name_start;
            while name_end < chars.len() && is_word(chars[name_end]) {
                name_end += 1;
            }
            let mut cursor = name_end;
            while cursor < chars.len() && chars[cursor].is_whitespace() {
                cursor += 1;
            }
            if name_end > name_start && chars.get(cursor) == Some(&'(') {
                if let Some((argument, _end)) = balanced_js_argument(&chars, cursor) {
                    calls.push((chars[name_start..name_end].iter().collect(), argument));
                    index = cursor + 1;
                    continue;
                }
            }
        }
        index += 1;
    }
    calls
}

/// 解码一个 JS 值字面量：先按 JSON 解析，再兜底单引号/反引号字面量。
pub fn decode_js_value(source: &str) -> Value {
    let source = source.trim();
    if let Ok(value) = serde_json::from_str::<Value>(source) {
        return value;
    }
    let characters: Vec<char> = source.chars().collect();
    if characters.len() >= 2 {
        let quote = characters[0];
        if (quote == '\'' || quote == '`') && characters[characters.len() - 1] == quote {
            let mut body: String = characters[1..characters.len() - 1].iter().collect();
            // 替换顺序照抄 Python 的 dict 迭代序，链式 replace 的结果依赖它。
            for (escaped, replacement) in [
                ("\\n", "\n".to_string()),
                ("\\r", "\r".to_string()),
                ("\\t", "\t".to_string()),
                ("\\\\", "\\".to_string()),
                (&format!("\\{quote}"), quote.to_string()),
            ] {
                body = body.replace(escaped, &replacement);
            }
            return Value::String(body);
        }
    }
    Value::String(source.to_string())
}

/// 逐个 yield 源码里的字符串字面量（已解码）。
fn js_string_values(source: &str) -> Vec<Value> {
    let chars: Vec<char> = source.chars().collect();
    let mut values = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        if is_quote(chars[index]) {
            let end = skip_js_string(&chars, index);
            let literal: String = chars[index..end].iter().collect();
            values.push(decode_js_value(&literal));
            index = end;
            continue;
        }
        if starts_with(&chars, index, "//") || starts_with(&chars, index, "/*") {
            index = skip_js_comment(&chars, index);
            continue;
        }
        index += 1;
    }
    values
}

/// 从任意候选里挑出补丁正文；候选顺序即 Python 的 `candidates` 顺序。
fn extract_patch_text(source: &Value, argument: Option<&str>) -> Option<String> {
    let mut candidates: Vec<Value> = Vec::new();
    if let Some(argument) = argument {
        candidates.push(decode_js_value(argument));
    }
    if let Value::String(text) = source {
        let decoded = decode_js_value(text);
        if decoded != *source {
            candidates.push(decoded);
        }
        candidates.extend(js_string_values(text));
    }
    candidates.push(source.clone());
    for candidate in candidates {
        let candidate = match &candidate {
            Value::Object(entries) => entries
                .get("patch_text")
                .filter(|value| truthy(value))
                .or_else(|| entries.get("patch").filter(|value| truthy(value)))
                .or_else(|| entries.get("input"))
                .cloned()
                .unwrap_or(Value::Null),
            other => other.clone(),
        };
        let Value::String(text) = candidate else {
            continue;
        };
        let Some(start) = text.find(BEGIN_PATCH) else {
            continue;
        };
        return Some(match text[start..].find(END_PATCH) {
            Some(offset) => text[start..start + offset + END_PATCH.len()].to_string(),
            None => text[start..].to_string(),
        });
    }
    None
}

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

/// 把补丁正文拆成结构化变更清单。
fn patch_changes(patch_text: &str) -> Vec<Value> {
    let headers: Vec<_> = PATCH_HEADER_RE.captures_iter(patch_text).collect();
    let starts: Vec<usize> = headers
        .iter()
        .map(|header| header.get(0).expect("整体匹配存在").start())
        .collect();
    let mut changes = Vec::with_capacity(headers.len());
    for (index, header) in headers.iter().enumerate() {
        let whole = header.get(0).expect("整体匹配存在");
        let operation = header[1].to_lowercase();
        let path = header[2].trim().to_string();
        let end = starts.get(index + 1).copied().unwrap_or(patch_text.len());
        let section = &patch_text[whole.end()..end];
        let move_to = if operation == "update" {
            PATCH_MOVE_RE.captures(section)
        } else {
            None
        };
        let mut change = Map::new();
        change.insert(
            "operation".into(),
            Value::from(if move_to.is_some() {
                "move".to_string()
            } else {
                operation
            }),
        );
        change.insert("path".into(), Value::from(path));
        change.insert(
            "hunk_count".into(),
            Value::from(HUNK_RE.find_iter(section).count() as i64),
        );
        if let Some(move_to) = move_to {
            change.insert("destination".into(), Value::from(move_to[1].trim()));
        }
        changes.push(Value::Object(change));
    }
    changes
}

fn patch_call(patch_text: &str) -> ToolCall {
    let mut input = Map::new();
    input.insert("operations".into(), Value::Array(patch_changes(patch_text)));
    input.insert("raw_patch".into(), Value::from(patch_text));
    ToolCall::new(
        "apply_patch",
        Some(CanonicalOp::FS_PATCH.to_string()),
        Value::Object(input),
    )
}

/// `"string" if isinstance(value, str) else type(value).__name__`。
///
/// 注意字符串报的是 `"string"` 而不是 Python 的类型名 `"str"`——这是原实现
/// 写死的字面量，不能用统一的类型名映射代替。
fn input_kind_name(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "string",
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(number) => {
            if number.is_f64() {
                "float"
            } else {
                "int"
            }
        }
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

fn input_summary(name: &str, argument: &str) -> Value {
    let value = decode_js_value(argument);
    let mut summary = Map::new();
    summary.insert("native_name".into(), Value::from(name));
    match &value {
        Value::Object(entries) => {
            let mut fields: Vec<String> = entries.keys().cloned().collect();
            fields.sort();
            summary.insert("input_kind".into(), Value::from("object"));
            summary.insert(
                "input_fields".into(),
                Value::Array(fields.into_iter().map(Value::from).collect()),
            );
        }
        other => {
            summary.insert("input_kind".into(), Value::from(input_kind_name(other)));
            summary.insert("input_fields".into(), Value::Array(Vec::new()));
        }
    }
    Value::Object(summary)
}

fn opaque_call(name: &str, native_input: &Value, calls: &[(String, String)]) -> ToolCall {
    let mut input = Map::new();
    input.insert("namespace".into(), Value::from("codex"));
    input.insert("name".into(), Value::from(name));
    input.insert("input".into(), native_input.clone());
    if !calls.is_empty() {
        let mut structure = Map::new();
        structure.insert(
            "kind".into(),
            Value::from(if calls.len() > 1 {
                "composite"
            } else {
                "single"
            }),
        );
        structure.insert("invocation_count".into(), Value::from(calls.len() as i64));
        structure.insert(
            "tool_names".into(),
            Value::Array(
                calls
                    .iter()
                    .map(|(name, _)| Value::from(name.as_str()))
                    .collect(),
            ),
        );
        input.insert("structure_summary".into(), Value::Object(structure));
        input.insert(
            "children".into(),
            Value::Array(
                calls
                    .iter()
                    .map(|(name, argument)| input_summary(name, argument))
                    .collect(),
            ),
        );
    }
    ToolCall::new(
        name,
        Some(CanonicalOp::TOOL_INVOKE.to_string()),
        Value::Object(input),
    )
}

/// 解析 `custom_tool_call`：JS 源码 → 规范调用。
pub fn parse_custom_call(payload: &Map<String, Value>, session: &mut Session) -> ToolCall {
    let source = payload.get("input").cloned().unwrap_or(Value::from(""));
    let native_name = payload
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("custom_tool")
        .to_string();
    if native_name == "apply_patch" {
        if let Some(patch_text) = extract_patch_text(&source, None) {
            return patch_call(&patch_text);
        }
        session.lose("migration.apply_patch_unparsed", Map::new());
        return opaque_call(&native_name, &source, &[]);
    }

    let calls = match &source {
        Value::String(text) => scan_tool_invocations(text),
        _ => Vec::new(),
    };
    if calls.len() != 1 {
        return opaque_call(&native_name, &source, &calls);
    }

    let (call_name, argument) = (&calls[0].0, &calls[0].1);
    if call_name == "exec_command" {
        let args = decode_js_value(argument);
        if let Some(entries) = args.as_object() {
            if let Some(shell_input) = decode_shell(entries) {
                return ToolCall::new(
                    "exec",
                    Some(CanonicalOp::SHELL_EXEC.to_string()),
                    Value::Object(shell_input),
                );
            }
        }
    } else if call_name == "apply_patch" {
        if let Some(patch_text) = extract_patch_text(&source, Some(argument)) {
            return patch_call(&patch_text);
        }
        session.lose("migration.apply_patch_unparsed", Map::new());
    }
    opaque_call(&native_name, &source, &calls)
}

/// `function_call.arguments` 的解析：JSON 对象优先，其余原样保留。
pub fn json_args(raw: &Value) -> Value {
    if raw.is_object() {
        return raw.clone();
    }
    // Python 的 `raw or "{}"`：falsy 值（None/""/0/[]）先替换成 "{}"。
    let source = if truthy(raw) {
        raw.clone()
    } else {
        Value::from("{}")
    };
    let Value::String(text) = &source else {
        // json.loads 收到非字符串抛 TypeError → `return raw or ""`。
        return if truthy(raw) {
            raw.clone()
        } else {
            Value::from("")
        };
    };
    match serde_json::from_str::<Value>(text) {
        Ok(value) if value.is_object() => value,
        Ok(_) => raw.clone(),
        Err(_) => {
            if truthy(raw) {
                raw.clone()
            } else {
                Value::from("")
            }
        }
    }
}

/// `spawn_agent` 入参归一成 `agent.spawn` 的规范形态。
pub fn spawn_input(raw: &Value) -> Value {
    let empty = Map::new();
    let args = raw.as_object().unwrap_or(&empty);
    let first_truthy = |keys: &[&str]| -> Option<Value> {
        keys.iter()
            .find_map(|key| args.get(*key).filter(|value| truthy(value)).cloned())
    };
    let mut result = Map::new();
    result.insert(
        "description".into(),
        Value::from(
            first_truthy(&["description"])
                .map(|value| python_str(&value))
                .unwrap_or_else(|| "migrated subagent".to_string()),
        ),
    );
    result.insert(
        "prompt".into(),
        Value::from(
            first_truthy(&["prompt", "message"])
                .map(|value| python_str(&value))
                .unwrap_or_default(),
        ),
    );
    result.insert(
        "subagent_type".into(),
        Value::from(
            first_truthy(&["subagent_type", "agent_type"])
                .map(|value| python_str(&value))
                .unwrap_or_else(|| "general".to_string()),
        ),
    );
    // 别名表：取第一个**非 None**（而非 truthy）的候选值。
    for (field, candidates) in [
        ("task_name", &["task_name"][..]),
        ("model", &["model"][..]),
        ("fork_mode", &["fork_mode", "mode"][..]),
        ("fork_turns", &["fork_turns"][..]),
        ("reasoning_effort", &["reasoning_effort"][..]),
    ] {
        let value = candidates
            .iter()
            .find_map(|key| args.get(*key).filter(|value| !value.is_null()));
        if let Some(value) = value {
            result.insert(field.into(), Value::from(python_str(value)));
        }
    }
    Value::Object(result)
}

/// 解析 `function_call`：spawn 特判 → 方言归一 → `tool.invoke` 兜底。
pub fn parse_function_call(payload: &Map<String, Value>) -> ToolCall {
    let name = match payload.get("name") {
        None => "?".to_string(),
        Some(value) => value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| python_str(value)),
    };
    let raw_arguments = payload
        .get("arguments")
        .cloned()
        .unwrap_or(Value::from("{}"));
    let args = json_args(&raw_arguments);
    let call_id = payload
        .get("call_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    if name == "spawn_agent" {
        let mut call = ToolCall::new(
            "spawn_agent",
            Some(CanonicalOp::AGENT_SPAWN.to_string()),
            spawn_input(&args),
        );
        call.source_call_id = call_id;
        return call;
    }
    if let Some((op, canonical)) = DIALECT.parse(&name, &args) {
        let mut call = ToolCall::new(name, Some(op.to_string()), canonical);
        call.source_call_id = call_id;
        return call;
    }
    let mut input = Map::new();
    input.insert("namespace".into(), Value::from("codex"));
    input.insert("name".into(), Value::from(name.as_str()));
    input.insert("input".into(), args);
    let mut call = ToolCall::new(
        name,
        Some(CanonicalOp::TOOL_INVOKE.to_string()),
        Value::Object(input),
    );
    call.source_call_id = call_id;
    call
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    fn session() -> Session {
        Session::new("codex", "s", "/tmp")
    }

    #[test]
    fn strings_and_comments_hide_tool_invocations() {
        assert!(scan_tool_invocations("const a = \"tools.exec_command(1)\";").is_empty());
        assert!(scan_tool_invocations("// tools.exec_command(1)").is_empty());
        assert!(scan_tool_invocations("/* tools.exec_command(1) */").is_empty());
        // 前缀是标识符字符时不算调用。
        assert!(scan_tool_invocations("mytools.exec_command(1)").is_empty());
    }

    #[test]
    fn nested_parentheses_and_strings_stay_balanced() {
        let calls = scan_tool_invocations("await tools.run({\"cmd\": \"a(b)\", \"n\": f(1)});");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "run");
        assert_eq!(calls[0].1, "{\"cmd\": \"a(b)\", \"n\": f(1)}");
    }

    #[test]
    fn whitespace_between_name_and_paren_is_allowed() {
        let calls = scan_tool_invocations("tools.exec_command\n  (1)");
        assert_eq!(calls, [("exec_command".to_string(), "1".to_string())]);
        // 未闭合的括号不产出调用。
        assert!(scan_tool_invocations("tools.exec_command(1").is_empty());
    }

    #[test]
    fn js_values_decode_json_first_then_quoted_literals() {
        assert_eq!(decode_js_value(" {\"a\": 1} "), json!({"a": 1}));
        assert_eq!(decode_js_value("'a\\nb'"), json!("a\nb"));
        assert_eq!(decode_js_value("`x\\ty`"), json!("x\ty"));
        // 双引号字面量走 JSON 分支。
        assert_eq!(decode_js_value("\"a\\nb\""), json!("a\nb"));
        // 裸标识符原样返回（strip 之后）。
        assert_eq!(decode_js_value("  patch "), json!("patch"));
    }

    #[test]
    fn exec_command_becomes_a_canonical_shell_call() {
        let call = parse_custom_call(
            &payload(json!({
                "name": "exec",
                "input": "const r = await tools.exec_command({\"cmd\":\"ls\",\"workdir\":\"/a\"});\n",
            })),
            &mut session(),
        );
        assert_eq!(call.name, "exec");
        assert_eq!(call.op.as_deref(), Some(CanonicalOp::SHELL_EXEC));
        assert_eq!(call.input, json!({"command": "ls", "workdir": "/a"}));
    }

    /// 关键顺序：`_decode_js_value(source)` 先 strip，结果与原串不等时会被排到
    /// 字符串字面量**之前**，于是补丁保留 JS 源码里的 `\n` 字面量。
    #[test]
    fn apply_patch_prefers_the_stripped_source_over_decoded_literals() {
        let call = parse_custom_call(
            &payload(json!({
                "name": "exec",
                "input": "const patch = \"*** Begin Patch\\n*** Add File: /a.txt\\n+hi\\n*** End Patch\";\ntext(await tools.apply_patch(patch));\n",
            })),
            &mut session(),
        );
        assert_eq!(call.name, "apply_patch");
        assert_eq!(call.op.as_deref(), Some(CanonicalOp::FS_PATCH));
        assert_eq!(
            call.input["raw_patch"],
            json!("*** Begin Patch\\n*** Add File: /a.txt\\n+hi\\n*** End Patch")
        );
        // 补丁体里没有真实换行 → 头部正则不命中 → operations 为空。
        assert_eq!(call.input["operations"], json!([]));
    }

    #[test]
    fn real_newlines_produce_structured_patch_operations() {
        let patch = "*** Begin Patch\n*** Update File: a.txt\n*** Move to: b.txt\n@@\n-x\n+y\n*** Delete File: c.txt\n*** End Patch";
        let call = parse_custom_call(
            &payload(json!({"name": "apply_patch", "input": patch})),
            &mut session(),
        );
        assert_eq!(
            call.input["operations"],
            json!([
                {"operation": "move", "path": "a.txt", "hunk_count": 1, "destination": "b.txt"},
                {"operation": "delete", "path": "c.txt", "hunk_count": 0},
            ])
        );
        assert_eq!(call.input["raw_patch"], json!(patch));
    }

    #[test]
    fn unparsed_apply_patch_records_a_loss_and_degrades() {
        let mut session = session();
        let call = parse_custom_call(
            &payload(json!({"name": "apply_patch", "input": "no patch here"})),
            &mut session,
        );
        assert_eq!(call.op.as_deref(), Some(CanonicalOp::TOOL_INVOKE));
        assert_eq!(session.loss.len(), 1);
        assert_eq!(session.loss[0].code, "migration.apply_patch_unparsed");
    }

    #[test]
    fn multiple_invocations_become_an_opaque_call_with_a_summary() {
        let source = "tools.a({\"x\":1});\ntools.b('y');\n";
        let call = parse_custom_call(
            &payload(json!({"name": "script", "input": source})),
            &mut session(),
        );
        assert_eq!(call.op.as_deref(), Some(CanonicalOp::TOOL_INVOKE));
        assert_eq!(
            call.input["structure_summary"],
            json!({"kind": "composite", "invocation_count": 2, "tool_names": ["a", "b"]})
        );
        assert_eq!(
            call.input["children"],
            json!([
                {"native_name": "a", "input_kind": "object", "input_fields": ["x"]},
                {"native_name": "b", "input_kind": "string", "input_fields": []},
            ])
        );
    }

    #[test]
    fn zero_invocations_keep_the_raw_source_without_a_summary() {
        let call = parse_custom_call(
            &payload(json!({"name": "script", "input": "console.log(1);"})),
            &mut session(),
        );
        assert_eq!(
            call.input,
            json!({"namespace": "codex", "name": "script", "input": "console.log(1);"})
        );
    }

    #[test]
    fn function_calls_route_through_spawn_dialect_and_fallback() {
        let spawn = parse_function_call(&payload(json!({
            "name": "spawn_agent",
            "call_id": "c1",
            "arguments": "{\"message\":\"go\",\"agent_type\":\"docs\",\"mode\":\"fork\"}",
        })));
        assert_eq!(spawn.op.as_deref(), Some(CanonicalOp::AGENT_SPAWN));
        assert_eq!(
            spawn.input,
            json!({
                "description": "migrated subagent",
                "prompt": "go",
                "subagent_type": "docs",
                "fork_mode": "fork",
            })
        );
        assert_eq!(spawn.source_call_id.as_deref(), Some("c1"));

        let shell = parse_function_call(&payload(json!({
            "name": "exec_command",
            "arguments": "{\"cmd\":\"pwd\"}",
        })));
        assert_eq!(shell.op.as_deref(), Some(CanonicalOp::SHELL_EXEC));
        assert_eq!(shell.input, json!({"command": "pwd"}));

        let unknown = parse_function_call(&payload(json!({
            "name": "mystery",
            "arguments": "{\"a\":1}",
        })));
        assert_eq!(unknown.op.as_deref(), Some(CanonicalOp::TOOL_INVOKE));
        assert_eq!(
            unknown.input,
            json!({"namespace": "codex", "name": "mystery", "input": {"a": 1}})
        );
    }

    #[test]
    fn json_args_keeps_non_object_payloads_verbatim() {
        assert_eq!(json_args(&json!({"a": 1})), json!({"a": 1}));
        assert_eq!(json_args(&json!("{\"a\": 1}")), json!({"a": 1}));
        // 合法 JSON 但不是对象 → 原样返回原串。
        assert_eq!(json_args(&json!("[1]")), json!("[1]"));
        assert_eq!(json_args(&json!("oops")), json!("oops"));
        // falsy 输入等价 "{}"。
        assert_eq!(json_args(&json!("")), json!({}));
        assert_eq!(json_args(&json!(null)), json!({}));
    }
}
