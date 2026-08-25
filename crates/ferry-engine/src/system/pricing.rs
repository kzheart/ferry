//! 多来源模型单价：LiteLLM + OpenRouter + models.dev。
//!
//! 每个网络来源独立缓存、独立失败降级；任何一家不可用都不会拖垮其余来源。最终
//! 返回扁平表 `{model_id: {input, output, cache_read, cache_write, source,
//! matched_key}}`，价格单位统一为 USD / 百万 token。内置少量价格只在动态来源
//! 无法覆盖时兜底。

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::paths::home_dir;

pub const LITELLM_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
pub const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/models";
pub const MODELS_DEV_URL: &str = "https://models.dev/api.json";
pub const TTL_SECONDS: u64 = 3600;
const FETCH_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_RETRIES: usize = 3;

/// 兼容旧调用：现在返回价格缓存目录。
pub fn cache_path() -> PathBuf {
    cache_dir()
}

fn cache_dir() -> PathBuf {
    home_dir().join(".ferry").join("pricing")
}

/// 离线兜底：少量常见公开模型的近似单价（USD / 百万 token）。
const FALLBACK: &[(&str, f64, f64, f64, f64)] = &[
    ("claude-opus-4", 15.0, 75.0, 1.5, 18.75),
    ("claude-sonnet-4", 3.0, 15.0, 0.3, 3.75),
    ("claude-3-5-haiku", 0.8, 4.0, 0.08, 1.0),
    ("gpt-5", 1.25, 10.0, 0.125, 0.0),
    ("gpt-5-mini", 0.25, 2.0, 0.025, 0.0),
    ("gpt-4o", 2.5, 10.0, 1.25, 0.0),
    ("deepseek-chat", 0.27, 1.1, 0.07, 0.0),
];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Price {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
}

impl Price {
    fn usable(&self) -> bool {
        [self.input, self.output, self.cache_read, self.cache_write]
            .into_iter()
            .flatten()
            .any(|value| value.is_finite() && value >= 0.0)
    }

    fn to_value(&self, source: &str, matched_key: &str) -> Value {
        let mut value = Map::new();
        value.insert("input".into(), number(self.input.unwrap_or(0.0)));
        value.insert("output".into(), number(self.output.unwrap_or(0.0)));
        value.insert("cache_read".into(), number(self.cache_read.unwrap_or(0.0)));
        value.insert(
            "cache_write".into(),
            number(self.cache_write.unwrap_or(0.0)),
        );
        value.insert("source".into(), Value::from(source));
        value.insert("matched_key".into(), Value::from(matched_key));
        Value::Object(value)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceStatus {
    pub source: &'static str,
    pub state: &'static str,
    pub fetched_at: i64,
    pub models: usize,
    pub error: Option<String>,
}

impl SourceStatus {
    pub fn to_value(&self) -> Value {
        let mut value = Map::new();
        value.insert("source".into(), Value::from(self.source));
        value.insert("state".into(), Value::from(self.state));
        value.insert("fetched_at".into(), Value::from(self.fetched_at));
        value.insert("models".into(), Value::from(self.models as i64));
        value.insert(
            "error".into(),
            self.error.as_deref().map_or(Value::Null, Value::from),
        );
        Value::Object(value)
    }
}

/// `pricing()` 的返回结构。
#[derive(Clone, Debug, PartialEq)]
pub struct Pricing {
    pub prices: Map<String, Value>,
    pub fetched_at: i64,
    /// multi | fallback
    pub source: &'static str,
    pub sources: Vec<SourceStatus>,
}

fn fallback_prices() -> Map<String, Value> {
    let mut prices = Map::new();
    for (model, input, output, cache_read, cache_write) in FALLBACK {
        let mut cost = Map::new();
        cost.insert("input".into(), number(*input));
        cost.insert("output".into(), number(*output));
        cost.insert("cache_read".into(), number(*cache_read));
        cost.insert("cache_write".into(), number(*cache_write));
        cost.insert("source".into(), Value::from("Built-in"));
        cost.insert("matched_key".into(), Value::from(*model));
        prices.insert((*model).to_string(), Value::Object(cost));
    }
    prices
}

/// Python 侧的兜底表混用 int/float 字面量；这里统一按整数优先输出，
/// 保持 JSON 里 `15` 而不是 `15.0`。
fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9e15 {
        Value::from(value as i64)
    } else {
        Value::from(value)
    }
}

/// 把 models.dev 的分层响应压成 provider-qualified 数据集。保留供应商前缀，
/// 避免同名模型被后遍历到的 reseller 静默覆盖。
fn parse_models_dev(api: &Value) -> HashMap<String, Price> {
    let mut prices = HashMap::new();
    let Some(providers) = api.as_object() else {
        return prices;
    };
    for (provider_id, provider) in providers {
        let Some(models) = provider.get("models").and_then(Value::as_object) else {
            continue;
        };
        for (model_key, model) in models {
            let Some(cost) = model.get("cost").and_then(Value::as_object) else {
                continue;
            };
            let read = |key: &str| {
                cost.get(key)
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite() && *value >= 0.0)
            };
            let price = Price {
                input: read("input"),
                output: read("output"),
                cache_read: read("cache_read"),
                cache_write: read("cache_write"),
            };
            if !price.usable() {
                continue;
            }
            let model_id = model
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .unwrap_or(model_key);
            prices.insert(format!("{provider_id}/{model_id}").to_lowercase(), price);
        }
    }
    prices
}

