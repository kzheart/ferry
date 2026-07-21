// Pure contract checks for frontend agent-agnostic tool helpers.
// Kept free of browser/i18n imports so it can run under plain Node.
import assert from "node:assert/strict";

function hydrate(list) {
  const TOOLS = [];
  const TOOL_NAME = {};
  for (const m of list) {
    TOOLS.push(m.id);
    TOOL_NAME[m.id] = m.display_name || m.id;
  }
  return { TOOLS, TOOL_NAME, manifests: list };
}

function toolManifest(manifests, tool) {
  return manifests.find(item => item.id === tool) || null;
}

function toolCapabilities(manifests, tool) {
  return toolManifest(manifests, tool)?.capabilities || [];
}

function toolHasCapability(manifests, tool, capability) {
  return toolCapabilities(manifests, tool).includes(capability);
}

function toolsWithCapability(manifests, tools, capability) {
  return tools.filter(tool => toolHasCapability(manifests, tool, capability));
}

function toolReferenceKind(manifests, tool) {
  return toolManifest(manifests, tool)?.reference_kind === "id" ? "id" : "path";
}

function sessionRef(manifests, session) {
  return toolReferenceKind(manifests, session.tool) === "id"
    ? session.id
    : (session.path || session.id);
}

const { TOOLS, manifests } = hydrate([
  {
    id: "claude",
    display_name: "Claude Code",
    reference_kind: "path",
    capabilities: ["browse", "migrate-source", "migrate-target"],
  },
  {
    id: "opencode",
    display_name: "OpenCode",
    reference_kind: "id",
    capabilities: ["browse", "migrate-source", "migrate-target"],
  },
  {
    id: "readonly",
    display_name: "Read Only",
    reference_kind: "path",
    capabilities: ["browse"],
  },
]);

assert.deepEqual(TOOLS, ["claude", "opencode", "readonly"]);
assert.equal(toolReferenceKind(manifests, "claude"), "path");
assert.equal(toolReferenceKind(manifests, "opencode"), "id");
assert.equal(toolReferenceKind(manifests, "missing"), "path");
assert.deepEqual(toolCapabilities(manifests, "readonly"), ["browse"]);
assert.equal(toolHasCapability(manifests, "readonly", "migrate-source"), false);
assert.deepEqual(
  toolsWithCapability(manifests, TOOLS, "migrate-target"),
  ["claude", "opencode"],
);
assert.equal(
  sessionRef(manifests, { tool: "claude", id: "sid", path: "/tmp/a.jsonl" }),
  "/tmp/a.jsonl",
);
assert.equal(
  sessionRef(manifests, { tool: "opencode", id: "ses_1", path: "/tmp/ignored" }),
  "ses_1",
);
assert.equal(
  sessionRef(manifests, { tool: "readonly", id: "sid", path: "" }),
  "sid",
);

console.log("tool-contract checks passed");
