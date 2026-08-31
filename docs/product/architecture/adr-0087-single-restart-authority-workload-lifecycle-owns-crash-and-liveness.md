# ADR-0087 — Single restart authority: `WorkloadLifecycle` owns crash + liveness restart under one budget; `ServiceLifecycle` demotes to readiness/membership + liveness-terminate

## Status

Accepted. 2026-08-25. Decision-makers: Morgan (proposing); user
ratification via `/nw-design` Decision 1 = **guide** (the kubelet-shape
single-authority fold-in is locked through prior discussion; this ADR
records it and pins the mechanism). Tags: phase-1, reconciler-primitive,
application-arch, restart-authority, service-health-check.

**Amended 2026-08-31 (TRC-ARCH-002).** Reset-on-emit forgot the liveness
target when Stop lost twice to the exit observer or was cancelled after an
ambiguous commit. The existing consecutive-failure counter now remains reached
until the liveness terminal and process-local route tail converge (or a
different terminal wins). Service actual hydration reuses ADR-0086's fifth
route port for exact current-terminal/route inputs. Restart authority and the
shared observed terminal remain unchanged; no receipt or restart decision
returns to ServiceLifecycle.

**Amended 2026-08-31 (TRC-ARCH-003).** Cross-reconciler batch order is not a
handoff barrier: WorkloadLifecycle may restart after exact-tail removal before
ServiceLifecycle observes exact+unrouted. The existing Service View therefore
records the accepted Running row's logical `updated_at`. A changed logical
attempt resets the old liveness counter before dispatch, and only a V2 probe
bearing that exact identity may count. Probe latest-row LWW compares attempt
before wall time, so equality/rollback cannot re-admit the old decision. This
is attempt memory, not a Stop receipt; Service still emits no Restart and
Workload still reads no Service View.

**Blocks / precedes ADR-0086.** This behaviour change lands as a
**precursor** to the `overdrive-reconcilers` hydration crate-move
(ADR-0086): it dissolves the one cross-reconciler read at its root, so
ADR-0086 no longer needs its fifth read-port (`RestartBudgetView`) — see
ADR-0086 (amended 2026-08-25, 5→4 ports) § D5 and the sequencing in
§ "Migration slice sketch" below.

**Implements** `.claude/rules/reconcilers.md` § "Single restart
authority — never split one budget across reconcilers" (the rule; this
ADR is its implementing decision). **Supersedes** the ADR-0055 §7
"LivenessRestartGovernor" future direction (a *separate* liveness-restart
governor is no longer the target — single authority replaces it).

**Companion**: ADR-0037 (`TerminalCondition` publication boundary +
action-shim single-writer), ADR-0078 (`restart_counts` attempts vs
observed `CrashFacts.restart_count`), ADR-0081/0083 (`StoppedBy`
Ending-Class dispositions + `is_platform_reclaimed`), ADR-0054/0055/0080
(probe subsystem + roles), ADR-0084 (`interests()` wakeup).

## Context

The restart budget is today **split across two reconcilers by restart
cause**:

- **`WorkloadLifecycle`** owns crash-restart. Its private
  `View.restart_counts: BTreeMap<AllocationId, u32>` counts restart
  *attempts*, gated at `RESTART_BACKOFF_CEILING` (=5). It emits
  `RestartAllocation` for a restartable Failed/Terminated alloc and,
  on ceiling, `FinalizeFailed { BackoffExhausted { attempts } }`
  (`workload_lifecycle.rs:687-843`, `:709`, `:733`).
- **`ServiceLifecycle`** makes its OWN liveness-restart decision. On a
  liveness probe reaching its consecutive-failure `threshold` on a
  Running alloc it **reads `WorkloadLifecycle`'s budget** —
  `fact.restart_count` hydrated via
  `ReconcilerRuntime::restart_status_for_alloc`
  (`reconciler_runtime.rs:499`, `:3418`) — and decides restart-vs-finalize:
  `restart_count < CEILING` → `RestartAllocation { reason:
  LivenessExhausted }`; `restart_count >= CEILING` → `FinalizeFailed {
  ServiceFailed { LivenessProbeFailed } }`
  (`service_lifecycle.rs:753-820`, `:790`).

