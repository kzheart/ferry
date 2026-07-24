import assert from "node:assert/strict";
import test from "node:test";

import {
  supportsAssistantReplyEditing,
  supportsSessionEditing,
} from "./agentEditSupport.js";
import { AGENT_IDS } from "./generated/agents.js";

test("当前静态 Agent 契约定义编辑范围", () => {
  assert.deepEqual(AGENT_IDS, ["claude", "codex", "opencode"]);
  for (const tool of AGENT_IDS) assert.equal(supportsSessionEditing(tool), true);
  assert.equal(supportsAssistantReplyEditing("claude"), true);
  assert.equal(supportsAssistantReplyEditing("codex"), true);
  assert.equal(supportsAssistantReplyEditing("opencode"), false);
  assert.equal(supportsSessionEditing("unknown"), false);
});
