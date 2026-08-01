import type { AgentTool } from "@earendil-works/pi-agent-core";
import { Type } from "@earendil-works/pi-ai";
import {
  AGENT_CAPABILITIES,
  AGENT_EDIT_OPERATIONS,
  AGENT_IDS,
  type AgentId,
} from "../server/generated/agents.js";
import {
  OPAQUE_SESSION_REF_MAX_LENGTH,
  OPAQUE_SESSION_REF_MIN_LENGTH,
} from "../server/generated/session-ref.js";

const opaqueSessionRef = Type.String({
  minLength: OPAQUE_SESSION_REF_MIN_LENGTH,
  maxLength: OPAQUE_SESSION_REF_MAX_LENGTH,
  pattern: "^[A-Za-z0-9_-]+$",
});

// 相对写法必须写进 schema:模型没有时钟,不知道能这么写就会去 shell 里问 date。
const timePoint = Type.Union(
  [Type.Integer({ minimum: 0 }), Type.String({ maxLength: 64 })],
  {
    description:
      'Epoch milliseconds, ISO8601, "now", or a relative offset such as "now-7d" or "-12h" (units s/m/h/d/w). Results also carry a now field you can compute from.',
  },
);

const timeRange = Type.Object(
  {
    from: Type.Optional(timePoint),
    to: Type.Optional(timePoint),
  },
  { additionalProperties: false },
);

const editOps = Type.Array(
  Type.Union([
    Type.Object(
      {
        op: Type.Literal("delete-turn"),
        turn: Type.Integer({ minimum: 1 }),
      },
      { additionalProperties: false },
    ),
    Type.Object(
      {
        op: Type.Literal("rewrite"),
        locator: Type.String({
          pattern: "^fml_",
          maxLength: 512,
          description:
            "Copy this value exactly from context messages[].locator or content-search matches[].locator. Never invent or transform it.",
        }),
        text: Type.String({ minLength: 1, maxLength: 20_000 }),
      },
      { additionalProperties: false },
    ),
  ]),
  { minItems: 1, maxItems: 50 },
);

const operationIntent = Type.Union([
  Type.Literal("preview"),
  Type.Literal("execute"),
]);

const metadataPatch = Type.Object(
  {
    name: Type.Optional(Type.String({ maxLength: 200 })),
    pinned: Type.Optional(Type.Boolean()),
    archived: Type.Optional(Type.Boolean()),
    tags: Type.Optional(
      Type.Array(Type.String({ maxLength: 64 }), { maxItems: 20 }),
    ),
  },
  { additionalProperties: false },
);

const agentEnum = (values: readonly string[]) =>
  Type.Unsafe({ type: "string", enum: [...values] });

const cleanupScope = Type.Object(
  {
    agents: Type.Optional(
      Type.Array(agentEnum(AGENT_IDS), { maxItems: 32 }),
    ),
    projects: Type.Optional(
      Type.Array(Type.String({ minLength: 1, maxLength: 256 }), {
        maxItems: 20,
      }),
    ),
    updated_before: Type.Optional(timePoint),
  },
  { additionalProperties: false },
);

const cleanupScopeId = Type.String({
  minLength: 16,
  maxLength: 16,
  pattern: "^[0-9a-f]{16}$",
});

const cleanupTarget = Type.Object(
  {
    tool: agentEnum(AGENT_IDS),
    ref: opaqueSessionRef,
    reason: Type.Optional(Type.String({ maxLength: 300 })),
  },
  { additionalProperties: false },
);

const cleanupVerdict = Type.Object(
  {
    tool: agentEnum(AGENT_IDS),
    ref: opaqueSessionRef,
    verdict: Type.Union([
      Type.Literal("delete"),
      Type.Literal("keep"),
      Type.Literal("ask_user"),
    ]),
    reason: Type.Optional(Type.String({ maxLength: 300 })),
  },
  { additionalProperties: false },
);

