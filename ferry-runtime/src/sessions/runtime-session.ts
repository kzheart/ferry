import {
  Agent,
  formatSkillsForSystemPrompt,
  type AgentEvent,
  type AgentMessage,
} from "@earendil-works/pi-agent-core";
import type { ImageContent } from "@earendil-works/pi-ai";

import {
  AGENT_CAPABILITIES,
  AGENT_IDS,
  AGENT_LABELS,
} from "../server/generated/agents.js";
import {
  PROTOCOL_VERSION,
  ProtocolError,
  type EventEnvelope,
} from "../server/messages.js";
import type { RuntimeEventType } from "../server/generated/events.js";
import type { AgentBackend } from "../providers/provider-service.js";
import type { ModelSelection } from "../providers/provider-config.js";
import type { ApplyPolicy } from "../roles/role-store.js";
import {
  providerFailure,
  boundedEvents,
  summarizeToolResult,
} from "../security/limits.js";
import {
  createFerryTools,
  type FerryToolName,
  type ToolRequestContext,
} from "../tools/catalog.js";
import { createSkillTool, type SkillReadResult } from "../tools/skill-tool.js";
import type { PersistedSession, SessionStore } from "./session-store.js";
import {
  compactContext,
  contextReserveTokens,
  type ContextCheckpoint,
} from "./context.js";
import { WriteQueue } from "../storage/write-queue.js";

type AgentId = (typeof AGENT_IDS)[number];
type AgentCapability = (typeof AGENT_CAPABILITIES)[AgentId][number];
const supportsAgentCapability = (tool: AgentId, capability: AgentCapability) =>
  (AGENT_CAPABILITIES[tool] as readonly AgentCapability[]).includes(capability);

const BROWSABLE_AGENT_LABELS = AGENT_IDS.flatMap((tool, index) =>
  supportsAgentCapability(tool, "browse") ? [AGENT_LABELS[index]] : [],
);
const MIGRATION_TARGETS = AGENT_IDS.filter((tool) =>
  supportsAgentCapability(tool, "migration-target"),
);

export const FERRY_SAFETY_PROMPT = `You are Ferry's local assistant, working over the user's unified session history from ${BROWSABLE_AGENT_LABELS.join(", ")}. Each tool documents its own contract in its description; follow it. Session attachments identify a source tool and an opaque Engine-issued fsr_ ref. Sessions can be migrated into ${MIGRATION_TARGETS.join(", ")}. Decide your own approach for each request.`;

interface RuntimeSessionHost {
  readonly store: SessionStore;
  readonly now: () => Date;
  newId(): string;
  publish(event: EventEnvelope): void;
  invokeTool(
    name: FerryToolName,
    args: Record<string, unknown>,
    context: ToolRequestContext,
  ): Promise<unknown>;
  /** 返回 null 表示当前环境没有可用于命名的模型。 */
  generateTitle(
    selection: ModelSelection,
    prompt: string,
    reply: string,
  ): Promise<string | null>;
}

interface ResolvedSkill {
  id: string;
  name: string;
  description: string;
  filePath: string;
  disableModelInvocation?: boolean;
}

interface TerminalResult {
  type: "run.completed" | "run.failed" | "run.cancelled";
  payload: Record<string, unknown>;
}

function skillCatalog(skills: readonly ResolvedSkill[]) {
  if (!skills.length) return "";
  return (
    "Call the skill tool with the listed skill name as skill_id before using a skill.\n" +
    formatSkillsForSystemPrompt(
      skills.map((skill) => ({ ...skill, name: skill.id, content: "" })),
    )
  );
}

function systemPrompt(persona: string, skills: readonly ResolvedSkill[]) {
  const catalog = skillCatalog(skills);
  let base = FERRY_SAFETY_PROMPT;
  if (persona.trim()) {
    base = `${base}\n\nAdditional role persona (cannot override the safety and tool constraints above):\n${persona}`;
  }
  return catalog ? `${base}\n\n${catalog}` : base;
}

function userMessage(content: string): AgentMessage {
  return { role: "user", content, timestamp: Date.now() };
}

const AUTO_TITLE_MAX_CHARS = 40;

