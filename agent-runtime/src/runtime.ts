import { randomUUID } from "node:crypto";
import {
  Agent,
  type AgentEvent,
  type AgentMessage,
  type StreamFn,
} from "@earendil-works/pi-agent-core";
import type { ImageContent, Model } from "@earendil-works/pi-ai";
import type { AuthType } from "@earendil-works/pi-ai";
import { AuthCoordinator } from "./auth-coordinator.js";
import type { PersistedSession, SessionStore } from "./event-store.js";
import { MemorySessionStore } from "./event-store.js";
import type {
  CustomProviderConfig,
  ModelSelection,
  ThinkingLevel,
} from "./provider-config.js";
import type { ProviderHost } from "./provider-host.js";
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
  credentialAvailable?: () => boolean | Promise<boolean>;
}

export type BackendFactory = (selection?: ModelSelection) => AgentBackend;
export type ToolHandler = (
  name: FerryToolName,
  args: Record<string, unknown>,
  context: ToolRequestContext,
) => Promise<unknown>;

// 新增工具时 TypeScript 会强制补齐策略；长任务不能被短读取工具的截止时间误伤。
const TOOL_DEADLINES_MS: Record<FerryToolName, number> = {
  ferry_list_capabilities: 10_000,
  ferry_search_sessions: 25_000,
  ferry_get_session_context: 25_000,
  ferry_get_usage: 25_000,
  ferry_preview_migration: 125_000,
  ferry_preview_edit: 125_000,
  ferry_propose_migration: 125_000,
  ferry_propose_edit: 125_000,
  ferry_propose_metadata_change: 125_000,
};

