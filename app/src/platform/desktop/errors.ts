import i18n from "../../i18n/index.js";
import {
  FERRY_ERROR_POLICIES,
  isFerryErrorCode,
} from "../../shared/contracts/generated/errors.js";
import type { IpcError } from "../../shared/contracts/generated/ipc.js";

type ErrorParams = Record<string, unknown>;

function translateError(code: string, params: ErrorParams) {
  if (code === "edit.operation_unsupported") {
    if (params.capability) {
      return i18n.t("errors:edit.operation_unsupported_with_capability", {
        tool: params.tool ?? "",
        capability: params.capability,
      });
    }
    return i18n.t("errors:edit.operation_unsupported_with_operation", {
      tool: params.tool ?? "",
      operation: params.operation ?? "",
      mode: params.mode ? `（${String(params.mode)}）` : "",
    });
  }
  if (code === "edit.turn_out_of_range") {
    if (params.turn_count != null) {
      return i18n.t("errors:edit.turn_out_of_range_with_count", {
        requested_turn: params.requested_turn,
        turn_count: params.turn_count,
      });
    }
    return i18n.t("errors:edit.turn_out_of_range_invalid");
  }
  if (code === "probe.process_failed") {
    if (params.exit_code != null) {
      return i18n.t("errors:probe.process_failed_with_code", {
        exit_code: params.exit_code,
      });
    }
    return i18n.t("errors:probe.process_failed", { exit_code: "" });
  }
  const key = `errors:${code}`;
  const fallback = i18n.t(key, { ...params, defaultValue: null });
  if (fallback != null) return fallback;
  return i18n.t("errors:fallback", { code });
}

export class EngineError extends Error {
  readonly code: string;
  readonly params: ErrorParams;
  readonly category: string | undefined;
  readonly retryable: boolean;

  constructor(payload: IpcError = { code: "internal.unexpected" }) {
    const code = payload.code || "internal.unexpected";
    const params = payload.params || {};
    const policy = isFerryErrorCode(code)
      ? FERRY_ERROR_POLICIES[code]
      : undefined;
    super(translateError(code, params));
    this.name = "EngineError";
    this.code = code;
    this.params = params;
    this.category = policy?.category ?? payload.category;
    this.retryable = policy?.retryable ?? Boolean(payload.retryable);
  }
}

export function throwEngineError(error: unknown): never {
  if (typeof error === "string") {
    throw new Error(error || i18n.t("errors:engineCallFail"));
  }
  if (error && typeof error === "object" && "code" in error) {
    throw new EngineError(error as IpcError);
  }
  throw new EngineError();
}
