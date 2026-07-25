import type { AgentTool } from "@earendil-works/pi-agent-core";
import { Type } from "@earendil-works/pi-ai";
import type { TaskGraph, WorkflowRunResult } from "../agents/scheduler.js";

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
    max_depth: Type.Optional(Type.Integer({ minimum: 1, maximum: 8 })),
    task_timeout_ms: Type.Optional(
      Type.Integer({ minimum: 1_000, maximum: 30 * 60_000 }),
    ),
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
    spec: TaskGraph,
    onUpdate: (payload: unknown) => void,
    signal?: AbortSignal,
  ) => Promise<WorkflowRunResult>,
  // 角色 id 必须列出来。不列的话模型会照任务内容编一个("searcher"、
  // "reviewer"),整次委派在创建子会话时就失败。
  roleIds: readonly string[] = [],
): AgentTool {
  const known = [...new Set(roleIds)];
  return {
    name: "delegate_agents",
    label: "delegate_agents",
    description:
      "Delegate independent or dependent read-only tasks to Ferry roles. Tasks without dependencies run in bounded parallel; depends_on creates fan-in. Use this for work that benefits from multiple perspectives, then synthesize the returned results." +
      (known.length > 0
        ? ` role_id must be one of the roles that exist here: ${known.join(", ")} — never invent a role name.`
        : ""),
    parameters,
    executionMode: "sequential",
    async execute(_toolCallId, params, signal, onUpdate) {
      if (known.length > 0) {
        const requested = (params as TaskGraph)?.tasks ?? [];
        const unknown = [
          ...new Set(
            requested
              .map((task) => task?.role_id)
              .filter((id): id is string => !!id && !known.includes(id)),
          ),
        ];
        if (unknown.length > 0) {
          throw new Error(
            `unknown role_id: ${unknown.join(", ")}; available roles: ${known.join(", ")}`,
          );
        }
      }
      const result = await execute(
        params as TaskGraph,
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
