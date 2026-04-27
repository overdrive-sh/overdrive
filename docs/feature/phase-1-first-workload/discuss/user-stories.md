<!-- markdownlint-disable MD024 -->

# User Stories — phase-1-first-workload

Four LeanUX stories, each delivering a single carpaccio slice from
`story-map.md`. All stories share the persona (Ana, Overdrive platform
engineer) and the vision context from `docs/product/vision.md`,
`docs/product/jobs.yaml` (J-OPS-003 added by this feature; J-OPS-002 +
J-PLAT-001/2/3 still active), and the platform commitments from
`docs/whitepaper.md` §4 (Workload isolation on co-located nodes), §6
(Process driver), and §18 (Job-lifecycle reconciler).

This feature **extends** the walking skeleton landed by
`phase-1-control-plane-core`. Every System Constraint from that feature
still applies; the additions here are the execution-layer constraints
that come with starting real OS processes.

> **Phase 1 is single-node.** Control plane and worker run co-located
> on one machine. There is exactly one node — the local host — and it
> is implicit. There is no operator-facing node-registration verb, no
> taint, no toleration, no multi-node placement choice. The local node
> is a precondition shared by every story in this file. The user
> stories for node registration and taint/toleration that previously
> appeared in this document were pulled per the 2026-04-27 scope
> correction (see `wave-decisions.md`).

## System Constraints (cross-cutting)

These extend the constraints from the prior feature — every constraint
from `docs/feature/phase-1-control-plane-core/discuss/user-stories.md`
still applies verbatim. The additional constraints specific to this
feature:

- **Reconciler purity is non-negotiable, even for the lifecycle
  reconciler.** The job-lifecycle reconciler shipped here MUST satisfy
  the existing `ReconcilerIsPure` DST invariant. No `.await`, no
  wall-clock reads, no direct store writes inside `reconcile`.
  Wall-clock comes from `tick.now`. The action shim is the I/O
  boundary; the reconciler is not.
- **Determinism in the scheduler is load-bearing.** The scheduler
  module is a pure synchronous function. All internal collections
  driving iteration order MUST be `BTreeMap` per
  `.claude/rules/development.md` ordered-collection rule. A `HashMap`
  in the scheduler's hot path is a blocking violation. Phase 1
  single-node makes the BTreeMap a one-element map at runtime; the
  determinism property still has to hold (Phase 2+ multi-node is a
  content change, not a structural one).
- **Newtypes are STRICT for new identifiers.** `CgroupPath` is a NEW
  newtype shipped by this feature (in `overdrive-host`). It MUST have
  `FromStr` (validating, case-insensitive where appropriate per
  newtype completeness rule), `Display`, serde + rkyv derives, and
  full proptest round-trip coverage. Raw `String` for any cgroup-path
  is a blocking violation. `NodeId` and `AllocationId` are STRICT
  newtypes inherited from phase-1-foundation; the discipline applies
  unchanged even though Phase 1 has exactly one `NodeId` value at
  runtime.
- **No new fields on existing aggregates.** The `Node::taints` and
  `Job::tolerations` extensions originally proposed in this wave were
  pulled per the 2026-04-27 scope correction. Phase 1 ships zero
  schema changes on `Node` / `Job`; the rkyv aggregate-roundtrip
  proptest from `phase-1-control-plane-core` continues to pass
  byte-identical with no extension.
- **Real infrastructure is gated behind `integration-tests` feature.**
  Per `.claude/rules/testing.md`, anything that spawns a real process,
  touches real cgroups, or binds a real socket goes behind the feature
  flag. Default `cargo nextest run` exercises only the in-process
  Sim* trait envelope. ProcessDriver's real-process tests live under
  `crates/overdrive-host/tests/integration/`; cgroup-isolation tests
  under `crates/overdrive-control-plane/tests/integration/`.
- **Action shim is the only I/O boundary in the convergence loop.**
  Per ADR-0013 + `.claude/rules/development.md` §Reconciler I/O. The
  lifecycle reconciler emits `Action::StartAllocation` (data); the
  shim consumes it and calls `Driver::start` (I/O). No new path
  through which the reconciler talks to the driver directly.
- **Linux-only for cgroups.** Phase 1 does not target macOS / Windows
  for the cgroup-aware ProcessDriver. macOS dev hosts use SimDriver
  in the default lane and must run the integration tests in a Linux
  VM (matches the Tier 3 testing pattern in `.claude/rules/testing.md`).

---

## US-01: First-fit scheduler scaffold

### Problem

The walking skeleton needs a placement decision function before the
lifecycle reconciler can emit `Action::StartAllocation`. Without it,
the reconciler has no way to pick a target node — the placement
logic would either inline into the reconciler (compromising purity if
it grows) or be punted (compromising the convergence-loop closure
this feature exists to deliver). The hardest property a scheduler
function carries is **determinism**: same inputs MUST produce same
output, otherwise DST replay diverges and every later DST invariant
about scheduler behaviour becomes unreliable. Phase 1 is single-node
— the input map carries exactly one entry — but the determinism
property still has to hold so Phase 2+ multi-node is a content
change, not a structural one.

