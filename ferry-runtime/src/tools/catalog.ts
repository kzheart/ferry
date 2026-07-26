import type { AgentTool } from "@earendil-works/pi-agent-core";
import { Type } from "@earendil-works/pi-ai";
import {
  AGENT_CAPABILITIES,
  AGENT_EDIT_OPERATIONS,
  AGENT_IDS,
  type AgentId,
} from "../server/generated/agents.js";
import {
  OPAQUE_SESSION_REF_MAX_LENGTH,
  OPAQUE_SESSION_REF_MIN_LENGTH,
} from "../server/generated/session-ref.js";

const opaqueSessionRef = Type.String({
  minLength: OPAQUE_SESSION_REF_MIN_LENGTH,
  maxLength: OPAQUE_SESSION_REF_MAX_LENGTH,
  pattern: "^[A-Za-z0-9_-]+$",
});

// 相对写法必须写进 schema:模型没有时钟,不知道能这么写就会去 shell 里问 date。
const timePoint = Type.Union(
  [Type.Integer({ minimum: 0 }), Type.String({ maxLength: 64 })],
  {
    description:
      'Epoch milliseconds, ISO8601, "now", or a relative offset such as "now-7d" or "-12h" (units s/m/h/d/w). Results also carry a now field you can compute from.',
  },
);

const timeRange = Type.Object(
  {
    from: Type.Optional(timePoint),
    to: Type.Optional(timePoint),
  },
  { additionalProperties: false },
);

const editOps = Type.Array(
  Type.Union([
    Type.Object(
      {
        op: Type.Literal("delete-turn"),
        turn: Type.Integer({ minimum: 1 }),
      },
      { additionalProperties: false },
    ),
    Type.Object(
      {
        op: Type.Literal("rewrite"),
        locator: Type.String({
          pattern: "^fml_",
          maxLength: 512,
          description:
            "Copy this value exactly from context messages[].locator or content-search matches[].locator. Never invent or transform it.",
        }),
        text: Type.String({ minLength: 1, maxLength: 20_000 }),
      },
      { additionalProperties: false },
    ),
  ]),
  { minItems: 1, maxItems: 50 },
);

const operationIntent = Type.Union([
  Type.Literal("preview"),
  Type.Literal("execute"),
]);

const metadataPatch = Type.Object(
  {
    name: Type.Optional(Type.String({ maxLength: 200 })),
    pinned: Type.Optional(Type.Boolean()),
    archived: Type.Optional(Type.Boolean()),
    tags: Type.Optional(
      Type.Array(Type.String({ maxLength: 64 }), { maxItems: 20 }),
    ),
  },
  { additionalProperties: false },
);

const sessionEditSchema = Type.Unsafe({
  type: "object",
  properties: {
    tool: Type.String({ minLength: 1, maxLength: 32 }),
    ref: opaqueSessionRef,
    ops: editOps,
    patch: metadataPatch,
    intent: operationIntent,
  },
  required: ["tool", "ref"],
  additionalProperties: false,
  oneOf: [
    {
      required: ["ops", "intent"],
      not: { required: ["patch"] },
    },
    {
      required: ["patch"],
      not: {
        anyOf: [{ required: ["ops"] }, { required: ["intent"] }],
      },
    },
  ],
});

const exposedSessionEditOperations = new Set(["delete-turn", "rewrite"]);
type AgentCapability = (typeof AGENT_CAPABILITIES)[AgentId][number];
const supportsAgentCapability = (tool: AgentId, capability: AgentCapability) =>
  (AGENT_CAPABILITIES[tool] as readonly AgentCapability[]).includes(capability);
const migrationTargets = AGENT_IDS.filter((tool) =>
  supportsAgentCapability(tool, "migration-target"),
);
const migrationSources = AGENT_IDS.filter((tool) =>
  supportsAgentCapability(tool, "migration-source"),
);
const agentEnum = (values: readonly string[]) =>
  Type.Unsafe({ type: "string", enum: [...values] });

function supportedSessionEditOperations(tool: AgentId): string[] {
  return AGENT_EDIT_OPERATIONS[tool].filter((operation) =>
    exposedSessionEditOperations.has(operation),
  );
}

function validateSessionEditOperations(input: Record<string, unknown>): void {
  const tool = input.tool;
  if (
    typeof tool !== "string" ||
    !(AGENT_IDS as readonly string[]).includes(tool)
  ) {
    throw new Error(
      `session_edit content ops require a known source tool: ${AGENT_IDS.join(", ")}`,
    );
  }
  if (!Array.isArray(input.ops)) {
    throw new Error("session_edit ops must be an array");
  }

  const requested = input.ops.map((operation) => {
    if (
      operation === null ||
      typeof operation !== "object" ||
      !("op" in operation) ||
      typeof operation.op !== "string"
    ) {
      throw new Error("session_edit ops must contain operation names");
    }
    return operation.op;
  });
  const supported = supportedSessionEditOperations(tool as AgentId);
  const unsupported = [
    ...new Set(requested.filter((operation) => !supported.includes(operation))),
  ];
  if (unsupported.length > 0) {
    throw new Error(
      `session_edit ${tool} does not support content operations: ${unsupported.join(", ")}; supported: ${supported.join(", ") || "none"}`,
    );
  }
}

