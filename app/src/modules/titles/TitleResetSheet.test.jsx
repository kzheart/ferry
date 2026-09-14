// 预览面板的三种形态:生成中、可编辑的结果表、模型缺失的错误态。
import assert from "node:assert/strict";
import { test, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";

let engineResult = null;
let runtimeResult = null;
let runtimeError = null;

vi.mock("../../platform/desktop/client.js", async (importOriginal) => ({
  ...(await importOriginal()),
  engine: async () => engineResult,
  runtime: async () => {
    if (runtimeError) throw runtimeError;
    return runtimeResult;
  },
}));

const { default: TitleResetSheet } = await import("./TitleResetSheet.jsx");

const SESSIONS = [
  { tool: "claude", ref: "fsr_a", id: "a", title: "旧 A" },
  { tool: "claude", ref: "fsr_b", id: "b", title: "手写 B" },
];

function prepare() {
  runtimeError = null;
  engineResult = {
    sessions: [
      { tool: "claude", ref: "fsr_a", title: "旧 A", title_source: "derived" },
      { tool: "claude", ref: "fsr_b", title: "手写 B", title_source: "manual" },
    ],
    errors: [],
    style: {},
  };
  runtimeResult = {
    items: [{ tool: "claude", ref: "fsr_a", title: "✨ 实现甲", type: "✨", skip: false,
      before: "旧 A" }],
    model: { provider_id: "p", model: "gpt-x" },
  };
}

const mount = (props = {}) => render(
  <TitleResetSheet sessions={SESSIONS} batch renameSession={async () => ({})}
    updateMetadata={async () => {}} rescan={() => {}} setToast={() => {}}
    onClose={() => {}} {...props} />,
);

test("生成完成后列出可编辑的新标题,手动命名的标灰跳过", async () => {
  prepare();
  mount();
  assert.ok(screen.getByText("overlays:titleReset.generating"));

  await act(async () => {});
  assert.ok(screen.getByText("overlays:titleReset.model"));
  const inputs = screen.getAllByLabelText("overlays:titleReset.after");
  assert.equal(inputs.length, 1);
  assert.equal(inputs[0].value, "✨ 实现甲");
  assert.ok(screen.getByText(/overlays:titleReset.reason.manual/));
  // 手动命名那条不计入可应用数
  assert.ok(screen.getByText("overlays:titleReset.apply"));
});

test("应用按钮把编辑过的标题交给写回", async () => {
  prepare();
  const renamed = [];
  mount({ renameSession: async (session, title) => { renamed.push(title); return {}; } });
  await act(async () => {});

  const input = screen.getAllByLabelText("overlays:titleReset.after")[0];
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype, "value").set;
    setter.call(input, "✨ 改过的标题");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const apply = [...document.querySelectorAll("button")]
    .find(button => button.textContent.includes("overlays:titleReset.apply"));
  await act(async () => { apply.click(); });

  assert.deepEqual(renamed, ["✨ 改过的标题"]);
});

test("没有可用模型时指向设置里的模型分区", async () => {
  prepare();
  runtimeError = Object.assign(new Error("no provider"), { code: "provider_unavailable" });
  let opened = 0;
  mount({ onOpenModels: () => { opened += 1; } });
  await act(async () => {});

  assert.ok(screen.getByText("overlays:titleReset.providerUnavailable"));
  const button = [...document.querySelectorAll("button")]
    .find(node => node.textContent.includes("overlays:titleReset.openModels"));
  await act(async () => { button.click(); });
  assert.equal(opened, 1);
});
