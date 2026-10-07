# Model-check findings, round 4 — guest-flow owner (netns-density-295), design revision 9

**Verdict.** The specification now models revision 9 (§ *Revision 8 decisions*
and § *Revision 9 decisions* of the feature delta): D16-CLAIM (ADR-0170),
D8a-HOLD (ADR-0169), D8a-LOOKUP (ADR-0171), D8a-ROUTE and U-6 restated, U-3,
the constant nft rules of ADR-0152 point 2, and the unchanged U-1, U-2, U-5 /
K-A4, SLOT-ABORT, session and slot rules. On the final spec bytes, **all 101
checks match their expected verdict: 24 properties hold, 77 expected
violations are found** (non-vacuity witnesses, teeth, environment faults and
documented trust-boundary behaviour). **No counterexample contradicts a design
rule of revision 9.** Every new rule has teeth: switching it off brings a
counterexample back. `cargo xtask quint verify-evidence --subsystem
guest-flow-owner` passes.

What the run adds beyond confirmation (details in § *Observations*):

1. **D8a-HOLD × D8a-LOOKUP: the steering recovery's quiesce widens the
   root-detach window to listening ports.** With the link detached, a port is
   delivered to a wildcard host service unless an intake listener is bound to
   it (K-D1). Quiescence closes every intake listener, so during the
   `GuestPrefixSteering` recovery's hold Overdrive's own marked clients
   (leg-S, probes) to a port the guest *is* listening on reach the wildcard
   service, where before the quiesce they reached the intake listener. Inside
   the trust boundary and bounded by the recovery, but the design does not say
   whether leg-S and probes dial workload addresses while forwarding is
   quiesced (underspecified point UP-1).
2. **D16-CLAIM: `GuestCidsHeldElsewhere` is decided per claim, not on a
   snapshot.** A foreign user can release an offset already tried and take one
   not yet tried within one walk, so the refusal can be returned when at no
   single instant were all free CIDs held. Harmless (non-terminal, placement
   retries); invariant 5's text should say "held at the moment its claim was
   tried".
3. **D8a-ROUTE: verify and route are two steps.** A root detach between them
   leaves the route without steering. That is the trust boundary (the audit
   detects it within one period); what holds is "never kept or added without a
   verify that returned Ok in this boot".

## Environment

| Item | Value |
|---|---|
| Host | Lima VM `overdrive`, `uname -r` = `7.0.0-34-generic` |
| Quint | 0.32.0 |
| Apalache | 0.56.1 (build 70cdaf4); TLC via `quint verify --backend tlc` |
| JVM | `openjdk version "21.0.12.1" 2026-08-18 LTS` |
| Runner | `cargo xtask lima run -- cargo xtask quint check --subsystem guest-flow-owner --record` (3 jobs), run id `281448-1791331281.413`, 2026-10-07T00:01:21Z – 00:21:06Z; git head `d34f054ac` (dirty: this revision's specs) |
| Evidence | `specs/quint/guest-flow-owner/evidence/` (`summary.json`, `summary.md`, `traces/`); `cargo xtask quint verify-evidence --subsystem guest-flow-owner` → `ok … evidence matches the specs; every outcome met expect` |

Final spec SHA-256:

```text
f1afeac60661250bf9f2394b1356c5b08b3b9ef028a1236f406231cdaa4b7242  cid_lease.qnt
6cc2220a27325da8f71240011843ff48ebea82a4972738f44c51ecd3e32d837d  owner_flows.qnt
e4a66e226f5b1b2dc7cf743a613b3b354b8d668750f9889d7a7e853e82f6140c  steering.qnt
b46148378362a12ce7778bbd252841a96082001cb78b67868bbb1af5132ec557  udp_slots.qnt
3830d8282884f1e021eb34e9e9f85dd45fc674fd219222ce127313d450187009  hazard/cid_lease_hazards.qnt
3bb69466801e4a666d72baf6b9e917ca5588a01d7da3b89d3ec57e40df45e472  hazard/owner_flows_hazards.qnt
1daab9fdc1c1327483f944f187ba3f5c2bef68dbac4b4345eafc7898aa021151  hazard/steering_hazards.qnt
edef8104826e53b50d2db4ef5a9efcf36a51b404f50e9c504c3ad3b9bbe98689  hazard/udp_slots_hazards.qnt
da4283f25937d555983ce1f189e53fac01f9226f254b60e3c40bf7d28b113af5  checks.toml
```

`udp_slots.qnt` and both hazard files of modules A and B are byte-identical to
round 3; `owner_flows.qnt` changed only by a header comment (its `quiesced`
latch is the projection "holder set non-empty" of D8a-HOLD).

## What changed in the specification

**Module C (`cid_lease.qnt`), rewritten for D16-CLAIM.** State: the host
kernel's CID holder table (free / foreign / a VMM that survived a `serve`
crash / our claim of allocation *a*), leases, next-fit cursor, the stepwise
`assign` walk under the pool lock, and where each claim's file is (slot / in
transit / VMM). Actions: walk begin, one claim ioctl per step (claim, `InUse`
skip, non-`InUse` error ending the walk with no change), `take_claim`,
`Vmm::create` Ok / Err, VMM exit before READY, stop, `pool.release`
(drops an untaken claim), redeploy, foreign take / release (unbounded in the
safety instances), surviving-VMM exit, `serve` crash + restart (serve-held
claims close, VMM-held devices survive). Deleted with revision 7: the
exclusion set, preference, fallback, READY/stop clearing and every check
built on them (17 checks).

**Module D (`steering.qnt`), rewritten for revision 9.** State: `serve` up /
boot step / this boot's verify result; nft rules (+ leg-C divert); route;
the pinned link; steering-map entries; intake listeners; allocation, session,
last `ListenState`; holder set, latch, per-component recovery
(`IpRules`, `GuestPrefixSteering`, `FlowListeners`). Boot order: sweep →
start_shared_owner (rules, leg-C) → steering converge (may fail) → verify
(refuses, removing the tagged route) → route → open. Every per-allocation
event is one step that also closes each listener whose port stops serving
(closing removes the entry in the same step, K-L3). Landing is a state
predicate for marked (leg-S / probes, present only while `serve` is up),
unmarked host-local and remote clients on every port. Deleted with
revision 8: the `intake_listeners` element set, admit / revoke / re-assert,
resetting listeners, D8a-REVOKE / REASSERT / PROBE / FLUSH properties
(the element set survives only as hazard `BUG_ELEMENT_SET`), and the single
unowned latch (survives only as hazard `BUG_ANY_RESTORE`).

## Bounds and state counts

| Area | Instance(s) | Bound | Distinct states (TLC, exhaustive) |
|---|---|---|---|
| A — sessions, flows, `TcpAccept` | `flows_ok` | 1 CID, 2 generations, 2 control connections, 2 guest-opened flows, 1 host-opened flow; Apalache 12 and 14 steps | unchanged from round 3 |
| B — UDP slots | `slots_live`, `slots_ok` | 2 slots, 2 sockets, 1–2 destinations, 2–3 association ids; Apalache 8–12 steps | unchanged from round 3 |
| C — CID lease | `lease_ok` (2 workloads, unbounded foreign churn, crash/restart, 1 pre-READY exit, 1 claim error) | 3 offsets | 142,068 |
| C | `lease_three` (3 workloads) | 3 offsets | 542,706 |
| C | `lease_live` (2 foreign changes, 2 exits, no restart) | 3 offsets | 45,544 (liveness graph) |
| D — steering | `steer_ok` (crash, foreign flush, root entry deletion, listener-task exit, steer failing forever, 1 bind and 1 converge failure) | 1 address, 2 ports (1 declared), 2 generations | 46,324 |
| D | `steer_envDetach` (all of the above + root link detach) | same | 284,324 |
| D | `steer_two` (2 declared ports, rollback) | same | 4,155 |
| D | liveness instances (`steer_live`, `steer_detachLive`, `steer_flushLive`) | 1 port, 1 generation | 946 – 3,330 |
| D | `steer_ok` Apalache | 16 steps | — |

## Every check — expected vs recorded

All expectations were written before the first run (the predictions file of
this round); no expectation was changed after a run. Two checks were added
before the recording (`steering-flush-no-wildcard`,
`steering-reach-only-serving`: named views of `Safety`) and one after the first
recording (`steering-env-detach-held-servable-wildcard`, observation 1),
each with its prediction stated before it ran.

