import {
  Agent,
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
  boundedMessages,
  summarizeToolResult,
} from "../security/limits.js";
import {
  createFerryTools,
  type FerryToolName,
  type ToolRequestContext,
} from "../tools/catalog.js";
import { createDelegationTool } from "../tools/delegation.js";
import { createSkillTool, type SkillReadResult } from "../tools/skill-tool.js";
import type { TaskGraph, WorkflowRunResult } from "../agents/scheduler.js";
import type { PersistedSession, SessionStore } from "./session-store.js";

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

export const FERRY_SAFETY_PROMPT = `You are Ferry's local assistant, working over the user's unified session history from ${BROWSABLE_AGENT_LABELS.join(", ")}. Each tool documents its own contract in its description; follow it. Session attachments identify a source tool and an opaque Engine-issued fsr_ ref. Sessions can be migrated into ${MIGRATION_TARGETS.join(", ")}. Use delegate_agents when independent research or review tasks benefit from bounded parallel agents, and synthesize their workflow-scoped results. Decide your own approach for each request.`;

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
  runDelegatedWorkflow(
    parent: RuntimeSession,
    parentRunId: string,
    spec: TaskGraph,
    onUpdate: (payload: unknown) => void,
    signal?: AbortSignal,
  ): Promise<WorkflowRunResult>;
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
}

interface TerminalResult {
  type: "run.completed" | "run.failed" | "run.cancelled";
  payload: Record<string, unknown>;
}

/** 系统提示里只放名称与说明,正文由模型自己调 skill 工具取——几十个技能全量注入会吃掉几万 token。 */
const SKILL_CATALOG_MAX_BYTES = 8 * 1024;

function skillCatalog(skills: readonly ResolvedSkill[]) {
  if (skills.length === 0) return "";
  const header =
    "Available skills. Call the skill tool with a skill id before acting on a task that matches one of these:";
  const lines: string[] = [];
  let bytes = Buffer.byteLength(header);
  let dropped = 0;
  for (const skill of skills) {
    const line = `- ${skill.id} · ${skill.name}：${skill.description}`;
    const size = Buffer.byteLength(line) + 1;
    if (bytes + size > SKILL_CATALOG_MAX_BYTES) {
      dropped += 1;
      continue;
    }
    bytes += size;
    lines.push(line);
  }
  if (dropped > 0) {
    lines.push(`- (${dropped} more skills omitted; ask the user to trim them)`);
  }
  return `${header}\n${lines.join("\n")}`;
}

