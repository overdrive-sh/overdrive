# DESIGN wave decisions — `backend-discovery-bridge-service-reachability`

**Wave**: DESIGN | **Scope**: APPLICATION (component-level) | **Mode**: PROPOSE → REVISE
| **Architect**: Morgan | **Date**: 2026-05-13 (revised 2026-05-20 for ADR-0049 / 0050 / 0051 landing)

**Inherits from**:

- ADR-0035 (reconciler trait collapsed to one sync method; runtime owns View persistence)
- ADR-0036 (runtime owns all hydration; reconciler is pure over pre-computed inputs)
- ADR-0040 (SERVICE_MAP / BACKEND_MAP / MAGLEV_MAP three-map split + HASH_OF_MAPS atomic-swap)
- ADR-0042 (`ServiceMapHydrator` reconciler + `Action::DataplaneUpdateService` + `service_hydration_results` table)
- ADR-0045 (`bpf_redirect_neigh` datapath; two XDP programs, no TC)
- ADR-0046 (collision-free `BackendId` allocator)
- ADR-0047 (`WorkloadKind` discriminator; per-kind workload spec)
- ADR-0048 (rkyv versioned envelope; `ServiceBackendRow = ServiceBackendRowV1`)
- **ADR-0049 (platform-issued `ServiceVipAllocator`)** — VIPs are
  allocated at admission keyed by `spec_digest`; persisted in the
  allocator's own `IntentStore` table; consumed by downstream
  hydrators via `ServiceVipAllocator::get(&spec_digest)`. `Listener`
  no longer carries a `vip` field (parser-level removal, 2026-05-14
  amendment).
- **ADR-0050 (intent-side `WorkloadIntent` aggregate)** — persisted
  intent decodes as `WorkloadIntent::{Job(JobV1) | Service(ServiceV1) |
  Schedule(ScheduleV1)}` via `WorkloadIntent::from_store_bytes`. The
  bridge's intent read matches on the `Service(ServiceV1)` variant.
- **ADR-0051 (wire-side `SubmitSpecInput`)** — separate from the
  intent shape; the bridge never sees it directly (admission projects
  onto `WorkloadIntent` before the bridge runs).
- brief.md §§ 44–53 (Phase 2.2 dataplane extension) + §§ 54–62 (workload-kind-discriminator extension)

**Tracks**: GH #174 (backend discovery bridge) + GH #175 (wire `EbpfDataplane` into production single-mode boot)

**Companion ADR**: ADR-0052 (originally numbered ADR-0049 in this
DESIGN; renumbered when ADR-0049 was reassigned to the VIP allocator).

---

## 1. Scope

This DESIGN covers **both** GH #174 and GH #175 as one nWave feature
with **one walking-skeleton e2e gate**. The joint scope is deliberate:
#175's value is unobservable without #174 — every `update_service`
call would receive `backends: []` because no production code path
writes `ServiceBackendRow` today. Splitting the features would leave
half a system landing first that can't demonstrate its own purpose
through the walking-skeleton acceptance shape demanded by ASR-2.2-04
(closure of J-PLAT-004).

The system-level architecture (XDP forward + reverse-NAT, HoM
atomic swap, Maglev permutation, action-shim dispatch, hydrator
reconciler, ObservationStore row shapes) is settled in ADRs 40–48.
This DESIGN is component-level: where the new producer of
`ServiceBackendRow` lives in `overdrive-control-plane`, what its
typed `View` carries, how it triggers, how `EbpfDataplane` boots into
production single-mode, and what error variant the boot path
surfaces on failure.

Out of scope (explicitly):

- VIP allocation — closed by ADR-0049 (delivered 2026-05-19). The
  bridge consumes the allocator-issued VIP via
  `ServiceVipAllocator::get(&spec_digest)`; it does not allocate.
