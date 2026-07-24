import { test, vi } from "vitest";
import assert from "node:assert/strict";
import { memo, useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";

import {
  SessionEditingProvider,
  useSessionEditingSurface,
} from "./sessionEditing.jsx";

const SURFACE = { ops: [], dirtyOps: [{ id: "op1" }], applying: false };

function Probe() {
  const { dirtyOps } = useSessionEditingSurface();
  return <span>{`待应用 ${dirtyOps.length}`}</span>;
}

test("Provider 内可以拿到编辑面", () => {
  render(
    <SessionEditingProvider value={SURFACE}>
      <Probe />
    </SessionEditingProvider>,
  );
  assert.ok(screen.getByText("待应用 1"));
});

test("缺 Provider 时在渲染期抛错", () => {
  const silenced = vi.spyOn(console, "error").mockImplementation(() => {});
  assert.throws(() => render(<Probe />), /SessionEditingProvider/);
  silenced.mockRestore();
});

test("value 引用不变时,memo 化的消费者不重渲染", () => {
  // 详情区是 memo 组件,靠这一点躲开侧边栏交互引起的整条时间线重绘。
  // 注入端一旦每次渲染都新建 value 对象,这层优化会静默失效。
  let renders = 0;
  const Counted = memo(function Counted() {
    useSessionEditingSurface();
    renders += 1;
    return null;
  });

  function Host() {
    const [tick, setTick] = useState(0);
    return (
      <SessionEditingProvider value={SURFACE}>
        <button onClick={() => setTick(tick + 1)}>无关状态变化</button>
        <Counted />
      </SessionEditingProvider>
    );
  }

  render(<Host />);
  assert.equal(renders, 1);
  fireEvent.click(screen.getByText("无关状态变化"));
  assert.equal(renders, 1);
});
