# Capability Architecture Refactoring Roadmap

Each phase must leave one implementation path, pass its focused tests plus the
relevant full regression suites, and end in a Conventional Commit with a
Chinese description.

## 1. Safety baseline — complete

- Run frontend tests in CI.
- Run Python tests in CI.
- Run Rust fmt, clippy, tests, and check in CI.
- Keep Node format, typecheck, tests, and build as mandatory gates.
- Record vocabulary, runtime boundaries, data ownership, and Windows policy.

## 2. Current-structure session adapters — complete

- Delete `FormatProfile`, `FormatRegistry`, `VersionRange`, and version status.
- Replace dynamic package discovery with explicit adapter composition.
- Remove old field aliases and old native-shape fallbacks.
- Keep one current fixture family per session source.
- Add structural-change failure tests.

## 3. Canonical semantics and native documents — complete

- Make `ToolResult` the sole tool-result representation.
- Move editing fidelity into adapter-private `NativeDocument` types.
- Remove `RawRecord`, `Message.raw`, and Ferry-private native extension fields
  only after native edit and migration tests prove they are unnecessary.

## 4. Unified operations — complete

- Introduce immutable `operation.plan/apply/status/cancel`.
- Route UI and Ferry Runtime mutations through the same operation lifecycle.
- Merge authored assistant replies into edit changes.
- Delete `save_as`, `dry_run` permission routing, migration handoff, and the
  preview/propose/authorize API families.

## 5. Concurrent Session Engine — complete

- Multiplex requests by ID.
- Separate bounded pure reads from the ordered serial lane.
- Move native writes to one cancellable mutation job queue with durable status
  snapshots.

## 6. Ferry Runtime — complete for bounded multi-agent workflows

- Split the current runtime by session, provider, tool, event, and persistence
  responsibilities; rename the package and binary if the new boundary is clear.
- Use `WorkflowRun`, `TaskGraph`, and `TaskNode` with bounded concurrency,
  cancellation propagation, failure aggregation, budgets, cycle/depth checks,
  and fan-out/fan-in synthesis.
- Do not introduce long-term memory.

## 7. Backend-owned workflows and UI — complete

- Keep organization generation in Ferry Runtime, not React.
- Continue thinning the application shell after workspace and overlay split.
- Add workflow graph, parallel worker status, approval, and synthesis views.
- Move contracts, query/state models, and module controllers to TypeScript.

## 8. Contracts, IPC, storage, and final cleanup — complete

- Converge Engine and Ferry Runtime on one IPC framing.
- Generate stable method, operation, event, and error definitions.
- Add contract-drift checks.
- Keep Python Engine as the one SQLite owner and isolate capability stores.
- Isolate macOS and Windows platform implementations.
- Enable a restrictive production CSP and platform-owned bundle targets.
- Build both sidecars through one repository entry point.
- Delete obsolete paths, dependencies, locale keys, tests, and documentation.

Repository-wide verification and deleted-concept guards remain permanent CI
gates rather than a temporary migration phase.
