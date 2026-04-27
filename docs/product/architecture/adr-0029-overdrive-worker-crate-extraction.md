# ADR-0029 — Dedicated `overdrive-worker` crate (class `adapter-host`); ProcessDriver + workload-cgroup management + node_health writer extracted from `overdrive-host`

## Status

Accepted. 2026-04-27. Decision-makers: User-proposed, ratified
2026-04-27. Tags: phase-1, first-workload, application-arch,
crate-boundary.

## Context

The Phase 1 first-workload feature needs a workload-supervision
subsystem: a place that hosts `ProcessDriver`, manages workload
cgroup scopes (`overdrive.slice/workloads.slice/<alloc_id>.scope`),
and writes the local node's `node_health` row at startup. The
DISCUSS wave pre-decided that `ProcessDriver` would live in
`crates/overdrive-host/src/driver/process.rs`, alongside the
existing host-OS adapters (`SystemClock`, `OsEntropy`,
`TcpTransport`).

DESIGN re-examined that placement. The whitepaper's architecture
diagram (§3) draws an explicit boundary between two co-resident
subsystems on every node: the **control plane** (intent, scheduler,
reconcilers, CA) and the **node agent / worker** (drivers, cgroup
management, dataplane, telemetry collection). §4 makes the boundary
runtime-selectable through the `[node] role` config — `control-plane`
| `worker` | `control-plane+worker`. Phase 1 is single-node and runs
both subsystems in one process via `control-plane+worker`, but Phase
2+ multi-node deployments will route some nodes to control-plane
only and others to worker only — and the supporting types must
already live in the right crate by then or the migration is a
refactor under pressure rather than a config change.

Three observations forced the extraction now:

1. **`overdrive-host` was originally a host-OS-primitives crate.**
   ADR-0016 extracted it from `overdrive-core` to hold the production
   `Clock` / `Entropy` / `Transport` bindings — small adapters that
   bind a port trait to a host-OS syscall. `ProcessDriver`,
   workload-cgroup management, and the `node_health` row writer are a
   different beast: they are workload-supervision subsystems with
   their own lifecycle, their own cgroup hierarchy, their own
   integration-tests-gated test suite, and (in Phase 2+) their own
   eBPF/Cloud-Hypervisor/Wasmtime dependencies. Squatting them in
   `overdrive-host` muddies what the crate is *for*.

2. **The Phase 2+ multi-node split forces the worker extraction
   anyway.** Once a node can declare `role = "control-plane"`
   (control-plane subsystems only, no driver) or `role = "worker"`
   (worker subsystems only, no router/handlers), the binary's link
   graph needs a clean way to include or exclude the worker code.
   Doing the extraction *now*, in Phase 1 paper-only, is the cheapest
   moment — there is no implementation yet to refactor, no integration
   tests to relocate, no dst-lint scope to recompute. Deferring means
   refactoring `ProcessDriver` out of `overdrive-host` once it has
   tests, integration callers, and metadata declarations.

3. **The scheduler crate extraction (ADR-0024) sets the precedent
   one level up.** The user-override decision in ADR-0024 chose a
   dedicated `overdrive-scheduler` crate over a module-inside-
   `overdrive-control-plane` placement, on the grounds that the
   discipline (BTreeMap-only iteration, banned-API contract) becomes
   mechanically enforced rather than convention-bound. The same
   strategic logic applies here: the worker subsystem's boundary
   (driver impls, cgroup management, node-health writing) becomes a
   compile-time enforced concern when it lives in its own crate. The
   control-plane crate cannot accidentally depend on driver internals
   because the worker crate is not on its dep edge.

## Decision

### 1. New crate `crates/overdrive-worker`, class `adapter-host`

```toml
# crates/overdrive-worker/Cargo.toml
[package]
name        = "overdrive-worker"
description = "Worker subsystem: ProcessDriver + workload-cgroup management + node_health writer. Hosted by the binary alongside overdrive-control-plane when [node] role includes worker."
edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
authors.workspace      = true
repository.workspace   = true
publish                = false

[package.metadata.overdrive]
crate_class = "adapter-host"

[features]
# Workspace-wide convention. Every member declares this feature so
# `cargo {check,test,mutants} --features integration-tests` resolves
# uniformly under per-package scoping. See `.claude/rules/testing.md`
# § Workspace convention.
integration-tests = []

[dependencies]
overdrive-core.workspace = true   # Driver port trait, AllocationSpec / Handle / State,
                                  # NodeId, Region, Resources, AllocStatusRow, NodeHealthRow
thiserror.workspace      = true   # DriverError envelope (existing pattern)
tokio.workspace          = true   # tokio::process::Command, async runtime
tracing.workspace        = true   # structured logging (warn-and-continue per ADR-0026)

[dev-dependencies]
overdrive-sim.workspace = true   # SimDriver fixtures for cross-crate tests if needed
proptest.workspace      = true   # newtype + handle round-trip

[lints]
workspace = true
```

