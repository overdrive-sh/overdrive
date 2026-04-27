# ADR-0023 — Action shim placement: `reconciler_runtime::action_shim` submodule, 100 ms tick cadence in production, DST-driven ticks under simulation

## Status

Accepted. 2026-04-27. Decision-makers: Morgan (proposing), user
ratification 2026-04-27. Tags: phase-1, first-workload,
application-arch.

## Context

ADR-0013 establishes the `Reconciler` trait as a pure synchronous
function, with all I/O hosted by the surrounding runtime. The trait
emits `Vec<Action>`; the runtime is responsible for executing those
actions. ADR-0013 §2b enumerates the per-tick pipeline:

```
1. Pick reconciler from registry by name
2. Open (or reuse) LibsqlHandle for name
3. tick <- TickContext::snapshot(clock)
4. view <- reconciler.hydrate(target, db).await
5. (actions, next_view) =
       reconciler.reconcile(&desired, &actual, &view, &tick)
6. Persist diff(view, next_view) to libsql
7. Dispatch actions to the runtime's action shim   ← THIS ADR
```

`.claude/rules/development.md` § Reconciler I/O reinforces the
discipline: external calls become `Action::HttpCall` data; responses
arrive via the `external_call_results` observation table on the next
tick. The pattern matches Anvil (USENIX OSDI '24) `reconcile_core` +
async shim.

The Phase 1 first-workload feature ships three new lifecycle
Actions (`StartAllocation`, `StopAllocation`, `RestartAllocation`)
that need a real shim — not the `Action::HttpCall` runtime that
Phase 3 will deliver, but a parallel allocation-management shim
that dispatches to `Driver::start` / `Driver::stop`. Three decisions
attach to the shim:

- **Where** the shim lives in the source tree.
- **What** its function signature is, and how it consumes
  `Vec<Action>` from the reconciler runtime.
- **When** the shim runs — in production it ticks on a schedule;
  under DST the harness drives ticks explicitly.

The DISCUSS wave (Key Decision 5) pre-decided that the shim is a
new runtime layer in `overdrive-control-plane`. This ADR fills in
the placement, signature, and tick semantics.

## Decision

### 1. Module placement: `overdrive-control-plane::reconciler_runtime::action_shim`

The shim is a submodule of the existing `reconciler_runtime`
module:

```
crates/overdrive-control-plane/src/
├── reconciler_runtime.rs           ← existing; re-exports action_shim
└── reconciler_runtime/
    └── action_shim.rs              ← NEW
```

(The exact filesystem shape is a crafter decision; the *module path*
is fixed: `crate::reconciler_runtime::action_shim`.) This places
the shim where ADR-0013 §1 puts every other piece of runtime
infrastructure — alongside `EvaluationBroker` and
`ReconcilerRegistry`. The shim's natural peers are the broker (which
queues evaluations) and the registry (which holds reconcilers); they
are all part of the same per-tick pipeline.

The shim does NOT live in `overdrive-core`. It performs I/O
(`Driver::start` is async; `ObservationStore::write` is async);
both are banned in `core`-class crates by `dst-lint`. It lives in
the `adapter-host`-class control-plane crate where real-infra calls
are expected and permitted.

### 2. Signature: `dispatch(actions, &dyn Driver, &dyn ObservationStore, &TickContext) -> Result<(), ShimError>`

```rust
// in overdrive-control-plane::reconciler_runtime::action_shim

/// Dispatch a reconciler's emitted `Vec<Action>` against the active
/// driver and observation store. Called by the runtime after every
/// `reconcile` call.
///
/// Per ADR-0013 §2b step 7. The shim is the async I/O boundary
/// that `reconcile` cannot cross — every `.await` in the
/// post-reconcile pipeline lives here.
pub async fn dispatch(
    actions: Vec<Action>,
    driver:  &dyn Driver,
    obs:     &dyn ObservationStore,
    tick:    &TickContext,
) -> Result<(), ShimError> {
    for action in actions {
        match action {
            Action::Noop => { /* nothing */ }
            Action::StartAllocation { alloc_id, job_id, node_id, spec } => {
                match driver.start(&spec).await {
                    Ok(handle) => obs.write(/* AllocStatusRow Running */).await?,
                    Err(e)     => obs.write(/* AllocStatusRow Failed */).await?,
                }
            }
            Action::StopAllocation { alloc_id } => {
                let handle = lookup_handle(alloc_id, obs).await?;
                driver.stop(&handle).await.map(|()| ())?;
                obs.write(/* AllocStatusRow Terminated */).await?;
            }
            Action::RestartAllocation { alloc_id } => {
                /* StopAllocation followed by a fresh StartAllocation;
                   alloc_id changes. */
            }
            Action::HttpCall { … }     => { /* Phase 3 runtime — not shipped here */ }
            Action::StartWorkflow { … } => { /* Phase 3+ workflow runtime */ }
        }
    }
    Ok(())
}
```

