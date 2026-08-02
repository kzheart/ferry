import { expect, test, vi } from "vitest";
import { act, render } from "@testing-library/react";
import { useEffect, useState } from "react";

const applied = { operation: [], shell: [] };
// 选择应答:记录调用参数,并允许用例让宿主拒收这次应答
const choiceCalls = [];
let choiceRespondFails = false;
// 事件订阅的回调:测试里直接调它来模拟 runtime 推事件
let emit = () => {};
let sessionList = [];
// 记录 runtime 命令调用,创建类断言用
const runtimeCalls = [];
let createdCount = 0;

// bash 提案与 Engine 提案共用同一张审批卡,分流只看 plan_id 前缀——这条走错就会
// 把 shell 命令送进 Engine 的 operation 状态机。
vi.mock("../../platform/desktop/client.js", async (importOriginal) => ({
  ...(await importOriginal()),
  onRuntimeEvent: async (handler) => {
    emit = handler;
    return () => {};
  },
  runtime: async (method, params) => {
    runtimeCalls.push({ method, params });
    if (method === "sessions.list") return sessionList;
    // 回放接口返回事件数组;打开会话要靠它把时间线建出来
    if (method === "events.replay") return [];
    if (method === "session.create") {
      return { session_id: `created-${++createdCount}` };
    }
    return {};
  },
  operationApply: async (planId) => {
    applied.operation.push(planId);
    return { status: "applied" };
  },
  shellApply: async (planId) => {
    applied.shell.push(planId);
    return { exit_code: 0, stdout: "hello\n" };
  },
  choiceRespond: async (sessionId, requestId, answer) => {
    choiceCalls.push({ sessionId, requestId, answer });
    if (choiceRespondFails) throw new Error("choice.request_not_found");
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

test("新会话按 purpose 创建;后续消息不重建,普通 newChat 重置 general", async () => {
  runtimeCalls.length = 0;
  const harness = await mountWithSessions([]);
  const creates = () =>
    runtimeCalls.filter((call) => call.method === "session.create");

  // 优化新会话:session.create 带 purpose
  await act(async () => harness.get().newChat("session-optimization"));
  await act(async () => { await harness.get().send("优化这段会话"); });
  expect(creates()).toHaveLength(1);
  expect(creates()[0].params).toMatchObject({
    role_id: "default",
    purpose: "session-optimization",
  });

  // 已有会话继续发消息:不再创建,purpose 不可切换
  await act(async () => { await harness.get().send("继续调整"); });
  expect(creates()).toHaveLength(1);

  // 普通 newChat(即使收到点击事件对象)必须回到 general 语义:省略 purpose
  await act(async () => harness.get().newChat({ type: "click" }));
  await act(async () => { await harness.get().send("普通对话"); });
  expect(creates()).toHaveLength(2);
  expect(creates()[1].params).not.toHaveProperty("purpose");

  harness.unmount();
});

// 返回对象经 context 发给整棵 Ask Ferry 子树:每次渲染新建一个,根组件任何
// state 变化都会把所有消费者连带重渲染,并且把 toast 的自动关闭计时器一直重置掉。
function mountWithToast() {
  let api;
  let bump = () => {};
  function Probe() {
    const [, setTick] = useState(0);
    bump = () => setTick((tick) => tick + 1);
    const ferry = useAskFerry();
    api = ferry;
    // 与 AskFerry.jsx 的错误提示自动关闭逻辑一致
    useEffect(() => {
      if (!ferry.lastError) return undefined;
      const id = setTimeout(ferry.clearError, 6000);
      return () => clearTimeout(id);
    }, [ferry.lastError, ferry.clearError]);
    return null;
  }
  const view = render(<Probe />);
  return { get: () => api, bump: () => bump(), unmount: () => view.unmount() };
}

test("无关重渲染时返回值引用不变", async () => {
  sessionList = [];
  const harness = mountWithToast();
  await act(async () => {});
  const before = harness.get();

  await act(async () => harness.bump());

  expect(harness.get()).toBe(before);
  harness.unmount();
});

test("错误提示约 6 秒后自动清除，中途的无关重渲染不会重置计时器", async () => {
  vi.useFakeTimers();
  try {
    sessionList = [];
    const harness = mountWithToast();
    await act(async () => {});
    act(() => harness.get().reportError(new Error("boom")));
    expect(harness.get().lastError).toBeTruthy();

    await act(async () => vi.advanceTimersByTime(3000));
    await act(async () => harness.bump());
    await act(async () => vi.advanceTimersByTime(3100));

    expect(harness.get().lastError).toBe(null);
    harness.unmount();
  } finally {
    vi.useRealTimers();
  }
});

function mountCounting() {
  let api;
  let renders = 0;
  function Probe() {
    api = useAskFerry();
    renders += 1;
    return null;
  }
  const view = render(<Probe />);
  return { get: () => api, renders: () => renders, unmount: () => view.unmount() };
}

test("连续 delta 按帧合并成一次更新，非 delta 事件到达时立即冲刷且不乱序", async () => {
  vi.useFakeTimers();
  try {
    sessionList = [{ session_id: "s1", title: "chat" }];
    const harness = mountCounting();
    await act(async () => {});
    await act(async () => harness.get().openSession("s1"));

    const before = harness.renders();
    for (let seq = 1; seq <= 5; seq += 1) {
      await act(async () =>
        emit({ type: "content.delta", session_id: "s1", seq,
          payload: { delta: `d${seq}` } }));
    }
    // 缓冲期内一次都不该重渲染,时间线也还没变
    expect(harness.renders()).toBe(before);
    expect(harness.get().activeLog.items).toEqual([]);

    await act(async () => vi.advanceTimersByTime(20));
    expect(harness.get().activeLog.items.map((item) => item.text))
      .toEqual(["d1d2d3d4d5"]);
    expect(harness.renders()).toBe(before + 1);

    // 非 delta 事件必须先把缓冲落地,否则工具行会插到未冲刷的文本前面
    await act(async () =>
      emit({ type: "content.delta", session_id: "s1", seq: 6,
        payload: { delta: "d6" } }));
    await act(async () =>
      emit({ type: "tool.started", session_id: "s1", seq: 7,
        payload: { tool_call_id: "c1", name: "session_search", args: {} } }));

    expect(harness.get().activeLog.items.map((item) => [item.kind, item.text]))
      .toEqual([["assistant", "d1d2d3d4d5d6"], ["tool", undefined]]);
    harness.unmount();
  } finally {
    vi.useRealTimers();
  }
});

// 选择卡应答:成功要把卡片推进到已回答,失败必须退回 pending——
// 否则卡片停在「已回答」而 Runtime 一无所知,用户既看不出失败也没法重试。
async function mountWithPendingChoice() {
  sessionList = [{ session_id: "s1", title: "chat" }];
  const harness = mount();
  await act(async () => {});
  await act(async () => harness.get().openSession("s1"));
  await act(async () => emit({
    type: "choice.requested", session_id: "s1", run_id: "run-1",
    payload: { request_id: "req-1", tool_call_id: "call-1", question: "选哪个?",
      options: [{ label: "A" }, { label: "B" }], allow_custom: true },
  }));
  return harness;
}
const choiceItem = harness =>
  harness.get().activeLog.items.find((item) => item.kind === "choice");

test("respondChoice 成功后卡片变成已回答,并带上 sessionId 调用宿主", async () => {
  choiceRespondFails = false;
  choiceCalls.length = 0;
  const harness = await mountWithPendingChoice();
  expect(choiceItem(harness).status).toBe("pending");

  await act(async () => {
    await harness.get().respondChoice("s1", "req-1",
      { answered: true, selected: ["A"], custom_text: "备注" });
  });

  expect(choiceCalls).toEqual([{
    sessionId: "s1", requestId: "req-1",
    answer: { answered: true, selected: ["A"], custom_text: "备注" },
  }]);
  expect(choiceItem(harness)).toMatchObject({
    status: "answered", answered: true, selected: ["A"], customText: "备注",
  });
  harness.unmount();
});

test("respondChoice 失败回滚成 pending,并原样留下已选项与自由输入", async () => {
  choiceRespondFails = true;
  choiceCalls.length = 0;
  const harness = await mountWithPendingChoice();

  await act(async () => {
    await harness.get().respondChoice("s1", "req-1",
      { answered: true, selected: ["B"], custom_text: "写了一半" });
  });

  expect(choiceItem(harness)).toMatchObject({
    status: "pending", answered: false, selected: ["B"], customText: "写了一半",
  });
  expect(harness.get().lastError).toBeTruthy();
  choiceRespondFails = false;

  // 回到 pending 之后必须能重试,并且这次真的推进到已回答
  await act(async () => {
    await harness.get().respondChoice("s1", "req-1",
      { answered: true, selected: ["B"], custom_text: "写了一半" });
  });
  expect(choiceItem(harness).status).toBe("answered");
  expect(choiceCalls).toHaveLength(2);
  harness.unmount();
});

test("跳过(answered:false)记成未作答,不写入任何选择", async () => {
  choiceRespondFails = false;
  choiceCalls.length = 0;
  const harness = await mountWithPendingChoice();

  await act(async () => {
    await harness.get().respondChoice("s1", "req-1",
      { answered: false, selected: [], custom_text: "" });
  });

  expect(choiceItem(harness)).toMatchObject({
    status: "unanswered", answered: false, selected: [],
  });
  expect(choiceCalls[0].answer.answered).toBe(false);
  harness.unmount();
});
