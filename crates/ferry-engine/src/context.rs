//! 能力包共享的端口集合。
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
/// 这是历史契约值，与 crate 版本正交：`version` RPC 是宿主的对照面，
/// `CARGO_PKG_VERSION`（CLI 包版本，与 App 产品版本独立）不参与其中，
/// 三者不要互相同步。
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
