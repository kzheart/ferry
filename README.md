# Ferry

Ferry 是 Claude Code、Codex CLI 和 OpenCode 的桌面会话管理工具。它可以浏览会话、在不同 Agent 之间迁移上下文，以及删除轮次、裁剪工具输出或改写会话内容。

## 使用

安装发布页提供的 macOS 或 Windows 构建后启动 Ferry。应用会读取本机已安装 Agent 的会话；执行迁移或编辑时会创建新会话或备份，避免直接破坏原会话。

## 源码开发

需要 Node.js 22、Rust 和 Python 3.12：

```sh
cd app
npm install
npm run tauri dev
```

调试构建缺少 bundled sidecar 时会从仓库运行 Python 引擎。仓库位置默认从 `app/src-tauri` 推导，也可通过 `FERRY_REPO` 指定。前端构建可运行 `npm run build`，Rust 壳可在 `app/src-tauri` 中运行 `cargo check --locked`。

## Sidecar 构建

生产包使用 PyInstaller 冻结 Python 引擎，不依赖用户安装 Python。构建脚本支持 `aarch64-apple-darwin` 和 `x86_64-pc-windows-msvc`：

```sh
python3.12 -m pip install -r requirements-build.txt
./scripts/build-sidecar.sh
./dist/ferry-engine health
./dist/ferry-engine rpc '{"method":"version"}'
./dist/ferry-engine rpc '{"method":"env"}'
```

也可直接使用源码入口：`python3 sidecar.py health` 或 `python3 -m engine.api health`。

## 发布签名

Release workflow 始终要求 `TAURI_SIGNING_PRIVATE_KEY` 和 `TAURI_UPDATER_PUBLIC_KEY`，私钥密码可留空；updater 包及 `latest.json` 中的签名不会因系统签名缺失而关闭。Apple Developer ID 与 Windows Authenticode secrets 是可选的，但必须分别完整配置才会启用对应平台的证书导入和系统签名。未配置时仍可发布，macOS Gatekeeper 或 Windows SmartScreen 可能向用户显示警告。
