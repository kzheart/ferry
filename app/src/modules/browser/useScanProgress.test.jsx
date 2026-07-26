import { act, render } from "@testing-library/react";
import { expect, test, vi } from "vitest";

let progress = { state: "running", phase: "reading", processed: 7, total: 20 };

vi.mock("../../platform/desktop/client.js", async (importOriginal) => ({
  ...(await importOriginal()),
  engine: async (method) => (method === "scan_progress" ? progress : {}),
}));

const { useScanProgress } = await import("./useBrowserData.js");

function mount(scanning) {
  let value;
  function Probe({ scanning: on }) {
    value = useScanProgress(on);
    return null;
  }
  const view = render(<Probe scanning={scanning} />);
  return {
    get: () => value,
    rerender: (on) => view.rerender(<Probe scanning={on} />),
    unmount: () => view.unmount(),
  };
}

// 进度状态从根组件搬到这里之后,设置页的进度条仍然要照常推进。
test("扫描期间轮询进度，扫描结束后停掉轮询并清空", async () => {
  vi.useFakeTimers();
  try {
    const harness = mount(true);
    expect(harness.get()).toBe(null);

    await act(async () => vi.advanceTimersByTime(360));
    expect(harness.get()).toEqual(progress);

    progress = { state: "running", phase: "finalizing", processed: 20, total: 20 };
    await act(async () => vi.advanceTimersByTime(360));
    expect(harness.get().phase).toBe("finalizing");

    await act(async () => harness.rerender(false));
    expect(harness.get()).toBe(null);
    // 停止后即使时间继续走,也不该再有进度进来
    progress = { state: "running", phase: "reading", processed: 1, total: 9 };
    await act(async () => vi.advanceTimersByTime(1000));
    expect(harness.get()).toBe(null);

    harness.unmount();
  } finally {
    vi.useRealTimers();
  }
});

test("不在扫描时不轮询", async () => {
  vi.useFakeTimers();
  try {
    const harness = mount(false);
    await act(async () => vi.advanceTimersByTime(2000));
    expect(harness.get()).toBe(null);
    harness.unmount();
  } finally {
    vi.useRealTimers();
  }
});
