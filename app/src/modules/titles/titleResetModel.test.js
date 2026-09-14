import assert from "node:assert/strict";
import { test } from "vitest";

import {
  applicableTitleRows,
  buildTitleRows,
  dedupeNotes,
  sampleSessions,
  splitManualSessions,
  titleRowKey,
} from "./titleResetModel.js";

const evidence = {
  sessions: [
    { tool: "claude", ref: "fsr_a", title: "旧 A", title_source: "derived" },
    { tool: "claude", ref: "fsr_b", title: "手写 B", title_source: "manual" },
  ],
  errors: [
    { tool: "cursor", ref: "fsr_c", error: { code: "session.not_found" } },
  ],
};

test("批量默认把手动命名的会话挡在生成之外", () => {
  const off = splitManualSessions(evidence, false);
  assert.deepEqual(off.candidates.map(row => row.ref), ["fsr_a"]);
  assert.deepEqual(off.manual.map(row => row.ref), ["fsr_b"]);

  const on = splitManualSessions(evidence, true);
  assert.equal(on.candidates.length, 2);
  assert.equal(on.manual.length, 0);
});

test("预览行合并生成结果、跳过的手动命名与取证失败", () => {
  const rows = buildTitleRows({
    evidence,
    items: [{ tool: "claude", ref: "fsr_a", title: "✨ 实现新标题", type: "✨",
      skip: false, reason: null, before: "旧 A" }],
    includeManual: false,
    sessionByKey: new Map([[titleRowKey("claude", "fsr_a"), { tool: "claude", ref: "fsr_a" }]]),
  });

  assert.deepEqual(rows.map(row => [row.ref, row.skip, row.reason]), [
    ["fsr_a", false, null],
    ["fsr_b", true, "manual"],
    ["fsr_c", true, "error"],
  ]);
  assert.equal(rows[0].checked, true);
  assert.equal(rows[1].checked, false);
  assert.equal(rows[2].detail, "session.not_found");
});

test("只写回勾上的、没跳过的、标题确实变了的行", () => {
  const base = {
    key: "k", tool: "claude", ref: "fsr_a", session: {}, skip: false, checked: true,
    before: "旧", after: "新",
  };
  assert.equal(applicableTitleRows([base]).length, 1);
  assert.equal(applicableTitleRows([{ ...base, checked: false }]).length, 0);
  assert.equal(applicableTitleRows([{ ...base, skip: true }]).length, 0);
  assert.equal(applicableTitleRows([{ ...base, after: "  " }]).length, 0);
  assert.equal(applicableTitleRows([{ ...base, after: "旧" }]).length, 0);
  assert.equal(applicableTitleRows([{ ...base, session: null }]).length, 0);
});

test("备注去重,试生成只挑有 ref 的会话", () => {
  assert.deepEqual(dedupeNotes(["a", "a", "", null, "b"]), ["a", "b"]);
  const picked = sampleSessions(
    [{ tool: "claude", ref: "fsr_a" }, { tool: "claude" }, { ref: "fsr_c" }],
    5,
  );
  assert.deepEqual(picked.map(row => row.ref), ["fsr_a"]);
});
