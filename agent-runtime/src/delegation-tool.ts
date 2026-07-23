import type { AgentTool } from "@earendil-works/pi-agent-core";
import { Type } from "@earendil-works/pi-ai";
import type { WorkflowResult, WorkflowSpec } from "./workflow.js";

const task = Type.Object(
  {
    id: Type.String({
      pattern: "^[A-Za-z0-9_-]+$",
      minLength: 1,
      maxLength: 64,
    }),
    role_id: Type.String({
      pattern: "^[A-Za-z0-9_-]+$",
      minLength: 1,
      maxLength: 128,
    }),
    instruction: Type.String({ minLength: 1, maxLength: 20_000 }),
    depends_on: Type.Optional(
      Type.Array(Type.String({ minLength: 1, maxLength: 64 }), {
        maxItems: 32,
      }),
    ),
  },
  { additionalProperties: false },
);

const parameters = Type.Object(
  {
    tasks: Type.Array(task, { minItems: 1, maxItems: 32 }),
    max_concurrency: Type.Optional(Type.Integer({ minimum: 1, maximum: 8 })),
    max_output_chars: Type.Optional(
      Type.Integer({ minimum: 1_000, maximum: 200_000 }),
    ),
    failure_policy: Type.Optional(
      Type.Union([Type.Literal("fail_fast"), Type.Literal("continue")]),
    ),
  },
  { additionalProperties: false },
);

export function createDelegationTool(
  execute: (
    spec: WorkflowSpec,
    onUpdate: (payload: unknown) => void,
    signal?: AbortSignal,
  ) => Promise<WorkflowResult>,
): AgentTool {
  return {
    name: "delegate_agents",
    label: "delegate_agents",
    description:
      "Delegate independent or dependent read-only tasks to Ferry roles. Tasks without dependencies run in bounded parallel; depends_on creates fan-in. Use this for work that benefits from multiple perspectives, then synthesize the returned results.",
    parameters,
    executionMode: "sequential",
    async execute(_toolCallId, params, signal, onUpdate) {
      const result = await execute(
        params as WorkflowSpec,
        (payload) =>
          onUpdate?.({
            content: [{ type: "text", text: "Delegated agents are working" }],
            details: payload,
          }),
        signal,
      );
      return {
        content: [{ type: "text", text: JSON.stringify(result) }],
        details: result,
      };
    },
  } as AgentTool;
}
