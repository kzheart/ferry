<h4 align="right"><a href="./README.md">English</a> | <strong>简体中文</strong></h4>

<h1 align="center">
  <img src="./app/src-tauri/icons/icon.png" alt="Ferry" width="128" />
  <br>
  Ferry
</h1>

<h3 align="center">管理并迁移你的 Coding Agent 会话</h3>

<p align="center">
  在一个界面里浏览本机所有 Agent 的会话，
  并把一段对话从一个 Agent 搬到另一个，不丢掉已经积累起来的上下文。
</p>

<p align="center">
  <a href="https://github.com/kzheart/ferry/releases"><img src="https://img.shields.io/github/v/release/kzheart/ferry?style=flat-square&labelColor=black&color=8b5cf6&logo=github&label=Release" alt="Release" /></a>
  <a href="https://github.com/kzheart/ferry/releases"><img src="https://img.shields.io/github/downloads/kzheart/ferry/total?style=flat-square&labelColor=black&color=8b5cf6&logo=github&label=Downloads" alt="Downloads" /></a>
  <a href="#下载"><img src="https://img.shields.io/badge/macOS%20%7C%20Windows-supported-8b5cf6?style=flat-square&labelColor=black" alt="Platforms" /></a>
  <a href="./LICENSE"><img src="https://img.shields.io/github/license/kzheart/ferry?style=flat-square&labelColor=black&color=8b5cf6&label=License" alt="License" /></a>
</p>

<div align="center">
  <img src="./docs/screenshots/browser.png" alt="Ferry 会话浏览" width="92%" />
</div>

## 为什么需要 Ferry

每个 Coding Agent 都把会话锁在自己的存储里——`~/.claude`、`~/.codex`、
OpenCode 的本地数据库。它们彼此看不见对方的历史，而你想看，就得手动翻 JSONL 文件。

- **所有 Agent 共用一个会话库。** 你用的每个 Agent——目前是 Claude Code、Codex CLI、
  OpenCode——会话并排展示，可按标题、目录、命令搜索；工具调用、推理摘要、会话树
  都在同一套界面里呈现。
- **迁移会话，上下文不丢。** 迁移前就告诉你：哪些内容原生保留、哪些会降级、
  哪些无法迁移——*在写入之前*。源会话全程只读，不会被修改。
- **看清 token 花在哪。** 一整年的活跃度、按模型拆分的成本，以及背后的使用习惯。

## 功能

### 迁移前先看清损耗

各家 Agent 的会话格式不同，迁移很难无损。Ferry 会在你确认之前把代价摆出来，
按目标 Agent 的原生格式写入，并给出可以直接粘进终端的接续命令。

<div align="center">
  <img src="./docs/screenshots/migrate.png" alt="迁移损耗预览" width="88%" />
</div>

每次迁移都会留下记录，可以回溯一个会话从哪来、迁过来时付出了什么代价。

<div align="center">
  <img src="./docs/screenshots/history.png" alt="迁移历史" width="88%" />
</div>

### 看懂自己的用量

按模型统计的 token 与估算成本、52 周活跃热力图、主力模型随时间的变迁，
以及会话真正流向了哪些项目。

<div align="center">
  <img src="./docs/screenshots/overview.png" alt="总览页" width="88%" />
</div>

<div align="center">
  <img src="./docs/screenshots/overview-detail.png" alt="成本与项目分布" width="88%" />
</div>

### 接续之前先改一改

删除对话轮次、改写消息、原地补写回复。每次修改都会先以 diff 预览，
应用前自动备份，随时可以回滚。

### 其他

- 启动时自动检测已安装的 Agent、可用模型与本地会话数据
- 完整的会话树，包含子会话（subagent）对话
- 本地重命名、打标签、置顶、归档，不改动原始文件
- 应用内更新：显示下载进度，确认后再安装

## 下载

从 [Releases 页面](https://github.com/kzheart/ferry/releases/latest) 获取最新版本。

| 平台 | 文件 |
| --- | --- |
| macOS（Apple Silicon） | `Ferry_<version>_aarch64.dmg` |
| Windows（64 位） | `Ferry_<version>_x64-setup.exe` |

> macOS 首次打开若被系统拦截，在 **系统设置 → 隐私与安全性** 中允许运行即可。

Ferry 直接读取本机 Agent 的会话存储，不上传任何数据，也不需要注册账号。

## 开发

需要 **Node.js 20+**、**Rust**（stable）和 **Python 3.12**——
引擎以 PyInstaller sidecar 的形式与 Tauri 外壳一起分发。

```bash
# 1. 构建 Python 引擎 sidecar
python -m pip install -r requirements-build.txt
python scripts/build-sidecar.py --clean

# 2. 安装前端依赖并运行
cd app
npm ci
npm run tauri dev
```

打包正式版本：

```bash
cd app && npm run tauri build
```

引擎代码有改动时记得重新构建 sidecar——`npm run tauri build` 只会打包已存在的
二进制，不会替你重新构建它。

## 许可证

[MIT](./LICENSE) © kzheart