const sessionEditSupportDescription = AGENT_IDS.filter((tool) =>
  supportsAgentCapability(tool, "edit"),
)
  .map((tool) => `${tool}: ${supportedSessionEditOperations(tool).join(", ")}`)
  .join("; ");

export const FERRY_TOOL_NAMES = [
  "session_search",
  "session_read",
  "usage",
  "migrate",
  "session_edit",
  "bash",
] as const;

export type FerryToolName = (typeof FERRY_TOOL_NAMES)[number];

export interface ToolRequestContext {
  sessionId: string;
  runId: string;
  toolCallId: string;
  applyPolicy?: "manual" | "auto";
  signal?: AbortSignal;
  onUpdate: (payload: unknown) => void;
}

interface FerryToolPort {
  invoke(
    name: FerryToolName,
    args: Record<string, unknown>,
    context: ToolRequestContext,
  ): Promise<unknown>;
}

const schemas = {
  session_search: Type.Object(
    {
      query: Type.Optional(Type.String({ minLength: 1, maxLength: 500 })),
      patterns: Type.Optional(
        Type.Array(Type.String({ minLength: 1, maxLength: 500 }), {
          maxItems: 16,
        }),
      ),
      agents: Type.Optional(
        Type.Array(Type.String({ maxLength: 32 }), { maxItems: 8 }),
      ),
      projects: Type.Optional(
        Type.Array(Type.String({ maxLength: 256 }), { maxItems: 20 }),
      ),
      time_range: Type.Optional(timeRange),
      limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 50 })),
      scope: Type.Optional(
        Type.Union([
          Type.Literal("any"),
          Type.Literal("metadata"),
          Type.Literal("content"),
        ]),
      ),
      include_tool_outputs: Type.Optional(Type.Boolean()),
    },
    { additionalProperties: false },
  ),
  session_read: Type.Object(
    {
      tool: Type.String({ minLength: 1, maxLength: 32 }),
      ref: opaqueSessionRef,
      terms: Type.Optional(
        Type.Array(Type.String({ minLength: 1, maxLength: 100 }), {
          minItems: 1,
          maxItems: 20,
        }),
      ),
      roles: Type.Optional(
        Type.Array(
          Type.Union([Type.Literal("user"), Type.Literal("assistant")]),
          { minItems: 1, maxItems: 2 },
        ),
      ),
      from_message: Type.Optional(Type.Integer({ minimum: 1 })),
      limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 50 })),
      include_tool_outputs: Type.Optional(Type.Boolean()),
      max_bytes: Type.Optional(
        Type.Integer({ minimum: 1024, maximum: 65_536 }),
      ),
    },
    { additionalProperties: false },
  ),
  usage: Type.Object(
    {
      agents: Type.Optional(
        Type.Array(Type.String({ maxLength: 32 }), { maxItems: 8 }),
      ),
      projects: Type.Optional(
        Type.Array(Type.String({ maxLength: 256 }), { maxItems: 20 }),
      ),
      time_range: Type.Optional(timeRange),
    },
    { additionalProperties: false },
  ),
  migrate: Type.Object(
    {
      source_tool: agentEnum(migrationSources),
      ref: opaqueSessionRef,
      target_tool: agentEnum(migrationTargets),
      max_turn: Type.Optional(Type.Integer({ minimum: 1 })),
      intent: operationIntent,
    },
    { additionalProperties: false },
  ),
  // Function-tool providers require an object root. Conditional constraints
  // live inside oneOf while the execution boundary validates them again.
  session_edit: sessionEditSchema,
  bash: Type.Object(
    {
      command: Type.String({ minLength: 1, maxLength: 4_000 }),
      cwd: Type.Optional(Type.String({ maxLength: 1_024 })),
      timeout_ms: Type.Optional(
        Type.Integer({ minimum: 1_000, maximum: 120_000 }),
      ),
    },
    { additionalProperties: false },
  ),
} as const;

