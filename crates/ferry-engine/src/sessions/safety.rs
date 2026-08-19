//! 会话查询与操作返回值的输入边界和体积限制。
//!
//! 两条容易踩空的口径必须记牢：
//! - `truncate_text` 按**字符**计数（Python `len(str)`）；
//! - `_take`（agent_read）按 **UTF-8 字节**计数。
//!
//! DTO 体积判定用的是 `json.dumps(..., ensure_ascii=False)` 的字节数，
//! Python 的默认分隔符带空格（`", "` / `": "`），与 canonical_json 的紧凑
//! 分隔符不同，因此这里另起一套 [`python_json`]。

use std::fmt::Write as _;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde_json::{Map, Value};

use crate::errors::{DomainError, DomainResult};
use crate::jsonutil::sha256_hex;

/// Agent DTO 的硬上限：64 KiB。
pub const MAX_AGENT_DTO_BYTES: usize = 64 * 1024;

/// 当前 UTC 毫秒（`int(datetime.now(timezone.utc).timestamp() * 1000)`）。
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

/// 按**字符**截断；第二个返回值表示是否发生截断。
pub fn truncate_text(value: &str, limit: usize) -> (String, bool) {
    let mut characters = value.chars();
    let head: String = characters.by_ref().take(limit).collect();
    if characters.next().is_none() {
        (value.to_string(), false)
    } else {
        (head, true)
    }
}

/// 与 Python `json.dumps(value, ensure_ascii=False[, sort_keys=True])` 逐字节
/// 一致的序列化：分隔符是 `", "` 与 `": "`，非 ASCII 原样输出。
///
/// DTO 预算判定只关心字节数，但「只关心字节数」正是必须逐字节一致的理由。
pub fn python_json(value: &Value, sort_keys: bool) -> String {
    let mut out = String::new();
    write_python_json(&mut out, value, sort_keys);
    out
}

/// DTO 预算判定的字节数。
pub fn python_json_len(value: &Value) -> usize {
    python_json(value, false).len()
}

fn write_python_json(out: &mut String, value: &Value, sort_keys: bool) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::String(text) => write_python_string(out, text),
        Value::Array(items) => {
            out.push('[');
            for (position, item) in items.iter().enumerate() {
                if position > 0 {
                    out.push_str(", ");
                }
                write_python_json(out, item, sort_keys);
            }
            out.push(']');
        }
        Value::Object(entries) => {
            let mut keys: Vec<&String> = entries.keys().collect();
            if sort_keys {
                keys.sort_unstable();
            }
            out.push('{');
            for (position, key) in keys.into_iter().enumerate() {
                if position > 0 {
                    out.push_str(", ");
                }
                write_python_string(out, key);
                out.push_str(": ");
                write_python_json(out, &entries[key], sort_keys);
            }
            out.push('}');
        }
    }
}