// Function-tool providers require an object root. Intent-specific field
// constraints are repeated at the execution boundary below.
const sessionCleanupSchema = Type.Unsafe({
  type: "object",
  properties: {
    intent: Type.Union([
      Type.Literal("inventory"),
      Type.Literal("triage"),
      Type.Literal("preview"),
      Type.Literal("execute"),
    ]),
    scope: cleanupScope,
    cursor: Type.Optional(Type.String({ minLength: 1, maxLength: 512 })),
    scope_id: cleanupScopeId,
    verdicts: Type.Array(cleanupVerdict, { minItems: 1, maxItems: 100 }),
    targets: Type.Array(cleanupTarget, { minItems: 1, maxItems: 500 }),
  },
  required: ["intent"],
  additionalProperties: false,
  oneOf: [
    {
      properties: { intent: Type.Literal("inventory") },
      not: {
        anyOf: [
          { required: ["scope_id"] },
          { required: ["verdicts"] },
          { required: ["targets"] },
        ],
      },
    },
    {
      properties: { intent: Type.Literal("triage") },
      required: ["scope_id", "verdicts"],
      not: {
        anyOf: [
          { required: ["scope"] },
          { required: ["cursor"] },
          { required: ["targets"] },
        ],
      },
    },
    {
      properties: { intent: Type.Literal("preview") },
      required: ["scope_id", "targets"],
      not: {
        anyOf: [
          { required: ["scope"] },
          { required: ["cursor"] },
          { required: ["verdicts"] },
        ],
      },
    },
    {
      properties: { intent: Type.Literal("execute") },
      required: ["scope_id", "targets"],
      not: {
        anyOf: [
          { required: ["scope"] },
          { required: ["cursor"] },
          { required: ["verdicts"] },
        ],
      },
    },
  ],
});

const askUserSchema = Type.Object(
  {
    question: Type.String({ minLength: 1, maxLength: 500 }),
    options: Type.Array(
      Type.Object(
        {
          label: Type.String({ minLength: 1, maxLength: 80 }),
          description: Type.Optional(Type.String({ maxLength: 200 })),
          recommended: Type.Optional(Type.Boolean()),
        },
        { additionalProperties: false },
      ),
      { minItems: 2, maxItems: 6 },
    ),
    multi_select: Type.Optional(Type.Boolean()),
    allow_custom: Type.Optional(Type.Boolean()),
  },
  { additionalProperties: false },
);

const sessionEditSchema = Type.Unsafe({
  type: "object",
  properties: {
    tool: Type.String({ minLength: 1, maxLength: 32 }),
    ref: opaqueSessionRef,
    ops: editOps,
    patch: metadataPatch,
    intent: operationIntent,
  },
  required: ["tool", "ref"],
  additionalProperties: false,
  oneOf: [
    {
      required: ["ops", "intent"],
      not: { required: ["patch"] },
    },
    {
      required: ["patch"],
      not: {
        anyOf: [{ required: ["ops"] }, { required: ["intent"] }],
      },
    },
  ],
});

const exposedSessionEditOperations = new Set(["delete-turn", "rewrite"]);
type AgentCapability = (typeof AGENT_CAPABILITIES)[AgentId][number];
const supportsAgentCapability = (tool: AgentId, capability: AgentCapability) =>
  (AGENT_CAPABILITIES[tool] as readonly AgentCapability[]).includes(capability);
const migrationTargets = AGENT_IDS.filter((tool) =>
  supportsAgentCapability(tool, "migration-target"),
);
const migrationSources = AGENT_IDS.filter((tool) =>
  supportsAgentCapability(tool, "migration-source"),
);
const promptAgents = AGENT_IDS.filter((tool) =>
  supportsAgentCapability(tool, "prompt"),
);

function supportedSessionEditOperations(tool: AgentId): string[] {
  return AGENT_EDIT_OPERATIONS[tool].filter((operation) =>
    exposedSessionEditOperations.has(operation),
  );
}

