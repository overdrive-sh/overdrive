# ADR-0273: A broker submit wakes the convergence loop at once; the cadence tick stays as the backstop

## Status

**Accepted, 2026-11-03.** The user ruled "add it" and then accepted the shape choices S1 to
S6 in the feature delta (§ D17) as proposed, on 2026-11-03: S1 `Notify::notify_one`; S2 signal
after the lock is released; S3 one submit path including the loop's own resync; S4 `BrokerDrain`
without `submit`, `broker_mutex` removed; S5 `submit_evaluation` and `BrokerDrain` `pub`,
`submitted` `pub(crate)`; S6 no Quint model (S-48 seeded sim invariant plus mutation). Not implemented. GH #318, GH #334. The exact contract (D17), the
gate statement, the model decision and the DISTILL obligation (S-48) live in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md).
Related: [ADR-0084](adr-0084-reconciler-cadence-and-interest-declarations.md) (the wakeup
model this decision keeps: resync routes through the broker and re-arms at most once
per period), ADR-0035 and ADR-0036 (the reconciler runtime),
[ADR-0270](adr-0270-a-probe-less-service-is-stable-when-the-guest-reports-every-declared-listener-open.md)
(its cold-start claim depends on this wake).

## Context

The convergence loop (`spawn_convergence_loop`,
`crates/overdrive-control-plane/src/lib.rs:4915-5052 @ 04052ad75`) is one task. Each
iteration submits the due resync evaluations, calls `EvaluationBroker::drain_pending`,
admits the eligible evaluations as concurrent futures, and then parks in a `select!`
with three arms: shutdown, the completion of an in-flight evaluation, and
`clock.sleep(sleep_for)` where `sleep_for` is the tick cadence (`tick_period_ms`,
default 100 ms) or the earliest deferred eligibility, whichever is sooner
(`:5022-5049`, `:4996-5004`). **No arm reacts to a submit.** A repository search for
`Notify` finds none in the loop or the runtime.

Every edge-triggered producer therefore waits for the next loop iteration: the interest
router that fans an accepted `alloc_status` or probe row out to its reconcilers
(`lib.rs:5105-5113`), the workload submit handler (`handlers.rs:75`), the
`EnqueueEvaluation` action (`action_shim/enqueue_evaluation.rs:59`), the reclamation
actions (`action_shim/reclamation.rs:255-274`) and the runtime's own self-re-enqueue
(`reconciler_runtime.rs:1767-1773`). A submit made while an evaluation is in flight is
picked up when that evaluation completes; a submit made while the loop is parked waits
for the cadence timer. The hops of the exec-to-`Stable` chain whose submitter is not an
in-flight evaluation (a row written by the guest-listener probe task, the shim or the
exit observer, routed by the interest router) each lose up to one cadence period before
`reconcile` runs. With the guest-reported readiness of ADR-0270 these hops are the
remaining non-event latency on the cold-start path.

The tick is also the level-triggered backstop of the wakeup model
(`.claude/rules/reconcilers.md` § "The wakeup model"): the design must keep it.

## Decision

**A submit to the evaluation broker wakes the convergence loop immediately. The loop
selects on a wake beside its cadence timer; the cadence tick stays as the backstop.**

- **Primitive: a `tokio::sync::Notify` owned by the reconciler runtime, signalled with
  `notify_one`.** `notify_one` stores one permit when no task is waiting, and the next
  `notified().await` consumes it (tokio `sync/notify.rs:623-629`; "only a single permit
  is stored", `:44-46`, tokio 1.54.1, the version in the local registry; the workspace
  declares `tokio = "1"`). A submit that lands between the loop's drain and its park is
  therefore not lost, and any number of submits before the loop next parks coalesce into
  one wake. `notify_waiters` stores no permit (`:710-712`) and loses exactly that
  submit; it is not used.
- **Signal after the lock.** The submitter takes the broker lock, inserts, releases the
  lock, then signals. The loop never holds the broker lock across an `.await` (existing
  rule), so the wake cannot be observed before the evaluation is pending. A `notified()`
  future dropped by the `select!` after `notify_one` handed it the notification passes it on
  or leaves the permit (`sync/notify.rs:1361-1371`), so a biased `select!` that completes on
  another arm does not swallow a wake.
