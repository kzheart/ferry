# Ferry Architecture

This document is the architectural source of truth for the ongoing clean
architecture refactor. It describes the intended ownership boundaries rather
than preserving the shape of the current implementation.

## Vocabulary

Ferry uses two concepts that must not share the same name:

- **Session source**: an external coding tool whose current native session
  structure Ferry can read or write. The built-in session sources are Claude
  Code, Codex CLI, and OpenCode.
- **Ferry agent**: an LLM-backed worker that participates in a Ferry workflow.
  A role such as planner, researcher, coder, or reviewer configures a Ferry
  agent; it is not an external session format.

External tool versions are not part of the Ferry domain. Each session adapter
supports exactly the current structure implemented in the repository. Readers
ignore unrelated additional fields, reject changes to required structure, and
never select parsers by CLI version.

Ferry's own IPC protocol and durable-store schema remain exact, independently
versioned contracts. They protect against mixed binaries and corrupt data; they
do not provide backward compatibility.

## Runtime boundaries

```text
React UI
  |
  | desktop commands and events
  v
Tauri / Rust Host
  |-- Python Session Engine
  `-- Ferry Runtime
```

### React UI

Owns presentation, local form drafts, navigation, workflow visualization, and
user approval interactions. It may start work and subscribe to state, but it
does not coordinate a transaction or a workflow across processes.

### Tauri / Rust Host

Owns desktop integration, cross-platform process supervision, IPC routing,
timeouts, cancellation, WebView exposure, approval policy, and event bridging.
It is the only trust boundary between the UI, Ferry Runtime, and mutating
Session Engine operations.

Rust does not parse Claude, Codex, or OpenCode native data.

### Python Session Engine

Owns session indexing, current native-format adapters, the canonical semantic
model, adapter-private native documents, migration, editing, metadata,
snapshots, revisions, compare-and-swap checks, verification, rollback, and
operation audit.

Every mutation converges on:

```text
operation.plan -> approval -> operation.apply
```

Status and cancellation are addressed through `operation.status` and
`operation.cancel`. A plan is immutable and applying it does not accept a
second copy of the business parameters.

### Ferry Runtime

Owns providers, authentication, model selection, roles, conversations, workflow
runs, task graphs, Ferry agent execution, tool planning, bounded scheduling,
result artifacts, and synthesis.

Ferry Runtime cannot write an external session directly. A requested mutation
travels through the Rust approval gateway to a Session Engine operation.

The first multi-agent execution model is deliberately bounded fan-out/fan-in:

```text
planner -> parallel task nodes -> synthesizer
```

Every workflow has concurrency, task-count, depth, time, and cost/token limits.
Cancellation propagates to running descendants. Cycles and unbounded recursive
delegation are rejected.

Ferry does not provide long-term agent memory. Workflow input, events, and
result artifacts live only within the workflow/conversation persistence model.

## Data ownership

External Claude, Codex, and OpenCode stores remain external sources of truth.
Ferry never migrates or rewrites them merely to upgrade Ferry-owned state.

Python Engine is the only writer of `ferry-state.sqlite3`. Its exact schema is
currently version 3 and owns immutable operation plans, operation audit,
delete-recovery handles, and session metadata. The database uses WAL plus
`BEGIN IMMEDIATE` for every state transition and metadata CAS. A schema other
than the exact current version fails at startup; old JSON metadata and older
SQLite schemas are not read or migrated.

Other Ferry-owned stores (migration history, summaries, organization proposals,
and Runtime conversation event logs) have not yet been consolidated. They must
continue to be accessed only by their designated process until they move into
the Python-owned SQLite boundary; Rust and Ferry Runtime never open
`ferry-state.sqlite3` directly.

The SQLite boundary follows these rules:

- one process is the database writer and schema owner;
- other runtimes use an IPC port instead of opening the database for writes;
- mutations use transactions and explicit revision checks;
- UI caches are disposable and rebuildable;
- schema mismatch fails explicitly instead of running compatibility migrations.

The static external-session contract starts at `contracts/agents.json` and is
generated into the UI, Rust Host, Python Engine, and Ferry Runtime by
`scripts/generate-contracts.py`. CI rejects generated-file drift. It contains
only current built-in Agent identities and launch policy, never external Agent
version ranges or compatibility status.

`contracts/engine-methods.json` is the equivalent policy source for every
Session Engine endpoint: WebView exposure, read/index/mutation classification,
timeout class, and retry safety. It is generated into Rust and Python. The
Rust host owns a correlation ID for every Engine request and multiplexes JSONL
responses by that ID, so an individual caller never holds the process-manager
lock while waiting. The Engine uses a bounded four-worker lane only for the
contract's explicitly declared pure reads; index refreshes, native-session
reads, JSON-backed stores, and every mutation remain on the ordered serial
lane. Protocol output has a single writer, so parallel responses may be
out-of-order but are always correlated by `request_id`.

## Cross-platform boundary

Windows support is retained. `app/src-tauri/src/platform/` is the initial Rust
platform boundary. It already owns reveal-in-file-manager behavior: macOS has
the native implementation and Windows has an explicit, compilable unsupported
stub. The following capabilities still need to move behind the same boundary:

- process spawning and hidden-console policy;
- executable and bundled-sidecar naming;
- terminal or shell launch;
- reveal-in-file-manager;
- window decoration and visual effects.

macOS implementations may use private or native APIs. Windows implementations
must not be emulated with scattered `cfg` branches in business modules. A
feature may remain unavailable on Windows during development only through an
explicit platform implementation that returns a structured unsupported error.
Windows sidecar builds and Rust compilation remain CI gates.

## Non-negotiable safety properties

- Snapshot before destructive native-data mutation.
- Revision and compare-and-swap validation.
- Immutable operation plans with expiry and one-time approval.
- Re-read and validate migration output.
- Roll back failed writes.
- Keep mutation audit records.
- Redact credentials and bound persisted model/tool output.
- Allowlist all WebView and Ferry Runtime methods.
- Apply timeouts, cancellation, and bounded concurrency.
- Never let Node write external session stores directly.

## Dependency direction

```text
UI -> generated Agent contract and desktop command layer
Rust host -> generated Agent policy and IPC boundary
Ferry Runtime -> tool port -> Rust gateway
Rust gateway -> Session Engine application API
Session Engine application -> adapter contracts
adapter -> current native store
```

No layer imports or reproduces another layer's implementation details.