function validateSessionEditOperations(input: Record<string, unknown>): void {
  const tool = input.tool;
  if (
    typeof tool !== "string" ||
    !(AGENT_IDS as readonly string[]).includes(tool)
  ) {
    throw new Error(
      `session_edit content ops require a known source tool: ${AGENT_IDS.join(", ")}`,
    );
  }
  if (!Array.isArray(input.ops)) {
    throw new Error("session_edit ops must be an array");
  }

  const requested = input.ops.map((operation) => {
    if (
      operation === null ||
      typeof operation !== "object" ||
      !("op" in operation) ||
      typeof operation.op !== "string"
    ) {
      throw new Error("session_edit ops must contain operation names");
    }
    return operation.op;
  });
  const supported = supportedSessionEditOperations(tool as AgentId);
  const unsupported = [
    ...new Set(requested.filter((operation) => !supported.includes(operation))),
  ];
  if (unsupported.length > 0) {
    throw new Error(
      `session_edit ${tool} does not support content operations: ${unsupported.join(", ")}; supported: ${supported.join(", ") || "none"}`,
    );
  }
}

const sessionEditSupportDescription = AGENT_IDS.filter((tool) =>
  supportsAgentCapability(tool, "edit"),
)
  .map((tool) => `${tool}: ${supportedSessionEditOperations(tool).join(", ")}`)
  .join("; ");

export const FERRY_TOOL_NAMES = [
  "session_search",
  "session_read",
  "usage",
  "migrate",
  "session_edit",
  "session_cleanup",
  "ask_user",
  "agent_prompt",
  "bash",
] as const;

export type FerryToolName = (typeof FERRY_TOOL_NAMES)[number];

export interface ToolRequestContext {
  sessionId: string;
  runId: string;
  toolCallId: string;
  applyPolicy?: "manual" | "auto";
  signal?: AbortSignal;
  onUpdate: (payload: unknown) => void;
}

interface FerryToolPort {
  invoke(
    name: FerryToolName,
    args: Record<string, unknown>,
    context: ToolRequestContext,
  ): Promise<unknown>;
}

/**
 * 会话优化等受限用途的工具策略。字段全是字面量:策略不做配置组合,
 * 只表达"启用了优化约束"这一件事,具体规则在执行边界内实现。
 */
export interface FerryToolPolicy {
  sessionEdit?: {
    allowedOperations: readonly ["rewrite"];
    requireReadUserLocator: true;
    requireMatchingPreview: true;
  };
}

/** 批次指纹:tool + ref + 按原顺序的 (locator, text)。任何变化都要求重新 preview。 */
function sessionEditBatchFingerprint(input: Record<string, unknown>): string {
  const ops = (input.ops as Array<Record<string, unknown>>).map((operation) => [
    operation.locator,
    operation.text,
  ]);
  return JSON.stringify([input.tool, input.ref, ops]);
}

/** 从 session_read 成功结果里收集可改写的用户消息 locator。 */
function collectEditableUserLocators(
  details: unknown,
  target: Set<string>,
): void {
  if (details === null || typeof details !== "object") return;
  const result = details as Record<string, unknown>;
  for (const key of ["messages", "matches"]) {
    const entries = result[key];
    if (!Array.isArray(entries)) continue;
    for (const entry of entries) {
      if (entry === null || typeof entry !== "object") continue;
      const item = entry as Record<string, unknown>;
      if (
        item.role === "user" &&
        item.editable === true &&
        typeof item.locator === "string" &&
        item.locator.startsWith("fml_")
      ) {
        target.add(item.locator);
      }
    }
  }
}

function enforceSessionEditPolicy(
  input: Record<string, unknown>,
  readUserLocators: ReadonlySet<string>,
  previewedBatches: ReadonlySet<string>,
): void {
  if (input.patch !== undefined) {
    throw new Error(
      "session optimization does not allow metadata patch; only rewrite ops on user messages are permitted",
    );
  }
  const ops = input.ops as Array<Record<string, unknown>>;
  for (const operation of ops) {
    if (operation.op !== "rewrite") {
      throw new Error(
        `session optimization only allows rewrite ops; got ${String(operation.op)}`,
      );
    }
    if (
      typeof operation.locator !== "string" ||
      !readUserLocators.has(operation.locator)
    ) {
      throw new Error(
        "session optimization can only rewrite user messages this session has read: " +
          "call session_read first and copy an editable user message locator exactly",
      );
    }
  }
  if (
    input.intent === "execute" &&
    !previewedBatches.has(sessionEditBatchFingerprint(input))
  ) {
    throw new Error(
      "session optimization requires a successful preview of this exact batch before execute; " +
        'run session_edit with intent "preview" first and keep the ops identical',
    );
  }
}

