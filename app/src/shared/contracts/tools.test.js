import assert from "node:assert/strict";
import { test } from "vitest";

import {
  supportsEditOperation,
} from "./agentEditSupport.js";
import {
  agentsWithCapability,
  supportsAgentCapability,
} from "./tools.js";
import { AGENTS, AGENT_IDS } from "./generated/agents.js";

test("当前静态 Agent 契约定义逐操作编辑范围", () => {
  assert.deepEqual(AGENT_IDS, Object.keys(AGENTS));
  for (const [tool, agent] of Object.entries(AGENTS)) {
    for (const operation of agent.editOperations) {
      assert.equal(supportsEditOperation(tool, operation), true);
    }
  }
  assert.equal(supportsEditOperation("unknown", "rewrite"), false);
  assert.equal(supportsEditOperation("__proto__", "rewrite"), false);
  assert.equal(supportsEditOperation(AGENT_IDS[0], "unknown"), false);

  const rewriteOnly = Object.entries(AGENTS).find(([, agent]) =>
    agent.editOperations.length === 1
    && agent.editOperations[0] === "rewrite");
  assert.ok(rewriteOnly);
  assert.equal(supportsEditOperation(rewriteOnly[0], "delete-turn"), false);
  assert.equal(
    supportsEditOperation(rewriteOnly[0], "replace-assistant-reply"),
    false,
  );

  const fullEditor = Object.entries(AGENTS).find(([, agent]) =>
    ["delete-turn", "rewrite", "replace-assistant-reply"]
      .every(operation => agent.editOperations.includes(operation)));
  assert.ok(fullEditor);
});

test("能力 helper 按静态矩阵筛选 Agent", () => {
  for (const capability of [
    "browse",
    "resume",
    "migration-source",
    "migration-target",
    "edit",
    "delete",
    "probe",
    "models",
  ]) {
    assert.deepEqual(
      agentsWithCapability(capability),
      AGENT_IDS.filter(tool =>
        AGENTS[tool].capabilities.includes(capability)),
    );
  }
  assert.equal(supportsAgentCapability("unknown", "browse"), false);
  assert.equal(supportsAgentCapability("__proto__", "browse"), false);
});
