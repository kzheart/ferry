import { basename } from "node:path";
import { ProtocolError, requireString } from "../server/messages.js";

export interface NativeForkRequest {
  tool: string;
  sessionId: string;
  sourceRef: string;
  cwd: string;
  lastMessageId: string;
  executable: string;
}
export type NativeForkResult = { sessionId: string };
export type NativeForkAdapter = (
  request: NativeForkRequest,
) => Promise<NativeForkResult>;

export function parseRequest(
  params: Record<string, unknown>,
): NativeForkRequest {
  const tool = requireString(params, "tool", 32);
  const executable = requireString(params, "executable", 4096);
  if (
    ![tool, `${tool}.exe`, `${tool}.cmd`].includes(
      basename(executable).toLowerCase(),
    )
  ) {
    throw new ProtocolError("invalid_params", "Unexpected native executable");
  }
  return {
    tool,
    executable,
    sessionId: requireString(params, "sessionId", 512),
    sourceRef: requireString(params, "sourceRef", 8192),
    cwd: requireString(params, "cwd", 8192),
    lastMessageId: requireString(params, "lastMessageId", 512),
  };
}

export function unavailable(message: string): never {
  throw new ProtocolError("invalid_params", message);
}
