import test from "node:test";
import assert from "node:assert/strict";
import { applyEvent, emptyLog, patchApproval } from "./agentChatModel.js";

test("structured tool details survive event reduction and become entities", () => {
  let log = applyEvent(emptyLog(), {
    type: "tool.started", timestamp: "2026-01-01T00:00:00Z",
    payload: { tool_call_id: "call_1", name: "session_search", args: { query: "ferry" } },
  });
  const details = { sessions: [{ tool: "codex", ref: "fsr_1", title: "Ferry" }] };
  log = applyEvent(log, {
    type: "tool.completed", timestamp: "2026-01-01T00:00:01Z",
    payload: { tool_call_id: "call_1", name: "session_search",
      is_error: false, result: { text: "fallback", details } },
  });
  assert.equal(log.items[0].result.details, details);
  assert.equal(log.items[0].entities[0].title, "Ferry");
});

test("replay reconstructs an approval card from a persisted operation plan", () => {
  let log = applyEvent(emptyLog(), {
    type: "tool.started", timestamp: "2026-01-01T00:00:00Z", run_id: "run",
    payload: { tool_call_id: "call_1", name: "session_edit", args: {} },
  });
  log = applyEvent(log, {
    type: "tool.completed", timestamp: "2026-01-01T00:00:01Z", run_id: "run",
    payload: { tool_call_id: "call_1", is_error: false, result: {
      text: "fallback", details: { status: "pending", operation: {
        plan_id: "op_1", kind: "edit", preview: { changes: [] },
      } },
    } },
  });
  assert.equal(log.items[1].kind, "approval");
  assert.equal(log.items[1].operation.plan_id, "op_1");
  assert.equal(log.items[1].status, "pending");
});

test("operation plans use plan_id as the approval identity", () => {
  const log = applyEvent(emptyLog(), {
    type: "operation.proposed",
    run_id: "run-1",
    payload: {
      tool: "migrate",
      operation: { plan_id: "op_plan", kind: "migration", preview: {} },
    },
  });
  const updated = patchApproval(log, "op_plan", { status: "applied" });
  assert.equal(updated.items[0].status, "applied");
});

test("workflow events expose parallel agent task state in the parent chat", () => {
  let log = applyEvent(emptyLog(), {
    type: "workflow.started",
    run_id: "run-1",
    payload: { task_count: 2 },
  });
  log = applyEvent(log, {
    type: "task.started",
    run_id: "run-1",
    payload: { task_id: "research", role_id: "researcher" },
  });
  log = applyEvent(log, {
    type: "task.started",
    run_id: "run-1",
    payload: { task_id: "review", role_id: "reviewer" },
  });
  log = applyEvent(log, {
    type: "task.completed",
    run_id: "run-1",
    payload: { task_id: "research" },
  });
  log = applyEvent(log, {
    type: "workflow.completed",
    run_id: "run-1",
    payload: { status: "completed" },
  });

  assert.deepEqual(log.items[0], {
    kind: "workflow",
    runId: "run-1",
    status: "completed",
    taskCount: 2,
    tasks: [
      { id: "research", roleId: "researcher", status: "completed" },
      { id: "review", roleId: "reviewer", status: "running" },
    ],
  });
});
