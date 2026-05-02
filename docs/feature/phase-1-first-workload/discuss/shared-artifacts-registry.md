# Shared Artifact Registry — phase-1-first-workload

Data that flows across journey steps in this feature, layered on top of
the registry from `phase-1-control-plane-core` (still in force —
`job_spec_bytes`, `rest_endpoint`, `intent_key`, `reconciler_registry`,
`evaluation_broker_state`, `alloc_row`, `spec_digest`, `openapi_schema`
all carry forward unchanged). This file documents only the **NEW**
shared artifacts introduced by `phase-1-first-workload`.

> **Phase 1 is single-node.** Control plane and worker run co-located on
> one machine. There is exactly one `node_health` row at runtime,
> written by the server at startup as an implementation detail. No
> operator-facing node-registration verb exists. Taints, tolerations,
> and multi-node placement choice are explicit Phase 1 non-goals (see
> `wave-decisions.md` § "Scope correction (2026-04-27)"). The
> `node_id` artifact below documents how the single-node precondition
> threads through the type system; it is NOT a story or a CLI flow.

## New artifacts in this feature

```yaml
shared_artifacts:

  node_id:
    source_of_truth: >
      `overdrive-core::aggregate::Node::id`, which is a `NodeId`
      newtype constructed via `NodeId::from_str(...)` with validation
      and normalization rules from phase-1-foundation. **Phase 1
      single-node precondition**: exactly one Node aggregate exists at
      runtime (the local host); the server writes its `NodeHealthRow`
      at startup. The `NodeId` newtype obligation is STRICT (FromStr,
      Display, serde, rkyv, full proptest round-trip) — see System
      Constraints in `user-stories.md`.
    consumers:
      - "IntentStore key (nodes/<NodeId>) via IntentKey::for_node(&NodeId)"
      - "ObservationStore::node_health_rows().node_id (the single row)"
      - "ObservationStore::alloc_status_rows().node_id (placement output)"
      - "Scheduler module: input enumeration of placement candidates (BTreeMap with one entry)"
      - "Lifecycle reconciler: actual.placements_by_node_id"
      - "ProcessDriver: cgroup scope path component IF DESIGN picks node-scoped scopes (otherwise allocation-scoped only)"
    owner: "overdrive-core (NodeId newtype) + this feature (the typed flow through scheduler / reconciler / driver)"
    integration_risk: >
      LOW — Phase 1 single-node makes node_id a precondition rather
      than a multi-step join key for placement choice. The risk is
      confined to the type discipline: every consumer must reference
      the same `NodeId::Display` form and the same
      `IntentKey::for_node(&NodeId)` function. Phase 2+ (multi-node)
      raises the risk to HIGH; Phase 1 keeps it LOW because there's
      no operator-facing variability.
    validation: >
      Proptest: for any valid NodeId, `IntentKey::for_node(id)`
      returns `nodes/<NodeId::Display>` byte-for-byte. Acceptance
      test: submit a job, observe its allocation lands on the local
      `node_id` and the same `node_id` appears in `alloc status`
      output under the Running allocation's `node_id` column.

  node_capacity:
    source_of_truth: >
      `overdrive-core::aggregate::Node::capacity`, a `Resources`
      struct (the same Resources from `traits/driver.rs` — single
      source per US-01 of phase-1-control-plane-core). The local
      node's capacity is configured at server startup (DESIGN owns
      whether this comes from a config file, env var, or autodetection
      — Phase 1 may simply require operators to set it explicitly via
      `overdrive serve --cpu N --memory M` or analogous).
    consumers:
      - "Scheduler first-fit capacity check"
      - "Allocation accounting: `node.capacity - sum(running_allocs.resources)`"
      - "CLI `cluster status` output (for operator situational awareness)"
    owner: "overdrive-core (Resources struct) + this feature (single-node startup wiring)"
    integration_risk: >
      MEDIUM — the capacity figure drives every Pending-vs-Running
      decision in Phase 1. If the configured capacity does not match
      what the kernel can actually deliver, the Pending error message
      is misleading and operators waste time. Phase 1 mitigation:
      Resources is already a single-source struct; the configured
      figure is the only number consulted; Phase 2+ may auto-detect
      from the host kernel.
    validation: >
      Acceptance test: configure capacity, submit a job within
      capacity (observe Running) and a job exceeding capacity (observe
      Pending with reason naming the requested-vs-free numbers).

  placement_decision:
    source_of_truth: >
      The scheduler module's pure function output: given
      `(BTreeMap<NodeId, NodeView>, JobView, Vec<AllocStatusRow>)`,
      it returns `Result<NodeId, PlacementError>` where
      `PlacementError ∈ {NoCapacity, NoHealthyNode}`. The lifecycle
      reconciler calls this function and wraps the result in either
      `Action::StartAllocation { node_id, ... }` (on Ok) or a `Pending`
      allocation status with reason text (on Err). Phase 1 single-node:
      the BTreeMap has exactly one entry; the function still returns
      via the same predicate path (capacity check), and the
      determinism property still has to hold.
    consumers:
      - "Lifecycle reconciler reconcile() body"
      - "Allocation Pending reason rendering"
      - "DST invariant: `SchedulerRespectsNodeCapacity`"
    owner: "this feature (scheduler module — DESIGN picks crate boundary, proposed `overdrive-control-plane::scheduler` or split into `crates/overdrive-scheduler`)"
    integration_risk: >
      HIGH — the placement function is the convergence loop's brain.
      Determinism is load-bearing: same inputs MUST produce same
      output, otherwise the same DST seed produces different
      trajectories. Phase 1 is first-fit on a one-element map; even
      that needs the BTreeMap iteration discipline so Phase 2+ is a
      content change not a structural one. Mitigation: all internal
      collections that drive iteration are BTreeMap per
      `.claude/rules/development.md` ordered-collection rules; the
      function is a pure `fn` (sync, no .await, no random); a proptest
      covers same-inputs-same-output across reorderings of the input
      vec.
    validation: >
      Proptest: for any `(nodes, job, allocs)`, calling the scheduler
      twice produces equal output. Twin-invocation property test under
      DST. Unit test: a job submitted against a local node with
      sufficient capacity returns `Ok(<the_local_node_id>)`; a job
      exceeding capacity returns `Err(NoCapacity { needed, max_free })`.

  alloc_id:
    source_of_truth: >
      `AllocationId` newtype (already in phase-1-foundation) emitted
      by the lifecycle reconciler via the `Entropy` port — concretely,
      the reconciler reads the ID from the runtime's hydrated view
      (which used the runtime's seeded RNG to mint it). The ID flows
      into `Action::StartAllocation { alloc_id, ... }`, into the
      action shim's call to `Driver::start(spec_with_alloc_id)`, into
      `ProcessDriver`'s cgroup scope path
      (`overdrive.slice/workloads.slice/<alloc_id>.scope`), and into
      the `AllocStatusRow` written by the action shim. STRICT-newtype
      obligation applies — already established by phase-1-foundation.
    consumers:
      - "Reconciler emits in Action::StartAllocation"
      - "Action shim dispatches to Driver::start"
      - "ProcessDriver creates cgroup scope at workloads.slice/<alloc_id>.scope"
      - "ObservationStore alloc_status_rows.alloc_id"
      - "CLI alloc status output"
    owner: "phase-1-foundation (AllocationId newtype) + this feature (the multi-hop flow)"
    integration_risk: >
      HIGH — alloc_id is the multi-hop join key. A drift between
      what the reconciler emits, what the cgroup scope is named, and
      what the CLI displays would mean the operator sees an alloc_id
      that doesn't correspond to any real cgroup or process. Operators
      grep `systemctl status overdrive.slice/workloads.slice/<alloc_id>.scope`
      to debug. Mitigation: one mint site (the reconciler reading via
      Entropy port); the value is passed by reference everywhere
      after.
    validation: >
      DST invariant: `NoDoubleScheduling` — for any allocation, exactly
      one node carries it in alloc_status_rows (vacuous-pass shape
      under N=1 nodes; the invariant still has to hold). Acceptance
      test (Linux-only, integration-tests gated): submit a job,
      capture the alloc_id from `alloc status` output, assert
      `/sys/fs/cgroup/overdrive.slice/workloads.slice/<alloc_id>.scope`
      exists on disk and contains exactly one PID.

  alloc_state:
    source_of_truth: >
      `overdrive-core::traits::driver::AllocationState` enum, already
      shipped in phase-1-foundation (`Pending` / `Running` / `Draining`
      / `Terminated` / `Failed`). The state transitions are owned by
      the action shim — it calls `Driver::status(handle)` and writes
      the resulting state into `AllocStatusRow.state` via
      `ObservationStore::write`.
    consumers:
      - "Driver::status return value"
      - "Action shim writes into AllocStatusRow"
      - "Lifecycle reconciler reads from State.actual"
      - "CLI alloc status output (rendered with color + label)"
    owner: "overdrive-core (enum) + this feature (action shim transitions)"
    integration_risk: >
      HIGH — the state machine is the lifecycle reconciler's input
      signal. A wrongly-reported Terminated state would cause the
      reconciler to emit an unnecessary StartAllocation; a wrongly-
      reported Running state would prevent recovery. Mitigation:
      `Driver::status` is the single authority; the action shim does
      not invent states.
    validation: >
      DST invariants: `JobScheduledAfterSubmission` (eventually
      Running after submit) and `DesiredReplicaCountConverges`
      (eventually `count(state=Running) == replicas`). Acceptance
      test: submit a job, poll `alloc status` until state is Running
      within N seconds.

  cgroup_path:
    source_of_truth: >
      Two distinct paths, both derived deterministically:
      - **Workload scope**: `overdrive.slice/workloads.slice/<alloc_id>.scope`
        — derived by `ProcessDriver::start` from the AllocationSpec's
        alloc_id field. Created by ProcessDriver before the child
        process is exec'd.
      - **Control-plane slice**: `overdrive.slice/control-plane.slice`
        — created by the server bootstrap once at `overdrive serve`
        startup; the running control-plane process enrols itself.
      `CgroupPath` newtype wraps the path with full STRICT-newtype
      obligations (FromStr, Display, validation; lives in
      `overdrive-host`).
    consumers:
      - "ProcessDriver::start (creates workload scope, writes process PID into cgroup.procs)"
      - "ProcessDriver::stop (removes workload scope after process exits)"
      - "Server boot (creates and enrols into control-plane slice)"
      - "Operator host-side debug via `systemd-cgls` / cat /sys/fs/cgroup/..."
    owner: "this feature (ProcessDriver in `overdrive-host` + server bootstrap CgroupManager)"
    integration_risk: >
      MEDIUM — cgroup path is host-side, not part of the wire
      contract; risk is confined to the action shim getting the path
      wrong on cleanup (orphaned scopes), or the server failing to
      enrol itself (control-plane runs in the root slice, no
      isolation). Mitigation: one derivation function, called from
      both the start and stop paths; a server-boot smoke test asserts
      `/proc/self/cgroup` reports the control-plane slice; a
      teardown test asserts the workload scope is removed after a
      successful stop.
    validation: >
      Integration test (Linux + integration-tests feature, NOT in the
      default lane): start a workload, read /proc/<pid>/cgroup,
      assert it shows the expected workload scope path; stop the
      workload, assert the scope no longer exists. Server-side smoke
      test: at boot, read /proc/self/cgroup, assert it shows the
      control-plane slice.

  restart_count:
    source_of_truth: >
      `JobLifecycleView` (a NEW struct that this feature lands as the
      lifecycle reconciler's `Self::View` type) carries
      `restart_counts: BTreeMap<AllocationId, u32>` and
      `next_attempt_at: BTreeMap<AllocationId, Instant>` (the latter
      is read from `tick.now`, NOT `Instant::now()`, per
      `.claude/rules/development.md` `tick.now` rule). The view is
      hydrated from the lifecycle reconciler's private libSQL DB at
      `<data_dir>/reconcilers/job-lifecycle/memory.db`.
    consumers:
      - "Lifecycle reconciler's reconcile() body — backoff decision"
      - "(Future) telemetry export"
    owner: "this feature (JobLifecycleView + libSQL schema in lifecycle reconciler's hydrate())"
    integration_risk: >
      LOW — internal to the lifecycle reconciler. The risk is that a
      bug in the backoff math turns a transient failure into an
      infinite restart loop. Mitigation: capped in the view's logic;
      tested via a DST scenario where the SimDriver always fails to
      start.
    validation: >
      DST scenario: configure SimDriver to fail-on-start; assert that
      after N reconciler ticks the alloc transitions to
      `Failed (backoff exhausted)` and the reconciler stops emitting
      StartAllocation actions for that alloc_id.

  driver_handle:
    source_of_truth: >
      `Arc<dyn Driver>` held by `AppState` — the action shim's
      reference to the production driver. In Phase 1 single-mode, this
      is a `Arc<ProcessDriver>` instantiated once at server boot.
      Future Phase 2+ wiring may select per-DriverType (Process /
      MicroVm / Wasm) via a DriverRegistry; Phase 1 ships the simplest
      possible: one driver, statically typed.
    consumers:
      - "Action shim (NEW): reads from AppState; calls start/stop/status"
    owner: "this feature (AppState extension + ProcessDriver instantiation)"
    integration_risk: >
      MEDIUM — AppState already exists and threads through every
      handler; adding `driver: Arc<dyn Driver>` is additive but
      changes constructor signatures. A test fixture that uses
      `SimDriver` instead of `ProcessDriver` exercises the same
      action shim without spawning real processes.
    validation: >
      Compile-time: AppState construction in `run_server_with_obs`
      passes a `Box<dyn Driver>` (or `Arc<dyn Driver>`); test
      fixtures pass `Arc<SimDriver>`. DST harness uses `SimDriver`
      end-to-end.
```

