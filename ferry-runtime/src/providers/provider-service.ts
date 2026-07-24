import type { AuthType, Model } from "@earendil-works/pi-ai";
import type { StreamFn } from "@earendil-works/pi-agent-core";
import type { RuntimeErrorCode } from "../server/generated/errors.js";
import { ProtocolError } from "../server/messages.js";
import {
  AuthCoordinator,
  type AuthCoordinatorEvent,
} from "./auth-coordinator.js";
import type {
  CustomProviderConfig,
  ModelSelection,
} from "./provider-config.js";
import type { ProviderHost } from "./provider-host.js";

export interface AgentBackend {
  model: Model<string>;
  streamFn: StreamFn;
  provider?: string;
  modelId?: string;
  credentialAvailable?: () => boolean | Promise<boolean>;
}

export interface ProviderServiceOptions {
  host?: ProviderHost;
  fallbackBackend: AgentBackend;
  emitAuth(event: AuthCoordinatorEvent): void;
  idFactory(): string;
  isProviderInUse(providerId: string): boolean;
  selectSessionModel(
    sessionId: string,
    selection: ModelSelection,
    backend: AgentBackend,
  ): Promise<unknown>;
}

export class ProviderService {
  private readonly auth: AuthCoordinator | undefined;

  constructor(private readonly options: ProviderServiceOptions) {
    this.auth = options.host
      ? new AuthCoordinator(
          (providerId, type, interaction) =>
            options.host!.login(providerId, type, interaction),
          options.emitAuth,
          options.idFactory,
        )
      : undefined;
  }

  async status() {
    const selection = this.options.host
      ? await this.options.host.defaultModel()
      : {
          provider:
            this.options.fallbackBackend.provider ??
            this.options.fallbackBackend.model.provider,
          model:
            this.options.fallbackBackend.modelId ??
            this.options.fallbackBackend.model.id,
        };
    const configured = this.options.host
      ? await this.options.host.isConfigured(selection.provider)
      : await this.options.fallbackBackend.credentialAvailable?.();
    return {
      provider: selection.provider,
      model: selection.model,
      thinking: selection.thinking ?? "off",
      credential: configured ? "available" : "unavailable",
      provider_count: this.options.host
        ? (await this.options.host.providers()).length
        : 1,
    };
  }

  async providers() {
    return this.options.host?.providers() ?? [];
  }

  models(providerId: string, query = "", limit = 100) {
    if (!this.options.host) return [];
    try {
      return this.options.host.listModels(providerId, query, limit);
    } catch (error) {
      throw failure("provider_not_found", error, "provider not found");
    }
  }

  async enabledModels() {
    return this.options.host?.enabledModels() ?? [];
  }

  async catalogModels() {
    return this.options.host?.catalogModels() ?? [];
  }

