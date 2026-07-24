#!/usr/bin/env node
import { readdir, readFile } from "node:fs/promises";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const appSource = join(root, "app", "src");
const topLevelDirectories = new Set(
  (await readdir(appSource, { withFileTypes: true }))
    .filter(entry => entry.isDirectory())
    .map(entry => entry.name),
);

async function sourceFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(entries.map(async entry => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    return [path];
  }));
  return nested
    .flat()
    .filter(path => [".js", ".jsx", ".ts", ".tsx"].includes(extname(path)));
}

async function checkDirectory(directory, check) {
  const files = await sourceFiles(directory);
  const violations = [];
  for (const file of files) {
    const source = await readFile(file, "utf8");
    violations.push(...check(file, source));
  }
  return violations;
}

const violations = [
  ...["domain"].filter(directory => topLevelDirectories.has(directory))
    .map(directory => `app/src/${directory}: 不允许重新引入横向 DDD 目录`),
  ...(await checkDirectory(join(appSource, "features"), (file, source) => {
    const relative = file.slice(root.length + 1);
    return /\b(?:from\s+|import\s*\()["']@tauri-apps\//.test(source)
      ? [`${relative}: feature 不允许直接依赖 Tauri`]
      : [];
  })),
  ...(await checkDirectory(join(appSource, "shell"), (file, source) => {
    const relative = file.slice(root.length + 1);
    return /\b(?:from\s+|import\s*\()["']@tauri-apps\//.test(source)
      ? [`${relative}: shell 不允许直接依赖 Tauri`]
      : [];
  })),
];

if (violations.length) {
  console.error(`前端架构边界检查失败:\n${violations.join("\n")}`);
  process.exitCode = 1;
}
