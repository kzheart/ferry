//! 扫描阶段的 token 用量归一化辅助。
//!
//! 三个工具的原始 token 字段口径不同，统一成
//! `{"input", "output", "cache_read", "cache_write"}`；其中 input 只计未命中
//! 缓存的输入（缓存读取单独放 cache_read），便于按多来源公开单价分档估算。
//!
//! 分层备注：`empty_tokens` / `add_tokens` / `has_tokens` / `dominant_model` /
//! `iso_ms` 在 Python 侧被 `adapters/**/scanner.py` 反向引用。Rust 禁止
//! `adapters → sessions`，方案 §1.1 要求把这几个纯函数落到
//! `adapters/shared/scanner`。WP-B2 尚未提供，故先在此实现；WP-E 去重时
//! 把它们搬过去、这里改 re-export 即可。

use serde_json::{Map, Value};

use crate::errors::DomainResult;
use crate::system::pricing::pricing;

use super::index::AgentSessionIndex;
use super::safety::{finalize_dto, now_ms, string_set, validated_interval};

/// 归一化 token 桶的四个键，顺序即 DTO 里的键序。
pub const TOKEN_KEYS: [&str; 4] = ["input", "output", "cache_read", "cache_write"];

const MAX_USAGE_BUCKETS: usize = 15;

/// 归一化后的 token 计数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
}

impl Tokens {
    pub fn get(&self, key: &str) -> i64 {
        match key {
            "input" => self.input,
            "output" => self.output,
            "cache_read" => self.cache_read,
            _ => self.cache_write,
        }
    }

    pub fn total(&self) -> i64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    pub fn to_value(self) -> Value {
        let mut payload = Map::new();
        for key in TOKEN_KEYS {
            payload.insert(key.into(), Value::from(self.get(key)));
        }
        Value::Object(payload)
    }

    /// 从原生 JSON 桶读数：非数值一律按 0（Python `int(other.get(key) or 0)`）。
    pub fn from_value(value: &Value) -> Self {
        let read = |key: &str| -> i64 {
            match value.get(key) {
                Some(Value::Number(number)) => number
                    .as_i64()
                    .or_else(|| number.as_f64().map(|float| float.trunc() as i64))
                    .unwrap_or(0),
                Some(Value::Bool(true)) => 1,
                _ => 0,
            }
        };
        Self {
            input: read("input"),
            output: read("output"),
            cache_read: read("cache_read"),
            cache_write: read("cache_write"),
        }
    }
}

/// `empty_tokens()`。
pub fn empty_tokens() -> Tokens {
    Tokens::default()
}

/// `add_tokens(acc, other)`。
pub fn add_tokens(accumulator: &mut Tokens, other: &Tokens) {
    accumulator.input += other.input;
    accumulator.output += other.output;
    accumulator.cache_read += other.cache_read;
    accumulator.cache_write += other.cache_write;
}

/// `has_tokens(tokens)`。
pub fn has_tokens(tokens: &Tokens) -> bool {
    TOKEN_KEYS.iter().any(|key| tokens.get(key) != 0)
}

/// 出现 token 最多的模型作为该会话的代表模型。
///
/// `by_model` 保持插入序（Python dict）：并列时先出现的胜出，因为 `max()`
/// 只在严格大于时才换人。
pub fn dominant_model(by_model: &[(String, Tokens)]) -> String {
    by_model
        .iter()
        .fold(None::<(&str, i64)>, |best, (model, tokens)| {
            let total = tokens.total();
            match best {
                Some((_, best_total)) if best_total >= total => best,
                _ => Some((model.as_str(), total)),
            }
        })
        .map(|(model, _)| model.to_string())
        .unwrap_or_default()
}

