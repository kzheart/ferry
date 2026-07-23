import test from "node:test";
import assert from "node:assert/strict";
import {
  existingProposalFor,
  generationInput,
  proposalTargets,
} from "./organizationModel.js";

test("reuses an existing proposal for unchanged fingerprints", () => {
  const backbones = [{ tool: "codex", id: "a", fingerprint: "fp" }];
  assert.equal(existingProposalFor([
    { proposal_id: "p", status: "pending",
      targets: [{ tool: "codex", id: "a", fingerprint: "fp" }] },
  ], backbones).proposal_id, "p");
});

test("only pending segments carry original text while cached segments use digest", () => {
  const input = generationInput([{
    tool: "codex", id: "a", segments: [
      { hash: "h1", digest: "cached" }, { hash: "h2", digest: null },
    ], pending_sources: [{ hash: "h2", text: "source transcript" }],
  }], [{ tool: "codex", id: "a", title: "T" }], "en");
  assert.deepEqual(input.sessions[0].segments, [
    { hash: "h1", text: "cached", digest: "cached" },
    { hash: "h2", text: "source transcript", digest: null },
  ]);
});

test("maps generated clusters and dead-session evidence into proposal patches", () => {
  const targets = proposalTargets({ sessions: [{
    tool: "codex", id: "a", fingerprint: "fp",
    segments: [{ hash: "h1" }],
  }] }, {
    sessions: [{ tool: "codex", id: "a", title: "Title", summary: "Summary",
      tags: ["tag"], dead: true, dead_reason: "superseded" }],
    clusters: [{ id: "cluster", name: "Project",
      members: [{ tool: "codex", id: "a" }] }],
  });
  assert.deepEqual(targets[0].suggested, {
    name: "Title", summary: "Summary", tags: ["tag"],
    dead_candidate: true, dead_reason: "superseded",
    cluster_id: "cluster", cluster_name: "Project",
  });
});
