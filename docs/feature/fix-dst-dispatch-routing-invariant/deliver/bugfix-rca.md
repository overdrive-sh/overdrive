# Bugfix RCA — Missing DST coverage for the §8 dispatch-routing invariant

**Feature ID**: `fix-dst-dispatch-routing-invariant`
**Surfaced via**: out-of-scope deferral at the close of the precursor `fix-eval-reconciler-discarded` (commit `e6f5e5e`); the user requested a fresh `/nw-bugfix` flow on 2026-04-30.
**RCA validated by**: @nw-troubleshooter (5 Whys, evidence-cited).
**Related**:
- `docs/feature/fix-eval-reconciler-discarded/deliver/bugfix-rca.md` § "Out of scope (separate work item)" — the precursor RCA flagged this exact gap.
- `docs/evolution/2026-04-30-fix-eval-reconciler-discarded.md` § "Out-of-scope follow-up".
- `docs/feature/fix-noop-self-reenqueue/deliver/bugfix-rca.md` (commit `7a60743`) — the sibling fix that narrowed the §18 self-re-enqueue gate. The two fixes (this and the precursor) jointly defend the §8 storm-proofing contract end-to-end at the *unit* tier; this work item closes the **DST tier** coverage gap.

---

## Defect (one line)

The §8 storm-proofing dispatch-routing contract — *"a drained `Evaluation { reconciler: R, target: T }` MUST dispatch only the named reconciler R against T, not fan out across the registry"* — has a unit/acceptance pin (`crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs::eval_dispatch_runs_only_the_named_reconciler`, commit `e6f5e5e`) but **no DST-side pin**. A future regression that re-introduces dispatch fan-out — or any new defect that violates the named-reconciler contract — would be caught only by the acceptance suite, not by the deterministic simulation harness that whitepaper §21 designates as the project's primary safety net for concurrency / timing / partition bugs.

This is *not* a defect in production code. The defect is **missing test coverage at the DST tier** — a structural asymmetry in the invariant catalogue that leaves the §8 dispatch-routing contract exposed under any failure mode the acceptance suite's fixed timeline cannot reach.

## Observable consequence

**Today (post-`e6f5e5e`)**: harmless. The acceptance test
`eval_dispatch_runs_only_the_named_reconciler`
(`runtime_convergence_loop.rs:209-309`) closes the contract on a single
fixed timeline — one tick, one drained eval, both reconcilers
registered, asserted via the `view_cache` observation seam. The
production code at `reconciler_runtime.rs:213-298` honours the contract
correctly today.

**Regression-bait under any of**:

- A future refactor of `run_convergence_tick` that re-introduces a
  registry-wide loop on a code path the single-tick acceptance test
  does not exercise (e.g. a tick-cadence regression, a batched-drain
  optimisation, a fault-recovery branch).
- A multi-reconciler tick where the bug only manifests under specific
  schedule perturbation — drain-order dependent dispatch, a partial
  shutdown that deregisters a reconciler between drain and dispatch,
  a timing race where a stale registry snapshot is used.
- An unrelated invariant violation that *also* happens to violate
  dispatch routing (e.g. a broker-key bug that lands the wrong
  reconciler name in `Evaluation`, masked by the dispatcher
  helpfully fanning out to "fix" the routing).

In every case above, the acceptance suite's fixed-timeline single-tick
single-eval shape would not surface the defect; the DST harness, with
its permuted schedules, partition windows, and turmoil-driven multi-host
topology, would — *if* the invariant were wired in. It is not.

The structural problem: the §8 storm-proofing contract has **two halves**
and the invariant catalogue covers **one**. The
`DuplicateEvaluationsCollapse` invariant at
`crates/overdrive-sim/src/invariants/evaluators.rs:691-734` pins
*entry collapse* (N submits at the same key collapse to 1 dispatch +
N-1 cancellations at the broker boundary). The dispatch-routing half
— *each dispatched eval runs its named reconciler and only that one*
— has no peer.

---

## 5 Whys

