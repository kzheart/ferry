import { act, renderHook } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { FerryRuntimeProvider } from "../../shared/capabilities/ferryRuntime.jsx";
import { useAgentComposer } from "./useAgentComposer.js";

function mount() {
  const ferry = { activeId: "session-1", send: vi.fn(async () => "session-1"), reportError: vi.fn() };
  const view = renderHook(({ logItems }) => useAgentComposer({
    attachments: [], setAttachments: vi.fn(), logItems,
  }), {
    initialProps: { logItems: [] },
    wrapper: ({ children }) => <FerryRuntimeProvider value={ferry}>{children}</FerryRuntimeProvider>,
  });
  return { ...view, ferry };
}

test.each([
  { isComposing: true },
  { nativeEvent: { isComposing: true } },
  { keyCode: 229 },
  { nativeEvent: { keyCode: 229 } },
])("输入法确认候选词时不会发送消息: %j", async flags => {
  const { result, ferry } = mount();
  act(() => result.current.setText("正在输入中文"));
  const event = { key: "Enter", preventDefault: vi.fn(), ...flags };
  await act(async () => result.current.composerProps.onKeyDown(event));
  expect(ferry.send).not.toHaveBeenCalled();
  expect(event.preventDefault).not.toHaveBeenCalled();
  expect(result.current.text).toBe("正在输入中文");
});

test("普通回车发送,Shift+Enter保留换行", async () => {
  const { result, ferry } = mount();
  act(() => result.current.setText("消息"));
  await act(async () => result.current.composerProps.onKeyDown({ key: "Enter", shiftKey: true }));
  expect(ferry.send).not.toHaveBeenCalled();
  const event = { key: "Enter", preventDefault: vi.fn() };
  await act(async () => result.current.composerProps.onKeyDown(event));
  expect(event.preventDefault).toHaveBeenCalled();
  expect(ferry.send).toHaveBeenCalledTimes(1);
});

test("上翻历史不被新消息打断,回到底部后恢复跟随", () => {
  const { result, rerender } = mount();
  const viewport = { scrollTop: 200, scrollHeight: 1000, clientHeight: 300 };
  result.current.scrollRef.current = viewport;
  act(() => result.current.onScroll());
  expect(result.current.showScrollToBottom).toBe(true);
  viewport.scrollHeight = 1200;
  rerender({ logItems: [{ text: "新内容" }] });
  expect(viewport.scrollTop).toBe(200);
  act(() => result.current.scrollToBottom());
  expect(viewport.scrollTop).toBe(1200);
  expect(result.current.showScrollToBottom).toBe(false);
  viewport.scrollHeight = 1500;
  rerender({ logItems: [{ text: "继续输出" }] });
  expect(viewport.scrollTop).toBe(1500);
});

test("切换会话重置历史滚动状态并显示新会话底部", () => {
  const { result, rerender, ferry } = mount();
  const viewport = { scrollTop: 100, scrollHeight: 1000, clientHeight: 300 };
  result.current.scrollRef.current = viewport;
  act(() => result.current.onScroll());
  expect(result.current.showScrollToBottom).toBe(true);
  ferry.activeId = "session-2";
  rerender({ logItems: [] });
  expect(result.current.showScrollToBottom).toBe(false);
  expect(viewport.scrollTop).toBe(1000);
});

test("用户手动滚到底部时隐藏返回按钮", () => {
  const { result } = mount();
  const viewport = { scrollTop: 100, scrollHeight: 1000, clientHeight: 300 };
  result.current.scrollRef.current = viewport;
  act(() => result.current.onScroll());
  viewport.scrollTop = 690;
  act(() => result.current.onScroll());
  expect(result.current.showScrollToBottom).toBe(false);
});
