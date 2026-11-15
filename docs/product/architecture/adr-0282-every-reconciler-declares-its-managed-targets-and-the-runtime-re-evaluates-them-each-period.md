# ADR-0282 — Every reconciler declares its managed targets and the runtime re-evaluates them each period

## Status

Proposed. 2026-11-13. Author: Morgan (`nw-solution-architect`), DESIGN wave, APPLICATION
layer, feature `reconciler-convergence-guarantees`. The user approved decision D-1 of the
feature delta on 2026-11-13. The ADR stays `Proposed` until independent design review
completes.

**Supersedes in part (proposed), ADR-0084:** the per-reconciler `resync_schedule`
declaration (§1 Piece A, §4) and the interest router's periodic relist (Amendment
2026-08-23). ADR-0084's `interests()` edge-wake declaration, the List-then-Watch router,
the `Lagged` relist and the broker LWW rules stand. ADR-0084 is implemented, so on
acceptance the architect adds a pointer line to its status; its decision text is not
edited. Does not amend ADR-0035, ADR-0036 or ADR-0086's ownership ruling (the hydration
boundary is where the new method lives).

## Context

Convergence is level-triggered inside `reconcile`, but the decision to run `reconcile`
for a target is edge-triggered plus two partial backstops. The grounded verification
(`.context/convergence-verification.md`, HEAD `fb71b400f`) found these target-keyed wake
gaps, each reachable in a real deploy:

- A hydrate error (`run_convergence_tick_inner`, `?` on the two hydrate calls) or a View
  persist error ends the evaluation before the self-requeue. Nothing else re-submits it.
- `service-map-hydrator` has no wake hook. After a `Failed` dataplane result, the next
  tick falls inside the backoff window, emits `Noop` and returns no deadline. A steady
  service gets no further backend-row write.
- The router's periodic relist enumerates `alloc_status` rows only. A fresh deploy whose
  first hydrate failed has intent and no `alloc_status` row, so no relist reaches it.
  `ProbeResult` has no list leg at all.
- `register` submits no boot evaluation, so existing intents are only woken through the
  router's `alloc_status` list.

ADR-0084 split the backstop by reconciler partition: row-backed reconcilers rely on the
router relist, host-backed ones declare `resync_schedule`. The split exists because the
runtime cannot enumerate a reconciler's targets generically. It leaves a reconciler's
backstop dependent on which observation family it happens to read, and it makes
`resync_schedule` and the relist two mechanisms for one need. The user ruled that targets
are enumerated from intent and from observation, not from `alloc_status` alone (D4).

Kernel and appliance facts: none. The change is control-plane only; every fault is an
Overdrive-internal failure (a store read error, a redb write error), which is the
appliance threat model.

## Decision

1. Each reconciler declares, through its own impure hydration surface, the set of
   targets it manages: every target for which it has desired or actual state, in its own
   target shape, enumerated from intent and from observation.
2. The runtime runs one resync pass per period. A pass asks every registered reconciler
   for its managed targets and submits one `Immediate` evaluation per target through
   `ReconcilerRuntime::submit_evaluation`. The first pass runs at boot. Passes are
   anchored (no drift) and run at most once per period.
3. An evaluation that ends in a hydrate error, a View persist error or a dispatch error
   is not requeued locally. The next resync pass is its retry. The runtime logs the
   failure with the reconciler, target and typed error.
4. `resync_schedule`, `ResyncSchedule`, `ResyncScope`, the cadence table and the router's
   periodic relist are deleted in the same cut. `interests()` and the router's
   List-then-Watch and `Lagged` relist remain as edge wakes.

5. The pass is the level-triggered backstop for every wake, present and future. Host-backed
   reconcilers have no row to wake them. GH #275 (open) tracks a host-edge wakeup for them.
   A host watcher built under #275 submits through the same
   `ReconcilerRuntime::submit_evaluation` as every other wake. The pass stays beside it:
   #275 itself requires the cadence sweep to remain, because netlink and inotify drop
   events under pressure and state orphaned by a reboot produces no event.
6. Nothing replaces per-reconciler cadence. No reconciler can ask for a faster sweep than
   the global period. Lower host-drift latency comes from #275's edge wakes, not from a
   shorter period.

The period is a compile-time constant of 30 s, not an operator setting. That is a scope
limitation, tracked by GH #356 ("Make the reconciler resync period operator-configurable")
(feature delta D-2).

## Alternatives considered

| Option | Why it loses |
|---|---|
| Per-reconciler `resync_schedule` for every reconciler | The schedule resolves to targets through `ResyncScope`, which has one variant (`LocalNode`). Per-workload targets need a scope that enumerates managed resources. That is this ADR's enumeration under another name, kept as a second periodic mechanism beside the router relist. |
| Fix each gap locally: requeue on hydrate and persist errors, `next_evaluation_at` on the hydrator, seed existing intents at boot | Four mechanisms for one class. A local requeue on a persistent error (an undecodable intent, a refused hydrate) loops without a backoff memory. Each new reconciler must remember to add its own. The fresh-deploy-without-row and `ProbeResult` gaps are not closed by any of them. |
| Keep ADR-0084 and extend the router relist to intent and more row families | The router keys on observation rows and derives workload targets inline from them. Extending it to every target shape moves per-reconciler target knowledge into the router, which does not own the reconciler's hydration (ADR-0086). |
| Additive: keep `resync_schedule`, add the pass only for row-backed reconcilers | Leaves three overlapping periodic mechanisms and the "never both" discipline in `reconcilers.md`. Both current `resync_schedule` users (`vm-reclamation`, `gateway-svid-lifecycle`) declare 30 s, equal to the new period, so removing it loses no behaviour. |

## Consequences

Positive:

- One backstop for every reconciler and every target shape; a reconciler author cannot
  forget it, because the method is required.
- The three error paths, the hydrator wake, the fresh-deploy gap and the `ProbeResult`
  gap close with one mechanism and no per-error requeue.
- `build_cadence_table` and its stale comment disappear.

Negative and bounds:

- Evaluations per period equal the sum over reconcilers of their managed-target counts.
  Today the router submits one per interested reconciler per `alloc_status` target. The
  increase is the hydrator's service targets and intent-only targets.
- A pass is a burst. Evaluations admit oldest first under the 8-evaluation cap, so an
  edge wake submitted during a burst queues behind it. The broker collapses same-key
  submits (`cancelled` counts them) and merges a pending `NotBefore` to `Immediate`; a
  reconciler's own backoff gate makes an early evaluation a no-op.
- Worst-case recovery from a hydrate or persist error is one period plus queue time.
- Host-drift latency is bounded by the period (30 s) until #275 adds host edge wakes. The
  two `resync_schedule` users declare 30 s today, so no current behaviour slows down.
- The broker gains a read-only accessor for its pending keys so the formal protocol
  model's conformance driver has a projection. Its signature is pinned in the feature
  delta and approved with D-1.
- `workflow-lifecycle` is included by the uniform method and enumerates no targets in
  production today (empty registry); no dedicated wake is added for it.

Links: [feature delta](../../feature/reconciler-convergence-guarantees/feature-delta.md)
(interface contract D-1, `Formal protocol model`), ADR-0084, ADR-0086, ADR-0102, ADR-0273, GH #275, GH #356.