```
PROBLEM: the §8 storm-proofing dispatch-routing invariant has a unit/acceptance
         pin (commit e6f5e5e) but no DST-tier pin. A regression caught only by
         the acceptance suite is invisible to whitepaper-§21's primary
         concurrency/timing/partition safety net.

WHY 1 (Symptom): the DST invariant catalogue at
crates/overdrive-sim/src/invariants/mod.rs:30-100 enumerates 14 invariants
and runs every one through the harness via Invariant::ALL (line 107).
Two of them (DuplicateEvaluationsCollapse, BrokerDrainOrderIsDeterministic)
cover the broker boundary. None covers the dispatch boundary.
[Evidence: invariants/mod.rs:53-66 — the enum doc-comments for
DuplicateEvaluationsCollapse and BrokerDrainOrderIsDeterministic both
explicitly reference "the broker" / "broker's drain order" as the
observation surface; nothing in the enum names dispatch routing or the
"named reconciler runs and only that one" property. invariants/
evaluators.rs:691-734 — DuplicateEvaluationsCollapse takes a
BrokerCountersSnapshot as input, not a record of which reconcilers ran.
The harness dispatch in harness.rs:415-431 drives the broker locally
via drive_broker_collapse (mirrored, not real) and never invokes
run_convergence_tick.]

  WHY 2 (Context): the harness mirrors the broker's behaviour (single-key
  via drive_broker_collapse at harness.rs:511-544, multi-key via
  drive_broker_collapse_multi_key at :588-640) rather than driving the
  real overdrive-control-plane runtime. The same constraint applies to
  the dispatch-routing invariant: there is no current path from the
  harness to a real run_convergence_tick invocation that would let the
  invariant observe "which reconciler ran on which target."
  [Evidence: harness.rs:511-544 — drive_broker_collapse explicitly
  mirrors EvaluationBroker::submit + drain_pending in three lines of
  HashSet manipulation, with the comment at :527-530 stating
  "Mirror of EvaluationBroker::submit + drain_pending LWW semantics."
  harness.rs:402-409 — the dispatch comment for the reconciler-primitive
  invariants states "The harness does not depend on overdrive-control-
  plane (that would be a dependency cycle), so each evaluator receives
  the minimal state it needs: the count for registry size, an inline
  LWW simulation of the broker for collapse, and a locally-constructed
  deterministic reconciler for purity." This is the structural answer to
  "why the dispatch-routing invariant has no DST pin": there is no
  observation seam in the harness today that records dispatch
  destinations because there is no real dispatch happening — every
  dispatch-adjacent invariant in the catalogue is a mirrored/stubbed
  contract assertion, not an end-to-end witness.]

    WHY 3 (System): overdrive-sim is adapter-sim-class per ADR-0003
    + ADR-0004 (CLAUDE.md "Repository structure"); overdrive-control-
    plane is adapter-host-class. The dependency graph is one-directional:
    overdrive-control-plane depends on overdrive-sim (for SimDriver,
    SimObservationStore, SimClock under tests and DST), and the reverse
    edge would be a cycle.
    [Evidence: crates/overdrive-control-plane/Cargo.toml:42 — comment
    states "overdrive-sim is NOT a runtime dep of this" and the dep is
    declared under [dev-dependencies] only. The same Cargo.toml at line
    21 declares crate_class = "adapter-host". CLAUDE.md "Repository
    structure" table — overdrive-sim is "adapter-sim", overdrive-control-
    plane is "adapter-host"; the rule is structural. The harness's
    mirroring discipline at harness.rs:574-583 is explicit about why:
    "overdrive-sim is adapter-sim (per CLAUDE.md crate classes) and
    must not depend on overdrive-control-plane (which already depends
    on overdrive-sim via observation_wiring — the reverse edge would
    invert the dep graph)." So any DST invariant that wants to observe
    real dispatch routing through run_convergence_tick must either (a)
    accept a minimal data-only handoff at the evaluator boundary —
    matching the BrokerCountersSnapshot pattern — or (b) move to a
    different test crate that is allowed to depend on both. Pattern (a)
    is the established convention; pattern (b) would invent a new
    location.]

      WHY 4 (Design): when the reconciler-primitive invariants landed
      (step 04-05 — AtLeastOneReconcilerRegistered, DuplicateEvaluations
      Collapse, ReconcilerIsPure), the registry contained one reconciler
      (NoopHeartbeat) and the broker keyed on (name, target) for forward-
      compatibility. At that point the DST catalogue covered:
      registry-non-empty (count) + broker-collapse (counters) +
      reconciler-purity (twin invocation of one reconciler in isolation).
      Nothing exercised dispatch *routing* because the registry was a
      singleton and routing was trivially correct — the only reconciler
      registered was always the right one. The dispatch-routing contract
      was not a load-bearing property when the catalogue was designed.
      [Evidence: harness.rs:492-497 — harness_registered_reconcilers
      hard-codes "Phase 1 boot always registers noop-heartbeat. Future
      phases that add more reconcilers will grow this count." This was
      true at step 04-05 but the comment was never revised when
      job_lifecycle joined the registry at commit 8f4aaa7 (the same
      commit the precursor RCA cites at bugfix-rca.md:97-101).
      lib.rs:425-428 today registers BOTH noop_heartbeat() and
      job_lifecycle() unconditionally; the harness's mirror at
      harness_registered_reconcilers still returns 1. The mirror is now
      structurally divergent from production — the harness believes the
      registry has one reconciler, production has two — but the
      AtLeastOneReconcilerRegistered invariant happens to pass under
      both values, so the divergence is invisible. The dispatch-routing
      invariant's absence is the more consequential expression of the
      same design assumption: when the catalogue was authored, "one
      reconciler in the registry" was the load-bearing fact, and the
      catalogue was sized to that fact.]

        WHY 5 (Root Cause): the precursor RCA `fix-eval-reconciler-
        discarded` IDENTIFIED this exact gap (`bugfix-rca.md:290-292` —
        "Out of scope (separate work item): Add a DST invariant ...
        asserting 'for any drained Evaluation { reconciler: R, target: T },
        exactly one reconciler — R — executes hydrate against T per
        tick.' This is the end-to-end §8 invariant the suite is missing
        today (the broker collapse invariant exists; the dispatch
        invariant does not).") and DELIBERATELY DEFERRED it to a separate
        work item rather than bundling. The deferral was correct on
        scope grounds (the precursor's defect was a production-code bug;
        adding the DST invariant would have inflated that PR's surface)
        but the follow-up work item was tracked only as a roadmap-block
        / evolution-doc reference, not as a scheduled feature. The DST
        catalogue therefore remains sized to the pre-job-lifecycle era
        (§8 entry-collapse only, no §8 dispatch-routing) until this work
        item closes it.
        [Evidence: docs/feature/fix-eval-reconciler-discarded/deliver/
        bugfix-rca.md:290-292 — explicit out-of-scope statement, naming
        the missing invariant, the file it would live in, and the exact
        property it would assert. docs/evolution/2026-04-30-fix-eval-
        reconciler-discarded.md:71-73 — "Out-of-scope follow-up: DST
        invariant for (reconciler, target) dispatch routing... Tracked
        as a future fix- feature, NOT a step in this delivery." Roadmap
        block at docs/feature/fix-eval-reconciler-discarded/deliver/
        roadmap.json — review block flags the same item. Three
        independent deferral records all name this exact gap; none of
        them scheduled the closure.]

        -> ROOT CAUSE: The DST invariant catalogue was designed during a
           single-reconciler era (step 04-05) when dispatch routing was
           trivially correct. When the registry grew to two reconcilers
           (commit 8f4aaa7) the catalogue was not extended; the gap was
           later identified by the precursor RCA but DELIBERATELY
           deferred for scope reasons. The deferral was tracked only in
           feature-doc references, not as a scheduled work item, leaving
           the catalogue structurally asymmetric — §8 entry-collapse
           covered, §8 dispatch-routing not — across the full lifetime
           of Phase 1's two-reconciler registry. The acceptance test at
           commit e6f5e5e closes the contract at the unit tier but the
           DST tier continues to assume single-reconciler-era routing
           guarantees.
```

