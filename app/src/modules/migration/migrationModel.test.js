import { test } from "vitest";
import assert from "node:assert/strict";
import { migrationOriginFor } from "./migrationModel.js";

const rows = [
  { id: 1, src: "opencode", dst: "claude", source_id: "a", session_id: "s1",
    time: "2026-08-01T00:00:00Z" },
  { id: 2, src: "codex", dst: "claude", source_id: "b", session_id: "s1",
    time: "2026-08-20T00:00:00Z" },
  { id: 3, src: "claude", dst: "codex", source_id: "s1", session_id: "s9",
    time: "2026-08-21T00:00:00Z" },
];

test("按目标会话匹配迁入记录,多次命中取最近一条", () => {
  const origin = migrationOriginFor(rows, { tool: "claude", id: "s1" });
  assert.equal(origin.id, 2);
  assert.equal(origin.src, "codex");
});

test("不是迁移产物、回滚记录或无效身份都不构成出处", () => {
  assert.equal(migrationOriginFor(rows, { tool: "opencode", id: "a" }), null);
  assert.equal(migrationOriginFor(rows, { tool: "claude", id: "s2" }), null);
  assert.equal(migrationOriginFor(rows, {}), null);
  assert.equal(
    migrationOriginFor(
      [{ src: "codex", dst: "claude", session_id: "s1", rolled_back: true,
        time: "2026-08-22T00:00:00Z" }],
      { tool: "claude", id: "s1" },
    ),
    null,
  );
});
