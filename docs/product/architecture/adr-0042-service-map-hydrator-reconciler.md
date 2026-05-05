# ADR-0042 — `ServiceMapHydrator` reconciler + `Action::DataplaneUpdateService` + `service_hydration_results` observation table

## Status

Accepted. 2026-05-05. Decision-makers: Morgan (proposing); user
ratified `lgtm` against
`docs/feature/phase-2-xdp-service-map/design/proposal-draft.md`
(2026-05-05). Tags: phase-2, reconciler, dataplane-port,
observation-store, action-shim, j-plat-004.

**Companion ADRs**: ADR-0040 (three-map split + HASH_OF_MAPS atomic-
swap primitive), ADR-0041 (weighted Maglev + REVERSE_NAT shape +
endianness lockstep).

## Context

`docs/product/jobs.yaml`'s `J-PLAT-004` (reconciler convergence)
flips from `status: deferred` to `status: active` with this feature.
The `Reconciler` trait (ADR-0035, collapsed shape) and the
`AnyState` enum (ADR-0021 amended by ADR-0036) are mature; what is
missing is the **first non-trivial reconciler against a real
(non-Sim) Dataplane port body** — the closing primitive that makes
the dataplane work observable from the control-plane side.

Three concrete questions are settled together:

1. **What Action variant does the hydrator emit?** A new typed
   variant, a generic `DataplaneCall { op: DataplaneOp }`, or a
   reuse of `HttpCall`?

2. **What ObservationStore surface does the hydrator's `actual`
   projection read?** Derive `actual` from the last-emitted
   `Action`, write a new `service_hydration_results` table, or
   re-read `service_backends` and assume convergence after a
   single tick?

3. **How is the failure surface modelled?** A
   `terminal: Option<TerminalCondition>` channel (ADR-0037), a
   typed dispatch error, or a generic `String` reason?

These questions extend the substrate established by:

- **ADR-0035** — `Reconciler` trait collapsed to one sync method
  `(desired, actual, view, tick) → (actions, next_view)`. No
  `.await`, no I/O.
- **ADR-0036** — runtime owns hydration of `desired` and `actual`
  via async surfaces on `AnyReconciler`; reconciler authors write
  `reconcile` only.
- **ADR-0021** — `AnyState` enum + `JobLifecycleState` shape; per-
  reconciler typed projection of intent + observation.
- **ADR-0023** — action shim placement at
  `overdrive-control-plane::reconciler_runtime::action_shim`;
  100 ms tick cadence; exhaustive Action match.
- **ADR-0037** — reconciler emits typed `TerminalCondition`; every
  terminal claim has a single typed source.
- **`development.md` § Persist inputs, not derived state** —
  reconciler View persists inputs, not deadlines.

## Decision

### 1. New typed Action variant `Action::DataplaneUpdateService`

Append to the `pub enum Action` block in
`crates/overdrive-core/src/reconciler.rs`:

```rust
/// Replace the backend set for a service VIP in the kernel-side
/// `SERVICE_MAP` / `BACKEND_MAP` / `MAGLEV_MAP` tuple.
DataplaneUpdateService {
    service_id:  ServiceId,
    vip:         ServiceVip,
    backends:    Vec<Backend>,
    correlation: CorrelationKey,
},
```

(Q-Action=A. Full doc-comment shape + invariants in
`design/architecture.md` § 7.)

The `correlation` field is required (not optional) — service
hydration is correlation-keyed end-to-end so the next tick can
locate the `service_hydration_results` row deterministically. The
hydrator constructs the value via the existing
`CorrelationKey::derive(target: &str, spec_hash: &ContentHash,
purpose: &str)` constructor in `crates/overdrive-core/src/id.rs`:

```rust
let target = format!("service-map-hydrator/{service_id}");
let spec_hash = ContentHash::of(&fingerprint.to_le_bytes()[..]);
let correlation = CorrelationKey::derive(
    &target,
    &spec_hash,
    "update-service",
);
```

Three-input shape matches the project's existing
`CorrelationKey::derive` precedent — same constructor used by
`HttpCall` reconcilers per § 18 of the whitepaper. No new
constructor surface is added; the hydrator never fabricates raw
correlation strings.

