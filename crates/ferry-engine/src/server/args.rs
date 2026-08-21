//! CLI 薄客户端的参数解析：flag 拆分与时间换算。
//!
//! 只做「命令行形状 → 引擎参数形状」的机械换算，**不做业务校验**：
//! `--exhaustive` 必须与 `--regex` 同用之类的规则由引擎判定，CLI 重复一遍只会
//! 让两处词表漂移。唯一的例外是这里必须自己完成的换算——时间。
//!
//! 时间一律按 **UTC** 解释：std 没有时区数据库，引 chrono 只为解析
//! `YYYY-MM-DD` 不划算；相对量（`7d` / `24h` / `90m`）没有时区歧义，是推荐写法。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

/// 解析后的命令行。
#[derive(Debug, Default)]
pub struct Parsed {
    positional: Vec<String>,
    values: BTreeMap<String, Vec<String>>,
    switches: BTreeSet<String>,
}

impl Parsed {
    pub fn positional(&self, index: usize, name: &str) -> Result<&str, String> {
        self.positional
            .get(index)
            .map(String::as_str)
            .ok_or_else(|| format!("缺少参数 <{name}>"))
    }

    pub fn positionals(&self) -> &[String] {
        &self.positional
    }

    /// 同名 flag 取最后一次（`--limit 5 --limit 9` 用 9）。
    pub fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name)?.last().map(String::as_str)
    }

    /// 可重复 flag 的全部取值（`--pattern`）。
    pub fn repeated(&self, name: &str) -> Vec<String> {
        self.values.get(name).cloned().unwrap_or_default()
    }

    pub fn has(&self, name: &str) -> bool {
        self.switches.contains(name)
    }

    pub fn int(&self, name: &str) -> Result<Option<i64>, String> {
        match self.value(name) {
            None => Ok(None),
            Some(text) => text
                .parse::<i64>()
                .map(Some)
                .map_err(|_| format!("--{name} 必须是整数: {text}")),
        }
    }

    /// 逗号分隔转数组：`--agent claude,codex`。
    pub fn list(&self, name: &str) -> Option<Vec<String>> {
        let mut items: Vec<String> = Vec::new();
        for raw in self.values.get(name)? {
            items.extend(
                raw.split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_string),
            );
        }
        Some(items)
    }
}

/// 拆 `--flag value` / `--flag=value` / `--switch`。
///
/// 未知 flag 直接报错：拼错的 flag 被当成 query 静默吞掉，是最难查的那种错。
pub fn parse(argv: &[String], value_flags: &[&str], switches: &[&str]) -> Result<Parsed, String> {
    let mut parsed = Parsed::default();
    let mut index = 0;
    while index < argv.len() {
        let argument = argv[index].as_str();
        if let Some(body) = argument.strip_prefix("--") {
            let (name, inline) = match body.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (body, None),
            };
            if switches.contains(&name) && inline.is_none() {
                parsed.switches.insert(name.to_string());
                index += 1;
                continue;
            }
            if !value_flags.contains(&name) {
                return Err(format!("未知参数: --{name}"));
            }
            let value = match inline {
                Some(value) => value,
                None => {
                    index += 1;
                    argv.get(index)
                        .cloned()
                        .ok_or_else(|| format!("--{name} 缺少取值"))?
                }
            };
            parsed
                .values
                .entry(name.to_string())
                .or_default()
                .push(value);
            index += 1;
            continue;
        }
        parsed.positional.push(argument.to_string());
        index += 1;
    }
    Ok(parsed)
}

/// `--since/--until` → epoch-ms。
///
/// 接受 `YYYY-MM-DD`、`YYYY-MM-DDTHH:MM`（UTC）与相对量 `7d`/`24h`/`90m`/`30s`。
pub fn parse_instant(text: &str, now_ms: i64) -> Result<i64, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("时间不能为空".to_string());
    }
    if let Some(relative) = parse_relative(text)? {
        return Ok(now_ms - relative);
    }
    let (date, time) = match text.split_once('T') {
        Some((date, time)) => (date, Some(time)),
        None => (text, None),
    };
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return Err(format!(
            "无法解析时间 {text}（支持 YYYY-MM-DD、YYYY-MM-DDTHH:MM、7d/24h/90m）"
        ));
    }
    let year: i64 = numeric(parts[0], text)?;
    let month: i64 = numeric(parts[1], text)?;
    let day: i64 = numeric(parts[2], text)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(format!("时间越界: {text}"));
    }
    let (hour, minute, second) = match time {
        None => (0, 0, 0),
        Some(clock) => {
            let units: Vec<&str> = clock.split(':').collect();
            if units.len() < 2 || units.len() > 3 {
                return Err(format!("无法解析时间 {text}"));
            }
            let hour: i64 = numeric(units[0], text)?;
            let minute: i64 = numeric(units[1], text)?;
            let second: i64 = match units.get(2) {
                Some(value) => numeric(value, text)?,
                None => 0,
            };
            if hour > 23 || minute > 59 || second > 59 {
                return Err(format!("时间越界: {text}"));
            }
            (hour, minute, second)
        }
    };
    let days = days_from_civil(year, month as u32, day as u32);
    Ok((days * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000)
}