- Health-check probing (GH #170) — a Running alloc is a healthy backend for Phase 2.2; refinement is future work.
- Multi-node owner-writer logic — Phase 1 / 2.2 is single-node per `feedback_phase1_single_node_scope.md`.
- Schedule workload kind — only Service-kind workloads produce backends.
- Job workload kind — `JobV1` carries no listeners per ADR-0050 § 2.

---

## 2. Reuse Analysis (HARD GATE)

Before proposing a new component, every existing component whose
responsibility overlaps the feature scope was inspected. The table
below records every candidate and the verdict.

| Existing component | What it does today | Overlap with feature scope | Decision |
|---|---|---|---|
| `WorkloadLifecycle` (`crates/overdrive-core/src/reconciler.rs`) | Watches `WorkloadIntent` + `AllocStatusRow`; emits `StartAllocation` / `RestartAllocation` / `StopAllocation` / `FinalizeFailed`; recently extended (2026-05-13..-19) with Service-variant handling: `service_spec_digest` populated in `hydrate_desired`/`hydrate_actual` for Service kinds; emits `Action::ReleaseServiceVip` on observed terminal state per ADR-0049 § 6; tracks emission via `view.released_for_terminal: BTreeSet<ContentHash>`. | Reads the same intent shape (`WorkloadIntent::Service`) the bridge needs, plus Running alloc set per workload. Now also owns VIP reclamation. | **Extend? Considered; rejected — see Q174.1 below.** Separation of concerns stronger, not weaker, after the lifecycle gained Service-VIP reclamation. Separate reconciler. |
| `ServiceMapHydrator` (`crates/overdrive-core/src/reconciler.rs:1778`) | Watches `service_backends` + `service_hydration_results`; emits `DataplaneUpdateService`. | **Reads `ServiceBackendRow`**, does NOT write it. The bridge sits *upstream* of this reconciler in the data flow. | **Cannot reuse — opposite I/O direction.** The hydrator is the consumer the bridge writes for. |
| `exit_observer` (`crates/overdrive-control-plane/src/worker/exit_observer.rs`) | Drains `ExitEvent`s from `Driver`; writes `AllocStatusRow` transitions to obs; broadcasts `LifecycleEvent`. | Writes `AllocStatusRow` — same row shape the bridge reads. | **Read-shape collision only.** The bridge consumes obs rows; the observer produces them. Co-located in the worker subsystem, distinct concerns. |
| Action shim `dataplane_update_service` (`crates/overdrive-control-plane/src/action_shim/dataplane_update_service.rs`) | Dispatches `Action::DataplaneUpdateService` → `Dataplane::update_service` → writes `service_hydration_results`. | Calls `Dataplane::update_service` — the trait #175 binds to `EbpfDataplane`. | **Unchanged by this feature.** Its dispatch path is the consumer of the hydrator's output; both #174 and #175 sit upstream of it. |
| `LocalObservationStore` (`crates/overdrive-store-local/`) | Persists `ObservationRow` variants to redb; serves typed-row helpers (`service_backends_rows`, `alloc_status_rows`, etc.). | Already has `ObservationRow::ServiceBackend` variant — write surface exists. | **Reuse as-is.** No store-trait changes; the bridge is a new writer through the existing trait surface. |
| `EvaluationBroker` (`overdrive-control-plane::eval_broker`) | Storm-proof ingress for reconciler evaluations. Keyed `(ReconcilerName, TargetResource)`. | Will be enqueued when alloc-state changes for Service-kind workloads. | **Reuse as-is.** New reconciler kind registers normally. |
| `ReconcilerRuntime` (`overdrive-control-plane::reconciler_runtime`) | Owns `bulk_load` + `write_through` against `ViewStore`; routes tick → `reconcile`. | New reconciler kind needs a new `AnyReconciler` enum arm + `hydrate_desired` / `hydrate_actual` match arms. | **Reuse pattern; extend at three lines** (one `AnyReconciler` variant + two match arms). |
| `NoopDataplane` (`overdrive-host::dataplane`) | Production single-mode placeholder; `update_service` returns `Ok(())`. | Production today wires this. | **Delete from production boot path** (single-cut migration per `feedback_single_cut_greenfield_migrations.md`). Retained as a test fixture under `overdrive-host` only if grep finds active test users; deleted otherwise. See § 4 below. |
| `EbpfDataplane::new(client_iface, backend_iface)` (`crates/overdrive-dataplane/src/lib.rs:508`) | Production-side `Dataplane` impl. Loads BPF ELF, attaches XDP, owns pin path. | This is exactly what #175 needs to instantiate. | **Reuse as-is.** Already takes two ifaces, already returns typed `DataplaneError`, already implements the trait. The boot path just needs to construct it correctly. |

**Verdict on the gate**: the bridge IS a new component. Every existing
candidate was inspected; none owns the responsibility "watch `Running
allocations of Service-kind workloads` AND write `ServiceBackendRow`".
`WorkloadLifecycle` could be extended, but the costs (Q174.1 analysis
below) outweigh the benefit. `ServiceMapHydrator` is the downstream
consumer, not a candidate for write-side reuse.

---

## 3. Open questions, options, and recommendations

### Q174.1 — Component shape: new reconciler vs extend `WorkloadLifecycle`

**Why this is open**: the issue body explicitly names both shapes
("A new reconciler (or extension of the existing `WorkloadLifecycle`)").
`WorkloadLifecycle` already reads exactly the inputs (`Job` + alloc
status) and runs on the same evaluation broker.

#### Options

| Option | Shape | Trade-offs |
|---|---|---|
| **A** | **New reconciler `backend-discovery-bridge`** (kebab-case) in `crates/overdrive-control-plane/src/reconcilers/backend_discovery_bridge/` with its own typed `State` + `View`. Per-target keying = `JobId` (one workload at a time). | + Separation of concerns — `WorkloadLifecycle` is already complex (kind branching, restart budget, terminal claim); adding a second emit channel doubles its surface. + Bridge owns its own `View` (`last_written_fingerprint` per service for dedup), kept narrow. + Failure isolation — if the bridge's `reconcile` panics or returns wrong actions, `WorkloadLifecycle`'s alloc-state machine is untouched. + Different evaluation triggers: alloc-state changes AND `service_backends` row read-back, distinct from `WorkloadLifecycle`'s `(Job, AllocStatusRow)` trigger pair. + Mirrors the upstream `ServiceMapHydrator` shape — one reconciler per producer, one per consumer. − Adds a new `AnyReconciler` variant + register call + two `hydrate_*` arms. ~10 LoC added in the runtime. |
| **B** | **Extend `WorkloadLifecycle::reconcile`** to also emit a "write `ServiceBackendRow`" action when `actual.allocations` projected against `desired.workload_kind == Service` produces a different backend set than last tick. | + Single reconciler reads `WorkloadIntent` once. + No new `AnyReconciler` variant. − **Violates single-responsibility for a reconciler that's already complex AND was just extended again.** The 2026-05-13..-19 amendments added Service-variant `service_spec_digest` population in hydrate, terminal-state observation gating, and `Action::ReleaseServiceVip` emission gated by `view.released_for_terminal`. Layering "obs-row writes vs alloc-state actions vs VIP reclamation" on top of "kind branching + restart budget + terminal claim" concentrates four orthogonal concerns in one reconciler. − Forces a new `Action` variant `Action::WriteServiceBackendRow { row }` because the reconciler still cannot perform I/O — and that action's shim is functionally identical to a direct ObservationStore write, but indirected through a one-purpose action. − Inflates `WorkloadLifecycleView` to also carry `last_written_fingerprint` per service. The View already grew with `released_for_terminal` for ADR-0049 § 6; piling on a third concern compounds. − No isolation benefit: a regression in the bridge logic risks the lifecycle path — and the lifecycle is now the only path that drives VIP reclamation. |
| **C** | **Action-shim-side**: extend the existing action shim or `exit_observer` to write `ServiceBackendRow` as a side-effect of alloc-state transitions. | + No new reconciler. − **Violates the §18 / ADR-0042 boundary.** The action shim is "execute the action, write the observation row that confirms it." Writing intent-side projections (the backend set is derived from intent + observation, not from an action) doesn't fit. − Couples the observer to intent-side knowledge (listener decls) it has no business reading today. − Hides the convergence loop. The bridge's job is exactly the §18 reconciler shape: "keep the cluster's observable state (`service_backends`) reflecting the projection of intent + actual that the bridge owns." Reconciler is the correct primitive. |

#### Recommendation: **Option A — new reconciler `backend-discovery-bridge`**

The bridge is a peer to `ServiceMapHydrator` in shape and function:
both reconcile observation rows, both have a typed retry/dedup View,
both are Service-only. Naming and module layout mirror the hydrator
1:1 — this is the pattern-establishment moment for "any future
Service-kind observation row writer follows the same shape."

The marginal complexity cost (one `AnyReconciler` variant + two
`hydrate_*` arms, ~30 LoC) is bounded; the benefit (testability,
isolation, mirror with the downstream hydrator) is structural.

### Q174.2 — Trigger shape

**Why this is open**: ADR-0035 / ADR-0036 specify `reconcile` is pure
over `(desired, actual, view, tick)`; the runtime pre-computes
`desired` and `actual` and ticks the broker. The question is what
populates the bridge's `desired` (intent side) and `actual`
(observation side) and what enqueues evaluations.

#### Options

| Option | Trigger / hydration shape | Trade-offs |
|---|---|---|
| **A** | **Broker-pending enqueue on `AllocStatusRow` change** (mirrors the convergence-loop §18 wiring). `desired` projection sources `WorkloadIntent::Service(ServiceV1).listeners` (intent — read via `IntentKey::for_workload(&workload_id)` + `WorkloadIntent::from_store_bytes` per ADR-0050) AND the allocator-issued `assigned_vip = ServiceVipAllocator::get(&intent.spec_digest()?)` per ADR-0049 § 5a. `actual` projection = `BTreeMap<AllocationId, AllocStatusRow>` filtered to Running and to the target `WorkloadId`. View = `last_written_fingerprint: BTreeMap<ServiceId, BackendSetFingerprint>` (dedup). | + Reuses the broker-pending pattern already in place. + The runtime's existing `hydrate_desired` already reads `WorkloadIntent` via the `read_job` helper (which now returns `Some(JobV1)` for Job kinds, `None` + `Some(spec_digest)` for Service kinds — `WorkloadLifecycle` already consumes this). The bridge's arm reads the same key but matches on `Service(ServiceV1)` to project listeners. `hydrate_actual` reads `ObservationStore::alloc_status_rows_for_workload` — already wired. + Evaluations are enqueued naturally on the same broker that drives `WorkloadLifecycle`. + Pure-sync `reconcile` per ADR-0035 — no `.await`, no I/O. + The allocator lookup is synchronous in-memory (`PersistentServiceVipAllocator::get` is sync; see `crates/overdrive-dataplane/src/allocators/persistent_service_vip.rs:251`); cleanly composes inside `hydrate_desired`. |
| **B** | **Subscription-based**: the bridge subscribes to `ObservationStore::subscribe()` (the `ObservationSubscription` stream) and the runtime treats incoming events as evaluation triggers. | − Adds a second evaluation source to the runtime; new failure modes (subscription drop, backpressure). − ADR-0035's broker is the single dispatcher; bypassing it would erode that property. − Heavier wire — the broker collapses N transitions on M backends to one evaluation, the subscription delivers N. |
| **C** | **Per-tick poll** without broker hint — the bridge evaluates every 100ms regardless of trigger. | − Wasted ticks when nothing has changed. − Still requires the runtime to provide hydrated `desired`/`actual`, so no implementation simplification. |

#### Recommendation: **Option A — broker-pending on alloc-state change**

The bridge is a §18 reconciler; the runtime's existing convergence
loop IS the trigger mechanism. The bridge's View dedups via
`last_written_fingerprint` so the steady-state cost of "evaluation
enqueued but inputs unchanged" is one `BTreeMap::get` + one
`fingerprint(...)` call + no action emitted + no obs write.

Concretely: when `exit_observer.handle_exit_event` writes a Failed /
Terminated `AllocStatusRow`, OR `WorkloadLifecycle.reconcile` emits
`StartAllocation` / `RestartAllocation`, the broker is enqueued with
`(WorkloadLifecycleName, target=WorkloadId)`. We extend the runtime's
convergence loop to ALSO enqueue
`(BackendDiscoveryBridgeName, target=WorkloadId)` on the same alloc-state
changes — single line in the convergence-loop spawn site.

**Concrete `hydrate_desired` arm shape** (the runtime's match):

```rust
AnyReconciler::BackendDiscoveryBridge(_) => {
    let workload_id = workload_id_from_target(target)?;
    // Read the workload intent via the same path WorkloadLifecycle uses.
    let key = IntentKey::for_workload(&workload_id);
    let Some(bytes) = state.store.get(key.as_bytes()).await
        .map_err(|e| ConvergenceError::IntentRead(e.to_string()))? else {
        return Ok(AnyState::BackendDiscoveryBridge(/* empty */));
    };
    let intent = WorkloadIntent::from_store_bytes(
        bytes.as_ref(), &state.intent_redb_path, Some(key.as_str()))
        .map_err(|e| ConvergenceError::IntentRead(e.to_string()))?;
    let WorkloadIntent::Service(service_v1) = &intent else {
        // Job and Schedule kinds produce no backend rows.
        return Ok(AnyState::BackendDiscoveryBridge(/* empty */));
    };
    let spec_digest = intent.spec_digest()
        .map_err(|e| ConvergenceError::IntentRead(e.to_string()))?;
    // Consult the allocator memo (ADR-0049 § 5a). In Phase 1 the VIP
    // is allocated synchronously at admission (ADR-0049 § 4) so by
    // the time we hydrate it is present; missing-here is logged at
    // debug and the bridge skips the tick.
    let Some(assigned_vip) = state.allocator.lock().await.get(&spec_digest) else {
        return Ok(AnyState::BackendDiscoveryBridge(/* empty — log debug */));
    };
    // Project listeners → (ServiceId, ProjectedListener) using the
    // allocator-issued VIP for ServiceId derivation.
    let listeners = service_v1.listeners.iter().map(|l| {
        let service_id = ServiceId::derive(&assigned_vip, l.port, "service-map");
        (service_id, ProjectedListener {
            vip: assigned_vip.clone(),
            port: l.port, protocol: l.protocol,
        })
    }).collect::<BTreeMap<_,_>>();
    Ok(AnyState::BackendDiscoveryBridge(/* desired filled */))
}
```

The shape pattern-matches the existing `WorkloadLifecycle` hydrate
arm (which `read_job` already special-cases for Service) and the
existing allocator surface (`PersistentServiceVipAllocator::get` is
sync; the `tokio::sync::Mutex::lock` is the only `.await` and is part
of the hydrate path, not `reconcile`).

### Q174.3 — Owner-writer / LWW Lamport counter source

**Why this is open**: ADR-0042 § 4 documents LWW semantics on
`service_backends` (PK = `service_id`, ordering = `lamport_counter`,
tie-break = `writer_node_id`). Phase 1 is single-node, so there is
trivially one writer. The question is **whether the counter source
preserves a future multi-node owner-writer arrangement**.

#### Options

| Option | Counter source | Trade-offs |
|---|---|---|
| **A** | **`tick.tick + 1` per write** (mirrors `action_shim::dataplane_update_service.rs:121-122` which uses `tick.tick.saturating_add(1)`). | + Identical precedent — the action shim already constructs `LogicalTimestamp { counter: tick.tick.saturating_add(1), writer: writer.clone() }`. + Monotonic within a single tick lifetime. + Forward-compat with Corrosion: when a second node starts writing, the writer-node-id breaks ties; Lamport counters across nodes don't need to be globally monotonic for LWW correctness — the per-node monotonicity + writer tiebreak IS the CR-SQLite semantics. |
| **B** | **Runtime-provided global monotonic counter** (a new `Arc<AtomicU64>` injected at `AppState` construction). | + Globally monotonic within the node lifetime, simpler to reason about. − New shared mutable state — `Arc<AtomicU64>` is a port-trait-shape concern that should be behind a `Counter` trait if introduced. − Doesn't help cross-node LWW — every writer still needs its own clock anchor. − Premature: ADR-0042 already pins the model on `tick.tick + writer`. |
| **C** | **Wall-clock millis** from `tick.now_unix`. | − Breaks under DST replay — `SimClock` advances by `tick()` and the granularity is not millis. − Erodes the §21 K3 property (seed → bit-identical trajectory). − No precedent in the codebase. |

#### Recommendation: **Option A — `tick.tick + 1` with `writer_node_id` from `AppState.node_id`**

Identical shape to the action shim. The `writer_node_id` field on
`AppState` (already set on lib.rs:571–573 with the placeholder
`NodeId::new("local")`) is the SSOT for "who wrote this row in
single-node Phase 2.2." When Phase 2 multi-node lands and a real
node-bootstrap identity replaces the placeholder (per the comment at
lib.rs:567–570), every `LogicalTimestamp` LWW write picks up the new
value with no bridge code change — the bridge reads
`state.node_id` exactly once at construction time.

The model does **not** preclude future multi-node:
- In Phase 2 with multiple writers per service, each writer
  generates its own monotonic counter from its own `tick.tick`; LWW
  resolution uses `counter` first, then `writer_node_id` as the
  deterministic tiebreak.
- The "owner-writer" pattern (only one node writes a given service's
  row) is a discipline layered on top, not encoded in the row shape.
- A future cluster-wide convention "the node hosting the
  `WorkloadLifecycle`-elected primary writer of the workload writes
  the bridge row" is additive: no change to the bridge's row
  construction.

### Q174.4 — VIP source (where the bridge reads the assigned VIP)

**Why this is open**: ADR-0049 (delivered 2026-05-19) made VIPs
platform-issued only. The operator-supplied `Listener.vip` field was
removed at the parser layer (ADR-0049 § 5, 2026-05-14 amendment), so
the bridge can no longer derive the VIP from intent. The question
this revision must answer is: **where does the bridge read the
allocator-issued VIP from?**

The prior resolution ("skip the listener when `vip.is_none()`") is
*structurally moot* — there is no `vip` field on `Listener` at all
anymore (`crates/overdrive-core/src/aggregate/workload_spec.rs:392`
confirms `Listener { port, protocol }`). The skip-vs-include decision
no longer exists; the new decision is the read path.

#### Options

| Option | VIP read source | Trade-offs |
|---|---|---|
| **A** | **Bridge reads allocator state directly** via `ServiceVipAllocator::get(&spec_digest)` at hydrate time. `spec_digest = WorkloadIntent::spec_digest(&intent)?`. | + Matches ADR-0049 § 5a's chosen placement (Option C — "the allocator's own persisted memo IS the source of truth"). + No second source of truth (no observation row to seed, no aggregate field). + Synchronous in-memory call (`PersistentServiceVipAllocator::get` is sync); cleanly composes inside `hydrate_desired`. + Restart-survival is the allocator's responsibility, already covered by its `bulk_load` + probe at boot (ADR-0049 § 1). + Same `spec_digest` keying `WorkloadLifecycle` already uses for `Action::ReleaseServiceVip` correlation — consistent across reconcilers. − Couples the bridge to the allocator's `get` surface; if the allocator type ever moves, the hydrate arm follows. (The allocator is a stable Phase 1 primitive; the coupling cost is bounded.) |
| **B** | **Bridge reads a derived view that joins `WorkloadIntent::Service` + allocator state** (a thin helper fn on `AppState`). | + Encapsulates the "intent + allocator" composition in one place. − No real isolation benefit — the helper would have one caller (the bridge). − Adds a layer with no test surface of its own. |
| **C** | **Allocator writes the assigned VIP back into a Service-scoped intent field that the bridge already reads via `hydrate_desired`.** | − Forbidden by ADR-0049 § 5a's rejected Option A — would reintroduce "a policy field on the operator-facing struct" exactly the smell the parser-level removal is fixing. − Re-couples the aggregate to allocator output (derived state on the spec). − Requires post-admission IntentStore mutation, which the admission path does not do. |
| **D** | **Allocator writes an observation row (`service_assignments`) that the bridge reads.** | − Same as ADR-0049 § 5a's rejected Option B — creates a second source of truth + chicken-and-egg on restart hydration ordering. |

#### Recommendation: **Option A — bridge reads `ServiceVipAllocator::get(&spec_digest)` directly**

This is structurally consistent with ADR-0049 § 5a's decision:
"downstream consumers of the assigned VIP (e.g. `ServiceMapHydrator`)
consult the allocator via `ServiceVipAllocator::get(&spec_digest)`."
The bridge IS such a downstream consumer; consulting the allocator at
hydrate time IS the canonical shape.

The composition order is already correct: the allocator is built in
`bulk_load_service_vip_allocator` before `AppState::new` (per
`crates/overdrive-control-plane/src/lib.rs:667-679`), which is
before the runtime ticks. The bridge inherits this ordering invariant
— `state.allocator.lock().await.get(&spec_digest)` is structurally
safe inside `hydrate_desired`.

**Edge case — allocator memo absent for the workload's spec_digest**:
In Phase 1's submit-time allocation path (ADR-0049 § 4), the VIP is
allocated synchronously BEFORE the IntentStore write — so by the
time the bridge ever runs against a persisted Service intent, the
allocator memo IS populated. A missing memo entry is therefore a
structural impossibility in Phase 1; if it occurs (e.g., a bug, a
storage corruption, an unanticipated race), the bridge logs a debug
event (`bridge.allocator_memo_absent`) and returns an empty `desired`
state for that workload, deferring the convergence to a subsequent
tick. This is correct under the §18 convergence model (eventual
consistency once the allocator state is reconciled).

A telemetry counter on the bridge's `View` for "allocator-memo-absent
ticks" is intentionally NOT added — Phase 1's structural guarantee
means non-zero values would indicate a bug worth a structured event
in production, not a per-target counter.

### Q174.5 — View shape

**Why this is open**: ADR-0035 § "Persist inputs, not derived state"
binds the View to inputs only. The bridge's View must carry enough
state to (a) dedup writes when inputs haven't changed and (b) survive
crash recovery via `bulk_load`.

#### Options

| Option | View shape | Trade-offs |
|---|---|---|
| **A** | `pub struct BackendDiscoveryBridgeView { last_written_fingerprint: BTreeMap<ServiceId, BackendSetFingerprint> }`. Per-target the runtime holds one View per `JobId`, but the field maps `ServiceId` because one workload can have N listeners → N services. | + Pure dedup memory: same fingerprint → don't write the row. + `BackendSetFingerprint` is `u64` per ADR-0042 / `dataplane::fingerprint::fingerprint(&vip, &backends)` — small, content-addressed, derived from inputs. + No derived deadlines or schedules. + Crash-safe: `bulk_load` rehydrates the dedup map; first tick after recovery rewrites everything (cheap idempotent step) — see § 5.2 below. |
| **B** | Empty View (`type View = ()`). Re-write `ServiceBackendRow` every tick regardless of fingerprint. | − Wastes ObservationStore writes on every tick. − LWW counter increments every tick even when content is unchanged, polluting the row stream. − No dedup signal for the runtime's eq-diff skip (ADR-0035 § Changelog 2026-05-04 lets the runtime elide `write_through` when next_view equals current; without inputs in the View, the eq check is trivially true and the elision triggers, BUT the row write still fires every tick — the ViewStore elision doesn't gate the obs write). |
| **C** | View carries the full last-written row: `BTreeMap<ServiceId, ServiceBackendRow>`. | − Stores derived state (the row IS the output of applying intent + actual). Violates "persist inputs, not derived state." − Multiplies View size by O(backends) — at 100 backends per service × 100 services, View is megabytes; redb write-through fsync per tick gets expensive. − Fingerprint dedup gives the same correctness signal at u64 cost. |

#### Recommendation: **Option A — `BTreeMap<ServiceId, BackendSetFingerprint>` View**

Pure inputs (fingerprints are content-hashes of inputs, addressable
by inputs). The fingerprint covers `(assigned_vip, backends)`, so a
VIP change automatically falsifies dedup and triggers a write — no
additional View field is required for VIP-change tracking. (In
practice, VIP changes do not occur in Phase 1 steady state: the
allocator's content-addressed memo returns the same VIP for the same
`spec_digest` across resubmits per ADR-0049 § 1.)

The dedup check inside `reconcile`:

```rust
// Pseudocode — actual code is the crafter's responsibility
for (service_id, desired_row) in projected_rows {
    let fp = fingerprint(&desired_row.vip, &desired_row.backends);
    if view.last_written_fingerprint.get(&service_id) == Some(&fp) {
        continue;  // No change; no action.
    }
    actions.push(Action::WriteServiceBackendRow {
        row: desired_row,
        correlation: ...,
    });
    next_view.last_written_fingerprint.insert(*service_id, fp);
}
// GC: drop entries for services no longer in projected_rows
next_view.last_written_fingerprint
    .retain(|sid, _| projected_rows.contains_key(sid));
```

`BTreeMap` per `.claude/rules/development.md` § "Ordered-collection
choice".

### Q175.1 — `ControlPlaneError` variant for `EbpfDataplane::new` failure

**Why this is open**: `.claude/rules/development.md` § "Never flatten
a typed error to `Internal(String)`" forbids
`.map_err(|e| ControlPlaneError::internal(...))` on a typed
infra error. `EbpfDataplane::new` returns
`Result<Self, DataplaneError>`, which carries typed variants
(`IfaceNotFound { iface }`, `MapAllocFailed { source }`,
`LoadFailed(String)`, `Io(#[from] io::Error)`, `Busy`).

#### Options

| Option | Variant shape | Trade-offs |
|---|---|---|
| **A** | **New `ControlPlaneError::DataplaneBoot(#[from] DataplaneBootError)`** with a small wrapper enum: `DataplaneBootError::Open { source: DataplaneError }`, `DataplaneBootError::Probe { source: ProbeError }` (the boot path also probes the dataplane per Earned Trust). Mirrors `ViewStoreBootError` precedent. | + Identical precedent in the same `error.rs` file — `ViewStoreBootError::{Open, Probe, BulkLoad}` + `ControlPlaneError::ViewStoreBoot(#[from] _)`. + Wrapper carries boot-specific context (`iface: String` on `Open`) that the bare `DataplaneError` doesn't surface. + Earned Trust: lets a future `Dataplane::probe()` method (recommended below) flow through a `Probe` variant; structurally compatible with adding the probe later. + `Display` carries operator-actionable guidance per `error.rs` precedent (e.g., "loader rejected interface `eth0`: ENODEV — verify the interface name with `ip link show`"). |
| **B** | **`ControlPlaneError::Dataplane(#[from] DataplaneError)`** — direct pass-through, no wrapper. | + Smallest surface (one new variant, no wrapper struct). − No boot-specific context (which iface? which step — pin path setup, ELF load, attach?). − `DataplaneError` is the *runtime* error surface — the same shape `update_service` returns. Conflating boot-time and steady-state errors loses the operator's ability to distinguish "the server failed to start" from "an in-flight reconcile failed." The `ViewStoreBoot` / `Cgroup` / `CgroupBootstrap` / `WorkloadsBootstrap` precedents all use a wrapper specifically to separate boot from steady-state. |
| **C** | Reuse `ControlPlaneError::Internal(String)` via `.map_err(|e| ControlPlaneError::internal("ebpf dataplane", e))`. | − **Forbidden** by the development.md rule cited above. Hard rejection. |

#### Recommendation: **Option A — `ControlPlaneError::DataplaneBoot(#[from] DataplaneBootError)`**

Concrete shape (the crafter implements; this records the decision):

```rust
// crates/overdrive-control-plane/src/error.rs
#[derive(Debug, Error)]
pub enum DataplaneBootError {
    /// `EbpfDataplane::new(client_iface, backend_iface)` failed. The
    /// underlying `DataplaneError` distinguishes IfaceNotFound,
    /// MapAllocFailed, LoadFailed, Io.
    #[error("EbpfDataplane construction failed (client_iface={client_iface}, backend_iface={backend_iface}): {source}\n\
             \n\
             Try: `ip link show <iface>` to verify the interface exists, \
             `mount | grep bpffs` to verify /sys/fs/bpf is mounted, and \
             `dmesg | tail` for kernel-side BPF verifier errors.")]
    Construct {
        client_iface: String,
        backend_iface: String,
        #[source]
        source: DataplaneError,
    },

    /// Earned-Trust probe failed after construction. (See Q175 follow-up below.)
    #[error("EbpfDataplane probe failed: {source}")]
    Probe {
        #[source]
        source: DataplaneError,
    },
}
```

`ControlPlaneError` gains `#[error(transparent)] DataplaneBoot(#[from] DataplaneBootError)` and `to_response` adds the matching arm (StatusCode::INTERNAL_SERVER_ERROR, never reaches HTTP because boot failure precedes listener bind — same exhaustiveness pattern as `ViewStoreBoot`).

### Q175.2 — Interface configuration source

**Why this is open**: `EbpfDataplane::new(client_iface, backend_iface)`
takes two interface names. The production binary needs to source
these from somewhere.

#### Options

| Option | Source | Trade-offs |
|---|---|---|
| **A** | **Operator config (`overdrive.toml`)** — new `[dataplane]` section in the config file with `client_iface = "lb_veth_a"` and `backend_iface = "lb_veth_b"`. ADR-0019 already establishes TOML as the operator config format and the config loader is established (`ConfigDir`). | + Operator-controlled, version-controlled with the rest of the host config. + Per-host customisation (different test envs / production hosts have different iface names). + Default values for Lima-dev convenience (e.g., `lo` fallback or a Lima-specific default) can ship in the example config. + No CLI surface change. |
| **B** | **CLI flag** on `overdrive serve` (or whichever command binds `lib.rs::serve_with_config`). | − Operator must remember to pass on every restart. − Surface duplicates what `overdrive.toml` already does for every other operator-tunable. |
| **C** | **Auto-detect** — find the first non-loopback IPv4 interface and use it for both. | − Wrong for production where multi-NIC is the norm. − Lima dev box currently uses dedicated veth pairs for XDP testing (`lb_veth_a` / `lb_veth_b` per `tests/integration/atomic_swap.rs`); auto-detect would pick `eth0` which won't have the test ifaces. − Doesn't model the `client_iface` vs `backend_iface` separation; the kernel sees two physical surfaces and the bridge can't reasonably guess which is which. |

#### Recommendation: **Option A — operator config `[dataplane]` section**

Concrete shape (in `crates/overdrive-cli/src/config.rs` or equivalent):

```toml
[dataplane]
# Interface the forward-path XDP program (xdp_service_map_lookup)
# attaches to. Operator-pinned per ADR-0045 § Operational. Required
# for production single-mode boot when the dataplane is enabled.
client_iface = "lb_veth_a"

# Interface the reverse-path XDP program (xdp_reverse_nat_lookup)
# attaches to. Per ADR-0045 § Decision: backend-facing veth ingress.
backend_iface = "lb_veth_b"
```

The boot path resolves the config like every other field; missing
section produces a typed `ConfigError` (already in the precedent
shape for missing `[tls]` / `[cgroup]` keys). A `[dataplane]` section
that's structurally invalid (empty strings, missing one of the two
keys) gives a parse error with named guidance.

**Deferral surfaced for user approval**: Should `[dataplane]`
**default to disabled** (`NoopDataplane` retained as a fallback for
hosts that don't want XDP) or **default to enabled** (boot refuses
without the section)? The single-cut migration rule says "delete
`NoopDataplane`" — but a host without kernel BPF support (an aarch64
CI box without virtio-net XDP, an old kernel) needs *some* path that
doesn't panic. The right shape is probably:

