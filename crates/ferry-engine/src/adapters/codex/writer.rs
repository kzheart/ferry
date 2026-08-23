//! Codex writer：规范化中间格式 → rollout JSONL（可被 `codex exec resume` 加载）。
//!
//! - 结构模板来自声明式格式配置档（`native_schema`），真实 CLI 样本仅用于测试
//!   配置档与原生格式保持一致。
//! - `shell.exec` 原生映射为 `exec_command`；`fs.write` 映射为 `apply_patch`
//!   （Add File）；其余工具降级为叙述文本（narration）。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::Rng as _;
use serde_json::{Map, Value};

use crate::adapters::shared::dialect::shell_quote;
use crate::adapters::shared::migration::{RenderDecision, ToolVerdict};
use crate::adapters::shared::narration::narrate;
use crate::adapters::shared::writing::python_json_dumps;
use crate::errors::{DomainError, DomainResult};
use crate::model::{tool_result_text, AgentEdge, BlockKind, Message, Session, Timestamp, ToolCall};
use crate::system::paths::home_dir;
use crate::tool_ops::{has_valid_tool_input, CanonicalOp};

use super::native_schema::templates;
use super::registry::{register_tree, RegistryNode};

// ---------------------------------------------------------------------------
// 随机 id 与时间戳
// ---------------------------------------------------------------------------

/// `secrets.token_hex(n)`：2n 个小写十六进制字符。
pub fn token_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::rng().fill(&mut buffer[..]);
    buffer.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `secrets.token_urlsafe(18)[:24]`：18 字节 base64url 恰好 24 个字符（无填充）。
pub fn token_urlsafe_24() -> String {
    use base64::Engine as _;
    let mut buffer = [0u8; 18];
    rand::rng().fill(&mut buffer[..]);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buffer)
}

/// `_uuid7()`：毫秒时间戳前缀 + 随机尾部，写入 version 7 与 RFC 4122 变体位。
pub fn uuid7() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64;
    let mut bytes = [0u8; 16];
    bytes[..6].copy_from_slice(&millis.to_be_bytes()[2..]);
    rand::rng().fill(&mut bytes[6..]);
    bytes[6] = (bytes[6] & 0x0F) | 0x70;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Howard Hinnant 的 `civil_from_days`：纪元天数 → 民用日期。
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (year + i64::from(month <= 2), month, day)
}

/// 本机时区相对 UTC 的秒偏移。
///
/// Python 的 `time.strftime(...)` 不带时间实参时用**本地时间**；Rust 标准库没有
/// 本地时区，unix 上走 `libc::localtime_r`，其他平台退化成 UTC。
fn local_offset_seconds(epoch_seconds: i64) -> i64 {
    #[cfg(unix)]
    {
        // SAFETY：`localtime_r` 把结果写进调用方提供的 `tm`，无全局状态。
        unsafe {
            let time = epoch_seconds as libc::time_t;
            let mut parts: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&time, &mut parts).is_null() {
                return 0;
            }
            parts.tm_gmtoff as i64
        }
    }
    #[cfg(not(unix))]
    {
        let _ = epoch_seconds;
        0
    }
}

struct Clock {
    epoch_seconds: i64,
    millis: i64,
}

fn now() -> Clock {
    let delta = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    Clock {
        epoch_seconds: delta.as_secs() as i64,
        millis: (delta.as_millis() % 1000) as i64,
    }
}

fn format_parts(epoch_seconds: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = epoch_seconds.div_euclid(86_400);
    let rest = epoch_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    (year, month, day, rest / 3600, (rest % 3600) / 60, rest % 60)
}

/// `time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime()) + f".{ms:03d}Z"`。
pub fn now_iso() -> String {
    let clock = now();
    let (year, month, day, hour, minute, second) = format_parts(clock.epoch_seconds);
    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{:03}Z",
        clock.millis
    )
}

/// epoch 毫秒 → `isoformat(timespec="milliseconds")` + `Z`。
fn iso_from_millis(millis: i64) -> String {
    let seconds = millis.div_euclid(1000);
    let fraction = millis.rem_euclid(1000);
    let (year, month, day, hour, minute, second) = format_parts(seconds);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{fraction:03}Z")
}

/// 保留 canonical 时间；仅在来源缺失时生成当前 UTC 时间。
fn timestamp(value: Option<&Timestamp>) -> String {
    match value {
        Some(Timestamp::Text(text)) if !text.trim().is_empty() => text.clone(),
        Some(Timestamp::Millis(value)) => {
            // `value > 10_000_000_000` 判定单位是毫秒还是秒。
            let millis = if *value > 10_000_000_000 {
                *value
            } else {
                *value * 1000
            };
            iso_from_millis(millis)
        }
        _ => now_iso(),
    }
}

// ---------------------------------------------------------------------------
// JSON 序列化辅助
// ---------------------------------------------------------------------------

/// `json.dumps(value)`（默认 `ensure_ascii=True`）：非 ASCII 转成 `\uXXXX`。
fn json_dumps_ascii(value: &Value) -> String {
    let mut out = String::new();
    for character in python_json_dumps(value).chars() {
        if character.is_ascii() {
            out.push(character);
            continue;
        }
        let mut buffer = [0u16; 2];
        for unit in character.encode_utf16(&mut buffer) {
            out.push_str(&format!("\\u{unit:04x}"));
        }
    }
    out
}

/// `dict.update`：已存在的键保持位置只换值，新键追加到末尾。
fn dict_update(target: &mut Map<String, Value>, other: &Map<String, Value>) {
    for (key, value) in other {
        target.insert(key.clone(), value.clone());
    }
}

fn clone_template(templates: &Map<String, Value>, key: &str) -> Value {
    templates
        .get(key)
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()))
}

fn payload_mut(record: &mut Value) -> &mut Map<String, Value> {
    record
        .as_object_mut()
        .expect("模板记录是对象")
        .entry("payload")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .expect("payload 是对象")
}

fn set(record: &mut Value, key: &str, value: Value) {
    record
        .as_object_mut()
        .expect("模板记录是对象")
        .insert(key.to_string(), value);
}

// ---------------------------------------------------------------------------
// 记录渲染
// ---------------------------------------------------------------------------

fn message_record(
    templates: &Map<String, Value>,
    role: &str,
    text: &str,
    created_at: Option<&Timestamp>,
) -> Value {
    let mut record = clone_template(templates, &format!("message.{role}"));
    set(&mut record, "timestamp", Value::from(timestamp(created_at)));
    let payload = payload_mut(&mut record);
    let mut block = Map::new();
    block.insert(
        "type".into(),
        Value::from(if role == "user" {
            "input_text"
        } else {
            "output_text"
        }),
    );
    block.insert("text".into(), Value::from(text));
    payload.insert("content".into(), Value::Array(vec![Value::Object(block)]));
    // 原生 user message 不携带 id；assistant message 以 msg_* 标识。
    if role == "user" {
        let rebuilt: Map<String, Value> = payload
            .iter()
            .filter(|(key, _)| key.as_str() != "id")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        *payload = rebuilt;
    } else {
        payload.insert("id".into(), Value::from(format!("msg_{}", token_hex(25))));
    }
    record
}

