// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const OPERATION_PLAN_ID_PREFIX = "op_";
export const OPERATION_KINDS = Object.freeze([
  "edit",
  "migration",
  "metadata",
  "delete",
  "restore-delete",
]);
export const EDIT_OPERATION_KINDS = Object.freeze([
  "delete-turn",
  "rewrite",
  "replace-assistant-reply",
]);
export const OPERATION_STATUSES = Object.freeze([
  "planned",
  "queued",
  "applying",
  "applied",
  "failed",
  "cancelled",
  "expired",
]);
export const OPERATION_TERMINAL_STATUSES = Object.freeze([
  "applied",
  "failed",
  "cancelled",
  "expired",
]);
export const OPERATION_SUCCESS_STATUS = "applied";