The `Vec<Backend>` is in deterministic `BTreeMap<BackendId,
Backend>::iter()` order, which is what makes Maglev permutation
byte-identical across nodes given identical inputs. The
`fingerprint: BackendSetFingerprint` value (a `pub type
BackendSetFingerprint = u64;` in
`crates/overdrive-core/src/dataplane/mod.rs`; see architecture.md
§ 6 *Type aliases* for the full rationale) is the canonical
content-hash of `(vip, backends)` per `development.md` § Hashing
requires deterministic serialization — rkyv-archived bytes,
blake3 digest, truncated to u64.

### 2. `ServiceMapHydrator` reconciler

A new reconciler kind, `service-map-hydrator`, lives at
`crates/overdrive-control-plane/src/reconcilers/service_map_hydrator/`.

Per-target keying = `ServiceId`. The evaluation broker keys
evaluations on `(ReconcilerName, ServiceId)` per ADR-0023's
storm-proof ingress — a row-change burst on N backends of one
service collapses to ONE pending evaluation, not N.

Per-reconciler `State` projection:

```rust
pub struct ServiceMapHydratorState {
    pub desired: BTreeMap<ServiceId, ServiceDesired>, // hydrated
                                                       // from
                                                       // service_backends
    pub actual:  BTreeMap<ServiceId, ServiceHydrationStatus>,
                                                       // hydrated
                                                       // from
                                                       // service_hydration_results
}
```

Both projections use `BTreeMap` per `development.md`
§ Ordered-collection choice — deterministic iteration is what
makes Maglev permutation byte-identical and what makes the ESR
DST invariant `HydratorIdempotentSteadyState` decidable.

Per-reconciler `View`:

```rust
pub struct ServiceMapHydratorView {
    pub retries: BTreeMap<ServiceId, RetryMemory>,
}

pub struct RetryMemory {
    pub attempts:                u32,
    pub last_failure_seen_at:    UnixInstant,
    pub last_attempted_fingerprint: Option<BackendSetFingerprint>,
}
```

Per `development.md` § Persist inputs, not derived state — the
View carries `attempts` and `last_failure_seen_at` (inputs to the
backoff policy), NOT a `next_attempt_at` deadline (derived). The
deadline is recomputed every tick as
`last_failure_seen_at + backoff_for_attempt(attempts)`. Never
persisted.

`reconcile` is sync, pure, no `.await`, no wall-clock read; full
skeleton lives in `design/architecture.md` § 8.

Hydration shape (runtime-owned, NOT in `reconcile`):

| Projection | Source | Hydrator surface |
|---|---|---|
| `desired.desired` | `service_backends` rows for the target `ServiceId` | New match arm in the runtime's free-function `hydrate_desired` per ADR-0036 |
| `actual.actual` | `service_hydration_results` row for the target `ServiceId` (NEW per § 4 below) | New match arm in the runtime's free-function `hydrate_actual` per ADR-0036 |
| `view.retries` | `RedbViewStore::bulk_load` at register; `write_through` after each tick | Runtime-owned per ADR-0035 |

Concrete arm signatures — extending the existing free functions
in `crates/overdrive-control-plane/src/reconciler_runtime.rs`
(around lines 769 / 825) — match the JobLifecycle precedent
exactly:

```rust
// Inside the existing free fn `hydrate_desired`. The function
// signature itself does not change; only a new match arm lands.
async fn hydrate_desired(
    reconciler: &AnyReconciler,
    target: &TargetResource,
    state: &AppState,
) -> Result<AnyState, ConvergenceError> {
    match reconciler {
        // ... existing arms (NoopHeartbeat, JobLifecycle) ...
        AnyReconciler::ServiceMapHydrator(_) => {
            let service_id = service_id_from_target(target)?;
            let rows = state
                .obs
                .service_backends_rows(&service_id)
                .await
                .map_err(|e| ConvergenceError::ObservationRead(e.to_string()))?;
            // ... assemble BTreeMap<ServiceId, ServiceDesired>;
            // wrap row.vip (Ipv4Addr) into ServiceVip at boundary;
            // compute fingerprint via dataplane::fingerprint(...) ...
            Ok(AnyState::ServiceMapHydrator(/* state */))
        }
    }
}

// Same shape for `hydrate_actual`, reading
// state.obs.service_hydration_results_rows(&service_id) and
// projecting into BTreeMap<ServiceId, ServiceHydrationStatus>.
```

Three load-bearing properties of the placement:

