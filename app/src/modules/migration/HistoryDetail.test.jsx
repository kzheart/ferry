// 迁移历史详情:影响与验收两张卡是迁移记录的主体,缺字段时也不能整页塌掉。
import { test } from "vitest";
import assert from "node:assert/strict";
import { render, screen } from "@testing-library/react";

import HistoryDetail from "./HistoryDetail.jsx";
import { histStatus, STATUS_CODE } from "./historyStatus.js";

test("状态判定按写入结果走", () => {
  assert.equal(histStatus({ session_id: "x" }), STATUS_CODE.success);
  assert.equal(histStatus({ rolled_back: true }), STATUS_CODE.rolledBack);
  assert.equal(histStatus({}), STATUS_CODE.failed);
});

test("迁移记录渲染出影响与验收", () => {
  render(<HistoryDetail h={{
    id: "history_2", time: 1787377949048, src: "claude", dst: "codex",
    title: "迁一个", session_id: "sid-2", msg_count: 12,
    loss: { exact: 10, degraded: 2, dropped: 0 },
  }} />);

  assert.ok(screen.getByText("迁一个"));
  assert.ok(screen.getByText(/migration:history.impactReport/));
  assert.ok(screen.getByText(/migration:history.verdict/));
});
