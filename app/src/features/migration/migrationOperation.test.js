import assert from "node:assert/strict";
import test from "node:test";

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
  probe: true,
  probeModel: "provider/model",
};

test("builds the current migration operation input", () => {
  assert.deepEqual(migrationPlanInput(base), {
    kind: "migration",
    source_tool: "claude",
    ref: "fsr_current",
    target_tool: "codex",
    max_turn: 4,
    probe: true,
    probe_model: "provider/model",
  });
  assert.deepEqual(migrationPlanInput({
    ...base,
    maxTurn: undefined,
    probe: false,
  }), {
    kind: "migration",
    source_tool: "claude",
    ref: "fsr_current",
    target_tool: "codex",
    probe: false,
  });
});

test("target, scope, probe and model changes invalidate a cached plan", () => {
  const input = migrationPlanInput(base);
  const planned = { key: migrationPlanKey(input), plan: { plan_id: "op_1" } };

  assert.equal(matchingMigrationPlan(planned, input), planned.plan);
  for (const changed of [
    { targetTool: "opencode" },
    { maxTurn: 5 },
    { probe: false },
    { probeModel: "provider/other" },
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