### Who

- Overdrive platform engineer wiring the lifecycle reconciler |
  motivated by the need for a pure deterministic placement function
  the reconciler can call from inside `reconcile(...)` without
  crossing the purity boundary.

### Solution

Land a pure synchronous function `schedule(nodes:
&BTreeMap<NodeId, NodeView>, job: &JobView, current_allocs:
&[AllocStatusRow]) -> Result<NodeId, PlacementError>` in
`overdrive-control-plane::scheduler` (DESIGN may move it to a
dedicated `overdrive-scheduler` crate if the boundary clarity wins
warrant it; either is acceptable). It enumerates nodes via BTreeMap
iteration, computes free capacity by subtracting running allocs'
resources from each node's total, and returns the first node with
sufficient capacity. `PlacementError` covers `NoCapacity { needed,
max_free }` and `NoHealthyNode`.

### Domain Examples

#### 1: Happy Path — Local node with capacity

Given `BTreeMap{NodeId("local") -> NodeView { capacity: 4000 mCPU,
8 GiB }}`, a job requesting 2000 mCPU / 4 GiB, and zero running
allocs. `schedule(...)` returns `Ok(NodeId("local"))`.

#### 2: Edge Case — Capacity accounting subtracts running allocs

Given `BTreeMap{NodeId("local") -> NodeView { capacity: 4000 mCPU,
8 GiB }}`, a running allocation consuming 3000 mCPU / 6 GiB on the
local node, and a new job requesting 2000 mCPU. `schedule(...)`
returns `Err(PlacementError::NoCapacity)` (only 1000 mCPU free).

#### 3: Error Boundary — Capacity exhausted

Given a single node with 4000 mCPU and 4 GiB free; a job requesting
8 GiB. `schedule(...)` returns `Err(PlacementError::NoCapacity {
needed: 8 GiB, max_free: 4 GiB })`.

### UAT Scenarios (BDD)

#### Scenario: Scheduler is deterministic for the same input

Given a fixed input `(nodes, job, allocs)`
When `schedule(...)` is called twice in succession
Then both calls return the same `Result<NodeId, PlacementError>`

#### Scenario: First-fit accepts the local node when capacity covers the job

Given the local node with sufficient capacity
And a job whose resources fit on the local node
When `schedule(...)` is called
Then the result is `Ok(<the_local_node_id>)`

#### Scenario: Scheduler rejects when no node has capacity

Given the local node with 4000 mCPU / 4 GiB free
And a job requesting 8 GiB
When `schedule(...)` is called
Then the result is `Err(PlacementError::NoCapacity { needed, max_free })`
And `needed == 8 GiB` and `max_free == 4 GiB`

#### Scenario: Capacity accounting subtracts running allocs

Given the local node with 4000 mCPU / 8 GiB total
And a running allocation consuming 3000 mCPU / 6 GiB on that node
When a 2000 mCPU job is submitted
Then `schedule(...)` returns `Err(PlacementError::NoCapacity)` (only 1000 mCPU free)

#### Scenario: Empty node set returns NoHealthyNode

Given an empty BTreeMap of nodes
When `schedule(...)` is called for any job
Then the result is `Err(PlacementError::NoHealthyNode)`

### Acceptance Criteria

- [ ] `schedule(nodes, job, current_allocs) -> Result<NodeId, PlacementError>` exists as a pure synchronous function with no `.await`, no `Instant::now`, no `rand::*`
- [ ] `PlacementError` enum carries `NoCapacity { needed: Resources, max_free: Resources }` and `NoHealthyNode`
- [ ] All internal iteration is via `BTreeMap` per `.claude/rules/development.md`
- [ ] A proptest covers determinism: for any valid `(nodes, job, allocs)` input, two successive calls return equal results
- [ ] A proptest covers BTreeMap-order invariance: inputs constructed in different traversal orders produce the same result
- [ ] `dst-lint` does not flag the scheduler module (which lives in `adapter-host` class `overdrive-control-plane`, not scanned, but the function should still be safely portable to a `core` crate if DESIGN later moves it)
- [ ] Documentation comment on `schedule` lists the determinism contract and the BTreeMap-only iteration rule

### Outcome KPIs

- **Who**: Overdrive platform engineer wiring the lifecycle reconciler
- **Does what**: calls `schedule(...)` from a pure reconciler body and gets a deterministic placement decision
- **By how much**: 100% of identical-input calls return identical results (proptest); 0 `HashMap` iterations in the scheduler hot path (grep)
- **Measured by**: proptest for determinism; manual review for BTreeMap-only iteration; future Slice 3 DST `SchedulerRespectsNodeCapacity` invariant (lands with the lifecycle reconciler that calls it)
- **Baseline**: greenfield — no scheduler exists today

