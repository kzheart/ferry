import { afterEach, expect, test, vi } from "vitest";
import {
  mkdtemp,
  mkdir,
  readFile,
  writeFile,
  rm,
  realpath,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { forkClaude } from "../src/native-fork/claude.js";
import { forkPi } from "../src/native-fork/pi.js";
import { turnAtRecord } from "../src/native-fork/codex.js";
const roots: string[] = [];
afterEach(async () => {
  vi.unstubAllEnvs();
  await Promise.all(
    roots.splice(0).map((root) => rm(root, { recursive: true, force: true })),
  );
});
async function workspace() {
  const root = await realpath(
    await mkdtemp(join(tmpdir(), "ferry-fork-test-")),
  );
  roots.push(root);
  const cwd = join(root, "work");
  await mkdir(cwd);
  return { root, cwd };
}

test("Claude SDK fork retains the full selected answer, drops future messages, and leaves source intact", async () => {
  const { root, cwd } = await workspace();
  vi.stubEnv("CLAUDE_CONFIG_DIR", join(root, "claude"));
  const dir = join(
    root,
    "claude",
    "projects",
    cwd.replace(/[^a-zA-Z0-9]/g, "-"),
  );
  await mkdir(dir, { recursive: true });
  const id = randomUUID();
  const ids = Array.from({ length: 4 }, () => randomUUID());
  const rows = ids.map((uuid, i) => ({
    uuid,
    parentUuid: ids[i - 1] ?? null,
    type: i % 2 ? "assistant" : "user",
    sessionId: id,
    cwd,
    timestamp: new Date().toISOString(),
    isSidechain: false,
    message: {
      role: i % 2 ? "assistant" : "user",
      content: [{ type: "text", text: i < 2 ? "kept" : "future" }],
      ...(i % 2 ? { stop_reason: "end_turn" } : {}),
    },
  }));
  const sourceRef = join(dir, `${id}.jsonl`);
  const before = rows.map((row) => JSON.stringify(row)).join("\n") + "\n";
  await writeFile(sourceRef, before);
  const result = await forkClaude({
    tool: "claude",
    executable: "claude",
    sessionId: id,
    cwd,
    sourceRef,
    lastMessageId: ids[1]!,
  });
  const child = await readFile(join(dir, `${result.sessionId}.jsonl`), "utf8");
  expect(child).toContain("kept");
  expect(child).not.toContain("future");
  expect(await readFile(sourceRef, "utf8")).toBe(before);
});

test("Pi native SessionManager forks at an answer without loading the agent or modifying the original", async () => {
  const { root, cwd } = await workspace();
  vi.stubEnv("PI_CODING_AGENT_DIR", join(root, "pi"));
  const dir = join(root, "sessions");
  await mkdir(dir);
  const id = randomUUID();
  const rows: any[] = [
    {
      type: "session",
      version: 3,
      id,
      cwd,
      timestamp: new Date().toISOString(),
    },
  ];
  for (let i = 0; i < 4; i++)
    rows.push({
      type: "message",
      id: `m${i}`,
      parentId: i ? `m${i - 1}` : null,
      timestamp: new Date().toISOString(),
      message: {
        role: i % 2 ? "assistant" : "user",
        content: [{ type: "text", text: i < 2 ? "kept" : "future" }],
        ...(i % 2
          ? {
              stopReason: "stop",
              model: "fixture",
              provider: "fixture",
              api: "anthropic-messages",
            }
          : {}),
      },
    });
  const sourceRef = join(dir, `${id}.jsonl`);
  const before = rows.map((row) => JSON.stringify(row)).join("\n") + "\n";
  await writeFile(sourceRef, before);
  const result = await forkPi({
    tool: "pi",
    executable: "pi",
    sessionId: id,
    cwd,
    sourceRef,
    lastMessageId: "m1",
  });
  const { readdir } = await import("node:fs/promises");
  const file = (await readdir(dir)).find((name) =>
    name.includes(result.sessionId),
  );
  expect(file).toBeTruthy();
  const child = await readFile(join(dir, file!), "utf8");
  expect(child).not.toContain("future");
  expect(child).toContain("kept");
  expect(await readFile(sourceRef, "utf8")).toBe(before);
});

test("Codex source locators count nonblank native records and require a turn context", async () => {
  const { root } = await workspace();
  const path = join(root, "rollout.jsonl");
  await writeFile(
    path,
    '\n{"type":"turn_context","payload":{"turn_id":"turn1"}}\n\n{"type":"response_item","payload":{"role":"assistant"}}\n',
  );
  expect(await turnAtRecord(path, "record:1")).toBe("turn1");
  await expect(turnAtRecord(path, "record:0")).rejects.toThrow("native turn");
  await expect(turnAtRecord(path, "record:9")).rejects.toThrow(
    "no longer exists",
  );
});