fn result_payload(tool: &ToolCall, output: &str) -> Map<String, Value> {
    let mut payload = Map::new();
    let Some(result) = tool.result.as_ref() else {
        payload.insert("output".into(), Value::from(output));
        return payload;
    };
    payload.insert("status".into(), Value::from(status_name(result.status)));
    payload.insert("output".into(), Value::from(tool_result_text(Some(result))));
    if let Some(stdout) = result.stdout.as_deref() {
        payload.insert("stdout".into(), Value::from(stdout));
    }
    if let Some(stderr) = result.stderr.as_deref() {
        payload.insert("stderr".into(), Value::from(stderr));
    }
    if let Some(exit_code) = result.exit_code {
        payload.insert("exit_code".into(), Value::from(exit_code));
    }
    if let Some(truncated) = result.truncated {
        payload.insert("truncated".into(), Value::Bool(truncated));
    }
    payload
}

fn status_name(status: crate::model::ToolResultStatus) -> &'static str {
    use crate::model::ToolResultStatus as Status;
    match status {
        Status::Success => "success",
        Status::Error => "error",
        Status::Interrupted => "interrupted",
        Status::Running => "running",
        Status::Pending => "pending",
        Status::Unknown => "unknown",
    }
}

struct ExecArgs<'a> {
    command: &'a str,
    workdir: &'a str,
    stdout: &'a str,
    exit_code: Option<i64>,
    started_at: Option<&'a Timestamp>,
    ended_at: Option<&'a Timestamp>,
    result: Option<Map<String, Value>>,
}

fn exec_pair(templates: &Map<String, Value>, args: &ExecArgs<'_>) -> Vec<Value> {
    let mut call = clone_template(templates, "response_item.custom_tool_call");
    let mut output = clone_template(templates, "response_item.custom_tool_call_output");
    let call_id = format!("call_{}", token_urlsafe_24());
    set(
        &mut call,
        "timestamp",
        Value::from(timestamp(args.started_at)),
    );
    set(
        &mut output,
        "timestamp",
        Value::from(timestamp(args.ended_at.or(args.started_at))),
    );
    {
        let payload = payload_mut(&mut call);
        payload.insert("id".into(), Value::from(format!("ctc_{}", token_hex(25))));
        payload.insert("call_id".into(), Value::from(call_id.as_str()));
        payload.insert("name".into(), Value::from("exec"));
        let mut arguments = Map::new();
        arguments.insert("cmd".into(), Value::from(args.command));
        arguments.insert("workdir".into(), Value::from(args.workdir));
        arguments.insert("yield_time_ms".into(), Value::from(10000));
        arguments.insert("max_output_tokens".into(), Value::from(1000));
        payload.insert(
            "input".into(),
            Value::from(format!(
                "const r = await tools.exec_command({});\ntext(JSON.stringify(r));\n",
                json_dumps_ascii(&Value::Object(arguments))
            )),
        );
    }
    let mut inner = Map::new();
    inner.insert("chunk_id".into(), Value::from(token_hex(3)));
    inner.insert(
        "wall_time_seconds".into(),
        serde_json::Number::from_f64(0.01).map_or(Value::Null, Value::Number),
    );
    inner.insert(
        "original_token_count".into(),
        Value::from((args.stdout.chars().count() / 4).max(1) as i64),
    );
    inner.insert("output".into(), Value::from(args.stdout));
    if let Some(exit_code) = args.exit_code {
        inner.insert("exit_code".into(), Value::from(exit_code));
    }
    if let Some(result) = args.result.as_ref().filter(|value| !value.is_empty()) {
        dict_update(&mut inner, result);
    }
    {
        let payload = payload_mut(&mut output);
        payload.insert("id".into(), Value::from(format!("fco_{}", uuid7())));
        payload.insert(
            "output".into(),
            Value::from(json_dumps_ascii(&wrapped_output(&python_json_dumps(
                &Value::Object(inner),
            )))),
        );
        let rebuilt: Map<String, Value> = payload
            .iter()
            .filter(|(key, _)| key.as_str() != "internal_chat_message_metadata_passthrough")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        *payload = rebuilt;
    }
    vec![call, output]
}

fn wrapped_output(inner: &str) -> Value {
    let mut header = Map::new();
    header.insert("type".into(), Value::from("input_text"));
    header.insert(
        "text".into(),
        Value::from("Script completed\nWall time 0.1 seconds\nOutput:\n"),
    );
    let mut body = Map::new();
    body.insert("type".into(), Value::from("input_text"));
    body.insert("text".into(), Value::from(inner));
    Value::Array(vec![Value::Object(header), Value::Object(body)])
}

fn apply_patch_pair(
    templates: &Map<String, Value>,
    patch: &str,
    output: &str,
    result: Option<Map<String, Value>>,
) -> Vec<Value> {
    let mut records = exec_pair(
        templates,
        &ExecArgs {
            command: "",
            workdir: "",
            stdout: "{}",
            exit_code: Some(0),
            started_at: None,
            ended_at: None,
            result: None,
        },
    );
    payload_mut(&mut records[0]).insert(
        "input".into(),
        Value::from(format!(
            "const patch = {};\ntext(await tools.apply_patch(patch));\n",
            json_dumps_ascii(&Value::from(patch))
        )),
    );
    let payload = match result.filter(|value| !value.is_empty()) {
        Some(result) => result,
        None => {
            let mut fallback = Map::new();
            fallback.insert("output".into(), Value::from(output));
            fallback
        }
    };
    payload_mut(&mut records[1]).insert(
        "output".into(),
        Value::from(json_dumps_ascii(&wrapped_output(&python_json_dumps(
            &Value::Object(payload),
        )))),
    );
    records
}

fn input_of(tool: &ToolCall) -> Map<String, Value> {
    tool.input.as_object().cloned().unwrap_or_default()
}

fn text_field(input: &Map<String, Value>, key: &str) -> Option<String> {
    match input.get(key)? {
        Value::Null | Value::Bool(false) => None,
        Value::String(text) if text.is_empty() => None,
        Value::String(text) => Some(text.clone()),
        other => Some(crate::adapters::shared::dialect::python_str(other)),
    }
}

