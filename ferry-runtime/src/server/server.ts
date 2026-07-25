#!/usr/bin/env node
import { join } from "node:path";
import { homedir } from "node:os";
import { AgentRuntime } from "../runtime/runtime.js";
import { EngineSessionStore } from "../sessions/engine-store.js";
import { FileProviderConfigStore } from "../providers/provider-config-store.js";
import { FileRoleStore } from "../roles/role-store.js";
import { FileSkillStore } from "../skills/skill-store.js";
import { ProviderHost } from "../providers/provider-host.js";
import { PROTOCOL_VERSION, type ResponseEnvelope } from "./messages.js";
import { serveRuntime } from "./server-loop.js";

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
    skillStore: new FileSkillStore(dataDirectory),
    providerHost,
    deferRestore: true,
  });
  await serveRuntime(runtime, process.stdin, write);
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
