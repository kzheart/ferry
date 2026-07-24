import {
  ProtocolError,
  isObject,
  optionalString,
  requireString,
  type CommandEnvelope,
} from "../server/messages.js";
import type { RoleInput } from "./role-store.js";
import type { RoleService } from "./role-service.js";

type RoleCommandResult =
  | { handled: true; result: unknown }
  | { handled: false };

export async function dispatchRoleCommand(
  service: RoleService,
  command: CommandEnvelope,
): Promise<RoleCommandResult> {
  const params = command.params;
  switch (command.method) {
    case "roles.list":
      return { handled: true, result: await service.list() };
    case "role.create":
      return {
        handled: true,
        result: await service.create(requireRole(params)),
      };
    case "role.update":
      return {
        handled: true,
        result: await service.update(
          requireString(params, "role_id", 128),
          requireRole(params),
        ),
      };
    case "role.copy":
      return {
        handled: true,
        result: await service.copy(
          requireString(params, "source_role_id", 128),
          requireString(params, "role_id", 128),
          optionalString(params, "name", 200),
        ),
      };
    case "role.delete":
      return {
        handled: true,
        result: await service.delete(requireString(params, "role_id", 128)),
      };
    default:
      return { handled: false };
  }
}

function requireRole(params: Record<string, unknown>): RoleInput {
  if (!isObject(params.role)) {
    throw new ProtocolError("invalid_params", "role must be an object");
  }
  return params.role as unknown as RoleInput;
}