fn plus_lines(body: &str) -> String {
    body.lines()
        .map(|line| format!("+{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_shell_exec(
    templates: &Map<String, Value>,
    tool: &ToolCall,
    cwd: &str,
) -> Option<Vec<Value>> {
    let input = input_of(tool);
    let command = text_field(&input, "command")?;
    let output = tool_result_text(tool.result.as_ref());
    let stdout = tool
        .result
        .as_ref()
        .and_then(|result| result.stdout.clone())
        .unwrap_or_else(|| output.clone());
    let exit_code = tool.result.as_ref().and_then(|result| result.exit_code);
    let workdir = text_field(&input, "workdir").unwrap_or_else(|| cwd.to_string());
    Some(exec_pair(
        templates,
        &ExecArgs {
            command: &command,
            workdir: &workdir,
            stdout: &stdout,
            exit_code,
            started_at: tool.started_at.as_ref(),
            ended_at: tool.ended_at.as_ref(),
            result: Some(result_payload(tool, &output)),
        },
    ))
}

fn write_fs_read(templates: &Map<String, Value>, tool: &ToolCall, cwd: &str) -> Option<Vec<Value>> {
    let input = input_of(tool);
    let file_path = text_field(&input, "file_path")?;
    let command = format!("cat {}", shell_quote(&file_path));
    let output = tool_result_text(tool.result.as_ref());
    Some(exec_pair(
        templates,
        &ExecArgs {
            command: &command,
            workdir: cwd,
            stdout: &output,
            exit_code: tool.result.as_ref().and_then(|result| result.exit_code),
            started_at: tool.started_at.as_ref(),
            ended_at: tool.ended_at.as_ref(),
            result: Some(result_payload(tool, &output)),
        },
    ))
}

fn write_fs_write(
    templates: &Map<String, Value>,
    tool: &ToolCall,
    _cwd: &str,
) -> Option<Vec<Value>> {
    let input = input_of(tool);
    let file_path = text_field(&input, "file_path")?;
    let body = input
        .get("content")
        .map(|value| match value {
            Value::String(text) => text.clone(),
            other => crate::adapters::shared::dialect::python_str(other),
        })
        .unwrap_or_default();
    let patch = format!(
        "*** Begin Patch\n*** Add File: {file_path}\n{}\n*** End Patch",
        plus_lines(&body)
    );
    let output = tool_result_text(tool.result.as_ref());
    let output = if output.is_empty() {
        "{}".to_string()
    } else {
        output
    };
    Some(apply_patch_pair(
        templates,
        &patch,
        &output,
        Some(result_payload(tool, &output)),
    ))
}

fn write_fs_edit(
    templates: &Map<String, Value>,
    tool: &ToolCall,
    _cwd: &str,
) -> Option<Vec<Value>> {
    let input = input_of(tool);
    let file_path = text_field(&input, "file_path")?;
    let old = input
        .get("old")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let new = input
        .get("new")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut hunk = vec!["@@".to_string()];
    hunk.extend(old.lines().map(|line| format!("-{line}")));
    hunk.extend(new.lines().map(|line| format!("+{line}")));
    let patch = format!(
        "*** Begin Patch\n*** Update File: {file_path}\n{}\n*** End Patch",
        hunk.join("\n")
    );
    let output = tool_result_text(tool.result.as_ref());
    let output = if output.is_empty() {
        "{}".to_string()
    } else {
        output
    };
    Some(apply_patch_pair(
        templates,
        &patch,
        &output,
        Some(result_payload(tool, &output)),
    ))
}

fn write_fs_patch(
    templates: &Map<String, Value>,
    tool: &ToolCall,
    _cwd: &str,
) -> Option<Vec<Value>> {
    let input = input_of(tool);
    let patch = text_field(&input, "raw_patch")?;
    let output = tool_result_text(tool.result.as_ref());
    let output = if output.is_empty() {
        "{}".to_string()
    } else {
        output
    };
    Some(apply_patch_pair(
        templates,
        &patch,
        &output,
        Some(result_payload(tool, &output)),
    ))
}

fn write_fs_search(
    templates: &Map<String, Value>,
    tool: &ToolCall,
    cwd: &str,
) -> Option<Vec<Value>> {
    let input = input_of(tool);
    let query = text_field(&input, "query")?;
    let mut command = vec![
        "rg".to_string(),
        "--line-number".to_string(),
        "--color".to_string(),
        "never".to_string(),
    ];
    if let Some(glob) = text_field(&input, "glob") {
        command.push("-g".to_string());
        command.push(glob);
    }
    command.push("--".to_string());
    command.push(query);
    command.push(text_field(&input, "path").unwrap_or_else(|| ".".to_string()));
    let quoted = command
        .iter()
        .map(|part| shell_quote(part))
        .collect::<Vec<_>>()
        .join(" ");
    let output = tool_result_text(tool.result.as_ref());
    let workdir = text_field(&input, "workdir").unwrap_or_else(|| cwd.to_string());
    Some(exec_pair(
        templates,
        &ExecArgs {
            command: &quoted,
            workdir: &workdir,
            stdout: &output,
            exit_code: tool.result.as_ref().and_then(|result| result.exit_code),
            started_at: tool.started_at.as_ref(),
            ended_at: tool.ended_at.as_ref(),
            result: Some(result_payload(tool, &output)),
        },
    ))
}

fn write_fs_glob(templates: &Map<String, Value>, tool: &ToolCall, cwd: &str) -> Option<Vec<Value>> {
    let input = input_of(tool);
    let pattern = text_field(&input, "pattern")?;
    let command = [
        "rg".to_string(),
        "--files".to_string(),
        "-g".to_string(),
        pattern,
        "--".to_string(),
        text_field(&input, "path").unwrap_or_else(|| ".".to_string()),
    ];
    let quoted = command
        .iter()
        .map(|part| shell_quote(part))
        .collect::<Vec<_>>()
        .join(" ");
    let output = tool_result_text(tool.result.as_ref());
    Some(exec_pair(
        templates,
        &ExecArgs {
            command: &quoted,
            workdir: cwd,
            stdout: &output,
            exit_code: tool.result.as_ref().and_then(|result| result.exit_code),
            started_at: tool.started_at.as_ref(),
            ended_at: tool.ended_at.as_ref(),
            result: Some(result_payload(tool, &output)),
        },
    ))
}

type OpWriter = fn(&Map<String, Value>, &ToolCall, &str) -> Option<Vec<Value>>;

/// 规范操作 → 原生记录渲染器。
const OP_WRITERS: &[(&str, OpWriter)] = &[
    (CanonicalOp::SHELL_EXEC, write_shell_exec),
    (CanonicalOp::FS_READ, write_fs_read),
    (CanonicalOp::FS_WRITE, write_fs_write),
    (CanonicalOp::FS_EDIT, write_fs_edit),
    (CanonicalOp::FS_PATCH, write_fs_patch),
    (CanonicalOp::FS_SEARCH, write_fs_search),
    (CanonicalOp::FS_GLOB, write_fs_glob),
];

/// `OP_FIDELITY`：可渲染的操作默认 native，随后被显式降级项覆盖。
///
/// Codex 没有原生 read 工具；渲染成 `cat` 保住了内容但改变了工具语义，
/// 迁移预览必须如实披露。
pub fn op_fidelity(op: &str) -> ToolVerdict {
    match op {
        CanonicalOp::FS_READ
        | CanonicalOp::FS_SEARCH
        | CanonicalOp::FS_GLOB
        | CanonicalOp::WEB_FETCH
        | CanonicalOp::WEB_SEARCH
        | CanonicalOp::TOOL_INVOKE => ToolVerdict::Degrade,
        CanonicalOp::AGENT_SPAWN => ToolVerdict::Native,
        other if OP_WRITERS.iter().any(|(name, _)| *name == other) => ToolVerdict::Native,
        _ => ToolVerdict::Degrade,
    }
}

/// 用目标端映射表渲染一次规范操作。
fn native_records(
    templates: &Map<String, Value>,
    tool: &ToolCall,
    cwd: &str,
    message_time: Option<&Timestamp>,
) -> Option<Vec<Value>> {
    let op = tool.op.as_deref()?;
    let writer = OP_WRITERS
        .iter()
        .find(|(name, _)| *name == op)
        .map(|(_, writer)| *writer)?;
    if !has_valid_tool_input(Some(op), &tool.input) {
        return None;
    }
    let mut records = writer(templates, tool, cwd)?;
    if records.is_empty() {
        return Some(records);
    }
    // Claude 等来源通常只有消息时间、没有独立工具时间：沿用所属消息的时间，
    // 避免历史工具记录被标成迁移时刻。
    if tool.started_at.is_none() {
        if let Some(message_time) = message_time {
            set(
                &mut records[0],
                "timestamp",
                Value::from(timestamp(Some(message_time))),
            );
        }
    }
    if tool.ended_at.is_none() {
        if let Some(message_time) = message_time {
            let fallback = tool.started_at.as_ref().unwrap_or(message_time);
            let last = records.len() - 1;
            set(
                &mut records[last],
                "timestamp",
                Value::from(timestamp(Some(fallback))),
            );
        }
    }
    Some(records)
}

// ---------------------------------------------------------------------------
// 会话记录装配
// ---------------------------------------------------------------------------

/// 工具判定回调：由 `CodexMigrationTarget` 传入 `evaluate_tool`。
pub type ToolDecider<'a> =
    &'a dyn Fn(&ToolCall, &Session, Option<&Message>) -> DomainResult<RenderDecision>;

/// 一次子会话链接记录的生成参数。
struct ChildLink {
    child_index: usize,
    child_id: String,
    agent_path: String,
}

#[allow(clippy::too_many_arguments)]
fn session_records(
    templates: &Map<String, Value>,
    session: &Session,
    losses: &mut Vec<crate::events::Event>,
    cwd: &str,
    sid: &str,
    root_id: &str,
    parent_id: Option<&str>,
    depth: i64,
    agent_path: Option<&str>,
    child_links: &mut HashMap<String, Vec<ChildLink>>,
    tree: &Tree<'_>,
    decider: Option<ToolDecider<'_>>,
) -> DomainResult<Vec<Value>> {
    let now = timestamp(
        session
            .messages
            .iter()
            .find_map(|message| message.created_at.as_ref()),
    );
    let mut meta = clone_template(templates, "session_meta");
    set(&mut meta, "timestamp", Value::from(now.as_str()));
    {
        let payload = payload_mut(&mut meta);
        payload.insert("id".into(), Value::from(sid));
        payload.insert("session_id".into(), Value::from(root_id));
        payload.insert("timestamp".into(), Value::from(now.as_str()));
        payload.insert("cwd".into(), Value::from(cwd));
        payload.insert("originator".into(), Value::from("codex-tui"));
        payload.insert("source".into(), Value::from("cli"));
        payload.insert("thread_source".into(), Value::from("user"));
        payload.insert(
            "model_provider".into(),
            Value::from(
                session
                    .model_provider
                    .clone()
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "openai".to_string()),
            ),
        );
        match session.model.as_deref().filter(|value| !value.is_empty()) {
            Some(model) => {
                payload.insert("model".into(), Value::from(model));
            }
            None => {
                let rebuilt: Map<String, Value> = payload
                    .iter()
                    .filter(|(key, _)| key.as_str() != "model")
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                *payload = rebuilt;
            }
        }
        payload.insert("memory_mode".into(), Value::from("enabled"));
        payload.insert("history_mode".into(), Value::from("legacy"));
        payload.insert(
            "agent_path".into(),
            agent_path.map_or(Value::Null, Value::from),
        );
        if let Some(parent_id) = parent_id {
            payload.insert("parent_thread_id".into(), Value::from(parent_id));
            payload.insert("forked_from_id".into(), Value::from(parent_id));
            let mut spawn = Map::new();
            spawn.insert("parent_thread_id".into(), Value::from(parent_id));
            spawn.insert(
                "agent_path".into(),
                agent_path.map_or(Value::Null, Value::from),
            );
            spawn.insert("depth".into(), Value::from(depth));
            spawn.insert(
                "agent_nickname".into(),
                session
                    .agent_nickname
                    .as_deref()
                    .map_or(Value::Null, Value::from),
            );
            spawn.insert(
                "agent_role".into(),
                session
                    .agent_role
                    .as_deref()
                    .map_or(Value::Null, Value::from),
            );
            let mut subagent = Map::new();
            subagent.insert("thread_spawn".into(), Value::Object(spawn));
            let mut source = Map::new();
            source.insert("subagent".into(), Value::Object(subagent));
            payload.insert("source".into(), Value::Object(source));
            payload.insert("thread_source".into(), Value::from("subagent"));
        }
    }

    let mut out = vec![meta];
    for message in &session.messages {
        let mut texts: Vec<String> = Vec::new();
        let role = if message.role == "user" || message.role == "assistant" {
            message.role.as_str()
        } else {
            "user"
        };
        for block in &message.blocks {
            match block.kind {
                BlockKind::Text => {
                    let text = if message.role == "user" || message.role == "assistant" {
                        block.text.clone()
                    } else {
                        format!("[{}]\n{}", message.role, block.text)
                    };
                    texts.push(text);
                }
                BlockKind::Tool => {
                    let Some(tool) = block.tool.as_ref() else {
                        continue;
                    };
                    let decision = match decider {
                        Some(decider) => Some(decider(tool, session, Some(message))?),
                        None => None,
                    };
                    if tool.op.as_deref() == Some(CanonicalOp::AGENT_SPAWN) {
                        if let Some(decision) = decision.as_ref() {
                            if decision.rendered.is_none() {
                                losses.push(degraded_event(tool, Some(decision)));
                                texts.push(narrate(tool));
                            }
                        }
                        continue;
                    }
                    let renderable = decision
                        .as_ref()
                        .map(|decision| decision.rendered.is_some())
                        .unwrap_or(true);
                    let native = if renderable {
                        native_records(templates, tool, cwd, message.created_at.as_ref())
                    } else {
                        None
                    };
                    match native.filter(|records| !records.is_empty()) {
                        Some(native) => {
                            if !texts.is_empty() {
                                out.push(message_record(
                                    templates,
                                    role,
                                    &texts.join("\n\n"),
                                    message.created_at.as_ref(),
                                ));
                                texts.clear();
                            }
                            out.extend(native);
                        }
                        None => {
                            losses.push(degraded_event(tool, decision.as_ref()));
                            texts.push(narrate(tool));
                        }
                    }
                }
                _ => {}
            }
        }
        if !texts.is_empty() {
            out.push(message_record(
                templates,
                role,
                &texts.join("\n\n"),
                message.created_at.as_ref(),
            ));
        }
        // 子会话 spawn 是父消息的一部分；写在对应消息之后而非会话尾部。
        if let Some(source_id) = message.source_id.as_deref().filter(|id| !id.is_empty()) {
            if let Some(links) = child_links.remove(source_id) {
                for link in links {
                    out.extend(child_link_records(
                        session,
                        tree,
                        &link,
                        message.created_at.as_ref(),
                    ));
                }
            }
        }
    }
    // 缺少可定位消息的旧来源退化为追加；仍保留确定性 child 顺序。
    let mut leftover: Vec<String> = child_links.keys().cloned().collect();
    leftover.sort();
    for key in leftover {
        if let Some(links) = child_links.remove(&key) {
            for link in links {
                out.extend(child_link_records(session, tree, &link, None));
            }
        }
    }
    Ok(out)
}

fn degraded_event(tool: &ToolCall, decision: Option<&RenderDecision>) -> crate::events::Event {
    let mut params = Map::new();
    params.insert("tool_name".into(), Value::from(tool.name.as_str()));
    if let Some(decision) = decision {
        params.insert("fidelity".into(), Value::from(decision.fidelity.as_str()));
        params.insert(
            "reason_codes".into(),
            Value::Array(
                decision
                    .reason_codes
                    .iter()
                    .map(|code| Value::from(code.as_str()))
                    .collect(),
            ),
        );
        params.insert(
            "ignored_fields".into(),
            Value::Array(
                decision
                    .ignored_fields
                    .iter()
                    .map(|field| Value::from(field.as_str()))
                    .collect(),
            ),
        );
    }
    crate::events::event("migration.tool_degraded", params)
}

fn assistant_result(session: &Session) -> String {
    for message in session.messages.iter().rev() {
        if message.role != "assistant" {
            continue;
        }
        let text = message
            .blocks
            .iter()
            .filter(|block| block.kind == BlockKind::Text && !block.text.is_empty())
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            return text;
        }
    }
    String::new()
}