The crate is class `adapter-host`. It is NOT scanned by `dst-lint` —
real-infra calls (cgroupfs writes, `tokio::process::Command::spawn`,
`hostname` reads) are expected and permitted.

### 2. Crate contents

The crate hosts five logical concerns; the precise filesystem layout
is the crafter's call, but the architectural contents are fixed:

- **`ProcessDriver`** — the `Driver` trait impl from ADR-0026.
  Linux-only conditional compilation (`#[cfg(target_os = "linux")]`);
  macOS / Windows builds skip the module. Calls `tokio::process::
  Command::spawn`; writes `cgroup.procs`; tracks the live PID.
- **Workload-cgroup management** — creates and tears down
  `overdrive.slice/workloads.slice/<alloc_id>.scope` per ADR-0026.
  Writes `cpu.weight`, `memory.max`. `rmdir`s the scope after process
  reap. Five filesystem operations per workload lifecycle, no
  `cgroups-rs` dep.
- **`node_health` row writer** — formerly inline in
  `overdrive-control-plane`'s `run_server_with_obs_and_driver` boot
  sequence per ADR-0025. Now a worker-subsystem startup
  responsibility: the worker resolves `NodeId` (hostname fallback +
  `[node].id` override) and `Region` (default `"local"` +
  `[node].region` override) at its own startup, computes capacity from
  config or sentinel, and writes one `node_health` row to the
  `ObservationStore` before the worker is considered "started." Phase
  2+ multi-node has a worker on each node writing its own row.
- **Worker subsystem entrypoint** — a `Worker` struct (or equivalent
  shape) that composes the above and exposes:
  - A constructor / boot function the binary calls during startup
    (`Worker::start(config, obs)` or similar).
  - An `Arc<dyn Driver>` accessor the binary plugs into the
    control-plane's `AppState::driver` field per ADR-0022 (when both
    subsystems are co-located in the same binary).
- **`CgroupPath` newtype** — originally targeted for `overdrive-host`
  by the Slice 2 brief; moves with `ProcessDriver` since the only
  caller is workload-cgroup management. STRICT-newtype obligations
  (FromStr, Display, validation) unchanged.

### 3. Dependency direction: `overdrive-core ← overdrive-worker ← overdrive-cli`

```
overdrive-core    ←  overdrive-scheduler     ←  overdrive-control-plane  ←  overdrive-cli
                  ←  overdrive-host                                       ←  overdrive-cli
                  ←  overdrive-store-local                                ←  overdrive-control-plane
                  ←  overdrive-worker                                     ←  overdrive-cli
                  ←  overdrive-sim (dev/test)
```

Critical edges:

- **`overdrive-control-plane` does NOT depend on `overdrive-worker`.**
  The action shim (ADR-0023) calls `Driver::start/stop/status`
  against `&dyn Driver`; the impl is plugged in at `AppState`
  construction time by the binary. The control-plane crate sees only
  the trait surface, never the worker's implementation crate.
- **`overdrive-cli` depends on both** `overdrive-control-plane` and
  `overdrive-worker`. The `overdrive serve` subcommand instantiates
  control-plane subsystem + (when `[node] role` includes worker)
  worker subsystem, threads `Arc<dyn Driver>` from the worker into
  the control-plane's `AppState`. This is the binary-composition
  point; it is the only place both subsystems are visible together.
- **`overdrive-worker` depends only on `overdrive-core`** (and the
  permitted helpers `thiserror`, `tokio`, `tracing`). It does NOT
  depend on `overdrive-control-plane`, `overdrive-store-local`, or
  `overdrive-host`. The cycle that would otherwise form
  (`overdrive-control-plane ← overdrive-worker ←
  overdrive-control-plane`) does not exist.

The graph remains acyclic. ADR-0003 (crate-class labelling), ADR-0016
(`overdrive-host` extraction), and ADR-0024 (`overdrive-scheduler`
extraction) all remain consistent with the extended graph.

### 4. Composition at the binary

`overdrive-cli`'s `serve` subcommand is the composition root. It
hard-depends on both `overdrive-control-plane` and `overdrive-worker`
(both are on its `[dependencies]` block; no feature gate). At
runtime it reads `[node] role` from the operator config and selects
which subsystems boot:

