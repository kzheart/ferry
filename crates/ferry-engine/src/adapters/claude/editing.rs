//! Claude Code 会话文件原语：解析、快照、原子写入与结构校验。
//!
//! 语义事实源：`engine/adapters/claude/editing.py`。
//!
//! 轮次/编辑语义统一由 [`super::codec`] 持有；跨工具编排由 `operations` 负责。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::TryRngCore as _;
use serde_json::{Map, Value};

use crate::adapters::shared::editing::write_jsonl;
use crate::adapters::shared::scanner::iter_lines;
use crate::errors::{DomainError, DomainResult};
use crate::system::paths::home_dir;
use crate::system::snapshots::snapshot_file;

/// 引用 → 会话文件路径：先当路径试，再按 `~/.claude/projects/*/<ref>.jsonl` 检索。
pub fn resolve(reference: &str) -> DomainResult<PathBuf> {
    let direct = Path::new(reference);
    if direct.exists() {
        return Ok(direct.to_path_buf());
    }
    let pattern = home_dir().join(format!(".claude/projects/*/{reference}.jsonl"));
    let hit = glob::glob(&pattern.to_string_lossy())
        .ok()
        .and_then(|mut paths| paths.find_map(Result::ok));
    hit.ok_or_else(|| DomainError::session_not_found("claude", reference))
}

/// 单测辅助：HOME 是进程级状态，改写它的用例必须串行（lib 测试默认多线程）。
///
/// 互斥锁在 [`crate::system::paths::testing`]，是 **crate 级**的一把——别的模块
/// 只要读 `home_dir()` 就会被这里的改写影响，各自造锁挡不住。
#[cfg(test)]
pub(crate) mod testing {
    use std::path::Path;

    pub(crate) use crate::system::paths::testing::EnvGuard as HomeGuard;

    /// 在作用域内把 HOME 指向沙箱，析构时恢复原值并释放锁。
    pub(crate) fn home_guard(home: &Path) -> HomeGuard {
        HomeGuard::home(home)
    }
}

/// 编辑前快照。
pub fn backup(
    path: &Path,
    reason_code: &str,
    tool: &str,
    extra: Option<&Map<String, Value>>,
) -> DomainResult<PathBuf> {
    snapshot_file(path, reason_code, tool, extra)
        .map_err(|error| DomainError::internal(format!("claude 快照写入失败: {error}")))
}

/// 读入全部非空行。
pub fn load(path: &Path) -> DomainResult<Vec<Value>> {
    let lines = iter_lines(path)
        .map_err(|error| DomainError::internal(format!("读取 claude 会话失败: {error}")))?;
    let mut records = Vec::new();
    for line in lines {
        let line =
            line.map_err(|error| DomainError::internal(format!("读取 claude 会话失败: {error}")))?;
        if line.trim().is_empty() {
            continue;
        }
        records.push(
            serde_json::from_str(&line).map_err(|error| {
                DomainError::internal(format!("claude 会话行解析失败: {error}"))
            })?,
        );
    }
    Ok(records)
}

/// 原子落盘（就地编辑版：不建父目录、不 fsync）。
pub fn save(path: &Path, records: &[Value]) -> DomainResult<()> {
    write_jsonl(path, records)
        .map_err(|error| DomainError::internal(format!("写入 claude 会话失败: {error}")))
}

// ---------------------------------------------------------------------------
// 标识符与时间戳（codec / writer 共用；Python 侧分别来自 uuid / secrets / time）
// ---------------------------------------------------------------------------

/// `uuid.uuid4().hex`：32 位小写十六进制，含版本位与变体位。
pub fn uuid4_hex() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .expect("系统 CSPRNG 不可用");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    hex_lower(&bytes)
}

