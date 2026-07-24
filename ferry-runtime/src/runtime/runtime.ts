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
import { parseOrganizerInput } from "../organizing/organizer.js";
import {
  runOrganizationWorkflow,
  type OrganizationEngineMethod,
} from "../organizing/organization.js";
import {
  DEFAULT_ROLE_ID,
  EphemeralRoleStore,
  type RoleInput,
  type RoleStore,
} from "../roles/role-repository.js";
import { ProtocolError, type EventEnvelope } from "../server/messages.js";
import {
  FERRY_TOOL_NAMES,
  type FerryToolName,
  type ToolRequestContext,
} from "../tools/catalog.js";
import {
  WorkflowRun,
  type TaskGraph,
  type WorkflowRunEvent,
} from "../agents/scheduler.js";
import { RuntimeEventBus } from "./event-bus.js";

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

interface DeferredTool {
  sessionId: string;
  resolve(value: unknown): void;
  reject(error: Error): void;
  cleanup(): void;
}

export class AgentRuntime {
  store: SessionStore;
  readonly roleStore: RoleStore;
  readonly now: () => Date;
  private readonly sessions = new Map<string, RuntimeSession>();
  private readonly events: RuntimeEventBus;
  private readonly pendingTools = new Map<string, DeferredTool>();
  private readonly organizationRuns = new Map<string, Promise<unknown>>();
  private readonly backendFactory: BackendFactory;
  private readonly providerHost: ProviderHost | undefined;
  private readonly toolHandler: ToolHandler | undefined;
  private readonly engineHandler: EngineHandler | undefined;
  private readonly idFactory: () => string;
  private readonly backendInfo: AgentBackend;
  private readonly providersService: ProviderService;
  private readonly toolDeadlinesMs: Record<FerryToolName, number>;

  private constructor(
    options: RuntimeOptions,
    backendFactory: BackendFactory,
    defaultSelection?: ModelSelection,
  ) {
    this.store = options.store ?? new EphemeralSessionStore();
    this.roleStore = options.roleStore ?? new EphemeralRoleStore();
    this.now = options.now ?? (() => new Date());
    this.events = new RuntimeEventBus(this.now);
    this.idFactory = options.idFactory ?? randomUUID;
    this.toolDeadlinesMs = { ...TOOL_DEADLINES_MS, ...options.toolDeadlinesMs };
    this.toolHandler = options.toolHandler;
    this.engineHandler = options.engineHandler;
    this.providerHost = options.providerHost;
    this.backendFactory = backendFactory;
    this.backendInfo = this.backendFactory(defaultSelection);
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
    return this.providersService.providers();
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
    this.events.emit(
      "engine.request",
      { request_id: requestId, method, params },
      sessionId,
      sessionId,
    );
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
