#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const runtimeRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(runtimeRoot, "..");
const ipcProtocol = JSON.parse(
  readFileSync(join(repositoryRoot, "contracts", "ipc.json"), "utf8"),
).protocol;
const seaNode = process.env.FERRY_SEA_NODE || process.execPath;
const target = process.argv[2] ?? hostTarget();
const expected = hostTarget();
if (target !== expected) {
  throw new Error(
    `SEA must be built natively: requested ${target}, host is ${expected}`,
  );
}

const windows = process.platform === "win32";
const extension = windows ? ".exe" : "";
const work = join(runtimeRoot, ".sea");
const bundle = join(work, "bundle.cjs");
const blob = join(work, "sea-prep.blob");
const config = join(work, "sea-config.json");
const output = join(
  repositoryRoot,
  "app",
  "src-tauri",
  "binaries",
  `ferry-runtime-${target}${extension}`,
);

rmSync(work, { recursive: true, force: true });
mkdirSync(work, { recursive: true });
mkdirSync(dirname(output), { recursive: true });

await build({
  entryPoints: [join(runtimeRoot, "src", "server", "server.ts")],
  outfile: bundle,
  bundle: true,
  platform: "node",
  format: "cjs",
  target: "node22",
  sourcemap: false,
  minify: false,
  logLevel: "info",
});

const seaVersion = spawnSync(seaNode, ["--version"], { encoding: "utf8" });
if (seaVersion.error || seaVersion.status !== 0) {
  throw seaVersion.error ?? new Error("cannot determine SEA Node version");
}
const modernSea =
  Number(seaVersion.stdout.trim().replace(/^v/, "").split(".")[0]) >= 25;
writeFileSync(
  config,
  JSON.stringify({
    main: bundle,
    output: modernSea ? output : blob,
    disableExperimentalSEAWarning: true,
    useSnapshot: false,
    useCodeCache: false,
  }),
);
rmSync(output, { force: true });
if (modernSea) {
  run(seaNode, ["--build-sea", config]);
} else {
  run(seaNode, ["--experimental-sea-config", config]);
  copyFileSync(seaNode, output);
  if (!windows) chmodSync(output, 0o755);
  if (process.platform === "darwin") {
    spawnSync("codesign", ["--remove-signature", output], { stdio: "ignore" });
  }
  const postject = join(
    runtimeRoot,
    "node_modules",
    "postject",
    "dist",
    "cli.js",
  );
  const injection = [
    postject,
    output,
    "NODE_SEA_BLOB",
    blob,
    "--sentinel-fuse",
    "NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2",
  ];
  if (process.platform === "darwin") {
    injection.push("--macho-segment-name", "NODE_SEA");
  }
  run(process.execPath, injection);
}
if (process.platform === "darwin") {
  run("codesign", ["--sign", "-", output]);
}

await smokeSea(output, join(work, "smoke-data"));
process.stdout.write(`${output}\n`);

async function smokeSea(executable, dataDirectory) {
  const child = spawn(executable, [], {
    stdio: ["pipe", "pipe", "pipe"],
    env: { ...process.env, FERRY_RUNTIME_DATA_DIR: dataDirectory },
  });
  const messages = [];
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  const lines = createInterface({ input: child.stdout });
  lines.on("line", (line) => {
    messages.push(JSON.parse(line));
  });
  const send = (id, method, params = {}) => {
    child.stdin.write(
      `${JSON.stringify({ protocol: ipcProtocol, id, method, params })}\n`,
    );
  };
  const waitFor = async (predicate, label) => {
    const deadline = Date.now() + 30_000;
    while (Date.now() < deadline) {
      const match = messages.find(predicate);
      if (match) return match;
      if (child.exitCode !== null) {
        throw new Error(`SEA exited while waiting for ${label}: ${stderr}`);
      }
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    throw new Error(
      `SEA timed out waiting for ${label}: ${JSON.stringify(messages)} ${stderr}`,
    );
  };

  try {
    send("sea-health", "health");
    const health = await waitFor(
      (message) => message.id === "sea-health",
      "health",
    );
    if (
      health.ok !== true ||
      health.result?.model !== "deepseek-v4-flash" ||
      health.result?.provider_count !== 36
    ) {
      throw new Error(`SEA health mismatch: ${JSON.stringify(health)}`);
    }

    send("sea-providers", "providers.list");
    const providers = await waitFor(
      (message) => message.id === "sea-providers",
      "provider catalog",
    );
    const providerIds = providers.result?.map?.((provider) => provider.id);
    if (
      providers.ok !== true ||
      providerIds?.length !== 36 ||
      providerIds.includes("amazon-bedrock") ||
      providerIds.includes("google-vertex")
    ) {
      throw new Error(`SEA provider mismatch: ${JSON.stringify(providers)}`);
    }

    send("sea-auth-start", "auth.login.start", {
      provider_id: "openai-codex",
      auth_type: "oauth",
    });
    const started = await waitFor(
      (message) => message.id === "sea-auth-start",
      "OAuth start",
    );
    if (started.ok !== true) {
      throw new Error(`SEA OAuth failed to start: ${JSON.stringify(started)}`);
    }
    const loginId = started.result.login_id;
    const prompt = await waitFor(
      (message) =>
        message.type === "auth.prompt" &&
        message.payload?.login_id === loginId &&
        message.payload?.prompt?.type === "select",
      "bundled OAuth prompt",
    );
    if (prompt.payload.provider_id !== "openai-codex") {
      throw new Error(`SEA OAuth provider mismatch: ${JSON.stringify(prompt)}`);
    }
    send("sea-auth-cancel", "auth.login.cancel", { login_id: loginId });
    const cancelled = await waitFor(
      (message) =>
        message.type === "auth.cancelled" &&
        message.payload?.login_id === loginId,
      "OAuth cancellation",
    );
    if (cancelled.payload.provider_id !== "openai-codex") {
      throw new Error(`SEA OAuth cancellation mismatch`);
    }
  } finally {
    child.stdin.end();
    lines.close();
    if (child.exitCode === null) child.kill();
  }
}

function run(command, args) {
  const result = spawnSync(command, args, { stdio: "inherit" });
  if (result.error || result.status !== 0) {
    throw result.error ?? new Error(`${command} exited with ${result.status}`);
  }
}

function hostTarget() {
  if (process.platform === "darwin" && process.arch === "arm64") {
    return "aarch64-apple-darwin";
  }
  if (process.platform === "win32" && process.arch === "x64") {
    return "x86_64-pc-windows-msvc";
  }
  throw new Error(`unsupported SEA host: ${process.platform}/${process.arch}`);
}
