# ADR-0055 — `ServiceLifecycleReconciler`: typed View, pure `reconcile`, `Stable` as non-terminal condition extending ADR-0037

## Status

Accepted. 2026-05-24. Decision-makers: Morgan (proposing); DESIGN-wave
output of `docs/feature/service-health-check-probes/`.

**Amended 2026-08-31 (TRC-ARCH-003).** Liveness state is fenced by the
accepted Running row's logical `updated_at`, not wall-clock `started_at`.
Service View persists that existing identity before dispatch, and actual
hydration accepts only a V2 probe row bearing the exact same attempt.

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
// crates/overdrive-core/src/service_lifecycle.rs
// (path corrected 2026-08-02 — the module landed at the crate root,
//  not under `reconcilers/` as originally sketched)

/// `desired`/`actual` projection for the Service-kind reconciler.
/// Shape corrected 2026-08-02 to the implemented fact-bundle — see the
/// amendment immediately below this block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceLifecycleState {
    /// Per-alloc PRE-JOINED fact bundle: the alloc-status projection
    /// (`state` / `started_at` / `exit_code`), the LWW probe
    /// projection per role, and the spec-derived policy inputs for
    /// that alloc, all flattened onto one struct by the runtime's
    /// hydrate pass.
    pub allocs: BTreeMap<AllocationId, ServiceAllocFact>,

    /// Service-level dataplane identity (`service_id` / `vip` /
    /// `writer`) the readiness branch composes its `ServiceBackendRow`
    /// from. One-per-Service, so it does not live on the per-alloc
    /// fact. `None` when the Service has no VIP yet.
    pub service_dataplane: Option<ServiceDataplaneIdentity>,

    /// LWW stamp of the currently-stored `service_backends` row, or
    /// `None` when no row exists. Observed input for the readiness
    /// branch's write; never persisted in the View.
    pub prior_backend_row_at: Option<LogicalTimestamp>,
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
        BTreeMap<(AllocationId, ProbeIdx), u32>,

    /// Per-alloc readiness probe consecutive-success counters.
    /// INPUT (consumed by the readiness `successThreshold` gate per
    /// P2-Q8). The flap-protection predicate is recomputed from this
    /// counter plus the live `success_threshold`.
    pub readiness_consecutive_successes:
        BTreeMap<(AllocationId, ProbeIdx), u32>,

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
    pub startup_attempts_per_alloc: BTreeMap<AllocationId, u32>,
}
```

**Amendment 2026-08-02 — `ServiceLifecycleState` corrected to the
implemented fact-bundle projection. This is a STRUCTURAL correction,
not an accuracy annotation — the implementation deliberately built a
different `State` shape from the one sketched above, and that
divergence was until now recorded nowhere.** The decision this ADR
records is unaffected (see "What does NOT change" below).

The sketch above previously declared four parallel maps holding raw
observation rows:

```rust
pub spec:            Option<ServiceSpec>,
pub allocations:     BTreeMap<AllocationId, AllocStatusRow>,
pub probe_results:   BTreeMap<AllocationId, BTreeMap<ProbeIdx, ProbeResultRow>>,
pub service_backends: BTreeMap<AllocationId, ServiceBackendRow>,
```

None of those four fields exists. The implemented struct
(`crates/overdrive-core/src/service_lifecycle.rs:231-254`) carries
`allocs` / `service_dataplane` / `prior_backend_row_at` as shown. The
projection was inverted: instead of the reconciler joining four
row-keyed maps at decision time, **the runtime's hydrate pass
pre-joins everything into one `ServiceAllocFact` per alloc**
(`:70-216`) and the reconciler iterates `actual.allocs` once
(`:500`, `:697`, `:843`). The four sketched fields map across as:

| Sketched field | Implemented replacement |
|---|---|
| `spec: Option<ServiceSpec>` | Spec-derived policy flattened onto each fact: `max_attempts`, `startup_deadline`, `mechanic_summary`, `inferred`, `startup_probes_empty` (`:118-137`), `has_readiness_probe`, `readiness_success_threshold` (`:156`, `:162`), `has_liveness_probe`, `liveness_failure_threshold`, `restart_spec` (`:187`, `:193`, `:215`) |
| `allocations: …<AllocStatusRow>` | `ServiceAllocFact::{state, started_at, exit_code}` (`:74`, `:108`, `:111`) |
| `probe_results: …<BTreeMap<ProbeIdx, ProbeResultRow>>` | Three flat `Option<ProbeStatus>` fields — `latest_startup_probe` (`:114`), `latest_readiness_probe` (`:150`), `latest_liveness_probe` (`:181`) |
| `service_backends: …<ServiceBackendRow>` | `ServiceLifecycleState::{service_dataplane, prior_backend_row_at}` (`:247`, `:253`) |

**The probe row-set became a single status per role.** This is the
consequential half of the correction and it *strengthens* the
multi-probe gap recorded in the two amendments below (§ 3, § 5).
Those amendments locate the gap in the projection logic — the
hydrate filter pins `ProbeIdx::new(0)`
(`crates/overdrive-control-plane/src/reconciler_runtime.rs:3052-3056`,
`:3061-3065`, `:3069-3073`) and the spec side reads
`startup_probes[0]` / `.first()` (`:1963`, `:1981`, `:1996`). The
`State` shape shows the gap is **structural in the type**: a
`BTreeMap<ProbeIdx, …>` could hold probes `1..N`; an
`Option<ProbeStatus>` has nowhere to put them. Widening the hydrate
filter alone would therefore NOT close § 5 — closing it requires
changing `ServiceAllocFact` back to a per-role collection. That is a
larger change than the two amendments below imply, and it is recorded
here so the next reader costs it correctly.

**The unused argument is `desired`, not `actual`.** The old sketch's
first field said "`actual`: empty placeholder; not used". The
implementation inverted this: `reconcile` binds `_desired`
(`crates/overdrive-core/src/service_lifecycle.rs:491`) and reads
`actual` exclusively. Both projections are populated by the runtime
(desired at `crates/overdrive-control-plane/src/reconciler_runtime.rs:1880`,
actual at `:2963`), but every decision path reads `actual` because
the spec-derived policy inputs are flattened onto the actual-side
facts.

**What does NOT change.** Every decision in this ADR stands: § 1
(ServiceLifecycle is its own typed reconciler with disjoint
`State`/`View`), § 4 (`Stable` non-terminal, deduplicated
structurally via `View::stable_announced`), § 5 (AND-of-all —
specified, not implemented, per its own amendment), § 6
(`successThreshold` recomputed from the live spec), § 7 (no Phase 1
governor), § 8 (no port dependencies). The persist-inputs discipline
this section argues for is likewise preserved and is in fact carried
further than sketched: `ServiceAllocFact` re-derives every spec-side
threshold each tick rather than reading a persisted spec snapshot.

**Also corrected in the `View` sketch above:** the two per-probe
counters are flat tuple-keyed maps —
`BTreeMap<(AllocationId, ProbeIdx), u32>` (`:297`, `:303`) — not the
nested `BTreeMap<AllocationId, BTreeMap<ProbeIdx, u32>>` previously
shown. The keying is semantically the same per-`(alloc, probe_idx)`
pair the amendment below describes; only the container nesting
differed. Note further that the `View` sketch above is **abridged**:
the implemented struct carries four fields the sketch omits —
`startup_last_fail_seen_at` (`:316`), `observed` (`:338`),
`terminal_announced` (`:368`), and
`last_emitted_backend_fingerprint` (`:387`). The first three are
GAP-9/GAP-10 additions; the fourth is the emit-time fingerprint
ADR-0079 § D4 deliberately excludes from its convergence fix. They
are named here rather than folded into the sketch because each was
introduced by a decision recorded elsewhere, not by this ADR.

**Amendment 2026-08-02 — fourth field's name and key shape corrected.**
The field is `startup_attempts_per_alloc`, keyed
`BTreeMap<AllocationId, u32>` — ONE counter per alloc, not one per
`(alloc, probe_idx)`. Verified at
`crates/overdrive-core/src/service_lifecycle.rs:292`; the writer
`update_startup_attempts` takes `&mut BTreeMap<AllocationId, u32>` plus
an `alloc_id` with no `ProbeIdx` in scope (`:939-954`), and the
`StartupProbeFailed` gate reads it per alloc (`:650`). The sketch above
previously spelled the field `startup_attempts_per_probe:
BTreeMap<AllocationId, BTreeMap<ProbeIdx, u32>>`, which contradicted its
own "Per-alloc startup attempt counter" docstring and read as the
per-`(alloc, probe_idx)` keying that `liveness_consecutive_failures` and
`readiness_consecutive_successes` genuinely carry (`:297`, `:303`). That
distinction is live under ADR-0080 § D1 (`ProbeIdx` is per-role), so the
wrong name misdescribed the data model rather than merely mislabelling a
field. The decision is unchanged: `attempts` is the per-alloc
CONSECUTIVE startup-probe-failure streak per ADR-0057 §2 — incremented
on Fail, reset to 0 on Pass, untouched when no probe was observed this
tick. Note that `probe_idx` on the `StartupProbeFailed` payload (§ 3
step 2, § 4) is a distinct reporting field, not a key of this counter.

`Stable` IS NOT persisted as a derived field. The "Stable predicate"
is recomputed every tick. As specified by § 5 (AND-of-all), the
predicate is:

```
is_stable(alloc) =                                   // SPECIFIED
    spec.startup_probes.iter().all(|probe|
        actual.probe_results[alloc][probe.idx].status == Pass)
