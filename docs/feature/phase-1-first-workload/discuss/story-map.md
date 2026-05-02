# Story Map — phase-1-first-workload

## User: Ana, Overdrive platform engineer (distributed-systems SRE)

## Goal: Run `overdrive job submit` against the walking-skeleton control plane on her single-node dev host (control plane and worker co-located on one machine) and watch a real OS process come up under the lifecycle reconciler's convergence — placed by a deterministic scheduler, isolated from the control plane's own cgroup scope, and self-healing on crash.

This feature **is** the execution-layer extension of the `phase-1-control-plane-core` walking skeleton. It does NOT ship a new walking skeleton — it fills in the empty allocation rows that step 4 of the inherited `submit-a-job.yaml` journey was honest about being empty.

> **Phase 1 is single-node.** The control plane and worker are co-located
> on one machine. There is exactly one node — the local host — and it
> is implicit. There is no operator-facing node-registration verb, no
> taint, no toleration, no multi-node placement choice. The local node
> is a precondition, not a backbone activity.

## Backbone

User activities, left-to-right in chronological order over the lifetime of the feature:

| 1. Place an allocation | 2. Start a process | 3. Watch convergence | 4. Stop or recover |
|---|---|---|---|
| First-fit scheduler picks the local node when its capacity covers the job's resources; deterministic by BTreeMap iteration order even with N=1 | ProcessDriver in `overdrive-host` spawns a child process via `tokio::process` and confines it to a workload cgroup scope | Lifecycle reconciler converges declared replica count via `Action::StartAllocation` / `StopAllocation`, observable in `alloc status` | Operator stops a job (drains to Terminated) or watches the platform self-heal a crashed process; the control plane's own cgroup slice protects responsiveness even when the workload is bursting CPU on the same machine |

## Ribs (tasks under each activity)

### 1. Place an allocation

- 1.1 Scheduler module — pure first-fit function `schedule(nodes, job, current_allocs) -> Result<NodeId, PlacementError>` *(Walking Skeleton)*
- 1.2 `PlacementError` enum: `NoCapacity`, `NoHealthyNode`, each carrying actionable reason text *(Walking Skeleton)*
- 1.3 Capacity accounting: subtract running allocs' resources from node capacity to compute free *(Walking Skeleton)*
- 1.4 Deterministic ordering: candidate nodes iterated via BTreeMap (per `.claude/rules/development.md`) so first-fit is reproducible across runs (load-bearing for DST replay even at N=1) *(Walking Skeleton)*

### 2. Start a process

- 2.1 `ProcessDriver` struct in `overdrive-host` implementing the `Driver` trait *(Walking Skeleton)*
- 2.2 Child process spawn via `tokio::process::Command` *(Walking Skeleton)*
- 2.3 cgroup v2 scope creation: `overdrive.slice/workloads.slice/<alloc_id>.scope` (via `cgroups-rs` or direct cgroupfs writes — DESIGN picks) *(Walking Skeleton)*
- 2.4 PID tracking and persistence on `AllocationHandle` *(Walking Skeleton)*
- 2.5 `Driver::status` polls process state via PID + cgroup *(Walking Skeleton)*
- 2.6 `Driver::stop` SIGTERM → grace → SIGKILL; cgroup scope teardown *(Walking Skeleton)*
- 2.7 `CgroupPath` newtype wrapping the scope path *(Walking Skeleton)*

### 3. Watch convergence

- 3.1 `JobLifecycle` reconciler struct + `JobLifecycleView` (libSQL-hydrated retry/restart memory) *(Walking Skeleton)*
- 3.2 `AnyReconciler::JobLifecycle(JobLifecycle)` variant + match arms in `name`, `hydrate`, `reconcile` *(Walking Skeleton)*
- 3.3 `AnyReconcilerView::JobLifecycle(JobLifecycleView)` variant *(Walking Skeleton)*
- 3.4 `Action::StartAllocation { alloc_id, job_id, node_id, spec }`, `Action::StopAllocation { alloc_id }`, `Action::RestartAllocation { alloc_id }` enum variants *(Walking Skeleton)*
- 3.5 Action shim — runtime layer that consumes allocation-management Actions and calls into `Arc<dyn Driver>`, then writes `AllocStatusRow` *(Walking Skeleton)*
- 3.6 `AppState::driver: Arc<dyn Driver>` extension *(Walking Skeleton)*
- 3.7 Lifecycle reconciler registered in `run_server_with_obs()` *(Walking Skeleton)*
- 3.8 Backoff state in `JobLifecycleView` (restart_count, next_attempt_at) *(Walking Skeleton)*
- 3.9 New DST invariants: `JobScheduledAfterSubmission`, `DesiredReplicaCountConverges`, `NoDoubleScheduling`, `SchedulerRespectsNodeCapacity` *(Walking Skeleton)*

