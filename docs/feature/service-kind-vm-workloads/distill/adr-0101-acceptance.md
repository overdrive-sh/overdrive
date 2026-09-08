# ADR-0101 acceptance design and completeness

**Owner:** Codex, acceptance-designer role  
**Date:** 2026-09-08  
**Contract:** ADR-0101 revision 3; independent DESIGN review iteration 3 APPROVED  
**Disposition:** executable behavioral RED plus separately green controls; no implementation, DES events, or commit

## Grounded premise and boundary

Read before authorship: the accepted ADR, backend-eligibility convergence ruling,
architecture brief's affected Service section, feature delta, wave decisions,
`docs/analysis/e09-v2-failed-service-reachability.md`, and
`docs/research/backend-eligibility-convergence.md`. The original
`e09_v2_failed_service_reachability_spike.rs` was freshly executed unchanged.
Seed **257209** reached production startup Failed, a real WorkloadLifecycle
same-ID restart, retained `terminal_announced`, and a stored healthy backend
after six ServiceLifecycle ticks. Its final invariant failed for that reason.
No disputed row or policy memory was supplied by the test.

The accepted correction changes the complete-row author, not the failure
meaning: ServiceLifecycle projects current Running membership plus its existing
eligibility policy. A retained terminal veto forbids newly eligible publication.
Successful authoritative withdrawal precedes independently scheduled consumer
convergence. A failed withdrawal write may coexist temporarily with a reported
failure; neither report nor write is an all-consumer acknowledgement. Existing
connections, historical-listener garbage collection, and new recovery protocols
are excluded. This is greenfield: no migration, compatibility, or upgrade case
is introduced.

## Executable composition

`crates/overdrive-sim/tests/integration/service_backend_projection.rs`, wired
through the existing `tests/integration.rs`, uses the smallest current
driver-independent composition containing the relevant owners:

- Real validated Service intent/archive/allocator input ports, LocalIntentStore,
  and listener facts rebuilt by the production intent projection.
- Registered WorkloadLifecycle, ServiceLifecycle, ServiceMapHydrator, and
  VmReclamation; real ReconcilerRuntime, redb ViewStore, and serial action shim.
- SimDriver only for the process port, with every production ProbeRunner
  lifecycle hook forwarded, including Startup-role cancellation at Stable.
- Production ProbeRunner, SimClock, queue-driven TCP/HTTP probers, and
  SimObservationStore. Probe outcomes are fault inputs; probe rows are authored
  by ProbeRunner. Allocation and terminal rows are authored by action dispatch.
- Real ServiceBackendsResolve and DNS NameIndex List/Watch consumers, and
  ServiceMapHydrator's existing queued evaluation to SimDataplane's supported
  local Exec map. The test does not claim that VM mesh traffic uses this map.

No bridge is registered in the new fixture: it targets the approved sole-author
architecture using only APIs that already exist. The original bridge-enabled
diagnostic is retained separately as before-change reachability evidence.
The action-order complement obtains facts through the existing production
hydrate wrappers and the real persisted View; it does not fabricate a terminal
state for the pure reconciler. It subsequently drives serial dispatch too.

The fixture retains the VIP returned by the allocator independently of observed
rows. Every backend-row event and observation read is checked against that VIP,
including empty and unhealthy withdrawal rows. Planned writes use the same
oracle; BE-07's expected correlation fingerprint and BE-10's expected local-map
key also use the retained allocator assignment, never the published row's VIP.

## Scenarios

All Sim functions carry `CONTRACT_SHAPE: bounded-change.`. Names below are
the exact Rust functions in `service_backend_projection.rs`.

