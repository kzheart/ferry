#!/usr/bin/env node
// 把 Ferry 在本机清成「全新用户」:引擎库(标签/归档/索引/备份/接手记录)、
// Tauri 应用数据、WebView 存储、CLI 入口和 Ferry 自带的两份 skill。
// 这是破坏性操作,面向开发自测;加 --dry-run 只列出会删什么,不真删。
//
// 小写 ferry 的 WebKit/Caches 目录来自 `tauri dev` 未打包运行(按进程名落盘),
// dev.kzheart.ferry 来自打包应用(按 bundle id)。两边都清,首启体验才是真从零。
// Windows 对应清理 Roaming 应用数据、Local WebView2 数据和 Ferry CLI 的用户 PATH 项。
import { lstatSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { homedir } from "node:os";
import { join } from "node:path";

if (!["darwin", "win32"].includes(process.platform)) {
  console.error("fresh-start 目前只支持 macOS 和 Windows");
  process.exit(1);
}

const dryRun = process.argv.includes("--dry-run");
const BUNDLE_ID = "dev.kzheart.ferry";
const SKILLS = ["ferry", "ferry-resume"];
const home = homedir();

// 正在跑的实例会把刚删的状态又写回去,先请它们退场(不存在时静默)
if (!dryRun) {
  if (process.platform === "darwin") {
    for (const name of ["ferry-engine", "Ferry"]) {
      try {
        execFileSync("pkill", ["-x", name], { stdio: "ignore" });
        console.log(`已停止进程: ${name}`);
      } catch { /* 没在跑 */ }
    }
  } else {
    // /T 一并停止由桌面宿主拉起的 runtime/engine;/F 避免 WebView2 留住数据目录。
    for (const name of ["Ferry.exe", "ferry-engine.exe"]) {
      try {
        execFileSync("taskkill.exe", ["/F", "/T", "/IM", name], { stdio: "ignore" });
        console.log(`已停止进程: ${name}`);
      } catch { /* 没在跑 */ }
    }
  }
}

const skillTargets = [
  // Claude 入口可能是指向共享目录的 junction/symlink,必须在真身之前摘掉。
  ...SKILLS.map(name => join(home, ".claude", "skills", name)),
  ...SKILLS.map(name => join(home, ".agents", "skills", name)),
];
const targets = process.platform === "darwin"
  ? [
      join(home, ".ferry"),
      join(home, "Library/Application Support", BUNDLE_ID),
      join(home, "Library/WebKit", BUNDLE_ID),
      join(home, "Library/WebKit/ferry"),
      join(home, "Library/Caches", BUNDLE_ID),
      join(home, "Library/Caches/ferry"),
      join(home, "Library/HTTPStorages", BUNDLE_ID),
      join(home, "Library/Saved Application State", `${BUNDLE_ID}.savedState`),
      ...skillTargets,
      join(home, ".local", "bin", "ferry"),
    ]
  : [
      join(home, ".ferry"),
      // Tauri 的应用数据在 Roaming,WebView2 用户数据在 Local。
      process.env.APPDATA && join(process.env.APPDATA, BUNDLE_ID),
      process.env.LOCALAPPDATA && join(process.env.LOCALAPPDATA, BUNDLE_ID),
      ...skillTargets,
      // 这个目录只由 Ferry 管理,连同 cmd 垫片一起移除。
      process.env.LOCALAPPDATA && join(process.env.LOCALAPPDATA, "Ferry"),
    ].filter(Boolean);

let removed = 0;
for (const path of targets) {
  // lstat 能看见已经断开的 skill 链接,existsSync 看不见。
  if (!lstatSync(path, { throwIfNoEntry: false })) continue;
  if (dryRun) {
    console.log(`将删除: ${path}`);
  } else {
    // Windows 杀进程后句柄释放可能略有延迟,让 Node 自带的重试处理短暂占用。
    rmSync(path, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
    console.log(`已删除: ${path}`);
  }
  removed += 1;
}

if (process.platform === "win32" && process.env.LOCALAPPDATA) {
  const cliDir = join(process.env.LOCALAPPDATA, "Ferry", "bin");
  if (dryRun) {
    console.log(`将从用户 PATH 移除: ${cliDir}`);
  } else {
    const script = [
      "$d = $env:FERRY_FRESH_CLI_DIR",
      "$p = [Environment]::GetEnvironmentVariable('Path', 'User')",
      "if ($null -ne $p) {",
      "  $parts = @($p -split ';' | Where-Object { $_ -and $_ -ine $d })",
      "  [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')",
      "}",
    ].join("; ");
    execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], {
      env: { ...process.env, FERRY_FRESH_CLI_DIR: cliDir },
      stdio: "ignore",
    });
    console.log(`已从用户 PATH 移除: ${cliDir}`);
  }
}

console.log(removed === 0
  ? "本机没有 Ferry 状态,已是全新环境"
  : dryRun
    ? `dry-run 结束,共 ${removed} 处待清理`
    : `清理完成,共 ${removed} 处;下次启动即首次启动向导`);
