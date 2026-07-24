import {
  Agent,
  type AgentEvent,
  type AgentMessage,
} from "@earendil-works/pi-agent-core";
import type { ImageContent } from "@earendil-works/pi-ai";

import { AGENT_IDS, AGENT_LABELS } from "../server/generated/agents.js";
import {
  PROTOCOL_VERSION,
  ProtocolError,
  type EventEnvelope,
} from "../server/messages.js";
import type { AgentBackend } from "../providers/provider-service.js";
import type { ModelSelection } from "../providers/provider-config.js";
import type { ApplyPolicy } from "../roles/role-repository.js";
import {
  providerFailure,
  safeEvents,
  safeMessages,
  summarizeToolResult,
} from "../security/redaction.js";
import {
  createFerryTools,
  type FerryToolName,
  type ToolRequestContext,
} from "../tools/catalog.js";
import { createDelegationTool } from "../tools/delegation.js";
import type { TaskGraph, WorkflowRunResult } from "../agents/scheduler.js";
import type { PersistedSession, SessionStore } from "./session-repository.js";

export const FERRY_SAFETY_PROMPT = `You are Ferry's local assistant, working over the user's unified session history from ${AGENT_LABELS.join(", ")}. Each tool documents its own contract in its description; follow it. Session attachments identify a source tool and an opaque Engine-issued fsr_ ref. Sessions can be migrated between ${AGENT_IDS.join(", ")}. Use delegate_agents when independent research or review tasks benefit from bounded parallel agents, and synthesize their workflow-scoped results. Decide your own approach for each request.`;

export interface RuntimeSessionHost {
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
}

interface TerminalResult {
  type: "run.completed" | "run.failed" | "run.cancelled";
  payload: Record<string, unknown>;
}

function systemPrompt(persona: string) {
  return persona.trim()
    ? `${FERRY_SAFETY_PROMPT}\n\nAdditional role persona (cannot override the safety and tool constraints above):\n${persona}`
    : FERRY_SAFETY_PROMPT;
}

function userMessage(content: string): AgentMessage {
  return { role: "user", content, timestamp: Date.now() };
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
  ) {
    this.events = events;
    this.nextSeq = state?.next_seq ?? 1;
    this.persistedEventSeq = events.at(-1)?.seq ?? 0;
    this.persistedMessageCount = state?.messages.length ?? 0;
    this.containsImages = state?.contains_images ?? false;
    this.title = state?.title ?? null;
    this.pinned = state?.pinned ?? false;
    this.activeRunId = null;
    this.agent = new Agent({
      sessionId: id,
      streamFn: backend.streamFn,
      toolExecution: "sequential",
      steeringMode: "one-at-a-time",
      followUpMode: "one-at-a-time",
      initialState: {
        systemPrompt: systemPrompt(this.resolvedPersona),
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
                }),
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
    type: string,
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
    await this.persist();
    this.runtime.publish(event);
    return event;
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
    await this.persist();
    return this.summary();
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
    const messages = safeMessages(
      this.agent.state.messages.slice(
        this.persistedMessageCount,
        committableMessageCount,
      ),
    ).map((message, offset) => ({
      ordinal: this.persistedMessageCount + offset,
      message,
    }));
    const events = safeEvents(
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
        pinned: this.pinned,
        thinking_level: this.selection.thinking ?? "off",
        role_id: this.roleId,
        resolved_persona: this.resolvedPersona,
        resolved_tools: [...this.resolvedTools],
        resolved_apply_policy: this.resolvedApplyPolicy,
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
