import i18n from "../../i18n/index.js";
import { FERRY_ERROR_POLICIES }
  from "../contract/generated/errors.js";

function translateError(code, params) {
  const p = params || {};
  if (code === "edit.operation_unsupported") {
    if (p.capability) {
      return i18n.t("errors:edit.operation_unsupported_with_capability", {
        tool: p.tool ?? "", capability: p.capability,
      });
    }
    return i18n.t("errors:edit.operation_unsupported_with_operation", {
      tool: p.tool ?? "", operation: p.operation ?? "",
      mode: p.mode ? `（${p.mode}）` : "",
    });
  }
  if (code === "edit.turn_out_of_range") {
    if (p.turn_count != null) {
      return i18n.t("errors:edit.turn_out_of_range_with_count", {
        requested_turn: p.requested_turn, turn_count: p.turn_count,
      });
    }
    return i18n.t("errors:edit.turn_out_of_range_invalid");
  }
  if (code === "probe.process_failed") {
    if (p.exit_code != null) {
      return i18n.t("errors:probe.process_failed_with_code", { exit_code: p.exit_code });
    }
    return i18n.t("errors:probe.process_failed", { exit_code: "" });
  }
  const key = `errors:${code}`;
  const fallback = i18n.t(key, { ...p, defaultValue: null });
  if (fallback != null) return fallback;
  return i18n.t("errors:fallback", { code });
}

export class EngineError extends Error {
  constructor(payload) {
    const { code = "internal.unexpected", params = {} } = payload || {};
    const policy = FERRY_ERROR_POLICIES[code];
    super(translateError(code, params));
    this.name = "EngineError";
    this.code = code;
    this.params = params;
    this.category = policy?.category ?? payload?.category;
    this.retryable = policy?.retryable ?? !!payload?.retryable;
  }
}

export function throwEngineError(error) {
  if (typeof error === "string") throw new Error(error || i18n.t("errors:engineCallFail"));
  throw new EngineError(error);
}
