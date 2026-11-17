# Conformance driver — `evaluation_wake` (`reconciler-convergence-guarantees`, GH #351)

Driver: `crates/overdrive-control-plane/src/reconciler_runtime/quint_evaluation_wake.rs`
(registered in `reconciler_runtime.rs` under `#[cfg(all(test, feature = "integration-tests"))]`, `placement!(host_pure)`).
Spec, referenced in place: `specs/quint/reconciler-evaluation-wake/evaluation_wake.qnt`, conformance main `wake_conf`
(module `evaluation_wake`, ITF variables `wake_conf::evaluation_wake::*`). Contract shape: bounded-change.

## Owner and composition

The production owner is `ReconcilerRuntime` (the evaluation broker behind `BrokerDrain`) with the four production reconcilers the
model's keys name registered (`workload-lifecycle`, `service-lifecycle`, `svid-lifecycle`, `service-map-hydrator`), inside an `AppState`
composed as `restart_authority` composes it, with `SimObservationStore`, `SimDataplane`, `SimDriver`, `SimClock`, the sim CA and the
shared network test ports at the driven ports, and the production `RedbViewStore` at the View port. The only test-local type is
`FaultableViewStore`, a wrapper over `RedbViewStore` whose durable write can be made to fail (the sim `SimViewStore` cannot be used:
`overdrive-sim` links a different build of this crate, so it does not implement this crate's `ViewStore`). It adds no production surface.

The production `Action::EnqueueEvaluation` handoffs are on. The spec's `Finish` models them (`HAND`), so the driver suppresses nothing and
the action shim submits them during `run_convergence_tick`.

## Key to production pair (fixed)

All four keys sit on one Service workload `orders` that declares a listener.

| Model key | Model reconciler | Production reconciler | Target |
|---|---|---|---|
| `k1` | `r4W` | `workload-lifecycle` | `workload/orders` |
| `k3` | `r1S` | `service-lifecycle` | `workload/orders` (same target as `k1`) |
| `k4` | `r3V` | `svid-lifecycle` | `workload/orders` (same target as `k1`, `k3`) |
| `k2` | `r2M` | `service-map-hydrator` | `service/<id>` of the `orders` listener (alone on its target) |

The abstract names sort like the production names, so the pass order `r1S, r2M, r3V, r4W` is the production `BTreeMap` order
`service-lifecycle, service-map-hydrator, svid-lifecycle, workload-lifecycle`; the driver calls `resync_reconciler` in that order
(`PASS_ORDER`, 4 calls per pass). `key_name` / `target_of` implement the table; a pending key outside it projects as
`unmapped:<reconciler>@<target>`, so a surplus submit shows as a `pending` mismatch rather than an error. The `orders` listener's
`ServiceId` is fixed at composition by a scratch allocator over a scratch intent store (no effect on the world) so the key table can name
`k2`'s target before any VIP is assigned; the world change that assigns the VIP asserts the production allocator yields the same id.

## Tests (one per named run of `wake_conf`, plus randomized traces)

| Test (`quint_evaluation_wake_*`) | Run | Critical scenario | Invariants / properties the run asserts in the spec |
|---|---|---|---|
| `healthy_edge_converges` | `healthy_edge_converges` | happy path: edge wake converges, the start hands off to `k3`/`k4`, `k3`'s publish hands off to `k2` | `Safety` |
| `hydrate_error_recovers_by_pass` | `hydrate_error_recovers_by_pass` | slice 01 fault 1 | `Safety`; supports `ProgressByPass` |
| `persist_error_recovers_by_pass` | `persist_error_recovers_by_pass` | slice 01 fault 2 | `Safety`; `ProgressByPass` |
| `crash_between_fsync_and_dispatch_recovers_by_boot_pass` | same name | crash between View fsync and dispatch | `Safety`; `ProgressByPass` |
| `failed_dataplane_inside_backoff_recovers_without_row_write` | same name | slice 01 fault 3 (D-3) | `RetentionSchedulesBackedOffKey`, `Safety` |
| `fresh_deploy_without_row_is_evaluated` | same name | intent, no row, no edge wake | `Safety`; `ProgressByPass` |
| `enumeration_failure_of_one_reconciler_does_not_starve_others` | same name | enumeration failure of `r4W`; `k4` is evaluated | `Safety`; no partial set |
| `edge_wake_during_burst_is_admitted_after_burst` | same name | edge wake during a pass burst, per-target exclusion, handoffs | `OneActivePerTarget`, `ActiveWithinCap`, `Safety` |
| `pass_submit_merges_retained_not_before_to_immediate` | same name | V-4 merge | `EligCanonical` (observed through admission) |
| `world_changes_between_drain_and_finish` | same name | world read at `Finish` (D-12 d) | `Safety` |
| `conformance_wake_conf` (`#[quint_run]`) | randomized | all of the above under random interleavings | all; seed printed on failure |

## Action to driving port

| Action | Production call |
|---|---|
| `init` | compose the world (`WakeWorld::compose`) |
| `Tick` | none: the period timer is not a driving port (D-12 c); handler lets the runtime quiesce, comparison checks the firing changed no broker state |
| `PassStep({enumOk})` | `resync_reconciler(state, name)` for the next reconciler in pass order; `enumOk = false` arms a sticky read fault at `SimObservationStore` (allocation rows for the three workload-keyed reconcilers, service rows for the hydrator), disarmed after |
| `Drain` | `BrokerDrain::drain_pending(cap - in_flight, in_flight_targets, now, now_unix)`; the admitted evaluations are the driver's in-flight set (stimulus, D-12 d) |
| `Finish({k, o})` | `run_convergence_tick`; fault first: `dpFail` `SimDataplane::script_update_service_failure`; `hydrateErr` sticky read fault; `persistErr` `FaultableViewStore` write failure; `dispatchErr` one-shot `SimObservationStore` write failure: `inject_alloc_lifecycle_write_failure` for `k1`, and for `k3` while the listener has no VIP (it then writes no service row: its write is the Stable commit); `inject_write_failure` for `k2`, for `k3` with a VIP (the service-row writers) and for `k4` (D10: the `IssueSvid` dispatch audits through the generic `ObservationStore::write`, `action_shim/issue_svid.rs:122` via `ca_issuance::issue_and_audit`, so a failed issue leaves the SVID unheld and `k4` inside its own backoff window); faults disarmed after the tick. The handoffs (`k1` to `k3`, `k4`; `k3` to `k2` only with a VIP) are submitted by the production action shim inside the tick |
| `DivergeWake({k})` | world change, then `ReconcilerRuntime::submit_evaluation(.., Immediate)` |
| `DivergeDrop({k})` | world change only (a dropped edge wake) |
| `EdgeWake({k})` | `ReconcilerRuntime::submit_evaluation(.., Immediate)` |
| `WindowEnds({k})` | `SimClock::tick(RESTART_BACKOFF_DURATION)`: one global advance, whichever `k` the step names. `backoff_for_attempt` ignores the attempt (`RESTART_BACKOFF_DURATION`, 1 s); the hydrator's window end (`service_map_hydrator.rs` `backoff_window_elapsed`, `now >= last_failure_seen_at + backoff`) and svid-lifecycle's (`svid_lifecycle.rs:427`, `now < issue_retry_deadline`, `last_failure_seen_at + backoff`) are both anchored at the clock reading of the failing tick, so one advance of the constant passes both deadlines and makes every retained `NotBefore` eligible (the model's `inWin' = Set()`) |
| `Crash` | rebuild `ReconcilerRuntime` over the same View store (`register` runs `bulk_load`, all four reconcilers re-registered; the rebuilt broker has zero counters, matching the spec's `Crash`); in-flight set and pass cursor reset. Only the runtime is replaced: `AppState::identity` (the same `IdentityMgr`, so the held SVIDs survive, as the model's `held` assumes), the stores and the world survive |

## World construction (stimulus, never projected)

The managed-set rule (user ruling): a Service is managed by `service-lifecycle` only once it has an allocation or an assigned VIP, never on
intent alone. `workload-lifecycle` ignores `replicas` (`workload_lifecycle.rs:972-985`), so no second replica is used.

| Key | `need` | World change | Coupling the model carries |
|---|---|---|---|
| `k1` | no Running allocation | persist the `orders` Service intent (no VIP, no allocation: `service-lifecycle` and `svid-lifecycle` manage nothing); a Running allocation is replaced by a `Failed` row with a long-ago failure seeded (immediate restart). The allocation is no longer Running, so `svid-lifecycle`'s retry deadline disappears: a `k1` world change clears `k4`'s window (D11) | `need(k3)` when a VIP exists; `need(k4)` when an SVID is held (a `DropSvid` for an allocation no longer Running) |
| `k3` | `service-lifecycle` has a VIP and no observed `ServiceBackendRow` matching the projection | assign the `orders` listener's VIP and record its listener fact (never the intent alone), persist the intent (which brings `k1` into `has`); an already published row is replaced by a dominating over-broad row | |
| `k4` | a Running allocation with no SVID held | persist the intent, add a fresh Running allocation row (so a converged `k4` can need work again). A failed issue (`dispatchErr`, D10) leaves the allocation unheld with an `IssueRetry`: `k4` is inside its own window and an in-window `Finish(k4, ok)` is a Noop requeued with `NotBefore` (D11) | `has(k3)`, `need(k3)` (an allocation row of the Service makes `service-lifecycle` manage it); clears `need(k1)` |
| `k2` | desired differs from the result row | persist the intent, assign the `orders` listener's VIP and record its fact, write a `ServiceBackendRow` with a new backend set | `need(k3)` (the observed row differs from the projection) |

The driver tracks `vip_assigned` (stimulus, never projected) to choose the `dispatchErr` seam for `k3`.

Timestamps of precondition rows dominate the row they replace (`LogicalTimestamp::dominating`). The fixture methods carry `#[cfg(test)]`,
which is how dst-lint treats the row literals (as `convergence_property/` does); `cargo xtask dst-lint` passes.

## Projection to accessor

| Compared field | Spec variable | Accessor |
|---|---|---|
| `pending` | `pending` (FIFO) | `BrokerDrain::pending_keys()` (D-12 b), mapped through the key table |
| `dispatched`, `cancelled` | same | `BrokerDrain::counters()` |

Not projected: `has`, `need`, `inWin`, `elig`, `active`, `due`, `passPos`, `faults`, `drifts`, `conf_step` (model-only or stimulus). `from_driver`
reads only those two accessors; no `Sim*` adapter, driver map or literal.

## Pending status and semantic RED

The accessor and the port exist as inert scaffolds (`pending_keys` returns an empty list; `resync_reconciler` submits nothing;
`managed_targets` returns the empty set for all four reconcilers, and `ServiceMapHydrator::next_evaluation_at` is the trait default).
Each of the 11 tests (10 named runs plus the randomized one) keeps the full executable replay and carries
`#[should_panic(expected = "the pending queue the broker reports is empty where the model's is not")]`.
quint-connect's own text for a state mismatch is the generic `State invariant failed` (it prints the diverging fields only at trace
level), which any counter divergence, driver fault or world fault would also produce, so `WakeState` implements `PartialEq`
itself: equal states compare equal; counters equal with the broker's `pending` empty and the model's not (the signature of the inert
scaffold) panics with the text above; any other difference is an ordinary mismatch, which does not match the marker and fails the test.
All 11 pass on the test machine against the scaffolds, so each is RED for that one reason. Because the first step whose model `pending` is non-empty diverges from the scaffold's empty
`pending`, the `Drain`, `Finish`, `Crash`, `WindowEnds` and fault handlers are unexercised until DELIVER; the fidelity probe below exercised them.

## Fidelity probe (temporary real bodies, reverted)

The spec was replayed once against real production behaviour so the driver's world construction and the spec's abstraction are checked before DELIVER. Method: temporary real bodies for `BrokerDrain::pending_keys`, `resync_reconciler`, the four `managed_targets` and `ServiceMapHydrator::next_evaluation_at` (D-3), with the pending markers removed, on the test machine with `QUINT_VERBOSE=1`. Result: replayed fully, all 10 named runs and `#[quint_run]` on the default seed and `QUINT_SEED` 1 to 20 (100 traces each); the divergences found (D9, D10, D11) were fixed in the spec or the driver and no divergence remains. The temporary bodies were reverted and the tree verified byte-identical.

The DELIVER bodies must be the ones the replay passes with: the managed-set rule (D7) and `ServiceMapHydrator::next_evaluation_at` per D-3 (`last_failure_seen_at + backoff_for_attempt(attempts)` while in the future). The probe's `managed_targets` (D-1a allows a target with nothing to do): `workload-lifecycle` = workload intents plus allocation rows; `service-lifecycle` = Service intents with an allocation row or an assigned VIP (`ServiceVipView::assigned_vip`), plus Service-kind allocation rows; `svid-lifecycle` = workloads with an allocation row; `service-map-hydrator` = `all_service_backends_rows`. A different choice (for example a superset of targets) shows as an `unmapped:` or surplus `pending`. The random seeds are a sample, not a proof, and `dispatchErr` for `k1` is approximate (an allocation-row write failure after the start).

## Per-driver checklist (`testing.md` § "Quint conformance")

1. Placement/gate: crate `src/`, module declared under `#[cfg(all(test, feature = "integration-tests"))]` (`reconciler_runtime.rs`, beside `quint_liveness_gate`); no always-false `cfg`; the file has none (its `#[cfg(test)]` items are fixtures, for dst-lint). Pending markers are `#[should_panic(expected = "the pending queue the broker reports is empty where the model's is not")]`. Yes.
2. Driver holds the production owner: `EvaluationWakeDriver.world: WakeWorld` holds `AppState` whose `runtime: Arc<ReconcilerRuntime>` is the owner, with all four key reconcilers registered. Yes.
3. Every `step` action mapped; every handler calls an owner port or mutates the world through a production driven port: `switch!` in `Driver::step`; `Tick` is the one quiescence-only handler (pinned model-only, D-12 c), see the table; `dispatchErr` is armed at the production write seam each key's dispatch reaches (`k4`: the `IssueSvid` audit write); the handoffs are the production action shim's, not the driver's; no `_` arm, no `todo!`, no empty body. Yes, with that stated exception.
4. `State` fields: `pending` <- `pending_keys`, `dispatched`/`cancelled` <- `counters` (`State::from_driver`). Yes.
5. `from_driver` reads no `Sim*`, no driver map, no literal (the key table is a fixed mapping of the owner's keys). Yes.
6. Spec referenced in place; 10 `#[quint_test]` (all `[[check]]` runs of `checks.toml` on `wake_conf`) plus `#[quint_run]`. Yes.
7. `evaluation_wake` is a state machine (`step`); no `quint` CLI is spawned by the driver. Yes.

## Gaps and blockers (surfaced, nothing invented)

* **Resolved**: D1 to D8 (spec revision), D9 and D11 (spec revision), D10 (driver). All 10 named runs and the randomized run (default seed and `QUINT_SEED` 1 to 20) replay fully against production with temporary real bodies.
* **D-3 and `managed_targets` are scaffolds** in the tree; the probe used temporary real bodies. The DELIVER bodies must be the ones the replay passes with: the managed-set rule (D7) and `ServiceMapHydrator::next_evaluation_at` as in "Fidelity probe".
* The driver depends on sim seams another dispatch added (`script_alloc_status_rows_read_failure`, `script_service_rows_read_failure`, `script_update_service_failure`).
* Pre-existing: running `cargo xtask placement-lint` / `conformance-lint` on the macOS host fails to build; both pass through `cargo xtask metal run --`.
