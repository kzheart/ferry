# Changelog

All notable changes to Ferry are documented in this file. Every release adds one
`## [x.y.z] - YYYY-MM-DD` section here; the release workflow publishes that
section — and only that section — as the GitHub Release body and as the in-app
updater notes (`python scripts/release.py notes --version x.y.z`). A release
fails validation if its version has no section.

### Changelog writing guidelines

- **Language** — always English.
- **Structure** — group changes under `### Added`, `### Changed`, `### Removed`, `### Fixed`, `### Performance`.
- **Content** — be concise but descriptive. Each entry starts with a bold **short name** followed by a description, never a raw git commit subject.
- **Audience** — write for users, not contributors. Explain what changed and why, not how.
- **Scope** — one entry per logical change, not per commit. Merge related commits into a single entry.

## [Unreleased]

## [0.4.0] - 2026-07-21

### Added

- **Session image preview** — inline preview for images referenced in session messages.
- **Refresh button in session detail** — re-reads only the current session without a full rescan.
- **macOS native look & feel** — Rust-side native menu bar (Ferry / Edit / View / Window), vibrancy sidebar material, deep dark color scheme, compact sidebar with icon toolbar and ⌘K command palette.

### Changed

- **Settings language picker** — redesigned from radio cards to a native select dropdown, driven by locale metadata (`LOCALE_META`).
- **Narration always in English** — removed the language toggle on the migration confirm screen; narration is fixed to English since target agents read context, not UI text.

### Removed

- **Snapshot restore page** — the dedicated restore page has been removed.
- **Archive feature** — entire archive flow removed, replaced with inline delete + undo.

### Fixed

- **Window drag not working** — added missing `core:window` permissions (`start-dragging`, `toggle-maximize`, `set-theme`) that Tauri v2 silently denied.
- **Schedule chart legend overlap** — moved legend below the polar chart to fix text/swatch collisions; default window reduced from 1440×960 to 1120×760.
- **Duplicate pin badge on hover** — pin badge hidden on hover to avoid collision with the pin button.
- **Codex & OpenCode native resume** — fixed session continuation for migrated Codex and OpenCode sessions.

### Performance

- **Sidebar click & session switching** — row component memoized, content-visibility for off-screen rows, LRU cache for session detail, edit capability cached per tool. Click latency on 3000+ sessions dropped from ~200ms to <30ms.
- **Large list virtualization** — fixed row height + virtual DOM mount (visible ±300px), zero-recalc expand/collapse. Thousands of grouped rows no longer build full DOM on expand.
- **Filter popover anchoring** — popover now anchored below the filter button, right-aligned, clamped to window bounds. Pre-computed search index avoids rebuilding time/label strings for 3000+ rows on every filter change.

## [0.3.1] - 2026-07-21

### Fixed

- **CLI detection in packaged builds** — apps launched from Finder/Dock inherit
  launchd's minimal `PATH`, so the packaged app could scan session files yet
  report claude / codex / opencode as "not installed" in onboarding and the
  migration sheet. The Tauri shell now restores the login-shell `PATH` via
  `fix-path-env` before spawning the engine, and the engine resolves each CLI
  to an absolute path with `shutil.which` plus a fallback scan of common
  install locations (`~/.local/bin`, `~/.npm-global/bin`, `~/.bun/bin`,
  `~/.volta/bin`, `~/.opencode/bin`, Homebrew, nvm versions, `%APPDATA%\npm`).
  When a CLI is found via the fallback scan its directory is prepended to the
  engine's `PATH`, so runtime shims (e.g. codex's `#!/usr/bin/env node`) keep
  working; the resolved path is reused by probes, model discovery, and session
  commands so "detected as installed" and "actually runnable" can no longer
  diverge.
- **Windows CLI execution** — npm installs CLIs as `.cmd` shims that
  `CreateProcess` cannot launch by bare name; resolving through `shutil.which`
  (which honors `PATHEXT`) fixes detection and execution, and engine
  subprocesses now run with `CREATE_NO_WINDOW` so no console windows flash.
