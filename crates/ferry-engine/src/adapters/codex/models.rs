//! Codex CLI 模型发现。

use std::time::Duration;

use serde_json::{Map, Value};

use crate::adapters::contracts::{ModelCatalog, ModelDiscovery};
use crate::errors::{DomainError, DomainResult};
use crate::system::paths::home_dir;
use crate::system::{executables, probes};

/// `codex debug models` 与 `~/.codex/config.toml` 的组合发现。
pub struct CodexModels;

fn row_model(row: &Value) -> Option<Map<String, Value>> {
    let entries = row.as_object()?;
    let slug = entries
        .get("slug")
        .filter(|value| !value.is_null())
        .or_else(|| entries.get("id"))
        .filter(|value| truthy(value))?;
    let slug = text(slug);
    let visibility = entries
        .get("visibility")
        .filter(|value| truthy(value))
        .map(text)
        .unwrap_or_else(|| "list".to_string());
    let mut label = entries
        .get("display_name")
        .filter(|value| truthy(value))
        .map(text)
        .unwrap_or_else(|| slug.clone());
    if visibility != "list" {
        label = format!("{label} ({visibility})");
    }
    let mut model = Map::new();
    model.insert("id".into(), Value::from(slug));
    model.insert("label".into(), Value::from(label));
    model.insert("source".into(), Value::from("cli"));
    Some(model)
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => crate::adapters::shared::dialect::python_str(other),
    }
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

/// `~/.codex/config.toml` 里的默认模型：首个 `model = ...`（排除 `model_*`）。
fn default_model() -> Option<String> {
    let raw = std::fs::read_to_string(home_dir().join(".codex/config.toml")).ok()?;
    for line in raw.lines() {
        let text = line.trim();
        if !text.starts_with("model") || !text.contains('=') || text.starts_with("model_") {
            continue;
        }
        let value = text
            .split_once('=')?
            .1
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        return Some(value.to_string()).filter(|value| !value.is_empty());
    }
    None
}

impl ModelCatalog for CodexModels {
    fn discover(&self) -> DomainResult<ModelDiscovery> {
        let command = executables::argv("codex", &["debug", "models"]);
        let result = probes::run(&command, None, Duration::from_secs(60), None)
            .map_err(|error| DomainError::internal(error.message))?;
        if result.returncode != Some(0) {
            let detail = if !result.stderr.is_empty() {
                result.stderr.as_str()
            } else if !result.stdout.is_empty() {
                result.stdout.as_str()
            } else {
                "codex debug models 失败"
            };
            return Err(DomainError::internal(
                detail.chars().take(300).collect::<String>(),
            ));
        }
        let data: Value = serde_json::from_str(&result.stdout).map_err(|error| {
            DomainError::internal(format!("codex debug models 输出异常: {error}"))
        })?;
        let rows = match &data {
            Value::Object(entries) => entries.get("models").cloned().unwrap_or(Value::Null),
            other => other.clone(),
        };
        let Value::Array(rows) = rows else {
            return Err(DomainError::internal("codex debug models 输出格式异常"));
        };
        let models: Vec<Map<String, Value>> = rows.iter().filter_map(row_model).collect();
        Ok(ModelDiscovery {
            rows: models,
            source: "cli".to_string(),
            default: default_model(),
        })
    }

    fn fallback(&self) -> Vec<Map<String, Value>> {
        ["gpt-5.4", "gpt-5.5", "o3"]
            .into_iter()
            .map(|model| {
                let mut row = Map::new();
                row.insert("id".into(), Value::from(model));
                row.insert("label".into(), Value::from(model));
                row.insert("source".into(), Value::from("fallback"));
                row
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rows_without_a_slug_are_skipped() {
        assert!(row_model(&json!({"display_name": "X"})).is_none());
        assert!(row_model(&json!("not-an-object")).is_none());
    }

    #[test]
    fn non_listed_models_carry_their_visibility_in_the_label() {
        assert_eq!(
            row_model(&json!({"slug": "gpt-5.4", "display_name": "GPT 5.4"})).unwrap(),
            json!({"id": "gpt-5.4", "label": "GPT 5.4", "source": "cli"})
                .as_object()
                .unwrap()
                .clone()
        );
        assert_eq!(
            row_model(&json!({"id": "o3", "visibility": "hidden"})).unwrap()["label"],
            json!("o3 (hidden)")
        );
    }

    #[test]
    fn fallback_lists_the_three_known_models() {
        let rows = CodexModels.fallback();
        let ids: Vec<&str> = rows.iter().map(|row| row["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["gpt-5.4", "gpt-5.5", "o3"]);
        assert!(rows.iter().all(|row| row["source"] == json!("fallback")));
    }
}
