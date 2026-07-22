<h4 align="right"><strong>English</strong> | <a href="./README.zh-CN.md">简体中文</a></h4>

<h1 align="center">
  <img src="./app/src-tauri/icons/icon.png" alt="Ferry" width="128" />
  <br>
  Ferry
</h1>

<p align="center">
  <strong>Unify, search, and migrate your coding agent sessions — all in one place.</strong>
</p>

<p align="center">
  Ferry brings together the conversation history of Claude Code, Codex CLI, and OpenCode
  into a single library. Browse thousands of sessions, migrate context between agents
  with an impact preview, and understand your token usage — privacy-first, no account required.
</p>

<p align="center">
  <a href="https://github.com/kzheart/ferry/releases"><img src="https://img.shields.io/github/v/release/kzheart/ferry?style=flat-square&labelColor=black&color=8b5cf6&logo=github&label=Release" alt="Release" /></a>
  <img src="https://img.shields.io/badge/built%20with-Tauri-8b5cf6?style=flat-square&labelColor=black&logo=tauri" alt="Tauri" />
  <a href="#download"><img src="https://img.shields.io/badge/macOS%20%7C%20Windows-supported-8b5cf6?style=flat-square&labelColor=black" alt="Platforms" /></a>
  <a href="./LICENSE"><img src="https://img.shields.io/github/license/kzheart/ferry?style=flat-square&labelColor=black&color=8b5cf6&label=License" alt="License" /></a>
  <img src="https://img.shields.io/github/last-commit/kzheart/ferry?style=flat-square&labelColor=black&color=8b5cf6&label=Last%20commit" alt="Last commit" />
</p>

<div align="center">
  <img src="./docs/screenshots/browser.png" alt="Ferry session browser" width="92%" />
</div>

---

## Table of Contents

- [Why Ferry](#why-ferry)
- [Supported Agents](#supported-agents)
- [Features](#features)
  - [Unified Session Library](#unified-session-library)
  - [Cross-Agent Migration](#cross-agent-migration)
  - [Usage Analytics](#usage-analytics)
  - [Session Editing](#session-editing)
- [Download](#download)
- [Development](#development)
- [Architecture](#architecture)
- [License](#license)

## Why Ferry

Coding agents keep their sessions in private stores — `~/.claude`, `~/.codex`,
OpenCode's local database. They can't see each other's history, and browsing them
means digging through JSONL files by hand.

Ferry solves three problems:

- **Unified library** — All agent sessions side by side, searchable by title, directory, or command, with tool calls, reasoning summaries, and session trees rendered in a single consistent view.
- **Cross-agent migration** — Move a conversation between agents with a migration impact preview upfront: see what maps natively, what gets downgraded, and what can't come along. Source sessions are never modified.
- **Usage insights** — Year-round activity view, cost by model and project, migration summaries, and insight cards that surface notable changes in your coding habits.

## Supported Agents

| Agent | Browse Sessions | Cross-Agent Migration |
| --- | :---: | :---: |
| Claude Code | ✓ | ✓ |
| Codex CLI | ✓ | ✓ |
| OpenCode | ✓ | ✓ |


## Features

### Unified Session Library

Browse every session from every agent in a single, consistent interface. Sessions are
grouped by recency and tagged with their source agent.

- **Search**: Hit `⌘K` to jump to any session by title, directory, or command.
- **Filter**: Narrow by source agent, time range, or project directory.
- **Scale**: Designed for large libraries — thousands of sessions stay responsive under click, scroll, and filter.
- **Session tree**: Full conversation topology — including subagent dialogues — with inline image preview.
- **Local metadata**: Rename, tag, and pin sessions without touching the originals. Deletions are backed up and undoable.

<div align="center">
  <img src="./docs/screenshots/search.png" alt="Command palette" width="88%" />
</div>

### Cross-Agent Migration

Move a conversation from one agent to another. Every agent stores sessions differently,
so migration is rarely lossless. Ferry shows you exactly what the cost is — _before_
anything is written.

- **Impact preview** — See what maps natively, what gets downgraded, and what drops, before you commit.
- **Native output** — Sessions are written in the target agent's own format.
- **Resume command** — Ferry hands back a terminal command to continue the conversation immediately.
- **Migration history** — Every migration is recorded, so you can trace where a session came from and what it cost to bring it across.

<div align="center">
  <img src="./docs/screenshots/migrate.png" alt="Migration impact preview" width="88%" />
</div>

<div align="center">
  <img src="./docs/screenshots/history.png" alt="Migration history" width="88%" />
</div>

### Usage Analytics

Understand your coding-agent habits over time:

- **Overview dashboard** — Total sessions, tokens consumed, estimated cost, and current streak.
- **Model breakdown** — Which models you've gravitated toward month over month.
- **Project breakdown** — Cost per project, with insight cards that surface notable changes (e.g., a repo whose spend jumped, a streak worth maintaining).
- **Activity heatmap** — A 52-week view of your daily coding activity.

<div align="center">
  <img src="./docs/screenshots/overview.png" alt="Overview dashboard" width="88%" />
</div>

<div align="center">
  <img src="./docs/screenshots/overview-detail.png" alt="Cost and project breakdown" width="88%" />
</div>

### Session Editing

Modify conversations before you resume them:

- **Delete turns** — Remove individual conversation rounds.
- **Rewrite messages** — Edit user prompts and AI responses in place.
- **Author replies** — Compose new AI responses and tool calls.
- **Safe by design** — Every change is previewed as a diff and backed up before application. Sessions can always be rolled back.

### More

- Auto-detects installed agents, available models, and local session data on startup
- Native macOS menu bar and sidebar vibrancy materials, following the system light/dark theme
- In-app updates with download progress and confirmation before install

## Download

[Download the latest release →](https://github.com/kzheart/ferry/releases/latest)

| Platform | File |
| --- | --- |
| macOS (Apple Silicon) | `Ferry_<version>_aarch64.dmg` |
| Windows (x64) | `Ferry_<version>_x64-setup.exe` |

> **macOS**: If the app is blocked on first launch, allow it under **System Settings → Privacy & Security**.

Ferry reads your agents' local session stores directly. Nothing is uploaded, and no account is required.

## Development

**Prerequisites**: Node.js 20+, Rust (stable), Python 3.12

The engine ships as a PyInstaller sidecar alongside the Tauri shell.

```bash
# 1. Build the Python engine sidecar
python -m pip install -r requirements-build.txt
python scripts/build-sidecar.py --clean

# 2. Install frontend dependencies and run
cd app
npm ci
npm run tauri dev
```

To produce a release bundle:

```bash
cd app && npm run tauri build
```

> Rebuild the sidecar whenever engine code changes — `npm run tauri build` bundles whatever binary is already present; it does not rebuild the sidecar.

## Architecture

| Layer | Technology | Role |
| --- | --- | --- |
| **Shell** | Tauri v2 (Rust) | Native window, menu bar, system tray, process management, in-app updates |
| **Frontend** | React 18 + Vite 6 | Session browser, search, editing, migration UI |
| **Engine** | Python 3.12 (PyInstaller sidecar) | Session scanning, read/write, migration logic, usage analytics |

The Tauri shell communicates with the Python engine via JSON-RPC over stdin/stdout.
Each coding agent is supported through a [plugin interface](./engine/adapters/base/plugin.py)
that abstracts session formats into a canonical model.

## License

[MIT](./LICENSE) © kzheart