- **One drainer, unchanged admission.** The loop remains the only caller of
  `drain_pending`. Oldest-first order (the broker's FIFO over a `BTreeMap`), the
  one-evaluation-per-target rule (`active_targets`), the capacity cap
  (`CONVERGENCE_MAX_IN_FLIGHT`), deferred eligibility (`NotBefore`) and the per-key
  last-writer-wins collapse are untouched. A wake only causes the next iteration to run
  now instead of at the timer.
- **Time stays the injected `Clock`.** The cadence arm is `clock.sleep`, driven by
  `SimClock` under DST. The wake arm has no clock and no production-only counterpart for
  the simulator: there is no arm that exists only for DST.
- **No busy loop.** The loop parks only on shutdown, a completion, a wake or the timer.
  Each wake is one stored permit consumed by one iteration, so the number of iterations
  is at most the number of submits plus the number of cadence ticks plus the number of
  completions. An iteration that can admit nothing (every pending target blocked by an
  in-flight evaluation) parks again and is woken by that evaluation's completion, as
  today.
- **Resync is unchanged (ADR-0084).** The due resync evaluations are still submitted
  through the broker at the top of each iteration and re-armed at most once per period.
  The loop's own resync submit and the runtime's self-re-enqueue (made inside an
  in-flight evaluation, which the completion arm already picks up) also go through
  `submit_evaluation`, so there is one submit path. The cost is at most one extra,
  cheap iteration per such submit (the stored permit is consumed at the next park and
  the drain finds the target already admitted or blocked); it adds no new
  self-perpetuating loop, because the loop never submits in response to a wake.

## Named surface (new public and runtime surface)

- `ReconcilerRuntime::submit_evaluation(&self, eval: Evaluation, now: Instant,
  eligibility: EvaluationEligibility)`: submits, releases the lock, signals the wake.
  This is the **only** production submit path.
- `ReconcilerRuntime::submitted(&self)`: a `pub(crate)` async method that completes when a
  wake is stored or delivered and consumes it. Only the convergence loop calls it.
- `ReconcilerRuntime::broker()` returns `BrokerDrain<'_>`, a new `pub` guard newtype over
  the mutex guard that exposes the drain-side operations
  (`drain_pending`, `next_eligible_at`, `counters`) and **no `submit`**; `broker_mutex()`
  is removed. The two functions that took the broker mutex to submit
  (`execute_reclaim_allocation`, the `EnqueueEvaluation` dispatch) take the runtime, and
  `InterestRouterBroker::from_runtime` submits through `submit_evaluation`. A submit
  that forgets to wake is thereby not expressible. `EvaluationBroker` in
  `overdrive-core` is unchanged and stays free of `tokio`.
  `InterestRouterBroker::from_shared_broker` (a standalone broker with no loop, used by
  router tests) is unchanged and does not wake.

## Alternatives considered

- **Shorten the cadence.** A shorter timer still floors the latency at its period and
  spends CPU on every idle tick on every node; it does not remove the hop.
- **Each producer drains inline** (the interest router calls the drain). A second
  drainer breaks per-target serialization and the single admission order.
- **`notify_waiters`, or a hand-rolled flag checked before parking.** Both leave the
  window between the drain and the park in which a submit is lost until the next
  cadence tick; the stored permit closes it with no flag to keep consistent.
- **A bounded `mpsc` channel of capacity one.** Coalesces like the permit but adds a
  second queue next to the broker for the same fact.
- **A `watch` on the broker's `queued` counter.** More machinery than a unit signal and
  it wakes on drains too.
- **Leave the tick.** Costs up to one period per non-event hop on the path the user has
  asked to make millisecond-scale.

## Consequences

- Positive: an edge-triggered submit is reconciled at once; the exec-to-`Stable` path
  loses its last non-event waits. The backstop and every ordering guarantee stay.
- Positive: a wake storm cannot starve the cadence arm or the resync schedule (both are
  evaluated at the top of every iteration, and resync re-arms once per period).
- Rejected for the model: a Quint hazard for a lost wake would model the rejected
  `notify_waiters` alternative; see the feature delta, § D17, S6.
- Negative: three runtime signatures change and the broker accessor is split; the tests
  that submit through `broker()` or assume that nothing is drained until the clock is
  advanced are rewritten (DISTILL confirms the set).
- Negative: `.claude/rules/development.md` § "Reconciler I/O" describes the steady-state
  tick as the drain period; at landing that sentence needs the wake added (not edited
  here).
- Not measured here: the latency benefit. V40 measures exec-to-`Stable` with and
  without the wake (a benchmark, not a gate).