### 4. Stop or recover

- 4.1 CLI `overdrive job stop <id>` subcommand *(Walking Skeleton — bundled into Slice 3 alongside the lifecycle reconciler)*
- 4.2 `POST /v1/jobs/{id}:stop` handler (path shape DESIGN-owned) *(Walking Skeleton)*
- 4.3 Lifecycle reconciler reads stopped intent, emits `StopAllocation` *(Walking Skeleton)*
- 4.4 Server bootstrap: create `overdrive.slice/control-plane.slice/`, enrol the running process *(Walking Skeleton — Slice 4)*
- 4.5 cgroup v2 delegation pre-flight check (refuses to start with actionable error if missing) *(Walking Skeleton — Slice 4)*
- 4.6 CLI `alloc status` Pending-row rendering with reason text *(Walking Skeleton — bundled into Slice 1's PlacementError surfacing)*

## Walking Skeleton

This feature does NOT introduce a new product-level walking skeleton — it
extends the one shipped by `phase-1-control-plane-core`. Inside this
feature, the absolute minimum end-to-end value slice that proves the
execution layer works at all is:

> "Operator submits a 1-replica process job; the local node (ample
> capacity) accepts it; ProcessDriver starts a real OS process inside
> a workload cgroup scope; `alloc status` shows `Running` with the
> matching `alloc_id`; control plane and workload are in distinct
> cgroups."

This is **Slices 1 through 3** (in dependency order) of the carpaccio
slicing below. Slice 4 (control-plane cgroup isolation hardening) is
an increment on top — demonstrably shippable on its own, but it does
not on its own deliver the "watch a real process come up" value.

## Release slices (elephant carpaccio) — 4-slice plan

Four slices, each ≤1-2 days end-to-end, each carrying a learning hypothesis
the slice can disprove, each with a production-data-shaped acceptance,
and each demonstrable in a single working session.

### Slice 1 — First-fit scheduler scaffold (single-node)

**Outcome**: A pure scheduler module (`schedule(nodes, job, allocs) -> Result<NodeId, PlacementError>`) lives in the control plane (or a dedicated crate — DESIGN picks). Phase 1 single-node: the input map carries exactly one entry (the local host); the function still iterates via BTreeMap (deterministic order is load-bearing for DST replay even at N=1), checks capacity, and returns the NodeId. No driver invocation; no real allocation lands. The function is exercised by a proptest using sim-shaped `NodeView` / `JobView` fixtures.

**Target KPI**: Determinism — calling `schedule(nodes, job, allocs)` twice with the same inputs returns the same NodeId or the same PlacementError.

**Hypothesis**: "If first-fit can't be expressed as a pure function over BTreeMap-ordered inputs, the scheduler is non-DST-replayable and every later DST invariant about scheduler behaviour fails. (Even with N=1, the determinism property has to hold so Phase 2+ multi-node is a content change, not a structural one.)"

**Disproves**: "The scheduler needs DB-backed memory or async I/O." (No — first-fit is a pure function over `(node_set, job, current_allocs)`.) Also: "With N=1 we don't need a real scheduler — the reconciler can pick the only node directly." (No — the placement function exists to be DST-replayable; N=1 is a degenerate case of the general predicate.)

**Delivers (story)**: US-01.

**Slice taste-test**:
- New components ≤ 4: scheduler module + PlacementError enum + capacity-accounting helper. Three components.
- No hypothetical abstractions: relies only on existing aggregates + Resources.
- Production-shaped AC: proptest against the function's contract; no I/O.
- IN scope: pure scheduler module, PlacementError, capacity accounting.
- OUT of scope: action shim, driver invocation, reconciler integration. **Taint/toleration is out of Phase 1 entirely.**

---

### Slice 2 — ProcessDriver (cgroup-aware, gated `integration-tests` feature)

