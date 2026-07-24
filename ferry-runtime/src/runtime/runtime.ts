import { randomUUID } from "node:crypto";
import { AGENT_IDS, AGENT_LABELS } from "../server/generated/agents.js";
import {
  Agent,
  type AgentEvent,
  type AgentMessage,
  type StreamFn,
} from "@earendil-works/pi-agent-core";
import type { ImageContent, Model } from "@earendil-works/pi-ai";
import type { AuthType } from "@earendil-works/pi-ai";
import { AuthCoordinator } from "../providers/auth-coordinator.js";
import { createDelegationTool } from "../tools/delegation.js";
import type {
  PersistedSession,
  SessionStore,
} from "../sessions/session-repository.js";
import { EphemeralSessionStore } from "../sessions/session-repository.js";
import type {
  CustomProviderConfig,
  ModelSelection,
  ThinkingLevel,
} from "../providers/provider-config.js";
import type { ProviderHost } from "../providers/provider-host.js";
import { parseOrganizerInput } from "../organizing/organizer.js";
import {
  runOrganizationWorkflow,
  type OrganizationEngineMethod,
} from "../organizing/organization.js";
import {
  DEFAULT_ROLE_ID,
  EphemeralRoleStore,
  type ApplyPolicy,
  type RoleInput,
  type RoleStore,
} from "../roles/role-repository.js";
import {
  PROTOCOL_VERSION,
  ProtocolError,
  type EventEnvelope,
} from "../server/messages.js";
import {
  createFerryTools,
  FERRY_TOOL_NAMES,
  type FerryToolName,
  type ToolRequestContext,
} from "../tools/catalog.js";
import {
  WorkflowRun,
  type TaskGraph,
  type WorkflowRunEvent,
} from "../agents/scheduler.js";
import {
  providerFailure,
  safeEvents,
  safeMessages,
  summarizeToolResult,
} from "../security/redaction.js";

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
export type EngineHandler = (
  method: OrganizationEngineMethod,
  params: Record<string, unknown>,
  workflowId: string,
) => Promise<unknown>;

// 新增工具时 TypeScript 会强制补齐策略；长任务不能被短读取工具的截止时间误伤。
const TOOL_DEADLINES_MS: Record<FerryToolName, number> = {
  session_search: 25_000,
  session_read: 25_000,
  usage: 25_000,
  migrate: 125_000,
  session_edit: 125_000,
};

export interface RuntimeOptions {
  store?: SessionStore;
  storeFactory?: (
    invoke: import("../sessions/engine-store.js").RuntimeEngineInvoke,
  ) => SessionStore;
  deferRestore?: boolean;
  backendFactory?: BackendFactory;
  providerHost?: ProviderHost;
  roleStore?: RoleStore;
  toolHandler?: ToolHandler;
  engineHandler?: EngineHandler;
  now?: () => Date;
  idFactory?: () => string;
  toolDeadlinesMs?: Partial<Record<FerryToolName, number>>;
}

export const FERRY_SAFETY_PROMPT = `You are Ferry's local assistant, working over the user's unified session history from ${AGENT_LABELS.join(", ")}. Each tool documents its own contract in its description; follow it. Session attachments identify a source tool and an opaque Engine-issued fsr_ ref. Sessions can be migrated between ${AGENT_IDS.join(", ")}. Use delegate_agents when independent research or review tasks benefit from bounded parallel agents, and synthesize their workflow-scoped results. Decide your own approach for each request.`;

