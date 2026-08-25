// AppOverlays 是纯渲染骨架:每个弹层什么时候出现、拿到哪些回调,全在这一层定型。
// 这些用例守的就是接线本身——某个 prop 忘了往下传时,类型检查看不出来,这里会红。
import { test } from "vitest";
import assert from "node:assert/strict";
import { fireEvent, render, screen } from "@testing-library/react";

import { AppOverlays } from "./AppOverlays.jsx";

// 全部关闭的基线。单个用例只覆盖自己要打开的那一个 bag,其余保持关闭,
// 于是"打开 A 却渲染出 B"这类串线会被立刻发现。
function closedOverlays() {
  return {
    t: key => key,
    floatChat: { mounted: false },
    peek: { open: false },
    migration: { open: false },
    editing: { diff: null, dirtyOps: [], confirmApply: false },
    search: { open: false, pane: null, results: [] },
    contextMenu: { open: false, items: null },
    tags: { selection: null },
    toast: { value: null },
    settings: { open: false },
    guide: { step: 0 },
  };
}

function renderOverlays(overrides) {
  return render(<AppOverlays {...closedOverlays()} {...overrides} />);
}

test("全部关闭时不渲染任何弹层", () => {
  const { container } = renderOverlays({});
  assert.equal(container.innerHTML, "");
});

test("标签弹层按批量与否取不同标题,并回传原始输入", () => {
  const confirmed = [];
  const tags = {
    selection: { batch: true, sessions: [{ id: "a" }, { id: "b" }] },
    initial: "x, y",
    onCancel: () => {},
    onConfirm: value => confirmed.push(value),
  };
  const { rerender } = renderOverlays({ tags });
  assert.ok(screen.getByText("app:prompt.tagsBatchTitle"));

  fireEvent.click(screen.getByText("app:prompt.save"));
  assert.deepEqual(confirmed, ["x, y"]);

  rerender(
    <AppOverlays
      {...closedOverlays()}
      tags={{ ...tags, selection: { batch: false, sessions: [{ id: "a" }] } }}
    />,
  );
  assert.ok(screen.getByText("app:prompt.tagsTitle"));
});

test("右键菜单渲染条目,点击后先关闭再执行动作", () => {
  const calls = [];
  renderOverlays({
    contextMenu: {
      open: true,
      x: 10,
      y: 20,
      items: [
        { label: "打开", onClick: () => calls.push("open") },
        { sep: true },
        { label: "删除", danger: true, onClick: () => calls.push("delete") },
      ],
      onClose: () => calls.push("close"),
    },
  });

  fireEvent.click(screen.getByText("删除"));
  assert.deepEqual(calls, ["close", "delete"]);
});

test("右键菜单的禁用项不触发动作", () => {
  const calls = [];
  renderOverlays({
    contextMenu: {
      open: true,
      x: 0,
      y: 0,
      items: [{ label: "迁移", disabled: true, onClick: () => calls.push("go") }],
      onClose: () => calls.push("close"),
    },
  });

  fireEvent.click(screen.getByText("迁移"));
  assert.deepEqual(calls, []);
});

test("搜索面板把结果条目的点击接到各自的 onClick", () => {
  const opened = [];
  renderOverlays({
    search: {
      open: true,
      pane: { placeholder: "搜索", query: "", onQuery: () => {} },
      results: [
        { id: "r1", title: "第一条", tool: "claude", meta: "repo", onClick: () => opened.push("r1") },
        { id: "r2", title: "第二条", tool: "codex", meta: "repo", onClick: () => opened.push("r2") },
      ],
      onClose: () => {},
    },
  });

  fireEvent.click(screen.getByText("第二条"));
  assert.deepEqual(opened, ["r2"]);
});

test("搜索面板缺少 pane 配置时不渲染", () => {
  const { container } = renderOverlays({
    search: { open: true, pane: null, results: [], onClose: () => {} },
  });
  assert.equal(container.innerHTML, "");
});

test("引导层只在 step 大于 0 时出现", () => {
  const steps = [{ target: "rail", view: "library", titleKey: "t", bodyKey: "b" }];
  const { container, rerender } = renderOverlays({
    guide: { step: 0, steps, onGo: () => {}, onFinish: () => {} },
  });
  assert.equal(container.innerHTML, "");

  rerender(
    <AppOverlays
      {...closedOverlays()}
      guide={{ step: 1, steps, onGo: () => {}, onFinish: () => {} }}
    />,
  );
  assert.notEqual(container.innerHTML, "");
});

test("提示条的关闭按钮接到 onDismiss", () => {
  let dismissed = false;
  renderOverlays({
    toast: {
      value: { kind: "ok", title: "已保存", desc: "元数据已写入" },
      onDismiss: () => { dismissed = true; },
    },
  });

  assert.ok(screen.getByText("已保存"));
  // 关闭键是图标按钮,只能按无障碍名称找——正好是它该有的定位方式
  fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  assert.equal(dismissed, true);
});