/// ISO8601（带 Z）转毫秒时间戳；已是数字则原样返回；解析失败返回 `None`。
///
/// naive datetime 当 UTC（对齐 `usage.py` 的 `replace(tzinfo=utc)`）。
pub fn iso_ms(value: &Value) -> Option<i64> {
    match value {
        Value::Null => None,
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|float| float.trunc() as i64)),
        other => {
            let text = match other {
                Value::String(text) => text.clone(),
                _ => other.to_string(),
            };
            super::safety::parse_iso8601_ms(&text)
        }
    }
}

/// Python `str.casefold()` 的近似：Rust 标准库没有 full case folding，
/// `to_lowercase()` 覆盖了实际会出现的路径/标题/模型名场景。
pub fn casefold(text: &str) -> String {
    text.to_lowercase()
}

fn norm_model(model: &str) -> String {
    casefold(model.rsplit('/').next().unwrap_or(model)).replace('_', "-")
}

fn norm_full_model(model: &str) -> String {
    casefold(model.trim()).replace('_', "-")
}

/// 单价表的归一化索引：同名归一后**先到者胜**（Python `setdefault`）。
pub fn price_index(prices: &Map<String, Value>) -> Vec<(String, Value)> {
    let mut index: Vec<(String, Value)> = Vec::new();
    for (key, value) in prices {
        let normalized = norm_model(key);
        if !index.iter().any(|(seen, _)| *seen == normalized) {
            index.push((normalized, value.clone()));
        }
    }
    index
}

/// 与总览页同一套匹配规则：只接受完整 key 或裸 model-part 的精确命中。
/// SKU 前缀猜测（如用 `gpt-5` 给 `gpt-5-mini` 计价）宁可判为未计价。
pub fn match_price<'a>(
    model: &str,
    prices: &'a Map<String, Value>,
    index: &'a [(String, Value)],
) -> Option<&'a Value> {
    if model.is_empty() || prices.is_empty() {
        return None;
    }
    if let Some(exact) = prices.get(&norm_full_model(model)) {
        return Some(exact);
    }
    let normalized = norm_model(model);
    if let Some((_, value)) = index.iter().find(|(key, _)| *key == normalized) {
        return Some(value);
    }
    None
}

fn cost_of(tokens: &Tokens, price: Option<&Value>) -> f64 {
    let Some(price) = price.filter(|price| !is_empty_price(price)) else {
        return 0.0;
    };
    TOKEN_KEYS
        .iter()
        .map(|key| {
            let rate = price.get(key).and_then(Value::as_f64).unwrap_or(0.0);
            tokens.get(key) as f64 * rate
        })
        .sum::<f64>()
        / 1_000_000.0
}

fn is_empty_price(price: &Value) -> bool {
    match price {
        Value::Object(entries) => entries.is_empty(),
        Value::Null => true,
        _ => false,
    }
}

fn usage_by_model(row: &Map<String, Value>, fallback: &Tokens) -> Vec<(String, Tokens)> {
    if let Some(entries) = row.get("usage_by_model").and_then(Value::as_object) {
        let usage: Vec<(String, Tokens)> = entries
            .iter()
            .filter(|(model, tokens)| !model.is_empty() && tokens.is_object())
            .map(|(model, tokens)| (model.clone(), Tokens::from_value(tokens)))
            .filter(|(_, tokens)| has_tokens(tokens))
            .collect();
        if !usage.is_empty() {
            return usage;
        }
    }
    let model = row.get("model").and_then(Value::as_str).unwrap_or_default();
    if model.is_empty() || !has_tokens(fallback) {
        Vec::new()
    } else {
        vec![(model.to_string(), *fallback)]
    }
}

/// Python `round(value, digits)`：十进制四舍六入五取偶。
fn round_to(value: f64, digits: i32) -> f64 {
    let factor = 10f64.powi(digits);
    (value * factor).round_ties_even() / factor
}

#[derive(Clone)]
struct Bucket {
    tokens: Tokens,
    cost: f64,
}