fn edge_for<'a>(parent: &'a Session, child: &Session) -> Option<&'a AgentEdge> {
    parent
        .agent_edges
        .iter()
        .find(|edge| edge.child_session_id == child.source_id)
}

fn edge_status(edge: Option<&AgentEdge>) -> String {
    let status = edge
        .and_then(|edge| edge.status.clone())
        .filter(|status| !status.is_empty())
        .unwrap_or_else(|| "closed".to_string());
    if status == "open" || status == "closed" {
        return status;
    }
    match status.to_lowercase().as_str() {
        "completed" | "complete" | "done" | "finished" | "failed" | "cancelled" | "canceled" => {
            "closed".to_string()
        }
        _ => "open".to_string(),
    }
}

/// 子会话链接的 5 条记录。
fn child_link_records(
    parent: &Session,
    tree: &Tree<'_>,
    link: &ChildLink,
    created_at: Option<&Timestamp>,
) -> Vec<Value> {
    let child = tree.nodes[link.child_index];
    let edge = edge_for(parent, child);
    let call_id = edge
        .and_then(|edge| edge.source_call_id.clone())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| format!("call_{}", token_urlsafe_24()));
    let prompt = edge.map(|edge| edge.prompt.clone()).unwrap_or_default();
    let agent_type = edge
        .and_then(|edge| edge.agent_type.clone())
        .filter(|value| !value.is_empty())
        .or_else(|| child.agent_type.clone());
    let mut arguments = Map::new();
    arguments.insert("message".into(), Value::from(prompt));
    if let Some(agent_type) = agent_type.filter(|value| !value.is_empty()) {
        arguments.insert("agent_type".into(), Value::from(agent_type));
    }
    let now = timestamp(created_at);
    let status = edge_status(edge);
    let call_status = if status == "closed" {
        "completed"
    } else {
        "in_progress"
    };

    let mut spawn = Map::new();
    spawn.insert("type".into(), Value::from("function_call"));
    spawn.insert("id".into(), Value::from(format!("fc_{}", token_hex(25))));
    spawn.insert("name".into(), Value::from("spawn_agent"));
    spawn.insert(
        "arguments".into(),
        Value::from(python_json_dumps(&Value::Object(arguments))),
    );
    spawn.insert("call_id".into(), Value::from(call_id.as_str()));
    // response_item 使用 Responses API 的状态枚举；SQLite 的 thread_spawn_edges
    // 用 open/closed，二者不能混写。
    spawn.insert("status".into(), Value::from(call_status));

    let mut activity = Map::new();
    activity.insert("type".into(), Value::from("sub_agent_activity"));
    activity.insert(
        "agent_thread_id".into(),
        Value::from(link.child_id.as_str()),
    );
    activity.insert("agent_path".into(), Value::from(link.agent_path.as_str()));
    activity.insert(
        "kind".into(),
        Value::from(if status == "closed" {
            "completed"
        } else {
            "working"
        }),
    );

    let result_text = assistant_result(child);
    let mut result = Map::new();
    result.insert("type".into(), Value::from("function_call_output"));
    result.insert("call_id".into(), Value::from(call_id.as_str()));
    let mut task_name = Map::new();
    task_name.insert("task_name".into(), Value::from(link.agent_path.as_str()));
    result.insert(
        "output".into(),
        Value::from(python_json_dumps(&Value::Object(task_name))),
    );

    let mut agent_message = Map::new();
    agent_message.insert("type".into(), Value::from("agent_message"));
    agent_message.insert("id".into(), Value::from(format!("amsg_{}", token_hex(12))));
    agent_message.insert("author".into(), Value::from(link.agent_path.as_str()));
    agent_message.insert("recipient".into(), Value::from("/root"));
    let mut content = Map::new();
    content.insert("type".into(), Value::from("input_text"));
    content.insert("text".into(), Value::from(result_text.as_str()));
    agent_message.insert("content".into(), Value::Array(vec![Value::Object(content)]));

    let mut event_message = Map::new();
    event_message.insert("type".into(), Value::from("agent_message"));
    event_message.insert("message".into(), Value::from(result_text.as_str()));

    vec![
        envelope(&now, "response_item", spawn),
        envelope(&now, "event_msg", activity),
        envelope(&now, "response_item", result),
        envelope(&now, "event_msg", event_message),
        envelope(&now, "response_item", agent_message),
    ]
}

