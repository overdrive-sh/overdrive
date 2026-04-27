# Research: Phase 1 First Workload — Codebase State

**Date**: 2026-04-27 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 22 local files

## Executive Summary

Feature B (phase-1-first-workload) delivers the walking skeleton outcome: "An operator submits a process job; scheduler places it; process driver starts it; job-lifecycle reconciler converges; control plane is cgroup-isolated from workload." Four GitHub issues are open (#14, #15, #20, #21). All their dependency issues (#7, #10, #12, #17) are closed and fully implemented.

The foundation is production-ready. The `Driver` trait, `SimDriver`, `AnyReconciler` enum-dispatch, `ReconcilerRuntime`, `EvaluationBroker`, the `Job`/`Node`/`Allocation` aggregates, `ObservationStore` (with `AllocStatusRow`, `NodeHealthRow`), and the full control-plane API surface (`POST /v1/jobs`, `GET /v1/nodes`, `GET /v1/allocs`) are all present. The four open issues divide cleanly into two layers — `overdrive-host` for process execution and cgroup isolation, `overdrive-control-plane` for scheduler and job-lifecycle reconciler — with sharp, well-defined integration points already wired by the dependency layer.

The most significant gap is that `State` in `reconciler.rs` is currently an opaque zero-field placeholder (`struct State;`). The job-lifecycle reconciler cannot be a pure function over `(desired, actual, view, tick)` until `State` carries real `Job`, `Node`, and `Allocation` data. This is the single largest design decision the DESIGN wave must resolve before DELIVER can begin on #21.

---

## Research Methodology

**Search Strategy**: Direct file reads of every named source file. Grepped for key symbols (`ProcessDriver`, `StartAllocation`, `JobLifecycle`, `scheduler`, `taint`, `tolerat`) across the crate tree to confirm absence of in-progress work.

**Source Selection**: All local — crate source files, ADR docs, feature distill docs, DST invariant evaluators.

**Quality Standards**: Every finding is sourced from file content read directly. No web fetch was performed — this is a codebase audit.

---

## Findings

### Finding 1: The `Driver` trait is fully defined; `SimDriver` is complete; `ProcessDriver` does not exist

**Evidence**: `crates/overdrive-core/src/traits/driver.rs` defines the full `Driver` trait with `start`, `stop`, `status`, `resize` methods; `AllocationSpec`, `AllocationHandle`, `AllocationState`, `Resources`, and `DriverType` (including `DriverType::Process`). `crates/overdrive-sim/src/adapters/driver.rs` implements `SimDriver` with configurable failure modes. Grep over `crates/` for `ProcessDriver` returns zero matches — no production implementation exists.

**Source**: `crates/overdrive-core/src/traits/driver.rs` lines 1–151; `crates/overdrive-sim/src/adapters/driver.rs` lines 1–99

**Confidence**: High

**Analysis**: Issue #14 (Process driver) has a clean landing zone. The trait is stable, the sim counterpart models exactly what the real implementation must do. The production implementation must live in `crates/overdrive-host/` as a new `src/driver.rs` module. Per ADR-0016, `overdrive-host` is the `adapter-host` crate for all production bindings of core port traits. The current `overdrive-host/src/lib.rs` explicitly names "Future: real `Driver`" as Phase 2+ wiring — issue #14 advances this.

---

### Finding 2: `overdrive-host` currently ships only `SystemClock`, `OsEntropy`, and a stub `TcpTransport`; no `Driver` or `Dataplane` implementation exists

**Evidence**: `crates/overdrive-host/src/lib.rs` exports only `SystemClock`, `OsEntropy`, `CountingOsEntropy`, and `TcpTransport`. The module doc states: "Phase 2 wires `TcpTransport` to `tokio::net::*` and adds host impls for `Dataplane`, `Driver`, `IntentStore`, `ObservationStore`, and `Llm`."

**Source**: `crates/overdrive-host/src/lib.rs` lines 1–31

**Confidence**: High

**Analysis**: The `ProcessDriver` for issue #14 belongs in `crates/overdrive-host/src/driver/process.rs` (or similar). It must declare `crate_class = "adapter-host"` (already declared at the crate level). The driver will use `tokio::process::Command`, which is an allowed dep in `adapter-host` crates. `dst-lint` scans only `core` crates — no lint gate will flag `Instant::now()` or `tokio::net::*` inside `overdrive-host`.

---

### Finding 3: `AnyReconciler` currently has exactly one production variant (`NoopHeartbeat`); `JobLifecycle` is absent and must be added

**Evidence**: `crates/overdrive-core/src/reconciler.rs` lines 732–819 show `AnyReconciler` with one production variant (`NoopHeartbeat`) and one `#[cfg(feature = "canary-bug")]` variant. The module-level doc comment at lines 44–51 makes the extension contract explicit: "Adding a new first-party reconciler means adding one variant and one match arm in each of `name`, `hydrate`, and `reconcile`." `AnyReconcilerView` currently has only `Unit`.

**Source**: `crates/overdrive-core/src/reconciler.rs` lines 720–819

**Confidence**: High

**Analysis**: Issue #21 (job-lifecycle reconciler) requires:
1. A new `JobLifecycle` struct implementing `Reconciler` with a real `View` type (backoff state, placement history, retry counts).
2. A new `AnyReconciler::JobLifecycle(JobLifecycle)` variant.
3. A new `AnyReconcilerView::JobLifecycle(JobLifecycleView)` variant.
4. Match arms in `AnyReconciler::name()`, `AnyReconciler::hydrate()`, and `AnyReconciler::reconcile()`.

The reconciler must be registered in `overdrive-control-plane/src/lib.rs::run_server_with_obs()` alongside `noop_heartbeat()`.

---

### Finding 4: `State` is a zero-field opaque placeholder; no `Job`/`Node`/`Allocation` data flows through `reconcile`

**Evidence**: `crates/overdrive-core/src/reconciler.rs` lines 342–352: "Opaque placeholder for the `desired` / `actual` state handed to a reconciler. Phase 2+ replaces with the real shape when a reconciler dereferences it; Phase 1 reconcilers (just `NoopHeartbeat`) treat `State` as opaque." The struct has no fields: `pub struct State;`

**Source**: `crates/overdrive-core/src/reconciler.rs` lines 342–352

**Confidence**: High

**Analysis**: This is the single largest design gap for issue #21. The `reconcile(desired: &State, actual: &State, ...)` signature exists, but `State` carries no data. The job-lifecycle reconciler needs:
- `desired` to carry: the `Job` spec (replica count, resources, driver type)
- `actual` to carry: the current set of `Allocation` records and their observed states from `ObservationStore`

Two design options exist:
- **Option A**: Replace `pub struct State;` with a generic/concrete struct carrying `BTreeMap<AllocationId, AllocStatusRow>` and `Option<Job>`. This is a breaking change to the `Reconciler` trait's input contract.
- **Option B**: Keep `State` opaque but add typed projection methods (`State::job()`, `State::alloc_status()`) — still requires the runtime to populate it with real data.

The DESIGN wave for issue #21 must decide this. ADR-0013 §2a's future `AnyReconciler` variants section lists `JobLifecycle` explicitly as a future variant but does not specify the `State` shape.

---

### Finding 5: `Action` enum has no allocation-management variants; `StartAllocation`, `StopAllocation`, `MigrateAllocation`, `RestartAllocation` do not exist

**Evidence**: `crates/overdrive-core/src/reconciler.rs` lines 359–407: `Action` has three variants: `Noop`, `HttpCall`, and `StartWorkflow`. The module doc states "Phase 1 ships `Noop`, `HttpCall`, and a `StartWorkflow` placeholder." No allocation-management actions exist.

**Source**: `crates/overdrive-core/src/reconciler.rs` lines 359–407

**Confidence**: High

**Analysis**: Issue #21 requires new `Action` variants. Based on whitepaper §18 "Built-in Primitives" and the roadmap-issues note (1.12: "start/stop/migrate/restart"), the minimum set is:
- `Action::StartAllocation { alloc_id, job_id, node_id, spec }`
- `Action::StopAllocation { alloc_id }`
- `Action::MigrateAllocation { alloc_id, target_node_id }` (may be Phase 3 scope)
- `Action::RestartAllocation { alloc_id }`

These actions are consumed by the runtime's action shim, which dispatches to the `Driver` trait. In Phase 1, the dispatch target will be `ProcessDriver` in `overdrive-host`. The relationship: reconciler emits `Action::StartAllocation` → runtime shim calls `driver.start(spec)` → returns `AllocationHandle` → runtime writes `AllocStatusRow { state: Running }` to `ObservationStore`.

---

### Finding 6: No scheduler code exists anywhere in the codebase

**Evidence**: Grep for `scheduler`, `first.fit`, `bin.pack`, `placement` across `crates/` returns matches only in `SimDriver` (the driver adapter description), `overdrive-cli/src/commands/alloc.rs` (CLI render logic), and `render_alloc_status.rs` (test fixtures). No scheduler module, no scheduler trait, no placement logic exists.

**Source**: Grep result across `crates/`; `walking-skeleton.md` line 186: "No scheduler. `alloc status` shows an explicit empty state naming `phase-1-first-workload` as the next feature."

**Confidence**: High

**Analysis**: Issue #15 starts from scratch. Per roadmap-issues 1.8: "first-fit" heuristic, with taint/toleration support arriving in issue #20 (1.11). The scheduler's inputs are available:
- `Node` aggregate (capacity: `cpu_milli`, `memory_bytes`) — in `crates/overdrive-core/src/aggregate/mod.rs`
- `NodeHealthRow` via `ObservationStore::node_health_rows()` — available in `AppState::obs`
- `Job` aggregate (resource requirements, replica count) — in `IntentStore` via `IntentKey::for_job()`
- `Allocation` aggregate (node binding) — in `IntentStore` via `IntentKey::for_allocation()`

A first-fit scheduler reads available `NodeHealthRow` records, filters by taint/toleration rules (issue #20), sums already-placed allocation resources from `AllocStatusRow`, picks the first node with enough remaining capacity, and emits `Action::StartAllocation`. The scheduler can live in `overdrive-control-plane` as a new module or can be part of the job-lifecycle reconciler itself. The DESIGN wave must decide the boundary.

---

### Finding 7: No taint/toleration fields exist on `Node` or `Job` aggregates

**Evidence**: `crates/overdrive-core/src/aggregate/mod.rs` defines `Node { id: NodeId, region: Region, capacity: Resources }` and `Job { id: JobId, replicas: NonZeroU32, resources: Resources }`. Neither carries taint or toleration fields. `NodeSpecInput` and `JobSpecInput` also lack these fields.

**Source**: `crates/overdrive-core/src/aggregate/mod.rs` lines 155–255

**Confidence**: High

**Analysis**: Issue #20 (cgroup isolation + taint/toleration) requires adding:
- `taints: Vec<Taint>` to `Node` / `NodeSpecInput`
- `tolerations: Vec<Toleration>` to `Job` / `JobSpecInput`

Per whitepaper §4: the canonical taint is `control-plane:NoSchedule`. The DESIGN wave must define the `Taint` and `Toleration` newtypes (effect: `NoSchedule` / `PreferNoSchedule`, key, value) per `.claude/rules/development.md` strict newtype discipline. These additions must be backward-compatible (rkyv schema evolution rules from `development.md` apply: additive-only changes).

---

### Finding 8: No systemd slice / cgroup isolation configuration exists; `overdrive-host` has no cgroup v2 wiring

**Evidence**: Grepping for `cgroup`, `overdrive.slice`, `control-plane.slice`, `workloads.slice` across `crates/` returns zero matches. No cgroup v2 configuration code exists.

**Source**: Grep result across the workspace

**Confidence**: High

**Analysis**: Issue #20 (control-plane cgroup isolation) requires:
1. The `ProcessDriver::start()` implementation must place each workload process into `/overdrive.slice/workloads.slice/<job-id>.scope` using the cgroups v2 API (via `cgroups-rs` or direct `cgroupfs` writes).
2. The control-plane itself must be confined to `/overdrive.slice/control-plane.slice/` at bootstrap — typically done in the systemd service unit or via a `CgroupManager` struct that writes the hierarchy at server startup.
3. The `resources: Resources { cpu_milli, memory_bytes }` from `AllocationSpec` maps to cgroup `cpu.weight` and `memory.max` settings on the workload scope.

This work spans both `overdrive-host` (the `ProcessDriver` implementation of cgroup placement) and `overdrive-cli` / server bootstrap (enrolling the control-plane process into its own reserved slice).

---

### Finding 9: `AppState` and server boot are wired; adding scheduler + reconciler requires only registration, not restructuring

**Evidence**: `crates/overdrive-control-plane/src/lib.rs` lines 148–281 show `run_server_with_obs()` which: opens `LocalIntentStore`, constructs `ReconcilerRuntime`, registers `noop_heartbeat()`, builds `AppState { store, obs, runtime }`. The router wires five handlers. The `ReconcilerRuntime::register()` API accepts any `AnyReconciler` variant.

**Source**: `crates/overdrive-control-plane/src/lib.rs` lines 148–281; `crates/overdrive-control-plane/src/reconciler_runtime.rs` lines 88–99

**Confidence**: High

**Analysis**: Adding the job-lifecycle reconciler to the server boot requires only:
1. `runtime.register(job_lifecycle()))?;` in `run_server_with_obs()` — one line.
2. The `job_lifecycle()` factory function mirrors `noop_heartbeat()`.
3. `AppState` does not need new fields if the Driver is accessed through a new `Arc<dyn Driver>` field or through the reconciler's action shim. However, the action shim that dispatches `Action::StartAllocation` to the driver will need a `Driver` reference, which likely requires `AppState` to carry `Arc<dyn Driver>`.

This `AppState` extension is a deliberate design decision for the DESIGN wave.

---

### Finding 10: `ObservationStore` has `alloc_status_rows()` and `node_health_rows()`; scheduler and reconciler can read placement state without new trait methods

**Evidence**: `crates/overdrive-core/src/traits/observation_store.rs` defines `alloc_status_rows() -> Result<Vec<AllocStatusRow>>` and `node_health_rows() -> Result<Vec<NodeHealthRow>>` on the `ObservationStore` trait. `AllocStatusRow` carries `{ alloc_id, job_id, node_id, state, updated_at }`.

**Source**: `crates/overdrive-core/src/traits/observation_store.rs` lines 226–246

**Confidence**: High

**Analysis**: The scheduler reads `node_health_rows()` to enumerate live nodes and their regions. It reads `alloc_status_rows()` filtered by `job_id` to count already-running replicas before deciding how many new allocations to start. Both are point-in-time snapshot reads — appropriate for the scheduling hot path.

The job-lifecycle reconciler, operating as a pure function over `(desired, actual, view, tick)`, receives these rows via the `actual: &State` parameter — once `State` is populated by the runtime with the current observation snapshot. The reconciler does not call `obs.alloc_status_rows()` directly; the runtime pre-hydrates `State` from the observation store before calling `reconcile`.

---

### Finding 11: DST invariant catalogue has no first-workload–specific invariants; the existing catalogue covers reconciler primitives

**Evidence**: `crates/overdrive-sim/src/invariants/mod.rs` defines `Invariant::ALL` with 11 variants. None name scheduler, process driver, job-lifecycle, or cgroup isolation. The three Phase-1-control-plane-core SCAFFOLD invariants (`AtLeastOneReconcilerRegistered`, `DuplicateEvaluationsCollapse`, `ReconcilerIsPure`) are fully implemented with evaluators. `ReconcilerIsPure` uses a twin-invocation check; `AtLeastOneReconcilerRegistered` checks registry count.

**Source**: `crates/overdrive-sim/src/invariants/mod.rs` lines 30–121; `crates/overdrive-sim/src/invariants/evaluators.rs` lines 640–875

**Confidence**: High

**Analysis**: Feature B needs new DST invariants. Candidates based on whitepaper §21 "Liveness" and "Convergence" sections:
- `JobScheduledAfterSubmission` — `assert_eventually!`: a submitted job has at least one `AllocStatusRow` within N ticks.
- `DesiredReplicaCountConverges` — `assert_eventually!`: `alloc_status` running count == `job.replicas` after reconciler runs.
- `NoDoubleScheduling` — `assert_always!`: each allocation appears on exactly one node.
- `ProcessDriverStartsOnStart` — allocated job transitions to `Running` state when `ProcessDriver::start()` succeeds.

These would go in new evaluator functions in `crates/overdrive-sim/src/invariants/evaluators.rs` and new variants in `Invariant` enum, with matching `as_canonical()` arms and `ALL` entries.

---

### Finding 12: Existing API handlers return honest empty state for nodes and allocs; no node-registration endpoint exists

**Evidence**: `crates/overdrive-control-plane/src/handlers.rs` lines 276–326 implement `alloc_status` and `node_list` handlers that read from `ObservationStore` and return `{"rows": []}` on empty state. There is no `POST /v1/nodes` handler, no node-registration endpoint. The walking skeleton doc (line 186) states "No real node agent / driver. `node list` shows an explicit empty state."

**Source**: `crates/overdrive-control-plane/src/handlers.rs` lines 276–326; `walking-skeleton.md` line 186

**Confidence**: High

**Analysis**: For issue #15 (scheduler) and #14 (process driver), the Phase 1 first-workload scenario needs a way for nodes to be known to the scheduler. Two options exist:
- **Option A**: Register nodes via the CLI (`overdrive node register`) which writes a `Node` aggregate to the `IntentStore` and a `NodeHealthRow` to the `ObservationStore`. This requires a new `POST /v1/nodes` handler.
- **Option B**: The scheduler operates against nodes pre-seeded in the `IntentStore` at server startup (single-node dev mode where the host node registers itself at boot).

The DESIGN wave must decide. The `NodeSpecInput` type and `Node::new()` constructor exist; adding a `POST /v1/nodes` handler is a straightforward addition following the `submit_job` pattern.

---

## Source Analysis

| Source | Type | Confidence | Cross-verified |
|---|---|---|---|
| `crates/overdrive-core/src/traits/driver.rs` | Codebase | High | Yes — cross-refs with sim |
| `crates/overdrive-sim/src/adapters/driver.rs` | Codebase | High | Yes — implements trait |
| `crates/overdrive-core/src/reconciler.rs` | Codebase | High | Yes — cross-refs with runtime |
| `crates/overdrive-control-plane/src/reconciler_runtime.rs` | Codebase | High | Yes — wires AnyReconciler |
| `crates/overdrive-control-plane/src/eval_broker.rs` | Codebase | High | Yes — used by runtime |
| `crates/overdrive-core/src/aggregate/mod.rs` | Codebase | High | Yes — cross-refs handlers |
| `crates/overdrive-core/src/traits/observation_store.rs` | Codebase | High | Yes — used by handlers |
| `crates/overdrive-control-plane/src/handlers.rs` | Codebase | High | Yes — uses aggregates + stores |
| `crates/overdrive-control-plane/src/lib.rs` | Codebase | High | Yes — boot path |
| `crates/overdrive-host/src/lib.rs` | Codebase | High | Yes — ADR-0016 cross-ref |
| `crates/overdrive-sim/src/invariants/mod.rs` | Codebase | High | Yes — evaluators |
| `crates/overdrive-sim/src/invariants/evaluators.rs` | Codebase | High | Yes — invariant bodies |
| `docs/product/architecture/adr-0013-reconciler-primitive-runtime.md` | ADR | High | Yes — reconciler.rs |
| `docs/product/architecture/adr-0016-overdrive-host-extraction-and-adapter-host-rename.md` | ADR | High | Yes — host/lib.rs |
| `docs/feature/phase-1-control-plane-core/distill/walking-skeleton.md` | Feature doc | High | Yes — handlers.rs |
| `.github/roadmap-issues.md` | Planning | High | Yes — whitepaper §18 |
| `docs/product/architecture/brief.md` | Architecture | High | Yes — port table |

---

## Gap Analysis Per Issue

### Issue #14 — Process driver (tokio::process + cgroups v2)

**What exists:**
- `Driver` trait fully defined in `overdrive-core` (stable API)
- `SimDriver` fully implemented in `overdrive-sim` (reference implementation)
- `overdrive-host` crate exists with `adapter-host` class, correct dep structure
- `DriverType::Process` variant exists in the enum
- `AllocationSpec` carries `resources: Resources { cpu_milli, memory_bytes }` — the cgroup limit inputs

**What is missing:**
- `crates/overdrive-host/src/driver/` module — does not exist
- `ProcessDriver` struct implementing `Driver` — does not exist
- cgroups v2 API integration (`cgroups-rs` or direct cgroupfs) — not in workspace deps
- Child process management via `tokio::process::Command` — not present
- PID tracking for `AllocationHandle::pid` — not populated by any driver
- cgroup scope creation: `/overdrive.slice/workloads.slice/<job-id>.scope`

**Integration needs:**
- `overdrive-host/Cargo.toml` must add `tokio` (process feature) and a cgroups crate
- `dst-lint` exempts `adapter-host` crates — `Instant::now()` and process spawning are allowed

---

### Issue #15 — Basic scheduler (first-fit)

**What exists:**
- `Node` aggregate with `capacity: Resources` — the scheduling input
- `Job` aggregate with `resources: Resources` and `replicas: NonZeroU32`
- `Allocation` aggregate linking `job_id` + `node_id`
- `ObservationStore::alloc_status_rows()` — current placement state
- `ObservationStore::node_health_rows()` — live node list
- `IntentKey::for_allocation()` — key derivation for writing new allocations
- `IntentStore::put()` / `put_if_absent()` — write allocation decisions

**What is missing:**
- Any scheduler module, struct, or function
- `POST /v1/nodes` handler for node registration (or alternative node-seeding mechanism)
- Scheduler invocation trigger (the reconciler calls it, or it is called by a dedicated scheduler reconciler)
- First-fit capacity comparison logic: `node.capacity - sum(running_allocs.resources) >= job.resources`
- Allocation ID generation (needs the `Entropy` trait or `AllocationId` from UUID)

**Key design question**: Is the scheduler a module called by the job-lifecycle reconciler, or a separate `AnyReconciler::Scheduler` variant? Roadmap note 1.8 says "taint/toleration support in 1.11" — taint filtering is likely in the scheduler regardless.

---

### Issue #20 — Control-plane cgroup isolation + scheduler taint/toleration

**What exists:**
- `Node` aggregate (no taint fields)
- `Job` aggregate (no toleration fields)
- `NodeSpecInput` / `JobSpecInput` (no taint/toleration fields)
- `ProcessDriver` design intent (issue #14)

**What is missing:**
- `Taint` and `Toleration` newtypes — not defined anywhere
- `taints: Vec<Taint>` field on `Node` / `NodeSpecInput`
- `tolerations: Vec<Toleration>` field on `Job` / `JobSpecInput`
- Taint-matching logic in the scheduler (effect: `NoSchedule` / `PreferNoSchedule`)
- Default `control-plane:NoSchedule` taint application at node-registration time for control-plane nodes
- Control-plane cgroup slice creation at server startup: `/overdrive.slice/control-plane.slice/`
- cgroup memory/CPU reservation for the control-plane slice

**rkyv schema evolution note**: Adding fields to `Node` and `Job` must be done per `.claude/rules/development.md` "additive-only schema migrations." Since `Node` and `Job` derive `rkyv::Archive`, adding new fields to the struct changes the archived byte layout. The proptest roundtrip invariant in `tests/acceptance/aggregate_roundtrip.rs` will catch any non-backward-compatible change. Serialization versioning strategy must be resolved in DESIGN.

---

### Issue #21 — Job-lifecycle reconciler

**What exists:**
- `Reconciler` trait fully defined (pre-hydration + TickContext)
- `AnyReconciler` enum with extension contract documented
- `ReconcilerRuntime` — registers and runs reconcilers
- `EvaluationBroker` — collapses duplicate evaluations
- `AllocStatusRow`, `NodeHealthRow` via `ObservationStore`
- `IntentKey::for_allocation()` — allocation key derivation
- `IntentKey::for_job()` — job key derivation

**What is missing:**
- `JobLifecycle` struct implementing `Reconciler`
- `AnyReconciler::JobLifecycle(JobLifecycle)` variant + match arms
- `AnyReconcilerView::JobLifecycle(JobLifecycleView)` variant
- `Action::StartAllocation`, `Action::StopAllocation`, `Action::RestartAllocation` variants
- **Critical**: `State` populated with real data (currently an opaque placeholder)
- Runtime action shim that dispatches `Action::StartAllocation` → `Driver::start()`
- A `Driver` reference accessible to the action shim (requires `AppState` extension or a dedicated dispatch layer)
- Registration call in `run_server_with_obs()`

**Critical constraint from ADR-0013 §2a**: The `JobLifecycle` reconciler's `View` type will likely be a struct carrying backoff counters, restart counts, and last-placed node per allocation. This is the first reconciler that will actually use `LibsqlHandle` — the Phase 1 `hydrate` implementations are all `Ok(())`. The libSQL schema management (CREATE TABLE IF NOT EXISTS) goes inside `hydrate()` per the pattern in ADR-0013 §2.

---

## Key Integration Points

The four issues form a directed dependency chain:

```
#20 cgroup isolation (Taint/Toleration on Node/Job)
      ↓ feeds
#15 scheduler (first-fit, taint-aware node selection)
      ↓ feeds
#21 job-lifecycle reconciler (emits Action::StartAllocation → calls scheduler)
      ↓ feeds
#14 process driver (executes Action::StartAllocation, places process in cgroup)
```

**Tighter integration notes:**

1. **Scheduler ↔ Reconciler boundary**: The job-lifecycle reconciler is the convergence loop. The scheduler is an allocation-placement algorithm called from within `reconcile()` (pure function — no I/O). The scheduler reads pre-hydrated node capacity and current allocation counts from `view: &JobLifecycleView`, not from the live store. This keeps `reconcile` pure. The scheduler's output is `Action::StartAllocation { ... }`, not a direct store write.

2. **Driver ↔ Action shim**: `Action::StartAllocation` is emitted by the reconciler (pure). The runtime's action shim (to be written, not yet existing) dispatches the action to `ProcessDriver::start()` and writes the resulting `AllocStatusRow { state: Running }` to `ObservationStore`. This shim is the async boundary that the reconciler itself must not cross.

3. **cgroup creation ↔ driver**: `ProcessDriver::start()` must create the cgroup scope for the workload AND create the process. The cgroup path `overdrive.slice/workloads.slice/<alloc-id>.scope` should be derived from `AllocationSpec::alloc` (the `AllocationId` newtype) to guarantee uniqueness.

4. **Node registration ↔ scheduler**: The scheduler needs at least one registered node to produce allocations. Feature B needs a node-registration mechanism that writes a `NodeHealthRow` to the `ObservationStore`. Whether this is a `POST /v1/nodes` HTTP handler, a CLI command, or automatic self-registration at `overdrive serve` startup is a DESIGN decision.

---

## Existing Test Scaffolding

### DST Invariants (all currently passing, none specific to Feature B)

| Invariant | Status | Notes |
|---|---|---|
| `SingleLeader` | Implemented, passes | Stub topology; Phase 2 replaces with real Raft |
| `IntentNeverCrossesIntoObservation` | Implemented, passes | Structural guard |
| `SnapshotRoundtripBitIdentical` | Implemented, passes | roundtrip on `LocalIntentStore` |
| `SimObservationLwwConverges` | Implemented, passes | CR-LWW convergence |
| `ReplayEquivalentEmptyWorkflow` | Implemented, passes | Phase 1 placeholder |
| `EntropyDeterminismUnderReseed` | Implemented, passes | `SimEntropy` |
| `AtLeastOneReconcilerRegistered` | Implemented, passes | Registry non-empty |
| `DuplicateEvaluationsCollapse` | Implemented, passes | Broker storm-mitigation |
| `BrokerDrainOrderIsDeterministic` | Implemented, passes | Deterministic drain |
| `ReconcilerIsPure` | Implemented, passes | Twin-invocation purity check |
| `IntentStoreReturnsCallerBytes` | Implemented, passes | ADR-0020 regression guard |

**Feature B needs new DST invariants** (not yet scaffolded):
- `JobScheduledAfterSubmission` — liveness
- `DesiredReplicaCountConverges` — convergence
- `NoDoubleScheduling` — safety
- `SchedulerRespectsNodeCapacity` — safety
- `SchedulerHonorsTaintNoSchedule` — safety (issue #20)

### Acceptance / Integration tests relevant to Feature B

Walking-skeleton acceptance tests in `crates/overdrive-cli/tests/integration/walking_skeleton.rs` currently test WS-1/WS-2/WS-3 — job submit, cluster status, idempotency. They assert an empty `alloc_status` response explicitly (the Feature B scenario is explicitly marked "not part of the walking skeleton"). Feature B will require its own acceptance test suite. The test-scenarios for `phase-1-first-workload` do not yet exist (no `docs/feature/phase-1-first-workload/` directory found).

---

## ADR Decisions Already Made (Relevant to Feature B)

| ADR | Decision | Impact on Feature B |
|---|---|---|
| ADR-0003 | `crate_class` taxonomy: `core | adapter-host | adapter-sim | binary` | `ProcessDriver` goes in `overdrive-host` (class `adapter-host`) |
| ADR-0013 | `AnyReconciler` enum-dispatch; `reconcile` is pure sync; `hydrate` is the only async I/O point; `State` is currently opaque; new reconcilers add variants | Job-lifecycle reconciler adds `AnyReconciler::JobLifecycle` variant; `State` must be populated with real data |
| ADR-0013 §2a | Future variants listed: `JobLifecycle(JobLifecycle)` (commented example) | Confirms the extension pattern for issue #21 |
| ADR-0016 | `overdrive-host` is the `adapter-host` crate for production bindings | `ProcessDriver` lives in `overdrive-host`; no real `Driver` in `core` or `sim` |
| ADR-0011 | `Node`, `Job`, `Allocation` aggregates live in `overdrive-core::aggregate` | Taint/toleration fields (issue #20) must be added to existing structs |
| ADR-0012 | `SimObservationStore` (now `LocalObservationStore`) is Phase 1 production observation store | Scheduler and reconciler read node/alloc state from this store |
| ADR-0005 | Integration tests gated behind `integration-tests` feature; every workspace member declares it | New integration tests for Feature B follow this pattern |
| ADR-0019 | TOML operator config format | Job spec with `taint`/`toleration` fields follows TOML shape |
| Whitepaper §18 | `Action::StartAllocation` is a typed action emitted by reconciler; dispatched by runtime shim to `Driver::start()` | Phase 1 does not yet have the action shim; it must be written |

---

## Open Questions

The following questions are explicitly ambiguous and must be resolved in the DESIGN wave before DELIVER can begin.

### Q1 — What is the real `State` shape?

The `reconcile(desired: &State, actual: &State, ...)` signature uses an opaque `State` placeholder. The job-lifecycle reconciler is the first reconciler that must dereference it. The DESIGN wave must define what `State` carries and how the runtime populates it from `IntentStore` and `ObservationStore` before calling `reconcile`.

**Options**: A generic/parameterized `State<D, A>` carrying typed desired/actual projections; a concrete `State` struct with `BTreeMap<AllocationId, AllocStatusRow>` and `Option<Job>`; or a typed-per-reconciler approach where each variant of `AnyReconciler` gets a matching `AnyState` variant. The last approach mirrors the `AnyReconcilerView` pattern already in place.

### Q2 — Is the scheduler a module or a reconciler variant?

The roadmap-issues entry 1.8 lists "Basic scheduler (first-fit)" separately from 1.12 "Job-lifecycle reconciler." This implies two separate components. However, a standalone scheduler reconciler (`AnyReconciler::Scheduler`) would need to read `Job` specs and produce `Allocation` records — similar logic to the job-lifecycle reconciler. The DESIGN wave must decide whether these are one reconciler or two, and if two, how they coordinate without racing.

### Q3 — How are nodes registered in the single-node walking skeleton for Feature B?

The scheduler needs at least one `NodeHealthRow` in the `ObservationStore` to produce any allocation. The DESIGN wave must specify: does the server auto-register the local node at boot, or does the operator register a node via `overdrive node register`? The latter requires a new `POST /v1/nodes` handler. The former is simpler for the walking skeleton but deviates from the multi-node mental model.

### Q4 — Where does the action shim live, and how does it get a `Driver` reference?

The reconciler emits `Action::StartAllocation`. Something must consume this action and call `driver.start(spec).await`. In Phase 1, this is the `ProcessDriver` in `overdrive-host`. The shim must be async (because `Driver::start` is async), and it must live in a wiring layer that can hold both `Arc<dyn Driver>` and `Arc<dyn ObservationStore>`. `AppState` is the natural holder. The DESIGN wave must extend `AppState` with a `driver: Arc<dyn Driver>` field (or a `driver_registry: DriverRegistry`).

### Q5 — How does rkyv schema evolution apply to adding `taints`/`tolerations` to `Node`/`Job`?

`Node` and `Job` derive `rkyv::Archive`. Adding fields changes the archived byte layout. Existing stored bytes in `LocalIntentStore` will fail deserialization after the schema change. The DESIGN wave must decide: (a) is a schema migration required before Feature B ships (bumping a version field), or (b) does Feature B assume a clean store (acceptable for Phase 1 dev-only deployments)?

### Q6 — What DST invariants should Feature B add to the catalogue?

The existing `Invariant` enum in `overdrive-sim` must be extended with Feature B invariants for the `--only <NAME>` DST gate to work. The DESIGN wave should enumerate the full set and add them as SCAFFOLD variants (with `SCAFFOLD: true` annotations in comments) so the CI catalogue is stable before DELIVER wires the evaluators.

---

## Research Metadata

Duration: ~1 session | Files read: 22 | Crates examined: 6 (`overdrive-core`, `overdrive-host`, `overdrive-sim`, `overdrive-control-plane`, `overdrive-cli`, `overdrive-store-local`) | ADRs read: 5 | Feature docs read: 3 | Confidence: High across all findings — all claims sourced from file contents with line references.
