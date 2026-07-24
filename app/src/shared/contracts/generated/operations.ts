// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const OPERATION_PLAN_ID_PREFIX = "op_" as const;
export const OPERATION_KINDS = [
  "edit",
  "migration",
  "metadata",
  "delete",
  "restore-delete",
] as const;
export const EDIT_OPERATION_KINDS = [
  "delete-turn",
  "rewrite",
  "replace-assistant-reply",
] as const;
export const OPERATION_STATUSES = [
  "planned",
  "queued",
  "applying",
  "applied",
  "failed",
  "cancelled",
  "expired",
] as const;
export const OPERATION_TERMINAL_STATUSES = [
  "applied",
  "failed",
  "cancelled",
  "expired",
] as const;
export const OPERATION_SUCCESS_STATUS = "applied" as const;
export type OperationKind = (typeof OPERATION_KINDS)[number];
export type EditOperationKind = (typeof EDIT_OPERATION_KINDS)[number];
export type OperationStatus = (typeof OPERATION_STATUSES)[number];
