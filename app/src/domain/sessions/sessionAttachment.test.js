import assert from "node:assert/strict";
import test from "node:test";

import {
  buildSessionPrompt,
  parseSessionAttachments,
  serializeSessionAttachment,
  sessionAttachment,
  sessionIdentity,
} from "./sessionAttachment.js";

test("会话身份包含来源工具，避免相同原生 id 串联", () => {
  assert.equal(sessionIdentity({ tool: "claude", id: "shared" }), "claude\0shared");
  assert.equal(sessionIdentity({ tool: "codex", id: "shared" }), "codex\0shared");
  assert.notEqual(
    sessionIdentity({ tool: "claude", id: "shared" }),
    sessionIdentity({ tool: "codex", id: "shared" }),
  );
});

test("会话附件只接受 Engine 签发的 opaque ref", () => {
  assert.equal(sessionAttachment({ tool: "claude", id: "native-id" }), null);
  assert.deepEqual(
    sessionAttachment({
      tool: "claude",
      ref: "fsr_current",
      id: "native-id",
      title: "当前会话",
    }),
    { tool: "claude", ref: "fsr_current", title: "当前会话" },
  );
});

test("会话附件序列化和提示词只传 opaque ref", () => {
  const source = {
    tool: "codex",
    ref: "fsr_current",
    id: "native-id",
    title: "审查",
  };
  const serialized = serializeSessionAttachment(source);
  assert.match(serialized, /fsr_current/);
  assert.doesNotMatch(serialized, /native-id/);
  assert.deepEqual(parseSessionAttachments(serialized), [{
    tool: "codex",
    ref: "fsr_current",
    title: "审查",
  }]);
  const prompt = buildSessionPrompt("检查", [sessionAttachment(source)]);
  assert.equal(
    prompt,
    '<ferry_session_refs>{"sessions":[{"tool":"codex","ref":"fsr_current"}]}</ferry_session_refs>\n\n检查',
  );
  assert.doesNotMatch(prompt, /session_id|native-id/);
});