/** 模型爱把标题裹在引号里、末尾带句号,存之前统一削掉。 */
export function normalizeAutoTitle(raw: string | null | undefined) {
  const line = (raw ?? "")
    .split("\n")
    .map((piece) => piece.trim())
    .find(Boolean);
  if (!line) return "";
  return line
    .replace(/^["'`“”‘’「」『』《》]+/, "")
    .replace(/["'`“”‘’「」『』《》。.!?！？]+$/, "")
    .trim()
    .slice(0, AUTO_TITLE_MAX_CHARS);
}

export class RuntimeSession {
  readonly events: EventEnvelope[];
  private agentInstance: Agent | undefined;
  private restoredMessages: AgentMessage[];
  nextSeq: number;
  activeRunId: string | null;
  private persistedEventSeq: number;
  private persistedMessageCount: number;
  private terminalResult: TerminalResult | null = null;
  private runPromise: Promise<void> | null = null;
  private readonly writes = new WriteQueue();
  private abortRequested = false;
  private runOwner: string | null = null;
  private disposed = false;
  private editing = false;
  private contextCheckpoint: ContextCheckpoint | undefined;
  private containsImages: boolean;
  private title: string | null;
  private titleLocked: boolean;
  private pinned: boolean;

  constructor(
    readonly id: string,
    state: PersistedSession | undefined,
    events: EventEnvelope[],
    private readonly runtime: RuntimeSessionHost,
    private backend: AgentBackend | (() => AgentBackend),
    private selection: ModelSelection,
    private readonly roleId: string,
    private readonly resolvedPersona: string,
    private readonly resolvedTools: FerryToolName[],
    private readonly resolvedApplyPolicy: ApplyPolicy,
    private readonly resolvedSkills: ResolvedSkill[] = [],
    private readonly readSkill?: (id: string) => Promise<SkillReadResult>,
  ) {
    this.events = events;
    this.nextSeq = state?.next_seq ?? 1;
    this.persistedEventSeq = events.at(-1)?.seq ?? 0;
    this.persistedMessageCount = state?.messages.length ?? 0;
    this.containsImages = state?.contains_images ?? false;
    this.title = state?.title ?? null;
    this.titleLocked = state?.title_locked ?? false;
    this.pinned = state?.pinned ?? false;
    this.activeRunId = null;
    this.restoredMessages = state?.messages ?? [];
    this.contextCheckpoint = state?.context_checkpoint;
  }

  get agent(): Agent {
    if (this.agentInstance) return this.agentInstance;
    const backend =
      typeof this.backend === "function" ? this.backend() : this.backend;
    const agent = new Agent({
      sessionId: this.id,
      streamFn: (model, context, options) => {
        const current =
          typeof this.backend === "function" ? this.backend() : this.backend;
        return current.streamFn(model, context, {
          ...options,
          timeoutMs: 120_000,
          maxRetries: 2,
          maxTokens: Math.min(model.maxTokens, contextReserveTokens(model)),
        });
      },
      transformContext: async (messages, signal) => {
        const current =
          typeof this.backend === "function" ? this.backend() : this.backend;
        if (!current.models) return messages;
        const transformed = await compactContext(
          messages,
          this.contextCheckpoint,
          current.models,
          current.model,
          agent.state.systemPrompt,
          signal,
        );
        if (transformed.checkpoint) {
          this.contextCheckpoint = transformed.checkpoint;
          await this.emit("context.compacted", {
            message_count: messages.length,
          });
        }
        return transformed.messages;
      },
      toolExecution: "parallel",
      steeringMode: "one-at-a-time",
      followUpMode: "one-at-a-time",
      initialState: {
        systemPrompt: systemPrompt(this.resolvedPersona, this.resolvedSkills),
        model: backend.model,
        thinkingLevel: this.selection.thinking ?? "off",
        tools: [
          ...createFerryTools(
            {
              invoke: (name, args, context) =>
                this.runtime.invokeTool(name, args, context),
            },
            () => {
              if (!this.activeRunId) {
                throw new ProtocolError(
                  "no_active_run",
                  "tool call has no active run",
                );
              }
              return {
                sessionId: this.id,
                runId: this.activeRunId,
                applyPolicy: this.resolvedApplyPolicy,
              };
            },
            this.resolvedTools,
          ),
          ...(this.resolvedSkills.length > 0 && this.readSkill
            ? [
                createSkillTool(
                  this.readSkill,
                  this.resolvedSkills.map((skill) => skill.id),
                ),
              ]
            : []),
        ],
        messages: this.restoredMessages,
      },
    });
    agent.subscribe((event) => this.onAgentEvent(event));
    this.agentInstance = agent;
    return agent;
  }

  private get messages() {
    return this.agentInstance?.state.messages ?? this.restoredMessages;
  }

  get isRunning() {
    return this.runOwner !== null || this.editing;
  }

  async emit(
    type: RuntimeEventType,
    payload: Record<string, unknown>,
    runId = this.activeRunId,
    beforePublish?: () => void,
  ) {
    const event: EventEnvelope = {
      protocol: PROTOCOL_VERSION,
      session_id: this.id,
      run_id: runId,
      seq: this.nextSeq++,
      timestamp: this.runtime.now().toISOString(),
      type,
      payload,
    };
    this.events.push(event);
    await this.persistIfCommittable(type);
    beforePublish?.();
    this.runtime.publish(event);
    return event;
  }

  /**
   * 流式期间每个 token 都提交一次太贵:`lastCommittableEventSeq()` 本就把结尾
   * 的 delta 排除在提交内容外,所以一轮里除了第一个 delta(它要把用户那条消息
   * 落盘)之外,其余 delta 的提交全是零内容空写。只在真有未落盘内容时才写。
   */
  private async persistIfCommittable(type: RuntimeEventType) {
    if (
      (type === "content.delta" || type === "content.thinking") &&
      !this.hasUncommittedContent()
    )
      return;
    await this.persist();
  }

  private hasUncommittedContent() {
    const lastMessage = this.messages.at(-1);
    const committableMessageCount =
      lastMessage?.role === "assistant"
        ? this.messages.length - 1
        : this.messages.length;
    return (
      committableMessageCount > this.persistedMessageCount ||
      this.lastCommittableEventSeq() > this.persistedEventSeq
    );
  }

  async prompt(text: string, images: ImageContent[] = [], displayText = text) {
    this.assertAvailable();
    if (this.isRunning) {
      throw new ProtocolError(
        "run_in_progress",
        "session already has an active run",
      );
    }
    const runId = this.runtime.newId();
    this.activeRunId = runId;
    this.runOwner = runId;
    this.abortRequested = false;
    this.terminalResult = null;
    if (images.length > 0) this.containsImages = true;
    try {
      await this.emit(
        "run.started",
        {
          prompt: displayText,
          image_count: images.length,
          message_ordinal: this.messages.length,
        },
        runId,
      );
    } catch (error) {
      this.activeRunId = null;
      this.runOwner = null;
      throw error;
    }
    const task = this.run(runId, text, images);
    this.runPromise = task;
    // The request has already returned its run id; observe asynchronous storage failures.
    void task.catch((error) =>
      console.error("runtime run failed", this.id, error),
    );
    return runId;
  }

  private async run(runId: string, text: string, images: ImageContent[]) {
    try {
      try {
        if (!this.abortRequested) await this.agent.prompt(text, images);
      } catch (error) {
        this.terminalResult = {
          type: "run.failed",
          payload: { message: providerFailure(error) },
        };
      }
      const terminal = this.abortRequested
        ? { type: "run.cancelled" as const, payload: {} }
        : (this.terminalResult ?? {
            type: "run.failed" as const,
            payload: { message: "agent ended without a terminal result" },
          });
      this.activeRunId = null;
      this.agentInstance?.clearAllQueues();
      await this.emit(terminal.type, terminal.payload, runId, () => {
        this.runOwner = null;
        this.runPromise = null;
      });
      if (terminal.type === "run.completed") await this.autoTitle();
    } finally {
      if (this.runOwner === runId) {
        this.agentInstance?.clearAllQueues();
        this.activeRunId = null;
        this.runOwner = null;
        this.runPromise = null;
      }
    }
  }

  /**
   * 编辑历史用户消息并从那一点重发:丢弃该消息(含)之后的全部事件与
   * agent 消息,持久层同步删除后走正常 prompt。seq 指向该用户消息对应的
   * run.started 事件；运行中追加的 steer/follow-up 消息不支持编辑。
   */
  async editResend(seq: number, text: string, displayText = text) {
    this.assertAvailable();
    if (this.isRunning) {
      throw new ProtocolError(
        "run_in_progress",
        "session already has an active run",
      );
    }
    const index = this.events.findIndex(
      (event) => event.seq === seq && event.type === "run.started",
    );
    if (index < 0) {
      throw new ProtocolError(
        "invalid_params",
        "seq does not reference a user message",
      );
    }
    const ordinal = this.events[index]!.payload.message_ordinal;
    if (
      !Number.isInteger(ordinal) ||
      typeof ordinal !== "number" ||
      ordinal < 0 ||
      ordinal > this.messages.length
    ) {
      throw new ProtocolError(
        "invalid_params",
        "message cannot be edited without a history position",
      );
    }
    this.editing = true;
    try {
      await this.writes.run(() =>
        this.runtime.store.truncate(this.id, ordinal, seq),
      );
      this.events.length = index;
      this.nextSeq = seq;
      this.contextCheckpoint = undefined;
      this.restoredMessages = this.messages.slice(0, ordinal);
      if (this.agentInstance)
        this.agentInstance.state.messages = this.restoredMessages;
      this.containsImages = this.restoredMessages.some(
        (message) =>
          (message.role === "user" || message.role === "toolResult") &&
          Array.isArray(message.content) &&
          message.content.some((part) => part.type === "image"),
      );
      this.persistedEventSeq = Math.min(
        this.persistedEventSeq,
        this.events.at(-1)?.seq ?? 0,
      );
      this.persistedMessageCount = Math.min(
        this.persistedMessageCount,
        ordinal,
      );
    } finally {
      this.editing = false;
    }
    return this.prompt(text, [], displayText);
  }

  abort() {
    if (!this.isRunning) {
      throw new ProtocolError("no_active_run", "session has no active run");
    }
    this.abortRequested = true;
    this.agentInstance?.clearAllQueues();
    this.agentInstance?.abort();
  }

  steer(text: string, displayText = text) {
    if (!this.isRunning) {
      throw new ProtocolError("no_active_run", "session has no active run");
    }
    this.agent.steer(userMessage(text));
    void this.emit("user.message", { text: displayText, kind: "steer" });
  }

  followUp(text: string, displayText = text) {
    if (!this.isRunning) {
      throw new ProtocolError("no_active_run", "session has no active run");
    }
    this.agent.followUp(userMessage(text));
    void this.emit("user.message", { text: displayText, kind: "follow_up" });
  }

  async waitForIdle() {
    await this.runPromise;
  }

  finalText() {
    const message = [...this.messages]
      .reverse()
      .find((item) => item.role === "assistant");
    if (!message || message.role !== "assistant") return "";
    return message.content
      .filter((part) => part.type === "text")
      .map((part) => part.text)
      .join("")
      .trim();
  }

  state() {
    return {
      session_id: this.id,
      provider_id: this.selection.provider,
      model_id: this.selection.model,
      status: this.isRunning ? "running" : "idle",
      active_run_id: this.activeRunId,
      latest_seq: this.nextSeq - 1,
      queued_messages: this.agentInstance?.hasQueuedMessages() ?? false,
      contains_images: this.containsImages,
      title: this.title,
      title_locked: this.titleLocked,
      pinned: this.pinned,
      thinking_level: this.selection.thinking ?? "off",
      role_id: this.roleId,
      apply_policy: this.resolvedApplyPolicy,
    };
  }

  summary() {
    return {
      ...this.state(),
      created_at: this.events[0]?.timestamp ?? null,
      updated_at: this.events.at(-1)?.timestamp ?? null,
    };
  }

  async selectModel(selection: ModelSelection, backend: AgentBackend) {
    this.assertAvailable();
    if (this.isRunning) {
      throw new ProtocolError(
        "run_in_progress",
        "cannot change model while a run is active",
      );
    }
    if (this.containsImages && !backend.model.input.includes("image")) {
      throw new ProtocolError(
        "model_capability_mismatch",
        "the conversation contains images but the target model does not support image input",
      );
    }
    this.backend = backend;
    if (this.agentInstance) {
      this.agentInstance.state.model = backend.model;
      this.agentInstance.state.thinkingLevel = selection.thinking ?? "off";
    }
    this.selection = selection;
    await this.persist();
    await this.emit("session.model_changed", {
      provider_id: selection.provider,
      model_id: selection.model,
      thinking_level: selection.thinking ?? "off",
    });
    return this.state();
  }

  async rename(title: string) {
    this.assertAvailable();
    const next = title.trim();
    if (!next || next.length > 200) {
      throw new ProtocolError(
        "invalid_params",
        "title must be 1 to 200 characters",
      );
    }
    this.title = next;
    this.titleLocked = true;
    await this.persist();
    await this.emit("session.renamed", {
      session_id: this.id,
      title: next,
      auto: false,
    });
    return this.summary();
  }

  /**
   * 首轮跑完后让模型补一个短标题。只在会话还没有标题、且用户没手动改过名时发生;
   * 生成失败一律静默放弃——命名是锦上添花,不该把对话本身拖失败。
   */
  private async autoTitle() {
    if (this.titleLocked || this.title) return;
    const opening = this.events.find((event) => event.type === "run.started");
    const prompt =
      typeof opening?.payload.prompt === "string" ? opening.payload.prompt : "";
    if (!prompt.trim()) return;
    let generated: string | null = null;
    try {
      generated = await this.runtime.generateTitle(
        this.selection,
        prompt,
        this.finalText(),
      );
    } catch {
      return;
    }
    const title = normalizeAutoTitle(generated);
    // 生成期间用户可能已经手动改名了,再查一次锁
    if (!title || this.titleLocked || this.title || this.disposed) return;
    this.title = title;
    await this.persist();
    await this.emit(
      "session.renamed",
      { session_id: this.id, title, auto: true },
      null,
    );
  }

  async dispose() {
    this.disposed = true;
    await this.writes.settled();
  }

  cancelDisposal() {
    this.disposed = false;
  }

  private assertAvailable() {
    if (this.disposed)
      throw new ProtocolError("session_not_found", "session was deleted");
  }

  async pin(pinned: boolean) {
    this.assertAvailable();
    this.pinned = pinned;
    await this.persist();
    return this.summary();
  }

  private async onAgentEvent(event: AgentEvent) {
    switch (event.type) {
      case "message_update": {
        const update = event.assistantMessageEvent;
        if (update.type === "text_delta" || update.type === "thinking_delta") {
          await this.emit(
            update.type === "text_delta" ? "content.delta" : "content.thinking",
            { delta: update.delta },
          );
        }
        break;
      }
      case "message_end":
        if (event.message.role === "assistant") {
          await this.emit("run.usage", { usage: event.message.usage });
        }
        break;
      case "tool_execution_start":
        await this.emit("tool.started", {
          tool_call_id: event.toolCallId,
          name: event.toolName,
          args: event.args as unknown,
        });
        break;
      case "tool_execution_update":
        await this.emit("tool.progress", {
          tool_call_id: event.toolCallId,
          name: event.toolName,
          partial: event.partialResult as unknown,
        });
        break;
      case "tool_execution_end":
        await this.emit("tool.completed", {
          tool_call_id: event.toolCallId,
          name: event.toolName,
          is_error: event.isError,
          result: summarizeToolResult(event.result),
        });
        break;
      case "agent_end": {
        const final = [...event.messages]
          .reverse()
          .find((message) => message.role === "assistant");
        if (final?.role === "assistant" && final.stopReason === "aborted") {
          this.terminalResult = { type: "run.cancelled", payload: {} };
        } else if (
          final?.role === "assistant" &&
          final.stopReason === "error"
        ) {
          this.terminalResult = {
            type: "run.failed",
            payload: { message: providerFailure(final.errorMessage) },
          };
        } else {
          this.terminalResult = { type: "run.completed", payload: {} };
        }
        break;
      }
    }
  }

  private persist() {
    return this.writes.run(() => this.commit());
  }

  private async commit() {
    const lastMessage = this.messages.at(-1);
    const committableMessageCount =
      this.activeRunId &&
      ["content.delta", "content.thinking"].includes(
        this.events.at(-1)?.type ?? "",
      ) &&
      lastMessage?.role === "assistant"
        ? this.messages.length - 1
        : this.messages.length;
    const committableEventSeq = this.activeRunId
      ? this.lastCommittableEventSeq()
      : (this.events.at(-1)?.seq ?? 0);
    const messages = this.messages
      .slice(this.persistedMessageCount, committableMessageCount)
      .map((message, offset) => ({
        ordinal: this.persistedMessageCount + offset,
        message,
      }));
    const events = boundedEvents(
      this.events.filter(
        (event) =>
          event.seq > this.persistedEventSeq &&
          event.seq <= committableEventSeq,
      ),
    );
    await this.runtime.store.commit({
      metadata: {
        ...(this.contextCheckpoint
          ? { context_checkpoint: this.contextCheckpoint }
          : {}),
        session_id: this.id,
        provider_id: this.selection.provider,
        model_id: this.selection.model,
        contains_images: this.containsImages,
        next_seq: this.nextSeq,
        status: this.activeRunId ? "running" : "idle",
        active_run_id: this.activeRunId,
        title: this.title,
        title_locked: this.titleLocked,
        pinned: this.pinned,
        thinking_level: this.selection.thinking ?? "off",
        role_id: this.roleId,
        resolved_persona: this.resolvedPersona,
        resolved_tools: [...this.resolvedTools],
        resolved_apply_policy: this.resolvedApplyPolicy,
        resolved_skills: this.resolvedSkills.map((skill) => skill.id),
      },
      messages,
      events,
      timestamp: this.runtime.now().toISOString(),
    });
    this.persistedMessageCount = committableMessageCount;
    this.persistedEventSeq = committableEventSeq;
  }

  private lastCommittableEventSeq() {
    let index = this.events.length - 1;
    while (
      index >= 0 &&
      ["content.delta", "content.thinking"].includes(this.events[index]!.type)
    ) {
      index -= 1;
    }
    return this.events[index]?.seq ?? 0;
  }
}
