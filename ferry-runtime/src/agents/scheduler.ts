import { ProtocolError } from "../server/messages.js";

export type WorkflowFailurePolicy = "fail_fast" | "continue";
export type TaskStatus =
  | "pending"
  | "running"
  | "completed"
  | "failed"
  | "cancelled"
  | "skipped";

export interface TaskNode {
  id: string;
  role_id: string;
  instruction: string;
  depends_on?: string[];
}

export interface TaskGraph {
  tasks: TaskNode[];
  max_concurrency?: number;
  max_depth?: number;
  task_timeout_ms?: number;
  max_output_chars?: number;
  failure_policy?: WorkflowFailurePolicy;
}

export interface TaskResult {
  task_id: string;
  role_id: string;
  status: TaskStatus;
  output?: string;
  error?: string;
  started_at?: number;
  finished_at?: number;
}

export interface WorkflowRunResult {
  status: "completed" | "failed" | "cancelled";
  tasks: TaskResult[];
}

export type WorkflowRunEvent =
  | { type: "workflow.started"; task_count: number }
  | { type: "task.started"; task_id: string; role_id: string }
  | { type: "task.completed"; task_id: string }
  | { type: "task.failed"; task_id: string; error: string }
  | { type: "task.cancelled"; task_id: string }
  | { type: "task.skipped"; task_id: string; reason: string }
  | { type: "workflow.completed"; status: WorkflowRunResult["status"] };

export interface TaskExecutionContext {
  signal: AbortSignal;
  dependency_results: Readonly<Record<string, string>>;
}

export type TaskExecutor = (
  task: TaskNode,
  context: TaskExecutionContext,
) => Promise<string>;

const MAX_TASKS = 32;
const MAX_CONCURRENCY = 8;
const MAX_DEPTH = 8;
const MAX_INSTRUCTION_CHARS = 20_000;
const MAX_RESULT_CHARS = 40_000;
const MAX_WORKFLOW_OUTPUT_CHARS = 200_000;
const DEFAULT_TASK_TIMEOUT_MS = 5 * 60_000;

function boundedInteger(
  value: unknown,
  fallback: number,
  minimum: number,
  maximum: number,
  field: string,
) {
  const result = value ?? fallback;
  if (
    typeof result !== "number" ||
    !Number.isInteger(result) ||
    result < minimum ||
    result > maximum
  ) {
    throw new ProtocolError("invalid_workflow", `${field} is invalid`);
  }
  return result;
}

