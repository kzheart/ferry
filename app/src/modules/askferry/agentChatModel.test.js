import { test } from "vitest";
import assert from "node:assert/strict";
import { createElement } from "react";
import { render } from "@testing-library/react";
import { AgentToolTrace } from "./AgentToolTrace.jsx";
import { applyEvent, emptyLog, patchApproval, patchChoice, TOOL_LEVEL } from "./agentChatModel.js";

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

test("choice.requested creates a pending choice and choice.resolved answers it", () => {
  let log = applyEvent(emptyLog(), {
    type: "choice.requested", run_id: "run-choice",
    payload: {
      request_id: "req-1", tool_call_id: "call-choice", question: "清理哪些会话?",
      options: [{ label: "旧会话", recommended: true }, { label: "全部" }],
      multi_select: false, allow_custom: true,
    },
  });
  assert.deepEqual(log.items[0], {
    kind: "choice", requestId: "req-1", callId: "call-choice",
    question: "清理哪些会话?",
    options: [{ label: "旧会话", recommended: true }, { label: "全部" }],
    multiSelect: false, allowCustom: true, status: "pending", answered: false,
    selected: [], customText: "", runId: "run-choice", requestedAt: undefined,
  });
  log = applyEvent(log, {
    type: "choice.resolved", run_id: "run-choice",
    payload: { request_id: "req-1", answered: true, selected: ["旧会话"], custom_text: "" },
  });
  assert.equal(log.items[0].status, "answered");
  assert.deepEqual(log.items[0].selected, ["旧会话"]);
});

test("ask_user replay normalizes tool.started and tool.completed into one answered card", () => {
  let log = applyEvent(emptyLog(), {
    type: "tool.started", run_id: "run-replay", timestamp: "2026-01-01T00:00:00Z",
    payload: { tool_call_id: "call-replay", name: "ask_user", args: {
      question: "继续吗?", options: [{ label: "继续" }, { label: "停止" }],
    } },
  });
  log = applyEvent(log, {
    type: "tool.completed", run_id: "run-replay", timestamp: "2026-01-01T00:00:01Z",
    payload: { tool_call_id: "call-replay", name: "ask_user", is_error: false,
      result: { text: "answer", details: {
        answered: true, selected: ["继续"], custom_text: "",
      } } },
  });
  const choice = log.items.find(item => item.kind === "choice");
  assert.equal(choice.status, "answered");
  assert.deepEqual(choice.selected, ["继续"]);
  assert.equal(log.items.filter(item => item.kind === "choice").length, 1);
});

test("run terminal marks an unanswered choice without inventing a selection", () => {
  let log = applyEvent(emptyLog(), {
    type: "choice.requested", run_id: "run-pending",
    payload: { request_id: "req-pending", question: "选择", options: [{ label: "A" }, { label: "B" }] },
  });
  log = applyEvent(log, { type: "run.cancelled", run_id: "run-pending", payload: {} });
  assert.equal(log.items[0].status, "unanswered");
  assert.deepEqual(log.items[0].selected, []);
});

test("choice response patches only the matching timeline item", () => {
  let log = applyEvent(emptyLog(), {
    type: "choice.requested", payload: {
      request_id: "req-a", question: "A", options: [{ label: "1" }, { label: "2" }],
    },
  });
  log = applyEvent(log, {
    type: "choice.requested", payload: {
      request_id: "req-b", question: "B", options: [{ label: "x" }, { label: "y" }],
    },
  });
  const updated = patchChoice(log, "req-b", {
    status: "answered", answered: true, selected: ["y"], customText: "",
  });
  assert.equal(updated.items[0].status, "pending");
  assert.equal(updated.items[1].status, "answered");
});

test("回放靠 tool.request 补回 request_id:卡片能用它应答,不再退化成 tool_call_id", () => {
  // events.replay 里只有带 seq 的 runtime 事件;host 的 choice.requested 不在其中
  let log = applyEvent(emptyLog(), {
    type: "tool.started", run_id: "run-replay", seq: 1,
    payload: { tool_call_id: "call-replay", name: "ask_user", args: {
      question: "继续吗?", options: [{ label: "继续" }, { label: "停止" }],
      allow_custom: true,
    } },
  });
  assert.equal(log.items.find(item => item.kind === "choice").requestId, "call-replay");

  log = applyEvent(log, {
    type: "tool.request", run_id: "run-replay", seq: 2,
    payload: { request_id: "req-replay", tool_call_id: "call-replay",
      name: "ask_user", args: { question: "继续吗?",
        options: [{ label: "继续" }, { label: "停止" }], allow_custom: true } },
  });

  const choices = log.items.filter(item => item.kind === "choice");
  assert.equal(choices.length, 1);
  // 应答要用 request_id:宿主的挂起表就是按它做键的
  assert.equal(choices[0].requestId, "req-replay");
  assert.equal(choices[0].callId, "call-replay");
  assert.equal(choices[0].status, "pending");
  assert.equal(choices[0].allowCustom, true);
});

test("tool.request 只为 ask_user 建卡,别的工具不产生选择卡", () => {
  const log = applyEvent(emptyLog(), {
    type: "tool.request", run_id: "run", seq: 1,
    payload: { request_id: "req", tool_call_id: "call", name: "bash",
      args: { command: "ls" } },
  });
  assert.equal(log.items.filter(item => item.kind === "choice").length, 0);
});

