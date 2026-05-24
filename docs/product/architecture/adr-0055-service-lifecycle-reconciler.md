# ADR-0055 — `ServiceLifecycleReconciler`: typed View, pure `reconcile`, `Stable` as non-terminal condition extending ADR-0037

## Status

Accepted. 2026-05-24. Decision-makers: Morgan (proposing); DESIGN-wave
output of `docs/feature/service-health-check-probes/`.

Tags: phase-1, service-kind, application-arch, reconciler-primitive.

**Companion ADRs**: ADR-0054 (ProbeRunner), ADR-0056 (per-kind
streaming evolution), ADR-0057 (TOML spec), ADR-0058 (default-probe
inference). **Extends**: ADR-0037 (typed TerminalCondition;
non-terminal `Stable` is novel) and ADR-0035/0036 (typed View
runtime contract).

## Context

ADR-0047 split the Phase 1 reconciler into per-kind behaviour. The
Job kind already has typed terminal conditions (`Completed`,
`Failed`) per ADR-0037 Amendment 2026-05-10. The Service kind needs
two things ADR-0037 did not anticipate:

1. **A non-terminal condition.** `Stable` is the operator-meaningful
   "the Service is serving" claim — but unlike `BackoffExhausted` or
   `Stopped`, the Service alloc continues after `Stable` is emitted
   (it accepts traffic, runs readiness/liveness, may restart). The
   reconciler must announce `Stable` once without that announcement
   forclosing further state transitions.
2. **A reaction to continuous observation rows.** `ProbeResultRow`
   (ADR-0054) lands on every tick; the reconciler reads them and
   produces `Action::SetBackendHealthy` (readiness),
   `Action::RestartAllocation` (liveness threshold), or
   `Action::SetTerminalCondition(Stable | Failed)` (startup gate).

Open questions resolved here (P1-Q3 part 1, P2-Q7, P2-Q8, P2-Q9
architectural shape):

- How does the reconciler's `View` shape capture inputs for stable
  detection without persisting derived state?
- What is the AND/OR semantic when multiple startup probes are
  declared?
- What is the readiness `successThreshold` shape (configurable
  consecutive-success requirement)?
- How does the architecture leave room for future cascading-restart
  rate-limiting (research D6) without coupling Phase 1 to it?

## Decision

### 1. Crate placement — `overdrive-control-plane::reconcilers::service_lifecycle`

`ServiceLifecycleReconciler` lives at
`crates/overdrive-control-plane/src/reconcilers/service_lifecycle/`
(new module tree). The existing `WorkloadLifecycle` reconciler is
**not** split into Service / Job sibling structs; instead,
`WorkloadLifecycle::reconcile` branches on `desired.kind()` and
dispatches to per-kind helper functions. The new
`ServiceLifecycleReconciler` is the body of the new Service branch
extracted into its own typed reconciler IFF the per-kind branching
within WorkloadLifecycle exceeds a maintainability threshold (~ 600
LOC in the body).

**Phase 1 decision: ServiceLifecycleReconciler IS its own typed
reconciler — separate `AnyReconciler` variant, separate `AnyState`
variant, separate `AnyReconcilerView` variant.** Rationale:

- The Service `View` shape (consecutive-failures-per-liveness-probe,
  last-startup-pass-tick-per-probe, current-readiness-status) is
  structurally disjoint from the Job `View` shape (`restart_counts`
  per ADR-0035 §"Worked example"). Sharing a single struct with
  optional fields would violate `development.md` § "Sum types over
  sentinels".
- `WorkloadLifecycle` per ADR-0035 stays as the Job-kind reconciler;
  this ADR adds `ServiceLifecycle` as a sibling.
- `WorkloadLifecycle`'s body for `desired.kind() == Service`
  currently handles the existing `ConvergedRunning` path; that path
  is removed from `WorkloadLifecycle` and re-homed under
  `ServiceLifecycleReconciler`. Single-cut migration per
  `feedback_single_cut_greenfield_migrations.md`.

### 2. Typed `State`, `View`, `AnyState` / `AnyReconcilerView` variants