- **Refuse boot without the `[dataplane]` section** in production (single-cut, no `NoopDataplane` fallback).
- **Tests** install `Arc<SimDataplane>` directly via test harness construction (per the existing test convention in `service_hydration_results` shim — tests never see `NoopDataplane`).

This is the recommendation. Sole user-facing question: do you want a
config-file escape hatch (`[dataplane] disabled = true`) for hosts
that genuinely cannot run XDP? My recommendation is **no** — that
escape hatch is itself a "future deferral that compounds." If a host
can't run XDP, it can't run Service workloads; admission-side
refusal is honest. **Surfacing as a blocker** in the return message
for user decision.

### Q175.3 — Attach-mode fallback location

**Why this is open**: `.claude/rules/development.md` § "Attach mode —
native vs generic (`SKB_MODE`)" mandates a single structured
`tracing::warn!(name: "xdp.attach.fallback_generic", ...)` emission
on the `EOPNOTSUPP`/`ENOTSUP` fallback. The `EbpfDataplane::new`
implementation at `crates/overdrive-dataplane/src/lib.rs:246-253`
already documents that it does the native-first-then-SKB fallback;
the question is **where the warn event is emitted** so the
operator-facing log shows it once per boot.

#### Options

| Option | Emit site | Trade-offs |
|---|---|---|
| **A** | **Inside `EbpfDataplane::new`** at the moment the fallback decision is taken. The function already has access to the iface name and the underlying `errno`. | + Co-locates the event with the decision. + One source of truth — tests can subscribe to the `tracing` event and assert. + Matches the precedent of the `cgroup` pre-flight emitting from within `cgroup_preflight.rs`, not from the boot-path wrapper. − Couples `overdrive-dataplane` to `tracing` (already a workspace dep so this is structural neutrality). |
| **B** | **In `lib.rs::serve_with_config`** boot composition path, branching on a returned `attach_mode: AttachMode` field on `EbpfDataplane`. | − Forces `EbpfDataplane::new` to expose its attach-mode result publicly (`AttachMode::Native` / `AttachMode::Generic`) instead of keeping it as an implementation detail. − The fallback decision per-iface (forward-path may use native; reverse-path may need SKB), so a single field on the returned struct doesn't capture per-iface information cleanly. − Doubles the emit logic between the construction site and the boot site. |
| **C** | **Inside the loader's `should_fallback_to_generic` classifier** at `crates/overdrive-dataplane/src/lib.rs`. | + Already-existing classifier; the warn would be one log line at the existing call site. + Matches the `.claude/rules/development.md` § "Attach mode" comment: "The userspace classifier `should_fallback_to_generic` in `crates/overdrive-dataplane/src/lib.rs` enforces this." − Same module as (A) but at a finer-grained call site. |

