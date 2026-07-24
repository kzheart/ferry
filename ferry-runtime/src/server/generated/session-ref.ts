// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
import type { AgentId } from "./agents.js";

export const OPAQUE_SESSION_REF_PREFIX = "fsr_" as const;
export const OPAQUE_SESSION_REF_MIN_LENGTH = 8 as const;
export const OPAQUE_SESSION_REF_MAX_LENGTH = 128 as const;

export interface SessionRef {
  tool: AgentId;
  ref: string;
}

export function isOpaqueSessionRef(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length >= OPAQUE_SESSION_REF_MIN_LENGTH &&
    value.length <= OPAQUE_SESSION_REF_MAX_LENGTH &&
    value.startsWith(OPAQUE_SESSION_REF_PREFIX) &&
    /^[A-Za-z0-9_-]+$/.test(value)
  );
}