```

**As implemented it is a disjunction of two single-probe branches,
not a conjunction over declared probes:**

```
is_stable(alloc) =                                   // IMPLEMENTED
       (fact.startup_probes_empty && fact.state == Running)   // opt-out
    || (fact.state == Running
        && fact.latest_startup_probe == Some(Pass))           // single probe
```

— `crates/overdrive-core/src/service_lifecycle.rs:557` (the ADR-0058
§4 / ADR-0059 Q5 empty-probes opt-out) and `:580-581`, whose own
comment reads "Running + **any** startup probe Pass". The AND-of-all
decision of § 5 stands and is unchanged; its non-implementation is
recorded in § 5's own amendment and, structurally, in the `State`
amendment above — `latest_startup_probe` is one `Option<ProbeStatus>`,
so no conjunction over ≥2 probes is expressible at this call site.

The load-bearing claim of this passage survives both corrections
intact: the predicate remains a **pure function of inputs recomputed
every tick, never a persisted derived field**. Only its arity and its
argument source change — it is a pure function of `actual` + `tick`,
because the spec-side inputs it would otherwise read from `desired`
are flattened onto `actual.allocs[alloc]` by the hydrate pass (see
the `State` amendment above). The `stable_announced` set continues to
record only "did we already emit the deciding action?" — a
publication-side invariant, not a derived state cache
(`:501-506`, `:597`).

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
// Body shape corrected 2026-08-02 to the implemented iteration —
// see the `State` amendment in § 2.
impl Reconciler for ServiceLifecycleReconciler {
    const NAME: &'static str = "service-lifecycle";
    type State = ServiceLifecycleState;
    type View  = ServiceLifecycleView;

    fn reconcile(
        &self,
        _desired: &Self::State,   // unused — spec inputs ride on `actual`
        actual:   &Self::State,
        view:     &Self::View,
        tick:     &TickContext,
    ) -> (Vec<Action>, Self::View) {
        let mut actions = Vec::new();
        let mut next = view.clone();

        // No `desired.spec` guard: a Service whose intent is absent
        // hydrates to `allocs: BTreeMap::new()`, so the loop is empty
        // and the tick is a no-op by construction.
        for (alloc_id, fact) in &actual.allocs {
            // dedup guards, then the per-alloc branch ladder
            // (opt-out Stable / Stable / EarlyExit / StartupProbeFailed);
            // readiness + liveness run in their own passes over the
            // same `actual.allocs` map.
        }

        (actions, next)
    }
}
```

