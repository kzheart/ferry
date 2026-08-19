//! 组合根：装配 AdapterRegistry / 索引 / 内容索引 / 操作服务 / EngineService。
//!
//! 语义事实源：`engine/bootstrap.py`。
//!
//! 装配顺序与 Python 逐行对应：
//! `create_registry()`（5 个 adapter 依 `AGENT_IDS` 顺序 build）→ `EngineContext`
//! → `AgentSessionIndex` → `OperationService` → `EngineService`。

use std::sync::Arc;

use crate::adapters::registry::create_registry;
use crate::app::{Engine, IndexResolver};
use crate::context::{EngineContext, ENGINE_VERSION};
use crate::operations::service::OperationService;
use crate::operations::types::{Ports, Resolver};
use crate::server::cli::CliDeps;
use crate::server::rpc::EngineService;
use crate::sessions::content_index::ContentIndex;
use crate::sessions::index::{AgentSessionIndex, SessionPorts};

/// 等价 `bootstrap.create_context()`。
pub fn create_context() -> Result<Arc<EngineContext>, String> {
    Ok(Arc::new(EngineContext::new(
        create_registry()?,
        ENGINE_VERSION,
    )))
}

/// 等价 `bootstrap.build_engine(ports=None)`。
pub fn build_engine(ports: Option<Arc<EngineContext>>) -> Result<Arc<Engine>, String> {
    let ports = match ports {
        Some(ports) => ports,
        None => create_context()?,
    };
    let index = Arc::new(AgentSessionIndex::new(
        Arc::clone(&ports) as Arc<dyn SessionPorts>
    ));
    let operations = OperationService::new(
        Arc::clone(&ports) as Ports,
        Arc::new(IndexResolver::new(Arc::clone(&index))) as Resolver,
    );
    Ok(Arc::new(Engine::new(
        ports,
        index,
        operations,
        Some(Arc::new(ContentIndex::new(None))),
    )))
}

/// CLI 入口用的依赖包：把 `warm_agent_search` / `enable_live_updates` / `close`
/// 三个钩子接到同一个 [`Engine`] 实例上。
pub fn build_cli_deps() -> Result<CliDeps, String> {
    let engine = build_engine(None)?;
    let service: Arc<dyn EngineService> = Arc::clone(&engine) as Arc<dyn EngineService>;
    let warm = Arc::clone(&engine);
    let live = Arc::clone(&engine);
    let close = Arc::clone(&engine);
    Ok(CliDeps {
        service,
        warm_agent_search: Some(Arc::new(move || warm.warm_agent_search())),
        enable_live_updates: Some(Arc::new(move |notifier| live.enable_live_updates(notifier))),
        close: Some(Arc::new(move || close.close())),
    })
}
