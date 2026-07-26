import { dispatchProviderCommand } from "../providers/commands.js";
import { FERRY_CONTRACT_HASH } from "../server/generated/ipc.js";
import type { AgentRuntime } from "./runtime.js";
import { dispatchRoleCommand } from "../roles/commands.js";
import { dispatchSessionCommand } from "../sessions/commands.js";
import { dispatchSkillCommand } from "../skills/commands.js";
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
    // 逐个域试派发,命中即止——未命中的域不该被无谓地调一遍
    const domains = [
      () => dispatchProviderCommand(runtime.providerService, command),
      () => dispatchRoleCommand(runtime.roleService, command),
      () => dispatchSkillCommand(runtime.skillService, command),
      () => dispatchSessionCommand(runtime, command),
    ];
    let handled = false;
    for (const dispatchDomain of domains) {
      const outcome = await dispatchDomain();
      if (outcome.handled) {
        result = outcome.result;
        handled = true;
        break;
      }
    }
    if (!handled) {
      handled = true;
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
        default:
          handled = false;
      }
    }
    // 未知方法必须报错:静默返回 ok+undefined 会把契约漂移伪装成成功
    if (!handled) {
      throw new ProtocolError(
        "unknown_method",
        `unknown method ${command.method}`,
      );
    }
    return { protocol: PROTOCOL_VERSION, id: command.id, ok: true, result };
  } catch (error) {
    if (!(error instanceof ProtocolError)) {
      // 回给前端的文案是脱敏的;真实异常不留痕就只能靠猜。
      console.error(`command ${command.method} failed`, error);
    }
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