- The runtime's `hydrate_desired` / `hydrate_actual` are **free
  functions in the runtime module**, NOT methods on
  `AnyReconciler`. `AnyReconciler` is the dispatch enum; the
  match-arm body lives in the runtime. ADR-0036's
  "runtime owns hydration" placement is preserved.
- Both arms read **only the `ObservationStore`** (`state.obs.*`).
  `service_backends` is observation per ADR-0023;
  `service_hydration_results` is observation per § 4 below.
  Neither arm touches the IntentStore. The `state.store`
  (IntentStore) field on `AppState` is unused for this
  reconciler kind, present on the receiver only because the
  existing function signature carries it for the JobLifecycle
  arm.
- Each arm produces a *partial* `ServiceMapHydratorState`. The
  runtime merges the two partials into the single `State` value
  passed to `reconcile`, matching the JobLifecycle precedent's
  `desired.allocations` / `actual.allocations` projection split
  (`reconciler_runtime.rs` ~line 788 / ~line 847).

ESR pair (locked names from DISCUSS):

- **`HydratorEventuallyConverges`** — for every `service_id`,
  `actual.fingerprint == desired.fingerprint` is reached within a
  bounded number of ticks given a stable `desired`.
- **`HydratorIdempotentSteadyState`** — once
  `actual.fingerprint == desired.fingerprint` for all services,
  the hydrator emits zero `DataplaneUpdateService` actions per
  tick.

Both invariants live in `crates/overdrive-sim/src/invariants/` and
run on every PR per `.claude/rules/testing.md` § Tier 1.

### 3. Action shim wrapper at `action_shim/service_hydration.rs`

A new file in
`crates/overdrive-control-plane/src/action_shim/service_hydration.rs`
hosts:

```rust
pub async fn dispatch(
    action: Action::DataplaneUpdateService,
    dataplane: &dyn Dataplane,
    obs:       &dyn ObservationStore,
    tick:      &TickContext,
) -> Result<(), ServiceHydrationDispatchError> { ... }

#[derive(thiserror::Error, Debug)]
pub enum ServiceHydrationDispatchError {
    #[error("dataplane update_service failed")]
    Dataplane(#[from] DataplaneError),
    #[error("observation write failed")]
    Observation(#[from] ObservationStoreError),
}
```

The shim:

- **On `Ok(())`** — writes `service_hydration_results` row with
  `status: Completed { fingerprint, applied_at: tick.now }`.
- **On `Err(DataplaneError::*)`** — writes `service_hydration_results`
  row with `status: Failed { fingerprint, failed_at: tick.now,
  reason: Display::to_string(&err) }`.

The shim's error type does **NOT** carry a
`terminal: Option<TerminalCondition>` field. Service hydration
cannot terminate an allocation — `TerminalCondition` is exclusively
for *allocation lifecycle* terminal claims per ADR-0037. Mixing
the channels would erode ADR-0037's "every terminal claim has a
single typed source" invariant. Retry-budget logic lives in the
View; failure observability lives in `service_hydration_results`.

### 4. New `service_hydration_results` ObservationStore table

The schema is replicated inline here (this ADR is the schema
lockpoint; design/architecture.md mirrors the same shape, but the
ADR is canonical):