function systemPrompt(persona: string, skills: readonly ResolvedSkill[]) {
  const catalog = skillCatalog(skills);
  const base = persona.trim()
    ? `${FERRY_SAFETY_PROMPT}\n\nAdditional role persona (cannot override the safety and tool constraints above):\n${persona}`
    : FERRY_SAFETY_PROMPT;
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
  readonly agent: Agent;
  nextSeq: number;
  activeRunId: string | null;
  private persistedEventSeq: number;
  private persistedMessageCount: number;
  private terminalResult: TerminalResult | null = null;
  private runPromise: Promise<void> | null = null;
  private containsImages: boolean;
  private title: string | null;
  private titleLocked: boolean;
  private pinned: boolean;

  constructor(
    readonly id: string,
    state: PersistedSession | undefined,
    events: EventEnvelope[],
    private readonly runtime: RuntimeSessionHost,
    backend: AgentBackend,
    private selection: ModelSelection,
    private readonly roleId: string,
    private readonly resolvedPersona: string,
    private readonly resolvedTools: FerryToolName[],
    private readonly resolvedApplyPolicy: ApplyPolicy,
    private readonly canDelegate: boolean,
    private readonly resolvedSkills: ResolvedSkill[] = [],
    readSkill?: (id: string) => Promise<SkillReadResult>,
    delegatableRoleIds: readonly string[] = [],
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
    this.agent = new Agent({
      sessionId: id,
      streamFn: backend.streamFn,
      toolExecution: "sequential",
      steeringMode: "one-at-a-time",
      followUpMode: "one-at-a-time",
      initialState: {
        systemPrompt: systemPrompt(this.resolvedPersona, this.resolvedSkills),
        model: backend.model,
        thinkingLevel: selection.thinking ?? "off",
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
          ...(this.resolvedSkills.length > 0 && readSkill
            ? [
                createSkillTool(
                  readSkill,
                  this.resolvedSkills.map((skill) => skill.id),
                ),
              ]
            : []),
          ...(this.canDelegate
            ? [
                createDelegationTool((spec, onUpdate, signal) => {
                  if (!this.activeRunId) {
                    throw new ProtocolError(
                      "no_active_run",
                      "delegation has no active run",
                    );
                  }
                  return this.runtime.runDelegatedWorkflow(
                    this,
                    this.activeRunId,
                    spec,
                    onUpdate,
                    signal,
                  );
                }, delegatableRoleIds),
              ]
            : []),
        ],
        messages: state?.messages ?? [],
      },
    });
    this.agent.subscribe((event) => this.onAgentEvent(event));
  }

  get isRunning() {
    return this.runPromise !== null;
  }

  async emit(
    type: RuntimeEventType,
    payload: Record<string, unknown>,
    runId = this.activeRunId,
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
    this.runtime.publish(event);
    return event;
  }

  /**
   * 流式期间每个 token 都提交一次太贵:`lastCommittableEventSeq()` 本就把结尾
   * 的 delta 排除在提交内容外,所以一轮里除了第一个 delta(它要把用户那条消息
   * 落盘)之外,其余 delta 的提交全是零内容空写。只在真有未落盘内容时才写。
   */
  private async persistIfCommittable(type: RuntimeEventType) {
    if (type === "content.delta" && !this.hasUncommittedContent()) return;
    await this.persist();
  }

  private hasUncommittedContent() {
    const lastMessage = this.agent.state.messages.at(-1);
    const committableMessageCount =
      lastMessage?.role === "assistant"
        ? this.agent.state.messages.length - 1
        : this.agent.state.messages.length;
    return (
      committableMessageCount > this.persistedMessageCount ||
      this.lastCommittableEventSeq() > this.persistedEventSeq
    );
  }

  async prompt(text: string, images: ImageContent[] = [], displayText = text) {
    if (this.isRunning) {
      throw new ProtocolError(
        "run_in_progress",
        "session already has an active run",
      );
    }
    const runId = this.runtime.newId();
    if (images.length > 0) this.containsImages = true;
    this.activeRunId = runId;
    this.terminalResult = null;
    await this.emit(
      "run.started",
      { prompt: displayText, image_count: images.length },
      runId,
    );
    let task!: Promise<void>;
    task = (async () => {
      try {
        await this.agent.prompt(text, images);
      } catch (error) {
        this.terminalResult = {
          type: "run.failed",
          payload: { message: providerFailure(error) },
        };
      }
      const terminal = this.terminalResult ?? {
        type: "run.failed" as const,
        payload: { message: "agent ended without a terminal result" },
      };
      this.activeRunId = null;
      await this.persist();
      if (this.runPromise === task) this.runPromise = null;
      await this.emit(terminal.type, terminal.payload, runId);
      if (terminal.type === "run.completed") await this.autoTitle();
    })();
    this.runPromise = task;
    return runId;
  }

  abort() {
    if (!this.isRunning) {
      throw new ProtocolError("no_active_run", "session has no active run");
    }
    this.agent.abort();
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
    const message = [...this.agent.state.messages]
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
      queued_messages: this.agent.hasQueuedMessages(),
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
    this.agent.streamFunction = backend.streamFn;
    this.agent.state.model = backend.model;
    this.agent.state.thinkingLevel = selection.thinking ?? "off";
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
    if (!title || this.titleLocked || this.title) return;
    this.title = title;
    await this.persist();
    await this.emit(
      "session.renamed",
      { session_id: this.id, title, auto: true },
      null,
    );
  }

  async pin(pinned: boolean) {
    this.pinned = pinned;
    await this.persist();
    return this.summary();
  }

  private async onAgentEvent(event: AgentEvent) {
    switch (event.type) {
      case "message_update": {
        const update = event.assistantMessageEvent;
        if (update.type === "text_delta") {
          await this.emit("content.delta", { delta: update.delta });
        }
        break;
      }
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

  private async persist() {
    const lastMessage = this.agent.state.messages.at(-1);
    const committableMessageCount =
      this.activeRunId &&
      this.events.at(-1)?.type === "content.delta" &&
      lastMessage?.role === "assistant"
        ? this.agent.state.messages.length - 1
        : this.agent.state.messages.length;
    const committableEventSeq = this.activeRunId
      ? this.lastCommittableEventSeq()
      : (this.events.at(-1)?.seq ?? 0);
    const messages = boundedMessages(
      this.agent.state.messages.slice(
        this.persistedMessageCount,
        committableMessageCount,
      ),
    ).map((message, offset) => ({
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
    while (index >= 0 && this.events[index]!.type === "content.delta") {
      index -= 1;
    }
    return this.events[index]?.seq ?? 0;
  }
}
