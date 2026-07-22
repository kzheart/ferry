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

export const FERRY_TOOL_NAMES = [
  "ferry_list_capabilities",
  "ferry_search_sessions",
  "ferry_get_session_context",
  "ferry_get_usage",
  "ferry_preview_migration",
  "ferry_preview_edit",
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
  ferry_list_capabilities: Type.Object({}, { additionalProperties: false }),
  ferry_search_sessions: Type.Object(
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
  ferry_get_session_context: Type.Object(
    {
      tool: Type.String({ minLength: 1, maxLength: 32 }),
      ref: Type.String({ minLength: 1, maxLength: 512 }),
      from_turn: Type.Optional(Type.Integer({ minimum: 1 })),
      to_turn: Type.Optional(Type.Integer({ minimum: 1 })),
      include_tool_outputs: Type.Optional(Type.Boolean()),
      max_bytes: Type.Optional(
        Type.Integer({ minimum: 1024, maximum: 65_536 }),
      ),
    },
    { additionalProperties: false },
  ),
  ferry_get_usage: Type.Object(
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
  ferry_preview_migration: Type.Object(
    {
      source_tool: Type.String({ minLength: 1, maxLength: 32 }),
      ref: Type.String({ minLength: 1, maxLength: 512 }),
      target_tool: Type.String({ minLength: 1, maxLength: 32 }),
      max_turn: Type.Optional(Type.Integer({ minimum: 1 })),
    },
    { additionalProperties: false },
  ),
  ferry_preview_edit: Type.Object(
    {
      tool: Type.String({ minLength: 1, maxLength: 32 }),
      ref: Type.String({ minLength: 1, maxLength: 512 }),
      ops: Type.Optional(
        Type.Array(
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
                locator: Type.String({ minLength: 1, maxLength: 512 }),
                text: Type.String({ minLength: 1, maxLength: 20_000 }),
              },
              { additionalProperties: false },
            ),
          ]),
          { minItems: 1, maxItems: 50 },
        ),
      ),
      turn: Type.Optional(Type.Integer({ minimum: 1 })),
      reply: Type.Optional(
        Type.Object(
          {
            items: Type.Array(
              Type.Union([
                Type.Object(
                  {
                    kind: Type.Literal("text"),
                    text: Type.String({ minLength: 1, maxLength: 20_000 }),
                  },
                  { additionalProperties: false },
                ),
                Type.Object(
                  {
                    kind: Type.Literal("tool"),
                    name: Type.String({ minLength: 1, maxLength: 120 }),
                    input: Type.Union([
                      Type.String({ maxLength: 20_000 }),
                      Type.Record(Type.String(), Type.Unknown()),
                    ]),
                    output: Type.String({ maxLength: 20_000 }),
                  },
                  { additionalProperties: false },
                ),
              ]),
              { minItems: 1, maxItems: 100 },
            ),
          },
          { additionalProperties: false },
        ),
      ),
    },
    { additionalProperties: false },
  ),
} as const;

const descriptions: Record<FerryToolName, string> = {
  ferry_list_capabilities:
    "List Ferry capabilities using a privacy-filtered response.",
  ferry_search_sessions: "Search the Engine's bounded scanned-session index.",
  ferry_get_session_context:
    "Read a bounded, redacted slice of an indexed session.",
  ferry_get_usage: "Get privacy-filtered aggregate usage.",
  ferry_preview_migration: "Preview a session migration without writing data.",
  ferry_preview_edit: "Preview a session edit without writing data.",
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
