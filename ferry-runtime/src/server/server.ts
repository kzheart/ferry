#!/usr/bin/env node
import { join } from "node:path";
import { homedir } from "node:os";
import { AgentRuntime } from "../runtime/runtime.js";
import { EngineSessionStore } from "../sessions/engine-store.js";
import { FileProviderConfigStore } from "../providers/provider-config-store.js";
import { FileRoleStore } from "../roles/role-repository.js";
import { ProviderHost } from "../providers/provider-host.js";
import { dispatch } from "../runtime/command-router.js";
import {
  PROTOCOL_VERSION,
  ProtocolError,
  parseCommand,
  type ResponseEnvelope,
} from "./messages.js";
import { readJsonLines } from "./jsonl.js";

function write(value: unknown) {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

async function main() {
  const dataDirectory =
    process.env.FERRY_RUNTIME_DATA_DIR ?? join(homedir(), ".ferry");
  const providerHost = await ProviderHost.create(
    new FileProviderConfigStore(join(dataDirectory, "providers.json")),
  );
  const runtime = await AgentRuntime.create({
    storeFactory: (invoke) => new EngineSessionStore(invoke),
    roleStore: new FileRoleStore(join(dataDirectory, "roles.json")),
    providerHost,
    deferRestore: true,
  });
  runtime.subscribe(write);
  await runtime.restore();

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
            error: failure.toEnvelope(),
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
        category: "validation",
        retryable: false,
        params: {
          message:
            error instanceof Error ? error.message : "invalid JSONL input",
        },
      },
    };
    write(response);
    process.exitCode = 1;
  }
}

void main().catch(() => {
  write({
    protocol: PROTOCOL_VERSION,
    id: "startup",
    ok: false,
    error: {
      code: "startup_failed",
      category: "internal",
      retryable: true,
      params: { message: "Ferry runtime failed to start" },
    },
  } satisfies ResponseEnvelope);
  process.exitCode = 1;
});
