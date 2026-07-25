import type { AgentMessage } from "@earendil-works/pi-agent-core";
import type { EventEnvelope } from "../server/messages.js";

const MAX_TOOL_RESULT_CHARS = 8_000;
const MAX_TOOL_DETAILS_CHARS = 64_000;

export function truncateText(value: string, limit: number): string {
  return value.length > limit ? `${value.slice(0, limit)}…` : value;
}

function boundedStructure(
  value: unknown,
  budget: { nodes: number },
  depth = 0,
): [unknown, boolean] {
  if (budget.nodes <= 0 || depth > 8) {
    return [null, true];
  }
  budget.nodes -= 1;
  if (
    value === null ||
    typeof value === "boolean" ||
    typeof value === "number"
  ) {
    return [value, false];
  }
  if (typeof value === "string") {
    return [value, false];
  }
  if (Array.isArray(value)) {
    const result: unknown[] = [];
    let truncated = value.length > 200;
    for (const item of value.slice(0, 200)) {
      const [child, childTruncated] = boundedStructure(
        item,
        budget,
        depth + 1,
      );
      result.push(child);
      truncated ||= childTruncated;
    }
    return [result, truncated];
  }
  if (typeof value === "object") {
    const entries = Object.entries(value);
    const output: Record<string, unknown> = {};
    let truncated = entries.length > 200;
    for (const [key, item] of entries.slice(0, 200)) {
      const [child, childTruncated] = boundedStructure(
        item,
        budget,
        depth + 1,
      );
      output[key] = child;
      truncated ||= childTruncated;
    }
    return [output, truncated];
  }
  return [String(value), false];
}

function boundedJson(value: unknown, maxBytes = MAX_TOOL_DETAILS_CHARS): unknown {
  const [structured, structurallyTruncated] = boundedStructure(
    value,
    { nodes: 2_000 },
  );
  const bounded = structurallyTruncated
    ? { truncated: true, value: structured }
    : structured;
  const encoded = JSON.stringify(bounded);
  if (encoded === undefined || Buffer.byteLength(encoded) <= maxBytes) {
    return bounded;
  }
  return {
    truncated: true,
    preview: truncateText(encoded, Math.max(0, maxBytes - 64)),
  };
}

export function providerFailure(error?: unknown) {
  const detail =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : "";
  return detail
    ? `provider request failed: ${truncateText(detail, 1_000)}`
    : "provider request failed";
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
    : { ...summary, details: boundedJson(details) };
}

export function boundedMessages(messages: AgentMessage[]): AgentMessage[] {
  return messages.map((message): AgentMessage => {
    if (message.role === "assistant") {
      return {
        ...message,
        ...(message.errorMessage
          ? { errorMessage: truncateText(message.errorMessage, 1_000) }
          : {}),
        content: message.content.map((part) => {
          if (part.type === "text") {
            return { ...part, text: truncateText(part.text, 16_000) };
          }
          if (part.type === "thinking") {
            return {
              ...part,
              thinking: truncateText(part.thinking, 16_000),
            };
          }
          if (part.type === "toolCall") {
            return {
              ...part,
              arguments: boundedJson(part.arguments) as Record<string, unknown>,
            };
          }
          return part;
        }),
      };
    }
    if (message.role === "user") {
      if (typeof message.content === "string") {
        return {
          ...message,
          content: truncateText(message.content, 16_000),
        };
      }
      return {
        ...message,
        content: message.content.map((part) =>
          part.type === "text"
            ? { ...part, text: truncateText(part.text, 16_000) }
            : part,
        ),
      };
    }
    if (message.role === "toolResult") {
      return {
        ...message,
        ...(message.details === undefined
          ? {}
          : { details: boundedJson(message.details) }),
        content: message.content.map((part) =>
          part.type === "text"
            ? { ...part, text: truncateText(part.text, 4_000) }
            : part.type === "image"
              ? { ...part, data: truncateText(part.data, 64_000) }
              : part,
        ),
      };
    }
    return message;
  });
}

export function boundedEvents(events: EventEnvelope[]): EventEnvelope[] {
  return events.map((event) => {
    const payload = { ...event.payload };
    if ("args" in payload) payload.args = boundedJson(payload.args);
    if ("partial" in payload) payload.partial = boundedJson(payload.partial);
    if (event.type === "tool.completed") {
      const result = payload.result as
        | { text?: unknown; details?: unknown }
        | undefined;
      if (result) {
        payload.result = {
          ...result,
          ...(typeof result.text === "string"
            ? {
                text: truncateText(result.text, 4_000),
                text_truncated: result.text.length > 4_000,
              }
            : {}),
          ...(result.details === undefined
            ? {}
            : { details: boundedJson(result.details) }),
        };
      }
    }
    for (const [field, limit] of [
      ["message", 1_000],
      ["prompt", 16_000],
      ["text", 16_000],
      ["delta", 16_000],
    ] as const) {
      const value = payload[field];
      if (typeof value === "string" && value.length > limit) {
        payload[field] = truncateText(value, limit);
        payload[`${field}_truncated`] = true;
      }
    }
    return { ...event, payload };
  });
}
