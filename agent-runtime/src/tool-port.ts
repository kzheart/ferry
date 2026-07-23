import type { AgentTool } from "@earendil-works/pi-agent-core";
import { Type } from "@earendil-works/pi-ai";

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
      ref: Type.Optional(Type.String({ minLength: 1, maxLength: 512 })),
      session_id: Type.Optional(Type.String({ minLength: 1, maxLength: 512 })),
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
      ref: Type.String({ minLength: 1, maxLength: 512 }),
      target_tool: Type.String({ minLength: 1, maxLength: 32 }),
      max_turn: Type.Optional(Type.Integer({ minimum: 1 })),
      dry_run: Type.Optional(Type.Boolean()),
    },
    { additionalProperties: false },
  ),
  session_edit: Type.Union([
    Type.Object(
      {
        tool: Type.String({ minLength: 1, maxLength: 32 }),
        ref: Type.String({ minLength: 1, maxLength: 512 }),
        ops: editOps,
        dry_run: Type.Optional(Type.Boolean()),
      },
      { additionalProperties: false },
    ),
    Type.Object(
      {
        tool: Type.String({ minLength: 1, maxLength: 32 }),
        ref: Type.String({ minLength: 1, maxLength: 512 }),
        patch: Type.Object(
          {
            name: Type.Optional(Type.String({ maxLength: 200 })),
            pinned: Type.Optional(Type.Boolean()),
            archived: Type.Optional(Type.Boolean()),
            tags: Type.Optional(
              Type.Array(Type.String({ maxLength: 64 }), { maxItems: 20 }),
            ),
          },
          { additionalProperties: false },
        ),
      },
      { additionalProperties: false },
    ),
  ]),
} as const;

const descriptions: Record<FerryToolName, string> = {
  session_search:
    "Search session metadata (title, project, source tool, and model). Returns fsr_ refs; it does not search message bodies or native session IDs.",
  session_read:
    "Read one indexed session. Provide either ref (an fsr_ value from session_search) or session_id (a native ID from a session attachment, resolved internally) — exactly one. By default returns a bounded, redacted page of messages; paginate with next_from_message, never turn numbers. Pass terms to search visible text instead and get matching snippets. Every returned message carries message_count, turn_count, an fml_ locator, and an editable flag; only editable=true messages may be rewritten, and locators must be copied exactly. message_count and turn_count differ. If a search match has complete=false, re-read that message without terms before editing its full text.",
  usage: "Get privacy-filtered aggregate usage.",
  migrate:
    "Migrate a session into another agent's format (targets: claude, codex, opencode). Set dry_run true to preview the impact without changing anything; otherwise this creates an approval-gated migration that, once applied, writes an immutable copy in the target format. source_tool and target_tool are agent names; ref is an fsr_ value.",
  session_edit:
    "Edit one session in place. Pass ops to rewrite or delete message turns, OR patch to change metadata (rename, pin, archive, tags) — exactly one. For content ops, set dry_run true to preview the diff without changing anything; otherwise this creates an approval-gated edit that rewrites the original after revision checks and a recovery snapshot (Auto mode applies synchronously). For rewrite ops, copy an editable message's fml_ locator exactly from a recent session_read and batch all intended rewrites into one call. Use patch only when the user explicitly asks to rename, pin, archive, or tag a session.",
};

export function createFerryTools(
  port: FerryToolPort,
  getContext: () => Omit<ToolRequestContext, "toolCallId" | "onUpdate">,
): AgentTool[] {
  return FERRY_TOOL_NAMES.map((name) => ({
    name,
    label: name,
    description: descriptions[name],
    parameters: schemas[name],
    executionMode: "sequential",
    async execute(toolCallId, params, signal, onUpdate) {
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
