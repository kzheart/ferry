import { unavailable, type NativeForkAdapter } from "./types.js";

export const forkClaude: NativeForkAdapter = async (request) => {
  const { forkSession, getSessionMessages } =
    await import("@anthropic-ai/claude-agent-sdk");
  const messages = await getSessionMessages(request.sessionId, {
    dir: request.cwd,
  });
  const last = messages.find((m) => m.uuid === request.lastMessageId);
  if (!last || last.type !== "assistant")
    unavailable("Claude branch message no longer exists");
  const result = await forkSession(request.sessionId, {
    dir: request.cwd,
    upToMessageId: request.lastMessageId,
  });
  const child = await getSessionMessages(result.sessionId, {
    dir: request.cwd,
  });
  const expected = messages.slice(0, messages.indexOf(last) + 1);
  if (
    child.length !== expected.length ||
    child.some(
      (message, i) =>
        JSON.stringify(message.message) !==
        JSON.stringify(expected[i]?.message),
    )
  ) {
    unavailable(
      `Claude created ${result.sessionId}, but its fork boundary could not be verified. Do not retry automatically.`,
    );
  }
  return result;
};
