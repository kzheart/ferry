import assert from "node:assert/strict";
import test from "node:test";

import {
  buildLibraryGroups,
  buildLibraryIndex,
  libraryFilterCount,
  libraryTokenDescriptors,
  visibleLibraryIds,
} from "./libraryResourcePaneModel.js";

const t = (key, params = {}) => `${key}:${JSON.stringify(params)}`;
const tools = ["claude", "codex", "opencode"];
const now = Date.now();
const sessions = [
  { tool: "claude", id: "a", title: "Payment", dir: "/work/payments", updated: now, tree_count: 2 },
  { tool: "codex", id: "b", title: "Search", dir: "/work/search", updated: now - 2 * 86400e3 },
];
const filter = { src: tools, time: "all", dir: null, mig: false, sub: false, tag: null };

test("会话库投影优先置顶，并按当前筛选分组", () => {
  const index = buildLibraryIndex({
    sessions,
    metadata: { "claude\u0000a": { pinned: true, tags: ["finance"] } },
    migratedSessionKeys: new Set(["codex\u0000b"]),
    t,
  });
  const groups = buildLibraryGroups({ index, filter, query: "pay", t });

  assert.deepEqual(groups.map(group => group.key), ["pinned"]);
  assert.deepEqual(groups[0].rows.map(row => row.key), ["claude\u0000a"]);
  assert.equal(index[1].mig, true);
});

test("折叠分组只影响可见导航顺序，不重建分组", () => {
  const index = buildLibraryIndex({ sessions, metadata: {}, migratedSessionKeys: new Set(), t });
  const groups = buildLibraryGroups({ index, filter, query: "", t });

  assert.deepEqual(groups.map(group => group.key), ["today", "last7"]);
  assert.deepEqual(visibleLibraryIds(groups, { today: true }), ["codex\u0000b"]);
  assert.deepEqual(groups[0].rows.map(row => row.key), ["claude\u0000a"]);
});

test("筛选 token 与数量来自同一份过滤状态", () => {
  const active = { ...filter, src: ["claude"], time: "last7", dir: "payments", mig: true, tag: "finance" };
  const tokens = libraryTokenDescriptors(active, tools, { claude: "Claude" }, t);

  assert.equal(libraryFilterCount(active, tools), 5);
  assert.deepEqual(tokens.map(token => token.kind), ["source", "time", "dir", "mig", "tag"]);
});
