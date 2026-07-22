#!/usr/bin/env node
import { join } from "node:path";
import { AgentRuntime } from "./runtime.js";
import { FileSessionStore } from "./event-store.js";
import { dispatch } from "./commands.js";
import {
  PROTOCOL_VERSION,
  ProtocolError,
  parseCommand,
  type ResponseEnvelope,
} from "./protocol.js";
import { readJsonLines } from "./jsonl.js";

function write(value: unknown) {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

const dataDirectory =
  process.env.FERRY_AGENT_DATA_DIR ?? join(process.cwd(), ".runtime-data");
const runtime = await AgentRuntime.create({
  store: new FileSessionStore(dataDirectory),
});
runtime.subscribe(write);

try {
  for await (const line of readJsonLines(process.stdin)) {
    void (async () => {
      let id = "unknown";
      try {
        const raw = JSON.parse(line) as unknown;
        if (
          typeof raw === "object" &&
          raw !== null &&
          "id" in raw &&
          typeof raw.id === "string"
        )
          id = raw.id;
        write(await dispatch(runtime, parseCommand(raw)));
      } catch (error) {
        const failure =
          error instanceof ProtocolError
            ? error
            : new ProtocolError("invalid_json", "input is not valid JSON");
        const response: ResponseEnvelope = {
          protocol: PROTOCOL_VERSION,
          id,
          ok: false,
          error: { code: failure.code, message: failure.message },
        };
        write(response);
      }
    })();
  }
} catch (error) {
  const response: ResponseEnvelope = {
    protocol: PROTOCOL_VERSION,
    id: "unknown",
    ok: false,
    error: {
      code: "invalid_framing",
      message: error instanceof Error ? error.message : "invalid JSONL input",
    },
  };
  write(response);
  process.exitCode = 1;
}
