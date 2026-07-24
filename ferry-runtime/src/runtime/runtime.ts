import { randomUUID } from "node:crypto";
import type { AuthType, ImageContent } from "@earendil-works/pi-ai";
import type { SessionStore } from "../sessions/session-repository.js";
import { EphemeralSessionStore } from "../sessions/session-repository.js";
import { RuntimeSession } from "../sessions/runtime-session.js";
import type {
  CustomProviderConfig,
  ModelSelection,
  ThinkingLevel,
} from "../providers/provider-config.js";
import type { ProviderHost } from "../providers/provider-host.js";
import {
  ProviderService,
  type AgentBackend,
} from "../providers/provider-service.js";
import { OrganizationCoordinator } from "../organizing/coordinator.js";
import {
  DEFAULT_ROLE_ID,
  EphemeralRoleStore,
  type RoleStore,
} from "../roles/role-repository.js";
import { RoleService } from "../roles/role-service.js";
import { ProtocolError, type EventEnvelope } from "../server/messages.js";
import {
  FERRY_TOOL_NAMES,
  type FerryToolName,
  type ToolRequestContext,
} from "../tools/catalog.js";
import {
  RuntimeGateway,
  type EngineHandler,
  type ToolHandler,
} from "../tools/gateway.js";
import type { TaskGraph } from "../agents/scheduler.js";
import { runDelegatedWorkflow as runDelegation } from "../agents/delegation-runner.js";
import { RuntimeEventBus } from "./event-bus.js";

export type BackendFactory = (selection?: ModelSelection) => AgentBackend;

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

export class AgentRuntime {
  store: SessionStore;
  readonly roleService: RoleService;
  readonly now: () => Date;
  private readonly sessions = new Map<string, RuntimeSession>();
  private readonly events: RuntimeEventBus;
  private readonly backendFactory: BackendFactory;
  private readonly providerHost: ProviderHost | undefined;
  private readonly idFactory: () => string;
  private readonly backendInfo: AgentBackend;
  private readonly providersService: ProviderService;
  private readonly gateway: RuntimeGateway;
  private readonly organization: OrganizationCoordinator;

  private constructor(
    options: RuntimeOptions,
    backendFactory: BackendFactory,
    defaultSelection?: ModelSelection,
  ) {
    this.store = options.store ?? new EphemeralSessionStore();
    this.roleService = new RoleService(
      options.roleStore ?? new EphemeralRoleStore(),
    );
    this.now = options.now ?? (() => new Date());
    this.events = new RuntimeEventBus(this.now);
    this.idFactory = options.idFactory ?? randomUUID;
    this.providerHost = options.providerHost;
    this.backendFactory = backendFactory;
    this.backendInfo = this.backendFactory(defaultSelection);
    this.gateway = new RuntimeGateway({
      newId: this.idFactory,
      events: this.events,
      emitToolRequest: (sessionId, runId, payload) =>
        this.session(sessionId)
          .emit("tool.request", payload, runId)
          .then(() => undefined),
      ...(options.toolHandler ? { toolHandler: options.toolHandler } : {}),
      ...(options.engineHandler
        ? { engineHandler: options.engineHandler }
        : {}),
      ...(options.toolDeadlinesMs
        ? { toolDeadlinesMs: options.toolDeadlinesMs }
        : {}),
    });
    this.organization = new OrganizationCoordinator({
      ...(this.providerHost ? { providerHost: this.providerHost } : {}),
      newId: this.idFactory,
      invokeEngine: (method, params, workflowId) =>
        this.gateway.invokeEngine(method, params, workflowId),
    });
    this.providersService = new ProviderService({
      ...(this.providerHost ? { host: this.providerHost } : {}),
      fallbackBackend: this.backendInfo,
      emitAuth: (event) => this.events.emit(event.type, event.payload),
      idFactory: this.idFactory,
      isProviderInUse: (providerId) =>
        [...this.sessions.values()].some(
          (session) => session.state().provider_id === providerId,
        ),
      selectSessionModel: (sessionId, selection, backend) =>
        this.session(sessionId).selectModel(selection, backend),
    });
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
        runtime.gateway.invokeEngine(method, params, sessionId),
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
    return this.providersService.status();
  }