#### Recommendation: **Option A — emit from `EbpfDataplane::new` itself**

The emit is part of the construction contract. The function already
takes both iface names and knows which one is being attached.
Test assertions hook the `tracing` subscriber. The classifier
(`should_fallback_to_generic`) is the *pure decision function*; the
emit + retry is the imperative dispatch and lives at the same level
as the retry call to `xdp.attach(iface, XdpFlags::SKB_MODE)`. This
matches the `cgroup_preflight` precedent.

### Q175.4 — Graceful shutdown sequencing

**Why this is open**: `.claude/rules/debugging.md` § "Leftover XDP
attachments across runs" documents the failure mode when an XDP
program survives the process. The bridge / dataplane boot path must
detach + unpin on shutdown.

#### Options

| Option | Shutdown shape | Trade-offs |
|---|---|---|
| **A** | **Drop guard on `EbpfDataplane`** — RAII detach and unpin on `Drop`. Production composition holds an `Arc<EbpfDataplane>`; on `serve_with_config` return, the final `Arc` clone is the boot composition's local; dropping the `Arc` drops the `EbpfDataplane`. | + Already the convention — aya's `XdpLinkId` Drop already detaches; the project owns one extra cleanup (the bpffs pin on the outer SERVICE_MAP). + No new method on the `Dataplane` trait surface. + Symmetric with `cgroup_manager`'s Drop-based scope cleanup. − Drop runs on panic too, which is correct for cleanup. − Drop runs in the thread that drops the `Arc`; for tokio-runtime-bound operations (closing fd, unlinking pin), this is fine because they're sync syscalls. − The `Arc<dyn Dataplane>` shape means the concrete type's Drop fires only when no other Arc clone outlives — needs care that no stray clone in the worker subsystem holds it past `serve_with_config`'s return. |
| **B** | **New `Dataplane::shutdown(&self) -> Result<(), DataplaneError>` method** on the trait, called explicitly in the boot path's shutdown branch. | − Extends the trait surface for a concern (cleanup) that RAII handles. − Asymmetric — `NoopDataplane` and `SimDataplane` would need empty shutdown methods. − If the explicit call is forgotten or panics, the resource leaks anyway; RAII is the catch-all. |
| **C** | **Control-plane-owned cleanup hook** — a separate `dataplane_cleanup()` fn invoked from the shutdown branch. | − Same problems as (B); also moves cleanup logic away from the type that knows what needs cleaning. |

