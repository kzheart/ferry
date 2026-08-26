import assert from "node:assert/strict";
import { test } from "vitest";

import {
  isWindowsProjectPath,
  normalizeProjectPath,
  projectPathKey,
  repoOf,
  toTimeline,
} from "./sessionModel.js";

test("toTimeline defers unreached compactions while more pages remain", () => {
  const rounds = [{ n: 1 }, { n: 2 }];
  const compactions = [
    { id: "c1", after_turn: 1 },
    { id: "c2", after_turn: 5 },
    { id: "c3", after_turn: 9 },
  ];
  const timeline = toTimeline(rounds, compactions, true);
  assert.deepEqual(
    timeline.map(item => item.key),
    ["round:1", "compaction:c1", "round:2"],
  );
});

test("toTimeline appends out-of-range compactions once fully loaded", () => {
  const rounds = [{ n: 1 }, { n: 2 }];
  const compactions = [
    { id: "c1", after_turn: 2 },
    { id: "c2", after_turn: 9 },
  ];
  const timeline = toTimeline(rounds, compactions, false);
  assert.deepEqual(
    timeline.map(item => item.key),
    ["round:1", "round:2", "compaction:c1", "compaction:c2"],
  );
});

test("toTimeline merges same-turn compactions into one group item", () => {
  const rounds = [{ n: 1 }, { n: 2 }];
  const compactions = [
    { id: "c1", after_turn: 1 },
    { id: "c2", after_turn: 1 },
    { id: "c3", after_turn: 2 },
  ];
  const timeline = toTimeline(rounds, compactions, false);
  assert.deepEqual(
    timeline.map(item => item.key),
    ["round:1", "compaction:c1", "round:2", "compaction:c3"],
  );
  assert.deepEqual(
    timeline[1].compactions.map(item => item.id),
    ["c1", "c2"],
  );
});

test("repoOf 取路径最后一段，Windows 反斜杠与 Unix 斜杠一样", () => {
  assert.equal(repoOf("/work/payments"), "payments");
  assert.equal(repoOf("D:\\code\\ferry"), "ferry");
  assert.equal(repoOf("d:\\code\\ferry\\"), "ferry");
  assert.equal(repoOf("C:/Users/12467/Desktop/rweixin"), "rweixin");
  assert.equal(repoOf("ferry"), "ferry");
  assert.equal(repoOf(""), "");
});

test("项目路径身份跨 Agent 统一 Windows 斜杠、设备前缀、盘符大小写和尾斜杠", () => {
  assert.equal(normalizeProjectPath("c:/Users/me/work/app/"), "C:\\Users\\me\\work\\app");
  assert.equal(normalizeProjectPath("\\\\?\\C:\\Users\\me\\work\\app"), "C:\\Users\\me\\work\\app");
  assert.equal(
    projectPathKey("C:\\Users\\Me\\work\\app"),
    projectPathKey("c:/users/me/work/app/"),
  );
  assert.equal(normalizeProjectPath("/Users/me/work/app/"), "/Users/me/work/app");
  assert.notEqual(projectPathKey("/Users/Me/app"), projectPathKey("/Users/me/app"));
  assert.equal(isWindowsProjectPath("\\\\server\\share\\app"), true);
  assert.equal(isWindowsProjectPath("/Users/me/app"), false);
});
