import i18n from "../../shared/i18n/index.js";
import {
  FERRY_ERROR_POLICIES,
  isFerryErrorCode,
} from "../../shared/contracts/generated/errors.js";
import type { IpcError } from "../../shared/contracts/generated/ipc.js";

type ErrorParams = Record<string, unknown>;

function translateError(code: string, params: ErrorParams): string {
  if (code === "agent.reference_invalid") {
    // 引擎已经会为 session_changed 定向重扫自愈,走到用户面前的只剩「一直在被写入」
    // 和「已经不在了」两种;重搜会拿回同一个 ref,所以不能再提示「刷新或重新搜索」。
    if (params.reason === "session_changed") {
      return i18n.t("errors:agent.reference_invalid_session_changed");
    }
    if (params.reason === "session_missing") {
      return i18n.t("errors:agent.reference_invalid_session_missing");
    }
  }
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
  // returnNull: false 配置下缺失 key 时 t() 返回 key 本身,不能用返回值判缺失
  const key = `errors:${code}`;
  if (i18n.exists(key)) return String(i18n.t(key, params as never));
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