#### Recommendation: **Option A — Drop guard on `EbpfDataplane`**

The XDP detach is already RAII (`XdpLinkId::Drop`). The new cleanup
the project adds is the bpffs pin unlink:

```rust
// Pseudocode — crafter implementation
impl Drop for EbpfDataplane {
    fn drop(&mut self) {
        // Unlink the SERVICE_MAP pin at /sys/fs/bpf/overdrive/SERVICE_MAP.
        // Failure logs at debug — by the time Drop runs, panic propagation
        // is in-flight and can't bubble errors. The leftover-pin cleanup
        // discipline in .claude/rules/debugging.md is the operator-side
        // safety net if Drop is skipped (SIGKILL).
        let _ = std::fs::remove_file(self.pin_dir.join(SERVICE_MAP_NAME));
        // XdpLinkId fields drop automatically; aya detaches.
    }
}
```

A SIGKILL still leaves a leak — that's the `.claude/rules/debugging.md`
cleanup-discipline scenario; not a bug to fix in code.

### Q175.5 — BPF object path resolution

**Why this is open**: `EbpfDataplane::new` loads a BPF ELF from disk
(or from a `target/bpf/overdrive_bpf.o` build artifact). The
production binary needs to find this file at runtime.

#### Options

| Option | Resolution shape | Trade-offs |
|---|---|---|
| **A** | **`OVERDRIVE_BPF_OBJECT` env var, falling back to embedded bytes** via `include_bytes!()` from a build-time path resolved by `crates/overdrive-dataplane/build.rs`. | + Already the codebase pattern — see `.claude/rules/testing.md` § "BPF-object-dependent crates work via env override" + `crates/overdrive-dataplane/build.rs` reads `OVERDRIVE_BPF_OBJECT` first. + Production binaries embed at build time (the `include_bytes!` path); dev/test override via env. + No runtime file open by default — the binary is self-contained. − Build-time-dependency: the BPF object MUST be built before the `overdrive-control-plane` binary. `cargo xtask bpf-build` is the wrapper; documented. |
| **B** | **Runtime path discovery** — `EbpfDataplane::new` takes a `Path` arg, the boot path passes one from operator config. | − Adds a `[dataplane] bpf_object_path` config key the operator must maintain. − Production install story is more brittle (move the file = boot breaks). − Doesn't match existing precedent. |
| **C** | **Packaged install path `/usr/lib/overdrive/overdrive_bpf.o`** baked into the binary. | − Forces operators to use a specific packaging layout. − Single-binary distribution becomes "binary + .o file"; harder to deploy. |

