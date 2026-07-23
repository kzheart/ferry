import test from "node:test";
import assert from "node:assert/strict";
import { applyEvent, emptyLog } from "./agentChatModel.js";

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

test("replay reconstructs an approval card from persisted tool details", () => {
  let log = applyEvent(emptyLog(), {
    type: "tool.started", timestamp: "2026-01-01T00:00:00Z", run_id: "run",
    payload: { tool_call_id: "call_1", name: "session_edit", args: {} },
  });
  log = applyEvent(log, {
    type: "tool.completed", timestamp: "2026-01-01T00:00:01Z", run_id: "run",
    payload: { tool_call_id: "call_1", is_error: false, result: {
      text: "fallback", details: { status: "pending", operation: {
        operation_id: "op_1", kind: "edit", preview: { changes: [] },
      } },
    } },
  });
  assert.equal(log.items[1].kind, "approval");
  assert.equal(log.items[1].operation.operation_id, "op_1");
  assert.equal(log.items[1].status, "pending");
});
