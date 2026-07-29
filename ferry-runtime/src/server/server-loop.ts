import type { Readable } from "node:stream";
import type { AgentRuntime } from "../runtime/runtime.js";
import { dispatch } from "../runtime/command-router.js";
import {
  PROTOCOL_VERSION,
  ProtocolError,
  parseCommand,
  type ResponseEnvelope,
} from "./messages.js";
import { readJsonLines } from "./jsonl.js";

type RuntimeWriter = (value: unknown) => void;
type RestoreOutcome = { ok: true } | { ok: false; error: unknown };

function failedResponse(id: string, error: unknown): ResponseEnvelope {
  const failure =
    error instanceof ProtocolError
      ? error
      : new ProtocolError("invalid_json", "input is not valid JSON");
  return {
    protocol: PROTOCOL_VERSION,
    id,
    ok: false,
    error: failure.toEnvelope(),
  };
}

/**
 * 先消费 stdin，再等待恢复完成。恢复期间只能让 Host 的 tool.result 穿透，
 * 否则 Runtime 会等待 Engine 回包，而 Host 回包又滞留在尚未读取的 stdin 中。
 */
export async function serveRuntime(
  runtime: AgentRuntime,
  input: Readable,
  write: RuntimeWriter,
) {
  runtime.subscribe(write);
  // stderr 由宿主接到日志文件;恢复失败会连带 health 失败,必须留下现场。
  const restoreStarted = Date.now();
  const restore: Promise<RestoreOutcome> = runtime.restore().then(
    () => {
      console.error(
        `session restore completed in ${Date.now() - restoreStarted}ms`,
      );
      return { ok: true as const };
    },
    (error) => {
      console.error(
        `session restore failed after ${Date.now() - restoreStarted}ms`,
        error,
      );
      return { ok: false as const, error };
    },
  );

  const handle = async (line: string) => {
    let id = "unknown";
    try {
      const raw = JSON.parse(line) as unknown;
      if (
        typeof raw === "object" &&
        raw !== null &&
        "id" in raw &&
        typeof raw.id === "string"
      ) {
        id = raw.id;
      }
      const command = parseCommand(raw);
      if (command.method !== "tool.result") {
        const outcome = await restore;
        if (!outcome.ok) {
          throw new ProtocolError(
            "internal_error",
            "runtime session restore failed",
          );
        }
      }
      write(await dispatch(runtime, command));
    } catch (error) {
      write(failedResponse(id, error));
    }
  };

  try {
    for await (const line of readJsonLines(input)) {
      // 必须并发处理：先到的 health 会等待恢复，后到的 tool.result 则需要
      // 立即穿透以解除恢复中的 Engine 请求。
      void handle(line);
    }
  } catch (error) {
    write({
      protocol: PROTOCOL_VERSION,
      id: "unknown",
      ok: false,
      error: {
        code: "invalid_framing",
        category: "validation",
        retryable: false,
        params: {
          message:
            error instanceof Error ? error.message : "invalid JSONL input",
        },
      },
    } satisfies ResponseEnvelope);
  }
}
