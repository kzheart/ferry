import { createReadStream } from "node:fs";
import { createInterface } from "node:readline";
import { NativeRpc } from "./rpc.js";
import { unavailable, type NativeForkAdapter } from "./types.js";

/** Ferry's Codex source locator counts non-empty JSONL records from zero. */
export async function turnAtRecord(
  path: string,
  locator: string,
): Promise<string> {
  if (!/^record:\d+$/.test(locator))
    unavailable("Codex native record locator is missing");
  const target = Number(locator.slice(7));
  const stream = createReadStream(path);
  const lines = createInterface({ input: stream, crlfDelay: Infinity });
  let ordinal = 0;
  let turn: string | undefined;
  try {
    for await (const line of lines) {
      if (!line.trim()) continue;
      const record = JSON.parse(line);
      if (
        record.type === "turn_context" &&
        typeof record.payload?.turn_id === "string"
      )
        turn = record.payload.turn_id;
      if (ordinal++ === target) {
        if (
          !turn ||
          record.type !== "response_item" ||
          record.payload?.role !== "assistant"
        )
          unavailable("Cannot map Codex answer to a native turn");
        return turn;
      }
    }
    return unavailable("Codex branch record no longer exists");
  } finally {
    lines.close();
    stream.destroy();
  }
}

export const forkCodex: NativeForkAdapter = async (request) => {
  const lastTurnId = await turnAtRecord(
    request.sourceRef,
    request.lastMessageId,
  );
  const rpc = new NativeRpc(request.executable, ["app-server"], request.cwd);
  try {
    await rpc.call("initialize", {
      clientInfo: { name: "ferry", version: "0.9.1" },
      capabilities: {},
    });
    rpc.notify("initialized");
    const source = await rpc.call("thread/read", {
      threadId: request.sessionId,
      includeTurns: true,
    });
    const turns: any[] = source?.thread?.turns ?? [];
    const turn = turns.find((t) => t.id === lastTurnId);
    if (!turn || turn.status !== "completed")
      unavailable(
        "Codex turn is not completed, or this version cannot read it",
      );
    const result = await rpc.call("thread/fork", {
      threadId: request.sessionId,
      lastTurnId,
    });
    const sessionId = result?.thread?.id;
    if (typeof sessionId !== "string" || sessionId === request.sessionId)
      unavailable("Codex did not return a new thread ID");
    // Read back: older servers may ignore unknown parameters. Never claim a bounded fork
    // succeeded unless the returned history ends at the requested native turn.
    const child = await rpc.call("thread/read", {
      threadId: sessionId,
      includeTurns: true,
    });
    const inherited: any[] = child?.thread?.turns ?? [];
    if (
      inherited.at(-1)?.id !== lastTurnId ||
      inherited.length !== turns.findIndex((t) => t.id === lastTurnId) + 1
    ) {
      unavailable(
        `Codex created ${sessionId}, but its fork boundary could not be verified. Do not retry automatically.`,
      );
    }
    return { sessionId };
  } finally {
    rpc.close();
  }
};