function validate(spec: TaskGraph) {
  if (!Array.isArray(spec.tasks) || spec.tasks.length === 0) {
    throw new ProtocolError("invalid_workflow", "tasks must not be empty");
  }
  if (spec.tasks.length > MAX_TASKS) {
    throw new ProtocolError("invalid_workflow", "task budget exceeded");
  }
  const ids = new Set<string>();
  const byId = new Map<string, TaskNode>();
  for (const task of spec.tasks) {
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(task.id) || ids.has(task.id)) {
      throw new ProtocolError("invalid_workflow", "task id is invalid");
    }
    if (!/^[A-Za-z0-9_-]{1,128}$/.test(task.role_id)) {
      throw new ProtocolError("invalid_workflow", "task role_id is invalid");
    }
    if (
      typeof task.instruction !== "string" ||
      !task.instruction.trim() ||
      task.instruction.length > MAX_INSTRUCTION_CHARS
    ) {
      throw new ProtocolError(
        "invalid_workflow",
        "task instruction is invalid",
      );
    }
    if (
      task.depends_on !== undefined &&
      (!Array.isArray(task.depends_on) ||
        task.depends_on.length > MAX_TASKS ||
        task.depends_on.some(
          (dependency) =>
            typeof dependency !== "string" ||
            dependency === task.id ||
            task.depends_on!.filter((value) => value === dependency).length > 1,
        ))
    ) {
      throw new ProtocolError(
        "invalid_workflow",
        "task dependencies are invalid",
      );
    }
    ids.add(task.id);
    byId.set(task.id, {
      ...task,
      instruction: task.instruction.trim(),
      depends_on: [...(task.depends_on ?? [])],
    });
  }
  for (const task of byId.values()) {
    if (task.depends_on!.some((dependency) => !byId.has(dependency))) {
      throw new ProtocolError(
        "invalid_workflow",
        "task dependency does not exist",
      );
    }
  }

  const maxDepth = boundedInteger(
    spec.max_depth,
    MAX_DEPTH,
    1,
    MAX_DEPTH,
    "max_depth",
  );
  const depths = new Map<string, number>();
  const visiting = new Set<string>();
  const depth = (taskId: string): number => {
    const cached = depths.get(taskId);
    if (cached !== undefined) return cached;
    if (visiting.has(taskId)) {
      throw new ProtocolError("invalid_workflow", "task graph has a cycle");
    }
    visiting.add(taskId);
    const task = byId.get(taskId)!;
    const value =
      1 +
      Math.max(0, ...task.depends_on!.map((dependency) => depth(dependency)));
    visiting.delete(taskId);
    if (value > maxDepth) {
      throw new ProtocolError("invalid_workflow", "task graph is too deep");
    }
    depths.set(taskId, value);
    return value;
  };
  for (const taskId of ids) depth(taskId);

  const maxConcurrency = boundedInteger(
    spec.max_concurrency,
    3,
    1,
    MAX_CONCURRENCY,
    "max_concurrency",
  );
  const timeout = boundedInteger(
    spec.task_timeout_ms,
    DEFAULT_TASK_TIMEOUT_MS,
    1_000,
    30 * 60_000,
    "task_timeout_ms",
  );
  const maxOutputChars = boundedInteger(
    spec.max_output_chars,
    MAX_WORKFLOW_OUTPUT_CHARS,
    1_000,
    MAX_WORKFLOW_OUTPUT_CHARS,
    "max_output_chars",
  );
  const failurePolicy = spec.failure_policy ?? "fail_fast";
  if (!["fail_fast", "continue"].includes(failurePolicy)) {
    throw new ProtocolError("invalid_workflow", "failure_policy is invalid");
  }
  return {
    tasks: [...byId.values()],
    maxConcurrency,
    timeout,
    maxOutputChars,
    failurePolicy,
  };
}

function errorText(error: unknown) {
  const value = error instanceof Error ? error.message : String(error);
  return value.slice(0, 1_000);
}

/** 一次有界多 Agent 协作的运行实例。 */
export class WorkflowRun {
  private readonly controller = new AbortController();
  private readonly taskControllers = new Map<string, AbortController>();
  private started = false;

  constructor(
    private readonly spec: TaskGraph,
    private readonly executor: TaskExecutor,
    private readonly onEvent: (event: WorkflowRunEvent) => void = () => {},
    private readonly now: () => number = Date.now,
  ) {}

  cancel() {
    this.controller.abort();
    for (const controller of this.taskControllers.values()) controller.abort();
  }

