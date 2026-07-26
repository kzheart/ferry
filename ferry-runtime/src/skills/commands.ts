import {
  ProtocolError,
  optionalString,
  requireString,
  type CommandEnvelope,
  type DispatchOutcome,
} from "../server/messages.js";
import type { SkillService } from "./skill-service.js";

export async function dispatchSkillCommand(
  service: SkillService,
  command: CommandEnvelope,
): Promise<DispatchOutcome> {
  const params = command.params;
  switch (command.method) {
    case "skills.list":
      return { handled: true, result: await service.list() };
    case "skills.candidates":
      return { handled: true, result: await service.candidates() };
    case "skill.import": {
      const candidateId = optionalString(params, "candidate_id", 512);
      const path = optionalString(params, "path", 1024);
      return {
        handled: true,
        result: await service.import({
          ...(candidateId ? { candidateId } : {}),
          ...(path ? { path } : {}),
          overwrite: params.overwrite === true,
        }),
      };
    }
    case "skill.delete":
      return {
        handled: true,
        result: await service.delete(requireString(params, "skill_id", 64)),
      };
    case "skills.global.set":
      return {
        handled: true,
        result: await service.setGlobal(requireIds(params)),
      };
    case "skill.source.add":
      return {
        handled: true,
        result: await service.addSource(requireString(params, "path", 1024)),
      };
    case "skill.source.remove":
      return {
        handled: true,
        result: await service.removeSource(
          requireString(params, "source_id", 128),
        ),
      };
    case "skill.read":
      return {
        handled: true,
        result: await service.read(requireString(params, "skill_id", 64)),
      };
    default:
      return { handled: false };
  }
}

function requireIds(params: Record<string, unknown>): string[] {
  const value = params.skill_ids;
  if (
    !Array.isArray(value) ||
    value.some(
      (item) =>
        typeof item !== "string" || item.length === 0 || item.length > 64,
    )
  ) {
    throw new ProtocolError(
      "invalid_params",
      "skill_ids must be an array of skill ids",
    );
  }
  return value as string[];
}
