//! 能力包共享的端口集合（`engine/context.py::EngineContext` 的等价物）。
//!
//! Python 侧 `EngineContext` 是一个装了五个可调用对象的 dataclass；Rust 侧
//! 各能力包只依赖自己那份窄 trait（`sessions::index::SessionPorts` 与
//! `operations::types::OperationPorts`），本结构是它们唯一的生产实现。
//!
//! 两个刻意保留的 Python 语义：
//! 1. `state_dir()` / `cache_factory()` 每次调用都现取——Python 那边存的是函数
//!    而不是求值结果，`FERRY_DATA_DIR` 之类的环境变量在进程运行期改动也要生效
//!    （测试依赖这一点）；
//! 2. `version` 是 `engine.__version__`，与 crate 版本无关（见 [`ENGINE_VERSION`]）。

use std::path::PathBuf;
use std::sync::Arc;

use crate::adapters::contracts::{AgentAdapter, ScanCache};
use crate::adapters::registry::AdapterRegistry;
use crate::errors::DomainResult;
use crate::operations::types::OperationPorts;
use crate::sessions::index::SessionPorts;
use crate::sessions::scan_cache::shared_cache;
use crate::system::snapshots::data_dir;

/// 引擎对外自称的版本号。
///
/// **必须逐字等于 `engine/__init__.py::__version__`**：`version` RPC 是宿主与
/// 等价性脚本的对照面，Rust crate 自己的 `CARGO_PKG_VERSION`（跟随 app 版本）
/// 与它无关。
pub const ENGINE_VERSION: &str = "0.1.0";

/// 组合根持有的运行上下文。
pub struct EngineContext {
    registry: AdapterRegistry,
    version: String,
}

impl EngineContext {
    pub fn new(registry: AdapterRegistry, version: impl Into<String>) -> Self {
        Self {
            registry,
            version: version.into(),
        }
    }

    pub fn registry(&self) -> &AdapterRegistry {
        &self.registry
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

impl SessionPorts for EngineContext {
    fn adapter(&self, name: &str) -> DomainResult<&AgentAdapter> {
        self.registry.get(name)
    }

    fn adapters(&self) -> Vec<String> {
        self.registry.ids().to_vec()
    }

    fn cache_factory(&self) -> Arc<dyn ScanCache> {
        shared_cache()
    }
}

impl OperationPorts for EngineContext {
    fn adapter(&self, tool: &str) -> DomainResult<AgentAdapter> {
        self.registry.get(tool).cloned()
    }

    fn adapters(&self) -> Vec<String> {
        self.registry.ids().to_vec()
    }

    fn state_dir(&self) -> PathBuf {
        data_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_version_tracks_the_python_engine_not_the_crate() {
        // crate 版本跟随 app（0.6.x），引擎自称版本跟随 engine/__init__.py。
        assert_eq!(ENGINE_VERSION, "0.1.0");
        assert_ne!(ENGINE_VERSION, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn empty_registry_reports_no_adapters() {
        let context = EngineContext::new(AdapterRegistry::default(), ENGINE_VERSION);
        assert!(SessionPorts::adapters(&context).is_empty());
        assert!(OperationPorts::adapter(&context, "claude").is_err());
    }
}
