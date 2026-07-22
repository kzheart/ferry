import { randomUUID } from "node:crypto";
import type {
  AuthEvent,
  AuthInteraction,
  AuthPrompt,
  AuthType,
  Credential,
} from "@earendil-works/pi-ai";

export interface AuthCoordinatorEvent {
  type:
    | "auth.prompt"
    | "auth.event"
    | "auth.completed"
    | "auth.failed"
    | "auth.cancelled";
  payload: Record<string, unknown>;
}

interface DeferredPrompt {
  id: string;
  resolve(value: string): void;
  reject(error: Error): void;
}

interface PendingLogin {
  id: string;
  providerId: string;
  type: AuthType;
  controller: AbortController;
  prompt?: DeferredPrompt | undefined;
}

export type LoginExecutor = (
  providerId: string,
  type: AuthType,
  interaction: AuthInteraction,
) => Promise<Credential>;

function abortError() {
  const error = new Error("authentication cancelled");
  error.name = "AbortError";
  return error;
}

function promptDto(prompt: AuthPrompt) {
  return {
    type: prompt.type,
    message: prompt.message,
    ...("placeholder" in prompt && prompt.placeholder
      ? { placeholder: prompt.placeholder }
      : {}),
    ...(prompt.type === "select"
      ? {
          options: prompt.options.map((option) => ({
            id: option.id,
            label: option.label,
            ...(option.description ? { description: option.description } : {}),
          })),
        }
      : {}),
  };
}

export class AuthCoordinator {
  private readonly logins = new Map<string, PendingLogin>();
  private readonly providers = new Map<string, string>();

  constructor(
    private readonly login: LoginExecutor,
    private readonly emit: (event: AuthCoordinatorEvent) => void,
    private readonly idFactory: () => string = randomUUID,
  ) {}

  start(providerId: string, type: AuthType) {
    if (this.providers.has(providerId)) {
      throw new Error("provider authentication is already in progress");
    }
    const id = this.idFactory();
    const pending: PendingLogin = {
      id,
      providerId,
      type,
      controller: new AbortController(),
    };
    this.logins.set(id, pending);
    this.providers.set(providerId, id);
    void this.run(pending);
    return { login_id: id, provider_id: providerId, auth_type: type };
  }

  isProviderActive(providerId: string) {
    return this.providers.has(providerId);
  }

  respond(loginId: string, promptId: string, value: string) {
    const pending = this.logins.get(loginId);
    if (!pending?.prompt || pending.prompt.id !== promptId) {
      throw new Error("authentication prompt not found");
    }
    const prompt = pending.prompt;
    pending.prompt = undefined;
    prompt.resolve(value);
    return { accepted: true };
  }

  cancel(loginId: string) {
    const pending = this.logins.get(loginId);
    if (!pending) throw new Error("authentication login not found");
    pending.controller.abort();
    pending.prompt?.reject(abortError());
    pending.prompt = undefined;
    return { accepted: true };
  }

  private async run(pending: PendingLogin) {
    try {
      const credential = await this.login(pending.providerId, pending.type, {
        signal: pending.controller.signal,
        prompt: (prompt) => this.ask(pending, prompt),
        notify: (event) => this.notify(pending, event),
      });
      if (pending.controller.signal.aborted) throw abortError();
      this.emit({
        type: "auth.completed",
        payload: {
          login_id: pending.id,
          provider_id: pending.providerId,
          credential_type: credential.type,
        },
      });
    } catch (error) {
      this.emit({
        type:
          pending.controller.signal.aborted ||
          (error instanceof Error && error.name === "AbortError")
            ? "auth.cancelled"
            : "auth.failed",
        payload: {
          login_id: pending.id,
          provider_id: pending.providerId,
          ...(pending.controller.signal.aborted
            ? {}
            : { message: "provider authentication failed" }),
        },
      });
    } finally {
      pending.prompt?.reject(abortError());
      this.logins.delete(pending.id);
      if (this.providers.get(pending.providerId) === pending.id) {
        this.providers.delete(pending.providerId);
      }
    }
  }

  private ask(pending: PendingLogin, prompt: AuthPrompt) {
    if (pending.controller.signal.aborted) return Promise.reject(abortError());
    if (pending.prompt) {
      return Promise.reject(
        new Error("provider requested concurrent authentication prompts"),
      );
    }
    const promptId = this.idFactory();
    return new Promise<string>((resolve, reject) => {
      const deferred: DeferredPrompt = { id: promptId, resolve, reject };
      pending.prompt = deferred;
      const cancel = () => {
        if (pending.prompt === deferred) pending.prompt = undefined;
        reject(abortError());
      };
      pending.controller.signal.addEventListener("abort", cancel, {
        once: true,
      });
      prompt.signal?.addEventListener("abort", cancel, { once: true });
      this.emit({
        type: "auth.prompt",
        payload: {
          login_id: pending.id,
          prompt_id: promptId,
          provider_id: pending.providerId,
          prompt: promptDto(prompt),
        },
      });
    });
  }

  private notify(pending: PendingLogin, event: AuthEvent) {
    this.emit({
      type: "auth.event",
      payload: {
        login_id: pending.id,
        provider_id: pending.providerId,
        event,
      },
    });
  }
}
