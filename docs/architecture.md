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
  | typed desktop commands and events
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

Ferry-owned durable state may use SQLite under these rules:

- one process is the database writer and schema owner;
- other runtimes use an IPC port instead of opening the database for writes;
- mutations use transactions and explicit revision checks;
- UI caches are disposable and rebuildable;
- schema mismatch fails explicitly instead of running compatibility migrations.

The storage owner will be selected before the SQLite migration begins and
recorded as an ADR. Until then, existing stores keep their current owners.

## Cross-platform boundary

Windows support is retained. Platform behavior is isolated behind explicit
Rust interfaces:

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
UI -> generated desktop contract
Rust host -> generated protocol and policy
Ferry Runtime -> tool port -> Rust gateway
Rust gateway -> Session Engine application API
Session Engine application -> adapter contracts
adapter -> current native store
```

No layer imports or reproduces another layer's implementation details.