fn envelope(timestamp: &str, kind: &str, payload: Map<String, Value>) -> Value {
    let mut record = Map::new();
    record.insert("timestamp".into(), Value::from(timestamp));
    record.insert("type".into(), Value::from(kind));
    record.insert("payload".into(), Value::Object(payload));
    Value::Object(record)
}

fn destination(sessions_dir: &Path, sid: &str, ordinal: usize) -> PathBuf {
    let clock = now();
    let local = clock.epoch_seconds + local_offset_seconds(clock.epoch_seconds);
    let (year, month, day, hour, minute, second) = format_parts(local);
    let suffix = if ordinal > 0 {
        format!("-{ordinal}")
    } else {
        String::new()
    };
    sessions_dir
        .join(format!("{year:04}"))
        .join(format!("{month:02}"))
        .join(format!("{day:02}"))
        .join(format!(
            "rollout-{year:04}-{month:02}-{day:02}T{hour:02}-{minute:02}-{second:02}{suffix}-{sid}.jsonl"
        ))
}

// ---------------------------------------------------------------------------
// 树遍历与落盘
// ---------------------------------------------------------------------------

/// 会话树的扁平视图：`nodes` 是前序序列，`children` 记录每个节点的孩子下标。
struct Tree<'a> {
    nodes: Vec<&'a Session>,
    children: Vec<Vec<usize>>,
}