  subscribe(listener: (event: EventEnvelope) => void) {
    return this.events.subscribe(listener);
  }

  publish(event: EventEnvelope) {
    this.events.publish(event);
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
    const role = await this.roleService.resolve(requestedRoleId);
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
    return this.providersService.providers();
  }

  async startOrganization(input: unknown) {
    return this.organization.start(input);
  }

  models(providerId: string, query = "", limit = 100) {
    return this.providersService.models(providerId, query, limit);
  }

  async enabledModels() {
    return this.providersService.enabledModels();
  }

  async catalogModels() {
    return this.providersService.catalogModels();
  }

  async testProvider(providerId: string, modelId?: string) {
    return this.providersService.testProvider(providerId, modelId);
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
    return this.providersService.saveCustomModel(providerId, input);
  }

  async deleteCustomModel(providerId: string, modelId: string) {
    return this.providersService.deleteCustomModel(providerId, modelId);
  }

  async setProviderEnabled(providerId: string, enabled: boolean) {
    return this.providersService.setProviderEnabled(providerId, enabled);
  }

  async setVisibleModels(providerId: string, modelIds: string[] | null) {
    return this.providersService.setVisibleModels(providerId, modelIds);
  }

  async refreshModels() {
    return this.providersService.refreshModels();
  }

  async config() {
    return this.providersService.config();
  }

  async saveApiKey(
    providerId: string,
    key: string,
    fields?: Record<string, string>,
  ) {
    return this.providersService.saveApiKey(providerId, key, fields);
  }

  async logoutProvider(providerId: string) {
    return this.providersService.logoutProvider(providerId);
  }

  startAuthentication(providerId: string, type: AuthType) {
    return this.providersService.startAuthentication(providerId, type);
  }

  respondAuthentication(loginId: string, promptId: string, value: string) {
    return this.providersService.respondAuthentication(
      loginId,
      promptId,
      value,
    );
  }

  cancelAuthentication(loginId: string) {
    return this.providersService.cancelAuthentication(loginId);
  }

  async selectModel(sessionId: string | undefined, selection: ModelSelection) {
    return this.providersService.selectModel(sessionId, selection);
  }

  async saveCustomProvider(config: CustomProviderConfig, clearApiKey = false) {
    return this.providersService.saveCustomProvider(config, clearApiKey);
  }

  async deleteCustomProvider(providerId: string) {
    return this.providersService.deleteCustomProvider(providerId);
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
    return runDelegation(
      parent,
      parentRunId,
      spec,
      onUpdate,
      {
        createTaskSession: async (roleId) => {
          const id = `wf_${this.newId()}`.slice(0, 128);
          await this.createSession(id, undefined, roleId, false, [
            "session_search",
            "session_read",
            "usage",
          ]);
          return id;
        },
        prompt: (sessionId, instruction) =>
          this.prompt(sessionId, instruction, [], instruction).then(
            () => undefined,
          ),
        waitForIdle: (sessionId) => this.waitForIdle(sessionId),
        finalText: (sessionId) => this.session(sessionId).finalText(),
        abort: (sessionId) => this.session(sessionId).abort(),
        isRunning: (sessionId) =>
          this.sessions.get(sessionId)?.isRunning ?? false,
        deleteSession: (sessionId) => this.deleteSession(sessionId),
        now: () => this.now().getTime(),
      },
      signal,
    );
  }

  async invokeTool(
    name: FerryToolName,
    args: Record<string, unknown>,
    context: ToolRequestContext,
  ) {
    return this.gateway.invokeTool(name, args, context);
  }

  completeTool(
    requestId: string,
    sessionId: string,
    ok: boolean,
    value: unknown,
  ) {
    return this.gateway.complete(requestId, sessionId, ok, value);
  }

  private session(id: string) {
    const session = this.sessions.get(id);
    if (!session)
      throw new ProtocolError("session_not_found", "session not found");
    return session;
  }
}