  async testProvider(providerId: string, modelId?: string) {
    const host = this.requireHost("provider config unavailable");
    try {
      return await host.testProvider(providerId, modelId);
    } catch (error) {
      throw failure("provider_unreachable", error, "provider test failed");
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
    const host = this.requireHost("provider config unavailable");
    try {
      return await host.saveCustomModel(providerId, input);
    } catch (error) {
      throw failure("invalid_params", error, "custom model is invalid");
    }
  }

  async deleteCustomModel(providerId: string, modelId: string) {
    const host = this.requireHost("provider config unavailable");
    try {
      return await host.deleteCustomModel(providerId, modelId);
    } catch (error) {
      throw failure("provider_not_found", error, "provider not found");
    }
  }

  async setProviderEnabled(providerId: string, enabled: boolean) {
    const host = this.requireHost("provider config unavailable");
    try {
      return await host.setProviderEnabled(providerId, enabled);
    } catch (error) {
      throw failure("provider_not_found", error, "provider not found");
    }
  }

  async setVisibleModels(providerId: string, modelIds: string[] | null) {
    const host = this.requireHost("provider config unavailable");
    try {
      return await host.setVisibleModels(providerId, modelIds);
    } catch (error) {
      throw failure("model_not_found", error, "model not found");
    }
  }

  async refreshModels() {
    return this.requireHost("model refresh unavailable").refreshModels();
  }

  async config() {
    return this.requireHost(
      "provider config unavailable",
    ).store.publicSnapshot();
  }

  async saveApiKey(
    providerId: string,
    key: string,
    fields?: Record<string, string>,
  ) {
    const host = this.requireHost("provider config unavailable");
    this.ensureProviderAuthIdle(providerId);
    try {
      await host.saveApiKey(providerId, key, fields);
      return {
        provider_id: providerId,
        configured: true,
        credential_type: "api_key",
      };
    } catch (error) {
      throw failure("invalid_provider_config", error, "provider config failed");
    }
  }

  async logoutProvider(providerId: string) {
    const host = this.requireHost("provider config unavailable");
    this.ensureProviderAuthIdle(providerId);
    try {
      await host.logout(providerId);
      return { provider_id: providerId, configured: false };
    } catch (error) {
      throw failure("provider_not_found", error, "provider logout failed");
    }
  }

  startAuthentication(providerId: string, type: AuthType) {
    const host = this.requireHost("provider authentication unavailable");
    if (!this.auth) {
      throw new ProtocolError(
        "unsupported",
        "provider authentication unavailable",
      );
    }
    const provider = host.models.getProvider(providerId);
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
      throw failure("auth_in_progress", error, "authentication is in progress");
    }
  }

  respondAuthentication(loginId: string, promptId: string, value: string) {
    if (!this.auth) {
      throw new ProtocolError("unsupported", "authentication unavailable");
    }
    try {
      return this.auth.respond(loginId, promptId, value);
    } catch (error) {
      throw failure(
        "auth_prompt_not_found",
        error,
        "authentication prompt not found",
      );
    }
  }

  cancelAuthentication(loginId: string) {
    if (!this.auth) {
      throw new ProtocolError("unsupported", "authentication unavailable");
    }
    try {
      return this.auth.cancel(loginId);
    } catch (error) {
      throw failure(
        "auth_login_not_found",
        error,
        "authentication login not found",
      );
    }
  }

  async selectModel(sessionId: string | undefined, selection: ModelSelection) {
    const host = this.requireHost("model selection unavailable");
    let backend: AgentBackend;
    try {
      backend = host.backend(selection);
    } catch (error) {
      throw failure("model_not_found", error, "model not found");
    }
    if (sessionId) {
      return this.options.selectSessionModel(sessionId, selection, backend);
    }
    await host.selectDefault(selection);
    return { ...selection };
  }

  async saveCustomProvider(config: CustomProviderConfig, clearApiKey = false) {
    const host = this.requireHost("custom providers unavailable");
    if (host.isCustom(config.id) && this.options.isProviderInUse(config.id)) {
      throw new ProtocolError(
        "provider_in_use",
        "custom provider is used by a session",
      );
    }
    try {
      await host.saveCustomProvider(config, clearApiKey);
      return { provider_id: config.id, configured: true };
    } catch (error) {
      throw failure(
        "invalid_provider_config",
        error,
        "custom provider save failed",
      );
    }
  }

  async deleteCustomProvider(providerId: string) {
    const host = this.requireHost("custom providers unavailable");
    if (this.options.isProviderInUse(providerId)) {
      throw new ProtocolError(
        "provider_in_use",
        "custom provider is used by a session",
      );
    }
    try {
      await host.deleteCustomProvider(providerId);
      return { provider_id: providerId, deleted: true };
    } catch (error) {
      throw failure(
        "invalid_provider_config",
        error,
        "custom provider delete failed",
      );
    }
  }

  private requireHost(message: string) {
    if (!this.options.host) throw new ProtocolError("unsupported", message);
    return this.options.host;
  }

  private ensureProviderAuthIdle(providerId: string) {
    if (this.auth?.isProviderActive(providerId)) {
      throw new ProtocolError(
        "auth_in_progress",
        "provider authentication is in progress",
      );
    }
  }
}

function failure(code: RuntimeErrorCode, error: unknown, fallback: string) {
  return new ProtocolError(
    code,
    error instanceof Error ? error.message : fallback,
  );
}