Concrete details (the exact `match` body, the alloc-handle lookup
strategy, the `AllocStatusRow` shape with logical-timestamp ordering
per ADR-0012) are the crafter's remit. The architectural contract is:

- The shim is `async fn` — `driver.start(…)`, `driver.stop(…)`, and
  `obs.write(…)` are all `async`.
- The shim takes `&dyn Driver` and `&dyn ObservationStore`, NOT
  `Arc<…>`. The runtime's caller (the dispatch path inside the
  runtime tick loop) holds the Arcs and passes references.
- The shim does NOT take `&LibsqlHandle`. Reconciler libSQL is the
  reconciler's private memory; the shim never reads or writes it.
  Persisted state changes the shim cares about flow through
  `ObservationStore`.
- Every action variant has an explicit arm. Phase 1's five-variant
  enum (Noop + HttpCall + StartWorkflow + the three lifecycle
  variants from US-03) is exhaustively matched. New variants
  produce non-exhaustive-match compile errors at extension time.

### 3. Error handling: `ShimError` enum, `#[from]` pass-through

```rust
#[derive(Debug, thiserror::Error)]
pub enum ShimError {
    #[error("driver failure")]
    Driver(#[from] DriverError),
    #[error("observation write failure")]
    Observation(#[from] ObservationStoreError),
    #[error("alloc handle missing for {alloc_id}")]
    HandleMissing { alloc_id: AllocationId },
}
```

Per `development.md` § Errors: `thiserror`-typed enum,
pass-through `#[from]` embedding, no `eyre::Report` in the public
surface. The runtime's tick loop maps `ShimError` to a tick-failure
record (logged + emitted as observation telemetry); a single bad
action does not crash the runtime.

A driver failure that produces a `DriverError::SpawnFailed` (e.g.
binary missing) is dispatched: the shim writes `AllocStatusRow {
state: Failed { reason } }` and returns `Ok(())` — the failure is
*expected* and *recorded*, not surfaced as `ShimError`. `ShimError`
is reserved for failures the shim itself cannot resolve into an
observation row (e.g. the observation store itself is broken).

### 4. Production tick cadence: 100 ms, runtime-driven

The runtime's reconciler-evaluation loop fires on a **100 ms tick
cadence** in production. Concretely, the runtime spawns a
background `tokio::task` that:

```
loop {
    // Drain the broker for any queued evaluations.
    for eval in broker.drain() {
        run_one_evaluation(eval).await;   // ADR-0013 §2b pipeline + shim
    }
    // Sleep until the next tick.
    clock.sleep(Duration::from_millis(100)).await;
}
```

100 ms is fast enough that operator-visible convergence happens
within human reaction time (a `job submit` → Running roundtrip
consumes 1-3 ticks). It is slow enough that the per-tick I/O budget
(libSQL read, intent-store read, observation-store read +
potentially N writes) does not saturate a development-class
machine. The cadence is configurable via `ServerConfig` —
production defaults to 100 ms; dev / integration tests may pick
faster cadences for tighter feedback loops.

The `clock.sleep(…)` call goes through the injected `Clock` trait,
NOT `tokio::time::sleep` directly. This preserves the DST seam:
under simulation, `SimClock` advances ticks under the harness's
control, not the wall clock.

### 5. Simulation tick cadence: DST harness drives ticks explicitly

Under DST, the runtime's tick task is constructed against
`SimClock`. The harness advances simulated time by exact tick
counts (`sim.advance(Duration::from_millis(100)).await` per tick,
or larger spans to cover N reconciler evaluations). The runtime
loop's body is identical; only the `clock.sleep(…)` resolves
differently — it returns immediately when simulated time has been
advanced past the deadline.

This matches turmoil's documented usage pattern (per ADR-0006 and
whitepaper §21) and preserves the K3 DST property: same seed →
bit-identical trajectory, including the exact set of reconciler
evaluations that ran.

## Alternatives considered

### Alternative A — Action shim in a separate `overdrive-action-shim` crate

Extract the shim into a new top-level crate
(`crates/overdrive-action-shim`).

**Rejected.** The shim's natural peers are `EvaluationBroker` and
`ReconcilerRegistry`, which already live in
`overdrive-control-plane::reconciler_runtime`. Splitting them
across crates introduces a runtime-internal dep edge with no
testability or reusability gain — the shim has exactly one caller
(the runtime tick loop) and reaches into the runtime's per-tick
context. Hosting it in a sibling submodule preserves the runtime's
internal cohesion without growing the crate count.

