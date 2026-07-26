import { test } from "vitest";
import assert from "node:assert/strict";
import { entitiesFromToolResult, FERRY_ENTITY, navigationActionFor, rendererForEntity }
  from "./ferryEntities.js";

test("maps search details to clickable Session entities", () => {
  const result = { details: { sessions: [
    { tool: "codex", ref: "fsr_1", title: "Fix CI", project: "ferry" },
    { tool: "claude", ref: "fsr_2", title: "Design" },
  ] } };
  const entities = entitiesFromToolResult("session_search", result);
  assert.deepEqual(entities.map(entity => entity.type),
    [FERRY_ENTITY.session, FERRY_ENTITY.session]);
  assert.deepEqual(navigationActionFor(entities[0]),
    { view: "library", sessionId: undefined, ref: "fsr_1", tool: "codex",
      locator: undefined, turn: undefined });
});

test("maps migration, edit and usage details without stringifying them", () => {
  const migration = entitiesFromToolResult("migrate", { details: {
    plan_id: "op_m", kind: "migration", affected_refs: ["fsr_a"],
    preview: { source_tool: "claude", target_tool: "codex", loss: {} },
  } })[0];
  const edit = entitiesFromToolResult("session_edit", { details: {
    plan_id: "op_e", kind: "edit", affected_refs: ["fsr_b"],
    preview: { tool: "codex", changes: [{ locator: "fml_1" }] },
  } })[0];
  const usage = entitiesFromToolResult("usage", { details: {
    sessions: 2, tokens: { input: 10, output: 4 }, by_agent: { codex: { input: 10 } },
    filters: { time_range: { from: 1, to: 2 }, agents: ["codex"], projects: null },
  } })[0];

  assert.equal(migration.type, FERRY_ENTITY.migration);
  assert.equal(edit.type, FERRY_ENTITY.edit);
  assert.deepEqual(edit.locators, ["fml_1"]);
  assert.equal(usage.type, FERRY_ENTITY.usage);
  assert.equal(rendererForEntity(migration), "migration-preview");
  assert.equal(rendererForEntity(edit), "edit-diff");
  assert.equal(rendererForEntity(usage), "usage-slice");
  assert.deepEqual(navigationActionFor(edit), {
    view: "library", sessionId: undefined, ref: "fsr_b", tool: "codex",
    locator: "fml_1",
  });
  assert.deepEqual(navigationActionFor(migration), {
    view: "history", migrationId: "op_m", ref: "fsr_a",
  });
  assert.deepEqual(navigationActionFor(usage), {
    view: "overview", timeRange: { from: 1, to: 2 },
    agents: ["codex"], projects: null,
  });
});

test("supports explicit discriminated entities and keeps unknown results as text fallback", () => {
  const entities = entitiesFromToolResult("future_tool", { details: { entities: [
    { type: "Session", tool: "opencode", session_id: "ses_1", title: "Native" },
  ] } });
  assert.equal(entities[0].sessionId, "ses_1");
  assert.equal(rendererForEntity(entities[0]), "session-card");
  assert.deepEqual(entitiesFromToolResult("future_tool", { details: { value: 1 } }), []);
});

test("session_edit 的 rewrite args 规范化为候选 proposals,delete-turn 不混入", () => {
  const args = {
    tool: "codex", ref: "fsr_b", intent: "preview",
    ops: [
      { op: "rewrite", locator: "fml_1", text: "更清晰的提问 1" },
      { op: "delete-turn", turn: 2 },
      { op: "rewrite", locator: "fml_3", text: "更清晰的提问 3" },
    ],
  };
  const entity = entitiesFromToolResult("session_edit", { details: {
    kind: "edit", ref: "fsr_b",
    preview: { tool: "codex", changes: [{ locator: "fml_1" }, { locator: "fml_3" }] },
  } }, args)[0];

  assert.equal(entity.type, FERRY_ENTITY.edit);
  assert.equal(entity.intent, "preview");
  assert.deepEqual(entity.proposals, [
    { locator: "fml_1", text: "更清晰的提问 1" },
    { locator: "fml_3", text: "更清晰的提问 3" },
  ]);
  // 完整候选文本保留,locator 原样携带
  assert.equal(entity.proposals[1].locator, "fml_3");

  // 没有 args 的旧路径不受影响
  const bare = entitiesFromToolResult("session_edit", { details: {
    kind: "edit", plan_id: "op_e", preview: { tool: "codex", changes: [] },
  } })[0];
  assert.deepEqual(bare.proposals, []);
});

test("unwraps auto-applied operation envelopes", () => {
  const entity = entitiesFromToolResult("migrate", { details: {
    status: "applied",
    operation: {
      plan_id: "op_auto", kind: "migration", affected_refs: ["fsr_1"],
      preview: { source_tool: "claude", target_tool: "opencode" },
    },
    result: { saved_as: "/tmp/session.json" },
  } })[0];
  assert.equal(entity.id, "op_auto");
  assert.equal(entity.status, "applied");
  assert.equal(entity.savedAs, "/tmp/session.json");
});
