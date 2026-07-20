import { execFile } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const appDir = dirname(fileURLToPath(import.meta.url));
const repo = resolve(appDir, "..");
// 版本单一来源:app/package.json(src-tauri/tauri.conf.json 的 version 也指向它)
const pkg = JSON.parse(readFileSync(resolve(appDir, "package.json"), "utf8"));

// 纯浏览器开发时的引擎桥:POST /api/rpc → python3 -m engine.api rpc
// (Tauri 环境下前端直接走 engine_rpc command,不经过这里)
const engineBridge = {
  name: "ferry-engine-bridge",
  configureServer(server) {
    server.middlewares.use("/api/rpc", (req, res) => {
      if (req.method !== "POST") { res.statusCode = 405; return res.end(); }
      let body = "";
      req.on("data", c => { body += c; });
      req.on("end", () => {
        execFile("python3", ["-m", "engine.api", "rpc", body],
          { cwd: repo, maxBuffer: 64 * 1024 * 1024 }, (err, stdout, stderr) => {
            res.setHeader("Content-Type", "application/json");
            if (err && !stdout) {
              res.end(JSON.stringify({ ok: false, error: String(stderr || err) }));
            } else res.end(stdout);
          });
      });
    });
  },
};

export default defineConfig({
  define: { __APP_VERSION__: JSON.stringify(pkg.version) },
  plugins: [react(), engineBridge],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
});