The reconciler struct is `ServiceLifecycleReconciler`
(`crates/overdrive-core/src/service_lifecycle.rs:480`); the
`AnyReconciler` / `AnyState` / `AnyReconcilerView` variants are all
spelled `ServiceLifecycle(…)` and wrap it
(`crates/overdrive-core/src/reconcilers/mod.rs:812`, `:354`, `:947`).
The empty-intent case is handled on the hydrate side rather than by a
guard in the body — `hydrate_actual` returns
`ServiceLifecycleState { allocs: BTreeMap::new(), .. }` when the
Service's intent is absent
(`crates/overdrive-control-plane/src/reconciler_runtime.rs:2915-2919`).

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

**Amendment 2026-08-02 — the per-probe iteration in steps 2–4 is
specified but not implemented.** Those steps describe reading a result
for each declared probe (`for each probe in spec.startup_probes` /
`readiness_probes` / `liveness_probes`), indexed as
`actual.probe_results[alloc_id][probe.idx]` — a field that does not
exist; see the `State` amendment in § 2 for the shape that shipped in
its place. The implemented hydrate path consults exactly ONE row per
role: the three `latest_probe_status` projections in
`hydrate_service_alloc_facts` filter on `(role, ProbeIdx::new(0))` and
reduce with `max_by_key` on `last_observed_at_unix_ms` — startup at
`crates/overdrive-control-plane/src/reconciler_runtime.rs:3052-3056`,
readiness at `:3061-3065`, liveness at `:3069-3073`. The spec-side
threshold projections are single-probe for the same reason:
`spec_facts_for_service` reads `svc.startup_probes[0]` (`:1963`),
`readiness_facts_for_service` reads `.first()` (`:1981`), and
`liveness_facts_for_service` reads `.first()` (`:1996`).

