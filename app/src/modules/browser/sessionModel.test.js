import assert from "node:assert/strict";
import { test } from "vitest";

import { sessionRef, toTimeline } from "./sessionModel.js";

test("sessionRef only returns the Engine-issued opaque reference", () => {
  assert.equal(sessionRef({
    tool: "codex",
    id: "native-session-id",
    path: "/private/session.jsonl",
    ref: "fsr_current",
  }), "fsr_current");
});

test("toTimeline defers unreached compactions while more pages remain", () => {
  const rounds = [{ n: 1 }, { n: 2 }];
  const compactions = [
    { id: "c1", after_turn: 1 },
    { id: "c2", after_turn: 5 },
    { id: "c3", after_turn: 9 },
  ];
  const timeline = toTimeline(rounds, compactions, true);
  assert.deepEqual(
    timeline.map(item => item.key),
    ["round:1", "compaction:c1", "round:2"],
  );
});

test("toTimeline appends out-of-range compactions once fully loaded", () => {
  const rounds = [{ n: 1 }, { n: 2 }];
  const compactions = [
    { id: "c1", after_turn: 2 },
    { id: "c2", after_turn: 9 },
  ];
  const timeline = toTimeline(rounds, compactions, false);
  assert.deepEqual(
    timeline.map(item => item.key),
    ["round:1", "round:2", "compaction:c1", "compaction:c2"],
  );
});

test("toTimeline merges same-turn compactions into one group item", () => {
  const rounds = [{ n: 1 }, { n: 2 }];
  const compactions = [
    { id: "c1", after_turn: 1 },
    { id: "c2", after_turn: 1 },
    { id: "c3", after_turn: 2 },
  ];
  const timeline = toTimeline(rounds, compactions, false);
  assert.deepEqual(
    timeline.map(item => item.key),
    ["round:1", "compaction:c1", "round:2", "compaction:c3"],
  );
  assert.deepEqual(
    timeline[1].compactions.map(item => item.id),
    ["c1", "c2"],
  );
});
