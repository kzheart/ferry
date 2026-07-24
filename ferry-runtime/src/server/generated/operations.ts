// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
import type { AgentId } from "./agents.js";

export interface TextReplyItem {
  kind: "text";
  text: string;
}

export interface ToolReplyItem {
  kind: "tool";
  name: string;
  input: Record<string, unknown> | string;
  output: string;
}

export type AssistantReplyItem = TextReplyItem | ToolReplyItem;
export interface AssistantReply {
  items: AssistantReplyItem[];
}

export interface MetadataPatch {
  name?: string;
  pinned?: boolean;
  archived?: boolean;
  tags?: string[];
}

export interface DeleteTurnOperation {
  op: "delete-turn";
  turn: number;
}

export interface RewriteOperation {
  op: "rewrite";
  locator: string;
  text: string;
}

export interface ReplaceAssistantReplyOperation {
  op: "replace-assistant-reply";
  turn: number | string;
  reply: AssistantReply;
}

export type EditOperation =
  | DeleteTurnOperation
  | RewriteOperation
  | ReplaceAssistantReplyOperation;

export interface EditOperationInput {
  kind: "edit";
  tool: AgentId;
  ref: string;
  ops: EditOperation[];
  probe?: boolean;
}

export interface MigrationOperationInput {
  kind: "migration";
  source_tool: AgentId;
  ref: string;
  target_tool: AgentId;
  max_turn?: number;
  probe?: boolean;
  probe_model?: string;
}

export interface MetadataOperationInput {
  kind: "metadata";
  tool: AgentId;
  ref: string;
  patch: MetadataPatch;
}

export interface DeleteOperationInput {
  kind: "delete";
  tool: AgentId;
  ref: string;
}

export interface RestoreDeleteOperationInput {
  kind: "restore-delete";
  recovery_id: string;
}

export type OperationInput =
  | EditOperationInput
  | MigrationOperationInput
  | MetadataOperationInput
  | DeleteOperationInput
  | RestoreDeleteOperationInput;
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
