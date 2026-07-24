import type { AgentMessage } from "@earendil-works/pi-agent-core";
import type { EventEnvelope } from "../server/messages.js";

const MAX_TOOL_RESULT_CHARS = 8_000;
const MAX_TOOL_DETAILS_CHARS = 64_000;

export function safeText(value: string, limit: number): string {
  const redacted = value
    .replace(/\bBearer\s+[A-Za-z0-9._~+/=-]{8,}/gi, "[REDACTED]")
    .replace(/\bsk-[A-Za-z0-9_-]{12,}\b/g, "[REDACTED]")
    .replace(/\b(?:gh[opusr]|github_pat)_[A-Za-z0-9_]{16,}\b/g, "[REDACTED]")
    .replace(
      /\b[A-Z0-9_]*(?:API_KEY|TOKEN|SECRET|PASSWORD|PRIVATE_KEY)[A-Z0-9_]*\s*[:=]\s*[^\s,;]+/gi,
      "[REDACTED]",
    )
    .replace(
      /-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----[\s\S]*?-----END (?:[A-Z0-9]+ )*PRIVATE KEY-----/g,
      "[REDACTED]",
    )
    .replace(/\b[A-Z]:[\\/][^\s\]\[)(}{"']+/gi, "[ABSOLUTE_PATH]")
    .replace(/(?<![:\w])\/(?:[^/\s]+\/)*[^\s\]\[)(}{"']+/g, "[ABSOLUTE_PATH]");
  return redacted.length > limit ? `${redacted.slice(0, limit)}…` : redacted;
}

export function providerFailure(error?: unknown) {
  const detail =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : "";
  return detail
    ? `provider request failed: ${safeText(detail, 1_000)}`
    : "provider request failed";
}

function safeStructured(
  value: unknown,
  budget = { remaining: MAX_TOOL_DETAILS_CHARS },
  depth = 0,
): unknown {
  if (budget.remaining <= 0 || depth > 8) return "[truncated]";
  if (
    value === null ||
    typeof value === "boolean" ||
    typeof value === "number"
  ) {
    return value;
  }
  if (typeof value === "string") {
    const text = safeText(value, Math.min(8_000, budget.remaining));
    budget.remaining -= text.length;
    return text;
  }
  if (Array.isArray(value)) {
    return value
      .slice(0, 200)
      .map((item) => safeStructured(item, budget, depth + 1));
  }
  if (typeof value === "object") {
    const output: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value).slice(0, 200)) {
      if (budget.remaining <= 0) break;
      output[key] = safeStructured(item, budget, depth + 1);
    }
    return output;
  }
  return String(value);
}

export function summarizeToolResult(result: unknown) {
  const blocks = (result as { content?: unknown })?.content;
  const text = Array.isArray(blocks)
    ? blocks
        .map((block) =>
          typeof (block as { text?: unknown })?.text === "string"
            ? (block as { text: string }).text
            : "",
        )
        .filter(Boolean)
        .join("\n")
    : "";
  let raw = text;
  if (!raw) {
    try {
      raw = JSON.stringify(result ?? null) ?? "";
    } catch {
      raw = String(result);
    }
  }
  const summary =
    raw.length > MAX_TOOL_RESULT_CHARS
      ? { text: raw.slice(0, MAX_TOOL_RESULT_CHARS), truncated: true }
      : { text: raw, truncated: false };
  const details = (result as { details?: unknown })?.details;
  return details === undefined
    ? summary
    : { ...summary, details: safeStructured(details) };
}

export function safeMessages(messages: AgentMessage[]): AgentMessage[] {
  return messages.map((message): AgentMessage => {
    if (message.role === "assistant") {
      return {
        ...message,
        ...(message.errorMessage
          ? { errorMessage: safeText(message.errorMessage, 1_000) }
          : {}),
        content: message.content
          .filter((part) => part.type !== "thinking")
          .map((part) =>
            part.type === "text"
              ? { ...part, text: safeText(part.text, 16_000) }
              : part.type === "toolCall"
                ? { ...part, arguments: { omitted: true } }
                : part,
          ),
      };
    }
    if (message.role === "user") {
      if (typeof message.content === "string") {
        return { ...message, content: safeText(message.content, 16_000) };
      }
      return {
        ...message,
        content: message.content.map((part) =>
          part.type === "image"
            ? {
                type: "text" as const,
                text: `[image omitted: ${part.mimeType}]`,
              }
            : part.type === "text"
              ? { ...part, text: safeText(part.text, 16_000) }
              : part,
        ),
      };
    }
    if (message.role === "toolResult") {
      return {
        ...message,
        details: undefined,
        content: message.content.map((part) =>
          part.type === "image"
            ? {
                type: "text" as const,
                text: `[image omitted: ${part.mimeType}]`,
              }
            : part.type === "text"
              ? { ...part, text: safeText(part.text, 4_000) }
              : part,
        ),
      };
    }
    return message;
  });
}

export function safeEvents(events: EventEnvelope[]): EventEnvelope[] {
  const safe = events.map((event) => {
    const payload = { ...event.payload };
    if (event.type === "tool.started" || event.type === "tool.request") {
      if ("args" in payload) payload.args = "[omitted]";
    }
    if (event.type === "tool.progress" && "partial" in payload) {
      payload.partial = "[omitted]";
    }
    if (event.type === "tool.completed") {
      const result = payload.result as
        | { text?: unknown; details?: unknown }
        | undefined;
      if (result && typeof result.text === "string") {
        payload.result = {
          ...result,
          text: safeText(result.text, 4_000),
          ...(result.details === undefined
            ? {}
            : { details: safeStructured(result.details) }),
        };
      }
    }
    if (typeof payload.message === "string") {
      payload.message = safeText(payload.message, 1_000);
    }
    if (typeof payload.prompt === "string") {
      payload.prompt = safeText(payload.prompt, 16_000);
    }
    if (typeof payload.text === "string") {
      payload.text = safeText(payload.text, 16_000);
    }
    return { ...event, payload };
  });
  let index = 0;
  while (index < safe.length) {
    const first = safe[index]!;
    if (
      first.type !== "content.delta" ||
      typeof first.payload.delta !== "string"
    ) {
      index += 1;
      continue;
    }
    let end = index + 1;
    while (
      end < safe.length &&
      safe[end]!.type === "content.delta" &&
      safe[end]!.run_id === first.run_id &&
      typeof safe[end]!.payload.delta === "string"
    ) {
      end += 1;
    }
    const raw = safe
      .slice(index, end)
      .map((event) => event.payload.delta)
      .join("");
    const redacted = safeText(raw, 16_000);
    if (redacted !== raw) {
      first.payload.delta = redacted;
      for (let cursor = index + 1; cursor < end; cursor += 1) {
        safe[cursor]!.payload.delta = "";
      }
    }
    index = end;
  }
  return safe;
}
