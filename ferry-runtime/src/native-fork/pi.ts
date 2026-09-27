import { dirname } from "node:path";
import { unavailable, type NativeForkAdapter } from "./types.js";

/** SessionManager forks the native entry path without starting an agent runtime. */
export const forkPi: NativeForkAdapter = async (request) => {
  const { SessionManager } = await import("@earendil-works/pi-coding-agent");
  const source = SessionManager.open(
    request.sourceRef,
    dirname(request.sourceRef),
  );
  if (source.getSessionId() !== request.sessionId)
    unavailable("Pi source identity changed");
  const branch = source.getBranch();
  const at = branch.findIndex((entry) => entry.id === request.lastMessageId);
  const answer = branch[at];
  if (
    !answer ||
    answer.type !== "message" ||
    answer.message.role !== "assistant" ||
    answer.message.stopReason !== "stop"
  ) {
    unavailable("Pi branch point is not a completed assistant answer");
  }
  if (!source.createBranchedSession(answer.id))
    unavailable("Pi did not persist the fork");
  const sessionId = source.getSessionId();
  if (sessionId === request.sessionId)
    unavailable("Pi did not create a new session");
  return { sessionId };
};
