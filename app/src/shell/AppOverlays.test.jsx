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
    sessionDelete: { prepared: null },
    historyDelete: { history: null },
    batchDelete: { prepared: null },
    tags: { selection: null },
    toast: { value: null },
    railTip: { value: null },
    settings: { open: false },
    libraryFilter: { open: false },
    historyFilter: { open: false },
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

test("单会话删除确认按可撤销与否切换确认按钮语义", () => {
  const calls = [];
  const bag = {
    prepared: {
      session: { id: "s1", title: "会话", tool: "claude", tree_count: 1 },
      plan: { preview: { undoable: true } },
    },
    onCancel: () => calls.push("cancel"),
    onConfirm: () => calls.push("confirm"),
  };
  const { rerender } = renderOverlays({ sessionDelete: bag });
  fireEvent.click(screen.getByText("overlays:delete.confirmUndoable"));
  assert.deepEqual(calls, ["confirm"]);

  rerender(
    <AppOverlays
      {...closedOverlays()}
      sessionDelete={{
        ...bag,
        prepared: { ...bag.prepared, plan: { preview: { undoable: false } } },
      }}
    />,
  );
  assert.ok(screen.getByText("overlays:delete.confirmIrreversible"));
});

test("批量删除与单条删除是两个独立弹层", () => {
  const calls = [];
  renderOverlays({
    batchDelete: {
      prepared: [
        { session: { id: "a" }, plan: { preview: { undoable: true } } },
        { session: { id: "b" }, plan: { preview: { undoable: false } } },
      ],
      onCancel: () => calls.push("cancel"),
      onConfirm: () => calls.push("confirm"),
    },
  });

  assert.equal(screen.queryByText("overlays:delete.title"), null);
  fireEvent.click(screen.getByText("overlays:delete.confirmBatch"));
  assert.deepEqual(calls, ["confirm"]);
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

test("筛选弹层按各自的开关独立出现", () => {
  const { container, rerender } = renderOverlays({
    libraryFilter: {
      open: true,
      value: { src: [], time: "all", dir: null, mig: false, sub: false, tag: null },
      onChange: () => {},
      counts: {},
      dirs: [],
      tags: [],
      anchor: null,
      onClose: () => {},
      onClear: () => {},
    },
  });
  assert.ok(screen.getByText("overlays:filter.source"));
  assert.equal(screen.queryByText("overlays:filter.timeRange") !== null, true);

  rerender(<AppOverlays {...closedOverlays()} />);
  assert.equal(container.innerHTML, "");
});

test("导航轨提示按 railOnly 决定横向位置", () => {
  const { container } = renderOverlays({
    railTip: { value: { top: 120, label: "资料库" }, railOnly: true },
  });
  const tip = screen.getByText("资料库");
  assert.equal(tip.style.left, "86px");
  assert.equal(tip.style.top, "120px");
  assert.ok(container.innerHTML.includes("资料库"));
});

test("引导层只在 step 大于 0 时出现", () => {
  const { container, rerender } = renderOverlays({
    guide: { step: 0, onGo: () => {}, onFinish: () => {} },
  });
  assert.equal(container.innerHTML, "");

  rerender(
    <AppOverlays
      {...closedOverlays()}
      guide={{ step: 1, onGo: () => {}, onFinish: () => {} }}
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
  fireEvent.click(screen.getByText("×"));
  assert.equal(dismissed, true);
});
