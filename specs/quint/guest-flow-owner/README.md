# Quint specification: guest-flow owner (netns-density-295)

Model-checked specification of the concurrent, ordered and crash-sensitive protocols of the vsock
attachment replacement, and the oracle DISTILL's `quint-connect` conformance drivers replay against
the production owners (`.claude/rules/design.md` § "Concurrent protocols carry a model-checked Quint
specification", `.claude/rules/testing.md` § "Quint conformance").

Every module here is a state machine (state variables, `init`, `step`) with named runs for its critical
scenarios. Each run is listed in `checks.toml` (`backend = "evaluator"`). Every design decision has a
hazard instance in `hazard/` that removes or weakens its rule, with `expect = "violation"` (WITNESS-HAZARD-RULE, below).

Run: `cargo xtask lima run -- cargo xtask quint typecheck`, then
`cargo xtask lima run -- cargo xtask quint check --subsystem guest-flow-owner [--name <check>] [--ci]`.
Evidence of the last full run: `evidence/summary.md` (written only by `--record`; `cargo xtask quint
verify-evidence --subsystem guest-flow-owner` fails when it no longer matches the specs).

Decision ids are (INB-1, R2-END, R2-DRAIN,
CID-NOCLAIM, RESET-EXIT, QHOLD, NO-SURVIVE) and the ADRs they live in (ADR-0146, -0156,
-0157, -0160, -0165, -0166, -0169, -0173, -0177, -0180, -0182, -0183, -0184), and the DISTILL upstream-issue
contracts (`docs/feature/netns-density-295/distill/upstream-issues.md`, pinned in their owning delta sections):
D-UP-1 (the conformance step record), D-UP-3 (a leaked quiescence hold), D-UP-4 with PD-1 (a restore with a hold
another ledger issued: `ForeignHold`, nothing changes, fail-stop `ForeignQuiescenceHold`), D-UP-8 (the guest-held control session),
D-UP-9 (the VM driver's VMM-exit step), and the model abstractions
D-UP-19 (no accept order and no reclamation order: any pending connection is accepted, and `Reclaim` kills every
orphan in one step) and D-UP-20 (the beacon instance's enabled actions; `lease_live` on three abstract offsets; the
idle release of an attached slot only). D-UP-4's ledger is modelled only through its observable rule: `quiescence` has
one owner, and a hold another ledger issued enters it only as the component-free stimulus `ForeignRestore`. The
claim that such a hold, dropped unredeemed, counts as leaked at the ledger that issued it is **model-only**: that
ledger belongs to another owner, no variable or invariant here represents it, and no conformance driver can project
it. It is discharged outside the model by a sim/unit scenario: two ledgers; the owner refuses a hold the second ledger
issued (`ForeignHold`); the hold is dropped; the second ledger's `leaked()` rises by one and the owner's does not
(see § *Model-only claims*). Spike evidence: `docs/feature/netns-density-295/spike/` (`findings-v29` ..
`findings-v32`, `findings-ka5`, `findings-evmod`, `findings-evmod-followup`, `findings-v1..v4-guest-slot`).

The vsock data path is the in-kernel event-driven forwarder (EVMOD-ADOPT, ADR-0197): on each side one module (it unloads
only at zero module references, MODULE-REFCOUNT) forwards every guest flow by peeking its source, sending without
blocking and consuming only what the target accepted. Its approved interface contracts are EVMOD-END (end classification
in the forwarder's callback), EVMOD-PAIR (attach at pairing), EVMOD-CTL (the context, ADR-0199), EVMOD-REC (the end-record
completion channel, ADR-0202), EVMOD-STREAM-END (how a `GuestStream` resolves an end), EVMOD-STREAM-ABORT,
EVMOD-ATTACH-END, EVMOD-DGRAM (datagram mode, ADR-0201) and EVMOD-AB (release at the end record); the modules that encode
them are `fwd_pair`, and the `flow_quota`, `owner_flows` and `udp_slots` rules that depend on them.

**WITNESS-HAZARD-RULE.** Hazards are one per design decision (an ADR-level rule), not
one per clause: a rule's variants (a check and its run, two invariants of one rule, a second instance of the same
rule) are not separate hazards. A non-vacuity witness is kept only where no named run reaches its state; the others
are cut, because a named run on a conformance main already shows the behaviour is reachable. The witnesses kept are
`owner-flows-witness-phantom-connect`, `fwd-pair-witness-watch-half-close` and `fwd-pair-dgram-witness-vmm-exit`. An
invariant's teeth are the hazard of the decision it defends. The `OFF` switches are only the rules of the hazards
`checks.toml` lists: every `OFF` string in a module is the rule of one. A
property that TLC checks exhaustively on a main is not repeated as a bounded Apalache check on the same main; the
two bounded Apalache checks that remain, `owner-flows-two-safety-apalache` (`flows_two`: two concurrent host-opened
dials) and `udp-slots-safety-apalache` (`slots_ok`: two destinations), cover instances no exhaustive check covers.

## The conformance step record `conf_step` (D-UP-1)

`quint test` (which `#[quint_test]` runs) emits no `mbt::actionTaken` / `mbt::nondetPicks`, so every stateful
module records the action that produced each state in a state variable of its own, `conf_step`. Its value is a
variant whose tag is the action's exact name (the name a driver's `switch!` arm matches) and whose value is the
record of that action's parameters by parameter name, or the unit value for a parameterless action. The initial
state carries `init`, except where `init` delegates to a parameterised action, which it then records: `InitWith`
(`owner_flows`, `fwd_pair`), `InitWith` / `InitDown` (`quiescence`). A wrapper that only picks a parameter records the
action it takes (`HostReadReq` → `HostReadReqN`, `HostFlowEnds` → `HostFlowEndsMsg`, `GuestPairOrRefuse` →
`GuestPairOrRefuseOk`, `Restart` → `RestartWith`, `GuestAudit` → `GuestAuditIdle`,
`releaseAny` → `GuestReleaseSock`).

The variant type lives in a definitions-only module beside each spec (`<spec>_conf`, e.g. `owner_flows_conf`); the
spec imports only the type, and each action builds its value with the builtin `variant("<Action>", …)`, because a
constructor of the same name as an action cannot share the action's namespace. `conf_step` is model-only and
write-only: no guard, effect or property reads it.

`conf_step` changes only on a module's conformance main. Every module takes `const REC: bool`, and each action
assigns `conf_step' = if (REC) variant(…) else conf_step`, so every model-checking, liveness and hazard instance
(`REC = false`) keeps `conf_step` at its initial value and no TLC or Apalache check carries it in its state space.
Because `conf_step` is write-only, a recording and a non-recording instance of one configuration have the same
behaviours of every other variable, so a property checked on the second holds on the first. Only named runs
(`backend = "evaluator"`) are checked on a conformance main. (Measured: with `conf_step` recorded on `flows_ok`,
`owner-flows-safety-tlc` grew from 11.8 M to more than 22.9 M distinct states.)