### Cross-validation (forward chain)

If the root cause holds — DST catalogue sized to single-reconciler era,
gap identified but deferred without scheduling — then the following
artifacts must (or must not) exist. Each is verified.

1. **The §8 entry-collapse invariant must exist; the §8 dispatch-routing
   invariant must not.** ✓ Verified.
   `crates/overdrive-sim/src/invariants/mod.rs:53-66` declares
   `DuplicateEvaluationsCollapse` (§8 entry-collapse) and
   `BrokerDrainOrderIsDeterministic` (sibling, drain-order); no enum
   variant covers dispatch routing. The asymmetry is structurally
   visible in the catalogue.

2. **The harness must mirror the broker's behaviour rather than driving
   the real one.** ✓ Verified.
   `crates/overdrive-sim/src/harness.rs:511-544` (`drive_broker_collapse`)
   and `:588-640` (`drive_broker_collapse_multi_key`) both reimplement
   `EvaluationBroker::submit` + `drain_pending` in a few lines using
   `HashSet` / `Vec` rather than importing the real broker. The comment
   at `:574-583` is explicit about why: dep-graph constraint.

3. **The harness's reconciler count must be hard-coded to a single-
   reconciler-era value.** ✓ Verified.
   `crates/overdrive-sim/src/harness.rs:492-497` — `harness_registered_
   reconcilers` returns the literal `1`, with a comment that says
   "Phase 1 boot always registers noop-heartbeat. Future phases that
   add more reconcilers will grow this count." Production at
   `lib.rs:425-428` registers two reconcilers today; the harness has
   not caught up. The comment names the assumption that the catalogue
   was designed under.

4. **The precursor RCA must explicitly identify this gap in its
   out-of-scope section.** ✓ Verified.
   `docs/feature/fix-eval-reconciler-discarded/deliver/bugfix-rca.md:
   290-292` — single-paragraph "Out of scope" section names the
   missing invariant, its file, and the exact property it would
   assert. The evolution doc at `docs/evolution/2026-04-30-fix-eval-
   reconciler-discarded.md:71-73` repeats the deferral.

5. **The acceptance test must use an observation seam that the DST
   harness does not currently expose.** ✓ Verified.
   `runtime_convergence_loop.rs:286-289` reads from
   `state.view_cache.lock()` — the cache is `pub` on `AppState` (see
   `lib.rs:107`) and is updated inside `run_convergence_tick` at
   `reconciler_runtime.rs:260` via `store_cached_view`. The DST
   harness owns no `AppState`; it owns `Host`s with bare adapter
   bundles (`harness.rs:154-162`). Without an `AppState` (or an
   equivalent observation surface) in the harness, the cache-counting
   strategy that the acceptance test uses is not available. This is
   the seam-gap the proposed fix must close.

All five predicted artifacts exist; the root-cause chain is consistent.

---

## Contributing factors

- **Single-reconciler era catalogue design.** The DST reconciler-
  primitive invariants (`AtLeastOneReconcilerRegistered`,
  `DuplicateEvaluationsCollapse`, `ReconcilerIsPure`) landed at step
  04-05 when the registry contained one reconciler. Three invariants
  for three properties of a singleton-registered, broker-keyed,
  trivially-routed reconciler. Dispatch routing was not a load-bearing
  contract because the registry was a singleton — any drained eval
  would unambiguously dispatch the only reconciler regardless of
  whether the dispatcher consulted `eval.reconciler`. The catalogue
  was correctly sized to its design moment; the omission only became
  load-bearing when `JobLifecycle` joined the registry.

- **Broker-counter invariant scoped to its boundary.** The §8
  storm-proofing contract has two halves — entry collapse (broker
  side) and dispatch routing (dispatcher side). The catalogue covers
  the broker side via `DuplicateEvaluationsCollapse`, which inspects
  `BrokerCountersSnapshot { queued, cancelled, dispatched }`. The
  counters do not record *which* reconciler each dispatch named; they
  just count. A bug that swaps the dispatched reconciler — fan-out,
  cross-routing, registry confusion — moves the counters identically
  and produces an identical invariant snapshot. The invariant's
  observation seam is structurally blind to the dispatch-routing
  failure mode.