```rust
// crates/overdrive-core/src/reconcilers/service_lifecycle.rs (new)

/// `desired`/`actual` projection for the Service-kind reconciler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceLifecycleState {
    /// `desired`: the ServiceSpec from the IntentStore (per ADR-0050).
    /// `actual`: empty placeholder; not used (per ADR-0021 the projection
    /// is per-reconciler).
    pub spec: Option<ServiceSpec>,

    /// Observation: per-alloc status rows.
    pub allocations: BTreeMap<AllocationId, AllocStatusRow>,

    /// Observation: latest probe results per alloc, keyed by probe_idx.
    pub probe_results: BTreeMap<AllocationId, BTreeMap<ProbeIdx, ProbeResultRow>>,

    /// Observation: per-alloc backend rows (consumed by readiness branch).
    pub service_backends: BTreeMap<AllocationId, ServiceBackendRow>,
}

/// Typed `View` — persisted by the runtime via `ViewStore::write_through`.
#[derive(
    Debug, Clone, PartialEq, Eq, Default,
    Serialize, Deserialize,
)]
pub struct ServiceLifecycleView {
    /// Per-alloc liveness probe consecutive-failure counters.
    /// INPUT (per `.claude/rules/development.md` § "Persist inputs,
    /// not derived state"). The restart-trigger predicate is
    /// recomputed every tick from this map plus the live
    /// `failure_threshold` from the spec.
    pub liveness_consecutive_failures:
        BTreeMap<AllocationId, BTreeMap<ProbeIdx, u32>>,

    /// Per-alloc readiness probe consecutive-success counters.
    /// INPUT (consumed by the readiness `successThreshold` gate per
    /// P2-Q8). The flap-protection predicate is recomputed from this
    /// counter plus the live `success_threshold`.
    pub readiness_consecutive_successes:
        BTreeMap<AllocationId, BTreeMap<ProbeIdx, u32>>,

    /// Per-alloc record of "Stable was announced for this alloc".
    /// INPUT — distinguishes the deciding-tick announcement from
    /// subsequent steady-state ticks. WITHOUT this flag, the
    /// reconciler would re-emit `Action::SetTerminalCondition(Stable)`
    /// every tick after the startup gate passes; the action shim
    /// would re-write the same row N times and re-broadcast the same
    /// event N times.
    pub stable_announced: BTreeSet<AllocationId>,

    /// Per-alloc startup attempt counter (informational + drives
    /// `StartupProbeFailed { attempts }` reporting).
    pub startup_attempts_per_probe:
        BTreeMap<AllocationId, BTreeMap<ProbeIdx, u32>>,
}
```

`Stable` IS NOT persisted as a derived field. The "Stable predicate"
is recomputed every tick from:

```
is_stable(alloc) =
    spec.startup_probes.iter().all(|probe|
        actual.probe_results[alloc][probe.idx].status == Pass)
```

— pure function of `actual` + `spec`. The `stable_announced` set
records only "did we already emit the deciding action?" — a
publication-side invariant, not a derived state cache.

`AnyState` and `AnyReconcilerView` gain new variants (additive
per `overdrive-core::reconcilers::mod`):

```rust
pub enum AnyState {
    Unit, WorkloadLifecycle(...), ServiceMapHydrator(...), BackendDiscoveryBridge(...),
    ServiceLifecycle(ServiceLifecycleState),  // NEW
}

pub enum AnyReconcilerView {
    Unit, WorkloadLifecycle(...), ServiceMapHydrator(...), BackendDiscoveryBridge(...),
    ServiceLifecycle(ServiceLifecycleView),  // NEW
}

pub enum AnyReconciler {
    NoopHeartbeat(...), WorkloadLifecycle(...), ServiceMapHydrator(...),
    BackendDiscoveryBridge(...),
    ServiceLifecycle(ServiceLifecycle),  // NEW
}
```

### 3. `reconcile` body — pure decision tree

```rust
impl Reconciler for ServiceLifecycle {
    const NAME: &'static str = "service-lifecycle";
    type State = ServiceLifecycleState;
    type View  = ServiceLifecycleView;

    fn reconcile(
        &self,
        desired: &ServiceLifecycleState,
        actual:  &ServiceLifecycleState,
        view:    &ServiceLifecycleView,
        tick:    &TickContext,
    ) -> (Vec<Action>, ServiceLifecycleView) {
        let mut actions = Vec::new();
        let mut next = view.clone();

        let Some(spec) = desired.spec.as_ref() else {
            return (actions, next); // Service deleted; cleanup by GC reconciler
        };

        for (alloc_id, alloc_row) in &actual.allocations {
            decide_per_alloc(
                spec, alloc_row,
                actual.probe_results.get(alloc_id).unwrap_or(&BTreeMap::new()),
                view, &mut next, &mut actions, tick,
            );
        }

        (actions, next)
    }
}
```

