//! 把 canonical 会话写成当前 Pi v3 JSONL。
//!
//! 写入前有三道验收，缺一不可：先写 `.tmp` → reader 复读 → **用真实 pi RPC
//! 加载验证**（`probe::probe_path`）→ 再复读 → `os.replace` 到正式文件名。
//! 探针不过就删掉临时文件并报错，绝不把半成品留在 pi 的会话目录里。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use crate::adapters::shared::dialect::python_str;
use crate::adapters::shared::migration::{RenderDecision, ToolVerdict};
use crate::adapters::shared::narration::narrate;
use crate::adapters::shared::writing::python_json_dumps;
use crate::errors::{DomainError, DomainResult};
use crate::model::{tool_result_text, BlockKind, Message, Session, ToolCall, ToolResultStatus};
use crate::system::paths::{home_dir, pi_session_roots, process_environ};
use crate::tool_ops::CanonicalOp;

use super::dialect::DIALECT;

/// plan / preview / writer 三路共用的调用级判定入口（`MigrationTargetBase::evaluate_tool`）。
pub type ToolDecider<'a> =
    &'a (dyn Fn(&ToolCall, &Session, Option<&Message>) -> DomainResult<RenderDecision> + 'a);

/// 规范操作在 pi 端的保真度。
///
/// 注意 `tool.invoke` 是 **native**：pi 的原生 `toolCall` 可以承载任意
/// `name` + `arguments`，外部私有调用照样写得进去（其他家多半只能降级）。
pub fn op_fidelity(op: &str) -> ToolVerdict {
    match op {
        CanonicalOp::TOOL_INVOKE => ToolVerdict::Native,
        CanonicalOp::FS_PATCH
        | CanonicalOp::WEB_FETCH
        | CanonicalOp::WEB_SEARCH
        | CanonicalOp::AGENT_SPAWN => ToolVerdict::Degrade,
        // `binding_for` 只索引可写绑定，等价 Python 的 `DIALECT.write_ops()`。
        other if DIALECT.binding_for(other).is_some() => ToolVerdict::Native,
        _ => ToolVerdict::Degrade,
    }
}

// ---------------------------------------------------------------------------
// 时间戳与 id（Python 用 time.strftime + uuid4）
// ---------------------------------------------------------------------------

fn now() -> (i64, u32) {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| (elapsed.as_secs() as i64, elapsed.subsec_millis()))
        .unwrap_or((0, 0))
}

/// `int(time.time() * 1000)`。
pub(super) fn now_millis() -> i64 {
    let (seconds, millis) = now();
    seconds * 1000 + i64::from(millis)
}

/// 民用日期（Howard Hinnant `civil_from_days`）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = (if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    }) as u32;
    (year + i64::from(month <= 2), month, day)
}

fn utc_fields(seconds: i64) -> (i64, u32, u32, i64, i64, i64) {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    (year, month, day, rest / 3600, (rest % 3600) / 60, rest % 60)
}

/// `time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime())`。
pub(super) fn iso_stamp() -> String {
    let (year, month, day, hour, minute, second) = utc_fields(now().0);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.000Z")
}

/// `time.strftime("%Y-%m-%dT%H-%M-%S", time.gmtime())`（文件名用，冒号不合法）。
fn filename_stamp() -> String {
    let (year, month, day, hour, minute, second) = utc_fields(now().0);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}-{minute:02}-{second:02}")
}