const descriptions: Record<FerryToolName, string> = {
  session_search:
    'Search the whole session library: metadata (title, project, source tool, model) and, by default, full-text message content across every session — use this instead of reading sessions one by one or shelling out to grep. scope narrows matching to metadata or content only; default any matches either. Query words are ANDed within one message (or one metadata row): every word must appear there, substring-matched, so prefer one or two distinctive words. Quote "a phrase" for exact adjacency. For OR — alternative wordings, or a set of independent patterns like leaked-credential prefixes (sk-ant, ghp_, AKIA, "BEGIN PRIVATE KEY", "password=") — pass patterns: an array of up to 16 strings matched as a union, so one call covers them all; a session matches if ANY pattern matches, and each pattern keeps the same AND-within-a-message/phrase rules as query. Pass query, patterns, or both (at least one is required). Do NOT space-separate alternatives inside a single query string expecting OR — that ANDs them and silently returns nothing. Content hits carry matched_in, content_match_count and content_matches (message/turn/role plus a size-bounded original snippet) — jump to a hit with session_read from_message. Coding sessions keep most substance (code, file contents, command output) inside tool calls, so pass include_tool_outputs true before concluding a term is absent from content. Check content_index in the result: when ready is false, pending_sessions are still being indexed and content results are partial — say so rather than presenting them as complete. Only the first 16KB per message is content-indexed; session_read with terms scans one session exhaustively. total_matches is how many sessions matched and returned is how many came back — when total_matches exceeds returned you have seen a sample, not the library, so never describe the result as the user\'s complete history. record_count counts raw transcript records and is larger than the message_count session_read reports for the same session. An fsr_ ref stops resolving once that session is written to again; if a read fails with reason session_changed, search again and use the fresh ref.',
  session_read:
    "Read one indexed session using an fsr_ ref returned by session_search. By default returns a size-bounded page of original messages; paginate with next_from_message, never turn numbers. Pass terms to search that session's content and get matching snippets; searched_scope tells you what was covered — by default only visible message text, and coding sessions keep most of their substance (code, file contents, command output) inside tool calls, so pass include_tool_outputs true before concluding a term is absent. Every returned message carries message_count, turn_count, an fml_ locator, and an editable flag; only editable=true messages may be rewritten, and locators must be copied exactly. message_count and turn_count differ, and both differ from search's record_count. If a search match has complete=false, re-read that message without terms before editing its full text.",
  usage:
    "Get aggregate usage: tokens and estimated cost overall, by_agent, by_model and by_project (each bucket keeps only the top spenders). cost is an estimate computed from public per-model prices, not a bill; models listed in unpriced_models had no price match and contribute tokens but no cost. Never invent amounts of your own — report these numbers or say they are unavailable.",
  migrate: `Migrate a session into another agent's format (targets: ${migrationTargets.join(", ")}). intent is required: use preview to inspect the impact without changing anything, or execute to create an approval-gated migration that writes an immutable copy in the target format once approved. source_tool and target_tool are agent names; ref is an fsr_ value.`,
  session_edit: `Edit one session in place. Pass ops to rewrite or delete message turns, OR patch to change metadata (rename, pin, archive, tags) — exactly one. Content ops available through this tool by source: ${sessionEditSupportDescription}. Content ops require intent: use preview to inspect the diff, or execute to create an approval-gated edit that rewrites the original after revision checks and a recovery snapshot (Auto mode applies synchronously). Metadata patch does not accept intent. For rewrite ops, copy an editable message's fml_ locator exactly from a recent session_read and batch all intended rewrites into one call. Use patch only when the user explicitly asks to rename, pin, archive, or tag a session.`,
  bash: "Run a shell command on the user's machine. The command really executes; unless the session is in Auto mode every call needs the user's approval first, so keep commands single-purpose and explain destructive ones before proposing them. Returns exit_code, stdout and stderr; output over 64KB is truncated.",
};

export function createFerryTools(
  port: FerryToolPort,
  getContext: () => Omit<ToolRequestContext, "toolCallId" | "onUpdate">,
  allowedTools: readonly FerryToolName[] = FERRY_TOOL_NAMES,
): AgentTool[] {
  return allowedTools.map((name) => ({
    name,
    label: name,
    description: descriptions[name],
    parameters: schemas[name],
    executionMode: "sequential",
    async execute(toolCallId, params, signal, onUpdate) {
      const input = params as Record<string, unknown>;
      if (name === "session_search") {
        const hasQuery =
          typeof input.query === "string" && input.query.trim().length > 0;
        const hasPatterns =
          Array.isArray(input.patterns) && input.patterns.length > 0;
        if (!hasQuery && !hasPatterns)
          throw new Error("session_search requires query or patterns");
      }
      if (
        name === "migrate" &&
        input.intent !== "preview" &&
        input.intent !== "execute"
      ) {
        throw new Error("migrate requires intent preview or execute");
      }
      if (name === "session_edit") {
        const hasOps = input.ops !== undefined;
        const hasPatch = input.patch !== undefined;
        if (hasOps === hasPatch) {
          throw new Error("session_edit requires exactly one of ops or patch");
        }
        if (hasOps && input.intent !== "preview" && input.intent !== "execute")
          throw new Error("session_edit ops require intent preview or execute");
        if (hasPatch && input.intent !== undefined)
          throw new Error("session_edit metadata patch does not accept intent");
        if (hasOps) validateSessionEditOperations(input);
      }
      if (name === "bash" && String(input.command ?? "").trim().length === 0) {
        throw new Error("bash requires a non-empty command");
      }
      const active = getContext();
      const details = await port.invoke(
        name,
        params as Record<string, unknown>,
        {
          ...active,
          ...(signal ? { signal } : {}),
          toolCallId,
          onUpdate(payload) {
            onUpdate?.({
              content: [{ type: "text", text: "Tool is still running" }],
              details: payload,
            });
          },
        },
      );
      return {
        content: [{ type: "text", text: JSON.stringify(details) }],
        details,
      };
    },
  })) as AgentTool[];
}
