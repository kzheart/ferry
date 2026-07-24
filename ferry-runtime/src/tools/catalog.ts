import type { AgentTool } from "@earendil-works/pi-agent-core";
import { Type } from "@earendil-works/pi-ai";
import { AGENT_IDS } from "../server/generated/agents.js";
import {
  OPAQUE_SESSION_REF_MAX_LENGTH,
  OPAQUE_SESSION_REF_MIN_LENGTH,
} from "../server/generated/session-ref.js";

const opaqueSessionRef = Type.String({
  minLength: OPAQUE_SESSION_REF_MIN_LENGTH,
  maxLength: OPAQUE_SESSION_REF_MAX_LENGTH,
  pattern: "^[A-Za-z0-9_-]+$",
});

const timeRange = Type.Object(
  {
    from: Type.Optional(
      Type.Union([
        Type.Integer({ minimum: 0 }),
        Type.String({ maxLength: 64 }),
      ]),
    ),
    to: Type.Optional(
      Type.Union([
        Type.Integer({ minimum: 0 }),
        Type.String({ maxLength: 64 }),
      ]),
    ),
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

export const FERRY_TOOL_NAMES = [
  "session_search",
  "session_read",
  "usage",
  "migrate",
  "session_edit",
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

export interface FerryToolPort {
  invoke(
    name: FerryToolName,
    args: Record<string, unknown>,
    context: ToolRequestContext,
  ): Promise<unknown>;
}

const schemas = {
  session_search: Type.Object(
    {
      query: Type.String({ minLength: 1, maxLength: 500 }),
      agents: Type.Optional(
        Type.Array(Type.String({ maxLength: 32 }), { maxItems: 8 }),
      ),
      projects: Type.Optional(
        Type.Array(Type.String({ maxLength: 256 }), { maxItems: 20 }),
      ),
      time_range: Type.Optional(timeRange),
      limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 50 })),
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
      source_tool: Type.String({ minLength: 1, maxLength: 32 }),
      ref: opaqueSessionRef,
      target_tool: Type.String({ minLength: 1, maxLength: 32 }),
      max_turn: Type.Optional(Type.Integer({ minimum: 1 })),
      intent: operationIntent,
    },
    { additionalProperties: false },
  ),
  // Function-tool providers require an object root. Conditional constraints
  // live inside oneOf while the execution boundary validates them again.
  session_edit: sessionEditSchema,
} as const;

const descriptions: Record<FerryToolName, string> = {
  session_search:
    "Search session metadata (title, project, source tool, and model). Returns fsr_ refs; it does not search message bodies or native session IDs.",
  session_read:
    "Read one indexed session using an fsr_ ref returned by session_search. By default returns a bounded, redacted page of messages; paginate with next_from_message, never turn numbers. Pass terms to search visible text instead and get matching snippets. Every returned message carries message_count, turn_count, an fml_ locator, and an editable flag; only editable=true messages may be rewritten, and locators must be copied exactly. message_count and turn_count differ. If a search match has complete=false, re-read that message without terms before editing its full text.",
  usage: "Get privacy-filtered aggregate usage.",
  migrate: `Migrate a session into another agent's format (targets: ${AGENT_IDS.join(", ")}). intent is required: use preview to inspect the impact without changing anything, or execute to create an approval-gated migration that writes an immutable copy in the target format once approved. source_tool and target_tool are agent names; ref is an fsr_ value.`,
  session_edit:
    "Edit one session in place. Pass ops to rewrite or delete message turns, OR patch to change metadata (rename, pin, archive, tags) — exactly one. Content ops require intent: use preview to inspect the diff, or execute to create an approval-gated edit that rewrites the original after revision checks and a recovery snapshot (Auto mode applies synchronously). Metadata patch does not accept intent. For rewrite ops, copy an editable message's fml_ locator exactly from a recent session_read and batch all intended rewrites into one call. Use patch only when the user explicitly asks to rename, pin, archive, or tag a session.",
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
