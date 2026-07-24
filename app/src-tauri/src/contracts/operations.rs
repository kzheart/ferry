// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
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