fn uuid4_bytes() -> [u8; 16] {
    let mut bytes: [u8; 16] = rand::random();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    bytes
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// `uuid.uuid4().hex[:size]`。
pub(super) fn uuid4_hex(size: usize) -> String {
    hex(&uuid4_bytes()).chars().take(size).collect()
}

/// `str(uuid.uuid4())`。
fn uuid4_hyphenated() -> String {
    let raw = hex(&uuid4_bytes());
    format!(
        "{}-{}-{}-{}-{}",
        &raw[0..8],
        &raw[8..12],
        &raw[12..16],
        &raw[16..20],
        &raw[20..32]
    )
}

// ---------------------------------------------------------------------------
// 记录构建
// ---------------------------------------------------------------------------

/// assistant 终态字段里的 usage 模板（editor.validate 会逐项检查）。
pub(super) fn zero_usage() -> Value {
    json!({
        "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0,
        "totalTokens": 0,
        "cost": {"input": 0, "output": 0, "cacheRead": 0,
                 "cacheWrite": 0, "total": 0},
    })
}

/// 无 decider 时的原生渲染；`None` 表示 pi 端没有这个调用的原生形态。
fn native_input(tool: &ToolCall) -> Option<(String, Value)> {
    if tool.op.as_deref() == Some(CanonicalOp::TOOL_INVOKE) {
        // Python 用 `value["name"]` / `value["input"]`，缺键是 KeyError → 降级。
        let entries = tool.input.as_object()?;
        let name = entries.get("name")?;
        let input = entries.get("input")?;
        return Some((python_str(name), input.clone()));
    }
    let op = tool.op.as_deref()?;
    let (name, native) = DIALECT.render(op, &tool.input)?;
    Some((name.to_string(), Value::Object(native)))
}

/// 返回 `(name, arguments)`；`None` 表示该调用降级为叙述文本。
fn tool_native(
    tool: &ToolCall,
    session: &Session,
    message: &Message,
    decider: Option<ToolDecider>,
) -> DomainResult<Option<(String, Value)>> {
    let Some(decider) = decider else {
        return Ok(native_input(tool));
    };
    let decision = decider(tool, session, Some(message))?;
    let Some(rendered) = decision.rendered else {
        return Ok(None);
    };
    let name = match rendered.get("name") {
        Some(value) if super::tool_calls::truthy(value) => python_str(value),
        _ => tool.name.clone(),
    };
    let input = rendered
        .get("input")
        .cloned()
        .unwrap_or_else(|| tool.input.clone());
    Ok(Some((name, input)))
}

/// 构造整份 v3 记录流（header + 线性 message 链）。
pub fn records(
    session: &Session,
    cwd: &str,
    sid: &str,
    parent_session: Option<&str>,
    decider: Option<ToolDecider>,
) -> DomainResult<Vec<Value>> {
    let mut header = Map::new();
    header.insert("type".into(), Value::from("session"));
    header.insert("version".into(), Value::from(3));
    header.insert("id".into(), Value::from(sid));
    header.insert("timestamp".into(), Value::from(iso_stamp()));
    header.insert("cwd".into(), Value::from(cwd));
    if let Some(parent) = parent_session.filter(|value| !value.is_empty()) {
        header.insert("parentSession".into(), Value::from(parent));
    }
    let mut out: Vec<Value> = vec![Value::Object(header)];
    let mut parent = Value::Null;

    for message in &session.messages {
        let mut content: Vec<Value> = Vec::new();
        let mut tools: Vec<(&ToolCall, String)> = Vec::new();
        for block in &message.blocks {
            match (block.kind, block.tool.as_ref(), block.image.as_ref()) {
                (BlockKind::Text, _, _) => {
                    content.push(json!({"type": "text", "text": block.text}));
                }
                (BlockKind::Thinking, _, _) if message.role == "assistant" => {
                    content.push(json!({"type": "thinking", "thinking": block.text}));
                }
                (BlockKind::Image, _, Some(image)) => {
                    content.push(json!({"type": "image", "data": image.data,
                                        "mimeType": image.mime_type}));
                }
                (BlockKind::Tool, Some(tool), _) => {
                    let Some((name, arguments)) = tool_native(tool, session, message, decider)?
                    else {
                        content.push(json!({"type": "text", "text": narrate(tool)}));
                        continue;
                    };
                    let call_id = match tool.source_call_id.as_deref() {
                        Some(value) if !value.is_empty() => value.to_string(),
                        _ => format!("call_{}", uuid4_hex(16)),
                    };
                    content.push(json!({"type": "toolCall", "id": call_id,
                                        "name": name, "arguments": arguments}));
                    tools.push((tool, call_id));
                }
                _ => {}
            }
        }

        let entry_id = uuid4_hex(12);
        let mut native = Map::new();
        native.insert("role".into(), Value::from(message.role.as_str()));
        native.insert("content".into(), Value::Array(content));
        native.insert("timestamp".into(), Value::from(now_millis()));
        if message.role == "assistant" {
            native.insert("api".into(), Value::from("ferry"));
            native.insert(
                "provider".into(),
                Value::from(
                    session
                        .model_provider
                        .as_deref()
                        .filter(|value| !value.is_empty())
                        .unwrap_or("ferry"),
                ),
            );
            native.insert(
                "model".into(),
                Value::from(
                    session
                        .model
                        .as_deref()
                        .filter(|value| !value.is_empty())
                        .unwrap_or("migrated"),
                ),
            );
            native.insert("usage".into(), zero_usage());
            native.insert(
                "stopReason".into(),
                Value::from(if tools.is_empty() { "stop" } else { "toolUse" }),
            );
        }
        out.push(
            json!({"type": "message", "id": entry_id, "parentId": parent,
                        "timestamp": iso_stamp(), "message": Value::Object(native)}),
        );
        parent = Value::from(entry_id);

        for (tool, call_id) in tools {
            let result_id = uuid4_hex(12);
            let is_error = tool
                .result
                .as_ref()
                .is_some_and(|result| result.status == ToolResultStatus::Error);
            out.push(json!({
                "type": "message", "id": result_id, "parentId": parent,
                "timestamp": iso_stamp(),
                "message": {
                    "role": "toolResult", "toolCallId": call_id,
                    "toolName": tool.name,
                    "content": [{"type": "text",
                                 "text": tool_result_text(tool.result.as_ref())}],
                    "isError": is_error,
                    "timestamp": now_millis(),
                },
            }));
            parent = Value::from(result_id);
        }
    }
    Ok(out)
}

/// 迁移写入：返回 `{"session_id", "dest"}`（跨包约定的两个必备键）。
pub fn write(
    session: &Session,
    cwd: &str,
    root: Option<&Path>,
    decider: Option<ToolDecider>,
) -> DomainResult<Map<String, Value>> {
    let root: PathBuf = match root {
        Some(path) => path.to_path_buf(),
        None => pi_session_roots(&process_environ(), &home_dir())
            .into_iter()
            .next()
            .ok_or_else(|| DomainError::internal("Pi 没有可用的会话根目录"))?,
    };
    fs::create_dir_all(&root)
        .map_err(|error| DomainError::internal(format!("创建 Pi 会话目录失败: {error}")))?;
    let (session_id, path) = publish(session, cwd, None, &root, decider)?;
    let mut result = Map::new();
    result.insert("session_id".into(), Value::from(session_id));
    result.insert("dest".into(), Value::from(path.to_string_lossy().as_ref()));
    Ok(result)
}

/// 单个节点的写入 + 子会话递归；`parentSession` 存父会话的**文件路径**。
fn publish(
    node: &Session,
    node_cwd: &str,
    parent_session: Option<&str>,
    root: &Path,
    decider: Option<ToolDecider>,
) -> DomainResult<(String, PathBuf)> {
    let sid = uuid4_hyphenated();
    let path = root.join(format!("{}_{sid}.jsonl", filename_stamp()));
    let temporary = root.join(format!(".{sid}.{}.tmp", std::process::id()));
    let rows = records(node, node_cwd, &sid, parent_session, decider)?;
    let mut payload = String::new();
    for row in &rows {
        payload.push_str(&python_json_dumps(row));
        payload.push('\n');
    }
    fs::write(&temporary, payload.as_bytes())
        .map_err(|error| DomainError::internal(format!("写入 Pi 临时会话失败: {error}")))?;

    let temporary_ref = temporary.to_string_lossy().into_owned();
    super::reader::read(&temporary_ref)?;
    let report = super::probe::probe_path(&temporary_ref, Some(node_cwd))?;
    if report.status != "passed" {
        let _ = fs::remove_file(&temporary);
        return Err(DomainError::internal("Pi RPC 无法加载生成会话"));
    }
    super::reader::read(&temporary_ref)?;
    fs::rename(&temporary, &path)
        .map_err(|error| DomainError::internal(format!("落盘 Pi 会话失败: {error}")))?;

    let parent_path = path.to_string_lossy().into_owned();
    for child in &node.children {
        let child_cwd = if child.cwd.is_empty() {
            node_cwd
        } else {
            child.cwd.as_str()
        };
        publish(child, child_cwd, Some(&parent_path), root, decider)?;
    }
    Ok((sid, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::pi::migration::PiMigrationTarget;
    use crate::adapters::shared::migration::MigrationTargetBase;
    use crate::model::{text_tool_result, Block};

    fn contents(rows: &[Value]) -> Vec<String> {
        rows.iter()
            .filter_map(|row| row.get("message"))
            .filter_map(|message| message.get("content"))
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(|part| part.get("type").and_then(Value::as_str))
            .map(str::to_string)
            .collect()
    }

    fn tool_message(tool: ToolCall) -> Session {
        let mut block = Block::new(BlockKind::Tool);
        block.tool = Some(tool);
        let mut message = Message::new("assistant");
        message.blocks = vec![block];
        let mut session = Session::new("fixture", "root", "/tmp");
        session.messages = vec![message];
        session
    }

    #[test]
    fn tool_invoke_is_native_unlike_the_other_targets() {
        assert_eq!(op_fidelity(CanonicalOp::TOOL_INVOKE), ToolVerdict::Native);
        assert_eq!(op_fidelity(CanonicalOp::SHELL_EXEC), ToolVerdict::Native);
        assert_eq!(op_fidelity(CanonicalOp::FS_EDIT), ToolVerdict::Native);
        assert_eq!(op_fidelity(CanonicalOp::FS_PATCH), ToolVerdict::Degrade);
        assert_eq!(op_fidelity(CanonicalOp::WEB_FETCH), ToolVerdict::Degrade);
        assert_eq!(op_fidelity(CanonicalOp::WEB_SEARCH), ToolVerdict::Degrade);
        assert_eq!(op_fidelity(CanonicalOp::AGENT_SPAWN), ToolVerdict::Degrade);
        assert_eq!(op_fidelity("nope"), ToolVerdict::Degrade);
    }

    #[test]
    fn narrates_tool_calls_the_target_cannot_render() {
        // 外部 namespace 的 TOOL_INVOKE 在 pi 端没有原生形态，必须走叙述降级。
        let mut foreign = ToolCall::new(
            "native_lookup",
            Some(CanonicalOp::TOOL_INVOKE.to_string()),
            json!({"namespace": "codex", "name": "native_lookup",
                   "input": {"query": "x"}}),
        );
        foreign.result = Some(text_tool_result("output", ToolResultStatus::Success));
        let session = tool_message(foreign.clone());
        let target = PiMigrationTarget;
        assert!(target
            .evaluate_tool(&foreign, &session, session.messages.first())
            .unwrap()
            .rendered
            .is_none());

        let decider = |tool: &ToolCall, node: &Session, message: Option<&Message>| {
            target.evaluate_tool(tool, node, message)
        };
        let rows = records(&session, "/tmp", "sid", None, Some(&decider)).unwrap();
        assert_eq!(contents(&rows), ["text"]);
    }

    #[test]
    fn native_tool_calls_emit_a_paired_tool_result_entry() {
        let mut call = ToolCall::new(
            "read",
            Some(CanonicalOp::FS_READ.to_string()),
            json!({"file_path": "/raw/input.txt"}),
        );
        call.source_call_id = Some("call-fixed".into());
        call.result = Some(text_tool_result("raw output", ToolResultStatus::Success));
        let session = tool_message(call);
        let rows = records(&session, "/tmp", "sid", None, None).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(contents(&rows), ["toolCall", "text"]);
        let assistant = &rows[1]["message"];
        assert_eq!(assistant["stopReason"], json!("toolUse"));
        assert_eq!(assistant["api"], json!("ferry"));
        assert_eq!(assistant["usage"]["cost"]["total"], json!(0));
        assert_eq!(assistant["content"][0]["id"], json!("call-fixed"));
        assert_eq!(
            assistant["content"][0]["arguments"],
            json!({"path": "/raw/input.txt"})
        );
        let result = &rows[2]["message"];
        assert_eq!(result["role"], json!("toolResult"));
        assert_eq!(result["toolCallId"], json!("call-fixed"));
        assert_eq!(result["isError"], json!(false));
        assert_eq!(result["content"][0]["text"], json!("raw output"));
        // parentId 链：header → assistant → toolResult。
        assert_eq!(rows[1]["parentId"], Value::Null);
        assert_eq!(rows[2]["parentId"], rows[1]["id"]);
    }

    #[test]
    fn header_carries_the_parent_session_path_only_when_given() {
        let session = Session::new("fixture", "root", "/tmp");
        let rows = records(&session, "/work", "sid", None, None).unwrap();
        assert!(rows[0].get("parentSession").is_none());
        assert_eq!(rows[0]["version"], json!(3));
        assert_eq!(rows[0]["cwd"], json!("/work"));
        let rows = records(&session, "/work", "sid", Some("/root/a.jsonl"), None).unwrap();
        assert_eq!(rows[0]["parentSession"], json!("/root/a.jsonl"));
    }

    #[test]
    fn generated_ids_have_the_python_shapes() {
        assert_eq!(uuid4_hex(12).len(), 12);
        let uuid = uuid4_hyphenated();
        assert_eq!(uuid.len(), 36);
        assert_eq!(uuid.chars().nth(14), Some('4'));
        let stamp = iso_stamp();
        assert_eq!(stamp.len(), 24);
        assert!(stamp.ends_with(".000Z"));
        assert_eq!(filename_stamp().len(), 19);
    }

    #[test]
    fn utc_fields_match_known_epochs() {
        assert_eq!(utc_fields(0), (1970, 1, 1, 0, 0, 0));
        // 2026-07-25T00:00:00Z（黄金基线钉住的 mtime）。
        assert_eq!(utc_fields(1_784_937_600), (2026, 7, 25, 0, 0, 0));
    }
}
