//! models.dev 模型单价：抓取 + 磁盘缓存，供前端估算成本。
//!
//! 返回扁平表 `{model_id: {input, output, cache_read, cache_write}}`，单位为每
//! 百万 token 的美元价。抓不到时退回上次缓存，再退回内置兜底表——始终返回可用
//! 的表，不因离线而报错（匹配不上的模型前端只记 token、不计价）。

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use super::paths::home_dir;

pub const MODELS_DEV_URL: &str = "https://models.dev/api.json";
pub const TTL_SECONDS: u64 = 7 * 24 * 3600;
const FETCH_TIMEOUT: Duration = Duration::from_secs(8);

/// 单价缓存文件位置。
pub fn cache_path() -> PathBuf {
    home_dir().join(".ferry").join("models-dev.json")
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

/// `pricing()` 的返回结构。
#[derive(Clone, Debug, PartialEq)]
pub struct Pricing {
    pub prices: Map<String, Value>,
    pub fetched_at: i64,
    /// cache | stale | network | fallback
    pub source: &'static str,
}

fn fallback_prices() -> Map<String, Value> {
    let mut prices = Map::new();
    for (model, input, output, cache_read, cache_write) in FALLBACK {
        let mut cost = Map::new();
        cost.insert("input".into(), number(*input));
        cost.insert("output".into(), number(*output));
        cost.insert("cache_read".into(), number(*cache_read));
        cost.insert("cache_write".into(), number(*cache_write));
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

fn cost_field(cost: &Map<String, Value>, key: &str) -> Value {
    match cost.get(key) {
        // Python 用 `cost.get(k) or 0`：0 / null / 缺失都落到 0。
        Some(value) if value.as_f64().is_some_and(|float| float != 0.0) => value.clone(),
        _ => Value::from(0),
    }
}

/// 把 models.dev 的分层响应压成 `{model_id: cost}`。
pub fn flatten(api: &Value) -> Map<String, Value> {
    let mut prices = Map::new();
    let Some(providers) = api.as_object() else {
        return prices;
    };
    for provider in providers.values() {
        let Some(models) = provider.get("models").and_then(Value::as_object) else {
            continue;
        };
        for (model_id, model) in models {
            let Some(cost) = model.get("cost").and_then(Value::as_object) else {
                continue;
            };
            if cost.is_empty() {
                continue;
            }
            let mut entry = Map::new();
            for key in ["input", "output", "cache_read", "cache_write"] {
                entry.insert(key.into(), cost_field(cost, key));
            }
            prices.insert(model_id.clone(), Value::Object(entry));
        }
    }
    prices
}

fn read_cache() -> Option<Value> {
    let text = std::fs::read_to_string(cache_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn cached_prices(cached: &Value) -> Option<Map<String, Value>> {
    cached
        .get("prices")
        .and_then(Value::as_object)
        .filter(|prices| !prices.is_empty())
        .cloned()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

fn fetch() -> Option<Value> {
    let response = ureq::get(MODELS_DEV_URL)
        .config()
        .timeout_global(Some(FETCH_TIMEOUT))
        .build()
        .header("User-Agent", "ferry/1.0")
        .call()
        .ok()?;
    let text = response.into_body().read_to_string().ok()?;
    serde_json::from_str(&text).ok()
}

/// 四级降级：cache（新鲜）→ network → stale → fallback。
///
/// `cached_only` 给在线请求路径用：那里不能为了刷新单价去等一次网络往返。
pub fn pricing(force: bool, cached_only: bool) -> Pricing {
    let cached = read_cache();
    if !force {
        if let Some(cached) = cached.as_ref() {
            let fetched_at = cached
                .get("fetched_at")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let age_seconds = (now_ms() - fetched_at) as f64 / 1000.0;
            if age_seconds < TTL_SECONDS as f64 {
                if let Some(prices) = cached.get("prices").and_then(Value::as_object) {
                    return Pricing {
                        prices: prices.clone(),
                        fetched_at,
                        source: "cache",
                    };
                }
            }
        }
    }
    if cached_only {
        return stale_or_fallback(cached.as_ref());
    }
    if let Some(api) = fetch() {
        let prices = flatten(&api);
        if !prices.is_empty() {
            let fetched_at = now_ms();
            let mut payload = Map::new();
            payload.insert("prices".into(), Value::Object(prices.clone()));
            payload.insert("fetched_at".into(), Value::from(fetched_at));
            let path = cache_path();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(
                &path,
                serde_json::to_string(&Value::Object(payload)).unwrap_or_default(),
            );
            return Pricing {
                prices,
                fetched_at,
                source: "network",
            };
        }
    }
    stale_or_fallback(cached.as_ref())
}

fn stale_or_fallback(cached: Option<&Value>) -> Pricing {
    if let Some(prices) = cached.and_then(cached_prices) {
        return Pricing {
            prices,
            fetched_at: cached
                .and_then(|value| value.get("fetched_at"))
                .and_then(Value::as_i64)
                .unwrap_or(0),
            source: "stale",
        };
    }
    Pricing {
        prices: fallback_prices(),
        fetched_at: 0,
        source: "fallback",
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
        // 缺失字段补 0（Python 的 `cost.get(k) or 0`）。
        assert_eq!(prices["claude-x"]["cache_read"], Value::from(0));
    }

    #[test]
    fn fallback_table_matches_the_python_constants() {
        let prices = fallback_prices();
        assert_eq!(prices.len(), 7);
        assert_eq!(prices["claude-opus-4"]["input"], Value::from(15));
        assert_eq!(prices["claude-opus-4"]["cache_write"], Value::from(18.75));
        assert_eq!(prices["gpt-5"]["input"], Value::from(1.25));
    }

    #[test]
    fn stale_beats_fallback_when_a_cache_exists() {
        let cached = json!({"prices": {"m": {"input": 1}}, "fetched_at": 5});
        let result = stale_or_fallback(Some(&cached));
        assert_eq!(result.source, "stale");
        assert_eq!(result.fetched_at, 5);
        assert_eq!(stale_or_fallback(None).source, "fallback");
        assert_eq!(
            stale_or_fallback(Some(&json!({"prices": {}}))).source,
            "fallback"
        );
    }
}
