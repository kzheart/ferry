# AI-generated titles — `ferry title`

`ferry title` regenerates session titles in a consistent, user-configured style and writes
them back with the same native rename path as `ferry rename`. Every subcommand except
`style` addresses sessions as **tool/ref groups**: a token that is an agent id (`claude`,
`codex`, `opencode`, `pi`, `grok`, `cursor`) switches the current tool, every other token
is an `fsr_` ref of that tool, so one command can mix agents:

```bash
ferry title suggest claude fsr_aaa fsr_bbb codex fsr_ccc
```

- `ferry title style [--file PATH]` — print the user's style (`result.style`), or replace
  it from a JSON file. Fields: `preset` (`ferry` default = emoji type prefix + verb-first
  statement; `english` = lowercase English label, no prefix; `bracket` = `[type] body`;
  `custom` = only `instructions`/`examples`), `language` (`follow` the user's own
  language in the session, or fixed `zh` / `en`), `max_chars` (body length cap, default
  16), `type_prefix`, `types[]` (`emoji`, `zh`, `en`), `instructions`, `examples[]`.
  The style is Ferry-wide: the desktop app and the CLI read the same file.
- `ferry title evidence <groups>` — read-only. Returns `sessions[]` with the current
  `title`, `title_source`, `project`, `message_count`, the first user messages, the last
  assistant message, touched `files`, and `rename_capability`, plus `errors[]` for refs
  that did not resolve and `style` (the same object as `title style`). Use this when
  **you** are going to write the titles yourself: follow `style` exactly, show the user
  an old → new table, then write back one by one with `ferry rename` (or, for Cursor,
  direct them to the desktop app).
- `ferry title suggest <groups> [--include-manual]` — asks Ferry's own model (the one
  configured for Ask Ferry in the desktop app) for titles. Returns `items[]` with
  `before`, `title`, `type`, `skip`, `reason`, plus `model` and `skipped`. Nothing is
  written. Needs the Ferry runtime: if the desktop app has no model configured the
  command fails with `provider_unavailable` — tell the user to configure one under
  **Settings -> Models**, do not substitute your own guess silently.
- `ferry title reset <groups> [--include-manual] [--apply]` — `suggest` plus write-back.
  Without `--apply` it prints the preview only; with `--apply` it renames every item whose
  `skip` is false and adds `status`, `native` (with the usual `notes`) or `error` per
  item. Evidence errors and skipped items are preserved in the final result; exit 0 only
  when evidence collection and every applied item succeeded.
- `ferry title apply --file PATH|-` — batch write-back of a JSON array
  `[{"tool","ref","title"}]`, for titles the user edited after a preview. Same per-item
  output as `reset --apply`.

`title_source` tells you where the current title came from and drives the default skip
rule: `manual` (a Ferry-local name override, Claude `/rename`, or a Grok manual title)
is **skipped unless `--include-manual`**; Ferry-local names take precedence in the preview.
For the other sources, `native` (the agent's own store, auto or manual
unknown), `derived` (first message truncated) and `empty` are fair game. Sessions with
fewer than 3 messages come back as `skip: true, reason: "too_short"`; `too_long` and
`duplicate` (same title already proposed inside the same project) are the other reasons.

Rules for this command family:

1. **Preview first, always.** Show the user the old → new list (`reset` without
   `--apply`, or `suggest`) and get an explicit yes before `--apply` / `apply`. Writing a
   title makes the agent treat it as a manual name (Claude, Grok), which cannot be undone
   by Ferry.
2. Batch scope is what the user named — a project, a time window, "these five". Resolve it
   with `ferry search` first and pass the resulting refs; never sweep the whole library
   because the user said "my sessions look messy".
3. Relay `native.notes` (Codex / OpenCode restart hints) and every non-applied item with
   its `reason` or `error`. Do not re-run failed items blindly; a `session_changed` error
   means re-search for a fresh ref.
4. Cursor has no native rename: `reset --apply` and `apply` store the title as Ferry-local
   metadata, which only Ferry shows. Say so when Cursor sessions are in the batch.