/// 保留原 public helper 的兼容语义：只输出 models.dev 的扁平四桶表。
pub fn flatten(api: &Value) -> Map<String, Value> {
    parse_models_dev(api)
        .into_iter()
        .map(|(key, price)| {
            let model = key.rsplit('/').next().unwrap_or(&key).to_string();
            (model.clone(), price.to_value("Models.dev", &key))
        })
        .collect()
}

fn parse_litellm(api: &Value) -> HashMap<String, Price> {
    let Some(models) = api.as_object() else {
        return HashMap::new();
    };
    models
        .iter()
        .filter(|(key, _)| !key.to_ascii_lowercase().starts_with("github_copilot/"))
        .filter_map(|(key, value)| {
            let per_million = |name: &str| {
                value
                    .get(name)
                    .and_then(Value::as_f64)
                    .filter(|rate| rate.is_finite() && *rate >= 0.0)
                    .map(|rate| rate * 1_000_000.0)
            };
            let price = Price {
                input: per_million("input_cost_per_token"),
                output: per_million("output_cost_per_token"),
                cache_read: per_million("cache_read_input_token_cost"),
                cache_write: per_million("cache_creation_input_token_cost"),
            };
            price.usable().then(|| (key.to_lowercase(), price))
        })
        .collect()
}

