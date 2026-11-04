# ADR-0273: A broker submit wakes the convergence loop at once; the cadence tick stays as the backstop

## Status

**Accepted, 2026-11-03.** Not implemented. GH #318, GH #334. The exact contract (D17), the
interface signatures, the model decision and the DISTILL obligation (S-48) live in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md).
Related: [ADR-0084](adr-0084-reconciler-cadence-and-interest-declarations.md) (the wakeup
model this decision keeps: resync routes through the broker and re-arms at most once
per period), ADR-0035 and ADR-0036 (the reconciler runtime),
[ADR-0270](adr-0270-a-probe-less-service-is-stable-when-the-guest-reports-every-declared-listener-open.md)
(its cold-start claim leans on this wake).

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
  `notified().await` consumes it (tokio 1.52.1, the version `Cargo.lock` pins:
  `sync/notify.rs:627-632`; "only a **single** permit is stored", `:48-49`). A submit that
  lands between the loop's drain and its park is therefore not lost, and any number of
  submits before the loop next parks coalesce into one wake. `notify_waiters` stores no
  permit (`:714`) and loses exactly that submit; it is not used.
- **Signal after the lock.** The submitter takes the broker lock, inserts, releases the
  lock, then signals. The loop never holds the broker lock across an `.await` (existing
  rule), so the wake cannot be observed before the evaluation is pending. A `notified()`
  future dropped by the `select!` after `notify_one` handed it the notification passes it on
  or leaves the permit (`drop_notified`, `sync/notify.rs:1365-1375` @ tokio 1.52.1), so a
  biased `select!` that completes on another arm does not swallow a wake.
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

## Surface

The reconciler runtime gains one public submit method, the only production submit path (it
submits, releases the lock, then signals the wake); a crate-private wait method only the
convergence loop calls; and a public drain-side guard type that exposes the drain operations
and no submit. The broker accessor returns that guard and the raw mutex accessor is removed,
so a submit that forgets to wake is not expressible. The two functions that took the mutex to
submit take the runtime, and the interest router's runtime-backed handle submits through the
new method. `EvaluationBroker` in `overdrive-core` is unchanged and stays free of `tokio`;
the standalone-broker router handle used by router tests is unchanged and does not wake.
The exact signatures are in the feature delta's interface table.

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
- The latency benefit follows from the mechanism (no timer wait on an edge-triggered hop)
  and is not measured here.