So one pool of `RESTART_BACKOFF_CEILING` attempts is drawn down by **two
independent reconcilers**, one of them by reaching into the other's
private `View`. Research RQ3 (22 sources; kubelet, Nomad, OTP, systemd,
Akka) is unambiguous: **every mature orchestrator unifies restart
authority under one owner, and one owner's budget legitimately spans
multiple causes** — the kubelet is the exact precedent for "one
per-container budget covering BOTH crash exits AND liveness-probe kills."
No examined system splits one budget across two controllers by cause.
The correct k8s mapping is **kubelet-vs-Service, not
Deployment-vs-Service**: the node agent (≈ `WorkloadLifecycle`) is the
sole restart authority; the Service layer only maps *readiness* →
endpoint membership and restarts nothing.

The user has **locked** the kubelet-shape fix. This ADR records it and
pins the mechanism the four hard constraints require (cause-preservation,
the terminate mechanism, budget unification, DST testability).

## Decision

### D1. `WorkloadLifecycle` becomes the SOLE restart authority

Crash restart AND liveness restart draw on `WorkloadLifecycle`'s **one**
`restart_counts` budget, incremented at its **single** existing emit site
(`workload_lifecycle.rs:829-831`). A liveness-caused restart increments
the same counter a crash does; `RESTART_BACKOFF_CEILING` caps the union.
`ServiceLifecycle` makes **no** restart decision and reads **no** budget.

### D2. `ServiceLifecycle` demotes to readiness → membership + liveness-**terminate**

The readiness → `Backend.healthy` → `ServiceBackendRow` membership branch
is **unchanged** (`service_lifecycle.rs:673-689`,
`readiness_backend_row_action`). The liveness branch changes from
*restart decision* to *detect-and-terminate*:

- On `state == Running AND consecutive_failures >=
  liveness_failure_threshold` (the existing predicate, unchanged), the
  reconciler emits **`Action::StopAllocation { alloc_id, terminal:
  Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }) }`** —
  a terminate. **The termination IS the signal** (kubelet: liveness kill →
  the container is restarted under the same `restartPolicy`+backoff).
- It reads no budget, makes no restart-vs-finalize decision, carries no
  `restart_count`/`restart_spec`, and emits neither `RestartAllocation`
  nor `FinalizeFailed` on the liveness path.
- The consecutive-failure counter is **not reset on emit**. While Running,
  probe Fail/Pass maintenance remains unchanged; after threshold emission the
  reached counter is retained while the row is non-Running. It makes
  `Terminated/None` re-author the liveness target after bounded loss and makes
  exact liveness terminal+routed re-emit only the action-shim tail. Exact
  liveness terminal+unrouted (or a different `Some(..)` terminal) clears the
  counter and emits nothing, so the replacement Running attempt starts clean.
  Draining waits; Pending/Failed do not emit. Broker in-flight collapse
  prevents simultaneous duplicate actions, while a repeat after a completed or
  cancelled dispatch is legitimate level-triggered convergence.
- The clean-replacement claim is ordering-independent. Add exactly
  `#[serde(default)] liveness_attempt:
  BTreeMap<AllocationId, LogicalTimestamp>` to `ServiceLifecycleView` and
  `status_updated_at: LogicalTimestamp` to `ServiceAllocFact`. Before Running
  counter maintenance, missing/different logical identity clears the old
  counter and stores the current value in `next_view`; that View fsyncs before
  action dispatch. Hydration exposes a liveness result only when
  `ProbeResultRowV2.alloc_attempt == Some(status_updated_at)`. A first already-
  visible exact-attempt Fail seeds the reset counter at one. Non-Running repair
  retains marker+counter; exact-unrouted/mismatch clears both. No broker
  ordering, cross-View read, or receipt participates. Replay remains owned by
  the exact terminal plus route-membership facts; the marker is only an attempt
  fence. ADR-0048/0054 own attempt-first probe LWW and the exact changed
  Running-hook/ProbeRunner signatures.

This satisfies the rule's demotion exactly: *"the service/membership
reconciler owns readiness → backend membership and emits **no** restart"*
— a `StopAllocation`-terminate is detection, not a restart or budget
decision.

### D3. The liveness **cause travels on the shared observed `AllocStatusRow`** — a new `StoppedBy::LivenessProbe` disposition

This is the crux (Hard Constraint 1). The cause is **not** carried in
memory or across a cross-reconciler read; it is **published on the shared
observed object** — the idiomatic mechanism (research RQ2: controller A
reads controller B's *published output as shared state*, never B's
private memory; ADR-0078 discipline: an operator-meaningful fact goes on
the shared observed surface).

