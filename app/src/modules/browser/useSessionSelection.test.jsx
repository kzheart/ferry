// 内容变更触发的详情重载必须保住已加载窗口:打回第一页会把
// 正读着的用户甩离原位(滚动位置随内容高度骤减被夹住)。
import { test, vi, afterEach } from "vitest";
import assert from "node:assert/strict";
import { act, render, waitFor } from "@testing-library/react";

const showCalls = [];
let showImpl = async () => ({ messages: [], turns: [], returned_message_count: 0 });

vi.mock("../../platform/desktop/client.js", () => ({
  engine: async (method, params) => {
    if (method !== "show") return {};
    showCalls.push(params);
    return showImpl(params);
  },
}));

const { useSessionSelection } = await import("./useSessionSelection.js");
const { sessionIdentity } = await import("./sessionAttachment.js");

afterEach(() => { showCalls.length = 0; });

function mount(props) {
  let value;
  function Probe(p) {
    value = useSessionSelection(p.hook);
    return null;
  }
  const view = render(<Probe hook={props} />);
  return {
    state: () => value,
    update: next => view.rerender(<Probe hook={next} />),
  };
}

const session = revision => ({
  tool: "claude", id: "s1", ref: "fsr_s1", revision,
  updated: 1, size: 10,
});

test("已读到末尾的会话在内容变更后整段重载,不打回第一页", async () => {
  showImpl = async () => ({
    messages: [{}], turns: [], returned_message_count: 40,
    next_from_message: null,
  });
  const props = {
    sessions: [session("r1")], ready: true,
    onSelect: () => {}, onFallbackLoad: () => {},
  };
  const view = mount(props);
  await act(async () => { view.state().select(sessionIdentity(session("r1"))); });
  await waitFor(() => assert.equal(showCalls.length, 1));
  assert.equal(showCalls[0].limit, 30, "首次加载仍是第一页");
  await waitFor(() =>
    assert.equal(view.state().detail?.data?.returned_message_count, 40));

  // 扫描发现内容变了(revision 变化)→ 自动重载
  await act(async () => {
    view.update({ ...props, sessions: [session("r2")] });
  });
  await waitFor(() => assert.equal(showCalls.length, 2));
  assert.equal(
    showCalls[1].limit, undefined,
    "无下一页说明看到了结尾,重载必须整段拉取",
  );
});

test("跳到最新的整段拉取不带 limit,且同步锁挡住并发的哨兵分页", async () => {
  showImpl = async () => ({
    messages: [{}], turns: [], returned_message_count: 30,
    next_from_message: 31,
  });
  const view = mount({
    sessions: [session("r1")], ready: true,
    onSelect: () => {}, onFallbackLoad: () => {},
  });
  await act(async () => { view.state().select(sessionIdentity(session("r1"))); });
  await waitFor(() =>
    assert.equal(view.state().detail?.data?.next_from_message, 31));

  showImpl = async () => ({
    messages: [{}], turns: [], returned_message_count: 60,
    next_from_message: null,
  });
  await act(async () => {
    view.state().loadMore(true); // 跳底按钮:拉全剩余
    view.state().loadMore(); // 哨兵同时触发:必须被锁挡下
  });
  await waitFor(() => assert.equal(showCalls.length, 2));
  assert.equal(showCalls[1].from_message, 31);
  assert.equal(showCalls[1].limit, undefined, "整段拉取不能带 limit");
});

test("读到一半的会话按已加载条数重载,窗口不缩水", async () => {
  showImpl = async () => ({
    messages: [{}], turns: [], returned_message_count: 90,
    next_from_message: 91,
  });
  const props = {
    sessions: [session("r1")], ready: true,
    onSelect: () => {}, onFallbackLoad: () => {},
  };
  const view = mount(props);
  await act(async () => { view.state().select(sessionIdentity(session("r1"))); });
  await waitFor(() =>
    assert.equal(view.state().detail?.data?.returned_message_count, 90));

  await act(async () => {
    view.update({ ...props, sessions: [session("r2")] });
  });
  await waitFor(() => assert.equal(showCalls.length, 2));
  assert.equal(showCalls[1].limit, 90, "重载窗口 = 已加载条数");
});