`decide_per_alloc` follows the per-role priority order:

1. **Terminal check first.** If `alloc_row.state == Failed` AND
   `view.stable_announced.contains(alloc_id) == false` AND
   `tick.now_unix - alloc_row.started_at < startup_deadline`:
   emit `Action::SetTerminalCondition(Failed { reason: EarlyExit { ... } })`
   — closes US-08.
2. **Startup gate.** If `!view.stable_announced.contains(alloc_id)`:
   - For each `probe in spec.startup_probes`: read
     `actual.probe_results[alloc_id][probe.idx]`.
   - If ALL probes have `status == Pass` (AND-semantics per P2-Q7):
     emit `Action::SetTerminalCondition(Stable { settled_in:
     tick.now_unix - alloc_row.started_at, witness:
     last_passing_probe })`; insert `alloc_id` into
     `next.stable_announced`.
   - Else if `tick.now_unix - alloc_row.started_at >= startup_deadline`:
     emit `Action::SetTerminalCondition(Failed { reason:
     StartupProbeFailed { probe_idx, attempts, last_fail } })`.
   - Else: no startup-related action; await more probe results.
3. **Readiness branch** (only when `stable_announced`):
   - For each `probe in spec.readiness_probes`: read
     `actual.probe_results[alloc_id][probe.idx]`.
   - If Pass AND `view.readiness_consecutive_successes[alloc][probe]
     + 1 >= spec.success_threshold`: increment counter; current
     backend healthy. Else if Fail: reset counter to 0; backend
     unhealthy.
   - Emit `Action::WriteServiceBackendRow { row: row.with_healthy(...)
     }` IFF the healthy flag differs from `actual.service_backends`.
4. **Liveness branch** (only when `stable_announced`):
   - For each `probe in spec.liveness_probes`: read result.
   - If Pass: reset `next.liveness_consecutive_failures[alloc][probe]
     = 0`.
   - If Fail: increment. If counter `>= spec.failure_threshold`:
     emit `Action::RestartAllocation { alloc_id, kind:
     WorkloadKind::Service, reason:
     RestartReason::LivenessExhausted { ... } }`. **Critical**:
     per P2-Q9, the restart `Action` is emitted unconditionally;
     a Phase 2+ rate-limiter slots in as a new reconciler that
     consumes RestartAllocation actions and emits filtered
     downstream actions. This ADR does not implement the
     rate-limiter; it makes its addition non-breaking.

The function is < 200 LOC, pure, sync, no `.await`, no I/O.

### 4. `Stable` as non-terminal condition — extension to ADR-0037

ADR-0037's `TerminalCondition` is defined as the reconciler's claim
that "no further convergence work will be attempted." For Service
kind, **`Stable` is announced once but does not foreclose further
work** — readiness, liveness, and restarts continue.

This ADR extends `TerminalCondition` with a non-terminal variant:

```rust
// Amendment to TerminalCondition (per ADR-0037 §5 SemVer convention:
// new variants are additive minor).
pub enum TerminalCondition {
    // ... existing variants ...

    /// SERVICE-KIND ONLY. Reconciler's announcement that the Service
    /// has reached operator-meaningful liveness (all startup probes
    /// passing). Unlike other variants, `Stable` is NON-TERMINAL:
    /// the reconciler continues to process readiness, liveness, and
    /// restart for the alloc after emission.
    ///
    /// The action shim writes this once on the deciding tick;
    /// subsequent ticks do NOT re-emit (gated by
    /// `View::stable_announced`).
    Stable {
        settled_in: Duration,
        witness: ProbeWitness,
    },

    /// SERVICE-KIND ONLY. Reconciler's claim that the Service
    /// failed to reach Stable within startup_deadline OR exited
    /// before any startup probe could pass.
    Failed {
        reason: ServiceFailureReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, /* rkyv + serde */)]
#[non_exhaustive]
pub enum ServiceFailureReason {
    StartupProbeFailed {
        probe_idx: ProbeIdx,
        attempts: u32,
        last_fail: ProbeFailure,
        elapsed: Duration,
        startup_deadline: Duration,
    },
    EarlyExit {
        exit_code: i32,
        elapsed: Duration,
        startup_deadline: Duration,
        stderr_tail: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, /* rkyv + serde */)]
pub struct ProbeWitness {
    pub probe_idx: ProbeIdx,
    pub role: ProbeRole,
    pub mechanic_summary: String, // "tcp 0.0.0.0:8080" | "http GET http://..."
    pub inferred: bool,            // true iff Slice 01 default probe
}
```

