import type { AgentTool } from "@earendil-works/pi-agent-core";
import { Type } from "@earendil-works/pi-ai";
import {
  WORKFLOW_ID_CHARACTER_CLASS,
  WORKFLOW_LIMITS,
  type TaskGraph,
  type WorkflowRunResult,
} from "../agents/scheduler.js";

// schema 的上限与 scheduler 的校验必须同源,否则模型能构造出通过 schema 却被拒的工作流
const idPattern = `^${WORKFLOW_ID_CHARACTER_CLASS}+$`;

const task = Type.Object(
  {
    id: Type.String({
      pattern: idPattern,
      minLength: 1,
      maxLength: WORKFLOW_LIMITS.maxTaskIdChars,
    }),
    role_id: Type.String({
      pattern: idPattern,
      minLength: 1,
      maxLength: WORKFLOW_LIMITS.maxRoleIdChars,
    }),
    instruction: Type.String({
      minLength: 1,
      maxLength: WORKFLOW_LIMITS.maxInstructionChars,
    }),
    depends_on: Type.Optional(
      Type.Array(
        Type.String({
          minLength: 1,
          maxLength: WORKFLOW_LIMITS.maxTaskIdChars,
        }),
        { maxItems: WORKFLOW_LIMITS.maxTasks },
      ),
    ),
  },
  { additionalProperties: false },
);

const parameters = Type.Object(
  {
    tasks: Type.Array(task, {
      minItems: 1,
      maxItems: WORKFLOW_LIMITS.maxTasks,
    }),
    max_concurrency: Type.Optional(
      Type.Integer({ minimum: 1, maximum: WORKFLOW_LIMITS.maxConcurrency }),
    ),
    max_depth: Type.Optional(
      Type.Integer({ minimum: 1, maximum: WORKFLOW_LIMITS.maxDepth }),
    ),
    task_timeout_ms: Type.Optional(
      Type.Integer({
        minimum: WORKFLOW_LIMITS.minTaskTimeoutMs,
        maximum: WORKFLOW_LIMITS.maxTaskTimeoutMs,
      }),
    ),
    max_output_chars: Type.Optional(
      Type.Integer({
        minimum: WORKFLOW_LIMITS.minOutputChars,
        maximum: WORKFLOW_LIMITS.maxOutputChars,
      }),
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
