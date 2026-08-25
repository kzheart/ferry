#!/usr/bin/env node
// 把 Ferry 在本机的全部状态清成「全新用户」:引擎库(标签/归档/索引/备份/接手记录)、
// Tauri 应用数据、WebView 存储(localStorage 里的首启与引导标记就在这)和系统缓存。
// 这是破坏性操作,面向开发自测;加 --dry-run 只列出会删什么,不真删。
//
// 小写 ferry 的 WebKit/Caches 目录来自 `tauri dev` 未打包运行(按进程名落盘),
// dev.kzheart.ferry 来自打包应用(按 bundle id)。两边都清,首启体验才是真从零。
import { existsSync, rmSync } from "node:fs";
import { execSync } from "node:child_process";
import { homedir } from "node:os";
import { join } from "node:path";

if (process.platform !== "darwin") {
  console.error("fresh-start 目前只支持 macOS");
  process.exit(1);
}

const dryRun = process.argv.includes("--dry-run");
const BUNDLE_ID = "dev.kzheart.ferry";
const home = homedir();

// 正在跑的实例会把刚删的状态又写回去,先请它们退场(不存在时静默)
if (!dryRun) {
  for (const name of ["ferry-engine", "Ferry"]) {
    try {
      execSync(`pkill -x ${name}`, { stdio: "ignore" });
      console.log(`已停止进程: ${name}`);
    } catch { /* 没在跑 */ }
  }
}

const targets = [
  join(home, ".ferry"),
  join(home, "Library/Application Support", BUNDLE_ID),
  join(home, "Library/WebKit", BUNDLE_ID),
  join(home, "Library/WebKit/ferry"),
  join(home, "Library/Caches", BUNDLE_ID),
  join(home, "Library/Caches/ferry"),
  join(home, "Library/HTTPStorages", BUNDLE_ID),
  join(home, "Library/Saved Application State", `${BUNDLE_ID}.savedState`),
];

let removed = 0;
for (const path of targets) {
  if (!existsSync(path)) continue;
  if (dryRun) {
    console.log(`将删除: ${path}`);
  } else {
    rmSync(path, { recursive: true, force: true });
    console.log(`已删除: ${path}`);
  }
  removed += 1;
}

console.log(removed === 0
  ? "本机没有 Ferry 状态,已是全新环境"
  : dryRun
    ? `dry-run 结束,共 ${removed} 处待清理`
    : `清理完成,共 ${removed} 处;下次启动即首次启动向导`);
