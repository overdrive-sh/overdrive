# Audit — reconciler-handoff topology (post UI-04 + UI-05)

**Date**: 2026-05-21
**Investigator**: nw-troubleshooter (Rex)
**Trigger**: three sequential reconciler-handoff gaps surfaced during step 02-04
walking-skeleton (UI-04 Service-arm hydrate; UI-05 bridge → hydrator handoff;
new gap surfaced this session: WorkloadLifecycle → bridge for long-lived
workloads). Goal: enumerate every production handoff before applying the
new fix so we land any additional fixes in a single combined commit.
**Method**: per-reconciler audit + per-edge classification + architecture.md
§ 3 step-by-step trace. Pure static analysis (Read / Grep / Glob).

---

## Trigger / convergence-loop primer (load-bearing for the audit)

The convergence loop is **edge-triggered**, not periodic. The spawn loop
at `crates/overdrive-control-plane/src/lib.rs:1114-1163`
(`spawn_convergence_loop`) does:

```rust
loop {
    let pending = state.runtime.broker().drain_pending();   // L1130-1133
    for eval in pending {
        run_convergence_tick(&state, &eval.reconciler, ...).await;
    }
    clock.sleep(cadence).await;                              // L1158-1161
}
```

`drain_pending()` returns only evaluations that were `submit`ted since the
prior drain. There is no per-reconciler periodic wake-up; a reconciler that
is **never enqueued never ticks**. Self-re-enqueue
(`reconciler_runtime.rs:971-976`) only fires when reconcile emitted at least
one non-Noop action — a reconciler whose `desired==actual` cannot bootstrap
itself.

**This is the load-bearing property the audit checks against**: for every
reconciler we must identify the production code paths that submit it to the
broker; any reconciler whose enqueue depends on a state transition that the
production code path does not detect is a wiring gap.

---

## Registered reconcilers (production boot)

Cited from `crates/overdrive-control-plane/src/lib.rs::run_server_with_obs_and_driver`:

| # | Reconciler | Registration site | Construction site |
|---|---|---|---|
| 1 | `noop-heartbeat` | `lib.rs:832` | `lib.rs:1172` (`noop_heartbeat()`) |
| 2 | `workload-lifecycle` | `lib.rs:833` | `lib.rs:1188` (`workload_lifecycle()`) |
| 3 | `backend-discovery-bridge` | `lib.rs:957` | `lib.rs:1194+` (`backend_discovery_bridge(host_ipv4, node_id)`) |
| 4 | `service-map-hydrator` | `lib.rs:969` | `lib.rs:1233-area` (`service_map_hydrator()`) |

UI-05 added (4) at `lib.rs:969`. The set is complete; no other
`runtime.register(...)` calls exist on the production boot path (grep verified).

---

## Per-reconciler audit

### R1. `noop-heartbeat`

- **Triggers**: enqueued initially by … nothing in production code that
  I could find. The reconciler exists primarily to feed the
  `AtLeastOneReconcilerRegistered` invariant and the
  `ReconcilerIsPure` twin-invocation check
  (`reconciler.rs:987-991`). Its `reconcile` body always emits
  `vec![Action::Noop]` (`reconciler.rs:1043`) — `Action::Noop` is short-
  circuited at the action shim (`action_shim/mod.rs:417`), and the
  self-re-enqueue gate `has_work` (`reconciler_runtime.rs:971`) is true
  for *any* non-empty `actions`, so once enqueued it would self-re-enqueue
  indefinitely. **Not in scope for this audit** (the Phase 1 contract is
  proof-of-life only; not wired into any production observable).

- **Observes**: nothing (Unit state).
- **Emits**: `Action::Noop` only.
- **Downstream consumers**: none.

### R2. `workload-lifecycle`

- **Triggers** (production):
  1. **Submit handler** — `handlers.rs:378` / `:409` / `:676` call
     `enqueue_workload_lifecycle_eval` (defined `handlers.rs:57-76`) on
     every POST `/v1/jobs` (`Inserted` and `Unchanged` branches) and on
     stop. This is the seed.
  2. **Exit observer** — `worker/exit_observer.rs:233` enqueues
     `workload-lifecycle` for `job/<workload_id>` after every
     successful `obs.write(AllocStatusRow)` driven by a driver exit
     event.
  3. **Self-re-enqueue** — `reconciler_runtime.rs:971-976` when
     `reconcile` emits work or the View signals backoff-pending
     (`view_has_backoff_pending`, `reconciler_runtime.rs:1004-1027`).