/// 小写十六进制编码（避免 `format!` 逐字节分配）。
pub fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// `str(uuid.uuid4())`：带连字符的 8-4-4-4-12 形态。
pub fn uuid4() -> String {
    let hex = uuid4_hex();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// `secrets.token_urlsafe(nbytes)`：CSPRNG + URL-safe base64 无填充。
pub fn token_urlsafe(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::rngs::OsRng
        .try_fill_bytes(&mut buffer)
        .expect("系统 CSPRNG 不可用");
    URL_SAFE_NO_PAD.encode(buffer)
}

/// 纪元秒 → `%Y-%m-%dT%H:%M:%S`（UTC）。
pub fn utc_iso_seconds(epoch_seconds: i64) -> String {
    let (days, seconds) = (
        epoch_seconds.div_euclid(86_400),
        epoch_seconds.rem_euclid(86_400),
    );
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

/// `datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")`。
///
/// Python 在微秒恰为 0 时省略小数部分；这里恒输出 6 位，概率上等价。
pub fn utc_iso_now_micros() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "{}.{:06}Z",
        utc_iso_seconds(now.as_secs() as i64),
        now.subsec_micros()
    )
}

/// 当前纪元秒（浮点，对齐 `time.time()`）。
pub fn epoch_seconds_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64())
        .unwrap_or_default()
}

/// Howard Hinnant `civil_from_days` 的逆运算，纪元天数 → 民用日期。
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
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

/// 取一条记录的 uuid 文本形态；缺失/非字符串一律视为「没有 uuid」。
fn uuid_of(record: &Value) -> Option<&str> {
    record.get("uuid").and_then(Value::as_str)
}

fn parent_of(record: &Value) -> Option<&str> {
    record.get("parentUuid").and_then(Value::as_str)
}

/// 删除消息后重连 parentUuid 链：指向被删节点的，改指其最近存活祖先。
pub fn relink(records: &mut [Value], removed_uuids: &BTreeSet<String>) {
    let parents: BTreeMap<String, Option<String>> = records
        .iter()
        .filter_map(|record| {
            uuid_of(record).map(|uuid| {
                (
                    uuid.to_string(),
                    parent_of(record).map(std::string::ToString::to_string),
                )
            })
        })
        .collect();
    let nearest_alive = |start: &str| -> Option<String> {
        let mut cursor = Some(start.to_string());
        // parents 是有限映射，链上出现环时 `removed_uuids` 之外的节点会终止循环；
        // 这里额外记访问集，防止被删节点自成环导致死循环。
        let mut seen: BTreeSet<String> = BTreeSet::new();
        while let Some(current) = cursor {
            if !removed_uuids.contains(&current) || !seen.insert(current.clone()) {
                return Some(current);
            }
            cursor = parents.get(&current).cloned().flatten();
        }
        None
    };
    for record in records.iter_mut() {
        let Some(parent) = parent_of(record).map(std::string::ToString::to_string) else {
            continue;
        };
        if !removed_uuids.contains(&parent) {
            continue;
        }
        let replacement = nearest_alive(&parent);
        if let Some(entries) = record.as_object_mut() {
            entries.insert(
                "parentUuid".into(),
                replacement.map_or(Value::Null, Value::from),
            );
        }
    }
}