The "non-terminal" semantics are encoded structurally via the
`View::stable_announced` set, not via a flag on `TerminalCondition`
itself. From the action shim's perspective, every emission is a
write; the reconciler's deduplication via the View IS the gate.

This means ADR-0037 §1 layering ("reconciler decides terminal-or-not
from inputs in scope; streaming forwards without re-deriving")
**is preserved verbatim** — the streaming consumer cannot tell
`Stable` apart from `BackoffExhausted` structurally; both flow
through `LifecycleEvent.terminal: Some(...)`. The reconciler-level
distinction (Service continues to process probes; Job stops) lives
in the reconciler body, not in `TerminalCondition`.

### 5. AND-of-all for multi-startup-probe Stable (P2-Q7 resolution)

When the Service spec declares ≥2 `[[health_check.startup]]` probes,
the `Stable` predicate is **AND-of-all** (every startup probe must
have `status == Pass`). Rationale:

- Each declared probe represents an operator-stated invariant
  ("listener bound AND `/healthz` returns 2xx AND warmup script
  exits 0"). OR-semantics would mean any single probe can satisfy
  the invariant, defeating the operator's intent.
- Aligns with Kubernetes' implicit AND-semantic (K8s requires all
  containers' `startupProbe` to pass; with N startup probes within
  a single container Phase 1 sees no precedent — this is a Phase 1
  extension).
- The `witness` field names the LAST probe to cross its threshold
  (the one whose Pass closed the AND-gate). This is the probe whose
  result tick triggered the deciding evaluation; named explicitly
  for operator diagnosis.

OR-semantics is reserved for a future operator-configurable knob
(e.g. `[health_check].startup_combinator = "any" | "all"` with
default `"all"`); out of scope for Phase 1.

### 6. `successThreshold` for readiness (P2-Q8 resolution)

```toml
[[health_check.readiness]]
type = "http"
path = "/healthz"
port = 8080
success_threshold = 1   # default; configurable up to N
failure_threshold = 1   # readiness-only; default 1
```

`successThreshold` default = 1 matches Kubernetes default (research
D1, § 5.1). Operators configure higher values when their `/healthz`
endpoint is known to flap (e.g. a slow background warmer). The
counter lives in `View::readiness_consecutive_successes` per
P2-Q8 acceptance criterion in `feature-delta.md`.

Per `.claude/rules/development.md` § "Persist inputs, not derived
state": the counter (input) is persisted; the gate decision (`backend
healthy` boolean) is recomputed every tick against the live
`success_threshold` from the spec. A future change to the threshold
takes effect on the next tick without migrating any persisted state.

### 7. Cascading-restart rate-limiter (P2-Q9 resolution — Phase 2+ surface)

Phase 1 is single-node single-replica per
`feedback_phase1_single_node_scope.md`; cascading-restart risk does
not manifest. **The architecture is shaped to make Phase 2+
rate-limiting non-breaking**:

- `Action::RestartAllocation` is emitted unconditionally by the
  `ServiceLifecycleReconciler::reconcile` body.
- A future Phase 2+ `LivenessRestartGovernor` reconciler reads
  `Action::RestartAllocation` from a queue (or from the
  ObservationStore once actions are persisted), filters by per-Service
  budget, and re-emits filtered actions onto the action-shim queue.
- Phase 1 ships **no governor**; the existing
  `RESTART_BACKOFF_CEILING` per-alloc budget IS the budget Phase 1
  honours. Multi-replica cross-alloc throttling is the deferred
  surface.

No `gh issue create` required at this design site: the architecture
allows the future addition; the user is not promised it. If
operators experience cross-replica restart storms in Phase 2+, the
governor is added then with its own ADR.

### 8. Earned Trust — reconciler has no port deps, but the runtime probes its ViewStore

`ServiceLifecycleReconciler` per ADR-0035 / ADR-0036 has no port
dependencies (it is pure). The runtime's `ViewStore::probe()` (per
ADR-0035 §"Boot / register") covers ServiceLifecycle's typed View
persistence path; no new probe surface is introduced by this ADR.

## Considered alternatives

### Alternative A — Extend `WorkloadLifecycle` reconciler in-place

Keep one reconciler, branch on `kind()` inside `reconcile`. Rejected
because the `View` shapes are disjoint (Job: `restart_counts`,
`last_failure_seen_at`; Service: `consecutive_failures_per_probe`,
`stable_announced`, etc.) and shared-struct-with-optional-fields
violates `development.md` § "Sum types over sentinels".

### Alternative B — `Stable` as a separate `Condition` (Kubernetes-shape)

Introduce a second enum `Condition` distinct from `TerminalCondition`
for non-terminal-but-published claims. Rejected: the action shim is
already plumbed for `TerminalCondition` writes to row + broadcast
(ADR-0037 §4). A parallel `Condition` enum doubles the publication
surface for one new variant; the `View::stable_announced` set
provides the deduplication structurally without a second pathway.

### Alternative C — OR-semantic for multi-startup-probe Stable

Allow operators to declare 2 startup probes where ANY pass = Stable.
Rejected for P2-Q7 above (defeats operator intent). The combinator
knob is reserved for a future iteration.

### Alternative D — Implement rate-limiter in Phase 1

Land the `LivenessRestartGovernor` reconciler now. Rejected: Phase 1
has no cascading surface (single-replica). Premature design surface
without a real use case.

## Consequences

### Positive

- **Service-kind logic lives in its own reconciler** with disjoint
  `State` / `View` shapes; Job-kind logic in `WorkloadLifecycle` is
  unchanged.
- **`Stable` non-terminal semantics encoded structurally** via
  `View::stable_announced`; ADR-0037's layering rule is preserved.
- **`Stable` is recomputed every tick** from observation inputs; no
  derived state persisted.
- **AND-semantic for multi-probe startup** matches operator
  intent; future OR knob is non-breaking.
- **Liveness rate-limiter is non-blocking architecture**; Phase 2+
  governor slots in cleanly.

### Negative

- **One new reconciler + new AnyState / AnyReconcilerView variants**
  to maintain. Each AnyReconciler match arm adds ~5 LOC; bounded.
- **`ServiceLifecycleView` carries five maps** (liveness counters,
  readiness counters, stable-announced set, startup-attempt
  counters). Memory cost: O(allocs × probes) per node; for Phase 1
  single-node single-replica + 3 probes = ~100 B. Bounded.
- **TerminalCondition gains 2 variants** (`Stable`, `Failed`); per
  ADR-0037 §5 additive minor SemVer; existing fixtures unaffected
  (Service kind is greenfield at this ADR).

### Quality-attribute impact

| Attribute | Impact |
|---|---|
| Maintainability — modifiability | Service vs Job branches independently evolvable; no shared optional fields |
| Maintainability — testability | Pure sync reconcile; property-test invariants on (probe results × view) → actions |
| Reliability — surface coherence | `Stable` deduplication via View; no double-emission |
| Functional correctness — operator intent | AND-of-all startup probes matches declared invariants |
| Compatibility — evolvability | Future governor non-breaking; future combinator knob non-breaking |

## Cross-references

- ADR-0037 — TerminalCondition; this ADR adds `Stable`, `Failed`
  variants
- ADR-0035 / ADR-0036 — Reconciler runtime + AnyState; this ADR adds
  variants
- ADR-0047 — workload kind discriminator; Service-kind branch
- ADR-0050 — ServiceSpec intent aggregate; consumed as `desired`
- ADR-0054 — ProbeRunner; produces `ProbeResultRow` consumed here
- ADR-0056 — per-kind streaming; `Stable` / `Failed` cross
  `ServiceSubmitEvent` boundary via action shim
- ADR-0057 — `[[health_check.*]]` TOML; declares `failure_threshold`,
  `success_threshold` consumed here
- `feature-delta.md` P1-Q3, P2-Q7, P2-Q8, P2-Q9
- `.claude/rules/development.md` § "Reconciler I/O", § "Persist
  inputs, not derived state", § "Sum types over sentinels", §
  "Ordered-collection choice"

## Changelog

- 2026-05-24 — Initial accepted version. Resolves P1-Q3 (in part),
  P2-Q7, P2-Q8, P2-Q9 from
  `docs/feature/service-health-check-probes/feature-delta.md`.