function systemPrompt(persona: string) {
  return persona.trim()
    ? `${FERRY_SAFETY_PROMPT}\n\nAdditional role persona (cannot override the safety and tool constraints above):\n${persona}`
    : FERRY_SAFETY_PROMPT;
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

class RuntimeSession {
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
    private readonly runtime: AgentRuntime,
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
              if (!this.activeRunId)
                throw new ProtocolError(
                  "no_active_run",
                  "tool call has no active run",
                );
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
    if (!this.isRunning)
      throw new ProtocolError("no_active_run", "session has no active run");
    this.agent.abort();
  }

  steer(text: string, displayText = text) {
    if (!this.isRunning)
      throw new ProtocolError("no_active_run", "session has no active run");
    this.agent.steer(userMessage(text));
    void this.emit("user.message", { text: displayText, kind: "steer" });
  }

  followUp(text: string, displayText = text) {
    if (!this.isRunning)
      throw new ProtocolError("no_active_run", "session has no active run");
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

export class AgentRuntime {
  store: SessionStore;
  readonly roleStore: RoleStore;
  readonly now: () => Date;
  private readonly sessions = new Map<string, RuntimeSession>();
  private readonly listeners = new Set<(event: EventEnvelope) => void>();
  private readonly pendingTools = new Map<string, DeferredTool>();
  private readonly organizationRuns = new Map<string, Promise<unknown>>();
  private readonly backendFactory: BackendFactory;
  private readonly providerHost: ProviderHost | undefined;
  private readonly toolHandler: ToolHandler | undefined;
  private readonly engineHandler: EngineHandler | undefined;
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
    this.store = options.store ?? new EphemeralSessionStore();
    this.roleStore = options.roleStore ?? new EphemeralRoleStore();
    this.now = options.now ?? (() => new Date());
    this.idFactory = options.idFactory ?? randomUUID;
    this.toolDeadlinesMs = { ...TOOL_DEADLINES_MS, ...options.toolDeadlinesMs };
    this.toolHandler = options.toolHandler;
    this.engineHandler = options.engineHandler;
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
    if (options.storeFactory) {
      runtime.store = options.storeFactory((method, params, sessionId) =>
        runtime.invokeInternalEngine(method, params, sessionId),
      );
    }
    if (!options.deferRestore) await runtime.restore();
    return runtime;
  }

  async restore() {
    if (this.sessions.size > 0) {
      throw new ProtocolError(
        "already_restored",
        "runtime sessions already restored",
      );
    }
    for (const record of await this.store.loadAll()) {
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
        this,
        this.backendFactory(selection),
        selection,
        record.state.role_id ?? DEFAULT_ROLE_ID,
        record.state.resolved_persona ?? "",
        (record.state.resolved_tools ?? FERRY_TOOL_NAMES).filter(
          (name): name is FerryToolName =>
            (FERRY_TOOL_NAMES as readonly string[]).includes(name),
        ),
        record.state.resolved_apply_policy ?? "auto",
        true,
      );
      this.sessions.set(session.id, session);
      if (record.state.status === "running" && record.state.active_run_id) {
        await session.emit(
          "run.interrupted",
          { reason: "runtime_restarted" },
          record.state.active_run_id,
        );
      }
    }
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

  async createSession(
    requestedId?: string,
    requestedModel?: ModelSelection,
    requestedRoleId = DEFAULT_ROLE_ID,
    canDelegate = true,
    toolOverride?: FerryToolName[],
  ) {
    const id = requestedId ?? this.newId();
    if (this.sessions.has(id))
      throw new ProtocolError("session_exists", "session already exists");
    if (!/^[A-Za-z0-9_-]{1,128}$/.test(id))
      throw new ProtocolError("invalid_params", "invalid session_id");
    const role = await this.roleStore.get(requestedRoleId);
    if (!role) throw new ProtocolError("role_not_found", "role not found");
    const fallbackSelection = this.providerHost
      ? await this.providerHost.defaultModel()
      : {
          provider:
            this.backendInfo.provider ?? this.backendInfo.model.provider,
          model: this.backendInfo.modelId ?? this.backendInfo.model.id,
        };
    const baseSelection = requestedModel ?? role.model ?? fallbackSelection;
    const thinking =
      requestedModel?.thinking ??
      role.thinking ??
      role.model?.thinking ??
      fallbackSelection.thinking;
    const selection: ModelSelection = {
      provider: baseSelection.provider,
      model: baseSelection.model,
      ...(thinking ? { thinking } : {}),
    };
    const session = new RuntimeSession(
      id,
      undefined,
      [],
      this,
      this.backendFactory(selection),
      selection,
      role.id,
      role.persona,
      toolOverride ?? [...role.tools],
      role.apply_policy,
      canDelegate,
    );
    this.sessions.set(id, session);
    await session.emit("session.created", {
      provider_id: selection.provider,
      model_id: selection.model,
      role_id: role.id,
    });
    return session.state();
  }

  async prompt(
    sessionId: string,
    text: string,
    images: ImageContent[] = [],
    displayText = text,
  ) {
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
    return { run_id: await session.prompt(text, images, displayText) };
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

  roles() {
    return this.roleStore.list();
  }

  async createRole(input: RoleInput) {
    return this.mutateRole(() => this.roleStore.create(input));
  }

  async updateRole(id: string, input: RoleInput) {
    return this.mutateRole(() => this.roleStore.update(id, input));
  }

  async deleteRole(id: string) {
    await this.mutateRole(() => this.roleStore.delete(id));
    return { role_id: id, deleted: true };
  }

  async startOrganization(input: unknown) {
    if (!this.providerHost) {
      throw new ProtocolError(
        "unsupported",
        "organization generation unavailable",
      );
    }
    let key: string;
    try {
      key = JSON.stringify(input);
    } catch {
      throw new ProtocolError(
        "invalid_params",
        "organization input is invalid",
      );
    }
    const running = this.organizationRuns.get(key);
    if (running) return running;
    const task = this.runOrganization(input).finally(() => {
      this.organizationRuns.delete(key);
    });
    this.organizationRuns.set(key, task);
    return task;
  }

  private async runOrganization(input: unknown) {
    try {
      const workflowId = this.newId();
      return await runOrganizationWorkflow(
        input,
        workflowId,
        {
          invoke: (method, params, id) =>
            this.invokeOrganizationEngine(method, params, id),
        },
        (value) => this.providerHost!.organize(parseOrganizerInput(value)),
      );
    } catch (error) {
      if (error instanceof ProtocolError) throw error;
      throw new ProtocolError(
        "organization_failed",
        error instanceof Error ? error.message : "organization failed",
      );
    }
  }

  async copyRole(sourceId: string, id: string, name?: string) {
    return this.mutateRole(() =>
      this.roleStore.copy(sourceId, { id, ...(name ? { name } : {}) }),
    );
  }

  private async mutateRole<T>(operation: () => Promise<T>) {
    try {
      return await operation();
    } catch (error) {
      throw new ProtocolError(
        "invalid_role",
        error instanceof Error ? error.message : "role is invalid",
      );
    }
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

  steer(sessionId: string, text: string, displayText = text) {
    this.session(sessionId).steer(text, displayText);
    return { accepted: true };
  }

  followUp(sessionId: string, text: string, displayText = text) {
    this.session(sessionId).followUp(text, displayText);
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

  async runDelegatedWorkflow(
    parent: RuntimeSession,
    parentRunId: string,
    spec: TaskGraph,
    onUpdate: (payload: unknown) => void,
    signal?: AbortSignal,
  ) {
    let eventQueue = Promise.resolve();
    const publish = (event: WorkflowRunEvent) => {
      onUpdate(event);
      const { type, ...payload } = event;
      eventQueue = eventQueue.then(() =>
        parent.emit(type, payload, parentRunId).then(() => undefined),
      );
    };
    const workflow = new WorkflowRun(
      spec,
      async (task, context) => {
        const childId = `wf_${this.newId()}`.slice(0, 128);
        await this.createSession(childId, undefined, task.role_id, false, [
          "session_search",
          "session_read",
          "usage",
        ]);
        const dependencyContext = Object.keys(context.dependency_results).length
          ? `\n\nDependency results:\n${JSON.stringify(
              context.dependency_results,
            )}`
          : "";
        const instruction =
          "Complete this delegated read-only task. Do not propose or execute mutations. " +
          "Return a concise result for the parent agent.\n\n" +
          task.instruction +
          dependencyContext;
        const abort = () => {
          const child = this.sessions.get(childId);
          if (child?.isRunning) child.abort();
        };
        context.signal.addEventListener("abort", abort, { once: true });
        try {
          await this.prompt(childId, instruction, [], task.instruction);
          await this.waitForIdle(childId);
          if (context.signal.aborted) throw new Error("task cancelled");
          const output = this.session(childId).finalText();
          if (!output) throw new Error("delegated agent returned no result");
          return output;
        } finally {
          context.signal.removeEventListener("abort", abort);
          const child = this.sessions.get(childId);
          if (child?.isRunning) {
            child.abort();
            await child.waitForIdle();
          }
          if (this.sessions.has(childId)) await this.deleteSession(childId);
        }
      },
      publish,
      () => this.now().getTime(),
    );
    const abortWorkflow = () => workflow.cancel();
    if (signal?.aborted) workflow.cancel();
    else signal?.addEventListener("abort", abortWorkflow, { once: true });
    try {
      const result = await workflow.start();
      await eventQueue;
      return result;
    } finally {
      signal?.removeEventListener("abort", abortWorkflow);
    }
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
          apply_policy: context.applyPolicy,
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

  private async invokeOrganizationEngine(
    method: OrganizationEngineMethod,
    params: Record<string, unknown>,
    workflowId: string,
  ) {
    return this.invokeInternalEngine(method, params, workflowId);
  }

  private async invokeInternalEngine(
    method:
      | OrganizationEngineMethod
      | import("../sessions/engine-store.js").RuntimeEngineMethod,
    params: Record<string, unknown>,
    sessionId: string,
  ) {
    if (this.engineHandler && !method.startsWith("runtime_sessions.")) {
      return this.engineHandler(
        method as OrganizationEngineMethod,
        params,
        sessionId,
      );
    }
    const requestId = this.newId();
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const result = new Promise<unknown>((resolve, reject) => {
      const cleanup = () => {
        if (timeout) clearTimeout(timeout);
      };
      this.pendingTools.set(requestId, {
        sessionId,
        resolve,
        reject,
        cleanup,
      });
      timeout = setTimeout(() => {
        if (!this.pendingTools.delete(requestId)) return;
        cleanup();
        reject(new Error("organization engine gateway timed out"));
      }, 125_000);
    });
    this.publish({
      protocol: PROTOCOL_VERSION,
      session_id: sessionId,
      run_id: sessionId,
      seq: this.runtimeSequence++,
      timestamp: this.now().toISOString(),
      type: "engine.request",
      payload: { request_id: requestId, method, params },
    });
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
