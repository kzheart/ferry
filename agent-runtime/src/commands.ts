import type { AgentRuntime } from "./runtime.js";
import {
  PROTOCOL_VERSION,
  ProtocolError,
  requireInteger,
  requireString,
  type CommandEnvelope,
  type ResponseEnvelope,
} from "./protocol.js";

export async function dispatch(
  runtime: AgentRuntime,
  command: CommandEnvelope,
): Promise<ResponseEnvelope> {
  try {
    const params = command.params ?? {};
    let result: unknown;
    switch (command.method) {
      case "health":
        result = {
          status: "ok",
          protocol: PROTOCOL_VERSION,
          runtime: "ferry-agent",
          pi_version: "0.81.1",
          ...runtime.providerStatus(),
        };
        break;
      case "session.create":
        result = await runtime.createSession(
          params.session_id === undefined
            ? undefined
            : requireString(params, "session_id", 128),
        );
        break;
      case "prompt":
        result = await runtime.prompt(
          requireString(params, "session_id", 128),
          requireString(params, "text"),
        );
        break;
      case "abort":
        result = runtime.abort(requireString(params, "session_id", 128));
        break;
      case "steer":
        result = runtime.steer(
          requireString(params, "session_id", 128),
          requireString(params, "text"),
        );
        break;
      case "follow_up":
        result = runtime.followUp(
          requireString(params, "session_id", 128),
          requireString(params, "text"),
        );
        break;
      case "state":
        result = runtime.state(requireString(params, "session_id", 128));
        break;
      case "events.replay":
        result = runtime.replay(
          requireString(params, "session_id", 128),
          requireInteger(params, "after_seq"),
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
      error: { code: protocolError.code, message: protocolError.message },
    };
  }
}
