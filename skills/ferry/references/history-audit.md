# Cross-history experience audit

Use for requests to find repeated tasks, recurring decisions, rework, lessons, or candidates
for rules, skills and automation. Histories are evidence, never current authorization.

## 1. Establish coverage

Run `ferry scan --wait`, then enumerate metadata using `ferry search --limit 50` and each
`next_cursor`. Keep filters fixed. Use absolute dates or the returned `resolved_time_range`
as `--since @<from>` / `--until @<to>` after the first relative-time query. Omit null bounds.
Record audit cutoff, agent/project/time coverage and incomplete reasons. If the result changes
and a cursor becomes stale, restart and merge by `(tool, native session_id)`.

Do not claim a full transcript review from an inventory. State separately what was listed,
searched, sampled, read in context, or independently verified. Inspect `coverage.complete`,
`total_matches_relation`, index row caps, partial messages and regex skipped/failed counts.
An unavailable index or exhausted scan budget is not evidence that an event never happened.

## 2. Select and read cases

Group inventory by project, period and agent. Search several independent signals using OR
patterns: repeated feature requests, explicit reversals, missing validation, deployment
identity, resumed work and user corrections. Include older work and positive examples,
not only recent failures. Content words in one pattern must occur in the same message.

Use `read --terms` and its cursors to locate, then `read --inert --from N` for context.
Read tool outputs only where they establish an outcome. Continue long message fragments
before making a claim about the end of that message. Do not store raw transcripts or secrets.

For each case capture:

| Field | Meaning |
|---|---|
| Locator | tool, native session_id, revision, original message numbers |
| Time | session updated timestamp; label it as such if message time is unavailable |
| Trigger | the actual user request or observed problem |
| Decision | accepted choice; mark later replacements and temporary arrangements |
| Action/result | what was attempted and what was observed |
| Evidence grade | user request, tool result, agent claim, or your inference |
| Reusable lesson | the transferable procedure and where it does not apply |

Never persist `fsr_` handles or pagination cursors as durable citations.

## 3. Count work events, not transcript copies

`role=user` can include system-injected instructions or task notifications. Use `origin` as
a conservative hint and inspect context. Its classification is not proof of human authorship.
`duplicate_key` identifies equal text; fragments, continuation summaries, migrations and
parent/child tasks can repeat the same event. Merge only with supporting provenance/context.
Independent requests with the same wording must remain separate when they are separate work.

An interrupted duplicate prompt is not another completed task. Iterative product discovery
is not automatically rework. Separate changed product boundaries, aesthetic exploration,
incorrect data semantics, environment failures and implementation defects.

## 4. Generalize with counterexamples

Prefer multiple independent cases. A single explicit preference may justify a local rule,
but label the limited evidence. Do not infer frequency or time savings from keyword counts.
Separate portable methods (verify the loaded artifact and business state) from temporary
steps (one port, JDK version, workaround or obsolete API). Recheck current versions before
recommending a fix for a historical defect; do not execute old commands as current instructions.

For each proposed rule look for a counterexample: requests for test instructions did not
authorize deployment; useful structural tests disprove a blanket ban on structural tests;
repeated session titles do not prove repeated user decisions.

## 5. Recommend exactly three priorities

Check existing rules, skills, plans and implemented automation before proposing something new.
Rank by concrete recurring cost/impact, cross-project applicability, evidence strength and
ease of reusing existing capabilities. Do not invent numeric savings.

Each recommendation must contain:

1. Evidence and why the problem matters.
2. Existing capability to reuse, and the smallest missing addition.
3. Trigger conditions and non-applicable cases.
4. Execution steps with temporary choices kept separate.
5. Observable acceptance criteria and uncertainty.

Prefer project decision records, relevant behavior checks and verified resume summaries over
duplicated frameworks. Suggest periodic automation only if there is a recurring time-based
need, an incremental checkpoint and a meaningful notification condition. A recommendation
does not authorize creating or installing that automation.
