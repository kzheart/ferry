import type {
  OperationInput,
  OperationKind,
  OperationStatus,
} from "./generated/operations.js";

export type { OperationInput };

export interface OperationPlan {
  plan_id: string;
  kind?: OperationKind;
  status?: OperationStatus;
  input?: OperationInput;
  preview?: Record<string, unknown>;
  [key: string]: unknown;
}

export interface OperationState {
  plan_id: string;
  status: OperationStatus;
  error_type?: string;
  error_message?: string;
  result?: unknown;
  [key: string]: unknown;
}