fn flatten<'a>(session: &'a Session, tree: &mut Tree<'a>) -> usize {
    let index = tree.nodes.len();
    tree.nodes.push(session);
    tree.children.push(Vec::new());
    for child in &session.children {
        let child_index = flatten(child, tree);
        tree.children[index].push(child_index);
    }
    index
}

/// 写出整棵 rollout 树，返回根会话的 `(session_id, 文件路径)`。
pub fn write(
    session: &Session,
    cwd: Option<&str>,
    sessions_dir: Option<&Path>,
    state_db: Option<&Path>,
    decider: Option<ToolDecider<'_>>,
) -> DomainResult<(String, PathBuf)> {
    let templates = templates();
    let mut tree = Tree {
        nodes: Vec::new(),
        children: Vec::new(),
    };
    flatten(session, &mut tree);

    let root_id = uuid7();
    let base_cwd = cwd
        .map(str::to_string)
        .unwrap_or_else(|| session.cwd.clone());
    let output_root = sessions_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home_dir().join(".codex").join("sessions"));
    let ids: Vec<String> = (0..tree.nodes.len())
        .map(|index| if index == 0 { root_id.clone() } else { uuid7() })
        .collect();

    // agent_path 与边状态：同一父下重名的 agent_path 追加 -2/-3 后缀。
    let mut agent_paths: Vec<String> = vec![String::new(); tree.nodes.len()];
    let mut edge_statuses: Vec<Option<String>> = vec![None; tree.nodes.len()];
    agent_paths[0] = tree.nodes[0]
        .agent_path
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/root".to_string());
    assign_tree_fields(&tree, 0, &mut agent_paths, &mut edge_statuses);

    let mut paths: Vec<PathBuf> = vec![PathBuf::new(); tree.nodes.len()];
    let mut parents: Vec<Option<String>> = vec![None; tree.nodes.len()];
    let mut working_dirs: Vec<String> = vec![String::new(); tree.nodes.len()];
    let mut pending: Vec<PathBuf> = Vec::new();
    let mut losses: Vec<crate::events::Event> = Vec::new();

    let emit_result = emit(
        &templates,
        &tree,
        0,
        None,
        0,
        Some(&agent_paths[0].clone()),
        0,
        &EmitContext {
            cwd,
            base_cwd: &base_cwd,
            output_root: &output_root,
            ids: &ids,
            agent_paths: &agent_paths,
            root_id: &root_id,
        },
        &mut paths,
        &mut parents,
        &mut working_dirs,
        &mut pending,
        &mut losses,
        decider,
    )
    .and_then(|()| {
        let registry = state_db
            .map(Path::to_path_buf)
            .unwrap_or_else(|| registry_path(&output_root));
        let nodes: Vec<RegistryNode<'_>> = (0..tree.nodes.len())
            .map(|index| RegistryNode {
                session: tree.nodes[index],
                session_id: ids[index].clone(),
                path: paths[index].clone(),
                parent_id: parents[index].clone(),
                cwd: working_dirs[index].clone(),
                agent_path: agent_paths[index].clone(),
                status: edge_statuses[index].clone(),
            })
            .collect();
        let cli_version = templates
            .get("session_meta")
            .and_then(|record| record.get("payload"))
            .and_then(|payload| payload.get("cli_version"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        register_tree(&registry, &nodes, &cli_version)
    });

    if let Err(error) = emit_result {
        for path in &pending {
            let _ = fs::remove_file(path);
        }
        return Err(error);
    }
    Ok((root_id, paths[0].clone()))
}

fn registry_path(output_root: &Path) -> PathBuf {
    output_root
        .parent()
        .map(|parent| parent.join("state_5.sqlite"))
        .unwrap_or_else(|| PathBuf::from("state_5.sqlite"))
}

fn assign_tree_fields(
    tree: &Tree<'_>,
    index: usize,
    agent_paths: &mut Vec<String>,
    edge_statuses: &mut Vec<Option<String>>,
) {
    let parent_path = agent_paths[index].clone();
    let mut used: Vec<String> = Vec::new();
    for (child_index, child) in tree.children[index].iter().enumerate() {
        let child_session = tree.nodes[*child];
        let base = child_session
            .agent_path
            .clone()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| {
                let leaf = child_session
                    .agent_id
                    .clone()
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| (child_index + 1).to_string());
                format!("{parent_path}/{leaf}")
            });
        let mut path = base.clone();
        let mut suffix = 2;
        while used.contains(&path) {
            path = format!("{base}-{suffix}");
            suffix += 1;
        }
        used.push(path.clone());
        edge_statuses[*child] = Some(edge_status(edge_for(tree.nodes[index], child_session)));
        agent_paths[*child] = path;
        assign_tree_fields(tree, *child, agent_paths, edge_statuses);
    }
}