function sessionCleanupBatchFingerprint(input: Record<string, unknown>): string {
  const targets = (input.targets as Array<Record<string, unknown>>)
    .map((target) => [target.tool, target.ref])
    .sort(([leftTool, leftRef], [rightTool, rightRef]) => {
      const left = `${String(leftTool)}\u0000${String(leftRef)}`;
      const right = `${String(rightTool)}\u0000${String(rightRef)}`;
      return left < right ? -1 : left > right ? 1 : 0;
    });
  return JSON.stringify([input.scope_id, targets]);
}

function validateSessionCleanupInput(input: Record<string, unknown>): void {
  const intent = input.intent;
  const has = (field: string) => input[field] !== undefined;
  if (
    intent !== "inventory" &&
    intent !== "triage" &&
    intent !== "preview" &&
    intent !== "execute"
  ) {
    throw new Error("session_cleanup requires intent inventory, triage, preview or execute");
  }
  if (intent === "inventory") {
    if (has("scope_id") || has("verdicts") || has("targets")) {
      throw new Error("session_cleanup inventory only accepts scope and cursor");
    }
    return;
  }
  if (intent === "triage") {
    if (
      typeof input.scope_id !== "string" ||
      !/^[0-9a-f]{16}$/.test(input.scope_id) ||
      !Array.isArray(input.verdicts) ||
      input.verdicts.length < 1 ||
      input.verdicts.length > 100 ||
      has("scope") ||
      has("cursor") ||
      has("targets")
    ) {
      throw new Error(
        "session_cleanup triage requires scope_id and 1-100 verdicts only",
      );
    }
    return;
  }
  if (
    typeof input.scope_id !== "string" ||
    !/^[0-9a-f]{16}$/.test(input.scope_id) ||
    !Array.isArray(input.targets) ||
    input.targets.length < 1 ||
    input.targets.length > 500 ||
    has("scope") ||
    has("cursor") ||
    has("verdicts")
  ) {
    throw new Error(
      `session_cleanup ${intent} requires scope_id and 1-500 targets only`,
    );
  }
}

function validateAskUserInput(input: Record<string, unknown>): void {
  if (
    typeof input.question !== "string" ||
    input.question.length < 1 ||
    input.question.length > 500 ||
    !Array.isArray(input.options) ||
    input.options.length < 2 ||
    input.options.length > 6
  ) {
    throw new Error("ask_user requires a question and 2-6 options");
  }
  const labels = new Set<string>();
  for (const option of input.options) {
    if (
      option === null ||
      typeof option !== "object" ||
      typeof option.label !== "string" ||
      option.label.length < 1 ||
      option.label.length > 80 ||
      labels.has(option.label) ||
      (option.description !== undefined &&
        (typeof option.description !== "string" ||
          option.description.length > 200)) ||
      (option.recommended !== undefined &&
        typeof option.recommended !== "boolean")
    ) {
      throw new Error("ask_user options must have unique bounded labels");
    }
    labels.add(option.label);
  }
  for (const field of ["multi_select", "allow_custom"]) {
    if (input[field] !== undefined && typeof input[field] !== "boolean") {
      throw new Error(`ask_user ${field} must be boolean`);
    }
  }
}