```text
role = "control-plane+worker"  (Phase 1 default; single-node)
   → boot control-plane subsystem
   → boot worker subsystem
   → thread Arc<ProcessDriver> from worker into control-plane AppState
   → bind listener
   → spawn axum_server task

role = "control-plane"  (Phase 2+ dedicated control-plane node)
   → boot control-plane subsystem
   → AppState::driver wired against a future RemoteDriver impl that
     proxies Driver::start/stop/status over RPC to remote workers
   → bind listener
   → spawn axum_server task

role = "worker"  (Phase 2+ dedicated worker node)
   → boot worker subsystem only
   → register with regional control plane via tarpc / postcard-rpc
   → no listener, no router, no handlers
```

The worker code is **linked into the binary** even when `role =
"control-plane"` is selected — Phase 1 binary size is negligible
(`ProcessDriver` is small). When the worker grows materially in
Phase 2+ (Cloud Hypervisor, Wasmtime, eBPF programs), splitting into
`overdrive-control-plane-bin` and `overdrive-worker-bin` becomes a
mechanical change against this same crate boundary. **No feature
flag on `overdrive-control-plane`** — feature-gating foundational
crates carries a maintenance tax (two compile shapes to test,
conditional code paths) that the Phase 1 footprint does not justify.

Phase 2+ multi-node introduces a `RemoteDriver` impl in a future
crate (`overdrive-rpc-driver` or similar) that proxies the same
`Driver` trait over RPC. The action shim calls `Driver::*` against
it exactly as it does against `ProcessDriver` today. The shim
contract from ADR-0023 stays stable across the Phase 1 → Phase 2+
transition.

### 5. `overdrive-host` shrinks back to host-OS primitives

ADR-0016 introduced `overdrive-host` to hold production bindings of
the core port traits to the host OS / kernel / network — `SystemClock`,
`OsEntropy`, `TcpTransport`. The DISCUSS wave for first-workload
provisionally placed `ProcessDriver` there because no worker crate
existed yet. With this ADR, `ProcessDriver` and its companions move
to `overdrive-worker`, restoring `overdrive-host` to its original
intent: the small set of host-OS adapters that production wires
into the port traits.

ADR-0016 stays Accepted; this ADR notes the partial reversal as a
**refinement**, not a supersession. The ADR-0016 boundary
(`adapter-host` adapters for host-OS primitives) is preserved; what
moves is the in-DESIGN-wave-only proposal that ProcessDriver fit
into the same crate.

## Alternatives considered

### Alternative A — Leave drivers in `overdrive-host`; extract worker later (Phase 2+)

Keep `ProcessDriver` in `overdrive-host` for Phase 1; extract
`overdrive-worker` when Phase 2+ multi-node deployment makes the
boundary visibly necessary.

**Rejected.** Phase 1 first-workload is the cheapest possible moment
for the extraction: paper-only, no implementation yet, no integration
tests to relocate, no dst-lint scope changes to compute. Deferring
means refactoring `ProcessDriver` out of `overdrive-host` once it
has tests + integration callers, and the refactor lands under Phase
2+ time pressure rather than as a DESIGN-wave decision. The
"extract under inheritance pressure" failure mode is exactly what
ADR-0016, ADR-0017, and ADR-0024 were each motivated to avoid.

### Alternative B — Feature-flag `overdrive-control-plane` to optionally depend on `overdrive-worker`

`overdrive-control-plane` declares `[features] worker = ["dep:overdrive-worker"]`;
`overdrive-cli` enables the `worker` feature when `role` includes
worker. Binary code paths conditionally compile worker integration.

**Rejected.** This was the user's initial framing; the orchestrator
recommendation that the user ratified explicitly preferred
binary-composition. Three reasons:

- **Maintenance tax.** Feature-gated foundational crates carry two
  compile shapes that must each be tested in CI (control-plane
  alone, control-plane+worker). Conditional `#[cfg(feature =
  "worker")]` blocks in the control-plane source are a known
  long-term-decay shape.
- **Binary-size win is not yet real.** Phase 1 ProcessDriver is small.
  The win materialises in Phase 2+ when worker grows Cloud
  Hypervisor + Wasmtime + eBPF; at that point a split-binary
  approach (`overdrive-control-plane-bin`, `overdrive-worker-bin`)
  is the mechanically simpler answer than a feature-gated single
  binary.
- **The control-plane crate stays clean either way.** Whether the
  worker is feature-gated or binary-composed, `overdrive-control-plane`
  never imports worker-specific types. The composition decision
  affects only the binary, not the library crate. Binary-composition
  is the simpler shape and we adopt it unconditionally.

### Alternative C — Channel-decoupled action shim (worker reads `tokio::mpsc`; control-plane writes to it)

