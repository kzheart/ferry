//! 模型发现与用户扩展合并。

use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::adapters::contracts::ModelCatalog;
use crate::errors::DomainResult;
use crate::system::paths::home_dir;

/// 用户自定义模型清单位置。
pub fn models_config_path() -> PathBuf {
    home_dir().join(".ferry/models.json")
}

/// 读取用户扩展的模型 id 列表；文件缺失/损坏一律当作空。
///
/// 接受两种条目形态：非空字符串，或带非空 `id` 的对象。
pub fn user_model_ids(tool: &str) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(models_config_path()) else {
        return Vec::new();
    };
    let Ok(document) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let Some(raw) = document.get(tool).and_then(Value::as_array) else {
        return Vec::new();
    };
    raw.iter()
        .filter_map(|item| match item {
            Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
            Value::Object(entry) => {
                entry
                    .get("id")
                    .filter(|id| !matches!(id, Value::Null))
                    .map(|id| match id {
                        Value::String(text) => text.trim().to_string(),
                        other => other.to_string(),
                    })
            }
            _ => None,
        })
        .collect()
}

/// `models` RPC 的结果结构。
#[derive(Clone, Debug, PartialEq)]
pub struct ModelList {
    pub tool: String,
    pub default: Option<String>,
    pub models: Vec<Map<String, Value>>,
    pub source: String,
    /// discover 失败时的诊断文本（截断到 400 字符）。
    pub error: Option<String>,
    pub allow_custom: bool,
    pub config_path: String,
}

impl ModelList {
    pub fn to_value(&self) -> Value {
        let mut payload = Map::new();
        payload.insert("tool".into(), Value::from(self.tool.as_str()));
        payload.insert(
            "default".into(),
            self.default.as_deref().map_or(Value::Null, Value::from),
        );
        payload.insert(
            "models".into(),
            Value::Array(self.models.iter().cloned().map(Value::Object).collect()),
        );
        payload.insert("source".into(), Value::from(self.source.as_str()));
        payload.insert(
            "error".into(),
            self.error.as_deref().map_or(Value::Null, Value::from),
        );
        payload.insert("allow_custom".into(), Value::Bool(self.allow_custom));
        payload.insert("config_path".into(), Value::from(self.config_path.as_str()));
        Value::Object(payload)
    }
}

/// 组装 `models` 结果：discover 失败退回 fallback，再并入用户扩展并按 id 去重。
pub fn list_models(tool_name: &str, catalog: &dyn ModelCatalog) -> DomainResult<ModelList> {
    let (mut rows, source, default, error) = match catalog.discover() {
        Ok(discovery) => (discovery.rows, discovery.source, discovery.default, None),
        Err(failure) => {
            let message: String = failure.message().chars().take(400).collect();
            (
                catalog.fallback(),
                "fallback".to_string(),
                None,
                Some(message),
            )
        }
    };
    for model in user_model_ids(tool_name) {
        let mut row = Map::new();
        row.insert("id".into(), Value::from(model.as_str()));
        row.insert("label".into(), Value::from(model.as_str()));
        row.insert("source".into(), Value::from("user"));
        rows.push(row);
    }
    let mut seen: Vec<String> = Vec::new();
    let mut models = Vec::new();
    for row in rows {
        let Some(id) = row.get("id").and_then(Value::as_str) else {
            continue;
        };
        if id.is_empty() || seen.iter().any(|existing| existing == id) {
            continue;
        }
        seen.push(id.to_string());
        models.push(row);
    }
    Ok(ModelList {
        tool: tool_name.to_string(),
        default,
        models,
        source,
        error,
        allow_custom: true,
        config_path: models_config_path().to_string_lossy().into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::contracts::ModelDiscovery;
    use crate::errors::DomainError;

    struct Catalog {
        fail: bool,
    }

    fn row(id: &str) -> Map<String, Value> {
        let mut row = Map::new();
        row.insert("id".into(), Value::from(id));
        row
    }

    impl ModelCatalog for Catalog {
        fn discover(&self) -> DomainResult<ModelDiscovery> {
            if self.fail {
                return Err(DomainError::internal("boom"));
            }
            Ok(ModelDiscovery {
                rows: vec![row("a"), row("a"), row("b")],
                source: "cli".into(),
                default: Some("a".into()),
            })
        }

        fn fallback(&self) -> Vec<Map<String, Value>> {
            vec![row("fallback-1")]
        }
    }

    #[test]
    fn duplicate_ids_are_dropped_and_order_is_kept() {
        let result = list_models("claude", &Catalog { fail: false }).unwrap();
        let ids: Vec<&str> = result
            .models
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(result.source, "cli");
        assert_eq!(result.default.as_deref(), Some("a"));
        assert!(result.error.is_none());
        assert!(result.allow_custom);
    }

    #[test]
    fn discovery_failures_fall_back_without_raising() {
        let result = list_models("claude", &Catalog { fail: true }).unwrap();
        assert_eq!(result.source, "fallback");
        assert_eq!(result.error.as_deref(), Some("boom"));
        assert_eq!(result.models.len(), 1);
        assert!(result.default.is_none());
    }
}
