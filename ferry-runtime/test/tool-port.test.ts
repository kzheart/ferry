import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { describe, expect, it, vi } from "vitest";
import {
  AGENT_CAPABILITIES,
  AGENT_IDS,
} from "../src/server/generated/agents.js";
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
const agentPromptTool = tools.find((tool) => tool.name === "agent_prompt")!;
const agentPromptSchema = agentPromptTool.parameters;
const sessionDeleteTool = tools.find((tool) => tool.name === "session_delete")!;
const sessionDeleteSchema = sessionDeleteTool.parameters;
const askUserTool = tools.find((tool) => tool.name === "ask_user")!;
const askUserSchema = askUserTool.parameters;

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

  it("validates session_delete tool, refs and intent shape", () => {
    expect(
      Check(sessionDeleteSchema, {
        tool: "claude",
        refs: ["fsr_session"],
        intent: "preview",
      }),
    ).toBe(true);
    expect(
      Check(sessionDeleteSchema, {
        tool: "claude",
        refs: ["fsr_session_a", "fsr_session_b"],
        intent: "execute",
      }),
    ).toBe(true);
    expect(
      Check(sessionDeleteSchema, {
        tool: "claude",
        refs: [],
        intent: "preview",
      }),
    ).toBe(false);
    expect(
      Check(sessionDeleteSchema, {
        tool: "claude",
        ref: "fsr_session",
        intent: "preview",
      }),
    ).toBe(false);
    expect(
      Check(sessionDeleteSchema, {
        tool: "claude",
        refs: ["fsr_session"],
      }),
    ).toBe(false);
  });

  it("gates session_delete on a per-batch preview burned by execute", async () => {
    const invoke = vi.fn(async () => ({ ok: true }));
    const deleteTool = createFerryTools({ invoke }, () => ({
      sessionId: "session",
      runId: "run",
    })).find((tool) => tool.name === "session_delete")!;
    const run = (input: Record<string, unknown>) =>
      deleteTool.execute("call", input, undefined, undefined);

    // Provider 未必真的执行 function schema:删除入口不能只靠 schema 兜底
    await expect(
      run({ tool: "nope", refs: ["fsr_session"], intent: "preview" }),
    ).rejects.toThrow("requires a known tool");
    await expect(
      run({ tool: "codex", refs: ["fsr_a", "fsr_a"], intent: "preview" }),
    ).rejects.toThrow("unique non-empty refs");
    await expect(run({ tool: "codex", refs: ["fsr_session"] })).rejects.toThrow(
      "intent preview or execute",
    );
    await expect(
      run({ tool: "codex", refs: ["fsr_session"], intent: "execute" }),
    ).rejects.toThrow("requires a successful preview");

    await run({ tool: "codex", refs: ["fsr_a", "fsr_b"], intent: "preview" });
    await expect(
      run({ tool: "codex", refs: ["fsr_a"], intent: "execute" }),
    ).rejects.toThrow("requires a successful preview");
    await expect(
      run({ tool: "claude", refs: ["fsr_a", "fsr_b"], intent: "execute" }),
    ).rejects.toThrow("requires a successful preview");
    // 顺序无关:指纹按排序后的 ref 集合计算
    await run({ tool: "codex", refs: ["fsr_b", "fsr_a"], intent: "execute" });
    // 同一批不能凭一次预览删两次
    await expect(
      run({ tool: "codex", refs: ["fsr_a", "fsr_b"], intent: "execute" }),
    ).rejects.toThrow("requires a successful preview");
    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it("bounds ask_user option counts", () => {
    const option = { label: "option" };
    expect(
      Check(askUserSchema, { question: "Choose", options: [option] }),
    ).toBe(false);
    expect(
      Check(askUserSchema, {
        question: "Choose",
        options: [option, { label: "other" }],
      }),
    ).toBe(true);
    expect(
      Check(askUserSchema, {
        question: "Choose",
        options: Array.from({ length: 6 }, (_, index) => ({
          label: `option-${index}`,
        })),
      }),
    ).toBe(true);
    expect(
      Check(askUserSchema, {
        question: "Choose",
        options: Array.from({ length: 7 }, (_, index) => ({
          label: `option-${index}`,
        })),
      }),
    ).toBe(false);
  });

  it("exposes agent_prompt only for prompt-capable agents with strict bounds", () => {
    const promptAgents = AGENT_IDS.filter((agent) =>
      (AGENT_CAPABILITIES[agent] as readonly string[]).includes("prompt"),
    );
    for (const tool of promptAgents) {
      expect(
        Check(agentPromptSchema, {
          tool,
          ref: "fsr_session",
          prompt: "继续完成任务",
          model: "configured-model",
          timeout_sec: 360,
        }),
      ).toBe(true);
    }
    expect(
      Check(agentPromptSchema, {
        tool: "unknown",
        ref: "fsr_session",
        prompt: "继续",
      }),
    ).toBe(false);
    expect(
      Check(agentPromptSchema, {
        tool: "codex",
        ref: "fsr_session",
        prompt: "",
      }),
    ).toBe(false);
    expect(
      Check(agentPromptSchema, {
        tool: "codex",
        ref: "fsr_session",
        prompt: "继续",
        timeout_sec: 361,
      }),
    ).toBe(false);
    expect(
      Check(agentPromptSchema, {
        tool: "codex",
        ref: "fsr_session",
        prompt: "继续",
        session_id: "bypass",
      }),
    ).toBe(false);
    expect(agentPromptTool.executionMode).toBe("sequential");
    expect(agentPromptTool.description).toContain("high-privilege");
    expect(agentPromptTool.description).toContain("next_ref");
  });

  it("returns agent_prompt text and next_ref through the existing tool port", async () => {
    const invoke = vi.fn(async () => ({
      status: "completed",
      text: "implemented",
      next_ref: "fsr_next",
    }));
    const promptTool = createFerryTools({ invoke }, () => ({
      sessionId: "session",
      runId: "run",
    })).find((tool) => tool.name === "agent_prompt")!;

    const result = await promptTool.execute(
      "call",
      {
        tool: "pi",
        ref: "fsr_session",
        prompt: "继续",
        timeout_sec: 120,
      },
      undefined,
      undefined,
    );

    expect(invoke).toHaveBeenCalledWith(
      "agent_prompt",
      {
        tool: "pi",
        ref: "fsr_session",
        prompt: "继续",
        timeout_sec: 120,
      },
      expect.objectContaining({
        sessionId: "session",
        runId: "run",
        toolCallId: "call",
      }),
    );
    expect(result).toMatchObject({
      details: {
        text: "implemented",
        next_ref: "fsr_next",
      },
    });
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

  it("rejects source-unsupported content operations before invoking the port", async () => {
    const invoke = vi.fn(async () => ({}));
    const editTool = createFerryTools({ invoke }, () => ({
      sessionId: "session",
      runId: "run",
    })).find((tool) => tool.name === "session_edit")!;
    const execute = (params: Record<string, unknown>) =>
      editTool.execute("call", params, undefined, undefined);

    await expect(
      execute({
        tool: "opencode",
        ref: "fsr_session",
        ops: [
          { op: "rewrite", locator: "fml_message", text: "updated" },
          { op: "delete-turn", turn: 1 },
        ],
        intent: "preview",
      }),
    ).rejects.toThrow(
      "opencode does not support content operations: delete-turn; supported: rewrite",
    );
    await expect(
      execute({
        tool: "unknown",
        ref: "fsr_session",
        ops: [{ op: "rewrite", locator: "fml_message", text: "updated" }],
        intent: "preview",
      }),
    ).rejects.toThrow("content ops require a known source tool");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("passes source-supported content operations to the port", async () => {
    const invoke = vi.fn(async () => ({}));
    const editTool = createFerryTools({ invoke }, () => ({
      sessionId: "session",
      runId: "run",
    })).find((tool) => tool.name === "session_edit")!;

    await editTool.execute(
      "call",
      {
        tool: "opencode",
        ref: "fsr_session",
        ops: [{ op: "rewrite", locator: "fml_message", text: "updated" }],
        intent: "preview",
      },
      undefined,
      undefined,
    );

    expect(invoke).toHaveBeenCalledOnce();
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
});
