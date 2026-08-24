import assert from "node:assert/strict";
import { test } from "vitest";

import { createSessionContextMenu } from "./sessionContextMenu.js";

function createInput(overrides = {}) {
  const session = {
    id: "native-1",
    ref: "fsr_current",
    tool: "claude",
    title: "Session",
    path: "/tmp/session.jsonl",
  };
  return {
    menu: { key: "claude:native-1" },
    sessionsByKey: { "claude:native-1": session },
    selectedId: "claude:native-1",
    multiIds: [],
    metaFor: () => ({}),
    updateMetadata: () => {},
    setTagSelection: () => {},
    setRename: () => {},
    setBatchDelete: () => {},
    setMultiIds: () => {},
    setAgentAttachments: () => {},
    setView: () => {},
    setMenu: () => {},
    setToast: () => {},
    select: () => {},
    setMigration: () => {},
    settings: { terminalApp: "Terminal" },
    isFeatureEnabled: () => true,
    t: (key, params) => params?.n ? `${key}:${params.n}` : key,
    askDelete: () => {},
    ...overrides,
  };
}

test("会话菜单把重命名动作交给 browser 能力调用方", () => {
  let renamed = null;
  const items = createSessionContextMenu(createInput({
    setRename: session => {
      renamed = session;
    },
  }));

  items.find(item => item.label === "app:ctx.rename").onClick();

  assert.equal(renamed.id, "native-1");
});

test("多选菜单只暴露批量标签、删除和取消动作", () => {
  const input = createInput({
    menu: { key: "claude:native-1", multi: true },
    multiIds: ["claude:native-1"],
  });
  const items = createSessionContextMenu(input);

  assert.deepEqual(
    items.filter(item => !item.sep).map(item => item.label),
    ["app:ctx.addTags", "app:ctx.deleteN:1", "app:ctx.cancelMulti"],
  );
});

test("缺少能力的会话不显示恢复、迁移和删除动作", () => {
  const input = createInput({
    menu: { key: "readonly:native-1" },
    sessionsByKey: {
      "readonly:native-1": {
        id: "native-1",
        ref: "fsr_current",
        tool: "readonly",
        title: "Read only",
      },
    },
  });
  const labels = createSessionContextMenu(input)
    .filter(item => !item.sep)
    .map(item => item.label);
  assert.equal(labels.includes("app:ctx.resumeTerminal"), false);
  assert.equal(labels.includes("app:ctx.copyResume"), false);
  assert.equal(labels.includes("app:ctx.migrateTo"), false);
  assert.equal(labels.includes("app:ctx.deleteSession"), false);
});

test("标了 feature 的菜单项跟着开关走,没标的照旧", () => {
  const labelsWith = (isFeatureEnabled) =>
    createSessionContextMenu(createInput({ isFeatureEnabled }))
      .filter(item => !item.sep)
      .map(item => item.label);

  assert.equal(labelsWith(() => true).includes("app:ctx.addToAgent"), true);
  const off = labelsWith(() => false);
  assert.equal(off.includes("app:ctx.addToAgent"), false);
  // 只掉标了 feature 的那一项,其余动作一个不少
  assert.equal(off.includes("app:ctx.rename"), true);
  assert.equal(off.includes("app:ctx.resumeTerminal"), true);
});

test("续聊只有一条入口,点击把会话交给回调", () => {
  let picked = null;
  const items = createSessionContextMenu(createInput({
    onResumeElsewhere: session => { picked = session; },
  }));
  const labels = items.filter(item => !item.sep).map(item => item.label);
  assert.equal(labels.filter(label => label === "app:ctx.copyResumeElsewhere").length, 1);

  items.find(item => item.label === "app:ctx.copyResumeElsewhere").onClick();
  assert.ok(picked);
  assert.equal(picked.tool, "claude");
});

test("有项目目录时可在 Finder 中显示", () => {
  const items = createSessionContextMenu(createInput({
    sessionsByKey: {
      "claude:native-1": {
        id: "native-1",
        ref: "fsr_current",
        tool: "claude",
        title: "Session",
        path: "/tmp/session.jsonl",
        dir: "/Users/kzheart/code/ferry",
      },
    },
  }));
  const reveal = items.find(item => item.label === "app:ctx.revealInFinder");
  assert.equal(reveal.disabled, false);
});

test("没有项目目录时禁用在 Finder 中显示", () => {
  const items = createSessionContextMenu(createInput());
  const reveal = items.find(item => item.label === "app:ctx.revealInFinder");
  assert.equal(reveal.disabled, true);
  assert.equal(reveal.disabledHint, "app:ctx.noProjectDir");
});
