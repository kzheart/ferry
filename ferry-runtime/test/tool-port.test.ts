import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";
import { createFerryTools } from "../src/tools/catalog.js";

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
const migrateTool = tools.find((tool) => tool.name === "migrate")!;
const migrateSchema = migrateTool.parameters;

describe("Ferry mutation tool schemas", () => {
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
        intent: "preview",
      }),
    ).toBe(true);
    expect(
      Check(sessionEditSchema, {
        tool: "codex",
        ref: "fsr_session",
        ops: [{ op: "delete-turn", turn: 1 }],
        intent: "execute",
      }),
    ).toBe(true);
    expect(
      Check(sessionEditSchema, {
        tool: "codex",
        ref: "fsr_session",
        patch: { pinned: true },
      }),
    ).toBe(true);
    expect(
      Check(sessionEditSchema, {
        tool: "codex",
        ref: "fsr_session",
        ops: [{ op: "delete-turn", turn: 1 }],
      }),
    ).toBe(false);
    expect(
      Check(sessionEditSchema, {
        tool: "codex",
        ref: "fsr_session",
        patch: { pinned: true },
        intent: "preview",
      }),
    ).toBe(false);
  });

  it("requires migration intent in the schema", () => {
    const migration = {
      source_tool: "claude",
      ref: "fsr_session",
      target_tool: "codex",
    };
    expect(Check(migrateSchema, migration)).toBe(false);
    expect(Check(migrateSchema, { ...migration, intent: "preview" })).toBe(
      true,
    );
    expect(Check(migrateSchema, { ...migration, intent: "execute" })).toBe(
      true,
    );
    expect(Check(migrateSchema, { ...migration, intent: "invalid" })).toBe(
      false,
    );
  });

  it("enforces content intent and metadata boundaries during execution", async () => {
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
        intent: "execute",
      }),
    ).rejects.toThrow("requires exactly one");
    await expect(
      execute({
        tool: "codex",
        ref: "fsr_session",
        patch: { pinned: true },
        intent: "preview",
      }),
    ).rejects.toThrow("metadata patch does not accept intent");
    await expect(
      execute({
        tool: "codex",
        ref: "fsr_session",
        ops: [{ op: "delete-turn", turn: 1 }],
      }),
    ).rejects.toThrow("ops require intent");
    await expect(
      execute({
        tool: "codex",
        ref: "fsr_session",
        ops: [{ op: "delete-turn", turn: 1 }],
        intent: "invalid",
      }),
    ).rejects.toThrow("ops require intent");
  });

  it("enforces migration intent during execution", async () => {
    const execute = (params: Record<string, unknown>) =>
      migrateTool.execute("call", params, undefined, undefined);
    const migration = {
      source_tool: "claude",
      ref: "fsr_session",
      target_tool: "codex",
    };

    await expect(execute(migration)).rejects.toThrow(
      "requires intent preview or execute",
    );
    await expect(execute({ ...migration, intent: "invalid" })).rejects.toThrow(
      "requires intent preview or execute",
    );
  });

  it("describes the explicit operation intent", () => {
    expect(migrateTool.description).toContain("intent is required");
    expect(sessionEditTool.description).toContain(
      "Metadata patch does not accept intent",
    );
  });
});
