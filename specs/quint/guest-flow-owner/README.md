# Quint specification: guest-flow owner (netns-density-295)

Model-checked specification of the concurrent, ordered and crash-sensitive protocols of the vsock
attachment replacement, and the oracle DISTILL's `quint-connect` conformance drivers replay against
the production owners (`.claude/rules/design.md` § "Concurrent protocols carry a model-checked Quint
specification", `.claude/rules/testing.md` § "Quint conformance").

Every module here is a state machine (state variables, `init`, `step`) with named runs for its critical
scenarios. Each run is listed in `checks.toml` (`backend = "evaluator"`). Every design rule has a
hazard instance in `hazard/` that removes or weakens it, with `expect = "violation"`.

Run: `cargo xtask lima run -- cargo xtask quint typecheck`, then
`cargo xtask lima run -- cargo xtask quint check --subsystem guest-flow-owner [--name <check>] [--ci]`.
Evidence of the last full run: `evidence/summary.md` (written only by `--record`; `cargo xtask quint
verify-evidence --subsystem guest-flow-owner` fails when it no longer matches the specs).

Decision ids are the ones of the 2026-10-11 DESIGN round (INB-1, UDP-EMPTY, R2-END, R2-DRAIN,
CID-NOCLAIM, RESET-EXIT, UDP-SEQ, QHOLD, NO-SURVIVE) and the ADRs they live in (ADR-0146, -0156,
-0157, -0160, -0165, -0166, -0169, -0173, -0177, -0180, -0182, -0183, -0184). Spike evidence:
`docs/feature/netns-density-295/spike/` (`findings-v29` .. `findings-v32`, `findings-drain-t1-t6`).

## Modules

| Module (file) | Protocol | Decisions defended |
|---|---|---|
| `owner_flows.qnt` | control sessions, flow admission, launch attribution (session, flows, beacon), host-opened dial | ATTR-TOKEN, ATTR-LIVE-CLAIM, INB-1, U-1, U-5, H-1, DG-5-E, DG-5-F |
| `flow_end.qnt` | how one paired TCP flow ends across the VM boundary and how a half-close drain completes | R2-END, R2-DRAIN (A1..A7), RESET-EXIT (PID-1 crash), DG-5-C, DG-5-F, INB-1 (the dialer's end) |
| `flow_quota.qnt` | the per-allocation flow quota counts draining flows | R2-DRAIN, ADR-0147 |
| `cid_lease.qnt` | CID = base + lease offset; lease released after VMM exit; boot reclamation before assign; guest reset ends the VMM | CID-NOCLAIM, RESET-EXIT, NO-SURVIVE |
| `quiescence.qnt` | counted, move-only quiescence hold; per-component recoveries; boot step 8 refuses boot | QHOLD, ADR-0124, ADR-0184 |
| `udp_slots.qnt` | guest UDP slot pool and datagram associations | D25, SLOT-ABORT, U-2, CELL-RECREATE |
| `udp_empty.qnt` | the owner's handling of the empty-datagram notice | UDP-EMPTY |

The parts of each module an instance does not use stay constant. Instances (`const CFG`): `owner_flows`
`core` / `core2` / `sess` / `beacon`; `flow_end` `all` / `clean` / `dv` / `do` / `ring` / `abort` / `two` (the byte
count and the enabled environment faults); `cid_lease` `ok` / `conf_ok` / `live`; `quiescence` `ok` /
`live`; `udp_slots` `ok` / `live` / `conf`. `OFF` names the rules switched off; the design instances
switch none off.

Coverage note: the `flow_end` safety invariants are checked exhaustively per fault group (instances `clean`,
`dv`, `do`, `ring`, `abort`, `two`). The all-faults instance `end_all`, which the named runs use, is too large
for exhaustive search (TLC: over 30 min; Apalache: no 7-step bound within 15 min), and so is the abort-side
group combined with the guest-side kernel faults. Faults of different groups therefore meet only along the
named runs, each of which checks `Safety` at every step.

### What the model contains nothing of

Removed by the round and absent from every module: host-local delivery steering of the guest prefix,
intake listeners / the intake port block / listen-state mirroring, prefix landing and boot, the shared
table's boot-continues path, the kernel CID claim (claim port, survivors, fd handoff), `HalfClose` /
`HalfCloseAck` control messages, a drain deadline / recheck / `DrainTimeout`, quiescence holder names,
`StillQuiesced { holders }`, `QuiescenceHolderBusy`, VM survival across a `serve` restart.

### Not modelled, with the reason

- Kernel-program verdict invariants (never pass or redirect a zero-length skb; pull the whole skb before a
  redirect into UDP / SEQPACKET; the ADR-0149 length check; `TCP_NODELAY` and the buffer invariant of
  SEQPACKET legs): properties of one BPF program on one skb, not of an ordered protocol between owners.
  They are Tier-2 / Tier-3 facts (V-29, V-32).
- Owner implementation rules T1..T6 of R2-DRAIN that are not orderings between owners (install the flow
  vsock at `connect()`, no `SO_RCVLOWAT` above 1, key events on flow-id tokens, `SO_RCVBUF` of the guest
  intake listener): DELIVER and Tier-3.
- GW-LOCAL (the gateway as the one host-local prefix address): a boot-converged route, no protocol.
- Time: deadlines are nondeterministic failure actions, and the order of simultaneously due deadlines is
  not modelled. There is no deadline in the drain (R2-DRAIN).
- Datagram order (D15-R4 admits empty-datagram reordering).

## `owner_flows`: control sessions, admission, attribution, the dial

State: `alloc` (CID table state), `gen` / `vmGen` / `vmUp` (lease and launch), `quiesced`, `claim` and `sess`
(the control-session `ClaimSet` and connections), `flows` (guest-opened flows), `hf` / `gf` / `dl` and the
control-message sets (host-opened flows and the dialer's results), `drvLive` / `bclaim` / `bcn` (the VM
driver's beacon), ghosts (`badPaired`, `crossMsg`, `heldByOther`, `resurrected`, `phantom`, `readyCross`,
`bHeldByOther`, `abortDiscarded`).

Actions: `Provision`, `VmStart`, `VmStop` (a VMM exit also resets every requested host dial of the stopped launch: the dial
returns `Unreachable { ECONNRESET }`), `Activate`, `ActivatePending`, `ActivateQuiesced`, `ActivateRetiring` (the refused activations),
`Teardown`, `Release`, `Quiesce`, `Restore`;
`GuestOpenSession`, `HostAcceptSession`, `HostReadOpen`, `HostClaimSession`, `HostSessionLoss`;
`GuestOpenFlow`, `HostAcceptFlow`, `HostReadReq`, `HostInstall`, `HostStartConnect`, `HostConnectDone`,
`HostConnectDiscard`, `HostFlowEnds`; the dial `DialOpen`, `DialNotServing`, `DialQuiesced`,
`DialNoSession`, `DialUnreachable`, `GuestAcceptV`, `GuestPairOrRefuse`, `GuestConnectTimesOut`, `HostRecvPaired`,
`HostRecvRefused`, `HostRecvTimedOut`, `HostDialDeadline`, `DialCancel`, `GuestNeverReplies`, `HostAbortAccept`, `HostRecvAbort`,
`GuestRecvAbort`, `GuestSeesHostClose`, `GuestInstalledError`, `GuestSessionLossObserved`,
`GuestQueueReset`; the beacon `GuestOpenBeacon`, `HostAcceptBeacon`, `HostReadReady`, `DriverStop`.

Invariants: `OneLiveSession` (L-3), `NoInstallBeforeRead` (H-1), `NoAdmitWhenClosed` (U-1),
`CurrentGeneration` (ATTR-TOKEN, ATTR-LIVE-CLAIM), `NoCrossLaunchMsg` (ATTR-TOKEN),
`NoCrossLaunchSessionHeld` (ATTR-LIVE-CLAIM), `ClosedIsTerminal` (U-5), `GuestFlowsDoNotOutliveSession`
(DG-5-F), `TypedRefusalChangesNothing` (INB-1; `NotServing`, `Quiesced`, `NoSession`, `Unreachable`), `DialDeadlineAfterGuestReply` and `NoPairedAfterDialEnded`
(INB-1, `GUEST_DIAL_PAIRING_DEADLINE` > `GUEST_INBOUND_CONNECT_TIMEOUT`), `BeaconCurrentLaunch`,
`BeaconNoCrossLaunchReady`, `BeaconNoCrossLaunchHeld` (ATTR-TOKEN / ATTR-LIVE-CLAIM for the beacon).
Temporal: `GuestHalfEnds` (U-5 with K-A4, DG-5-E), `SessionBecomesLive` (D18a; checked on instance `sess`: two control connections and two launches, no guest-opened flows and no dial slots, because the property mentions sessions only and the full `core` instance exceeded 900 s in TLC liveness checking), `ReadyReleased` (ADR-0157).

## `flow_end`: flow end and half-close drain

One paired flow between the guest owner (`g`) and the host owner (`h`); the opener is `g` (TcpConnect) or
`h` (a dial: the host side is the dialer's `GuestStream`, which has no cells and no drain race).
Direction d_x flows from the far end of x to the far end of the other side. Four drain cells `Vg`, `Vh`
(target = the flow vsock, `sent` = vsock `tx_cnt`) and `Og`, `Oh` (target = the outer TCP leg, `sent` =
TCP `write_seq`).

State (`f`, one record): per side `open`, `dead`, `loss`, `rcv` / `fin` / `rst` (the far end), `eofSeen` /
`cls` (the owner's classification: none / clean / abort), `shut`, `vdead`, `dlv` (bytes delivered to the
other side's redirect), `wire` (SHUTDOWN ops in flight), `ring` / `lateRec` / `lost` / `seenLost` (the
kernel ring and the lost counter), `gotShut`, `uWake` (an end with no record); per cell `sent`, `cell`
`{sent, goal, armed, fired}`, `dr` (the owner's drain phase), `kchk`, `gone`, `refusedOnce`, `armShort`,
`seen`; per side kernel `skip1` / `skip2` / `miss` (the two fexit programs and the recursion-miss counter);
ghosts `eofDeliv`, `farReset`, `srcAborted`, `aborted`, `faulted`.

Actions, owner: `HcPair`, `HcRecvPaired`, `OwnSeesEof`, `OwnGoal`, `OwnArm`, `OwnRead`, `OwnBackstop`,
`OwnShut`, `HcLateClose`, `OwnRing`, `OwnSeeLost`, `OwnAuditTick` (H-3: the weakly fair periodic counter read on the 1 s audit tick),
`OwnUnannounced`, `HcLossObserved`. Kernel: `KSend`,
`KCheck`, `KRefuse`, `KDeliver`, `KRecv`, `KLateRecord` (hazard only: it fires only under `wakeBeforeRecord`, the A5-off
hazard; the design never takes it). Environment: `FarWrite`, `FarFin`, `FarRst`,
`EnvMiss1`, `EnvMiss2`, `EnvRingFull`, `EnvClassifierSkip`, `EnvStaleDrain`, `EnvStaleEnd`, `EnvTargetGone`, `EnvUnannounced`,
`HostOwnerDies`, `GuestPid1Crash`, `HcSessionDrop`, and (hazard only) `EnvPerfSkip`.

Invariants: `NoEarlyShutdown` (R2-DRAIN: no direction is shut down before every byte forwarded toward it was
sent), `NoEofOnPeerLoss` (R2-END, A4, A6: a far end reads EOF only if the other far end half-closed),
`AbortNeverCleanEof` (R2-END), `EndedFlowHoldsNoArmedCell` (R2-DRAIN), `NoCompletedFlowStaysOpen` (DG-5-C),
`AbortOnlyOnFault` (R2-END, A5: a clean flow aborts only on a fault; a classifier skip, `EnvClassifierSkip`, is a fault). Temporal: `EveryEndResolves` (R2-DRAIN: no
drain deadline; the end of a direction completes on its record or its backstop read, or the flow aborts; the counters are read
at each record dispatch and on the 1 s audit tick, H-3), `CleanHalfCloseCompletes` (R2-END).

Loss counters by record class (H-2): a dropped record increments exactly one per-cell counter. `ring_lost` counts a lost
flow-end / drain / redirect-refused record (`flow_end` `lost`; the owner aborts every flow whose count rose, A7).
`empty_lost` counts a lost EmptyDatagram notice (`udp_empty` `lost`; the owner compensates the live flow of the current
epoch and never aborts). The two modules model the two classes separately, so a lost empty notice cannot abort an
association; the exclusivity of the classes is a property of the kernel program that writes the record (UE-1: "exactly
one notice or one lost-counter increment"), checked at Tier 3.

## `flow_quota`: the per-allocation quota counts draining flows

The dial's `Capacity` outcome is modelled here (the allocation quota), not in `owner_flows`, which does not check capacity.

Three allocations, quota 1, two flow slots (node capacity below the sum of the quotas: the model assumes no relation). A slot is free / active / draining (drain pending) / half. State:
`slot` (`{own, st, pend}`), ghosts `g` (`refusedUnder`, `clash`, `refused`, `otherAdmitted`). Actions:
`Admit`, `Refuse` (the typed capacity outcome, no state change), `HalfClose`, `DrainDone` (absent for a
guest that withholds credit), `EndFlow`, `Abort`. Invariants: `QuotaBound`,
`QuotaRefusalOnlyAtOwnQuota`, `DrainingFlowHoldsItsSlot` (R2-DRAIN, ADR-0147).
Property: a draining flow holds its quota slot exactly like an open flow; a guest withholding credit holds at
most its own quota. No node-capacity assumption.

## `cid_lease`: CID lease, reclamation, guest reset

Offsets, workloads, the kernel's vhost-vsock holder table `dev` (free / an allocation's VMM / an orphan of the
previous process), `lease`, `cursor`, `al` (placement), `up` / `reclaimed` / `bootLeft` (serve process). Actions:
`Assign`, `Launch`, `Stop`, `VmmCrash`, `GuestReset`, `Release`, `Redeploy`, `Crash`, `Restart`, `Reclaim`,
`OrphanExit`. Invariants: `CidUnique` (ADR-0156), `NoLaunchClash` (CID-NOCLAIM), `LeaseOutlivesVmm`
(release-last, ADR-0133), `AssignAfterReclaim` (NO-SURVIVE), `GuestResetEndsVmm` (RESET-EXIT). Temporal:
`EventuallyPlaced`.

## `quiescence`: counted hold and boot refusal

`up` / `stop` / `table` (serve process and the shared mTLS nft table), `cnt` (hold count), `latch`, `rec`
(per-component recovery), `dmg`, `att`, `repLeft`, `flowsOpen`, ghosts. Actions: `Damage`, `OpenFlow`,
`Crash`, `Restart` (boot step 8 writes the table, or boot is refused), `RecQuiesce`, `RecRepair`, `RepFail`,
`RecRedo`, `RecRestore`, `RecDropHold` (increments the leaked-hold count, derived: holds dropped without a restore),
`FailStop` (the recovery supervisor: attempt budget spent, or leaked-hold count above zero). Invariants:
`LeakedHoldKeepsForwardingQuiesced`, `LatchIffCount`, `CountCoversRecoveries`,
`NoReopenBeforeRepair`, `NoRepairWithoutHold`, `QuiescedHoldsNoFlow` (QHOLD), `ServeRunsOnlyWithTable`
(ADR-0184). Temporal: `C1EpisodeEnds`, `C2EpisodeEnds`, `C1Repaired` (QHOLD, ADR-0124, UP-7).

## `udp_slots`: guest UDP slots

Unchanged by this round: slot claim (`SlotHeldOnce`), release (`ReleasedSlotCarriesNoData`), route currency
(`RouteCurrent`), SLOT-ABORT (`LatePairedAborted`), U-2 (`NoReassociationWithoutNewDatagram`), CELL-RECREATE
(`NoStaleGenerationStart`); temporal `ReleasedSlotBecomesFree`.

## `udp_empty`: empty-datagram notice

One flow cell reused by successive associations. State: `live`, `ep` (cell epoch), `owed` (empty frames the
verdict dropped), `sent` (real empty datagrams the owner sent), `ring`, `lost` / `seen`. Actions: `Start`,
`Empty(full)`, `OwnNotice`, `OwnReadLost`, `Remove`, `Idle`. Invariants: `NoSpuriousEmpty` (A2 shape: a
notice of another epoch sends nothing), `LostCountResetAtCellInitialisation` (the cell's lost count is reset when it is initialised for a new
flow; compensation applies only to the live flow of the current epoch; removal reads nothing). Temporal: `EveryEmptyEventuallySent`.

## Critical scenarios ↔ named runs

Every run holds (`expect = "holds"`) in `checks.toml`; the run-based hazards expect a violation.

### `owner_flows` (instance `flows_ok` unless noted)

| Scenario | Run |
|---|---|
| happy path: lease, VM start, control session claimed on the live token, activation | `ControlSessionOpensAndCidActivates` |
| guest-opened flow admitted, request read in full, installed, connected, paired | `GuestOpenedFlowIsAdmittedReadInstalledAndPaired` |
| request in two pieces: nothing installed before the whole request is read (H-1) | `RequestReadInPiecesInstallsOnlyAfterFullRead` |
| two first connects race: one live session (L-3) | `RacingFirstConnectsLeaveOneLiveSession` |
| session loss aborts flows, reconnect restores | `SessionLossAbortsFlowsAndReconnectRestores` |
| quiescence aborts flows, refuses admission, restore reopens | `QuiesceAbortsFlowsAndRefusesAdmissionUntilRestore` |
| activation waits for quiescence to end (gate boundary) | `ActivationWaitsForQuiescenceToEnd` |
| in-flight connect of an aborted flow is discarded (U-1) | `InFlightConnectOfAbortedFlowIsDiscarded` |
| attributed flow ends, CID keeps serving | `AttributedFlowEndsAndCidKeepsServing` |
| teardown aborts everything, CID leased again | `TeardownAbortsEverythingThenCidIsLeasedAgain` |
| dead launch's queued session closed at read (K-A2') | `StaleLaunchSessionIsClosedAtReadAndDoesNotPoisonClaim` |
| teardown between accept and read (ATTR-LIVE-CLAIM) | `TeardownBetweenAcceptAndReadClosesWithoutClaim` |
| dead launch's queued flow closed at request read (ATTR-TOKEN) | `StaleLaunchFlowIsClosedAtRequestReadWithNoMessage` |
| VMM death makes a live session dead, then loss is processed | `VmDeathMakesLiveSessionDeadThenLossIsProcessed` |
| dial pairs and returns the stream (INB-1) | `DialPairsAndReturnsStream` |
| dial refused: typed error | `DialRefusedReturnsTypedError` |
| dial aborted before the guest accepts V_g (U-5, K-A4) | `DialAbortedBeforeGuestAcceptsEndsByHostClose` |
| queued V_g reset by the host's close (K-A4) | `DialQueuedVgIsResetByHostClose` |
| late Paired for a closed flow discarded (U-5) | `LatePairedForClosedHostFlowIsDiscarded` |
| session loss aborts the dial on both sides | `SessionLossAbortsDialOnBothSides` |
| quiescence aborts the dial and sends Abort | `QuiesceAbortsDialAndSendsAbort` |
| session loss aborts every flow before any reconnect (DG-5-F) | `SessionLossAbortsAllFlowsBeforeReconnect` (`flows_two`) |
| installed V_g, host close: silent end (DG-5-E) | `InstalledVgHostCloseEndsFlowSilently` |
| installed V_g error: Abort (DG-5-E) | `InstalledVgErrorAbortsWithAbort` |
| guest connect timeout: `TimedOut` | `DialTimedOutReturnsTypedError` |
| `NotServing`, `Quiesced`, `NoSession`: typed, no state change | `DialNotServingIsATypedResultWithNoStateChange`, `DialWhileQuiescedIsATypedResultWithNoStateChange`, `DialWithoutLiveSessionIsATypedResultWithNoStateChange` |
| VM gone, session loss not yet observed by the owner: `Unreachable` (vsock connect fails), no state change; not `NoSession` | `DialToAGoneVmIsUnreachable` |
| VM gone, session loss observed by the owner: `NoSession` (the owner never reaches the vsock connect), no state change; not `Unreachable` | `DialAfterObservedSessionLossWithVmGoneReturnsNoSession` |
| pairing deadline above the guest timeout (INB-1) | `PairingDeadlineWaitsForTheGuestAnswer`, `PairingDeadlineFiresOnlyAfterGuestTimeoutAndLateReplyIsDiscarded` |
| VMM exit during an in-flight dial: the orphan sweep resets the host dial vsock; `Unreachable { ECONNRESET }` at once, not `PairingDeadline`, no message (H-1, K-C4) | `VmExitDuringDialReturnsUnreachableReset`, `VmExitDuringDialLeavesNoPairingDeadline` |
| live VM whose guest never replies: `PairingDeadline`, V_h closed, `Abort` on the live session | `PairingDeadlineEndsDialToALiveVmWhoseGuestNeverReplies` |
| in-flight dial ended by the owner reports the admission variant of its cause (L-9): quiescence `Quiesced`, teardown `NotServing`, observed session loss `NoSession`; never `Unreachable { ECONNABORTED }` | `QuiesceAbortsDialAndSendsAbort`, `TeardownEndsPendingDialWithNotServing`, `SessionLossAbortsDialOnBothSides` |
| the dialer drops the dial future: ends like the pairing deadline, no result | `DroppedDialFutureEndsLikeThePairingDeadline` |
| a dial after the guest reconnects pairs (G-V3 dial row 5, L-7) | `DialAfterReconnectPairs` (`flows_two`) |
| activation while the control session is pending: refused, changes nothing, activates once the session is live (G-V3, L-7) | `ActivationIsImpossibleWhileTheSessionIsPending`, `ActivationWithSessionPendingReturnsPendingThenActivates` |
| activate after teardown (`Retiring`): refused, changes nothing, never activates (G-V3, L-7) | `ActivateAfterTeardownIsRefusedAndChangesNothing`, `ActivateAfterTeardownNeverActivates` |
| beacon happy path (`flows_beacon`) | `BeaconReadyReleasedThenDriverStopReleasesClaim` |
| beacon after driver.stop; stop between accept and read; stale launch; second connection; VMM death | `BeaconAfterDriverStopIsRefusedAtAccept`, `DriverStopBetweenAcceptAndReadyClosesWithoutClaim`, `StaleLaunchBeaconIsClosedAtReadyAndDoesNotPoisonClaim`, `SecondBeaconConnectionOfSameLaunchIsHeld`, `VmDeathClosesUnreadBeaconConnection` |

### `flow_end` (instance `end_all`)

| Scenario | Run |
|---|---|
| happy path, guest-opened flow: both directions half-close, drain, close cleanly | `TcpConnectBothDirectionsHalfCloseThenCloseCleanly` |
| happy path, dial: the dialer reads the guest's end directly | `TcpAcceptDialerBothDirectionsHalfCloseThenCloseCleanly` |
| a clean close's 0x3 is benign while the peer's drain is pending | `CleanCloseZeroX3IsBenignWhilePeerDrainPending` |
| both fexit programs miss the last send; the level backstop completes the drain (A1) | `RecursionMissOnLastCallIsCoveredByBackstop` |
| one fexit program misses; the other covers; the kernel fires (A1) | `SingleProgramMissIsCoveredByTheOtherAndKernelFires` |
| ring full drops the completion record; the owner aborts on the rise it reads (A7) | `RingDropAbortsTheFlowAndResetsBothFarEnds`, `RingDropIsNotActedOnBeforeTheOwnerReadsTheCount` |
| both programs skip the last send after arm; the audit tick's counter read finds it and the backstop completes (A1, H-3) | `BothSkipAfterArmIsFoundByTheAuditTick` |
| the classifier is skipped on same-program recursion: the clean end becomes an unannounced-end abort (A5, M-2, a fault) | `ClassifierSkipTurnsACleanEndIntoAnAbort` |
| a record of another epoch is discarded (A2) | `StaleEpochRecordIsDiscarded`, `StaleEpochEndRecordIsDiscarded` |
| redirect refused: one record, abort (A3) | `RefusedRedirectAbortsTheFlow` |
| vsock end with no record: unannounced end, abort (A4) | `UnannouncedEndAbortsWithReset` |
| TCP source reset read as EOF: abort, never clean EOF (A6) | `TcpSourceResetIsAbortNotCleanEof` |
| far end resets after Paired | `FarEndResetAfterPairedAbortsBothOwners` |
| serve crash: the guest gets 0x3 and aborts | `HostOwnerDeathGivesGuestAnAbort` |
| guest PID 1 crash: VMM exits, no record, the host owner aborts (RESET-EXIT) | `GuestPid1CrashEndsHostFlowWithNoClassificationRecord` |
| control session drops: both owners abort (DG-5-F) | `SessionDropAbortsFlowAtBothOwners` |
| outer EOF before Paired is carried out after Paired | `EarlyOuterEofIsCarriedOutAfterPaired` |
| the 0x3 record is lost to a full ring: lost counter and the record-less end abort | `LostAbortRecordStillEndsTheFlow` |

### `flow_quota` (`quota_ok`), `udp_empty` (`empty_ok`)

| Scenario | Run |
|---|---|
| admission stops at the allocation quota (typed capacity outcome) | `AdmissionStopsAtTheAllocationQuota` |
| a draining flow still counts | `DrainingFlowStillCountsAgainstTheQuota` |
| a draining flow holds its quota slot like an open flow; a withholding guest holds at most its own quota | `WithholdingGuestHoldsAtMostItsOwnQuota` |
| drain completes, flow ends, slot reused | `CompletedDrainAndEndReleaseTheSlot` |
| abort of a draining flow releases slot and cell | `AbortOfDrainingFlowReleasesSlotAndCell` |
| notice sends one empty datagram | `EmptyFrameNoticeSendsOneEmptyDatagram` |
| stale-epoch notice discarded after cell reuse | `StaleEpochNoticeIsDiscardedAfterCellReuse` |
| ring drop compensated by the counted number | `RingDropIsCompensatedByTheCountedNumber` |
| notice and compensation send each dropped frame once | `NoticeAndCompensationTogetherSendEachDroppedFrameOnce` |
| lost count reset at cell reuse; a removed association is not compensated | `LostCountIsResetWhenTheCellIsReusedForANewFlow` |

### `cid_lease` (`lease_ok`, `lease_conf_ok`), `quiescence` (`quiesce_ok`, `quiesce_live`)

| Scenario | Run |
|---|---|
| next-fit assign, the VMM opens its own device | `AssignThenVmmOpensItsOwnDevice` |
| release while the VMM runs is refused; stop exits the VMM, release is last | `ReleaseWhileTheVmmRunsIsRefused`, `StopExitsTheVmmThenReleaseIsLast` |
| next-fit continues from the cursor | `NextFitContinuesFromTheCursor` |
| guest reset ends the VMM, placement retries (RESET-EXIT) | `GuestResetEndsTheVmmAndPlacementRetries` |
| VMM crash frees its CID | `VmmCrashFreesItsCidAndPlacementRetries` |
| serve crash: the orphan is reclaimed before any assign (NO-SURVIVE) | `ServeCrashOrphanIsReclaimedBeforeAnyAssign`, `ReclamationReleasesTheOrphanDeviceThenAssignProceeds` |
| orphan exits by itself before reclamation; unlaunched lease leaves nothing | `OrphanExitsByItselfBeforeReclamation`, `ServeCrashWithUnlaunchedLeaseLeavesNothingToReclaim` |
| one recovery quiesces, repairs, restores (QHOLD) | `SingleRecoveryQuiescesRepairsRestores` |
| two recoveries overlap: reopen only at count zero | `TwoOverlappingRecoveriesKeepForwardingQuiescedUntilTheLastRestore`, `NoFlowOpensWhileCountedHoldRemains`, `TwoOverlappingRecoveriesReopenOnlyAtZero` |
| a dropped hold keeps forwarding quiesced, then fail-stop | `DroppedHoldKeepsForwardingQuiescedThenFailStops` |
| repair failure / failed post-repair audit count as attempts (UP-7) | `RepairFailureCountsAttemptThenRepairs`, `PostRepairAuditFailureCountsAttempt` |
| spent budget fail-stops, restart converges (ADR-0124) | `SpentBudgetFailStopsAndRestartConverges` |
| crash during a recovery | `CrashDuringRecoveryThenRestartRecovers` |
| failed boot write of the shared table refuses boot, retry succeeds (ADR-0184) | `FailedBootTableWriteRefusesBootThenRetrySucceeds` |

### `udp_slots` (`slots_ok`, `slots_conf`)

`DatagramAssociationClaimsPairsAndRoutes`, `TwoSocketsClaimDistinctSlots`,
`RefusedDiscardsParkedFramesAndOnlyNewDatagramReassociates`,
`HostAbortOfLiveAssociationEndsItAndKeepsSlotOwned`, `StaleGenerationReadinessEventIsDiscarded`,
`LatePairedForReleasedSlotIsAnsweredWithAbort`, `LostReleaseReportIsFoundByAudit`,
`SocketCloseReleasesEverySlotItOwns`, `IdleSlotReleaseEndsAssociation`,
`AuditTickReleasesOrphansAndIdleSlotsInOneStep`, `PairingDeadlineAbortsAndLatePairedIsDiscarded`,
`RecycledSlotNeverDeliversPreviousOwnersFrames`.

## Assumptions (kernel, VMM, network, environment) and what discharges them

Nothing below is a modelled fact. Each is an input the model takes as given.

| Id | Assumption | Discharged by |
|---|---|---|
| K-A1 | peer-CID attribution is authentic: vhost stamps the device CID, a forged `src_cid` is dropped | proven: V-10 (`findings-v10.md`, holds) |
| K-A2' | at VMM death, accepted host connections fail at their next I/O; a connection still in a host accept queue stays queued and is accepted later, under the next lease, with its buffered first record | proven: V-22 (`findings-v22.md`, P-37) |
| K-A3 | a flow socket installed before its request is read hands the request bytes to the verdict, which drops them | proven: P-23 (H-1) |
| K-A4 | the host's close of a host-opened V_h (aborted or session-lost dial) reaches the guest and tears down V_g, accepted or still in the accept queue | proven: V-24 (`findings-v24.md`, P-38) |
| K-C1 | a duplicate CID is refused at device creation, the first holder unaffected; a CID is free right after the last reference closes | proven: P-16, V-30 Q2d (`findings-v30-guest-reset.md`). Unreachable in the design (lease uniqueness); reached only by hazards |
| K-C4 | the VMM is the only holder of its vhost-vsock fd; no Overdrive process keeps a copy past VMM exit, so the device is released and host flows reset at VMM exit | proven: V-30 Q2c, Q2d |
| K-C5 | Boot reclamation kills every VMM the new process does not supervise before any `assign`, and a reclaimed VMM's device is released before its cgroup can be removed. Source: an exiting task drops its files (kernel/exit.c:971 @ v7.0) and runs its task work (kernel/exit.c:976 @ v7.0), where the final fput of a user task is deferred (fs/file_table.c:528-530 @ v7.0) and reaches vhost_vsock_dev_release; cgroup_rmdir waits in cgroup_drain_dying until every task has passed cgroup_task_dead (kernel/cgroup/cgroup.c:6309 @ v7.0). OPEN: the Tier-3 serve-restart reclamation scenario V-RECLAIM; not contract-shaping | OPEN validation item V-RECLAIM (Tier-3 serve-restart reclamation scenario of NO-SURVIVE; not contract-shaping) |
| K-C6 | a guest reset (PID-1 crash, triple fault) ends the VMM in the pinned fork | validation item of ADR-0182: fork build with the change, PID-1 crash under `panic=1` and triple fault; not contract-shaping, blocks DELIVER of the fork step (V-30 Q3 is a reading, not a result) |
| A1 | each `fexit(skb_send_sock)` program may miss a call; the other program's cumulative read covers it; a call both miss is covered by the level backstop | V-31 item 4 (3,000/3,000 at 30..90 % forced skips; 96,000 real flows, 0 stuck); the both-skip residual is a validation note of ADR-0183 |
| A5 | with a raw tracepoint the SHUTDOWN / RST record precedes the EOF wake and is not skipped for another program's activity; it is skipped only on the same program's recursion on one CPU (`kernel/trace/bpf_trace.c:2080-2083` @ v7.0), counted in the classifier's `recursion_misses`; such a skip is an A4 unannounced-end abort (`EnvClassifierSkip`, a fault) | V-31 (0 of 1,312 reps raw, 7 of 1,173 perf); microVM batch of the integrated owner on the final raw-tracepoint build (validation item of ADR-0183, not contract-shaping) |
| A6 | the TCP-source end check classifies a destination reset | V-31 item 2b (0 clean EOF after reset in 12,000 trials) |
| A8 | at a classified 0x2 the survivor's forwarded count equals the peer's final-hop sent | T4 (`findings-drain-t1-t6.md`) |
| A9 | the `fentry(sock_poll)` backstop runs in the owner's own syscall and is not skipped | V-31 item 4; same residual note as A1 |
| K-B1 | closing T_g / Q_g is observed by the host as the association's end, even when accepted after the guest closed | P-22; P-37 |
| K-B2 | the guest `sock_release` report may be lost; the level-triggered audit finds the slot | R5-14 (Tier-3) |
| K-B3 | recreating a slot's cells discards parked frames; emptying them does not | V-5(c) (P-41) |
| UE-1 | The verdict never passes or redirects a zero-length skb; a dropped empty frame yields exactly one notice or one lost-counter increment | V-29 items 1b, 2, 3 (findings-v29-udp-empty.md) |
| UE-2 | `send(fd, NULL, 0)` on the association's connected host UDP socket emits one empty datagram | V-29 (the udp_bpf sendmsg citation is the ADR-0165 editor's) |
| Q-1 | A task exit of a runtime component is detected by the 1 s audit; a repair that returns `Err` is a failed attempt | OPEN validation item: DST invariant under UP-7 (ADR-0124) |

## Conformance granularity: `flow_end`

For `flow_end` the driving port is the owner's event dispatch, one call per owner
   input event (direction EOF, flow-event ring record, recursion-miss rise, ring-lost rise, unannounced
   end). The owner's drain sub-steps are calls on the driven `guest_flow` forwarder port, in the pinned order
   store goal, arm, read sent, backstop poll, shutdown; the conformance driver schedules the kernel actions
   at those call boundaries through the Sim forwarder, the named arbiter for kernel `sent`, ring contents
   and fexit skip state. Tier-3 pair: V-31.

## Underspecified points

None open. The architect resolutions of the 2026-10-11 round are modelled:

1. INB-1: a dial to an `Active`, not quiesced allocation whose control session is not live is the typed result
   `NoSession` (immediate, no state change; `DialNoSession`, `TypedRefusalChangesNothing`). The load-bearing
   rule that a dial never opens without a live session is covered by the existing `dialNoAdmission` hazard.
   The order of decision (2026-10-13) follows only facts the owner observes: Active (else `NotServing`) →
   not quiesced (else `Quiesced`) → the control session live as the owner observes it (else `NoSession`,
   whatever `vmUp`) → the vsock connect, whose failure is `Unreachable`. The owner does not observe `vmUp`:
   `DialUnreachable` requires the session live to the owner (`sessLive`, which counts a `dead` session whose
   loss is unprocessed) and the VM gone; `DialNoSession` requires only `not(sessLive)`.
2. QHOLD: a dropped hold increments the owner's leaked-hold count; the recovery supervisor (ADR-0124)
   fail-stops serve when it is above zero (`FailStop`; hazard `leakIgnored`).
3. UDP-EMPTY: the read-before-removal rule is withdrawn; the cell's lost count is reset at cell
   initialisation for a new flow (hazard `noResetAtReuse`).
4. R2-DRAIN: no isolation claim beyond the quota; the model assumes no relation between node capacity and the
   sum of quotas.
5. H-1 (r1): a VMM exit while a dial awaits its pairing reply: the device-release orphan sweep resets the host dial
   vsock (`drivers/vhost/vsock.c:745-750` @ v7.0, `sk_err = ECONNRESET`); `VmStop` ends every requested host dial of the
   stopped launch with `Unreachable { ECONNRESET }`, no message. `PairingDeadline` remains for a running VM whose guest
   never replies.
6. L-9 (r1): a dial in flight that the owner ends reports the admission variant of its cause: quiescence `Quiesced`,
   allocation teardown or retire `NotServing`, observed control-session loss `NoSession`. A vsock connect or I/O error
   (VM or device gone) is `Unreachable { errno }`. No owner-initiated end is `Unreachable { ECONNABORTED }`.
7. H-3 (r1): the owner reads `recursion_misses`, the ring-drop and the per-cell lost counters at every flow-event
   dispatch and on the fixed 1 s audit tick (`OwnAuditTick`, weakly fair). No drain deadline: a direction completes on
   its record or its backstop read. Hazard `end_bugNoPeriodicRead`: with no periodic read a both-skip on the last send
   strands the direction (`CleanHalfCloseCompletes`, `EveryEndResolves`).
8. M-2 (r1): `EnvClassifierSkip` is a fault; `AbortOnlyOnFault` and `EveryEndResolves` are checked on instance
   `end_skip` with it enabled.
9. M-4 (r1): `owner_flows` models neither capacity nor the quota; no Quint module models the
   `GuestStreamDial` capacity refusal (the dial's `Capacity` outcome); it is discharged by the seeded-sim scenario
   R5-53 against the production owner (`docs/feature/netns-density-295/feature-delta.md`). `flow_quota`
   (`AdmissionStopsAtTheAllocationQuota` and the quota invariants) models the admission quota only. Dial cancellation is `DialCancel`
   (`DroppedDialFutureEndsLikeThePairingDeadline`).

## Hazards

Each row removes one design rule (or kernel assumption) and shows the checker finds a counterexample.

| Check | Instance | Rule removed | Property / run that fails |
|---|---|---|---|
| `owner-flows-hazard-split-claim` | `flows_bugSplitClaim` | L-3 off: session check and claim are two steps | `OneLiveSession` |
| `owner-flows-hazard-install-early` | `flows_bugInstallEarly` | H-1 off: install before the request is read | `NoInstallBeforeRead` |
| `owner-flows-hazard-admit-split` | `flows_bugAdmitSplit` | U-1 off: admission check and flow-table registration are two steps | `NoAdmitWhenClosed` |
| `owner-flows-hazard-paired-no-recheck` | `flows_bugPairedNoRecheck` | U-1 off: the step-6 continuation ignores that the flow was closed | `NoAdmitWhenClosed` |
| `owner-flows-hazard-no-abort-on-loss` | `flows_bugNoAbortOnLoss` | session loss does not abort the CID's flows | `NoAdmitWhenClosed` |
| `owner-flows-hazard-teardown-skips-flows` | `flows_bugTeardownSkipsFlows` | teardown does not abort the CID's flows | `CurrentGeneration` |
| `owner-flows-hazard-late-paired-resurrects` | `flows_bugLatePairedResurrects` | U-5 off: the host acts on a Paired for a flow it already closed | `ClosedIsTerminal` |
| `owner-flows-hazard-peer-cid-only` | `flows_bugPeerCidOnly` | ATTR-TOKEN off: attribution by peer CID alone (the peer-CID-only rule), under K-A2′ (V-22, P-37) | `CurrentGeneration` |
| `owner-flows-hazard-peer-cid-only-message` | `flows_bugPeerCidOnly` | ATTR-TOKEN off: attribution by peer CID alone (the peer-CID-only rule), under K-A2′ (V-22, P-37) | `NoCrossLaunchMsg` |
| `owner-flows-hazard-claim-before-token-held` | `flows_bugClaimBeforeToken` | ATTR-LIVE-CLAIM order off: the session claim is taken at accept, before the ControlOpen is read | `NoCrossLaunchSessionHeld` |
| `owner-flows-hazard-no-claim-recheck-held` | `flows_bugNoClaimRecheck` | ATTR-LIVE-CLAIM off (comparison with the lease token, its counterexample kept as teeth): the CID state is checked at accept only, and the claim step compares with the lease token, Retiring included | `NoCrossLaunchSessionHeld` |
| `owner-flows-hazard-no-claim-recheck-current` | `flows_bugNoClaimRecheck` | ATTR-LIVE-CLAIM off (comparison with the lease token, its counterexample kept as teeth): the CID state is checked at accept only, and the claim step compares with the lease token, Retiring included | `CurrentGeneration` |
| `owner-flows-alt-teardown-closes-reading-safety` | `flows_altTeardownClosesReading` | rejected alternative to ATTR-LIVE-CLAIM: teardown also closes connections in `reading`, and the claim step compares with the lease token (no live check) | `Safety` |
| `owner-flows-env-guest-misses-host-close` | `flows_envGuestMissesHostClose` | K-A4 dropped: the guest never observes the host's close of V_h (V-24) | `GuestHalfEnds` |
| `owner-flows-alt-tombstone-misses-close` | `flows_altTombstoneMissesClose` | candidate: the guest tombstones an Abort for an unaccepted flow, with K-A4 dropped | `GuestHalfEnds` |
| `owner-flows-hazard-reconnect-before-loss-abort` | `flows_bugReconnectBeforeLossAbort` | DG-5-F off: the guest may open a new control session while a flow of the lost session is still open (the loss is observed per flow, later, and a reconnect can overtake it) | `GuestFlowsDoNotOutliveSession` |
| `owner-flows-hazard-no-installed-watch` | `flows_bugNoInstalledWatch` | DG-5-E off: an installed, unrouted V_g is not watched for terminal readiness (the host's close of V_h and an error on it go unnoticed until the flow is routed) | `GuestHalfEnds` |
| `owner-flows-hazard-beacon-no-live-recheck` | `flows_bugBeaconNoLiveRecheck` | ATTR-LIVE-CLAIM off for the beacon: READY compared with the lease's token, without the live check | `BeaconCurrentLaunch` |
| `owner-flows-hazard-beacon-no-live-recheck-held` | `flows_bugBeaconNoLiveRecheck` | ATTR-LIVE-CLAIM off for the beacon: READY compared with the lease's token, without the live check | `BeaconNoCrossLaunchHeld` |
| `owner-flows-hazard-beacon-peer-cid-only` | `flows_bugBeaconPeerCidOnly` | ATTR-TOKEN off for the beacon: READY attributed by peer CID alone | `BeaconNoCrossLaunchReady` |
| `owner-flows-hazard-beacon-peer-cid-only-current` | `flows_bugBeaconPeerCidOnly` | ATTR-TOKEN off for the beacon: READY attributed by peer CID alone | `BeaconCurrentLaunch` |
| `owner-flows-hazard-beacon-driver-stop-keeps-claim` | `flows_bugDriverStopKeepsClaim` | ADR-0157 off: driver.stop closes the beacon session but does not release the CID's claim; the next launch's READY is held out | `ReadyReleased` |
| `owner-flows-hazard-dial-no-admission` | `flows_bugDialNoAdmission` | the owner opens a dial without the Active ∧ live session ∧ not-quiesced check | `NoAdmitWhenClosed` |
| `owner-flows-hazard-dial-no-session-check` | `flows_bugDialNoSessionCheck` | INB-1 step 3 alone off: a dial opens to an Active, not quiesced CID with no live control session (the Active and quiesce checks kept) | `NoAdmitWhenClosed` |
| `owner-flows-hazard-refusal-registers` | `flows_bugRefusalRegisters` | INB-1 off: a typed admission refusal registers (and closes) the dial slot instead of changing nothing | `TypedRefusalChangesNothing` |
| `owner-flows-hazard-activate-no-guard` | `flows_bugActivateNoGuard` | G-V3 off: activate marks the CID Active without a live control session or while quiesced | `ActivateOnlyWithLiveSession` |
| `owner-flows-hazard-owner-deadline-shorter` | `flows_bugOwnerDeadlineShorter` | GUEST_DIAL_PAIRING_DEADLINE not above GUEST_INBOUND_CONNECT_TIMEOUT: the owner may abandon a dial the guest can still answer | `DialDeadlineAfterGuestReply` |
| `owner-flows-hazard-owner-deadline-shorter-late-paired` | `flows_bugOwnerDeadlineShorter` | GUEST_DIAL_PAIRING_DEADLINE not above GUEST_INBOUND_CONNECT_TIMEOUT: the owner may abandon a dial the guest can still answer | `NoPairedAfterDialEnded` |
| `flow-end-hazard-no-tcp-end-check` | `end_bugNoTcpEndCheck` | A6 off: every TCP-source EOF is a clean half-close (a destination reset is delivered as a clean EOF) | `NoEofOnPeerLoss` |
| `flow-end-hazard-eof-is-half-close` | `end_bugEofIsHalfClose` | R2-END off: a vsock end is a half-close whatever its class (a 0x3 abort is read as a half-close) | `AbortNeverCleanEof` |
| `flow-end-hazard-eof-is-half-close-peer-loss` | `end_bugEofIsHalfClose` | R2-END off: a vsock end is a half-close whatever its class (a 0x3 abort is read as a half-close) | `NoEofOnPeerLoss` |
| `flow-end-hazard-any-shutdown-aborts` | `end_bugAnyShutdownAborts` | R2-END off: a 0x3 aborts even when no direction is open (a clean close aborts the peer's pending drain) | `run CleanCloseZeroX3IsBenignWhilePeerDrainPending` |
| `flow-end-hazard-no-unannounced-abort` | `end_bugNoUnannouncedAbort` |  | `EveryEndResolves` |
| `flow-end-hazard-perf-tracepoint-strands-flow` | `end_bugPerfTracepointNoA4` | A4 off: an end with no classification record is ignored A4 off together with A5 failing: a skipped SHUTDOWN record strands the flow | `EveryEndResolves` |
| `flow-end-hazard-perf-tracepoint` | `end_bugPerfTracepoint` | A5 off (perf-attached tracepoint): a SHUTDOWN record can be skipped silently; the end is then an unannounced end and aborts (a spurious abort of a clean flow) | `AbortOnlyOnFault` |
| `flow-end-hazard-wake-before-record` | `end_bugWakeBeforeRecord` | A5 off: the EOF wake can precede the record | `AbortOnlyOnFault` |
| `flow-end-hazard-no-epoch` | `end_bugNoEpoch` | A2 off: a record of another epoch is acted on | `NoEarlyShutdown` |
| `flow-end-hazard-no-redirect-notice` | `end_bugNoRedirectNotice` | A3 off: a refused redirect writes no record | `EveryEndResolves` |
| `flow-end-hazard-ignore-ring-drop` | `end_bugIgnoreRingDrop` | A7 off: the owner ignores a raised lost counter | `EveryEndResolves` |
| `flow-end-hazard-no-periodic-read` | `end_bugNoPeriodicRead` | H-3 off: the counters are read only at a record dispatch, never on the audit tick: a both-skip on the last send strands the direction | `CleanHalfCloseCompletes` |
| `flow-end-hazard-no-periodic-read-every-end` | `end_bugNoPeriodicRead` | H-3 off (as above) | `EveryEndResolves` |
| `flow-end-hazard-no-periodic-read-run` | `end_bugNoPeriodicRead` | H-3 off (as above) | `run BothSkipAfterArmIsFoundByTheAuditTick` |
| `flow-end-witness-classifier-skip-abort` | `end_skip` | witness (expect violation of `WitnessNoAbort`): the classifier skip turns a clean end into an abort, reachable | `WitnessNoAbort` |
| `flow-end-hazard-no-backstop` | `end_bugNoBackstop` | A1 off: no level backstop | `EveryEndResolves` |
| `flow-end-hazard-per-call-counter` | `end_bugPerCallCounter` | A1 off: the cell counts calls instead of reading the cumulative kernel quantity (two bytes) | `EveryEndResolves` |
| `flow-end-hazard-arm-first` | `end_bugArmFirst` | R2-DRAIN off: the owner arms before it stores the goal (two bytes) | `NoEarlyShutdown` |
| `flow-end-hazard-read-before-arm` | `end_bugReadBeforeArm` | R2-DRAIN off: the owner reads sent before it arms | `EveryEndResolves` |
| `flow-end-hazard-abort-keeps-armed` | `end_bugAbortKeepsArmed` | an abort leaves the cells armed | `EndedFlowHoldsNoArmedCell` |
| `flow-end-hazard-separate-close` | `end_bugSeparateClose` | DG-5-C off: the clean close is a step of its own after the second direction completes | `run TcpAcceptDialerBothDirectionsHalfCloseThenCloseCleanly` |
| `flow-end-hazard-separate-close-completed-flow-open` | `end_bugSeparateCloseClean` | DG-5-C off (as above), checked as the invariant | `NoCompletedFlowStaysOpen` |
| `cid-lease-hazard-release-before-vmm` | `lease_bugReleaseBeforeVmm` | release-last off: the lease is released while the VMM still holds the device | `NoLaunchClash` |
| `cid-lease-hazard-release-before-vmm-outlives` | `lease_bugReleaseBeforeVmm` | release-last off: the lease is released while the VMM still holds the device | `LeaseOutlivesVmm` |
| `cid-lease-hazard-assign-split` | `lease_bugAssignSplit` | ADR-0156 off: the next-fit assign walk is not one step under the pool lock (choose, then take later without a recheck) | `CidUnique` |
| `cid-lease-hazard-assign-before-reclaim` | `lease_bugAssignBeforeReclaim` | NO-SURVIVE off: assign runs before boot reclamation killed the previous process's VMMs | `NoLaunchClash` |
| `cid-lease-hazard-assign-before-reclaim-order` | `lease_bugAssignBeforeReclaim` | NO-SURVIVE off: assign runs before boot reclamation killed the previous process's VMMs | `AssignAfterReclaim` |
| `cid-lease-hazard-reset-reboots` | `lease_bugResetReboots` | RESET-EXIT off: a guest reset reboots the guest inside the same VMM | `GuestResetEndsVmm` |
| `cid-lease-hazard-reclaim-never-completes` | `lease_bugReclaimNeverCompletes` | NO-SURVIVE off (liveness): boot reclamation kills the orphans but never records its completion, so `assign` stays disabled and a waiting workload is never placed | `EventuallyPlaced` |
| `quiescence-hazard-bool-latch` | `quiesce_bugBoolLatch` | QHOLD: a boolean latch — the first recovery's restore reopens while the second is still repairing | `NoReopenBeforeRepair` |
| `quiescence-hazard-bool-latch-preempts-repair` | `quiesce_live_bugBoolLatch` |  | `C1Repaired` |
| `quiescence-hazard-drop-decrements` | `quiesce_bugDropDecrements` | QHOLD: a dropped hold releases the count (the guard's drop decrements) | `NoReopenBeforeRepair` |
| `quiescence-hazard-drop-decrements-count` | `quiesce_bugDropDecrements` | QHOLD: a dropped hold releases the count (the guard's drop decrements) | `CountCoversRecoveries` |
| `quiescence-hazard-restore-clears-latch` | `quiesce_bugRestoreClearsLatch` | QHOLD: a restore reopens admission whatever the remaining count | `LatchIffCount` |
| `quiescence-hazard-quiesce-skips-flows` | `quiesce_bugQuiesceSkipsFlows` | QHOLD: the first hold latches admission but does not abort the open flows | `QuiescedHoldsNoFlow` |
| `quiescence-hazard-drop-decrements-leaked` | `quiesce_bugDropDecrements` | QHOLD: a dropped hold releases the count, so the leaked-hold count is not kept | `LeakedHoldKeepsForwardingQuiesced` |
| `quiescence-hazard-leak-ignored` | `quiesce_live_bugLeakIgnored` | ADR-0124: the supervisor does not consult the leaked-hold count; a dropped hold keeps forwarding quiesced for good | `C1EpisodeEnds` |
| `quiescence-hazard-repair-without-hold` | `quiesce_bugRepairNoHold` | a recovery repairs without holding quiescence | `NoRepairWithoutHold` |
| `quiescence-hazard-redo-not-counted` | `quiesce_live_bugRedoNoCount` | UP-7: a failed post-repair audit is not a failed attempt | `C2EpisodeEnds` |
| `quiescence-hazard-boot-continues` | `quiesce_bugBootContinues` | ADR-0184: boot continues with serve running after a failed write of the shared table | `ServeRunsOnlyWithTable` |
| `udp-slots-hazard-claim-split` | `slots_bugClaimSplit` | slot claim is two steps (check, then take) | `SlotHeldOnce` |
| `udp-slots-hazard-keep-frames` | `slots_bugKeepFrames` | P-35: release returns the slot with frames still parked | `ReleasedSlotCarriesNoData` |
| `udp-slots-hazard-late-paired-by-slot-route` | `slots_bugLatePairedBySlot` | a late Paired is matched by slot, not by flow id | `RouteCurrent` |
| `udp-slots-hazard-late-paired-by-slot-data` | `slots_bugLatePairedBySlot` | a late Paired is matched by slot, not by flow id | `ReleasedSlotCarriesNoData` |
| `udp-slots-hazard-ignore-late-paired` | `slots_bugIgnoreLatePaired` | SLOT-ABORT off: a late Paired is dropped silently | `LatePairedAborted` |
| `udp-slots-hazard-keep-frames-on-end` | `slots_bugKeepFramesOnEnd` | U-2 off: Refused / Abort / the pairing deadline leaves the parked frames in the slot's cell | `NoReassociationWithoutNewDatagram` |
| `udp-slots-hazard-no-audit-fairness` | `slots_liveTeeth` | liveness teeth: no fairness on the audit — a lost sock_release report strands the slot | `NoAuditFairness` |
| `udp-slots-hazard-cells-emptied-only` | `slots_bugCellsEmptiedOnly` | CELL-RECREATE off (K-B3 as falsified, P-41): the cells are emptied, not recreated; the residue in the sender-side queue / psock backlog refills them and re-triggers readiness | `NoReassociationWithoutNewDatagram` |
| `udp-slots-hazard-fd-dispatch` | `slots_bugFdDispatch` | CELL-RECREATE off: readiness dispatched by slot (raw descriptor) only, ignoring the cell generation | `NoReassociationWithoutNewDatagram` |
| `udp-slots-hazard-fd-dispatch-generation` | `slots_bugFdDispatch` | CELL-RECREATE off: readiness dispatched by slot (raw descriptor) only, ignoring the cell generation | `NoStaleGenerationStart` |
| `flow-quota-hazard-count-active-only-bound` | `quota_bugCountActiveOnly` | R2-DRAIN off: the quota counts active flows only — a draining flow keeps its slot but not its count, so one withholding guest holds more than its own quota | `QuotaBound` |
| `flow-quota-hazard-node-wide-quota` | `quota_bugNodeWideQuota` | the quota is per allocation: with one quota for the node, a withholding guest exhausts the node | `QuotaRefusalOnlyAtOwnQuota` |
| `flow-quota-hazard-free-at-half-close` | `quota_bugFreeAtHalfClose` | R2-DRAIN off: the flow's slot is released at the half-close while its drain cell is pending | `DrainingFlowHoldsItsSlot` |
| `flow-quota-hazard-count-active-only-run` | `quota_bugCountActiveOnly` | R2-DRAIN off: the quota counts active flows only — a draining flow keeps its slot but not its count, so one withholding guest holds more than its own quota | `run DrainingFlowStillCountsAgainstTheQuota` |
| `flow-quota-hazard-node-wide-quota-run` | `quota_bugNodeWideQuota` | the quota is per allocation: with one quota for the node, a withholding guest exhausts the node | `run WithholdingGuestHoldsAtMostItsOwnQuota` |
| `flow-quota-hazard-free-at-half-close-run` | `quota_bugFreeAtHalfClose` | R2-DRAIN off: the flow's slot is released at the half-close while its drain cell is pending | `run DrainingFlowStillCountsAgainstTheQuota` |
| `udp-empty-hazard-no-epoch` | `empty_bugNoEpoch` | A2 shape off: a notice is acted on whatever its epoch (a stale notice sends an empty datagram on the new association) | `NoSpuriousEmpty` |
| `udp-empty-hazard-no-compensation` | `empty_bugNoCompensation` | ring-drop compensation off: the owner ignores a raised lost counter | `EveryEmptyEventuallySent` |
| `udp-empty-hazard-no-reset-at-reuse` | `empty_bugNoResetAtReuse` | the cell's lost count is not reset when the cell is initialised for a new flow | `NoSpuriousEmpty` |
| `udp-empty-hazard-no-reset-at-reuse-lost-count` | `empty_bugNoResetAtReuse` | the cell's lost count is not reset at cell initialisation | `LostCountResetAtCellInitialisation` |
| `udp-empty-hazard-no-epoch-run` | `empty_bugNoEpoch` | A2 shape off: a notice is acted on whatever its epoch (a stale notice sends an empty datagram on the new association) | `run StaleEpochNoticeIsDiscardedAfterCellReuse` |
| `udp-empty-hazard-no-reset-at-reuse-run` | `empty_bugNoResetAtReuse` | the cell's lost count is not reset when the cell is initialised for a new flow | `run LostCountIsResetWhenTheCellIsReusedForANewFlow` |
| `udp-empty-hazard-no-compensation-run` | `empty_bugNoCompensation` | ring-drop compensation off: the owner ignores a raised lost counter | `run RingDropIsCompensatedByTheCountedNumber` |