| Check | Instance | Property | Backend / bound | Expected | Recorded | s |
|---|---|---|---|---|---|---|
| `owner-flows-safety-tlc` | `flows_ok` | `Safety` | tlc / exhaustive | holds | holds | 13 |
| `owner-flows-safety-apalache` | `flows_ok` | `Safety` | apalache / max_steps=12 | holds | holds | 210 |
| `owner-flows-guest-half-ends` | `flows_ok` | `GuestHalfEnds` | tlc / exhaustive | holds | holds | 24 |
| `owner-flows-witness-paired` | `flows_ok` | `WitnessNoPaired` | apalache / max_steps=14 | violation | violation | 31 |
| `owner-flows-witness-reuse` | `flows_ok` | `WitnessNoReuse` | apalache / max_steps=14 | violation | violation | 9 |
| `owner-flows-witness-quiesce-abort` | `flows_ok` | `WitnessNoQuiesceAbort` | apalache / max_steps=14 | violation | violation | 13 |
| `owner-flows-witness-late-completion` | `flows_ok` | `WitnessNoLateCompletion` | apalache / max_steps=14 | violation | violation | 21 |
| `owner-flows-witness-accept-paired` | `flows_ok` | `WitnessNoAcceptPaired` | apalache / max_steps=14 | violation | violation | 10 |
| `owner-flows-witness-phantom-connect` | `flows_ok` | `WitnessNoPhantomConnect` | apalache / max_steps=14 | violation | violation | 12 |
| `owner-flows-witness-queue-reset` | `flows_ok` | `WitnessNoQueueReset` | apalache / max_steps=14 | violation | violation | 9 |
| `owner-flows-hazard-split-claim` | `flows_bugSplitClaim` | `OneLiveSession` | apalache / max_steps=14 | violation | violation | 12 |
| `owner-flows-hazard-install-early` | `flows_bugInstallEarly` | `NoInstallBeforeRead` | apalache / max_steps=14 | violation | violation | 13 |
| `owner-flows-hazard-admit-split` | `flows_bugAdmitSplit` | `NoAdmitWhenClosed` | apalache / max_steps=14 | violation | violation | 21 |
| `owner-flows-hazard-paired-no-recheck` | `flows_bugPairedNoRecheck` | `NoAdmitWhenClosed` | apalache / max_steps=14 | violation | violation | 124 |
| `owner-flows-hazard-no-abort-on-loss` | `flows_bugNoAbortOnLoss` | `NoAdmitWhenClosed` | apalache / max_steps=14 | violation | violation | 14 |
| `owner-flows-hazard-teardown-skips-flows` | `flows_bugTeardownSkipsFlows` | `CurrentGeneration` | apalache / max_steps=14 | violation | violation | 33 |
| `owner-flows-hazard-late-paired-resurrects` | `flows_bugLatePairedResurrects` | `ClosedIsTerminal` | apalache / max_steps=14 | violation | violation | 13 |
| `owner-flows-env-stale-conn` | `flows_envStaleConn` | `CurrentGeneration` | apalache / max_steps=14 | violation | violation | 16 |
| `owner-flows-env-guest-misses-host-close` | `flows_envGuestMissesHostClose` | `GuestHalfEnds` | tlc / exhaustive | violation | violation | 129 |
| `owner-flows-alt-tombstone-misses-close` | `flows_altTombstoneMissesClose` | `GuestHalfEnds` | tlc / exhaustive | violation | violation | 9 |
| `udp-slots-safety-tlc` | `slots_live` | `Safety` | tlc / exhaustive | holds | holds | 26 |
| `udp-slots-release-liveness` | `slots_live` | `ReleasedSlotBecomesFree` | tlc / exhaustive | holds | holds | 73 |
| `udp-slots-safety-apalache` | `slots_ok` | `Safety` | apalache / max_steps=8 | holds | holds | 478 |
| `udp-slots-witness-late-paired` | `slots_ok` | `WitnessNoLatePaired` | apalache / max_steps=12 | violation | violation | 8 |
| `udp-slots-witness-routed` | `slots_ok` | `WitnessNoRouted` | apalache / max_steps=12 | violation | violation | 9 |
| `udp-slots-witness-two-slots-one-socket` | `slots_ok` | `WitnessNoTwoSlotsOneSocket` | apalache / max_steps=12 | violation | violation | 8 |
| `udp-slots-witness-reassociation` | `slots_ok` | `WitnessNoReassociation` | apalache / max_steps=12 | violation | violation | 9 |
| `udp-slots-hazard-claim-split` | `slots_bugClaimSplit` | `SlotHeldOnce` | apalache / max_steps=12 | violation | violation | 9 |
| `udp-slots-hazard-keep-frames` | `slots_bugKeepFrames` | `ReleasedSlotCarriesNoData` | apalache / max_steps=12 | violation | violation | 17 |
| `udp-slots-hazard-late-paired-by-slot-route` | `slots_bugLatePairedBySlot` | `RouteCurrent` | apalache / max_steps=12 | violation | violation | 30 |
| `udp-slots-hazard-late-paired-by-slot-data` | `slots_bugLatePairedBySlot` | `ReleasedSlotCarriesNoData` | apalache / max_steps=12 | violation | violation | 87 |
| `udp-slots-hazard-ignore-late-paired` | `slots_bugIgnoreLatePaired` | `LatePairedAborted` | apalache / max_steps=12 | violation | violation | 10 |
| `udp-slots-hazard-keep-frames-on-end` | `slots_bugKeepFramesOnEnd` | `NoReassociationWithoutNewDatagram` | apalache / max_steps=12 | violation | violation | 9 |
| `udp-slots-hazard-no-audit-fairness` | `slots_liveTeeth` | `NoAuditFairness` | tlc / exhaustive | violation | violation | 8 |
| `cid-lease-safety-tlc` | `lease_ok` | `Safety` | tlc / exhaustive | holds | holds | 7 |
| `cid-lease-three-safety-tlc` | `lease_three` | `Safety` | tlc / exhaustive | holds | holds | 18 |
| `cid-lease-live-safety-tlc` | `lease_live` | `Safety` | tlc / exhaustive | holds | holds | 6 |
| `cid-lease-eventually-placed` | `lease_live` | `EventuallyPlaced` | tlc / exhaustive | holds | holds | 8 |
| `cid-lease-witness-skip` | `lease_ok` | `WitnessNoSkip` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-witness-orphan-skip` | `lease_ok` | `WitnessNoOrphanSkip` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-witness-held-elsewhere` | `lease_ok` | `WitnessNoHeldElsewhere` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-witness-claim-error` | `lease_ok` | `WitnessNoClaimErr` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-witness-running` | `lease_ok` | `WitnessNoRunning` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-refusal-snapshot` | `lease_ok` | `RefusalSnapshotConsistent` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-hazard-check-then-claim` | `lease_bugCheckThenClaim` | `NoLaunchClash` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-hazard-check-then-claim-held` | `lease_bugCheckThenClaim` | `ClaimHeld` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-hazard-release-before-vmm` | `lease_bugReleaseBeforeVmm` | `NoLaunchClash` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-hazard-release-before-vmm-held` | `lease_bugReleaseBeforeVmm` | `ClaimHeld` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-hazard-remember-inuse` | `lease_bugRemember` | `NoSkipWhileClaimable` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-hazard-remember-inuse-placed` | `lease_live_bugRemember` | `EventuallyPlaced` | tlc / exhaustive | violation | violation | 8 |
| `steering-safety-tlc` | `steer_ok` | `Safety` | tlc / exhaustive | holds | holds | 7 |
| `steering-flush-no-wildcard` | `steer_ok` | `NoWildcardReached` | tlc / exhaustive | holds | holds | 7 |
| `steering-reach-only-serving` | `steer_ok` | `ReachOnlyServing` | tlc / exhaustive | holds | holds | 7 |
| `steering-noflush-safety-tlc` | `steer_noflush` | `Safety` | tlc / exhaustive | holds | holds | 6 |
| `steering-noflush-unmarked-on-legc` | `steer_noflush` | `UnmarkedOnLegC` | tlc / exhaustive | holds | holds | 6 |
| `steering-two-safety-tlc` | `steer_two` | `Safety` | tlc / exhaustive | holds | holds | 5 |
| `steering-safety-apalache` | `steer_ok` | `Safety` | apalache / max_steps=16 | holds | holds | 193 |
| `steering-flush-unmarked-bypass` | `steer_ok` | `UnmarkedOnLegC` | tlc / exhaustive | violation | violation | 5 |
| `steering-two-activation-completes` | `steer_two` | `ActivationCompletes` | tlc / exhaustive | holds | holds | 5 |
| `steering-serving-reachable` | `steer_live` | `ServingReachable` | tlc / exhaustive | holds | holds | 5 |
| `steering-boot-opens` | `steer_boot` | `BootOpens` | tlc / exhaustive | holds | holds | 5 |
| `steering-witness-steered` | `steer_ok` | `WitnessNoSteered` | tlc / exhaustive | violation | violation | 5 |
| `steering-witness-second-allocation` | `steer_ok` | `WitnessNoSecondAllocation` | tlc / exhaustive | violation | violation | 5 |
| `steering-witness-activation-err` | `steer_two` | `WitnessNoActivationErr` | tlc / exhaustive | violation | violation | 5 |
| `steering-witness-boot-refusal` | `steer_ok` | `WitnessNoBootRefusal` | tlc / exhaustive | violation | violation | 5 |
| `steering-witness-resteer` | `steer_ok` | `WitnessNoResteer` | tlc / exhaustive | violation | violation | 5 |
| `steering-witness-overlap` | `steer_ok` | `WitnessNoOverlap` | tlc / exhaustive | violation | violation | 5 |
| `steering-witness-still-quiesced` | `steer_ok` | `WitnessNoStillQuiesced` | tlc / exhaustive | violation | violation | 5 |
| `steering-witness-crash-with-route` | `steer_ok` | `WitnessNoCrashWithRoute` | tlc / exhaustive | violation | violation | 5 |
| `steering-witness-legc` | `steer_ok` | `WitnessNoLegC` | tlc / exhaustive | violation | violation | 5 |
| `steering-hazard-element-set` | `steer_bugElementSet` | `ReachOnlyServing` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-element-set-flush` | `steer_bugElementSetFlush` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 9 |
| `steering-hazard-element-close-first` | `steer_bugElementCloseFirst` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-steering-in-nft` | `steer_bugSteeringInNft` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-unpinned` | `steer_bugUnpinned` | `NothingDeliveredWhileDown` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-nonloopback-assign` | `steer_bugNonloopback` | `RemoteNeverDirect` | tlc / exhaustive | violation | violation | 9 |
| `steering-hazard-entry-outlives-close` | `steer_bugEntryOutlives` | `EntryNamesServingListener` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-route-first` | `steer_bugRouteFirst` | `RouteImpliesSteering` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-route-first-wildcard` | `steer_bugRouteFirst` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-no-verify` | `steer_bugNoVerify` | `ConvergeNeedsSteering` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-no-verify-wildcard` | `steer_bugNoVerify` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-rollback-leaves-listener` | `steer_bugRollback` | `ActivationAllOrNothing` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-rollback-leaves-listener-reach` | `steer_bugRollback` | `ReachOnlyServing` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-act-ok-partial` | `steer_bugActOkPartial` | `ActivationAllOrNothing` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-teardown-keeps-listeners` | `steer_bugTeardownKeeps` | `NoReleaseWithListener` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-quiesce-keeps` | `steer_bugQuiesceKeeps` | `QuiescedClosesListeners` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-quiesce-keeps-reach` | `steer_bugQuiesceKeeps` | `ReachOnlyServing` | tlc / exhaustive | violation | violation | 9 |
| `steering-hazard-any-restore` | `steer_bugAnyRestore` | `NoReopenBeforeRepair` | tlc / exhaustive | violation | violation | 8 |
| `steering-hazard-repair-without-hold` | `steer_bugRepairNoHold` | `NoRepairWithoutHold` | tlc / exhaustive | violation | violation | 8 |
| `steering-env-detach-safety` | `steer_envDetach` | `SafetyUnderDetach` | tlc / exhaustive | holds | holds | 15 |
| `steering-env-detach-wildcard` | `steer_envDetach` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 9 |
| `steering-env-detach-route-implies-steering` | `steer_envDetach` | `RouteImpliesSteering` | tlc / exhaustive | violation | violation | 9 |
| `steering-env-detach-down` | `steer_envDetach` | `NothingDeliveredWhileDown` | tlc / exhaustive | violation | violation | 8 |
| `steering-env-detach-remote-direct` | `steer_envDetach` | `RemoteNeverDirect` | tlc / exhaustive | violation | violation | 8 |
| `steering-env-detach-held-servable-wildcard` | `steer_envDetach` | `WitnessNoServableWildcardWhileHeld` | tlc / exhaustive | violation | violation | 8 |
| `steering-detach-exposure-ends` | `steer_detachLive` | `DetachExposureEnds` | tlc / exhaustive | holds | holds | 9 |
| `steering-hazard-any-restore-exposure` | `steer_detachLiveAnyRestore` | `DetachExposureEnds` | tlc / exhaustive | violation | violation | 9 |
| `steering-hazard-no-recovery-fairness` | `steer_detachLiveNoFairness` | `DetachExposureEndsNoRecoveryFairness` | tlc / exhaustive | violation | violation | 129 |
| `steering-flush-legc-bypass-ends` | `steer_flushLive` | `LegCBypassEnds` | tlc / exhaustive | holds | holds | 9 |
| `steering-steer-forever-unreachable` | `steer_steerForever` | `ServingReachable` | tlc / exhaustive | violation | violation | 8 |
| `owner-flows-safety-apalache-deep` | `flows_ok` | `Safety` | apalache / max_steps=14 | holds | holds | 481 |

## First-pass results and the modelling fixes

The first full run (unrecorded) matched 94 of 98 checks. The four mismatches,
and one in the re-run, were my modelling errors; each is stated exactly. The
first-pass logs are kept below in condensed form (the runner writes
`evidence/` only for the final recorded run; earlier runs are not retained
there).

| Check | Predicted | First pass | What was wrong in the model | Fix |
|---|---|---|---|---|
| `steering-env-detach-safety` | holds | violation | The ghost behind `ConvergeNeedsSteering` read the live link at the route step, not the result of this boot's verify. Trace: converge → verify Ok → **root detach** → route added. Under a root detach that is the trust boundary (measured by `RouteImpliesSteering`), not a converge that ignored verification | A process-scoped `verified` flag set by the verify step; the ghost fires when the route step runs without it (observation 3) |
| `steering-hazard-unpinned` | violation | holds | The hazard instance lacked the fault the rule defends against. With the nft rules present, every non-exempt client is diverted or dropped and the exempt (marked) clients exist only while `serve` is up, so an unpinned program is exposed while `serve` is down only when the rules are also gone | The instance enables `ENV_FOREIGN_FLUSH` (observation 5) |
| `steering-detach-exposure-ends`, `steering-flush-legc-bypass-ends` | holds | violation | The fairness set omitted the owner's own steps. Trace: `ActivateBegin` then stuttering — the recovery's quiesce is serialized behind the activation item, which never completed | `weakFair(OwnerStep)` added (the owner's sequencer; not fairness against other recoveries) |
| `steering-detach-exposure-ends` (second pass) | holds | violation | The fairness set omitted the recovery's own re-repair step. Trace: quiesce → repair (link back) → **root detaches again** → recovery is `repaired` but damaged, `RecRedo` never taken, stutter | `weakFair(RecRedo(GuestPrefixSteering))` (and for `IpRules` in `LegCBypassEnds`) |
| `udp-slots-safety-apalache` (first recording) | holds | timed out at 900 s | Not a model error: the spec is unchanged from round 3 (467 s in the unrecorded run); under three parallel jobs it exceeded the 900 s default | `timeout_secs = 1800` for it and `owner-flows-safety-apalache-deep` (no property changed); re-recorded |

After the fixes both liveness teeth were re-inspected to confirm they fail for
the intended reason, not an unrelated stutter:
`steering-hazard-any-restore-exposure` loops *quiesce(Steering) →
quiesce(FlowListeners) → repair(FlowListeners) → restore(FlowListeners)
[ends every hold] → listener exit → …* with the steering repair never
continuously enabled; `steering-hazard-no-recovery-fairness` holds the
steering quiesce forever while other recoveries cycle and the repair is never
taken.

## Counterexamples against design rules

**None.** Every `holds` check of a revision-9 rule holds, on exhaustive TLC
search of each instance and on Apalache to 16 steps for `steer_ok`.

## Teeth — one hazard per rule (all `violation`, as predicted)

| Rule | Hazard instance | Broken property | Counterexample in short |
|---|---|---|---|
| D16-CLAIM: claim and lease are one step | `lease_bugCheckThenClaim` | `NoLaunchClash`, `ClaimHeld` | assign sees CID free → foreign takes it → `create` clashes |
| D16-CLAIM: serve holds the claim until the VMM does | `lease_bugReleaseBeforeVmm` | `NoLaunchClash`, `ClaimHeld` | `take_claim` closes serve's file → foreign takes the CID → `create` clashes |
| D16-CLAIM: nothing remembered about `InUse` | `lease_bugRemember`, `lease_live_bugRemember` | `NoSkipWhileClaimable`, `EventuallyPlaced` | offset skipped while unheld; a workload refuses forever while a free CID stays unheld |
| D8a-LOOKUP: the listener is the decision | `steer_bugElementSet` | `ReachOnlyServing` | revision-8 shape: element and listener change in separate steps — port 1 steered and Active, guest stops listening, and until the element is removed (indefinitely while removal fails) the listener stays bound and a marked connect completes (the old D8a-PROBE) |
| D8a-LOOKUP: same | `steer_bugElementSetFlush` | `NoWildcardReached` | foreign flush during boot deletes the marked-reset rule and elements; boot adds the route → wildcard on every port without a listener (the old D8a-FLUSH) |
| D8a-LOOKUP: same | `steer_bugElementCloseFirst` | `NoWildcardReached` | listener closed before its element is removed → element steers to the wildcard |
| D8a-LOOKUP: not in the flushable table | `steer_bugSteeringInNft` | `NoWildcardReached` | foreign flush removes the steering → route without a lookup decision |
| D8a-LOOKUP: pinned | `steer_bugUnpinned` | `NothingDeliveredWhileDown` | crash removes the program, flush removes the rules → host-local client reaches the wildcard while `serve` is down |
| D8a-LOOKUP: loopback ingress only | `steer_bugNonloopback` | `RemoteNeverDirect` | divert flushed → remote connection assigned the intake directly, bypassing leg-C |
| K-L3 (entry dies with the listener) | `steer_bugEntryOutlives` | `EntryNamesServingListener` | port 1 steered during activation, `serve` crashes: the listener closes, the entry survives naming a closed socket |
| D8a-ROUTE: converge, verify, then route | `steer_bugRouteFirst` | `RouteImpliesSteering`, `NoWildcardReached` | first boot adds the route before converging → wildcard until converge (indefinitely if it crashes there) |
| D8a-ROUTE: refuse when unverified | `steer_bugNoVerify` | `ConvergeNeedsSteering`, `NoWildcardReached` | converge fails, boot continues → route without steering |
| U-6: rollback closes every listener | `steer_bugRollback` | `ActivationAllOrNothing`, `ReachOnlyServing` | port 1 bound, port 2 bind fails, rollback closes only port 2 → Provisioned CID with a listener left |
| U-6: Ok only when every wanted port is steered | `steer_bugActOkPartial` | `ActivationAllOrNothing` | Ok returned before the steer |
| U-3 | `steer_bugTeardownKeeps` | `NoReleaseWithListener` | teardown Ok with a listener open → release |
| D8a-HOLD / M-7: quiesce closes listeners | `steer_bugQuiesceKeeps` | `QuiescedClosesListeners`, `ReachOnlyServing` | latch set, listener still open and reachable |
| D8a-HOLD: only a holder ends its hold | `steer_bugAnyRestore` | `NoReopenBeforeRepair`; `DetachExposureEnds` (`steer_detachLiveAnyRestore`) | FlowListeners restore reopens while the steering recovery has not repaired; the lasso above |
| D8a-HOLD: repair only while holding | `steer_bugRepairNoHold` | `NoRepairWithoutHold` | repair runs with no hold (see observation 6) |
| D8a-HOLD: the bound rests on the recovery's own steps | `steer_detachLiveNoFairness` | `DetachExposureEndsNoRecoveryFairness` | the steering repair is never taken |
| Retained: U-1, U-5, K-A2, K-A4, session claim, install-after-read, slot rules, SLOT-ABORT, audit fairness | `flows_bug*`, `flows_env*`, `slots_bug*` | as round 3 | unchanged teeth, all still `violation` |

## Observations

1. **D8a-HOLD × D8a-LOOKUP (and M-7): the steering recovery's hold widens the
   root-detach window to listening ports.** Check
   `steering-env-detach-held-servable-wildcard` (violation, as predicted).
   Minimal trace: boot opens (converge, verify Ok, route) → **root detaches
   the link** → provision, session, activate, guest reports port 1 listening →
   the audit finds `GuestPrefixSteering` damaged → its recovery quiesces
   (`holders = {GuestPrefixSteering}`, every intake listener closed) → a marked
   connection (leg-S / probe) to `workload_addr:1` finds no lookup decision
   and no specifically-bound listener and lands on the wildcard host service.
   Without the quiesce, an open intake listener on port 1 would have taken it
   by its specific bind (K-D1). The window ends at the repair
   (`steering-detach-exposure-ends` holds). It is inside the trust boundary
   (root detached the link), but for its duration it reaches Overdrive's own
   inbound path (leg-S delivering decrypted mesh traffic) on ports the guest
   serves, which the design text does not mention.
2. **D16-CLAIM: refusal on a per-claim basis.** Check
   `cid-lease-refusal-snapshot` (violation, documented). Trace: foreign holds
   offsets 0 and 1 → walk tries 0 (`InUse`), 1 (`InUse`) → foreign releases 0
   and takes 2 → walk tries 2 (`InUse`) → `GuestCidsHeldElsewhere`, while
   offset 0 is unheld. Each skip was the kernel's answer at its own step
   (`NoSkipWhileClaimable` holds); the refusal is non-terminal and the next
   placement retry claims offset 0. Invariant 5 of § *Formal protocol model*
   ("only when every free offset's CID is held by another holder at that
   step") holds per claim, not for the walk as one instant.
3. **D8a-ROUTE: the verify → route window.** Checks
   `steering-env-detach-route-implies-steering` / `-wildcard` (violation, trust
   boundary). Minimal trace: converge → verify Ok → route → root detaches →
   the wildcard is reachable on any port with no intake listener. The same
   window exists between the verify step and the route step. Holds:
   `ConvergeNeedsSteering` (the route is never kept or added without a verify
   that returned Ok in this boot) and `DetachExposureEnds`.
4. **ADR-0171 consequences: a foreign flush moves unmarked host-local
   traffic off leg-C.** Check `steering-flush-unmarked-bypass` (violation,
   documented). Trace: boot installs rules, converges, **flush during boot**,
   verifies, adds the route (the rules are no longer the route's
   precondition), activation steers port 1 → an unmarked host-local client
   reaches the intake listener directly instead of leg-C. Never a host
   service (`steering-flush-no-wildcard` holds); never a non-serving port
   (`steering-reach-only-serving` holds); ended by the `IpRules` recovery
   under fairness of its own steps (`steering-flush-legc-bypass-ends` holds).
   Boot can therefore open with the nft rules absent, which matches revision
   9's point 2; ADR-0171 states the leg-C consequence.
5. **ADR-0171 alternatives, wording.** The rejected "unpinned program"
   alternative exposes the prefix while `serve` is down only when the nft
   rules are also gone (or a non-Overdrive root process uses the exempt
   mark): with the rules present every non-exempt client is diverted or
   dropped. The rejection stands — the design does not want to depend on the
   flushable table — but the ADR sentence "with the program gone the prefix
   delivers to wildcard host services while `serve` is down" is exact only
   together with a flushed table.
6. **D8a-HOLD: what the hold defends in revision 9.** With the element set
   gone, no safety property of module D depends on the repair running inside
   the hold; the teeth `steering-hazard-repair-without-hold` is the rule
   itself (a ghost). The hold is load-bearing for the bound: without it
   (`BUG_ANY_RESTORE`) another recovery's restore pre-empts the steering
   repair forever under weak fairness of the steering recovery's steps.
7. **Not checked: activation under unbounded root entry deletion.**
   `ActivationCompletes` runs on `steer_two`, which has no root entry
   deletion. With root deleting each entry right after `steer`, activation
   can be kept from `Ok` (trust boundary; activation is bounded by its EXEC
   budget). Stated as an instance choice, not a result.

## Underspecified points (decision id — what must be pinned)

- **UP-1 (D8a-HOLD, D8a-LOOKUP, M-7):** whether leg-S and the probes dial
  workload addresses while forwarding is quiesced. Quiescence refuses guest
  flows and closes intake listeners; the text does not say that leg-S stops
  delivering inbound connections or that probes stop. If they continue, during
  the `GuestPrefixSteering` recovery after a root detach they reach wildcard
  host services on ports the guest serves (observation 1). Pin either that
  leg-S / probes are gated by the latch, or that the steering recovery keeps
  listeners open (it repairs only the link), or accept and state the window.
- **UP-2 (D8a-ROUTE, ADR-0124):** failure of `converge` during the steering
  recovery's repair is not modelled (the repair is one successful step);
  the design relies on ADR-0124's bound and fail-stop. Nothing new to pin if
  that is the intent; the model gives ordering, not the time bound.
- **UP-3 (§ Formal protocol model text):** the section's module list,
  invariants 6–9 and 11, its progress list (D8a-REASSERT, D8a-FLUSH) and its
  assumption table (K-D6, K-D2's `intake_listeners`, K-D3's "elements", no
  K-L1–K-L5) predate revision 9. The specification now states the revision-9
  set (README § *Assumptions*); the section needs the same update by the
  architect.
- **UP-4 (D16-CLAIM wording):** invariant 5's "at that step" (observation 2).

## Assumptions and their discharging items

| ID | Assumption | Discharged by |
|---|---|---|
| K-A1 | Peer-CID attribution is authentic | V-10 (bind level, P-17) |
| K-A2 | Released vhost device's connections reset before CID reuse | V-22 |
| K-B1 | Closing `T_g` / `Q_g` is observed as the association's end | V-22 |
| K-A3 | Socket installed before request read → verdict drops it | P-23 (+ R5-9) |
| K-A4 | Host close of `V_h` reaches the guest, accept queue included | V-24 |
| A-FID | Host flow ids not reused within a pairing window | `FlowId` contract (design property) |
| K-B2 | `sock_release` reports, or the audit finds orphans | R5-14 |
| K-B3 | Emptying / recreating cells discards parked frames | V-5(c) |
| K-C1 | Duplicate CID refused at creation; free at last close | P-16 |
| K-C2 | Foreign vhost users take / release CIDs at any time | Environment, modelled as actions |
| K-C3 | Claim exclusive while referenced, survives VMM handoff, released at last close | V-25 |
| K-D1, K-D2, K-D4 | Delivery with route / without lookup decision; nft divert / reject / drop; no local delivery without the route | V-19 |
| K-D3 | nft rules / route persist across crash; batches atomic; foreign deletion a fault | V-19 (persistence), V-23 |
| K-D5 | Closing a listener resets its accept queue | V-13 |
| K-L1 | `sk_lookup` runs for loopback-ingress connections to a `local`-route address before listener / wildcard lookup; drop ⇒ reset | V-26 |
| K-L2 | TPROXY-assigned packets bypass `sk_lookup` | V-26 |
| K-L3 | Closing a listener removes it from the socket map in the same step | V-26 |
| K-L4 | Pinned link and map run with no process; no live socket after owner exit | V-26 |
| K-L5 | Ingress ifindex: loopback for host-local, receiving device otherwise | V-26 |

K-D6 is deleted with the element set. **No assumption lacks a discharging
item** (K-C2 is an environment fact modelled as actions).

## Trust-boundary bound (root link detach)

Measured on `steer_envDetach` (safety, 284,324 states) and
`steer_detachLive` (liveness):

- **Holds under root detach** (`steering-env-detach-safety`): a non-serving
  port is never reachable; every entry names an open listener of a serving
  port; activation all-or-nothing; no release with a listener; quiescence
  closes listeners; the holder rules; the route is never kept or added
  without this boot's verify.
- **Exposure:** while the link is detached and the route present, a
  connection that reaches the socket lookup lands on a wildcard host service
  for every port with no intake listener bound (`-wildcard`). Who is affected
  depends on the nft rules: with them, only marked clients (leg-S / probes —
  Overdrive's own, present while `serve` is up); with the rules also flushed,
  any host-local and remote client (`-remote-direct` shows a remote client
  then reaching an intake directly); with `serve` down, only if the rules are
  also flushed (`-down`: crash → flush → detach). During the steering
  recovery's hold the exposed set includes listening ports (observation 1).
- **End:** `steering-detach-exposure-ends` holds: with `serve` up, the
  exposure ends under fairness of the `GuestPrefixSteering` recovery's own
  steps only (quiesce: strong — it waits its turn behind at most one
  activation item; repair and re-repair: weak) plus the owner's own steps, and
  **no fairness against other recoveries**; across a crash, at the next boot's
  converge. In time: one audit period to detect plus the recovery's ADR-0124
  bound, else fail-stop (ADR-0169, ADR-0171); the model checks the ordering,
  V-23 / V-26 measure the time. Without D8a-HOLD the end is not guaranteed
  (`steering-hazard-any-restore-exposure`), nor without the recovery's
  fairness (`steering-hazard-no-recovery-fairness`).
- A deleted entry only refuses its port until the audit re-steers it
  (`steering-serving-reachable` holds with unbounded root entry deletion; the
  witness `steering-witness-resteer` shows the re-steer).

## CI flags

Every check measured under two minutes on one attempt carries `ci = true`.
Not CI: the Apalache safety runs (`owner-flows-safety-apalache` 210 s,
`-deep` 481 s, `udp-slots-safety-apalache` 478 s, `steering-safety-apalache`
193 s) and `owner-flows-hazard-paired-no-recheck` (124 s). Two CI checks
(`owner-flows-env-guest-misses-host-close`, `steering-hazard-no-recovery-fairness`)
show about 129 s in the recording only because their first attempt hit the
runner's start-up bound and was retried; their single attempts take about 9 s.

---

# Round 4b — D8a-FENCE (revision 9, items 3–7 of § *Revision 9 decisions*)

Appended 2026-10-07. The round-4 content above is unchanged. The round-4
recorded run (`281448-1791331281.413`) was untracked in git and is replaced by
this round's `--record`; it is preserved verbatim in
`docs/feature/netns-density-295/design/quint-guest-flow-owner-evidence-r4/`.

## Predictions (written before the first run of round 4b)

Hypothesis: the revision-9 text with D8a-FENCE (fence first in the steering
recovery; `local` only from a converge_shared that just verified; fence on
every unverified outcome; fence persists while `serve` is stopped) removes the
round-4 observation-1 trace and keeps the root-detach exposure inside "detach →
next fence", which is at most two steps of the steering recovery.

Predicted outcomes (each is the `expect` of `checks.toml`, set before running):

- Every round-4 design property of module D still holds on `steer_ok`,
  `steer_noflush`, `steer_two`, `steer_live`, `steer_boot`; every round-4 teeth
  still `violation`.
- `steer_envDetach`: `SafetyUnderDetach` holds, including the new
  `ExposureOnlyInTamperWindow`, `ExposureWithinTwoSteeringSteps`,
  `FencedWhileSteeringRecovers`, `SteeringHoldNoWildcard`,
  `RouteLocalOnlyVerified`, `NoElsewhere`, `FenceHoldsWhileStopped`,
  `NothingWhileStopped`.
- `WitnessNoServableWildcardWhileHeld` still violated, but only through
  "repair verified → `local` → root detaches again" (state `repaired` / `found`
  inside the hold), not through the round-4 quiesce trace.
- `WitnessNoServableWildcardOtherHold` violated: a FlowListeners / IpRules hold
  overlapping a detach the audit has not found yet exposes listening ports.
  Falsification: if it holds, quiescence never widens an exposure.
- Liveness: `DetachExposureEnds` holds under weak fairness of the audit's
  findings and the fence only (no owner / sequencer / repair fairness);
  `SteeringRecoveryResolves` holds on `steer_detachLive` and on
  `steer_repairForever` (ends in fail-stop with the fence).
- D8a-FENCE teeth: quiesce-before-fence, remove-not-fence, no-fresh-verify,
  restore-lifts-fence each violate their named property.
- Root route tampering (`steer_envRootRoute`): `ExposureOnlyInTamperWindow` and
  `ReachOnlyServing` hold; `NoElsewhere`, `RouteLocalOnlyVerified`,
  `FenceHoldsWhileStopped` violated (trust boundary); `RouteTamperEnds`
  violated (nothing observes the route while `serve` is up — to be reported as
  an underspecified point).
- Module C: `Safety` (now with `RefusalEachClaimInUse`) holds on all three
  instances; `cid-lease-hazard-remember-refusal` violated.

## Verdict (round 4b)

On the final spec bytes, **all 126 checks match their expected verdict: 28
properties hold, 98 expected violations are found** (non-vacuity witnesses,
teeth, environment faults, documented trust-boundary behaviour). **No
counterexample contradicts a design rule of revision 9 including D8a-FENCE.**
Every D8a-FENCE rule has teeth. The round-4 observation-1 trace (steering
recovery quiesces while the link is detached) is now reached only in the
hazard `steer_bugQuiesceBeforeFence`. `cargo xtask quint verify-evidence
--subsystem guest-flow-owner` passes on recorded run `332173-1791340295.607`.

What the run adds beyond confirmation:

1. **F-4b-1 (D8a-FENCE × D8a-HOLD, M-7): another component's quiesce still
   widens an undetected detach's exposure.** The steering recovery's own
   quiesce never does (`SteeringQuiesceNeverWidens` holds). But an IpRules or
   FlowListeners recovery whose first quiesce falls between a root detach and
   the audit's finding closes open intake listeners while the prefix is
   `local` and unverified (`steering-env-detach-other-hold-widens`,
   `QuiescenceNeverWidens` violated). Bounded: it is inside the detach → fence
   window, which still ends within two steering-recovery steps
   (`ExposureWithinTwoSteeringSteps` holds).
2. **The trust-boundary window is two steps of the steering recovery and depends
   on nothing else.** The two steps are the audit's finding and the fence. The
   exposure ends under weak fairness of those two steps (and of the re-finding
   after a repair) plus boot / restart. No fairness of the owner, the
   sequencer, the quiesce, the repair or any other recovery is needed
   (`steering-detach-exposure-ends`). Round 4 needed the owner's and the quiesce's fairness.
3. **UP-5: nothing observes the route while `serve` is up.** A root removal
   (or replacement) of the tagged route persists until the next boot
   (`steering-root-route-removal-persists`).

## Environment (round 4b)

| Item | Value |
|---|---|
| Host | Lima VM `overdrive`, `uname -r` = `7.0.0-34-generic`, 15 GiB, 8 vCPU |
| Quint / Apalache / TLC / JVM | 0.32.0 / 0.56.1 (build 70cdaf4) / TLC2 2.19 / `openjdk 21.0.12.1 2026-08-18 LTS` |
| Recorded run | `cargo xtask lima run -- cargo xtask quint check --subsystem guest-flow-owner --record --jobs 2`, run id **`332173-1791340295.607`**, 2026-10-07T02:31:35Z – 03:05:25Z, git head `d34f054ac` (dirty: this revision's specs) |
| Evidence | `specs/quint/guest-flow-owner/evidence/`; `cargo xtask quint verify-evidence --subsystem guest-flow-owner` → `ok guest-flow-owner: evidence matches the specs; every outcome met expect` |
| Round-4 evidence | preserved in `docs/feature/netns-density-295/design/quint-guest-flow-owner-evidence-r4/` (untracked in git, so `--record` would otherwise have erased it) |

Final spec SHA-256:

```text
6d2b243273eae76428fd88c72fce368c9bb1b46f91179d2438a8c6c16f463d54  cid_lease.qnt
6cc2220a27325da8f71240011843ff48ebea82a4972738f44c51ecd3e32d837d  owner_flows.qnt
4fc2f5fe7094ccd88df107d4787589596d9b109d7123e8de30b78dbb9b7296c3  steering.qnt
b46148378362a12ce7778bbd252841a96082001cb78b67868bbb1af5132ec557  udp_slots.qnt
3830d8282884f1e021eb34e9e9f85dd45fc674fd219222ce127313d450187009  hazard/cid_lease_hazards.qnt
3bb69466801e4a666d72baf6b9e917ca5588a01d7da3b89d3ec57e40df45e472  hazard/owner_flows_hazards.qnt
ef49cbd9b7b0ae9b3e989c4f75cee2b22d68b3c247b239fe33bf82a1c2fb0886  hazard/steering_hazards.qnt
edef8104826e53b50d2db4ef5a9efcf36a51b404f50e9c504c3ad3b9bbe98689  hazard/udp_slots_hazards.qnt
0483c04a8e8fa3baa17b91aedd3d0db5594dad2a6e3876254caf63a6b9156d43  checks.toml
```

Modules A and B and their hazards are byte-identical to round 4 (all 48 of
their checks re-ran and matched).

## What changed in the specification (round 4b)

**Module D (`steering.qnt`)** now models D8a-FENCE:

- `route ∈ {absent, local, fence}`. `absent` appears only before the first
  convergence, after a root removal, or in the remove-not-fence hazard.
- Landing gains two rules. The fence refuses every client: marked, unmarked,
  remote and leg-C-diverted (K-L6). With no tagged route the packet lands
  *elsewhere* (K-D4 restated).
- Boot: converge → verify + probe → `local`. A converge or verify/probe failure
  fences and refuses.
- The steering recovery is split into one step each, with a crash possible
  between any two:
  - audit finding → **fence** → quiesce → converge → verify + probe → `local`
    → clean audit → restore;
  - a failed converge, verify or probe fences and the attempt fails;
  - after the repair, damage again → finding (attempt + 1) → fence → repair
    again under the same hold;
  - with the attempt budget (`MAXATT` = 2) spent → **fail-stop** (`serve`
    exits; the fence stays).
- The repair can fail a bounded number of times, or forever
  (`steer_repairForever`).
- New environment fault: root removes or replaces the route (`ENV_ROOT_ROUTE`).
- History variables, used only by properties:
  - `estab` (Overdrive has set the route);
  - `verSince` (a verify returned Ok after the last fence);
  - `tamper` (a root detach, or a root `local`, after the last fence);
  - `win` (steering-recovery steps taken while exposed, capped at 2);
  - `stop` (why `serve` is down).
- Leg-S and probes are present whenever `serve` is up, quiesced or not (UP-1 as
  pinned).

**Module C (`cid_lease.qnt`)**: invariant 5 is restated per revision 9 item 5.
`RefusalEachClaimInUse` is added to `Safety`: `GuestCidsHeldElsewhere` only
after every free offset's claim in this call returned `InUse`, each at its own
moment (`NoSkipWhileClaimable`); a snapshot is not required
(`RefusalSnapshotConsistent` stays a documented violation). Teeth:
`cid-lease-hazard-remember-refusal`.

## Properties (D8a-FENCE), as checked

| Property | Statement | Where it holds |
|---|---|---|
| `ExposureOnlyInTamperWindow` | a wildcard host service is reached only after a root detach (or root `local` route) and before the next fence | `SafetyUnderDetach` (5,730,308 states); `steer_envRootRoute` |
| `ExposureWithinTwoSteeringSteps` | at most one steering-recovery step happens while exposed before the step that ends it (the fence) | `SafetyUnderDetach` |
| `FencedWhileSteeringRecovers` | from the fence until the repair has verified and put `local` back, the route is the fence | `Safety`, `SafetyUnderDetach` |
| `SteeringHoldNoWildcard` | while the steering recovery holds and has not re-verified, no host service is reached | `SafetyUnderDetach` |
| `SteeringQuiesceNeverWidens` | the steering recovery's quiesce never closes open listeners while the prefix is `local` and unverified | `SafetyUnderDetach` |
| `RouteLocalOnlyVerified` | the route is `local` only if a verify returned Ok after the last fence (Overdrive's steps; root `local` is the trust boundary) | `Safety`, `SafetyUnderDetach` |
| `ConvergeNeedsSteering` | converge_shared sets `local` only after its own verify + probe | `Safety`, `SafetyUnderDetach` |
| `NoElsewhere` | once the route exists, prefix traffic is never routed off the node | `Safety`, `SafetyUnderDetach` |
| `FenceHoldsWhileStopped`, `NothingWhileStopped` | while `serve` is down after a boot refusal or a fail-stop the prefix is fenced and every connection refused | `Safety`, `SafetyUnderDetach` |
| `DetachExposureEnds` | the exposure ends under WF(audit finding, re-finding, fence) + WF(boot, restart) only | `steer_detachLive` (58,284 states) |
| `SteeringRecoveryResolves` | from the audit's finding: eventually `local` + attached + recovery idle, or `serve` stopped with the fence; under SF(steering recovery's own steps), WF(owner), boot / restart, ◇□up | `steer_detachLive`, `steer_repairForever` (repair fails forever → fail-stop) |
| `UnmarkedOnLegC` under flush | documented violation, unchanged (revision 9 item 7): an unmarked host-local client reaches the steered intake of a serving port, never a host service (`NoWildcardReached` holds), never a non-serving port (`ReachOnlyServing` holds); ended by the IpRules recovery (`LegCBypassEnds` holds) | `steer_ok` |

## Bounds and state counts (module D, round 4b)

| Instance | Faults / bounds | Distinct states (TLC, exhaustive) |
|---|---|---|
| `steer_ok` | crash, foreign flush, root entry deletion, listener-task exit, steer failing forever, 1 bind, 1 boot converge/verify failure; 2 ports (1 declared), 2 generations | 47,364 |
| `steer_noflush` / `steer_two` | as round 4 | 4,359 / 4,165 |
| `steer_envDetach` | root link detach + crash, flush, listener exit, steer forever, 1 bind, 1 boot failure, 1 repair failure, `MAXATT` 2 (root entry deletion left to `steer_ok`; it does not touch the route) | 5,730,308 (395 s) |
| `steer_envRootRoute` | root route removal / replacement + crash, flush, entry deletion, listener exit | 276,258 |
| `steer_detachLive` / `steer_repairForever` | 1 port, 1 generation, unbounded detach, listener exit, crash; 1 repair failure / repair failing forever | 58,284 / 32,178 |
| `steer_ok` Apalache | 12 steps recorded (126 s); 16 steps passed unrecorded in 1,190 s (run `319084-1791337024.407`) | — |

## Every check of modules C and D — expected vs recorded (run `332173-1791340295.607`)

Modules A and B: 48 checks, unchanged, all as round 4 (see the table above and
`evidence/summary.md`).

| Check | Instance | Property | Backend / bound | Expected | Recorded | s |
|---|---|---|---|---|---|---|
| `cid-lease-safety-tlc` | `lease_ok` | `Safety` | tlc / exhaustive | holds | holds | 7 |
| `cid-lease-three-safety-tlc` | `lease_three` | `Safety` | tlc / exhaustive | holds | holds | 16 |
| `cid-lease-live-safety-tlc` | `lease_live` | `Safety` | tlc / exhaustive | holds | holds | 5 |
| `cid-lease-eventually-placed` | `lease_live` | `EventuallyPlaced` | tlc / exhaustive | holds | holds | 7 |
| `cid-lease-witness-skip` | `lease_ok` | `WitnessNoSkip` | tlc / exhaustive | violation | violation | 4 |
| `cid-lease-witness-orphan-skip` | `lease_ok` | `WitnessNoOrphanSkip` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-witness-held-elsewhere` | `lease_ok` | `WitnessNoHeldElsewhere` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-witness-claim-error` | `lease_ok` | `WitnessNoClaimErr` | tlc / exhaustive | violation | violation | 4 |
| `cid-lease-witness-running` | `lease_ok` | `WitnessNoRunning` | tlc / exhaustive | violation | violation | 4 |
| `cid-lease-refusal-snapshot` | `lease_ok` | `RefusalSnapshotConsistent` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-hazard-check-then-claim` | `lease_bugCheckThenClaim` | `NoLaunchClash` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-hazard-check-then-claim-held` | `lease_bugCheckThenClaim` | `ClaimHeld` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-hazard-release-before-vmm` | `lease_bugReleaseBeforeVmm` | `NoLaunchClash` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-hazard-release-before-vmm-held` | `lease_bugReleaseBeforeVmm` | `ClaimHeld` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-hazard-remember-inuse` | `lease_bugRemember` | `NoSkipWhileClaimable` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-hazard-remember-refusal` | `lease_bugRemember` | `RefusalEachClaimInUse` | tlc / exhaustive | violation | violation | 5 |
| `cid-lease-hazard-remember-inuse-placed` | `lease_live_bugRemember` | `EventuallyPlaced` | tlc / exhaustive | violation | violation | 8 |
| `steering-safety-tlc` | `steer_ok` | `Safety` | tlc / exhaustive | holds | holds | 9 |
| `steering-flush-no-wildcard` | `steer_ok` | `NoWildcardReached` | tlc / exhaustive | holds | holds | 9 |
| `steering-reach-only-serving` | `steer_ok` | `ReachOnlyServing` | tlc / exhaustive | holds | holds | 9 |
| `steering-noflush-safety-tlc` | `steer_noflush` | `Safety` | tlc / exhaustive | holds | holds | 6 |
| `steering-noflush-unmarked-on-legc` | `steer_noflush` | `UnmarkedOnLegC` | tlc / exhaustive | holds | holds | 6 |
| `steering-two-safety-tlc` | `steer_two` | `Safety` | tlc / exhaustive | holds | holds | 6 |
| `steering-safety-apalache` | `steer_ok` | `Safety` | apalache / max_steps=12 | holds | holds | 126 |
| `steering-flush-unmarked-bypass` | `steer_ok` | `UnmarkedOnLegC` | tlc / exhaustive | violation | violation | 6 |
| `steering-two-activation-completes` | `steer_two` | `ActivationCompletes` | tlc / exhaustive | holds | holds | 6 |
| `steering-serving-reachable` | `steer_live` | `ServingReachable` | tlc / exhaustive | holds | holds | 6 |
| `steering-boot-opens` | `steer_boot` | `BootOpens` | tlc / exhaustive | holds | holds | 6 |
| `steering-witness-steered` | `steer_ok` | `WitnessNoSteered` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-second-allocation` | `steer_ok` | `WitnessNoSecondAllocation` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-boot-refusal` | `steer_ok` | `WitnessNoBootRefusal` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-resteer` | `steer_ok` | `WitnessNoResteer` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-overlap` | `steer_ok` | `WitnessNoOverlap` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-still-quiesced` | `steer_ok` | `WitnessNoStillQuiesced` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-crash-with-route` | `steer_ok` | `WitnessNoCrashWithRoute` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-legc` | `steer_ok` | `WitnessNoLegC` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-fenced-while-down` | `steer_ok` | `WitnessNoRefusedWhileDown` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-activation-err` | `steer_two` | `WitnessNoActivationErr` | tlc / exhaustive | violation | violation | 6 |
| `steering-hazard-element-set` | `steer_bugElementSet` | `ReachOnlyServing` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-element-set-flush` | `steer_bugElementSetFlush` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-element-close-first` | `steer_bugElementCloseFirst` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 12 |
| `steering-hazard-steering-in-nft` | `steer_bugSteeringInNft` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 133 |
| `steering-hazard-unpinned` | `steer_bugUnpinned` | `NothingDeliveredWhileDown` | tlc / exhaustive | violation | violation | 12 |
| `steering-hazard-nonloopback-assign` | `steer_bugNonloopback` | `RemoteNeverDirect` | tlc / exhaustive | violation | violation | 12 |
| `steering-hazard-entry-outlives-close` | `steer_bugEntryOutlives` | `EntryNamesServingListener` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-route-first` | `steer_bugRouteFirst` | `RouteImpliesSteering` | tlc / exhaustive | violation | violation | 12 |
| `steering-hazard-route-first-wildcard` | `steer_bugRouteFirst` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 12 |
| `steering-hazard-no-verify` | `steer_bugNoVerify` | `ConvergeNeedsSteering` | tlc / exhaustive | violation | violation | 12 |
| `steering-hazard-no-verify-wildcard` | `steer_bugNoVerify` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-rollback-leaves-listener` | `steer_bugRollback` | `ActivationAllOrNothing` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-rollback-leaves-listener-reach` | `steer_bugRollback` | `ReachOnlyServing` | tlc / exhaustive | violation | violation | 12 |
| `steering-hazard-act-ok-partial` | `steer_bugActOkPartial` | `ActivationAllOrNothing` | tlc / exhaustive | violation | violation | 12 |
| `steering-hazard-teardown-keeps-listeners` | `steer_bugTeardownKeeps` | `NoReleaseWithListener` | tlc / exhaustive | violation | violation | 12 |
| `steering-hazard-quiesce-keeps` | `steer_bugQuiesceKeeps` | `QuiescedClosesListeners` | tlc / exhaustive | violation | violation | 12 |
| `steering-hazard-quiesce-keeps-reach` | `steer_bugQuiesceKeeps` | `ReachOnlyServing` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-any-restore` | `steer_bugAnyRestore` | `NoReopenBeforeRepair` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-repair-without-hold` | `steer_bugRepairNoHold` | `NoRepairWithoutHold` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-quiesce-before-fence` | `steer_bugQuiesceBeforeFence` | `SteeringHoldNoWildcard` | tlc / exhaustive | violation | violation | 12 |
| `steering-hazard-quiesce-before-fence-window` | `steer_bugQuiesceBeforeFence` | `ExposureWithinTwoSteeringSteps` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-quiesce-before-fence-servable` | `steer_bugQuiesceBeforeFence` | `FencedWhileSteeringRecovers` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-quiesce-before-fence-widens` | `steer_bugQuiesceBeforeFence` | `SteeringQuiesceNeverWidens` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-remove-not-fence` | `steer_bugRemoveNotFence` | `NoElsewhere` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-remove-not-fence-down` | `steer_bugRemoveNotFence` | `FenceHoldsWhileStopped` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-no-fresh-verify` | `steer_bugNoFreshVerify` | `RouteLocalOnlyVerified` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-no-fresh-verify-ghost` | `steer_bugNoFreshVerify` | `ConvergeNeedsSteering` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-restore-lifts-fence` | `steer_bugRestoreLiftsFence` | `ExposureOnlyInTamperWindow` | tlc / exhaustive | violation | violation | 13 |
| `steering-hazard-restore-lifts-fence-route` | `steer_bugRestoreLiftsFence` | `RouteLocalOnlyVerified` | tlc / exhaustive | violation | violation | 13 |
| `steering-env-detach-safety` | `steer_envDetach` | `SafetyUnderDetach` | tlc / exhaustive | holds | holds | 395 |
| `steering-env-detach-wildcard` | `steer_envDetach` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 18 |
| `steering-env-detach-route-implies-steering` | `steer_envDetach` | `RouteImpliesSteering` | tlc / exhaustive | violation | violation | 17 |
| `steering-env-detach-down` | `steer_envDetach` | `NothingDeliveredWhileDown` | tlc / exhaustive | violation | violation | 17 |
| `steering-env-detach-remote-direct` | `steer_envDetach` | `RemoteNeverDirect` | tlc / exhaustive | violation | violation | 18 |
| `steering-env-detach-held-servable-wildcard` | `steer_envDetach` | `WitnessNoServableWildcardWhileHeld` | tlc / exhaustive | violation | violation | 20 |
| `steering-env-detach-other-hold-widens` | `steer_envDetach` | `QuiescenceNeverWidens` | tlc / exhaustive | violation | violation | 18 |
| `steering-witness-recovery-fence` | `steer_envDetach` | `WitnessNoRecoveryFence` | tlc / exhaustive | violation | violation | 19 |
| `steering-witness-one-step-exposure` | `steer_envDetach` | `WitnessNoOneStepExposure` | tlc / exhaustive | violation | violation | 17 |
| `steering-witness-fail-stop` | `steer_repairForever` | `WitnessNoFailStop` | tlc / exhaustive | violation | violation | 18 |
| `steering-detach-exposure-ends` | `steer_detachLive` | `DetachExposureEnds` | tlc / exhaustive | holds | holds | 28 |
| `steering-recovery-resolves` | `steer_detachLive` | `SteeringRecoveryResolves` | tlc / exhaustive | holds | holds | 34 |
| `steering-repair-forever-resolves` | `steer_repairForever` | `SteeringRecoveryResolves` | tlc / exhaustive | holds | holds | 27 |
| `steering-hazard-any-restore-exposure` | `steer_detachLiveAnyRestore` | `DetachExposureEnds` | tlc / exhaustive | violation | violation | 22 |
| `steering-hazard-any-restore-resolves` | `steer_detachLiveAnyRestore` | `SteeringRecoveryResolves` | tlc / exhaustive | violation | violation | 21 |
| `steering-hazard-no-recovery-fairness` | `steer_detachLiveNoFairness` | `DetachExposureEndsNoRecoveryFairness` | tlc / exhaustive | violation | violation | 20 |
| `steering-flush-legc-bypass-ends` | `steer_flushLive` | `LegCBypassEnds` | tlc / exhaustive | holds | holds | 18 |
| `steering-steer-forever-unreachable` | `steer_steerForever` | `ServingReachable` | tlc / exhaustive | violation | violation | 18 |
| `steering-env-root-route-window` | `steer_envRootRoute` | `ExposureOnlyInTamperWindow` | tlc / exhaustive | holds | holds | 43 |
| `steering-env-root-route-reach-only-serving` | `steer_envRootRoute` | `ReachOnlyServing` | tlc / exhaustive | holds | holds | 37 |
| `steering-env-root-route-elsewhere` | `steer_envRootRoute` | `NoElsewhere` | tlc / exhaustive | violation | violation | 17 |
| `steering-env-root-route-unverified-local` | `steer_envRootRoute` | `RouteLocalOnlyVerified` | tlc / exhaustive | violation | violation | 15 |
| `steering-env-root-route-down` | `steer_envRootRoute` | `FenceHoldsWhileStopped` | tlc / exhaustive | violation | violation | 13 |
| `steering-root-route-removal-persists` | `steer_rootRouteLive` | `RouteTamperEnds` | tlc / exhaustive | violation | violation | 13 |

## First-pass results and the modelling fixes (round 4b)

The runner keeps only the final recorded run in `evidence/`. Earlier passes are
condensed here; their logs are under `target/quint/guest-flow-owner/` until the
next run. No expectation was changed after a run. Each fix below is a
modelling or harness error stated exactly; none weakened a property.

| Pass | What happened | Exact cause | Fix |
|---|---|---|---|
| 1 (`--ci`, unrecorded) | 71 of 126 tool errors (every TLC check on module D) | `verSince' = verSince or link` and `tamper' = tamper or rv == "local"` parse as `(x' = …) or …`, so a successor left the variable unassigned (TLC: "variable is not assigned: verSince") | parenthesised both right-hand sides |
| 2 (`--ci`, unrecorded) | 6 design checks on `steer_envDetach` tool errors after 46–260 s; `steering-hazard-any-restore-exposure` timed out at 900 s; everything else matched | (a) kernel OOM-killer killed TLC (`dmesg`: "Out of memory: Killed process … (java)"): three TLC runs of a 3.7 M-state instance in parallel on a 15 GiB VM; (b) `win` and `att` were unbounded in the `BUG_ANY_RESTORE` instance (the bug resets the recovery without resetting them; the TLC queue stayed constant while states grew) | (a) dropped root entry deletion from `steer_envDetach` and dropped six named-view checks that are conjuncts of `SafetyUnderDetach` (one check keeps them all); (b) `win` capped at 2 (the invariant distinguishes ≤ 1 from ≥ 2 only), `att` reset when the steering recovery returns to idle by a restore (per-recovery budget, UP-7) |
| trace review | `WitnessNoServableWildcardOtherHold` was violated, but its trace showed no widening: the port had no intake listener yet when the IpRules quiesce ran | the predicate was too weak for the claim | replaced by the ghost `QuiescenceNeverWidens` (a first-holder quiesce closes open listeners while `local` and unverified); its trace now shows the bound listener closing (F-4b-1) |
| trace review | the hazard check `quiesce-before-fence-widens` on `QuiescenceNeverWidens` reached the design path (IpRules quiesce) instead of the rule | wrong property for the teeth | added `SteeringQuiesceNeverWidens` (in `SafetyUnderDetach`); the hazard's trace is now exactly round-4 observation 1 |
| record 1 (`319207-1791338254.507`) | 125 of 126; `udp-slots-safety-apalache` (module B, byte-identical to round 4) timed out at 1,800 s | Z3 contention with two other jobs (443 s alone) | re-recorded with `--jobs 2` → 126 of 126 (`332173-1791340295.607`); `steering-safety-apalache` recorded at 12 steps to fit |

## Counterexamples against design rules (round 4b)

**None.** Every `holds` check of revision 9 (items 1–7) holds.

## Teeth — D8a-FENCE (all `violation`, as predicted)

| Rule | Hazard instance | Broken property | Counterexample in short |
|---|---|---|---|
| Fence before the steering recovery's quiesce | `steer_bugQuiesceBeforeFence` | `SteeringQuiesceNeverWidens`, `SteeringHoldNoWildcard`, `ExposureWithinTwoSteeringSteps`, `FencedWhileSteeringRecovers` | boot → workload Active, port 1 listening, intake bound → **root detach** → audit finds → recovery **quiesces** (listener closed) → marked client to port 1 lands on the wildcard (round-4 observation 1) |
| Fence, never removal | `steer_bugRemoveNotFence` | `NoElsewhere`, `FenceHoldsWhileStopped` | first boot: rules, converge **fails** → route *removed* (absent), startup refused → restart → a marked client's packet to the prefix leaves by the default route; while stopped the prefix is not fenced |
| `local` only after a fresh verify | `steer_bugNoFreshVerify` | `RouteLocalOnlyVerified`, `ConvergeNeedsSteering` | detach → finding → fence → quiesce → converge → **`local` without verify + probe** |
| Only a verifying converge_shared lifts the fence | `steer_bugRestoreLiftsFence` | `ExposureOnlyInTamperWindow`, `RouteLocalOnlyVerified` | flush + detach → IpRules quiesces → steering found → IpRules repaired → steering **fenced** → IpRules **restore reopens and lifts the fence** → `local` with the link detached → wildcard, with no root action since the fence |
| Round-4 teeth | all `steer_bug*` of round 4 | as round 4 | unchanged, all `violation` |
| D16-CLAIM: refusal only after asking the kernel for every free offset | `lease_bugRemember` | `RefusalEachClaimInUse` | an offset remembered as `InUse` is not claimed; refusal returned without asking |

## Observations (round 4b)

1. **F-4b-1 — quiescence by another holder widens an undetected detach**
   (D8a-FENCE, D8a-HOLD, M-7, UP-1 as pinned). Check
   `steering-env-detach-other-hold-widens` (violation). Minimal trace:
   - boot opens (converge, verify Ok, `local`);
   - foreign nft flush (IpRules damaged); provision, session, activate;
   - **root detaches the link**;
   - the guest reports port 1 listening and the intake listener binds (a
     marked client to port 1 reaches the intake by its specific bind, K-D1);
   - **the IpRules recovery quiesces first** (it is the first holder: every
     intake listener closes);
   - a marked client (leg-S / probe) to port 1 lands on the wildcard host
     service.

   The text of `fence_guest_prefix` says the fence comes first "so closing the
   intake listeners never widens the exposure of unverified steering". The
   model shows that holds for the steering recovery's own quiesce
   (`SteeringQuiesceNeverWidens` holds), not for a quiesce by another holder
   before the audit has found the detach. It stays inside the trust-boundary
   window: the steering audit's finding and the fence end it
   (`ExposureWithinTwoSteeringSteps` holds; the fence does not wait for the
   other hold). Options for the architect (not chosen here):
   - (a) state it as part of the window;
   - (b) the first holder's quiesce runs `GuestPrefixSteering::verify`
     (read-only) and fences when it fails;
   - (c) every first-holder quiesce fences, since forwarding is down anyway,
     and only converge_shared's verify-then-`local` lifts it. Each recovery's
     repair is converge_shared already.
2. **A fresh root detach inside the steering hold after the repair's verify**
   exposes listening ports until the re-finding and fence
   (`steering-env-detach-held-servable-wildcard`, violation, documented).
   Trace:
   - detach → finding → fence → quiesce → converge → verify Ok;
   - **root detaches again**;
   - `local` (the route step does not re-verify);
   - the hold has every listener closed → wildcard.

   This is the same verify → route window as round-4 observation 3, now inside
   the hold. Re-closed by the next audit finding and the fence (two steps).
3. **Window measured in steps.** For a root link detach while `serve` is up,
   the exposure lasts at most 2 steps of the steering recovery: the audit's
   finding and the fence (`ExposureWithinTwoSteeringSteps` holds). One step
   does not suffice (`steering-witness-one-step-exposure`: still exposed after
   the finding). No other Overdrive step is a precondition: not the owner, the
   per-allocation sequencer, the quiesce, the repair, or another recovery
   (`steering-detach-exposure-ends` holds with fairness on the finding and the
   fence only). In time: at most one audit period plus one route replace.
   After the fence, the recovery resolves to `local` behind verified steering,
   or to fail-stop with the fence, under fairness of its own steps
   (`steering-recovery-resolves`), also when the repair fails forever
   (`steering-repair-forever-resolves`, witness `steering-witness-fail-stop`).
4. **Root route tampering** (trust boundary). `ExposureOnlyInTamperWindow` and
   `ReachOnlyServing` still hold. Removal routes prefix traffic elsewhere and
   is not repaired while `serve` is up (UP-5). Root `local` over the fence
   violates `RouteLocalOnlyVerified`; root route changes while stopped break
   `FenceHoldsWhileStopped`. All of these are documented trust-boundary
   behaviour.
5. **Nft flush exposure unchanged** (revision 9 item 7, round-4 observation 4).
   Under flush, an unmarked host-local client reaches the steered intake of a
   serving port (`steering-flush-unmarked-bypass`). It never reaches a host
   service (`steering-flush-no-wildcard`) or a non-serving port
   (`steering-reach-only-serving`), and the IpRules recovery ends it
   (`steering-flush-legc-bypass-ends`).

## Underspecified points (round 4b)

- **UP-1, UP-2, UP-4: closed** by revision 9 items 3–5. They are modelled as
  pinned: leg-S and probes dial at any time; a repair can fail, possibly
  forever, then fail-stop with the fence; the D16-CLAIM wording.
- **UP-3** (§ *Formal protocol model* text) is still open for the architect.
  It should list D8a-FENCE, K-L6, and the round-4b property set.
- **UP-5 (D8a-FENCE, D8a-ROUTE): does `audit_shared` observe the tagged route?**
  Nothing in the text checks that the route is `local` when the steering
  verifies, or the fence otherwise. Root removal makes prefix traffic leave the
  node, and root replacing `local` by the fence makes the prefix refuse; both
  persist until the next boot (`steering-root-route-removal-persists`). Pin
  either that the audit treats a route not of the expected form as damage of
  `GuestPrefixSteering` (fence-then-repair), or that this is the trust boundary
  with that duration.
- **UP-6 (D8a-FENCE, ADR-0124): does a re-attempt re-fence first?** Modelled
  as yes: damage found after the repair leads to a finding, then the fence,
  then the repair again. The text says only that the recovery's first step is
  the fence. If a re-attempt did not re-fence, a re-detach in state `repaired`
  would stay exposed until the next converge (still bounded by the recovery's
  own steps, but no longer two steps).
- **UP-7 (ADR-0124 budget):** the model assumes the following, none of which
  the text states:
  - the attempt budget is per recovery episode: reset when the steering
    recovery returns to idle, and by a crash;
  - an audit failure after a completed repair counts as a failed attempt.
- **UP-8 (converge_shared as every recovery's repair):** converge_shared also
  converges the steering. Two questions are open:
  - Does an IpRules or FlowListeners repair fail, and fence, when the steering
    does not verify? Its attempt would then count against that recovery's
    budget and could fail-stop `serve` for a steering fault first.
  - Can that repair lift the fence while the steering recovery is fenced but
    not yet holding? It does so through the same verify-then-`local` rule, so
    this is safe.

  The model gives the steering repair only to the steering recovery.

## Assumptions and their discharging items (round 4b)

| ID | Assumption | Discharged by |
|---|---|---|
| K-A1 | Peer-CID attribution is authentic | V-10 (bind level, P-17) |
| K-A2 | Released vhost device's connections reset before CID reuse | V-22 |
| K-B1 | Closing `T_g` / `Q_g` is observed as the association's end | V-22 |
| K-A3 | Socket installed before request read → verdict drops it | P-23 (+ R5-9) |
| K-A4 | Host close of `V_h` reaches the guest, accept queue included | V-24 |
| A-FID | Host flow ids not reused within a pairing window | `FlowId` contract (design property) |
| K-B2 | `sock_release` reports, or the audit finds orphans | R5-14 |
| K-B3 | Emptying / recreating cells discards parked frames | V-5(c) |
| K-C1 | Duplicate CID refused at creation; free at last close | P-16 |
| K-C2 | Foreign vhost users take / release CIDs at any time | Environment, modelled as actions |
| K-C3 | Claim exclusive while referenced, survives VMM handoff, released at last close | V-25 |
| K-D1, K-D2 | Delivery with `local` and no lookup decision; nft divert / reject / drop | V-19 |
| K-D3 | nft rules / route persist across a `serve` exit; batches atomic; foreign deletion a fault | V-19 (persistence), V-23 |
| K-D4 (restated) | With no tagged route, the prefix is not local: packets are routed elsewhere unless the nft rules divert / reject first | V-19 |
| K-D5 | Closing a listener resets its accept queue | V-13 |
| K-L1 – K-L5 | `sk_lookup` order, TPROXY bypass, removal at close, pinned with no process, ingress ifindex | V-26 |
| **K-L6 (new)** | A tagged `prohibit` route for the prefix in the local table refuses host-local connects at once and refuses remote packets, leg-C TPROXY-diverted ones included, without forwarding them; `local` ↔ `prohibit` replace is atomic | **V-26**. Note for V-26: the leg-C case depends on the local table (rule priority 0) being consulted before the TPROXY fwmark rule's table. V-26 should exercise a diverted remote SYN against the fence, not only a host-local connect. |

**No assumption lacks a discharging item.**

## CI flags (round 4b)

120 of 126 checks carry `ci = true`. Not CI:
- `owner-flows-safety-apalache` (187 s);
- `owner-flows-safety-apalache-deep` (450 s);
- `owner-flows-hazard-paired-no-recheck` (109 s, kept as in round 4);
- `udp-slots-safety-apalache` (443 s);
- `steering-safety-apalache` (126 s at 12 steps);
- `steering-env-detach-safety` (395 s).

Three CI checks show 132–139 s in the recording only because their first
attempt hit the runner's start-up bound and was retried (`attempts` = 2):
`owner-flows-hazard-install-early`, `owner-flows-hazard-no-abort-on-loss`,
`steering-hazard-steering-in-nft`. Their single attempts take 13–15 s, as in
round 4.

# Round 4c — item 7 of § *Revision 9 decisions* (F-4b-1 fix, UP-5, UP-6, UP-7, UP-8)

Appended 2026-10-07. Rounds 4 and 4b above are unchanged. The round-4b recorded
run (`332173-1791340295.607`) is replaced by this round's `--record`; it is
preserved verbatim in
`docs/feature/netns-density-295/design/quint-guest-flow-owner-evidence-r4b/`.

## Predictions (written before the first run of round 4c)

Hypothesis: with item 7 (first hold of any holder: verify, fence if unverified,
then close), UP-5 (`LocalRoute` audited every period and repaired by its own
recovery), UP-6/UP-7 (every steering attempt begins with the fence; a failed
post-repair audit is a failed attempt) and UP-8 (`converge_shared` is total, so
every recovery's repair re-establishes the steering before `local`), no Overdrive
quiesce widens an undetected detach, and a root route removal or root fence ends
under fairness of the `LocalRoute` recovery alone.

Predicted outcomes (each is the `expect` in `checks.toml`, set before running):

- Item 7: `QuiescenceNeverWidens` holds under root detach (it is now a conjunct
  of `SafetyUnderDetach`); the F-4b-1 trace is reached only in
  `steer_bugQuiesceNoFence`. A first-hold fence that fails (it "does not stop
  the quiesce") reaches the widening (`steering-env-fence-fail-widens`), still
  inside the tamper window.
- Item 7 read for every Overdrive step: a per-allocation listener close
  (`ListenState` off, session loss, teardown, bring-up failure) during a detach
  the audit has not found yet widens the exposure (`CloseNeverWidens`
  violated) — not covered by item 7, which pins only the quiesce. A route step
  after a root detach between verify and route widens it (`RouteNeverWidens`
  violated; item 8 documents this).
- UP-8 has consequences for the steering recovery's own guarantees:
  - another recovery's `converge_shared` verifies the steering and lifts the
    steering recovery's fence while that recovery is fenced or holding
    (`FencedWhileSteeringRecovers` violated; restated as
    `FencedOrVerifiedWhileSteeringRecovers`, which holds);
  - after such a lift a fresh root detach exposes during the steering hold
    (`SteeringHoldNoWildcard` violated), and can last three steps of the
    steering recovery (its in-flight route step, the finding, the fence):
    `ExposureWithinTwoSteeringSteps` violated, `ExposureWithinThreeSteeringSteps`
    holds;
  - the exposure no longer ends under fairness of the finding and the fence
    alone when another recovery can lift the fence
    (`steering-detach-exposure-ends-find-fence` violated on `steer_detachLive`),
    but still does with no other recovery (`steer_detachLiveSolo`, holds); with
    fairness of all steering-recovery steps and of in-flight `converge_shared`
    calls returning it ends (`DetachExposureEnds` holds).
- UP-9 (not stated by the design; assumed by the design instances): one
  `converge_shared` call and a fence exclude one another. Dropping it, a fence
  of the steering recovery between another recovery's verify and its route
  step is undone by that stale route step over a detached link
  (`steer_envDetachConcurrent`: `ExposureOnlyInTamperWindow` and
  `RouteLocalOnlyVerified` violated; `ConvergeNeedsSteering` holds, since each
  call did verify).
- UP-5: under root route removal / fence / `local`, `SafetyUnderRouteTamper`
  holds, with prefix traffic leaving the node only between the root removal and
  the next Overdrive route write, within four steps of the `LocalRoute`
  recovery (finding + quiesce, converge, verify + probe, route); a root fence
  over verified steering ends within the same four steps. Both end under
  fairness of the `LocalRoute` recovery (`RouteTamperEnds`, `RootFenceEnds`
  hold); with the audit ignoring the route form both are violated.
- UP-6 teeth: re-attempt without the fence breaks
  `DetachExposureEndsFindFence` on `steer_detachLiveSolo`. UP-7 teeth: redo not
  counted breaks `SteeringRecoveryResolves` (root re-detaching forever).
  UP-8 teeth: a partial converge breaks `ConvergeNeedsSteering`,
  `ExposureOnlyInTamperWindow` and `RouteLocalOnlyVerified`.
- D8a-FENCE's order (fence before the steering recovery's own quiesce) is
  predicted to have no teeth left: item 7's first-hold verify + fence covers it
  (`steering-hazard-quiesce-before-fence-covered` holds); only both off (revision
  9 as first written) widen.
- Every other round-4b property and teeth keeps its verdict (modules A–C are
  byte-identical except the shared header of `checks.toml`).

### Predictions added after the first CI pass (written before these checks ran)

The first CI pass (run `345630-1791343858.019`, unrecorded) contradicted three
predictions (`steering-detach-exposure-ends`, `steering-recovery-resolves`,
`steering-repair-forever-resolves`; see *Counterexamples* below). Their
expectations are not changed. Three checks are added:

- `DetachExposureEndsWithOwner` (`steer_detachLive`): holds. Hypothesis: the
  only extra precondition is the activation item the steering recovery's quiesce
  waits behind. Falsification: a fair run that still keeps the exposure.
- `SteeringRecoveryResolvesAllRecoveries` on `steer_detachLive` and on
  `steer_repairForever`: holds. Hypothesis: the steering recovery's outcome
  ("`local` again", or stopped) now depends on every recovery whose failed
  `converge_shared` can fence. Falsification: a run fair to every recovery that
  never resolves.

Instances resized after out-of-memory kills (three 8 GiB TLC heaps on a 15 GiB
VM) and two 900 s timeouts; every verdict on them is predicted unchanged:
`steer_envDetach` (now one declared port, MAXGEN 1, crash, flush, detach, one boot
converge failure, one repair failure; no listener-task exit, no bind failure, no
steer failure), `steer_envRootRoute` (one port, MAXGEN 1, no entry deletion, no
listener-task exit), `steer_bugQuiesceBeforeFence(NoVerify)` (one port, MAXGEN 1,
flush + detach + crash), `steer_detachLiveAnyRestore` (no crash).
`steer_envDetachSmall` is dropped (`steer_envDetach` now has its shape).

The `DetachExposureEndsWithOwner` run on `steer_detachLive` held (9,820,292
states with the fairness product, 1,264 s, unrecorded). For cost the three added
checks run on crash-free copies (`steer_detachLiveNoCrash`,
`steer_repairForeverNoCrash`); predicted: all three hold.

`steer_envDetachConcurrent` and `steer_envFenceFail` lose crash (cost; each
targets one interleaving); predicted verdicts unchanged.

## Verdict (round 4c)

Recorded run **`440354-1791359873.950`** (`--jobs 1`, 2026-10-07T07:57:53Z –
10:45:50Z, git head `d34f054ac`, dirty: this revision's specs): **151 checks;
145 match their expected verdict (31 properties hold, 114 expected violations
found); 6 do not.** No tool errors or timeouts in the record. `cargo xtask quint
verify-evidence --subsystem guest-flow-owner` **fails**, on exactly these six
outcome mismatches (the spec and `checks.toml` hashes match):

| Check | Expected | Recorded | What it shows |
|---|---|---|---|
| `steering-detach-exposure-ends` | holds | violation | F-4c-2: the detach window's end needs the owner (an activation item) |
| `steering-recovery-resolves` | holds | violation | F-4c-3: another recovery's failed `converge_shared` fences the prefix; the steering recovery cannot lift it |
| `steering-repair-forever-resolves` | holds | violation | F-4c-3 (same, the repair failing forever) |
| `steering-recovery-resolves-all-recoveries` | holds | violation | F-4c-4: root detaching repeatedly + overlapping episodes, never resolved, never fail-stopped |
| `steering-repair-forever-resolves-all-recoveries` | holds | violation | F-4c-4 (same) |
| `steering-hazard-any-restore-exposure` | violation | holds | D8a-HOLD's any-restore defect no longer prolongs the detach exposure: a FlowListeners restore follows its own total `converge_shared`, which re-attaches the steering (UP-8). The rule keeps teeth on `NoReopenBeforeRepair` and `SteeringRecoveryResolves` (`steering-hazard-any-restore`, `-any-restore-resolves`: violation) |

The expectations of these six were written before their first run and are not
changed. The architect decides: change the design, or accept the behaviour and
change the expectation with that decision on record.

Item 7 does what it says for the quiesce:
- `QuiescenceNeverWidens` holds under root detach (in `SafetyUnderDetach`,
  14,743,005 states).
- The F-4b-1 trace is reached only in `steer_bugQuiesceNoFence`.

Item 7 does **not** make every Overdrive step non-widening (F-4c-1). UP-5 works:
a root route removal or root fence ends within four steps of the `LocalRoute`
recovery, under that recovery's fairness alone. UP-8, as written, weakens three
guarantees round 4b had established for the steering recovery (F-4c-2 to
F-4c-4). The design also leaves one ordering unstated (UP-9). Without it, a
fresh fence can be undone by a stale route step (F-4c-5).

## Environment (round 4c)

| Item | Value |
|---|---|
| Host | Lima VM `overdrive`, `uname -r` = `7.0.0-34-generic`, 15 GiB, 8 vCPU |
| Quint / Apalache / TLC / JVM | 0.32.0 / 0.56.1 (build 70cdaf4) / TLC2 2.19 / `openjdk 21.0.12.1 2026-08-18 LTS` |
| Recorded run | `cargo xtask lima run -- cargo xtask quint check --subsystem guest-flow-owner --record --jobs 1`, run id **`440354-1791359873.950`** |
| Evidence | `specs/quint/guest-flow-owner/evidence/` (`failures/` holds the six mismatch logs) |
| Earlier evidence | round 4b: `quint-guest-flow-owner-evidence-r4b/`; this round's first recording (`--jobs 2`, run `424146-1791353069.285`: 142 of 151 matched, the same six mismatches plus four tool failures under memory/CPU contention — one OOM kill, one Apalache start-up failure, two 1,800 s timeouts of checks that finish alone): `quint-guest-flow-owner-evidence-r4c-jobs2/` |

Final spec SHA-256 (modules A–C and their hazards byte-identical to round 4b):

```text
6d2b243273eae76428fd88c72fce368c9bb1b46f91179d2438a8c6c16f463d54  cid_lease.qnt
6cc2220a27325da8f71240011843ff48ebea82a4972738f44c51ecd3e32d837d  owner_flows.qnt
c56b567c25b43608579e638935d3ebb55ce5081b33743b32b1c81e753a833fa2  steering.qnt
b46148378362a12ce7778bbd252841a96082001cb78b67868bbb1af5132ec557  udp_slots.qnt
3830d8282884f1e021eb34e9e9f85dd45fc674fd219222ce127313d450187009  hazard/cid_lease_hazards.qnt
3bb69466801e4a666d72baf6b9e917ca5588a01d7da3b89d3ec57e40df45e472  hazard/owner_flows_hazards.qnt
7d1a7ddbbfb973ebcfcc1e0e1105789613de0d1388f9b508889c3ab5b7fc40cd  hazard/steering_hazards.qnt
edef8104826e53b50d2db4ef5a9efcf36a51b404f50e9c504c3ad3b9bbe98689  hazard/udp_slots_hazards.qnt
caaf014a5dbcaaff6fc1c73ddcac1b68278f7f504514e5c6a3f8dd923fe244cc  checks.toml
```

## What changed in the specification (round 4c)

Module D (`steering.qnt`) only:

- **Item 7 (F-4b-1 fix).** `quiesceTo`, first holder: `verify` (link
  attached?) → if unverified, fence (as `fence_guest_prefix`) → close every
  intake listener. A fence failure there (`FENCE_FAILURES`, new fault) is
  reported and the quiesce goes on.
- **Four recoveries.** `IpRules`, `FlowListeners`, `LocalRoute` (new) and
  `GuestPrefixSteering`.
  - The three non-steering ones run: finding + quiesce → repair → clean audit
    → restore. Their repair is the component fix (rules / listener task) plus
    the total `converge_shared` (UP-8): converge → verify + probe → `local`.
    A failure fences and fails the attempt.
  - The steering recovery runs: finding → fence → quiesce → `converge_shared`
    → restore.
  - Every attempt of the steering recovery begins with the fence (UP-6). A
    fence failure there fails the attempt.
  - A damaged-again audit after a repair is a failed attempt for every
    recovery (UP-7).
  - Each recovery has a per-episode budget, then fail-stop.
- **UP-5.** `LocalRoute` is damaged iff the route is absent / different, or
  is the fence while the steering verifies and no steering recovery episode
  stands. Root may now remove, replace, fence or set `local` on the route.
- **UP-9 assumption (`OWNER_SERIAL`).** One `converge_shared` call
  (converge … route step) and a fence never interleave. Not stated by the
  design; `steer_envDetachConcurrent` drops it.
- **History variables.**
  - `tamper` is reset also by an Ok verify, which makes
    `ExposureOnlyInTamperWindow` stronger.
  - New: `rmv` / `rfn` (root removed / fenced since Overdrive's last route
    write).
  - New counters: `winE` / `winF` (`LocalRoute`-recovery steps taken while
    traffic leaves the node / while root-fenced). `win` is capped at 3.
  - New ghosts:
    - `widenClose`: a per-allocation close widened the exposure;
    - `widenRoute`: a route step lifted a fence over a detached link;
    - `quiesceFenced`: a witness that a first-hold fence happened.
  - `verified` is per caller.
- **Restated, with the reason recorded.**
  - `FenceHoldsWhileStopped` / `NothingWhileStopped`: scoped to "no root
    tampering since the last fence / Ok verify"; also allow `local` behind
    attached steering. A non-steering fail-stop with verified steering leaves
    `local`, and the pinned program drops every lookup while `serve` is down.
  - `FencedOrVerifiedWhileSteeringRecovers` replaces
    `FencedWhileSteeringRecovers` in `SafetyUnderDetach` (UP-8 lets another
    verified repair lift the fence).
  - `ExposureWithinThreeSteeringSteps` replaces the two-step bound in
    `SafetyUnderDetach`.
  - The round-4b forms stay as checks and are now documented violations.
- **Instances.**
  - `steer_envDetach` shrank: one declared port, MAXGEN 1, crash, flush,
    detach, one boot converge failure, one repair failure. It no longer has a
    listener-task exit, a bind failure, steer failing forever or a second
    allocation (14.7 M states instead of 5.7 M at the larger round-4b shape).
  - `steer_envRootRoute` shrank: one port, MAXGEN 1.
  - New: `steer_envDetachConcurrent`, `steer_envFenceFail`,
    `steer_detachLiveSolo`, `steer_detachLiveNoCrash`,
    `steer_repairForeverNoCrash`, `steer_bugQuiesceNoFence`,
    `steer_bugPartialConverge`, `steer_bugQuiesceBeforeFenceNoVerify`,
    `steer_detachLiveSoloRedoNoFence`, `steer_detachLiveRedoNoCount`,
    `steer_rootRouteLiveAuditIgnores`.
- **Checks dropped or replaced.**
  - `steering-env-detach-other-hold-widens`: `QuiescenceNeverWidens` is now a
    `SafetyUnderDetach` conjunct.
  - `steering-hazard-quiesce-before-fence{,-window,-servable,-widens}`:
    replaced by `-covered` and `-round4{,-any}`; see *Teeth*.
  - `steering-root-route-removal-persists`: replaced by
    `steering-root-route-removal-ends` and `steering-root-fence-ends`.
  - `steering-env-root-route-{window,reach-only-serving}`: now conjuncts of
    `SafetyUnderRouteTamper`.

## Bounds and state counts (module D, round 4c, recorded run)

| Instance | Faults / bounds | Distinct states |
|---|---|---|
| `steer_ok` | crash, flush, root entry deletion, listener-task exit, steer failing forever, 1 bind, 1 boot failure; 2 ports (1 declared), MAXGEN 2 | 313,680 (91–160 s) |
| `steer_noflush` / `steer_two` | as round 4 | 11,521 / 10,302 |
| `steer_envDetach` | root detach, crash, flush, 1 boot failure, 1 repair failure, budget 2; 1 port, MAXGEN 1 | 14,743,005 (564 s) |
| `steer_envDetachConcurrent` | as `steer_envDetach` without crash / boot / repair failures, UP-9 dropped | 10,917,490 (341 s) |
| `steer_envFenceFail` | as above with 1 fence failure, UP-9 kept | 14,685,574 (437 s) |
| `steer_bugQuiesceBeforeFence` | as above, fence-first order off | 4,484,240 (188 s) |
| `steer_envRootRoute` | root route removal / replacement / fence / `local`, crash, flush, 1 bind, steer failing forever, 1 boot failure; 1 port | 700,008 (47 s) |
| `steer_detachLive` (fairness product of `DetachExposureEndsWithOwner`) | 1 port, MAXGEN 1, unbounded detach, listener exit, crash, 1 repair failure | 9,820,292 (1,214 s) |
| `steer_detachLiveSolo` / `steer_rootRouteLive` / `steer_flushLive` | liveness instances | 9,919 / 4,012 / 10,132 |
| `steer_detachLiveAnyRestore` (no crash) | D8a-HOLD off | 7,137,030 (691 s) |
| `steer_ok` Apalache | 12 steps | 1,274 s |

## Every check of module D — expected vs recorded (run `440354-1791359873.950`)

Modules A–C: 52 checks, byte-identical specs, all matched (see `evidence/summary.md`).

| Check | Instance | Property | Backend / bound | Expected | Recorded | s |
|---|---|---|---|---|---|---|
| `steering-safety-tlc` | `steer_ok` | `Safety` | tlc / exhaustive | holds | holds | 91 |
| `steering-flush-no-wildcard` | `steer_ok` | `NoWildcardReached` | tlc / exhaustive | holds | holds | 160 |
| `steering-reach-only-serving` | `steer_ok` | `ReachOnlyServing` | tlc / exhaustive | holds | holds | 98 |
| `steering-noflush-safety-tlc` | `steer_noflush` | `Safety` | tlc / exhaustive | holds | holds | 14 |
| `steering-noflush-unmarked-on-legc` | `steer_noflush` | `UnmarkedOnLegC` | tlc / exhaustive | holds | holds | 12 |
| `steering-two-safety-tlc` | `steer_two` | `Safety` | tlc / exhaustive | holds | holds | 9 |
| `steering-safety-apalache` | `steer_ok` | `Safety` | apalache / max_steps=12 | holds | holds | 1274 |
| `steering-flush-unmarked-bypass` | `steer_ok` | `UnmarkedOnLegC` | tlc / exhaustive | violation | violation | 7 |
| `steering-two-activation-completes` | `steer_two` | `ActivationCompletes` | tlc / exhaustive | holds | holds | 7 |
| `steering-serving-reachable` | `steer_live` | `ServingReachable` | tlc / exhaustive | holds | holds | 6 |
| `steering-boot-opens` | `steer_boot` | `BootOpens` | tlc / exhaustive | holds | holds | 6 |
| `steering-witness-steered` | `steer_ok` | `WitnessNoSteered` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-second-allocation` | `steer_ok` | `WitnessNoSecondAllocation` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-boot-refusal` | `steer_ok` | `WitnessNoBootRefusal` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-resteer` | `steer_ok` | `WitnessNoResteer` | tlc / exhaustive | violation | violation | 7 |
| `steering-witness-overlap` | `steer_ok` | `WitnessNoOverlap` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-still-quiesced` | `steer_ok` | `WitnessNoStillQuiesced` | tlc / exhaustive | violation | violation | 7 |
| `steering-witness-crash-with-route` | `steer_ok` | `WitnessNoCrashWithRoute` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-legc` | `steer_ok` | `WitnessNoLegC` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-fenced-while-down` | `steer_ok` | `WitnessNoRefusedWhileDown` | tlc / exhaustive | violation | violation | 6 |
| `steering-witness-activation-err` | `steer_two` | `WitnessNoActivationErr` | tlc / exhaustive | violation | violation | 6 |
| `steering-hazard-element-set` | `steer_bugElementSet` | `ReachOnlyServing` | tlc / exhaustive | violation | violation | 22 |
| `steering-hazard-element-set-flush` | `steer_bugElementSetFlush` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 24 |
| `steering-hazard-element-close-first` | `steer_bugElementCloseFirst` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 21 |
| `steering-hazard-steering-in-nft` | `steer_bugSteeringInNft` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-unpinned` | `steer_bugUnpinned` | `NothingDeliveredWhileDown` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-nonloopback-assign` | `steer_bugNonloopback` | `RemoteNeverDirect` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-entry-outlives-close` | `steer_bugEntryOutlives` | `EntryNamesServingListener` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-route-first` | `steer_bugRouteFirst` | `RouteImpliesSteering` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-route-first-wildcard` | `steer_bugRouteFirst` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-no-verify` | `steer_bugNoVerify` | `ConvergeNeedsSteering` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-no-verify-wildcard` | `steer_bugNoVerify` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-rollback-leaves-listener` | `steer_bugRollback` | `ActivationAllOrNothing` | tlc / exhaustive | violation | violation | 21 |
| `steering-hazard-rollback-leaves-listener-reach` | `steer_bugRollback` | `ReachOnlyServing` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-act-ok-partial` | `steer_bugActOkPartial` | `ActivationAllOrNothing` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-teardown-keeps-listeners` | `steer_bugTeardownKeeps` | `NoReleaseWithListener` | tlc / exhaustive | violation | violation | 21 |
| `steering-hazard-quiesce-keeps` | `steer_bugQuiesceKeeps` | `QuiescedClosesListeners` | tlc / exhaustive | violation | violation | 21 |
| `steering-hazard-quiesce-keeps-reach` | `steer_bugQuiesceKeeps` | `ReachOnlyServing` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-any-restore` | `steer_bugAnyRestore` | `NoReopenBeforeRepair` | tlc / exhaustive | violation | violation | 21 |
| `steering-hazard-repair-without-hold` | `steer_bugRepairNoHold` | `NoRepairWithoutHold` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-remove-not-fence` | `steer_bugRemoveNotFence` | `NoElsewhere` | tlc / exhaustive | violation | violation | 19 |
| `steering-hazard-remove-not-fence-down` | `steer_bugRemoveNotFence` | `FenceHoldsWhileStopped` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-no-fresh-verify` | `steer_bugNoFreshVerify` | `RouteLocalOnlyVerified` | tlc / exhaustive | violation | violation | 21 |
| `steering-hazard-no-fresh-verify-ghost` | `steer_bugNoFreshVerify` | `ConvergeNeedsSteering` | tlc / exhaustive | violation | violation | 21 |
| `steering-hazard-restore-lifts-fence` | `steer_bugRestoreLiftsFence` | `ExposureOnlyInTamperWindow` | tlc / exhaustive | violation | violation | 22 |
| `steering-hazard-restore-lifts-fence-route` | `steer_bugRestoreLiftsFence` | `RouteLocalOnlyVerified` | tlc / exhaustive | violation | violation | 23 |
| `steering-hazard-quiesce-no-fence` | `steer_bugQuiesceNoFence` | `QuiescenceNeverWidens` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-partial-converge` | `steer_bugPartialConverge` | `ConvergeNeedsSteering` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-partial-converge-window` | `steer_bugPartialConverge` | `ExposureOnlyInTamperWindow` | tlc / exhaustive | violation | violation | 21 |
| `steering-hazard-partial-converge-route` | `steer_bugPartialConverge` | `RouteLocalOnlyVerified` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-quiesce-before-fence-round4` | `steer_bugQuiesceBeforeFenceNoVerify` | `SteeringQuiesceNeverWidens` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-quiesce-before-fence-round4-any` | `steer_bugQuiesceBeforeFenceNoVerify` | `QuiescenceNeverWidens` | tlc / exhaustive | violation | violation | 20 |
| `steering-hazard-quiesce-before-fence-covered` | `steer_bugQuiesceBeforeFence` | `SteeringQuiesceNeverWidens` | tlc / exhaustive | holds | holds | 188 |
| `steering-env-detach-safety` | `steer_envDetach` | `SafetyUnderDetach` | tlc / exhaustive | holds | holds | 564 |
| `steering-env-detach-wildcard` | `steer_envDetach` | `NoWildcardReached` | tlc / exhaustive | violation | violation | 22 |
| `steering-env-detach-route-implies-steering` | `steer_envDetach` | `RouteImpliesSteering` | tlc / exhaustive | violation | violation | 20 |
| `steering-env-detach-down` | `steer_envDetach` | `NothingDeliveredWhileDown` | tlc / exhaustive | violation | violation | 20 |
| `steering-env-detach-remote-direct` | `steer_envDetach` | `RemoteNeverDirect` | tlc / exhaustive | violation | violation | 21 |
| `steering-env-detach-held-servable-wildcard` | `steer_envDetach` | `WitnessNoServableWildcardWhileHeld` | tlc / exhaustive | violation | violation | 23 |
| `steering-env-detach-close-widens` | `steer_envDetach` | `CloseNeverWidens` | tlc / exhaustive | violation | violation | 28 |
| `steering-env-detach-route-widens` | `steer_envDetach` | `RouteNeverWidens` | tlc / exhaustive | violation | violation | 28 |
| `steering-env-detach-fence-lifted-by-other-repair` | `steer_envDetach` | `FencedWhileSteeringRecovers` | tlc / exhaustive | violation | violation | 22 |
| `steering-env-detach-hold-wildcard-after-lift` | `steer_envDetach` | `SteeringHoldNoWildcard` | tlc / exhaustive | violation | violation | 21 |
| `steering-env-detach-two-step-window` | `steer_envDetach` | `ExposureWithinTwoSteeringSteps` | tlc / exhaustive | violation | violation | 25 |
| `steering-witness-recovery-fence` | `steer_envDetach` | `WitnessNoRecoveryFence` | tlc / exhaustive | violation | violation | 22 |
| `steering-witness-one-step-exposure` | `steer_envDetach` | `WitnessNoOneStepExposure` | tlc / exhaustive | violation | violation | 21 |
| `steering-witness-quiesce-fence` | `steer_envDetach` | `WitnessNoQuiesceFence` | tlc / exhaustive | violation | violation | 27 |
| `steering-witness-fail-stop` | `steer_repairForever` | `WitnessNoFailStop` | tlc / exhaustive | violation | violation | 27 |
| `steering-env-detach-concurrent-window` | `steer_envDetachConcurrent` | `ExposureOnlyInTamperWindow` | tlc / exhaustive | violation | violation | 22 |
| `steering-env-detach-concurrent-route` | `steer_envDetachConcurrent` | `RouteLocalOnlyVerified` | tlc / exhaustive | violation | violation | 22 |
| `steering-env-detach-concurrent-converge` | `steer_envDetachConcurrent` | `ConvergeNeedsSteering` | tlc / exhaustive | holds | holds | 341 |
| `steering-env-fence-fail-widens` | `steer_envFenceFail` | `QuiescenceNeverWidens` | tlc / exhaustive | violation | violation | 22 |
| `steering-env-fence-fail-window` | `steer_envFenceFail` | `ExposureOnlyInTamperWindow` | tlc / exhaustive | holds | holds | 437 |
| `steering-detach-exposure-ends` | `steer_detachLive` | `DetachExposureEnds` | tlc / exhaustive | holds | violation **(unexpected)** | 25 |
| `steering-detach-exposure-ends-find-fence` | `steer_detachLive` | `DetachExposureEndsFindFence` | tlc / exhaustive | violation | violation | 23 |
| `steering-solo-detach-exposure-ends-find-fence` | `steer_detachLiveSolo` | `DetachExposureEndsFindFence` | tlc / exhaustive | holds | holds | 20 |
| `steering-recovery-resolves` | `steer_detachLive` | `SteeringRecoveryResolves` | tlc / exhaustive | holds | violation **(unexpected)** | 23 |
| `steering-repair-forever-resolves` | `steer_repairForever` | `SteeringRecoveryResolves` | tlc / exhaustive | holds | violation **(unexpected)** | 23 |
| `steering-detach-exposure-ends-with-owner` | `steer_detachLive` | `DetachExposureEndsWithOwner` | tlc / exhaustive | holds | holds | 1214 |
| `steering-recovery-resolves-all-recoveries` | `steer_detachLiveNoCrash` | `SteeringRecoveryResolvesAllRecoveries` | tlc / exhaustive | holds | violation **(unexpected)** | 275 |
| `steering-repair-forever-resolves-all-recoveries` | `steer_repairForeverNoCrash` | `SteeringRecoveryResolvesAllRecoveries` | tlc / exhaustive | holds | violation **(unexpected)** | 262 |
| `steering-hazard-redo-no-fence` | `steer_detachLiveSoloRedoNoFence` | `DetachExposureEndsFindFence` | tlc / exhaustive | violation | violation | 22 |
| `steering-hazard-redo-no-count` | `steer_detachLiveRedoNoCount` | `SteeringRecoveryResolves` | tlc / exhaustive | violation | violation | 24 |
| `steering-hazard-any-restore-exposure` | `steer_detachLiveAnyRestore` | `DetachExposureEnds` | tlc / exhaustive | violation | holds **(unexpected)** | 691 |
| `steering-hazard-any-restore-resolves` | `steer_detachLiveAnyRestore` | `SteeringRecoveryResolves` | tlc / exhaustive | violation | violation | 25 |
| `steering-hazard-no-recovery-fairness` | `steer_detachLiveNoFairness` | `DetachExposureEndsNoRecoveryFairness` | tlc / exhaustive | violation | violation | 22 |
| `steering-flush-legc-bypass-ends` | `steer_flushLive` | `LegCBypassEnds` | tlc / exhaustive | holds | holds | 22 |
| `steering-steer-forever-unreachable` | `steer_steerForever` | `ServingReachable` | tlc / exhaustive | violation | violation | 20 |
| `steering-env-root-route-safety` | `steer_envRootRoute` | `SafetyUnderRouteTamper` | tlc / exhaustive | holds | holds | 47 |
| `steering-env-root-route-elsewhere` | `steer_envRootRoute` | `NoElsewhere` | tlc / exhaustive | violation | violation | 22 |
| `steering-env-root-route-unverified-local` | `steer_envRootRoute` | `RouteLocalOnlyVerified` | tlc / exhaustive | violation | violation | 20 |
| `steering-env-root-route-down` | `steer_envRootRoute` | `FenceHoldsWhileStopped` | tlc / exhaustive | violation | violation | 21 |
| `steering-witness-local-route-repair` | `steer_envRootRoute` | `WitnessNoLocalRouteRepair` | tlc / exhaustive | violation | violation | 21 |
| `steering-witness-three-step-elsewhere` | `steer_envRootRoute` | `WitnessNoThreeStepElsewhere` | tlc / exhaustive | violation | violation | 20 |
| `steering-witness-three-step-root-fence` | `steer_envRootRoute` | `WitnessNoThreeStepRootFence` | tlc / exhaustive | violation | violation | 23 |
| `steering-root-route-removal-ends` | `steer_rootRouteLive` | `RouteTamperEnds` | tlc / exhaustive | holds | holds | 22 |
| `steering-root-fence-ends` | `steer_rootRouteLive` | `RootFenceEnds` | tlc / exhaustive | holds | holds | 21 |
| `steering-hazard-audit-ignores-route-removal` | `steer_rootRouteLiveAuditIgnores` | `RouteTamperEnds` | tlc / exhaustive | violation | violation | 21 |
| `steering-hazard-audit-ignores-route-fence` | `steer_rootRouteLiveAuditIgnores` | `RootFenceEnds` | tlc / exhaustive | violation | violation | 21 |

## First-pass results and the modelling fixes (round 4c)

The runner keeps only the last recorded run in `evidence/`. The earlier passes
are condensed here. No expectation was changed after any run.

| Pass | What happened | Exact cause | Action |
|---|---|---|---|
| CI pass, unrecorded (`345630-1791343858.019`) | 144 checks, 135 matched; 4 tool errors, 2 timeouts, 3 unexpected | The tool errors were kernel OOM kills (`dmesg`: "Out of memory: Killed process … (java)"), three 8 GiB TLC heaps on a 15 GiB VM. The two timeouts were `steer_envRootRoute` (8.2 M states and growing) and `steer_detachLiveAnyRestore`. The unexpected results are the design results F-4c-2 / F-4c-3 | Instances resized (*What changed*). No property or expectation changed. The three predictions written after this pass are in *Predictions added after the first CI pass* |
| single checks, unrecorded | `DetachExposureEndsWithOwner` held (9.8 M states, 1,264 s). The two `…AllRecoveries` checks were violated (F-4c-4). `steering-hazard-any-restore-exposure` held (unexpected) | design results, not modelling errors | recorded as found |
| record 1 (`424146-1791353069.285`, `--jobs 2`) | 142 of 151 matched: the 6 design mismatches plus 4 tool failures | OOM kill of `steering-hazard-any-restore-exposure` (`dmesg`, pid 438215); Apalache start-up failure of `owner-flows-hazard-admit-split` (module A, unchanged); 1,800 s timeouts of `steering-detach-exposure-ends-with-owner` and `owner-flows-safety-apalache-deep` under CPU contention | preserved in `quint-guest-flow-owner-evidence-r4c-jobs2/`; re-recorded with `--jobs 1` → no tool failures |

The pass-1 counterexample logs of the unexpected checks were kept outside the
repository during the round. The recorded traces of the same checks are in
`evidence/traces/` and `evidence/failures/`.

## Counterexamples against design rules (round 4c)

**F-4c-1 — item 7 covers the quiesce, not every listener close (D8a-FENCE item
7, D23, M-7, U-6).** Check `steering-env-detach-close-widens`
(`CloseNeverWidens` violated; predicted). Minimal trace:

1. boot opens (`local`, steering verified);
2. provision, session open;
3. **root detaches the link**;
4. the guest reports port 1 listening;
5. activation begins, and the owner binds the intake listener for port 1 (a
   marked client to port 1 reaches the intake by its specific bind, K-D1);
6. `steer` fails, so the activation rollback closes the listener (U-6);
7. port 1 now reaches the wildcard host service.

The same holds for `ListenState` off, session loss and teardown: every close
the M-7 order performs. It stays inside the trust-boundary window
(`ExposureOnlyInTamperWindow` holds), and the steering recovery's finding and
fence end it. The dispatcher's strengthened property ("the set of exposed ports
never grows through an Overdrive step") is therefore **false** as stated; it
holds for the quiesce only. Options:
- (a) state the per-allocation closes as part of the window;
- (b) every listener close by the owner, not only the first hold, runs
  `verify` and fences if it fails;
- (c) accept and document.

**F-4c-2 — the detach window's end needs the owner under UP-8 (D8a-FENCE item 8,
UP-8, D8a-HOLD).** Check `steering-detach-exposure-ends` (**unexpected**:
predicted holds). Recorded trace:

1. root detaches;
2. the FlowListeners recovery quiesces;
3. the steering recovery finds the detach and **fences** (state `fenced`, not
   yet holding);
4. FlowListeners' total `converge_shared` re-attaches, verifies and **puts
   `local` back**;
5. **root detaches again**;
6. FlowListeners restores, and forwarding reopens (no other holder);
7. an activation item begins.

The steering recovery's next step is its quiesce, which as first holder would
verify and fence. It waits behind the activation item, so the exposure lasts
until the owner completes the activation.

With weak fairness of the owner added, the exposure ends
(`DetachExposureEndsWithOwner` holds, 9,820,292 states). With no other recovery,
the finding and the fence alone suffice, as in round 4b
(`steering-solo-detach-exposure-ends-find-fence` holds). With another recovery
they do not (`steering-detach-exposure-ends-find-fence`: violation, predicted).

**F-4c-3 — the steering recovery's outcome now depends on other recoveries
(UP-8, ADR-0124, D8a-FENCE).** Checks `steering-recovery-resolves` and
`steering-repair-forever-resolves` (**unexpected**; both held in round 4b).
Recorded trace:

1. the steering recovery finds a root detach and fences, then `serve`
   crashes (the episode ends);
2. on restart, root detaches again between boot's verify and its route step,
   and boot puts `local` over the detached link (item 8's verify → route
   window);
3. FlowListeners quiesces: first hold, so verify fails and the prefix is
   **fenced** (item 7);
4. FlowListeners' `converge_shared` converges; its probe **fails**, so it
   fences and the attempt fails;
5. `LocalRoute` damage (fence while the steering verifies): the `LocalRoute`
   recovery quiesces;
6. nothing in the steering recovery's own steps lifts that fence.

"`local` behind verified steering" now needs the FlowListeners or `LocalRoute`
recovery to progress.

**F-4c-4 — with root detaching repeatedly, recovery episodes can repeat
forever within budget (UP-7 per-episode budget × UP-8).** Checks
`steering-recovery-resolves-all-recoveries` and
`steering-repair-forever-resolves-all-recoveries` (**unexpected**: predicted
holds with every recovery fair). The lasso, simplified:

1. the steering recovery finds and fences;
2. FlowListeners' and `LocalRoute`'s converges re-attach the link and set
   `local`;
3. the steering recovery's episode restores while another recovery's failed
   probe has fenced the prefix (or while it is still `found`);
4. **root detaches again**, and the loop returns.

Each recovery restores inside its budget, which resets per episode. The node
neither reaches `local` behind verified steering nor fail-stops. This is
root-driven (the trust boundary) but unbounded. ADR-0124's bound holds per
episode, not across overlapping episodes. Options:
- (a) accept as trust-boundary behaviour;
- (b) a node-level budget across episodes;
- (c) a non-steering `converge_shared` does not lift a fence while a steering
  recovery episode stands, which also removes F-4c-2's and the three-step
  window's cause.

**F-4c-5 — UP-9 (unstated): a stale route step undoes a fresh fence
(D8a-FENCE, UP-8).** Checks `steering-env-detach-concurrent-window`
(`ExposureOnlyInTamperWindow` violated) and `-concurrent-route`
(`RouteLocalOnlyVerified` violated), on `steer_envDetachConcurrent`: predicted,
recorded. Minimal trace:

1. the IpRules recovery quiesces;
2. its `converge_shared` converges and **verifies** the steering;
3. **root detaches**;
4. the steering recovery finds the detach and **fences**;
5. IpRules' `converge_shared` route step puts **`local`** back over the
   detached link.

The prefix is now exposed with no root action since the fence. Each call did
verify (`ConvergeNeedsSteering` holds there). With the owner's route writes
serialized (one `converge_shared` call and a fence exclude one another) the
design instances never reach it. **The design does not state that
serialization.**

**Documented consequences (predicted, recorded as violations).**
- `RouteNeverWidens` (`steering-env-detach-route-widens`): item 8's root detach
  between a verify and its route step.
- `FencedWhileSteeringRecovers` (`-fence-lifted-by-other-repair`): UP-8, another
  verified repair lifts the fence.
- `SteeringHoldNoWildcard` (`-hold-wildcard-after-lift`): after that lift, a
  fresh detach exposes during the steering hold.
- `ExposureWithinTwoSteeringSteps` (`-two-step-window`). Trace:
  1. both the IpRules and steering recoveries hold;
  2. IpRules' repair lifts the fence;
  3. the steering recovery's `converge_shared` converges and verifies;
  4. **root detaches**;
  5. the steering recovery's route step (exposed) → re-finding (still
     exposed) → fence.

  Three steps; item 8 claims two.
- `QuiescenceNeverWidens` with a failed first-hold fence
  (`steering-env-fence-fail-widens`). Item 7's "a fence failure here … does not
  stop the quiesce" re-opens F-4b-1 for that attempt. It stays inside the
  tamper window (`steering-env-fence-fail-window` holds, 14,685,574 states).

## The three trust-boundary windows, in steps

| Root action | What is exposed | Ends at | Window (steps) | Checks |
|---|---|---|---|---|
| **Link detach** | wildcard host services on every port without an open intake listener; plus ports whose listener a per-allocation close takes down meanwhile (F-4c-1) | the steering recovery's fence; or an Ok `verify` (a first-hold quiesce's fence; another recovery's `converge_shared`) | **2 steering-recovery steps** (finding, fence) when no other recovery has lifted the fence; **at most 3** (in-flight route step, re-finding, fence) under UP-8. That step can wait behind an activation item (F-4c-2). In time: one audit period + one route replace, or + the activation item | `ExposureOnlyInTamperWindow`, `ExposureWithinThreeSteeringSteps` hold (14,743,005 states); `WitnessNoOneStepExposure`, `ExposureWithinTwoSteeringSteps` violated; `DetachExposureEndsWithOwner` holds |
| **Route removal** (or a different route) | prefix traffic leaves the node by another route (no host service is reached) | the next Overdrive route write: the `LocalRoute` recovery's `local`, another recovery's `converge_shared`, or a fence | **4 `LocalRoute`-recovery steps** (finding + quiesce, converge, verify + probe, route); still standing after 3 | `ElsewhereOnlyInRemovalWindow`, `ElsewhereWithinRouteRepair` hold (700,008 states); `WitnessNoThreeStepElsewhere` violated; `RouteTamperEnds` holds under fairness of the `LocalRoute` recovery |
| **Root fence** (over verified steering) | nothing exposed; every connection to the prefix refused (availability) | the `LocalRoute` recovery's `local` | **4 `LocalRoute`-recovery steps**; still standing after 3 | `RootFenceWithinRouteRepair` holds; `WitnessNoThreeStepRootFence` violated; `RootFenceEnds` holds |

When the `LocalRoute` recovery's budget is spent (root removing the route
repeatedly), `serve` fail-stops with the route as root left it. If that is
absent, traffic leaves the node while `serve` is down until the next boot's
`converge_shared` (`steering-env-root-route-down`, documented). The fence is
pinned for the steering fail-stop only.

## Teeth (round 4c)

| Rule | Hazard instance | Broken property | Counterexample in short |
|---|---|---|---|
| Item 7 (F-4b-1): first hold of any holder verifies + fences before closing | `steer_bugQuiesceNoFence` | `QuiescenceNeverWidens` | activated, port 1 listening → **root detach** → the guest reports port 1 again, bound → **IpRules quiesces without verify** → listener closed, port 1 → wildcard |
| UP-8: every repair's `converge_shared` re-establishes the steering first | `steer_bugPartialConverge` | `ConvergeNeedsSteering`, `ExposureOnlyInTamperWindow`, `RouteLocalOnlyVerified` | flush → **root detach** → IpRules quiesces (item 7 fences) → IpRules repair **skips the steering**, `local` over the detached link → exposed with no root action since the fence |
| UP-5: the audit observes the route's form | `steer_rootRouteLiveAuditIgnores` | `RouteTamperEnds`, `RootFenceEnds` | root removes (fences) the route → nothing finds it while `serve` stays up |
| UP-6: every steering attempt begins with the fence | `steer_detachLiveSoloRedoNoFence` | `DetachExposureEndsFindFence` | repaired → **root detach** → re-finding goes straight to the repair, no fence → the exposure needs the repair's fairness |
| UP-7: a failed post-repair audit is a failed attempt | `steer_detachLiveRedoNoCount` | `SteeringRecoveryResolves` | root re-detaches after every repair → the attempt count never rises → never fail-stops, never resolves |
| D8a-FENCE's order + item 7 (revision 9 as first written) | `steer_bugQuiesceBeforeFenceNoVerify` | `SteeringQuiesceNeverWidens`, `QuiescenceNeverWidens` | round-4 observation 1 |
| **D8a-FENCE's order alone** (fence before the steering recovery's own quiesce) | `steer_bugQuiesceBeforeFence` | none: `SteeringQuiesceNeverWidens` **holds** (`-covered`, 4,484,240 states), as predicted | item 7's first-hold verify + fence covers the steering recovery's quiesce. When another holder holds, the order adds one exposed step, but that is inside the three-step bound. **The order has no remaining teeth on any checked property.** |
| Round-4 / 4b teeth | all other `steer_bug*` | as round 4b | all `violation` except `steering-hazard-any-restore-exposure` (see *Verdict*) |

## Underspecified points (round 4c)

- **UP-9 (D8a-FENCE, UP-8, ADR-0124; new): are the owner's route writes
  serialized?** The text does not say whether a `converge_shared` call
  (converge → verify + probe → route) and `fence_guest_prefix` (or a first-hold
  fence) can interleave. The two runtime paths are concurrent recoveries, which
  D8a-HOLD makes explicit. Without serialization: F-4c-5. Pin either:
  - one owner lock across `converge_shared`'s verify → route and every fence; or
  - a conditional route step: replace the fence with `local` only if no fence
    was written since this call's verify.
- **UP-10 (UP-8 × D8a-FENCE; new): may another recovery's `converge_shared`
  lift the steering recovery's fence while that episode stands?** As written,
  yes. This yields F-4c-2, F-4c-4 and the three-step detach window (item 8
  claims two). Pin either "lifts" (and restate item 8's bound) or "does not
  lift while a `Recovery(GuestPrefixSteering)` episode stands".
- **UP-11 (ADR-0124, UP-7; new): is the attempt budget per episode with no
  node-level bound?** With overlapping episodes and repeated root detach the
  recoveries can cycle forever (F-4c-4).
- **UP-12 (D8a-FENCE; new): does a non-steering fail-stop fence the prefix?**
  The text pins the fence for the steering recovery's fail-stop and for boot
  refusal. After an IpRules / FlowListeners / `LocalRoute` fail-stop the route
  stays as it was: `local` with verified steering (harmless while `serve` is
  down, K-L4), or absent after a root removal (traffic leaves the node until
  the next boot).
- **F-4c-1** is also an underspecified point of item 7: only the first-hold
  quiesce verifies. A listener close by the M-7 order does not.
- **"Different route"** (UP-5) is modelled as `absent` (traffic leaves the
  node). A different route of another type (e.g. `blackhole`) would refuse,
  not leak. The window is the same.
- UP-3 (§ *Formal protocol model* text) still open; UP-5 to UP-8 closed by item
  7 and modelled as pinned.

## Assumptions and their discharging items (round 4c)

All round-4b assumptions unchanged (K-A1 … K-L6 → V-10, V-22, P-23, V-24,
`FlowId`, R5-14, V-5(c), P-16, V-25, V-19, V-23, V-13, V-26). Added:

| ID | Assumption | Discharged by |
|---|---|---|
| K-L7 | `GuestPrefixSteering::verify` observes a root detach that happened before it | V-26 |
| F-FENCE | A fence (route replace) can fail; modelled as a fault | Environment, modelled as an action (`steer_envFenceFail`) |
| UP-9 | The owner's route writes are serialized (design instances only) | **None — not stated by the design** |

**One assumption lacks a discharging item: UP-9.**

## CI flags (round 4c)

138 of 151 checks carry `ci = true`. Not CI:
- module A–C: as round 4b;
- `steering-safety-apalache` (1,274 s);
- `steering-hazard-quiesce-before-fence-covered` (188 s);
- `steering-env-detach-safety` (564 s);
- `steering-env-detach-concurrent-converge` (341 s);
- `steering-env-fence-fail-window` (437 s);
- `steering-detach-exposure-ends-with-owner` (1,214 s);
- `steering-recovery-resolves-all-recoveries` (275 s);
- `steering-repair-forever-resolves-all-recoveries` (262 s);
- `steering-hazard-any-restore-exposure` (691 s).

Two CI checks exceeded two minutes in the record:
- `steering-flush-no-wildcard`: 160 s, on `steer_ok`'s 313,680 states, which
  took 74–98 s in its sibling checks and in pass 1. VM variance; next revision
  should mark the three `steer_ok` TLC views non-CI or merge them.
- `udp-slots-safety-apalache` (1,035 s) is already non-CI.

---

# Round 4d — the design's answer to round 4c (Revision 9 decisions items 3, 7, 8 as of 2026-10-07)

Appended 2026-10-07. Rounds 4, 4b and 4c above are unchanged. The round-4c
recorded run (`440354-1791359873.950`) is preserved verbatim in
`docs/feature/netns-density-295/design/quint-guest-flow-owner-evidence-r4c/`
before this round's `--record` replaces `evidence/`.

Design text modelled (read, not edited): `feature-delta.md` § *Revision 9
decisions — 2026-10-07* items 3, 7, 8; § *Owner and provisioner* doc comments of
`converge_shared`, `repair_guest_prefix`, `audit_shared`, `quiesce_forwarding`,
`QuiescenceHolderMismatch` and the paragraph "Intake listener close (D8a-FENCE,
every cause)"; § *Driven port — guest-prefix steering* ("Owner" bullet);
ADR-0152 point 1, ADR-0169, ADR-0171.

## Predictions (written before the first run of round 4d)

Hypothesis: with one owner and one runtime repair of the guest prefix
(`repair_guest_prefix`: fence → converge → verify → probe → `local`, hold-checked;
no other repair writes the steering or the route), the serialized guest-prefix
order (no Overdrive fence between a lift's verify and its route write), and a
verify + fence before every quiesce call and every intake listener close, no
owner step widens a detach exposure, nothing but boot and the guest-prefix
recovery lifts a fence, and the guest-prefix recovery resolves under its own
fairness whatever the other recoveries do.

Every prediction below is the `expect` of its check in `checks.toml`.

- **Hold on the design instances.** `Safety` (`steer_ok`, `steer_noflush`,
  `steer_two`; TLC, and Apalache to 12 steps); `SafetyUnderDetach`
  (`steer_envDetach`), which now contains `NoOwnerStepWidens` (every quiesce
  and every per-allocation close), `FencedWhileSteeringRecovers`,
  `OnlyGuestPrefixRecoveryLifts` and `OpenExposureWithinTwoSteeringSteps`;
  `SafetyUnderRouteTamper` (`steer_envRootRoute`) with the windows below.
- **Liveness, holds.** `DetachExposureEndsFindFence` on `steer_detachLive`
  (with another recovery; it was violated in round 4c) and on the solo
  instance; `DetachExposureEndsWithOwner`; `SteeringRecoveryResolves` on
  `steer_detachLive` and `steer_repairForever` (both violated in round 4c);
  `SteeringRecoveryResolvesAllRecoveries` on both crash-free instances (both
  violated in round 4c); `LegCBypassEnds` without and with a concurrent root
  detach (`steer_flushDetachLive`, new: the IpRules recovery never waits on the
  guest-prefix recovery's progress, UP-11); `RouteTamperEnds` and
  `RootFenceEnds` under the guest-prefix recovery's fairness.
- **Predicted violations against the design's claims (findings, written now).**
  - **F-4d-1 (D8a-FENCE, UP-7, UP-12): `GuestPrefixFailStopFenced` violated.**
    The budget can be spent by a failed post-repair audit: lift → root detach →
    the audit finds it (failed attempt; budget now spent) → fail-stop. The last
    route write was the lift, so `serve` stops with `local` over the detached
    link, not with the fence. With the nft rules also gone, a host-local client
    reaches the wildcard host service while `serve` is down, and the exposure
    outlives the two-step bound (`ExposureWithinTwoSteeringSteps` violated;
    `OpenExposureWithinTwoSteeringSteps`, the same bound while `serve` is open,
    holds).
  - **F-4d-2 / UP-13 (D8a-FENCE item 8, D8a-HOLD, U-6 sequencer):
    `DetachExposureEnds` violated.** The guest-prefix recovery's only fence
    before its repair is its `quiesce_forwarding` call, which the sequencer
    orders behind an in-flight activation item. With no fairness of the
    owner's activation steps the detach exposure can last forever; with it,
    it ends (`DetachExposureEndsWithOwner` holds).
  - **F-4d-3 (item 7 / item 8): `NoOwnerStepWidens` violated on
    `steer_envCloseGap`.** A close's verify and its close are two operations;
    a root detach between a passing verify and the close makes that close
    widen the exposure — the same trust-boundary shape item 8 states for a
    lift. `ExposureOnlyInTamperWindow` still holds there.
  - **UP-15: a fence failure at a close.** The design states the quiesce goes
    on after a fence failure; for a close it does not say. Modelled the same way
    (the close goes on); `CloseNeverWidens` and `QuiescenceNeverWidens` violated
    on `steer_envFenceFail`, `ExposureOnlyInTamperWindow` holds.
- **Windows (trust boundary), counted in guest-prefix-recovery steps.**
  - Root link detach: at most 2 (finding + quiesce fence, or failed audit +
    the next attempt's fence) while `serve` is open; witness of 1 reached.
  - Root route removal: traffic leaves the node for at most 4 steps
    (`ElsewhereWithinGpsRepair`: `winE <= 3` holds; witness `winE >= 3`
    reached). Worst case: removal right after the lift — the post-repair audit
    is clean under the hold (UP-14, below), so restore, a new finding, quiesce,
    then the repair's fence.
  - Root fence: lifted within at most 7 steps (`RootFenceWithinGpsRepair`:
    `winF <= 6` holds; witness `winF >= 6` reached): restore, finding, quiesce,
    fence, converge, verify, lift.
- **UP-14 (to be confirmed by the windows): the post-repair audit runs under
  the guest-prefix hold, and the design's damage rule ignores the route form
  while that hold stands, so a route the root removed or fenced after the lift
  is not a failed attempt; the recovery restores and a new episode finds it.**
- **Teeth (violation predicted).** Item 7: `steer_bugQuiesceNoFence`
  (`QuiescenceNeverWidens`, `SteeringQuiesceNeverWidens`),
  `steer_bugJoinNoVerify` (`OpenExposureWithinTwoSteeringSteps`: a joining
  call that does not verify adds a step), `steer_bugCloseNoVerify`
  (`CloseNeverWidens`: the F-4c-1 trace). One owner:
  `steer_bugOtherRepairsPrefix` (`OnlyGuestPrefixRecoveryLifts`,
  `FencedWhileSteeringRecovers`). Serialized order:
  `steer_bugUnserializedLift` (`ExposureOnlyInTamperWindow`,
  `RouteLocalOnlyVerified`; the F-4c-5 trace; `ConvergeNeedsSteering` holds
  there). UP-6, UP-7, UP-5, D8a-LOOKUP, D8a-ROUTE, U-6, U-3, D8a-HOLD and the
  rest of D8a-FENCE: as round 4c, adapted to the new recovery
  (`steer_bugRestoreLiftsFence` additionally breaks
  `OnlyGuestPrefixRecoveryLifts`).
- **`steering-hazard-any-restore-exposure` re-examined: predicted to hold
  (renamed `…-covered`, on `DetachExposureEndsWithOwner`).** With any restore
  reopening, the guest-prefix recovery can be reset forever, but every reset
  needs another recovery to re-quiesce, and that quiesce call verifies and
  fences the detached prefix: an early restore cannot prolong the exposure.
  D8a-HOLD keeps its teeth for the guest prefix on `NoReopenBeforeRepair`
  (`steering-hazard-any-restore`) and on `SteeringRecoveryResolves`
  (`steering-hazard-any-restore-resolves`: the reset loop never resolves).
  On `DetachExposureEnds` (no owner fairness) the check would be vacuous
  (F-4d-2 fails it on the design too), so it is not used.

### Prediction added after the first recording (written before this check ran)

The first recording (run `473536-1791379293.603`, preserved in
`quint-guest-flow-owner-evidence-r4d-record1/`) matched 155 of 155. Its
root-detach safety instance overlaps the guest-prefix recovery with the IpRules
recovery only. One check is added on `steer_envDetachAll`: root detach
(unbounded), foreign flush and listener-task exit (all three recoveries can
hold at once), crash at any step, one bind failure, one steer failure, one boot
converge failure, one repair failure, attempt budget 2, one port, MAXGEN 1.
Prediction: `SafetyUnderDetach` **holds** (`steering-env-detach-all-safety`).
Falsification: any reachable state violating a conjunct.

First run of `steering-env-detach-all-safety` (unrecorded): **tool timeout** at
1,800 s with 21,516,677 distinct states and the search still growing (crash,
bind, steer, boot and repair failures all on). Not a verdict. The instance is
shrunk — no crash, no bind / steer / boot failure (all covered by
`steer_envDetach`); all three recoveries, unbounded root detach and one repair
failure kept. Prediction unchanged: holds.
