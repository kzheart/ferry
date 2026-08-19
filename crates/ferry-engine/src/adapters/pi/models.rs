//! Pi 模型清单：只读配置与 CLI 自述，不碰任何凭证。
//!
//! 语义事实源：`engine/adapters/pi/models.py`。

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::adapters::contracts::{ModelCatalog, ModelDiscovery};
use crate::adapters::shared::dialect::python_str;
use crate::errors::DomainResult;
use crate::system::executables;
use crate::system::paths::{expanduser, home_dir, process_environ};
use crate::system::probes;

use super::tool_calls::truthy;

/// `PI_CODING_AGENT_DIR`，默认 `~/.pi/agent`。
fn agent_dir() -> PathBuf {
    match std::env::var("PI_CODING_AGENT_DIR") {
        Ok(value) if !value.is_empty() => expanduser(&value),
        _ => home_dir().join(".pi").join("agent"),
    }
}

fn read_json(path: &std::path::Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or(Value::Null)
}

fn row(id: &str, label: &str, source: &str) -> Map<String, Value> {
    let mut entry = Map::new();
    entry.insert("id".into(), Value::from(id));
    entry.insert("label".into(), Value::from(label));
    entry.insert("source".into(), Value::from(source));
    entry
}

/// `settings.json` 里的默认模型；`provider/model` 拼接，没有 provider 就只用 model。
fn default_model(settings: &Value) -> Option<String> {
    if !settings.is_object() {
        return None;
    }
    let model = settings.get("defaultModel").filter(|value| truthy(value))?;
    Some(match settings.get("defaultProvider") {
        Some(provider) if !provider.is_null() => {
            format!("{}/{}", python_str(provider), python_str(model))
        }
        _ => python_str(model),
    })
}

/// `models.json` 里的用户自定义 provider/model。
fn custom_rows(custom: &Value) -> Vec<Map<String, Value>> {
    let mut rows = Vec::new();
    let Some(providers) = custom.get("providers").and_then(Value::as_object) else {
        return rows;
    };
    for (provider_id, provider) in providers {
        let Some(models) = provider.get("models").and_then(Value::as_array) else {
            continue;
        };
        for model in models {
            let Some(id) = model.get("id").filter(|value| truthy(value)) else {
                continue;
            };
            let model_id = format!("{provider_id}/{}", python_str(id));
            let label = match model.get("name") {
                Some(name) if truthy(name) => python_str(name),
                _ => model_id.clone(),
            };
            rows.push(row(&model_id, &label, "models.json"));
        }
    }
    rows
}

/// `pi --list-models` 的输出行：`<provider> <model> ...`，表头行跳过。
fn cli_rows() -> Vec<Map<String, Value>> {
    let Ok(config) = tempfile::tempdir() else {
        return Vec::new();
    };
    let source_settings = agent_dir().join("settings.json");
    if source_settings.is_file() {
        let _ = std::fs::copy(&source_settings, config.path().join("settings.json"));
    }
    let mut environ = process_environ();
    for (key, value) in [
        (
            "PI_CODING_AGENT_DIR",
            config.path().to_string_lossy().into_owned(),
        ),
        ("PI_OFFLINE", "1".into()),
        ("PI_SKIP_VERSION_CHECK", "1".into()),
        ("PI_TELEMETRY", "0".into()),
    ] {
        environ.insert(key.to_string(), value);
    }
    let env: Vec<(String, String)> = environ.into_iter().collect();
    let command = executables::argv(
        "pi",
        &[
            "--list-models",
            "--offline",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-themes",
            "--no-context-files",
            "--no-approve",
        ],
    );
    let Ok(output) = probes::run(&command, None, Duration::from_secs(10), Some(&env)) else {
        return Vec::new();
    };
    output
        .stdout
        .lines()
        .filter_map(|line| {
            let columns: Vec<&str> = line.split_whitespace().collect();
            if columns.len() < 2 || matches!(columns[0], "provider" | "Provider") {
                return None;
            }
            let model_id = format!("{}/{}", columns[0], columns[1]);
            Some(row(&model_id, line.trim(), "cli"))
        })
        .collect()
}

pub fn discover() -> ModelDiscovery {
    let directory = agent_dir();
    let default = default_model(&read_json(&directory.join("settings.json")));
    let mut rows = custom_rows(&read_json(&directory.join("models.json")));
    rows.extend(cli_rows());
    if let Some(default) = default.as_deref() {
        let known = rows
            .iter()
            .any(|entry| entry.get("id").and_then(Value::as_str) == Some(default));
        if !known {
            rows.push(row(default, default, "settings"));
        }
    }
    let source = if rows.is_empty() { "settings" } else { "cli" };
    ModelDiscovery {
        rows,
        source: source.to_string(),
        default,
    }
}

pub struct PiModels;

impl ModelCatalog for PiModels {
    fn discover(&self) -> DomainResult<ModelDiscovery> {
        Ok(discover())
    }

    /// pi 没有静态兜底清单：模型全靠 CLI/配置自述。
    fn fallback(&self) -> Vec<Map<String, Value>> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_model_joins_provider_and_model() {
        assert_eq!(
            default_model(&json!({"defaultProvider": "anthropic", "defaultModel": "opus"})),
            Some("anthropic/opus".to_string())
        );
        assert_eq!(
            default_model(&json!({"defaultModel": "opus"})),
            Some("opus".to_string())
        );
        assert_eq!(default_model(&json!({"defaultProvider": "x"})), None);
        assert_eq!(default_model(&json!([])), None);
    }

    #[test]
    fn custom_rows_come_from_models_json() {
        let rows = custom_rows(&json!({"providers": {
            "acme": {"models": [{"id": "m1", "name": "Model One"}, {"id": "m2"}]},
            "broken": {"models": "nope"},
        }}));
        let ids: Vec<&str> = rows.iter().map(|row| row["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["acme/m1", "acme/m2"]);
        assert_eq!(rows[0]["label"], json!("Model One"));
        // 缺 name 时回落成 id。
        assert_eq!(rows[1]["label"], json!("acme/m2"));
        assert_eq!(rows[0]["source"], json!("models.json"));
    }

    #[test]
    fn fallback_is_empty() {
        assert!(PiModels.fallback().is_empty());
    }
}