## Inherited from `phase-1-control-plane-core` (still in force)

- `job_spec_bytes` — JSON at the wire, rkyv at the store. Property
  still holds; **no new field is added to `Job` in Phase 1** (the
  `tolerations` extension proposed in the prior version of this wave
  was pulled per the 2026-04-27 scope correction).
- `rest_endpoint` — default `https://127.0.0.1:7001`. Unchanged.
- `intent_key` — `IntentKey::for_job(&JobId)` etc. Unchanged. (No
  new node-registration intent key is added; the local node row
  is server-bootstrap, not operator-driven.)
- `reconciler_registry` — extended (job-lifecycle alongside
  noop-heartbeat); same source, same display path.
- `evaluation_broker_state` — unchanged. Counters now move because
  the lifecycle reconciler is doing real work.
- `alloc_row` — table shape locked from phase-1-foundation. This
  feature is the first writer.
- `spec_digest` — same `ContentHash::of(rkyv_archive(Job))`. The
  `Job` aggregate shape is unchanged in Phase 1, so the digest
  property is also unchanged.
- `openapi_schema` — unchanged shape; this feature adds endpoints
  (`POST /v1/jobs/{id}:stop` per DESIGN's pick) but does NOT add
  a node-registration endpoint or new fields on Node / Job
  responses.

## Quality gates

- [x] **Single source of truth for every NEW artifact**: yes; each
  artifact above lists exactly one source.
- [x] **Multi-step join keys explicitly identified**: `alloc_id`,
  `alloc_state` are flagged HIGH and have must_match_across
  validation in `journey-submit-a-job-extended.yaml`. `node_id` is
  LOW under the single-node precondition.
- [x] **Determinism guardrails**: `placement_decision` calls out
  BTreeMap-only iteration per `development.md`. `restart_count`
  calls out `tick.now` not `Instant::now()`.
- [x] **No artifact lacks consumers**: every entry has at least 2
  consumers.
- [x] **Cross-feature register coherence**: nothing from the prior
  feature's registry is contradicted; all extensions are additive.

## Changelog

| Date | Change |
|---|---|
| 2026-04-27 | Initial registry for `phase-1-first-workload`. |
| 2026-04-27 | Scope correction — Phase 1 is single-node. Removed `node_taints` and `tolerations` artifacts. Reframed `node_id` as a single-node precondition (LOW risk in Phase 1; HIGH in Phase 2+ when multi-node placement choice lands). Removed every reference to `Node::taints` / `Job::tolerations` aggregate field plumbing. |