| ID | Given / When / Then | Executable function |
|---|---|---|
| BE-01 | Given one Running allocation without readiness and no startup decision; when ServiceLifecycle repeats; then eligibility is true before Stable and the complete row, including stamp, is unchanged. | `single_listener_preterminal_control_is_healthy_and_stable` |
| BE-02 | Given 1/3 listeners, including same-port TCP/UDP, and one production-scheduled Running allocation; when projection repeats; then exact ServiceId sets, allocator-issued VIP, backend SPIFFE identity, listener ports, host fallback, weight one and healthy values are preserved without stamp churn. Multi-allocation membership/order coverage is deferred to [#282](https://github.com/overdrive-sh/overdrive/issues/282#issuecomment-5584665927). | `complete_listener_projection_is_idempotent` |
| BE-03 | Given no allocation yet; when a current listener has/does not have its allocator assignment; then the former has an explicit empty row and the latter has no row; repeating is idempotent. | `empty_membership_and_absent_dataplane_are_distinct` |
| BE-04 | Given Running before startup decision; when the first backend publication receives a typed observation-write failure; then no row exists and later observed-state reconciliation repairs it without a probe transition. | `rejected_first_publication_is_repaired_from_observed_state` |
| BE-05 | Given a healthy baseline and real startup probe refusals; when ProbeRunner exhausts three attempts, then the Failed allocation is absent from membership; when WorkloadLifecycle restarts the same identity, then every subsequent projected backend remains ineligible under the unchanged terminal veto. | `startup_failure_withdraws_then_same_id_restart_remains_ineligible` |
| BE-06 | Given a healthy baseline and the third real startup failure; when the deciding withdrawal write fails, then the exact write error is returned but FinalizeFailed still drains; when normal redb registration reloads the View and the same identity restarts, then the retained veto repairs the still-healthy observed row. | `failed_withdrawal_drains_terminal_and_repairs_after_view_reload` |
| BE-07 | Given two current listeners and real deciding startup failure; when actions are computed from production hydration before/after normal runtime registration and process-local tick reset, then each stamp exactly equals dominating(current tick, owner writer, observed same-key prior), including absent, tick-ahead and prior-ahead cases; ServiceId-ordered write/handoff pairs with allocator-VIP/content-derived correlations precede FinalizeFailed, and serial dispatch publishes withdrawals before Failed. | `deciding_tick_orders_complete_row_handoffs_before_failure` |
| BE-08 | Given readiness threshold three and 1/3 listeners; when the same real Pass observation is reconciled three times, then readiness advances once per allocation/tick; a real Fail resets it and subsequent Pass observations recover without a terminal allocation decision. | `readiness_threshold_is_listener_independent_and_recovers` |
| BE-09 | Given healthy List/Watch consumers; when ProbeRunner produces Fail then Pass, then the resolver and DNS projection independently converge to unavailable then available after bounded task progress. | `mesh_and_dns_watch_control_follows_real_readiness_outcomes` |
| BE-10 | Given BE-09 plus the supported local map consumer; when publication queues the hydrator, then its existing evaluation installs/removes/restores the local backend; no synchronous consumer assertion is made at publication return. | `existing_consumers_follow_withdrawal_and_recovery_asynchronously` |
| BE-11 | Given a real Start then an operator stop marker through the intent port; when WorkloadLifecycle runs, then both transitions retain SVID wakes and wake ServiceLifecycle; the stopped allocation remains stopped and complete membership becomes empty. | `workload_start_and_stop_wake_the_service_projection_owner` |
| BE-12 | Given a healthy Service and actual liveness probe failures; when ServiceLifecycle stops and WorkloadLifecycle consumes its normal five-restart budget, then same-ID Restart and the final typed liveness failure retain the SVID wake and wake ServiceLifecycle; final membership is empty. | `liveness_restart_budget_and_finalization_keep_projection_handoffs` |
| BE-P1 | Given generated counters/thresholds including zero, one and u32 saturation edges; when every readiness-enabled × veto × absent/Pass/Fail combination executes the pure policy function, then exact health and complete View equality match the policy, including untouched unrelated allocation inputs. | `service_lifecycle::tests::backend_policy_preserves_veto_threshold_and_unrelated_view_inputs` |

BE-P1 is source-local in `overdrive-reconcilers/src/service_lifecycle.rs`, inside
`#[cfg(test)]` only, with the exact `/// CONTRACT_SHAPE: pure-function.` line.
It uses fixed proptest seed 257222 and 128 generated cases, each traversing all
12 finite policy combinations. It is a policy complement, not reachability
evidence for the constructed pure input. The original diagnostic and composed
scenarios prove reachability separately.

### BE-02 bounded correction — 2026-09-08

User authorization: “Yes, apply the bounded BE02 correction.” Both cases and
seeds remain: 257210 has one TCP listener; 257211 has 18082/UDP, 18081/TCP and
18081/UDP. Only the latter's requested allocation count changes from two to
one. WorkloadLifecycle genuinely schedules that allocation. Every existing
listener/VIP/backend/content and repeated-reconcile equality assertion remains.

The former two-replica fixture exceeded current normal WorkloadLifecycle
convergence: any Running allocation prevents further placement. It did not
prove that ServiceLifecycle lost an existing second allocation. Historical
failures remain recorded in `red-classification.md` and the DELIVER transfer
handoff. Restoring genuinely scheduled two-allocation membership/order coverage
is explicitly tracked in [#282](https://github.com/overdrive-sh/overdrive/issues/282)
and its [BE02 follow-up](https://github.com/overdrive-sh/overdrive/issues/282#issuecomment-5584665927).
Singleton identity equality is not multi-allocation ordering evidence.

Focused nextest run `c46116bd-0a5d-402d-ae26-e2941f6e60ec` passed the one Rust
test with both printed seeds, including all formerly blocked listener/identity
and three-repeat exact-row/no-stamp-churn suffixes; 35 tests skipped, exit 0.
This is execution evidence, not independent approval of the revised material.

### BE-10 approved revision-4 implementation handoff — 2026-09-08

The BE10 scenario and composed test are unchanged. Independently approved
ADR-0101 D7 now authorizes the existing local register/deregister action choice.
Fresh seed257221 RED, the exact paired pure-property complement, retained port
controls, source-local test-only rename patch and unexecuted recovery/native
boundaries are recorded in the
[bounded DISTILL handoff](be10-local-withdrawal-test-handoff.md).
This does not claim implementation GREEN or new consumer retry guarantees.

## Seed, progress, safety and cleanup oracles

Each composition prints its seed. Seeds 257209–257227 select the observation
adapter seed and bounded task-progress/clock increments; named scenarios fix
owner order deliberately. This is a reproducible bounded schedule family,
not an exhaustive scheduler exploration. No arbitrary Tokio abort, terminal
row seed, cache seed, persisted fingerprint injection, or new Sim seam exists.

Progress is bounded: 3 repair ticks; 4 startup probe advances; 6 post-restart
projection ticks; 12 maximum liveness owner cycles; each task-settle window is
64–80 cooperative yields. No wall-clock sleep is used. SimClock's initial wall
epoch is not compared byte-for-byte across runs. Exact domain outcomes and
allocation-relative ordering are the replay oracles.

Publication history is read from the existing lag-aware observation stream;
oracle lag fails loudly, not silently. BE-06 checks newly published backend
health after the retained veto; BE-07 checks successful write order before the
Failed observation. BE-07 first checks the production call-site stamps for a
missing row, a tick ahead of an observed row, and an observed row ahead of a
reset tick. The existing convergence loop initializes `tick_n` to zero
(`overdrive-control-plane/src/lib.rs:3353`) and passes it through the existing
`run_convergence_tick` argument (`:3383`). The test uses normal runtime/View
registration, retains production-authored observation unchanged, and resets
only this existing scheduling coordinate. It does not simulate an upgrade or
claim to restart a whole process, nor invent persistence or recovery behavior.
The expected stamp uses the AppState node writer and independently read prior
for the same ServiceId, not the action's own writer/counter. Nonempty action
checks and explicit tick/prior inequalities prevent vacuous branch coverage.
Exact row vectors/stamps, allocator VIP, exact listener/identity sets,
typed failure, unchanged reloaded View, DNS lookup, new-connection resolver
classification, and local map presence are independent assertion boundaries.
Probe supervisors are cancelled at fixture drop and consumer adapters are
dropped explicitly; unique TempDirs own the redb fixtures. No external process,
production binary, expectation runner, or kernel resource is created here.

## Seven-category / fifteen-item self-audit

| Check | Disposition | Evidence / scope reason |
|---|---|---|
| C1a empty/minimum | PASS | BE-03, no observed allocation/row; BE-01 singleton |
| C1b partition edges | PASS | BE-08 threshold 2/3, BE-P1 counter saturation and threshold 1/u32::MAX; beyond the typed integer range is not an input |
| C2a documented state model | PASS | Rust module docstring names allocation, readiness, veto, rejected-write and consumer states |
| C2b invalid transitions | PASS, scoped | BE-05/06 retained veto rejects re-eligibility; BE-11 repeated stopped state rejects restart; pure policy exhausts veto/status combinations. No new general lifecycle state machine is claimed. |
| C3 zero/one/many | PARTIAL: BE-02 multi-allocation coverage deferred to [#282](https://github.com/overdrive-sh/overdrive/issues/282#issuecomment-5584665927) | Observed membership 0/1; rows/listeners absent/1/3; TCP and UDP sharing a port. Singleton identity checks do not prove multi-allocation membership/order. A zero-listener Service is rejected by existing admission, so no impossible Service is fabricated. |
| C4a repeats | PASS | BE-01/02/03 exact repeat equality; BE-08 intended non-idempotent counter increments; BE-11 repeated stopped reconciliation |
| C4b inverse without prerequisite | PASS | BE-03 desired empty publication without prior membership or backend row |
| C5a decision combinations | PASS | BE-P1 full 2×2×3 policy truth table; BE-02/08 listener count/protocol/readiness combinations |
| C5b orthogonality | PASS | Listener count cannot multiply readiness counters; terminal veto cannot mutate unrelated View inputs |
| C6a malformed ingress | N/A | No new parser, public request, configuration input or decode policy; existing admission remains the fixture entrance |
| C6b exact errors | PASS, scoped | BE-04/06 require the exact injected Unreachable peer inside Shim/Observation, not any failure |
| C6c closed error surface | PASS, scoped | Both fault sites reject every other Result shape; no new error variant or general error-policy change |
| C7a resource fault | PASS | Actual observation write refusal before initial publication and on deciding withdrawal |
| C7b interruption/partial effect | PASS | BE-06 rejected effect plus persisted View and independently drained terminal; normal re-registration reload, not a forced unreachable abort |
| C7c concurrent actors | PASS, bounded | Production ProbeRunner tasks and live resolver/DNS drain tasks run alongside scheduled production-owner reconciliation; seed/progress bounds are explicit |

**Original DISTILL mechanical score: 15/15 addressed. BE-02 correction:
C3 now explicitly records deferred multi-allocation coverage under #282;
the revised acceptance material awaits independent review.** This is not an independent review
verdict or a claim that RED suffixes have executed. Shared World setup, owner
ticks, row assertions and probe advancement are reused across at least four
scenarios; the pure property avoids twelve separate truth-table functions.

## Limits, structural complements and handoff

The new suite presently fails on missing accepted behavior; downstream
assertions behind those first failures are authored but **not yet exercised**.
After CD-0101-01/02 remediation, BE-07 executes all three exact stamp cases and
the retained-VIP checks before its unchanged complete-pair-count RED. Its
post-pair correlation/handoff/serial-dispatch suffix remains unexecuted. VIP
checks execute for every observed publication; the still-missing explicit
empty publication remains an authored assertion, not green empty-row evidence.
BE-09 separately proves the full healthy/failure/recovery List/Watch trajectory
today. BE-10's local-map suffix remains pending the missing real handoff.
Remote-dataplane projection and forced watch-lag recovery are unchanged consumer
contracts, not newly claimed by this composed Exec fixture; retain their
existing component tests (including the resolver's
`relist_recovers_a_row_dropped_by_watch_lag` and
`relist_on_lagged_recovers_a_dropped_update`). Native E09-v2 remains the
separate VM Service-traffic oracle. No native/full-100 or mutation run occurred
during this DISTILL task.

DELIVER/review must check exact ADR-0101 public shape and direct retirement in
production registration, AnyReconciler/AnyState/AnyReconcilerView, runtime view
variants/helpers, old bridge module/exports, producer wakes, and old
bridge-specific tests/invariant wrappers. These are deletion/compiler-fallout
checks, not an AST/source-text test. The original diagnostic's now-retired
registration/ticks need a narrowly reviewed adaptation after its unchanged
before-change result is retained; its final veto invariant must not be weakened.
The pure property has one existing `backend_addr` fixture field requiring the
accepted mechanical `backend_ip` rename. No test needs a newly invented API.

Preserve all existing unrelated dirty files. Only this amendment's test files,
test-module addition, and DISTILL documentation are acceptance-designer output.
The fresh consolidated DESIGN+DISTILL reviewer owns the independent verdict;
neither production implementation nor test ownership transfers to an architect.
