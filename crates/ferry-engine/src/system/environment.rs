//! Installed session-source inspection.
//!
//! 「已检测」以会话库为准，CLI 为辅。Cursor 这类 GUI Agent 的 `--version` 在
//! Windows 上经常是 `.cmd` 垫片或直接拉起 Electron，不能当作安装信号。
//! 同类项目（drift-connectors、cursaves）也是看 `state.vscdb` 在不在。

use std::time::Duration;

use serde_json::{Map, Value};

use crate::adapters::registry::AdapterRegistry;
use crate::system::paths::{display_path, expand_location};
use crate::system::{executables, probes};

/// 单个 Agent 的可执行文件 / 会话库探测结果。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolPresence {
    /// 会话库存在，或 CLI `--version` 成功。UI 徽章看这个。
    pub installed: bool,
    pub path: Option<String>,
    /// 定位到了却跑不起来（如 Node 版本不达标），与「未安装」区分。
    pub broken: bool,
    /// 契约 `source_path` 展开后的目录/文件存在。
    pub store: bool,
    /// PATH / 兜底目录上找到了 CLI。
    pub cli: bool,
    pub store_path: Option<String>,
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
        payload.insert("store".into(), Value::Bool(self.store));
        payload.insert("cli".into(), Value::Bool(self.cli));
        payload.insert(
            "store_path".into(),
            self.store_path.as_deref().map_or(Value::Null, Value::from),
        );
        Value::Object(payload)
    }
}

const VERSION_TIMEOUT: Duration = Duration::from_secs(20);

/// 逐个 adapter：会话库表示“有历史可浏览”，CLI 探针表示“可执行操作”。
pub fn inspect(registry: &AdapterRegistry) -> Map<String, Value> {
    let mut out = Map::new();
    for tool in registry.ids() {
        let mut info = ToolPresence::default();
        if let Ok(adapter) = registry.get(tool) {
            if !adapter.manifest.source_path.is_empty() {
                let store_path = expand_location(&adapter.manifest.source_path);
                info.store = store_path.exists();
                info.store_path = Some(display_path(&store_path));
                if info.store {
                    info.path = info.store_path.clone();
                }
            }
            if let Some(executable) = adapter.manifest.executables.first() {
                if let Some(resolved) = executables::resolve(executable) {
                    info.cli = true;
                    if info.path.is_none() {
                        info.path = Some(resolved.clone());
                    }
                    if let Ok(output) = probes::run(
                        &[resolved, "--version".to_string()],
                        None,
                        VERSION_TIMEOUT,
                        None,
                    ) {
                        info.installed = output.returncode == Some(0);
                        info.broken = output.returncode != Some(0);
                        if !info.installed {
                            info.cli = false;
                        }
                    }
                }
            }
        }
        out.insert(tool.clone(), info.to_value());
    }
    out
}
