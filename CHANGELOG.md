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

## [0.8.2] - 2026-08-24

### Removed

- **Runtime probes** — removed the optional probe workflow from contracts,
  engine execution, and the migration UI. Structure validation, write guards,
  and reading older migration history that references probes still work.

### Fixed

- **Loading spinner speed** — session parsing and rescan spinners no longer
  spin abnormally fast after `html[data-reduce]` was applied unconditionally
  and shortened every animation to 0.001s.

## [0.8.1] - 2026-08-23

### Changed

- **Session handoff always on** — copy-resume and `ferry-resume` are no longer
  behind an experimental toggle; the session context menu and migration fallback
  are available by default.
- **Handoff moved to Agent integration** — handoff settings now live under
  Settings → Agent integration instead of Experimental.
- **System UI fonts** — replaced bundled Geist variable fonts with system fonts
  (SF Pro and PingFang on macOS) so mixed Chinese/Latin text reads more evenly
  and the app bundle is smaller.
- **Reduced motion by default** — reduced motion is always enabled at startup;
  the toggle was removed.
- **Skills page actions** — skill import and delete use the StateButton
  pattern; delete goes through a confirmation dialog like session and migration
  record deletion.
- **Onboarding button style** — "Re-view onboarding" now matches the other
  buttons in its section.

### Removed

- **Session optimization** — removed the session optimization feature entirely,
  including its UI, runtime roles, and purpose.

### Fixed

- **macOS window controls** — traffic light buttons no longer jump during live
  window resize.
- **CI** — rustfmt alignment and a Windows build fix for an unused-import gate
  on `same_target` tests.

## [0.8.0] - 2026-08-23

### Added

- **Migrate into Cursor** — Cursor is now a migration target, not just a
  source. Ferry writes a native Cursor chat that the model can actually read:
  both the visible conversation and the model-facing context are written, so
  Cursor picks up where the other agent left off instead of starting cold.
  Plain messages and terminal/shell tool calls migrate natively; every other
  tool call becomes history narration, exactly like the other targets.
  Subagent conversations keep their parent/child topology. Two prerequisites:
  quit Cursor completely before migrating (a running Cursor overwrites its
  database from memory, and Ferry refuses to write while it is up), and open
  the destination folder in Cursor at least once so the chat is filed under
  the right workspace. Failed migrations roll back the chat and its subagents
  and touch nothing else in Cursor's database.
- **`ferry` command-line tool** — a `ferry` command for coding agents and
  scripts: search sessions, read them page by page, usage statistics, resume
  commands, migration plan/apply/status, migration history, and index
  refresh — all as JSON with the engine's error envelopes passed through.
  While the desktop app is running the CLI shares its engine; otherwise it
  starts a background daemon that exits on its own when idle. Install it from
  Settings → Agent integration (a symlink in `~/.local/bin`, no sudo).
- **Ferry skill for coding agents** — a bundled `ferry` skill teaches Claude
  Code, Codex, OpenCode, and any other agent that reads `~/.agents/skills` to
  use the CLI well: find how something was solved before, audit what another
  agent actually did, migrate a session with an impact summary you must
  confirm before anything is written, and report token usage. One-click
  install from Settings → Agent integration.
- **Resume a session in another agent** (experimental) — right-click a
  session and choose "Copy resume instruction", then paste
  `/ferry-resume <agent> <session id>` into any coding agent that has the new
  `ferry-resume` skill. That agent reads the history through Ferry with the
  other agent's system prompts and instruction wrappers stripped, writes a
  short takeover summary, checks the repository state, and continues in its
  own session — nothing is written to any agent's store, so it works even
  when a native migration is refused (for example while Cursor is running).
  The skill also accepts a plain-language description of the session. Turn it
  on in Settings → Experimental.
- **Experimental features** — Settings gains an Experimental section where
  optional features can be switched on per machine; switches only control
  visibility and never delete data.

### Changed

- **Workspace layout** — the navigation rail and resource pane were rebuilt.
  One toggle (⌘⇧S, or ⌘B from the menu) hides both at once; the icon-only
  collapsed rail is gone, the rail gets its own background, and migration
  history rows now match session rows.
- **Ask Ferry is now an experimental feature** — the built-in assistant is off
  by default. Turn it on in Settings → Experimental; when off, the assistant,
  its provider / model / role / skill settings, and the floating chat are
  hidden and its runtime is not started.
- **Engine methods renamed for all callers** — the engine's API no longer
  names a specific caller (for example `agent_session_read` is now
  `session_read`); the desktop app, the built-in assistant, and the CLI share
  one method set. No user-visible change.

### Fixed

- **Session titles survive migration** — a Claude Code session that never got
  an AI-generated title now carries the same title everywhere. Ferry's session
  list already fell back to the opening question, but migration did not, so
  such sessions arrived at the destination unnamed.
- **Honest migration reports** — the migration impact report no longer lists
  input fields as dropped for tool calls that get rewritten as history text.
  Those calls keep their name, input, and result inside the text that is
  written; only calls dropped outright list their fields now.
