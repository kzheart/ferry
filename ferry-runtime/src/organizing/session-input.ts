/** 整理请求里会话列表的公共解析:外层信封与会话头部字段。 */
import { ProtocolError } from "../server/messages.js";

const MAX_SESSIONS = 50;

export function requiredText(value: unknown, field: string, max: number) {
  if (typeof value !== "string" || !value.trim() || value.length > max) {
    throw new ProtocolError("invalid_params", `${field} is invalid`);
  }
  return value.trim();
}

export interface SessionHeader {
  tool: string;
  id: string;
  title?: string;
  project?: string;
  updated_at?: string;
}

/** 校验 sessions 数组与 locale;单条会话的字段交给 parseSession。 */
export function parseSessionEnvelope<T>(
  value: unknown,
  parseSession: (record: Record<string, unknown>, index: number) => T,
): { sessions: T[]; locale?: string } {
  if (
    typeof value !== "object" ||
    value === null ||
    !Array.isArray((value as { sessions?: unknown }).sessions)
  ) {
    throw new ProtocolError("invalid_params", "sessions must be an array");
  }
  const raw = (value as { sessions: unknown[] }).sessions;
  if (raw.length === 0 || raw.length > MAX_SESSIONS) {
    throw new ProtocolError("invalid_params", "sessions count is invalid");
  }
  const sessions = raw.map((item, index) => {
    if (typeof item !== "object" || item === null) {
      throw new ProtocolError(
        "invalid_params",
        `sessions[${index}] is invalid`,
      );
    }
    return parseSession(item as Record<string, unknown>, index);
  });
  const locale = (value as { locale?: unknown }).locale;
  return {
    sessions,
    ...(typeof locale === "string" && locale.trim()
      ? { locale: locale.trim().slice(0, 32) }
      : {}),
  };
}

/** tool/id 与三个可选展示字段;各自独有的字段由调用方补。 */
export function parseSessionHeader(
  record: Record<string, unknown>,
): SessionHeader {
  return {
    tool: requiredText(record.tool, "session.tool", 64),
    id: requiredText(record.id, "session.id", 512),
    ...(typeof record.title === "string"
      ? { title: record.title.slice(0, 500) }
      : {}),
    ...(typeof record.project === "string"
      ? { project: record.project.slice(0, 500) }
      : {}),
    ...(typeof record.updated_at === "string"
      ? { updated_at: record.updated_at.slice(0, 128) }
      : {}),
  };
}
