import type { ProviderHost } from "../providers/provider-host.js";
import { ProtocolError } from "../server/messages.js";
import { parseOrganizerInput } from "./organizer.js";
import {
  runOrganizationWorkflow,
  type OrganizationEngineMethod,
} from "./organization.js";

export interface OrganizationCoordinatorOptions {
  providerHost?: ProviderHost;
  newId: () => string;
  invokeEngine: (
    method: OrganizationEngineMethod,
    params: Record<string, unknown>,
    workflowId: string,
  ) => Promise<unknown>;
}

export class OrganizationCoordinator {
  private readonly runs = new Map<string, Promise<unknown>>();

  constructor(private readonly options: OrganizationCoordinatorOptions) {}

  async start(input: unknown) {
    if (!this.options.providerHost) {
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
    const running = this.runs.get(key);
    if (running) return running;
    const task = this.run(input).finally(() => {
      this.runs.delete(key);
    });
    this.runs.set(key, task);
    return task;
  }

  private async run(input: unknown) {
    try {
      const workflowId = this.options.newId();
      return await runOrganizationWorkflow(
        input,
        workflowId,
        {
          invoke: (method, params, id) =>
            this.options.invokeEngine(method, params, id),
        },
        (value) =>
          this.options.providerHost!.organize(parseOrganizerInput(value)),
      );
    } catch (error) {
      if (error instanceof ProtocolError) throw error;
      throw new ProtocolError(
        "organization_failed",
        error instanceof Error ? error.message : "organization failed",
      );
    }
  }
}