- **Migration failures say why** — the result card now shows the engine's
  actual reason (such as "Cursor is running: quit it before migrating")
  instead of an error class name, and distinguishes a migration that was
  rolled back after validation, one kept despite a failed probe, and one
  stopped before anything was written. Cursor's "must be quit" check now runs
  when the plan is made, so you no longer walk through every step before
  being told.
- **Fewer spurious "session reference is no longer valid" errors** — when a
  session changes underneath an operation, the engine re-resolves it and
  retries briefly instead of failing the action; when the error does surface
  it now says whether the session was deleted or modified.

### Performance

- **Lower idle footprint** — the engine does much less work while nothing is
  happening, and reading sessions (notably OpenCode and Codex) is faster.

## [0.7.0] - 2026-08-19

### Added

- **Cursor sessions** — Ferry now reads Cursor IDE's chat history alongside
  the other agents: browse and search every conversation (including subagent
  trees, tool calls, and edit patches), and migrate them to Claude Code,
  Codex CLI, or OpenCode. Cursor is a read-only source — Ferry never writes
  to Cursor's database, and change detection is content-derived so Cursor's
  own background writes don't trigger needless rescans. Token usage is not
  available in Cursor's store and shows as empty.

### Changed

- **One session engine** — Ferry now ships a single, native session engine.
  The original Python engine, which the native one was validated against
  during the rewrite, has been removed along with its packaging and
  cross-checking tooling.

### Performance

- **Native engine speedups** — measured on 2000 regular sessions (40 records
  each) plus 5 large sessions (400 records each) in the Claude JSONL format on
  macOS, median of multiple runs, against the previous Python engine: startup
  216 ms → 18 ms (11.9x), cold scan 3099 ms → 366 ms (8.5x), metadata search
  2345 ms → 402 ms (5.8x), agent search 2393 ms → 326 ms (7.3x), regex search
  3916 ms → 336 ms (11.7x), opening a large session 327 ms → 38 ms (8.6x),
  usage aggregation 2370 ms → 256 ms (9.3x). Warm scan was already dominated
  by cache reads and is unchanged (22 ms → 33 ms). Content index construction
  is outside the measured window; both engines used the same SQLite FTS5
  mechanism.

### Fixed

- **Fingerprint rebuilds on a busy OpenCode database** — when another program
  was writing OpenCode's database continuously, a scan could rebuild the
  fingerprint index once per session and never finish. Rebuild results are now
  published even when the database moved underneath, so a scan pays for at
  most one rebuild.

## [0.6.1] - 2026-08-05

### Added

- **Choice cards in Ask Ferry** — the assistant can now ask multiple-choice
  questions directly in the conversation; pick an option (with optional notes)
  to answer inline. Answered cards collapse into a one-line summary and can be
  expanded again on click.
- **Batch session deletion** — delete up to 100 sessions in one operation: the
  preview freezes the full list, a single approval executes it, and each
  session reports back as succeeded, skipped, or failed.

### Changed

- **Permanent deletion always asks** — deleting sessions now always shows an
  approval card, even when an automatic approval policy is active, since
  deletion is irreversible.

### Removed

- **Overview insight cards** — the auto-generated insight section (cost
  spikes, idle repos, streaks, and similar cards) has been removed from the
  Overview page.
- **Deletion recovery snapshots** — deleted sessions are no longer snapshotted
  for undo; deletion is final. The separate cleanup pipeline was folded into
  the atomic batch deletion above.

### Fixed

- **Readable engine errors** — unmapped engine error codes no longer surface
  as raw identifiers in the UI; agent reference errors now come with clear
  messages and recovery hints in both languages.
- **Codex session editing** — editing no longer fails on sessions that contain
  developer or system messages, which most Codex sessions do.
- **Disabled button hover** — the disabled primary button in Ask Ferry no
  longer turns into an unreadable white block on hover in the light theme.

### Performance

- **Calmer live indexing** — session-store rescans now wait for changes to
  settle instead of re-scanning on every write, cutting rescan work during
  heavy activity to a fraction; deletions are evicted from the index instantly
  so the UI reflects them without a full rescan.

## [0.6.0] - 2026-07-31

### Added

- **Pi Agent and Grok Build support** — browse, search, resume, and migrate
  sessions from two more coding agents, with the same read/edit/migration
  safeguards as the existing tools.
- **Live session library** — the engine now watches agent session stores and
  pushes incremental updates, so the session list stays current while agents
  are running, without pressing refresh.
- **Native Coding Agent driving** — roles can allowlist `agent_prompt` to resume
  and actively drive Claude Code, Codex CLI, OpenCode, Pi Agent, or Grok Build
  sessions with the Agent's native high-privilege execution mode. The role
  allowlist is the authorization, so calls do not require per-run Ferry
  approval; each started run returns a fresh session reference for follow-up
  calls. This is separate from Ferry Provider completions and does not change
  the existing fixed-prompt probe behavior.
- **Jump to latest** — session details of active sessions offer a one-click
  jump to the newest messages, loading any remaining pages on the way down.
- **Skills for roles** — import skills from local agent skill libraries and
  attach them to roles; shell commands run through the existing approval flow.