- Environment inspection now reports `path` (resolved executable) and `broken`
  (found but `--version` fails, e.g. unsupported Node) alongside `installed`.

## [0.3.0] - 2026-07-21

### Added

- **Overview page** — a new top-level view that aggregates every scanned session
  into KPIs, token composition, an estimated cost table, working-hour patterns, a
  52-week contribution heatmap, repository rankings, migration flows, model rank
  shifts, and rotating insights. Charts are hand-written inline SVG that follow
  the active theme, with a GitHub-style green heatmap for light and dark modes
  and a per-agent filter driven by the engine's `tools` RPC.
- **Token and cost analysis in the engine** — all three scanners now parse and
  accumulate token usage, the dominant model, and creation time, normalized to a
  single `input` / `output` / `cache_read` / `cache_write` shape. Claude reads
  `message.usage`, Codex derives cache hits from its cumulative `token_count`,
  and OpenCode aggregates from the `message` table instead of the incomplete
  session rollup columns.
- **Pricing service** — a new use case and RPC that fetches unit prices from
  models.dev, flattens them, caches them on disk for 7 days, and falls back to a
  bundled table when offline.
- **AI reply and tool-call authoring** — sessions can now be extended with
  orchestrated assistant replies and tool calls, not just edited in place.
- **Internationalization** — the entire UI is now translatable via i18next, with
  Simplified Chinese and English bundled. Language follows the system locale by
  default and can be overridden in Settings. Locale files are split into
  per-feature namespaces (`common`, `app`, `browser`, `migration`, `snapshots`,
  `onboarding`, `settings`, `overlays`, `errors`, `events`, `overview`) with a
  contributor guide in `app/src/locales/README.md`.

### Changed

- **Plugin architecture** — each agent is now a `ToolPlugin` (manifest plus seven
  capability fields) assembled through a registry factory. The application layer
  contains no per-agent special cases; a read-only fake plugin is enough to pass
  the contract tests.
- **Unified turn model** — every agent has a single `TurnIndex` and
  `NativeEditCodec`, so reading, deleting, and replacing share one locator
  semantic, locked down by contract tests across all three agents.
- **RPC v2 error envelope** — errors carry `code` / `params` / `category` /
  `retryable` and are rendered from a code table on the front end instead of
  pre-translated strings.
- **Structured events** — loss, notes, and warnings are emitted as code + params
  events, and snapshot reasons moved to `reason_code` with dual-read/single-write
  compatibility for existing data.
- **Probe results** are now structured as `{status, code, params, diagnostic,
  isolation}`; stdout/stderr are treated as opaque diagnostics and are never
  translated.
- **Versioned narration** — the `historical-tool-call-v1` template ships in both
  zh-CN and en, and `content_locale` travels with the migration request so
  injected content is decoupled from the UI language.
- **Manifest as the single source of truth** — the front end hydrates tool
  metadata through the `tools` RPC at startup, resume commands are produced by the
  engine as launch descriptors, and the Rust side validates executables against
  the manifest allowlist instead of accepting a command assembled by the UI.
- Scan cache bumped from version 5 to 6 to force a re-parse that picks up the new
  usage fields.

### Removed

- Three editor facade layers, the OpenCode reader/writer forwarding shims, the
  legacy CLI dispatch path, the Rust `TerminalTool` enum, and assorted dead code.

### Fixed

- Missing identity colors (`--t-claude`, `--t-codex`, `--t-opencode`) that left
  the stacked repository ranking bars unpainted.
- Layout gaps in the overview insight area where a featured card stretched an
  otherwise empty track.

## [0.2.0] - 2026-07-20

- Session right-click menu, delete-to-trash with undo, and quick editing.
- Session metadata, manual snapshots, keyboard shortcuts, and multi-select batch
  operations.
- Instant session detail loading via direct SQLite reads, index caching, and a
  resident engine process.
- Inline rewriting of assistant messages in the original bubble.
- Cross-platform release pipeline with optional system signing.

## [0.1.0]

- Initial release.