/// `ensure_ascii=False` 的转义表：只转义 `"`、`\` 与 C0 控制字符。
fn write_python_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if control < ' ' => {
                let _ = write!(out, "\\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// `record_session_id`：扫描行的 id 优先，回落到 canonical session 的 source_id。
///
/// Python 是 `str(row.get("id") or source_id or "")`：只要 `id` 是**真值**就采纳，
/// 不要求它已经是字符串（契约上 `id` 必是 str，但脏行不该悄悄换成另一个会话的 id）。
pub fn record_session_id(row: &Map<String, Value>, source_id: Option<&str>) -> String {
    let from_row = match row.get("id") {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        // 数字 id 在 Python 里是真值，`str()` 后照常采纳；`0` 是假值。
        Some(Value::Number(number)) if number.as_f64() != Some(0.0) => Some(number.to_string()),
        // 其余形态（null/bool/容器/空串/0）在 Python 里都是假值或不该出现，
        // 一律回落 source_id。
        _ => None,
    };
    let value = from_row
        .or_else(|| {
            source_id
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_default();
    truncate_text(&value, 512).0
}

const TIME_FORMAT_HINT: &str =
    "epoch milliseconds, ISO8601, now, or a relative offset like now-7d / -7d (units s/m/h/d/w)";

static RELATIVE_TIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^\s*(?:now\s*)?(?:(?P<sign>[-+])\s*(?P<amount>\d{1,6})\s*(?P<unit>[smhdw]))?\s*$",
    )
    .expect("相对时间正则必须可编译")
});

fn unit_ms(unit: char) -> i64 {
    match unit.to_ascii_lowercase() {
        's' => 1_000,
        'm' => 60_000,
        'h' => 3_600_000,
        'd' => 86_400_000,
        _ => 604_800_000,
    }
}

/// 支持 `now` / `now-7d` / `-7d` 这类写法；不匹配返回 `None`。
fn relative_timestamp(text: &str) -> Option<i64> {
    let captures = RELATIVE_TIME.captures(text)?;
    let amount = captures.name("amount");
    if !text.trim().to_lowercase().starts_with("now") && amount.is_none() {
        return None;
    }
    let now = now_ms();
    let Some(amount) = amount else {
        return Some(now);
    };
    let magnitude: i64 = amount.as_str().parse().ok()?;
    let unit = captures.name("unit")?.as_str().chars().next()?;
    let delta = magnitude * unit_ms(unit);
    let negative = captures
        .name("sign")
        .map(|sign| sign.as_str() == "-")
        .unwrap_or(false);
    Some(if negative { now - delta } else { now + delta })
}

fn hint_error(message: &str, received: Option<&str>) -> DomainError {
    let mut params = Map::new();
    params.insert("accepts".into(), Value::from(TIME_FORMAT_HINT));
    if let Some(received) = received {
        params.insert(
            "received".into(),
            Value::from(truncate_text(received, 64).0),
        );
    }
    DomainError::new(
        "agent.request_invalid",
        "AgentRequestError",
        message,
        params,
    )
}

/// 时间入参归一成 epoch 毫秒。
pub fn timestamp(value: &Value) -> DomainResult<Option<i64>> {
    match value {
        Value::Null => Ok(None),
        Value::Bool(_) => Err(hint_error("时间格式无效", None)),
        Value::Number(number) => {
            let Some(float) = number.as_f64() else {
                return Ok(number.as_i64());
            };
            if !float.is_finite() {
                return Err(hint_error("时间必须是有限数值", None));
            }
            // Python `int(value)` 向零截断；整数直接取原值避免 f64 精度损失。
            Ok(Some(number.as_i64().unwrap_or(float.trunc() as i64)))
        }
        Value::String(text) => {
            if let Some(relative) = relative_timestamp(text) {
                return Ok(Some(relative));
            }
            parse_iso8601_ms(text)
                .map(Some)
                .ok_or_else(|| hint_error("时间格式无效", Some(text)))
        }
        // Python 走 `str(value)` 再解析，list/dict 的字符串化必然解析失败。
        other => Err(hint_error("时间格式无效", Some(&other.to_string()))),
    }
}

/// `datetime.fromisoformat(text.replace("Z", "+00:00"))` 的等价实现。
///
/// 覆盖 Python 3.11+ 接受的扩展格式与基本格式：日期 `YYYY-MM-DD` / `YYYYMMDD`，
/// 可选的任意单字符日期时间分隔符，时间 `HH[:MM[:SS[.f{1,6}]]]`（含无冒号形式），
/// 时区 `Z` / `±HH[:MM[:SS]]`。naive 值按 UTC 处理。
pub fn parse_iso8601_ms(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.len() < 8 {
        return None;
    }
    let (date_part, rest) = split_date(text)?;
    let (year, month, day) = date_part;
    let mut millis = civil_days(year, month, day)? * 86_400_000;
    if rest.is_empty() {
        return Some(millis);
    }
    // 第一个字符是日期/时间分隔符，Python 接受任意单字符。
    let mut time_text = &rest[rest.chars().next()?.len_utf8()..];
    let mut offset_ms = 0i64;
    if let Some(position) = time_text
        .rfind(['+', '-'])
        .filter(|position| *position > 0)
        .or_else(|| {
            time_text
                .find(['Z', 'z'])
                .filter(|position| *position + 1 == time_text.len())
        })
    {
        let (head, tail) = time_text.split_at(position);
        offset_ms = parse_offset(tail)?;
        time_text = head;
    }
    millis += parse_time_ms(time_text)?;
    Some(millis - offset_ms)
}

#[allow(clippy::type_complexity)]
fn split_date(text: &str) -> Option<((i64, u32, u32), &str)> {
    let bytes = text.as_bytes();
    if bytes.len() >= 10 && bytes[4] == b'-' && bytes[7] == b'-' {
        let year = text[0..4].parse().ok()?;
        let month = text[5..7].parse().ok()?;
        let day = text[8..10].parse().ok()?;
        return Some(((year, month, day), &text[10..]));
    }
    if bytes.len() >= 8 && bytes[0..8].iter().all(u8::is_ascii_digit) {
        let year = text[0..4].parse().ok()?;
        let month = text[4..6].parse().ok()?;
        let day = text[6..8].parse().ok()?;
        return Some(((year, month, day), &text[8..]));
    }
    None
}

fn parse_offset(text: &str) -> Option<i64> {
    if matches!(text, "Z" | "z") {
        return Some(0);
    }
    let sign = match text.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    Some(sign * parse_time_ms(&text[1..])?)
}

/// `HH`、`HH:MM`、`HH:MM:SS[.f{1,6}]` 与对应的无冒号形式。
fn parse_time_ms(text: &str) -> Option<i64> {
    if text.is_empty() {
        return Some(0);
    }
    let (main, fraction) = match text.split_once('.') {
        Some((main, fraction)) => (main, Some(fraction)),
        None => (text, None),
    };
    let digits: Vec<&str> = if main.contains(':') {
        main.split(':').collect()
    } else {
        if !main.as_bytes().iter().all(u8::is_ascii_digit) || main.len() % 2 != 0 {
            return None;
        }
        (0..main.len() / 2)
            .map(|group| &main[group * 2..group * 2 + 2])
            .collect()
    };
    if digits.is_empty() || digits.len() > 3 {
        return None;
    }
    let mut total = 0i64;
    for (position, chunk) in digits.iter().enumerate() {
        let value: i64 = chunk.parse().ok()?;
        total += value * [3_600_000i64, 60_000, 1_000][position];
    }
    if let Some(fraction) = fraction {
        if fraction.is_empty()
            || fraction.len() > 6
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        // 只取毫秒精度：更细的微秒在 epoch 毫秒里本就落不下。
        let mut padded = fraction.to_string();
        while padded.len() < 3 {
            padded.push('0');
        }
        total += padded[..3].parse::<i64>().ok()?;
    }
    Some(total)
}

/// 民用日期 → 自 1970-01-01 起的天数（Howard Hinnant `days_from_civil`）。
fn civil_days(year: i64, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

fn field_error(message: impl Into<String>, params: Map<String, Value>) -> DomainError {
    DomainError::new(
        "agent.request_invalid",
        "AgentRequestError",
        message,
        params,
    )
}

fn one_field(name: &str) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("field".into(), Value::from(name));
    params
}

/// 整数入参的范围校验；`None` 取默认值。布尔被显式拒绝。
pub fn bounded_int(
    value: Option<&Value>,
    default: i64,
    minimum: i64,
    maximum: i64,
    name: &str,
) -> DomainResult<i64> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(default);
    };
    let number = match value {
        Value::Number(number) if number.is_i64() || number.is_u64() => number
            .as_i64()
            .ok_or_else(|| field_error(format!("{name} 必须是整数"), one_field(name)))?,
        _ => return Err(field_error(format!("{name} 必须是整数"), one_field(name))),
    };
    if number < minimum || number > maximum {
        let mut params = one_field(name);
        params.insert("minimum".into(), Value::from(minimum));
        params.insert("maximum".into(), Value::from(maximum));
        return Err(field_error(format!("{name} 超出范围"), params));
    }
    Ok(number)
}