  async start(): Promise<WorkflowRunResult> {
    if (this.started) {
      throw new ProtocolError(
        "workflow_already_started",
        "workflow can only start once",
      );
    }
    this.started = true;
    const config = validate(this.spec);
    const states = new Map<string, TaskResult>(
      config.tasks.map((task) => [
        task.id,
        {
          task_id: task.id,
          role_id: task.role_id,
          status: "pending",
        },
      ]),
    );
    const outputs = new Map<string, string>();
    let active = 0;
    let settled = false;
    let failed = false;
    let outputChars = 0;
    this.onEvent({
      type: "workflow.started",
      task_count: config.tasks.length,
    });

    const final = await new Promise<WorkflowRunResult>((resolve) => {
      const finish = () => {
        if (settled || active > 0) return;
        const pending = [...states.values()].some(
          (state) => state.status === "pending",
        );
        if (pending && !this.controller.signal.aborted) return;
        settled = true;
        const status = failed
          ? "failed"
          : this.controller.signal.aborted
            ? "cancelled"
            : "completed";
        this.onEvent({ type: "workflow.completed", status });
        resolve({ status, tasks: [...states.values()] });
      };

      const skipBlocked = () => {
        let changed = true;
        while (changed) {
          changed = false;
          for (const task of config.tasks) {
            const state = states.get(task.id)!;
            if (state.status !== "pending") continue;
            const dependencies = task.depends_on ?? [];
            const blocked = dependencies.some((dependency) =>
              ["failed", "cancelled", "skipped"].includes(
                states.get(dependency)!.status,
              ),
            );
            if (blocked) {
              changed = true;
              state.status = "skipped";
              state.finished_at = this.now();
              this.onEvent({
                type: "task.skipped",
                task_id: task.id,
                reason: "dependency did not complete",
              });
            }
          }
        }
      };

      const cancelPending = () => {
        for (const state of states.values()) {
          if (state.status === "pending") {
            state.status = "cancelled";
            state.finished_at = this.now();
            this.onEvent({
              type: "task.cancelled",
              task_id: state.task_id,
            });
          }
        }
      };

      const pump = () => {
        if (settled) return;
        if (this.controller.signal.aborted) {
          cancelPending();
          finish();
          return;
        }
        skipBlocked();
        const ready = config.tasks.filter((task) => {
          const state = states.get(task.id)!;
          return (
            state.status === "pending" &&
            (task.depends_on ?? []).every(
              (dependency) => states.get(dependency)!.status === "completed",
            )
          );
        });
        for (const task of ready) {
          if (active >= config.maxConcurrency) break;
          const state = states.get(task.id)!;
          state.status = "running";
          state.started_at = this.now();
          active += 1;
          const controller = new AbortController();
          this.taskControllers.set(task.id, controller);
          const abort = () => controller.abort();
          this.controller.signal.addEventListener("abort", abort, {
            once: true,
          });
          let timedOut = false;
          const timeout = setTimeout(() => {
            timedOut = true;
            controller.abort();
          }, config.timeout);
          const dependencyResults = Object.fromEntries(
            (task.depends_on ?? []).map((dependency) => [
              dependency,
              outputs.get(dependency)!,
            ]),
          );
          this.onEvent({
            type: "task.started",
            task_id: task.id,
            role_id: task.role_id,
          });
          void this.executor(task, {
            signal: controller.signal,
            dependency_results: dependencyResults,
          })
            .then((output) => {
              if (controller.signal.aborted) {
                if (timedOut) {
                  failed = true;
                  state.status = "failed";
                  state.error = "task timed out";
                  this.onEvent({
                    type: "task.failed",
                    task_id: task.id,
                    error: state.error,
                  });
                  if (config.failurePolicy === "fail_fast") this.cancel();
                  return;
                }
                state.status = "cancelled";
                this.onEvent({
                  type: "task.cancelled",
                  task_id: task.id,
                });
                return;
              }
              if (typeof output !== "string") {
                throw new Error("task output must be a string");
              }
              const boundedOutput = output.slice(0, MAX_RESULT_CHARS);
              if (outputChars + boundedOutput.length > config.maxOutputChars) {
                throw new Error("workflow output budget exceeded");
              }
              state.status = "completed";
              state.output = boundedOutput;
              outputChars += state.output.length;
              outputs.set(task.id, state.output);
              this.onEvent({ type: "task.completed", task_id: task.id });
            })
            .catch((error) => {
              if (controller.signal.aborted) {
                if (timedOut) {
                  failed = true;
                  state.status = "failed";
                  state.error = "task timed out";
                  this.onEvent({
                    type: "task.failed",
                    task_id: task.id,
                    error: state.error,
                  });
                  if (config.failurePolicy === "fail_fast") this.cancel();
                  return;
                }
                state.status = "cancelled";
                this.onEvent({
                  type: "task.cancelled",
                  task_id: task.id,
                });
                return;
              }
              failed = true;
              state.status = "failed";
              state.error = errorText(error);
              this.onEvent({
                type: "task.failed",
                task_id: task.id,
                error: state.error,
              });
              if (config.failurePolicy === "fail_fast") this.cancel();
            })
            .finally(() => {
              clearTimeout(timeout);
              this.controller.signal.removeEventListener("abort", abort);
              this.taskControllers.delete(task.id);
              state.finished_at = this.now();
              active -= 1;
              pump();
              finish();
            });
        }
        finish();
      };

      this.controller.signal.addEventListener(
        "abort",
        () => {
          cancelPending();
          finish();
        },
        { once: true },
      );
      pump();
    });
    return final;
  }
}