### Technical Notes

- `NodeView` and `JobView` projection types: DESIGN may collapse these into `&Node` and `&Job` directly, OR define them as projection structs that the runtime hydrates from IntentStore + ObservationStore. The latter is more in line with the §18 `hydrate` pattern and is consistent with how Slice 3's `JobLifecycleView` is shaped.
- `current_allocs: &[AllocStatusRow]`: an in-memory snapshot of the relevant rows. The runtime hydrates this once per evaluation tick.
- `Resources` arithmetic: subtraction must handle the empty case cleanly. DESIGN owns whether to expose a `Resources::saturating_sub` helper.
- **Phase 1 is single-node** — the BTreeMap input has exactly one entry at runtime. The proptest generates arbitrary-cardinality maps to defend the determinism contract for Phase 2+; the production data shape is N=1.
- **Depends on**: phase-1-control-plane-core (`Node`, `Job`, `Resources`, `AllocStatusRow`).

---

## US-02: ProcessDriver — tokio::process + cgroups v2

### Problem

Whitepaper §6 commits to a process driver as a first-class workload
type. Without it, the platform can commit job specs to IntentStore
all day and never run a single workload. Ana — and any future
operator — has zero way to verify the convergence-loop assertion
made in `phase-1-control-plane-core` ("the reconciler primitive is
real"). Worse, when the lifecycle reconciler in Slice 3 emits its
first `Action::StartAllocation`, the action shim has no
production-grade `Driver` to dispatch into — only `SimDriver`, which
runs in the harness, not on the host.

### Who

- Overdrive platform engineer running a single-mode dev cluster on a
  Linux host | motivated by the need to actually run workloads under
  cgroup-isolated supervision before any of the §14 right-sizing,
  §13 policy, or §7 dataplane work can be tested end-to-end.

### Solution

Land `ProcessDriver` in `crates/overdrive-host/src/driver/process.rs`
implementing the `Driver` trait against `tokio::process::Command` and
cgroups v2 (via `cgroups-rs` dep, or direct cgroupfs writes — DESIGN
picks). `Driver::start` spawns the child, creates the workload cgroup
scope at `overdrive.slice/workloads.slice/<alloc_id>.scope`, places
the child PID into `cgroup.procs`, returns an `AllocationHandle`
carrying PID and scope path. `Driver::status` polls process state.
`Driver::stop` sends SIGTERM, waits the configurable grace, escalates
to SIGKILL, then removes the cgroup scope. Default-lane unit tests
use `SimDriver`. Linux-only integration tests under `integration-tests`
feature exercise real process + cgroup placement.

### Domain Examples

#### 1: Happy Path — Spawn `/bin/sleep 60` in a cgroup scope

Driver receives an AllocationSpec with `alloc_id =
AllocationId("a1b2c3...")`, binary `/bin/sleep`, args `["60"]`,
resources `{ cpu_milli: 1000, memory_bytes: 256 MiB }`. `start`
creates `/sys/fs/cgroup/overdrive.slice/workloads.slice/a1b2c3....scope`,
spawns the child via `tokio::process::Command`, writes the child PID
into `cgroup.procs`. Returns `AllocationHandle { pid: 12345, cgroup:
CgroupPath("overdrive.slice/workloads.slice/a1b2c3....scope") }`.
`/proc/12345/cgroup` shows the same path.

#### 2: Edge Case — Stop with grace, process exits cleanly

Driver receives `stop(handle, grace=Duration::from_secs(5))`. Sends
SIGTERM. Process exits within 1 second. Driver waits for child
reap, then removes the cgroup scope. Returns `Ok(())`.

#### 3: Error Boundary — Binary doesn't exist

Driver receives an AllocationSpec with binary path
`/nonexistent/payments`. `start` returns `Err(DriverError::SpawnFailed
{ binary: "/nonexistent/payments", source: <tokio::process::SpawnError> })`
without creating a cgroup scope. The action shim writes
`AllocStatusRow { state: Failed, reason: <message> }`.

### UAT Scenarios (BDD)

#### Scenario: ProcessDriver starts a child process and reports it Running

Given an AllocationSpec for a valid binary `/bin/sleep 60`
When the action shim calls `ProcessDriver::start`
Then the driver returns an `AllocationHandle` with a live PID
And `Driver::status(handle)` returns `AllocationState::Running`
And the cgroup scope `overdrive.slice/workloads.slice/<alloc_id>.scope` exists on the host

#### Scenario: ProcessDriver places the child process in the workload cgroup scope

Given a successful `Driver::start` returning `AllocationHandle { pid, cgroup }`
When the operator reads `/proc/<pid>/cgroup`
Then the path includes `overdrive.slice/workloads.slice/<alloc_id>.scope`
And the cgroup scope's `cgroup.procs` file contains `<pid>`

#### Scenario: ProcessDriver fails cleanly when the binary does not exist

Given an AllocationSpec with binary path `/nonexistent/payments`
When `Driver::start` is called
Then the driver returns `DriverError::SpawnFailed { binary }`
And no cgroup scope is created on the host
And the resulting AllocStatusRow's `state` is `Failed` with an actionable reason

#### Scenario: ProcessDriver::stop sends SIGTERM, waits, removes scope

Given a Running allocation
When the action shim calls `Driver::stop(handle, grace=Duration::from_secs(5))`
And the workload exits within the grace window
Then the cgroup scope is removed from the host
And `Driver::status(handle)` returns `AllocationState::Terminated`

#### Scenario: ProcessDriver::stop escalates to SIGKILL when SIGTERM is ignored

Given a Running allocation whose binary ignores SIGTERM
When the action shim calls `Driver::stop` with a 1-second grace
Then SIGKILL is sent after the grace expires
And the process is reaped
And the cgroup scope is removed
And `Driver::status` returns `AllocationState::Terminated`

#### Scenario: Default-lane unit tests do NOT spawn real processes

Given `cargo nextest run -p overdrive-host` (without `--features integration-tests`)
When the test suite runs
Then no real OS process is spawned
And no cgroup scope is created on the host
And the `Driver` trait surface is exercised via `SimDriver` fixtures

### Acceptance Criteria

- [ ] `ProcessDriver` struct in `crates/overdrive-host/src/driver/process.rs` implements `Driver`
- [ ] `Driver::start` spawns via `tokio::process::Command` and creates `overdrive.slice/workloads.slice/<alloc_id>.scope`
- [ ] `AllocationHandle` carries PID and `CgroupPath` (NEW newtype in `overdrive-host`)
- [ ] `Driver::status` polls process liveness and returns `AllocationState`
- [ ] `Driver::stop` sends SIGTERM, waits grace, escalates SIGKILL if needed; removes cgroup scope
- [ ] Linux-only integration test under `integration-tests` feature actually starts `/bin/sleep`, asserts `/proc/<pid>/cgroup` shows the expected scope, then stops cleanly
- [ ] Default-lane (no `integration-tests`) tests do NOT spawn real processes — `SimDriver` is the fixture
- [ ] `dst-lint` does not flag `overdrive-host` (class `adapter-host`, exempt)
- [ ] cgroups-rs (or chosen cgroup-management dep) declared in workspace `Cargo.toml`; `overdrive-host` adds it as a regular dep with no `tokio` feature creep into `core`-class crates

### Outcome KPIs

- **Who**: Overdrive platform engineer running on a Linux host
- **Does what**: trusts the platform to start a real workload under cgroup-isolated supervision
- **By how much**: 100% of `Driver::start` calls under the integration test produce a process whose `/proc/<pid>/cgroup` matches the AllocationHandle's cgroup path; 0 zombie processes after `Driver::stop`
- **Measured by**: integration test, gated `integration-tests` feature; CI exercises this on the Linux Tier 3 matrix per `.claude/rules/testing.md`
- **Baseline**: greenfield — no production driver exists today (only `SimDriver` in `overdrive-sim`)

### Technical Notes

- `cgroups-rs` is a maintained MIT-or-Apache-2 crate that wraps the cgroup v2 unified-hierarchy API. Direct cgroupfs writes (no dep) are also viable. DESIGN picks; either is acceptable.
- Resource enforcement on the cgroup scope (`cpu.weight`, `memory.max`): DESIGN owns whether this slice writes them or defers to a §14 right-sizing follow-on. Recommendation: write them in this slice from `AllocationSpec::resources`, since the data is already on the spec.
- macOS dev hosts run the default lane with `SimDriver`; integration tests require a Linux VM. This matches the Tier 3 testing pattern.
- Windows is explicitly out of scope for Phase 1.
- **Depends on**: phase-1-foundation `Driver` trait, `AllocationSpec`, `AllocationHandle`, `AllocationState`, `Resources`. Mechanically independent of US-01.

---

## US-03: Job-lifecycle reconciler + action shim (and `job stop`)

### Problem

Whitepaper §18's "Built-in Reconcilers" lists `Job lifecycle (start,
stop, migrate, restart)` as the convergence loop. Without it, every
component shipped in Slices 1-2 sits inert: the scheduler is a pure
function with no caller, ProcessDriver is a trait impl with no
dispatch site. Ana cannot verify the §18 architectural commitment
("reconcilers converge, workflows orchestrate, the reconciler is
pure") because the only shipped reconciler is `noop-heartbeat` which
has nothing to converge. Operators also have no way to reverse a
`job submit` — `overdrive job stop <id>` is the inverse affordance
that closes the allocation lifecycle in the operator-facing
direction.

### Who

- Overdrive platform engineer who will write Phase 2+ reconcilers and
  needs a reference implementation | DST harness (asserts convergence
  + purity invariants) | Ana via `overdrive alloc status`
  (operator-visible proof the loop closed) | operator who wants to
  stop a running job and see it drain cleanly to Terminated.

### Solution

Land `Action::{StartAllocation, StopAllocation, RestartAllocation}`
variants on the existing Action enum. Implement a `JobLifecycle`
reconciler with a real `Self::View = JobLifecycleView` that is
hydrated from a per-reconciler libSQL DB. The reconciler reads
`desired` (job spec from rkyv-hydrated IntentStore) and `actual`
(current AllocStatusRow set), calls Slice 1's `schedule(...)` to
pick a placement, and emits `Action::StartAllocation`. Add the
`AnyReconciler::JobLifecycle(JobLifecycle)` variant + `AnyReconcilerView::JobLifecycle(JobLifecycleView)`
variant + match arms in `name`, `hydrate`, `reconcile`. Land the
**action shim** — a NEW runtime layer in `overdrive-control-plane`
that consumes `Vec<Action>` from the reconciler runtime, dispatches
allocation-management actions to `Arc<dyn Driver>` (production:
ProcessDriver from Slice 2; DST: SimDriver), and writes
`AllocStatusRow` back to `ObservationStore`. Extend `AppState` with
`driver: Arc<dyn Driver>`. Register the lifecycle reconciler in
`run_server_with_obs()`. Add three new DST invariants. **Also land
`overdrive job stop <id>`** end-to-end: CLI subcommand + handler
(`POST /v1/jobs/{id}:stop` per DESIGN's pick) + lifecycle reconciler
reading stopped intent and emitting `Action::StopAllocation` for each
running allocation, which the action shim dispatches to
`Driver::stop` to drive the workload through Running → Draining →
Terminated.

### Domain Examples

#### 1: Happy Path — Submit, schedule, start, observe Running, then stop

Ana submits a 1-replica process job. The broker queues an evaluation
for `(JobLifecycle, "jobs/payments")`. The runtime hydrates State
(via `hydrate`, the only async I/O point — reads IntentStore for
desired, ObservationStore for actual, libSQL for view).
`reconcile(desired, actual, view, tick)` calls `schedule(...)`, gets
`Ok(NodeId("local"))`, emits `Action::StartAllocation { alloc_id:
AllocationId("a1b2c3..."), job_id: JobId("payments"), node_id:
NodeId("local"), spec }`. The action shim consumes it, calls
`ProcessDriver::start(spec)` on `Arc<dyn Driver>`, gets back
`AllocationHandle`, writes `AllocStatusRow { alloc_id, job_id,
node_id, state: Running, updated_at: tick.now }`. `overdrive alloc
status --job payments` shows the row. Ana then runs `overdrive job
stop payments`; lifecycle reconciler reads stopped intent, emits
`Action::StopAllocation`, action shim calls `Driver::stop`, scope
removed, `AllocStatusRow.state = Terminated`.

#### 2: Edge Case — Process crashes; reconciler restarts (with backoff)

Workload process is killed externally. Action shim's next
status-poll detects exit; writes `AllocStatusRow { state:
Terminated }`. On the next reconciler tick, `actual.replicas_running
= 0 < desired = 1`. Reconciler reads `view.restart_counts[old_alloc]
= 0`, emits a fresh `StartAllocation` with a new `alloc_id`. View's
NextView increments `restart_counts[new_alloc] = 0` (per-alloc
counter; reset by alloc_id). Backoff `next_attempt_at` is set from
`tick.now + initial_backoff`.

#### 3: Error Boundary — Binary always fails; backoff exhausts

Reconciler emits StartAllocation. Action shim calls `Driver::start`,
gets `DriverError::SpawnFailed`. Writes `AllocStatusRow { state:
Failed }`. Next tick: `view.restart_counts[alloc] = 1`, emit another
StartAllocation. Repeat until `restart_counts[alloc] == M` (configured
ceiling, e.g. 5). At that point reconciler emits NO action; AllocStatusRow
is updated with `state: Failed (backoff exhausted)`. The reconciler
stops attempting to restart until the operator changes desired state.

### UAT Scenarios (BDD)

#### Scenario: Job-lifecycle reconciler converges to declared replica count

Given a 1-replica job is submitted
And the lifecycle reconciler is registered
When the broker evaluates `(JobLifecycle, "jobs/<id>")`
And the action shim consumes the resulting StartAllocation
Then within N reconciler ticks `overdrive alloc status --job <id>` shows one Running allocation
And the AllocStatusRow's `node_id` matches the scheduler's decision (the local node)

#### Scenario: Job-lifecycle reconciler is pure

Given a fixed `(desired, actual, view, tick)` input
When the reconciler is invoked twice with the same inputs under DST
Then both invocations return equal `(Vec<Action>, NextView)` outputs
And the existing `ReconcilerIsPure` invariant holds

#### Scenario: Job-lifecycle reconciler does not call wall-clock or mint randomness

Given the lifecycle reconciler module
When `cargo xtask dst-lint` scans the source
Then no banned API (`Instant::now`, `SystemTime::now`, `rand::random`, `tokio::time::sleep`) appears in `reconcile`
And `tick.now` is the only wall-clock source consulted

#### Scenario: A killed workload process is restarted

Given a job is Running with one replica
When the workload process is killed externally
Then within N reconciler ticks the prior allocation's state is `Terminated`
And shortly after, a new allocation with a fresh `alloc_id` appears in `Running`
And the new alloc_id is different from the prior one

#### Scenario: Repeatedly-crashing process triggers backoff exhausted

Given a Job whose binary fails to spawn every time
When the lifecycle reconciler attempts to start it M times in succession
Then the reconciler's libSQL `view` records the restart count
And after the configured ceiling, the allocation enters `Failed (backoff exhausted)`
And the reconciler emits no further `StartAllocation` for that alloc_id

#### Scenario: Stopping a Running job drives it to Terminated cleanly

Given a Running allocation
When Ana runs `overdrive job stop <id>`
Then within N reconciler ticks the allocation transitions Running → Draining → Terminated
And the workload's cgroup scope is removed from the host
And `overdrive alloc status --job <id>` shows the terminal state

#### Scenario: Stopping a job that does not exist returns 404

When Ana runs `overdrive job stop unknown`
Then the CLI exits with code 1
And the output names the unknown job ID
And no allocation state changes

### Acceptance Criteria

- [ ] `Action::{StartAllocation, StopAllocation, RestartAllocation}` variants exist on the Action enum (additive)
- [ ] `JobLifecycle` reconciler struct + `JobLifecycleView` (libSQL-hydrated; `restart_counts: BTreeMap<AllocationId, u32>`, `next_attempt_at: BTreeMap<AllocationId, Instant>`, where `Instant` here is whatever the runtime's `TickContext::now` type resolves to)
- [ ] `AnyReconciler::JobLifecycle` + `AnyReconcilerView::JobLifecycle` variants exist with match arms in `name`, `hydrate`, `reconcile`
- [ ] Action shim consumes allocation-management actions from `runtime.drain()` and dispatches to `Arc<dyn Driver>`
- [ ] `AppState::driver: Arc<dyn Driver>` exists
- [ ] Lifecycle reconciler is registered at boot via `runtime.register(job_lifecycle())?`
- [ ] DST invariant `JobScheduledAfterSubmission` exists and passes (eventually invariant: a submitted job becomes Running within N ticks)
- [ ] DST invariant `DesiredReplicaCountConverges` exists and passes (eventually invariant: `count(state == Running) == job.replicas` for each submitted job)
- [ ] DST invariant `NoDoubleScheduling` exists and passes (always invariant: each allocation appears under exactly one node_id; vacuous-pass shape under N=1 nodes; the invariant still has to hold)
- [ ] Existing `ReconcilerIsPure` invariant continues to pass with the new reconciler in the catalogue
- [ ] `dst-lint` does not flag the lifecycle reconciler's `reconcile` body (no `Instant::now`, no `tokio::time::sleep`, no `rand::*`, no `.await`)
- [ ] `overdrive job stop <id>` CLI subcommand and corresponding handler exist (path shape `POST /v1/jobs/{id}:stop` per DESIGN's pick)
- [ ] Lifecycle reconciler emits `Action::StopAllocation` when desired state is stopped; action shim dispatches to `Driver::stop`
- [ ] `overdrive alloc status` Pending-row rendering surfaces `PlacementError::NoCapacity` reason text actionably (the start-side counterpart to the stop-side affordance)

### Outcome KPIs

- **Who**: Overdrive platform engineer + DST harness
- **Does what**: depends on a §18-compliant pure reconciler that converges declared intent and tolerates real-world failure modes; trusts `overdrive job stop` to drain cleanly
- **By how much**: 100% of submitted 1-replica jobs reach Running within N ticks under DST; 100% of `job stop` calls drive the allocation to Terminated within N+M ticks; 0 reconciler purity regressions on every PR; 100% of crash-recovery scenarios converge back to Running within N+M ticks (where M is bounded by the libSQL backoff state)
- **Measured by**: three new DST invariants gated in CI; the existing `ReconcilerIsPure` invariant covering the new reconciler; end-to-end integration test under `integration-tests` feature submitting a real job, asserting Running, killing the process, asserting recovery, then `job stop` asserting Terminated
- **Baseline**: greenfield — no lifecycle reconciler exists; only `noop-heartbeat`. No `job stop` handler exists.

### Technical Notes

- **HARD DEPENDENCY ON DESIGN: `State` shape**. The current `pub struct State;` placeholder cannot be dereferenced by this reconciler. DESIGN MUST decide between (a) a generic/parameterised `State<D, A>` carrying typed projections, (b) a concrete struct with `BTreeMap<AllocationId, AllocStatusRow>` and `Option<Job>`, or (c) per-reconciler typed state matching the `AnyReconciler` enum-dispatch pattern (e.g. `AnyState::JobLifecycle(JobLifecycleState)`). Option (c) is structurally consistent with the existing `AnyReconcilerView` pattern and the codebase research recommends it. **This is the single largest design decision DESIGN must resolve before DELIVER can begin on this story.** DoR flags it.
- The action shim crosses the async boundary that `reconcile` cannot. It lives in the runtime, NOT in the reconciler. DESIGN owns the exact placement (`overdrive-control-plane::reconciler_runtime::action_shim` proposed).
- `JobLifecycleView::next_attempt_at` is read against `tick.now` per `.claude/rules/development.md` `tick.now` rule. Calls to `Instant::now()` inside `reconcile` are blocking violations.
- `Action::MigrateAllocation` is mentioned in whitepaper §18 but explicitly OUT of scope for Phase 1 (Phase 3+ when migration via `overdrive-fs` lands).
- `overdrive job stop` HTTP endpoint shape: `POST /v1/jobs/{id}:stop` is one option; `DELETE /v1/jobs/{id}` is another (idempotent semantics differ). DESIGN owns; either satisfies the AC.
- **Phase 1 is single-node** — the scheduler returns the local node deterministically; the lifecycle reconciler's convergence loop has no multi-node placement choice to defend against.
- **Depends on**: US-01, US-02. **Hard DESIGN dependency on State shape**.

---

## US-04: Control-plane cgroup isolation

### Problem

Whitepaper §4 commits to "control plane processes run in dedicated
cgroups with kernel-enforced resource reservations." Without it, a
misbehaving workload that bursts CPU on the same host can starve the
control plane — the CLI hangs, reconciler ticks back up, observer
nodes see a flapping master. Phase 1 is single-node co-located —
control plane and worker run on one machine — which is exactly the
case §4 calls out. The kernel-level cgroup split is the structural
backstop for control-plane responsiveness under workload pressure;
this story validates the structural backstop against a real Linux
kernel under real CPU pressure.

### Who

- Overdrive platform engineer running on a Linux host | future
  operator deploying via systemd unit | DST harness (the
  proportional cousin: `SimDriver` cannot exhaust real CPU, so this
  story's correctness is asserted via Linux integration test, not
  DST).

### Solution

Land a cgroup v2 delegation pre-flight check at `overdrive serve`
startup. Land a server-bootstrap CgroupManager that creates
`overdrive.slice/control-plane.slice/` (idempotent) and enrols the
running process into it. Add a Linux-only integration test (gated
`integration-tests`) that submits a CPU-burst workload and asserts
`overdrive cluster status` continues to respond within 100 ms during
the burst. Add a smoke test reading `/proc/self/cgroup` at server
boot and asserting the control-plane slice path.

> **Scope note on GH #20**: Issue #20 covers both *control-plane
> cgroup isolation* (this story) and *scheduler taint/toleration
> support* (NOT this story). The taint/toleration half is explicitly
> deferred — Phase 1 is single-node, so taint/toleration delivers
> no value. The user is expected to split GH #20 into two issues
> with the latter scheduled for the multi-node phase.

### Domain Examples

#### 1: Happy Path — Server boots, enrols itself, control plane stays responsive

`overdrive serve` runs the pre-flight check, finds cgroup v2
delegated to the running UID, creates `overdrive.slice/control-plane.slice/`,
writes own PID into `cgroup.procs`. Server is fully operational. Ana
submits a CPU-burst job (e.g. `stress --cpu 4` running indefinitely).
The workload bursts to 100% CPU. Ana runs `time overdrive cluster
status` — returns within 12 ms. The kernel's cgroup CPU bandwidth
controller is enforcing the slice split.

#### 2: Edge Case — Server detects an existing slice and re-uses it

Server starts a second time (e.g. after a process restart);
`overdrive.slice/control-plane.slice/` already exists. Pre-flight
detects this, treats it as success, enrols the new PID. Old PID is
already gone (process exited).

#### 3: Error Boundary — Server refuses to start without cgroup v2 delegation

Operator runs `overdrive serve` as a non-root user without
`Delegate=yes` in their systemd user manager. Pre-flight check
detects the missing delegation: `/sys/fs/cgroup/user.slice/user-1000.slice/cgroup.subtree_control`
shows no `cpu` controller delegated. Server logs:
```
Error: cgroup v2 delegation required.

  Overdrive serve needs cpu and memory controllers delegated to UID 1000.
  This is typically configured via systemd user-unit `Delegate=yes`.

  Try:
    1. Run via the bundled systemd unit:  systemctl --user start overdrive
    2. Grant delegation manually:         sudo systemctl set-property user-1000.slice Delegate=yes
    3. Run as root (dev only):            sudo overdrive serve

  Documentation: https://docs.overdrive.sh/operations/cgroup-delegation
