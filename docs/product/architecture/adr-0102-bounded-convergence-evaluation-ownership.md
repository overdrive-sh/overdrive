# ADR-0102 — Bounded ownership of complete convergence evaluations

## Status

**APPROVED — independent DESIGN review APPROVED; user ratification APPROVED**, 2026-09-10.
[Review iteration 2](../../feature/vm-lifecycle-latency/design/review.md#iteration-2--remediation-re-review)
closed F-01. The user ratified this design and its behavioral decisions on
2026-09-10; implementation remains outstanding.
Feature [vm-lifecycle-latency](../../feature/vm-lifecycle-latency/feature-delta.md),
issues #283/#260. Accepted narrow amendment to ADR-0013 §8 and ADR-0035 §5;
does not replace accepted persistence, hydration, reclamation or ending policy.

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