- **Full-text session search** — search across session content from the
  command palette, including regex and multi-pattern queries.
- **Session optimization (experimental)** — review AI-suggested rewrites of
  your prompts as inline diffs; off by default behind a test-features toggle.
- **Automatic session titles** — Ask Ferry sessions name themselves after the
  first reply; double-click any title to rename it inline, in place of the old
  rename dialog.
- **Seamless message paging** — session details load older messages
  automatically as you scroll, instead of stopping at a page boundary.
- **Scan progress** — data-source scanning shows live per-tool progress in
  Settings.
- **Expanded onboarding** — the feature tour now covers the whole app in nine
  steps.

### Changed

- **Ask Ferry conversation view** — tool activity condenses into a compact
  timeline with in-place previews, messages support copy and edit-resend, the
  role picker moved into a header capsule, and a breathing indicator shows
  while the assistant is thinking.
- **Roles settings redone** — built-in roles are editable and restorable,
  skills and shell access appear on capability cards, and importable sources
  fold away until needed.
- **Migration diff cards** — differences now show source-to-target call
  mappings and name the parameters that would be lost, with clearer failure
  reasons.
- **Custom providers** — OpenAI- and Anthropic-compatible endpoints with
  automatic model discovery, connection testing, and API-key visibility
  toggle.
- **Keyboard-first navigation** — the session list and command palette are
  fully keyboard-navigable, with visible focus styles throughout.
- **Unified diagnostics** — all three processes now log to `~/.ferry/logs`.
- **App data location** — internal state moved to `~/.ferry`, separated from
  backup snapshots.

### Removed

- **Multi-agent delegation** — the experimental delegate-agents workflow was
  removed.
- **AI session organizing** — the session organizing workflow was removed.

### Fixed

- **Stable session references** — opening a session that its agent is actively
  writing no longer intermittently fails with `reference_invalid`, and the
  session list no longer shows ghost duplicate rows; session handles now stay
  stable across rescans.
- **Compaction markers** — context-compaction cards anchor to the real
  preceding message instead of drifting to the top of the session.
- **Resume feedback** — terminal-resume and copy-command failures now surface
  on the button instead of being silently swallowed.
- **Migration resilience** — a failing post-migration probe no longer rolls
  back output that already passed structural validation, and Unicode line
  splitting in JSONL sessions is handled correctly.
- **Ask Ferry reliability** — auto-approved actions no longer flash approval
  cards, duplicate plans are deduplicated, and message sending no longer
  fails with an internal runtime error.
- **Visual glitches** — Grok sessions no longer show blank icon circles, the
  sidebar no longer flashes on first frame, and a window-resize recursion
  crash on macOS is guarded.

### Performance

- **Faster busy-library scans** — per-file parse caching for Codex sessions,
  OpenCode fingerprint rebuilds moved off the hot path, parallel line-based
  JSONL scanning, a shared scan cache with persisted content digests, and
  millisecond session-list loads served from the in-memory live index.
- **Opening sessions during scans** — read requests dispatch in parallel with
  scanning, so session details open without queueing behind a rescan.
- **Smoother Ask Ferry streaming** — streamed replies batch per frame and the
  timeline memoizes, instead of re-rendering on every token.

## [0.5.0] - 2026-07-23

### Added

- **Ask Ferry assistant** — a built-in, provider-configurable AI workspace for discussing sessions, inspecting tool activity, and preparing safe session changes.
- **Agent-assisted session editing** — edit supported source sessions in place or as a copy, with explicit preview/confirmation and references that keep follow-up work anchored to the right conversation.
- **Migration preview and safeguards** — five-level fidelity previews, migration history management, safe preflight checks, and stronger preservation of ordering, tool calls, and subagent relationships across Claude Code, Codex, and OpenCode.
- **Context-compaction timeline** — session details now show compression boundaries, summary availability, trigger state, token changes, and the point where the live context resumed.
- **Session productivity controls** — resume commands for the terminal, copyable session commands, long-message folding, refresh-in-place, draggable sidebar navigation, and configurable terminal preferences.
- **Provider and model management** — dedicated model visibility controls, custom models, connection testing, dynamic model discovery, and Pi OAuth provider support.

### Changed

- **macOS-only distribution** — this release ships for Apple Silicon macOS; Windows installers and updater packages are temporarily unavailable.
- **Cross-agent session model** — unified tool-operation mapping and adapter contracts make session browsing, editing, and migration more consistent across supported agents.
- **Migration interface** — impact information is grouped into clearer cards and proportional indicators, with a more focused detail layout.
- **Settings experience** — theme and language controls now use compact native-style selectors; provider configuration is organized around each provider.

### Fixed

- **Reliable migration output** — fixed cross-tool message ordering, session-boundary handling, child-session association, and validation of restored sessions.
- **Safer runtime discovery** — strengthened CLI probing and session-reference handling so available tools and recovery commands match the installed runtime.

### Performance

- **Faster startup and navigation** — warmed engine startup and cached first paint reduce initial wait time, while large session details avoid rendering long message bodies until needed.

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
