//! Grok 模型目录。

use std::path::Path;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::adapters::contracts::{ModelCatalog, ModelDiscovery};
use crate::errors::{DomainError, DomainResult};
use crate::system::{executables, probes};

/// CLI 不可用时的兜底目录。
const FALLBACK: [(&str, &str); 1] = [("grok-code-fast-1", "grok-code-fast-1")];

fn row(id: &str, label: &str, source: &str) -> Map<String, Value> {
    let mut entry = Map::new();
    entry.insert("id".into(), Value::from(id));
    entry.insert("label".into(), Value::from(label));
    entry.insert("source".into(), Value::from(source));
    entry
}

/// `grok models` 的输出逐行取首个 token 当模型 id，整行当 label。
pub fn discover() -> DomainResult<ModelDiscovery> {
    let command = executables::argv("grok", &["models"]);
    let result = probes::run(&command, None::<&Path>, Duration::from_secs(15), None)
        .map_err(|error| DomainError::internal(error.message))?;
    if result.returncode != Some(0) {
        let reason: String = if result.stderr.is_empty() {
            "grok models failed".to_string()
        } else {
            result.stderr.clone()
        };
        return Err(DomainError::internal(
            reason.chars().take(400).collect::<String>(),
        ));
    }
    let mut rows = Vec::new();
    for line in result.stdout.split('\n') {
        let trimmed = line.trim();
        let value = trimmed.split_whitespace().next().unwrap_or("");
        if value.is_empty() {
            continue;
        }
        let lowered = value.to_lowercase();
        if lowered == "model" || lowered == "models" {
            continue;
        }
        rows.push(row(value, trimmed, "cli"));
    }
    let default = rows
        .first()
        .and_then(|first| first.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(ModelDiscovery {
        rows,
        source: "cli".to_string(),
        default,
    })
}

pub fn fallback() -> Vec<Map<String, Value>> {
    FALLBACK
        .iter()
        .map(|(id, label)| row(id, label, "fallback"))
        .collect()
}

/// [`ModelCatalog`] 的 grok 实现。
pub struct GrokModels;

impl ModelCatalog for GrokModels {
    fn discover(&self) -> DomainResult<ModelDiscovery> {
        discover()
    }

    fn fallback(&self) -> Vec<Map<String, Value>> {
        fallback()
    }
}