/// 按花费取前 15 项：上千个项目全量返回会撑爆 agent 的 DTO 预算。
fn top_by_cost(bucket: &[(String, Bucket)]) -> Value {
    let mut ordered: Vec<&(String, Bucket)> = bucket.iter().collect();
    // 稳定降序，排序键 = (cost, sum(tokens))。
    ordered.sort_by(|left, right| {
        let key = |entry: &(String, Bucket)| (entry.1.cost, entry.1.tokens.total());
        let (left_cost, left_tokens) = key(left);
        let (right_cost, right_tokens) = key(right);
        right_cost
            .partial_cmp(&left_cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(right_tokens.cmp(&left_tokens))
    });
    let mut payload = Map::new();
    for (name, entry) in ordered.into_iter().take(MAX_USAGE_BUCKETS) {
        let mut item = Map::new();
        item.insert("tokens".into(), entry.tokens.to_value());
        item.insert(
            "cost".into(),
            serde_json::Number::from_f64(entry.cost)
                .map(Value::Number)
                .unwrap_or(Value::from(0)),
        );
        payload.insert(name.clone(), Value::Object(item));
    }
    Value::Object(payload)
}

fn upsert<'a>(bucket: &'a mut Vec<(String, Bucket)>, key: &str) -> &'a mut Bucket {
    if let Some(position) = bucket.iter().position(|(name, _)| name == key) {
        return &mut bucket[position].1;
    }
    bucket.push((
        key.to_string(),
        Bucket {
            tokens: empty_tokens(),
            cost: 0.0,
        },
    ));
    let last = bucket.len() - 1;
    &mut bucket[last].1
}

