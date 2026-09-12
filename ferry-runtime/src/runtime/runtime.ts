import { randomUUID } from "node:crypto";
import type { ImageContent } from "@earendil-works/pi-ai";
import type {
  SessionSummary,
  SessionStore,
} from "../sessions/session-store.js";
import { EphemeralSessionStore } from "../sessions/session-store.js";
import { RuntimeSession } from "../sessions/runtime-session.js";
import type {
  ModelSelection,
  ThinkingLevel,
} from "../providers/provider-config.js";
import type { ProviderHost } from "../providers/provider-host.js";
import {
  ProviderService,
  type AgentBackend,
} from "../providers/provider-service.js";
import {
  DEFAULT_ROLE_ID,
  EphemeralRoleStore,
  type RoleStore,
} from "../roles/role-store.js";
import { RoleService } from "../roles/role-service.js";
import { EphemeralSkillStore, type SkillStore } from "../skills/skill-store.js";
import { SkillService } from "../skills/skill-service.js";
import { ProtocolError, type EventEnvelope } from "../server/messages.js";
import {
  FERRY_TOOL_NAMES,
  type FerryToolName,
  type ToolRequestContext,
} from "../tools/catalog.js";
import { RuntimeGateway, type ToolHandler } from "../tools/gateway.js";
import { RuntimeEventBus } from "./event-bus.js";

type BackendFactory = (selection?: ModelSelection) => AgentBackend;

interface RuntimeOptions {
  store?: SessionStore;
  storeFactory?: (
    invoke: import("../sessions/engine-store.js").RuntimeEngineInvoke,
  ) => SessionStore;
  deferRestore?: boolean;
  backendFactory?: BackendFactory;
  providerHost?: ProviderHost;
  roleStore?: RoleStore;
  skillStore?: SkillStore;
  toolHandler?: ToolHandler;
  now?: () => Date;
  idFactory?: () => string;
  toolDeadlinesMs?: Partial<Record<FerryToolName, number>>;
  /** 覆盖会话自动命名的取标题实现;缺省走 ProviderHost。 */
  titleGenerator?: TitleGenerator;
}

type TitleGenerator = (
  selection: ModelSelection,
  prompt: string,
  reply: string,
) => Promise<string | null>;

export class AgentRuntime {
  store: SessionStore;
  readonly roleService: RoleService;
  readonly skillService: SkillService;
  readonly now: () => Date;
  private readonly sessions = new Map<string, RuntimeSession>();
  private readonly summaries = new Map<string, SessionSummary>();
  private readonly loading = new Map<string, Promise<RuntimeSession>>();
  private restored = false;
  private readonly deleting = new Set<string>();
  private readonly events: RuntimeEventBus;
  private readonly backendFactory: BackendFactory;
  private readonly providerHost: ProviderHost | undefined;
  private readonly titleGenerator: TitleGenerator | undefined;
  private readonly idFactory: () => string;
  private readonly backendInfo: AgentBackend;
  readonly providerService: ProviderService;
  private readonly gateway: RuntimeGateway;

