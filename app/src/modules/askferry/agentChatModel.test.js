import { test } from "vitest";
import assert from "node:assert/strict";
import { createElement } from "react";
import { render } from "@testing-library/react";
import { AgentToolTrace } from "./AgentToolTrace.jsx";
import { applyEvent, emptyLog, patchApproval, TOOL_LEVEL } from "./agentChatModel.js";

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

test("优化 preview 的 rewrite args 变成候选实体,operation.proposed 仍走现有审批卡", () => {
  let log = applyEvent(emptyLog(), {
    type: "tool.started", timestamp: "2026-01-01T00:00:00Z", run_id: "run",
    payload: { tool_call_id: "call_1", name: "session_edit", args: {
      tool: "codex", ref: "fsr_target1", intent: "preview",
      ops: [{ op: "rewrite", locator: "fml_u1", text: "更清晰的提问" }],
    } },
  });
  log = applyEvent(log, {
    type: "tool.completed", timestamp: "2026-01-01T00:00:01Z", run_id: "run",
    payload: { tool_call_id: "call_1", name: "session_edit", is_error: false,
      result: { text: "fallback", details: {
        kind: "edit", ref: "fsr_target1",
        preview: { tool: "codex", changes: [{ locator: "fml_u1" }] },
      } } },
  });
  // 候选实体保留 locator 与完整文本,状态非 applied → UI 标记"尚未写入"
  assert.deepEqual(log.items[0].entities[0].proposals,
    [{ locator: "fml_u1", text: "更清晰的提问" }]);
  assert.notEqual(log.items[0].entities[0].status, "applied");

  // 最终 execute 的 operation.proposed 仍生成现有 pending 审批卡
  log = applyEvent(log, {
    type: "operation.proposed", run_id: "run",
    payload: { tool: "session_edit", operation: { plan_id: "op_opt", kind: "edit" } },
  });
  const approval = log.items.find(item => item.kind === "approval");
  assert.ok(approval, "operation.proposed 没有生成审批卡");
  assert.equal(approval.status, "pending");
  assert.equal(approval.operation.plan_id, "op_opt");
});

test("auto-applied operations render no approval card from either event path", () => {
  let log = applyEvent(emptyLog(), {
    type: "tool.started", timestamp: "2026-01-01T00:00:00Z", run_id: "run",
    payload: { tool_call_id: "call_1", name: "bash", args: {} },
  });
  // 自动模式:tool.completed 信封已是 applied,随后 Rust 补发 operation.applied(auto)
  log = applyEvent(log, {
    type: "tool.completed", timestamp: "2026-01-01T00:00:01Z", run_id: "run",
    payload: { tool_call_id: "call_1", is_error: false, result: {
      text: "ok", details: { status: "applied", operation: { plan_id: "op_a" } },
    } },
  });
  log = applyEvent(log, {
    type: "operation.applied", run_id: "run",
    payload: { tool: "bash", auto: true, operation: { plan_id: "op_a" } },
  });
  assert.equal(log.items.filter(it => it.kind === "approval").length, 0);
});

test("agent_prompt 展示为 mutation 且不生成 approval 卡", () => {
  assert.equal(TOOL_LEVEL.agent_prompt, "mutate");
  let log = applyEvent(emptyLog(), {
    type: "tool.started", timestamp: "2026-01-01T00:00:00Z", run_id: "run",
    payload: { tool_call_id: "call_agent", name: "agent_prompt", args: {
      tool: "codex", ref: "fsr_1", prompt: "继续",
    } },
  });
  log = applyEvent(log, {
    type: "tool.completed", timestamp: "2026-01-01T00:00:01Z", run_id: "run",
    payload: { tool_call_id: "call_agent", is_error: false, result: {
      text: "done", details: {
        status: "pending",
        operation: { plan_id: "untrusted_agent_output", kind: "edit" },
      },
    } },
  });

  assert.equal(log.items[0].name, "agent_prompt");
  assert.equal(log.items.filter(item => item.kind === "approval").length, 0);
});

test("agent_prompt 轨迹显示 mutation 徽章", () => {
  const { container } = render(createElement(AgentToolTrace, {
    rows: [{
      callId: "call_agent",
      name: "agent_prompt",
      args: { tool: "codex", ref: "fsr_1", prompt: "继续" },
      status: "ok",
      result: { text: "done" },
    }],
  }));

  const badge = container.querySelector('[data-tool-level="mutate"]');
  assert.ok(badge);
  assert.equal(badge.textContent.trim(), "settings:roles.mutationBadge");
  assert.ok(container.textContent.includes("settings:roles.tool.agent_prompt.label"));
});

test("manual approval keeps one card that advances in place on applied", () => {
  let log = applyEvent(emptyLog(), {
    type: "operation.proposed", run_id: "run",
    payload: { tool: "migrate", operation: { plan_id: "op_m", kind: "migration" } },
  });
  log = applyEvent(log, {
    type: "operation.applied", run_id: "run",
    payload: { tool: "migrate", auto: false, operation: { plan_id: "op_m" } },
  });
  const cards = log.items.filter(it => it.kind === "approval");
  assert.equal(cards.length, 1);
  assert.equal(cards[0].status, "applied");
  assert.equal(cards[0].operation.kind, "migration");
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