Add one variant to `StoppedBy` (`transition_reason.rs`):

```rust
pub enum StoppedBy {
    Operator, Reconciler, Process, SystemGc, PlatformReclaimed,
    /// A liveness probe (via ServiceLifecycle) terminated this instance;
    /// the workload's intent still stands and WorkloadLifecycle restarts
    /// it under its single budget. Restartable (NOT an intentional stop),
    /// crash-class (NOT platform-reclaim). Appended at the tail →
    /// discriminant 5, rkyv-additive per this enum's additive-position
    /// discipline; existing archived rows decode unchanged.
    LivenessProbe,
}
```

Flow, reusing the existing `StopAllocation` action + shim executor + the
`Stopped { by }` dual-field machinery (**no new action, no new
executor**):

1. `ServiceLifecycle` emits `StopAllocation { terminal: Stopped { by:
   LivenessProbe } }`.
2. The action-shim (ADR-0037 §4 single-writer) stops the driver and
   writes the alloc row `state = Terminated`, `terminal = Stopped { by:
   LivenessProbe }` (the action-supplied claim, propagated verbatim). The
   `StopAllocation` executor is **UNCHANGED** — it continues to hardcode
   `reason = Stopped { by: Reconciler }` (`action_shim/mod.rs:1973`); per
   its own contract (`:1944-1948`) operator/cause attribution "lands
   exclusively on `terminal`," so the cause travels on **`terminal`**, not
   `reason`. **Do NOT modify the shim's `reason` hardcode** — changing it
   would regress the wire-side `last_transition.reason` for *every* stop.
3. If the terminal proposal loses twice, ServiceLifecycle's retained threshold
   plus `Terminated/None` re-emits it. If cancellation leaves exact terminal
   plus route, exact+routed re-emits only release/route repair. The facts are
   `ServiceAllocFact::{terminal, driver_route_present, status_updated_at}`,
   hydrated from current plus ADR-0086's existing route snapshot; no target
   receipt is persisted.
4. `WorkloadLifecycle` may hydrate the Terminated row and restart it under
   budget (D4) before ServiceLifecycle's self-enqueued exact+unrouted pass;
   there is deliberately no ordering claim. On the next Running Service tick,
   the dominating logical `status_updated_at` retires the old counter before
   action selection and the old probe's attempt mismatches. If Service clears
   first, the same tick starts from empty state. Both orders require an
   exact-attempt probe before another liveness Stop, regardless of equal or
   rolled-back wall clocks.

Why reuse `StoppedBy` rather than a new terminal: `StoppedBy`'s de-facto
semantics is already "who/what ended this instance" (`Process`,
`PlatformReclaimed` are cause-ish, not literal initiators), the
dual-field `Stopped { by }` shape is exactly what the restartability and
exemption predicates already read, and a fieldless variant is the
minimal rkyv-additive change. `probe_idx` is not carried: Phase-1 liveness
is a single probe at index 0 (every current site hardcodes `probe_idx:
0`; the counter is keyed `(alloc_id, ProbeIdx::new(0))`). A future
multi-liveness-probe world would need a data-carrying marker — noted,
out of scope, not built.

### D4. `WorkloadLifecycle` observes the liveness-terminated row as **restartable**, and preserves the liveness terminal at exhaustion

`is_restartable(row)` = `state ∈ {Terminated, Draining, Failed} AND
!is_intentionally_stopped(row)`, where `is_intentionally_stopped`
matches `Stopped { by ∈ {Operator, SystemGc} }` only. A `Stopped { by:
LivenessProbe }` row is therefore **restartable by construction** — no
change to `is_restartable` — while an operator/GC stop is **not**
(the intentional-stop discriminator is preserved: operator stop is never
restarted, liveness kill always is).

Add a `WorkloadLifecycle`-private predicate mirroring
`is_platform_reclaimed` (dual-field: reads BOTH `terminal` and `reason`):

```rust
fn is_liveness_killed(row: &AllocStatusRow) -> bool {
    row.state == AllocState::Terminated
        && (matches!(row.terminal, Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }))
         || matches!(row.reason,   Some(TransitionReason::Stopped   { by: StoppedBy::LivenessProbe })))
}
```