Probes `1..N` of every role are nonetheless spawned and do write
durable rows — `project_probe_descriptors`
(`crates/overdrive-core/src/reconcilers/workload_lifecycle.rs:1253-1269`)
concatenates all three vectors in full, and `ProbeRunner::start_alloc`
spawns one task per descriptor via `iter().enumerate()`
(`crates/overdrive-worker/src/probe_runner/mod.rs:323`), each writing a
`ProbeResultRow` per tick (`:543`, `:551`; adapter-failure rows at
`:520`, `:528`). No decision path consults those rows.

The decision recorded above is unchanged; only its implementation
status is corrected here. ADR-0080 § "A fourth, pre-existing gap this
ADR deliberately does NOT address" is where the decision not to close
this gap as part of that ADR's scope is recorded. ADR-0080 is accepted
(2026-08-02); its **Stage 1 (D1–D4) is implemented**, making `ProbeIdx`
per-role and parser-assigned and adding `role` to the durable composite
key, so per-role probes `1..N` are stored distinctly rather than
sharing one key space across roles — the `(role, probe_idx)` filter at
`:3052-3056` / `:3061-3065` / `:3069-3073` is that Stage-1 shape.
**Stage 2 (D5 — the bridge taking sole ownership of
`ServiceBackendRow`) is NOT implemented.** Neither stage closes the
multi-probe gap: Stage 1 makes probes `1..N` addressable in the store
without making them readable by the reconciler, which per the `State`
amendment in § 2 has no field to receive them.

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

**Amendment 2026-08-02 — AND-of-all is specified but not implemented.**
No implemented decision path evaluates a conjunction over ≥2 startup
probes. The startup projection consults a single row, the one at
`probe_idx == ProbeIdx::new(0)`
(`crates/overdrive-control-plane/src/reconciler_runtime.rs:3052-3056`),
and `spec_facts_for_service` derives the startup thresholds from
`svc.startup_probes[0]` alone (`:1963`). A second declared startup
probe is spawned
(`crates/overdrive-worker/src/probe_runner/mod.rs:323`) and writes
durable rows (`:543`, `:551`) that no `Stable` predicate reads, so the
`witness` rule in the third bullet above — "the LAST probe to cross its
threshold" — has no implemented counterpart either.

**The gap is structural, not merely a projection choice.** The
implemented `Stable` branch is a single-`Option` test, not a
conjunction that happens to be evaluated over one element:
`fact.state == Running && matches!(fact.latest_startup_probe,
Some(ProbeStatus::Pass))`
(`crates/overdrive-core/src/service_lifecycle.rs:580-581`), whose own
comment reads "Running + **any** startup probe Pass". Because
`ServiceAllocFact` carries `latest_startup_probe: Option<ProbeStatus>`
(`:114`) rather than a per-probe collection, closing this section
requires **changing the `State` type**, not just widening the hydrate
filter — see the `State` amendment in § 2.

