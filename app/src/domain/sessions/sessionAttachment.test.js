import assert from "node:assert/strict";
import test from "node:test";

import { sessionIdentity } from "./sessionAttachment.js";

test("会话身份包含来源工具，避免相同原生 id 串联", () => {
  assert.equal(sessionIdentity({ tool: "claude", id: "shared" }), "claude\0shared");
  assert.equal(sessionIdentity({ tool: "codex", id: "shared" }), "codex\0shared");
  assert.notEqual(
    sessionIdentity({ tool: "claude", id: "shared" }),
    sessionIdentity({ tool: "codex", id: "shared" }),
  );
});