```
Server exits with code 1; no /v1 endpoint is bound.

### UAT Scenarios (BDD)

#### Scenario: Control plane stays responsive while a workload bursts CPU

Given the control plane is enrolled in `overdrive.slice/control-plane.slice/`
And a workload is enrolled in `overdrive.slice/workloads.slice/<alloc_id>.scope`
When the workload bursts to 100% CPU
Then `overdrive cluster status` returns within 100 ms on localhost
And the control plane's responsiveness is not degraded by the workload

#### Scenario: Server-boot smoke test confirms the control-plane slice

Given `overdrive serve` has just started
When the boot path reads `/proc/self/cgroup`
Then the path includes `overdrive.slice/control-plane.slice/`
And the assertion is part of the boot-time integrity test, not a runtime check

#### Scenario: Server refuses to start without cgroup v2 delegation

Given a host where cgroup v2 is not delegated to the running UID
When the operator runs `overdrive serve`
Then the server logs an actionable error explaining cgroup delegation
And exits with a non-zero code
And no /v1 endpoint is bound

#### Scenario: Server refuses to start on cgroup v1 hosts

Given a host with cgroup v1 only
When the operator runs `overdrive serve`
Then the server logs an actionable error explaining cgroup v2 unavailability
And exits with a non-zero code

#### Scenario: Pre-flight detects a delegated-but-stripped controller

Given a host where cgroup v2 is delegated but the `cpu` controller is not in `cgroup.subtree_control`
When the operator runs `overdrive serve`
Then the server logs an actionable error naming the missing controller
And exits with a non-zero code

### Acceptance Criteria

- [ ] cgroup v2 delegation pre-flight check exists in the server boot path; refuses to start if cgroup v2 is unavailable or delegation is missing
- [ ] Server creates `overdrive.slice/control-plane.slice/` (idempotent) at boot
- [ ] Server enrols its own PID into the control-plane slice via `cgroup.procs`
- [ ] Linux-only integration test gated `integration-tests` submits a CPU-burst job and asserts `overdrive cluster status` returns within 100 ms during the burst
- [ ] Server-boot smoke test reads `/proc/self/cgroup` and asserts the control-plane slice path
- [ ] Pre-flight error messages answer "what / why / how to fix" per `nw-ux-tui-patterns`
- [ ] Existing default-lane tests do NOT require cgroup v2 delegation (pre-flight is gated by `integration-tests` feature OR by a runtime config flag DESIGN picks)

### Outcome KPIs

- **Who**: Overdrive platform engineer running on a Linux host
- **Does what**: trusts the kernel cgroup hierarchy to protect control-plane responsiveness under workload pressure
- **By how much**: 100% of integration test runs show `cluster status` returning < 100 ms during a 100% CPU workload burst; 0 control-plane-starvation regressions across PRs touching the slice machinery
- **Measured by**: integration test under `integration-tests` feature on the Linux Tier 3 matrix per `.claude/rules/testing.md`
- **Baseline**: greenfield — no slice creation or enrolment exists

### Technical Notes

- Linux-only. macOS / Windows hosts cannot run this slice's integration test; default-lane tests must not depend on cgroup v2.
- DESIGN may relax the pre-flight to a warning rather than a hard refusal in dev-mode (e.g. `--allow-no-cgroups`). Recommendation: hard refusal by default; the dev escape hatch is an explicit flag named to communicate the trade-off.
- Future Phase 2+ work will extend this with `cpu.weight` / `memory.max` reservations on the control-plane slice; this story creates the slice but does not impose limits — limits are the responsibility of the host's systemd unit (or a future control-plane-managed sub-reconciler).
- **GH #20 split**: The taint/toleration half of GH #20 is explicitly deferred. Phase 1 single-node has no placement choice for a taint to gate; the deferral is recorded in `wave-decisions.md` § "Scope correction (2026-04-27)". The user is expected to track the split in two GitHub issues.
- **Depends on**: US-02 (ProcessDriver wiring; this story extends the cgroup symmetry to the control plane), US-03 (lifecycle reconciler producing real workloads to assert against in the integration test).

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-27 | Initial six user stories for `phase-1-first-workload` DISCUSS wave. |
| 2026-04-27 | Scope correction — Phase 1 is single-node. Removed the prior US-01 (Node registration) and US-05 (Taint/toleration) stories entirely. Re-numbered the remaining four stories: US-01 → scheduler, US-02 → ProcessDriver, US-03 → lifecycle reconciler + action shim + `overdrive job stop` (folded in from the deleted US-05), US-04 → control-plane cgroup isolation. Removed every reference to taints, tolerations, default `control-plane:NoSchedule`, `Node::taints`, `Job::tolerations`. Removed `Taint` and `Toleration` newtype obligations from System Constraints. Added explicit "Phase 1 is single-node" callouts to the System Constraints header and to each story. Noted the GH #20 split (cgroup-isolation in Phase 1; taint/toleration deferred to multi-node phase). |
