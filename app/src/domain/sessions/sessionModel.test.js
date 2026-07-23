import assert from "node:assert/strict";
import test from "node:test";

import { sessionRef } from "./sessionModel.js";

test("sessionRef only returns the Engine-issued opaque reference", () => {
  assert.equal(sessionRef({
    tool: "codex",
    id: "native-session-id",
    path: "/private/session.jsonl",
    ref: "fsr_current",
  }), "fsr_current");
});