test("choice.resolved 早于 requested 到达时答案不丢,后到的问题补齐同一张卡", () => {
  let log = applyEvent(emptyLog(), {
    type: "choice.resolved", run_id: "run-race",
    payload: { request_id: "req-race", tool_call_id: "call-race",
      answered: true, selected: ["继续"], custom_text: "" },
  });
  assert.equal(log.items[0].status, "answered");

  log = applyEvent(log, {
    type: "choice.requested", run_id: "run-race",
    payload: { request_id: "req-race", tool_call_id: "call-race",
      question: "继续吗?", options: [{ label: "继续" }, { label: "停止" }] },
  });

  const choices = log.items.filter(item => item.kind === "choice");
  assert.equal(choices.length, 1);
  // 补发的 requested 不能把已有答案打回 pending
  assert.equal(choices[0].status, "answered");
  assert.deepEqual(choices[0].selected, ["继续"]);
  assert.equal(choices[0].question, "继续吗?");
});

test("choice.resolved 只带 tool_call_id 也能命中回放建出来的卡", () => {
  let log = applyEvent(emptyLog(), {
    type: "tool.started", run_id: "run", seq: 1,
    payload: { tool_call_id: "call-x", name: "ask_user",
      args: { question: "?", options: [{ label: "A" }, { label: "B" }] } },
  });
  log = applyEvent(log, {
    type: "choice.resolved", run_id: "run",
    payload: { request_id: "req-x", tool_call_id: "call-x",
      answered: true, selected: ["A"], custom_text: "" },
  });

  const choices = log.items.filter(item => item.kind === "choice");
  assert.equal(choices.length, 1);
  assert.equal(choices[0].status, "answered");
});

// run 已终态时 Rust 直接返回 answered:false,既不发 choice.requested 也不发
// choice.resolved;卡片是 tool.request 建出来的,只能靠 tool.completed 落地。
test("run 终态下的 ask_user:tool.completed 把卡片落到未作答只读态", () => {
  const args = { question: "继续吗?", options: [{ label: "继续" }, { label: "停止" }] };
  let log = applyEvent(emptyLog(), {
    type: "run.started", run_id: "run-late", seq: 1, payload: { prompt: "p" },
  });
  log = applyEvent(log, {
    type: "tool.started", run_id: "run-late", seq: 2,
    payload: { tool_call_id: "call-late", name: "ask_user", args },
  });
  log = applyEvent(log, {
    type: "tool.request", run_id: "run-late", seq: 3,
    payload: { request_id: "req-late", tool_call_id: "call-late",
      name: "ask_user", args },
  });
  assert.equal(log.items.find(item => item.kind === "choice").status, "pending");

  // Engine 侧把宿主的默认答案原样带回来
  log = applyEvent(log, {
    type: "tool.completed", run_id: "run-late", seq: 4,
    payload: { tool_call_id: "call-late", name: "ask_user", is_error: false,
      result: { text: "{}", details: { answered: false, selected: [] } } },
  });

  const choices = log.items.filter(item => item.kind === "choice");
  assert.equal(choices.length, 1);
  assert.equal(choices[0].status, "unanswered");
  assert.equal(choices[0].answered, false);
  assert.deepEqual(choices[0].selected, []);
});

test("ask_user 以 is_error 收尾时同样落到未作答,不会假装拿到了选择", () => {
  let log = applyEvent(emptyLog(), {
    type: "tool.started", run_id: "run", seq: 1,
    payload: { tool_call_id: "call", name: "ask_user",
      args: { question: "?", options: [{ label: "A" }, { label: "B" }] } },
  });
  log = applyEvent(log, {
    type: "tool.completed", run_id: "run", seq: 2,
    payload: { tool_call_id: "call", name: "ask_user", is_error: true,
      result: { text: "tool gateway timed out" } },
  });

  const choice = log.items.find(item => item.kind === "choice");
  assert.equal(choice.status, "unanswered");
  assert.deepEqual(choice.selected, []);
});

// tool.completed 可能永远不来(运行被拆掉、sidecar 掉线),卡片就得靠 run 终态兜底。
// 这里的卡是在终态事件之后才补到的,所以兜底不能按 runId 过滤。
test("run 终态之后才补到的挂起卡,由下一次 run 终态兜底成未作答", () => {
  let log = applyEvent(emptyLog(), {
    type: "run.started", run_id: "run-1", seq: 1, payload: { prompt: "p" },
  });
  log = applyEvent(log, {
    type: "run.completed", run_id: "run-1", seq: 2, payload: {},
  });
  log = applyEvent(log, {
    type: "tool.request", run_id: "run-1", seq: 3,
    payload: { request_id: "req-orphan", tool_call_id: "call-orphan",
      name: "ask_user",
      args: { question: "?", options: [{ label: "A" }, { label: "B" }] } },
  });
  const orphan = () => log.items.find(item => item.kind === "choice");
  assert.equal(orphan().status, "pending");

  // 下一轮结束时这张孤儿卡必须被收掉,否则它永远可点,点了只会拿到 request_not_found
  log = applyEvent(log, {
    type: "run.started", run_id: "run-2", seq: 4, payload: { prompt: "p2" },
  });
  log = applyEvent(log, {
    type: "run.interrupted", run_id: "run-2", seq: 5, payload: {},
  });
  assert.equal(orphan().status, "unanswered");
  assert.equal(orphan().answered, false);
});
