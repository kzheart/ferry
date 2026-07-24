//! 写操作计划的跨进程输入契约。
//!
//! 本模块只表示 WebView 可以提交给可信 Host 的意图；校验、审批与 Engine
//! 调用仍分别由 sidecar 和 policy 层负责，避免把会话格式知识带入 Rust。

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind")]
pub(crate) enum OperationPlanInput {
    #[serde(rename = "edit")]
    Edit(EditOperationPlanInput),
    #[serde(rename = "migration")]
    Migration(MigrationOperationPlanInput),
    #[serde(rename = "metadata")]
    Metadata(MetadataOperationPlanInput),
    #[serde(rename = "delete")]
    Delete(DeleteOperationPlanInput),
    #[serde(rename = "restore-delete")]
    RestoreDelete(RestoreDeleteOperationPlanInput),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditOperationPlanInput {
    pub(crate) tool: String,
    #[serde(rename = "ref")]
    pub(crate) reference: String,
    pub(crate) ops: Vec<Value>,
    #[serde(default)]
    pub(crate) probe: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MigrationOperationPlanInput {
    pub(crate) source_tool: String,
    #[serde(rename = "ref")]
    pub(crate) reference: String,
    pub(crate) target_tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_turn: Option<u32>,
    #[serde(default)]
    pub(crate) probe: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) probe_model: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetadataOperationPlanInput {
    pub(crate) tool: String,
    #[serde(rename = "ref")]
    pub(crate) reference: String,
    pub(crate) patch: MetadataPatch,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteOperationPlanInput {
    pub(crate) tool: String,
    #[serde(rename = "ref")]
    pub(crate) reference: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestoreDeleteOperationPlanInput {
    pub(crate) recovery_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetadataPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pinned: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tags: Option<Vec<String>>,
}
