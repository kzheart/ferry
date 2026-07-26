import assert from "node:assert/strict";
import { test } from "vitest";
import { groupAgentTimeline, isAwaitingReply, mergeReadTools }
  from "./agentTimelineModel.js";

const tool = (name, callId, status = "complete") => ({
  kind: "tool",
  name,
  callId,
  status,
  startedAt: `${callId}-start`,
  endedAt: `${callId}-end`,
});

test("连续完成的同类只读工具合并为一项", () => {
  const rows = mergeReadTools([
    tool("session_read", "read-1"),
    tool("session_read", "read-2"),
    tool("usage", "usage-1"),
  ]);

  assert.equal(rows.length, 2);
  assert.deepEqual(rows[0].merged.map(item => item.callId), ["read-1", "read-2"]);
  assert.equal(rows[0].endedAt, "read-2-end");
  assert.equal(rows[1].callId, "usage-1");
  assert.equal(rows[1].merged, undefined);
});

test("写工具和运行中的只读工具保持独立", () => {
  const rows = mergeReadTools([
    tool("session_edit", "edit-1"),
    tool("session_edit", "edit-2"),
    tool("session_search", "search-1", "running"),
    tool("session_search", "search-2"),
  ]);

  assert.deepEqual(rows.map(item => item.callId),
    ["edit-1", "edit-2", "search-1", "search-2"]);
});

test("时间线只聚合连续工具，不跨消息合并", () => {
  const items = [
    { kind: "user", text: "开始" },
    tool("session_read", "read-1"),
    tool("session_read", "read-2"),
    { kind: "assistant", text: "完成" },
    tool("session_read", "read-3"),
  ];

  const grouped = groupAgentTimeline(items);

  assert.deepEqual(grouped.map(item => item.kind),
    ["user", "trace", "assistant", "trace"]);
  assert.equal(grouped[1].rows[0].merged.length, 2);
  assert.equal(grouped[3].rows[0].callId, "read-3");
});

test("运行中且末尾无活动指示时判定为等待回复", () => {
  assert.equal(isAwaitingReply("running", [{ kind: "user", text: "hi" }]), true);
  assert.equal(isAwaitingReply("running", [tool("session_search", "s-1")]), true);
  assert.equal(isAwaitingReply("running",
    [{ kind: "assistant", text: "done" }]), true);
  assert.equal(isAwaitingReply("running", []), true);
});

test("末尾自带活动指示或运行结束时不判定为等待", () => {
  assert.equal(isAwaitingReply("running",
    [tool("session_search", "s-1", "running")]), false);
  assert.equal(isAwaitingReply("running",
    [{ kind: "assistant", text: "…", streaming: true }]), false);
  assert.equal(isAwaitingReply("running",
    [{ kind: "approval", id: "a-1" }]), false);
  assert.equal(isAwaitingReply("complete", [{ kind: "user", text: "hi" }]), false);
});
