import assert from "node:assert/strict";
import test from "node:test";

import {
  buildHistoryGroups,
  buildHistoryItems,
  filterHistoryItems,
  historyFilterCount,
  historyTokenDescriptors,
} from "./historyResourcePaneModel.js";

const t = (key, params = {}) => `${key}:${JSON.stringify(params)}`;
const tools = ["claude", "codex", "opencode"];
const toolNames = { claude: "Claude", codex: "Codex", opencode: "OpenCode" };
const now = Date.now();

test("迁移历史按筛选条件和时间段投影", () => {
  const items = buildHistoryItems([
    { id: 1, src: "claude", dst: "codex", source_id: "a", title: "支付", time: now, session_id: "new-a" },
    { id: 2, src: "codex", dst: "claude", source_id: "b", title: "搜索", time: now - 2 * 86400e3, rolled_back: true },
  ]);
  const filtered = filterHistoryItems({
    items,
    filter: { src: tools, target: "codex", status: "all", time: "all" },
    query: "支付",
  });
  const groups = buildHistoryGroups({ items: filtered, selectedId: null, t, toolNames });

  assert.deepEqual(filtered.map(item => item._id), ["h1"]);
  assert.equal(groups[0].rows[0].selected, true);
  assert.equal(groups[0].rows[0].from, "Claude");
});

test("迁移历史 token 与数量使用同一筛选状态", () => {
  const filter = { src: ["claude"], target: "codex", status: "status.failed", time: "earlier" };
  const tokens = historyTokenDescriptors(filter, toolNames, t);

  assert.equal(historyFilterCount(filter, tools), 4);
  assert.deepEqual(tokens.map(token => token.kind), ["target", "status", "time"]);
});