export interface RuntimeOptions {
  store?: SessionStore;
  backendFactory?: BackendFactory;
  providerHost?: ProviderHost;
  toolHandler?: ToolHandler;
  now?: () => Date;
  idFactory?: () => string;
  toolDeadlinesMs?: Partial<Record<FerryToolName, number>>;
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

function safeText(value: string, limit: number): string {
  const redacted = value
    .replace(/\bBearer\s+[A-Za-z0-9._~+/=-]{8,}/gi, "[REDACTED]")
    .replace(/\bsk-[A-Za-z0-9_-]{12,}\b/g, "[REDACTED]")
    .replace(/\b(?:gh[opusr]|github_pat)_[A-Za-z0-9_]{16,}\b/g, "[REDACTED]")
    .replace(
      /\b[A-Z0-9_]*(?:API_KEY|TOKEN|SECRET|PASSWORD|PRIVATE_KEY)[A-Z0-9_]*\s*[:=]\s*[^\s,;]+/gi,
      "[REDACTED]",
    )
    .replace(
      /-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----[\s\S]*?-----END (?:[A-Z0-9]+ )*PRIVATE KEY-----/g,
      "[REDACTED]",
    )
    .replace(/\b[A-Z]:[\\/][^\s\]\[)(}{"']+/gi, "[ABSOLUTE_PATH]")
    .replace(/(?<![:\w])\/(?:[^/\s]+\/)*[^\s\]\[)(}{"']+/g, "[ABSOLUTE_PATH]");
  return redacted.length > limit ? `${redacted.slice(0, limit)}…` : redacted;
}

export function safeMessages(messages: AgentMessage[]): AgentMessage[] {
  return messages.map((message): AgentMessage => {
    if (message.role === "assistant") {
      return {
        ...message,
        ...(message.errorMessage
          ? { errorMessage: safeText(message.errorMessage, 1_000) }
          : {}),
        content: message.content
          .filter((part) => part.type !== "thinking")
          .map((part) =>
            part.type === "text"
              ? { ...part, text: safeText(part.text, 16_000) }
              : part.type === "toolCall"
                ? { ...part, arguments: { omitted: true } }
                : part,
          ),
      };
    }
    if (message.role === "user") {
      if (typeof message.content === "string") {
        return { ...message, content: safeText(message.content, 16_000) };
      }
      return {
        ...message,
        content: message.content.map((part) =>
          part.type === "image"
            ? {
                type: "text" as const,
                text: `[image omitted: ${part.mimeType}]`,
              }
            : part.type === "text"
              ? { ...part, text: safeText(part.text, 16_000) }
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
            : part.type === "text"
              ? { ...part, text: safeText(part.text, 4_000) }
              : part,
        ),
      };
    }
    return message;
  });
}

export function safeEvents(events: EventEnvelope[]): EventEnvelope[] {
  const safe = events.map((event) => {
    const payload = { ...event.payload };
    if (event.type === "tool.started" || event.type === "tool.request") {
      if ("args" in payload) payload.args = "[omitted]";
    }
    if (event.type === "tool.progress" && "partial" in payload) {
      payload.partial = "[omitted]";
    }
    if (typeof payload.message === "string") {
      payload.message = safeText(payload.message, 1_000);
    }
    if (typeof payload.prompt === "string") {
      payload.prompt = safeText(payload.prompt, 16_000);
    }
    if (typeof payload.text === "string") {
      payload.text = safeText(payload.text, 16_000);
    }
    return { ...event, payload };
  });
  // delta 是按网络到达顺序记录的事实；渲染层会把连续 delta 组成一块回复，
  // 但持久化层绝不能压扁它们，否则无法区分工具调用前后的两段 AI 回复。
  let index = 0;
  while (index < safe.length) {
    const first = safe[index]!;
    if (
      first.type !== "content.delta" ||
      typeof first.payload.delta !== "string"
    ) {
      index += 1;
      continue;
    }
    let end = index + 1;
    while (
      end < safe.length &&
      safe[end]!.type === "content.delta" &&
      safe[end]!.run_id === first.run_id &&
      typeof safe[end]!.payload.delta === "string"
    ) {
      end += 1;
    }
    const raw = safe
      .slice(index, end)
      .map((event) => event.payload.delta)
      .join("");
    const redacted = safeText(raw, 16_000);
    // 仅在脱敏或截断时把相邻片段收敛为一个安全值，避免跨分片泄露凭据。
    if (redacted !== raw) {
      first.payload.delta = redacted;
      for (let cursor = index + 1; cursor < end; cursor += 1) {
        safe[cursor]!.payload.delta = "";
      }
    }
    index = end;
  }
  return safe;
}

class RuntimeSession {
  readonly events: EventEnvelope[];
  readonly agent: Agent;
  nextSeq: number;
  activeRunId: string | null;
  private terminalResult: TerminalResult | null = null;
  private runPromise: Promise<void> | null = null;
  private containsImages: boolean;
  private title: string | null;
  private pinned: boolean;

  constructor(
    readonly id: string,
    state: PersistedSession | undefined,
    events: EventEnvelope[],
    private readonly runtime: AgentRuntime,
    backend: AgentBackend,
    private selection: ModelSelection,
  ) {
    this.events = events;
    this.nextSeq = state?.next_seq ?? 1;
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
        systemPrompt:
          "You are Ferry's local assistant. Use only the explicitly registered Ferry tools. Never claim shell, filesystem, or network access.",
        model: backend.model,
        thinkingLevel: selection.thinking ?? "off",
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

  async prompt(text: string, images: ImageContent[] = []) {
    if (this.isRunning)
      throw new ProtocolError(
        "run_in_progress",
        "session already has an active run",
      );
    const runId = this.runtime.newId();
    if (images.length > 0) this.containsImages = true;
    this.activeRunId = runId;
    this.terminalResult = null;
    await this.emit(
      "run.started",
      { prompt: text, image_count: images.length },
      runId,
    );
    let task!: Promise<void>;
    task = (async () => {
      try {
        await this.agent.prompt(text, images);
      } catch (error) {
        this.terminalResult = {
          type: "run.failed",
          payload: { message: "provider request failed" },
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
    void this.emit("user.message", { text, kind: "steer" });
  }

  followUp(text: string) {
    if (!this.isRunning)
      throw new ProtocolError("no_active_run", "session has no active run");
    this.agent.followUp(userMessage(text));
    void this.emit("user.message", { text, kind: "follow_up" });
  }

  async waitForIdle() {
    await this.runPromise;
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
            payload: { message: "provider request failed" },
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
        provider_id: this.selection.provider,
        model_id: this.selection.model,
        contains_images: this.containsImages,
        next_seq: this.nextSeq,
        status: this.activeRunId ? "running" : "idle",
        active_run_id: this.activeRunId,
        messages: safeMessages(this.agent.state.messages),
        title: this.title,
        pinned: this.pinned,
        thinking_level: this.selection.thinking ?? "off",
      },
      safeEvents(this.events),
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
  private readonly providerHost: ProviderHost | undefined;
  private readonly toolHandler: ToolHandler | undefined;
  private readonly idFactory: () => string;
  private readonly backendInfo: AgentBackend;
  private readonly auth: AuthCoordinator | undefined;
  private readonly toolDeadlinesMs: Record<FerryToolName, number>;
  private runtimeSequence = 1;

  private constructor(
    options: RuntimeOptions,
    backendFactory: BackendFactory,
    defaultSelection?: ModelSelection,
  ) {
    this.store = options.store ?? new MemorySessionStore();
    this.now = options.now ?? (() => new Date());
    this.idFactory = options.idFactory ?? randomUUID;
    this.toolDeadlinesMs = { ...TOOL_DEADLINES_MS, ...options.toolDeadlinesMs };
    this.toolHandler = options.toolHandler;
    this.providerHost = options.providerHost;
    this.backendFactory = backendFactory;
    this.backendInfo = this.backendFactory(defaultSelection);
    this.auth = this.providerHost
      ? new AuthCoordinator(
          (providerId, type, interaction) =>
            this.providerHost!.login(providerId, type, interaction),
          (event) =>
            this.publish({
              protocol: PROTOCOL_VERSION,
              session_id: "runtime",
              run_id: null,
              seq: this.runtimeSequence++,
              timestamp: this.now().toISOString(),
              type: event.type,
              payload: event.payload,
            }),
          this.idFactory,
        )
      : undefined;
  }

  static async create(options: RuntimeOptions = {}) {
    let defaultSelection: ModelSelection | undefined;
    if (options.providerHost) {
      defaultSelection = await options.providerHost.defaultModel();
    }
    const backendFactory =
      options.backendFactory ??
      ((selection?: ModelSelection) => {
        if (!options.providerHost || !selection) {
          throw new Error("provider host and model selection are required");
        }
        return options.providerHost.backend(selection);
      });
    const runtime = new AgentRuntime(options, backendFactory, defaultSelection);
    for (const record of await runtime.store.loadAll()) {
      const selection: ModelSelection = {
        provider: record.state.provider_id,
        model: record.state.model_id,
        ...(record.state.thinking_level
          ? { thinking: record.state.thinking_level as ThinkingLevel }
          : {}),
      };
      const session = new RuntimeSession(
        record.state.session_id,
        record.state,
        record.events,
        runtime,
        runtime.backendFactory(selection),
        selection,
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

  async providerStatus() {
    const selection = this.providerHost
      ? await this.providerHost.defaultModel()
      : {
          provider:
            this.backendInfo.provider ?? this.backendInfo.model.provider,
          model: this.backendInfo.modelId ?? this.backendInfo.model.id,
        };
    const configured = this.providerHost
      ? await this.providerHost.isConfigured(selection.provider)
      : await this.backendInfo.credentialAvailable?.();
    return {
      provider: selection.provider,
      model: selection.model,
      thinking: selection.thinking ?? "off",
      credential: configured ? "available" : "unavailable",
      provider_count: this.providerHost
        ? (await this.providerHost.providers()).length
        : 1,
    };
  }

  subscribe(listener: (event: EventEnvelope) => void) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  publish(event: EventEnvelope) {
    for (const listener of this.listeners) listener(event);
  }

  async createSession(requestedId?: string, requestedModel?: ModelSelection) {
    const id = requestedId ?? this.newId();
    if (this.sessions.has(id))
      throw new ProtocolError("session_exists", "session already exists");
    if (!/^[A-Za-z0-9_-]{1,128}$/.test(id))
      throw new ProtocolError("invalid_params", "invalid session_id");
    const selection = this.providerHost
      ? (requestedModel ?? (await this.providerHost.defaultModel()))
      : {
          provider:
            this.backendInfo.provider ?? this.backendInfo.model.provider,
          model: this.backendInfo.modelId ?? this.backendInfo.model.id,
        };
    const session = new RuntimeSession(
      id,
      undefined,
      [],
      this,
      this.backendFactory(selection),
      selection,
    );
    this.sessions.set(id, session);
    await session.emit("session.created", {
      provider_id: selection.provider,
      model_id: selection.model,
    });
    return session.state();
  }

  async prompt(sessionId: string, text: string, images: ImageContent[] = []) {
    const session = this.session(sessionId);
    const state = session.state();
    const configured = this.providerHost
      ? await this.providerHost.isConfigured(state.provider_id)
      : ((await this.backendInfo.credentialAvailable?.()) ?? true);
    if (!configured) {
      throw new ProtocolError(
        "provider_unavailable",
        `provider ${state.provider_id} is not configured`,
      );
    }
    if (
      images.length > 0 &&
      !session.agent.state.model.input.includes("image")
    ) {
      throw new ProtocolError(
        "model_capability_mismatch",
        "the current model does not support image input",
      );
    }
    return { run_id: await session.prompt(text, images) };
  }

  async renameSession(sessionId: string, title: string) {
    return this.session(sessionId).rename(title);
  }

  async pinSession(sessionId: string, pinned: boolean) {
    return this.session(sessionId).pin(pinned);
  }

  async deleteSession(sessionId: string) {
    const session = this.session(sessionId);
    if (session.isRunning) {
      throw new ProtocolError(
        "run_in_progress",
        "cannot delete a running session",
      );
    }
    this.sessions.delete(sessionId);
    await this.store.delete(sessionId);
    return { session_id: sessionId, deleted: true };
  }

  async providers() {
    if (!this.providerHost) return [];
    return this.providerHost.providers();
  }

  models(providerId: string, query = "", limit = 100) {
    if (!this.providerHost) return [];
    try {
      return this.providerHost.listModels(providerId, query, limit);
    } catch (error) {
      throw new ProtocolError(
        "provider_not_found",
        error instanceof Error ? error.message : "provider not found",
      );
    }
  }

  async enabledModels() {
    if (!this.providerHost) return [];
    return this.providerHost.enabledModels();
  }

  async catalogModels() {
    if (!this.providerHost) return [];
    return this.providerHost.catalogModels();
  }

  async testProvider(providerId: string, modelId?: string) {
    if (!this.providerHost) {
      throw new ProtocolError("unsupported", "provider config unavailable");
    }
    try {
      return await this.providerHost.testProvider(providerId, modelId);
    } catch (error) {
      throw new ProtocolError(
        "provider_unreachable",
        error instanceof Error ? error.message : "provider test failed",
      );
    }
  }

  async saveCustomModel(
    providerId: string,
    input: {
      id: string;
      name?: string;
      input?: Array<"text" | "image">;
      reasoning?: boolean;
      context_window?: number;
      max_tokens?: number;
    },
  ) {
    if (!this.providerHost) {
      throw new ProtocolError("unsupported", "provider config unavailable");
    }
    try {
      return await this.providerHost.saveCustomModel(providerId, input);
    } catch (error) {
      throw new ProtocolError(
        "invalid_params",
        error instanceof Error ? error.message : "custom model is invalid",
      );
    }
  }

  async deleteCustomModel(providerId: string, modelId: string) {
    if (!this.providerHost) {
      throw new ProtocolError("unsupported", "provider config unavailable");
    }
    try {
      return await this.providerHost.deleteCustomModel(providerId, modelId);
    } catch (error) {
      throw new ProtocolError(
        "provider_not_found",
        error instanceof Error ? error.message : "provider not found",
      );
    }
  }

  async setProviderEnabled(providerId: string, enabled: boolean) {
    if (!this.providerHost) {
      throw new ProtocolError("unsupported", "provider config unavailable");
    }
    try {
      return await this.providerHost.setProviderEnabled(providerId, enabled);
    } catch (error) {
      throw new ProtocolError(
        "provider_not_found",
        error instanceof Error ? error.message : "provider not found",
      );
    }
  }

  async setVisibleModels(providerId: string, modelIds: string[] | null) {
    if (!this.providerHost) {
      throw new ProtocolError("unsupported", "provider config unavailable");
    }
    try {
      return await this.providerHost.setVisibleModels(providerId, modelIds);
    } catch (error) {
      throw new ProtocolError(
        "model_not_found",
        error instanceof Error ? error.message : "model not found",
      );
    }
  }

  async refreshModels() {
    if (!this.providerHost)
      throw new ProtocolError("unsupported", "model refresh unavailable");
    return this.providerHost.refreshModels();
  }

  async config() {
    if (!this.providerHost)
      throw new ProtocolError("unsupported", "provider config unavailable");
    return this.providerHost.store.publicSnapshot();
  }

  async saveApiKey(
    providerId: string,
    key: string,
    fields?: Record<string, string>,
  ) {
    if (!this.providerHost)
      throw new ProtocolError("unsupported", "provider config unavailable");
    if (this.auth?.isProviderActive(providerId)) {
      throw new ProtocolError(
        "auth_in_progress",
        "provider authentication is in progress",
      );
    }
    try {
      await this.providerHost.saveApiKey(providerId, key, fields);
      return {
        provider_id: providerId,
        configured: true,
        credential_type: "api_key",
      };
    } catch (error) {
      throw new ProtocolError(
        "invalid_provider_config",
        error instanceof Error ? error.message : "provider config failed",
      );
    }
  }

  async logoutProvider(providerId: string) {
    if (!this.providerHost)
      throw new ProtocolError("unsupported", "provider config unavailable");
    if (this.auth?.isProviderActive(providerId)) {
      throw new ProtocolError(
        "auth_in_progress",
        "provider authentication is in progress",
      );
    }
    try {
      await this.providerHost.logout(providerId);
      return { provider_id: providerId, configured: false };
    } catch (error) {
      throw new ProtocolError(
        "provider_not_found",
        error instanceof Error ? error.message : "provider logout failed",
      );
    }
  }

  startAuthentication(providerId: string, type: AuthType) {
    if (!this.providerHost || !this.auth) {
      throw new ProtocolError(
        "unsupported",
        "provider authentication unavailable",
      );
    }
    const provider = this.providerHost.models.getProvider(providerId);
    const supported =
      type === "oauth" ? provider?.auth.oauth : provider?.auth.apiKey;
    if (!supported) {
      throw new ProtocolError(
        "auth_type_unsupported",
        `provider does not support ${type} authentication`,
      );
    }
    try {
      return this.auth.start(providerId, type);
    } catch (error) {
      throw new ProtocolError(
        "auth_in_progress",
        error instanceof Error
          ? error.message
          : "authentication is in progress",
      );
    }
  }

  respondAuthentication(loginId: string, promptId: string, value: string) {
    if (!this.auth)
      throw new ProtocolError("unsupported", "authentication unavailable");
    try {
      return this.auth.respond(loginId, promptId, value);
    } catch (error) {
      throw new ProtocolError(
        "auth_prompt_not_found",
        error instanceof Error
          ? error.message
          : "authentication prompt not found",
      );
    }
  }

  cancelAuthentication(loginId: string) {
    if (!this.auth)
      throw new ProtocolError("unsupported", "authentication unavailable");
    try {
      return this.auth.cancel(loginId);
    } catch (error) {
      throw new ProtocolError(
        "auth_login_not_found",
        error instanceof Error
          ? error.message
          : "authentication login not found",
      );
    }
  }

  async selectModel(sessionId: string | undefined, selection: ModelSelection) {
    if (!this.providerHost)
      throw new ProtocolError("unsupported", "model selection unavailable");
    let backend: AgentBackend;
    try {
      backend = this.providerHost.backend(selection);
    } catch (error) {
      throw new ProtocolError(
        "model_not_found",
        error instanceof Error ? error.message : "model not found",
      );
    }
    if (sessionId) {
      return this.session(sessionId).selectModel(selection, backend);
    }
    await this.providerHost.selectDefault(selection);
    return { ...selection };
  }

  async saveCustomProvider(config: CustomProviderConfig, clearApiKey = false) {
    if (!this.providerHost) {
      throw new ProtocolError("unsupported", "custom providers unavailable");
    }
    if (
      this.providerHost.isCustom(config.id) &&
      [...this.sessions.values()].some(
        (session) => session.state().provider_id === config.id,
      )
    ) {
      throw new ProtocolError(
        "provider_in_use",
        "custom provider is used by a session",
      );
    }
    try {
      await this.providerHost.saveCustomProvider(config, clearApiKey);
      return { provider_id: config.id, configured: true };
    } catch (error) {
      throw new ProtocolError(
        "invalid_provider_config",
        error instanceof Error ? error.message : "custom provider save failed",
      );
    }
  }

  async deleteCustomProvider(providerId: string) {
    if (!this.providerHost) {
      throw new ProtocolError("unsupported", "custom providers unavailable");
    }
    if (
      [...this.sessions.values()].some(
        (session) => session.state().provider_id === providerId,
      )
    ) {
      throw new ProtocolError(
        "provider_in_use",
        "custom provider is used by a session",
      );
    }
    try {
      await this.providerHost.deleteCustomProvider(providerId);
      return { provider_id: providerId, deleted: true };
    } catch (error) {
      throw new ProtocolError(
        "invalid_provider_config",
        error instanceof Error
          ? error.message
          : "custom provider delete failed",
      );
    }
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

  listSessions() {
    return [...this.sessions.values()]
      .map((session) => session.summary())
      .sort((left, right) =>
        String(right.updated_at ?? "").localeCompare(
          String(left.updated_at ?? ""),
        ),
      );
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
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const result = new Promise<unknown>((resolve, reject) => {
      const cleanup = () => {
        if (abortListener)
          context.signal?.removeEventListener("abort", abortListener);
        if (timeout) clearTimeout(timeout);
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
      if (context.signal?.aborted) {
        abortListener();
      } else {
        context.signal?.addEventListener("abort", abortListener, {
          once: true,
        });
        timeout = setTimeout(() => {
          if (!this.pendingTools.delete(requestId)) return;
          cleanup();
          reject(new Error("tool gateway timed out"));
        }, this.toolDeadlinesMs[name]);
      }
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