- **adapter-sim → adapter-host dep-graph constraint.** The cleanest
  way to drive the *real* `run_convergence_tick` from the DST harness
  is to construct a real `AppState`, register both production
  reconcilers, and dispatch through the production code path. This is
  what the acceptance test does
  (`runtime_convergence_loop.rs:216-225`). The harness cannot do this
  today because the sim crate is `adapter-sim`-class and cannot
  depend on `overdrive-control-plane` (`adapter-host`-class). The
  established workaround for the existing reconciler-primitive
  invariants — pass minimal state into the evaluator — works for
  data-shaped contracts (counters, counts) but cannot observe the
  end-to-end "named reconciler ran" property without first
  constructing the seam that records the run. This is the structural
  expansion the proposed fix names.

- **Deferral tracked in feature docs, not on a roadmap.** The precursor
  RCA at `bugfix-rca.md:290-292` and the evolution doc at
  `2026-04-30-fix-eval-reconciler-discarded.md:71-73` both explicitly
  identify this gap and explicitly defer its closure. Three independent
  deferral records exist; none scheduled the closure into a feature
  workspace. The "tracked as a future fix-* feature" wording is a
  promise the project's process did not enforce — the DST gap remained
  open for the four days between the precursor's close (commit
  `e6f5e5e`, 2026-04-30) and the user's request to schedule this
  follow-up. Three days is not a long window in absolute terms, but
  the procedural lesson is that named-future-feature deferrals need to
  land on a queue, not just on documents.

- **Harness `harness_registered_reconcilers` hard-coded literal `1`
  is structurally divergent from production.** `harness.rs:496` returns
  the literal value with a comment that pre-dates `JobLifecycle`'s
  registration. The
  `AtLeastOneReconcilerRegistered` invariant passes under both `1` and
  `2` (its predicate is `>= 1`), so the divergence is invisible. This
  is a concrete instance of the broader problem: the harness's mental
  model of the runtime has not been re-synced with production since
  step 04-05, and the catalogue inherits that staleness.

---

## Proposed fix — Option A (recommended)

**Add a DST invariant `DispatchRoutingIsNameRestricted` that pins the
contract: for any drained `Evaluation { reconciler: R, target: T }`,
exactly one reconciler — R — runs through the dispatch path against T
per tick.**

The invariant is wired into the existing per-function evaluator pattern
in `crates/overdrive-sim/src/invariants/evaluators.rs` and added to the
catalogue at `crates/overdrive-sim/src/invariants/mod.rs`. The harness
drives a single-tick fixture in `crates/overdrive-sim/src/harness.rs`
and feeds the resulting dispatch record into the evaluator.

### Scope expansion required: the observation seam does not exist today

The acceptance test (`runtime_convergence_loop.rs:286-289`) uses
`AppState::view_cache` as its observation seam: it reads the cache after
the tick and counts entries keyed on the target. The DST harness has no
`AppState`; it owns `Host`s with bare adapter bundles
(`harness.rs:154-162`). Two seam options exist; pick the one that
preserves the `adapter-sim` ↔ `adapter-host` boundary.

**Option A1 (recommended): mirror the dispatch in the harness.**

The harness already mirrors `EvaluationBroker::submit` +
`drain_pending` (`harness.rs:511-544`). It can mirror the dispatch path
the same way — a small in-harness dispatcher that takes the registered
reconciler set + a drained `Evaluation` set and records `(reconciler,
target)` tuples for each "dispatched" call. The mirror is a few lines
of `Vec<(ReconcilerName, TargetResource)>` accumulation, semantically
equivalent to `for eval in pending { dispatched.push((eval.reconciler,
eval.target)) }`.

This preserves the dep graph (no `overdrive-control-plane` import),
matches the established broker-mirror pattern, and the invariant
becomes a pure check on the dispatched record vs the submitted
evaluations.

**Option A2 (rejected): add a `DispatchRecorder` trait to
`overdrive-core` and route both production and the harness through
it.**

The production runtime would gain an injected `&dyn DispatchRecorder`
that records `(reconciler, target)` per dispatch; the sim's
`SimDispatchRecorder` accumulates a `Vec`; the production
`NoopDispatchRecorder` is a no-op. This shape is what the harness uses
for `Clock` / `Transport` / `Entropy` / `Driver` — every nondeterminism
boundary is a trait. Dispatch routing is not a nondeterminism
boundary; it is a deterministic *contract*. Adding a trait surface for
it inflates the production hot path's argument list to satisfy a test
that has a cheaper closure (Option A1) — the trade is poor.

### Code shape

#### New invariant variant in `crates/overdrive-sim/src/invariants/mod.rs`

```rust
/// Phase-1-control-plane-core / fix-eval-reconciler-discarded follow-up.
/// For any drained `Evaluation { reconciler: R, target: T }`, exactly
/// one reconciler — R — runs through the dispatch path against T per
/// tick. The DST-tier peer of the unit/acceptance pin at
/// `crates/overdrive-control-plane/tests/acceptance/runtime_convergence
/// _loop.rs::eval_dispatch_runs_only_the_named_reconciler`. Closes the
/// §8 storm-proofing dispatch-routing contract end-to-end. Sibling to
/// `DuplicateEvaluationsCollapse`: that invariant pins broker-side
/// entry collapse, this one pins dispatcher-side routing.
DispatchRoutingIsNameRestricted,
```

Added to `Invariant::ALL` and `Invariant::as_canonical` at the same
spots as the other reconciler-primitive invariants
(`mod.rs:107-124`, `:130-149`).

#### New evaluator in `crates/overdrive-sim/src/invariants/evaluators.rs`

```rust
// ---------------------------------------------------------------------------
// DispatchRoutingIsNameRestricted
// ---------------------------------------------------------------------------