const schemas = {
  session_search: Type.Object(
    {
      query: Type.Optional(Type.String({ minLength: 1, maxLength: 500 })),
      patterns: Type.Optional(
        Type.Array(Type.String({ minLength: 1, maxLength: 500 }), {
          maxItems: 16,
        }),
      ),
      regex: Type.Optional(Type.String({ minLength: 1, maxLength: 500 })),
      exhaustive: Type.Optional(Type.Boolean()),
      agents: Type.Optional(
        Type.Array(Type.String({ maxLength: 32 }), { maxItems: 8 }),
      ),
      projects: Type.Optional(
        Type.Array(Type.String({ maxLength: 256 }), { maxItems: 20 }),
      ),
      time_range: Type.Optional(timeRange),
      limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 50 })),
      scope: Type.Optional(
        Type.Union([
          Type.Literal("any"),
          Type.Literal("metadata"),
          Type.Literal("content"),
        ]),
      ),
      include_tool_outputs: Type.Optional(Type.Boolean()),
    },
    { additionalProperties: false },
  ),
  session_read: Type.Object(
    {
      tool: Type.String({ minLength: 1, maxLength: 32 }),
      ref: opaqueSessionRef,
      terms: Type.Optional(
        Type.Array(Type.String({ minLength: 1, maxLength: 100 }), {
          minItems: 1,
          maxItems: 20,
        }),
      ),
      roles: Type.Optional(
        Type.Array(
          Type.Union([Type.Literal("user"), Type.Literal("assistant")]),
          { minItems: 1, maxItems: 2 },
        ),
      ),
      from_message: Type.Optional(Type.Integer({ minimum: 1 })),
      limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 50 })),
      include_tool_outputs: Type.Optional(Type.Boolean()),
      max_bytes: Type.Optional(
        Type.Integer({ minimum: 1024, maximum: 65_536 }),
      ),
    },
    { additionalProperties: false },
  ),
  usage: Type.Object(
    {
      agents: Type.Optional(
        Type.Array(Type.String({ maxLength: 32 }), { maxItems: 8 }),
      ),
      projects: Type.Optional(
        Type.Array(Type.String({ maxLength: 256 }), { maxItems: 20 }),
      ),
      time_range: Type.Optional(timeRange),
    },
    { additionalProperties: false },
  ),
  migrate: Type.Object(
    {
      source_tool: agentEnum(migrationSources),
      ref: opaqueSessionRef,
      target_tool: agentEnum(migrationTargets),
      max_turn: Type.Optional(Type.Integer({ minimum: 1 })),
      intent: operationIntent,
    },
    { additionalProperties: false },
  ),
  // Function-tool providers require an object root. Conditional constraints
  // live inside oneOf while the execution boundary validates them again.
  session_edit: sessionEditSchema,
  session_cleanup: sessionCleanupSchema,
  ask_user: askUserSchema,
  agent_prompt: Type.Object(
    {
      tool: agentEnum(promptAgents),
      ref: opaqueSessionRef,
      prompt: Type.String({ minLength: 1, maxLength: 100_000 }),
      model: Type.Optional(Type.String({ minLength: 1, maxLength: 512 })),
      timeout_sec: Type.Optional(Type.Integer({ minimum: 1, maximum: 360 })),
    },
    { additionalProperties: false },
  ),
  bash: Type.Object(
    {
      command: Type.String({ minLength: 1, maxLength: 4_000 }),
      cwd: Type.Optional(Type.String({ maxLength: 1_024 })),
      timeout_ms: Type.Optional(
        Type.Integer({ minimum: 1_000, maximum: 120_000 }),
      ),
    },
    { additionalProperties: false },
  ),
} as const;

