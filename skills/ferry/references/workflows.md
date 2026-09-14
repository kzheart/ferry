# Workflows

Each recipe assumes you already ran `ferry search` to pick the session; command details
live in [cli.md](cli.md).

## Archaeology — "how did we solve this last time"

```bash
ferry search flaky playwright timeout ci --limit 8
ferry read codex fsr_1a2b3c4d --terms playwright,timeout --limit 20
ferry read codex fsr_1a2b3c4d --from 40 --limit 20
```

Pick the most plausible one or two hits (recency plus `content_match_count`), use `--terms`
to locate the relevant messages, then read those pages in context mode. Cite session title,
agent, and date. If nothing matches, say so — do not synthesize a plausible past solution.

## Audit — what did that other agent actually do

```bash
ferry search migrate database schema --agent opencode --since 2026-08-01
ferry read opencode fsr_9f8e7d6c --from 1 --limit 25 --tool-outputs
ferry read opencode fsr_9f8e7d6c --from 1 --limit 25 --tool-outputs --cursor <next_cursor>
```

Page through with `next_cursor`, preserving the original `--from` and `--tool-outputs`,
and rebuild a timeline: user intent -> tool calls (name
plus key inputs) -> results -> what changed on disk. Note truncation explicitly when
`truncation.omitted_blocks > 0`; an audit that silently skipped output is worthless.

## Migration or resume elsewhere — move a conversation to another agent

There are two ways, and they are not interchangeable. **Ask the user which one**, unless
they already said. One line each:

- **Migration** — the full conversation tree is written into the target's native store, so
  `<tool> --resume` continues it as if it had always lived there. Highest fidelity, but the
  target must support being migrated into (Cursor does not), and some blocks degrade or drop.
- **Resume elsewhere** — the user starts the other agent themselves and asks it to pick the
  session up with its `ferry-resume` skill: it reads the history with `ferry read --inert`,
  writes its own summary, checks the repo, and carries on. Nothing is written into any store
  and it always works (including target = the same agent, for a fresh context), but what
  reaches the new session is the receiving agent's understanding, not the original
  transcript.

**If they pick migration:**

```bash
ferry search auth refactor --agent claude --limit 5
ferry migrate plan claude fsr_4d5e6f7a --to codex
```

Read `preview.preview.differences.counts`, then **stop and report to the user**:

> Migrating "Auth refactor" (Claude Code -> Codex CLI): 142 exact, 9 degraded (Read/Edit
> tool calls become history narration), 3 dropped (2 images, 1 thinking block). Your Claude
> Code session is not modified. Apply?

Wait for an explicit yes. The plan expires 10 minutes after `plan`, so confirm promptly.
Only then:

```bash
ferry migrate apply op_Xk29fQ7pLm3vT1sB
```

`apply` already waits for the terminal status, so a follow-up `migrate status` is only
needed to re-read a finished plan. Check `status == "applied"`, then hand the user
`result.resume.display_command` from the same output — the new session already has its
resume command there, so no extra `ferry resume` call is needed. Also check
`result.validation.structure.ok`; `result.rolled_back: true` means the write was reverted.
If the plan expired while waiting for approval, re-plan and re-confirm — never apply a plan
whose impact the user has not seen.

**If they pick resume elsewhere:** you run **no command for it at all**. Hand the user this
instruction to paste into the target agent:

```
/ferry-resume <tool> <session_id>
```

`<tool>` is the source agent (`claude`, `codex`, `opencode`, `pi`, `grok`, `cursor`) and
`<session_id>` is the **`session_id` field** of the `ferry search` result — the native id,
not the ephemeral `fsr_` ref, which would be dead in the other agent anyway. So for

```bash
ferry search auth refactor --agent claude --limit 5
# → sessions[0].session_id = "01a02803-9a5f-7b91-8610-37945d3b9478"
```

you hand over `/ferry-resume claude 01a02803-9a5f-7b91-8610-37945d3b9478`, or the same thing
in prose ("用 ferry-resume skill 接手 claude 会话 01a02803-…") for a harness without slash
commands. The target agent's `ferry-resume` skill trades that id for a fresh ref itself via
`ferry search --session-id`. Do not launch the other agent yourself.

**When migration is refused, offer that instruction as the fallback.** If `migrate plan`
fails because the target has no `migration-target` capability (Cursor is in this set), or
`differences.counts.dropped` is a large share of the session, say so and proactively give
the user the `/ferry-resume <tool> <session_id>` line as the alternative — then let them
decide. Do not silently switch.

## Digest — summarize a stretch of work

```bash
ferry search --project /Users/me/code/api --since 7d --limit 50
ferry read claude fsr_... --limit 40
```

Scope by project and time window, list the sessions, then read each one cheaply (tool
outputs off) and roll up into themes: what was attempted, what landed, what is unresolved.
Read deeply only into the sessions that matter. `--project` is an exact path match and
`--limit` maxes out at 50.

## Usage report

```bash
ferry usage --since 2026-07-01 --until 2026-08-01
ferry usage --since 30d --project /Users/me/code/api
ferry usage --agent claude,codex --since 2026-08-01
```

Report totals, then the interesting breakdown (`by_model`, `by_project`, `by_agent`).
Always label cost as an estimate and mention `unpriced_models` if it is non-empty.

## Cross-history experience audit

When asked for repeated tasks, recurring decisions, rework, or reusable workflows, read
[the audit workflow](history-audit.md). It defines inventory and paging, evidence
grades, event deduplication, counterexamples, and choosing exactly three priorities while
reusing installed capabilities. A digest of a few recent sessions is not an all-history audit.