/// 字符串数组入参 → 去重集合。返回值保留原始出现顺序，方便复刻 Python
/// `sorted(...)` 之外的用法（Python `set` 本身无序，调用方一律显式排序）。
pub fn string_set(
    value: Option<&Value>,
    name: &str,
    maximum: usize,
    item_size: usize,
) -> DomainResult<Vec<String>> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(field_error(
            format!("{name} 必须是字符串数组"),
            one_field(name),
        ));
    };
    if !items.iter().all(Value::is_string) {
        return Err(field_error(
            format!("{name} 必须是字符串数组"),
            one_field(name),
        ));
    }
    if items.len() > maximum {
        let mut params = one_field(name);
        params.insert("maximum".into(), Value::from(maximum));
        return Err(field_error(format!("{name} 数量超出范围"), params));
    }
    let texts: Vec<&str> = items.iter().filter_map(Value::as_str).collect();
    if texts
        .iter()
        .any(|item| item.is_empty() || item.chars().count() > item_size)
    {
        let mut params = one_field(name);
        params.insert("maximum".into(), Value::from(item_size));
        return Err(field_error(format!("{name} 项长度超出范围"), params));
    }
    let mut unique: Vec<String> = Vec::new();
    for item in texts {
        if !unique.iter().any(|seen| seen == item) {
            unique.push(item.to_string());
        }
    }
    Ok(unique)
}

