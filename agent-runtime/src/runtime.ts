import { randomUUID } from "node:crypto";
import {
  Agent,
  type AgentEvent,
  type AgentMessage,
  type StreamFn,
} from "@earendil-works/pi-agent-core";
import type { Model } from "@earendil-works/pi-ai";
import {
  createDeepSeekBackend,
  DEEPSEEK_API_KEY_ENV,
} from "./deepseek-provider.js";
import type { PersistedSession, SessionStore } from "./event-store.js";
import { MemorySessionStore } from "./event-store.js";
import {
  PROTOCOL_VERSION,
  ProtocolError,
  type EventEnvelope,
} from "./protocol.js";
import {
  createFerryTools,
  type FerryToolName,
  type ToolRequestContext,
} from "./tool-port.js";

export interface AgentBackend {
  model: Model<string>;
  streamFn: StreamFn;
  provider?: string;
  modelId?: string;
  credentialAvailable?: () => boolean;
}

export type BackendFactory = () => AgentBackend;
export type ToolHandler = (
  name: FerryToolName,
  args: Record<string, unknown>,
  context: ToolRequestContext,
) => Promise<unknown>;

export interface RuntimeOptions {
  store?: SessionStore;
  backendFactory?: BackendFactory;
  toolHandler?: ToolHandler;
  now?: () => Date;
  idFactory?: () => string;
}

interface DeferredTool {
  sessionId: string;
  resolve(value: unknown): void;
  reject(error: Error): void;
  cleanup(): void;
}

interface TerminalResult {
  type: "run.completed" | "run.failed" | "run.cancelled";
  payload: Record<string, unknown>;
}

function userMessage(content: string): AgentMessage {
  return { role: "user", content, timestamp: Date.now() };
}

function safeMessages(messages: AgentMessage[]): AgentMessage[] {
  return messages.map((message): AgentMessage => {
    if (message.role === "assistant") {
      return {
        ...message,
        content: message.content.filter((part) => part.type !== "thinking"),
      };
    }
    if (message.role === "user" && Array.isArray(message.content)) {
      return {
        ...message,
        content: message.content.map((part) =>
          part.type === "image"
            ? {
                type: "text" as const,
                text: `[image omitted: ${part.mimeType}]`,
              }
            : part,
        ),
      };
    }
    if (message.role === "toolResult") {
      return {
        ...message,
        details: undefined,
        content: message.content.map((part) =>
          part.type === "image"
            ? {
                type: "text" as const,
                text: `[image omitted: ${part.mimeType}]`,
              }
            : part,
        ),
      };
    }
    return message;
  });
}

class RuntimeSession {
  readonly events: EventEnvelope[];
  readonly agent: Agent;
  nextSeq: number;
  activeRunId: string | null;
  private terminalResult: TerminalResult | null = null;
  private runPromise: Promise<void> | null = null;