/// `usage` RPC 的主体。
pub fn get_usage(
    agents: Option<&Value>,
    projects: Option<&Value>,
    time_range: Option<&Value>,
    index: &AgentSessionIndex,
) -> DomainResult<Map<String, Value>> {
    let allowed_agents = string_set(agents, "agents", 8, 32)?;
    // Python 侧是 `{item.casefold() for item in ...}`：折叠后还要再去一次重。
    let mut allowed_projects: Vec<String> = string_set(projects, "projects", 20, 256)?
        .iter()
        .map(|item| casefold(item))
        .collect();
    allowed_projects.sort_unstable();
    allowed_projects.dedup();
    let (start, end) = validated_interval(time_range)?;

    let mut total = empty_tokens();
    let mut by_agent: Vec<(String, Tokens)> = Vec::new();
    let mut by_model: Vec<(String, Bucket)> = Vec::new();
    let mut by_project: Vec<(String, Bucket)> = Vec::new();
    let mut sessions = 0i64;
    let prices = pricing(false, true).prices;
    let index_of_prices = price_index(&prices);
    let mut cost_total = 0.0f64;
    let mut unpriced_models: Vec<String> = Vec::new();

    for record in index.refresh()? {
        let row = &record.row;
        let updated = row.get("updated").and_then(Value::as_i64).unwrap_or(0);
        let project = row
            .get("dir")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !allowed_agents.is_empty() && !allowed_agents.contains(&record.tool) {
            continue;
        }
        if !allowed_projects.is_empty() && !allowed_projects.contains(&casefold(&project)) {
            continue;
        }
        if start.is_some_and(|start| updated < start) || end.is_some_and(|end| updated > end) {
            continue;
        }
        let Some(raw_tokens) = row.get("tokens").filter(|tokens| tokens.is_object()) else {
            continue;
        };
        let tokens = Tokens::from_value(raw_tokens);
        sessions += 1;
        add_tokens(&mut total, &tokens);
        {
            let slot = match by_agent.iter().position(|(name, _)| *name == record.tool) {
                Some(position) => &mut by_agent[position].1,
                None => {
                    by_agent.push((record.tool.clone(), empty_tokens()));
                    let last = by_agent.len() - 1;
                    &mut by_agent[last].1
                }
            };
            add_tokens(slot, &tokens);
        }
        let model_usage = usage_by_model(row, &tokens);
        let mut session_cost = 0.0;
        for (model, model_tokens) in model_usage {
            let price = match_price(&model, &prices, &index_of_prices);
            let cost = cost_of(&model_tokens, price);
            session_cost += cost;
            if price.is_none() && !unpriced_models.contains(&model) {
                unpriced_models.push(model.clone());
            }
            let entry = upsert(&mut by_model, &model);
            add_tokens(&mut entry.tokens, &model_tokens);
            entry.cost = round_to(entry.cost + cost, 6);
        }
        cost_total += session_cost;
        let project_entry = upsert(
            &mut by_project,
            if project.is_empty() {
                "unknown"
            } else {
                &project
            },
        );
        add_tokens(&mut project_entry.tokens, &tokens);
        project_entry.cost = round_to(project_entry.cost + session_cost, 6);
    }

    let mut agent_totals = Map::new();
    for (name, tokens) in &by_agent {
        agent_totals.insert(name.clone(), tokens.to_value());
    }
    unpriced_models.sort_unstable();
    let mut sorted_agents: Vec<&String> = allowed_agents.iter().collect();
    sorted_agents.sort_unstable();
    let mut sorted_projects: Vec<&String> = allowed_projects.iter().collect();
    sorted_projects.sort_unstable();

    let mut filters = Map::new();
    filters.insert(
        "agents".into(),
        if allowed_agents.is_empty() {
            Value::Null
        } else {
            Value::Array(
                sorted_agents
                    .into_iter()
                    .map(|item| Value::from(item.as_str()))
                    .collect(),
            )
        },
    );
    filters.insert(
        "projects".into(),
        if allowed_projects.is_empty() {
            Value::Null
        } else {
            Value::Array(
                sorted_projects
                    .into_iter()
                    .map(|item| Value::from(item.as_str()))
                    .collect(),
            )
        },
    );
    let mut interval = Map::new();
    interval.insert("from".into(), start.map(Value::from).unwrap_or(Value::Null));
    interval.insert("to".into(), end.map(Value::from).unwrap_or(Value::Null));
    filters.insert("time_range".into(), Value::Object(interval));

    let mut payload = Map::new();
    payload.insert("sessions".into(), Value::from(sessions));
    payload.insert("tokens".into(), total.to_value());
    payload.insert("by_agent".into(), Value::Object(agent_totals));
    // 金额按 LiteLLM / OpenRouter / models.dev 的公开单价估算：匹配不上时只计 token，
    // 这些模型名列在 unpriced_models 里，免得下游把估算当账单。
    payload.insert("by_model".into(), top_by_cost(&by_model));
    payload.insert("by_project".into(), top_by_cost(&by_project));
    payload.insert(
        "cost".into(),
        serde_json::Number::from_f64(round_to(cost_total, 4))
            .map(Value::Number)
            .unwrap_or(Value::from(0)),
    );
    payload.insert(
        "cost_basis".into(),
        Value::from("estimated_from_public_prices"),
    );
    payload.insert(
        "unpriced_models".into(),
        Value::Array(
            unpriced_models
                .into_iter()
                .take(20)
                .map(Value::from)
                .collect(),
        ),
    );
    payload.insert("currency".into(), Value::from("USD"));
    // 模型手里没有时钟：不给基准时间，它会去 shell 里跑 date 换算相对区间。
    payload.insert("now".into(), Value::from(now_ms()));
    payload.insert("filters".into(), Value::Object(filters));
    finalize_dto(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn prices() -> Map<String, Value> {
        json!({
            "anthropic/claude-sonnet-4-5": {"input": 3.0},
            "gpt-5": {"input": 1.0},
        })
        .as_object()
        .unwrap()
        .clone()
    }

    /// 不猜 SKU 前缀，完整 key 与裸 model-part 精确命中才可计价。
    #[test]
    fn match_price_requires_exact_model_identity() {
        let prices = prices();
        let index = price_index(&prices);
        assert_eq!(
            match_price("claude-sonnet-4-5-20250929", &prices, &index),
            None
        );
        assert_eq!(
            match_price("claude-sonnet-4-5", &prices, &index),
            Some(&json!({"input": 3.0}))
        );
        assert_eq!(match_price("gpt-5-mini", &prices, &index), None);
        assert_eq!(match_price("gpt-51", &prices, &index), None);
        assert_eq!(match_price("", &prices, &index), None);
        assert_eq!(
            match_price("anthropic/claude-sonnet-4-5", &prices, &index),
            Some(&json!({"input": 3.0}))
        );
    }

    /// 不带时区的时间戳按 UTC 解释——原生会话里这种写法很常见。
    #[test]
    fn iso_ms_treats_naive_datetimes_as_utc() {
        assert_eq!(iso_ms(&json!("2024-01-01T00:00:00Z")), Some(1704067200000));
        assert_eq!(iso_ms(&json!("2024-01-01T00:00:00")), Some(1704067200000));
        assert_eq!(iso_ms(&json!(5)), Some(5));
        assert_eq!(iso_ms(&json!(null)), None);
        assert_eq!(iso_ms(&json!("nope")), None);
    }

    #[test]
    fn token_arithmetic_matches_the_python_helpers() {
        let mut accumulator = empty_tokens();
        assert!(!has_tokens(&accumulator));
        let other = Tokens::from_value(&json!({"input": 3, "cache_read": 2, "output": null}));
        add_tokens(&mut accumulator, &other);
        assert_eq!(accumulator.input, 3);
        assert_eq!(accumulator.cache_read, 2);
        assert_eq!(accumulator.output, 0);
        assert!(has_tokens(&accumulator));
        assert_eq!(
            accumulator.to_value(),
            json!({"input": 3, "output": 0, "cache_read": 2, "cache_write": 0})
        );
    }

    #[test]
    fn dominant_model_keeps_the_first_on_ties() {
        let by_model = vec![
            (
                "a".to_string(),
                Tokens {
                    input: 5,
                    ..Tokens::default()
                },
            ),
            (
                "b".to_string(),
                Tokens {
                    input: 5,
                    ..Tokens::default()
                },
            ),
            (
                "c".to_string(),
                Tokens {
                    input: 1,
                    ..Tokens::default()
                },
            ),
        ];
        assert_eq!(dominant_model(&by_model), "a");
        assert_eq!(dominant_model(&[]), "");
    }

    #[test]
    fn cost_is_per_million_tokens() {
        let tokens = Tokens {
            input: 1_000_000,
            output: 500_000,
            ..Tokens::default()
        };
        let price = json!({"input": 3.0, "output": 15.0});
        assert!((cost_of(&tokens, Some(&price)) - 10.5).abs() < 1e-9);
        assert_eq!(cost_of(&tokens, None), 0.0);
    }

    #[test]
    fn buckets_are_capped_at_fifteen_by_cost() {
        let bucket: Vec<(String, Bucket)> = (0..20)
            .map(|position| {
                (
                    format!("model-{position}"),
                    Bucket {
                        tokens: Tokens {
                            input: position,
                            ..Tokens::default()
                        },
                        cost: position as f64,
                    },
                )
            })
            .collect();
        let top = top_by_cost(&bucket);
        let entries = top.as_object().unwrap();
        assert_eq!(entries.len(), 15);
        assert_eq!(entries.keys().next().unwrap(), "model-19");
        assert!(!entries.contains_key("model-4"));
    }

    #[test]
    fn rounding_follows_python_semantics() {
        assert_eq!(round_to(0.123_456_49, 6), 0.123_456);
        assert_eq!(round_to(2.5, 0), 2.0);
        assert_eq!(round_to(3.5, 0), 4.0);
    }
}