fn parse_openrouter(api: &Value) -> HashMap<String, Price> {
    let Some(models) = api.get("data").and_then(Value::as_array) else {
        return HashMap::new();
    };
    models
        .iter()
        .filter_map(|model| {
            let id = model.get("id").and_then(Value::as_str)?;
            let pricing = model.get("pricing")?;
            let parse = |key: &str| {
                pricing
                    .get(key)
                    .and_then(|value| {
                        value
                            .as_str()
                            .and_then(|text| text.parse::<f64>().ok())
                            .or_else(|| value.as_f64())
                    })
                    .filter(|rate| rate.is_finite() && *rate >= 0.0)
                    .map(|rate| rate * 1_000_000.0)
            };
            let price = Price {
                input: parse("prompt"),
                output: parse("completion"),
                cache_read: parse("input_cache_read"),
                cache_write: parse("input_cache_write"),
            };
            price.usable().then(|| (id.to_lowercase(), price))
        })
        .collect()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

fn source_cache_path(source: &str) -> PathBuf {
    cache_dir().join(format!("{source}.json"))
}

#[derive(Serialize, Deserialize)]
struct CachedSource {
    fetched_at: i64,
    prices: HashMap<String, Price>,
}

fn read_source_cache(source: &str, allow_stale: bool) -> Option<CachedSource> {
    let text = std::fs::read_to_string(source_cache_path(source)).ok()?;
    let cached: CachedSource = serde_json::from_str(&text).ok()?;
    if cached.fetched_at > now_ms() {
        return None;
    }
    if !allow_stale && now_ms().saturating_sub(cached.fetched_at) > (TTL_SECONDS * 1000) as i64 {
        return None;
    }
    (!cached.prices.is_empty()).then_some(cached)
}

fn write_source_cache(source: &str, data: &CachedSource) {
    let path = source_cache_path(source);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(body) = serde_json::to_vec(data) else {
        return;
    };
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    if std::fs::write(&temp, body).is_ok() {
        let _ = std::fs::rename(&temp, &path);
    } else {
        let _ = std::fs::remove_file(temp);
    }
}

fn fetch_json(url: &str) -> Result<Value, String> {
    let mut last = String::new();
    for attempt in 0..MAX_RETRIES {
        let result = ureq::get(url)
            .config()
            .timeout_global(Some(FETCH_TIMEOUT))
            .build()
            .header("User-Agent", "ferry/0.8")
            .call()
            .map_err(|error| error.to_string())
            .and_then(|response| {
                response
                    .into_body()
                    .read_to_string()
                    .map_err(|error| error.to_string())
            })
            .and_then(|text| serde_json::from_str(&text).map_err(|error| error.to_string()));
        match result {
            Ok(value) => return Ok(value),
            Err(error) => last = error,
        }
        if attempt + 1 < MAX_RETRIES {
            std::thread::sleep(Duration::from_millis(200 * (1 << attempt)));
        }
    }
    Err(last)
}

type Parser = fn(&Value) -> HashMap<String, Price>;

fn load_source(
    id: &'static str,
    label: &'static str,
    url: &str,
    parser: Parser,
    force: bool,
    cached_only: bool,
) -> (HashMap<String, Price>, SourceStatus) {
    if !force {
        if let Some(cached) = read_source_cache(id, false) {
            let models = cached.prices.len();
            return (
                cached.prices,
                SourceStatus {
                    source: label,
                    state: "cache",
                    fetched_at: cached.fetched_at,
                    models,
                    error: None,
                },
            );
        }
    }
    if !cached_only {
        match fetch_json(url).map(|api| parser(&api)) {
            Ok(prices) if !prices.is_empty() => {
                let fetched_at = now_ms();
                write_source_cache(
                    id,
                    &CachedSource {
                        fetched_at,
                        prices: prices.clone(),
                    },
                );
                let models = prices.len();
                return (
                    prices,
                    SourceStatus {
                        source: label,
                        state: "network",
                        fetched_at,
                        models,
                        error: None,
                    },
                );
            }
            result => {
                let error = match result {
                    Ok(_) => "返回的价格表为空".to_string(),
                    Err(error) => error,
                };
                if let Some(cached) = read_source_cache(id, true) {
                    let models = cached.prices.len();
                    return (
                        cached.prices,
                        SourceStatus {
                            source: label,
                            state: "stale",
                            fetched_at: cached.fetched_at,
                            models,
                            error: Some(error),
                        },
                    );
                }
                return (
                    HashMap::new(),
                    SourceStatus {
                        source: label,
                        state: "unavailable",
                        fetched_at: 0,
                        models: 0,
                        error: Some(error),
                    },
                );
            }
        }
    }
    if let Some(cached) = read_source_cache(id, true) {
        let models = cached.prices.len();
        return (
            cached.prices,
            SourceStatus {
                source: label,
                state: "stale",
                fetched_at: cached.fetched_at,
                models,
                error: None,
            },
        );
    }
    (
        HashMap::new(),
        SourceStatus {
            source: label,
            state: "unavailable",
            fetched_at: 0,
            models: 0,
            error: None,
        },
    )
}

fn normalize_model(model: &str) -> String {
    model.trim().to_ascii_lowercase().replace([' ', '_'], "-")
}

fn model_part(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}

fn key_preference(key: &str) -> (usize, &str) {
    const PROVIDERS: [&str; 6] = [
        "openai/",
        "anthropic/",
        "google/",
        "x-ai/",
        "deepseek/",
        "mistralai/",
    ];
    (
        PROVIDERS
            .iter()
            .position(|prefix| key.starts_with(prefix))
            .unwrap_or(100),
        key,
    )
}

/// 数据源优先级：LiteLLM > OpenRouter > models.dev > Built-in。
/// 每个 qualified key 都保留；同时为裸模型生成一个确定的 canonical alias。
fn merge_sources(sources: Vec<(&'static str, HashMap<String, Price>)>) -> Map<String, Value> {
    let mut result = fallback_prices();
    let mut canonical: HashMap<String, (usize, &str, String, Price)> = HashMap::new();
    for (source_rank, (source, data)) in sources.into_iter().enumerate() {
        for (key, price) in data {
            let normalized = normalize_model(&key);
            if !result.contains_key(&normalized)
                || result[&normalized].get("source").and_then(Value::as_str) == Some("Built-in")
            {
                result.insert(normalized.clone(), price.to_value(source, &key));
            }
            let part = normalize_model(model_part(&normalized));
            let replace = canonical
                .get(&part)
                .is_none_or(|(current_rank, _, current, _)| {
                    source_rank < *current_rank
                        || (source_rank == *current_rank
                            && key_preference(&normalized) < key_preference(current))
                });
            if replace {
                canonical.insert(part, (source_rank, source, normalized, price));
            }
        }
    }
    for (part, (_, source, key, price)) in canonical {
        result.insert(part, price.to_value(source, &key));
    }
    result
}

/// Tokscale 风格的多源降级：每个来源 fresh cache → network → stale → empty，
/// 最后再叠加 built-in。fresh cache TTL 与 Tokscale 一致为 1 小时；在线 RPC 会在
/// 过期后刷新网络，usage_stats 只读缓存。
pub fn pricing(force: bool, cached_only: bool) -> Pricing {
    let specs: [(&str, &str, &str, Parser); 3] = [
        ("litellm", "LiteLLM", LITELLM_URL, parse_litellm),
        ("openrouter", "OpenRouter", OPENROUTER_URL, parse_openrouter),
        ("models-dev", "Models.dev", MODELS_DEV_URL, parse_models_dev),
    ];
    let loaded: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = specs
            .into_iter()
            .map(|(id, label, url, parser)| {
                scope.spawn(move || {
                    let (data, status) = load_source(id, label, url, parser, force, cached_only);
                    (label, data, status)
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .collect()
    });
    let mut datasets = Vec::with_capacity(loaded.len());
    let mut statuses = Vec::with_capacity(loaded.len());
    for (label, data, status) in loaded {
        datasets.push((label, data));
        statuses.push(status);
    }
    let fetched_at = statuses
        .iter()
        .map(|status| status.fetched_at)
        .max()
        .unwrap_or(0);
    let has_dynamic = datasets.iter().any(|(_, data)| !data.is_empty());
    Pricing {
        prices: merge_sources(datasets),
        fetched_at,
        source: if has_dynamic { "multi" } else { "fallback" },
        sources: statuses,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flatten_keeps_only_models_that_declare_a_cost() {
        let api = json!({
            "anthropic": {"models": {
                "claude-x": {"cost": {"input": 3, "output": 15}},
                "no-cost": {"cost": {}},
                "missing": {},
            }},
            "not-a-provider": 1,
        });
        let prices = flatten(&api);
        assert_eq!(prices.len(), 1);
        assert_eq!(prices["claude-x"]["input"], Value::from(3));
        assert_eq!(prices["claude-x"]["cache_read"], Value::from(0));
        assert_eq!(prices["claude-x"]["source"], Value::from("Models.dev"));
    }

    #[test]
    fn litellm_converts_per_token_rates_to_per_million() {
        let data = parse_litellm(&json!({
            "openai/gpt-x": {
                "input_cost_per_token": 0.000002,
                "output_cost_per_token": 0.00001,
                "cache_read_input_token_cost": 0.0000002
            },
            "github_copilot/gpt-x": {
                "input_cost_per_token": 99
            }
        }));
        assert_eq!(data.len(), 1);
        assert_eq!(data["openai/gpt-x"].input, Some(2.0));
        assert_eq!(data["openai/gpt-x"].output, Some(10.0));
        assert!((data["openai/gpt-x"].cache_read.unwrap() - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn openrouter_accepts_string_rates_and_keeps_provider_key() {
        let data = parse_openrouter(&json!({"data": [{
            "id": "anthropic/claude-x",
            "pricing": {
                "prompt": "0.000003",
                "completion": "0.000015",
                "input_cache_read": "0.0000003"
            }
        }]}));
        assert_eq!(data["anthropic/claude-x"].input, Some(3.0));
        assert_eq!(data["anthropic/claude-x"].output, Some(15.0));
        assert_eq!(data["anthropic/claude-x"].cache_read, Some(0.3));
    }

    #[test]
    fn source_priority_and_provenance_are_deterministic() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "openai/gpt-x".into(),
            Price {
                input: Some(1.0),
                ..Default::default()
            },
        );
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "openai/gpt-x".into(),
            Price {
                input: Some(2.0),
                ..Default::default()
            },
        );
        let prices = merge_sources(vec![
            ("LiteLLM", litellm),
            ("OpenRouter", openrouter),
            ("Models.dev", HashMap::new()),
        ]);
        assert_eq!(prices["openai/gpt-x"]["input"], Value::from(1));
        assert_eq!(prices["gpt-x"]["source"], Value::from("LiteLLM"));
        assert_eq!(prices["gpt-x"]["matched_key"], Value::from("openai/gpt-x"));
    }

    #[test]
    fn cache_ttl_matches_tokscale() {
        assert_eq!(TTL_SECONDS, 3600);
    }
}
