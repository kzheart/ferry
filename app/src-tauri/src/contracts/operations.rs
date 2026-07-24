// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const OPERATION_PLAN_ID_PREFIX: &str = "op_";
pub(crate) const OPERATION_KINDS: &[&str] =
    &["edit", "migration", "metadata", "delete", "restore-delete"];
pub(crate) const EDIT_OPERATION_KINDS: &[&str] =
    &["delete-turn", "rewrite", "replace-assistant-reply"];
pub(crate) const OPERATION_STATUSES: &[&str] = &[
    "planned",
    "queued",
    "applying",
    "applied",
    "failed",
    "cancelled",
    "expired",
];
pub(crate) const OPERATION_TERMINAL_STATUSES: &[&str] =
    &["applied", "failed", "cancelled", "expired"];
pub(crate) const OPERATION_SUCCESS_STATUS: &str = "applied";

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

impl OperationPlanInput {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Edit(_) => "edit",
            Self::Migration(_) => "migration",
            Self::Metadata(_) => "metadata",
            Self::Delete(_) => "delete",
            Self::RestoreDelete(_) => "restore-delete",
        }
    }
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