Dual-field by construction — this mirrors `is_platform_reclaimed`
(`transition_reason.rs:1071`) exactly. Under the current `StopAllocation`
shim path the cause fires via the **`terminal`** branch (the shim
hardcodes `reason = Reconciler`, D3 step 2); the `reason` branch is
**defensive** (matches `is_platform_reclaimed`'s shape and would catch a
future direct-observer writer), not a live path today.

In the restart-budget branch (`workload_lifecycle.rs:687-843`) the
ceiling-exhaustion terminal becomes **cause-aware** (Hard Constraint 1 —
the liveness terminal must NOT be flattened to `BackoffExhausted`):

```
if attempts >= RESTART_BACKOFF_CEILING && !is_platform_reclaimed(failed) {
    // idempotency guard: already-finalized → no re-emit
    if matches!(failed.terminal, Some(BackoffExhausted { .. }
                                    | ServiceFailed { reason: LivenessProbeFailed { .. } })) {
        return (vec![], view.clone());
    }
    let terminal = if is_liveness_killed(failed) {
        TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::LivenessProbeFailed { probe_idx: 0, attempts },
        }
    } else {
        TerminalCondition::BackoffExhausted { attempts }
    };
    return (vec![FinalizeFailed { alloc_id, terminal: Some(terminal) }], view.clone());
}
```

The restart-emission path (below the ceiling), the `restart_counts`
increment, and the backoff-window recompute are **unchanged** — the
liveness restart flows through the identical crash-restart machinery, so
the budget unifies by construction (D5). The `Action::RestartAllocation`
carries `reason: None` (the crash-restart shape): the restart's cause is
the prior row's observable `Stopped { by: LivenessProbe }` terminal, not
a field on the action — exactly as the crash-restart cause is implicit in
the prior row's crash terminal.

**`attempts` semantics at the liveness terminal — a noted, forced
change.** Today `ServiceLifecycle` stamps `LivenessProbeFailed { attempts:
consecutive_failures }` (the liveness streak at the deciding tick). Under
single authority `WorkloadLifecycle` stamps `attempts = restart_counts`
(the restart-budget count consumed, = `CEILING` at exhaustion). This is
**forced** — preserving `consecutive_failures` would require reading
`ServiceLifecycle`'s private View, the very cross-read this ADR
eliminates — and it is the **more correct** reading: it parallels
`BackoffExhausted { attempts }` (same "attempts consumed" meaning), and is
what an operator wants ("restarted 5× for liveness, gave up"). See
§ Consequences for the one contested value-semantics point surfaced to
the user.

### D5. Budget unification + `is_platform_reclaimed` exemption interaction (ADR-0078)

- **Unifies by construction.** The liveness restart is now emitted at
  `WorkloadLifecycle`'s single increment site, so `restart_counts` is one
  budget spanning both causes — there is literally one increment site and
  one ceiling. A crash and a liveness kill on the same alloc draw the
  same pool (the kubelet shape; verified against Hard Constraint 3).
- **Two quantities stay distinct (ADR-0078 § D3), unchanged.**
  `restart_counts` (View, *attempts*, incl. driver `StartRejected`) is the
  authority's budget; observed `CrashFacts.restart_count`
  (`AllocStatusRow.restart_count`, *completed* restarts) still increments
  when the shim observes `terminal → Running` — a liveness restart is
  still a terminal→Running the shim observes, so both quantities move,
  correctly, as different numbers. "Do not source one from the other"
  holds.
- **Liveness is crash-class, NOT platform-reclaim → consumes budget.**
  `is_platform_reclaimed` reads `true` only for `StoppedBy::PlatformReclaimed`;
  its exhaustive `by_reclaims_platform` match gains a `LivenessProbe =>
  false` arm (the new variant forces a compile error there → mapped to
  `false`, the smallest correct diff). So a liveness kill is **NOT**
  exempt from the ceiling — N liveness kills exhaust the budget and fire
  the terminal, exactly as crashes do. The exemption (brief §105a.10,
  `workload_lifecycle.rs:709`) continues to hold for genuine platform
  reclamation and only that. (Verified against Hard Constraint 3.)

### D6. Pure-sync `reconcile` + DST replay preserved; the trajectory is Tier-1 testable