  constructor(
    readonly id: string,
    state: PersistedSession | undefined,
    events: EventEnvelope[],
    private readonly runtime: AgentRuntime,
    backend: AgentBackend,
  ) {
    this.events = events;
    this.nextSeq = state?.next_seq ?? 1;
    this.activeRunId = null;
    this.agent = new Agent({
      sessionId: id,
      streamFn: backend.streamFn,
      toolExecution: "sequential",
      steeringMode: "one-at-a-time",
      followUpMode: "one-at-a-time",
      initialState: {
        systemPrompt:
          "You are Ferry's local assistant. Use only the explicitly registered Ferry tools. Never claim shell, filesystem, or network access.",
        model: backend.model,
        thinkingLevel: "off",
        tools: createFerryTools(
          {
            invoke: (name, args, context) =>
              this.runtime.invokeTool(name, args, context),
          },
          () => {
            if (!this.activeRunId)
              throw new ProtocolError(
                "no_active_run",
                "tool call has no active run",
              );
            return { sessionId: this.id, runId: this.activeRunId };
          },
        ),
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

  async prompt(text: string) {
    if (this.isRunning)
      throw new ProtocolError(
        "run_in_progress",
        "session already has an active run",
      );
    const runId = this.runtime.newId();
    this.activeRunId = runId;
    this.terminalResult = null;
    await this.emit("run.started", {}, runId);
    let task!: Promise<void>;
    task = (async () => {
      try {
        await this.agent.prompt(text);
      } catch (error) {
        this.terminalResult = {
          type: "run.failed",
          payload: {
            message: error instanceof Error ? error.message : "unknown failure",
          },
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
    if (!this.isRunning)
      throw new ProtocolError("no_active_run", "session has no active run");
    this.agent.abort();
  }

  steer(text: string) {
    if (!this.isRunning)
      throw new ProtocolError("no_active_run", "session has no active run");
    this.agent.steer(userMessage(text));
  }

  followUp(text: string) {
    if (!this.isRunning)
      throw new ProtocolError("no_active_run", "session has no active run");
    this.agent.followUp(userMessage(text));
  }

  async waitForIdle() {
    await this.runPromise;
  }

  state() {
    return {
      session_id: this.id,
      status: this.isRunning ? "running" : "idle",
      active_run_id: this.activeRunId,
      latest_seq: this.nextSeq - 1,
      queued_messages: this.agent.hasQueuedMessages(),
    };
  }

  private async onAgentEvent(event: AgentEvent) {
    switch (event.type) {
      case "message_update": {
        const update = event.assistantMessageEvent;
        if (update.type === "text_delta")
          await this.emit("content.delta", { delta: update.delta });
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
            payload: { message: final.errorMessage ?? "provider error" },
          };
        } else {
          this.terminalResult = { type: "run.completed", payload: {} };
        }
        break;
      }
    }
  }

  private persist() {
    return this.runtime.store.save(
      {
        session_id: this.id,
        next_seq: this.nextSeq,
        status: this.activeRunId ? "running" : "idle",
        active_run_id: this.activeRunId,
        messages: safeMessages(this.agent.state.messages),
      },
      this.events,
    );
  }
}

export class AgentRuntime {
  readonly store: SessionStore;
  readonly now: () => Date;
  private readonly sessions = new Map<string, RuntimeSession>();
  private readonly listeners = new Set<(event: EventEnvelope) => void>();
  private readonly pendingTools = new Map<string, DeferredTool>();
  private readonly backendFactory: BackendFactory;
  private readonly toolHandler: ToolHandler | undefined;
  private readonly idFactory: () => string;
  private readonly backendInfo: AgentBackend;

  private constructor(options: RuntimeOptions) {
    this.store = options.store ?? new MemorySessionStore();
    this.now = options.now ?? (() => new Date());
    this.idFactory = options.idFactory ?? randomUUID;
    this.toolHandler = options.toolHandler;
    this.backendFactory = options.backendFactory ?? createDeepSeekBackend;
    this.backendInfo = this.backendFactory();
  }

  static async create(options: RuntimeOptions = {}) {
    const runtime = new AgentRuntime(options);
    for (const record of await runtime.store.loadAll()) {
      const session = new RuntimeSession(
        record.state.session_id,
        record.state,
        record.events,
        runtime,
        runtime.backendFactory(),
      );
      runtime.sessions.set(session.id, session);
      if (record.state.status === "running" && record.state.active_run_id) {
        await session.emit(
          "run.interrupted",
          { reason: "runtime_restarted" },
          record.state.active_run_id,
        );
      }
    }
    return runtime;
  }

  newId() {
    return this.idFactory();
  }

  providerStatus() {
    return {
      provider: this.backendInfo.provider ?? this.backendInfo.model.provider,
      model: this.backendInfo.modelId ?? this.backendInfo.model.id,
      credential: this.backendInfo.credentialAvailable?.()
        ? "available"
        : "unavailable",
      credential_env: DEEPSEEK_API_KEY_ENV,
    };
  }

  subscribe(listener: (event: EventEnvelope) => void) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  publish(event: EventEnvelope) {
    for (const listener of this.listeners) listener(event);
  }

  async createSession(requestedId?: string) {
    const id = requestedId ?? this.newId();
    if (this.sessions.has(id))
      throw new ProtocolError("session_exists", "session already exists");
    if (!/^[A-Za-z0-9_-]{1,128}$/.test(id))
      throw new ProtocolError("invalid_params", "invalid session_id");
    const session = new RuntimeSession(
      id,
      undefined,
      [],
      this,
      this.backendFactory(),
    );
    this.sessions.set(id, session);
    await session.emit("session.created", {});
    return session.state();
  }

  async prompt(sessionId: string, text: string) {
    if (this.backendInfo.credentialAvailable?.() === false) {
      throw new ProtocolError(
        "provider_unavailable",
        `${DEEPSEEK_API_KEY_ENV} is not configured for deepseek-v4-flash`,
      );
    }
    return { run_id: await this.session(sessionId).prompt(text) };
  }

  abort(sessionId: string) {
    this.session(sessionId).abort();
    return { accepted: true };
  }

  steer(sessionId: string, text: string) {
    this.session(sessionId).steer(text);
    return { accepted: true };
  }

  followUp(sessionId: string, text: string) {
    this.session(sessionId).followUp(text);
    return { accepted: true };
  }

  state(sessionId: string) {
    return this.session(sessionId).state();
  }

  replay(sessionId: string, afterSeq: number) {
    return this.session(sessionId).events.filter(
      (event) => event.seq > afterSeq,
    );
  }

  waitForIdle(sessionId: string) {
    return this.session(sessionId).waitForIdle();
  }

  async invokeTool(
    name: FerryToolName,
    args: Record<string, unknown>,
    context: ToolRequestContext,
  ) {
    if (this.toolHandler) return this.toolHandler(name, args, context);
    const requestId = this.newId();
    let abortListener: (() => void) | undefined;
    const result = new Promise<unknown>((resolve, reject) => {
      const cleanup = () => {
        if (abortListener)
          context.signal?.removeEventListener("abort", abortListener);
      };
      const deferred: DeferredTool = {
        sessionId: context.sessionId,
        resolve,
        reject,
        cleanup,
      };
      this.pendingTools.set(requestId, deferred);
      abortListener = () => {
        this.pendingTools.delete(requestId);
        cleanup();
        reject(new Error("tool request aborted"));
      };
      if (context.signal?.aborted) abortListener();
      else
        context.signal?.addEventListener("abort", abortListener, {
          once: true,
        });
    });
    if (!this.pendingTools.has(requestId)) return result;
    try {
      await this.session(context.sessionId).emit(
        "tool.request",
        {
          request_id: requestId,
          tool_call_id: context.toolCallId,
          name,
          args,
        },
        context.runId,
      );
    } catch (error) {
      const pending = this.pendingTools.get(requestId);
      this.pendingTools.delete(requestId);
      pending?.cleanup();
      pending?.reject(
        error instanceof Error ? error : new Error("tool request failed"),
      );
    }
    return result;
  }

  completeTool(
    requestId: string,
    sessionId: string,
    ok: boolean,
    value: unknown,
  ) {
    const pending = this.pendingTools.get(requestId);
    if (!pending || pending.sessionId !== sessionId) {
      throw new ProtocolError("unknown_tool_request", "tool request not found");
    }
    this.pendingTools.delete(requestId);
    pending.cleanup();
    if (ok) pending.resolve(value);
    else
      pending.reject(
        new Error(
          typeof value === "string" ? value : "tool gateway rejected request",
        ),
      );
    return { accepted: true };
  }

  private session(id: string) {
    const session = this.sessions.get(id);
    if (!session)
      throw new ProtocolError("session_not_found", "session not found");
    return session;
  }
}
