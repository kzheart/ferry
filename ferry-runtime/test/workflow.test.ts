import { describe, expect, it } from "vitest";
import { WorkflowRun, type WorkflowRunEvent } from "../src/core/workflow.js";

const deferred = <T>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
};

describe("bounded multi-agent workflow", () => {
  it("runs fan-out in parallel and fan-in after both dependencies", async () => {
    const first = deferred<string>();
    const second = deferred<string>();
    const started: string[] = [];
    const run = new WorkflowRun(
      {
        max_concurrency: 2,
        tasks: [
          { id: "research", role_id: "researcher", instruction: "research" },
          { id: "code", role_id: "coder", instruction: "inspect code" },
          {
            id: "review",
            role_id: "reviewer",
            instruction: "merge findings",
            depends_on: ["research", "code"],
          },
        ],
      },
      async (task, context) => {
        started.push(task.id);
        if (task.id === "research") return first.promise;
        if (task.id === "code") return second.promise;
        expect(context.dependency_results).toEqual({
          research: "research result",
          code: "code result",
        });
        return "reviewed";
      },
    );

    const resultPromise = run.start();
    await Promise.resolve();
    expect(started).toEqual(["research", "code"]);
    first.resolve("research result");
    await Promise.resolve();
    expect(started).toEqual(["research", "code"]);
    second.resolve("code result");

    const result = await resultPromise;
    expect(started).toEqual(["research", "code", "review"]);
    expect(result.status).toBe("completed");
    expect(result.tasks.at(-1)).toMatchObject({
      task_id: "review",
      status: "completed",
      output: "reviewed",
    });
  });

  it("never exceeds the configured concurrency", async () => {
    let active = 0;
    let peak = 0;
    const gates = Array.from({ length: 5 }, () => deferred<string>());
    const run = new WorkflowRun(
      {
        max_concurrency: 2,
        tasks: gates.map((_, index) => ({
          id: `task_${index}`,
          role_id: "worker",
          instruction: `task ${index}`,
        })),
      },
      async (task) => {
        active += 1;
        peak = Math.max(peak, active);
        const value = await gates[Number(task.id.slice(5))]!.promise;
        active -= 1;
        return value;
      },
    );

    const resultPromise = run.start();
    await Promise.resolve();
    expect(peak).toBe(2);
    for (const gate of gates) {
      gate.resolve("done");
      await Promise.resolve();
      await Promise.resolve();
    }
    expect((await resultPromise).status).toBe("completed");
    expect(peak).toBe(2);
  });

  it("propagates cancellation to running and pending tasks", async () => {
    const events: WorkflowRunEvent[] = [];
    const run = new WorkflowRun(
      {
        max_concurrency: 1,
        tasks: [
          { id: "active", role_id: "worker", instruction: "wait" },
          { id: "pending", role_id: "worker", instruction: "wait" },
        ],
      },
      (_task, { signal }) =>
        new Promise<string>((_resolve, reject) => {
          signal.addEventListener("abort", () => reject(new Error("aborted")), {
            once: true,
          });
        }),
      (event) => events.push(event),
    );

    const resultPromise = run.start();
    await Promise.resolve();
    run.cancel();
    const result = await resultPromise;

    expect(result.status).toBe("cancelled");
    expect(result.tasks.map((task) => task.status)).toEqual([
      "cancelled",
      "cancelled",
    ]);
    expect(events.at(-1)).toEqual({
      type: "workflow.completed",
      status: "cancelled",
    });
  });

  it("continues independent tasks but skips failed dependencies", async () => {
    const run = new WorkflowRun(
      {
        failure_policy: "continue",
        max_concurrency: 2,
        tasks: [
          { id: "failed", role_id: "worker", instruction: "fail" },
          { id: "independent", role_id: "worker", instruction: "continue" },
          {
            id: "blocked",
            role_id: "reviewer",
            instruction: "blocked",
            depends_on: ["failed"],
          },
        ],
      },
      async (task) => {
        if (task.id === "failed") throw new Error("expected failure");
        return "ok";
      },
    );

    const result = await run.start();

    expect(result.status).toBe("failed");
    expect(
      Object.fromEntries(
        result.tasks.map((task) => [task.task_id, task.status]),
      ),
    ).toEqual({
      failed: "failed",
      independent: "completed",
      blocked: "skipped",
    });
  });

  it("fails a task that would exceed the total output budget", async () => {
    const run = new WorkflowRun(
      {
        max_concurrency: 1,
        max_output_chars: 1_000,
        failure_policy: "continue",
        tasks: [
          { id: "first", role_id: "worker", instruction: "first" },
          { id: "second", role_id: "worker", instruction: "second" },
        ],
      },
      async (task) => (task.id === "first" ? "a".repeat(600) : "b".repeat(600)),
    );

    const result = await run.start();

    expect(result.status).toBe("failed");
    expect(result.tasks).toMatchObject([
      { task_id: "first", status: "completed", output: "a".repeat(600) },
      {
        task_id: "second",
        status: "failed",
        error: "workflow output budget exceeded",
      },
    ]);
  });

  it("rejects cycles, excessive depth and task budgets", async () => {
    await expect(
      new WorkflowRun(
        {
          tasks: [
            {
              id: "a",
              role_id: "worker",
              instruction: "a",
              depends_on: ["b"],
            },
            {
              id: "b",
              role_id: "worker",
              instruction: "b",
              depends_on: ["a"],
            },
          ],
        },
        async () => "unused",
      ).start(),
    ).rejects.toThrow("cycle");

    await expect(
      new WorkflowRun(
        {
          max_depth: 2,
          tasks: [
            { id: "a", role_id: "worker", instruction: "a" },
            {
              id: "b",
              role_id: "worker",
              instruction: "b",
              depends_on: ["a"],
            },
            {
              id: "c",
              role_id: "worker",
              instruction: "c",
              depends_on: ["b"],
            },
          ],
        },
        async () => "unused",
      ).start(),
    ).rejects.toThrow("too deep");

    await expect(
      new WorkflowRun(
        {
          tasks: Array.from({ length: 33 }, (_, index) => ({
            id: `task_${index}`,
            role_id: "worker",
            instruction: "task",
          })),
        },
        async () => "unused",
      ).start(),
    ).rejects.toThrow("budget");
  });

  it("enforces the delegated task timeout", async () => {
    const run = new WorkflowRun(
      {
        task_timeout_ms: 1_000,
        tasks: [{ id: "slow", role_id: "worker", instruction: "wait" }],
      },
      (_task, { signal }) =>
        new Promise<string>((_resolve, reject) => {
          signal.addEventListener("abort", () => reject(new Error("aborted")), {
            once: true,
          });
        }),
    );

    const result = await run.start();
    expect(result).toMatchObject({
      status: "failed",
      tasks: [{ task_id: "slow", status: "failed", error: "task timed out" }],
    });
  }, 2_000);
});
