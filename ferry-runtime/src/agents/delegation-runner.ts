import type { RuntimeSession } from "../sessions/runtime-session.js";
import {
  WorkflowRun,
  type TaskGraph,
  type WorkflowRunEvent,
  type WorkflowRunResult,
} from "./scheduler.js";

export interface DelegationRunnerPort {
  createTaskSession(roleId: string): Promise<string>;
  prompt(sessionId: string, instruction: string): Promise<void>;
  waitForIdle(sessionId: string): Promise<unknown>;
  finalText(sessionId: string): string;
  abort(sessionId: string): void;
  isRunning(sessionId: string): boolean;
  deleteSession(sessionId: string): Promise<unknown>;
  now(): number;
}

export async function runDelegatedWorkflow(
  parent: RuntimeSession,
  parentRunId: string,
  spec: TaskGraph,
  onUpdate: (payload: unknown) => void,
  port: DelegationRunnerPort,
  signal?: AbortSignal,
): Promise<WorkflowRunResult> {
  let eventQueue = Promise.resolve();
  const publish = (event: WorkflowRunEvent) => {
    onUpdate(event);
    const { type, ...payload } = event;
    eventQueue = eventQueue.then(() =>
      parent.emit(type, payload, parentRunId).then(() => undefined),
    );
  };
  const workflow = new WorkflowRun(
    spec,
    async (task, context) => {
      const childId = await port.createTaskSession(task.role_id);
      const dependencyContext = Object.keys(context.dependency_results).length
        ? `\n\nDependency results:\n${JSON.stringify(
            context.dependency_results,
          )}`
        : "";
      const instruction =
        "Complete this delegated read-only task. Do not propose or execute mutations. " +
        "Return a concise result for the parent agent.\n\n" +
        task.instruction +
        dependencyContext;
      const abort = () => {
        if (port.isRunning(childId)) port.abort(childId);
      };
      context.signal.addEventListener("abort", abort, { once: true });
      try {
        await port.prompt(childId, instruction);
        await port.waitForIdle(childId);
        if (context.signal.aborted) throw new Error("task cancelled");
        const output = port.finalText(childId);
        if (!output) throw new Error("delegated agent returned no result");
        return output;
      } finally {
        context.signal.removeEventListener("abort", abort);
        if (port.isRunning(childId)) {
          port.abort(childId);
          await port.waitForIdle(childId);
        }
        await port.deleteSession(childId);
      }
    },
    publish,
    port.now,
  );
  const abortWorkflow = () => workflow.cancel();
  if (signal?.aborted) workflow.cancel();
  else signal?.addEventListener("abort", abortWorkflow, { once: true });
  try {
    const result = await workflow.start();
    await eventQueue;
    return result;
  } finally {
    signal?.removeEventListener("abort", abortWorkflow);
  }
}
