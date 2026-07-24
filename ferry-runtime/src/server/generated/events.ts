// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const FERRY_EVENT_TYPES = [
  "auth.cancelled",
  "auth.completed",
  "auth.event",
  "auth.failed",
  "auth.prompt",
  "content.delta",
  "engine.request",
  "operation.applied",
  "operation.failed",
  "operation.proposed",
  "run.cancelled",
  "run.completed",
  "run.failed",
  "run.interrupted",
  "run.started",
  "runtime.disconnected",
  "session.created",
  "session.model_changed",
  "task.cancelled",
  "task.completed",
  "task.failed",
  "task.skipped",
  "task.started",
  "tool.completed",
  "tool.progress",
  "tool.request",
  "tool.started",
  "user.message",
  "workflow.completed",
  "workflow.started",
] as const;
export const RUNTIME_EVENT_TYPES = [
  "auth.cancelled",
  "auth.completed",
  "auth.event",
  "auth.failed",
  "auth.prompt",
  "content.delta",
  "engine.request",
  "run.cancelled",
  "run.completed",
  "run.failed",
  "run.interrupted",
  "run.started",
  "session.created",
  "session.model_changed",
  "task.cancelled",
  "task.completed",
  "task.failed",
  "task.skipped",
  "task.started",
  "tool.completed",
  "tool.progress",
  "tool.request",
  "tool.started",
  "user.message",
  "workflow.completed",
  "workflow.started",
] as const;
export type FerryEventType = (typeof FERRY_EVENT_TYPES)[number];
export type RuntimeEventType = (typeof RUNTIME_EVENT_TYPES)[number];
export function isRuntimeEventType(value: unknown): value is RuntimeEventType {
  return (
    typeof value === "string" &&
    (RUNTIME_EVENT_TYPES as readonly string[]).includes(value)
  );
}
