import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { NativeRpc } from "./rpc.js";
import { unavailable, type NativeForkAdapter } from "./types.js";

async function records(
  directory: string,
  name = "updates.jsonl",
): Promise<any[]> {
  return (await readFile(join(directory, name), "utf8"))
    .split("\n")
    .filter((line) => line.trim())
    .map((line) => JSON.parse(line));
}
function promptId(row: any): string | undefined {
  return row?.params?._meta?.promptId ?? row?.params?.update?.prompt_id;
}
export const forkGrok: NativeForkAdapter = async (request) => {
  const source = await records(request.sourceRef);
  const id = request.lastMessageId.replace(/:assistant$/, "");
  const completed = source.findLast(
    (row) =>
      promptId(row) === id &&
      row.params?.update?.sessionUpdate === "turn_completed",
  );
  if (completed?.params.update.stop_reason !== "end_turn")
    unavailable("Grok answer is not complete");
  const prompt = source.findLast(
    (row) =>
      row.params?.update?.sessionUpdate === "user_message_chunk" &&
      promptId(row) === id,
  );
  const targetPromptIndex =
    prompt?.params?._meta?.promptIndex ??
    prompt?.params?.update?._meta?.promptIndex;
  if (!Number.isSafeInteger(targetPromptIndex) || targetPromptIndex < 0)
    unavailable("Grok native prompt index is unavailable");
  // Old/compacted model histories can lack the requested coordinate. The native
  // fallback recount is not enough to promise an exact boundary, so fail closed.
  const chat = await records(request.sourceRef, "chat_history.jsonl");
  if (
    !chat.some(
      (row) => row.type === "user" && row.prompt_index === targetPromptIndex,
    ) ||
    chat.some(
      (row) =>
        row.type === "user" &&
        !row.synthetic_reason &&
        !Number.isSafeInteger(row.prompt_index),
    )
  ) {
    unavailable(
      "Grok model history lacks reliable prompt indices (legacy or compacted session); use Ferry Resume from this turn",
    );
  }
  const stop = chat.findIndex(
    (row) => row.type === "user" && row.prompt_index > targetPromptIndex,
  );
  const expected = stop < 0 ? chat : chat.slice(0, stop);
  const rpc = new NativeRpc(
    request.executable,
    ["agent", "--no-leader", "stdio"],
    request.cwd,
  );
  try {
    await rpc.call("initialize", {
      protocolVersion: 1,
      clientCapabilities: {},
      clientInfo: { name: "ferry", version: "0.9.1" },
    });
    const result = await rpc.call("_x.ai/session/fork", {
      sourceSessionId: request.sessionId,
      sourceCwd: request.cwd,
      newCwd: request.cwd,
      targetPromptIndex,
    });
    const sessionId = result?.newSessionId;
    if (
      typeof sessionId !== "string" ||
      !/^[a-zA-Z0-9-]+$/.test(sessionId) ||
      sessionId === request.sessionId
    )
      unavailable("Grok did not return a new session ID");
    const child = await records(join(dirname(request.sourceRef), sessionId));
    const ends = child.filter(
      (row) => row.params?.update?.sessionUpdate === "turn_completed",
    );
    if (
      promptId(ends.at(-1)) !== id ||
      child.some((row) => {
        const index =
          row.params?._meta?.promptIndex ??
          row.params?.update?._meta?.promptIndex;
        return typeof index === "number" && index > targetPromptIndex;
      })
    )
      unavailable(
        `Grok created ${sessionId}, but its fork boundary could not be verified. Do not retry automatically.`,
      );
    const childChat = await records(
      join(dirname(request.sourceRef), sessionId),
      "chat_history.jsonl",
    );
    const content = (row: any) =>
      JSON.stringify([row.type, row.content, row.prompt_index]);
    if (
      childChat.length !== expected.length ||
      childChat.some((row, i) => content(row) !== content(expected[i]))
    ) {
      unavailable(
        `Grok created ${sessionId}, but its model history boundary could not be verified. Do not retry automatically.`,
      );
    }
    return { sessionId };
  } finally {
    rpc.close();
  }
};
