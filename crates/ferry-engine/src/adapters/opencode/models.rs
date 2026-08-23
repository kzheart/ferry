//! OpenCode CLI 模型发现。

use std::time::Duration;

use serde_json::{Map, Value};

use crate::adapters::contracts::{ModelCatalog, ModelDiscovery};
use crate::errors::{DomainError, DomainResult};
use crate::system::{executables, probes};

/// `opencode models` 的超时（Python `timeout=90`）。
const DISCOVER_TIMEOUT: Duration = Duration::from_secs(90);

/// 表格边框字符：CLI 有时把模型列表画成框，边框行不是模型。
const BORDER_PREFIXES: [char; 3] = ['┌', '│', '└'];

/// 跑一次 `opencode models`，逐行筛出模型 id。
pub fn discover() -> DomainResult<ModelDiscovery> {
    let argv = executables::argv("opencode", &["models"]);
    let output = probes::run(&argv, None, DISCOVER_TIMEOUT, None)
        .map_err(|error| DomainError::internal(error.message))?;
    if output.returncode != Some(0) {
        let raw = if !output.stderr.is_empty() {
            output.stderr.as_str()
        } else if !output.stdout.is_empty() {
            output.stdout.as_str()
        } else {
            "opencode models 失败"
        };
        return Err(DomainError::internal(
            raw.chars().take(300).collect::<String>(),
        ));
    }
    let rows = model_rows(&output.stdout);
    if rows.is_empty() {
        return Err(DomainError::internal("opencode models 未返回任何模型"));
    }
    Ok(ModelDiscovery {
        rows,
        source: "cli".into(),
        default: None,
    })
}

/// `opencode models` 的 stdout → 模型行。
fn model_rows(stdout: &str) -> Vec<Map<String, Value>> {
    let mut rows = Vec::new();
    for line in stdout.lines() {
        let model = line.trim();
        if model.is_empty() || model.starts_with(BORDER_PREFIXES) {
            continue;
        }
        // 带空格且不含斜杠的行是说明文字，不是 `provider/model` 形态的 id。
        if model.contains(' ') && !model.contains('/') {
            continue;
        }
        let mut row = Map::new();
        row.insert("id".into(), Value::from(model));
        row.insert("label".into(), Value::from(model));
        row.insert("source".into(), Value::from("cli"));
        rows.push(row);
    }
    rows
}

/// OpenCode 没有内置兜底模型表。
pub fn fallback() -> Vec<Map<String, Value>> {
    Vec::new()
}

/// `contracts::ModelCatalog` 的 opencode 实现。
pub struct OpenCodeModels;

impl ModelCatalog for OpenCodeModels {
    fn discover(&self) -> DomainResult<ModelDiscovery> {
        discover()
    }

    fn fallback(&self) -> Vec<Map<String, Value>> {
        fallback()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borders_and_prose_lines_are_filtered_out() {
        let stdout = "┌ providers\n\
                      │ ignored\n\
                      anthropic/claude-sonnet-4\n\
                      \n\
                      Available models below\n\
                      openai/gpt-5 pro\n\
                      bare-model\n\
                      └ end\n";
        let rows = model_rows(stdout);
        let ids: Vec<&str> = rows.iter().map(|row| row["id"].as_str().unwrap()).collect();
        // 带空格但含斜杠的行仍算模型（Python 的判定就是这么写的）。
        assert_eq!(
            ids,
            [
                "anthropic/claude-sonnet-4",
                "openai/gpt-5 pro",
                "bare-model"
            ]
        );
        assert_eq!(model_rows("").len(), 0);
    }
}
