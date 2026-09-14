import type { ProviderHost } from "../providers/provider-host.js";
import { ProtocolError, isObject } from "../server/messages.js";
import { generateTitles, type TitleItem } from "./title-generator.js";
import type { TitleEvidence } from "./title-prompt.js";
import { normalizeTitleStyle } from "./title-style.js";

const MAX_SESSIONS = 50;

function boundedText(value: unknown, max: number): string | null {
  return typeof value === "string" && value.length > 0
    ? value.slice(0, max)
    : null;
}

function stringList(value: unknown, maxItems: number, max: number): string[] {
  if (!Array.isArray(value)) return [];
  return value
    .map((item) => boundedText(item, max))
    .filter((item): item is string => item !== null)
    .slice(0, maxItems);
}

/** 证据来自引擎,字段宽松取用:缺字段只是少些线索,不该让整批请求失败。 */
function parseEvidence(value: unknown): TitleEvidence {
  if (!isObject(value)) {
    throw new ProtocolError("invalid_params", "each session must be an object");
  }
  const tool = boundedText(value.tool, 32);
  const ref = boundedText(value.ref, 512);
  if (!tool || !ref) {
    throw new ProtocolError("invalid_params", "session requires tool and ref");
  }
  return {
    tool,
    ref,
    session_id: boundedText(value.session_id, 256),
    revision:
      typeof value.revision === "number" || typeof value.revision === "string"
        ? value.revision
        : null,
    title: boundedText(value.title, 512),
    title_source: boundedText(value.title_source, 32),
    project: boundedText(value.project, 1_024),
    message_count:
      typeof value.message_count === "number" ? value.message_count : 0,
    turn_count: typeof value.turn_count === "number" ? value.turn_count : null,
    user_messages: stringList(value.user_messages, 3, 400),
    last_assistant_message: boundedText(value.last_assistant_message, 600),
    files: stringList(value.files, 20, 512),
  };
}

export function parseTitleGenerateParams(params: Record<string, unknown>) {
  const sessions = params.sessions;
  if (
    !Array.isArray(sessions) ||
    sessions.length === 0 ||
    sessions.length > MAX_SESSIONS
  ) {
    throw new ProtocolError(
      "invalid_params",
      `sessions must hold 1-${MAX_SESSIONS} items`,
    );
  }
  return {
    sessions: sessions.map(parseEvidence),
    style: normalizeTitleStyle(params.style),
  };
}

export interface TitleGenerateResult {
  items: TitleItem[];
  model: { provider_id: string; model: string };
  skipped: number;
}

export async function runTitleGenerate(
  host: ProviderHost | undefined,
  params: Record<string, unknown>,
): Promise<TitleGenerateResult> {
  if (!host) {
    throw new ProtocolError(
      "provider_unavailable",
      "no provider is configured for title generation",
    );
  }
  const { sessions, style } = parseTitleGenerateParams(params);
  const { selection, complete } = await host.titleCompleter();
  let items: TitleItem[];
  try {
    items = await generateTitles({ sessions, style, complete });
  } catch (error) {
    if (error instanceof ProtocolError) throw error;
    // 模型调用失败（没配 key、连不上、超时）对调用方是「模型不可用」，
    // 把原因带出去，别让它塌成一句 internal error。
    const reason = error instanceof Error ? error.message : String(error);
    throw new ProtocolError(
      "provider_unavailable",
      `title generation failed: ${reason}`,
    );
  }
  return {
    items,
    model: { provider_id: selection.provider, model: selection.model },
    skipped: items.filter((item) => item.skip).length,
  };
}
