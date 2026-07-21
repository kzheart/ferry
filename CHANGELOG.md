# Changelog

All notable changes to Ferry are documented in this file.

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