struct EmitContext<'a> {
    cwd: Option<&'a str>,
    base_cwd: &'a str,
    output_root: &'a Path,
    ids: &'a [String],
    agent_paths: &'a [String],
    root_id: &'a str,
}

#[allow(clippy::too_many_arguments)]
fn emit(
    templates: &Map<String, Value>,
    tree: &Tree<'_>,
    index: usize,
    parent: Option<usize>,
    depth: i64,
    agent_path: Option<&str>,
    ordinal: usize,
    context: &EmitContext<'_>,
    paths: &mut [PathBuf],
    parents: &mut [Option<String>],
    working_dirs: &mut [String],
    pending: &mut Vec<PathBuf>,
    losses: &mut Vec<crate::events::Event>,
    decider: Option<ToolDecider<'_>>,
) -> DomainResult<()> {
    let node = tree.nodes[index];
    let sid = context.ids[index].clone();
    let node_cwd = context
        .cwd
        .map(str::to_string)
        .filter(|value| !value.is_empty())
        .or_else(|| Some(node.cwd.clone()).filter(|value| !value.is_empty()))
        .unwrap_or_else(|| context.base_cwd.to_string());

    let mut child_links: HashMap<String, Vec<ChildLink>> = HashMap::new();
    for child_index in &tree.children[index] {
        let child = tree.nodes[*child_index];
        let edge = edge_for(node, child);
        let key = edge
            .and_then(|edge| edge.spawn_message_id.clone())
            .filter(|id| !id.is_empty())
            .unwrap_or_default();
        child_links.entry(key).or_default().push(ChildLink {
            child_index: *child_index,
            child_id: context.ids[*child_index].clone(),
            agent_path: context.agent_paths[*child_index].clone(),
        });
    }

    let records = session_records(
        templates,
        node,
        losses,
        &node_cwd,
        &sid,
        context.root_id,
        parent.map(|parent| context.ids[parent].as_str()),
        depth,
        agent_path,
        &mut child_links,
        tree,
        decider,
    )?;

    let dest = destination(context.output_root, &sid, ordinal);
    if let Some(parent_dir) = dest.parent() {
        fs::create_dir_all(parent_dir)
            .map_err(|error| DomainError::internal(format!("创建 Codex 会话目录失败: {error}")))?;
    }
    let temporary = dest.with_extension("tmp");
    pending.push(temporary.clone());
    let payload: String = records
        .iter()
        .map(python_json_dumps)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&temporary, payload)
        .map_err(|error| DomainError::internal(format!("写入 Codex 会话失败: {error}")))?;
    fs::rename(&temporary, &dest)
        .map_err(|error| DomainError::internal(format!("发布 Codex 会话失败: {error}")))?;
    pending.retain(|path| path != &temporary);
    pending.push(dest.clone());
    paths[index] = dest;
    parents[index] = parent.map(|parent| context.ids[parent].clone());
    working_dirs[index] = node_cwd;

    let mut next_ordinal = ordinal + 1;
    for child_index in &tree.children[index] {
        emit(
            templates,
            tree,
            *child_index,
            Some(index),
            depth + 1,
            Some(&context.agent_paths[*child_index].clone()),
            next_ordinal,
            context,
            paths,
            parents,
            working_dirs,
            pending,
            losses,
            decider,
        )?;
        next_ordinal += subtree_size(tree, *child_index);
    }
    Ok(())
}

