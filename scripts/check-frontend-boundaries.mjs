#!/usr/bin/env node
import { readdir, readFile } from "node:fs/promises";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const appSource = join(root, "app", "src");

async function sourceFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(entries.map(async entry => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    return [path];
  }));
  return nested.flat().filter(path => [".js", ".jsx"].includes(extname(path)));
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
  ...(await checkDirectory(join(appSource, "domain"), (file, source) => {
    const relative = file.slice(root.length + 1);
    const found = [];
    if (extname(file) === ".jsx") found.push(`${relative}: domain 不允许 JSX 文件`);
    if (/\bfrom\s+["']react["']|\bimport\s*\(["']react["']\)/.test(source)) {
      found.push(`${relative}: domain 不允许依赖 React`);
    }
    return found;
  })),
  ...(await checkDirectory(join(appSource, "features"), (file, source) => {
    const relative = file.slice(root.length + 1);
    return /\b(?:from\s+|import\s*\()["']@tauri-apps\//.test(source)
      ? [`${relative}: feature 不允许直接依赖 Tauri`]
      : [];
  })),
];

if (violations.length) {
  console.error(`前端架构边界检查失败:\n${violations.join("\n")}`);
  process.exitCode = 1;
}