The scheduler crate extraction (ADR-0024) IS justified separately
— the scheduler is a pure function with a dst-lint enforcement
benefit. The shim has no equivalent benefit; it is `async`, calls
`Driver::start`, and is firmly in `adapter-host` territory.

### Alternative B — Shim is a method on `ReconcilerRuntime`

Add `ReconcilerRuntime::dispatch_actions(&self, actions: Vec<Action>)`
as a method, with the runtime owning the driver via
`self.driver`.

**Rejected.** The runtime would gain a `driver: Arc<dyn Driver>`
field that duplicates the same field on `AppState` (ADR-0022). One
of them would have to be the SSOT, and `AppState` is the natural
owner: `AppState` is already the per-server-instance container for
shared state, and the runtime is constructed *before* the driver is
in scope at the `run_server` boot path. Threading the driver
through the runtime constructor is mechanically identical to
threading it through `AppState`; doing it on `AppState` keeps the
dispatch path explicit (the runtime tick loop reads
`state.driver.as_ref()`) and keeps the runtime free of driver-shape
knowledge.

### Alternative C — Shim runs on a separate task, communicating via channel

The reconciler runtime emits `Vec<Action>` to a `tokio::mpsc`
channel; a separate task (the shim) consumes the channel.

**Rejected for Phase 1.** Channel-based decoupling buys
back-pressure and concurrency, neither of which is a Phase 1 need.
The reconciler runtime tick loop is sequential by design — only
one reconciler evaluates per tick, and the shim's dispatch is
expected to complete before the next tick fires (driver.start
takes milliseconds; the 100 ms tick gives ample headroom). Adding a
channel introduces a queue-depth invariant the runtime would have
to defend (what if the shim is slower than the broker?), additional
cancellation surface, and cross-task error propagation complexity.
Phase 2+ may revisit this if production driver latency turns out
to materially exceed the tick budget.

### Alternative D — Edge-triggered: shim runs immediately when an action is emitted

Instead of a periodic tick, have the runtime invoke the shim
synchronously in the same task as soon as `reconcile` returns.

**Accepted in shape.** This is what decision 4 above actually does
— the per-evaluation pipeline calls the shim in-line at step 7.
The "100 ms tick cadence" describes the *broker drain rate*, not a
delay between `reconcile` returning and `dispatch` running. The
ADR-0013 evaluation broker queues evaluations; the runtime drains
the queue every 100 ms; each drained evaluation runs the
hydrate-then-reconcile-then-dispatch pipeline synchronously within
its tick. This is the level-triggered semantics ADR-0013 §
"Triggering Model — Hybrid by Design" already documents; the shim
is the natural step-7 caller.

## Consequences

### Positive

- **One async I/O boundary in the convergence loop.** The shim is
  the *only* place a driver method is called from inside the
  reconciler-runtime pipeline. `reconcile` stays pure;
  `hydrate_desired` / `hydrate_actual` (ADR-0021) and `hydrate`
  (ADR-0013) are read-only. All workload mutations go through the
  shim.
- **Compile-time exhaustive Action match.** New Action variants
  produce non-exhaustive-match compile errors. Phase 2's first
  external-call dispatch (`Action::HttpCall` runtime) becomes a
  new arm — the shim's signature can stay stable, or grow a
  parallel `dispatch_http(…)` method, as the Phase 3 ADR
  decides.
- **DST-safe by construction.** The `clock.sleep(…)` indirection
  through the injected `Clock` trait means the same shim code
  runs in production (against `SystemClock`) and under DST
  (against `SimClock`). No conditional compilation; no
  `#[cfg(test)]` bodies; the sim adapter is the only divergence.
- **Production tick cadence is operator-tunable without code
  changes.** The `ServerConfig` field is a millisecond integer;
  a future operator who wants 50 ms ticks for tighter
  responsiveness or 500 ms for lower CPU overhead changes one
  config value. The 100 ms default is a calibrated starting
  point, not a hard architectural commitment.
- **Failure isolation is per-action, not per-tick.** A failing
  driver call writes a `Failed` allocation row and the shim
  proceeds to the next action in the batch. One bad action does
  not stall the next reconciler's evaluation.

### Negative

- **The runtime tick task adds one always-on `tokio::task`.**
  The task wakes every 100 ms regardless of whether any
  evaluations are queued. Phase 1 single-mode this is a
  negligible CPU footprint (one timer fire + one broker peek per
  tick); Phase 2+ multi-mode may want to migrate to a
  notification-driven wake. Acknowledged; not a Phase 1 blocker.
