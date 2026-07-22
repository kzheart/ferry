import type { AgentTool } from "@earendil-works/pi-agent-core";
import { Type } from "@earendil-works/pi-ai";

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
    },
    { additionalProperties: false },
  ),
  ferry_get_usage: Type.Object(
    {
      from: Type.Optional(Type.String({ maxLength: 64 })),
      to: Type.Optional(Type.String({ maxLength: 64 })),
      projects: Type.Optional(
        Type.Array(Type.String({ maxLength: 256 }), { maxItems: 20 }),
      ),
    },
    { additionalProperties: false },
  ),
  ferry_preview_migration: Type.Object(
    {
      source_tool: Type.String({ minLength: 1, maxLength: 32 }),
      ref: Type.String({ minLength: 1, maxLength: 512 }),
      target_tool: Type.String({ minLength: 1, maxLength: 32 }),
    },
    { additionalProperties: false },
  ),
  ferry_preview_edit: Type.Object(
    {
      tool: Type.String({ minLength: 1, maxLength: 32 }),
      ref: Type.String({ minLength: 1, maxLength: 512 }),
      instructions: Type.String({ minLength: 1, maxLength: 20_000 }),
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
