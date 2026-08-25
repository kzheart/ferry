import { act, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";

const indexState = vi.hoisted(() => ({
  contentIndex: null,
  options: [],
}));

vi.mock("../modules/onboarding/public.js", () => ({
  BoatGlyph: () => <span aria-hidden>boat</span>,
  useIndexProgress: (options) => {
    indexState.options.push(options);
    return { contentIndex: indexState.contentIndex };
  },
}));

import { IndexProgressPill } from "./IndexProgressPill.jsx";
import { WorkspaceToolbar } from "./WorkspaceToolbar.jsx";

const busy = () => ({
  ready: false,
  indexed_sessions: 40,
  pending_sessions: 60,
  building: true,
});

const ready = () => ({
  ready: true,
  indexed_sessions: 100,
  pending_sessions: 0,
  building: false,
});

beforeEach(() => {
  indexState.contentIndex = null;
  indexState.options = [];
});

test("首次索引完成后显示 3 秒，轮询新对象不会续期，之后的索引保持静默", () => {
  vi.useFakeTimers();
  try {
    indexState.contentIndex = busy();
    const view = render(<IndexProgressPill active />);
    expect(screen.getByText(/app:indexPill\.busy/)).toBeTruthy();

    act(() => {
      indexState.contentIndex = ready();
      view.rerender(<IndexProgressPill active />);
    });
    expect(screen.getByText("app:indexPill.done")).toBeTruthy();
    expect(indexState.options.at(-1).active).toBe(false);

    act(() => vi.advanceTimersByTime(1500));
    act(() => {
      // scan_progress 每轮都会返回新对象；这不应重置完成态的隐藏计时器。
      indexState.contentIndex = ready();
      view.rerender(<IndexProgressPill active />);
    });
    act(() => vi.advanceTimersByTime(1500));
    expect(screen.queryByText("app:indexPill.done")).toBeNull();

    act(() => {
      indexState.contentIndex = busy();
      view.rerender(<IndexProgressPill active />);
    });
    expect(screen.queryByText(/app:indexPill\.busy/)).toBeNull();
  } finally {
    vi.useRealTimers();
  }
});

test("非首次启动不展示也不开启进度轮询", () => {
  indexState.contentIndex = busy();
  render(<IndexProgressPill active={false} />);

  expect(screen.queryByText(/app:indexPill\.busy/)).toBeNull();
  expect(indexState.options.at(-1).active).toBe(false);
});

test("标题栏仅在首启会话挂载索引提示", () => {
  indexState.contentIndex = busy();
  const view = render(<WorkspaceToolbar collapsed={false} onToggle={() => {}} />);
  expect(screen.queryByText(/app:indexPill\.busy/)).toBeNull();
  expect(indexState.options).toHaveLength(0);

  view.rerender(
    <WorkspaceToolbar collapsed={false} onToggle={() => {}} showIndexProgress />,
  );
  expect(screen.getByText(/app:indexPill\.busy/)).toBeTruthy();
  expect(indexState.options.at(-1).active).toBe(true);
});

test("在首启向导里已经索引完成时直接退场", () => {
  indexState.contentIndex = ready();
  const view = render(<IndexProgressPill active />);
  expect(screen.queryByText("app:indexPill.done")).toBeNull();

  act(() => {
    indexState.contentIndex = busy();
    view.rerender(<IndexProgressPill active />);
  });
  expect(screen.queryByText(/app:indexPill\.busy/)).toBeNull();
});
