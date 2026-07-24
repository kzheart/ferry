# Ferry Architecture

This document is the architectural source of truth for the ongoing refactor.
It describes the intended ownership boundaries rather than preserving the
shape of the current implementation.

## Package convention

Ferry does not use `domain/application/infrastructure` as repository-wide
horizontal layers. The repository is split by process boundary first; each
process is then split by product capability. A capability package owns its
rules, types, coordination, and private persistence adapter together.

This follows the practical shape used by comparable open-source agent
projects: OpenHands groups its SDK by capabilities such as agent,
conversation, event, LLM, security, subagent, tool, and workspace; Cherry
Studio separates Electron process boundaries and then groups renderer/main
code by features, services, stores, providers, tools, and IPC.

The intended top-level layout is:

```text
app/             React UI and Tauri host
engine/          external session engine
ferry-runtime/   Ferry multi-agent runtime
contracts/       generated cross-process contracts
scripts/         build and architecture checks
```

Package names describe what the code does, not which abstract layer it belongs
to. Shared code must stay small and concrete; a generic `core` or `utils`
package must not become a second application.

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

Current structure may legitimately contain more than one record subtype when
the upstream runtime itself emits them. Codex is the important example:
`response_item.function_call` / `function_call_output` and
`response_item.custom_tool_call` / `custom_tool_call_output` coexist in
current rollouts, with the former also representing `spawn_agent`. This is one
current native union, not a version fallback; the reader must reject unknown
subtypes instead of selecting a parser by CLI version.

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

```text
app/src/
  main.jsx         frontend bootstrap
  shell/           desktop layout, navigation, routing, global overlays
  modules/         vertical product capabilities and their local models
  platform/        typed desktop client, cache, updater, platform errors
  shared/          generated contracts, i18n, styles, concrete UI primitives
  assets/          bundled images and icons
```

Module-specific models stay beside the capability that owns them. The frontend
does not recreate a cross-feature `domain/` layer; shared display helpers must
be concrete UI modules, not a generic business bucket.

### Tauri / Rust Host

Owns desktop integration, cross-platform process supervision, IPC routing,
timeouts, cancellation, WebView exposure, approval policy, and event bridging.
It is the only trust boundary between the UI, Ferry Runtime, and mutating
Session Engine operations.

Rust does not parse Claude, Codex, or OpenCode native data.

```text
app/src-tauri/src/
  engine/          Python Engine client, policy, public commands
  runtime/         Ferry Runtime gateway and approval routing
  operations/      typed operation input, validation, request encoding
  process/         shared JSONL process transport
  desktop/         terminal, reveal, window, menu, platform implementations
  contracts/       generated shared contracts
```

Windows support remains a first-class platform boundary under
`desktop/platform/windows.rs`; capability packaging must not remove its build
or CI path.

### Python Session Engine

Owns session indexing, current native-format adapters, the canonical semantic
model, adapter-private native documents, migration, editing, metadata,
snapshots, revisions, compare-and-swap checks, verification, rollback, and
operation audit.

Every mutation converges on:

```text
operation.plan -> approval -> operation.apply
```

`operation.apply` only queues the immutable plan and returns its job status;
the Engine's single mutation worker performs the native write separately.
Status and cancellation are addressed through `operation.status` and
`operation.cancel`: a queued task can be cancelled before it starts, while an
already-applying native write is allowed to finish so snapshot/CAS/rollback
semantics are never interrupted halfway through. A plan is immutable and
applying it does not accept a second copy of the business parameters.

The process entry point creates one Engine service for each sidecar lifetime.
It owns its session index and single-worker operation queue; RPC dispatchers
and CLI commands receive that same service explicitly. Dependencies are passed
to capability services and have no implicit process-global reconfiguration API.

Deterministic organization state is owned by
`engine/organization/`: `summaries.py` manages content-addressed
digest inputs/results and `proposals.py` manages proposal validation and
decisions. Model execution remains in Ferry Runtime and reaches these use cases
only through the Rust/Engine gateway.

The target Engine packages are capability-oriented:

```text
engine/
  server/          RPC server and generated protocol
  sessions/        scan, index, read, search, assets, usage
  operations/      plan, apply, edit, migrate, metadata, delete, verify
  organization/    summaries and organization proposals
  adapters/        current Claude, Codex, and OpenCode structures
    shared/        codec, editing, migration, scan primitives reused by adapters
  storage/         SQLite composition plus capability-owned stores
  system/          paths, executables, resources, and probes
  app.py           process capability facade
  bootstrap.py     process composition
  context.py       explicit shared dependencies
```

### Ferry Runtime

