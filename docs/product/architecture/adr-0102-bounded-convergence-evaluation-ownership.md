# ADR-0102 — Bounded ownership of complete convergence evaluations

## Status

**APPROVED — independent DESIGN review APPROVED; user ratification APPROVED**, 2026-09-10.
[Review iteration 2](../../feature/vm-lifecycle-latency/design/review.md#iteration-2--remediation-re-review)
closed F-01. The user ratified this design and its behavioral decisions on
2026-09-10; implementation remains outstanding.
Feature [vm-lifecycle-latency](../../feature/vm-lifecycle-latency/feature-delta.md),
issues #283/#260. Accepted narrow amendment to ADR-0013 §8 and ADR-0035 §5;
does not replace accepted persistence, hydration, reclamation or ending policy.

**User-authorized ad-hoc implementation amendment, 2026-09-12:**
[Retry eligibility](#amendment-2026-09-12--retry-eligibility) below corrects the
proven no-action re-admission loop on PR #290. The user directed an architect
to pin this bounded design and a crafter to implement it immediately afterward.
No independent DESIGN review of this amendment has occurred. Its exact
interface and eligibility clauses govern that authorized implementation;
the original approval above records the earlier design only.

## Decision and actual ownership keys

Extend the current `EvaluationBroker` and `spawn_convergence_loop`. The owner
admits at most **eight complete evaluations**, with at most **one active
`TargetResource`** across all reconciler names. Admission precedes hydration;
ownership ends only after the owner consumes the runtime's result, including
its awaited shim dispatch and existing self-re-enqueue. Use owned futures in
the existing convergence task; no detached driver calls or per-key task actors.

| Current target / entry | Required scheduling interpretation |
|---|---|
| `workload/<WorkloadId>` from HTTPS handlers and accepted observation routing | One workload lane across WorkloadLifecycle, ServiceLifecycle, SvidLifecycle and other evaluations naming that target. Their aggregate snapshots and Views are not allocation-key records. |
| `service/<ServiceId>` from ServiceLifecycle to service-map-hydrator | One separate service projection lane; no new allocation lifecycle authority. |
| `node/<NodeId>` for VmReclamation resync | One node evaluation lane; existing `try_begin_reclamation` and write-time guards arbitrate individual allocation effects. No node-wide lock over workload lanes. |
| Other existing workflow/target values | Same exact-target exclusivity; no new interpretation, routing vocabulary or View key. |

This is deliberately conservative where allocations share one workload View.
Distinct workload targets are independent admission candidates. Current
WorkloadLifecycle stops all Running rows for its workload, and in the Run
branch a Running allocation suppresses fresh placement (`workload_lifecycle.rs:
547–575,659–711`). Splitting these evaluations by allocation would require
different hydration and View ownership, outside the proven need. Do not infer
an allocation ID by parsing a target or assume all target strings are workloads.

VmReclamation already holds a driver allocation claim around its whole
kill/discard/write operation (`action_shim/reclamation.rs:50–80,168–177`),
and VmDriver acquires its Starting claim before host effects. Its node-scoped
evaluation may run beside workload evaluations without bypassing these gates.
The independent exit observer is not enrolled in a scheduling lease; existing
LWW and accepted-session authorship remain authoritative. Workflow emit-drain
is likewise its existing separately owned action path; this ADR does not
claim to serialize arbitrary action producers that do not use the broker.

## Admission, coalescing and fairness contract

- Pending identity remains `(ReconcilerName, TargetResource)`. A duplicate
  replaces only pending work and preserves its original queue age/order;
  it never cancels an active effect. While a key runs, at most one later
  evaluation for that key remains pending and hydrates latest state later.
- Admission is FIFO among eligible pending keys, skipping any active target.
  Within one admission result, targets are distinct. New self-requeues join
  behind previously pending eligible work. A blocked target cannot obstruct
  free capacity for another target. Starts, stops, health and resync share
  the same eight slots; no priority lane or reserved stop slot.
- Pending evaluations stay in the existing broker until actually admitted;
  no second pending queue or scheduler persistence store. Bounded concurrency
  limits active work, not the number of distinct operator-submitted targets.
  Existing cancelable-record/reaper behavior remains unchanged.
- Queue timestamps are explicit inputs from the existing `Clock`; deterministic
  tie/order is submission order under the existing broker lock. Preserve ordered
  collections. Coalescing must not reset age or permit lexical-key starvation.
- Admission consumes capacity only when the entire evaluation can run. A
  result is consumed even when it is `Err(ConvergenceError)`; the owner logs it
  and releases the target/capacity. Runtime and shim retain their current error
  handling; the scheduler does not retry a detached action independently.
- With a finite eligible queue and at least one repeatedly available slot,
  every pending target is eventually admitted. Eight stalled operations may
  fill all capacity until their existing owner deadlines complete; this ADR
  makes no unlimited-capacity or universal-driver-liveness promise.

## Exact interface contracts

Only these public signatures change. `Instant`/`Duration` mean `std::time`
types; collections are `std::collections`. All new broker metadata is private,
process-local and discarded with the broker; `Evaluation`, `BrokerCounters`,
TargetResource, Action, TickContext, View and persisted/wire shapes do not change.

| Existing interface | Exact proposed signature |
|---|---|
| `EvaluationBroker::submit` | `pub fn submit(&mut self, eval: Evaluation, now: Instant)` |
| `EvaluationBroker::drain_pending` | `pub fn drain_pending(&mut self, limit: usize, blocked_targets: &BTreeSet<TargetResource>, now: Instant) -> Vec<(Evaluation, Duration)>` |
| `InterestRouterBroker::from_runtime` | `pub fn from_runtime(runtime: Arc<reconciler_runtime::ReconcilerRuntime>, clock: Arc<dyn Clock>) -> Self` |
| `InterestRouterBroker::from_shared_broker` | `pub fn from_shared_broker(broker: Arc<parking_lot::Mutex<EvaluationBroker>>, clock: Arc<dyn Clock>) -> Self` |
| `action_shim::enqueue_evaluation::dispatch` | `pub fn dispatch(action: &Action, broker: &mut EvaluationBroker, now: Instant)` |

`submit` retains the first pending submission's `now` through replacement.
`drain_pending` returns no more than `limit` distinct eligible targets, oldest
eligible first; each Duration is `now.saturating_duration_since(first_pending_at)`.
`limit == 0`, no pending work or all targets blocked produces an empty result
without mutation. `queued` still counts pending evaluations, `dispatched`
increments only by actual admissions, `cancelled` counts superseded pending
records; the cancelable vector and `reap_cancelable` behavior are unchanged.
No second drain/submit method is added to preserve a stale signature.

Interest-router constructors capture the existing clock in their submit-only
closure; the router gains no read/drain capability. Reclamation's existing
clock parameter, handlers' AppState clock, runtime self-re-enqueue clock and
shim's existing clock supply submission time at the actual submit call. Cadence
submissions use the owner's current clock snapshot. Existing helper signatures
not listed stay unchanged. All direct consumers, including pure/default-lane
broker tests and Sim harness compatibility callers, receive mechanical fallout;
do not change unrelated observation-router `drain_pending` methods.

These signatures remain exactly unchanged:

```
pub async fn run_convergence_tick(
    state: &AppState,
    reconciler_name: &ReconcilerName,
    target: &TargetResource,
    now: Instant,
    tick_n: u64,
    deadline: Instant,
) -> Result<(), ConvergenceError>

fn spawn_convergence_loop(
    state: AppState,
    clock: Arc<dyn Clock>,
    cadence: Duration,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<()>
```

The private policy constant is `CONVERGENCE_MAX_IN_FLIGHT: usize = 8` in the
existing convergence-owner module. No ServerConfig field, CLI flag, public
scheduler trait, public result enum or new persistence type is introduced.
The runtime's integration-only network-provisioner seam remains unchanged.

## Timing, completion and shutdown

The owner continues cadence/resync processing while evaluations await I/O and
can refill released capacity immediately. New submissions are discovered
within the existing cadence when capacity is free; no broker-notification
protocol is needed. Resync remains declaration-driven and anchored to its
existing next-wake policy. Do not hold the broker guard over an await.

Snapshot `now` at each admission, set `deadline = now + cadence`, and give each
admission the next monotonic `tick_n` (not one shared tick number per concurrent
batch). `TickContext.deadline` remains a pure reconciliation budget hint. It
does not cancel hydration, View fsync, driver calls, cleanup or terminal writes.
Existing VM boot/request/grace deadlines retain their separate owners.

Preserve runtime order: hydrate → clone this View → pure reconcile → durable
View write → hot View insert → awaited action dispatch → existing has-work
re-enqueue, including its error path → result consumption → lease release.
Different targets must never overwrite each other's View entries. Existing
short hydration locks are not widened across dispatch or redesigned here.

On the existing shutdown token, close admission and await every admitted
evaluation to its real result. Do not abort effect futures or release permits
early. Leave nonadmitted work unexecuted; submissions and self-requeues arriving
during drain also stay pending. Once every admitted result has been consumed
(immediately if none are active), the convergence owner takes one snapshot of
the existing broker's `counters().queued`
under its existing lock. That count is `pending_at_exit` in the owner's single
`convergence.drain.completed` event, emitted after releasing the lock and before
the convergence task returns. “Remaining pending work” in this report means
the coalesced pending entries at that locked read, not allocation identities or
a final server-shutdown backlog. Each pending `(ReconcilerName, TargetResource)`
key counts once, including submissions during drain that precede the snapshot.

A submission linearized after that snapshot, including one before the event
is emitted or while later producer joins are pending, is outside the reported
count. It remains process-local pending work, is not admitted by the closed
convergence owner, and is released with ordinary broker teardown. The event is
not revised and there is no second/final pending-work report or additional
producer barrier. Committed intent retains its existing owner; neither counted
nor later pending work is reported as completed. No backlog drain or new
restart/recovery subsystem is introduced. The nonadmitted shutdown disposition
is the user-approved change from old all-pending batch admission, ratified
on 2026-09-10.

After convergence join, preserve `ServerHandle::shutdown` order: workflow
emit-drain join → interest-router join → HTTP drain/server join → exit-observer
cancel/join → remaining DNS/worker/resolver teardown. Exit observers remain
live while active evaluations drain. A **consumed** observer event finishes its
existing retry sequence (four attempts, 50/100/200 ms backoff); existing biased
cancellation does not promise draining every unread queued event. This ADR does
not add that stronger guarantee. The supplied HTTP `drain_deadline` is not a
lifecycle timeout. No new per-evaluation Tokio task means no new per-evaluation
JoinError or detached-panic recovery protocol; preserve the existing owner task
failure boundary rather than claiming a panic was successful completion.

## Observability

Use existing tracing, not a new telemetry backend. Pin events/fields:

| Event name | Fields |
|---|---|
| `convergence.evaluation.admitted` | `reconciler`, `target`, `tick`, `queue_ms`, `active`, `capacity` |
| `convergence.evaluation.completed` | `reconciler`, `target`, `tick`, `elapsed_ms`, `outcome` (`ok`/`error`), existing structured `error` when present |
| `convergence.drain.completed` | `elapsed_ms`, `admitted_at_close`, `completed_during_drain`, `pending_at_exit` (coalesced pending-entry count at the convergence-owner exit snapshot defined above; excludes later submissions) |

Durations use the injected clock and saturating elapsed conversion; `active`
is the count after admission, `capacity` is eight. Existing broker counters
provide queue depth. Events summarize owner facts, not a replacement state
store or new public health status.

## Changed Assumptions

| Superseded wording, quoted exactly | Accepted amendment |
|---|---|
| ADR-0013 §8: “`drain_pending()`: empties `pending`, dispatches each via the runtime's invocation path → `dispatched++`.” | Drain only eligible work up to free capacity; retain blocked/excess work and queue age in the same broker. Existing counter fields remain. |
| ADR-0035 §5: “for evaluation in broker.drain_pending():” | Retain the per-evaluation durable order, with bounded concurrent evaluation owners and exclusion on the existing target. This does not repartition the View. |
| Current owner documentation `lib.rs:3286–3290`: “Cancellation via `shutdown` is observed in `tokio::select!` between ticks so an in-flight dispatch always completes before exit.” | Close admission when cancellation is observed; drain every admitted evaluation. Nonadmitted pending work is explicitly not drained. |

ADR-0035's “Step ordering 7 → 8 is load-bearing” and ADR-0013's advisory
deadline rationale are retained, not superseded. ADR-0086's current hydration
ownership and ADR-0100's accepted-session watcher gate remain unchanged.

## Alternatives and validation

Preferred: extend the fixed-capacity owner above. Per-allocation actors would
need retirement/queue ownership and conflict with existing workload Views;
an inner driver pool leaves the outer serial await and fails the established
liveness requirement. Unkeyed concurrent evaluation permits overlapping Views
and competing same-workload actions. These alternatives are not implemented.

The feature delta's G-1 and executable-obligation table are the blocking
acceptance contract: seed 283001, saturation/fairness/coalescing, same-target
View/terminal complements, real shutdown drain, native concurrency and full
E09 v2. No performance improvement is claimed until those lanes pass.

## Amendment 2026-09-12 — Retry eligibility

### Premise and bounded scope

Observed production facts at `bd7946e2a82e3df48b767ecd1b8d3d79bb475dd0`:

- `run_server` (`lib.rs:1766`) → `run_server_with_obs_and_drivers` composes the owner at
  `overdrive-control-plane/src/lib.rs:3064`; the interest router submits into
  that same runtime broker (`lib.rs:3071–3098`). The owner admits before
  hydration (`lib.rs:3362–3403`), consumes completion and releases the target
  (`lib.rs:3428–3445`), then immediately scans for more work.
- `run_convergence_tick_inner` hydrates and reconciles, persists the next View,
  awaits dispatch, and re-enqueues before returning the dispatch outcome
  (`reconciler_runtime.rs:1399–1612`). `persist_view` retains its existing
  equality-based write elision (`reconciler_runtime.rs:549`); no-op spinning
  repeats hydration, computation and owner events, not necessarily fsync.
  `view_has_backoff_pending` folds three policy families into a boolean
  (`reconciler_runtime.rs:1635–1730`). The broker has no eligibility time
  (`overdrive-core/src/eval_broker.rs:81–151`).
- `SvidLifecycle` records retry inputs before dispatch
  (`overdrive-reconcilers/src/svid_lifecycle.rs:471–496`). A CA/audit failure
  returns before the hold is installed
  (`overdrive-control-plane/src/action_shim/issue_svid.rs:96–104`). The next
  reconciliation sees the same unheld allocation before its retry deadline,
  emits only `Noop`, and retains retry memory. The boolean keeps it queued;
  immediate refill admits it again without time advancing. No forced
  cancellation or hypothetical task owner is needed for this path.
- The supplied production-owner Sim reproduction, seed **283001**, held the
  injected clock before the retry deadline and failed after **three admissions
  in 0.21 seconds**. This is the reproduction reported with this design task,
  not a new execution claimed here. The production path above was re-read.
  Retain and rerun the regression during implementation; a broker-only test
  is not its replacement.

This amendment changes admission of no-action transitional requeues while
preserving existing cadence discovery of external submissions. Retry budgets, startup verdicts, eight-slot
capacity, driver/reclamation ownership, persistence, and the separately fixed
guest shutdown spin retain their contracts. Configurable concurrency is out
of scope.

### Exact interface and ownership contract

Keep one pending map in `EvaluationBroker`. Eligibility belongs to a pending
`(ReconcilerName, TargetResource)` key and is independent of first-submission
time and FIFO ordinal. Only an **active evaluation** owns target-wide exclusion;
a deferred key reserves neither its target nor a capacity slot.

Add this public type only in `overdrive_core::eval_broker`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationEligibility {
    Immediate,
    NotBefore(UnixInstant),
}
```

It is transient data without serde/rkyv or a crate-root re-export. `UnixInstant`
is the existing core type; `Instant`/`Duration` remain `std::time` values.
`Evaluation`, `TickContext`, every View and every persisted/wire shape stay
unchanged. Only the following public interfaces change or are added:

| Owner / interface | Exact signature |
|---|---|
| Replace `EvaluationBroker::submit` | `pub fn submit(&mut self, eval: Evaluation, now: Instant, eligibility: EvaluationEligibility)` |
| Replace `EvaluationBroker::drain_pending` | `pub fn drain_pending(&mut self, limit: usize, blocked_targets: &BTreeSet<TargetResource>, now: Instant, now_unix: UnixInstant) -> Vec<(Evaluation, Duration)>` |
| Add `EvaluationBroker::next_eligible_at` | `pub fn next_eligible_at(&self, blocked_targets: &BTreeSet<TargetResource>) -> Option<UnixInstant>` |
| Add default-provided pure `Reconciler::next_evaluation_at` | `fn next_evaluation_at(&self, desired: &Self::State, actual: &Self::State, next_view: &Self::View, tick: &TickContext) -> Option<UnixInstant>`; default returns `None` |
| Add `AnyReconciler::next_evaluation_at` forwarding in `overdrive-reconcilers` | `pub fn next_evaluation_at(&self, desired: &AnyState, actual: &AnyState, next_view: &AnyReconcilerView, tick: &TickContext) -> Option<UnixInstant>` |

In particular, `reconcile`'s tuple, `run_convergence_tick`, its existing
integration-only provisioner variant, `spawn_convergence_loop`, interest-router
constructors, shim dispatcher and runtime broker accessors keep their shapes.
No parallel submit/drain API, scheduler trait, runtime result enum, clock port
extension, notification API/state, or per-key task is authorized.

Call the hook only after an evaluation produces **no non-`Noop` action**, with
its hydrated desired/actual, returned next View and original TickContext. It
returns the earliest strictly future wall-clock boundary requiring time-driven
reconsideration. `None` means no time-driven reconsideration from these inputs;
it never suppresses external submissions. The hook cannot hydrate, read a clock,
mutate a View/store, or dispatch. Erased forwarding follows the same compatible
variant matches and programmer-error mismatch behavior as existing
`AnyReconciler::reconcile`, without lifecycle policy. Implementers share their
existing private decision predicates/deadline arithmetic between reconciliation
and this hook where necessary. Runtime and broker do not duplicate that policy.

### Reconciler-owned time boundaries

All three families currently recognized by `view_has_backoff_pending` move to
the hook. Remove that central predicate and any helper left without a
production consumer; retain View fields and their persisted input semantics.

| Owning reconciler | Exact no-action scheduling decision |
|---|---|
| `SvidLifecycle`, first issuance retry | For each still-desired Running allocation which is unheld, has no `ever_issued` success fact, and has retained retry memory, use `last_failure_seen_at + backoff_for_attempt(attempts)`. Successful held and non-Running allocations contribute none. |
| `SvidLifecycle`, rotation retry | For each still-desired Running allocation held within the existing near-expiry window with retained retry memory, use `min(last_failure_seen_at + backoff_for_attempt(attempts), held.not_after - ROTATION_DEADLINE_MARGIN)`. Subtraction uses saturating epoch-duration arithmetic through existing `UnixInstant::as_unix_duration` / `from_unix_duration`, not a new public operator. Preserve the existing inclusive clamp and private margin. Return the minimum future deadline across first-issuance and rotation candidates. |
| `WorkloadLifecycle`, restart backoff | Use the candidate actually selected by the current Run branch, after stop/absent, generation, Running, operator-stop and Job-natural-exit decisions. If restart is suppressed only by backoff, return `last_failure_seen_at[candidate] + backoff_for_attempt(restart_counts[candidate])`. Apply the current ordinary exhaustion rule and existing platform-reclamation exemption. Do not scan unrelated historical View entries or let an earlier historical deadline override the selected candidate. |
| `ServiceLifecycle`, startup window | Among currently hydrated allocations in `next_view.observed` and neither announcement set, with `started_at`, no startup Pass, and state other than `Terminated`, use the earliest future window end. The end is `started_at + Duration::from_millis(u64::try_from(startup_deadline.as_millis()).unwrap_or(u64::MAX))`, matching the existing millisecond comparison. At/after that end, reconciliation applies its current attempts/deadline/no-Pass rule. If attempts remain insufficient and no action results, return `None` for that allocation and await its already-declared `ProbeResult`/`AllocStatus` interests and periodic relist. Missing `started_at`, removed allocations and announced/terminated allocations contribute none. |

An absent WorkloadLifecycle restart count means zero, as in the current
decision; an absent failure timestamp means no backoff suppression. The hook
does not index a missing map entry. The production references are `svid_lifecycle.rs:344–512`,
`workload_lifecycle.rs:519–974`, and `service_lifecycle.rs:474–475,485–658,
1278–1325`. Deadlines are recomputed from current inputs on each real
evaluation, never persisted. Stale View membership alone is no longer a
scheduling policy. Other reconciler kinds use the default `None`; this does
not add a new timer policy to them.

Preserve SVID's immediate restart-recovery precedence (`unheld && ever_issued`),
clear-on-success, and rotation panic-zone behavior. Those branches already emit
real actions. After **any** real action, including a failed `IssueSvid`, retain
one immediate confirming self-requeue: it hydrates the effect's actual outcome
and promptly clears successful retry memory. Thus a failed issue can have its
issuing evaluation and one confirming no-action evaluation before its deadline;
the latter must defer, preventing an unbounded admission series. A mixture of
real actions and deferred allocations also confirms immediately. Only an
evaluation with no real action uses the hook.

### Duplicate semantics, admission and waking

`submit` retains existing replacement/cancelable semantics and original pending
age/ordinal. Eligibility merges by **earliest opportunity**: `Immediate` wins
against either value; two `NotBefore` values keep their minimum. Eligibility
merge is commutative, so an external event submitted during an active evaluation
cannot be postponed by its later self-requeue. A later external event promotes
a deferred key without resetting age or changing cardinality. Each duplicate
still increments `cancelled` once and retains the superseded Evaluation for the
existing reaper. New keys receive fresh age/ordinal; admission removes their
pending metadata.

All existing external producers (handlers, accepted observation events, relist,
resync, reclamation and `EnqueueEvaluation` handoffs) submit `Immediate`.
Runtime action-emitted confirming requeues also submit `Immediate`; only a
no-action `Some(at)` submits `NotBefore(at)`. No-action `None` submits nothing.
A requeue never cancels an active effect.

`drain_pending` admits `Immediate`, or `NotBefore(at)` when `now_unix >= at`,
only if the target is neither active nor already admitted in this batch.
Preserve FIFO **among eligible keys**, including an older deferred key's
original position when it becomes eligible. Skipping `(R1,T)` for time must
not block eligible `(R2,T)`. Queue age remains monotonic
`now.saturating_duration_since(first_pending_at)`. Waiting/wake scans change
no counters; only actual admissions increment `dispatched`.

`next_eligible_at` is a read-only minimum over pending `NotBefore` times whose
targets are not active. Ignore `Immediate` entries; return `None` if there are
no candidates. The owner uses it after admission when capacity remains; with
full capacity it does not arm a pending-deadline wake. Excluding active targets
prevents a due but target-blocked key from creating zero-duration wake cycles.

The only new private broker metadata is
`eligibility: EvaluationEligibility` on each existing pending entry. Submission
does not notify the owner. While admission is open, the existing owner selects
between shutdown, active completion and injected clock sleep. Bound its one
sleep by `min(cadence, nearest_pending_deadline - current_unix_time)` when
capacity remains and `next_eligible_at` returns a deadline; otherwise retain
the cadence. The subtraction uses existing saturating `UnixInstant`
arithmetic. On every wake take fresh injected clock snapshots and scan
admission; timer readiness never bypasses `now_unix >= at`.

An external submission promotes the pending key to `Immediate` at submission,
but discovery by a sleeping owner remains within the existing cadence, or
sooner through an unrelated completion or already-armed earlier deadline.
Promotion removes the retry deadline as an admission gate; it does not promise
a new wake or admission at fixed SimClock time. Completion immediately refills
already pending eligible work. Resync and the router's independent relist
retain their existing policies. There is one owner sleep, no notification
protocol, detached/per-key timer or second pending store.

### Lifecycle Gate Ownership and ordering

| Signal/state | Owner and promise | Inputs that may gate it / excluded authority |
|---|---|---|
| Pending key eligibility | Reconciler supplies a time boundary; broker merges incoming work | Exact key's deadline or immediate submission; never another key's deadline |
| Active `TargetResource` | Convergence owner excludes overlapping complete evaluations | Capacity and eligible FIFO admission; a pending key cannot hold an active claim |
| Retry permission / Service startup verdict | Existing reconciler policy | Existing attempts, observed facts and deadlines; broker eligibility neither grants an action nor authors lifecycle state |

**Gate G-2 — pending key admission time.** The production entry/owner path
above is the evidence. The broker owns the admission predicate; passing means
this pending key may begin a complete evaluation, not that its effect succeeds.
Unavailable time leaves it pending without an error, active claim, or counter
change. Deadline arrival is eligibility, not a timeout failure. Duplicate input
merges as specified. No new parse/disconnection error domain is added;
hydration and dispatch keep their typed errors. Late events hydrate current
state and cannot authorize a stale terminal write. Admission close overrides
deadline eligibility and external submission promotion. Process loss discards derived scheduling data;
existing boot/list hydration recomputes from intent, observation and persisted
View inputs, without a new recovery protocol. No feature flag is introduced.

Counterexample: a deferred SVID key and an eligible WorkloadLifecycle
operator-stop key share a workload target. The stop evaluation may start after
the prior active evaluation completes, before SVID retry eligibility. Neither
`Running`, Service `Stable`, terminal authorship nor driver/cleanup claims
acquires a new gate. Seeded production-owner Sim evidence proves scheduling;
no new kernel effect, external integration or adapter probe is introduced.

Compute the optional time before moving `next_view` into persistence. Preserve
the complete order: hydration → View read → pure reconcile and optional pure
time decision → existing durable write/hot insert (or equality elision) →
validation/awaited shim dispatch → self-requeue with chosen eligibility →
unchanged dispatch result → owner result consumption and target/capacity
release. Persistence failure follows its existing path; no submission precedes
persistence. Dispatch errors still pass through requeue before returning.
No post-dispatch hydration or independent retry owner is added.

Shutdown disables admission and timer processing and drains
only admitted evaluations. A deferred deadline or external submit during drain
stays pending; joining does not wait for pending deadlines. Preserve the exact
`pending_at_exit` snapshot/later-submission semantics, the existing guarded
cancellation arm, and server join order in this ADR.

### Changed Assumptions and alternatives

| Superseded wording | Amendment |
|---|---|
| This ADR: “Only these public signatures change.” | The amendment table replaces submit/drain and adds only the pure time hook, erased forwarding and deadline read. No compatibility submit/drain variants. |
| ADR-0067 D8: “Interaction with `view_has_backoff_pending` — still correct, no adjustment.” | Retain D8/D10 retry/clamp policy; replace the central boolean with reconciler-owned no-action time. The quoted assertion depended on the prior cadence-separated owner. |

The original clause “New submissions are discovered within the existing
cadence when capacity is free; no broker-notification protocol is needed”
remains in force.

Selected: extend the existing broker entry and existing owner. Sleeping after
every completion delays unrelated eligible work; retaining a target lease
during backoff delays another reconciler's same-target stop. Both simpler
patches fail the required counterexample. A submission notification protocol
would change the accepted cadence-discovery behavior without being necessary
to fix the reproduced loop. Per-key sleepers or a second deferred queue
duplicate pending ownership. The selected cost is one eligibility field
per entry and scans of the existing pending set; no new
service, store or dependency. System context and containers remain the
architecture brief's existing C4 boundaries; this is internal to control-plane.

### Blocking implementation evidence

Every new/transitioned test carries its per-test Contract Shape declaration.

1. **`convergence_backoff_admission_respects_eligibility`, seed 283001.** Drive
   the real spawned owner and registered SVID reconciler with existing Sim
   driven-port issuance failure injection. With the clock fixed before retry,
   allow the issuing evaluation and one confirming no-action evaluation, but
   no third admission or further hydration without a new input. Retain the
   failing-current-code witness and print its seed. Advance just before, at
   and after the deadline: no early admission, eventual retry at/after it.
   Include successful issuance/clear-on-success and failed rotation/clamp.
2. **`convergence_external_submission_promotes_deferred_key`.** Drive an
   external producer while the key is deferred, then advance SimClock through
   the accepted cadence while remaining before the retry deadline. With
   capacity, prove reconsideration without waiting for that retry deadline.
   Cover external-before-self-requeue and self-requeue-before-external ordering
   and repeated duplicates. Also prove discovery through an unrelated active
   completion before cadence when one occurs; no notification mechanism or
   scan/sleep registration race test is required.
3. **`convergence_deferred_key_does_not_reserve_target`.** A different eligible
   reconciler on the same target proceeds after actual completion, never
   overlaps it and never waits for the deferred key. Include WorkloadLifecycle
   stop/SVID retry and an independent healthy target immediately refilling a
   released slot.
4. Seeded owner coverage also drives the actual WorkloadLifecycle restart and
   ServiceLifecycle startup paths: deadline without Pass, Pass before deadline,
   insufficient attempts after deadline awaiting observations, pre-Running,
   removed/terminated inputs, ordinary exhaustion and the existing reclamation
   exemption. Assert unchanged startup/terminal/restart policy. This covers the
   other existing users of the replaced boolean, not new policy requirements.
5. Broker properties cover earliest-eligibility duplicate merge, complete
   FIFO/age/counter/cancelable deltas, multiple deadlines, due/active targets,
   full/zero capacity and all-deferred scans. Deadline read/cadence scanning is not
   admission. Pure hook properties compare its boundaries with owning
   reconciliation behavior. Source-local pure properties use exactly
   `/// CONTRACT_SHAPE: pure-function.`.
6. **`convergence_shutdown_leaves_deferred_pending`.** Cancel with deferred-only
   and active-plus-deferred work; join without advancing the pending deadline,
   drain active effects to their real results, preserve exact exit-snapshot
   counters and leave during-drain submissions unexecuted.

The bounded-change evidence universe comprises participating broker entries,
owner admissions, hydration/effect records, typed Views and observation/identity
outcomes, with explicit complements for unrelated keys and lifecycle state.
Rust typechecking enforces signature fallout; existing purity checks and seeded
Sim tests enforce the effect boundary. Tests stay in-process, without spawning
the production binary or emitting expectation evidence. This documentation
amendment changes no production/test code and claims no passing regression or
performance result.
