//! Installed session-source executable inspection.

use std::time::Duration;

use serde_json::{Map, Value};

use crate::adapters::registry::AdapterRegistry;
use crate::system::{executables, probes};

/// 单个 Agent 的可执行文件探测结果。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolPresence {
    pub installed: bool,
    pub path: Option<String>,
    /// 定位到了却跑不起来（如 Node 版本不达标），与「未安装」区分。
    pub broken: bool,
}

impl ToolPresence {
    pub fn to_value(&self) -> Value {
        let mut payload = Map::new();
        payload.insert("installed".into(), Value::Bool(self.installed));
        payload.insert(
            "path".into(),
            self.path.as_deref().map_or(Value::Null, Value::from),
        );
        payload.insert("broken".into(), Value::Bool(self.broken));
        Value::Object(payload)
    }
}

const VERSION_TIMEOUT: Duration = Duration::from_secs(20);

/// 逐个 adapter 探测 manifest 里的首个可执行文件。
pub fn inspect(registry: &AdapterRegistry) -> Map<String, Value> {
    let mut out = Map::new();
    for tool in registry.ids() {
        let mut info = ToolPresence::default();
        if let Ok(adapter) = registry.get(tool) {
            if let Some(executable) = adapter.manifest.executables.first() {
                if let Some(resolved) = executables::resolve(executable) {
                    info.path = Some(resolved.clone());
                    if let Ok(output) = probes::run(
                        &[resolved, "--version".to_string()],
                        None,
                        VERSION_TIMEOUT,
                        None,
                    ) {
                        info.installed = output.returncode == Some(0);
                        info.broken = output.returncode != Some(0);
                    }
                }
            }
        }
        out.insert(tool.clone(), info.to_value());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_executables_report_neither_installed_nor_broken() {
        let payload = ToolPresence::default().to_value();
        assert_eq!(payload["installed"], Value::Bool(false));
        assert_eq!(payload["broken"], Value::Bool(false));
        assert_eq!(payload["path"], Value::Null);
    }

    #[test]
    fn empty_registry_yields_an_empty_report() {
        assert!(inspect(&AdapterRegistry::default()).is_empty());
    }
}