The reconciler runtime emits `Vec<Action>` into a `tokio::mpsc`
channel; the worker subsystem consumes the channel from a separate
task and dispatches to its drivers.

**Rejected.** ADR-0023 §Alternative C already rejected channel-
decoupling for Phase 1: no back-pressure or concurrency need yet,
the reconciler runtime is sequential by design, and adding a channel
introduces queue-depth invariants the runtime must defend (what if
the consumer is slower than the producer?), additional cancellation
surface, and cross-task error propagation complexity. The worker
extraction does not change that calculus — the shim contract stays
"shim calls `Driver::*` in-process," and the worker crate's public
surface is just the `Driver` trait impl.

Phase 2+ multi-node introduces the channel/RPC boundary inside a
`RemoteDriver` impl: the impl marshals `Driver::start/stop/status`
calls over RPC to the actual worker on a remote node. The shim sees
the same trait surface; the channel is an internal concern of one
specific impl, not an architectural seam at the shim/runtime
boundary.

## Consequences

### Positive

- **Clean control-plane vs worker split that matches whitepaper §3
  architecture.** The diagram drew the boundary; the crate graph now
  reflects it. Phase 2+ multi-node migration (`role = "control-plane"`
  vs `role = "worker"` on different nodes) becomes additive: a new
  `RemoteDriver` impl crate (`overdrive-rpc-driver` or similar) is
  introduced, the binary composition rewires `AppState::driver` to
  the remote impl when `role = "control-plane"`, no shim/control-
  plane refactor required.
- **Compile-time enforcement of the boundary.**
  `overdrive-control-plane` cannot depend on driver internals because
  `overdrive-worker` is not on its dep edge. A future contributor
  trying to import `ProcessDriver` from inside the control-plane
  crate gets a compile error (the type is not visible). Convention
  becomes mechanical.
- **`overdrive-host` regains its original shape.** ADR-0016's
  "host-OS primitives" intent is preserved; squatting drivers there
  is reversed. Future host-OS adapters (e.g., `SystemEntropy` ↔
  `getentropy`, `TcpTransport` ↔ `tokio::net`) live alongside their
  peers without being mixed into workload-supervision concerns.
- **Phase 2+ split-binary deployments are mechanically free.** Two
  binaries (`overdrive-control-plane-bin`, `overdrive-worker-bin`)
  built from the existing crate graph; no library refactor needed.
- **Extraction precedent extended.** The pattern from ADR-0016
  (`overdrive-host`) and ADR-0024 (`overdrive-scheduler`) — extract
  per architectural class, eagerly, when the seam is clear — applies
  one level up. The strategic logic is uniform across the workspace.

### Negative

- **One more crate to maintain.** Workspace grows from seven to
  eight Rust crates (excluding `xtask`). Each new member adds CI
  overhead (a few seconds per `cargo check` / `cargo clippy` /
  `cargo nextest run` invocation). The cost is paid once and
  amortises across every PR. The workspace-feature self-test
  (`every_workspace_member_declares_integration_tests_feature`)
  catches a missing `integration-tests = []` declaration at PR
  time.
- **The binary links worker code even when `role =
  "control-plane"`.** Phase 1 negligible (`ProcessDriver` is small).
  Phase 2+ may revisit when the worker grows Cloud Hypervisor +
  Wasmtime + eBPF; at that point split binaries are the natural
  follow-on. Acknowledged; not a Phase 1 blocker.
- **Slice 2 ProcessDriver lands in a new crate the codebase hasn't
  tested yet.** Mitigated by the established `adapter-host`
  precedent from `overdrive-host`, `overdrive-store-local`, and
  `overdrive-control-plane` — the shape is well-understood. The
  Slice 2 integration test (`crates/overdrive-worker/tests/integration/process_driver.rs`)
  remains gated by the `integration-tests` feature exactly as the
  DISCUSS-wave brief specified; only the crate name changes.

### Quality-attribute impact

- **Maintainability — testability**: positive. The worker subsystem
  can be exercised in isolation under DST + integration tests; the
  control-plane crate's tests are unaffected by worker-internal
  changes.
- **Maintainability — modifiability**: positive. Clearer ownership
  boundaries; adding a second driver (Phase 2+ MicroVm via Cloud
  Hypervisor) extends `overdrive-worker` without touching control-
  plane code.
- **Maintainability — analyzability**: positive. `cargo doc -p
  overdrive-worker` produces a focused doc for the worker subsystem.
- **Reliability — fault tolerance**: neutral. Runtime semantics are
  identical; only the source-tree organisation changes.
