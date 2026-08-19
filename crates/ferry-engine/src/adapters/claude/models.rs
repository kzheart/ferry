//! Claude 模型发现。

use serde_json::{Map, Value};

use crate::adapters::contracts::{ModelCatalog, ModelDiscovery};
use crate::adapters::shared::dialect::python_str;
use crate::errors::DomainResult;
use crate::system::paths::home_dir;

/// `(id, label)`；顺序即下发顺序。
pub const ALIASES: &[(&str, &str)] = &[
    ("default", "默认(账号推荐)"),
    ("best", "best"),
    ("fable", "fable · Fable 5"),
    ("opus", "opus"),
    ("sonnet", "sonnet"),
    ("haiku", "haiku"),
    ("opus[1m]", "opus[1m]"),
    ("sonnet[1m]", "sonnet[1m]"),
    ("opusplan", "opusplan"),
];

fn row(id: &str, label: &str, source: &str) -> Map<String, Value> {
    let mut item = Map::new();
    item.insert("id".into(), Value::from(id));
    item.insert("label".into(), Value::from(label));
    item.insert("source".into(), Value::from(source));
    item
}

fn read_json(path: &std::path::Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// 别名清单 + `settings.json` 里的默认模型 + `~/.claude.json` 的缓存条目。
pub fn discover() -> ModelDiscovery {
    let mut rows: Vec<Map<String, Value>> = ALIASES
        .iter()
        .map(|(id, label)| row(id, label, "alias"))
        .collect();

    let home = home_dir();
    let mut default = None;
    for name in [".claude/settings.json", ".claude/settings.local.json"] {
        let Some(config) = read_json(&home.join(name)) else {
            continue;
        };
        // Python 的 `config.get("model")` 走真值判断：空串/0/null 都不算默认值。
        if let Some(model) = config.as_object().and_then(|entries| entries.get("model")) {
            if truthy(model) {
                default = Some(python_str(model));
                break;
            }
        }
    }

    if let Some(cache) = read_json(&home.join(".claude.json"))
        .as_ref()
        .and_then(|value| value.get("additionalModelOptionsCache"))
        .and_then(Value::as_array)
    {
        for item in cache {
            let Some(entries) = item.as_object() else {
                continue;
            };
            let Some(value) = entries.get("value").filter(|value| truthy(value)) else {
                continue;
            };
            let id = python_str(value);
            let label = entries
                .get("label")
                .filter(|label| truthy(label))
                .map_or_else(|| id.clone(), python_str);
            rows.push(row(&id, &label, "cache"));
        }
    }

    ModelDiscovery {
        rows,
        source: "alias".to_string(),
        default,
    }
}

/// 发现失败时的兜底清单。
pub fn fallback() -> Vec<Map<String, Value>> {
    ALIASES
        .iter()
        .map(|(id, label)| row(id, label, "fallback"))
        .collect()
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

/// `contracts::ModelCatalog` 的 claude 实现。
pub struct ClaudeModels;

impl ModelCatalog for ClaudeModels {
    fn discover(&self) -> DomainResult<ModelDiscovery> {
        Ok(discover())
    }

    fn fallback(&self) -> Vec<Map<String, Value>> {
        fallback()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_lists_every_alias() {
        let rows = fallback();
        assert_eq!(rows.len(), ALIASES.len());
        assert_eq!(rows[0]["id"], Value::from("default"));
        assert_eq!(rows[0]["label"], Value::from("默认(账号推荐)"));
        assert!(rows.iter().all(|row| row["source"] == "fallback"));
    }

    /// discover 依赖真实 HOME，这里只固定「别名部分恒定、source 恒为 alias」。
    #[test]
    fn discover_always_starts_with_the_aliases() {
        let discovery = discover();
        assert_eq!(discovery.source, "alias");
        assert!(discovery.rows.len() >= ALIASES.len());
        for (index, (id, _)) in ALIASES.iter().enumerate() {
            assert_eq!(discovery.rows[index]["id"], Value::from(*id));
            assert_eq!(discovery.rows[index]["source"], Value::from("alias"));
        }
    }
}