Owns providers, authentication, model selection, roles, conversations, workflow
runs, task graphs, Ferry agent execution, tool planning, bounded scheduling,
result artifacts, and synthesis. Its Node package identity is `@ferry/runtime`;
its source and packaged sidecar are both named `ferry-runtime` on macOS and
Windows. Platform-specific executable suffixes remain isolated in build and
process-launch code.

Its source packages follow the same responsibility boundaries:

```text
ferry-runtime/src/
  server/          JSONL server, envelopes, generated contracts
  runtime/         command routing and runtime orchestration
  agents/          bounded scheduling, delegation, task graph
  providers/       provider configuration, authentication, model host
  sessions/        conversations and their persistence
  roles/           role definitions and repositories
  tools/           Ferry tool catalog and delegation
  organizing/      summary and organization workflows
  security/        redaction and runtime limits
```

Ferry Runtime cannot write an external session directly. A requested mutation
travels through the Rust approval gateway to a Session Engine operation.

The first multi-agent execution model is deliberately bounded fan-out/fan-in:

```text
planner -> parallel task nodes -> synthesizer
```

Every `WorkflowRun` has concurrency, task-count, depth, per-task timeout, and
total persisted-output limits. A task timeout is a workflow failure, not a user
cancellation, so failure policy and fan-in synthesis can handle it explicitly.
Provider cost and token accounting are not yet a scheduler input. Cancellation
propagates to running descendants. Cycles and unbounded recursive delegation
are rejected.

Ferry does not provide long-term agent memory. Workflow input, events, and
result artifacts live only within the workflow/conversation persistence model.

## Data ownership

External Claude, Codex, and OpenCode stores remain external sources of truth.
Ferry never migrates or rewrites them merely to upgrade Ferry-owned state.

Python Engine is the only writer of `ferry-state.sqlite3`. Its exact schema is
currently version 8 and owns immutable operation plans, operation audit,
delete-recovery handles, session metadata, migration history, session summary
backbones, organization proposals, organization signals, and Ferry Runtime
session/event records. Session metadata
is identified by the exact `(tool, native_session_id)` pair, never by a bare
native ID. Migration history has a database-generated immutable ID and is
listed in append order from newest to oldest. Summary backbones use the same
composite identity and retain only workflow-scoped digest cache data, not
long-term Agent memory. Organization approval checks summary fingerprints,
applies metadata CAS, writes the proposal state, and records its signal in one
SQLite transaction. The
database uses WAL plus `BEGIN IMMEDIATE` for every state transition and
metadata CAS. A schema other than the exact current version fails at startup;
old JSON metadata and older SQLite schemas are not read or migrated.

`engine/storage/database.py` owns only the connection, exact schema, and store
composition. Runtime sessions, operation plans/recovery, metadata, migration
history, summaries, and organization transactions each have a named store.
The organization store deliberately owns its cross-table transaction as one
capability; it is not split into repository abstractions that could break
atomic approval.

The UI uses the same pair as its local session identity for list keys,
selection, multi-selection, context menus, and detail caching. Native session
IDs remain adapter data and must never become cross-tool UI identifiers.

Runtime session/event records enter the database only through internal Engine
RPC after the Runtime has redacted them; Python stores these records as opaque
JSON and never interprets Provider or AgentMessage semantics. Rust and Ferry
Runtime never open
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
Session Engine endpoint: public/trusted UI/internal exposure,
read/index/mutation classification, timeout class, and retry safety.
`contracts/ipc.json` defines the exact `ferry-ipc/1` request, response, error,
and event envelopes. Generated constants include a contract hash; both
sidecars must return the expected service identity and hash during handshake.
The Rust host owns an `id` for every Engine request and multiplexes JSONL
responses by that ID, so an individual caller never holds the process-manager
lock while waiting. The Engine uses a bounded four-worker lane only for the
contract's explicitly declared pure reads; index refreshes, native-session
reads and every mutation remain on the ordered serial lane. Protocol output
has a single writer, so parallel responses may arrive out of order but remain
correlated by the top-level `id`, including structured error responses.

## Cross-platform boundary

Windows support is retained. `app/src-tauri/src/platform/` is the Rust platform
boundary. It owns reveal-in-file-manager and terminal launch behavior: macOS
has the native implementation and Windows has explicit, compilable unsupported
stubs. The following capabilities still need to move behind the same boundary:

- process spawning and hidden-console policy;
- executable and bundled-sidecar naming;
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
Rust gateway -> Session Engine capability API
Session Engine capability API -> adapter contracts
adapter -> current native store
```

No layer imports or reproduces another layer's implementation details.
