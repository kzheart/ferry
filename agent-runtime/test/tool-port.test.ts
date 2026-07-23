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
const sessionEditTool = tools.find((tool) => tool.name === "session_edit")!;

describe("session_edit schema", () => {
  it("uses an object root accepted by function-tool providers", () => {
    for (const tool of tools) {
      expect(tool.parameters).toMatchObject({ type: "object" });
    }
  });

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

  it("rejects ambiguous edit modes at the execution boundary", async () => {
    const execute = (params: Record<string, unknown>) =>
      sessionEditTool.execute("call", params, undefined, undefined);
    await expect(
      execute({
        tool: "codex",
        ref: "fsr_session",
      }),
    ).rejects.toThrow("requires exactly one");
    await expect(
      execute({
        tool: "codex",
        ref: "fsr_session",
        ops: [{ op: "delete-turn", turn: 1 }],
        patch: { pinned: true },
      }),
    ).rejects.toThrow("requires exactly one");
    await expect(
      execute({
        tool: "codex",
        ref: "fsr_session",
        patch: { pinned: true },
        dry_run: true,
      }),
    ).rejects.toThrow("dry_run is only valid");
    await expect(
      execute({
        tool: "codex",
        ref: "fsr_session",
        patch: { pinned: true },
        dry_run: false,
      }),
    ).rejects.toThrow("dry_run is only valid");
  });
});