**Outcome**: `ProcessDriver` in `overdrive-host` implements the `Driver` trait against `tokio::process` + cgroups v2. Default unit tests use `SimDriver` (no real processes); a Linux-only integration test (gated behind `integration-tests`) actually starts a child process inside `overdrive.slice/workloads.slice/<alloc_id>.scope`, asserts `/proc/<pid>/cgroup` matches, then stops it cleanly.

**Target KPI**: `Driver::start` returns a handle whose PID is alive and whose cgroup scope path exists on the host; `Driver::stop` removes the scope; `Driver::status` correctly reports Running → Terminated.

**Hypothesis**: "If we can't drive `tokio::process` + cgroup placement from a clean Driver impl in `overdrive-host` without polluting the core compile path, the adapter-host boundary doesn't actually pay for itself. Conversely, if we can, every future driver type (microvm, wasm) follows the same pattern."

**Disproves**: "We need to invent a new abstraction layer to manage cgroups separately from the driver." (No — the driver IS the cgroup-aware spawn site.)

**Delivers (story)**: US-02.

**Slice taste-test**:
- New components ≤ 4: ProcessDriver struct + CgroupPath newtype + tokio::process spawn helper + cgroup scope manager (could be one module). Four components, deliberately at the upper end.
- No hypothetical abstractions: depends on `Driver` trait already in `core` and `cgroups-rs` (or direct cgroupfs); both are real today.
- Production-shaped AC: integration-tests-gated test on Linux that starts a real `/bin/sleep 60` inside a real cgroup scope.
- IN scope: ProcessDriver, cgroup scope creation/teardown, PID tracking.
- OUT of scope: invocation from action shim (Slice 3), control-plane slice creation (Slice 4), right-sizing.

---

### Slice 3 — Job-lifecycle reconciler + action shim (the convergence loop closes; includes `job stop`)

**Outcome**: `Action::{StartAllocation, StopAllocation, RestartAllocation}` variants exist on the Action enum. `JobLifecycle` reconciler is registered alongside `noop-heartbeat`; it reads `desired` (job spec from the rkyv-hydrated IntentStore) and `actual` (current AllocStatusRow set), computes the diff, calls into the Slice 1 scheduler module, and emits `Action::StartAllocation`. The action shim (NEW runtime layer) consumes those actions, calls into `Arc<dyn Driver>` (Slice 2 ProcessDriver in production; SimDriver under DST), and writes the resulting AllocStatusRow back to ObservationStore. **`AppState` extends with `driver: Arc<dyn Driver>`.** This slice ALSO ships **`overdrive job stop <id>`** end-to-end (CLI + handler + reconciler reading stopped intent + action shim calling `Driver::stop`); stop is the inverse of start through the same lifecycle path. **`State` and `JobLifecycleView` shapes are defined here — this slice is blocked on a DESIGN decision for the `State` shape (see DoR item below)**.

**Target KPI**: Submitting a 1-replica job to the single-node cluster produces a Running allocation within N reconciler ticks; `overdrive job stop` drives it back to Terminated within N reconciler ticks; DST `JobScheduledAfterSubmission` and `DesiredReplicaCountConverges` invariants pass.

**Hypothesis**: "If we can't close the loop end-to-end via the §18 reconciler primitive (pure reconcile + async action shim) AND give the operator a clean stop-and-drain affordance, the §18 architectural commitment is performative. Conversely, if we can — and DST proves convergence — Phase 2+ reconciler work has a reference implementation."

**Disproves**: "The lifecycle reconciler must perform I/O." (No — it stays pure; the shim does the I/O.) Also: "`job stop` belongs in a separate slice from convergence." (No — stop is the inverse of start through the same shim; splitting them would force two slices to land the same I/O machinery.)

**Delivers (story)**: US-03.

**Slice taste-test**:
- New components ≤ 4: JobLifecycle reconciler + AnyReconciler/View variants (one extension each, two match-arm sites = treat as one), action shim, AppState driver extension, `job stop` CLI+handler. **Borderline at the upper limit because stop is bundled here**, but stop is an additive handler that mirrors `submit_job` and consumes the same action shim — marginal cost over a stop-less version is small.
- HARD DEPENDENCY: DESIGN must pick the `State` shape (see DoR). This is the codebase research's flagged structural blocker.
- Production-shaped AC: real ProcessDriver (under integration-tests feature) AND SimDriver (default lane via DST) both exercise the same shim.
- IN scope: lifecycle reconciler, action shim, allocation-management Action variants, AppState extension, lifecycle reconciler registration, three new DST invariants, `overdrive job stop` end-to-end.
- OUT of scope: control-plane slice (Slice 4), cgroup pre-flight check (Slice 4). **Taint/toleration is out of Phase 1 entirely.**