| Module | Conformance main (`REC = true`) | Configuration | `Config::nondet` path |
|---|---|---|---|
| `owner_flows` (host and guest owners) | `flows_conf` | `core2` (the core with two host-opened flows: every core run replays on it) | `flows_conf::owner_flows::conf_step` |
| `owner_flows` (VM driver, beacon) | `flows_beacon_conf` | `beacon` (D-UP-20: its own enabled actions) | `flows_beacon_conf::owner_flows::conf_step` |
| `fwd_pair` | `pair_conf` | `conf` (every mode — stream, watch, datagram —, 4 units and 3 datagrams each way, faults unbounded) | `pair_conf::fwd_pair::conf_step` |
| `flow_quota` | `quota_conf` | as `quota_ok` | `quota_conf::flow_quota::conf_step` |
| `cid_lease` | `lease_conf_ok` | `conf_ok` (the five offsets of the /29 test pool; `Crash` not enabled in the #325 window, item 9) | `lease_conf_ok::cid_lease::conf_step` |
| `cid_lease` (D-18) | `lease_conf_three` | `ok` (`lease_ok`'s three offsets; carries only `PoolExhaustedRestartCleansThePredecessorThenPlaces`, unreachable on the five offsets) | `lease_conf_three::cid_lease::conf_step` |
| `cid_lease` (pending GH #325) | `lease_conf_325` | `conf_325` (as `conf_ok`, the #325 window kept; carries only `RedeployLeasedAtServeCrashIsPlacedAgain`) | `lease_conf_325::cid_lease::conf_step` |
| `quiescence` | `quiesce_conf` | `conf` (the crash of `ok` and the unbounded repair failures of `live`) | `quiesce_conf::quiescence::conf_step` |
| `udp_slots` | `slots_conf` | `conf` (as `ok`, MAXFID above a driver's step bound) | `slots_conf::udp_slots::conf_step` |

`owner_flows` has two conformance mains because its two kinds of driver replay disjoint step relations (D-UP-20:
the beacon instance enables only the VM driver's actions); each driver still names one main and one path. A
driver names its main in `#[quint_test(main = …)]`, `#[quint_run(main = …)]` and `Config::nondet`; a driver
pointed at a non-recording main fails loudly at its first step (every step decodes as the initial value;
`docs/feature/netns-density-295/spike/findings-qt1-conf-step.md`). The ITF name is
`<main>::<module>::conf_step`, one element of the `Config::nondet` path (Quint qualifies a variable with the main
module and the module that declares it).

## Modules

| Module (file) | Protocol | Decisions defended |
|---|---|---|
| `owner_flows.qnt` | control sessions, flow admission, launch attribution (session, flows, beacon), host-opened dial | ATTR-TOKEN, ATTR-LIVE-CLAIM, INB-1, U-1, U-5, H-1 (attach after the request), EVMOD-PAIR, DG-5-E, DG-5-F, D-UP-8, D-UP-9 |
| `fwd_pair.qnt` | one guest flow end to end, forwarded on each side by the in-kernel forwarder: forwarding, backpressure, end classification, pair end, the control context and its end record, the watched dial, datagram mode | EVMOD-ADOPT, EVMOD-END, EVMOD-PAIR, EVMOD-CTL, EVMOD-DGRAM, R2-END, END-IDEMPOTENT, CLEAN-FULL-END, ADR-0183, D20, RESET-EXIT (the host half) |
| `fwd_safe.qnt` | the forwarder's pair (or watch) lifecycle at its real lock and RCU boundaries: callback dispatch, `sk_user_data` load and use, the end's restore / clear / id-and-record / free, the RCU grace period and `cancel_work_sync`, control calls by id, the context's release, the orphan sweep's unlocked stores; the wakeups that make backpressure resume. Not a conformance target (module-internal C, ADR-0204) | EVMOD-SAFE (ADR-0204), EVMOD-REC (ADR-0202), EVMOD-END (ADR-0180, the sweep row), WATCH-ABORT, § Forwarding (Trigger, Run) |
| `flow_quota.qnt` | the per-allocation flow quota counts half-closed flows; a place is released at the end record, read in queue order | ADR-0147, ADR-0183, EVMOD-AB |
| `cid_lease.qnt` | CID = base + lease offset; lease released after VMM exit; boot reclamation before assign; guest reset ends the VMM | CID-NOCLAIM, RESET-EXIT, NO-SURVIVE |
| `quiescence.qnt` | counted, move-only quiescence hold; leaked holds; foreign holds; per-component recoveries; boot step 8 refuses boot | QHOLD, ADR-0124, ADR-0184, D-UP-3, D-UP-4 (PD-1) |
| `udp_slots.qnt` | guest UDP slot pool and datagram associations | D25, SLOT-ABORT, U-2, EVMOD-DGRAM (the slot socket's queue discarded at an end) |

The parts of each module an instance does not use stay constant. Instances (`const CFG`): `owner_flows`
`core` / `core2` / `one` (the core with one guest-opened flow: the instance of `GuestHalfEnds`) / `sess` / `beacon` (`core2` has `flows_two`, the
non-recording instance of the bounded Safety check, and `flows_conf`, the conformance main); `fwd_pair` `ok` (stream: one unit each way) / `watch` (a dial: one unit each way) / `live`
(stream, one unit each way: the liveness instance) / `wlive` (a dial, 1 unit gh, none hg: the watch liveness
instance of T-5 and T-6) / `stall` (stream, 4 units gh, none hg, no fault) / `dgram` (one
datagram each way) / `dlive` (1 datagram gh, none hg, no send-buffer toggles: the liveness instance of EV-DGRAM
D1) / `mid` (stream mode with 2 units and datagram mode with 2 datagrams, gh only, one fault: the multi-unit and
multi-datagram instance, `MidSafety`) / `dhaz` (datagram mode, 3 datagrams gh, one fault: only the hazard of the datagram
invariants instantiates it) / `conf`; `cid_lease` `ok` / `live` (three abstract offsets; D-UP-20) / `conf_ok`
(the five offsets of the /29 test pool: `lease_conf_ok`, and its non-recording `lease_five` for the safety check) /
`conf_325` (as `conf_ok` with the #325 window kept: `lease_conf_325`, pending GH #325); `lease_conf_three` records
`conf_step` on `ok` (D-18);
`quiescence` `ok` / `live` / `conf`; `udp_slots` `ok` / `live` / `conf`; `fwd_safe` `pair` / `watch` (safety, two
faults or control calls) / `plive` / `wlive` (liveness, one) / `runs` (the named runs, unbounded). `const REC` selects the conformance mains
(above). `OFF` names the rules switched off; the design instances switch none off.

Coverage note (`fwd_pair`): every model-checking instance carries at most ONE environment fault or owner abort per
trace (`TcpRst`, `HolderDrop`, `VmmExit`, `UdpErr`, `Abort`, `CtxRelease`, `CloseUnattached`, `DgHalfClose`: `FMAX` = 1; the first unbounded instance
had more than 9.7 M distinct states after 15 min). Two faults meeting in one trace — a reset racing an owner's
abort — are therefore checked only along the named runs on `pair_conf`, each of which checks `Safety` (or
`DgramSafety`) at every step. The `ok`, `watch` and `dgram` instances carry one unit (one datagram) per direction The
multi-unit and multi-datagram behaviour is checked exhaustively on `pair_mid` (`fwd-pair-mid-safety-tlc`: 2 units and
2 datagrams in one direction with one fault, so a reset, an exit or a send block can arrive while the second unit or
datagram is queued or mid-adoption), and along the named runs on `pair_conf` (4 units, 3 datagrams each way) and, for a
short send, on `pair_stall`. `MidSafety` is `Safety` in a stream or watch trace and `DgramSafety` in a datagram trace
(the stream invariants count units in the vsock queue, which holds frame items in datagram mode). The datagram
invariants have teeth on `pair_dhaz` (3 datagrams): a datagram consumed with only part of its frame accepted corrupts the
frame of the datagram that follows it, so two datagrams cannot violate `DatagramsOnceInOrder`. The liveness instance carries one unit each way: TLC's liveness mode is two
orders of magnitude slower per state on this module than its safety mode.

### What the model contains nothing of

Absent from every module: host-local delivery steering of the guest prefix,
intake listeners / the intake port block / listen-state mirroring, prefix landing and boot, the shared
table's boot-continues path, the kernel CID claim (claim port, survivors, fd handoff), `HalfClose` /
`HalfCloseAck` control messages, a drain deadline / recheck / `DrainTimeout`, quiescence holder names,
`StillQuiesced { holders }`, `QuiescenceHolderBusy`, VM survival across a `serve` restart.

Absent from every module (the data path is the in-kernel forwarder, not a BPF sockmap redirect): the `sk_skb`
verdicts, sockhash routes and install at establishment, the `SO_RCVLOWAT` re-arm, the guest TCP parking cells, the
drain cells (`goal`, `armed`, `fired`, `sent`), the two `fexit` counters and the `fentry` backstop, recursion-miss and
lost-counter reads, `RedirectRefused`, the flow-event ring, `FlowCell` / `CellEpoch` and epochs, the raw-tracepoint
classifier, the valve (never approved), and — on approval of EVMOD-DGRAM — SEQPACKET from the guest, the framing and
reassembly cells and CELL-RECREATE, the empty-datagram notice and owner-sent empties, and the guest-`lo`
unframe program.

### Not modelled, with the reason

- The forwarder module's internals: `FWD_CHUNK` (a run moves one unit here), `FWD_RUN_BUDGET` and requeue, the
  per-CPU workqueue (EVMOD-WQ), the scratch pool, the inventory counters, the ABI version
  (EVMOD-LIFE; no permanence flag exists, MODULE-REFCOUNT), the kTLS refusal (EVMOD-KTLS) and the attach-time typed refusals of EVMOD-CTL: a refused attach is
  modelled by its far-end effect only (the owner closes the flow with linger, as for an abort). Tier-3 (the module
  suite, the probe's gold test) and the seeded sim.
- The module's build, load and guest packaging (EVMOD-BUILD, ADR-0203; EVMOD-LOAD, ADR-0205; EVMOD-GUEST-KIT,
  ADR-0206) and the appliance image kernel, which has no module unloading (`CONFIG_MODULE_UNLOAD` off, ADR-0068 § 7):
  boot-time facts with no interleaving; the module's unload, its references and the lifetime of its text are not
  modelled at all (`fwd_safe` checks the pair's state, not the override's text);
  a missing or mismatched module is a refused start (G-V1, G-V7), checked by the probe tests, not a protocol state.
- The ADR-0149 length check and oversize datagrams (`dgram_oversize`): a datagram's size is 0..2 units here.
- GW-LOCAL (the gateway as the one host-local prefix address): a boot-converged route, no protocol.
- Time: deadlines are nondeterministic failure actions, and the order of simultaneously due deadlines is
  not modelled. There is no deadline in the drain (R2-DRAIN).
- Byte counts: a unit stands for any number of bytes; a window or credit of 1 unit stands for the socket's buffer.

## `owner_flows`: control sessions, admission, attribution, the dial

State: `alloc` (CID table state), `gen` / `vmGen` / `vmUp` (lease and launch), `quiesced`, `claim` and `sess`
(the control-session `ClaimSet` and connections), `flows` (guest-opened flows), `hf` / `gf` / `dl` and the
control-message sets (host-opened flows and the dialer's results), `drvLive` / `bclaim` / `bcn` (the VM
driver's beacon), `gSess` (D-UP-8: the control connection the guest owner holds as its session; guest-side state),
`conf_step`, ghosts (`badPaired`,
`crossMsg`, `heldByOther`, `resurrected`, `phantom`, `readyCross`, `bHeldByOther`, `abortDiscarded`).

D-UP-8: `gSess` is set when the guest opens its session (`GuestOpenSession`, enabled only while `gSess` = none),
cleared when the guest observes the loss (`GuestSessionLossObserved`, enabled also when the session it holds was closed
by the host with no flow of it) and at `VmStop` (the guest owner dies with the VM). The racing second first-connect of
L-3 (R5-18) is not the guest owner's: it is `WorkloadOpenSession`, a connection to the control port opened by a workload
in the guest while the guest owner's own first connect is still unresolved on the host, on a connection slot never
used; it does not touch `gSess`. QPROJ-5 `session` ⇔ `gSess` ≠ none.
D-UP-9: `VmStop` is the VM driver's one per-CID step for a launch's VMM exit (a guest reset included): it ends the
launch's liveness (`drvLive`), closes its beacon sessions and releases the beacon claim, as `driver.stop` does.
D-UP-19: an accept takes any pending connection of its listener (no accept order is modelled; no invariant depends on
it). D-UP-20: on `flows_beacon` only `Provision` (a new assignment, once the previous launch ended), `VmStart`,
`VmStop`, `DriverStop` and the beacon actions are enabled.

Actions: `Provision`, `VmStart`, `VmStop` (a VMM exit also resets every requested host dial of the stopped launch: the dial
returns `Unreachable { ECONNRESET }`), `Activate`, `ActivatePending`, `ActivateQuiesced`, `ActivateRetiring` (the refused activations),
`Teardown`, `Release`, `Quiesce`, `Restore`;
`GuestOpenSession`, `WorkloadOpenSession`, `HostAcceptSession`, `HostReadOpen`, `HostClaimSession`, `HostSessionLoss`, `HostDeadSessionLoss`;
`GuestOpenFlow`, `HostAcceptFlow`, `HostReadReq`, `HostStartConnect`, `HostConnectDone` (attach (V_h, D), then `Paired`:
EVMOD-PAIR),
`HostConnectDiscard`, `HostFlowEnds`; the dial `DialOpen`, `DialNotServing`, `DialQuiesced`,
`DialNoSession`, `DialUnreachable`, `GuestAcceptV`, `GuestPairOrRefuse`, `GuestConnectTimesOut`, `HostRecvPaired`,
`HostRecvRefused`, `HostRecvTimedOut`, `HostDialDeadline`, `DialCancel`, `GuestNeverReplies`, `HostAbortAccept`, `HostRecvAbort`,
`GuestRecvAbort`, `GuestSeesHostClose`, `GuestAcceptedError`, `GuestSessionLossObserved`,
`GuestQueueReset`; the beacon `GuestOpenBeacon`, `HostAcceptBeacon`, `HostReadReady`, `DriverStop`.

Under EVMOD-PAIR the host attaches a TcpConnect pair once D's connect completed, in the step that sends `Paired`, and
the guest attaches a TcpAccept pair (V_g, C) once C's connect completed, in the step that sends `Paired`
(`GuestPairOrRefuseOk`, `gf` `attached`); before that the guest owner holds V_g itself and watches it (DG-5-E: `acc`,
accepted and unattached). There is no install, arm or parking step. A dial's V_h is watched, never forwarded; the
dialer's end class is the watch's end record (`fwd_pair`).

Invariants: `OneLiveSession` (L-3), `NoAttachBeforeRead` (H-1: no flow is connected or attached before its request
was read in full — an attached socket's bytes are payload), `NoAdmitWhenClosed` (U-1),
`CurrentGeneration` (ATTR-TOKEN, ATTR-LIVE-CLAIM), `NoCrossLaunchMsg` (ATTR-TOKEN),
`NoCrossLaunchSessionHeld` (ATTR-LIVE-CLAIM), `ClosedIsTerminal` (U-5), `GuestFlowsDoNotOutliveSession`
(DG-5-F), `TypedRefusalChangesNothing` (INB-1; `NotServing`, `Quiesced`, `NoSession`, `Unreachable`), `DialDeadlineAfterGuestReply` and `NoPairedAfterDialEnded`
(INB-1, `GUEST_DIAL_PAIRING_DEADLINE` > `GUEST_INBOUND_CONNECT_TIMEOUT`), `BeaconCurrentLaunch`,
`BeaconNoCrossLaunchReady`, `BeaconNoCrossLaunchHeld` (ATTR-TOKEN / ATTR-LIVE-CLAIM for the beacon),
`GuestSessionOfCurrentLaunch` (D-UP-8: the session the guest holds belongs to the running launch).
Temporal: `GuestHalfEnds` (U-5 with K-A4, DG-5-E; checked on instance `one` — one guest-opened flow, one host-opened flow —
because the property mentions the host-opened flow only and `core` (15.1 M states) took 53 min and 10 GB), `SessionBecomesLive` (D18a; checked on instance `sess`: two control connections and two launches, no guest-opened flows and no dial slots, because the property mentions sessions only and the full `core` instance exceeded 900 s in TLC liveness checking; the instance takes its steps from `sessStep`, since `coreStep`'s flow and dial picks range over empty sets there and would disable every core action; its teeth are the environment variant `owner-flows-sess-env-guest-misses-session-loss`, K-A5S dropped), `ReadyReleased` (ADR-0157).

## `fwd_pair`: the in-kernel forwarder (EVMOD)

One guest flow end to end. Two sides, `g` and `h`, each with one forwarder context (`ctx`: open / released / gone;
`gen`: the number of the host owner's current context, 1 after the `Forwarder` recovery; `oldCtx`: the state of the
context it replaced) and at most one pair or watch (`pr`: `{ph, rec, out, cause, errno, byRel, closed, nrec, cgen}`, `cgen`
the context it was attached in; `ph` `refused` when the attach returned `HalfEnded`; `errno` the `SocketError` errno
of a TCP reset; `byRel`: ended with no record — by its context's release, by the owner's close of a flow it never
attached, or by a refused attach). Two directions, `gh` (written at
the guest's far end, read at the host's) and `hg`. Per direction (`dir`): the writer (`wr`, `wfin`), the queues `q0`
(the source TCP socket's receive queue, window `W` = 1), `q1` (the vsock: units sent and not consumed by the peer
forwarder, credit `C` = 1; in datagram mode frame items, credit `CD` = 3), `q2` (the target TCP socket toward the
reader, `W` = 1), the forwarder memory `h0` / `h1` (hazard only), the ends `srcEnd` / `srcShut` (at the source side's
pair) and `dstEnd` / `dstShut` (at the target side's pair), the reader (`got`, `eof`, `rst`, `shutRd`) and the ghosts
`loss` (the stream was cut before its writer's FIN) and `lost`. Per side's vsock half (`vs`): `ps` (peer_shutdown),
`sent` (the flags this side sent), `err` (the device release's sk_err), `cls` (the class the callback recorded), and
`sshut` (`ownShut`: this side's `sk_shutdown & SEND_SHUTDOWN`, read by the callback under the socket lock and set in
the step that sends the SHUTDOWN, CLEAN-FULL-END), `st` (`vsSt`: `est`, or `closing` once a packet left
`peer_shutdown == SHUTDOWN_MASK` with the receive queue empty — an RST, or a 0x3 with nothing queued), and
the ghosts `misc` (a transition classified otherwise than the table gives with the socket's true send state),
`tcls` (the class of the transition by the EVMOD-END table), `ucls` (the class the forwarder acted on),
`att` (an end was present when the callbacks were installed), `clsAt` / `ncls` (where, and how many times, the end of
the incoming direction was classified) and `snap` (the state a non-atomic attach read; hazard only).
`ctl`: per direction, the SHUTDOWN / RST packets in flight. `tcpErr`: a TCP half's pending reset error (`0`,
`ECONNRESET`, `EPIPE`); `terr`: a UDP half's error. `attRes` (`none` / `ok` / `halfEnded`) and `attEnded` (the
owner's `forwarder_attach_ended` count): the attach result each owner received. `strmAbort` (the dial's stream was
ended by the host owner), `hres` (the outcome the `GuestStream` reported: `HalfClose`, `GuestAbort`, `OwnerAbort`) and
`guestAbortSent` (an `Abort { flow }` written and not yet taken; model-only). `ep`: the far end of each
side (the guest application, the far peer, the dialer). `dg`: the datagram-mode state. `gh`: ghosts and the fault
budget. Modes (`mode`): `stream` (a TcpConnect: both pairs), `watch` (a dial: the guest's TcpAccept pair and the host's
watched V_h, read and written by the dialer directly) and `dgram` (a datagram association: UDP halves, frames).

A forwarder run moves one unit: it peeks the source, sends if the target has room, and consumes only what the target
accepted (one atomic step). A packet's processing (`VCtl`) is the receiving side's `sk_state_change`: it classifies
the transition (S newly set without R → clean; S with R → abort; R newly set while this side still sends → abort; R
newly set after this side shut its send → none; after the pair's end → none) and, for a clean end, the target is shut
only once the vsock queue is empty (`HopEnd1`). A TCP source's FIN is end-of-stream after its queue (`HopEnd0`), and a
TCP reset is one action, `TcpRst(s, after_fin)` (T-1): it sets the half's `tcpErr` (`EPIPE` when the half is in
`CLOSE_WAIT` — it received its peer's FIN and the forwarder has not sent its own — else `ECONNRESET`; `tcp_reset`,
`net/ipv4/tcp_input.c:4918-4930 @ v7.0`) and runs its `sk_error_report` in the same step, which aborts a live pair
`SocketError { side, errno }` — also after a FIN read as end-of-stream; a pair whose directions both ended is
complete and ignores it. A vsock end-of-stream whose class was recorded neither at attach nor by a callback aborts
the pair and counts `unclassified_eof` (`UnclassifiedEof`; never enabled under the design's rules). Owners: `Attach`
(host first, guest on `Paired`, never after the host's attach was refused; the callbacks are installed and the half's
state read in one step — the locked section of the delta's "Attach order" — and an end present then is classified
there, once, with this side's send open: {S} only clean, any R abort; the attach-time run forwards what is queued.
EVMOD-ATTACH-END, T-3 / T-4: attach accepts a TCP half in `ESTABLISHED` or `CLOSE_WAIT` and a vsock half in
`ESTABLISHED` with any peer bits; it refuses an ended half — `tcpErr` set, the vsock's `err` (the sweep) or `st =
closing` — with `HalfEnded`: no pair, no record; in the same step the owner ends the flow by its pre-`Paired` close,
the TCP half with `SO_LINGER{1,0}` and the vsock half ({S, R}), the host also writing `Abort { flow }` while the
session is live, and `attEnded` rises by one), `GuestAbort` (the guest owner takes `Abort { flow }`: it aborts the
flow's pair, or closes the flow it has not attached), `CloseUnattached` (the guest owner
closes a flow it has not attached — a pending intake child that reset, the pairing deadline, session loss — with
`SO_LINGER{1,0}` on the TCP half, D20, and {S, R} on V_g; no record), `Abort` (Detached; of a watch — WATCH-ABORT, EVMOD-STREAM-ABORT, T-5 —
one owner step: the record queued, the callbacks restored, the module's reference dropped and V_h untouched; the
stream ended by the owner, `strmAbort`, so every later call returns `ConnectionAborted`; `Abort { flow }` written
while the session is live), `CtxRelease` (the owner's death, or the close of
the old context in the recovery: attached pairs and watches abort with no record, a watched V_h is shut
`SHUT_RDWR`; a flow not yet attached is closed
by the owner's descriptors with `SO_LINGER{1,0}`, D20), `Reopen` (the host's `Forwarder` recovery opens a new context
after the old one was closed; at most once per trace; a record queued in the old context is never read), `TakeEnd`,
`OwnerClose` (the vsock close's {S, R}; in datagram mode the guest discards the slot socket's queue). Environment:
`Write`, `Fin`, `Read`, `HolderEnd(k)` (the holder's end observation on V_h, F11 corrected: `eof` (a read of 0) or
`err` (a read error); after the release's `SHUT_RDWR` either, queued bytes or not; after a guest SHUTDOWN `eof` once the
queue is empty; after the orphan sweep `err` (`ECONNRESET`) once the queue is empty — and before the sweep's
`sk_error_report` (`FwdErr`) has run, because `VmmExit` and `FwdErr` are two steps), `ReadEof` (for the dialer: the
`GuestStream`'s reported end, `hres` — `OwnerAbort` at once when the owner ended the stream (its abort or its
context's release); else the watch's record, `HalfClose` or `GuestAbort`; else it waits for the record in flight;
EVMOD-STREAM-END), `ShutRd`, `TcpRst`, `HolderDrop` (T-6: the holder drops its `GuestStream`: the watch is aborted, then V_h
closed — the guest sees 0x3 — in one step; a stream the owner already ended only closes V_h), `VmmExit`,
`TrailingRst`, `SendDg`, `DRead`, `UdpBlock` / `UdpUnblock` (the target UDP socket's send would block), `UdpErr`,
`DgHalfClose` (a peer outside the design ends a datagram vsock's direction with a 0x2: the stimulus for EV-DGRAM D1,
F12), and (hazard only) `DropRecord`, `AttachRead` / `AttachHook` (checkThenHook), `WatchRecord`
(a `conf_step` variant no step produces). The orphan sweep's two steps (T-9) are `VmmExit` (the state
becomes readable: `swp` holds while the watch is live with `vs(h).err`) and `FwdErr(h)` (its `sk_error_report`). The watch's callbacks (`VCtl`, `FwdErr` on V_h) queue the
record and then call the original callback in one step (EVMOD-STREAM-END, EV-END change 3; ghost `woken`). In datagram mode a clean end
of the incoming direction completes the pair once the vsock queue is empty, and a head frame left incomplete when the
peer ended its direction aborts the pair `FrameInvalid` at the next run (`Hop1`, EV-DGRAM D1).

Invariants (`Safety`): `DeliveredIsInOrderPrefix`, `UnitsConserved`, `ConsumeOnlyAfterAccept`, `QueuesWithinBounds`,
`NoEarlyShutdown` (ADR-0183), `AbortNeverCleanEof` and `NoEofOnPeerLoss` (R2-END), `ClassFixedAtTransition`
(EVMOD-END), `OneEndRecordPerPair` and `ReleasedContextHoldsNoPair` (EVMOD-CTL; a live pair belongs to its owner's
current, open context), `NoForwardAfterEnd`, `NoPairOfReleasedContextForwards` and `OneContextPerOwner` (the
recovery's order: no pair of a replaced context forwards; never two open contexts), `AttachTimeEndClassifiedOnce`
(EVMOD-PAIR, U-EV-2: an end present when the callbacks were installed is classified exactly once, at attach; no end
is classified twice), `NoUnclassifiedEof` (the `unclassified_eof` alarm never fires), `ResetNeverEndsClean` (a pair
whose TCP half was reset while the pair was open never completes, whatever its `recvmsg` returned) and
`NoHolderReadAfterWatchAbort` (U-EV-3: the holder of a watch its owner aborted reads no further byte),
`HolderOutcomeFromWatch` (EVMOD-STREAM-END, F11, T-5: the `GuestStream`'s reported end is `OwnerAbort` iff the owner ended
the watch, else the class of the watch's record, whatever the holder's read returned — judged when the stream reports
it, ghost `gh.hresCls`, because a later owner end of the recovery leaves the reported end as it was, U-R1),
`RecoveryEndsHeldStream` (EVMOD-CTL recovery step 1, U-R1: after the `Forwarder` recovery's close no stream the holder
still holds is left un-ended by its owner), `WatchRecordBeforeWake` (EVMOD-STREAM-END, EV-END
change 3: a holder woken by a watch's end finds the record queued), `TcpResetEndsPair`
(T-1: no pair is live once its TCP half's reset was reported), `ResetErrnoMatchesState` (T-1: `EPIPE` iff the reset
half was in `CLOSE_WAIT`), `AbortOnlyAfterFault` (T-3, T-8: a pair or flow ends aborted only after a fault or abort
action), `RefusedAttachLeavesNoPair` and `EndedHalfNeverEndsClean` (T-4: a `HalfEnded` attach creates no pair and no
record, and no far end of the flow reads end-of-stream after it), `WatchAbortLeavesHolderSocket` (T-5),
`NoLiveWatchAfterDrop` (T-6), `OwnShutBeforeShutdownSent` (T-8) and `CtxGoneIffVmDown` (`ctx(g) = gone`
exactly when the VM is down, so the init conformance driver treats `ctx(g) = gone` as determined by the `VmmExit`
stimulus it applied and never projects it; its non-vacuity witness is `fwd-pair-dgram-witness-vmm-exit` on `pair_dgram`: no named run
exits the VMM in datagram mode); `ClassFixedAtTransition` also requires that no
transition is classified otherwise than with the socket's true send state (CLEAN-FULL-END). Datagram mode
(`DgramSafety`): `DatagramsOnceInOrder` (empties included), `FrameNeverSplitAcrossDatagrams`, `DropsOnlyAtSourceQueue`
(a datagram is dropped only at a UDP receive queue — the source socket's when it is full or discarded at a slot
release, or the far receiver's when it is full, as for any UDP socket — never in the forwarder or the vsock), with
`QueuesWithinBounds`, `OneEndRecordPerPair`, `ReleasedContextHoldsNoPair`, `NoForwardAfterEnd`,
`NoPairOfReleasedContextForwards`, `OneContextPerOwner`, `AttachTimeEndClassifiedOnce`, `AbortOnlyAfterFault`,
`RefusedAttachLeavesNoPair`, `OwnShutBeforeShutdownSent`, `CtxGoneIffVmDown`, `IncompleteFrameAtEndAborts`
(EV-DGRAM D1: a live pair whose vsock peer ended its direction with the head frame incomplete is aborted
`FrameInvalid` by its next run, or by `FwdErr` when an error is pending; it is the rule's state form, and its teeth
in time are `EveryEndResolves` on `dlive`). Temporal (instance
`live`; `StalledSenderIsBlocked` on `stall`; `EveryEndResolves` also on `dlive`, where a datagram pair whose peer
ended its direction is due to end): `EveryUnitDelivered`, `EveryEndResolves` (every pair ends after both
clean ends or at an abort and its owner reads the record, unless a reader stalls), `CleanHalfCloseCompletes`,
`StalledSenderIsBlocked` (while a reader stopped, its writer — writing as fast as it can — is eventually unable to
send; its bound is `QueuesWithinBounds`), `EndedHalfFlowResolves` (T-4, on `live`: a refused flow is closed at both
owners) and, on the watch liveness instance `wlive` (1 unit `gh`, none `hg`), `GuestPairEndsAfterOwnerAbort` (T-5) and
`GuestPairEndsAfterHolderDrop` (T-6).

Fairness is stated as the state formula `eventually(always(not(sysEnabled)))`, where `sysEnabled` is the disjunction
of the guard predicates the system actions are built on (`canAttach`, `canHop0S`, …). Every action of these
instances can occur only finitely often, so every behaviour ends in stuttering, and weak fairness of each system
action is then equivalent to that formula; stated through `weakFair`, TLC's liveness mode evaluated the enabledness of
every action in every state at about 1,000 states a minute.

Host-side projection pins (D-3 to D-5), as modelled: the host owner reads an end record and
closes the flow's descriptors in one dispatch, so `TakeEnd(h)` carries `OwnerClose`'s effect outside watch mode and
`OwnerClose` is the guest's own dispatch only (D-4); a `CtxRelease(h)` that finds the host pair or watch live is the
host owner's death and is final — no `Reopen(h)` follows it (ghost `gh.hostDead`, run `HostOwnerDeathIsFinal`) —
while one that finds none live is the `Forwarder` recovery's close, after its step 1 closed the descriptors of a host
flow whose record is queued and unread (the record stays queued in the released context and is never read) (D-5).
D-3 needs no model change: `strmAbort` is set only by the owner's abort of the stream; the dial-entry lifetime rule
is the projection.

## `fwd_safe`: the forwarder's late-callback rules at their lock and RCU boundaries

`fwd_pair` runs every callback, forwarder run and owner call as one atomic step, which is sound for delivery,
classification and records but cannot show a late callback, a freed pair or a lost wakeup. `fwd_safe` splits the
actors at the points where another actor can enter, for one pair (or one watch) and one forwarded direction:

- a callback invocation is four steps: the kernel reaches the call site (`Write` = vsock enqueue + `sk_data_ready`
  under `lock_sock`, `PeerFin` = a 0x2 + `sk_state_change` under `lock_sock`, `TcpRx` = TCP input calling the
  target's `sk_data_ready` override, `SwReport` = the sweep's `sk_error_report`); `Dispatch`
  (the callback pointer read); `Load` (`rcu_read_lock` + `smp_load_acquire(sk_user_data)`, rule 1); `Use` (the pair
  found: note the direction, queue the work; a watch queues its record first) or `Fallback` (NULL: call the
  socket's current callback, rule 2);
- the write-space trigger: `sk_write_space` is never overridden. `Read` (the far reader frees target space
  while `SOCK_NOSPACE` is set) starts a wake through the socket's own `sk_write_space` (`ws` = `called`);
  `WsEnter` takes the wait-queue head's lock `wqlk(t)` and enters the forwarder's wait-queue entry (`wqe`) if it is
  on the queue (`ws` = `entry`); `WsLeave` runs it (a touch of the pair, `queue_work`) and drops the
  lock. The end removes the entry (`wt`, `remove_wait_queue` under `lock_sock(t)`) before `t`'s restore and clear
  and before the free work is queued; `wt` waits while a wake holds `wqlk(t)`. Only `t`'s entry is modelled
  (the source's write-space wake belongs to the other direction, which runs the same code);
- the orphan sweep runs with no socket lock inside one hold of `vsock_table_lock` (`drivers/vhost/vsock.c:746-750`,
  `net/vmw_vsock/af_vsock.c:458`, `:471 @ v7.0`): its two visible stores `SwState` (`peer_shutdown`) and `SwErr`
  (`sk_err`) in either order (no barrier), then `SwReport` (`sk_error_report`), then `SwUnlock`, interleaving with the
  forwarder's run; EVMOD-SWEEP: a run that reads an end (end-of-stream or an error) with no recorded class waits in
  `WEnd` → `WRecheck` for `vsock_table_lock` and acts on the class recorded by then (`SocketError` from the sweep's
  override);
- the work item (`WStart` clears PENDING before the function, `WMove` / `WBlock` / `WDrained` / `WEnd`, `WRet`); the
  end in the pair's own work item restores then clears per half (`rs`, `us`, then `wt` the write-space entry's
  removal, `rt`, `ut`: separate steps), removes the
  id and queues the record in one section (`idrec`), queues the free with `queue_rcu_work` (`free`);
- the free: `RcuGp` (every reader inside at queue time has left), `FreeCancel` (`cancel_work_sync`: disable, drop
  PENDING), `FreeFlush` (wait for the running item), `FreeDone` (kfree);
- control calls by id under the record-queue lock: `CtlAbort` (`OVD_FWD_ABORT`, and `OVD_FWD_ABORT_ALL`, which acts
  on each id as `ABORT` does), `CtlState` (`PAIR_STATE`), the context's release `CtxRel`, the owner's `TakeEnd`;
  alternative (d) (the `ctlQueueOutsideRcu` hazard) as `CtlLookup` / `CtlUnlock` / `CtlQueue`. Attach is not
  modelled: it holds `lock_sock`, `sk_callback_lock` and `vsock_table_lock`, which exclude every callback of the half
  (F9).

Not modelled: the module's unload and the lifetime of its text — the module
references, `delete_module`, `module_exit`'s drain, a ULP's saved copy of an override, the classes of the callers that
reach an override and rule 5's removal under `lock_sock(s)`. The appliance image has `CONFIG_MODULE_UNLOAD` off
(ADR-0068 § 7), so the unload is a dev-host concern checked by the module suite (EVMOD-LIFE, ADR-0198), not a protocol
state. The rules modelled are rules 1-4 of EVMOD-SAFE and the write-space entry's removal.

Invariants (`Safety`): `NoTouchAfterFree` (no override, run, control call or `queue_work` touches freed state),
`NoOverrideCallsItself`, `OneEndRecord`, `RecordAtIdRemoval` (no lost record: the id leaves only with
its record, none at release), `OneFree`, `FreeAfterWorkCompletes`, `FreeAfterReaders`, `NoIdNamesFreedState`,
`CleanEndOnlyAfterPeerFin`, `NoUnclassifiedEnd` (the `unclassified_eof` alarm never fires, including for a run that
reads the half inside the orphan sweep's stores: EVMOD-SWEEP's `vsock_table_lock` re-check, F-S1).
Temporal, under weak fairness of every system step and of the far reader (not of the vsock peer's writes, the
sweep's start, the control calls' start or the release), stated as `eventually(always(not(sysEnabled)))`:
`EveryUnitDelivered`, `StalledPairResumes` (a pair stalled on a full target resumes once the reader reads),
`EndProducesRecord`, `SocketEndYieldsRecord` (the peer's 0x2 or the sweep eventually yields the record),
`RequestedAbortEnds`, `EndedStateFreed`. Each rule's hazard is in § Hazards; the wakeup hazards drop
the write-space wait-queue entry, `sk_data_ready` or `sk_error_report` and show the deadlock (the pair stalls, or
the end never yields its record, for ever).

## `flow_quota`: the per-allocation quota counts half-closed flows

The dial's `Capacity` outcome is modelled here (the allocation quota), not in `owner_flows`, which does not check capacity.

Three allocations, quota 1, two node flow places (node capacity below the sum of the quotas: the model assumes no
relation). A place is free / active / half (one direction half-closed, the pair still forwarding) / ended (the pair's
end record queued, not yet read). State: `slot` (`{own, st, fwd}`), ghosts `g` (`refusedUnder`, `clash`, `refused`,
`otherAdmitted`). Actions: `Admit`, `Refuse` (the typed capacity outcome, no state change), `HalfClose`, `Complete`
(absent for a reader that never reads), `Abort`, `ReadEnd` (the host owner reads the end record and releases the place:
EVMOD-AB). Invariants: `QuotaBound`, `QuotaRefusalOnlyAtOwnQuota`, `DrainingFlowHoldsItsSlot` (a pair that still
forwards — a half-closed one included — holds its place until its end record is read; ADR-0147, EVMOD-AB).
Property: a half-closed flow holds its quota place exactly like an open flow; a guest withholding reads holds at
most its own quota. M-1: the end records are queued in the order the pairs ended (`endq`) and `ReadEnd(s)` reads the
record of place `s` only at the head of the queue, as production `take_end` does. No node-capacity assumption. If the user keeps SLOT-RELEASE instead of EVMOD-AB, the release
waits for the vsock peer's release and a hazard `release-at-host-end` is added (feature delta, change list).

## `cid_lease`: CID lease, reclamation, guest reset

Offsets, workloads, the kernel's vhost-vsock holder table `dev` (free / an allocation's VMM / an orphan of the
previous process), `lease`, `cursor`, `al` (placement), `att` (the reconciler's restart count of the workload's current
allocation, `WorkloadLifecycleView::restart_counts`, persisted), `up` / `reclaimed` / `bootLeft` (serve process), `conf_step`.
Each workload's `al` carries its placement lease (`off`) and at most one predecessor lease (`pred`, state `pst`:
`held` / `retiring` / `none`). Actions:
`Assign`, `AssignRefused`, `Launch`, `Stop`, `VmmCrash`, `GuestReset`, `Finalize`, `Release`, `Redeploy`, `Crash`,
`Restart`, `Reclaim` (D-UP-19:
one `vm_reclamation_boot::converge` call kills every orphan in one step; the order it kills them in is not
modelled, since no invariant depends on it), `OrphanExit` (an orphan that exits by itself, which also covers one
killed before a later crash). Invariants: `CidUnique` (ADR-0156; every placement and predecessor lease),
`NoLaunchClash` (CID-NOCLAIM), `LeaseOutlivesVmm`
(release-last, ADR-0133), `AssignAfterReclaim` (NO-SURVIVE), `GuestResetEndsVmm` (RESET-EXIT),
`StoppedWorkloadNotPlaced` (the T-5 ruling: a stopped workload is not placed again until it is redeployed, across a
serve crash included, because the stop intent is persisted), `ExhaustsOnlyAtCeiling` (RESTART-SSOT: a workload is
finalised failed only once its restart count reached the ceiling), `ExhaustsOnlyOnACrashClassFailure` (RESTART-SSOT:
an exhausted workload's last ending, ghost `g.lastEnd`, is a crash-class failure, never a reclamation or a
`Redeploy`), `FailedHoldsNoLease` (DR6-CLOSE, item 7.5: a `failed` workload holds no lease) and `NoUntrackedLease`
(DR6-CLOSE: every leased offset is its holder's placement lease `off` or predecessor lease `pred`; an untracked lease
is a leaked CID), all in `Safety`. Temporal: `EventuallyPlaced`. `wait` / `done` are the
node's persisted placement intent as the placement owner reads it (T-5, DR2-CLOSE): the
conformance composition runs `WorkloadLifecycle` through the production convergence tick and the real `submit` /
`stop` / `restart` handlers, so `Assign` is a tick that places a `wait` workload, `Launch` / `Release` are the held
provisioner `provision` / `teardown` boundaries of one dispatch, `Stop` is the stop handler then a tick, `Redeploy` is
the restart handler, and `wait` / `done` / `retry` come from the reconciler's `hydrate_desired` and
`restart_status_for_alloc` (QPROJ-2W). `Crash` keeps a workload stopped and not yet released `done`.

Crash-class restart granularity (feature delta, the `cid_lease` map, item 7; DR5-CLOSE), production's dispatch
boundaries: `VmmCrash` / `GuestReset` free the device and run no tick, so the workload is `ended` with its lease still
Admitted (below the attachment cap the reclaim pass skips the pending restart owner). `Assign` from `ended` with `retry`
is the `RestartAllocation` tick: it leases the successor first and holds the old lease as the predecessor
(`pst` = `held`); `Launch` then retires the predecessor (`retiring`) and its `Release` frees it. No tick of the workload
(`Assign`, `Stop`, `Finalize`) runs while a predecessor teardown is held, and none runs before boot reclamation:
`run_server` calls `vm_reclamation_boot::converge` (`lib.rs:3955`) before it spawns the convergence loop (`:4212`), so
`Assign`, `AssignRefused` and `Finalize` require `reclaimed` (DR6-CLOSE; `assignBeforeReclaim` bypasses it for `Assign`
only). `Redeploy` requires it too: the HTTP listener that serves the restart verb binds (`lib.rs:4368`) after that
`converge`. The `Finalize` and `Redeploy` guards keep randomized conformance traces to production's boot order; they
are not load-bearing for `Safety` (after a `serve` crash no workload holds a lease, and `Assign` still needs `reclaimed`). `AssignRefused` is the same tick when the pool
has no free offset: the predecessor is cleaned at once and the workload waits with its count. At the ceiling,
`Finalize` (`FinalizeFailed`) retires the lease (`retiring` with `ex`) and its `Release` writes the terminal
(`failed`); without a lease (after a `serve` crash) it writes the terminal at once. `Stop` of an `ended` workload
reclaims its lease. A `Crash` loses every predecessor lease; an `ended` / finalising workload with `ex` stays `ended`
with no lease (the new process finalises it again).

Restart budget (RESTART-SSOT, GH #324; feature delta, the `cid_lease` map, items 2-6). The decision is the
reconciler's `restart_decision(attempts, platform_reclaimed, generation_replacement)` = `attempts < CEILING ||
platform_reclaimed || generation_replacement` (`overdrive-reconcilers/src/workload_lifecycle.rs:1114-1119`). `att` is
the restart count of the workload's current allocation (`WorkloadLifecycleView::restart_counts`, persisted; projected
through `attempt_index_for_alloc − 1`), and it changes only at placement, as `restart_counts` does (`:1214-1221`,
`:1339`): `Assign` gives the new allocation `att + 1` when the predecessor is a crash-class row (the model-only field
`chg`), `att` under either exemption, and 0 on a first placement. A `VmmCrash`, a `GuestReset` and a `Launch` clash
are crash-class failures: `retry = att < CEIL`, `ex = not(retry)`; a crash leads to the restart above (`retry`) or to
`Finalize` then `Release` (`failed`, `FinalizeFailed`, `BackoffExhausted`); a clash's `Release` gives `wait` (with
`chg`) or `ended` (`ex`); a stop's gives `done`. A `Crash` with a running VMM leaves an orphan that boot
reclamation kills (platform-reclaimed: the workload waits whatever its count, `chg` false). `Redeploy`, enabled from
`done` and `failed`, is a generation replacement (waits whatever its count, `chg` false). A failed workload stays
failed across a serve crash. A workload leased but not launched at a serve crash has no row, so it goes back to `wait`
with its predecessor's count and `chg` (item 3). A generation replacement leased at the crash is placed again: the
behaviour GH #325 is to deliver (RESTART-GEN-CRASH: production loses it today and #295 does not
fix it; pending #325). `lease_conf_ok` does not enable `Crash` in that window; `lease_conf_325` keeps it (item 9). The
ghost `g.pendingGen` records the
`Redeploy`'s pre-state until the placed allocation's first row (`Launch`); it does not affect behaviour in the
design, and hazard `crashLosesPlacedGeneration` uses it. `att` stops at the ceiling, so the state space is finite.
`CEIL` is the production 5 on `lease_conf_ok` / `lease_five` (the fifth crash-class failure, at `att` = 4, is placed
again; the sixth, at `att` = 5, exhausts the budget) and is scaled to 2 on `lease_ok`, `lease_live` and the `lease_bug*`
instances; the predicate's shape is the same. The pre-fix `restart_status_for_alloc` (`attempts + 1 < CEILING`, no
exemptions) is hazard `retryOffByOne`. Hazard `reclaimNotExempt` removes the platform-reclaim exemption alone.

Item-7 hazards (DR6-CLOSE): `refusalKeepsPredecessor` (7.4: a pool refusal leaves the predecessor held until the next launch),
`terminalBeforeRelease` (7.5: FinalizeFailed writes the terminal before it releases the lease) and
`tickDuringPredecessorTeardown` (the 7.3 step boundary: the `predRetiring` guard dropped from `Assign`, `Stop` and
`Finalize`). Those change the dispatch order without leaking a CID, so `Safety` holds on them and the named run is the
discriminator. The #325-window guard of item 9 has its own checks: the
run `CrashIsNotEnabledInThe325Window` (its last `expect` before `Crash` establishes every other precondition of
`Crash`) holds on `lease_conf_ok` and is violated on `lease_conf_325`, and its positive twin
`CrashIsEnabledInThe325WindowWithoutTheGuard` (the same prefix, then `Crash` taken) holds on `lease_conf_325` and is
violated on `lease_conf_ok`, which pins both violations to the guard.

## `quiescence`: counted hold and boot refusal

`up` / `stop` / `table` (serve process and the shared mTLS nft table), `cnt` (hold count), `latch`, `rec`
(per-component recovery: idle / held / repaired), `dmg`, `att`, `repLeft`, `flowsOpen`, `leaked` (D-UP-3: the
owner's leaked-hold count, process-scoped), `foreign` (D-UP-4: the supervisor's `restore` returned `ForeignHold`;
process-scoped, until the fail-stop), ghosts, `conf_step`. Actions: `Damage`, `OpenFlow`, `ForeignRestore` (D-UP-4,
PD-1: the supervisor restores a hold another `QuiescenceLedger` issued — a composition defect, no recovery involved;
nothing in the owner changes, `foreign` is set), `LeakHold` (D-UP-3:
a hold taken from the owner and dropped without a restore — an Overdrive defect, no recovery involved: count + 1, the
first hold latches and aborts the flows, leaked + 1), `Crash`, `Restart` (boot step 8 writes the table, or boot is
refused), `RecQuiesce`, `RecRepair`, `RepFail` (the only failed-repair cause: counted, the hold kept, UP-7),
`RecRedo`, `RecRestore`, `FailStop` (the supervisor's audit tick: c's attempt budget spent, or — whatever any recovery
does — a leaked-hold count above zero, cause `LeakedQuiescenceHold`, or a `ForeignHold` result, cause
`ForeignQuiescenceHold`). Invariants: `ForeignRestoreChangesNothing` (D-UP-4: a foreign restore changes neither the
count, nor the latch, nor the leaked-hold count), `LeakedHoldKeepsForwardingQuiesced`,
`LatchIffCount`, `CountCoversRecoveries` (the count is the recoveries holding a hold plus the leaked holds),
`NoReopenBeforeRepair`, `QuiescedHoldsNoFlow` (QHOLD), `ServeRunsOnlyWithTable` (ADR-0184).
Temporal: `C1EpisodeEnds`, `C2EpisodeEnds`, `C1Repaired` (QHOLD, ADR-0124, UP-7), `LeakedHoldEndsInFailStop` (D-UP-3),
`ForeignHoldEndsInFailStop` (D-UP-4, PD-1).

## `udp_slots`: guest UDP slots

Slot claim (`SlotHeldOnce`), release (`ReleasedSlotCarriesNoData`), route currency (`RouteCurrent`: an attached slot
forwards to its own current association), SLOT-ABORT (`LatePairedAborted`), U-2 (`NoReassociationWithoutNewDatagram`);
temporal `ReleasedSlotBecomesFree`. EVMOD-DGRAM: datagrams the application sends before `Paired` wait in the slot
socket's own receive queue (`parked`); on `Paired` the guest attaches the pair (`attached`) and the attach-time run
forwards them; every end of an association discards the queue without reading it; readiness (`GuestReady`) is the
level-triggered readability of an unattached slot socket, so an emptied queue starts nothing until a new datagram
arrives. The end of an association as the guest learns it — `Abort` / `Refused`, or the pair's end record — is one
handling (`GuestRecvEnd`). D-UP-20: `GuestAuditIdle(idle)` releases as idle only an attached (`Paired`) slot — a slot
awaiting `Paired` reaches its 5 s pairing deadline (`FLOW_PAIRING_DEADLINE`) long before its 120 s idle
(`GUEST_DATAGRAM_SLOT_IDLE`) — with no condition on the other slots; the orphan release stays in the same tick.

## Critical scenarios ↔ named runs

Every run holds (`expect = "holds"`) in `checks.toml`; the run-based hazards expect a violation. Every run is
checked on its module's conformance main (D-UP-1), the main a driver pins it on with `#[quint_test]`.

### `owner_flows` (conformance main `flows_conf` unless noted)

| Scenario | Run |
|---|---|
| happy path: lease, VM start, control session claimed on the live token, activation | `ControlSessionOpensAndCidActivates` |
| guest-opened flow admitted, request read in full, connected, attached, paired (EVMOD-PAIR) | `GuestOpenedFlowIsAdmittedReadAttachedAndPaired` |
| request in two pieces: nothing connected or attached before the whole request is read (H-1) | `RequestReadInPiecesAttachesOnlyAfterFullRead` |
| the guest's first connect and a racing first connect opened by a workload (`WorkloadOpenSession`) race: one live session (L-3, R5-18, D-UP-8) | `RacingFirstConnectsLeaveOneLiveSession` |
| session loss aborts flows; the guest observes the loss, reconnects; the CID serves again | `SessionLossAbortsFlowsAndReconnectRestores` |
| the guest holds its session until it observes the loss and re-opens only then (D-UP-8, DG-5-F) | `GuestReopensSessionOnlyAfterObservingItsLoss`, `GuestObservesSessionLossThenReopens` |
| the guest owner dies with its VM: `VmStop` clears the session it held (D-UP-8) | `VmStopClearsTheGuestSession` |
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
| session loss aborts every flow before any reconnect (DG-5-F) | `SessionLossAbortsAllFlowsBeforeReconnect` |
| accepted, unattached V_g, host close: silent end (DG-5-E) | `AcceptedVgHostCloseEndsFlowSilently` |
| accepted, unattached V_g error: Abort (DG-5-E) | `AcceptedVgErrorAbortsWithAbort` |
| guest connect timeout: `TimedOut` | `DialTimedOutReturnsTypedError` |
| `NotServing`, `Quiesced`, `NoSession`: typed, no state change | `DialNotServingIsATypedResultWithNoStateChange`, `DialWhileQuiescedIsATypedResultWithNoStateChange`, `DialWithoutLiveSessionIsATypedResultWithNoStateChange` |
| VM gone, session loss not yet observed by the owner: `Unreachable` (vsock connect fails), no state change; not `NoSession` | `DialToAGoneVmIsUnreachable` |
| VM gone, session loss observed by the owner: `NoSession` (the owner never reaches the vsock connect), no state change; not `Unreachable` | `DialAfterObservedSessionLossWithVmGoneReturnsNoSession` |
| pairing deadline above the guest timeout (INB-1) | `PairingDeadlineWaitsForTheGuestAnswer`, `PairingDeadlineFiresOnlyAfterGuestTimeoutAndLateReplyIsDiscarded` |
| VMM exit during an in-flight dial: the orphan sweep resets the host dial vsock; `Unreachable { ECONNRESET }` at once, not `PairingDeadline`, no message (H-1, K-C4) | `VmExitDuringDialReturnsUnreachableReset`, `VmExitDuringDialLeavesNoPairingDeadline` |
| live VM whose guest never replies: `PairingDeadline`, V_h closed, `Abort` on the live session | `PairingDeadlineEndsDialToALiveVmWhoseGuestNeverReplies` |
| in-flight dial ended by the owner reports the admission variant of its cause (L-9): quiescence `Quiesced`, teardown `NotServing`, observed session loss `NoSession`; never `Unreachable { ECONNABORTED }` | `QuiesceAbortsDialAndSendsAbort`, `TeardownEndsPendingDialWithNotServing`, `SessionLossAbortsDialOnBothSides` |
| the dialer drops the dial future: ends like the pairing deadline, no result | `DroppedDialFutureEndsLikeThePairingDeadline` |
| a dial after the guest reconnects pairs (G-V3 dial row 5, L-7) | `DialAfterReconnectPairs` |
| activation while the control session is pending: refused, changes nothing, activates once the session is live (G-V3, L-7) | `ActivationIsImpossibleWhileTheSessionIsPending`, `ActivationWithSessionPendingReturnsPendingThenActivates` |
| activate after teardown (`Retiring`): refused, changes nothing, never activates (G-V3, L-7) | `ActivateAfterTeardownIsRefusedAndChangesNothing`, `ActivateAfterTeardownNeverActivates` |
| beacon happy path (`flows_beacon_conf`) | `BeaconReadyReleasedThenDriverStopReleasesClaim` |
| beacon after driver.stop; stop between accept and read; stale launch; second connection; VMM death (`flows_beacon_conf`) | `BeaconAfterDriverStopIsRefusedAtAccept`, `DriverStopBetweenAcceptAndReadyClosesWithoutClaim`, `StaleLaunchBeaconIsClosedAtReadyAndDoesNotPoisonClaim`, `SecondBeaconConnectionOfSameLaunchIsHeld`, `VmDeathClosesUnreadBeaconConnection` |
| VMM exit with READY released: one step ends the launch and releases the claim; the next launch's READY takes it (D-UP-9) | `VmExitEndsLaunchAndReleasesBeaconClaim` (`flows_beacon_conf`) |

### `fwd_pair` (conformance main `pair_conf`)

| Scenario | Run |
|---|---|
| the pinned attach order: host first, guest on `Paired` (prefixes) | `Paired`, `DgramPaired` |
| happy path: bytes both ways, a FIN each way, both pairs complete, each owner reads its record and closes; the close's {S, R} reaches an ended pair and changes nothing | `CleanBothWaysCompletes` |
| a stopped reader stops the forwarder and then the writer; nothing grows; reading resumes everything | `TargetStallBlocksSenderThenResumes` |
| bytes before attach — the application's write before `Paired`, the far peer's server-first banner before the host attached — wait in their own queue and are forwarded (EVMOD-PAIR, P-22 / P-23) | `EarlyBytesBeforeAttachDelivered` |
| the application's write and FIN before `Paired` are carried out after attach (EVMOD-PAIR) | `EarlyFinBeforeAttachCarriedOut` |
| the guest application resets: the guest pair aborts, the host classifies the 0x3 and resets the far peer, never an EOF (R2-END) | `GuestAppResetReachesRemoteAsReset` |
| the far peer resets: the guest application sees a reset | `RemoteResetReachesGuestAppAsReset` |
| the peer's 0x2 and later 0x3 (its close after this side shut its send) arrive before this side drained: the half-close stays clean (CLEAN-FULL-END) | `HalfCloseThenPeerCloseIsClean` |
| the peer closes with this side's incoming direction open: abort (R2-END) | `PeerCloseWithDirectionOpenAborts` |
| a far peer that shuts down only its read side stalls like a slow reader; no deadline, no abort (owner policy: none) | `ReadShutdownPeerStallsLikeSlowReader` |
| the owner's abort resets both far ends | `OwnerAbortResetsBothFarEnds` |
| the owner's death: the context's release aborts its pair with no record; both far ends see resets (EVMOD-CTL) | `ContextReleaseResetsEveryPair` |
| the VMM exits: the host vsock's error aborts the host pair and resets the far peer | `VmmExitAbortsHostHalf` |
| an RST after the pair completed is inert (END-IDEMPOTENT) | `TrailingRstAfterCompletionIsInert` |
| a dial: the guest application resets; the watch's record is an abort before the end is readable, so `GuestStream` returns `Err(ConnectionReset)` | `WatchedStreamSeesGuestResetAsAbort` |
| datagrams of sizes 1, 0 and 2 delivered once, whole and in order (EVMOD-DGRAM) | `DatagramsInOrderWithEmpty` |
| the vsock accepts part of a frame; the datagram stays queued and the run resumes, nothing duplicated or lost | `PartialFrameResumedNotDuplicated` |
| the target's send blocks: the vsock fills, the source queue fills, the next datagram is dropped there only | `StalledUdpTargetDropsAtSource` |
| the host forwards the destination's FIN as 0x2 and then aborts (0x3) before the guest TcpConnect attaches: the guest's attach classifies the end once, at attach, as an abort (EVMOD-PAIR, U-EV-2) | `HostHalfCloseThenAbortBeforeGuestAttachIsAbort` |
| the guest owner closes a flow before the host attached, its 0x3 still in flight at the host's attach: the attach is accepted and the callback classifies the 0x3 as an abort; the far peer sees a reset (T-4, the in-flight branch; U-EV3-1) | `GuestCloseBeforeHostAttachAborts` |
| the same close processed before the host attaches: V_h is `CLOSING`, the attach is refused `HalfEnded`, the far peer sees a reset, `Abort { flow }` written (T-4, EVMOD-ATTACH-END) | `GuestCloseWithEmptyQueueBeforeHostAttachIsRefusedAndFlowAborts` |
| the destination resets before the host attaches (`eg-drst`): refused `HalfEnded`; the guest closes its unattached flow on `Abort { flow }` (T-4) | `DestinationResetBeforeHostAttachIsRefusedAndFlowAborts` |
| the intake child resets before `Paired` (`eg-rst`): the guest attach is refused `HalfEnded`; the host pair aborts on V_g's 0x3 (T-4) | `IntakeChildResetBeforePairedIsRefusedAndFlowAborts` |
| the destination's banner and the host's 0x2 reach V_g before the guest attaches (`eg-sffin`): the attach seeds a clean half-close and both pairs complete (T-2) | `EarlyVsockHalfCloseBeforeGuestAttachCompletes` |
| the application's write and FIN on the intake child before `Paired` (`eg-wf`): the guest attaches in `CLOSE_WAIT`, every unit delivered, records `Completed` (T-3) | `AppWriteAndFinBeforePairedAttachesInCloseWaitAndCompletes` |
| the far peer's FIN and then a reset with the guest→remote direction open: the guest application reads the FIN's end-of-stream, then the pair aborts `SocketError` `EPIPE` in the reset's step and it sees a reset (EVMOD-END, T-1) | `RemoteFinThenResetAbortsPair` |
| the guest application's FIN and then its reset with the host→guest direction open (`g-rst-hg-only`): `SocketError` `EPIPE`; the host classifies the 0x3 after the 0x2 as an abort (T-1) | `GuestAppFinThenResetAbortsPairWithEpipe` |
| crossing half-closes: each side's 0x2 sent before it processes the other's; the host completes and closes; the guest classifies the 0x3 as none and completes (T-8, CLEAN-FULL-END) | `PeerCloseRacingOwnHalfCloseIsClean` |
| the host's `Forwarder` recovery: abort-all, close of the old context (its step 1 closed the flow's descriptors), a new context opened; the old record is never read (EVMOD-CTL, U-EV-4, D-5) | `ForwarderRecoveryAbortsThenReopens` |
| the host owner's death (a release with its pair live) is final: the pair ends with no record and no recovery reopen follows (D-5) | `HostOwnerDeathIsFinal` |
| the owner aborts a dial's watched stream: record queued, V_h untouched, the stream reports `OwnerAbort` at once and passes no unit; `Abort { flow }` makes the guest abort its pair (WATCH-ABORT, EVMOD-STREAM-ABORT, T-5) | `OwnerAbortOfWatchLeavesVhAndStreamReportsAborted` |
| after the owner's abort and the guest's `Abort { flow }`, the holder's drop only closes V_h; its 0x3 reaches an ended guest pair (T-5, T-6) | `OwnerAbortSendsGuestAbortThenHolderDropIsInert` |
| the context's release shuts the watched V_h; the holder's read fails `ENOTCONN` (F11 corrected); the stream reports the owner's own end (T-5) | `ContextReleaseShutsWatchedVhAndHolderReadsNotConnected` |
| U-R1: the watch already ended (the guest reset, the stream reported `GuestAbort`); the `Forwarder` recovery's close ends the held stream as an owner abort (`strmAbort`, `Abort { flow }` written), the reported end stays `GuestAbort`, and the new context opens | `RecoveryEndsHeldStreamWhoseWatchEnded` |
| the context's release ends a watch with no record: the `GuestStream` resolves the holder's end from the owner's own end of the watch, `OwnerAbort` (EVMOD-STREAM-END) | `ReleasedWatchWithNoRecordReadsAsAbort` |
| the holder drops its stream: the watch is aborted and V_h closed; the guest aborts on the 0x3 (T-6) | `DroppedStreamAbortsWatchThenGuestSeesAbort` |
| the VMM exits with a dial live: the orphan sweep's end is readable before its `sk_error_report`; the holder reads the error, finds no record and no owner end and waits; the callback queues the record and then wakes it; the stream reports the record's abort (EV-END change 3, EVMOD-STREAM-END) | `VmmExitWithWatchRecordInFlightResolvesFromRecord` |
| a datagram vsock's peer ends the direction (0x2) with only the header of a frame queued: the run aborts the pair `FrameInvalid` instead of waiting (EV-DGRAM D1) | `TruncatedFrameAtPeerEndAbortsFrameInvalid` |

### `flow_quota` (`quota_conf`)

| Scenario | Run |
|---|---|
| admission stops at the allocation quota (typed capacity outcome) | `AdmissionStopsAtTheAllocationQuota` |
| a half-closed flow still counts | `DrainingFlowStillCountsAgainstTheQuota` |
| a half-closed flow holds its quota place like an open flow; a withholding guest holds at most its own quota | `WithholdingGuestHoldsAtMostItsOwnQuota` |
| the pair completes; the place is released at the owner's read of its end record, then reused (EVMOD-AB) | `CompletedPairReleasesTheSlotAtItsEndRecord` |
| an abort of a half-closed flow; the place is released at the end record (EVMOD-AB) | `AbortOfHalfClosedFlowReleasesTheSlotAtItsEndRecord` |
| two pairs end; their records are read in queue order and the later one is not readable first (M-1) | `EndRecordsAreReadInQueueOrder`, `EndRecordsAreReleasedInQueueOrder` |

### `cid_lease` (`lease_conf_ok`), `quiescence` (`quiesce_conf`)

| Scenario | Run |
|---|---|
| next-fit assign, the VMM opens its own device | `AssignThenVmmOpensItsOwnDevice` |
| release while the VMM runs is refused; stop exits the VMM, release is last | `ReleaseWhileTheVmmRunsIsRefused`, `StopExitsTheVmmThenReleaseIsLast` |
| next-fit continues from the cursor | `NextFitContinuesFromTheCursor` |
| guest reset ends the VMM, its lease stays Admitted; the restart leases the successor, then retires and releases the predecessor (RESET-EXIT, item 7) | `GuestResetEndsTheVmmAndPlacementRetries` |
| VMM crash frees its CID's device; the same restart sequence (item 7) | `VmmCrashFreesItsCidAndPlacementRetries` |
| serve crash: the orphan is reclaimed before any assign (NO-SURVIVE) | `ServeCrashOrphanIsReclaimedBeforeAnyAssign`, `ReclamationReleasesTheOrphanDeviceThenAssignProceeds` |
| orphan exits by itself before reclamation; unlaunched lease leaves nothing | `OrphanExitsByItselfBeforeReclamation`, `ServeCrashWithUnlaunchedLeaseLeavesNothingToReclaim` |
| two orphans: one boot reclamation (one converge call) kills both before any assign (D-UP-19) | `ReclamationKillsEveryOrphanBeforeAssign` |
| serve crashes after a stop and before the release: the workload stays stopped across the restart, and is placed again only after a redeploy (T-5 ruling) | `StoppedWorkloadStaysStoppedAcrossServeCrash`, `StoppedWorkloadIsPlacedAgainOnlyAfterRedeploy` |
| RESTART-SSOT: the fifth crash-class failure (`att` = 4) is placed again; the sixth exhausts the budget and the workload is not placed again until a `Redeploy`, which places it with its count (hazard `retryOffByOne`) | `FifthCrashAtCeilingBoundaryIsPlacedAgain`, `FailureAtCeilingExhaustsTheBudget`, `RedeployPlacesAnExhaustedWorkloadAgainWithItsCount` |
| RESTART-SSOT: a reclaimed workload at and past the ceiling is placed again and keeps its count (hazard `reclaimNotExempt`) | `ServeCrashPlacesAWorkloadAtTheCeilingAgain`, `ReclaimedWorkloadPastTheCeilingIsPlacedAgain` |
| a successor leased but not launched at a serve crash is placed again with the same count | `ServeCrashBeforeLaunchPlacesTheSuccessorAgainWithItsCount` |
| serve crash while FinalizeFailed's teardown is held: the restarted process finalises it (item 7) | `ServeCrashDuringFinalizeFinalizesAfterRestart` |
| a stop of a workload whose VMM crashed reclaims its lease (item 7) | `StopOfAnEndedWorkloadReclaimsItsLease` |
| **pending GH #325** (`lease_conf_325`): a generation replacement leased at a crash is not lost (hazard `crashLosesPlacedGeneration`) | `RedeployLeasedAtServeCrashIsPlacedAgain` |
| `lease_ok`, and replayed on the recording main `lease_conf_three` (D-18; `ok`'s three offsets, since the five-offset pool never exhausts here): a pool refusal of a restart's successor cleans the predecessor, then the workload is placed with its count (item 7; hazard `refusalKeepsPredecessor`) | `PoolExhaustedRestartCleansThePredecessorThenPlaces` |
| model only, not a `#[quint_test]` (holds on `lease_conf_ok`, violated on `lease_conf_325`): the #325-window guard refuses a `serve` crash while a `Redeploy`'s placement is leased and not launched (item 9) | `CrashIsNotEnabledInThe325Window` |
| model only, not a `#[quint_test]` (holds on `lease_conf_325`, violated on `lease_conf_ok`): the positive twin, `Crash` is enabled in that window without the guard (item 9) | `CrashIsEnabledInThe325WindowWithoutTheGuard` |
| one recovery quiesces, repairs, restores (QHOLD) | `SingleRecoveryQuiescesRepairsRestores` |
| two recoveries overlap: reopen only at count zero | `TwoOverlappingRecoveriesKeepForwardingQuiescedUntilTheLastRestore`, `NoFlowOpensWhileCountedHoldRemains`, `TwoOverlappingRecoveriesReopenOnlyAtZero` |
| a leaked hold (`LeakHold`, during a live recovery) keeps forwarding quiesced past the recovery's restore, then the supervisor fail-stops serve; the restart converges (D-UP-3) | `DroppedHoldKeepsForwardingQuiescedThenFailStops` |
| a restore with a hold another ledger issued (`ForeignRestore`, during a live recovery) returns `ForeignHold` and changes nothing — count, latch, leaked count, the recovery's own hold; the supervisor fail-stops serve (`ForeignQuiescenceHold`); the restart converges (D-UP-4, PD-1; R5-32's foreign-hold step) | `ForeignHoldRestoreChangesNothingThenFailStops` |
| repair failure / failed post-repair audit count as attempts (UP-7) | `RepairFailureCountsAttemptThenRepairs`, `PostRepairAuditFailureCountsAttempt` |
| spent budget fail-stops, restart converges (ADR-0124) | `SpentBudgetFailStopsAndRestartConverges` |
| crash during a recovery | `CrashDuringRecoveryThenRestartRecovers` |
| failed boot write of the shared table refuses boot, retry succeeds (ADR-0184) | `FailedBootTableWriteRefusesBootThenRetrySucceeds` |

### `fwd_safe` (`safe_runs`; not a conformance main)

| Scenario | Run |
|---|---|
| happy path with backpressure: `sk_data_ready` kicks the run; a full target stops it (`EAGAIN`, `SOCK_NOSPACE`); the reader's read wakes `sk_write_space`, which wakes the wait queue and runs the forwarder's entry; the pair resumes | `TargetStallResumesOnWriteSpace` |
| the reader's wakeup lands while the stopped run is still running: PENDING was cleared before the run, one more run follows | `WakeupDuringRunIsNotLost` |
| a clean end: classified in `sk_state_change`, FIN forwarded, restore / clear / id + record / free work, grace period, cancel, free; one record read | `CleanEndProducesOneRecordAndFrees` |
| a TCP input callback enters the target's override as the end restores and clears: it reads NULL and calls the restored original (rule 2) | `LateCallbackAfterEndCallsOriginal` |
| a callback holds the pair when the free is queued: the grace period waits for it; its `queue_work` is cancelled by the free (rule 3) | `CallbackHoldingPairDelaysFree` |
| `PAIR_STATE` after the id left the table: `ENOENT`, the pair untouched (rule 4) | `ControlCallAfterIdRemovalIsEnoent` |
| `ABORT` racing the pair's own clean end: cancelled by the free; one record | `AbortRacingEndIsCancelledByFree` |
| the orphan sweep's `sk_error_report` aborts the pair `SocketError` | `SweepReportedEndAbortsSocketError` |
| EVMOD-SWEEP (F-S1): a run reads the source between the sweep's `peer_shutdown` and `sk_err` stores; it waits for `vsock_table_lock` until the sweep's `sk_error_report` returned, then aborts `SocketError`; no alarm | `SweepStoresBeforeErrReadRechecksUnderTableLock` |
| EVMOD-SWEEP, weak ordering: the sweep's `sk_err` store is visible first; the run reads `-ECONNRESET` with no class recorded and waits for the lock the same way | `SweepErrVisibleFirstRechecksUnderTableLock` |
| the context's release ends a pair with no record | `ReleaseEndsPairWithoutRecord` |
| a watch's own end (record, then the original), then the owner's `ABORT` is `ENOENT`; one record | `WatchOverrideEndThenOwnerAbortIsEnoent` |
| the owner's `ABORT` of a watch races the sweep's callback inside the override: the abort owns the end; one record; the grace period waits for the override | `WatchOwnerAbortRacingSweepOneRecord` |
| the write-space closure: a waker (`sock_wfree`, no lock, no RCU section) holds `wqlk(t)` inside the entry's function as the end reaches the entry's removal; the removal waits; the pair ends and the free runs | `WfreeWakeFinishesBeforeRemoveWait` |

### `udp_slots` (`slots_conf`)

`DatagramAssociationClaimsPairsAndRoutes`, `TwoSocketsClaimDistinctSlots`,
`RefusedDiscardsParkedFramesAndOnlyNewDatagramReassociates`,
`HostAbortOfLiveAssociationEndsItAndKeepsSlotOwned`, `DatagramQueuedBeforeAnEndIsDiscardedWithTheQueue` (EVMOD-DGRAM),
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
| K-A3 | a flow socket attached before its request is read has the request bytes forwarded to the target as payload (why H-1 exists) | the forwarder's semantics (an attach-time run forwards the queued bytes): P-67 (`findings-evmod.md`) |
| K-A4 | the host's close of a host-opened V_h (aborted or session-lost dial) reaches the guest and tears down V_g, accepted or still in the accept queue | proven: V-24 (`findings-v24.md`, P-38) |
| K-A5S | the end of a guest-opened control connection on the host (closed at accept or read, or lost) reaches the guest owner, which observes it (`GuestSessionLossObserved`, D-UP-8). Source: releasing an established vsock stream sends the peer a SHUTDOWN, or an RST after the peer's SHUTDOWN (`virtio_transport_release` → `virtio_transport_close`, `net/vmw_vsock/virtio_transport_common.c:1312-1319, 1280-1296` @ v7.0), whichever side connected | proven: K-A5 spike (`docs/feature/netns-density-295/spike/findings-ka5.md`, WORKS: 4,750 trials on 7.0.0-29 host and guest, every host close, linger-0 close, process kill, listener close with a queued child and refused connect ended the guest's connection with no guest timer; 0 hangs, 0 false ends). The guest observes an established end as EOF / `EPOLLRDHUP`, never `ECONNRESET` (a connect-time outcome only); a reset close is indistinguishable from a graceful one |
| K-C1 | a duplicate CID is refused at device creation, the first holder unaffected; a CID is free right after the last reference closes | proven: P-16, V-30 Q2d (`findings-v30-guest-reset.md`). Unreachable in the design (lease uniqueness); reached only by hazards |
| K-C4 | the VMM is the only holder of its vhost-vsock fd; no Overdrive process keeps a copy past VMM exit, so the device is released and host flows reset at VMM exit | proven: V-30 Q2c, Q2d |
| K-C5 | Boot reclamation kills every VMM the new process does not supervise before any `assign`, and a reclaimed VMM's device is released before its cgroup can be removed. Source: an exiting task drops its files (kernel/exit.c:971 @ v7.0) and runs its task work (kernel/exit.c:976 @ v7.0), where the final fput of a user task is deferred (fs/file_table.c:528-530 @ v7.0) and reaches vhost_vsock_dev_release; cgroup_rmdir waits in cgroup_drain_dying until every task has passed cgroup_task_dead (kernel/cgroup/cgroup.c:6309 @ v7.0). OPEN: the Tier-3 serve-restart reclamation scenario V-RECLAIM; not contract-shaping | OPEN validation item V-RECLAIM (Tier-3 serve-restart reclamation scenario of NO-SURVIVE; not contract-shaping) |
| K-C6 | a guest reset (PID-1 crash, triple fault) ends the VMM in the pinned fork | validation item of ADR-0182: fork build with the change, PID-1 crash under `panic=1` and triple fault; not contract-shaping, blocks DELIVER of the fork step (V-30 Q3 is a reading, not a result) |
| K-B1 | closing T_g / Q_g is observed by the host as the association's end, even when accepted after the guest closed | P-22; P-37 |
| K-B2 | the guest `sock_release` report may be lost; the level-triggered audit finds the slot | R5-14 (Tier-3) |
| K-B4 | draining a slot socket with `recvmsg(MSG_DONTWAIT \| MSG_TRUNC)` until `EAGAIN` leaves its receive queue empty: a UDP socket has no other queue a datagram can come back from | proven: P-72 § 7 (V-EV-DGRAM, `findings-ev-dgram.md`) |
| F1 | a peek returns no vsock credit and no TCP window; only a dequeue advances `fwd_cnt` and sends the credit update (`net/vmw_vsock/virtio_transport_common.c:806-809`, `:536`, `:583`, `:654-655` @ v7.0), and a credit update or an ACK that frees space wakes the writer (`:1440-1441`; `include/net/tcp.h:770-776` @ v7.0) | proven: P-67, P-68 (`findings-evmod.md`, `findings-evmod-followup.md`: the stalled-flow bounds) |
| F2 | the forwarder's `sk_state_change` runs per received SHUTDOWN / RST with the flags already in `peer_shutdown`, before end-of-stream is readable (`virtio_transport_common.c:1443-1464`, `:1244-1253`, `:1661`, `:1705` @ v7.0; `af_vsock.c:2326-2328`) | proven: P-69 (V-EV-END, `findings-ev-end.md`) |
| F3 (restated) | a TCP reset is reported by the half's `sk_error_report` (`tcp_done_with_error`, `net/ipv4/tcp_input.c:4890-4902` @ v7.0) with `sk_err` `EPIPE` in `TCP_CLOSE_WAIT`, else `ECONNRESET` (`tcp_reset`, `tcp_input.c:4918-4930`), and the next `recvmsg` / `sendmsg` returns it only while the forwarder still reads that half (after a FIN it does not: P-69 hazard 16), so the model's `TcpRst` reports it in its own step (T-1); a FIN is end-of-stream after every queued byte (`tcp_input.c:4953-4954`, `tcp.c:2794-2795`) | proven: P-69 (V-EV-END), P-68 (rst / kill shapes) |
| F4 | a vsock `SHUT_WR` never overtakes accepted bytes (`virtio_transport_common.c:1084-1097` @ v7.0) | proven: T3 (P-56) |
| F5 | `tcp_abort` sends a reset (`net/ipv4/tcp.c:5087-5141` @ v7.0) | proven: P-68 (23 of 23 close, kill and RST shapes ended with no live pair) |
| F6 | the release of a forwarder context (the last close of `/dev/ovd_fwd`, or its process's exit) aborts every pair and watch of the context before it returns, with no record; every other id produces exactly one end record, never dropped | proven: P-70 (V-EV-CTL, `findings-ev-ctl.md`) |
| F7 | the VMM's exit sets `sk_err` (`ECONNRESET`) on every host vsock to the guest, reported through `sk_error_report` (`drivers/vhost/vsock.c:746-750` @ v7.0) | proven: V-30 Q2c (K-C4) |
| F8 (restated) | a UDP socket's full receive queue drops (`net/ipv4/udp.c:1721-1731` @ v7.0); a frame of at most 59,008 B fits the vsock buffer because datagram-mode attach refuses a vsock half whose `buffer_size` is below `MAX_GUEST_DATAGRAM_FRAME` (`BufferTooSmall`; default 256 KiB, `net/vmw_vsock/af_vsock.c:199`) | proven: P-72 D3 (V-EV-DGRAM, `findings-ev-dgram.md`) |
| F9 | attach reads the half's state and installs its callbacks in one section, and every transition that changes that state runs under a lock the section holds: vsock receive processing under `lock_sock` (`net/vmw_vsock/virtio_transport_common.c:1661`, `:1705` @ v7.0), TCP input backlogged while the socket is owned and processed at `release_sock` (`net/ipv4/tcp_ipv4.c:2318-2321`), the device release's orphan sweep under `vsock_table_lock` (`net/vmw_vsock/af_vsock.c:453-471`, `drivers/vhost/vsock.c:721-750`); so a transition is in the attach-time state or seen by a callback, never neither (the model's atomic `Attach`) | discharged: the source reading and P-71 (`h_noseed`, `findings-ev-pair.md`); the widened-window race stress is a check of the module suite (DELIVER). Ended halves (TCP `CLOSE` with `sk_err`, vsock `CLOSING`, the orphan sweep's `sk_err`) are refused, not accepted (EVMOD-ATTACH-END; F14) |
| F10 | a TCP reset received after a FIN leaves `recvmsg` returning end-of-stream (`SOCK_DONE` is tested before `sk_err`, `net/ipv4/tcp.c:2794`, `:2797` @ v7.0; `SOCK_DONE` is set only by `tcp_fin`, `net/ipv4/tcp_input.c:4954`) and is reported by `sk_error_report` (`tcp_done_with_error`, `tcp_input.c:4890-4900`) | proven: P-69 (V-EV-END (c), the FIN-then-reset shape) |
| F11 (corrected) | a vsock socket whose own receive side is shut returns 0 from `recvmsg` at once only while it is `ESTABLISHED` (`net/vmw_vsock/af_vsock.c:2545-2548` @ v7.0); once it left `ESTABLISHED` without `SOCK_DONE` (`vsock_shutdown` clears it, `:1220`) it returns `-ENOTCONN` (`:2522-2531`); a send on it fails. After the orphan sweep (which stores `SS_UNCONNECTED`, numerically `TCP_ESTABLISHED`, in `sk_state`, `drivers/vhost/vsock.c:746-750`) the queued bytes are read and then `-ECONNRESET` is returned (`af_vsock.c:2326-2328`, `:2439-2440`); after a peer SHUTDOWN, 0 once the queue is empty. The model's `HolderEnd` choices follow this | proven: P-70 § 3.5 (`findings-ev-ctl.md`: `ENOTCONN` in 10 / 400, 4 / 400, 1,916 / 4,096 after the forwarder's `SHUT_RDWR`); the sweep-path order: P-69 (640 of 640 records present at the holder's observation) |
| F12 | the datagram source forwarder sends nothing on a vsock it shut (EPIPE); the only datagram-mode 0x2 is a peer outside the design (`DgHalfClose`; no Overdrive owner half-closes a datagram vsock, U-EV-5), by which EV-DGRAM D1 is checked | P-72 § 8 (`findings-ev-dgram.md`: an independent peer left a frame incomplete at its end and the pair hung; with the rule it aborted at once) |
| F13 | the forwarder's abort of a watch sends nothing on the vsock and changes no socket state of V_h (WATCH-ABORT) | the module code; the spike's `ABORT` (P-70 § 3.8, `findings-ev-ctl.md`) and the module suite's case "`ABORT` of a watch sends nothing on the vsock and changes no socket state" (DELIVER) |
| F14 | a vsock `CLOSING` is set only on a connected socket, when a packet leaves `peer_shutdown == SHUTDOWN_MASK` with the receive queue empty — a 0x3 (`virtio_transport_common.c:1448-1452`) or an RST (`:1463-1464`), both through `virtio_transport_do_close`, which sets `TCP_CLOSING` only when `vsock_stream_has_data` is 0 (`:1249-1252`); with bytes queued the socket stays `ESTABLISHED`, for an RST too — and the orphan sweep writes the state (`sk_state = SS_UNCONNECTED`, `drivers/vhost/vsock.c:748` @ v7.0) and then `sk_err = ECONNRESET` (`:749`), then runs `sk_error_report` (`:750`), all under `vsock_table_lock` (`net/vmw_vsock/af_vsock.c:458-471` @ v7.0), which the attach section holds, so an attach sees both writes or neither; the order inside the sweep is immaterial to attach (U-EV3-6) | discharged: the source reading and P-70 § 3.6 / P-71 (`eg-rst`, `eg-drst`: the refusals measured, `findings-ev-ctl.md`, `findings-ev-pair.md`) |
| S1 | RCU: a grace period started after a reader entered ends after it left; `queue_rcu_work` (`kernel/workqueue.c:2649-2664` @ v7.0) | the source reading; V-EV-SAFE |
| S2 | `smp_store_release` / `smp_load_acquire` order the restore before the clear for an override that reads NULL; `rcu_assign_pointer(p, NULL)` is a plain `WRITE_ONCE` (`include/linux/rcupdate.h:575-576` @ v7.0) | the source reading; V-EV-SAFE |
| S3 | workqueue: PENDING cleared before the function runs (`kernel/workqueue.c:3254`, `:3288`), non-reentrant (`:1212-1214`), `cancel_work_sync` disables, drops a pending run and waits for a running one; a `queue_work` on a disabled item is dropped (`:4444-4466`, `:2415`) | the source reading; V-EV-SAFE |
| S4 | the vsock receive path runs the callbacks under `lock_sock` (`virtio_transport_common.c:1661`, `:1705`); the orphan sweep stores `SOCK_DONE`, `peer_shutdown`, `sk_state`, `sk_err` in that order without the socket lock and then calls `sk_error_report` (`drivers/vhost/vsock.c:725-750`); a `recvmsg` that reads `peer_shutdown & SEND_SHUTDOWN` before `sk_err` returns 0 (`af_vsock.c:2326-2328`, `:2439-2442`) | the source reading; V-EV-SAFE; FINDING F-S1 |
| S5 | a TCP target wakes `sk_write_space` after an `EAGAIN` send once space frees (`net/core/stream.c:175-181`), and the socket's own `sk_write_space` wakes its wait queue with `EPOLLOUT` (`net/core/stream.c:34-51`, `net/core/sock.c:3617-3660` @ v7.0) | the source reading; V-EV-SAFE's full-target resume cases (TCP, vsock and UDP targets, DELIVER). P-67 measured the earlier `sk_write_space` override and is not evidence for the wait-queue entry |
| S6 | the wait-queue function runs only with the head's lock held and interrupts disabled, and `remove_wait_queue` takes that lock (`kernel/sched/wait.c:69-76`, `:104-108`, `:118-127` @ v7.0); `sock_wfree` calls `sk_write_space` with no socket lock, outside any RCU section and with bottom halves enabled for every socket without `SOCK_USE_WRITE_QUEUE` (`net/core/sock.c:2675-2693` @ v7.0; only TCP sets it, `net/ipv4/tcp.c:477`) | the source reading; V-EV-SAFE's full-target resume cases |
| Q-1 | A task exit of a runtime component is detected by the 1 s audit; a repair that returns `Err` is a failed attempt | OPEN validation item: DST invariant under UP-7 (ADR-0124) |

## Model-only claims and what discharges them

| Claim | Module | Why the model does not check it | Discharged by |
|---|---|---|---|
| a foreign hold (issued by another `QuiescenceLedger`) that the owner refused with `ForeignHold` and that is then dropped unredeemed counts as leaked at its issuing ledger (D-UP-4, PD-1) | `quiescence` | the issuing ledger is another owner's; the model has one owner, the hold enters only as the stimulus `ForeignRestore`, and nothing here represents the second ledger's leaked count | a sim/unit scenario (DISTILL): two ledgers; the owner's `restore` of a hold issued by the second ledger returns `ForeignHold`; the hold is dropped; the second ledger's `leaked()` rises by one and the owner's `leaked()` is unchanged |

## Conformance granularity: `fwd_pair`

For `fwd_pair` the driving ports are the owners' dispatch (`dispatch_next` in the guest, the host owner's event step)
and the `FlowForwarder` calls (`attach`, `abort`, `abort_all` through the context's release, `take_end`); the
forwarder's runs and callbacks (`Hop0`, `Hop1`, `HopEnd0`, `HopEnd1`, `VCtl`, `FwdErr`) and the environment actions are
the Sim adapters' scripted facts. The projection reads `FlowForwarder::pair_state` (per direction `source_end`,
`target_shut`, `moved`) and `FlowForwarder::take_end` (the end records), and the flow phases through
`flow_observation()` (host) and `guest_observation()` (guest). `SimFlowForwarder` is the named arbiter of the
forwarder's kernel facts — the queues, the per-direction state, the classification, the end records — and of the far
readers' delivered units (`// conformance-lint: sim-arbiter-ok`), paired with the Tier-3 runs V-EV-END and V-EV-CTL
and the module suite. Model-only, never projected: `loss`, `lost`, `tcls`, `ucls`, `fwdAfterEnd`, `trail`, `faults`,
`sdrop` / `rdrop` / `fdrop`, `conf_step`, `att`, `clsAt`, `ncls`, `snap`, `fwdOld`, `rstOpen`
and `holderReadAfterAbort` (QPROJ-8, U-EV-8), and `vs(s).misc`, `guestAbortSent`, `gh.hobs`,
`gh.woken`, `gh.vhTouched` and `gh.dropped` (the holder's observation and drop are the driver's own
calls on the `GuestStream`; the stream's reported end is `hres`). `vs(s).sshut` is not model-only: QPROJ-8 classes
it as an arbiter read (`SimFlowForwarder`'s per-half own-shutdown flag, Tier-3 pair P-71 `h_lateshut`), as it does
`vs(s).st` and `vs(s).err`. QPROJ-8 classes the
variables (U-EV-8): `gen` and a pair's `cgen` are owner observations
(`flow_observation().forwarder_generation` and the `generation` of the flow's `Paired` phase, both
`ForwarderGeneration`); `oldCtx` is read from `SimFlowForwarder`'s context table (named arbiter, Tier-3 pair P-70);
`unclassified` is an owner observation (`inventory().unclassified_eof`). The owner actions map to the
host owner's `Forwarder` recovery (`Abort` as `abort_all`, `CtxRelease` as the drop of the old
`KernelFlowForwarder`, `Reopen` as `KernelFlowForwarder::open()` and `probe()`) and the guest owner's close of a flow
it has not attached (`CloseUnattached`); `AttachRead` / `AttachHook` and `UnclassifiedEof` are reached only by
hazards (the latter is the `unclassified_eof` counter of `inventory()`). `HolderEnd`
is the driver's own `read` on its `GuestStream`'s V_h in the Sim adapter (the scripted end fact, Eof or Err), `ReadEof`
on the dialer the stream's resolved result; `DgHalfClose` is a scripted vsock fact of the Sim adapter (no owner path
produces it); `WatchRecord` is a `conf_step` variant no step produces (the drivers map it to a hazard-only error).

The variables of T-1 to T-9 are classed as QPROJ-8 pins them: `tcpErr(s)` is the
`SimFlowForwarder` TCP half's `sk_err` (named arbiter; the projected outcome is the record
`ForwardEnd { outcome: Aborted { cause: SocketError { side, errno } } }`, here `pr(s).cause` / `pr(s).errno`);
`tcpSt(t, s)` is a derived definition over `tcpErr`, the far writer's FIN and this side's own FIN, not projected (its
inputs are: `tcpErr`, `dir(d).wfin`, `dir(d).dstShut`); `vsSt(s)` / `vsErr(s)` are `vs(s).st` / `vs(s).err` (`SimFlowForwarder`'s per-half state and
`sk_err`, arbiter); `attRes(s)` is the attach / watch result the owner's dispatch received and `attEnded(s)` the
owner's `forwarder_attach_ended` (host `flow_observation()`, guest `guest_observation()`); `strmAbort` is the dial's
entry leaving `flow_observation().cids[cid].dials` in the aborting owner step; `hres` is what the driver, as the
holder, observes from the `GuestStream`; `guestAbortSent` is model-only (its delivery is `owner_flows`'
`toGuestAbort`); `liveWatch` is the derived `live(t, h)` in mode watch, not projected (its input `pr(h).ph` is,
cross-checked against `inventory().live_watches`); `vh` is `vs(h)` with the vsock queues `dir(gh).q1` /
`dir(hg).q1` (the Sim vsock socket, arbiter; the ghost `vhTouched` is model-only); `frq(d)` is the derived
`headPartial(t, d)`, not projected (its input `dir(d).q1` is the Sim vsock queue, arbiter); `ownShut(s)` is
`vs(s).sshut` (arbiter); `swp(h)` is `vs(h).err` while the watch is live (the sweep's first step taken, its
`sk_error_report` — `FwdErr(h)` — not yet run). Model-only: `dropped`, `vhTouched`. Actions: `TcpRst(s, after_fin)` (the Sim far peer's reset after or before its FIN;
`SimFlowForwarder` runs the half's `sk_error_report` in the same step; `after_fin` equals the far writer's FIN in the
state, so a trace names it explicitly), `Attach(s)` with an ended half (the same dispatch; `HalfEnded { side, errno
}`), `Abort(h)` on a watch (the owner's stream abort and its `Abort { flow }` write in one owner step), `GuestAbort(g)`
(the guest owner's dispatch of `Abort { flow }`), `HolderDrop` (the driver drops its `GuestStream`). The reset and its report are one step (`TcpRst`, T-1).

## Pinned design points

Each point below is a rule the design pins and the model encodes.

1. INB-1: a dial to an `Active`, not quiesced allocation whose control session is not live is the typed result
   `NoSession` (immediate, no state change; `DialNoSession`, `TypedRefusalChangesNothing`). The load-bearing
   rule that a dial never opens without a live session is covered by the `dialNoAdmission` hazard.
   The order of decision follows only facts the owner observes: Active (else `NotServing`) →
   not quiesced (else `Quiesced`) → the control session live as the owner observes it (else `NoSession`,
   whatever `vmUp`) → the vsock connect, whose failure is `Unreachable`. The owner does not observe `vmUp`:
   `DialUnreachable` requires the session live to the owner (`sessLive`, which counts a `dead` session whose
   loss is unprocessed) and the VM gone; `DialNoSession` requires only `not(sessLive)`.
2. QHOLD (D-UP-3): a dropped hold increments the owner's leaked-hold count; the stimulus is the component-free
   `LeakHold`; the recovery supervisor (ADR-0124) fail-stops serve at its audit tick when the count is above zero,
   whatever the recoveries do (`FailStop`; `LeakedHoldEndsInFailStop`; hazard `leakIgnored`). A failed repair is only
   `RepFail` (counted, the hold kept); no `Err` propagates out of a recovery with its hold.
3. ADR-0147: no isolation claim beyond the quota; the model assumes no relation between node capacity and the
   sum of quotas.
4. H-1: a VMM exit while a dial awaits its pairing reply: the device-release orphan sweep resets the host dial
   vsock (`drivers/vhost/vsock.c:745-750` @ v7.0, `sk_err = ECONNRESET`); `VmStop` ends every requested host dial of the
   stopped launch with `Unreachable { ECONNRESET }`, no message. `PairingDeadline` remains for a running VM whose guest
   never replies.
5. L-9: a dial in flight that the owner ends reports the admission variant of its cause: quiescence `Quiesced`,
   allocation teardown or retire `NotServing`, observed control-session loss `NoSession`. A vsock connect or I/O error
   (VM or device gone) is `Unreachable { errno }`. No owner-initiated end is `Unreachable { ECONNABORTED }`.
6. M-4: `owner_flows` models neither capacity nor the quota; no Quint module models the
   `GuestStreamDial` capacity refusal (the dial's `Capacity` outcome); it is discharged by the seeded-sim scenario
   R5-53 against the production owner (`docs/feature/netns-density-295/feature-delta.md`). `flow_quota`
   (`AdmissionStopsAtTheAllocationQuota` and the quota invariants) models the admission quota only. Dial cancellation is `DialCancel`
   (`DroppedDialFutureEndsLikeThePairingDeadline`).
7. U-EV-1: attach accepts a TCP half that received its FIN before attach (`TCP_CLOSE_WAIT`) and a vsock half whose
   peer already ended its direction; the attach-time run carries an early end out (`EarlyFinBeforeAttachCarriedOut`).
8. U-EV-2: an end that reached a vsock half before attach is classified at attach, once, from the accumulated flags
   ({S} clean, R present abort), with this side's send open (F9).
9. U-EV-3: a watch ended by its owner leaves V_h untouched (WATCH-ABORT); end-of-stream with no record is an abort.
10. U-EV-4: the host owner is never replaced in-process; the `Forwarder` recovery is abort-all → close old → open
    new, modelled in `fwd_pair`; `owner_flows` needs no owner-death action.
11. U-EV-5: a clean class completes a datagram pair, but no owner path produces a datagram-mode 0x2; the model keeps
    the rule and reaches it through `DgHalfClose`, a scripted fact of the Sim adapter.
12. U-EV-6: a UDP target send returns `-EAGAIN` only when that socket's own send buffer is full; a far receiver that
    stops reading drops at its own receive queue. The model has both (`UdpBlock`, `dq2` full) and
    `DropsOnlyAtSourceQueue` reads "only at a UDP receive queue — the source's or the far receiver's — never inside the
    forwarder or the vsock". The run `StalledUdpTargetDropsAtSource` is the blocked-send case.
13. U-EV-7 / U-EV-8: `AbortCause::Unclassified` (record cause 5) is the model's label; § Conformance granularity
    classes the variables.
14. U-EV-9 (EVMOD-DGRAM D1): the callback's class decides first. An abort-class end (0x3, RST) aborts `PeerAbort`
    whatever is queued; every production end of a datagram vsock is of this class. A clean-class end (0x2, no
    production producer) sends every whole frame and then completes, or aborts `FrameInvalid` when a frame is left
    incomplete (ADR-0201). The model encodes both (`ClassFixedAtTransition`; `DgHalfClose`). The conformance run
    `TruncatedFrameAtPeerEndAbortsFrameInvalid` needs the Sim adapter to script the 0x2.
15. WATCH-ABORT (ADR-0211): the owner's abort of a watch does not shut V_h; the `Err` branch of F11 is
    `ContextReleaseShutsWatchedVhAndHolderReadsNotConnected`, the `Eof` branch `ReleasedWatchWithNoRecordReadsAsAbort`.
16. U-EV-11: after the orphan sweep `sk_state` holds `SS_UNCONNECTED`, which equals `TCP_ESTABLISHED`
    (`drivers/vhost/vsock.c:748`, `include/uapi/linux/net.h:50`, `include/net/tcp_states.h:13` @ v7.0), so the holder
    reads the queued bytes and then `-ECONNRESET` (`net/vmw_vsock/af_vsock.c:2326-2328`, `:2439-2440`), not 0. The
    error is readable before `sk_error_report`, and the stream waits for the in-flight record (T-9).
17. T-4 (`GuestCloseBeforeHostAttachAborts`): the run encodes the in-flight branch (the guest's 0x3 still in transit at
    the host's attach: accepted, then aborted by the callback). The queued-data branch has no production producer on
    V_h — the host attaches before `Paired` (EVMOD-PAIR), reads the guest's request first (`NoAttachBeforeRead`), and
    the guest sends nothing more on V_g before its own attach — and is reached on the guest side by
    `HostHalfCloseThenAbortBeforeGuestAttachIsAbort`. A guest close processed before the host's attach with nothing
    queued leaves V_h `CLOSING` and is refused (`GuestCloseWithEmptyQueueBeforeHostAttachIsRefusedAndFlowAborts`).
18. T-1 (`ResetErrnoMatchesState`): `EPIPE` iff the half is in `CLOSE_WAIT`; a half in `LAST_ACK` gets `ECONNRESET`
    (`net/ipv4/tcp_input.c:4918-4930 @ v7.0`); the pair aborts either way and the owners treat both as a reset.
19. T-4, datagram mode: a connected UDP half with a pending `sk_err` (an ICMP error) is not refused at attach: the
    attach-time run of the UDP → vsock direction reads the error first (`__skb_recv_udp`,
    `net/ipv4/udp.c:1986-1988 @ v7.0`) and the pair aborts `SocketError`, which the host owner reports as
    `DestinationUnreachable` (D26-PINS) (`UdpErr` → `FwdErr`). A refusal in datagram mode is reached only through V_g
    `CLOSING` (the host's abort before the guest attaches).
20. T-4, the dial's watch: the watch is installed when V_h connects, before the request is written, so `fwd_pair` has
    no refused-watch state. In `owner_flows` a refused dial watch maps by its cause: the orphan sweep (`ECONNRESET`,
    the VMM exited) is the `VmStop` path (`VmExitDuringDialReturnsUnreachableReset`, no message); any other
    `HalfEnded` end of V_h before the request is `HostAbortAccept(h)` (`Unreachable { errno }`, `Abort { flow }` while
    the session is live).
21. T-5 (`GuestPairEndsAfterOwnerAbort`): `fwd_pair`'s only session-down state is the VM's end (`VmmExit`), after which
    the guest side is gone; a lost session aborts every flow on both sides, which `owner_flows` checks
    (`SessionLossAbortsDialOnBothSides`, `GuestFlowsDoNotOutliveSession`).
22. F-S1 (EVMOD-SWEEP, ADR-0180; `fwd_safe`): the orphan sweep stores `peer_shutdown = SHUTDOWN_MASK` before
    `sk_err = ECONNRESET` with no socket lock and no barrier (`drivers/vhost/vsock.c:747`, `:749` @ v7.0), so a
    forwarder run already queued can read `V_h` inside the stores and see an end with no class recorded. The run takes
    `vsock_table_lock`, which the sweep holds across its stores and its `sk_error_report`, re-reads the recorded class
    and acts on it (`SocketError { ECONNRESET }`). Modelled as `WEnd` (the unlocked read; an unclassified end parks the
    run in `wrun = "lock"`) and `WRecheck` (enabled only when the sweep does not hold the lock); the sweep's two
    visible stores may land in either order. `NoUnclassifiedEnd` is part of `Safety`; the hazard is
    `fwd-safe-hazard-no-sweep-recheck`, and the runs are `SweepStoresBeforeErrReadRechecksUnderTableLock` and
    `SweepErrVisibleFirstRechecksUnderTableLock`. `fwd_pair`'s `VmmExit` sets the host half's `sk_err` with the state in
    one step, so it has no window. `fwd_pair` names the invariant `NoUnclassifiedEof`; in `fwd_safe` it is
    `NoUnclassifiedEnd`.
23. U-R1 (EVMOD-CTL recovery step 1 with a held dial; D-3, D-5): a held `GuestStream` whose watch already ended
    (`guest_end: Some(_)`) is ended by the recovery as an owner abort — its `dials` entry leaves, `Abort { flow }` is
    written while the session is live — while what the stream already returned and its `end()` class stay as they
    were. Modelled in `recoveryClose` (the `CtxRelease(h)` that found no live watch): `strmAbort` set,
    `guestAbortSent` while the VM is up, `hres` unchanged; `HolderOutcomeFromWatch` is judged at the moment the stream
    reports its end (ghost `gh.hresCls`); invariant `RecoveryEndsHeldStream`; run
    `RecoveryEndsHeldStreamWhoseWatchEnded` on `pair_conf`.
24. T-5 ruling: the composition drives `WorkloadLifecycle`; `wait` / `done` / `retry` come from QPROJ-2W. `Crash`
    keeps a stopped, unreleased workload `done` (`StoppedWorkloadNotPlaced`, hazard `crashForgetsStop`), and the
    conformance configuration bounds each workload's failures at 4. The D-4 / D-5 pins are modelled as stated in
    § `fwd_pair`. M-1 is modelled in § `flow_quota`.

## Modelling choices for `fwd_pair`

- Instances: besides `pair_ok`, `pair_dgram` and `pair_conf`, `fwd_pair` has `pair_watch` (the dial), `pair_live`
  (liveness), `pair_wlive` (the watch's liveness), `pair_dlive` (the datagram liveness) and `pair_stall`
  (`StalledSenderIsBlocked`), because the stream instance with every fault exceeds TLC's budget and TLC's liveness mode
  is far slower per state (§ *Modules*, coverage note). Windows and credits are one unit; the datagram credit is three
  items, the largest frame.
- Hazard files: every `fwd_pair` hazard file holds at most two instances (`<name>.qnt`, `<name>_2.qnt`, …); a file whose
  instances plus the instances of `fwd_pair.qnt` it imports exceed ten runs the Apalache server out of its fixed
  2,048 MiB compile heap (`java.lang.OutOfMemoryError`, reported as a tool error).
- `fwd-pair-hazard-release-keeps-pairs` violates `ReleasedContextHoldsNoPair` and `NoForwardAfterEnd`, not
  `NoEofOnPeerLoss`: a pair that keeps forwarding after its owner's death still delivers every byte and ends as
  before; no far end is shown an EOF it should not see.
- `fwd-pair-hazard-check-then-hook` (state read, a transition, callbacks installed) violates
  `AttachTimeEndClassifiedOnce` and `NoUnclassifiedEof` on the design's alarm (the unclassified end-of-stream aborts).
- `ClassFixedAtTransition` also requires that no transition is classified otherwise than with the socket's true send
  state; `IncompleteFrameAtEndAborts` is the datagram rule's state form, and its teeth in time are `EveryEndResolves` on
  `pair_dlive`.
- `NoHolderReadAfterWatchAbort` has its teeth in `holderTrustsRead` (a stream that passes reads to V_h after the
  owner's abort: `fwd-pair-hazard-holder-trusts-read`).
- `owner_flows` has no owner-death action: the host owner is never replaced in-process, and the `Forwarder` recovery
  (abort-all, close the old context, open a new one) is modelled in `fwd_pair`.
- `udp_slots` has no generation cells: "the slot queue not discarded at an end" is the hazard `keepFramesOnEnd`
  (U-2 off), and the run `DatagramQueuedBeforeAnEndIsDiscardedWithTheQueue` states the rule.
- `fwd-pair-watch-safety-tlc` is `ci = false` and `fwd-pair-safety-tlc` carries `timeout_secs = 2400`: both exceed the
  two-minute `ci` bound.

## Hazards

One hazard per design decision (WITNESS-HAZARD-RULE): each row removes one design rule (or kernel assumption) and shows
the checker finds a counterexample. A rule's variants — the same hazard as a named run, a second invariant of the same
rule, a second instance — are not separate rows. Two hazards are `run` checks: `cid-lease-hazard-crash-loses-placed-generation-run` and
`cid-lease-hazard-refusal-keeps-predecessor-run` are the teeth of the named runs of `lease_conf_325` (pending GH #325)
and `lease_conf_three` (D-18), which carry one run each and no invariant that sees the rule.

| Check | Instance | Rule removed | Property / run that fails |
|---|---|---|---|
| `owner-flows-hazard-split-claim` | `flows_bugSplitClaim` | L-3 off: session check and claim are two steps | `OneLiveSession` |
| `owner-flows-hazard-attach-before-read` | `flows_bugAttachEarly` | H-1 off: connect and attach before the request is read | `NoAttachBeforeRead` |
| `owner-flows-hazard-no-abort-on-loss` | `flows_bugNoAbortOnLoss` | DG-5-F off: session loss does not abort the CID's flows | `NoAdmitWhenClosed` |
| `owner-flows-hazard-late-paired-resurrects` | `flows_bugLatePairedResurrects` | U-5 off: the host acts on a Paired for a flow it already closed | `ClosedIsTerminal` |
| `owner-flows-sess-env-guest-misses-session-loss` | `flows_sess_envGuestMissesSessionLoss` | K-A5S dropped: the guest owner never observes the end of the control connection it holds (D-UP-8), so it never re-opens a session, and a launch whose session ended before it went live, or was lost, never holds one again | `SessionBecomesLive` |
| `owner-flows-hazard-peer-cid-only-message` | `flows_bugPeerCidOnly` | ATTR-TOKEN off: attribution by peer CID alone (the peer-CID-only rule), under K-A2′ (V-22, P-37) | `NoCrossLaunchMsg` |
| `owner-flows-hazard-no-claim-recheck-held` | `flows_bugNoClaimRecheck` | ATTR-LIVE-CLAIM off (comparison with the lease token): the CID state is checked at accept only, and the claim step compares with the lease token, Retiring included | `NoCrossLaunchSessionHeld` |
| `owner-flows-hazard-no-accepted-watch` | `flows_bugNoAcceptedWatch` | DG-5-E off, on the instance GuestHalfEnds is checked on: an accepted, unattached V_g is not watched for terminal readiness (the host's close of V_h and an error on it go unnoticed until the flow is attached) | `GuestHalfEnds` |
| `owner-flows-hazard-beacon-no-live-recheck` | `flows_bugBeaconNoLiveRecheck` | ATTR-LIVE-CLAIM off for the beacon: READY compared with the lease's token, without the live check | `BeaconCurrentLaunch` |
| `owner-flows-hazard-beacon-peer-cid-only` | `flows_bugBeaconPeerCidOnly` | ATTR-TOKEN off for the beacon: READY attributed by peer CID alone | `BeaconNoCrossLaunchReady` |
| `owner-flows-hazard-dial-no-admission` | `flows_bugDialNoAdmission` | INB-1: the owner opens a dial without the Active ∧ live session ∧ not-quiesced check | `NoAdmitWhenClosed` |
| `owner-flows-hazard-owner-deadline-shorter` | `flows_bugOwnerDeadlineShorter` | INB-1: GUEST_DIAL_PAIRING_DEADLINE not above GUEST_INBOUND_CONNECT_TIMEOUT: the owner may abandon a dial the guest can still answer | `DialDeadlineAfterGuestReply` |
| `owner-flows-hazard-refusal-registers` | `flows_bugRefusalRegisters` | INB-1 off: a typed admission refusal registers (and closes) the dial slot instead of changing nothing | `TypedRefusalChangesNothing` |
| `owner-flows-hazard-activate-no-guard` | `flows_bugActivateNoGuard` | G-V3 off: activate marks the CID Active without a live control session or while quiesced | `ActivateOnlyWithLiveSession` |
| `fwd-pair-hazard-consume-on-short-send` | `pair_bugConsumeOnShortSend` | EVMOD-ADOPT (consume only what the target accepted) off: a short send consumes the whole peek (on the instance with more units than the pipeline holds: a short send needs a full target) | `UnitsConserved` |
| `fwd-pair-hazard-eof-as-fin-peer-loss` | `pair_bugEofAsFin` | R2-END off: every vsock end is forwarded as a FIN | `NoEofOnPeerLoss` |
| `fwd-pair-hazard-release-keeps-pairs` | `pair_bugReleaseKeepsPairs` | EVMOD-CTL off: the context's release leaves its pairs forwarding | `ReleasedContextHoldsNoPair` |
| `fwd-pair-hazard-lossy-end-record` | `pair_bugLossyEndRecord` | EVMOD-REC off: an end record may be dropped | `OneEndRecordPerPair` |
| `fwd-pair-hazard-consume-partial-frame` | `pair_dhazConsumePartialFrame` | EVMOD-DGRAM off: a datagram is consumed when only part of its frame was accepted; the next datagram's items then join the cut frame (three datagrams in one direction) | `DatagramsOnceInOrder` |
| `fwd-pair-hazard-shutdown-on-callback-delivery` | `pair_liveShutdownOnCallback` | ADR-0183 off, on the liveness instance: the target is shut in the callback, so the units queued behind the early FIN are stranded | `EveryUnitDelivered` |
| `fwd-pair-hazard-check-then-hook` | `pair_bugCheckThenHook` | EVMOD-PAIR off: attach reads the half's state and installs its callbacks in a later step (the transition between them is recorded by neither; the unclassified_eof alarm then aborts the pair) | `AttachTimeEndClassifiedOnce` |
| `fwd-pair-hazard-holder-trusts-read` | `pair_bugHolderTrustsRead` | EVMOD-STREAM-END off: the GuestStream reports the holder's read result (0 → clean), not the watch's | `HolderOutcomeFromWatch` |
| `fwd-pair-hazard-ended-attach-graceful-close` | `pair_bugEndedAttachGracefulClose` | EVMOD-ATTACH-END off: on HalfEnded the owner closes the surviving TCP half without SO_LINGER{1,0} (a FIN, not a reset) | `EndedHalfNeverEndsClean` |
| `fwd-pair-hazard-watch-abort-shuts-vh` | `pair_bugWatchAbortShutsVh` | WATCH-ABORT off (the rejected EVMOD-WATCH-SHUT): the owner's abort of a watch shuts V_h | `WatchAbortLeavesHolderSocket` |
| `fwd-pair-hazard-drop-without-abort` | `pair_bugDropWithoutAbort` | EVMOD-STREAM-ABORT (T-6) off: the holder's drop only closes V_h; the forwarder's reference keeps the socket open (P-70 § 4.4) | `NoLiveWatchAfterDrop` |
| `cid-lease-hazard-release-before-vmm-outlives` | `lease_bugReleaseBeforeVmm` | release-last off: the lease is released while the VMM still holds the device | `LeaseOutlivesVmm` |
| `cid-lease-hazard-assign-before-reclaim` | `lease_bugAssignBeforeReclaim` | NO-SURVIVE off: assign runs before boot reclamation killed the previous process's VMMs | `NoLaunchClash` |
| `cid-lease-hazard-reset-reboots` | `lease_bugResetReboots` | RESET-EXIT off: a guest reset reboots the guest inside the same VMM | `GuestResetEndsVmm` |
| `cid-lease-hazard-crash-forgets-stop` | `lease_bugCrashForgetsStop` | T-5 ruling off: a serve crash forgets the stop intent, so a stopped workload is placed again after the restart | `StoppedWorkloadNotPlaced` |
| `cid-lease-hazard-retry-off-by-one` | `lease_bugRetryOffByOne` | RESTART-SSOT off (retryOffByOne): the full pre-fix `restart_status_for_alloc` as the restart decision — restart iff attempts + 1 < CEILING, with no exemptions | `ExhaustsOnlyAtCeiling` |
| `cid-lease-hazard-reclaim-not-exempt` | `lease_bugReclaimNotExempt` | RESTART-SSOT off (reclaimNotExempt): only the platform-reclaim exemption removed | `ExhaustsOnlyOnACrashClassFailure` |
| `cid-lease-hazard-crash-loses-placed-generation-run` | `lease_conf_bugCrashLosesPlacedGeneration` | pending GH #325 off (crashLosesPlacedGeneration): its named run is the teeth for `lease_conf_325` | `run RedeployLeasedAtServeCrashIsPlacedAgain` |
| `cid-lease-hazard-assign-split` | `lease_bugAssignSplit` | ADR-0156 off: the next-fit assign walk is not one step under the pool lock (choose, then take later without a recheck) | `CidUnique` |
| `cid-lease-hazard-refusal-keeps-predecessor-run` | `lease_bugRefusalKeepsPredecessor` | item 7.4 off (refusalKeepsPredecessor): a pool refusal leaves the predecessor held until the next launch. Its run is a `lease_ok` run (three offsets): the teeth for `lease_conf_three` | `run PoolExhaustedRestartCleansThePredecessorThenPlaces` |
| `cid-lease-hazard-terminal-before-release` | `lease_bugTerminalBeforeRelease` | item 7.5 off (terminalBeforeRelease): FinalizeFailed writes the terminal before it releases the lease | `FailedHoldsNoLease` |
| `cid-lease-hazard-tick-during-predecessor-teardown` | `lease_bugTickDuringPredecessorTeardown` | item 7.3 step boundary off (tickDuringPredecessorTeardown): a tick runs while the workload's dispatch is held at a predecessor teardown; Assign from `ended` then overwrites the retiring predecessor lease | `NoUntrackedLease` |
| `quiescence-hazard-bool-latch` | `quiesce_bugBoolLatch` | QHOLD: a boolean latch — the first recovery's restore reopens while the second is still repairing | `NoReopenBeforeRepair` |
| `quiescence-hazard-leak-ignored` | `quiesce_live_bugLeakIgnored` | ADR-0124 (D-UP-3): the supervisor does not consult the leaked-hold count — a leaked hold keeps forwarding quiesced for good | `LeakedHoldEndsInFailStop` |
| `quiescence-hazard-boot-continues` | `quiesce_bugBootContinues` | ADR-0184: boot continues with serve running after a failed write of the shared table | `ServeRunsOnlyWithTable` |
| `quiescence-hazard-quiesce-skips-flows` | `quiesce_bugQuiesceSkipsFlows` | QHOLD: the first hold latches admission but does not abort the open flows | `QuiescedHoldsNoFlow` |
| `quiescence-hazard-drop-decrements-leaked` | `quiesce_bugDropDecrements` | QHOLD (D-UP-3): a dropped hold releases the count (the guard's Drop decrements, besides counting the leak) | `LeakedHoldKeepsForwardingQuiesced` |
| `quiescence-hazard-foreign-decrements` | `quiesce_bugForeignDecrements` | D-UP-4 (PD-1): a restore with a hold another ledger issued is redeemed as the owner's own (decrements the count; admission reopens at zero) | `ForeignRestoreChangesNothing` |
| `quiescence-hazard-foreign-ignored` | `quiesce_live_bugForeignIgnored` | D-UP-4 (PD-1): the supervisor does not fail-stop on ForeignHold — serve keeps running with a composition defect | `ForeignHoldEndsInFailStop` |
| `udp-slots-hazard-claim-split` | `slots_bugClaimSplit` | slot claim is two steps (check, then take) | `SlotHeldOnce` |
| `udp-slots-hazard-keep-frames` | `slots_bugKeepFrames` | P-35: release returns the slot with datagrams still in its socket's queue | `ReleasedSlotCarriesNoData` |
| `udp-slots-hazard-late-paired-by-slot-route` | `slots_bugLatePairedBySlot` | a late Paired is matched by slot, not by flow id | `RouteCurrent` |
| `udp-slots-hazard-ignore-late-paired` | `slots_bugIgnoreLatePaired` | SLOT-ABORT off: a late Paired is dropped silently | `LatePairedAborted` |
| `udp-slots-hazard-keep-frames-on-end` | `slots_bugKeepFramesOnEnd` | U-2 / EVMOD-DGRAM off: Refused / Abort / the pairing deadline leaves the datagrams in the slot socket's queue (it is not discarded at an end of association) | `NoReassociationWithoutNewDatagram` |
| `udp-slots-hazard-no-audit-fairness` | `slots_liveTeeth` | liveness teeth: no fairness on the audit — a lost sock_release report strands the slot | `NoAuditFairness` |
| `flow-quota-hazard-count-active-only-bound` | `quota_bugCountActiveOnly` | ADR-0147 off: the quota counts active flows only — a half-closed or ended flow keeps its place but not its count, so one withholding guest holds more than its own quota | `QuotaBound` |
| `flow-quota-hazard-node-wide-quota` | `quota_bugNodeWideQuota` | the quota is per allocation: with one quota for the node, a withholding guest exhausts the node | `QuotaRefusalOnlyAtOwnQuota` |
| `cid-lease-hazard-reclaim-never-completes` | `lease_bugReclaimNeverCompletes` | NO-SURVIVE off (liveness): boot reclamation kills the orphans but never records that it completed, so a waiting workload is never placed | `EventuallyPlaced` |
| `owner-flows-hazard-beacon-driver-stop-keeps-claim` | `flows_bugDriverStopKeepsClaim` | ADR-0157, D-UP-9 off: the launch-end step (driver.stop, or the VM driver's handling of the VMM exit) closes the beacon session but does not release the CID's beacon claim — the next launch's READY is held out forever | `ReadyReleased` |
| `fwd-safe-hazard-no-sweep-recheck` | `safe_bugNoSweepRecheck` | EVMOD-SWEEP off (F-S1): a run acts on an end it read with no recorded class without the vsock_table_lock re-check | `NoUnclassifiedEnd` |
| `fwd-safe-hazard-remove-wait-after-free-queued` | `safe_bugRemoveWaitAfterFreeQueued` | the write-space entry removed by the free work after its cancel_work_sync, i.e. after the free work was queued (violates NoTouchAfterFree) | `NoTouchAfterFree` |
| `fwd-safe-hazard-rcu-assign-null` | `safe_bugRcuAssignNull` | rule 2 off (alternative (a)): rcu_assign_pointer(…, NULL) + rcu_dereference | `NoOverrideCallsItself` |
| `fwd-safe-hazard-no-grace-period` | `safe_bugNoGracePeriod` | rule 3 off: no grace period before the free | `NoTouchAfterFree` |
| `fwd-safe-hazard-ctl-queue-outside-rcu` | `safe_bugCtlQueueOutsideRcu` | rule 4 off (alternative (d)): the id looked up under RCU, the work queued after rcu_read_unlock | `NoTouchAfterFree` |
| `fwd-safe-hazard-watch-end-no-id-check` | `safe_bugWatchEndNoIdCheck` | rule 4 off (watch): the override queues the record without checking the id under the lock | `OneEndRecord` |
| `fwd-safe-hazard-no-write-space` | `safe_liveNoWriteSpace` | § Forwarding, Trigger off: the target's write-space wait-queue entry is not added | `StalledPairResumes` |
| `fwd-safe-hazard-no-data-ready` | `safe_liveNoDataReady` | § Forwarding, Trigger off: the source's sk_data_ready is not overridden | `EveryUnitDelivered` |
| `fwd-safe-hazard-no-error-report` | `safe_liveNoErrorReport` | § Forwarding, Trigger off: the source's sk_error_report is not overridden | `SocketEndYieldsRecord` |
| `fwd-safe-hazard-late-clear-pending` | `safe_liveLateClearPending` | the workqueue's clear-before-run replaced by a flag cleared at return (a lost wakeup) | `StalledPairResumes` |
| `fwd-safe-hazard-ctl-abort-no-queue` | `safe_liveCtlAbortNoQueue` | rule 4 / EVMOD-REC off: OVD_FWD_ABORT does not queue the work item | `RequestedAbortEnds` |
| `fwd-safe-hazard-end-split-no-requeue` | `safe_liveEndSplitNoRequeue` | § Order of a pair's end off: the end stops after the unhook and does not requeue for its record and free | `EndProducesRecord` |

Properties with no hazard of their own: `StalledSenderIsBlocked` (a writer of a bounded stream always ends unable to
write, whatever the forwarder does; the bound it stands for is `QueuesWithinBounds`) and `CleanHalfCloseCompletes`.
The ordering invariants over several units or datagrams (`DeliveredIsInOrderPrefix`, `DatagramsOnceInOrder`,
`FrameNeverSplitAcrossDatagrams`) are vacuous on the one-unit and one-datagram instances; they are checked exhaustively
on the middle-sized instance `pair_mid` (2 units and 2 datagrams in one direction, one fault) and along the named runs
on `pair_conf`. `DatagramsOnceInOrder` has the hazard `fwd-pair-hazard-consume-partial-frame`
(`pair_dhazConsumePartialFrame`, EVMOD-DGRAM off), violated on the three-datagram hazard instance
`pair_dhaz`; on `pair_mid` it holds.