The AND-of-all decision itself is unchanged; only its implementation
status is corrected here. ADR-0080 § "A fourth, pre-existing gap this
ADR deliberately does NOT address" is where the decision not to close
this gap as part of that ADR's scope is recorded, and it names
ratifying the readiness and liveness combinators this section leaves
specified for startup only as part of what closing it would require.

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

### 7a. Logical attempt fence for same-id liveness restart

TRC-ARCH-003 extends the implemented actual fact with exactly
`status_updated_at: LogicalTimestamp`, copied from the current
`AllocStatusRow.updated_at`, and extends the existing View with exactly:

```rust
#[serde(default)]
pub liveness_attempt: BTreeMap<AllocationId, LogicalTimestamp>,
```

`LogicalTimestamp` gains serde derives so the existing ViewStore persists the
input. On every Running fact, a missing/different marker clears that alloc's
liveness counter and stores `status_updated_at` in `next_view` before action
dispatch; an equal marker keeps ordinary counter maintenance. Non-Running
terminal/route repair retains marker+counter, while exact-unrouted or a
different terminal clears both.

Actual hydration supplies `latest_liveness_probe` only when
`ProbeResultRowV2.alloc_attempt.as_ref() == Some(&fact.status_updated_at)`.
Legacy `None`, older attempts, and newer non-matching attempts are all absent
for this decision. `started_at` and `last_observed_at_unix_ms` are not attempt
identity. ADR-0048/0054 own V2 and its attempt-first latest-row LWW, which lets
a lower/equal-wall-time first probe from the new logical attempt displace the
old row.

The counter update persists before any action exactly as every View diff does.
A matching Fail already visible on the first tick counts from one after reset;
no matching row emits no liveness Stop. This preserves threshold-1 semantics,
non-Running replay, the sole WorkloadLifecycle restart budget, and the pure
reconciler boundary without a receipt or cross-View read.

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
- **`ServiceLifecycleView` carries per-alloc counter maps and dedup
  sets** (liveness counters, readiness counters, stable-announced set,
  startup-attempt counters). Memory cost: O(allocs × probes) per node;
  for Phase 1 single-node single-replica + 3 probes = ~100 B. Bounded.
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

- 2026-08-31 — **Amendment (TRC-ARCH-003)** — liveness attempt isolation
  uses the accepted Running row's logical `updated_at`. The exact actual-fact
  field and serde-defaulted View map reset historical counters before action
  dispatch; hydration accepts only an exact-attempt ProbeResultRow V2. Wall
  clock is no longer an attribution input.
- 2026-05-24 — Initial accepted version. Resolves P1-Q3 (in part),
  P2-Q7, P2-Q8, P2-Q9 from
  `docs/feature/service-health-check-probes/feature-delta.md`.
- 2026-08-02 — **Amendment** — accuracy annotation only; no decision
  changed. Marked the multi-probe-per-role behaviour in § 3 (steps 2–4,
  the per-probe iteration) and § 5 (AND-of-all for multi-startup-probe
  `Stable`) as **specified but not implemented**. Every implemented
  consumer reads per-role index 0 only
  (`crates/overdrive-control-plane/src/reconciler_runtime.rs:3023-3030`,
  `:3035-3042`, `:3046-3053`, and `:1963` / `:1981` / `:1996`), while
  probes `1..N` are spawned and write durable rows
  (`crates/overdrive-worker/src/probe_runner/mod.rs:323`, `:543`) that
  nothing consults. ADR-0080 § "A fourth, pre-existing gap this ADR
  deliberately does NOT address" records the decision not to close the
  gap within that ADR's scope.