Both reconcilers stay pure-sync `(desired, actual, view, tick) →
(Vec<Action>, View)` — the liveness path now emits `StopAllocation`
(pure), reads no budget port. The whole trajectory —
liveness-Fail probe rows → `ServiceLifecycle` `StopAllocation` → shim
writes `Stopped { by: LivenessProbe }` → `WorkloadLifecycle`
`RestartAllocation` (budget++) → … → exhaust → `ServiceFailed {
LivenessProbeFailed }` — is driven **entirely by observation rows +
`SimClock` + `SimDriver` + `SimObservationStore`**, with no
cross-reconciler read to inject. This is **more** DST-testable than today
(today the liveness branch needed the `restart_status_for_alloc`
cross-read). The single-loop/single-clock replay-equivalence model is
structurally unchanged. See § "Migration slice sketch" for the two
mandated DST tests.

### D7. Delete the now-dead cross-read + liveness-restart vocabulary (single-cut greenfield)

Per `feedback_single_cut_greenfield_migrations.md` — no parallel path, no
deprecation shim. In the same arc that lands D1–D6, **delete**:

- The **cross-read CALL** in `hydrate_service_alloc_facts`
  (`reconciler_runtime.rs:3419`, the `restart_status_for_alloc(...)`
  invocation) and the `liveness_restart_spec` build (`:3424`). **KEEP the
  `ReconcilerRuntime::restart_status_for_alloc` method itself** (`:499`) —
  it has **four live callers in `streaming.rs`** (`:398,:438,:492,:544`)
  that render the operator-facing attempt-index on `overdrive deploy`
  streaming events, plus its two boundary unit tests. Those are the
  streaming/event layer, NOT the `ServiceLifecycle` hydration cross-read;
  only the hydration call is removed. This is the surface ADR-0086's
  `RestartBudgetView` was going to wrap — the *hydration* read is gone, so
  the port is never needed (the method's surviving callers do not cross
  the reconcilers-crate boundary — they live in control-plane's streaming
  layer alongside the runtime).
- `ServiceAllocFact.restart_count` and `ServiceAllocFact.restart_spec`
  (`service_lifecycle.rs:205,215`) — the fact no longer carries the
  budget or the restart spec.
- `RestartReason` (enum, `reconcilers/mod.rs:690`) and the
  `Action::RestartAllocation.reason` field (`:495`) — `LivenessExhausted`
  is `ServiceLifecycle`'s liveness-restart vocabulary, no longer emitted;
  crash-restart always passed `None`; the action-shim consumer destructures
  `reason: _` and ignores it (`action_shim/mod.rs:1626`), so the field is
  dead. The liveness cause now lives on the observed row's terminal (D3),
  not on the restart action.

## Alternatives considered

- **Detector→signal via a fresh observation row (OTP/Nomad
  detect-then-escalate shape).** `ServiceLifecycle` writes a
  "liveness-exhausted" observation row that `WorkloadLifecycle` reads.
  Rejected: it invents a new observation surface for a fact the existing
  `AllocStatusRow` terminal already carries once the alloc is terminated —
  the kubelet's "kill → same restart path" is the tighter, in-tree shape,
  and `StopAllocation` + the row's terminal is the single publication
  boundary ADR-0037 already owns.
- **The probe subsystem (`probe_runner`, ADR-0080) kills the workload
  directly** (literal kubelet-worker shape). Rejected: `probe_runner` is
  a worker-side observation-producer (writes `ProbeResultRow`s) and a
  terminal-surrender consumer (its tasks self-cancel when the alloc
  reaches terminal, `supervisor.rs`) — a direct kill there is an
  imperative worker-side side-channel **invisible to DST**, breaking the
  level-triggered / pure-reconcile / replay model that makes convergence
  verifiable. The decision must stay in the reconciler; the kill is an
  emitted `Action` executed by the shim.
- **Data-carrying liveness marker** (a `TerminalCondition::LivenessKilled
  { probe_idx, consecutive_failures }` on the row so the exhaustion
  terminal can preserve `consecutive_failures` and a real `probe_idx`).
  Rejected for Phase 1: more rkyv surface + a new terminal variant to
  carry a value (`consecutive_failures`) that is *less* operationally
  meaningful than the restart-attempt count, for a `probe_idx` that is
  always 0 in Phase 1. Revisit only if multi-liveness-probe lands.
