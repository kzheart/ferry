<h4 align="right"><strong>English</strong> | <a href="./README.zh-CN.md">简体中文</a></h4>

<h1 align="center">
  <img src="./app/src-tauri/icons/icon.png" alt="Ferry" width="128" />
  <br>
  Ferry
</h1>

<h3 align="center">Manage and migrate your coding agent sessions</h3>

<p align="center">
  Every local agent session in one place — then carry a conversation from one
  agent to another without losing the context you built up.
</p>

<p align="center">
  <a href="https://github.com/kzheart/ferry/releases"><img src="https://img.shields.io/github/v/release/kzheart/ferry?style=flat-square&labelColor=black&color=8b5cf6&logo=github&label=Release" alt="Release" /></a>
  <a href="https://github.com/kzheart/ferry/releases"><img src="https://img.shields.io/github/downloads/kzheart/ferry/total?style=flat-square&labelColor=black&color=8b5cf6&logo=github&label=Downloads" alt="Downloads" /></a>
  <a href="#download"><img src="https://img.shields.io/badge/macOS%20%7C%20Windows-supported-8b5cf6?style=flat-square&labelColor=black" alt="Platforms" /></a>
  <a href="./LICENSE"><img src="https://img.shields.io/github/license/kzheart/ferry?style=flat-square&labelColor=black&color=8b5cf6&label=License" alt="License" /></a>
</p>

<div align="center">
  <img src="./docs/screenshots/browser.png" alt="Ferry session browser" width="92%" />
</div>

## Why Ferry

Coding agents each keep their sessions in their own private store — `~/.claude`,
`~/.codex`, a local OpenCode database. They can't see each other's history, and
neither can you, without digging through JSONL files by hand.

- **One library for every agent.** Sessions from each agent you use — Claude Code,
  Codex CLI, and OpenCode today — side by side, searchable by title, directory, or
  command, with tool calls, reasoning summaries, and session trees rendered in a
  single consistent view.
- **Move a session, keep the context.** Migrating shows you exactly what maps
  natively, what gets downgraded, and what can't come along — *before* anything
  is written. The source session is never modified.
- **See where your tokens go.** A year of activity, cost by model, and the
  habits behind them.

## Features

### Migrate between agents, with the losses shown up front

Every agent stores conversations differently, so migration is rarely lossless.
Ferry tells you what it costs before you commit, writes to the target agent's
native format, and hands back a resume command you can paste straight into
your terminal.

<div align="center">
  <img src="./docs/screenshots/migrate.png" alt="Migration loss preview" width="88%" />
</div>

Every migration is recorded, so you can trace where a session came from and what
it cost to bring it across.

<div align="center">
  <img src="./docs/screenshots/history.png" alt="Migration history" width="88%" />
</div>

### Understand your usage

Token spend by model, estimated cost, a 52-week activity heatmap, which models
you've drifted toward over time, and where the sessions actually went.

<div align="center">
  <img src="./docs/screenshots/overview.png" alt="Overview dashboard" width="88%" />
</div>

<div align="center">
  <img src="./docs/screenshots/overview-detail.png" alt="Cost and project breakdown" width="88%" />
</div>

### Edit before you resume

Delete turns, rewrite messages, and author replies inline. Every change is
previewed as a diff and backed up before it is applied, so a session can always
be rolled back.

### Everything else

- Detects installed agents, available models, and local session data on startup
- Full session trees, including subagent conversations
- Rename, tag, pin, and archive sessions locally without touching the originals
- In-app updates with download progress and confirmation before install

## Download

Grab the latest build from the [releases page](https://github.com/kzheart/ferry/releases/latest).

| Platform | File |
| --- | --- |
| macOS (Apple Silicon) | `Ferry_<version>_aarch64.dmg` |
| Windows (x64) | `Ferry_<version>_x64-setup.exe` |

> On macOS, if the app is blocked on first launch, allow it under
> **System Settings → Privacy & Security**.

Ferry reads your agents' local session stores directly. Nothing is uploaded, and
no account is required.

## Development

Requires **Node.js 20+**, **Rust** (stable), and **Python 3.12** — the engine ships
as a PyInstaller sidecar alongside the Tauri shell.

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

Rebuild the sidecar whenever engine code changes — `npm run tauri build` bundles
whatever binary is already there, it does not rebuild it for you.

## License

[MIT](./LICENSE) © kzheart