#### Recommendation: **Option A — embedded bytes with `OVERDRIVE_BPF_OBJECT` dev override**

Already in place. The boot path doesn't need to know the BPF object's
location at all — `EbpfDataplane::new(client_iface, backend_iface)`
resolves it internally via `include_bytes!`-embedded data, with the
env var as the dev/test override.

No `[dataplane]` config field needed for the BPF object path. The
boot composition stays clean: read `[dataplane]` section, extract
`(client_iface, backend_iface)`, call `EbpfDataplane::new(client_iface, backend_iface)`.

### QJ.1 — Walking-skeleton scenario shape

**Why this is open**: ASR-2.2-04 demands hydrator ESR closure under
DST. The user-facing acceptance from #174 ("multiple replicas produce
multiple backends") and #175 ("BPF map contains the expected
SERVICE_MAP entry") need a single end-to-end gate the bridge + boot
wiring close.

#### Options

| Option | Scenario shape | Trade-offs |
|---|---|---|
| **A** | **One Tier 3 test** at `crates/overdrive-control-plane/tests/integration/backend_discovery_bridge/walking_skeleton.rs`. Submits a Service spec with one listener (`port=8080, protocol=tcp, vip=10.0.0.1`); waits for Running; reads `BACKEND_MAP` via the typed `BackendMapHandle` and asserts the expected backend entry is present. Optionally also opens a real TCP connection to `10.0.0.1:8080` and asserts data flow (extends to ASR-2.2-01 reuse). | + One test gates the whole feature — clear pass/fail signal. + Lives in the control-plane crate's `tests/integration/` (already gated by `integration-tests` feature). + Runs through `cargo xtask lima run --` per `.claude/rules/testing.md` § "Running tests — Lima VM". + Asserts on *observable kernel side effect* (`bpftool map dump` semantics via the typed handle) per `.claude/rules/testing.md` § "Tier 3 → Assertion rules" — NOT on program internal reachability. + Map-state assertion alone is sufficient; the real-TCP step is incremental. |
| **B** | **Two tests** — one in `overdrive-control-plane` (the bridge writes the row), one in `overdrive-dataplane` (the dataplane programs the map). Compose at CI level. | − Loses the cross-component property. The bridge could write the row and the dataplane could program a map, but the *connection* between them is exactly what J-PLAT-004 closes. Splitting blinds the gate to the bug class. |
| **C** | **DST-only walking skeleton** in `overdrive-sim` with `SimDataplane`. | − Doesn't exercise the real `EbpfDataplane`. The Tier 1 DST already runs the hydrator ESR pair; this would be redundant with the existing `HydratorEventuallyConverges` / `HydratorIdempotentSteadyState` invariants. − Doesn't gate #175. |

#### Recommendation: **Option A — single Tier 3 test**

Scenario shape (revised for ADR-0049 platform-issued VIPs):

```
GIVEN a control-plane configured with EbpfDataplane on Lima
       (client_iface=lb_veth_a, backend_iface=lb_veth_b)
  AND the BackendDiscoveryBridge reconciler registered
  AND the ServiceMapHydrator reconciler registered
  AND the ServiceVipAllocator bulk-loaded + probe-passed
       (allocator state is empty at boot for a fresh test fixture)
WHEN a Service spec is submitted with:
       id="walking-skeleton-svc"
       replicas=1
       [[listener]] port=8080, protocol=tcp
         (NO vip field — operator-supplied VIPs are unrepresentable
         per ADR-0049 § 5; the parser rejects an unknown `vip` field
         via #[serde(deny_unknown_fields)])
       [exec] cmd="bash -c 'while true; do nc -l 8080; done'"
  AND admission allocates VIP V via ServiceVipAllocator
       synchronously, before IntentStore write (per ADR-0049 § 4)
  AND the test reads V from the submit-echo response
  AND the alloc reaches Running
THEN within N reconciler ticks (≤ 5 × 100ms = 500ms),
     the BACKEND_MAP contains an entry whose ipv4 matches
     the host's lb_veth_a address and whose port = 8080
  AND the SERVICE_MAP for ServiceKey { vip=V, port=8080, proto=TCP }
     resolves to a non-empty inner map containing that BackendId
  AND (optional Phase 2.2 stretch) opening a TCP connection to
     V:8080 succeeds end-to-end and `nc -l 8080` accepts.
```

**Test setup must explicitly verify the allocator memo is populated
for the workload's `spec_digest` before declaring the precondition
met.** This is the new prerequisite that did not exist when VIPs
were operator-pinned. The verification is a single
`state.allocator.lock().await.get(&spec_digest).is_some()` assertion
in the fixture setup; failing it would indicate a regression in the
admission path's allocator wiring (ADR-0049 § 4), not in the bridge
itself, and surfacing it explicitly preserves the test's altitude per
`.claude/rules/debugging.md` § 7.

The optional TCP connection step is **deferred from the gate**. The
map-state assertion is sufficient and structurally avoids the
"`nc -l` race / port-not-yet-listening" flake class. The TCP step
can land as a follow-up test once the map gate is green; it's not on
the acceptance criteria for this feature unless it's needed to
satisfy ASR-2.2-04 closure (the existing S-2.2-17 already covers TCP
through-VIP at the dataplane level — the walking-skeleton's job is
the bridge + boot, not the dataplane itself).

### QJ.2 — Sequencing across DELIVER slices

**Why this is open**: #175's walking-skeleton fails without #174.
The crafter needs a sequencing plan.

#### Options

| Option | Sequencing | Trade-offs |
|---|---|---|
| **A** | **#175 first with walking-skeleton `#[ignore]`-gated; #174 next; walking-skeleton ungated at #174 land.** | + Allows #175 (boot composition) to land independently and gate-checked via existing `service_map_hydrator` shim tests with hand-injected `service_backends` rows. + Walking-skeleton sits with #175's boot work, ungated by #174. + Both deliveries get green CI from day 1. − The `#[ignore]` adds a one-line gate that must be removed on #174 land; the reviewer must verify. |
| **B** | **#174 first; #175 next.** Strict sequential. | + Walking-skeleton lands green at #175 (no ignore-gating). + #174 has its own DST invariant gate (existing `HydratorEventuallyConverges` / `HydratorIdempotentSteadyState`) — proves the bridge works without the real dataplane. − #174 lands with no end-to-end signal beyond DST. − Two separate PR trains for two issues that genuinely require each other for value. |
| **C** | **Single PR train, both at once.** | − Larger surface area in one PR. − Hard to review piecemeal. − The Tier 3 walking-skeleton + the bridge logic + the boot composition + the error variant land in one commit; reviewer can't isolate failures. |

#### Recommendation: **Option B — #174 first, then #175**

Concrete sequencing:

