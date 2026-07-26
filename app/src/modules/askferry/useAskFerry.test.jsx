import { expect, test, vi } from "vitest";
import { act, render } from "@testing-library/react";

const applied = { operation: [], shell: [] };
// 事件订阅的回调:测试里直接调它来模拟 runtime 推事件
let emit = () => {};
let sessionList = [];

// bash 提案与 Engine 提案共用同一张审批卡,分流只看 plan_id 前缀——这条走错就会
// 把 shell 命令送进 Engine 的 operation 状态机。
vi.mock("../../platform/desktop/client.js", async (importOriginal) => ({
  ...(await importOriginal()),
  onRuntimeEvent: async (handler) => {
    emit = handler;
    return () => {};
  },
  runtime: async (method) => (method === "sessions.list" ? sessionList : {}),
  operationApply: async (planId) => {
    applied.operation.push(planId);
    return { status: "applied" };
  },
  shellApply: async (planId) => {
    applied.shell.push(planId);
    return { exit_code: 0, stdout: "hello\n" };
  },
}));

const { useAskFerry } = await import("./useAskFerry.js");

function mount() {
  let api;
  function Probe() {
    api = useAskFerry();
    return null;
  }
  const view = render(<Probe />);
  return { get: () => api, unmount: () => view.unmount() };
}

async function mountWithSessions(list) {
  sessionList = list;
  const harness = mount();
  // 订阅与 sessions.list 都是 promise,等一拍让初始列表落到 state
  await act(async () => {});
  return harness;
}

const attentionOf = (harness, id) =>
  harness.get().sessions.find((s) => s.session_id === id)?.attention ?? null;

test("bash 提案走 shellApply,Engine 提案走 operationApply", async () => {
  const harness = mount();

  await act(async () => {
    await harness.get().approve("s1", { operation: { plan_id: "shl_abc" } });
  });
  expect(applied.shell).toEqual(["shl_abc"]);
  expect(applied.operation).toEqual([]);

  await act(async () => {
    await harness.get().approve("s1", { operation: { plan_id: "op_xyz" } });
  });
  expect(applied.operation).toEqual(["op_xyz"]);
  expect(applied.shell).toEqual(["shl_abc"]);

  harness.unmount();
});

test("后台会话按事件累积 attention,审批优先级最高且不被完成事件冲掉", async () => {
  const harness = await mountWithSessions([{ session_id: "s1", status: "idle" }]);

  await act(async () => emit({ type: "run.completed", session_id: "s1" }));
  expect(attentionOf(harness, "s1")).toBe("unread");

  await act(async () => emit({ type: "run.failed", session_id: "s1" }));
  expect(attentionOf(harness, "s1")).toBe("error");

  await act(async () => emit({ type: "tool.request", session_id: "s1" }));
  expect(attentionOf(harness, "s1")).toBe("approval");

  // 低等级不覆盖高等级
  await act(async () => emit({ type: "run.completed", session_id: "s1" }));
  expect(attentionOf(harness, "s1")).toBe("approval");

  // 新一轮开始意味着用户已经在推进这个会话
  await act(async () => emit({ type: "run.started", session_id: "s1" }));
  expect(attentionOf(harness, "s1")).toBe(null);
  expect(harness.get().sessions[0].status).toBe("running");

  harness.unmount();
});

test("session.renamed 更新标题;手动改名(auto=false)顺带锁住标题", async () => {
  const harness = await mountWithSessions([{ session_id: "s1", status: "idle" }]);
  const titleOf = () => harness.get().sessions[0];

  await act(async () => emit({
    type: "session.renamed", session_id: "s1",
    payload: { session_id: "s1", title: "检索会话历史", auto: true },
  }));
  expect(titleOf()).toMatchObject({ title: "检索会话历史" });
  expect(titleOf().title_locked).toBeFalsy();

  await act(async () => emit({
    type: "session.renamed", session_id: "s1",
    payload: { session_id: "s1", title: "我自己起的", auto: false },
  }));
  expect(titleOf()).toMatchObject({ title: "我自己起的", title_locked: true });

  // 自动命名事件不该把锁解开
  await act(async () => emit({
    type: "session.renamed", session_id: "s1",
    payload: { session_id: "s1", title: "又一个自动标题", auto: true },
  }));
  expect(titleOf().title_locked).toBe(true);

  // 改名事件不该被当成后台活动挂徽标
  expect(attentionOf(harness, "s1")).toBe(null);

  harness.unmount();
});

test("当前会话不攒 attention,打开会话会清掉已有徽标", async () => {
  const harness = await mountWithSessions([
    { session_id: "s1", status: "idle" },
    { session_id: "s2", status: "idle" },
  ]);

  await act(async () => emit({ type: "run.completed", session_id: "s2" }));
  expect(attentionOf(harness, "s2")).toBe("unread");

  await act(async () => harness.get().openSession("s2"));
  expect(attentionOf(harness, "s2")).toBe(null);

  // s2 已是 activeId,后续事件不再挂徽标
  await act(async () => emit({ type: "operation.proposed", session_id: "s2" }));
  expect(attentionOf(harness, "s2")).toBe(null);
  // 其他会话照常
  await act(async () => emit({ type: "operation.proposed", session_id: "s1" }));
  expect(attentionOf(harness, "s1")).toBe("approval");

  harness.unmount();
});