- **Performance — time behaviour**: neutral. The function calls are
  identical regardless of which crate they live in.
- **Compatibility — interoperability**: positive. A future
  third-party worker implementation (an `overdrive-worker-cloud-hypervisor`
  or `overdrive-worker-wasmtime` peer) gains a clear template.
- **Deployability**: positive. Phase 2+ split-binary deployments are
  mechanically free against this crate graph.

### Migration

Phase 1 is paper-only at the time of this ADR. The crate lands in
the same PRs as Slices 2 and the worker-subsystem startup wiring;
the integration test path for Slice 2 moves to the new crate's
`tests/integration/` directory. No pre-existing source moves; the
DISCUSS pre-decision (`ProcessDriver` in `overdrive-host`) was
explicitly subject to DESIGN revision per the DISCUSS wave-decisions
document.

## Compliance

- **ADR-0003 (crate-class labelling)**: `crate_class = "adapter-host"`
  declared; the new crate is correctly excluded from `dst-lint`'s
  banned-API scan (driver impls legitimately use real syscalls).
  Crate-class mechanism extended to a third `adapter-host` crate
  (`overdrive-host`, `overdrive-store-local`,
  `overdrive-control-plane` already; `overdrive-worker` joins).
- **ADR-0016 (`overdrive-host` extraction)**: original intent
  preserved (host-OS primitives stay; workload drivers move out).
  Refinement, not supersession.
- **ADR-0022 (`AppState::driver` extension)**: trait-object swap
  shape preserved; `Arc<dyn Driver>` is plugged in by the binary at
  composition time, with the impl now coming from `overdrive-worker`
  rather than `overdrive-host`. No control-plane signature changes.
- **ADR-0023 (action shim placement)**: shim contract stays
  "shim calls `Driver::*` against `&dyn Driver`"; only the impl
  crate changes.
- **ADR-0024 (`overdrive-scheduler` extraction)**: strategic
  precedent. Extract per architectural class, eagerly, when the seam
  is clear.
- **ADR-0025 (single-node startup wiring)**: hostname-fallback +
  `[node].id` config-override mechanism for `NodeId` resolution is
  unchanged. The `node_health` row writer relocates from
  control-plane bootstrap to worker-subsystem startup.
- **ADR-0026 (cgroup v2 direct writes)**: mechanism (direct
  `std::fs` writes, no `cgroups-rs`), resource enforcement
  (`cpu.weight` + `memory.max`), and warn-and-continue posture all
  unchanged. The workload-cgroup half of the responsibility splits
  out cleanly to the worker crate; the control-plane-cgroup half
  (`overdrive.slice/control-plane.slice/`) stays in
  `overdrive-control-plane`. Each subsystem owns its own cgroup
  hierarchy, mirroring whitepaper §4 *Workload Isolation on Co-located
  Nodes* exactly.
- **Workspace convention** (`.claude/rules/testing.md` § Workspace
  convention): `integration-tests = []` declaration is mandatory in
  every member's `Cargo.toml`. The new crate declares it (deliberate
  no-op for crates without integration tests; gating the slow lane
  for crates with them — which `overdrive-worker` will have, for
  Slice 2's real-cgroup integration test).

## References

- Whitepaper §3 — Architecture Overview; control plane vs node
  agent split.
- Whitepaper §4 — Control Plane (specifically *Control Plane and
  Worker on the Same Node*; *Workload Isolation on Co-located Nodes*).
- Whitepaper §5 — Node Agent.
- ADR-0003 — Core-crate labelling via `package.metadata.overdrive.crate_class`.
- ADR-0016 — `overdrive-host` extraction (host-OS adapters); the
  original intent this ADR restores.
- ADR-0021 — `AnyState` enum (lifecycle reconciler's State shape).
- ADR-0022 — `AppState::driver: Arc<dyn Driver>` extension; the
  trait-object swap surface this ADR's worker plugs into.
- ADR-0023 — Action shim placement; the shim that calls
  `Driver::start/stop/status` against the worker's impl.
- ADR-0024 — `overdrive-scheduler` extraction; the precedent for
  this ADR one level up.
- ADR-0025 — Single-node startup wiring; the boot sequence whose
  `node_health` row writer relocates to worker startup.
- ADR-0026 — cgroup v2 direct writes; the workload-cgroup half of
  the responsibility moves with `ProcessDriver`.
- `docs/feature/phase-1-first-workload/discuss/wave-decisions.md`
  — DISCUSS wave provisionally placed `ProcessDriver` in
  `overdrive-host`; the placement was explicitly subject to DESIGN
  revision.
- User ratification 2026-04-27 (orchestrator recommendation +
  "confirmed. proceed").
