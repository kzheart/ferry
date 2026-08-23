<h4 align="right"><strong>简体中文</strong> | <a href="./README.en.md">English</a></h4>

<h1 align="center">
  <img src="./app/src-tauri/icons/icon.png" alt="Ferry" width="128" />
  <br>
  Ferry
</h1>

<p align="center">
  <strong>统一管理、搜索、迁移你的 Coding Agent 会话 —— 并把这些历史交还给 Agent 自己使用。</strong>
</p>

<p align="center">
  Ferry 将 Claude Code、Codex CLI、OpenCode、Pi Agent、Grok Build 和 Cursor
  的对话历史汇入同一个会话库。
  浏览上千条会话、跨 Agent 迁移上下文并预览迁移影响、掌握 Token 用量；
  再通过 <code>ferry</code> 命令行与配套 skill，让任何 Coding Agent 都能检索、续接、审计这些历史 ——
  隐私优先，无需注册账号，数据不离开本机。
</p>

<p align="center">
  <a href="https://github.com/kzheart/ferry/releases"><img src="https://img.shields.io/github/v/release/kzheart/ferry?style=flat-square&labelColor=black&color=8b5cf6&logo=github&label=Release" alt="Release" /></a>
  <img src="https://img.shields.io/badge/built%20with-Tauri-8b5cf6?style=flat-square&labelColor=black&logo=tauri" alt="Tauri" />
  <a href="#下载"><img src="https://img.shields.io/badge/macOS-supported-8b5cf6?style=flat-square&labelColor=black" alt="平台" /></a>
  <a href="./LICENSE"><img src="https://img.shields.io/github/license/kzheart/ferry?style=flat-square&labelColor=black&color=8b5cf6&label=License" alt="License" /></a>
  <img src="https://img.shields.io/github/last-commit/kzheart/ferry?style=flat-square&labelColor=black&color=8b5cf6&label=Last%20commit" alt="Last commit" />
</p>

<div align="center">
  <img src="./docs/screenshots/browser.png" alt="Ferry 会话浏览" width="92%" />
</div>

---

## 目录