/// uuid 唯一、parentUuid 不悬空、tool_use 与 tool_result 双向完全配对。
pub fn check_invariants(records: &[Value]) -> DomainResult<()> {
    let uuids: Vec<&str> = records.iter().filter_map(uuid_of).collect();
    let unique: BTreeSet<&str> = uuids.iter().copied().collect();
    if unique.len() != uuids.len() {
        return Err(DomainError::internal("uuid 重复"));
    }
    for record in records {
        if let Some(parent) = parent_of(record) {
            if !unique.contains(parent) {
                return Err(DomainError::internal(format!("parentUuid 悬空: {parent}")));
            }
        }
    }
    let mut uses: BTreeSet<String> = BTreeSet::new();
    let mut results: BTreeSet<String> = BTreeSet::new();
    for record in records {
        let Some(content) = record
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for block in content {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                        uses.insert(id.to_string());
                    }
                }
                Some("tool_result") => {
                    if let Some(id) = block.get("tool_use_id").and_then(Value::as_str) {
                        results.insert(id.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    let orphans: Vec<&String> = results.difference(&uses).collect();
    if !orphans.is_empty() {
        return Err(DomainError::internal(format!(
            "孤儿 tool_result: {}",
            render_set(&orphans)
        )));
    }
    let unpaired: Vec<&String> = uses.difference(&results).collect();
    if !unpaired.is_empty() {
        return Err(DomainError::internal(format!(
            "未配对 tool_use: {}",
            render_set(&unpaired)
        )));
    }
    Ok(())
}

/// Python 的 `set` 字面量形态（`{'a', 'b'}`），文案与断言消息一致。
fn render_set(values: &[&String]) -> String {
    let parts: Vec<String> = values.iter().map(|value| format!("'{value}'")).collect();
    format!("{{{}}}", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn relink_points_at_the_nearest_surviving_ancestor() {
        let mut records = vec![
            json!({"uuid": "a", "parentUuid": null}),
            json!({"uuid": "b", "parentUuid": "a"}),
            json!({"uuid": "c", "parentUuid": "b"}),
            json!({"uuid": "d", "parentUuid": "c"}),
        ];
        relink(&mut records, &set(&["b", "c"]));
        assert_eq!(records[3]["parentUuid"], json!("a"));
        // 未被删的链路不动。
        assert_eq!(records[1]["parentUuid"], json!("a"));
    }

    #[test]
    fn relink_nulls_out_when_every_ancestor_is_gone() {
        let mut records = vec![
            json!({"uuid": "a", "parentUuid": null}),
            json!({"uuid": "b", "parentUuid": "a"}),
        ];
        relink(&mut records, &set(&["a"]));
        assert_eq!(records[1]["parentUuid"], json!(null));
    }

    #[test]
    fn invariants_reject_duplicates_dangling_parents_and_unpaired_tools() {
        assert!(check_invariants(&[json!({"uuid": "a"}), json!({"uuid": "a"})]).is_err());
        assert!(check_invariants(&[json!({"uuid": "a", "parentUuid": "zz"})]).is_err());

        let orphan = vec![json!({
            "uuid": "a",
            "message": {"content": [{"type": "tool_result", "tool_use_id": "t1"}]}
        })];
        let error = check_invariants(&orphan).unwrap_err();
        assert!(error.message().starts_with("孤儿 tool_result"));

        let unpaired = vec![json!({
            "uuid": "a",
            "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Bash"}]}
        })];
        assert!(check_invariants(&unpaired)
            .unwrap_err()
            .message()
            .starts_with("未配对 tool_use"));

        let paired = vec![
            json!({"uuid": "a", "message": {"content": [{"type": "tool_use", "id": "t1"}]}}),
            json!({"uuid": "b", "parentUuid": "a",
                   "message": {"content": [{"type": "tool_result", "tool_use_id": "t1"}]}}),
        ];
        assert!(check_invariants(&paired).is_ok());
    }

    #[test]
    fn load_and_save_round_trip_through_the_edit_variant() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("s.jsonl");
        std::fs::write(&path, "{\"a\": 1}\n\n{\"b\": \"中\"}\n").unwrap();
        let records = load(&path).unwrap();
        assert_eq!(records, vec![json!({"a": 1}), json!({"b": "中"})]);
        save(&path, &records).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"a\": 1}\n{\"b\": \"中\"}\n"
        );
    }

    #[test]
    fn identifiers_have_the_python_shapes() {
        let hex = uuid4_hex();
        assert_eq!(hex.len(), 32);
        assert_eq!(&hex[12..13], "4");
        assert!("89ab".contains(&hex[16..17]));
        let text = uuid4();
        assert_eq!(text.len(), 36);
        assert_eq!(
            text.match_indices('-')
                .map(|(index, _)| index)
                .collect::<Vec<_>>(),
            [8, 13, 18, 23]
        );
        // token_urlsafe(18) 恰好 24 个字符，Python 的 `[:24]` 是恒等切片。
        assert_eq!(token_urlsafe(18).len(), 24);
        assert_ne!(token_urlsafe(18), token_urlsafe(18));
    }

    #[test]
    fn utc_formatting_matches_strftime() {
        assert_eq!(utc_iso_seconds(0), "1970-01-01T00:00:00");
        assert_eq!(utc_iso_seconds(1_784_937_600), "2026-07-25T00:00:00");
        assert_eq!(utc_iso_seconds(1_718_454_896), "2024-06-15T12:34:56");
        let now = utc_iso_now_micros();
        assert!(now.ends_with('Z') && now.len() == 27, "{now}");
    }

    #[test]
    fn resolve_reports_session_not_found_for_unknown_ids() {
        let error = resolve("ferry-not-a-real-claude-session").unwrap_err();
        assert_eq!(error.code, "session.not_found");
    }
}