const descriptions: Record<FerryToolName, string> = {
  session_search:
    'Search the whole session library: metadata (title, project, source tool, model) and, by default, full-text message content across every session — use this instead of reading sessions one by one or shelling out to grep. scope narrows matching to metadata or content only; default any matches either. Query words are ANDed within one message (or one metadata row): every word must appear there, substring-matched, so prefer one or two distinctive words. Quote "a phrase" for exact adjacency. For OR — alternative wordings, or a set of independent patterns like leaked-credential prefixes (sk-ant, ghp_, AKIA, "BEGIN PRIVATE KEY", "password=") — pass patterns: an array of up to 16 strings matched as a union, so one call covers them all; a session matches if ANY pattern matches, and each pattern keeps the same AND-within-a-message/phrase rules as query. For shapes substring matching cannot express — token formats, numeric patterns, high-entropy secrets with no fixed prefix — pass regex: one Python-syntax regular expression (not combinable with query/patterns). Regex matches run against original transcripts, not the index, so they also see content beyond the per-message indexing cap; when the regex contains required literal fragments the index narrows which sessions get scanned, otherwise every session passing your filters is scanned newest-first within a time/byte budget. After a regex search check content_index.regex_scan: skipped_sessions with a skip_reason means the budget cut the scan short (narrow time_range or projects and retry), and a non-zero clipped_sessions_not_scanned means the literal prefilter excluded sessions whose indexed text was truncated — pass exhaustive: true to force scanning those too. Pass query, patterns, or regex (at least one is required). Do NOT space-separate alternatives inside a single query string expecting OR — that ANDs them and silently returns nothing. Content hits carry matched_in, content_match_count and content_matches (message/turn/role plus a size-bounded original snippet) — jump to a hit with session_read from_message. Coding sessions keep most substance (code, file contents, command output) inside tool calls, so pass include_tool_outputs true before concluding a term is absent from content. Check content_index in the result: when ready is false, pending_sessions are still being indexed and content results are partial — say so rather than presenting them as complete. Only the first 16KB per message is content-indexed for query/patterns; results from sessions where that cap dropped content carry partially_indexed_messages, and a lexical miss there is not proof of absence — escalate with regex (which scans originals) or session_read with terms. total_matches is how many sessions matched and returned is how many came back — when total_matches exceeds returned you have seen a sample, not the library, so never describe the result as the user\'s complete history. record_count counts raw transcript records and is larger than the message_count session_read reports for the same session. An fsr_ ref stops resolving once that session is written to again; if a read fails with reason session_changed, search again and use the fresh ref.',
  session_read:
    "Read one indexed session using an fsr_ ref returned by session_search. By default returns a size-bounded page of original messages; paginate with next_from_message, never turn numbers. Pass terms to search that session's content and get matching snippets; searched_scope tells you what was covered — by default only visible message text, and coding sessions keep most of their substance (code, file contents, command output) inside tool calls, so pass include_tool_outputs true before concluding a term is absent. Every returned message carries message_count, turn_count, an fml_ locator, and an editable flag; only editable=true messages may be rewritten, and locators must be copied exactly. message_count and turn_count differ, and both differ from search's record_count. If a search match has complete=false, re-read that message without terms before editing its full text.",
  usage:
    "Get aggregate usage: tokens and estimated cost overall, by_agent, by_model and by_project (each bucket keeps only the top spenders). cost is an estimate computed from public per-model prices, not a bill; models listed in unpriced_models had no price match and contribute tokens but no cost. Never invent amounts of your own — report these numbers or say they are unavailable.",
  migrate: `Migrate a session into another agent's format (targets: ${migrationTargets.join(", ")}). intent is required: use preview to inspect the impact without changing anything, or execute to create an approval-gated migration that writes an immutable copy in the target format once approved. source_tool and target_tool are agent names; ref is an fsr_ value.`,
  session_edit: `Edit one session in place. Pass ops to rewrite or delete message turns, OR patch to change metadata (rename, pin, archive, tags) — exactly one. Content ops available through this tool by source: ${sessionEditSupportDescription}. Content ops require intent: use preview to inspect the diff, or execute to create an approval-gated edit that rewrites the original after revision checks and a recovery snapshot (Auto mode applies synchronously). Metadata patch does not accept intent. For rewrite ops, copy an editable message's fml_ locator exactly from a recent session_read and batch all intended rewrites into one call. Use patch only when the user explicitly asks to rename, pin, archive, or tag a session.`,
  session_cleanup:
    "Clean up sessions through a mandatory workflow: first call intent inventory (paginate until next_cursor is null), then triage every row in the returned scope exactly once or in idempotent batches. Engine rejects preview/execute until covered equals total; use ask_user or verdict ask_user when uncertain. Call preview with the exact delete targets before execute. execute is the only deletion authorization point and may require approval; it returns recovery_ids for individually reversible deletions. Report the covered/total counts from Engine and never claim the scope was fully reviewed without those numbers. A changed scope_id or generation requires a fresh inventory.",
  ask_user:
    "Ask the user to choose among 2-6 options or provide custom text. This tool only collects information and does not authorize deletion or any other mutation. The user may not answer; when answered is false, do not assume a selection and continue safely.",
  agent_prompt: `Resume and actively drive a native Coding Agent session (${promptAgents.join(", ")}). This is a high-privilege mutation: the target Agent may run commands, use its configured tools, and modify the workspace and native session without a separate Ferry approval. Pass an fsr_ ref from session_search and the prompt to execute. The returned next_ref replaces the old ref after every started run; always use next_ref for the next call because the previous ref becomes stale. Calls execute sequentially and are never safe to retry automatically.`,
  bash: "Run a shell command on the user's machine. The command really executes; unless the session is in Auto mode every call needs the user's approval first, so keep commands single-purpose and explain destructive ones before proposing them. Returns exit_code, stdout and stderr; output over 64KB is truncated.",
};