fn numeric(text: &str, whole: &str) -> Result<i64, String> {
    text.parse::<i64>()
        .map_err(|_| format!("无法解析时间 {whole}"))
}

/// `7d` / `24h` / `90m` / `30s` / `2w` → 毫秒；不是相对量就回 `None`。
fn parse_relative(text: &str) -> Result<Option<i64>, String> {
    let Some(unit) = text.chars().last() else {
        return Ok(None);
    };
    let scale = match unit {
        's' => 1_000,
        'm' => 60_000,
        'h' => 3_600_000,
        'd' => 86_400_000,
        'w' => 604_800_000,
        _ => return Ok(None),
    };
    let digits = &text[..text.len() - unit.len_utf8()];
    if digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
        return Ok(None);
    }
    digits
        .parse::<i64>()
        .map(|amount| Some(amount * scale))
        .map_err(|_| format!("相对时间越界: {text}"))
}

/// Howard Hinnant 的 `days_from_civil`（[`crate::server::serve`] 里有它的逆）。
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 } as i64;
    let day_of_year = (153 * shifted_month + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// 组装引擎的 `time_range`：两端都缺就不下发这个参数。
pub fn time_range(
    since: Option<&str>,
    until: Option<&str>,
    now_ms: i64,
) -> Result<Option<Value>, String> {
    if since.is_none() && until.is_none() {
        return Ok(None);
    }
    let mut interval = Map::new();
    if let Some(since) = since {
        interval.insert("from".into(), Value::from(parse_instant(since, now_ms)?));
    }
    if let Some(until) = until {
        interval.insert("to".into(), Value::from(parse_instant(until, now_ms)?));
    }
    Ok(Some(Value::Object(interval)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn flags_values_and_positionals_are_separated() {
        let parsed = parse(
            &argv(&[
                "sqlite",
                "--agent",
                "claude,codex",
                "--limit=5",
                "--pattern",
                "a",
                "--pattern",
                "b",
                "--regex",
            ]),
            &["agent", "limit", "pattern"],
            &["regex"],
        )
        .expect("可解析");
        assert_eq!(parsed.positional(0, "query").unwrap(), "sqlite");
        assert_eq!(parsed.list("agent").unwrap(), ["claude", "codex"]);
        assert_eq!(parsed.int("limit").unwrap(), Some(5));
        assert_eq!(parsed.repeated("pattern"), ["a", "b"]);
        assert!(parsed.has("regex"));
        assert!(!parsed.has("exhaustive"));
    }

    #[test]
    fn unknown_and_incomplete_flags_are_rejected() {
        assert!(parse(&argv(&["--nope", "1"]), &["limit"], &[])
            .unwrap_err()
            .contains("未知参数"));
        assert!(parse(&argv(&["--limit"]), &["limit"], &[])
            .unwrap_err()
            .contains("缺少取值"));
        assert!(parse(&argv(&["--limit", "x"]), &["limit"], &[])
            .unwrap()
            .int("limit")
            .unwrap_err()
            .contains("必须是整数"));
    }

    #[test]
    fn absolute_times_are_utc() {
        assert_eq!(parse_instant("1970-01-01", 0).unwrap(), 0);
        assert_eq!(parse_instant("1970-01-02", 0).unwrap(), 86_400_000);
        assert_eq!(parse_instant("2024-01-01", 0).unwrap(), 1_704_067_200_000);
        assert_eq!(
            parse_instant("2024-01-01T12:30", 0).unwrap(),
            1_704_067_200_000 + 45_000_000
        );
        assert_eq!(
            parse_instant("2024-01-01T12:30:15", 0).unwrap(),
            1_704_067_200_000 + 45_015_000
        );
    }

    #[test]
    fn relative_times_count_back_from_now() {
        let now = 1_704_067_200_000;
        assert_eq!(parse_instant("7d", now).unwrap(), now - 604_800_000);
        assert_eq!(parse_instant("24h", now).unwrap(), now - 86_400_000);
        assert_eq!(parse_instant("90m", now).unwrap(), now - 5_400_000);
        assert_eq!(parse_instant("30s", now).unwrap(), now - 30_000);
        assert_eq!(parse_instant("2w", now).unwrap(), now - 1_209_600_000);
    }

    #[test]
    fn malformed_times_are_reported_not_guessed() {
        for text in [
            "",
            "昨天",
            "2024-13-01",
            "2024-01-01T25:00",
            "2024/01/01",
            "d",
        ] {
            assert!(parse_instant(text, 0).is_err(), "{text} 不该被接受");
        }
    }

    #[test]
    fn time_range_is_omitted_when_both_ends_are_absent() {
        assert!(time_range(None, None, 0).unwrap().is_none());
        let range = time_range(Some("1970-01-02"), Some("1970-01-03"), 0)
            .unwrap()
            .unwrap();
        assert_eq!(range["from"], Value::from(86_400_000));
        assert_eq!(range["to"], Value::from(172_800_000));
    }

    #[test]
    fn civil_day_conversion_round_trips() {
        for days in [-719_162, 0, 19_723, 25_000] {
            let (year, month, day) = crate::server::serve::civil_from_days_for_tests(days);
            assert_eq!(days_from_civil(year, month, day), days);
        }
    }
}