- **Keep the split, formalise it as a read-port** (ADR-0086's
  `RestartBudgetView`). Rejected: that is the *behaviour-preserving*
  cycle-break, not a fix — it survives only until this authority
  unification lands (`.claude/rules/reconcilers.md` § "Single restart
  authority"). Eliminating the split at the root removes the need for the
  port entirely.

## Consequences

### Positive

- **Restart authority is single-owner (kubelet shape).** One budget spans
  crash + liveness; the anti-pattern the rule names (a reconciler reading
  another's private budget) is gone at the root — the cross-read has no
  site left.
- **The cross-read AND the spec-carry dissolve.** The `restart_status_for_alloc`
  *hydration call*, `ServiceAllocFact.{restart_count,restart_spec}`,
  `liveness_restart_spec`, and `RestartReason` all delete (the
  `restart_status_for_alloc` *method* is retained for streaming, D7) —
  `ServiceLifecycle` hydration gets materially simpler, and ADR-0086 loses
  a read-port (5→4).
- **Hydration becomes purely observation-row-driven for the liveness
  path** → strictly more DST-injectable.
- **`ServiceLifecycle` is now cleanly a readiness/membership reconciler +
  a liveness detector** — the correct k8s Service mapping.

### Negative / neutral

- **One extra reconcile-tick of latency on a liveness restart.** The path
  is now terminate → observe row change → restart (mediated by the
  interest-router wake) rather than a direct restart. This is the
  kubelet's own detect→restart indirection; ~one tick (~100 ms + router
  wake). Neutral-acceptable — the cost of the correct single-authority
  boundary.
- **Operator-facing `LivenessProbeFailed.attempts` value shifts** from
  "consecutive liveness failures" to "restart attempts consumed" (D4).
  Forced by single authority; the more consistent reading. **This is the
  one contested value-semantics point surfaced to the user** (§ D4).
- **Interleaved crash+liveness → last-cause-wins at exhaustion.** If an
  alloc both crashes and fails liveness before the shared budget is spent,
  the terminal reflects the *most recent* kill's cause
  (`is_liveness_killed` reads the latest row). This mirrors the kubelet
  (one `RESTARTS` counter; the last cause is what's visible) and is the
  intended semantics.
- **Operator `overdrive deploy` streaming attempt-index now spans
  liveness restarts for Service allocs.** The retained
  `restart_status_for_alloc` (D7 — kept for the streaming layer) reads
  `restart_counts`, which now increments on liveness restarts too, so the
  `JobSubmitEvent` attempt-index (`streaming.rs:398,438,492,544`) reflects
  the *unified* crash+liveness budget — again the kubelet single-`RESTARTS`
  shape. An operator watching a liveness-restarting Service sees its
  attempt count advance; expected, not a regression.

### Quality-attribute impact

- **Maintainability — modularity**: strong positive (one authority; dead
  vocabulary deleted). **Testability**: strong positive (trajectory is
  pure observation-row DST). **Reliability**: neutral-positive (unified
  budget can no longer double-count or diverge across two reconcilers).
  **Performance**: neutral (one extra tick of restart latency).

## Compliance — what survives / what changes

- **The operator-facing liveness/crash terminal distinction survives**
  (Hard Constraint 1): a liveness-exhaustion loop finalises as
  `ServiceFailed { LivenessProbeFailed }`; a crash loop as
  `BackoffExhausted`. Attributed to the liveness cause via the observed
  `Stopped { by: LivenessProbe }` row, never flattened.
- **The intentional-stop discriminator survives** (Hard Constraint 2):
  `is_intentionally_stopped` (Operator | SystemGc) is unchanged; an
  operator stop is still never restarted, a liveness kill always is.
- **ADR-0037 (publication boundary / action-shim single-writer)** —
  unchanged; the liveness terminal + the `Stopped { by: LivenessProbe }`
  row are written through the same single-writer path.
- **ADR-0078 (`restart_counts` vs `CrashFacts.restart_count`)** —
  unchanged; the two quantities stay distinct (D5).
- **ADR-0081/0083 (`StoppedBy` + `is_platform_reclaimed`)** — additive:
  one tail variant; the exemption predicate gains a `false` arm; genuine
  platform-reclaim exemption is untouched.
- **ADR-0055 §7 ("LivenessRestartGovernor")** — **superseded**: single
  authority replaces the envisioned separate governor. The dead-reserved
  `BackoffCause::LivenessBudget` (forward-compat wire shape) is left in
  place (not churned) but its intended consumer no longer exists.
- **ADR-0086** — amended to 4 read-ports; the `RestartBudgetView` port and
  its § Compliance "a reconciler may read another reconciler's View"
  paragraph are removed. This ADR is its precursor.
- **rkyv schema evolution** — `StoppedBy::LivenessProbe` is an additive
  fieldless tail variant on an all-fieldless enum; `AllocStatusRow` embeds
  it via `TerminalCondition` / `TransitionReason`, but the variant adds no
  layout size (no field, no max-variant growth), so **existing golden-bytes
  fixtures decode UNCHANGED** — do NOT touch/re-mint `FIXTURE_Vn` (that
  collapses the evolution signal, per `.claude/rules/development.md` §
  "rkyv schema evolution"). **Add** one new golden fixture covering the
  `LivenessProbe` disposition.

## Migration slice sketch (precursor to ADR-0086; single-cut, no red intermediate)

Not a roadmap.json (a later `/nw-roadmap` step). This behaviour change is
its **own** arc, landing BEFORE ADR-0086's crate-move so the hydration
move is already cross-read-free.

**PA — single restart authority (this ADR).** One single-cut arc:

1. Add `StoppedBy::LivenessProbe` (tail variant) + the `by_reclaims_platform
   => false` arm + the `human_readable` arm + any exhaustive `StoppedBy`
   match sites; **ADD** one new `AllocStatusRow` schema-evolution golden
   fixture covering the `LivenessProbe` disposition — existing `FIXTURE_Vn`
   are **untouched** (fieldless tail variant, no layout growth; touching a
   prior fixture collapses the evolution signal, per § Compliance).
2. Add `WorkloadLifecycle::is_liveness_killed`; make the ceiling-exhaustion
   terminal cause-aware (D4); extend the idempotency guard to both
   terminals. Restart-emission + `restart_counts` unchanged.
3. `ServiceLifecycle`: liveness branch emits `StopAllocation { terminal:
   Stopped { by: LivenessProbe } }`; delete the budget-composition +
   `restart_count >= CEILING` finalize branch; retain the threshold-reached
   counter until exact-unrouted/mismatch and hydrate the exact terminal/route
   facts through ADR-0086's existing port. Add the exact `status_updated_at`
   fact and serde-defaulted `liveness_attempt` View map; filter liveness probe
   rows by exact V2 `alloc_attempt`; reset old-attempt counter+marker before
   action selection on the first Running tick. ADR-0048/0054 carry the logical
   attempt through the changed existing hook/runner signatures and compare it
   before wall time at the latest-row store.
4. Delete the cross-read + dead vocabulary (D7): the
   `restart_status_for_alloc` **call** in `hydrate_service_alloc_facts`
   (`:3419`) + `liveness_restart_spec` build (**keep the method** — its
   four `streaming.rs` callers stay), `ServiceAllocFact.{restart_count,
   restart_spec}`, `RestartReason`, `Action::RestartAllocation.reason`.

   **Tests (own to this ADR):**
   - **Preserved-liveness-terminal** (the crux): a Service alloc in a
     liveness loop that exhausts the budget → final
     `AllocStatusRow.terminal == ServiceFailed { LivenessProbeFailed { .. } }`,
     NOT `BackoffExhausted`; a crash loop → `BackoffExhausted` (both
     distinguished on the same alloc shape).
   - **Frozen-batch attempt handoff**: queue Service exact+routed and
     WorkloadLifecycle for one real drained batch; Service tail removes the
     route, Workload restarts the same id, and the following Service tick must
     reset to the new logical status identity, ignore old-attempt probe data,
     emit no Stop, and leave restart budget unchanged. Repeat with equal
     old/new `started_at`, millisecond collision, and clock rollback behind the
     old probe. A lower/equal-wall-time probe bearing the new dominating
     attempt must win LWW and count from one. Reverse evaluation order and cover
     V1 None/older/same/newer attempt complements.
   - **Budget-unification**: interleaved crash + liveness on one alloc draw
     ONE `restart_counts` pool → total restarts capped at `CEILING` across
     both causes (the kubelet shape).
   - **Liveness-not-exempt**: N liveness kills consume budget and exhaust
     (contrast: N platform-reclaims do not — the exemption still holds).
   - **DST trajectory (Tier-1)**: under `SimClock`/`SimDriver`/
     `SimObservationStore`, inject liveness-Fail `ProbeResultRow`s, advance
     the clock across backoff windows, assert the terminate→restart→…→
     `ServiceFailed{LivenessProbeFailed}` trajectory and `restart_counts ==
     CEILING`. Replay-equivalent under the seed.
   - **Cross-read-gone**: the `restart_status_for_alloc` call in
     `hydrate_service_alloc_facts` and `ServiceAllocFact.restart_count` are
     absent — proven by green build; the `.claude/rules/reconcilers.md`
     "single restart authority" symptom (a reconciler reading another's
     View *during hydration*) has no site left. (The method itself
     survives for the streaming attempt-index — that is the operator-event
     layer, not a reconciler read.)
   - **Mutation gate** (per `.claude/rules/testing.md` — reconciler logic
     is a mandatory target): the `is_liveness_killed` predicate and the
     terminal-selection branch in `WorkloadLifecycle` must be killed.

**PB — ADR-0086 S1–S4, now cross-read-free (4 ports).** The hydration
crate-move proceeds with **no `RestartBudgetView`**: S1 adds four
read-ports; `HydrationContext` carries four; `ReconcilerRuntime`
implements **no** read-port; S4 adds four `Sim*` impls. See ADR-0086
(amended) § D5 / § "Migration slice sketch".

**Why PA-first is load-bearing.** If PB went first, ADR-0086 would build
`RestartBudgetView` + `SimRestartBudgetView` only for PA to delete them —
wasteful and self-contradictory (a port whose whole justification is a
cross-read this ADR removes). PA-first means the hydration move never has
to port a cross-read that no longer exists.

## References

- `.claude/rules/reconcilers.md` § "Single restart authority" (the rule
  this ADR implements).
- `docs/research/architecture/reconciler-state-ownership-and-hydration-comprehensive-research.md`
  RQ2 (cross-controller = watch the shared object) / RQ3 (single-owner
  restart authority; the kubelet unified-budget precedent) / § Synthesis
  Decision 2.
- ADR-0086 (amended 2026-08-25 — 4 read-ports; this ADR is its precursor),
  ADR-0037 (`TerminalCondition` / action-shim single-writer), ADR-0078
  (`restart_counts` vs `CrashFacts.restart_count`), ADR-0081/0083
  (`StoppedBy` / `is_platform_reclaimed`), ADR-0054/0055/0080 (probe
  subsystem + roles), ADR-0084 (`interests()` wakeup).
- Code pinned: `crates/overdrive-core/src/service_lifecycle.rs`
  (`liveness_restart_action` :753-820, `:790`; `ServiceAllocFact`
  `restart_count` :205 / `restart_spec` :215);
  `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs`
  (restart-budget branch :687-843, ceiling :709, `BackoffExhausted` :733,
  `is_restartable`/`is_intentionally_stopped` :1151-1175);
  `crates/overdrive-core/src/transition_reason.rs` (`StoppedBy` :393,
  `is_platform_reclaimed` :1071, `ServiceFailureReason::LivenessProbeFailed`
  :726); `crates/overdrive-control-plane/src/reconciler_runtime.rs`
  (`restart_status_for_alloc` :499 — **retained** for streaming;
  `hydrate_service_alloc_facts` :3289, the removed restart_count join
  :3411-3446); `crates/overdrive-control-plane/src/streaming.rs`
  (`restart_status_for_alloc` callers :398/:438/:492/:544 — retained
  attempt-index consumers);
  `crates/overdrive-control-plane/src/action_shim/mod.rs` (`StopAllocation`
  executor :1967-1991, `reason = Reconciler` hardcode :1973 — unchanged;
  `RestartAllocation` ignores `reason: _` :1626);
  `crates/overdrive-worker/src/probe_runner/` (observation-producer, NOT
  terminator).

## Changelog

- 2026-08-25 — Initial accepted version. Precursor to ADR-0086 (5→4
  ports). Implements `.claude/rules/reconcilers.md` § "Single restart
  authority"; supersedes ADR-0055 §7 "LivenessRestartGovernor".
- 2026-08-31 — TRC-ARCH-002 amendment: liveness Stop keeps its existing
  threshold counter until exact terminal+route convergence, using only two
  actual-fact inputs from ADR-0086's route snapshot. No second restart owner,
  action receipt, or new port.
- 2026-08-31 — TRC-ARCH-003 amendment: same-id attempt identity is the
  accepted Running row's existing logical `updated_at`. One serde-defaulted
  View input resets the old counter before dispatch; hydration requires an
  exact-attempt ProbeResultRow V2 and its LWW compares attempt before wall
  time. Frozen-batch Service/Workload order and equal/rolled-back clocks are
  irrelevant without a receipt, broker barrier, or restart-owner change.