| Column | Type | Nullable | Notes |
|---|---|---|---|
| `service_id` | `ServiceId` (u64) | NO | Primary key. |
| `fingerprint` | `BackendSetFingerprint` (u64; type alias — see architecture.md § 6 *Type aliases*) | NO | Last attempted. Content-hash of `(vip, backends)` per `development.md` § Hashing requires deterministic serialization — rkyv-archived bytes, blake3 digest, truncated to u64. |
| `status` | tagged enum: `Pending` / `Completed` / `Failed` (see `ServiceHydrationStatus` in design § 8) | NO | The tagged enum is the discriminant; `applied_at` / `failed_at` / `reason` are payload fields whose nullability is conditional on the variant. |
| `applied_at` | `UnixInstant` | YES | Set on `Completed`; null otherwise. |
| `failed_at` | `UnixInstant` | YES | Set on `Failed`; null otherwise. |
| `reason` | `String` (bounded length — same convention as the project's other observation rows; truncate-with-suffix at the writer) | YES | Set on `Failed`; null otherwise. |
| `lamport_counter` | `u64` (per ObservationStore convention) | NO | LWW resolution (see *LWW resolution semantics* below). |
| `writer_node_id` | `NodeId` | NO | LWW tie-breaker; identifies the action shim that produced the row. |

**Migration discipline**: additive-only per `whitepaper.md`
§ *Consistency Guardrails*. The whitepaper forbids
`ALTER TABLE ADD COLUMN NULL` against existing tables cluster-wide
because the resulting backfill storm is one of the failure modes
Fly.io's Corrosion deployment has documented. The additive shape
is therefore that the **whole table is new** at first
introduction — no edits to `service_backends`, `alloc_status`, or
`node_health`. The `applied_at` / `failed_at` / `reason` columns
above ARE nullable, but they are nullable on a *fresh* table where
the nullability is the table's birth state, not a retroactive
column addition.

**LWW resolution semantics** (inherited from CR-SQLite per
ADR-0012 revised + whitepaper § Consistency Guardrails):

- The PK is `service_id` only; one row per service. Two writers
  emitting against the same `service_id` produce *one* surviving
  row, resolved by `lamport_counter` (higher wins;
  `writer_node_id` breaks ties deterministically).
- This is correct for the hydrator's purpose because the
  fingerprint is content-hashed: a "stale" row carrying an older
  fingerprint is the prior-state observation and is correctly
  superseded by the newer state. There is no risk of losing a
  meaningfully-distinct outcome to LWW; "the most recent
  fingerprint" is exactly what `actual` should reflect.
- Future per-region Corrosion adoption (Phase 2+) inherits the
  same row shape; the fingerprint's content-determinism makes
  cross-region LWW deterministic.

**Single-writer in Phase 2.2** — only the action shim's
`service_hydration` module writes; the hydrator reconciler is the
sole reader. Single-writer is consistent with
`LocalObservationStore`'s Phase 1 model (ADR-0012 revised); the
LWW machinery above is dormant until Phase 2 Corrosion.

Trait surface: typed row helpers
`service_hydration_results_rows(service_id)` /
`write_service_hydration_result(row)` extend the existing
`ObservationStore` trait, matching the existing `alloc_status_rows`
/ `node_health_rows` precedent.

### 5. Why the `actual` projection reads observation rows, not last-emitted action

The structural rationale that drives Drift 2's introduction of
`service_hydration_results`:

Deriving `actual` from "the last action I emitted" produces a
**write-only loop** that cannot detect a silently-failed dataplane
update. The reconciler would emit `DataplaneUpdateService`,
remember the fingerprint locally, and the next tick would see
`actual.fingerprint == fingerprint-I-just-tried-to-emit` even if
the kernel-side update failed. ESR would falsely claim
convergence. This is the exact failure shape J-PLAT-004 is meant
to close.

The fix is structural: `actual` reads what the action shim
**confirmed** by writing an observation row after the dataplane
call returned. The convergence check then becomes:
`desired.fingerprint == actual.fingerprint` if-and-only-if the
shim observed `Ok(())` from `Dataplane::update_service` AND wrote
the `Completed` row AND the next tick read it back. The path is
end-to-end observable.

This is `development.md` § Persist inputs, not derived state at
the cross-component boundary: `actual` is the *input* (the
observation row) the reconciler reads; the *output* (whether to
re-emit, with what backoff) is recomputed every tick from those
inputs and the live retry policy.

## Alternatives Considered

### A — Derive `actual` from last-emitted action (no `service_hydration_results`)

Keep the View richer; track per-service "last fingerprint emitted";
treat `desired.fingerprint == view.last_fingerprint` as
convergence. **Rejected**: write-only loop, can't detect silent
failure. ESR would false-positive. See § 5 above for the full
rationale.

### B — Reuse `Action::HttpCall` with an internal "dataplane://"
URL

Encode the dataplane call as a `HttpCall` with target
`dataplane://service-update/<service_id>`. **Rejected**: forces
string-sniffing on the action shim; loses type safety; the
existing `HttpCall` shim writes `external_call_results` rows that
mix dataplane + external HTTP outcomes. Erodes ADR-0023's
exhaustive-match property.

### C — Generic `Action::DataplaneCall { op: DataplaneOp }` enum

A single Action variant covers every future Dataplane port method
via an inner enum `DataplaneOp::UpdateService { ... } | UpdatePolicy { ... } | ...`. **Rejected**: over-engineering for
Phase 2.2's single-method scope. Future Dataplane port methods
(SERVICE_MAP delete, POLICY_MAP update at #25, flow-event drain at
#27) compose better as their own typed Action variants — each
carries its own typed payload, each lands its own `action_shim`
wrapper, the action-shim match stays exhaustive per-feature
without an internal `DataplaneOp` match nested inside. The
"Action enum grows linearly with the Dataplane port surface"
property is structural strength, not weakness.

### D — Add `terminal: Option<TerminalCondition>` to
`ServiceHydrationDispatchError`

Carry a `TerminalCondition` channel alongside the dispatch error
so a hard-failed hydration could mark the service as
permanently-failed. **Rejected**: violates ADR-0037 invariant
("every terminal claim has a single typed source"). Service
hydration cannot terminate an allocation — the worst case is
"this service is currently misconfigured and the kernel-side maps
don't match," which is fully expressible as a `Failed` row in
`service_hydration_results`. Operator-facing alerting on persistent
hydration failure is a future concern (Phase 3+ observability
ticket); the row shape supports it without a `TerminalCondition`
channel.

### E — `service_hydration_results` in IntentStore (not ObservationStore)

Persist the hydration outcome as authoritative intent. **Rejected**:
hydration outcome is what *is*, not what *should be* — the §4 / §18
intent-vs-observation boundary requires it on the observation
side. Cross-region future: per-region hydration outcomes converge
via Corrosion LWW (Phase 2+), which is incompatible with the
linearizable Raft path that intent flows through.

## Consequences

**Positive:**

- ASR-2.2-04 (hydrator ESR closure) becomes structurally achievable
  — `desired.fingerprint == actual.fingerprint` is observable,
  not assumed.
- J-PLAT-004 closes for this feature: first non-trivial reconciler
  emits a typed Action against a real Dataplane port body; ESR
  invariants run in DST.
- Precedent established for follow-on Dataplane-port reconcilers:
  POLICY_MAP hydrator (#25), flow-event drain reconciler (#27),
  future BPF LSM policy hydrator (#26) all follow the same shape
  (typed Action variant + observation-row failure surface +
  ESR pair).
- Single new ObservationStore table; additive-only migration; no
  edits to existing observation rows.
- Retry-budget logic centralised in View per
  `development.md` discipline; deadline recomputation picks up
  backoff-policy changes for free.

**Negative:**

- One additional Action variant (`DataplaneUpdateService`) to
  match-exhaustively in the action shim; small maintenance cost.
- One additional ObservationStore table on every node (storage
  cost is negligible — at most one row per active service).
- Adds a hydrate_actual round-trip to the reconciler tick that
  did not exist before; mitigated by `BTreeMap`-bulk-load at
  hydration time and by per-service keying that bounds the row
  count.

**Operational implications:**

- DST harness (`cargo xtask dst`) gains the two new ESR
  invariants; Tier 1 wall-clock budget delta is small (the
  hydrator's `reconcile` is a thin loop over `desired.desired`).
- `service_hydration_results` row writes are observable in
  operator-facing tools when the
  `cluster status` / dataplane diagnostics surfaces the
  observation store (Phase 2+ ticket; out of scope here).
- The hydrator becomes the reference shape for Phase 2's later
  Dataplane-side reconcilers; #25 / #26 / #27 will mirror the
  Action+row+ESR triad established here.

## References

- `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 7,
  § 8, § 12, § 14.
- `docs/feature/phase-2-xdp-service-map/design/wave-decisions.md`
  D9, D10.
- `docs/whitepaper.md` § 18 *Reconciler primitive*, § 4 *Intent /
  Observation split*, § *Consistency Guardrails*.
- `.claude/rules/development.md` § Persist inputs, not derived
  state; § Reconciler I/O; § Ordered-collection choice.
- `docs/product/jobs.yaml` — `J-PLAT-004`.
- ADR-0021 (`AnyState` enum) + ADR-0036 (runtime-owned hydration).
- ADR-0023 (action shim placement + tick cadence).
- ADR-0035 (collapsed `Reconciler` trait + `RedbViewStore`).
- ADR-0037 (`TerminalCondition` invariant — preserved).
- ADR-0040 (three-map split + HASH_OF_MAPS) — companion.
- ADR-0041 (weighted Maglev + REVERSE_NAT) — companion.
