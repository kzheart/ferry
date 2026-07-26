#!/usr/bin/env node
import { readdir, readFile } from "node:fs/promises";
import { basename, dirname, extname, join, relative, resolve, sep } from "node:path";
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

function importsFrom(source) {
  const imports = [];
  const pattern = /\b(?:from\s+|import\s*\()["']([^"']+)["']/g;
  for (const match of source.matchAll(pattern)) imports.push(match[1]);
  return imports;
}

function sourceArea(path) {
  const parts = relative(appSource, path).split(sep);
  return { area: parts[0], module: parts[0] === "modules" ? parts[1] : null };
}

function resolvedArea(file, specifier) {
  if (!specifier.startsWith(".")) return null;
  const target = resolve(dirname(file), specifier);
  const parts = relative(appSource, target).split(sep);
  return { target, area: parts[0], module: parts[0] === "modules" ? parts[1] : null };
}

function isPublicModuleEntry(target) {
  const stem = basename(target).replace(/\.(?:js|jsx|ts|tsx)$/, "");
  return stem === "public" || stem === "index";
}

const violations = [
  ...["domain"].filter(directory => topLevelDirectories.has(directory))
    .map(directory => `app/src/${directory}: 不允许重新引入横向 DDD 目录`),
  ...(await checkDirectory(join(appSource, "modules"), (file, source) => {
    const relative = file.slice(root.length + 1);
    return /\b(?:from\s+|import\s*\()["']@tauri-apps\//.test(source)
      ? [`${relative}: module 不允许直接依赖 Tauri`]
      : [];
  })),
  ...(await checkDirectory(join(appSource, "shell"), (file, source) => {
    const relative = file.slice(root.length + 1);
    return /\b(?:from\s+|import\s*\()["']@tauri-apps\//.test(source)
      ? [`${relative}: shell 不允许直接依赖 Tauri`]
      : [];
  })),
  ...(await Promise.all(
    ["platform", "shared"].map(area =>
      checkDirectory(join(appSource, area), (file, source) => {
        const path = file.slice(root.length + 1);
        return importsFrom(source).flatMap(specifier => {
          const target = resolvedArea(file, specifier);
          return target && ["modules", "shell"].includes(target.area)
            ? [`${path}: ${area} 不允许反向依赖 ${target.area}`]
            : [];
        });
      }),
    ),
  )).flat(),
  ...(await checkDirectory(join(appSource, "modules"), (file, source) => {
    const owner = sourceArea(file);
    const path = file.slice(root.length + 1);
    return importsFrom(source).flatMap(specifier => {
      const target = resolvedArea(file, specifier);
      if (!target || target.area !== "modules" || target.module === owner.module) {
        return [];
      }
      return isPublicModuleEntry(target.target)
        ? []
        : [`${path}: 同级模块 ${target.module} 只能通过 public/index 导入`];
    });
  })),
  ...(await checkDirectory(join(appSource, "shell"), (file, source) => {
    const path = file.slice(root.length + 1);
    return importsFrom(source).flatMap(specifier => {
      const target = resolvedArea(file, specifier);
      if (!target || target.area !== "modules") return [];
      return isPublicModuleEntry(target.target)
        ? []
        : [`${path}: shell 只能通过 public/index 导入模块 ${target.module}`];
    });
  })),
];

if (violations.length) {
  console.error(`前端架构边界检查失败:\n${violations.join("\n")}`);
  process.exitCode = 1;
}