- **Observes** (hydrate, `reconciler_runtime.rs:1042-1074` /
  `:1369-1407`):
  - `IntentStore::get(IntentKey::for_workload(workload_id))` for the
    WorkloadIntent (Job or Service).
  - `IntentStore::get(IntentKey::for_workload_kind(...))` for the
    kind discriminator (ADR-0047).
  - `IntentStore::get(IntentKey::for_workload_stop(...))` for the stop
    intent (ADR-0027).
  - `ObservationStore::alloc_status_rows()` filtered to this workload.

- **Emits** (`reconciler.rs:1445-1786` — five branches):
  - `Action::StopAllocation` (L1445, L1494) — when `desired_to_stop`.
  - `Action::FinalizeFailed` (L1611, L1680) — at restart-budget ceiling.
  - `Action::RestartAllocation` (L1720) — mid-budget restart.
  - `Action::StartAllocation` (L1786) — fresh alloc.
  - `Action::ReleaseServiceVip` (L1927 via helper) — terminal Service.

- **Downstream consumers**: action-shim arms in
  `action_shim/mod.rs:457` (FinalizeFailed), `:507` (Start), `:598`
  (Restart), `:698` (Stop), `:784` (ReleaseServiceVip).

### R3. `backend-discovery-bridge`

- **Triggers** (production):
  1. **Exit observer** — `worker/exit_observer.rs:253` enqueues
     `backend-discovery-bridge` for `job/<workload_id>` after every
     successful `obs.write(AllocStatusRow)` driven by a driver exit
     event. **Only fires on exit events** (see § "Per-edge
     classification" below).
  2. **Self-re-enqueue** — `reconciler_runtime.rs:971-976` when
     reconcile emits actions (i.e. when bridge already fired once and
     produced drift).

  Notably absent from triggers: the submit handler (does NOT enqueue the
  bridge), and **no enqueue site fires on Pending → Running transitions**.

- **Observes** (`reconciler_runtime.rs:1109-1124` /
  `:1465-1493`):
  - Desired side: via `hydrate_bridge_desired_listeners` —
    `IntentStore::get(IntentKey::for_workload)` then matches
    `WorkloadIntent::Service(ServiceV1)`; reads
    `ServiceVipAllocator::get(&spec_digest)`.
  - Actual side: `ObservationStore::alloc_status_rows()` filtered to
    `workload_id == this && state == Running`.

- **Emits** (`backend_discovery_bridge.rs:369-409`):
  - `Action::WriteServiceBackendRow` (L369) on fingerprint drift.
  - `Action::EnqueueEvaluation` (L405) for the
    `service-map-hydrator` — UI-05 paired emit.

- **Downstream consumers**:
  - `WriteServiceBackendRow` → `action_shim/mod.rs:800` →
    `write_service_backend_row::dispatch` → ObservationStore write.
  - `EnqueueEvaluation` → `action_shim/mod.rs:810` →
    `enqueue_evaluation::dispatch` → `EvaluationBroker::submit`.

### R4. `service-map-hydrator`

- **Triggers** (production):
  1. **Bridge UI-05 handoff** — bridge emits
     `Action::EnqueueEvaluation { reconciler: "service-map-hydrator",
     target: "service/<id>" }` (`backend_discovery_bridge.rs:405`)
     → action-shim wrapper at `action_shim/mod.rs:810` submits to broker.
  2. **Self-re-enqueue** — `reconciler_runtime.rs:971-976` when reconcile
     emits actions (i.e. drift between `desired.fingerprint` and
     `actual.fingerprint`). Self-re-enqueue stops the moment
     `actual.fingerprint == desired.fingerprint` (steady state per
     ADR-0042).

  Notably absent from triggers: nothing else. The bridge → hydrator
  handoff is the **only** production path that enqueues the hydrator.
  This is fine *because* the bridge enqueues the hydrator on every
  fingerprint-drift write — the hydrator only needs to wake up when
  the bridge's `service_backends` row changes.

- **Observes** (`reconciler_runtime.rs:1076-1102` / `:1408-1452`):
  - Desired: `ObservationStore::service_backends_rows(&service_id)`.
  - Actual: `ObservationStore::service_hydration_results_rows(&service_id)`.

- **Emits** (`reconciler.rs:2339-2344`):
  - `Action::DataplaneUpdateService`.

- **Downstream consumers**:
  - `action_shim/mod.rs:753` → `dataplane_update_service::dispatch` →
    `Dataplane::update_service` + writes `ServiceHydrationResultRow`.

---

## Per-handoff edge classification

A "handoff" is a producer → consumer edge where the producer's emission /
side-effect is the only thing that enables the consumer to tick on the
new state.

| # | Producer | Consumer | Trigger mechanism | Status | Citation |
|---|---|---|---|---|---|
| E1 | HTTP `POST /v1/jobs` | `workload-lifecycle` | `enqueue_workload_lifecycle_eval` | WIRED-CORRECTLY | `handlers.rs:378/409` |
| E2 | HTTP `DELETE /v1/jobs/<id>` (stop) | `workload-lifecycle` | `enqueue_workload_lifecycle_eval` | WIRED-CORRECTLY | `handlers.rs:676` |
| E3 | `WorkloadLifecycle` → action-shim `StartAllocation` → driver | `workload-lifecycle` (next tick to observe Running) | Self-re-enqueue (`has_work`) | WIRED-CORRECTLY | `reconciler_runtime.rs:971-976` |
| E4 | `WorkloadLifecycle` (backoff pending) | `workload-lifecycle` (next tick after backoff window) | `view_has_backoff_pending` predicate self-re-enqueue | WIRED-CORRECTLY | `reconciler_runtime.rs:1020-1025` |
| E5 | Driver exit event (process crashed/exited) | `workload-lifecycle` | `exit_observer` submit | WIRED-VIA-EXIT-OBSERVER-ONLY | `exit_observer.rs:233-236` |
| E6 | Driver exit event | `backend-discovery-bridge` | `exit_observer` submit | WIRED-VIA-EXIT-OBSERVER-ONLY | `exit_observer.rs:253-256` |
| E7 | `action_shim::StartAllocation` writes Running row | `backend-discovery-bridge` (needs to fire to write `ServiceBackendRow`) | **none** | **MISSING** | `action_shim/mod.rs:582-589` writes the row but does NOT enqueue the bridge |
| E8 | `BackendDiscoveryBridge` → `WriteServiceBackendRow` (+ paired UI-05 emit) | `service-map-hydrator` | `Action::EnqueueEvaluation` → action-shim → broker | WIRED-CORRECTLY | `backend_discovery_bridge.rs:405`; `action_shim/mod.rs:810`; `enqueue_evaluation.rs:58` |
| E9 | `ServiceMapHydrator` → `DataplaneUpdateService` writes `ServiceHydrationResultRow::Completed` | `service-map-hydrator` (next tick observes convergence; clears retry memory) | Self-re-enqueue (`has_work` while drift remains; stops when `actual.fp == desired.fp`) | WIRED-CORRECTLY | `reconciler_runtime.rs:971-976`; `reconciler.rs:2355-2362` |
| E10 | Submit handler | `backend-discovery-bridge` (first hydrate after submit) | **none** | **MISSING** (cold-start variant of E7) | `handlers.rs:355-410` — only enqueues `workload-lifecycle` |
| E11 | Submit handler | `service-map-hydrator` (first hydrate after submit) | **none** (chains through E8 once bridge fires) | WIRED-INDIRECTLY-VIA-BRIDGE | (depends on E7 firing first) |

### The "WIRED-VIA-EXIT-OBSERVER-ONLY" rows (E5, E6) are the gap pattern

The exit observer only invokes its post-`obs.write` enqueue (lines
233-256) when:

- An `ExitEvent` arrives on the driver's exit-event mpsc channel
  (`exit_observer.rs:198`).
- `run_with_retry` returns `RetryOutcome::Wrote` (L204-205) — i.e. the
  observer successfully wrote a transition row (the row the exit
  observer writes is a **terminal-state row** for an exited workload;
  the action shim writes the Pending → Running row at
  `action_shim/mod.rs:582`, which is a different path).

For a long-lived workload (e.g. the walking-skeleton Python echo
loop), the process never exits ⇒ the exit observer never fires ⇒
the bridge is never enqueued via E6. Even on the bridge's *first*
required tick (cold start, no exit), the only production path that
could enqueue it is E10 (submit handler) — which is MISSING.

E5 is the analogue for `workload-lifecycle`, but `workload-lifecycle`
is rescued by E1/E3 — the submit handler enqueues it explicitly and
its own emission of `StartAllocation` triggers self-re-enqueue on
the next tick after the Running row exists.

### Compare populations (per debugging.md § 5)

The fix-pattern that already exists (E8 — bridge → hydrator) and the
gap (E7 — Running write → bridge) have identical shape:

| Aspect | E8 (wired, UI-05) | E7 (gap) |
|---|---|---|
| Triggering write | `obs.write(ServiceBackend)` at `action_shim/write_service_backend_row.rs` | `obs.write(AllocStatus { state: Running })` at `action_shim/mod.rs:582` |
| Wake mechanism | `Action::EnqueueEvaluation` paired with the write | (none) |
| Producer reconciler | `BackendDiscoveryBridge` (dual-emit) | `WorkloadLifecycle` (`StartAllocation` is single-emit) |
| Consumer | `service-map-hydrator` | `backend-discovery-bridge` |

The shape-difference IS the third gap. The fix lives at the same
altitude as UI-05: `WorkloadLifecycle::reconcile` dual-emits
`Action::EnqueueEvaluation { reconciler: "backend-discovery-bridge",
target: "job/<workload_id>" }` alongside `Action::StartAllocation`,
**OR** the action-shim writes after `StartAllocation` enqueue the
bridge implicitly. Per the UI-05 design discussion
(`reconciler.rs:760-772`) the reconciler-side dual-emit was
deliberately preferred over action-shim-implicit enqueue ("would
couple the action shim to reconciler-pair-specific knowledge"); the
same rationale applies here.

E10 (submit-handler → bridge) is a cold-start variant of the same
fix. It can be solved at the submit handler (mirror
`enqueue_workload_lifecycle_eval`), OR it can be implicitly resolved by
E7 — once E7 is wired, the bridge fires automatically when
`StartAllocation` produces the first Running row. The current
production semantics already follow the "bridge waits for Running"
shape (its `actual.running` is empty until Running rows exist), so
the bridge's first useful tick is after E7 fires; enqueuing it at
submit time would just produce a Noop tick. **Recommendation: don't
fix E10 separately — E7's fix subsumes it.**

---

## architecture.md § 3 trace verification

Walking-skeleton steps 1-12, mapped to current production code:

| Step | Architecture claim | Production status | Citation |
|---|---|---|---|
| 1 | Operator submits Service spec | OK | `handlers.rs:355+` |
| 2 | `submit_workload`: validate + project + allocate VIP + persist intent + return echo | OK | `handlers.rs:355-410` |
| 3 | `WorkloadLifecycle.reconcile` emits `StartAllocation` | OK | `reconciler.rs:1786`; enqueued at submit by E1 |
| 4 | Action shim → ExecDriver spawn → Pending → Running; AllocStatusRow written by exit-observer / action shim | OK (Running row written by `action_shim/mod.rs:582`) | — |
| **5** | **"Broker re-enqueues `BackendDiscoveryBridge` for the workload (same enqueue site as `WorkloadLifecycle`, keyed by `WorkloadId`)"** | **MISSING for Pending → Running transitions.** The only enqueue site is `exit_observer.rs:253-256`, which is exit-event-only. For long-lived workloads (every Service workload by design), step 5 never fires. | `exit_observer.rs:253` is the only `backend_discovery_bridge_name()` submit |
| 6 | Runtime hydrates the bridge (desired = ServiceV1.listeners + assigned_vip; actual = Running allocs filtered) | OK (`reconciler_runtime.rs:1109-1124` / `:1465-1493`) — but only reached if step 5 fires | — |
| 7 | `BackendDiscoveryBridge.reconcile` emits `Action::WriteServiceBackendRow` on drift | OK if reached | `backend_discovery_bridge.rs:369` |
| 8 | Action shim writes `ObservationRow::ServiceBackend` | OK | `action_shim/mod.rs:800`; `write_service_backend_row.rs` |
| 9 | UI-05: bridge dual-emits `Action::EnqueueEvaluation` for hydrator | OK | `backend_discovery_bridge.rs:405` |
| 10 | `ServiceMapHydrator.reconcile` emits `DataplaneUpdateService` | OK if reached | `reconciler.rs:2339` |
| 11 | Action shim calls `EbpfDataplane::update_service` + writes `ServiceHydrationResultRow::Completed` | OK | `action_shim/mod.rs:753`; `dataplane_update_service::dispatch` |
| 12 | Hydrator on next tick converges | OK | `reconciler.rs:2355-2362` (self-re-enqueue stops when convergent) |

**Step 5 is the wiring gap.** Architecture.md's claim "same enqueue
site as `WorkloadLifecycle`" implies the bridge gets enqueued
wherever `WorkloadLifecycle` does — but that's only true at the
exit-observer site (E5/E6) and at no other site. The submit-handler
enqueue (E1) is `workload-lifecycle`-only.

---

## Findings

### F1 (NEW — confirmed) — WorkloadLifecycle → BackendDiscoveryBridge missing for Pending → Running

**Edge**: E7 (and its cold-start variant E10).

**Symptom**: For long-lived Service workloads (the entire Service
class), after `Action::StartAllocation` produces a Running
`AllocStatusRow` at `action_shim/mod.rs:582`, nothing enqueues
`backend-discovery-bridge`. The bridge therefore never ticks, never
observes the Running alloc, never emits `WriteServiceBackendRow`,
and the entire steps 6-12 chain stays cold.

**Citation**: `action_shim/mod.rs:507-590` (StartAllocation arm
writes the row; no broker.submit for bridge); `handlers.rs:57-76`
(submit handler only enqueues `workload-lifecycle`);
`exit_observer.rs:230-258` (the only bridge enqueue site,
exit-event-gated).

**Fix pattern**: mirror UI-05. WorkloadLifecycle's reconcile body
dual-emits `Action::EnqueueEvaluation { reconciler:
"backend-discovery-bridge", target: "job/<workload_id>" }` alongside
each `Action::StartAllocation` (and also alongside
`Action::RestartAllocation` / `Action::StopAllocation` /
`Action::FinalizeFailed` — every transition that changes the Running
set the bridge cares about). Action-shim dispatch already exists
(`action_shim/mod.rs:810` is generic over reconciler name + target).

**Estimated LOC**: ~15-25 lines in
`crates/overdrive-core/src/reconciler.rs` (the four arms that mint
the allocation actions). Test scaffold update: extend the existing
bridge-handoff DST evaluator (S-BDB-19) to also cover the
WorkloadLifecycle → bridge edge (it currently only exercises the
bridge → hydrator edge per memory ID #41832).

### F2 — Is there a fourth gap? — searched, none found

I enumerated every `Action::*` variant (`reconciler.rs:479-799`)
and traced each to its action-shim consumer:

| Variant | Consumer | Downstream wake | Status |
|---|---|---|---|
| `Noop` | short-circuit `action_shim/mod.rs:417` | — | OK |
| `HttpCall` | short-circuit `:417` (Phase 3) | — | N/A |
| `StartWorkflow` | short-circuit `:417` (Phase 3) | — | N/A |
| `StartAllocation` | `:507` driver.start + obs.write | (none — gap F1) | **GAP F1** |
| `RestartAllocation` | `:598` stop+start + obs.write | (none — gap F1 extension) | **GAP F1** |
| `StopAllocation` | `:698` driver.stop + obs.write | (none — gap F1 extension) | **GAP F1** |
| `FinalizeFailed` | `:457` obs.write Failed row | (none — gap F1 extension) | **GAP F1** |
| `DataplaneUpdateService` | `:753` Dataplane + obs.write | hydrator self-re-enqueue covers it | OK |
| `ReleaseServiceVip` | `:784` allocator release | (none needed — terminal action, no follow-on reconciler) | OK |
| `WriteServiceBackendRow` | `:800` obs.write | hydrator wake via paired EnqueueEvaluation (UI-05) | OK |
| `EnqueueEvaluation` | `:810` broker.submit | (transport, not state) | OK |

The only emit site without a downstream wake mechanism is the
WorkloadLifecycle action set (`StartAllocation`,
`RestartAllocation`, `StopAllocation`, `FinalizeFailed`) — all
four members of the same gap F1. **No fourth distinct gap was found.**

The `ReleaseServiceVip` arm has no downstream reconciler dependency
by design — it's a terminal cleanup action (the allocator releases
the VIP back to the pool; the next `allocate(&fresh)` would
re-issue it, and there is no reconciler that observes "the
allocator has free VIPs"). Confirmed by tracing
`release_service_vip::dispatch` — it returns Ok with no observable
side-effect outside the allocator's own memo.

### F3 — Defensive observation: the `service_hydration_results` Failed row has no explicit retry trigger

The hydrator's `should_dispatch` (`reconciler.rs:2327`) reads
`view.retries.get(service_id)` + `tick.now_unix` to decide whether
to re-dispatch after a Failed `ServiceHydrationResultRow`. The
retry happens on the **next time the hydrator is enqueued** —
which is when:

(a) the bridge writes a new fingerprint (E8 fires again), or
(b) the hydrator self-re-enqueues because it emitted an action
this tick (E9).

If a `DataplaneUpdateService` returns Failed AND the bridge does
NOT write a new fingerprint, the hydrator's retry deadline can
expire without anyone enqueuing the hydrator. The hydrator
would then wake on the next bridge tick (whenever the bridge
writes the next row).

This is a latent gap but **does not fire in the Phase 2.2 single-node
walking skeleton** (the dataplane is `EbpfDataplane` writing to
local BPF maps; failures are rare and the bridge re-writes on every
alloc-set change, which happens at least at exit). I am flagging
it for completeness but **not recommending it for the combined-fix
commit** — it requires a separate design discussion about whether
the hydrator should self-tick on a deadline (cron-shaped) or
whether the retry semantics should be reshaped. Out of scope here.

---

## Recommendation

**Single combined commit** addressing F1 (WorkloadLifecycle → bridge
for Pending → Running transitions and every other alloc-set
change).

**Mechanism**: dual-emit `Action::EnqueueEvaluation` from
`WorkloadLifecycle::reconcile` alongside every action that changes
the Running alloc set:
- alongside `Action::StartAllocation` (the cold-start case — fixes the
  walking-skeleton long-lived workload symptom)
- alongside `Action::RestartAllocation` (the crash-recovery case —
  bridge needs to re-write when alloc-id flips Running ↔ Running)
- alongside `Action::StopAllocation` (operator stop — bridge needs
  to remove backend entries from the row)
- alongside `Action::FinalizeFailed` (budget-exhausted terminal —
  bridge needs to remove backend entries)

This is exactly parallel to UI-05's bridge → hydrator dual-emit at
`backend_discovery_bridge.rs:405`.

**Why dual-emit at the reconciler** (rather than action-shim implicit
enqueue): same rationale as UI-05 (`reconciler.rs:760-772`) — keeping
the handoff explicit at the reconciler surface keeps the
cross-reconciler dependency readable in one place, rather than
requiring readers to consult the action-shim dispatch source to
discover the wiring.

**LOC estimate**: ~25-40 production LOC across the four arms in
`reconciler.rs:1445-1786` plus the helper-pair pattern already used
at `backend_discovery_bridge.rs:401-408` (ReconcilerName::new +
TargetResource::new with `expect`). Plus test extension: extend the
bridge-handoff DST evaluator (S-BDB-19) per memory ID #41832 to
cover the new edges.

**Migration / cleanup of the exit_observer enqueue (E6 at
`exit_observer.rs:253-256`)**: optional but recommended. With F1's
fix, `Action::StopAllocation` / `FinalizeFailed` enqueue the bridge
via reconciler dual-emit (the same `Action::EnqueueEvaluation`
shape). The exit-observer's bridge enqueue becomes redundant for
the terminal-transition case. Defer this cleanup to a follow-up
because removing it is a single-cut migration that needs explicit
scope (per the user's "single-cut greenfield migrations" rule in
CLAUDE.md). The combined fix commit should add the reconciler
dual-emit only; redundancy removal is a separate concern.

**Confidence**: high. The shape is identical to UI-05's pattern,
the action-shim dispatch arm for `Action::EnqueueEvaluation` is
already generic, the broker LWW semantics (per ADR-0013 §8)
collapses duplicate enqueues so over-emission is safe, and
self-re-enqueue handles steady-state.

---

## Not audited (depth-over-breadth tradeoff per timeout)

The audit is complete for the production code path enumerated.
Items I did NOT chase because they are out of scope per the user's
prompt:

- Sim adapter behaviour (UI-05 verified DST evaluator's
  spurious-pass; per prompt "don't re-audit").
- Tier 3 Lima execution (static analysis only per prompt).
- The action-shim's per-arm implementation details beyond
  "who calls whom" (per prompt: "only WHO calls whom matters
  at this altitude").
- The bridge's reconcile body shape (per prompt: "already
  audited extensively").
- Phase 3 surfaces (`HttpCall`, `StartWorkflow`) — Phase 1 stubs
  per `reconciler.rs:484-521`; the workflow runtime lands later.
- The full DST evaluator catalogue — I read only the names that
  surfaced via grep. A follow-up DST coverage audit would be
  separate work.

The audit's evidence is sufficient to land the F1 fix as a single
combined commit; no further investigation is required before
implementation.