  private constructor(
    options: RuntimeOptions,
    backendFactory: BackendFactory,
    defaultSelection?: ModelSelection,
  ) {
    this.store = options.store ?? new EphemeralSessionStore();
    this.roleService = new RoleService(
      options.roleStore ?? new EphemeralRoleStore(),
    );
    this.skillService = new SkillService(
      options.skillStore ?? new EphemeralSkillStore(),
    );
    this.now = options.now ?? (() => new Date());
    this.events = new RuntimeEventBus(this.now);
    this.idFactory = options.idFactory ?? randomUUID;
    this.providerHost = options.providerHost;
    this.titleGenerator = options.titleGenerator;
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
      ...(options.toolDeadlinesMs
        ? { toolDeadlinesMs: options.toolDeadlinesMs }
        : {}),
    });
    this.providerService = new ProviderService({
      ...(this.providerHost ? { host: this.providerHost } : {}),
      fallbackBackend: this.backendInfo,
      emitAuth: (event) => this.events.emit(event.type, event.payload),
      idFactory: this.idFactory,
      isProviderInUse: (providerId) =>
        this.listSessions().some(
          (session) => session.provider_id === providerId,
        ),
      selectSessionModel: async (sessionId, selection, backend) =>
        (await this.loadSession(sessionId)).selectModel(selection, backend),
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
    if (this.restored)
      throw new ProtocolError(
        "already_restored",
        "runtime sessions already restored",
      );
    for (const metadata of await this.store.list())
      this.summaries.set(metadata.session_id, metadata);
    this.restored = true;
  }

  private async loadSession(id: string): Promise<RuntimeSession> {
    if (this.deleting.has(id))
      throw new ProtocolError("session_not_found", "session is being deleted");
    const loaded = this.sessions.get(id);
    if (loaded) return loaded;
    const pending = this.loading.get(id);
    if (pending) return pending;
    if (!this.summaries.has(id))
      throw new ProtocolError("session_not_found", "session not found");
    const task = this.restoreSession(id);
    this.loading.set(id, task);
    try {
      return await task;
    } finally {
      this.loading.delete(id);
    }
  }

  private async restoreSession(id: string) {
    const record = await this.store.load(id);
    if (!record)
      throw new ProtocolError("session_not_found", "session not found");
    const selection: ModelSelection = {
      provider: record.state.provider_id,
      model: record.state.model_id,
      ...(record.state.thinking_level
        ? { thinking: record.state.thinking_level as ThinkingLevel }
        : {}),
    };
    const session = new RuntimeSession(
      id,
      record.state,
      record.events,
      this,
      () => this.backendFactory(selection),
      selection,
      record.state.role_id ?? DEFAULT_ROLE_ID,
      record.state.resolved_persona ?? "",
      (record.state.resolved_tools ?? FERRY_TOOL_NAMES).filter(
        (name): name is FerryToolName =>
          (FERRY_TOOL_NAMES as readonly string[]).includes(name),
      ),
      record.state.resolved_apply_policy ?? "auto",
      await this.skillService.resolveFor(record.state.resolved_skills ?? []),
      (skillId) => this.skillService.read(skillId),
    );
    this.sessions.set(id, session);
    try {
      if (record.state.status === "running" && record.state.active_run_id) {
        await session.emit(
          "run.interrupted",
          { reason: "runtime_restarted" },
          record.state.active_run_id,
        );
      }
      return session;
    } catch (error) {
      this.sessions.delete(id);
      throw error;
    }
  }

  newId() {
    return this.idFactory();
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
  ) {
    const id = requestedId ?? this.newId();
    if (
      this.sessions.has(id) ||
      this.summaries.has(id) ||
      this.deleting.has(id)
    )
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
      [...role.tools],
      role.apply_policy,
      await this.skillService.resolveFor(role.skills),
      (id) => this.skillService.read(id),
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
    const session = await this.loadSession(sessionId);
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

  async editResend(
    sessionId: string,
    seq: number,
    text: string,
    displayText = text,
  ) {
    const session = await this.loadSession(sessionId);
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
    return { run_id: await session.editResend(seq, text, displayText) };
  }

  async generateTitle(
    selection: ModelSelection,
    prompt: string,
    reply: string,
  ) {
    if (this.titleGenerator)
      return this.titleGenerator(selection, prompt, reply);
    if (!this.providerHost) return null;
    return this.providerHost.summarizeTitle(prompt, reply, selection);
  }

  async renameSession(sessionId: string, title: string) {
    return (await this.loadSession(sessionId)).rename(title);
  }

  async pinSession(sessionId: string, pinned: boolean) {
    return (await this.loadSession(sessionId)).pin(pinned);
  }

  async deleteSession(sessionId: string) {
    if (!this.sessions.has(sessionId) && !this.summaries.has(sessionId)) {
      throw new ProtocolError("session_not_found", "session not found");
    }
    const session = this.sessions.get(sessionId);
    if (session?.isRunning || this.loading.has(sessionId)) {
      throw new ProtocolError(
        "run_in_progress",
        "cannot delete a running or loading session",
      );
    }
    if (this.deleting.has(sessionId))
      throw new ProtocolError("session_not_found", "session is being deleted");
    this.deleting.add(sessionId);
    try {
      await session?.dispose();
      await this.store.delete(sessionId);
      this.sessions.delete(sessionId);
      this.summaries.delete(sessionId);
      return { session_id: sessionId, deleted: true };
    } catch (error) {
      session?.cancelDisposal();
      throw error;
    } finally {
      this.deleting.delete(sessionId);
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
    const session = this.sessions.get(sessionId);
    if (session) return session.state();
    const metadata = this.summaries.get(sessionId);
    if (!metadata)
      throw new ProtocolError("session_not_found", "session not found");
    return {
      session_id: sessionId,
      provider_id: metadata.provider_id,
      model_id: metadata.model_id,
      status: "idle",
      active_run_id: null,
      latest_seq: metadata.next_seq - 1,
      queued_messages: false,
      contains_images: metadata.contains_images,
      title: metadata.title ?? null,
      title_locked: metadata.title_locked ?? false,
      pinned: metadata.pinned ?? false,
      thinking_level: metadata.thinking_level ?? "off",
      role_id: metadata.role_id ?? DEFAULT_ROLE_ID,
      apply_policy: metadata.resolved_apply_policy ?? "auto",
    };
  }

  listSessions() {
    return [...new Set([...this.summaries.keys(), ...this.sessions.keys()])]
      .map(
        (id) =>
          this.sessions.get(id)?.summary() ?? {
            ...this.state(id),
            created_at: this.summaries.get(id)?.created_at ?? null,
            updated_at: this.summaries.get(id)?.updated_at ?? null,
          },
      )
      .sort((left, right) =>
        String(right.updated_at ?? "").localeCompare(
          String(left.updated_at ?? ""),
        ),
      );
  }

  async replay(sessionId: string, afterSeq: number) {
    return (await this.loadSession(sessionId)).events.filter(
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
