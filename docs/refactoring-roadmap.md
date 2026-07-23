# Clean Architecture Refactoring Roadmap

Each phase must leave one implementation path, pass its focused tests plus the
relevant full regression suites, and end in a Conventional Commit with a
Chinese description.

## 1. Safety baseline

- Run frontend tests in CI.
- Run Python tests in CI.
- Run Rust fmt, clippy, tests, and check in CI.
- Keep Node format, typecheck, tests, and build as mandatory gates.
- Record vocabulary, runtime boundaries, data ownership, and Windows policy.

## 2. Current-structure session adapters

- Delete `FormatProfile`, `FormatRegistry`, `VersionRange`, and version status.
- Replace dynamic package discovery with explicit adapter composition.
- Remove old field aliases and old native-shape fallbacks.
- Keep one current fixture family per session source.
- Add structural-change failure tests.

## 3. Canonical semantics and native documents

- Make `ToolResult` the sole tool-result representation.
- Move editing fidelity into adapter-private `NativeDocument` types.
- Remove `RawRecord`, `Message.raw`, and Ferry-private native extension fields
  only after native edit and migration tests prove they are unnecessary.

## 4. Unified operations

- Introduce immutable `operation.plan/apply/status/cancel`.
- Route UI and Ferry Runtime mutations through the same operation lifecycle.
- Merge authored assistant replies into edit changes.
- Delete `save_as`, `dry_run` permission routing, migration handoff, and the
  preview/propose/authorize API families.

## 5. Concurrent Session Engine

- Multiplex requests by ID.
- Separate bounded read execution from per-session mutation queues.
- Move long work to cancellable jobs with progress events.

## 6. Ferry Runtime

- Split the current runtime by session, provider, tool, event, and persistence
  responsibilities; rename the package and binary if the new boundary is clear.
- Add `Scheduler`, `TaskGraph`, `TaskNode`, scheduler limits, cancellation,
  failure aggregation, and fan-out/fan-in synthesis.
- Do not introduce long-term memory.

## 7. Backend-owned workflows and UI

- Move organization generation out of React.
- Split the application shell and workspaces.
- Add workflow graph, parallel worker status, approval, and synthesis views.
- Move contract, domain, and feature-controller code to TypeScript.

## 8. Contracts, IPC, storage, and final cleanup

- Converge Engine and Ferry Runtime on one IPC framing.
- Generate stable method, operation, event, and error definitions.
- Add contract-drift checks.
- Select one SQLite owner and migrate Ferry-owned durable state transactionally.
- Isolate macOS and Windows platform implementations.
- Delete obsolete paths, dependencies, locale keys, tests, and documentation.

The refactor is complete only after the repository-wide verification commands
pass and searches confirm that the deleted runtime concepts no longer exist.