---

### Slice 4 — Control-plane cgroup isolation (slice creation + bootstrap enrolment)

**Outcome**: At server startup, `overdrive serve` creates `overdrive.slice/control-plane.slice/` (if not present) and enrols the running process into it. cgroup v2 delegation pre-flight check refuses to start with an actionable error if delegation is missing. ProcessDriver from Slice 2 already places workloads in `overdrive.slice/workloads.slice/<alloc_id>.scope`; this slice adds the symmetric control-plane side. New integration test (gated `integration-tests`) reads `/proc/self/cgroup` in the running server, asserts the control-plane slice path; runs a workload at high CPU and asserts CLI responsiveness stays below 100 ms.

**Target KPI**: When a workload bursts CPU, `overdrive cluster status` returns within 100 ms on localhost (a real CPU-isolation assertion — proven against a host kernel).

**Hypothesis**: "If the kernel isn't actually enforcing the slice split, the §4 'control plane runs in dedicated cgroups with kernel-enforced resource reservations' claim is paper. The integration test is the disproof attempt: if the workload starves the control plane, the test fails and we know the slice isn't doing what we claimed."

**Disproves**: "Slice creation can wait for systemd unit packaging in DEVOPS." (No — the in-process bootstrap is the SSOT for the slice topology; a future systemd unit can pre-create the parent slice but the server still owns the per-instance hierarchy.) Also: "On a single-node co-located host, cgroup isolation is a Phase 2+ luxury." (No — Phase 1 is single-node co-located, which is exactly the topology cgroup isolation defends against.)

**Delivers (story)**: US-04.

**Note on GH issue #20**: Issue #20 is *"Control-plane cgroup isolation + scheduler taint/toleration support"*. **This slice covers only the cgroup-isolation half**. The taint/toleration half is explicitly out of Phase 1 — with one node there is no placement choice for a taint to gate against. The user is expected to split GH #20 into two separate issues (cgroup-isolation vs taint/toleration) with the latter scheduled for the multi-node phase.

**Slice taste-test**:
- New components ≤ 4: cgroup pre-flight check, server-bootstrap CgroupManager (creates slice, enrols self), integration test harness with CPU pressure. Three components.
- No hypothetical abstractions: extends Slice 2's cgroup wiring symmetrically to the control plane.
- Production-shaped AC: real Linux-only integration test asserting kernel-enforced isolation.
- IN scope: server-bootstrap slice creation, control-plane process enrolment, pre-flight delegation check, isolation integration test.
- OUT of scope: right-sizing reconciler reading memory pressure (Phase 2+ §14), eBPF-based pressure detection. **Scheduler taint/toleration support — the other half of GH #20 — is OUT (multi-node concern).**

## Slice ordering — dependency chain + learning leverage

The codebase research identifies a clean dependency chain: **#15 (scheduler) → #14 (driver invoked from reconciler's emitted Actions) → #21 (reconciler invokes scheduler and dispatches into shim) → #20-cgroup (control-plane slice extends Slice 2's symmetric workload-side wiring)**. The 4-slice plan above follows this ordering directly:

1. **Slice 1 (Scheduler scaffold)** — the highest-uncertainty pure-function piece. Determinism is load-bearing for every later DST invariant. Lands first so Slice 3's reconciler has a known-deterministic placement function to call.
2. **Slice 2 (ProcessDriver)** — Linux-specific, real-OS-dependent, gated `integration-tests`. Independent of Slice 1 mechanically; can run in parallel.
3. **Slice 3 (Lifecycle reconciler + action shim + `job stop`)** — the convergence loop. Depends on Slices 1 and 2. **Blocked by a DESIGN decision on `State` shape** — DoR flag.
4. **Slice 4 (Control-plane cgroup isolation)** — extends slice 2's cgroup wiring symmetrically. Last because the hypothesis ("kernel-level isolation actually holds under workload pressure") is best disproved with a real-Linux integration test against a fully-running platform.

## Priority Rationale

All four slices are inside the walking-skeleton extension. None of them on their own delivers the full "submit job + watch run" experience, but each is **demonstrable in isolation** and disproves a named hypothesis if wrong.

