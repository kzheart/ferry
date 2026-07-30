// 扫描失败时的状态:错误必须是这一次的,上次扫到的会话则要留着继续可用。
// 两者都错了的话,界面要么报着过期的故障原因,要么直接让用户的列表凭空清空。
import { afterEach, test, vi } from "vitest";
import assert from "node:assert/strict";
import { act, render, waitFor } from "@testing-library/react";

let scanImpl = async () => ({ tools: {}, sessions: [] });
let engineEventHandler = null;

vi.mock("../../platform/desktop/client.js", async importOriginal => ({
  ...(await importOriginal()),
  engine: async method => (method === "scan" ? scanImpl() : {}),
  onEngineEvent: async handler => {
    engineEventHandler = handler;
    return () => { engineEventHandler = null; };
  },
}));
vi.mock("../../platform/desktop/cache.js", () => ({
  cacheGet: async () => null,
  cacheSet: () => {},
}));

const { useBrowserData } = await import("./useBrowserData.js");

afterEach(() => {
  scanImpl = async () => ({ tools: {}, sessions: [] });
  engineEventHandler = null;
});

function mount() {
  let value;
  function Probe() {
    value = useBrowserData();
    return null;
  }
  render(<Probe />);
  return () => value;
}

test("扫描失败时记下错误,scanReady 保持假", async () => {
  scanImpl = async () => { throw new Error("permission denied"); };
  const state = mount();

  await waitFor(() => assert.equal(state().scan?.error, "permission denied"));
  assert.equal(state().scanReady, false);
});

test("扫描失败保留上次扫到的会话,列表不会凭空清空", async () => {
  const state = mount();
  await waitFor(() => assert.equal(state().scanReady, true));

  scanImpl = async () => ({ tools: { claude: {} }, sessions: [{ id: "a" }] });
  await act(async () => { await state().doScan(); });
  await waitFor(() => assert.equal(state().scan.sessions.length, 1));

  scanImpl = async () => { throw new Error("engine offline"); };
  await act(async () => { await state().doScan(); });

  await waitFor(() => assert.equal(state().scan.error, "engine offline"));
  assert.equal(state().scan.sessions.length, 1, "旧结果还在");
});

test("连续两次失败时报的是最新的原因,不是上一次留下的", async () => {
  scanImpl = async () => { throw new Error("第一次的原因"); };
  const state = mount();
  await waitFor(() => assert.equal(state().scan?.error, "第一次的原因"));

  scanImpl = async () => { throw new Error("第二次的原因"); };
  await act(async () => { await state().doScan(); });

  await waitFor(() => assert.equal(state().scan.error, "第二次的原因"));
});

test("sessions.changed 增量并入列表:更新、新增、删除且按 updated 排序", async () => {
  scanImpl = async () => ({
    tools: {}, generation: 3,
    sessions: [
      { ref: "fsr_a", id: "a", updated: 30 },
      { ref: "fsr_b", id: "b", updated: 20 },
    ],
  });
  const state = mount();
  await waitFor(() => assert.equal(state().scan.sessions.length, 2));
  await waitFor(() => assert.notEqual(engineEventHandler, null));

  await act(async () => {
    engineEventHandler({
      type: "sessions.changed",
      payload: {
        generation: 4,
        upserts: [
          { ref: "fsr_b", id: "b", updated: 50 },
          { ref: "fsr_c", id: "c", updated: 40 },
        ],
        removals: ["fsr_a"],
      },
    });
  });

  const sessions = state().scan.sessions;
  assert.deepEqual(sessions.map(s => s.ref), ["fsr_b", "fsr_c"]);
  assert.equal(sessions[0].updated, 50);
  assert.equal(state().scan.generation, 4);
});

test("同一会话被换发 ref 时按身份收敛,不出现两行鬼影", async () => {
  scanImpl = async () => ({
    tools: {}, generation: 3,
    sessions: [{ ref: "fsr_old", tool: "claude", id: "s1", updated: 10 }],
  });
  const state = mount();
  await waitFor(() => assert.equal(state().scan.sessions.length, 1));
  await waitFor(() => assert.notEqual(engineEventHandler, null));

  // 引擎侧误判后重签发:removal 丢失、只来了带新 ref 的 upsert
  await act(async () => {
    engineEventHandler({
      type: "sessions.changed",
      payload: {
        generation: 4,
        upserts: [{ ref: "fsr_new", tool: "claude", id: "s1", updated: 20 }],
        removals: [],
      },
    });
  });

  const sessions = state().scan.sessions;
  assert.equal(sessions.length, 1, "同一会话只能有一行");
  assert.equal(sessions[0].ref, "fsr_new");
});

test("代际断档时静默全量重拉,不套用不完整的增量", async () => {
  scanImpl = async () => ({
    tools: {}, generation: 3,
    sessions: [{ ref: "fsr_a", id: "a", updated: 1 }],
  });
  const state = mount();
  await waitFor(() => assert.equal(state().scan.sessions.length, 1));
  await waitFor(() => assert.notEqual(engineEventHandler, null));

  scanImpl = async () => ({
    tools: {}, generation: 9,
    sessions: [{ ref: "fsr_z", id: "z", updated: 2 }],
  });
  await act(async () => {
    engineEventHandler({
      type: "sessions.changed",
      payload: { generation: 8, upserts: [], removals: ["fsr_a"] },
    });
  });

  await waitFor(() => assert.deepEqual(
    state().scan.sessions.map(s => s.ref), ["fsr_z"],
  ));
  assert.equal(state().scan.generation, 9);
});

test("失败后再扫成功,错误随整份结果一起被换掉", async () => {
  scanImpl = async () => { throw new Error("boom"); };
  const state = mount();
  await waitFor(() => assert.equal(state().scan?.error, "boom"));

  scanImpl = async () => ({ tools: {}, sessions: [{ id: "a" }] });
  await act(async () => { await state().doScan(); });

  await waitFor(() => assert.equal(state().scanReady, true));
  assert.equal(state().scan.error, undefined);
});
