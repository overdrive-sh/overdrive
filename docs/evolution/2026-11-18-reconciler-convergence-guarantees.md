# Evolution — reconciler-convergence-guarantees (GH #351)

**Finalized:** 2026-11-18. **Feature slug:** `reconciler-convergence-guarantees`.
**Waves run:** DISCUSS → DESIGN → DISTILL → DELIVER → FINALIZE (no DIVERGE; recorded as a risk at DISCUSS).
**Decision records:** [ADR-0321](../product/architecture/adr-0321-every-reconciler-declares-its-managed-targets-and-the-runtime-re-evaluates-them-each-period.md) (managed targets and the resync pass), [ADR-0322](../product/architecture/adr-0322-the-same-host-local-backend-path-is-removed.md) (the same-host local-backend path is removed), [ADR-0323](../product/architecture/adr-0323-guest-udp-to-a-service-vip-is-refused-until-gh-310.md) (guest UDP to a VIP is refused until GH #310), [ADR-0324](../product/architecture/adr-0324-the-service-map-hydrator-reads-the-programmed-service-map.md) (the hydrator reads the programmed service map).
**Feature history:** [`docs/feature/reconciler-convergence-guarantees/`](../feature/reconciler-convergence-guarantees/) (feature delta, slices, spike, distill, deliver). **Quint model:** [`specs/quint/reconciler-evaluation-wake/`](../../specs/quint/reconciler-evaluation-wake/).

Status of the ADRs at finalize: all four still read `Proposed` in their Status sections, and the feature delta records that independent design review had not run when DESIGN closed. The user approved each decision in conversation (dates below); the ADR status lines were not updated. Also, ADR numbers 0296 to 0299 are each held by two files in `docs/product/architecture/` (the other four belong to the native-benchmark-program feature). Neither is fixed here.

---

## 1. Summary

Reconciler convergence became a runtime guarantee rather than a property each author must get right.

- Every reconciler now declares `managed_targets`, enumerated from intent and observation. A boot-anchored resync pass (30 s, a compile-time constant) submits an `Immediate` evaluation for every managed target of every registered reconciler. It replaces the per-reconciler `resync_schedule` cadence and the interest router's periodic relist.
- The one latch the audit proved reachable, `service-map-hydrator.last_applied_local_fingerprint`, was removed by deleting the whole same-host local-backend path, which had no production producer.
- The hydrator now decides whether its effect is in place from the resource it manages: a 64-bit fingerprint of the applied backend set, stored in the programmed `SERVICE_MAP` and read back through a new narrow port, `ProgrammedServiceView`. Its own `service_hydration_results` row is a receipt, no longer an input to `reconcile`.
- A shared multi-tick convergence property (`crates/overdrive-control-plane/src/convergence_property/`) covers every registered reconciler or records a reasoned no-obligation ruling.
- `.claude/rules/reconcilers.md` now carries the general View rule: a View decides when to act, never whether an effect is in place.

## 2. Problem (GH #351)

An audit found that about half of the reconcilers missed at least one of three duties: declare a wake that fires again, never let the View claim an effect happened, and emit only actions whose results hydrate can read back. Nothing mechanical caught it. The grounded verification (`.context/convergence-verification.md`, HEAD `fb71b400f`) corrected the issue before DISCUSS and supersedes it where they differ:

- the service-lifecycle `stable_announced` / `terminal_announced` latch claim is refuted (D-6);
- `workflow-lifecycle` being un-woken is not a production defect (empty `WorkflowRegistry`, D6, D-8);
- `RunningAllocationWithoutWorkloadAddr` is not production-reachable (a Tier 1 stimulus only);
- the hydrator's local-backend latch is unreachable in production but was the one latch DISCUSS thought was reachable (D-9).

What stayed real: a hydrate error and a View-write error returned early and never requeued; the interest router relisted only targets that already had an `alloc_status` row; a `Failed` dataplane result inside backoff left the hydrator un-woken; and no test generated an arbitrary starting `(actual, view)`, ran several ticks, or dropped actions.

## 3. Key decisions

All user approvals are recorded in the feature delta's "User approvals" table. Dates are 2026-11.

| ID | Date | Decision |
|---|---|---|
| D1 to D15 (DISCUSS) | 11-13 to 11-15 | Scope P1, P2, P3 plus the rule change. `observed_generation` stays in #325 (D2). Boot-only host infra stays in #197/#198/#199/#234 (D3). P1 enumerates from intent and observation (D4). The View may decide when, never whether (D9). One feature, three slices (D12). The new rule names no exception (D13). |
| D-1, D-1a, D-1b, D-1c | 11-13 | ADR-0321. Required `Reconciler::managed_targets`; crate-private `resync_reconciler`; the pass loops it in `BTreeMap` order; a failed enumeration logs, submits nothing for that reconciler and the pass continues. `resync_schedule`, `build_cadence_table`, `INTEREST_ROUTER_RELIST_PERIOD` and the router's periodic arm are removed. |
| D-2 | 11-13 | The 30 s period stays a constant; configurability tracked by #356. |
| D-3 | 11-13, kept 11-15 | The hydrator's `next_evaluation_at` retry deadline stays (the user rejected removing it). |
| D-5 | 11-13 | The VIP-release story moved to #211 (no production path deletes a workload spec). The `release_service_vip` executor doc fix stays in this feature. |
| D-6 | 11-13 | No change to `service-lifecycle`'s probe-driven stop markers (ADR-0191 decision 5); they are pacing keyed to an observed input. The probe-driven stop obligation is scoped out of the drop/skew progress oracle. |
| D-7, D-8 | 11-13, target 11-15 | The property drives the production tick over sim-composed ports; no new sim executor. `workflow-lifecycle` gets no dedicated wake; its managed target is `workflow/all`, returned only when an instance exists. |
| D-9 | 11-15 | Delete the ADR-0053 same-host local-backend path (ADR-0322): `cgroup_connect4_service`, `cgroup_sendmsg4_service`, `cgroup_recvmsg4_service`, `LOCAL_BACKEND_MAP`, `REVERSE_LOCAL_MAP`, the `Dataplane` methods, the actions and executors, the hydrator's local partition, and the owner's cgroup-placement check. Reason: with the map always empty the hook was a pass-through; #310 designs its own destination selection. |
| D-10, D-11 | 11-15 | ADR-0164's datagram clause is superseded; guest UDP to a VIP is refused with the existing `FlowRefusal::Policy` until #310 (ADR-0323). TCP-as-mesh (D24a) stays. |
| D-12 | 11-15 | Conformance pins: `resync_reconciler` as a crate-private driving port, an ordering clause on `BrokerDrain::pending_keys`. |
| D-13 | 11-15 | `ConflictRoute::Cgroup` removed in place; the enum stays one-variant, no new envelope version, the schema-evolution fixture untouched. |
| D-14 to D-19 | 11-16 | DISTILL rulings. The Quint spec models production-reachable pairs only (D-14). Inert scaffolds of exactly the pinned surface (D-15). The property lives in `src/convergence_property/` as a `#[cfg(test)]` module (D-16). The spec models the production handoff wakes (D-17). `service-lifecycle`'s managed set is a Service with an allocation row or an assigned VIP (D-18). Validation budget growth accepted, no time cap; `ProgressByPass` and `RetentionSchedulesBackedOffKey` are `ci = false` (D-19). |
| D-20 | 11-17 | The hydrator's `actual` is the programmed service map, read through `ProgrammedServiceView` (ADR-0324). The read is a 64-bit fingerprint stored in two extra slots of each inner `SERVICE_MAP` array, written before the atomic outer swap. Inner capacity becomes 16,383; XDP still indexes modulo 16,381. Rejected, with V-6 measurements: the full-table batch scan (5.41 ms p50 per service) and mmap-able inner maps (107 µs p50 plus `BACKEND_MAP` lookups). Also rejected: invalidating result rows at boot, which fixes a reboot only. |
| D-21 | 11-17 | The hydrator's managed set covers a Service with a workload intent or a `service_backends` row. An orphaned `service_hydration_results` row is the deletion case and belongs to #211. No list-all method is added to `ObservationStore`. |
| D-22 | 11-17 | The Quint managed-set model follows the approved managed sets (D-1a, D-18, D-21); production and the conformance driver are unchanged. Adds `VipImpliesHydratorManaged` and `K4ManagedIffRunningOrHeld`; the `svid-lifecycle` key can leave the managed set. |
| Dropped | 11-15, 11-16 | The model check `PendingIsAdmitted` (11-15). The Tier-3 `persist_view` scenario (11-16): redb 2.6.3 poisons a `Database` after a failed commit and `RedbViewStore` never reopens, so the oracle cannot hold; tracked by #360. |

## 4. Work completed

Source: `deliver/execution-log.json` (schema 3.0) and `git log f864e46fb..HEAD`. The roadmap's phases are in DELIVER order: the deletion first, then D-20, then the resync pass, then the close-out. Every step reached COMMIT with status `EXECUTED` / `PASS`.

| Step | What landed | Commit |
|---|---|---|
| 01-01 | Guest-flow owner refuses UDP to a VIP; deleted the cgroup-placement check and the connect4 datagram handling | `a065c5c7d` |
| 01-02 | Deleted the local-backend actions, executors, hydrator partition and the persisted `Cgroup` route tag | `cd33d72ac` |
| 01-03 | Deleted the local-backend `Dataplane` methods, both adapters, three BPF programs, two maps and their baselines | `cfcc67612` |
| 02-01 | `SimDataplane` implements `programmed_fingerprint` and refuses IPv6 backend sets before any write | `991761bbd` |
| 02-02 | `DataplanePorts`: one dataplane instance behind both the write port and the read port | `db0e5085f` |
| 02-03 | `EbpfDataplane` writes the applied-set fingerprint with the table and reads it back; inner capacity 16,383 | `b229f69ad` |
| 02-04 | Hydrator diffs against the programmed fingerprint; result-row arms deleted; D-3 retention added | `31e5044e1` |
| 03-01 | Ruled per-reconciler `managed_targets` enumeration replaces the scaffolds | `07e2624f2` |
| 03-02 | `resync_reconciler`, the boot-anchored pass loop and the D-1c removals | `4ebc41233` |
| 03-03 | `BrokerDrain::pending_keys` real body; quint-connect driver replays the spec against the production broker | `3c90c1a61` (spec: `288f857c0`; broker test: `545e9e0f9`) |
| 04-01 | `reconcilers.md`: a View decides when to act, never whether an effect is in place | `24989f544` |
| 04-02 | Corrected the `release_service_vip` executor doc and the false hydrator DST claim | `e946bbdfb` |

Execution-log exceptions: 01-03 was first logged SKIPPED (`BLOCKED_BY_DEPENDENCY`, metal box disk full, rsync ENOSPC) and re-run the same evening. 02-02 and 04-02 RED are `NOT_APPLICABLE` (behaviour-preserving composition change; comment-only change). 03-03 GREEN and COMMIT were blocked on 2026-11-17 because 3 of 11 conformance replays diverged from the model, and completed on 2026-11-18 after the D-22 spec correction. Two unrelated tooling commits landed in the range: `a50ff5934` (name the lease holder on `checkout_timeout`; block metal-run misuse) and `9ec61b56d` (parse the metal-run misuse check with shell-quote).

The DISTILL wave authored the convergence property, the conformance driver and the inert scaffolds before these steps. Its decisions are in [`distill/wave-decisions.md`](../feature/reconciler-convergence-guarantees/distill/wave-decisions.md) and [`distill/test-scenarios.md`](../feature/reconciler-convergence-guarantees/distill/test-scenarios.md).

## 5. Post-delivery review follow-ups

The per-slice reviews ran retroactively after all twelve steps were committed (see lessons). They produced these commits:

- `0f3a8e4bc` removed stale scaffold and deleted-path references left in comments and docs.
- `70946eedb` derived the sim's programmed fingerprint from the stored backend set and shared the hydrator's placement computation between production and sim.
- `17ebe5449` tightened the spec invariant `SharedIntentImpliesBase` to excuse only the orphan SVID (the held SVID with no allocation row) instead of a broader exemption.
- `84afc0afb` deleted the unreachable same-slot conflict row.
- `db8f74b0b` deleted `ServiceHydrationStatus::Pending`, which no writer produces. This changes the archived layout of the row, so `FIXTURE_V1` changed by one byte. Upgrade note: Phase 1 is greenfield single-cut, so the upgrade path is deleting the node's redb file.
- `2d7d400bf` collapsed the single reclaim-output violation into one struct and corrected stale row-kind counts.
- `6f1477228` rustfmt on the hydrator reconcile acceptance test and a trailing comma clippy rejects.
- `514783391` deleted the `service_hydration_results` observation row: its only writer was the dataplane-update shim, nothing read it, and a failed update is now logged as `dataplane.update_service.failed`.

## 6. Lessons learned

1. **Reviews were not run after each slice.** The per-slice adversarial and subtractive reviews that `feedback_review_after_every_slice` requires did not run as the steps landed. They ran retroactively over the finished range, so their findings (section 5) landed as seven follow-up commits rather than inside the steps that caused them.
2. **The slice 02 deletion inventory missed two dependents.** (a) The conflict-row test was deleted by mistake although its subject was route-agnostic; the unreachable same-slot conflict row went later in `84afc0afb`. (b) The D-9 deletion inventory did not consider other features' dependents: the connect-hook benchmark in `overdrive-bench-families` measured the connect4 hook this feature deleted. The native-benchmark-program feature later redefined that family (approved change CA-13, commit `05a77066b`) to measure the gateway connect4 program against no hook, and `smoke_connect_hook_cost` passes at HEAD once the metal lane is prepared (run `57295865f7b993f4f44e1b2e`).
3. **RED was not observed for 02-04.** The execution log records RED and GREEN with the same timestamp (`19:52:29Z`). The log shows the same identical-timestamp pattern for 01-01 and 03-02; I did not check whether RED was observed for those.
4. **Gates not run or unusable.** `verifier-regress` was not run (it is a Lima-only alias, and the project ran everything on metal). `bpf-clippy` and `bpf-unit` were unusable on metal.
5. **The Quint replay needed four spec corrections** before the model matched production: the managed-set coupling between keys, the partiality of dispatch, the hydrator's need being recomputed rather than latched, and the backoff window ending on publish. Replaying the spec against the real broker found modelling errors that checking alone had not; the checked model had been wrong in ways the checker could not see. Cost: `ProgressByPass` model-check time grew from about 80 to about 163 minutes (`evidence/summary.md`: 9,777.6 s), accepted by D-19. The evidence run is recorded against `70946eedb` with a dirty worktree; `17ebe5449` (the spec tightening) is newer. I did not re-run `cargo xtask quint verify-evidence` for this record.
6. **Ground the premise paid off twice.** Tracing production showed the local-backend latch unreachable (so delete, not fix, D-9) and the VIP-release story unreachable (no deletion path, so it moved to #211). The #248 precedent in `development.md` is the same lesson.
7. **A receipt is not the resource.** The hydrator's result row survived reboots that wiped the kernel maps; the fix (D-20) reads the programmed map itself.

## 7. Issues encountered

- Metal box disk full during 01-03 (rsync ENOSPC); the step re-ran after cleanup.
- 3 of 11 conformance replays diverged in 03-03 (surplus k2, missing k4); resolved by D-22.
- redb 2.6.3 poisoning after a failed commit made the Tier-3 `persist_view` scenario unholdable; dropped and tracked by #360.

## 8. Known residuals

- GH #283 spike test failure: pre-existing, out of scope.
- `overdrive-bench-families` dst-lint violations: pre-existing, out of scope.
- The `interest_row_kind` table covers 8 of 10 row kinds.
- The ADRs read `Proposed` and the ADR number collision with the benchmark feature stands (see the header).
- Limitations tracked elsewhere: #356 (period not configurable), #357 (edge-wake deploy latency during a pass), #270 (scan cost), #360 (redb poisoning), #310 (guest UDP to a VIP), #211 (VIP release retry and teardown), #325 (`observed_generation`), #358 (first VIP equals the host `ovd-veth-bk` address).

## 9. Verification expectations (EDD)

No EDD expectations were authored for this feature. `verification/expectations/` holds nothing for it. Its claims are internal (re-evaluation, View rule, deletion) and are covered by the test tiers and the Quint conformance replay.

## 10. Migrated permanent artifacts

None were copied (see Phase B). Lasting artifacts already sit at permanent homes: the four ADRs under `docs/product/architecture/`, the Quint subsystem at `specs/quint/reconciler-evaluation-wake/` (README, `checks.toml`, hazards, recorded evidence), the View rule in `.claude/rules/reconcilers.md`, and the corrected `development.md` § "Reconciler I/O". Slices: [01](../feature/reconciler-convergence-guarantees/slices/slice-01-periodic-reevaluation.md), [02](../feature/reconciler-convergence-guarantees/slices/slice-02-delete-same-host-local-backend-path.md), [04](../feature/reconciler-convergence-guarantees/slices/slice-04-convergence-property-and-view-rule.md). Spike: [findings](../feature/reconciler-convergence-guarantees/spike/findings-programmed-service-map-read.md).
