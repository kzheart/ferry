// 隐式取值的兜底:类型检查看不见 Context 的连接,所以缺 Provider 必须在挂载
// 那一刻就炸,而不是把 undefined 带到某次点击里去。
import { test, vi } from "vitest";
import assert from "node:assert/strict";
import { render, screen } from "@testing-library/react";

import { FerryRuntimeProvider, useFerryRuntime } from "./ferryRuntime.jsx";

function Probe() {
  const ferry = useFerryRuntime();
  return <span>{ferry.health.model}</span>;
}

test("Provider 内可以拿到注入的句柄", () => {
  render(
    <FerryRuntimeProvider value={{ health: { model: "opus" } }}>
      <Probe />
    </FerryRuntimeProvider>,
  );
  assert.ok(screen.getByText("opus"));
});

test("缺 Provider 时在渲染期抛错", () => {
  // React 会把渲染期异常打到 console.error,这里静音以免污染测试输出。
  const silenced = vi.spyOn(console, "error").mockImplementation(() => {});
  assert.throws(() => render(<Probe />), /FerryRuntimeProvider/);
  silenced.mockRestore();
});
