import { expect, test, vi } from "vitest";
import { act, render } from "@testing-library/react";

const applied = { operation: [], shell: [] };

// bash 提案与 Engine 提案共用同一张审批卡,分流只看 plan_id 前缀——这条走错就会
// 把 shell 命令送进 Engine 的 operation 状态机。
vi.mock("../../platform/desktop/client.js", async (importOriginal) => ({
  ...(await importOriginal()),
  onRuntimeEvent: async () => () => {},
  runtime: async () => ({}),
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
