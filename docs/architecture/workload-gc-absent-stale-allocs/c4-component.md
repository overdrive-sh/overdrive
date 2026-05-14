# C4 Component Diagram — Workload GC Touchpoints

Scope: control-plane internals affected by feature `workload-gc-absent-stale-allocs`. System Context (L1) and Container (L2) would be redundant for an intra-reconciler change; one Component diagram is sufficient.

```mermaid
C4Component
  title Component Diagram — WorkloadLifecycle reconciler GC arm

  Container_Boundary(cp, "overdrive-control-plane (control plane process)") {
    Component(broker, "EvaluationBroker", "Rust", "Enqueues per-target reconciler ticks on observation deltas and submit edges")
    Component(runtime, "ReconcilerRuntime", "Rust", "Owns hydrate / dispatch / view-persist loop; pulls from broker")
    Component(shim, "ActionShim", "Rust", "Writes AllocStatusRow (incl. terminal field) and broadcasts LifecycleEvent")
  }

  Container_Boundary(core, "overdrive-core (pure library)") {
    Component(recon, "WorkloadLifecycle::reconcile", "Rust pure fn", "GC arm fires when desired.job is None and non-terminal rows exist — emits StopAllocation with TerminalCondition::Stopped { by: SystemGC }")
    Component(tc, "TerminalCondition + StoppedBy", "Rust enum", "Adds StoppedBy::SystemGC variant (ADR-0037 amendment)")
  }

  ContainerDb(intent, "IntentStore", "redb", "Holds Job aggregates at jobs/<id>; absence drives the GC arm")
  ContainerDb(obs, "ObservationStore", "redb", "Holds alloc_status rows; source of orphan-row evidence and destination of terminal-stamped writes")

  Rel(runtime, intent, "Reads desired Job via hydrate_desired (returns None for orphan workload)")
  Rel(runtime, obs, "Reads non-terminal AllocStatusRows via hydrate_actual filtered by workload_id")
  Rel(runtime, recon, "Invokes pure reconcile(desired, actual, view, tick) once per tick")
  Rel(recon, tc, "Stamps Action::StopAllocation { terminal: Some(Stopped { by: SystemGC }) }")
  Rel(runtime, shim, "Dispatches emitted Actions")
  Rel(shim, obs, "Writes AllocStatusRow.terminal field (terminal-claim durability)")
  Rel(shim, broker, "Observation write triggers re-enqueue; next tick converges remaining rows")
  Rel(broker, runtime, "Drains pending evaluations")

  UpdateLayoutConfig($c4ShapeInRow="3", $c4BoundaryInRow="2")
```

**What the diagram shows:**

- **No new containers, no new components.** The GC arm extends `WorkloadLifecycle::reconcile` (a fn inside the existing component) and the `StoppedBy` enum gains one variant.
- **The data-flow loop converges on its own.** Tick → reconcile sees orphan rows → emits Stops → ActionShim writes terminal rows → observation delta re-enqueues target → next tick sees terminal rows only → emits zero Actions → arm quiesces.
- **Two read sources, one write destination.** Intent for `desired.job`, Observation for `actual.allocations`. Writes go to Observation only (terminal-stamped rows + lifecycle events).

**What the diagram does NOT show (and why):**

- No new arrows out of `overdrive-core` — the crate stays pure. All I/O is at the runtime/shim boundary.
- No race-resolver component — the per-target hydration boundary IS the LWW resolver; no separate component needed.
- No "OrphanDetector" or "GC sweeper" component — Option B's shape was rejected in favour of in-place extension.