- 2026-08-02 — **Amendment** — accuracy correction only; no decision
  changed. Corrected the fourth `ServiceLifecycleView` field in § 2 from
  `startup_attempts_per_probe: BTreeMap<AllocationId, BTreeMap<ProbeIdx,
  u32>>` to the implemented `startup_attempts_per_alloc:
  BTreeMap<AllocationId, u32>`
  (`crates/overdrive-core/src/service_lifecycle.rs:292`; writer
  `update_startup_attempts` at `:939-954`, read at `:650`). The old name
  inverted the key shape — it read as the per-`(alloc, probe_idx)`
  keying that `liveness_consecutive_failures` and
  `readiness_consecutive_successes` genuinely carry (`:297`, `:303`) —
  and contradicted the field's own "Per-alloc" docstring, so it
  misdescribed the data model rather than merely mislabelling a field.
  This ADR was the normative origin of the wrong name: `brief.md`
  carried it (corrected 2026-08-02) while ADR-0080 § D5 already cited
  the correct one, leaving two accepted ADRs in contradiction until now.
- 2026-08-02 — **Amendment** — **structural correction**; no decision
  changed. Unlike the two entries above (which corrected a field name
  and a stale status), this one replaces a whole type: § 2's
  `ServiceLifecycleState` sketch declared four parallel row-keyed maps
  (`spec`, `allocations`, `probe_results`, `service_backends`), none of
  which exists. The implementation deliberately built a **pre-joined
  per-alloc fact bundle** — `allocs: BTreeMap<AllocationId,
  ServiceAllocFact>` plus `service_dataplane` and
  `prior_backend_row_at`
  (`crates/overdrive-core/src/service_lifecycle.rs:231-254`;
  `ServiceAllocFact` at `:70-216`) — and the probe row-set collapsed to
  three flat `Option<ProbeStatus>` fields (`:114`, `:150`, `:181`).
  Corrected in § 2 (the `State` sketch, the two `View` counter key
  types, and the `is_stable` pseudocode) and § 3 (the `reconcile` body
  sketch), each annotated with the implemented shape. Three
  consequences are recorded rather than silently reinterpreted: (a) the
  multi-probe gap already noted in § 3 and § 5 is **structural in the
  `State` type**, not merely a hydrate-filter choice, so closing § 5
  costs a type change; (b) the unused `reconcile` argument is
  `_desired`, not `actual` as the old sketch claimed (`:491`) — the
  spec-derived policy inputs ride on the actual-side facts; (c) the
  `View` sketch is abridged, omitting four implemented fields
  (`startup_last_fail_seen_at`, `observed`, `terminal_announced`,
  `last_emitted_backend_fingerprint`) introduced by decisions recorded
  elsewhere. Also refreshed in this pass: the hydrate-projection line
  citations in the § 3 and § 5 amendments, stale after commits
  `3468ccda` / `75a62400` (`:3023-3030` / `:3035-3042` / `:3046-3053` →
  `:3052-3056` / `:3061-3065` / `:3069-3073`), and the § 3 amendment's
  claim that ADR-0080 is "not yet implemented" — its Stage 1 (D1–D4) is
  implemented; Stage 2 (D5) is not.
- 2026-08-02 — **Amendment** — accuracy correction only; no decision
  changed. Dropped the field count from the Consequences → Negative
  entry that read "**`ServiceLifecycleView` carries five maps**". The
  implemented View at that amendment cut had **eight** fields
  (`crates/overdrive-core/src/service_lifecycle.rs:289-388`): five
  `BTreeMap`s (`startup_attempts_per_alloc`,
  `liveness_consecutive_failures`, `readiness_consecutive_successes`,
  `startup_last_fail_seen_at`, `last_emitted_backend_fingerprint`) and
  three `BTreeSet`s (`stable_announced`, `terminal_announced`,
  `observed`). "Five" was *accidentally* defensible — it is the exact
  count of the `BTreeMap`s, the rest being `BTreeSet`s — while reading
  as a field count that is off by three. A number that resists
  correction is worse than one plainly wrong, so the count was dropped
  rather than restated; the parenthetical that follows it was already
  illustrative rather than exhaustive, and the O(allocs × probes)
  sizing argument it supports is unaffected. That eight-field inventory was
  carried in `brief.md` § 77. TRC-ARCH-003 later appends
  `liveness_attempt` as the sixth map and ninth field; § 7a and the current
  brief inventory supersede that historical count.