export function createFerryTools(
  port: FerryToolPort,
  getContext: () => Omit<ToolRequestContext, "toolCallId" | "onUpdate">,
  allowedTools: readonly FerryToolName[] = FERRY_TOOL_NAMES,
  policy?: FerryToolPolicy,
): AgentTool[] {
  // 策略状态只活在这一次工厂调用里:Runtime 重启即清空,宁可重读也不复用过期凭据
  const readUserLocators = new Set<string>();
  const previewedBatches = new Set<string>();
  const previewedCleanupBatches = new Set<string>();
  return allowedTools.map((name) => ({
    name,
    label: name,
    description: descriptions[name],
    parameters: schemas[name],
    executionMode: "sequential",
    async execute(toolCallId, params, signal, onUpdate) {
      const input = params as Record<string, unknown>;
      if (name === "session_search") {
        const hasQuery =
          typeof input.query === "string" && input.query.trim().length > 0;
        const hasPatterns =
          Array.isArray(input.patterns) && input.patterns.length > 0;
        const hasRegex =
          typeof input.regex === "string" && input.regex.trim().length > 0;
        if (!hasQuery && !hasPatterns && !hasRegex)
          throw new Error("session_search requires query, patterns or regex");
      }
      if (
        name === "migrate" &&
        input.intent !== "preview" &&
        input.intent !== "execute"
      ) {
        throw new Error("migrate requires intent preview or execute");
      }
      if (name === "session_edit") {
        const hasOps = input.ops !== undefined;
        const hasPatch = input.patch !== undefined;
        if (hasOps === hasPatch) {
          throw new Error("session_edit requires exactly one of ops or patch");
        }
        if (hasOps && input.intent !== "preview" && input.intent !== "execute")
          throw new Error("session_edit ops require intent preview or execute");
        if (hasPatch && input.intent !== undefined)
          throw new Error("session_edit metadata patch does not accept intent");
        if (hasOps) validateSessionEditOperations(input);
        if (policy?.sessionEdit) {
          enforceSessionEditPolicy(input, readUserLocators, previewedBatches);
        }
      }
      if (name === "session_cleanup") {
        validateSessionCleanupInput(input);
        if (
          input.intent === "execute" &&
          !previewedCleanupBatches.has(sessionCleanupBatchFingerprint(input))
        ) {
          throw new Error(
            "session_cleanup execute requires a successful preview of this exact target set first; " +
              'run session_cleanup with intent "preview" before execute',
          );
        }
      }
      if (name === "ask_user") validateAskUserInput(input);
      if (name === "bash" && String(input.command ?? "").trim().length === 0) {
        throw new Error("bash requires a non-empty command");
      }
      const active = getContext();
      const details = await port.invoke(
        name,
        params as Record<string, unknown>,
        {
          ...active,
          ...(signal ? { signal } : {}),
          toolCallId,
          onUpdate(payload) {
            onUpdate?.({
              content: [{ type: "text", text: "Tool is still running" }],
              details: payload,
            });
          },
        },
      );
      if (policy?.sessionEdit) {
        if (name === "session_read") {
          collectEditableUserLocators(details, readUserLocators);
        }
        if (name === "session_edit" && input.intent === "preview") {
          previewedBatches.add(sessionEditBatchFingerprint(input));
        }
      }
      if (name === "session_cleanup" && input.intent === "preview") {
        previewedCleanupBatches.add(sessionCleanupBatchFingerprint(input));
      }
      return {
        content: [{ type: "text", text: JSON.stringify(details) }],
        details,
      };
    },
  })) as AgentTool[];
}
