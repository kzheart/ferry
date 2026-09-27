import type { CommandEnvelope, DispatchOutcome } from "../server/messages.js";
import { ProtocolError } from "../server/messages.js";
import { forkClaude } from "./claude.js";
import { forkCodex } from "./codex.js";
import { forkGrok } from "./grok.js";
import { forkPi } from "./pi.js";
import { parseRequest, type NativeForkAdapter } from "./types.js";

const adapters: Record<string, NativeForkAdapter> = {
  claude: forkClaude,
  codex: forkCodex,
  pi: forkPi,
  grok: forkGrok,
};
export async function dispatchNativeFork(
  command: CommandEnvelope,
): Promise<DispatchOutcome> {
  switch (command.method) {
    case "native_session.fork":
      break;
    default:
      return { handled: false };
  }
  const request = parseRequest(command.params);
  const adapter = adapters[request.tool];
  if (!adapter)
    throw new ProtocolError(
      "invalid_params",
      "Native turn fork is unavailable for this engine",
    );
  try {
    return { handled: true, result: await adapter(request) };
  } catch (error) {
    if (error instanceof ProtocolError) throw error;
    throw new ProtocolError(
      "invalid_params",
      error instanceof Error ? error.message : "Native fork failed",
    );
  }
}
