import test from "node:test";
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
    operation_id: "op_m", kind: "migration", affected_refs: ["fsr_a"],
    preview: { source_tool: "claude", target_tool: "codex", loss: {} },
  } })[0];
  const edit = entitiesFromToolResult("session_edit", { details: {
    operation_id: "op_e", kind: "edit", affected_refs: ["fsr_b"],
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

test("unwraps auto-applied operation envelopes", () => {
  const entity = entitiesFromToolResult("migrate", { details: {
    status: "applied",
    operation: {
      operation_id: "op_auto", kind: "migration", affected_refs: ["fsr_1"],
      preview: { source_tool: "claude", target_tool: "opencode" },
    },
    result: { saved_as: "/tmp/session.json" },
  } })[0];
  assert.equal(entity.id, "op_auto");
  assert.equal(entity.status, "applied");
  assert.equal(entity.savedAs, "/tmp/session.json");
});
