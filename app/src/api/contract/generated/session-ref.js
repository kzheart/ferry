// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const OPAQUE_SESSION_REF_PREFIX = "fsr_";
export const OPAQUE_SESSION_REF_MIN_LENGTH = 8;
export const OPAQUE_SESSION_REF_MAX_LENGTH = 128;
export const isOpaqueSessionRef = value =>
  typeof value === "string" &&
  value.length >= OPAQUE_SESSION_REF_MIN_LENGTH &&
  value.length <= OPAQUE_SESSION_REF_MAX_LENGTH &&
  value.startsWith(OPAQUE_SESSION_REF_PREFIX) &&
  /^[A-Za-z0-9_-]+$/.test(value);
