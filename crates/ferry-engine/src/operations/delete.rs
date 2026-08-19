//! 会话删除用例：永久删除，不留恢复快照。
//!
//! 语义事实源：`engine/operations/delete.py`。

use serde_json::{Map, Value};

use crate::operations::types::{EngineResult, Ports};

pub struct SessionDeletionService {
    ports: Ports,
}

impl SessionDeletionService {
    pub fn new(ports: Ports) -> Self {
        Self { ports }
    }

    /// `lifecycle.delete(adapter, reference)`；能力门禁走 `delete`。
    pub fn delete(&self, tool: &str, reference: &str) -> EngineResult<Map<String, Value>> {
        let adapter = self.ports.adapter(tool)?;
        let lifecycle = adapter.require_lifecycle("delete")?;
        Ok(lifecycle.delete(&adapter, reference)?)
    }
}
