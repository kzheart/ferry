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
