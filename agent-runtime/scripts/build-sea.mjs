#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  mkdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const runtimeRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(runtimeRoot, "..");
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
  `ferry-agent-${target}${extension}`,
);

rmSync(work, { recursive: true, force: true });
mkdirSync(work, { recursive: true });
mkdirSync(dirname(output), { recursive: true });

await build({
  entryPoints: [join(runtimeRoot, "src", "cli.ts")],
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

const health = JSON.stringify({
  protocol: "ferry-agent/v1",
  id: "sea-smoke",
  method: "health",
  params: {},
});
const smoke = spawnSync(output, [], {
  input: `${health}\n`,
  encoding: "utf8",
  timeout: 30_000,
  env: { ...process.env, FERRY_AGENT_DATA_DIR: join(work, "smoke-data") },
});
if (smoke.error || smoke.status !== 0) {
  throw smoke.error ?? new Error(`SEA smoke failed: ${smoke.stderr}`);
}
const response = JSON.parse(smoke.stdout.trim().split(/\r?\n/).at(-1));
if (response.ok !== true || response.result?.model !== "deepseek-v4-flash") {
  throw new Error(`SEA health mismatch: ${smoke.stdout}`);
}
process.stdout.write(`${output}\n`);

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
