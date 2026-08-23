import assert from "node:assert/strict";
import { test } from "vitest";

import {
  matchingMigrationPlan,
  migrationPlanInput,
  migrationPlanKey,
} from "./migrationOperation.js";
const base = {
  sourceTool: "claude",
  ref: "fsr_current",
  targetTool: "codex",
  maxTurn: 4,
};

test("builds the current migration operation input", () => {
  assert.deepEqual(migrationPlanInput(base), {
    kind: "migration",
    source_tool: "claude",
    ref: "fsr_current",
    target_tool: "codex",
    max_turn: 4,
  });
  assert.deepEqual(migrationPlanInput({
    ...base,
    maxTurn: undefined,
  }), {
    kind: "migration",
    source_tool: "claude",
    ref: "fsr_current",
    target_tool: "codex",
  });
});

test("target and scope changes invalidate a cached plan", () => {
  const input = migrationPlanInput(base);
  const planned = { key: migrationPlanKey(input), plan: { plan_id: "op_1" } };

  assert.equal(matchingMigrationPlan(planned, input), planned.plan);
  for (const changed of [
    { targetTool: "opencode" },
    { maxTurn: 5 },
  ]) {
    assert.equal(
      matchingMigrationPlan(
        planned,
        migrationPlanInput({ ...base, ...changed }),
      ),
      null,
    );
  }
});