- **Cross-action atomicity is per-action.** If
  `dispatch(actions)` fails halfway through a 5-action batch, the
  first 2 actions have run their effects and observation rows;
  the latter 3 are dropped. The next reconciler tick will see
  the partial state in `actual` and re-emit the missing actions
  (the level-triggered guarantee). This is the correct semantics
  per whitepaper §18 — "missed or duplicated events do not lose
  state — the next evaluation sees the full current delta" —
  but operators should not expect transactional batches.
- **Tick latency floor is the cadence (100 ms).** A
  `job submit` → broker enqueue → next-drain → reconcile →
  dispatch → AllocStatusRow Running roundtrip is at minimum
  one tick (≤100 ms) and at maximum two ticks (~200 ms) on the
  default cadence. Sub-100 ms convergence requires reducing the
  cadence; `cluster status` HTTP latency (handler-only, not
  through reconciliation) is unaffected and remains well within
  the 100 ms target from ADR-0008's quality-attribute table.

### Quality-attribute impact

- **Maintainability — modifiability**: positive. Adding a new
  Action variant is a new shim arm and a new reconciler-emit site;
  the compiler enforces both.
- **Maintainability — testability**: positive. The shim takes
  `&dyn Driver` and `&dyn ObservationStore`; tests pass
  `SimDriver` + `SimObservationStore` and assert on the resulting
  observation rows. No fixture-theatre, no spy doubles.
- **Reliability — fault tolerance**: positive. Per-action error
  isolation; level-triggered re-evaluation absorbs transient
  failures.
- **Reliability — recoverability**: positive (level-triggered).
- **Performance — time behaviour**: positive (in-line dispatch
  within the per-evaluation pipeline; no channel hop).
- **Performance — resource utilisation**: marginally negative
  (always-on tick task). Acceptable at Phase 1 single-mode
  footprint.

## Compliance

- **ADR-0013 § Reconciler I/O**: the shim is the I/O boundary
  reconciler bodies cannot cross. Preserved.
- **ADR-0013 §2c (TickContext)**: the shim receives the same
  `&TickContext` the reconciler did, ensuring observation writes
  carry consistent timestamps with the action that produced
  them.
- **ADR-0017 `ReconcilerIsPure` invariant**: untouched — the
  shim is downstream of `reconcile`'s return value.
- **`development.md` § Errors**: `ShimError` is `thiserror`-typed,
  uses pass-through `#[from]`, returns `Result` (not `eyre`).
  Compliant.
- **`development.md` § Concurrency**: no lock held across
  `.await`. The shim's body is sequential (`for action in
  actions`); each action is a self-contained async block; no
  shared mutable state inside the shim itself.
- **`dst-lint`**: the shim lives in `adapter-host`-class
  `overdrive-control-plane` and is exempt from the banned-API
  scan. The reconciler bodies that emit Actions remain in
  `core`-class `overdrive-core` (or `core`-class
  `overdrive-scheduler` per ADR-0024) and are scanned. The seam
  is intact.

## References

- ADR-0013 — Reconciler primitive: trait, runtime, libSQL private
  memory.
- ADR-0021 — `AnyState` enum (the State counterpart to
  `AnyReconcilerView`).
- ADR-0022 — `AppState::driver: Arc<dyn Driver>` field.
- Whitepaper §18 — Reconciler and workflow primitives;
  evaluation broker; level-triggered semantics.
- USENIX OSDI '24 *Anvil* — `reconcile_core` + async shim
  pattern; the architectural precedent.
- `.claude/rules/development.md` § Reconciler I/O — codifies the
  pure-reconcile + async-shim discipline.
- `docs/feature/phase-1-first-workload/discuss/wave-decisions.md`
  — Key Decision 5 pre-decides the shim layer.

## Amendment 2026-04-27 — Worker Crate Extraction

This ADR is unchanged in shape. The shim still lives in
`overdrive-control-plane::reconciler_runtime::action_shim` and calls
`Driver::start` / `Driver::stop` / `Driver::status` against any
`&dyn Driver` impl — the trait-object signature is the seam. The
only relocation is **which crate hosts the production `Driver`
impl**: `overdrive-host` → `overdrive-worker` per ADR-0029. The
shim sees only the trait surface and is unaware of the impl-crate
change. Phase 2+ multi-node introduces a `RemoteDriver` impl in a
future crate (e.g. `overdrive-rpc-driver`) that proxies the same
trait over RPC; the shim's `dispatch(...)` signature stays stable.