fn subtree_size(tree: &Tree<'_>, index: usize) -> usize {
    1 + tree.children[index]
        .iter()
        .map(|child| subtree_size(tree, *child))
        .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{text_tool_result, Block, Message, ToolResultStatus};
    use rusqlite::Connection;
    use serde_json::json;

    fn state_db(path: &Path) {
        Connection::open(path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL,
                 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, source TEXT NOT NULL,
                 model_provider TEXT NOT NULL, cwd TEXT NOT NULL, title TEXT NOT NULL,
                 sandbox_policy TEXT NOT NULL, approval_mode TEXT NOT NULL,
                 agent_path TEXT, thread_source TEXT);
                 CREATE TABLE thread_spawn_edges (parent_thread_id TEXT NOT NULL,
                 child_thread_id TEXT NOT NULL PRIMARY KEY, status TEXT NOT NULL);",
            )
            .unwrap();
    }

    fn shell_session() -> Session {
        let mut session = Session::new("codex", "src", "/work");
        let mut user = Message::new("user");
        user.blocks.push(Block::text("run it"));
        user.source_id = Some("m1".into());
        session.messages.push(user);
        let mut assistant = Message::new("assistant");
        let mut tool = ToolCall::new(
            "Bash",
            Some(CanonicalOp::SHELL_EXEC.to_string()),
            json!({"command": "echo hi", "workdir": "/work"}),
        );
        tool.result = Some(text_tool_result("hi\n", ToolResultStatus::Success));
        let mut block = Block::new(BlockKind::Tool);
        block.tool = Some(tool);
        assistant.blocks.push(block);
        assistant.blocks.push(Block::text("done"));
        session.messages.push(assistant);
        session
    }

    fn read_records(path: &Path) -> Vec<Value> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn uuid7_carries_the_version_and_variant_bits() {
        let value = uuid7();
        assert_eq!(value.len(), 36);
        assert_eq!(&value[14..15], "7");
        assert!(["8", "9", "a", "b"].contains(&&value[19..20]));
        assert_ne!(uuid7(), value);
    }

    #[test]
    fn canonical_timestamps_survive_and_epochs_are_formatted() {
        assert_eq!(
            timestamp(Some(&Timestamp::Text("2024-01-01T00:00:00Z".into()))),
            "2024-01-01T00:00:00Z"
        );
        assert_eq!(
            timestamp(Some(&Timestamp::Millis(1_700_000_000_000))),
            "2023-11-14T22:13:20.000Z"
        );
        // 10 位以内按秒处理。
        assert_eq!(
            timestamp(Some(&Timestamp::Millis(1_700_000_000))),
            "2023-11-14T22:13:20.000Z"
        );
        assert!(timestamp(None).ends_with('Z'));
    }

    #[test]
    fn shell_calls_render_as_a_custom_tool_pair() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let db = temp.path().join("state_5.sqlite");
        state_db(&db);
        let session = shell_session();
        let (root_id, path) =
            write(&session, Some("/work"), Some(&sessions), Some(&db), None).unwrap();
        assert_eq!(root_id.len(), 36);
        let records = read_records(&path);
        assert_eq!(records[0]["type"], json!("session_meta"));
        assert_eq!(records[0]["payload"]["id"], json!(root_id));
        assert_eq!(records[1]["payload"]["role"], json!("user"));
        assert_eq!(records[2]["payload"]["type"], json!("custom_tool_call"));
        let input = records[2]["payload"]["input"].as_str().unwrap();
        assert!(input.starts_with("const r = await tools.exec_command({\"cmd\": \"echo hi\""));
        assert_eq!(
            records[3]["payload"]["type"],
            json!("custom_tool_call_output")
        );
        let output: Vec<Value> =
            serde_json::from_str(records[3]["payload"]["output"].as_str().unwrap()).unwrap();
        let inner: Value = serde_json::from_str(output[1]["text"].as_str().unwrap()).unwrap();
        assert_eq!(inner["output"], json!("hi\n"));
        assert_eq!(inner["status"], json!("success"));
        assert_eq!(records[4]["payload"]["content"][0]["text"], json!("done"));
    }

    #[test]
    fn fs_write_renders_an_add_file_patch() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let db = temp.path().join("state_5.sqlite");
        state_db(&db);
        let mut session = Session::new("codex", "src", "/work");
        let mut assistant = Message::new("assistant");
        let mut tool = ToolCall::new(
            "Write",
            Some(CanonicalOp::FS_WRITE.to_string()),
            json!({"file_path": "/a.txt", "content": "one\ntwo"}),
        );
        tool.result = Some(text_tool_result("", ToolResultStatus::Success));
        let mut block = Block::new(BlockKind::Tool);
        block.tool = Some(tool);
        assistant.blocks.push(block);
        session.messages.push(assistant);
        let (_id, path) = write(&session, None, Some(&sessions), Some(&db), None).unwrap();
        let records = read_records(&path);
        let input = records[1]["payload"]["input"].as_str().unwrap();
        assert!(
            input.contains("*** Begin Patch\\n*** Add File: /a.txt\\n+one\\n+two\\n*** End Patch")
        );
        assert!(input.starts_with("const patch = "));
    }

    #[test]
    fn unsupported_tools_degrade_to_narration() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let db = temp.path().join("state_5.sqlite");
        state_db(&db);
        let mut session = Session::new("codex", "src", "/work");
        let mut assistant = Message::new("assistant");
        let mut tool = ToolCall::new(
            "WebFetch",
            Some(CanonicalOp::WEB_FETCH.to_string()),
            json!({"url": "https://example.com"}),
        );
        tool.result = Some(text_tool_result("body", ToolResultStatus::Success));
        let mut block = Block::new(BlockKind::Tool);
        block.tool = Some(tool);
        assistant.blocks.push(block);
        session.messages.push(assistant);
        let (_id, path) = write(&session, None, Some(&sessions), Some(&db), None).unwrap();
        let records = read_records(&path);
        let text = records[1]["payload"]["content"][0]["text"]
            .as_str()
            .unwrap();
        assert!(text.starts_with("[History: tool WebFetch was previously invoked]"));
    }

    #[test]
    fn subagent_trees_write_five_link_records_and_register_edges() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let db = temp.path().join("state_5.sqlite");
        state_db(&db);
        let mut root = shell_session();
        let mut child = Session::new("codex", "child", "/work");
        let mut reply = Message::new("assistant");
        reply.blocks.push(Block::text("child answer"));
        child.messages.push(reply);
        child.agent_path = Some("/root/docs".into());
        let mut edge = AgentEdge::new("src", "child");
        edge.spawn_message_id = Some("m1".into());
        edge.source_call_id = Some("call-x".into());
        edge.prompt = "do docs".into();
        edge.status = Some("completed".into());
        root.agent_edges.push(edge);
        root.children.push(child);

        let (_id, path) = write(&root, None, Some(&sessions), Some(&db), None).unwrap();
        let records = read_records(&path);
        let kinds: Vec<&str> = records
            .iter()
            .map(|record| record["type"].as_str().unwrap())
            .collect();
        // 用户消息之后紧跟 5 条链接记录。
        assert_eq!(
            &kinds[2..7],
            [
                "response_item",
                "event_msg",
                "response_item",
                "event_msg",
                "response_item"
            ]
        );
        assert_eq!(records[2]["payload"]["name"], json!("spawn_agent"));
        assert_eq!(records[2]["payload"]["status"], json!("completed"));
        assert_eq!(records[2]["payload"]["call_id"], json!("call-x"));
        assert_eq!(records[3]["payload"]["kind"], json!("completed"));
        assert_eq!(records[6]["payload"]["type"], json!("agent_message"));
        assert_eq!(
            records[6]["payload"]["content"][0]["text"],
            json!("child answer")
        );

        let connection = Connection::open(&db).unwrap();
        let status: String = connection
            .query_row("SELECT status FROM thread_spawn_edges", [], |row| {
                row.get(0)
            })
            .unwrap();
        // SQLite 边用 open/closed，与 response_item 的 completed/in_progress 分离。
        assert_eq!(status, "closed");
        let threads: i64 = connection
            .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(threads, 2);
    }

    #[test]
    fn registry_failures_unlink_every_published_file() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let session = shell_session();
        // 注册库不存在 → register_tree 报错 → 已发布文件全部回滚。
        let error = write(
            &session,
            None,
            Some(&sessions),
            Some(&temp.path().join("missing.sqlite")),
            None,
        )
        .unwrap_err();
        assert!(error.message().contains("Codex 注册库不存在"));
        let leftovers: Vec<PathBuf> = walkdir::WalkDir::new(&sessions)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.into_path())
            .collect();
        assert!(leftovers.is_empty(), "残留文件: {leftovers:?}");
    }

    #[test]
    fn op_fidelity_marks_transformed_reads_as_degrade() {
        assert_eq!(op_fidelity(CanonicalOp::SHELL_EXEC), ToolVerdict::Native);
        assert_eq!(op_fidelity(CanonicalOp::FS_WRITE), ToolVerdict::Native);
        assert_eq!(op_fidelity(CanonicalOp::FS_PATCH), ToolVerdict::Native);
        assert_eq!(op_fidelity(CanonicalOp::AGENT_SPAWN), ToolVerdict::Native);
        assert_eq!(op_fidelity(CanonicalOp::FS_READ), ToolVerdict::Degrade);
        assert_eq!(op_fidelity(CanonicalOp::FS_SEARCH), ToolVerdict::Degrade);
        assert_eq!(op_fidelity(CanonicalOp::FS_GLOB), ToolVerdict::Degrade);
        assert_eq!(op_fidelity(CanonicalOp::TOOL_INVOKE), ToolVerdict::Degrade);
        assert_eq!(op_fidelity("nope"), ToolVerdict::Degrade);
    }

    #[test]
    fn non_ascii_arguments_are_escaped_like_python() {
        assert_eq!(
            json_dumps_ascii(&json!({"a": "中"})),
            "{\"a\": \"\\u4e2d\"}"
        );
        assert_eq!(json_dumps_ascii(&json!("x")), "\"x\"");
    }
}