1. **Slice 1 (closes #174)**: Implement `BackendDiscoveryBridge`
   reconciler + register in production boot. The bridge writes
   `ServiceBackendRow` rows under `NoopDataplane` — the existing
   action shim still dispatches to `NoopDataplane::update_service`
   which returns `Ok(())` and the hydrator observes a Completed row.
   Tier 1 DST proves the bridge correctness via a new
   `BridgeEventuallyWritesBackendRow` invariant (paralleling
   `HydratorEventuallyConverges`). No walking-skeleton yet.

2. **Slice 2 (closes #175)**: Replace `NoopDataplane` with
   `EbpfDataplane` in production boot. Add `DataplaneBootError`
   variant. Add `[dataplane]` operator config (`client_iface` /
   `backend_iface` — distinct from the existing
   `[dataplane.vip_allocator]` subsection delivered by ADR-0049 § 3
   that already nests under the same `[dataplane]` parent). Land the
   Tier 3 walking-skeleton test that subsumes both deliveries.

**Allocator dependency status (revised 2026-05-20)**: ADR-0049's
`ServiceVipAllocator` is already delivered (feature archived at
`docs/evolution/2026-05-19-service-vip-allocator.md`). Both slices
proceed against the allocator's `AppState` integration as a landed
dependency — `state.allocator: Arc<Mutex<PersistentServiceVipAllocator>>`
exists today (per `lib.rs:183`); the bridge's `hydrate_desired` arm
consumes it directly.

This gives a clean per-issue acceptance shape: #174's gate is its DST
invariant; #175's gate is the walking-skeleton. Each issue closes
with its own acceptance signal. The walking-skeleton fails until #175
lands because `NoopDataplane`'s no-op `update_service` doesn't
populate BACKEND_MAP — so the test correctly gates #175's PR.

The cost: in the window between Slice 1 landing and Slice 2 landing,
`ServiceBackendRow` rows are written and the hydrator emits
`Action::DataplaneUpdateService` actions that the shim dispatches to
`NoopDataplane`. The hydrator's `actual` projection sees
`Completed` rows back. **No production traffic flows** (Phase 2.2 is
not yet shipped); this is purely the integration of in-process
components against a no-op dataplane. The window is bounded by
"how long until Slice 2 lands" — measured in days, not weeks.

---

## 4. Deletion discipline — single-cut migration of `NoopDataplane`

Per `feedback_single_cut_greenfield_migrations.md` and the development.md
§ "Deletion discipline" rule, `NoopDataplane` is **deleted from the
production boot path** in the same commit that lands #175 Slice 2.
The crate `overdrive-host` retains `NoopDataplane` only if a `grep
-rn 'NoopDataplane' crates/` after the boot-path swap reveals active
test users; the working assumption is **delete the type entirely from
`crates/overdrive-host/src/dataplane.rs` AND remove the `pub use
dataplane::NoopDataplane` re-export from `lib.rs`**. Tests today install
`Arc<SimDataplane>` directly (per the docstring at `dataplane.rs:13-15`),
so a workspace-wide grep should return no active production users
once the boot path is rewired.

Per the same rule, no feature flag, no `[dataplane] enabled = true`
fallback, no "default to NoopDataplane if config missing." Production
boot refuses to start without `[dataplane]` section.

---

## 5. Cross-cutting design notes

### 5.1 Reconciler trait conformance

The new `BackendDiscoveryBridge` reconciler follows ADR-0035 / ADR-0036
verbatim:

- `type State = BackendDiscoveryBridgeState` — typed projection
  carrying `desired: BTreeMap<JobId, ServiceListenerSet>` (intent: the
  set of `(ServiceId, vip, port, protocol)` tuples derivable from the
  Service spec's `listeners` field, filtered to `vip.is_some()`) and
  `actual: BTreeMap<JobId, BTreeSet<AllocationId>>` (observation: the
  Running set per workload).
- `type View = BackendDiscoveryBridgeView` — `BTreeMap<ServiceId,
  BackendSetFingerprint>` per Q174.5 above.
- `fn reconcile(&self, desired, actual, view, tick) -> (Vec<Action>, View)` — pure sync.
- The new action variant the bridge emits is `Action::WriteServiceBackendRow { row, correlation }` — see § 5.2 below.

### 5.2 New Action variant — `Action::WriteServiceBackendRow`

The bridge needs an Action variant that the action shim dispatches to
`ObservationStore::write(ObservationRow::ServiceBackend(row))`. The
ADR-0023 action shim discipline ("every Action is an exhaustive
match") means we cannot side-channel the write through `reconcile`'s
return — reconcilers are pure, all side effects are Actions.

Concrete shape (architecture.md spec):

```rust
// In Action enum, crates/overdrive-core/src/reconciler.rs
WriteServiceBackendRow {
    row: ServiceBackendRow,
    correlation: CorrelationKey,
},
```

Action-shim wrapper lives at
`crates/overdrive-control-plane/src/action_shim/write_service_backend_row.rs`,
shape symmetric with `dataplane_update_service.rs`:

```rust
pub async fn dispatch(
    action: &Action,
    observation: &dyn ObservationStore,
    tick: &TickContext,
) -> Result<(), ObservationStoreError> {
    let Action::WriteServiceBackendRow { row, correlation: _ } = action else {
        panic!("wrong Action variant");
    };
    observation.write(ObservationRow::ServiceBackend(row.clone())).await
}
```

Action-shim match in `action_shim/mod.rs` gains one new arm.

### 5.3 Hydration / Runtime extension (3 sites)

Per ADR-0036, the runtime owns `hydrate_desired` / `hydrate_actual`.
The bridge needs two new match arms:

```rust
// In reconciler_runtime.rs (hydrate_desired) — see also Q174.2 above
// for the full shape with allocator lookup.
AnyReconciler::BackendDiscoveryBridge(_) => {
    let workload_id = workload_id_from_target(target)?;
    let key = IntentKey::for_workload(&workload_id);
    let Some(bytes) = state.store.get(key.as_bytes()).await? else {
        return Ok(AnyState::BackendDiscoveryBridge(/* empty */));
    };
    let intent = WorkloadIntent::from_store_bytes(
        bytes.as_ref(), &state.intent_redb_path, Some(key.as_str()))?;
    let WorkloadIntent::Service(service_v1) = &intent else {
        return Ok(AnyState::BackendDiscoveryBridge(/* empty */));
    };
    let spec_digest = intent.spec_digest()?;
    let Some(assigned_vip) = state.allocator.lock().await.get(&spec_digest) else {
        return Ok(AnyState::BackendDiscoveryBridge(/* empty — log debug */));
    };
    // Project (port, protocol) listeners onto ProjectedListener,
    // keyed by ServiceId derived from the allocator-issued VIP.
    let listeners: BTreeMap<ServiceId, ProjectedListener> =
        service_v1.listeners.iter().map(|l| {
            let service_id = ServiceId::derive(&assigned_vip, l.port, "service-map");
            (service_id, ProjectedListener {
                vip: assigned_vip.clone(), port: l.port, protocol: l.protocol,
            })
        }).collect();
    Ok(AnyState::BackendDiscoveryBridge(/* desired filled */))
}

// In reconciler_runtime.rs (hydrate_actual):
AnyReconciler::BackendDiscoveryBridge(_) => {
    let workload_id = workload_id_from_target(target)?;
    let allocs = state.obs.alloc_status_rows_for_workload(&workload_id).await?;
    let running: BTreeSet<AllocationId> = allocs.into_iter()
        .filter(|r| matches!(r.state, AllocState::Running))
        .map(|r| r.alloc_id)
        .collect();
    Ok(AnyState::BackendDiscoveryBridge(/* actual filled */))
}
```

The runtime's existing `register()` flow handles `bulk_load` of the
View via the `ViewStore` machinery — no new code there.

### 5.4 Endpoint derivation (the substantive bridge logic)

For each Running allocation × each pinned listener on the spec, the
bridge derives:

```
Backend {
    ipv4: <node_ip from alloc.node_id resolved against AppState.node_id>,
    port: listener.port,
    weight: 1,
    healthy: true,  // Phase 2.2 stub — #170 ships real health
    _pad: 0,
}
```

Phase 2.2 is single-node, so `alloc.node_id == AppState.node_id`
always — `node_ip` resolves to the configured `client_iface` IPv4
(read once at boot via `if_nametoindex` + `getifaddrs`, cached on
`AppState`). The bridge does NOT call `getifaddrs` itself — the
boot path injects `host_ipv4: Ipv4Addr` into the bridge's
`new(host_ipv4)` constructor or into the runtime's hydrate path.

Multi-node Phase 2 extension is structurally compatible:
`alloc.node_id` lookup against a `NodeHealthRow` (or future
`NodeAddressRow`) returns the IP per node. Out of scope for this
feature.

### 5.5 ServiceId derivation

`ServiceId` is a `u64` newtype, content-hashed per ADR-0040 § 1 from
`(VIP, port, scope)`. The bridge derives `ServiceId` from each
`(assigned_vip, listener.port, "service-map")` tuple, where
`assigned_vip` is sourced from `ServiceVipAllocator::get(&spec_digest)`
per Q174.4. The `listener` itself carries no VIP (ADR-0049 § 5 —
parser-level removal).

Per-Service identity is stable across resubmits because (a)
`spec_digest` is stable across resubmits of the same operator input
(ADR-0049 § 4 — spec digest invariance), and (b) the allocator's
content-addressed memo on that digest is stable (ADR-0049 § 1 —
memo-hit returns the existing VIP). Therefore `ServiceId` is stable
across resubmits without any spec or aggregate field carrying it.

### 5.6 Earned-Trust probe (deferred from initial scope)

Principle 12 (Earned Trust) requires every adapter to demonstrate it
can honor its contract. `EbpfDataplane::new` performs an implicit
probe today (loads the BPF ELF, attaches the programs — failure
surfaces as `LoadFailed`). A more thorough probe — write a synthetic
entry to BACKEND_MAP, read it back, assert equal — would prove the
HoM-pin-by-name reuse path works in the live environment.

**Recommendation**: ship a basic probe in #175 Slice 2 with the
shape `EbpfDataplane::probe()` that writes + reads a sentinel entry
to BACKEND_MAP using a reserved `BackendId::PROBE = u32::MAX`, and
the boot path calls it after construction (matching the ViewStore
probe-then-use pattern in `lib.rs:543-553`). The probe failure
surface flows through `DataplaneBootError::Probe`. **Surfacing as a
deferral for user approval** — landing the probe IS the Earned Trust
discipline; not landing it is technical debt that's load-bearing for
correctness.

### 5.7 Inner-map population for ServiceMapHydrator

The downstream `ServiceMapHydrator` consumes the bridge's output and
emits `Action::DataplaneUpdateService`, which the action shim
dispatches to `Dataplane::update_service(vip, backends)`. The
existing `EbpfDataplane::update_service` shape (verified at
`crates/overdrive-dataplane/src/lib.rs`) takes IPv4 and `Vec<Backend>`
and runs the Maglev + atomic-swap sequence internally — no new
dataplane work for this feature.

The bridge writes the `ServiceBackendRow`; the hydrator reads it;
the action shim dispatches it to the dataplane. Three components,
three responsibilities, one data flow. This is the §18 reference
shape.

---

## 6. Deferrals surfaced for user approval

Per CLAUDE.md § "Deferrals require GitHub issues — AND user approval
BEFORE creation", the following items are surfaced here for explicit
user decision; **no GH issues are created by this design wave**.

| # | Item | Recommendation | User decision needed |
|---|------|----------------|----------------------|
| D1 | `[dataplane] disabled = true` config escape hatch for hosts without XDP kernel support. | **Do not ship.** Boot refuses without `[dataplane]` `client_iface`/`backend_iface` keys. | Confirm or override. |
| D2 | Earned-Trust probe on `EbpfDataplane::probe()` — synthetic BACKEND_MAP write + read-back. | **Ship in #175 Slice 2.** Land alongside the boot composition. | Confirm or defer (creates a follow-up issue). |
| D3 | Walking-skeleton's optional "real TCP connection to VIP succeeds" step. | **Defer.** Map-state assertion is sufficient. The TCP step is already covered by S-2.2-17 at the dataplane level. | Confirm or override. |
| D4 | `node_ip` resolution — boot-time `getifaddrs` for the host's `client_iface` IPv4. | **Ship as part of the boot composition** in #175. The single-node assumption makes this a one-shot lookup at boot. | Confirm shape. |
| D5 | ~~Bridge `View` telemetry field `listeners_skipped: u64` for VIP-less listener observability.~~ | **WITHDRAWN (2026-05-20).** No longer applicable — operator-supplied VIPs are unrepresentable per ADR-0049 § 5; there is no VIP-less-listener class to skip or count. Allocator-memo-absent is a structural impossibility in Phase 1's submit-time path (Q174.4 above); if it occurs, a structured debug event is emitted at the hydrate site rather than a per-target counter. | None — withdrawn. |

None of these are blockers. Each is a small structural choice that
the user should confirm before DELIVER starts.

---

## 7. Cross-references to brief.md additions

This feature adds to `docs/product/architecture/brief.md`:

- **§ 63 (NEW)**: `BackendDiscoveryBridge` reconciler — placement
  under `crates/overdrive-control-plane/src/reconcilers/backend_discovery_bridge/`.
  Inputs (intent: `WorkloadIntent::Service(ServiceV1).listeners` per
  ADR-0050; allocator-issued VIP via
  `ServiceVipAllocator::get(&spec_digest)` per ADR-0049 § 5a;
  observation: `alloc_status_rows_for_workload` filtered to Running).
  Output: `Action::WriteServiceBackendRow`.
- **§ 64 (NEW)**: Production `EbpfDataplane` boot composition.
  `[dataplane]` config section (`client_iface` + `backend_iface` keys,
  distinct from the existing `[dataplane.vip_allocator]` subsection
  delivered by ADR-0049 § 3). `DataplaneBootError` variant on
  `ControlPlaneError`. RAII shutdown via Drop. `OVERDRIVE_BPF_OBJECT`
  build-time embed.

The brief.md edit is part of this DESIGN wave's deliverable; see
the targeted edit in the same artifact set. ADR companion for this
feature is **ADR-0052** (renumbered 2026-05-20 from ADR-0049 after
ADR-0049 was reassigned to the VIP allocator).

---

## 8. Quality-attribute scenarios

| ASR | Quality attribute | Scenario | Pass criterion |
|---|---|---|---|
| **ASR-BDB-01** | Correctness — bridge ESR closure | DST harness with `SimDataplane` + `SimObservationStore`; arbitrary sequence of alloc transitions (Pending → Running → Failed → Terminated) for a Service workload with multiple listeners; assert `service_backends` rows always converge to "exactly the set of Running allocs' endpoints filtered to pinned-VIP listeners." | New invariant `BridgeEventuallyWritesBackendRow` (paralleling `HydratorEventuallyConverges`) holds across the seeded fault catalogue (J-PLAT-004 closure for the write side). |
| **ASR-BDB-02** | Correctness — bridge idempotent steady-state | Once `actual` matches `desired` for every service, the bridge emits zero `Action::WriteServiceBackendRow` actions per tick. | New invariant `BridgeIdempotentSteadyState` holds in DST. |
| **ASR-BDB-03** | Reliability — production boot under correct config | Production boot with valid `[dataplane]` config on Lima (`lb_veth_a` / `lb_veth_b`). | `EbpfDataplane` constructs, attaches both XDP programs, BACKEND_MAP is reachable. No `health.startup.refused` event. |
| **ASR-BDB-04** | Reliability — production boot under invalid config | Production boot with `[dataplane]` section pointing at a non-existent iface. | `ControlPlaneError::DataplaneBoot(Construct { source: IfaceNotFound { iface }, .. })` surfaces; binary exits non-zero; operator-facing message names the iface and suggests `ip link show`. |
| **ASR-BDB-05** | End-to-end — walking skeleton | Tier 3 integration test per QJ.1 above; one Service workload reaches Running with a pinned VIP and one TCP listener. | BACKEND_MAP entry exists matching the host's iface IPv4 and the listener port; SERVICE_MAP inner-map resolves the (VIP, port) to a non-empty backend set; ≤ 5 reconciler ticks (≤ 500ms) from `AllocState::Running` to map entry. |

---

## 9. Open questions resolved by this DESIGN

| Question | Resolution |
|---|---|
| New reconciler vs extend `WorkloadLifecycle`? | New reconciler `backend-discovery-bridge` (Q174.1, Option A). Separation of concerns stronger after lifecycle gained Service-VIP reclamation. |
| Trigger / hydration shape? | Broker-pending on alloc-state change; runtime hydrates desired (`WorkloadIntent::Service(ServiceV1)` per ADR-0050 + `ServiceVipAllocator::get(&spec_digest)` per ADR-0049 § 5a) + actual (obs) (Q174.2, Option A). |
| Owner-writer / Lamport counter? | `tick.tick + 1` + `AppState.node_id`; multi-node compatible (Q174.3, Option A). |
| VIP source? | Bridge reads `ServiceVipAllocator::get(&spec_digest)` directly at hydrate time (Q174.4, Option A — revised 2026-05-20). The prior "skip VIP-less listeners" framing is moot; the field no longer exists. |
| View shape? | `BTreeMap<ServiceId, BackendSetFingerprint>` for dedup; fingerprint covers `(assigned_vip, backends)` so VIP changes naturally falsify dedup (Q174.5, Option A). |
| `ControlPlaneError` variant for boot failure? | `DataplaneBoot(#[from] DataplaneBootError)` with `{Construct, Probe}` arms (Q175.1, Option A). |
| Interface configuration source? | `[dataplane]` section in `overdrive.toml` with `client_iface` + `backend_iface` (Q175.2, Option A). |
| Where does attach-mode fallback emit? | Inside `EbpfDataplane::new` itself (Q175.3, Option A). |
| Shutdown shape? | RAII via `Drop` on `EbpfDataplane` (Q175.4, Option A). |
| BPF object path? | `include_bytes!` embed + `OVERDRIVE_BPF_OBJECT` env override; no operator config (Q175.5, Option A). |
| Walking skeleton scenario? | Single Tier 3 test asserting BACKEND_MAP + SERVICE_MAP state after Running (QJ.1, Option A). |
| Sequencing across DELIVER? | #174 first (bridge under `NoopDataplane`), then #175 (real `EbpfDataplane` + walking-skeleton) (QJ.2, Option B). |
