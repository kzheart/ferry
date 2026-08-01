import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { describe, expect, it, vi } from "vitest";
import {
  AGENT_CAPABILITIES,
  AGENT_EDIT_OPERATIONS,
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
const sessionCleanupTool = tools.find(
  (tool) => tool.name === "session_cleanup",
)!;
const sessionCleanupSchema = sessionCleanupTool.parameters;
const askUserTool = tools.find((tool) => tool.name === "ask_user")!;
const askUserSchema = askUserTool.parameters;

/** 一套启用了会话优化策略的工具;read/edit 共享同一次工厂调用的策略状态。 */
function optimizationTools(
  invoke: (name: string, args: Record<string, unknown>) => Promise<unknown>,
) {
  const created = createFerryTools(
    { invoke: invoke as never },
    () => ({ sessionId: "session", runId: "run" }),
    ["session_search", "session_read", "session_edit"],
    {
      sessionEdit: {
        allowedOperations: ["rewrite"],
        requireReadUserLocator: true,
        requireMatchingPreview: true,
      },
    },
  );
  return {
    readTool: created.find((tool) => tool.name === "session_read")!,
    editTool: created.find((tool) => tool.name === "session_edit")!,
  };
}

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

  it("validates the four session_cleanup intent branches", () => {
    const target = { tool: "claude", ref: "fsr_session" };
    const scopeId = "0123456789abcdef";
    expect(
      Check(sessionCleanupSchema, {
        intent: "inventory",
        scope: { agents: ["claude"], updated_before: "now-7d" },
        cursor: "cursor",
      }),
    ).toBe(true);
    expect(
      Check(sessionCleanupSchema, {
        intent: "triage",
        scope_id: scopeId,
        verdicts: [{ ...target, verdict: "delete", reason: "old" }],
      }),
    ).toBe(true);
    expect(
      Check(sessionCleanupSchema, {
        intent: "preview",
        scope_id: scopeId,
        targets: [target],
      }),
    ).toBe(true);
    expect(
      Check(sessionCleanupSchema, {
        intent: "execute",
        scope_id: scopeId,
        targets: [target],
      }),
    ).toBe(true);
    expect(
      Check(sessionCleanupSchema, {
        intent: "inventory",
        scope_id: scopeId,
      }),
    ).toBe(false);
    expect(
      Check(sessionCleanupSchema, {
        intent: "triage",
        scope_id: scopeId,
        scope: {},
        verdicts: [{ ...target, verdict: "keep" }],
      }),
    ).toBe(false);
    expect(
      Check(sessionCleanupSchema, {
        intent: "preview",
        scope_id: scopeId,
        targets: [target],
        verdicts: [{ ...target, verdict: "delete" }],
      }),
    ).toBe(false);
  });

  it("requires a matching cleanup preview before execute", async () => {
    const invoke = vi.fn(async () => ({ ok: true }));
    const cleanupTool = createFerryTools(
      { invoke },
      () => ({ sessionId: "session", runId: "run" }),
    ).find((tool) => tool.name === "session_cleanup")!;
    const base = {
      scope_id: "0123456789abcdef",
      targets: [{ tool: "claude", ref: "fsr_session" }],
    };

    await expect(
      cleanupTool.execute(
        "call",
        { ...base, intent: "execute" },
        undefined,
        undefined,
      ),
    ).rejects.toThrow("requires a successful preview");
    await cleanupTool.execute(
      "call",
      { ...base, intent: "preview" },
      undefined,
      undefined,
    );
    await expect(
      cleanupTool.execute(
        "call",
        {
          ...base,
          intent: "execute",
          targets: [{ tool: "claude", ref: "fsr_changed" }],
        },
        undefined,
        undefined,
      ),
    ).rejects.toThrow("requires a successful preview");
    await cleanupTool.execute(
      "call",
      { ...base, intent: "execute" },
      undefined,
      undefined,
    );
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

  it("optimizer_rejects_non_rewrite_ops", async () => {
    const invoke = vi.fn(async () => ({}));
    const { editTool } = optimizationTools(invoke);
    await expect(
      editTool.execute(
        "call",
        { tool: "codex", ref: "fsr_session", patch: { pinned: true } },
        undefined,
        undefined,
      ),
    ).rejects.toThrow("does not allow metadata patch");
    await expect(
      editTool.execute(
        "call",
        {
          tool: "codex",
          ref: "fsr_session",
          ops: [{ op: "delete-turn", turn: 1 }],
          intent: "preview",
        },
        undefined,
        undefined,
      ),
    ).rejects.toThrow("only allows rewrite ops");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("optimizer_requires_read_user_locator", async () => {
    const invoke = vi.fn(async (name: string) =>
      name === "session_read"
        ? {
            messages: [
              { role: "user", editable: true, locator: "fml_user_1" },
              { role: "assistant", editable: false, locator: "fml_asst_1" },
            ],
            matches: [
              { role: "user", editable: false, locator: "fml_user_locked" },
            ],
          }
        : {},
    );
    const { editTool, readTool } = optimizationTools(invoke);
    const preview = (locator: string) =>
      editTool.execute(
        "call",
        {
          tool: "codex",
          ref: "fsr_session",
          ops: [{ op: "rewrite", locator, text: "better" }],
          intent: "preview",
        },
        undefined,
        undefined,
      );

    // 未读取任何消息之前,一律拒绝
    await expect(preview("fml_user_1")).rejects.toThrow(
      "call session_read first",
    );
    await readTool.execute(
      "call",
      { tool: "codex", ref: "fsr_session" },
      undefined,
      undefined,
    );
    // assistant locator 和不可编辑的 user locator 都不进白名单
    await expect(preview("fml_asst_1")).rejects.toThrow(
      "call session_read first",
    );
    await expect(preview("fml_user_locked")).rejects.toThrow(
      "call session_read first",
    );
    await expect(preview("fml_user_1")).resolves.toBeDefined();
  });

  it("optimizer_requires_matching_preview", async () => {
    const invoke = vi.fn(async (name: string) =>
      name === "session_read"
        ? { messages: [{ role: "user", editable: true, locator: "fml_u1" }] }
        : {},
    );
    const { editTool, readTool } = optimizationTools(invoke);
    await readTool.execute(
      "call",
      { tool: "codex", ref: "fsr_session" },
      undefined,
      undefined,
    );
    const batch = (text: string, intent: string) =>
      editTool.execute(
        "call",
        {
          tool: "codex",
          ref: "fsr_session",
          ops: [{ op: "rewrite", locator: "fml_u1", text }],
          intent,
        },
        undefined,
        undefined,
      );

    // 没 preview 直接 execute:拒绝
    await expect(batch("draft", "execute")).rejects.toThrow(
      "requires a successful preview",
    );
    await batch("draft", "preview");
    // preview 之后偷偷改文案再 execute:拒绝
    await expect(batch("changed", "execute")).rejects.toThrow(
      "requires a successful preview",
    );
  });

  it("optimizer_allows_matching_execute", async () => {
    const invoke = vi.fn(async (name: string) =>
      name === "session_read"
        ? { messages: [{ role: "user", editable: true, locator: "fml_u1" }] }
        : {},
    );
    const { editTool, readTool } = optimizationTools(invoke);
    await readTool.execute(
      "call",
      { tool: "codex", ref: "fsr_session" },
      undefined,
      undefined,
    );
    const params = {
      tool: "codex",
      ref: "fsr_session",
      ops: [{ op: "rewrite", locator: "fml_u1", text: "final" }],
    };
    await editTool.execute(
      "call",
      { ...params, intent: "preview" },
      undefined,
      undefined,
    );
    await expect(
      editTool.execute(
        "call",
        { ...params, intent: "execute" },
        undefined,
        undefined,
      ),
    ).resolves.toBeDefined();
    // read + preview + execute 各一次都真正到达 port
    expect(invoke).toHaveBeenCalledTimes(3);
  });

  it("describes the explicit operation intent", () => {
    const exposedOperations = new Set(["delete-turn", "rewrite"]);
    const supportDescription = AGENT_IDS.flatMap((tool) => {
      const operations = AGENT_EDIT_OPERATIONS[tool].filter((operation) =>
        exposedOperations.has(operation),
      );
      return operations.length ? [`${tool}: ${operations.join(", ")}`] : [];
    }).join("; ");
    expect(migrateTool.description).toContain("intent is required");
    expect(sessionEditTool.description).toContain(
      "Metadata patch does not accept intent",
    );
    expect(sessionEditTool.description).toContain(supportDescription);
    expect(sessionCleanupTool.description).toContain("covered equals total");
    expect(askUserTool.description).toContain("does not authorize deletion");
  });
});