- [为什么需要 Ferry](#为什么需要-ferry)
- [支持的 Agent](#支持的-agent)
- [功能](#功能)
  - [统一会话库](#统一会话库)
  - [跨 Agent 迁移](#跨-agent-迁移)
  - [续聊到其他 Agent](#续聊到其他-agent)
  - [在 Coding Agent 里使用 Ferry（CLI + Skill）](#在-coding-agent-里使用-ferrycli--skill)
  - [用量分析](#用量分析)
  - [会话编辑](#会话编辑)
- [下载](#下载)
- [开发](#开发)
- [架构](#架构)
- [许可证](#许可证)

## 为什么需要 Ferry

各个 Coding Agent 把会话锁在自己的私有存储里 —— `~/.claude`、`~/.codex`、
OpenCode 的本地数据库、Cursor 的 `state.vscdb`。它们彼此看不见对方的历史，
想查看就得手动翻 JSONL 文件；而 Agent 自己更是对"上周怎么解决的"一无所知。

Ferry 解决四个问题：

- **统一会话库** —— 所有 Agent 的会话并排展示，可按标题、目录、命令搜索，工具调用、推理摘要、会话树在同一套界面里呈现。
- **跨 Agent 迁移** —— 在 Agent 之间搬运对话，迁移前先展示影响：哪些原生保留、哪些会降级、哪些无法迁移。源会话全程只读。
- **让 Agent 用上自己的历史** —— 一条 `ferry` 命令加两个 skill，Claude Code、Codex 等任何 Agent 都能全文检索过往会话、接手另一个会话继续干活、审计别的 Agent 做过什么。
- **用量统计** —— 全年活跃度视图、按模型和项目拆分的成本、迁移方向汇总。

## 支持的 Agent

| Agent | 浏览会话 | 跨 Agent 迁移 | 续聊（ferry-resume） |
| --- | :---: | :---: | :---: |
| Claude Code | ✓ | ✓ | ✓ |
| Codex CLI | ✓ | ✓ | ✓ |
| OpenCode | ✓ | ✓ | ✓ |
| Pi Agent | ✓ | ✓ | ✓ |
| Grok Build | ✓ | ✓ | ✓ |
| Cursor | ✓ | ✓ | ✓ |

迁入 Cursor 会直接在它的 `state.vscdb` 里写出一条原生会话，续聊时模型能看见迁进去
的历史。两个前提：先完全退出 Cursor（运行中的 Cursor 会用内存态覆盖数据库），
以及目标文件夹至少在 Cursor 里打开过一次（会话按 Cursor 自己的工作区 id 归档）。
纯文本消息与终端/Shell 类工具调用原生迁移，其余工具调用与其它目标端一样写成历史
叙述文本。

## 功能

### 统一会话库

在一个统一的界面中浏览所有 Agent 的所有会话。会话按时间分组，标注来源 Agent。

- **搜索**：按 `⌘K`，通过标题、目录或命令跳转到任意会话。
- **筛选**：按来源 Agent、时间范围或项目目录缩小范围。
- **大规模**：为大会话库设计 —— 上千条会话的点击、滚动与筛选依然跟手。
- **会话树**：完整对话拓扑，包含子会话（subagent）对话，会话内图片可直接预览。
- **本地元数据**：重命名、打标签、置顶，不修改原始文件；删除自动备份，可随时撤销。

<div align="center">
  <img src="./docs/screenshots/search.png" alt="命令面板" width="88%" />
</div>

### 跨 Agent 迁移

把一个会话从一个 Agent 迁移到另一个。各家 Agent 存储格式不同，迁移很难无损。
Ferry 会在你确认之前把代价摆出来 —— *在写入之前*。

- **影响预览** —— 看清哪些原生保留、哪些降级、哪些丢失，再决定是否执行。
- **原生输出** —— 按目标 Agent 的原生格式写入会话。
- **接续命令** —— Ferry 会给出可直接粘贴到终端继续对话的命令。
- **迁移历史** —— 每次迁移都有记录，可追溯会话来源和迁移代价。

<div align="center">
  <img src="./docs/screenshots/migrate.png" alt="迁移影响预览" width="88%" />
</div>

<div align="center">
  <img src="./docs/screenshots/history.png" alt="迁移历史" width="88%" />
</div>

### 续聊到其他 Agent

原生迁移保真但有前提（目标格式可写、Cursor 必须退出）。**续聊**是另一条永远走得通的路：
不写入任何存储，由接手的 Agent 自己读历史、写摘要、核对仓库，然后接着干。

1. 在 Ferry 里右键会话 → **复制续聊指令**，得到一行
   `/ferry-resume <agent> <session id>`。
2. 粘贴到任何装了 `ferry-resume` skill 的 Coding Agent。
3. 它通过 Ferry 读取历史，写一份接手摘要（目标、已完成、未完成、停在哪），核对仓库
   状态后继续。

目标可以是**同一个 Agent**：Claude Code 的会话续聊到一个全新的 Claude Code 会话，
就是换一个干净的上下文接着做。迁移被拒绝时（例如 Cursor 还在运行），Ferry 也会把这条
指令作为兜底提供。

### 在 Coding Agent 里使用 Ferry（CLI + Skill）

**安装**：设置 → **Agent 集成** → 一键安装 `ferry` 命令和 skill（装到 `~/.agents/skills`，
Claude Code、Codex、OpenCode 等共同读取，包含 `ferry` 与 `ferry-resume` 两个 skill）。
无需 sudo；桌面 App 不在运行时命令也照常可用。

`ferry` 命令让 Agent 能检索、分页阅读、迁移会话和查看用量；skill 负责教它什么时候用、
怎么读、哪些事必须先征得你同意（比如迁移写入前要你确认影响汇总）。装好之后，下面这些话
可以直接对 Agent 说：

| 场景 | 对 Agent 说 | 背后发生了什么 |
| --- | --- | --- |
| **上次怎么解决的** | "上次 Playwright 超时是怎么修的？找找以前的会话" | 跨 Agent、跨项目全文检索，定位相关消息后分页读取，引用会话标题和日期作答；找不到就明说，不编造 |
| **伪无限上下文** | 上下文快满时开新会话，粘入 `/ferry-resume claude <id>` | 新会话从旧会话末尾读起，写接手摘要、核对仓库状态再继续 —— 不是压缩，而是换一个干净的上下文接着做 |
| **跨 Agent 接力** | 在 Claude Code 里做完规划，把续聊指令粘到 Codex："接着这个会话把方案实现掉" | 接手方只读历史，不碰任何存储；原生迁移不可行时的兜底 |
| **审计另一个 Agent** | "看看刚才 Codex 在这个项目里改了什么，有没有问题" | 重建时间线：用户意图 → 工具调用 → 结果 → 落盘改动 |
| **挖掘可沉淀的工作流** | "翻一下最近两周的会话，找出我反复让你做的事和反复踩的坑，整理成 skill 或 CLAUDE.md 规则" | 按项目和时间窗口列会话，低成本通读，归纳重复模式与失败模式 |
| **周报 / 项目复盘** | "汇总这个项目最近 7 天的会话：尝试了什么、落地了什么、还没解决什么" | 按项目和时间窗口列会话，按主题汇总，只对关键会话深读 |
| **给新机器 / 新成员补上下文** | "读一遍这个项目的历史会话，写一份项目上手笔记 / 初版 CLAUDE.md" | 从历史里提取架构决策、约定和踩坑，产出项目记忆文件 |

### 用量分析

长期追踪你的 Coding Agent 使用习惯：

- **总览仪表盘** —— 会话总数、Token 消耗、估算成本、连续活跃天数。
- **模型分布** —— 主力模型如何逐月变迁。
- **项目分布** —— 每个项目的成本一目了然。
- **活跃热力图** —— 52 周的每日编码活跃度一览。

<div align="center">
  <img src="./docs/screenshots/overview.png" alt="总览页" width="88%" />
</div>

<div align="center">
  <img src="./docs/screenshots/overview-detail.png" alt="成本与项目分布" width="88%" />
</div>

### 会话编辑

接续对话前先修改内容：

- **删除轮次** —— 移除单个对话轮次。
- **改写消息** —— 原地编辑用户提示词和 AI 回复。
- **替换助手回复** —— 通过同一条编辑操作链路替换 AI 回复及其有序工具调用。
- **安全设计** —— 每次修改以 diff 预览，应用前自动备份，会话随时可回滚。

### 更多

- 启动时自动检测已安装的 Agent 与本地会话数据
- 原生 macOS 菜单栏与侧边栏毛玻璃材质，跟随系统浅色/深色主题
- 有新版本时侧栏出现更新按钮，一键下载、安装并重启；重启后展示本次更新内容

## 下载

[下载最新版本 →](https://github.com/kzheart/ferry/releases/latest)

| 平台 | 文件 |
| --- | --- |
| macOS（Apple Silicon） | `Ferry_<version>_aarch64.dmg` |

> **macOS**：首次打开若被系统拦截，在 **系统设置 → 隐私与安全性** 中允许运行即可。

Ferry 直接读取本机 Agent 的会话存储，不上传任何数据，也不需要注册账号。

## 开发

**环境要求**：Node.js 22.19+、Rust（stable）；Python 3.12 仅用于仓库的构建与契约
生成脚本。

会话引擎（Rust）与 Ferry Runtime（Node.js）以原生 sidecar 的形式与 Tauri 外壳一起分发。

```bash
# 开发模式：构建原生引擎与编译后的 TypeScript runtime
cargo build --manifest-path crates/ferry-engine/Cargo.toml
cd ferry-runtime && npm ci
cd ../app && npm ci
npm run desktop
```

debug 宿主运行 `crates/ferry-engine/target/{debug,release}/ferry-engine`；
没有产物时直接报错并提示构建命令，不存在回退路径。

从仓库根目录打包完整的原生发布版本：

```bash
python scripts/build.py
```

复用已安装的 npm 依赖：

```bash
python scripts/build.py --skip-install
```

根构建会校验原生 target 与工具链，生成两个 sidecar，再调用 Tauri。sidecar 只为
`aarch64-apple-darwin` 或 `x86_64-pc-windows-msvc` 原生构建；交叉构建 sidecar 会被
明确拒绝。

仅前端开发：

```bash
cd app
npm run dev
```

## 架构

| 层 | 技术 | 职责 |
| --- | --- | --- |
| **桌面宿主** | Tauri v2 (Rust) | 原生能力、进程监督、IPC、审批与事件路由 |
| **前端** | React 18 + Vite 6 | 展示、局部交互状态、工作流进度与用户审批 |
| **会话引擎** | Rust（原生 sidecar） | 原生会话格式适配、全文索引、查询、迁移操作、快照与校验；同时为 `ferry` CLI 提供服务 |
| **`ferry` CLI + Skill** | Rust + Markdown | 引擎的薄客户端；`ferry` / `ferry-resume` 两个 skill 供 Coding Agent 读取 |
| **Ferry Runtime** | Node.js 22 + TypeScript | 实验性内置助手（默认关闭）：Provider、角色、LLM 工作流 |

Rust 宿主分别监督会话引擎和 Ferry Runtime 两个 sidecar。外部 Coding Agent 是
会话来源；内置助手是实验性功能，需在设置 → 实验性功能中手动开启。

详见[架构文档](./docs/architecture.md)、[CLI 与 skill 设计](./docs/cli-skill-design.md)
与[续聊设计](./docs/handoff-design.md)。

## 许可证

[MIT](./LICENSE) © kzheart