/// JSON 结构的深度/节点数/键长校验。
pub fn validate_json_shape(value: &Value, max_depth: usize, max_nodes: usize) -> DomainResult<()> {
    fn visit(
        item: &Value,
        depth: usize,
        nodes: &mut usize,
        limits: (usize, usize),
    ) -> DomainResult<()> {
        *nodes += 1;
        if *nodes > limits.1 || depth > limits.0 {
            return Err(field_error("JSON 结构过深或项目过多", Map::new()));
        }
        match item {
            Value::Number(number) => {
                if number
                    .as_f64()
                    .map(|float| !float.is_finite())
                    .unwrap_or(false)
                {
                    return Err(field_error("JSON 不允许 NaN/Infinity", Map::new()));
                }
            }
            Value::Object(entries) => {
                if entries.keys().any(|key| key.chars().count() > 128) {
                    return Err(field_error(
                        "JSON key 必须是不超过 128 字符的字符串",
                        Map::new(),
                    ));
                }
                for child in entries.values() {
                    visit(child, depth + 1, nodes, limits)?;
                }
            }
            Value::Array(items) => {
                for child in items {
                    visit(child, depth + 1, nodes, limits)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut nodes = 0;
    visit(value, 0, &mut nodes, (max_depth, max_nodes))
}

/// Agent 编辑操作的白名单校验。
pub fn validate_agent_edit_ops(ops: Option<&Value>) -> DomainResult<()> {
    let items = ops
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty() && items.len() <= 50)
        .ok_or_else(|| field_error("ops 必须是 1 到 50 项的数组", Map::new()))?;
    validate_json_shape(ops.expect("上一步已校验存在"), 8, 2000)?;
    let mut rewrite_locators: Vec<&str> = Vec::new();
    for operation in items {
        let Some(entry) = operation.as_object() else {
            return Err(field_error("每个 edit op 必须是 object", Map::new()));
        };
        match entry.get("op").and_then(Value::as_str) {
            Some("delete-turn") => {
                let keys_ok =
                    entry.len() == 2 && entry.contains_key("op") && entry.contains_key("turn");
                let turn_ok = entry
                    .get("turn")
                    .and_then(|value| match value {
                        Value::Number(number) if number.is_i64() || number.is_u64() => {
                            number.as_i64()
                        }
                        _ => None,
                    })
                    .map(|turn| turn >= 1)
                    .unwrap_or(false);
                if !keys_ok || !turn_ok {
                    return Err(field_error("delete-turn 参数非法", Map::new()));
                }
            }
            Some("rewrite") => {
                let keys_ok = entry.len() == 3
                    && entry.contains_key("op")
                    && entry.contains_key("locator")
                    && entry.contains_key("text");
                if !keys_ok {
                    return Err(field_error("rewrite 参数非法", Map::new()));
                }
                let locator = entry.get("locator").and_then(Value::as_str);
                let text = entry.get("text").and_then(Value::as_str);
                let ok = matches!(locator, Some(value) if (1..=512).contains(&value.chars().count()))
                    && matches!(text, Some(value) if (1..=20_000).contains(&value.chars().count()));
                if !ok {
                    return Err(field_error("rewrite locator/text 超出范围", Map::new()));
                }
                rewrite_locators.push(locator.expect("上一步已校验"));
            }
            _ => {
                return Err(field_error(
                    "Agent edit 仅允许 delete-turn/rewrite",
                    Map::new(),
                ))
            }
        }
    }
    let mut unique = rewrite_locators.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != rewrite_locators.len() {
        return Err(field_error(
            "同一消息不能在一次编辑中重复改写",
            one_field("ops.locator"),
        ));
    }
    Ok(())
}

/// `time_range` 校验：只允许 `from`/`to`，且 from ≤ to。
pub fn validated_interval(value: Option<&Value>) -> DomainResult<(Option<i64>, Option<i64>)> {
    // Python 的 `value or {}`：falsy（None/{}/0/""/[]/False）一律当空区间。
    let interval = match value {
        None => Map::new(),
        Some(value) if is_falsy(value) => Map::new(),
        Some(Value::Object(entries)) => entries.clone(),
        Some(_) => return Err(field_error("time_range 必须且只能包含 from/to", Map::new())),
    };
    if interval.keys().any(|key| key != "from" && key != "to") {
        return Err(field_error("time_range 必须且只能包含 from/to", Map::new()));
    }
    let start = timestamp(interval.get("from").unwrap_or(&Value::Null))?;
    let end = timestamp(interval.get("to").unwrap_or(&Value::Null))?;
    if let (Some(start), Some(end)) = (start, end) {
        if start > end {
            return Err(field_error("time_range.from 不得晚于 to", Map::new()));
        }
    }
    Ok((start, end))
}

fn is_falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(flag) => !flag,
        Value::Number(number) => number.as_f64().map(|float| float == 0.0).unwrap_or(false),
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(entries) => entries.is_empty(),
    }
}

struct StructureBudget {
    nodes: i64,
}

fn bounded_structure(value: &Value, depth: usize, budget: &mut StructureBudget) -> (Value, bool) {
    if budget.nodes <= 0 || depth > 8 {
        return (Value::Null, true);
    }
    budget.nodes -= 1;
    match value {
        Value::Array(items) => {
            let mut result = Vec::new();
            let mut truncated = items.len() > 200;
            for item in items.iter().take(200) {
                let (child, child_truncated) = bounded_structure(item, depth + 1, budget);
                result.push(child);
                truncated = truncated || child_truncated;
            }
            (Value::Array(result), truncated)
        }
        Value::Object(entries) => {
            let mut result = Map::new();
            let mut truncated = entries.len() > 200;
            for (key, item) in entries.iter().take(200) {
                let (child, child_truncated) = bounded_structure(item, depth + 1, budget);
                result.insert(key.clone(), child);
                truncated = truncated || child_truncated;
            }
            (Value::Object(result), truncated)
        }
        scalar => (scalar.clone(), false),
    }
}

/// 结构与字节双限的 JSON 裁剪。
pub fn bounded_json(value: &Value, max_bytes: usize) -> Value {
    let mut budget = StructureBudget { nodes: 2000 };
    let (mut bounded, structurally_truncated) = bounded_structure(value, 0, &mut budget);
    if structurally_truncated {
        let mut wrapper = Map::new();
        wrapper.insert("truncated".into(), Value::Bool(true));
        wrapper.insert("value".into(), bounded);
        bounded = Value::Object(wrapper);
    }
    let encoded = python_json(&bounded, true);
    if encoded.len() <= max_bytes {
        return bounded;
    }
    let preview_limit = 4000.min(max_bytes.saturating_sub(256));
    let mut boundary = preview_limit.min(encoded.len());
    while boundary > 0 && !encoded.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let mut payload = Map::new();
    payload.insert("truncated".into(), Value::Bool(true));
    payload.insert("sha256".into(), Value::from(sha256_hex(encoded.as_bytes())));
    payload.insert("preview".into(), Value::from(&encoded[..boundary]));
    Value::Object(payload)
}

/// `bounded_json` 的默认预算（32 KiB）。
pub const DEFAULT_BOUNDED_JSON_BYTES: usize = 32 * 1024;

/// DTO 出口的最后一道闸：超过 64 KiB 直接拒绝。
pub fn finalize_dto(result: Map<String, Value>) -> DomainResult<Map<String, Value>> {
    if python_json_len(&Value::Object(result.clone())) > MAX_AGENT_DTO_BYTES {
        return Err(field_error("Agent DTO 超过 64 KiB", Map::new()));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn truncate_text_counts_characters_not_bytes() {
        assert_eq!(truncate_text("中文测试", 2), ("中文".to_string(), true));
        assert_eq!(truncate_text("中文", 10), ("中文".to_string(), false));
        assert_eq!(truncate_text("", 0), (String::new(), false));
    }

    /// 期望值来自 Python：`json.dumps(value, ensure_ascii=False)`。
    #[test]
    fn python_json_matches_the_default_separators() {
        assert_eq!(
            python_json(&json!({"a": 1, "b": [1, 2]}), false),
            r#"{"a": 1, "b": [1, 2]}"#
        );
        assert_eq!(python_json(&json!({"中": "文"}), false), r#"{"中": "文"}"#);
        assert_eq!(
            python_json(&json!({"b": 1, "a": 2}), true),
            r#"{"a": 2, "b": 1}"#
        );
        assert_eq!(python_json(&json!([]), false), "[]");
    }

    /// 期望值来自 Python：
    /// `int(datetime.fromisoformat(t.replace("Z","+00:00")).timestamp()*1000)`。
    #[test]
    fn iso8601_matches_python_fromisoformat() {
        assert_eq!(parse_iso8601_ms("1970-01-01"), Some(0));
        assert_eq!(
            parse_iso8601_ms("2024-01-01T00:00:00+00:00"),
            Some(1704067200000)
        );
        assert_eq!(parse_iso8601_ms("2024-01-01T00:00:00"), Some(1704067200000));
        assert_eq!(
            parse_iso8601_ms("2024-06-15T12:34:56.789+00:00"),
            Some(1718454896789)
        );
        assert_eq!(parse_iso8601_ms("2024-06-15 12:34:56"), Some(1718454896000));
        assert_eq!(
            parse_iso8601_ms("2024-06-15T12:34:56+08:00"),
            Some(1718426096000)
        );
        assert_eq!(parse_iso8601_ms("20240615T123456"), Some(1718454896000));
        assert_eq!(
            parse_iso8601_ms("2024-06-15T12:34:56-05:00"),
            Some(1718472896000)
        );
        assert_eq!(parse_iso8601_ms("not-a-date"), None);
    }

    #[test]
    fn timestamp_handles_relative_and_absolute_forms() {
        assert_eq!(timestamp(&Value::Null).unwrap(), None);
        assert!(timestamp(&json!(true)).is_err());
        assert_eq!(
            timestamp(&json!(1700000000000i64)).unwrap(),
            Some(1700000000000)
        );
        assert_eq!(timestamp(&json!(1.9)).unwrap(), Some(1));
        let now = now_ms();
        let week = timestamp(&json!("now-7d")).unwrap().unwrap();
        assert!((now - 604_800_000 - week).abs() < 5_000);
        let plain = timestamp(&json!("-7d")).unwrap().unwrap();
        assert!((now - 604_800_000 - plain).abs() < 5_000);
        let ahead = timestamp(&json!("now+1h")).unwrap().unwrap();
        assert!((now + 3_600_000 - ahead).abs() < 5_000);
        let bare = timestamp(&json!("now")).unwrap().unwrap();
        assert!((now - bare).abs() < 5_000);
        // "nowish" 之类既不是相对写法也不是 ISO：报错并带回显。
        let error = timestamp(&json!("nope")).unwrap_err();
        assert_eq!(error.code, "agent.request_invalid");
        assert_eq!(error.params()["received"], Value::from("nope"));
        assert_eq!(error.params()["accepts"], Value::from(TIME_FORMAT_HINT));
    }

    #[test]
    fn bounded_int_rejects_booleans_and_out_of_range() {
        assert_eq!(bounded_int(None, 20, 1, 50, "limit").unwrap(), 20);
        assert_eq!(bounded_int(Some(&json!(5)), 20, 1, 50, "limit").unwrap(), 5);
        assert!(bounded_int(Some(&json!(true)), 20, 1, 50, "limit").is_err());
        assert!(bounded_int(Some(&json!(1.5)), 20, 1, 50, "limit").is_err());
        let error = bounded_int(Some(&json!(99)), 20, 1, 50, "limit").unwrap_err();
        assert_eq!(error.message(), "limit 超出范围");
        assert_eq!(error.params()["maximum"], Value::from(50));
    }

    #[test]
    fn string_set_enforces_count_and_item_length() {
        assert!(string_set(None, "agents", 8, 32).unwrap().is_empty());
        assert_eq!(
            string_set(Some(&json!(["a", "b", "a"])), "agents", 8, 32).unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(string_set(Some(&json!(["a", 1])), "agents", 8, 32).is_err());
        assert!(string_set(Some(&json!([""])), "agents", 8, 32).is_err());
        assert!(string_set(Some(&json!(["a", "b"])), "agents", 1, 32).is_err());
    }

    #[test]
    fn validated_interval_rejects_unknown_keys_and_inverted_ranges() {
        assert_eq!(validated_interval(None).unwrap(), (None, None));
        assert_eq!(
            validated_interval(Some(&json!({"from": 1, "to": 2}))).unwrap(),
            (Some(1), Some(2))
        );
        assert!(validated_interval(Some(&json!({"from": 2, "to": 1}))).is_err());
        assert!(validated_interval(Some(&json!({"since": 1}))).is_err());
    }

    #[test]
    fn edit_ops_only_allow_delete_turn_and_rewrite() {
        assert!(validate_agent_edit_ops(Some(&json!([{"op": "delete-turn", "turn": 1}]))).is_ok());
        assert!(validate_agent_edit_ops(Some(&json!([{"op": "delete-turn", "turn": 0}]))).is_err());
        assert!(
            validate_agent_edit_ops(Some(&json!([{"op": "delete-turn", "turn": true}]))).is_err()
        );
        assert!(validate_agent_edit_ops(Some(&json!([
            {"op": "rewrite", "locator": "a", "text": "x"}
        ])))
        .is_ok());
        // 同一 locator 重复改写。
        assert!(validate_agent_edit_ops(Some(&json!([
            {"op": "rewrite", "locator": "a", "text": "x"},
            {"op": "rewrite", "locator": "a", "text": "y"}
        ])))
        .is_err());
        assert!(validate_agent_edit_ops(Some(&json!([{"op": "nope"}]))).is_err());
        assert!(validate_agent_edit_ops(Some(&json!([]))).is_err());
    }

    #[test]
    fn bounded_json_falls_back_to_a_hashed_preview() {
        let small = bounded_json(&json!({"a": 1}), 1024);
        assert_eq!(small, json!({"a": 1}));
        let wide = Value::Array((0..300).map(Value::from).collect());
        let clipped = bounded_json(&wide, 32 * 1024);
        assert_eq!(clipped["truncated"], Value::Bool(true));
        assert_eq!(clipped["value"].as_array().unwrap().len(), 200);
        let huge = Value::String("x".repeat(10_000));
        let hashed = bounded_json(&huge, 1024);
        assert_eq!(hashed["truncated"], Value::Bool(true));
        assert!(hashed["sha256"].as_str().unwrap().len() == 64);
        assert!(hashed["preview"].as_str().unwrap().len() <= 768);
    }

    #[test]
    fn finalize_dto_rejects_oversized_payloads() {
        let mut small = Map::new();
        small.insert("a".into(), Value::from(1));
        assert!(finalize_dto(small).is_ok());
        let mut large = Map::new();
        large.insert("a".into(), Value::from("x".repeat(MAX_AGENT_DTO_BYTES)));
        let error = finalize_dto(large).unwrap_err();
        assert_eq!(error.message(), "Agent DTO 超过 64 KiB");
    }

    #[test]
    fn record_session_id_prefers_the_scan_row() {
        let mut row = Map::new();
        row.insert("id".into(), Value::from("row-id"));
        assert_eq!(record_session_id(&row, Some("session-id")), "row-id");
        assert_eq!(
            record_session_id(&Map::new(), Some("session-id")),
            "session-id"
        );
        assert_eq!(record_session_id(&Map::new(), None), "");
        // 数字 id 在 Python 里是真值，`str()` 后采纳，不该悄悄换成 source_id。
        let mut numeric = Map::new();
        numeric.insert("id".into(), Value::from(42));
        assert_eq!(record_session_id(&numeric, Some("session-id")), "42");
        // 假值（空串 / null / 0）回落。
        for falsy in [Value::from(""), Value::Null, Value::from(0)] {
            let mut row = Map::new();
            row.insert("id".into(), falsy);
            assert_eq!(record_session_id(&row, Some("session-id")), "session-id");
        }
    }
}
