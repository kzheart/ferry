import { dispatchProviderCommand } from "../providers/commands.js";
import { FERRY_CONTRACT_HASH } from "../server/generated/ipc.js";
import type { AgentRuntime } from "./runtime.js";
import { dispatchRoleCommand } from "../roles/commands.js";
import { dispatchSessionCommand } from "../sessions/commands.js";
import {
  PROTOCOL_VERSION,
  ProtocolError,
  requireString,
  type CommandEnvelope,
  type ResponseEnvelope,
} from "../server/messages.js";

export async function dispatch(
  runtime: AgentRuntime,
  command: CommandEnvelope,
): Promise<ResponseEnvelope> {
  try {
    const params = command.params;
    let result: unknown;
    const providerCommand = await dispatchProviderCommand(
      runtime.providerService,
      command,
    );
    const roleCommand = await dispatchRoleCommand(runtime.roleService, command);
    const sessionCommand = await dispatchSessionCommand(runtime, command);
    if (providerCommand.handled) {
      result = providerCommand.result;
    } else if (roleCommand.handled) {
      result = roleCommand.result;
    } else if (sessionCommand.handled) {
      result = sessionCommand.result;
    } else
      switch (command.method) {
        case "health":
          result = {
            status: "ready",
            service: "ferry-runtime",
            contract_hash: FERRY_CONTRACT_HASH,
            pi_version: "0.81.1",
            ...(await runtime.providerService.status()),
          };
          break;
        case "organization.start":
          result = runtime.startOrganization({
            sessions: params.sessions,
            locale: params.locale,
          });
          break;
        case "organization.status":
          result = runtime.organizationStatus(
            requireString(params, "job_id", 128),
          );
          break;
        case "organization.cancel":
          result = runtime.cancelOrganization(
            requireString(params, "job_id", 128),
          );
          break;
        case "tool.result":
          result = runtime.completeTool(
            requireString(params, "request_id", 128),
            requireString(params, "session_id", 128),
            params.ok === true,
            params.ok === true ? params.result : params.error,
          );
          break;
      }
    return { protocol: PROTOCOL_VERSION, id: command.id, ok: true, result };
  } catch (error) {
    const protocolError =
      error instanceof ProtocolError
        ? error
        : new ProtocolError("internal_error", "internal runtime error");
    return {
      protocol: PROTOCOL_VERSION,
      id: command.id,
      ok: false,
      error: protocolError.toEnvelope(),
    };
  }
}