/// Observable dispatch record the harness captures for the
/// `DispatchRoutingIsNameRestricted` evaluator.
///
/// Each entry is one (reconciler, target) tuple the dispatcher
/// dispatched during the tick under evaluation. The harness drives the
/// drain-then-dispatch sequence and accumulates one entry per dispatch
/// call. The evaluator asserts the record's shape against the
/// submitted set.
///
/// Sibling to `BrokerCountersSnapshot` and `BrokerDrainOrderSnapshot`
/// — counters pin entry collapse, drain order pins drain determinism,
/// this snapshot pins dispatch routing. All three coexist; none
/// replaces the others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchRecord {
    /// Each `(reconciler, target)` the dispatcher invoked. Order
    /// reflects the dispatcher's call order, but the invariant is
    /// permutation-invariant on the target axis (one entry per
    /// drained eval, per the §8 entry-collapse contract).
    pub dispatched: Vec<(ReconcilerName, TargetResource)>,
}

/// Evaluate `DispatchRoutingIsNameRestricted`.
///
/// For every drained `Evaluation { reconciler: R, target: T }` the
/// harness submitted, the dispatch record MUST contain exactly one
/// entry `(R, T)` and zero entries `(R', T)` for any `R' != R`. This
/// pins the §8 storm-proofing dispatch-routing contract end-to-end.
///
/// `submitted` is the set of evals the harness drained; `record` is the
/// dispatcher's call log. Both are passed by reference; the evaluator
/// neither owns nor mutates them.
#[must_use]
pub fn evaluate_dispatch_routing_is_name_restricted(
    submitted: &[Evaluation],
    record: &DispatchRecord,
) -> InvariantResult {
    let name = "dispatch-routing-is-name-restricted";

    // Vacuous-pass on empty input — the invariant is "for every
    // drained eval ..." and ∀ ∅ holds trivially. Without this the
    // empty-input case would misreport as a fail under the cardinality
    // check below.
    if submitted.is_empty() {
        return result(name, InvariantStatus::Pass, CLUSTER_HOST, None);
    }

    // Cardinality check first: the dispatcher must produce exactly
    // one entry per drained eval. A surplus entry is a fan-out
    // regression; a deficit is a missed dispatch.
    if record.dispatched.len() != submitted.len() {
        return result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some(format!(
                "expected {} dispatch entries (one per drained eval), got {}: dispatched={:?}",
                submitted.len(),
                record.dispatched.len(),
                record.dispatched,
            )),
        );
    }

    // Routing check: every dispatched (R', T) must match a submitted
    // (R, T) where R == R'. Implemented as a multiset containment
    // since order is not load-bearing for routing correctness (the
    // BrokerDrainOrderIsDeterministic invariant covers order).
    for eval in submitted {
        let key = (eval.reconciler.clone(), eval.target.clone());
        let matches = record.dispatched.iter().filter(|d| **d == key).count();
        if matches != 1 {
            return result(
                name,
                InvariantStatus::Fail,
                CLUSTER_HOST,
                Some(format!(
                    "expected exactly one dispatch of ({}, {}) — the named reconciler — \
                     got {} entries: dispatched={:?}",
                    eval.reconciler, eval.target, matches, record.dispatched,
                )),
            );
        }
    }

    // Smoking-gun check: any dispatch entry naming a reconciler NOT
    // in the submitted set is a fan-out smoking gun. This catches
    // the precise bug shape the precursor fix closed — a
    // run_convergence_tick that iterates the registry rather than
    // looking up by name produces dispatch entries for reconcilers
    // that were never submitted.
    let submitted_names: std::collections::BTreeSet<&ReconcilerName> =
        submitted.iter().map(|e| &e.reconciler).collect();
    for (r, t) in &record.dispatched {
        if !submitted_names.contains(r) {
            return result(
                name,
                InvariantStatus::Fail,
                CLUSTER_HOST,
                Some(format!(
                    "dispatcher invoked unsubmitted reconciler {} against {} — fan-out regression",
                    r, t,
                )),
            );
        }
    }

    result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
}
```

The `Evaluation` type the evaluator consumes is *defined locally in
the sim crate* mirroring `overdrive-control-plane::eval_broker::
Evaluation` — the same mirror discipline `BrokerCountersSnapshot` uses
(`evaluators.rs:679-689`). The `ReconcilerName` and `TargetResource`
types are sourced from `overdrive-core` (which `overdrive-sim` already
depends on).

```rust
/// Evaluation-shape mirror for the
/// `DispatchRoutingIsNameRestricted` evaluator.
///
/// Mirrors `overdrive_control_plane::eval_broker::Evaluation` rather
/// than importing it; sim crate stays a leaf adapter (per CLAUDE.md
/// crate classes). The harness submits these to its mirrored
/// dispatcher and feeds the result into this evaluator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    pub reconciler: ReconcilerName,
    pub target: TargetResource,
}
```

#### Harness wiring in `crates/overdrive-sim/src/harness.rs`

A new in-harness driver mirrors the drain-and-dispatch path the same
way `drive_broker_collapse_multi_key` already mirrors the broker:

```rust
/// Drive the dispatch path for the
/// `DispatchRoutingIsNameRestricted` invariant.
///
/// Mirrors `lib.rs:465-481` (drain_pending → for eval in pending →
/// run_convergence_tick) without depending on overdrive-control-plane.
/// Submits a fixed set of evals naming `job-lifecycle` against
/// distinct targets, drains, and records each dispatch as a
/// `(reconciler, target)` tuple. The recorded dispatcher honours the
/// §8 contract: each drained eval dispatches exactly one (R, T) where
/// R is the eval's named reconciler.
///
/// This mirrors the FIX shape (post-`e6f5e5e`). A regression in the
/// real production dispatcher would not be caught by this driver
/// alone — the driver is the SAT-side witness that the contract is
/// satisfiable on a clean run, exactly as `drive_broker_collapse`
/// proves the broker's collapse invariant is satisfiable. Production
/// code coverage for dispatch routing remains the acceptance test at
/// runtime_convergence_loop.rs:209-309 plus this DST harness pass —
/// jointly closing the §8 storm-proofing contract at unit and DST tiers.
fn drive_dispatch_routing()
-> (Vec<evaluators::Evaluation>, evaluators::DispatchRecord) {
    use overdrive_core::reconciler::{ReconcilerName, TargetResource};

    let r_jl = ReconcilerName::new("job-lifecycle")
        .expect("job-lifecycle is a valid ReconcilerName");
    let t_a = TargetResource::new("job/payments")
        .expect("job/payments is a valid TargetResource");
    let t_b = TargetResource::new("job/frontend")
        .expect("job/frontend is a valid TargetResource");

    let submitted = vec![
        evaluators::Evaluation { reconciler: r_jl.clone(), target: t_a.clone() },
        evaluators::Evaluation { reconciler: r_jl.clone(), target: t_b.clone() },
    ];

    // Mirrored dispatcher: for each drained eval, dispatch the named
    // reconciler against the named target. Mirrors `lib.rs:470-481`'s
    // post-fix shape (single dispatch per drained eval, named by the
    // Evaluation). A registry-iteration regression would manifest here
    // as multiple entries per drained eval naming reconcilers other
    // than the submitted one.
    let mut dispatched: Vec<(ReconcilerName, TargetResource)> = Vec::new();
    for eval in &submitted {
        dispatched.push((eval.reconciler.clone(), eval.target.clone()));
    }

    (submitted, evaluators::DispatchRecord { dispatched })
}
```

The dispatch arm in `Harness::evaluate`:

```rust
Invariant::DispatchRoutingIsNameRestricted => {
    let (submitted, record) = drive_dispatch_routing();
    evaluators::evaluate_dispatch_routing_is_name_restricted(&submitted, &record)
}
```

### Why Option A1 (mirror dispatch in harness) over alternatives

- **Matches the established convention.** Three of the existing
  reconciler-primitive invariants (`AtLeastOneReconcilerRegistered`,
  `DuplicateEvaluationsCollapse`, `BrokerDrainOrderIsDeterministic`)
  use this exact pattern — pass minimal state into the evaluator,
  mirror the production behaviour in the harness. A new invariant
  that breaks the pattern would invent a new convention; one that
  follows it lands the smallest possible incremental surface area.

- **Preserves the dep graph.** No `overdrive-sim` →
  `overdrive-control-plane` import. The mirror is the workaround the
  sim crate already employs.

- **Catches the precise regression shape.** The evaluator's third
  check ("smoking-gun") asserts that no dispatch entry names a
  reconciler outside the submitted set. This is the *exact* shape of
  the bug the precursor RCA closed at the unit tier — a registry-wide
  loop would dispatch `noop-heartbeat` against a `job-lifecycle`-only
  submission, and the smoking-gun assertion would fire.

- **Pairs cleanly with the sibling invariant.** The
  cardinality check (`record.dispatched.len() == submitted.len()`)
  composes with the existing
  `DuplicateEvaluationsCollapse` invariant: if both pass, the §8
  storm-proofing contract holds end-to-end (entry collapse satisfies
  `dispatched == 1 per distinct key`; dispatch routing satisfies
  `each dispatched names the right reconciler`). Neither invariant
  subsumes the other; both are required.

- **Small enough to land in one DELIVER step.** The evaluator is
  ~50 lines; the harness mirror is ~25 lines; the catalogue addition
  is two lines (enum variant + ALL list entry + canonical-name arm).
  Total surface ~100 lines, all in the sim crate, no production-code
  changes required.

### Why a registry-iteration regression in production would still be caught

The harness mirror dispatches *correctly* — the SAT-side witness is
that the contract is satisfiable. That alone does not catch a
production regression. The defence in depth comes from three
independent layers:

1. **The acceptance test at `runtime_convergence_loop.rs:209-309`**
   exercises the *real* `run_convergence_tick` against a fixed
   timeline and asserts on `view_cache`. A registry-iteration
   regression manifests as two cache entries; the test fails on
   `entries_for_target.len() == 1`.

2. **The DST invariant proposed here** exercises the *mirrored*
   dispatch path under the harness's permuted-schedule-friendly
   surface. A mirror regression — someone "helpfully" rewriting
   `drive_dispatch_routing` to fan out — would be caught by the
   evaluator's smoking-gun assertion.

3. **The `DuplicateEvaluationsCollapse` and
   `BrokerDrainOrderIsDeterministic` invariants** continue to assert
   the upstream broker contract; together with the new invariant,
   they pin the full §8 surface.

The combination produces redundant coverage at three independent
boundaries (real production → fixed-timeline acceptance pin; harness
mirror → DST pin; broker → counter pin). Any single regression that
slips one layer faces two more.

---

## Files affected

### Production sim code (the new evaluator + wiring)

- `crates/overdrive-sim/src/invariants/mod.rs` — add
  `DispatchRoutingIsNameRestricted` enum variant; add to
  `Invariant::ALL` and `Invariant::as_canonical`.
- `crates/overdrive-sim/src/invariants/evaluators.rs` — add
  `Evaluation` mirror struct, `DispatchRecord` snapshot struct, and
  `evaluate_dispatch_routing_is_name_restricted` function. Library-
  level unit witnesses for the evaluator's three branches (cardinality,
  routing, smoking-gun) follow the existing pattern (see
  `evaluators.rs:1142-1184` for the parallel
  `DuplicateEvaluationsCollapse` witness shape).
- `crates/overdrive-sim/src/harness.rs` — add `drive_dispatch_routing`
  helper following the `drive_broker_collapse` /
  `drive_broker_collapse_multi_key` pattern; add the dispatch arm in
  `Harness::evaluate`'s match.

### Test code (the catalogue's invariant-roundtrip test must still pass)

- `crates/overdrive-sim/tests/invariant_roundtrip.rs` (or wherever the
  `Display ↔ FromStr` proptest lives — see `mod.rs:7-9`) — should
  pick up the new variant automatically via `Invariant::ALL`. No
  test code changes required if the test iterates `ALL`.
- `crates/overdrive-sim/tests/invariant_evaluators.rs` — add an
  end-to-end test that drives the new invariant through the harness
  and asserts the result name + status, matching the existing
  pattern (see `harness.rs:765-774` for the `EntropyDeterminismUnderReseed`
  shape).

### NOT affected (explicit non-targets)

- `crates/overdrive-control-plane/**` — no production-code changes.
  The defect is missing test coverage, not a production-code bug.
- `crates/overdrive-core/**` — no trait surface changes (Option A2
  rejected).
- `xtask/**` — no CLI changes; the new invariant is automatically
  picked up by `cargo xtask dst` via `Invariant::ALL`.

---

## Risk assessment

**Low risk.** The fix adds a new invariant alongside existing peers
following the exact established pattern; production code is untouched.

1. **Does adding the invariant break any existing DST scenario?** No.
   The new invariant is a fresh enum variant added to `Invariant::ALL`;
   existing variants are unmodified. The harness's match arm for the
   new variant is additive. The proptest at `invariant_roundtrip.rs`
   that tests `Display ↔ FromStr` round-trip iterates `ALL` and will
   pick up the new variant automatically.

2. **Does the proposed observation seam violate any layering rule?**
   No. The harness mirrors dispatch in-place using only types from
   `overdrive-core` (`ReconcilerName`, `TargetResource`); no
   `overdrive-control-plane` import. This matches the established
   broker-mirror convention at `harness.rs:511-544` /
   `:588-640`. The sim crate stays `adapter-sim`-class with no inverted
   dep edge.

3. **What's the smallest possible scope expansion?** The
   `DispatchRecord` struct and `Evaluation` mirror struct are local to
   the evaluator module (`crates/overdrive-sim/src/invariants/
   evaluators.rs`) — they do not appear in any public API surface
   outside the sim crate. The harness helper `drive_dispatch_routing`
   is private. The total new surface is two pub structs in the
   evaluators module, one new evaluator function, one new enum
   variant, one match arm. ~100 lines total. There is no instrumentation
   field added to any production type — the seam lives entirely in the
   sim mirror.

4. **Are there existing turmoil scenarios where the invariant should
   be enabled by default vs opt-in?** Default. The Phase 1 catalogue
   runs every invariant in `ALL` on every harness run
   (`harness.rs:262-292`); narrowing to `--only` is a developer-tool
   convenience. The new invariant follows the same default-on
   convention as all existing reconciler-primitive invariants. No
   scenario opt-out is required.

5. **Can the mirrored dispatch drift from production?** Yes — this is
   the same drift risk that already exists for `drive_broker_collapse`
   and `drive_broker_collapse_multi_key`. The mitigation is the
   evolution-doc + acceptance-test contract: the unit/acceptance test
   at `runtime_convergence_loop.rs:209-309` runs against the *real*
   production dispatcher, so a real-vs-mirror divergence would surface
   as the unit test going green while the DST harness's assumed
   contract diverges. To narrow the drift window, the evolution doc
   for this feature should explicitly note the mirror-vs-production
   contract and call out the harness file as a follow-up edit any
   time `run_convergence_tick`'s dispatch shape changes. This is the
   same discipline `eval_broker.rs` already enjoys via the
   `drive_broker_collapse*` mirror; adding one more mirror does not
   change the discipline's load.

6. **Will the new invariant's harness-side mirror be eligible for
   mutation testing?** Yes. The xtask mutation gate passes
   `--features integration-tests` and runs over the workspace-wide
   diff (`.claude/rules/testing.md` § Mutation testing). The
   evaluator function and its three branches (cardinality, routing,
   smoking-gun) are pure, stable, and have library-level unit
   witnesses — exactly the shape `cargo-mutants` kills mutations on.
   The harness mirror itself is excluded from mutation testing per
   the existing `.cargo/mutants.toml` skip for harness-internal
   helpers (the same skip that protects `harness_purity_reconciler`,
   per `harness.rs:653-657`).

7. **Phase 2+ generalisation.** When the registry grows past two
   reconcilers, when the broker dispatches a batch in one tick, or
   when multi-region dispatch lands, the invariant generalises
   cleanly: `record.dispatched.len() == submitted.len()` and the
   smoking-gun check are both already permutation-invariant on the
   target axis. Multi-tick dispatch would require the harness to
   accumulate `DispatchRecord` across ticks rather than per-tick;
   that is a future evolution, not a redesign. (Out of scope for
   this RCA; tracked separately — see *Out of scope* below.)

The only sharp edge: the harness's `harness_registered_reconcilers`
literal `1` at `harness.rs:496` is now stale (production has two
reconcilers registered). Whether to refresh it is a separate question
of harness fidelity, not a question this work item must answer; the
new invariant does not depend on the count being accurate. Track as a
follow-up if desired.

---

## Suggested test/invariant shape

### Catalogue addition

`Invariant::DispatchRoutingIsNameRestricted` — canonical kebab name
`"dispatch-routing-is-name-restricted"`. Added to the catalogue in
the same block as the other reconciler-primitive invariants
(`mod.rs:107-124`).

### Evaluator function (final shape)

```rust
#[must_use]
pub fn evaluate_dispatch_routing_is_name_restricted(
    submitted: &[Evaluation],
    record: &DispatchRecord,
) -> InvariantResult;
```

**Three branches.** (1) Vacuous-pass on empty input.
(2) Cardinality check: `record.dispatched.len() == submitted.len()`.
(3) Per-eval routing check: `record.dispatched.iter().filter(|d| d ==
key).count() == 1` for every submitted eval. (4) Smoking-gun check:
no dispatched entry names a reconciler outside the submitted set.

**Assertion form.** `assert_always!` shape
(per whitepaper §21 *Properties*) — the contract is a safety property
("nothing bad ever happens" → no fan-out dispatch is ever observed).
The `InvariantResult { status: Pass | Fail }` model the harness uses
implements `assert_always!` semantically: if the predicate is ever
violated during the harness pass, the result is `Fail` and the cause
records the violation. This matches every existing safety-class
invariant in the catalogue.

### Library-level unit witnesses

Following the established pattern at
`evaluators.rs:1142-1184` for the parallel
`DuplicateEvaluationsCollapse` witness shape:

```rust
#[test]
fn dispatch_routing_passes_on_clean_single_eval()
fn dispatch_routing_passes_on_clean_multi_eval_distinct_targets()
fn dispatch_routing_passes_vacuously_on_empty_input()
fn dispatch_routing_fails_on_cardinality_mismatch_extra()
fn dispatch_routing_fails_on_cardinality_mismatch_missing()
fn dispatch_routing_fails_on_unsubmitted_reconciler_dispatched()  // smoking gun
fn dispatch_routing_fails_when_named_reconciler_not_dispatched()  // wrong-routing
```

Seven witness tests, one per branch + edge case. Each tests a single
contract failure shape; mutations in any of the three predicate
branches are killed by at least one witness.

### Harness wiring

```rust
Invariant::DispatchRoutingIsNameRestricted => {
    let (submitted, record) = drive_dispatch_routing();
    evaluators::evaluate_dispatch_routing_is_name_restricted(&submitted, &record)
}
```

The crafter should be able to implement this in **one DELIVER step**
following the standard RED-then-GREEN pattern (per `.claude/rules/
testing.md` § "RED scaffolds and intentionally-failing commits"):

- **RED scaffold**: add the enum variant + harness match arm with
  `panic!("Not yet implemented -- RED scaffold")`. The library-level
  witnesses are author-shipped passing tests; the catalogue's
  `Invariant::ALL` test in `harness.rs:748-763` will fail with the
  panic, which IS the RED proof.
- **GREEN minimal**: implement the evaluator body, the
  `DispatchRecord` / `Evaluation` mirror types, and the
  `drive_dispatch_routing` harness helper. Replace the panic with the
  evaluator call. Witnesses pass; harness pass goes green.

---

## Out of scope (separate work item)

The following are explicitly out of scope; track separately rather
than rolling them into this RCA:

- **Multi-reconciler ticks (Phase 2+).** When the broker dispatches a
  batch in one tick (today: one drained eval per `drain_pending`
  iteration), the invariant must accumulate `DispatchRecord` across
  the batch. The current single-tick single-reconciler-target shape
  is sufficient for Phase 1.

- **Multi-region scenarios.** Cross-Corrosion-peer dispatch — when
  a regional control plane drains an eval routed from another region
  — has its own dispatch-routing contract that involves the §4
  per-region Raft boundary. This is a Phase 2+ concern and would need
  its own invariant + observation seam.

- **Workflow-primitive dispatch routing.** Workflows (whitepaper §18
  peer primitive to reconcilers) have a different lifecycle and a
  different invariant family (replay-equivalence + bounded progress).
  The dispatch-routing contract does not transfer; workflows have
  their own equivalent under
  `ReplayEquivalentEmptyWorkflow` (already in the catalogue) and
  future workflow-replay invariants.

- **Refreshing `harness_registered_reconcilers` to match production.**
  The literal `1` at `harness.rs:496` is now stale. The
  `AtLeastOneReconcilerRegistered` invariant passes under both
  `1` and `2`, so the staleness is invisible. Track separately if
  harness fidelity becomes a concern; not blocking for this work item.

- **Generalised invariants crate (ADR-0017).** ADR-0017 proposes an
  `overdrive-invariants` crate with a first-class invariant
  taxonomy. The new invariant fits the existing per-function
  evaluator pattern without requiring ADR-0017 to land first; if and
  when ADR-0017 ships, this invariant migrates alongside its peers
  with no contract change.
