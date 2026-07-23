import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";
import { createFerryTools } from "../src/tool-port.js";

const requireFromPiAi = createRequire(
  import.meta.resolve("@earendil-works/pi-ai"),
);
const { Check } = await import(
  pathToFileURL(requireFromPiAi.resolve("typebox/value")).href
);

const tools = createFerryTools(
  {
    async invoke() {
      return {};
    },
  },
  () => ({ sessionId: "session", runId: "run" }),
);
const sessionEditSchema = tools.find(
  (tool) => tool.name === "session_edit",
)!.parameters;

describe("session_edit schema", () => {
  it("accepts exactly one edit mode", () => {
    expect(
      Check(sessionEditSchema, {
        tool: "codex",
        ref: "fsr_session",
        ops: [{ op: "delete-turn", turn: 1 }],
      }),
    ).toBe(true);
    expect(
      Check(sessionEditSchema, {
        tool: "codex",
        ref: "fsr_session",
        ops: [{ op: "delete-turn", turn: 1 }],
        dry_run: true,
      }),
    ).toBe(true);
    expect(
      Check(sessionEditSchema, {
        tool: "codex",
        ref: "fsr_session",
        patch: { pinned: true },
      }),
    ).toBe(true);
  });

  it("rejects both, neither, and dry_run on metadata patches", () => {
    expect(
      Check(sessionEditSchema, {
        tool: "codex",
        ref: "fsr_session",
      }),
    ).toBe(false);
    expect(
      Check(sessionEditSchema, {
        tool: "codex",
        ref: "fsr_session",
        ops: [{ op: "delete-turn", turn: 1 }],
        patch: { pinned: true },
      }),
    ).toBe(false);
    expect(
      Check(sessionEditSchema, {
        tool: "codex",
        ref: "fsr_session",
        patch: { pinned: true },
        dry_run: true,
      }),
    ).toBe(false);
    expect(
      Check(sessionEditSchema, {
        tool: "codex",
        ref: "fsr_session",
        patch: { pinned: true },
        dry_run: false,
      }),
    ).toBe(false);
  });
});