| Priority | Slice | Why this order |
|---|---|---|
| 1 | Slice 1 (Scheduler scaffold) | Highest-uncertainty pure-function piece; proving determinism (even at N=1) before any I/O lands isolates the failure modes. Can run in parallel with Slice 2. |
| 2 | Slice 2 (ProcessDriver) | Independent of Slice 1 mechanically; the only Linux-host-coupled slice; isolated under `integration-tests` feature gate. Can run in parallel with Slice 1. |
| 3 | Slice 3 (Lifecycle reconciler + `job stop`) | The convergence loop. Depends on Slices 1-2. **Hard DoR dependency**: DESIGN must clarify `State` shape before crafter can start. |
| 4 | Slice 4 (Control-plane isolation) | Validates the structural defence-in-depth claim against a real Linux kernel. Last because it requires Slice 2 (ProcessDriver) to produce real workloads to assert against. |

Slices 1 and 2 can run in parallel.

## Slice taste-tests against the 4-slice plan

Re-running the elephant-carpaccio taste tests against the 4-slice plan:

| Property | Slice 1 | Slice 2 | Slice 3 | Slice 4 | Verdict |
|---|---|---|---|---|---|
| ≤4 new components | PASS (3) | PASS (4, upper end) | BORDERLINE (≈5 conceptually, including bundled `job stop`) | PASS (3) | OK — Slice 3's borderline is acceptable per its taste-test rationale |
| No hypothetical abstractions landing later | PASS | PASS | PASS | PASS | OK |
| Disproves a named pre-commitment | PASS | PASS | PASS | PASS | OK |
| Production-data-shaped AC | PASS (proptest) | PASS (Linux integration) | PASS (DST + Linux integration) | PASS (Linux integration) | OK |
| Demonstrable in single session | PASS | PASS | BORDERLINE (dense; demo is "submit → Running → stop → Terminated") | PASS | OK |
| Same-day dogfood moment | PASS (proptest output) | PASS (Linux dev workstation) | PASS (`cargo xtask dst` + Linux integration) | PASS (Linux dev workstation) | OK |

The 4-slice plan is **right-sized**: each slice is independently shippable, the borderlines on Slice 3 are acknowledged in its slice brief, and no slice cross-cuts more than 2 crates.

## Scope Assessment: PASS — 4 stories, 4 crates touched, estimated 4-6 days

- **Story count**: 4 stories (US-01 through US-04). Well within the ≤10 ceiling.
- **Bounded contexts / crates**: 4 (`overdrive-core` for Action variants + AnyReconciler variant; `overdrive-host` for ProcessDriver + CgroupPath newtype; `overdrive-control-plane` for scheduler + lifecycle reconciler + action shim + AppState extension + `job stop` handler + cgroup-isolation bootstrap; `overdrive-cli` for `job stop`). Within the ≤3-bounded-context oversized signal — `overdrive-sim` is touched only for invariant additions.
- **Walking-skeleton integration points**: 4 (lifecycle reconciler → scheduler, lifecycle reconciler → action shim, action shim → ProcessDriver, server bootstrap → cgroup hierarchy). Within the ≤5 oversized threshold.
- **Estimated effort**: 4-6 focused days (each slice ≤1-2 days, Slices 1 and 2 parallelisable, Slice 3 may stretch to 1-2 days due to the State + AppState + new variants + new invariants + `job stop` concentration).
- **Multiple independent user outcomes worth shipping separately**: no — the four slices are sequential on the same walking skeleton. Each demonstrably moves the operator forward, but none individually delivers the "watch a real process come up" outcome.
- **Verdict**: **RIGHT-SIZED** — 4 stories well below the upper end, 4 crates well-bounded, no slice cross-cuts more than 2 crates, all 4 integration points are familiar shapes from prior feature.

## Changelog

| Date | Change |
|---|---|
| 2026-04-27 | Initial story map for `phase-1-first-workload`. |
| 2026-04-27 | Scope correction — Phase 1 is single-node. Removed the "Register a node" backbone activity. Removed Slice 1 (node registration) and Slice 5 (taint/toleration). Re-numbered to a 4-slice plan: (1) first-fit scheduler scaffold, (2) ProcessDriver, (3) job-lifecycle reconciler + action shim + `job stop`, (4) control-plane cgroup isolation. Re-ran the elephant-carpaccio taste tests; 4-slice plan PASSES. GH #20's taint/toleration half deferred to a later phase (user expected to split #20 into two issues). |
