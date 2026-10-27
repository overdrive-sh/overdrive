// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — control sessions, flow admission, launch attribution
 * (session, flows and the VM driver's beacon) and the host-opened flow opened by a dial through the owner.
 * How a forwarded flow ends (half-close, abort) is module `fwd_pair` (the in-kernel forwarder, EVMOD).
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 *
 * Source of truth: docs/feature/netns-density-295/feature-delta.md § "vsock Attachment Replacement
 * DESIGN": § Control session (D18a / ADR-0166), § Wire contracts → Attribution (ATTR-TOKEN /
 * ADR-0177), § Per-kind total orders (TcpConnect steps 3-6, TcpAccept steps 1-6), § Owner and
 * provisioner (provision / activate / teardown / quiesce / restore, M-7 per-allocation order), §
 * Composition (the session claim released by teardown, the beacon claim by driver.stop, both before
 * pool.release), § Driven port — VM driver beacon (ADR-0157), INB-1 (ADR-0173: a host platform client
 * reaches a VM port through a dial that the owner opens), ADR-0160 (stop aborts flows); and the rulings:
 *   U-1  Flow admission is one step with flow-table registration; a step-6 continuation stops for a
 *        closed flow (TcpConnect host step 6; TcpAccept guest step 5).
 *   U-5  A `Paired`, `Refused` or other control message for an unknown or closed flow id is discarded
 *        and counted, with no reply, by either side. U-5 rests on assumption K-A4 below.
 *   ATTR-TOKEN Every guest-opened host connection is attributed by peer CID AND the launch token its
 *        first record carries. Flow: check-and-register (U-1), read the request, compare the token; a
 *        mismatch closes, unregisters and sends nothing. pool.release follows teardown and driver.stop.
 *   ATTR-LIVE-CLAIM A CID has a LIVE launch token only while its launch is live — owner: the CID
 *        Provisioned or Active; VM driver: from driver.start until driver.stop. A session claim (owner,
 *        port 1243) or a beacon claim (driver, port 1234) — a `ClaimSet` session claim, not a kernel CID
 *        claim — is taken only in one per-CID serialized step after the connection's first record has
 *        been read, which requires together that the record's token equals the CID's live launch token
 *        and that the claim is free. Teardown (-> Retiring) and driver.stop end the live token in the
 *        step that closes the launch's session and releases its claim; neither touches connections whose
 *        first record is unread (each is closed at its read).
 *   INB-1 (dial) The dialer (leg-S or the probe runner) calls the owner. The owner resolves the address
 *        to an Active VM lease and checks the port is declared (else NotServing), refuses while quiesced
 *        (Quiesced) and when the CID's control session is not live as the owner observes it (NoSession), in
 *        that order; then it connects the vsock, and a connect failure is Unreachable. The owner does not
 *        observe vmUp: with the VM gone the dial is Unreachable while the session's loss is unobserved
 *        (the session is `dead`) and NoSession once it is observed. Each is a typed result with no state change.
 *        Otherwise it opens V_h with peer = gateway:0 and the live token; the guest accepts, connects
 *        the application within GUEST_INBOUND_CONNECT_TIMEOUT and replies Paired / Refused / TimedOut.
 *        The owner's GUEST_DIAL_PAIRING_DEADLINE is strictly greater: the owner abandons a dial only
 *        after the guest replied or timed out (or can no longer reply); a live VM whose guest never
 *        replies ends the dial at the deadline (PairingDeadline). The host side of a dial is
 *        handed to the dialer and is never forwarded: the owner WATCHES it (EVMOD-PAIR), and the dialer's end
 *        class comes from the watch's end record (module fwd_pair).
 *   INB-1 (in-flight end, r1 H-1 / L-9) A dial the owner ends while it is in flight reports its cause with
 *        the admission variant of that cause: quiescence -> Quiesced, teardown -> NotServing, observed
 *        control-session loss -> NoSession. A VMM exit resets the dial's host vsock through the device-release
 *        orphan sweep (sk_err = ECONNRESET, drivers/vhost/vsock.c:721-751, 753-768 @ v7.0; K-C4): the dial
 *        returns Unreachable { ECONNRESET }. A vsock I/O error before pairing (the host's write fails, or the
 *        guest reports an error on V_g by Abort) returns Unreachable { errno } of that error. Dropping the dial
 *        future (cancellation) ends the dial exactly as the pairing deadline does (V_h closed, Abort on a
 *        live session, a late reply discarded); no result is returned. Capacity (the flow quota) is module
 *        flow_quota, not this one.
 *   G-V3 (activate) The only Provisioned -> Active edge requires a live control session and no quiescence;
 *        otherwise it returns ControlSessionPending / QuiescenceLatched (retry), and on a Retiring CID it
 *        is refused with no state change (named LeaseRetiring in the G-V3 boundary table).
 *   DG-5-E  The guest watches an accepted, unattached V_g: the host's close of V_h ends the flow with no
 *        message, an error aborts it with Abort; either leaves it Closed.
 *   EVMOD-PAIR (PROPOSED 2026-10-16) Attach happens at pairing and only after the request was read in full (H-1:
 *        an attached socket's bytes are payload): the host TcpConnect attaches (V_h, D) once D's connect
 *        completed, in the step that sends Paired; the guest TcpAccept attaches (V_g, C) once C's connect
 *        completed, in the step that sends Paired. There is no install, arm or parking step.
 *   DG-5-F  Guest session loss aborts every flow of the lost session in the one step that observes it,
 *        before any reconnect.
 *   D-UP-8  The guest owner holds its control connection as its session (`gSess`, guest-side state, not derived
 *        from the host's): set when the guest opens its session, cleared when it observes the session's loss
 *        (GuestSessionLossObserved) and at VmStop (the guest owner dies with the VM; the VM's next start runs a
 *        fresh guest owner). The guest re-opens its session only after it observed the loss (gSess = none); a
 *        second first-connect while its own is still unresolved on the host (L-3, R5-18's race) does not replace
 *        the session the guest holds. QPROJ-5: `session` ⇔ gSess ≠ none.
 *   D-UP-9  The VM driver handles a launch's VMM exit (a guest reset included) in one per-CID step that ends the
 *        launch's liveness and releases its beacon claim, the step shape of driver.stop (ATTR-LIVE-CLAIM): VmStop
 *        removes the launch from `drvLive` and frees `bclaim`.
 *
 * Abstraction:
 *   - CFG selects the instance: "core" (sessions, guest-opened flows, dials, the allocation lifecycle),
 *     "core2" (core with two dial slots, for the all-flows session-loss run), "one" (core with one guest-opened flow: the
 *     instance of the GuestHalfEnds liveness check), "sess" (core without
 *     guest-opened flows and dials: the instance of the SessionBecomesLive liveness check, whose property
 *     mentions sessions only) or "beacon" (VM start / exit, driver.stop and the beacon; ADR-0157). The parts
 *     share nothing else, so each is checked on its own instance; the state of the parts an instance switches
 *     off stays constant. UI-V-BEACON-ACTIONS: the beacon instance enables only the actions with a VM-driver
 *     driving port or stimulus — Provision (a new assignment: the launch token of the next VmStart), VmStart,
 *     VmStop, DriverStop and the beacon actions; activation, teardown, release, quiescence and every owner
 *     session, flow and dial action are disabled there.
 *   - D-UP-1 (conformance step record): `conf_step` records the action that produced the state, as a variant of
 *     `owner_flows_conf::ConfStep` whose tag is the action's name and whose value is the record of its parameters
 *     by parameter name (`init` records `InitWith`; the wrappers HostReadReq, HostFlowEnds, GuestPairOrRefuse record
 *     HostReadReqN, HostFlowEndsMsg, GuestPairOrRefuseOk). quint-connect reads the action and its picks from it
 *     (`Config::nondet`). Model-only.
 *   - ONE guest CID. Successive allocations (leases) that lease the CID are distinguished by `gen`; one
 *     launch per lease, so the launch token is the lease's `gen` (abstractly). The owner's live token is
 *     `gen` while alloc is Provisioned or Active; the driver's live token is `vmGen` while `drvLive`.
 *     Each connection carries the launch that opened it (`gen` field); its first record (ControlOpen /
 *     READY / request) is buffered at connect.
 *   - CONNS: guest control-session connections to host port 1243; BCONNS: guest beacon connections to
 *     host port 1234; FLOW_IDS: guest-opened flow connections (check-and-register, read the 32-byte
 *     request in userspace, compare the token, THEN connect and attach). HFLOWS: dial slots (host `hf`, guest `gf`).
 *     The host opens V_h only to the CID's current launch (vmGen == gen), and a V_g dies with its VM, so
 *     the guest's own token check on a TcpAccept request never fails in this abstraction.
 *   - Per-CID serialization (M-7): provision, activate, teardown, quiesce, restore, driver.stop, session
 *     / beacon claim and loss are each ONE atomic action. The read of a first record is its own step and
 *     interleaves with them.
 *   - `quiesced` is the node admission latch as flows see it (the counted hold: module `quiescence`).
 *   - Time is abstract: deadlines are nondeterministic failure actions. A live guest writes its first
 *     record right after connecting, so a read with nothing buffered fails only once the connection's VM
 *     is gone. Each deadline action fires its own deadline only; the order of simultaneously due
 *     deadlines is the driver's clock and is not modelled.
 *   - Not modelled: flow quotas and the dial's Capacity outcome (module flow_quota); the stale-entry
 *     4-tuple refusal of a new connect. A guest whose dial handler stalls (GuestNeverReplies) is an
 *     environment fault; its own half is out of scope.
 *
 * ASSUMPTIONS (kernel behaviour, not modelled mechanisms; discharged in README.md):
 *   K-A1  Peer-CID attribution is authentic (vhost stamps the device CID) ............ V-10 (P-17)
 *   K-A2′ (measured) At VMM death, the host connections of that VM that are already accepted fail at
 *         their next I/O; a connection still in a host accept queue stays queued and is accepted later
 *         — after the launch's teardown and the CID's next lease and launch included — with the CID as
 *         peer, delivering its buffered first record if it had one ................... V-22 (P-37)
 *   K-A3  A flow socket attached before its request is read forwards the request bytes to the target as
 *         payload (why H-1 exists) .................................................... P-67 / R5-9
 *   K-A4  The host's close of V_h of an aborted or session-lost dial reliably tears down the guest-side
 *         connection V_g — accepted, or still in the guest's accept queue. ... V-24 (P-38)
 *   K-A5S The end of a control connection on the host (refused at accept or read, or lost) reaches the guest
 *         owner, which observes it (GuestSessionLossObserved) as EOF / EPOLLRDHUP. OFF "K-A5S" drops it
 *         ........................................................................ findings-ka5.md (WORKS)
 *   K-C4  (cid_lease) At VMM exit the device's last close resets every host vsock to the guest
 *         (ECONNRESET), an in-flight dial's V_h included ................................ V-30 Q2c, Q2d
 *
 * OFF names the rules switched off (teeth), or a candidate alternative switched on; the design instance
 * switches none off.
 */
// D-UP-1: the variants of `conf_step` (a module of its own: a constructor would clash with the action of the same
// name, so the actions build their variant with the builtin `variant`)
module owner_flows_conf {
  type ConfStep =
    | InitWith({ o: str })
    | Provision | VmStart | VmStop | Activate | ActivatePending | ActivateQuiesced | ActivateRetiring
    | Teardown | Release | Quiesce | Restore
    | GuestOpenSession({ c: int }) | WorkloadOpenSession({ c: int }) | HostAcceptSession({ c: int }) | HostReadOpen({ c: int })
    | HostClaimSession({ c: int }) | HostSessionLoss({ c: int }) | HostDeadSessionLoss({ c: int })
    | GuestOpenFlow({ f: int }) | HostAcceptFlow({ f: int })
    | HostReadReqN({ f: int, n: int }) | HostStartConnect({ f: int })
    | HostConnectDone({ f: int }) | HostConnectDiscard({ f: int }) | HostFlowEndsMsg({ f: int, msg: bool })
    | DialOpen({ h: int }) | DialNotServing({ h: int, declared: bool }) | DialQuiesced({ h: int })
    | DialNoSession({ h: int }) | DialUnreachable({ h: int })
    | GuestAcceptV({ h: int }) | GuestPairOrRefuseOk({ h: int, ok: bool }) | GuestConnectTimesOut({ h: int })
    | HostRecvPaired({ h: int }) | HostRecvRefused({ h: int }) | HostRecvTimedOut({ h: int })
    | HostAbortAccept({ h: int }) | HostDialDeadline({ h: int }) | DialCancel({ h: int })
    | GuestNeverReplies({ h: int }) | GuestRecvAbort({ h: int }) | GuestSeesHostClose({ h: int })
    | GuestAcceptedError({ h: int }) | HostRecvAbort({ h: int }) | GuestSessionLossObserved({ c: int })
    | GuestQueueReset({ h: int })
    | GuestOpenBeacon({ b: int }) | HostAcceptBeacon({ b: int }) | HostReadReady({ b: int }) | DriverStop
}

module owner_flows {
  import owner_flows_conf.ConfStep from "./owner_flows"
  // ------------------------------------------------------------------ parameters
  const OFF: Set[str]
  const CFG: str            // "core" | "core2" | "one" | "sess" | "beacon"
  // D-UP-1: true only on a conformance main (`flows_conf`, `flows_beacon_conf`): `conf_step` records the action taken.
  // Every model-checking, liveness and hazard instance passes false and keeps `conf_step` at its initial value.
  const REC: bool

  val CORE: bool = CFG == "core" or CFG == "core2" or CFG == "one" or CFG == "sess"
  val BEACON: bool = CFG == "beacon"

  val CONNS: Set[int] = Set(1, 2)
  val BCONNS: Set[int] = Set(1, 2)
  val FLOW_IDS: Set[int] = if (CFG == "sess") Set() else if (CFG == "one") Set(1) else Set(1, 2)
  val MAXGEN: int = 2
  val HFLOWS: Set[int] = if (CFG == "core2") Set(1, 2) else if (CFG == "sess") Set() else Set(1)

  val BUG_SESSION_SPLIT_CLAIM: bool = OFF.contains("sessionSplitClaim")     // L-3 violated: "no live claim?" checked, then claimed later
  val BUG_ATTACH_BEFORE_READ: bool = OFF.contains("attachBeforeRead")       // H-1 violated: the acceptor connects and attaches V_h before the full request is read
  val BUG_NO_ABORT_ON_LOSS: bool = OFF.contains("noAbortOnLoss")            // control-session loss does not abort the CID's flows
  val BUG_LATE_PAIRED_RESURRECTS: bool = OFF.contains("latePairedResurrects") // U-5 off: the host acts on a Paired for a flow it closed
  val BUG_PEER_CID_ONLY: bool = OFF.contains("peerCidOnly")                 // ATTR-TOKEN off: attribution by peer CID alone
  val BUG_NO_CLAIM_RECHECK: bool = OFF.contains("noClaimRecheck")           // ATTR-LIVE-CLAIM off (comparison with the lease token): the claim
                                                                            // step compares with the LEASE token (Retiring included);
                                                                            // the CID state is checked at accept only
  val BUG_BEACON_NO_LIVE_RECHECK: bool = OFF.contains("beaconNoLiveRecheck") // ATTR-LIVE-CLAIM off for the beacon: READY compared with
                                                                            // the lease's token, without the live-launch check
  val BUG_BEACON_PEER_CID_ONLY: bool = OFF.contains("beaconPeerCidOnly")    // ATTR-TOKEN off for the beacon: READY attributed by peer CID
  val BUG_DRIVER_STOP_KEEPS_CLAIM: bool = OFF.contains("driverStopKeepsClaim") // ADR-0157 off: driver.stop closes the session but does not
                                                                            // release the CID's beacon claim
  val BUG_NO_ACCEPTED_WATCH: bool = OFF.contains("noAcceptedWatch")         // DG-5-E off: an accepted, unattached V_g is not watched
  val ENV_GUEST_MISSES_SESSION_LOSS: bool = OFF.contains("K-A5S")            // drop K-A5S: the guest never observes the end of its control connection
  val BUG_DIAL_NO_ADMISSION: bool = OFF.contains("dialNoAdmission")         // the owner opens a dial without the Active ∧ session ∧ not-quiesced check
  val BUG_REFUSAL_REGISTERS: bool = OFF.contains("refusalRegisters")        // a typed admission refusal registers (and closes) the dial slot
  val BUG_ACTIVATE_NO_GUARD: bool = OFF.contains("activateNoGuard")         // G-V3 off: activate marks the CID Active without a live session / while quiesced
  val BUG_OWNER_DEADLINE_SHORTER: bool = OFF.contains("ownerDeadlineShorter") // GUEST_DIAL_PAIRING_DEADLINE not above the guest timeout: the owner may abandon a dial the guest can still answer

  // ------------------------------------------------------------------ types
  // st: none | pending (host accept queue) | reading (accepted, CID check passed, first record not
  //     yet read) | checked (bug only) | live | dead (peer gone, loss not yet processed by the
  //     owner) | closed
  // gen: the launch that opened it. Its first record is buffered at connect (a live guest writes it
  // at once). A queued connection with nothing buffered fails its first read and is closed with no
  // effect, so it adds no behaviour and is not modelled.
  type Sess = { st: str, gen: int }
  // st: none | queued (accept queue) | accepted (registered, request not yet
  //     fully read and compared) | read (token matched: registration complete) |
  //     connecting | paired | closed
  // req: request bytes read in userspace: 0, 1 (partial), 2 (all of it)
  // inFlight: a non-blocking connect was issued and its completion has not been processed yet
  // A flow's request is buffered at connect (a live guest writes it at once). A queued flow whose
  // VM died keeps it (K-A2′); one with no buffered request would fail its first read and close with
  // no effect, which HostFlowEnds already covers.
  type Flow = { st: str, gen: int, req: int, inFlight: bool }
  // INB-1: the dialer's observation of its dials. res: none | pending | Ok | Refused | TimedOut | PairingDeadline | NotServing |
  // Quiesced | NoSession | Unreachable:ENODEV (the vsock connect failed: no state change) | Unreachable:ECONNRESET (the VMM
  // exited while the dial was in flight, or a payload-free guest Abort arrived before Paired) | Unreachable:io (another
  // vsock I/O error before pairing) | Cancelled (the dialer dropped the future; nothing is returned). NotServing /
  // Quiesced / NoSession are also the results of a dial the owner ended in
  // flight by teardown / quiescence / observed session loss (L-9). adm: slots whose current result is a typed admission
  // refusal (ghost). tout: TimedOut replies in flight. earlyDl: the owner abandoned a dial the guest could still answer
  // (ghost). lateReply: the guest replied to a dial the owner had already abandoned (ghost).
  type Dl = { res: int -> str, adm: Set[int], tout: Set[int], earlyDl: bool, lateReply: bool }

  // ------------------------------------------------------------------ state
  var alloc: str           // Absent | Provisioned | Active | Retiring   (owner CID table)
  var gen: int             // the lease currently holding the CID (its launch token)
  var vmGen: int           // the launch of the last VM started on the CID
  var vmUp: bool
  var quiesced: bool       // node admission latch
  var claim: int           // ClaimSet<GuestCid>: the connection holding the CID's session claim (0 = none)
  var sess: int -> Sess
  var flows: int -> Flow
  var badPaired: bool      // ghost: a Paired was emitted while its admission conditions did not hold
  var crossMsg: bool       // ghost: a control message was sent on the session for a flow of another launch
  var heldByOther: bool    // ghost: a session of one launch was refused because a connection of
                           //        another launch held the CID's claim (SessionHeld across launches)
  // host-opened TcpAccept flows
  // hf.st: none | req (V_h connected, request written, awaiting Paired / Refused) | paired | closed
  var hf: int -> { st: str, gen: int }
  // gf.st: none | acc (accepted, request read, unattached) | attached | closed | reset (K-A4: V_g torn down while still
  //        in the guest's accept queue)
  // gf.c: the guest's live control connection when it accepted V_g (0 = none)
  var gf: int -> { st: str, gen: int, c: int }
  var toHostPaired: Set[int]
  var toHostRefused: Set[int]
  var toGuestAbort: Set[int]
  var toHostAbort: Set[int]  // guest -> host Abort for a host-opened flow (DG-5-E: an error on an accepted, unattached V_g)
  var resurrected: bool    // ghost: a host flow left `closed`
  var abortDiscarded: Set[int] // ghost: Aborts the guest discarded because the flow was unknown to it
  var phantom: bool        // ghost: the guest connected the application for a flow the host had
                           //        already closed and whose Abort it discarded as unknown
  var dl: Dl
  // beacon (VM driver, port 1234; CFG "beacon")
  var drvLive: bool        // the driver's launch (vmGen) is live: started, not stopped
  var bclaim: int          // the CID's beacon claim (ClaimSet<GuestCid>): the connection holding it (0 = none)
  var bcn: int -> Sess     // beacon connections: none | pending | reading | live (claim held: READY released) | closed
  var bq: List[int]        // BFIFO1: pending beacon connection IDs in the one listener's FIFO accept order
  var readyCross: bool     // ghost: a READY of one launch was released while another launch is the CID's
  var bHeldByOther: bool   // ghost: a beacon connection was refused because one of another launch held the claim
  // activation (G-V3)
  var act: str             // the result of an activation attempt in the step that made it: none | Activated |
                           // ControlSessionPending | QuiescenceLatched | LeaseRetiring (every other step resets it to none)
  var badAct: bool         // ghost: the CID was marked Active without a live session, or while quiesced
  // D-UP-8: the control connection the guest owner holds as its session (0 = none; guest-side state)
  var gSess: int
  var conf_step: ConfStep  // D-UP-1: the action that produced this state (model-only)

  // ------------------------------------------------------------------ helpers
  // registered in the CID's flow table (U-1), including a registration awaiting its token check
  def registered(st: str): bool = st.in(Set("accepted", "read", "connecting", "paired"))
  // registration completed (the request's token matched)
  def attributed(st: str): bool = st.in(Set("read", "connecting", "paired"))
  def sessUp(st: str): bool = st.in(Set("live", "dead"))
  val liveCount: int = CONNS.filter(c => sessUp(sess.get(c).st)).size()
  val sessLive: bool = liveCount > 0
  // the CID's state accepts a control session (Accept step: an early close only)
  val cidAcceptsSession: bool = alloc.in(Set("Provisioned", "Active"))
  // the CID's LIVE launch token (ATTR-LIVE-CLAIM): `gen` while the CID is Provisioned or Active
  def tokenMatches(g: int): bool = cidAcceptsSession and g == gen
  // the CID's lease token (the rejected comparison; noClaimRecheck)
  def leaseTokenMatches(g: int): bool = alloc != "Absent" and g == gen
  val admissible: bool = alloc == "Active" and sessLive and not(quiesced)
  // the launch of the session control messages travel on (the claim holder), or -1
  val msgSessGen: int = if (claim != 0 and sessUp(sess.get(claim).st)) sess.get(claim).gen else -1
  def crossFor(g: int): bool = msgSessGen != -1 and msgSessGen != g
  // the connection's VM is running (it can still write)
  def writerUp(g: int): bool = vmUp and vmGen == g
  def abortFlow(f: Flow): Flow = if (registered(f.st)) { ...f, st: "closed" } else f
  val abortAll: int -> Flow = flows.keys().mapBy(k => abortFlow(flows.get(k)))
  def hOpen(st: str): bool = st.in(Set("req", "paired"))
  val abortAllH: int -> { st: str, gen: int } = hf.keys().mapBy(h =>
    if (hOpen(hf.get(h).st)) { ...hf.get(h), st: "closed" } else hf.get(h))
  val openH: Set[int] = HFLOWS.filter(h => hOpen(hf.get(h).st))
  // L-9: the dialer's observation of the dials the owner ended in flight, before any reply, by cause `r`
  def dlEnd(r: str): Dl = { ...dl, res: HFLOWS.mapBy(h =>
    if (hOpen(hf.get(h).st) and dl.res.get(h) == "pending") r else dl.res.get(h)) }
  // ... one dial `h` ended in flight by an I/O error
  def dlEndOne(h: int, r: str): Dl =
    if (hOpen(hf.get(h).st) and dl.res.get(h) == "pending") resSet(h, r) else dl
  def resSet(h: int, r: str): Dl = { ...dl, res: dl.res.set(h, r) }
  def replySent(h: int): bool = dl.res.get(h) == "PairingDeadline"
  // a host-side abort before Paired sends Abort only while the session is live
  def abortMsgs(liveAfter: bool): Set[int] = if (liveAfter) toGuestAbort.union(openH) else Set()
  action keepH = all {
    hf' = hf, gf' = gf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused,
    toGuestAbort' = toGuestAbort, resurrected' = resurrected, abortDiscarded' = abortDiscarded, toHostAbort' = toHostAbort,
    phantom' = phantom, dl' = dl,
  }
  action keepGhostsH = all { resurrected' = resurrected, abortDiscarded' = abortDiscarded, phantom' = phantom, dl' = dl }
  action keepGhostsHNoDl = all { resurrected' = resurrected, abortDiscarded' = abortDiscarded, phantom' = phantom }
  action keepAttr = all { crossMsg' = crossMsg, heldByOther' = heldByOther }
  action keepB = all {
    drvLive' = drvLive, bclaim' = bclaim, bcn' = bcn, bq' = bq,
    readyCross' = readyCross, bHeldByOther' = bHeldByOther,
  }
  // not an activation attempt: no activation result in this step
  action keepAct = all { act' = "none", badAct' = badAct }
  // the beacon unchanged, no activation result
  action keepX = all { keepB, keepAct }

  // ------------------------------------------------------------------ init
  action init = {
    InitWith("g")
  }
  action InitWith(o: str): bool = all {
    alloc' = "Absent",
    gen' = 0,
    vmGen' = 0,
    vmUp' = false,
    quiesced' = false,
    claim' = 0,
    sess' = CONNS.mapBy(c => { st: "none", gen: 0 }),
    flows' = FLOW_IDS.mapBy(f => { st: "none", gen: 0, req: 0, inFlight: false }),
    badPaired' = false, crossMsg' = false, heldByOther' = false,
    hf' = HFLOWS.mapBy(h => { st: "none", gen: 0 }),
    gf' = HFLOWS.mapBy(h => { st: "none", gen: 0, c: 0 }),
    toHostPaired' = Set(), toHostRefused' = Set(), toGuestAbort' = Set(), toHostAbort' = Set(),
    resurrected' = false, abortDiscarded' = Set(), phantom' = false,
    dl' = { res: HFLOWS.mapBy(h => "none"), adm: Set(), tout: Set(), earlyDl: false, lateReply: false },
    drvLive' = false, bclaim' = 0, bcn' = BCONNS.mapBy(b => { st: "none", gen: 0 }), bq' = List(),
    readyCross' = false, bHeldByOther' = false,
    act' = "none", badAct' = false,
    gSess' = 0,
    conf_step' = variant("InitWith", { o: if (REC) o else "g" }),
  }

  // ------------------------------------------------------------------ allocation lifecycle (owner, per-CID order)
  // pool.assign + provision: a new lease (and its launch token) for the CID
  // UI-V-BEACON-ACTIONS: on the beacon instance a new assignment (the launch token of the next VmStart), once the
  // previous launch ended
  action Provision = all {
    conf_step' = if (REC) variant("Provision", ()) else conf_step, gSess' = gSess,
    gen < MAXGEN,
    if (BEACON) and { not(vmUp), not(drvLive), vmGen == gen } else alloc == "Absent",
    alloc' = "Provisioned", gen' = gen + 1,
    vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim, sess' = sess, flows' = flows,
    badPaired' = badPaired, keepAttr, keepH, keepX,
  }
  // driver.start: CH launched with the leased CID and the lease's launch token (once per lease); the
  // driver's launch becomes live (only the beacon instance tracks it)
  action VmStart = all {
    conf_step' = if (REC) variant("VmStart", ()) else conf_step, gSess' = gSess,
    alloc == "Provisioned", not(vmUp), vmGen < gen,
    vmUp' = true, vmGen' = gen, drvLive' = BEACON,
    alloc' = alloc, gen' = gen, quiesced' = quiesced, claim' = claim, sess' = sess, flows' = flows,
    badPaired' = badPaired, keepAttr, keepH,
    bclaim' = bclaim, bcn' = bcn, bq' = bq,
    readyCross' = readyCross, bHeldByOther' = bHeldByOther, keepAct,
  }
  // VMM exit (driver.stop's kill, or a crash). K-A2′: connections already accepted fail at their next
  // I/O (a live session becomes `dead`; an accepted session or beacon connection whose first record
  // is unread, and an accepted flow whose request is not fully read, fail their read and are closed,
  // with no other effect);
  // queued connections stay queued with their buffered first record. A live beacon session keeps its
  // claim until driver.stop. H-1 (r1): the device-release orphan sweep resets the host vsock of every dial
  // of this launch still awaiting its pairing reply (K-C4, ECONNRESET): the owner ends it and the dial returns
  // Unreachable { ECONNRESET }; no message (the guest is gone). A paired dial's stream is reset (fwd_pair).
  action VmStop = all {
    conf_step' = if (REC) variant("VmStop", ()) else conf_step,
    vmUp,
    vmUp' = false,
    sess' = sess.keys().mapBy(c =>
      val s = sess.get(c)
      if (s.gen == vmGen and s.st == "live") { ...s, st: "dead" }
      else if (s.gen == vmGen and s.st == "reading") { ...s, st: "closed" }
      else s),
    flows' = flows.keys().mapBy(f =>
      val x = flows.get(f)
      if (x.gen == vmGen and x.st == "accepted" and x.req < 2) { ...x, st: "closed" }
      else x),
    bcn' = bcn.keys().mapBy(b =>
      if (bcn.get(b).gen == vmGen and bcn.get(b).st.in(Set("reading", "live"))) { ...bcn.get(b), st: "closed" } else bcn.get(b)),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, quiesced' = quiesced, claim' = claim,
    badPaired' = badPaired, keepAttr,
    gf' = gf.keys().mapBy(h => if (gf.get(h).gen == vmGen and gf.get(h).st != "none") { ...gf.get(h), st: "closed" } else gf.get(h)),
    hf' = hf.keys().mapBy(h => if (hf.get(h).st == "req" and hf.get(h).gen == vmGen) { ...hf.get(h), st: "closed" } else hf.get(h)),
    dl' = { ...dl, res: HFLOWS.mapBy(h =>
      if (hf.get(h).st == "req" and hf.get(h).gen == vmGen and dl.res.get(h) == "pending") "Unreachable:ECONNRESET"
      else dl.res.get(h)) },
    toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort,
    keepGhostsHNoDl,
    // D-UP-8: the guest owner dies with the VM. D-UP-9: the VMM exit ends the launch's liveness and releases its
    // beacon claim in this one per-CID step (the step shape of driver.stop; driverStopKeepsClaim keeps it)
    gSess' = 0,
    drvLive' = false, bclaim' = if (BUG_DRIVER_STOP_KEEPS_CLAIM) bclaim else 0,
    bq' = bq, readyCross' = readyCross, bHeldByOther' = bHeldByOther, keepAct,
  }
  // activate (G-V3): the only Provisioned -> Active edge; requires a live session and no quiescence
  // (activateNoGuard: neither is checked)
  def canActivate: bool = all {
    CORE, alloc == "Provisioned", BUG_ACTIVATE_NO_GUARD or (sessLive and not(quiesced)) }
  action Activate = all {
    conf_step' = if (REC) variant("Activate", ()) else conf_step, gSess' = gSess,
    canActivate,   // UI-V-BEACON-ACTIONS: no VM-driver driving port
    alloc' = "Active", act' = "Activated", badAct' = (badAct or not(sessLive and not(quiesced))),
    gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim, sess' = sess,
    flows' = flows, badPaired' = badPaired, keepAttr, keepH, keepB,
  }
  // activate refused, no state change: quiescence has priority over a missing live control session;
  // both outcomes are retried under the EXEC claim.
  action keepActRefused = all {
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim, sess' = sess,
    flows' = flows, badPaired' = badPaired, keepAttr, keepH, keepB, badAct' = badAct,
  }
  action ActivatePending = all {
    conf_step' = if (REC) variant("ActivatePending", ()) else conf_step, gSess' = gSess,
    CORE,   // UI-V-BEACON-ACTIONS: no VM-driver driving port
    alloc == "Provisioned", not(quiesced), not(sessLive), act' = "ControlSessionPending", keepActRefused,
  }
  action ActivateQuiesced = all {
    conf_step' = if (REC) variant("ActivateQuiesced", ()) else conf_step, gSess' = gSess,
    CORE,   // UI-V-BEACON-ACTIONS: no VM-driver driving port
    alloc == "Provisioned", quiesced, act' = "QuiescenceLatched", keepActRefused,
  }
  // activate after teardown: the CID is Retiring; refused, no state change (G-V3 row 4: LeaseRetiring; the
  // provisioner contract names the same case Err(PostconditionMismatch { ForwardingActivate }))
  action ActivateRetiring = all {
    conf_step' = if (REC) variant("ActivateRetiring", ()) else conf_step, gSess' = gSess,
    CORE,   // UI-V-BEACON-ACTIONS: no VM-driver driving port
    alloc == "Retiring", act' = "LeaseRetiring", keepActRefused,
  }
  // teardown: Retiring — which ends the CID's live launch token — closes the control session and
  // releases its claim, in one step; aborts every flow of the CID (registrations
  // awaiting their token check included, U-1). It does not touch connections in `reading`
  // (ATTR-LIVE-CLAIM).
  action Teardown = all {
    conf_step' = if (REC) variant("Teardown", ()) else conf_step, gSess' = gSess,
    CORE,   // UI-V-BEACON-ACTIONS: no VM-driver driving port
    alloc.in(Set("Provisioned", "Active")),
    alloc' = "Retiring",
    sess' = sess.keys().mapBy(c =>
      val s = sess.get(c)
      if (s.st.in(Set("live", "dead", "checked")))
        { ...s, st: "closed" }
      else s),
    claim' = 0,
    flows' = abortAll,
    gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, badPaired' = badPaired, keepAttr,
    hf' = abortAllH, gf' = gf, toHostPaired' = Set(), toHostRefused' = Set(), toGuestAbort' = Set(), toHostAbort' = Set(),
    keepGhostsHNoDl, dl' = { ...dlEnd("NotServing"), tout: Set() }, keepX,
  }
  // pool.release after driver.stop and teardown (ADR-0133): the CID may be leased again
  action Release = all {
    conf_step' = if (REC) variant("Release", ()) else conf_step, gSess' = gSess,
    CORE,   // UI-V-BEACON-ACTIONS: no VM-driver driving port
    alloc == "Retiring", not(vmUp), not(drvLive),
    alloc' = "Absent",
    gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim, sess' = sess,
    flows' = flows, badPaired' = badPaired, keepAttr, keepH, keepX,
  }
  // quiesce_forwarding: latch, abort every live flow
  action Quiesce = all {
    conf_step' = if (REC) variant("Quiesce", ()) else conf_step, gSess' = gSess,
    not(quiesced),
    quiesced' = true, flows' = abortAll,
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, claim' = claim, sess' = sess,
    badPaired' = badPaired, keepAttr, hf' = abortAllH, gf' = gf, toHostPaired' = toHostPaired,
    toHostRefused' = toHostRefused, toGuestAbort' = abortMsgs(sessLive), keepGhostsHNoDl, dl' = dlEnd("Quiesced"), keepX, toHostAbort' = toHostAbort,
  }
  action Restore = all {
    conf_step' = if (REC) variant("Restore", ()) else conf_step, gSess' = gSess,
    quiesced,
    quiesced' = false,
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, claim' = claim, sess' = sess,
    flows' = flows, badPaired' = badPaired, keepAttr, keepH, keepX,
  }

  // ------------------------------------------------------------------ control session (D18a, ATTR-TOKEN, ATTR-LIVE-CLAIM)
  action keepSessRest = all {
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, flows' = flows,
    badPaired' = badPaired, crossMsg' = crossMsg, keepH, keepX,
  }
  // the guest connects to (2, 1243) and writes its ControlOpen (its own launch token), and reconnects after it
  // observed the end of the connection it held (refusal or loss); a racing first connect (R5-18, L-3) is
  // WorkloadOpenSession
  def connLive(c: int): bool = CONNS.exists(x => x == c and sess.get(x).st == "live")
  // DG-5-F: no flow of a lost session outlives the dispatch that observes the loss, so a new connection
  // is not opened while any flow of a lost session is still open (a flow accepted with no live session,
  // gf.c = 0, belongs to no session)
  val lostSessionFlowOpen: bool = HFLOWS.exists(h =>
    gf.get(h).st.in(Set("acc", "attached")) and gf.get(h).c != 0 and not(connLive(gf.get(h).c)))
  // D-UP-8: the guest owner opens its session only while it holds none (it observed the loss of the one it held),
  // on a connection slot that is free or ended; DG-5-F: and with no flow of a lost session open
  def canGuestOpenSession(c: int): bool = all {
    vmUp, sess.get(c).st.in(Set("none", "closed")), gSess == 0, not(lostSessionFlowOpen) }
  action GuestOpenSession(c: int): bool = all {
    conf_step' = if (REC) variant("GuestOpenSession", { c: c }) else conf_step,
    canGuestOpenSession(c),
    sess' = sess.set(c, { st: "pending", gen: vmGen }),
    gSess' = c,
    claim' = claim, heldByOther' = heldByOther, keepSessRest,
  }
  // L-3 (R5-18), D-UP-8: a racing first-connect to the control port that is not the guest owner's — a workload in the
  // guest opens an AF_VSOCK connection to (2, 1243) and writes a ControlOpen with the launch token — while the guest
  // owner's own first connect is still unresolved on the host. A host-side stimulus: the guest owner does not hold it
  // (`gSess` unchanged).
  action WorkloadOpenSession(c: int): bool = all {
    conf_step' = if (REC) variant("WorkloadOpenSession", { c: c }) else conf_step,
    vmUp, c != gSess, gSess != 0, sess.get(gSess).st.in(Set("pending", "reading", "checked")),
    // a connection slot never used (as in the committed model: a slot's id stands for one connection, and a flow the
    // guest attributed to an ended connection still names its slot)
    sess.get(c).st == "none",
    sess' = sess.set(c, { st: "pending", gen: vmGen }),
    gSess' = gSess, claim' = claim, heldByOther' = heldByOther, keepSessRest,
  }
  // Accept: an early close for a CID not Provisioned or Active; it admits nothing.
  action HostAcceptSession(c: int): bool = all {
    conf_step' = if (REC) variant("HostAcceptSession", { c: c }) else conf_step, gSess' = gSess,
    sess.get(c).st == "pending",
    if (not(cidAcceptsSession)) all {
      sess' = sess.set(c, { ...sess.get(c), st: "closed" }), claim' = claim, heldByOther' = heldByOther,
    } else all {
      sess' = sess.set(c, { ...sess.get(c), st: "reading" }),
      claim' = claim, heldByOther' = heldByOther,
    },
    keepSessRest,
  }
  // Read the ControlOpen and, in the same per-CID step (ATTR-LIVE-CLAIM), compare its token with the
  // CID's LIVE launch token and take the claim (ClaimSet): a mismatch — another launch, or a CID no
  // longer Provisioned or Active — closes the connection with no claim (StaleLaunch); a held claim
  // closes it (SessionHeld).
  action HostReadOpen(c: int): bool = all {
    conf_step' = if (REC) variant("HostReadOpen", { c: c }) else conf_step, gSess' = gSess,
    sess.get(c).st == "reading",
    val s = sess.get(c)
    val tokOk = BUG_PEER_CID_ONLY or
                (if (BUG_NO_CLAIM_RECHECK) leaseTokenMatches(s.gen)
                 else tokenMatches(s.gen))
    if (not(tokOk)) all {
      sess' = sess.set(c, { ...s, st: "closed" }), claim' = claim, heldByOther' = heldByOther,
      keepSessRest,
    } else if (claim != 0) all {
      sess' = sess.set(c, { ...s, st: "closed" }), claim' = claim,
      heldByOther' = (heldByOther or sess.get(claim).gen != s.gen),
      keepSessRest,
    } else if (BUG_SESSION_SPLIT_CLAIM) all {
      sess' = sess.set(c, { ...s, st: "checked" }), claim' = claim, heldByOther' = heldByOther,
      keepSessRest,
    } else if (writerUp(s.gen)) all {
      sess' = sess.set(c, { ...s, st: "live" }), claim' = c, heldByOther' = heldByOther,
      keepSessRest,
    } else sessionLoss(c),
  }
  // BUG_SESSION_SPLIT_CLAIM: ... then claim, without re-checking.
  action HostClaimSession(c: int): bool = all {
    conf_step' = if (REC) variant("HostClaimSession", { c: c }) else conf_step, gSess' = gSess,
    BUG_SESSION_SPLIT_CLAIM, sess.get(c).st == "checked",
    sess' = sess.set(c, { ...sess.get(c), st: if (writerUp(sess.get(c).gen)) "live" else "dead" }), claim' = c,
    heldByOther' = heldByOther, keepSessRest,
  }
  // Loss (peer EOF / reset, or a malformed message): release the claim, abort every flow of the VM.
  action sessionLoss(c: int): bool = all {
    sess' = sess.set(c, { ...sess.get(c), st: "closed" }),
    claim' = if (claim == c) 0 else claim,
    flows' = if (BUG_NO_ABORT_ON_LOSS) flows else abortAll,
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced,
    badPaired' = badPaired, keepAttr, hf' = abortAllH, gf' = gf, toHostPaired' = Set(), toHostRefused' = Set(), toGuestAbort' = Set(), toHostAbort' = Set(),
    keepGhostsHNoDl, dl' = { ...dlEnd("NoSession"), tout: Set() }, keepX,
  }
  action HostSessionLoss(c: int): bool = all {
    conf_step' = if (REC) variant("HostSessionLoss", { c: c }) else conf_step, gSess' = gSess,
    sessUp(sess.get(c).st), sessionLoss(c),
  }
  // the owner processes the loss of a session whose VM is gone (fair in SessionBecomesLive)
  action HostDeadSessionLoss(c: int): bool = all {
    conf_step' = if (REC) variant("HostDeadSessionLoss", { c: c }) else conf_step, gSess' = gSess,
    sess.get(c).st == "dead", sessionLoss(c),
  }

  // ------------------------------------------------------------------ guest-opened flows (TcpConnect / datagram legs)
  action keepFlowRest = all {
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, heldByOther' = heldByOther, keepH, keepX,
  }
  // the guest connects and writes its request (with its launch token)
  action GuestOpenFlow(f: int): bool = all {
    conf_step' = if (REC) variant("GuestOpenFlow", { f: f }) else conf_step, gSess' = gSess,
    vmUp, flows.get(f).st == "none",
    flows' = flows.set(f, { st: "queued", gen: vmGen, req: 0, inFlight: false }),
    badPaired' = badPaired, crossMsg' = crossMsg, keepFlowRest,
  }
  // step 3: accept V_h; check Active + live session + not quiesced (+ quota); register in the
  // CID's flow table — one owner step (U-1). A Quiesced refusal remains outside admitted state,
  // reads the already-buffered complete request, emits Refused { Quiesced } and closes in this
  // same formal action; other unavailable-boundary cases close without a message.
  action HostAcceptFlow(f: int): bool = all {
    conf_step' = if (REC) variant("HostAcceptFlow", { f: f }) else conf_step, gSess' = gSess,
    flows.get(f).st == "queued",
    flows' = flows.set(f, { ...flows.get(f), st: if (admissible) "accepted" else "closed" }),
    badPaired' = badPaired, crossMsg' = crossMsg, keepFlowRest,
  }
  // step 3: read the request in userspace (in one or two pieces); once all of it is read, compare
  // its launch token with the CID's live launch token: a mismatch closes V_h and removes the
  // registration, with no control message (StaleLaunch); a match completes the registration.
  action HostReadReq(f: int): bool = {
    nondet n = Set(1, 2).oneOf()
    HostReadReqN(f, n)
  }
  action HostReadReqN(f: int, n: int): bool = all {
    conf_step' = if (REC) variant("HostReadReqN", { f: f, n: n }) else conf_step, gSess' = gSess,
    flows.get(f).st == "accepted", flows.get(f).req < 2,
    val x = flows.get(f)
    val r = if (x.req + n > 2) 2 else x.req + n
    val done = r == 2
    val tokOk = BUG_PEER_CID_ONLY or tokenMatches(x.gen)
    flows' = flows.set(f, { ...x, req: r,
                            st: if (not(done)) "accepted" else if (tokOk) "read" else "closed" }),
    badPaired' = badPaired, crossMsg' = crossMsg, keepFlowRest,
  }
  // steps 4-5: only once the request was read in full (H-1): policy, single resolution, issue a NON-BLOCKING
  // connect of D (attachBeforeRead: with the request still partly unread in V_h's queue)
  action HostStartConnect(f: int): bool = all {
    conf_step' = if (REC) variant("HostStartConnect", { f: f }) else conf_step, gSess' = gSess,
    flows.get(f).st == "read" or (BUG_ATTACH_BEFORE_READ and flows.get(f).st == "accepted"),
    flows' = flows.set(f, { ...flows.get(f), st: "connecting", inFlight: true }),
    badPaired' = badPaired, crossMsg' = crossMsg, keepFlowRest,
  }
  // step 6: the connect completed: attach (V_h, D) to the forwarder (EVMOD-PAIR), send Paired on the session.
  action HostConnectDone(f: int): bool = all {
    conf_step' = if (REC) variant("HostConnectDone", { f: f }) else conf_step, gSess' = gSess,
    flows.get(f).inFlight,
    flows.get(f).st == "connecting",
    flows' = flows.set(f, { ...flows.get(f), st: "paired", inFlight: false }),
    badPaired' = (badPaired or not(admissible and flows.get(f).gen == gen)),
    crossMsg' = (crossMsg or crossFor(flows.get(f).gen)),
    keepFlowRest,
  }
  // the in-flight completion of an aborted flow is discarded
  action HostConnectDiscard(f: int): bool = all {
    conf_step' = if (REC) variant("HostConnectDiscard", { f: f }) else conf_step, gSess' = gSess,
    flows.get(f).inFlight, flows.get(f).st == "closed",
    flows' = flows.set(f, { ...flows.get(f), inFlight: false }),
    badPaired' = badPaired, crossMsg' = crossMsg, keepFlowRest,
  }
  // refusals, request / pairing deadlines, connect failure, peer EOF/reset: the flow closes. A flow
  // whose request was read and matched may be answered (Refused / Abort on the session); one not yet
  // attributed is closed with no message (its flow id is unknown or another launch's).
  action HostFlowEnds(f: int): bool = {
    nondet msg = Set(true, false).oneOf()
    HostFlowEndsMsg(f, msg)
  }
  action HostFlowEndsMsg(f: int, msg: bool): bool = all {
    conf_step' = if (REC) variant("HostFlowEndsMsg", { f: f, msg: msg }) else conf_step, gSess' = gSess,
    registered(flows.get(f).st),
    val sends = msg and attributed(flows.get(f).st)
    all {
      flows' = flows.set(f, { ...flows.get(f), st: "closed" }),
      crossMsg' = (crossMsg or (sends and crossFor(flows.get(f).gen))),
    },
    badPaired' = badPaired, keepFlowRest,
  }

  // ------------------------------------------------------------------ host-opened flows (INB-1: a dial through the owner)
  action keepCore = all {
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, flows' = flows, badPaired' = badPaired, heldByOther' = heldByOther, keepX,
  }
  action keepCoreMsg = all { keepCore, crossMsg' = crossMsg }
  // V_g is in the guest's accept queue: the host connected V_h to this VM, the guest has not accepted
  def queuedV(h: int): bool = and {
    gf.get(h).st == "none", hf.get(h).st != "none", hf.get(h).gen == vmGen, vmUp }
  def guestHalf(h: int): bool = gf.get(h).st.in(Set("acc", "attached")) or queuedV(h)
  val liveConn: int = CONNS.fold(0, (acc, c) => if (sess.get(c).st == "live" and sess.get(c).gen == vmGen) c else acc)
  // the guest can still answer the dial: its VM and a live session exist and it has not decided
  def canStillReply(h: int): bool = and {
    vmUp, hf.get(h).gen == vmGen, sessLive, gf.get(h).st.in(Set("none", "acc")) }
  // steps 1-2 (INB-1): a platform client dialed an address that is an Active VM lease and a declared port, the
  // node is not quiesced and the VM is reachable: the owner connects V_h to (CID, 1242) and writes the request
  // with the lease token. The host side of the dial is handed to the dialer; the owner watches it (it is never
  // forwarded).
  def canDialOpen(h: int): bool = all {
    hf.get(h).st == "none", vmUp, vmGen == gen, alloc == "Active",
    or { BUG_DIAL_NO_ADMISSION, admissible } }
  action DialOpen(h: int): bool = all {
    conf_step' = if (REC) variant("DialOpen", { h: h }) else conf_step, gSess' = gSess,
    canDialOpen(h),
    hf' = hf.set(h, { st: "req", gen: gen }),
    dl' = { ...resSet(h, "pending"), adm: dl.adm.exclude(Set(h)) },
    gf' = gf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort,
    keepGhostsHNoDl, keepCoreMsg,
  }
  // typed results with no state change, in the owner's order of decision (each guard holds only when the earlier
  // checks passed): the address is not an Active VM lease or the port is not declared (NotServing); the node is
  // quiesced (Quiesced); the CID's control session is not live as the owner observes it (NoSession); the vsock
  // connect failed because the VM is gone (Unreachable). The owner does not observe vmUp; it observes the session
  // (sessLive counts a `dead` session, whose loss it has not processed yet).
  action DialNotServing(h: int, declared: bool): bool = all {
    conf_step' = if (REC) variant("DialNotServing", { h: h, declared: declared }) else conf_step, gSess' = gSess,
    hf.get(h).st == "none", alloc != "Active" or not(declared),
    refuse(h, "NotServing"),
  }
  action DialQuiesced(h: int): bool = all {
    conf_step' = if (REC) variant("DialQuiesced", { h: h }) else conf_step, gSess' = gSess,
    hf.get(h).st == "none", alloc == "Active", quiesced,
    refuse(h, "Quiesced"),
  }
  // INB-1 (architect resolution, 2026-10-13 order of decision): the allocation is Active and the node is not
  // quiesced, but the CID's control session is not live as the owner observes it: the typed result NoSession,
  // immediate, no state change, whatever vmUp (a VM gone with its session loss observed is NoSession)
  def canDialNoSession(h: int): bool = all {
    hf.get(h).st == "none", alloc == "Active", not(quiesced), not(sessLive) }
  action DialNoSession(h: int): bool = all {
    conf_step' = if (REC) variant("DialNoSession", { h: h }) else conf_step, gSess' = gSess,
    canDialNoSession(h),
    refuse(h, "NoSession"),
  }
  // the owner believes the session live (its loss is not observed yet) but the VM / device is gone: the vsock
  // connect fails (ENODEV), Unreachable, no state change
  def canDialUnreachable(h: int): bool = all {
    hf.get(h).st == "none", alloc == "Active", not(quiesced), sessLive, not(vmUp) }
  action DialUnreachable(h: int): bool = all {
    conf_step' = if (REC) variant("DialUnreachable", { h: h }) else conf_step, gSess' = gSess,
    canDialUnreachable(h),
    refuse(h, "Unreachable:ENODEV"),
  }
  // a typed admission refusal: the result, nothing else (refusalRegisters: the slot is registered and closed)
  action refuse(h: int, r: str): bool = all {
    dl' = { ...resSet(h, r), adm: dl.adm.union(Set(h)) },
    hf' = if (BUG_REFUSAL_REGISTERS) hf.set(h, { st: "closed", gen: gen }) else hf,
    keepH0,
  }
  action keepH0 = all {
    gf' = gf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    toHostAbort' = toHostAbort, resurrected' = resurrected, abortDiscarded' = abortDiscarded, phantom' = phantom,
    keepCoreMsg,
  }
  // step 3: the guest accepts V_g and reads the request (its own token); V_g stays unattached, held by the owner.
  action GuestAcceptV(h: int): bool = all {
    conf_step' = if (REC) variant("GuestAcceptV", { h: h }) else conf_step, gSess' = gSess,
    hf.get(h).st != "none", hf.get(h).gen == vmGen, vmUp, gf.get(h).st == "none",
    gf' = gf.set(h, { st: "acc", gen: vmGen, c: liveConn }),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort,
    keepGhostsH, keepCoreMsg,
  }
  // steps 4-5: connect C to the application (within GUEST_INBOUND_CONNECT_TIMEOUT); refused → Refused(errno);
  // otherwise (the flow is not Closed — U-1) attach (V_g, C) to the forwarder and send Paired (EVMOD-PAIR). Messages
  // need the live session.
  action GuestPairOrRefuse(h: int): bool = {
    nondet ok = Set(true, false).oneOf()
    GuestPairOrRefuseOk(h, ok)
  }
  action GuestPairOrRefuseOk(h: int, ok: bool): bool = all {
    conf_step' = if (REC) variant("GuestPairOrRefuseOk", { h: h, ok: ok }) else conf_step, gSess' = gSess,
    gf.get(h).st == "acc", vmUp, connLive(gf.get(h).c),
    all {
      gf' = gf.set(h, { ...gf.get(h), st: if (ok) "attached" else "closed" }),
      toHostPaired' = if (ok) toHostPaired.union(Set(h)) else toHostPaired,
      toHostRefused' = if (ok) toHostRefused else toHostRefused.union(Set(h)),
      phantom' = (phantom or (ok and hf.get(h).st == "closed" and abortDiscarded.contains(h))),
      dl' = { ...dl, lateReply: dl.lateReply or replySent(h) },
    },
    hf' = hf, toGuestAbort' = toGuestAbort, resurrected' = resurrected, abortDiscarded' = abortDiscarded, toHostAbort' = toHostAbort,
    keepCoreMsg,
  }
  // the guest's own connect to the application did not finish within GUEST_INBOUND_CONNECT_TIMEOUT: TimedOut
  action GuestConnectTimesOut(h: int): bool = all {
    conf_step' = if (REC) variant("GuestConnectTimesOut", { h: h }) else conf_step, gSess' = gSess,
    gf.get(h).st == "acc", vmUp, connLive(gf.get(h).c),
    gf' = gf.set(h, { ...gf.get(h), st: "closed" }),
    dl' = { ...dl, tout: dl.tout.union(Set(h)), lateReply: dl.lateReply or replySent(h) },
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort,
    keepGhostsHNoDl, keepCoreMsg,
  }
  // host step 6: Paired for an open flow → the dial returns Ok(GuestStream); for a closed / unknown flow → discarded (U-5)
  action HostRecvPaired(h: int): bool = all {
    conf_step' = if (REC) variant("HostRecvPaired", { h: h }) else conf_step, gSess' = gSess,
    toHostPaired.contains(h),
    toHostPaired' = toHostPaired.exclude(Set(h)),
    if (hf.get(h).st == "req") all {
      hf' = hf.set(h, { ...hf.get(h), st: "paired" }), resurrected' = resurrected,
      badPaired' = (badPaired or not(admissible and hf.get(h).gen == gen)),
      dl' = resSet(h, "Ok"),
    } else if (BUG_LATE_PAIRED_RESURRECTS and hf.get(h).st == "closed") all {
      hf' = hf.set(h, { ...hf.get(h), st: "paired" }), resurrected' = true, badPaired' = badPaired, dl' = dl,
    } else all {
      hf' = hf, resurrected' = resurrected, badPaired' = badPaired, dl' = dl,
    },
    gf' = gf, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort,
    abortDiscarded' = abortDiscarded, phantom' = phantom,
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, flows' = flows, crossMsg' = crossMsg, heldByOther' = heldByOther, keepX,
  }
  action HostRecvRefused(h: int): bool = all {
    conf_step' = if (REC) variant("HostRecvRefused", { h: h }) else conf_step, gSess' = gSess,
    toHostRefused.contains(h),
    toHostRefused' = toHostRefused.exclude(Set(h)),
    hf' = if (hf.get(h).st == "req") hf.set(h, { ...hf.get(h), st: "closed" }) else hf,
    dl' = if (hf.get(h).st == "req") resSet(h, "Refused") else dl,
    gf' = gf, toHostPaired' = toHostPaired, toGuestAbort' = toGuestAbort, keepGhostsHNoDl, keepCoreMsg, toHostAbort' = toHostAbort,
  }
  action HostRecvTimedOut(h: int): bool = all {
    conf_step' = if (REC) variant("HostRecvTimedOut", { h: h }) else conf_step, gSess' = gSess,
    dl.tout.contains(h),
    hf' = if (hf.get(h).st == "req") hf.set(h, { ...hf.get(h), st: "closed" }) else hf,
    dl' = if (hf.get(h).st == "req") { ...resSet(h, "TimedOut"), tout: dl.tout.exclude(Set(h)) } else { ...dl, tout: dl.tout.exclude(Set(h)) },
    gf' = gf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort,
    keepGhostsHNoDl, keepCoreMsg,
  }
  // the host closes an open host-opened flow (peer error, a failed vsock write): V_h closed with reset; Abort sent
  // while the session is live
  action HostAbortAccept(h: int): bool = all {
    conf_step' = if (REC) variant("HostAbortAccept", { h: h }) else conf_step, gSess' = gSess,
    hOpen(hf.get(h).st),
    hf' = hf.set(h, { ...hf.get(h), st: "closed" }),
    dl' = dlEndOne(h, "Unreachable:io"),
    toGuestAbort' = if (sessLive) toGuestAbort.union(Set(h)) else toGuestAbort, toHostAbort' = toHostAbort,
    crossMsg' = (crossMsg or (sessLive and crossFor(hf.get(h).gen))),
    gf' = gf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, keepGhostsHNoDl, keepCore,
  }
  // GUEST_DIAL_PAIRING_DEADLINE (strictly greater than GUEST_INBOUND_CONNECT_TIMEOUT): the owner abandons a dial
  // only once the guest has replied or timed out, or can no longer reply (VM gone, session lost). ownerDeadlineShorter
  // lets it fire while the guest can still answer.
  def canHostDialDeadline(h: int): bool =
    hf.get(h).st == "req" and (BUG_OWNER_DEADLINE_SHORTER or not(canStillReply(h)))
  action HostDialDeadline(h: int): bool = all {
    conf_step' = if (REC) variant("HostDialDeadline", { h: h }) else conf_step, gSess' = gSess,
    canHostDialDeadline(h),
    hf' = hf.set(h, { ...hf.get(h), st: "closed" }),
    dl' = { ...resSet(h, "PairingDeadline"), earlyDl: dl.earlyDl or canStillReply(h) },
    toGuestAbort' = if (sessLive) toGuestAbort.union(Set(h)) else toGuestAbort, toHostAbort' = toHostAbort,
    crossMsg' = (crossMsg or (sessLive and crossFor(hf.get(h).gen))),
    gf' = gf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, keepGhostsHNoDl, keepCore,
  }
  // cancellation (r1 M-4 / L-9): the dialer drops the dial future before it completes; the owner ends the dial
  // exactly as at the pairing deadline (V_h closed, Abort on a live session, a late reply discarded); no result
  action DialCancel(h: int): bool = all {
    conf_step' = if (REC) variant("DialCancel", { h: h }) else conf_step, gSess' = gSess,
    hf.get(h).st == "req",
    hf' = hf.set(h, { ...hf.get(h), st: "closed" }),
    dl' = resSet(h, "Cancelled"),
    toGuestAbort' = if (sessLive) toGuestAbort.union(Set(h)) else toGuestAbort, toHostAbort' = toHostAbort,
    crossMsg' = (crossMsg or (sessLive and crossFor(hf.get(h).gen))),
    gf' = gf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, keepGhostsHNoDl, keepCore,
  }
  // environment fault: the guest's dial handler on a live VM stalls and never replies (no Paired, Refused or
  // TimedOut): the guest can no longer reply, and the owner's pairing deadline ends the dial
  action GuestNeverReplies(h: int): bool = all {
    conf_step' = if (REC) variant("GuestNeverReplies", { h: h }) else conf_step, gSess' = gSess,
    hf.get(h).st == "req", hf.get(h).gen == vmGen, vmUp, gf.get(h).st.in(Set("none", "acc")),
    gf' = gf.set(h, { ...gf.get(h), st: "silent", gen: vmGen }),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort,
    keepGhostsH, keepCoreMsg,
  }
  // the guest receives Abort: closes a known flow; an unknown one is discarded (U-5)
  action GuestRecvAbort(h: int): bool = all {
    conf_step' = if (REC) variant("GuestRecvAbort", { h: h }) else conf_step, gSess' = gSess,
    toGuestAbort.contains(h),
    toGuestAbort' = toGuestAbort.exclude(Set(h)), toHostAbort' = toHostAbort,
    val st = gf.get(h).st
    all {
      gf' = if (st.in(Set("acc", "attached"))) gf.set(h, { ...gf.get(h), st: "closed" })
            else gf,
      abortDiscarded' = if (st == "none") abortDiscarded.union(Set(h)) else abortDiscarded,
    },
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, dl' = dl,
    resurrected' = resurrected, phantom' = phantom, keepCoreMsg,
  }
  // the guest observes the host's close of V_h (EOF / POLLRDHUP, TCPACCEPT-END) and ends its side (K-A4).
  // DG-5-E: the guest owner watches V_g from its accept, so this holds while gf(h) is `acc` (accepted, unattached:
  // the flow ends with NO message; no C is created, or a connecting C is closed) as well as once `attached` (the
  // forwarder's end record).
  // The flow is then Closed: a later pairing step (GuestPairOrRefuse) finds it Closed (U-1).
  action GuestSeesHostClose(h: int): bool = all {
    conf_step' = if (REC) variant("GuestSeesHostClose", { h: h }) else conf_step, gSess' = gSess,
    hf.get(h).st == "closed",
    gf.get(h).st == "attached" or (gf.get(h).st == "acc" and not(BUG_NO_ACCEPTED_WATCH)),
    gf' = gf.set(h, { ...gf.get(h), st: "closed" }),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort,
    keepGhostsH, keepCoreMsg,
  }
  // DG-5-E: an error or reset on the accepted, unattached V_g (`Hangup { Error }`): the flow ends, Closed,
  // and the guest sends Abort while the flow's session is live (a later Paired / Connected finds it Closed:
  // U-1, U-5)
  action GuestAcceptedError(h: int): bool = all {
    conf_step' = if (REC) variant("GuestAcceptedError", { h: h }) else conf_step, gSess' = gSess,
    not(BUG_NO_ACCEPTED_WATCH), gf.get(h).st == "acc",
    gf' = gf.set(h, { ...gf.get(h), st: "closed" }),
    toHostAbort' = if (connLive(gf.get(h).c)) toHostAbort.union(Set(h)) else toHostAbort,
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    keepGhostsH, keepCoreMsg,
  }
  // the host receives the guest's Abort: an open flow is aborted (no Abort echoed: the peer is gone); for
  // a closed flow it is discarded (U-5)
  action HostRecvAbort(h: int): bool = all {
    conf_step' = if (REC) variant("HostRecvAbort", { h: h }) else conf_step, gSess' = gSess,
    toHostAbort.contains(h),
    toHostAbort' = toHostAbort.exclude(Set(h)),
    hf' = if (hOpen(hf.get(h).st)) hf.set(h, { ...hf.get(h), st: "closed" }) else hf,
    dl' = dlEndOne(h, "Unreachable:ECONNRESET"),
    gf' = gf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    keepGhostsHNoDl, keepCoreMsg,
  }
  // DG-5-F: the guest observes the loss of control session `c` and, in this ONE step, aborts every flow of
  // it (before any reconnect: GuestOpenSession is disabled while one is open)
  // D-UP-8: also when the session the guest holds ended on the host (refused or lost: `closed`), with no flow of it;
  // the guest then holds no session
  def ofSession(h: int, c: int): bool = gf.get(h).st.in(Set("acc", "attached")) and gf.get(h).c == c
  action GuestSessionLossObserved(c: int): bool = all {
    conf_step' = if (REC) variant("GuestSessionLossObserved", { c: c }) else conf_step,
    not(ENV_GUEST_MISSES_SESSION_LOSS), CONNS.contains(c), not(connLive(c)),
    HFLOWS.exists(h => ofSession(h, c)) or (gSess == c and sess.get(c).st == "closed"),
    gSess' = if (gSess == c) 0 else gSess,
    gf' = gf.keys().mapBy(h => if (ofSession(h, c)) { ...gf.get(h), st: "closed" } else gf.get(h)),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort,
    keepGhostsH, keepCoreMsg,
  }
  // K-A4: the host's close of V_h ends a V_g still in the guest's accept queue
  action GuestQueueReset(h: int): bool = all {
    conf_step' = if (REC) variant("GuestQueueReset", { h: h }) else conf_step, gSess' = gSess,
    hf.get(h).st == "closed", queuedV(h),
    gf' = gf.set(h, { st: "reset", gen: vmGen, c: 0 }),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort,
    keepGhostsH, keepCoreMsg,
  }
  action guestCleanup = {
    nondet h = HFLOWS.oneOf()
    nondet c = CONNS.oneOf()
    any { GuestRecvAbort(h), GuestSeesHostClose(h), GuestSessionLossObserved(c), GuestQueueReset(h) }
  }

  // ------------------------------------------------------------------ beacon (VM driver, port 1234; ADR-0157, ATTR-TOKEN, ATTR-LIVE-CLAIM)
  action keepAllButB = all {
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, flows' = flows, badPaired' = badPaired, keepAttr, keepH, keepAct,
  }
  def guestHasBeacon: bool = BCONNS.exists(x =>
    bcn.get(x).gen == vmGen and bcn.get(x).st.in(Set("pending", "reading", "live")))
  // overdrive-init dials the beacon and (after the guest network) writes READY with its launch
  // token; it re-dials after a refusal within its attempt budget
  action GuestOpenBeacon(b: int): bool = all {
    conf_step' = if (REC) variant("GuestOpenBeacon", { b: b }) else conf_step, gSess' = gSess,
    BEACON, vmUp, bcn.get(b).st == "none" or (bcn.get(b).st == "closed" and not(guestHasBeacon)),
    bcn' = bcn.set(b, { st: "pending", gen: vmGen }),
    bq' = bq.append(b),
    drvLive' = drvLive, bclaim' = bclaim, readyCross' = readyCross, bHeldByOther' = bHeldByOther,
    keepAllButB,
  }
  // accept: an early close when no live launch holds the CID (NoLiveLaunch); otherwise held
  // unclaimed while its first line is read
  action HostAcceptBeacon(b: int): bool = all {
    conf_step' = if (REC) variant("HostAcceptBeacon", { b: b }) else conf_step, gSess' = gSess,
    BEACON, bcn.get(b).st == "pending", bq.length() > 0, bq.head() == b,
    bcn' = bcn.set(b, { ...bcn.get(b), st: if (drvLive) "reading" else "closed" }),
    bq' = bq.tail(),
    drvLive' = drvLive, bclaim' = bclaim, readyCross' = readyCross, bHeldByOther' = bHeldByOther,
    keepAllButB,
  }
  // the READY read and, in one per-CID step, the comparison with the live launch's token and the
  // beacon claim: a mismatch — another launch, or no live launch — closes it (StaleLaunch /
  // NoLiveLaunch); a held claim closes it (SessionHeld); otherwise it takes the claim and READY is
  // that launch's
  def beaconTokOk(g: int): bool =
    if (BUG_BEACON_PEER_CID_ONLY) true
    else if (BUG_BEACON_NO_LIVE_RECHECK) leaseTokenMatches(g)
    else drvLive and g == vmGen
  action HostReadReady(b: int): bool = all {
    conf_step' = if (REC) variant("HostReadReady", { b: b }) else conf_step, gSess' = gSess,
    BEACON, bcn.get(b).st == "reading",
    val x = bcn.get(b)
    if (not(beaconTokOk(x.gen))) all {
      bcn' = bcn.set(b, { ...x, st: "closed" }), bclaim' = bclaim, readyCross' = readyCross,
      bHeldByOther' = bHeldByOther,
    } else if (bclaim != 0) all {
      bcn' = bcn.set(b, { ...x, st: "closed" }), bclaim' = bclaim, readyCross' = readyCross,
      bHeldByOther' = (bHeldByOther or bcn.get(bclaim).gen != x.gen),
    } else all {
      bcn' = bcn.set(b, { ...x, st: "live" }), bclaim' = b,
      readyCross' = (readyCross or x.gen != vmGen), bHeldByOther' = bHeldByOther,
    },
    drvLive' = drvLive, bq' = bq, keepAllButB,
  }
  // driver.stop: ends the launch's liveness, closes its beacon session and releases the claim — one
  // step; it does not touch connections in `reading` (each is closed at its read). The VMM's exit
  // is VmStop, in either order.
  action DriverStop = all {
    conf_step' = if (REC) variant("DriverStop", ()) else conf_step, gSess' = gSess,
    BEACON, drvLive,
    drvLive' = false,
    bcn' = bcn.keys().mapBy(b => if (bcn.get(b).st == "live") { ...bcn.get(b), st: "closed" } else bcn.get(b)),
    bq' = bq,
    bclaim' = (if (BUG_DRIVER_STOP_KEEPS_CLAIM) bclaim else 0), readyCross' = readyCross, bHeldByOther' = bHeldByOther,
    keepAllButB,
  }

  // ------------------------------------------------------------------ next
  action lifeStep = any { Provision, VmStart, VmStop, Activate, ActivatePending, ActivateQuiesced, ActivateRetiring, Teardown, Release }
  action coreStep = {
    nondet c = CONNS.oneOf()
    nondet f = FLOW_IDS.oneOf()
    nondet h = HFLOWS.oneOf()
    nondet declared = Set(true, false).oneOf()
    any {
      DialOpen(h), DialNotServing(h, declared), DialQuiesced(h), DialNoSession(h), DialUnreachable(h), GuestConnectTimesOut(h), HostRecvTimedOut(h), HostDialDeadline(h),
      DialCancel(h), GuestNeverReplies(h),
      GuestAcceptV(h), GuestPairOrRefuse(h), HostRecvPaired(h), HostRecvRefused(h),
      HostAbortAccept(h), GuestRecvAbort(h), GuestSeesHostClose(h), GuestAcceptedError(h), HostRecvAbort(h),
      GuestSessionLossObserved(c), GuestQueueReset(h),
      Quiesce, Restore,
      GuestOpenSession(c), WorkloadOpenSession(c), HostAcceptSession(c), HostReadOpen(c), HostClaimSession(c),
      HostSessionLoss(c),
      GuestOpenFlow(f), HostAcceptFlow(f),
      HostReadReq(f), HostStartConnect(f), HostConnectDone(f),
      HostConnectDiscard(f), HostFlowEnds(f),
    }
  }
  action beaconStep = {
    nondet b = BCONNS.oneOf()
    any { GuestOpenBeacon(b), HostAcceptBeacon(b), HostReadReady(b), DriverStop }
  }
  // The sessions-only instance ("sess") has no flow ids and no dial slots: `coreStep`'s `nondet f` / `nondet h` range
  // over empty sets there, which disables every core action (TLC: \E over the empty set is FALSE), so the instance
  // takes its session steps from `sessStep` instead (the session, session-loss and quiescence actions of `coreStep`).
  action sessStep = {
    nondet c = CONNS.oneOf()
    any {
      GuestSessionLossObserved(c), Quiesce, Restore,
      GuestOpenSession(c), WorkloadOpenSession(c), HostAcceptSession(c), HostReadOpen(c), HostClaimSession(c),
      HostSessionLoss(c),
    }
  }
  action step = any {
    lifeStep,
    all { CORE, CFG != "sess", coreStep },
    all { CFG == "sess", sessStep },
    all { BEACON, beaconStep },
  }

  // ------------------------------------------------------------------ invariants
  // (1) A CID has at most one live control session.
  val OneLiveSession: bool = liveCount <= 1
  // (3, H-1) No flow is attached to the forwarder — nor its destination connected — before its full request has
  // been read: an attached socket's bytes are payload.
  val NoAttachBeforeRead: bool = FLOW_IDS.forall(f =>
    flows.get(f).st.in(Set("connecting", "paired")) implies flows.get(f).req == 2)
  // (4) No flow is admitted while quiesced, without a live control session, or for a non-Active CID;
  //     and no Paired is ever emitted under those conditions.
  val NoAdmitWhenClosed: bool = and {
    FLOW_IDS.forall(f => registered(flows.get(f).st) implies admissible),
    HFLOWS.forall(h => hOpen(hf.get(h).st) implies admissible),
    not(badPaired),
  }
  // (4+, ATTR-TOKEN, ATTR-LIVE-CLAIM) every live session and every attributed (registration-completed)
  // flow belongs to the allocation currently leasing the CID, and that allocation is not released.
  val CurrentGeneration: bool = and {
    CONNS.forall(c => sess.get(c).st.in(Set("live", "dead", "checked")) implies
      (sess.get(c).gen == gen and alloc != "Absent")),
    FLOW_IDS.forall(f => attributed(flows.get(f).st) implies
      (flows.get(f).gen == gen and alloc != "Absent")),
    HFLOWS.forall(h => hOpen(hf.get(h).st) implies (hf.get(h).gen == gen and alloc != "Absent")),
  }
  // (ATTR-TOKEN) no control message is ever sent on a session for a flow of another launch
  val NoCrossLaunchMsg: bool = not(crossMsg)
  // (ATTR-LIVE-CLAIM) a session is never refused SessionHeld because a connection of another launch
  // holds the CID's claim
  val NoCrossLaunchSessionHeld: bool = not(heldByOther)
  // U-5 / § Flow end: Closed is terminal — a late Paired never revives a host-opened flow
  val ClosedIsTerminal: bool = not(resurrected)
  // (DG-5-F) no flow of a lost control session outlives the dispatch that observes the loss: an open
  // guest TcpAccept flow's session connection is never replaced by a new connection while the flow is open
  val GuestFlowsDoNotOutliveSession: bool = HFLOWS.forall(h =>
    (gf.get(h).st.in(Set("acc", "attached")) and gf.get(h).c != 0) implies
      sess.get(gf.get(h).c).st.in(Set("live", "dead", "closed")))
  // (INB-1) the admission refusals NotServing, Quiesced, NoSession and Unreachable { ENODEV } are typed results with no state
  // change: the dial slot is untouched (the same variants reported for a dial ended in flight are not admission refusals)
  val TypedRefusalChangesNothing: bool = HFLOWS.forall(h =>
    dl.adm.contains(h) implies (hf.get(h).st == "none" and gf.get(h).st == "none"
                                and dl.res.get(h).in(Set("NotServing", "Quiesced", "NoSession", "Unreachable:ENODEV"))))
  // (G-V3) the CID is marked Active only with a live control session and no quiescence
  val ActivateOnlyWithLiveSession: bool = not(badAct)
  // (INB-1) GUEST_DIAL_PAIRING_DEADLINE exceeds GUEST_INBOUND_CONNECT_TIMEOUT: the owner never abandons a dial the guest can still answer
  val DialDeadlineAfterGuestReply: bool = not(dl.earlyDl)
  // (INB-1) no reply (Paired, Refused, TimedOut) is sent for a dial the owner already abandoned: no late Paired for an abandoned flow
  val NoPairedAfterDialEnded: bool = not(dl.lateReply)
  // (D-UP-8) the session the guest holds is a connection of the running launch: the guest owner dies with its VM
  val GuestSessionOfCurrentLaunch: bool = gSess != 0 implies (vmUp and sess.get(gSess).gen == vmGen)
  val Safety: bool = and {
    OneLiveSession, NoAttachBeforeRead, NoAdmitWhenClosed, CurrentGeneration, ClosedIsTerminal,
    NoCrossLaunchMsg, NoCrossLaunchSessionHeld, GuestFlowsDoNotOutliveSession,
    TypedRefusalChangesNothing, DialDeadlineAfterGuestReply, NoPairedAfterDialEnded, ActivateOnlyWithLiveSession,
    GuestSessionOfCurrentLaunch,
  }

  // ---- beacon (ATTR-TOKEN, ATTR-LIVE-CLAIM; CFG "beacon")
  // every held beacon claim belongs to the CID's live launch
  val BeaconCurrentLaunch: bool = and {
    bclaim != 0 implies (drvLive and bcn.get(bclaim).gen == vmGen),
    BCONNS.forall(b => bcn.get(b).st == "live" implies (bclaim == b)),
  }
  // no READY of one launch is released as another's
  val BeaconNoCrossLaunchReady: bool = not(readyCross)
  // no beacon connection is refused because a connection of another launch holds the claim
  val BeaconNoCrossLaunchHeld: bool = not(bHeldByOther)
  val BeaconQueueExact: bool = and {
    bq.indices().forall(i => BCONNS.contains(bq[i])),
    bq.indices().forall(i => bq.indices().forall(j => i == j or bq[i] != bq[j])),
    BCONNS.forall(b => (bq.indices().exists(i => bq[i] == b)) iff bcn.get(b).st == "pending"),
  }
  val BeaconSafety: bool = and {
    BeaconCurrentLaunch, BeaconNoCrossLaunchReady, BeaconNoCrossLaunchHeld, BeaconQueueExact,
  }


  // ------------------------------------------------------------------ liveness (TLC)
  // U-5 with K-A4: a guest-side TcpAccept connection whose host side is closed is eventually torn down
  temporal GuestHalfEnds =
    weakFair(guestCleanup, gf).implies(HFLOWS.forall(h =>
      always((hf.get(h).st == "closed" and guestHalf(h)).implies(eventually(not(guestHalf(h)))))))

  // (ATTR-TOKEN, D18a) a provisioned launch whose guest keeps reconnecting eventually holds a live
  // session — no connection of another launch keeps it out — under weak fairness of the host's
  // session steps and of the guest's connect / write, per connection
  val allVars = (alloc, gen, vmGen, vmUp, quiesced, claim, sess, flows, badPaired, crossMsg,
                 heldByOther, hf, gf, toHostPaired, toHostRefused, toGuestAbort, resurrected,
                 abortDiscarded, phantom, drvLive, bclaim, bcn, bq, readyCross, bHeldByOther, toHostAbort, dl, act, badAct,
                 gSess)
  val awaitingSession: bool = alloc == "Provisioned" and vmUp and vmGen == gen
  val launchLive: bool = CONNS.exists(c => sess.get(c).st == "live" and sess.get(c).gen == gen)
  temporal SessionBecomesLive =
    CONNS.forall(c => and {
      weakFair(GuestOpenSession(c), allVars), weakFair(HostAcceptSession(c), allVars),
      weakFair(HostReadOpen(c), allVars), weakFair(HostClaimSession(c), allVars),
      weakFair(HostDeadSessionLoss(c), allVars), weakFair(GuestSessionLossObserved(c), allVars),
    }).implies(always(awaitingSession.implies(eventually(launchLive or not(awaitingSession)))))

  // (ATTR-LIVE-CLAIM, beacon) a live launch whose guest keeps reconnecting gets its READY released,
  // under weak fairness of the driver's beacon steps and the guest's dial / write, per connection
  val awaitingReady: bool = drvLive and vmUp
  val readyLive: bool = BCONNS.exists(b => bcn.get(b).st == "live" and bcn.get(b).gen == vmGen)
  temporal ReadyReleased =
    BCONNS.forall(b => and {
      weakFair(GuestOpenBeacon(b), allVars), weakFair(HostAcceptBeacon(b), allVars),
      weakFair(HostReadReady(b), allVars),
    }).implies(always(awaitingReady.implies(eventually(readyLive or not(awaitingReady)))))


  // ------------------------------------------------------------------ named runs (critical scenarios)
  // Deterministic scenarios (`InitWith`, `HostReadReqN`, `HostFlowEndsMsg`, `GuestPairOrRefuseOk` fix the
  // nondeterministic choices), each ending in an `expect` of its outcome with the instance's safety
  // invariants checked along the way. Core runs belong to `flows_ok`, beacon runs to `flows_beacon`.
  // ---- core: control session (D18a), launch attribution (ATTR-TOKEN, ATTR-LIVE-CLAIM), flow admission (U-1)
  // happy path: lease, VM start, the guest opens the control session and the owner claims it on the live
  // launch token; activation; a guest-opened flow is admitted, its request read in full, then connected, attached
  // and paired
  run ControlSessionOpensAndCidActivates = InitWith("g")
    .then(Provision).expect(Safety and alloc == "Provisioned" and gen == 1)
    .then(VmStart).expect(Safety and vmUp and vmGen == 1)
    .then(GuestOpenSession(1)).then(HostAcceptSession(1)).expect(Safety and sess.get(1).st == "reading" and claim == 0)
    .then(HostReadOpen(1)).expect(Safety and sess.get(1).st == "live" and claim == 1 and sessLive)
    .then(Activate).expect(Safety and alloc == "Active" and admissible and act == "Activated")
  run GuestOpenedFlowIsAdmittedReadAttachedAndPaired = ControlSessionOpensAndCidActivates
    .then(GuestOpenFlow(1)).then(HostAcceptFlow(1)).expect(Safety and flows.get(1).st == "accepted" and flows.get(1).req == 0)
    .then(HostReadReqN(1, 2)).expect(Safety and flows.get(1).st == "read" and attributed(flows.get(1).st))
    .then(HostStartConnect(1)).expect(Safety and flows.get(1).st == "connecting" and flows.get(1).inFlight)
    .then(HostConnectDone(1)).expect(Safety and flows.get(1).st == "paired" and not(badPaired) and not(crossMsg))
  // H-1: the request arrives in two pieces; nothing is connected or attached before the whole request is read
  run RequestReadInPiecesAttachesOnlyAfterFullRead = ControlSessionOpensAndCidActivates
    .then(GuestOpenFlow(1)).then(HostAcceptFlow(1)).then(HostReadReqN(1, 1))
    .expect(Safety and flows.get(1).st == "accepted" and flows.get(1).req == 1)
    .then(HostReadReqN(1, 1)).expect(Safety and flows.get(1).st == "read" and flows.get(1).req == 2)
    .then(HostStartConnect(1)).then(HostConnectDone(1)).expect(Safety and flows.get(1).st == "paired" and flows.get(1).req == 2)
  // L-3 / R5-18: two first connects race; exactly one takes the claim, the other is closed (SessionHeld,
  // same launch); there is never more than one live session
  run RacingFirstConnectsLeaveOneLiveSession = InitWith("g")
    .then(Provision).then(VmStart).then(GuestOpenSession(1)).then(WorkloadOpenSession(2))
    .then(HostAcceptSession(1)).then(HostAcceptSession(2))
    .then(HostReadOpen(1)).expect(Safety and sess.get(1).st == "live" and claim == 1)
    .then(HostReadOpen(2)).expect(Safety and sess.get(2).st == "closed" and claim == 1 and liveCount == 1 and not(heldByOther))
  // session loss aborts every flow of the CID; a flow arriving while no session is live is closed with no
  // message (boundary: gate unavailable); the guest reconnects and the CID serves again
  run SessionLossAbortsFlowsAndReconnectRestores = GuestOpenedFlowIsAdmittedReadAttachedAndPaired
    .then(HostSessionLoss(1)).expect(Safety and sess.get(1).st == "closed" and claim == 0 and flows.get(1).st == "closed" and not(sessLive))
    .then(GuestOpenFlow(2)).then(HostAcceptFlow(2)).expect(Safety and flows.get(2).st == "closed" and not(crossMsg))
    .then(GuestSessionLossObserved(1)).expect(Safety and gSess == 0)
    .then(GuestOpenSession(1)).then(HostAcceptSession(1)).then(HostReadOpen(1))
    .expect(Safety and sess.get(1).st == "live" and claim == 1 and sessLive and alloc == "Active" and gSess == 1)
  // quiescence aborts live flows; admission is refused while quiesced (QuiescenceLatched); restore reopens
  run QuiesceAbortsFlowsAndRefusesAdmissionUntilRestore = GuestOpenedFlowIsAdmittedReadAttachedAndPaired
    .then(Quiesce).expect(Safety and quiesced and flows.get(1).st == "closed" and not(admissible))
    .then(GuestOpenFlow(2)).then(HostAcceptFlow(2)).expect(Safety and flows.get(2).st == "closed")
    .then(Restore).expect(Safety and not(quiesced) and admissible and alloc == "Active")
  // activation waits for quiescence to end: a Provisioned CID with a live session activates only after restore
  run ActivationWaitsForQuiescenceToEnd = InitWith("g")
    .then(Provision).then(VmStart).then(GuestOpenSession(1)).then(HostAcceptSession(1)).then(HostReadOpen(1))
    .then(Quiesce).expect(Safety and alloc == "Provisioned" and quiesced)
    .then(ActivateQuiesced).expect(Safety and act == "QuiescenceLatched" and alloc == "Provisioned")
    .then(Restore).then(Activate).expect(Safety and alloc == "Active" and admissible and act == "Activated")
  // G-V3 (L-7): activation is not possible while the control session is pending
  run ActivationIsImpossibleWhileTheSessionIsPending = InitWith("g")
    .then(Provision).then(VmStart)
    .expect(Safety and not(canActivate))
  // G-V3 (L-7, rows 2 and 5): activation with the control session pending returns ControlSessionPending with no state
  // change, also after the session was lost before activation; it activates once the session is live
  run ActivationWithSessionPendingReturnsPendingThenActivates = InitWith("g")
    .then(Provision).then(VmStart)
    .then(ActivatePending).expect(Safety and act == "ControlSessionPending" and alloc == "Provisioned" and not(sessLive))
    .then(GuestOpenSession(1)).then(HostAcceptSession(1)).then(HostReadOpen(1)).then(HostSessionLoss(1))
    .then(GuestSessionLossObserved(1))
    .then(ActivatePending).expect(Safety and act == "ControlSessionPending" and alloc == "Provisioned")
    .then(GuestOpenSession(1)).then(HostAcceptSession(1)).then(HostReadOpen(1))
    .then(Activate).expect(Safety and act == "Activated" and alloc == "Active" and not(badAct))
  // G-V3 (L-7, row 4): activate after teardown finds the CID Retiring: refused, no state change; nothing resurrects
  run ActivateAfterTeardownIsRefusedAndChangesNothing = InitWith("g")
    .then(Provision).then(VmStart).then(GuestOpenSession(1)).then(HostAcceptSession(1)).then(HostReadOpen(1))
    .then(Teardown).expect(Safety and alloc == "Retiring" and not(sessLive))
    .then(ActivateRetiring).expect(Safety and act == "LeaseRetiring" and alloc == "Retiring" and claim == 0)
  run ActivateAfterTeardownNeverActivates = InitWith("g")
    .then(Provision).then(VmStart).then(GuestOpenSession(1)).then(HostAcceptSession(1)).then(HostReadOpen(1))
    .then(Teardown).expect(Safety and not(canActivate))
  // U-1: the connect of a flow aborted while in flight completes later; the completion is discarded and
  // no Paired is sent
  run InFlightConnectOfAbortedFlowIsDiscarded = ControlSessionOpensAndCidActivates
    .then(GuestOpenFlow(1)).then(HostAcceptFlow(1)).then(HostReadReqN(1, 2)).then(HostStartConnect(1))
    .then(Quiesce).expect(Safety and flows.get(1).st == "closed" and flows.get(1).inFlight)
    .then(HostConnectDiscard(1)).expect(Safety and flows.get(1).st == "closed" and not(flows.get(1).inFlight) and not(badPaired))
  // an attributed flow ended by refusal / deadline / peer error is closed (a message may be sent on the
  // live session); the CID keeps serving
  run AttributedFlowEndsAndCidKeepsServing = GuestOpenedFlowIsAdmittedReadAttachedAndPaired
    .then(HostFlowEndsMsg(1, true)).expect(Safety and flows.get(1).st == "closed" and not(crossMsg) and admissible)
  // teardown (Retiring): closes the session and releases its claim, aborts every flow; the CID is released
  // after the VM stopped and leased again; the new lease's launch gets its own session
  run TeardownAbortsEverythingThenCidIsLeasedAgain = GuestOpenedFlowIsAdmittedReadAttachedAndPaired
    .then(Teardown).expect(Safety and alloc == "Retiring" and sess.get(1).st == "closed" and claim == 0 and flows.get(1).st == "closed")
    .then(GuestOpenFlow(2)).then(HostAcceptFlow(2)).expect(Safety and flows.get(2).st == "closed")
    .then(VmStop).then(Release).expect(Safety and alloc == "Absent")
    .then(Provision).then(VmStart).expect(Safety and alloc == "Provisioned" and gen == 2 and vmGen == 2)
    .then(GuestOpenSession(1)).then(HostAcceptSession(1)).then(HostReadOpen(1)).then(Activate)
    .expect(Safety and alloc == "Active" and claim == 1 and sess.get(1).gen == 2 and not(heldByOther))
  // K-A2': a control connection of a dead launch stays queued, is accepted after the CID's next lease and
  // launch exist, and is closed at its read on the stale token: no claim, nothing poisoned; the live
  // launch's own session then takes the claim
  run StaleLaunchSessionIsClosedAtReadAndDoesNotPoisonClaim = InitWith("g")
    .then(Provision).then(VmStart).then(GuestOpenSession(1)).then(VmStop).then(Teardown).then(Release)
    .then(Provision).then(VmStart).expect(Safety and gen == 2 and vmGen == 2 and sess.get(1).st == "pending" and sess.get(1).gen == 1)
    .then(HostAcceptSession(1)).expect(Safety and sess.get(1).st == "reading")
    .then(HostReadOpen(1)).expect(Safety and sess.get(1).st == "closed" and claim == 0 and not(heldByOther))
    .then(GuestOpenSession(2)).then(HostAcceptSession(2)).then(HostReadOpen(2))
    .expect(Safety and sess.get(2).st == "live" and claim == 2 and not(heldByOther))
  // ATTR-LIVE-CLAIM: a connection accepted while the CID was Provisioned is
  // read after teardown: the live token has ended, it is closed with no claim; the next launch is not
  // refused SessionHeld
  run TeardownBetweenAcceptAndReadClosesWithoutClaim = InitWith("g")
    .then(Provision).then(VmStart).then(GuestOpenSession(1)).then(HostAcceptSession(1))
    .then(Teardown).expect(Safety and alloc == "Retiring" and sess.get(1).st == "reading")
    .then(HostReadOpen(1)).expect(Safety and sess.get(1).st == "closed" and claim == 0 and not(heldByOther))
    .then(VmStop).then(Release).then(Provision).then(VmStart)
    .then(GuestOpenSession(2)).then(HostAcceptSession(2)).then(HostReadOpen(2))
    .expect(Safety and sess.get(2).st == "live" and claim == 2 and not(heldByOther))
  // ATTR-TOKEN on a flow: a flow of a dead launch queued across the next lease is accepted, its request
  // token compared with the live token and mismatching: closed, unregistered, no control message
  run StaleLaunchFlowIsClosedAtRequestReadWithNoMessage = InitWith("g")
    .then(Provision).then(VmStart).then(GuestOpenFlow(1)).then(VmStop).then(Teardown).then(Release)
    .then(Provision).then(VmStart).then(GuestOpenSession(1)).then(HostAcceptSession(1)).then(HostReadOpen(1)).then(Activate)
    .then(HostAcceptFlow(1)).expect(Safety and flows.get(1).st == "accepted" and flows.get(1).gen == 1 and gen == 2)
    .then(HostReadReqN(1, 2)).expect(Safety and flows.get(1).st == "closed" and not(crossMsg) and not(badPaired) and sessLive)
  // VmStop: a session still being read when the VMM dies is closed with no effect; a live one becomes
  // dead and the owner's processing of its loss releases the claim and aborts the flows
  run VmDeathMakesLiveSessionDeadThenLossIsProcessed = GuestOpenedFlowIsAdmittedReadAttachedAndPaired
    .then(VmStop).expect(Safety and sess.get(1).st == "dead" and claim == 1)
    .then(HostDeadSessionLoss(1)).expect(Safety and sess.get(1).st == "closed" and claim == 0 and flows.get(1).st == "closed")
  // D-UP-8 / DG-5-F: the guest holds its session until it observes the loss; it re-opens only then
  run GuestReopensSessionOnlyAfterObservingItsLoss = ControlSessionOpensAndCidActivates
    .expect(Safety and gSess == 1)
    .then(HostSessionLoss(1)).expect(Safety and sess.get(1).st == "closed" and gSess == 1)
    .expect(Safety and not(canGuestOpenSession(2)))
  run GuestObservesSessionLossThenReopens = ControlSessionOpensAndCidActivates
    .then(HostSessionLoss(1)).then(GuestSessionLossObserved(1)).expect(Safety and gSess == 0)
    .then(GuestOpenSession(2)).expect(Safety and gSess == 2 and sess.get(2).st == "pending")
  // D-UP-8: the guest owner dies with its VM: VmStop clears the session it held
  run VmStopClearsTheGuestSession = ControlSessionOpensAndCidActivates
    .then(VmStop).expect(Safety and gSess == 0 and sess.get(1).st == "dead")
  // ---- core: host-opened flows (INB-1 dial), U-1 / U-5 / K-A4
  // happy path: the dial opens V_h to the guest, the guest accepts V_g, connects the application, routes and
  // sends Paired; the dial returns Ok(GuestStream)
  run DialPairsAndReturnsStream = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).expect(Safety and hf.get(1).st == "req")
    .then(GuestAcceptV(1)).expect(Safety and gf.get(1).st == "acc" and gf.get(1).c == 1)
    .then(GuestPairOrRefuseOk(1, true)).expect(Safety and gf.get(1).st == "attached" and toHostPaired == Set(1))
    .then(HostRecvPaired(1)).expect(Safety and hf.get(1).st == "paired" and not(badPaired) and not(resurrected))
  // the application is not listening: Refused(ApplicationNotListening); the host closes the flow
  run DialRefusedReturnsTypedError = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(GuestAcceptV(1)).then(GuestPairOrRefuseOk(1, false))
    .expect(Safety and gf.get(1).st == "closed" and toHostRefused == Set(1))
    .then(HostRecvRefused(1)).expect(Safety and hf.get(1).st == "closed")
  // U-5: the host aborts before the guest accepted V_g; the guest discards the Abort for the unknown id,
  // accepts V_g and its pairing is ended by the host's close of V_h (TCPACCEPT-END, K-A4): no phantom connect
  run DialAbortedBeforeGuestAcceptsEndsByHostClose = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(HostAbortAccept(1)).expect(Safety and hf.get(1).st == "closed" and toGuestAbort == Set(1) and dl.res.get(1) == "Unreachable:io")
    .then(GuestRecvAbort(1)).expect(Safety and abortDiscarded == Set(1) and gf.get(1).st == "none")
    .then(GuestAcceptV(1)).expect(Safety and gf.get(1).st == "acc")
    .then(GuestSeesHostClose(1)).expect(Safety and gf.get(1).st == "closed" and not(phantom))
  // K-A4: V_g still in the guest's accept queue when the host closes V_h is torn down
  run DialQueuedVgIsResetByHostClose = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(HostAbortAccept(1)).then(GuestQueueReset(1))
    .expect(Safety and gf.get(1).st == "reset" and not(guestHalf(1)))
  // U-5 / ClosedIsTerminal: a Paired for a flow the host closed is discarded; the late Abort closes the
  // guest's attached half
  run LatePairedForClosedHostFlowIsDiscarded = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(GuestAcceptV(1)).then(GuestPairOrRefuseOk(1, true)).then(HostAbortAccept(1))
    .then(HostRecvPaired(1)).expect(Safety and hf.get(1).st == "closed" and not(resurrected))
    .then(GuestRecvAbort(1)).expect(Safety and gf.get(1).st == "closed")
  // session loss aborts the host-opened flow; the guest observes the session's loss and ends its half
  run SessionLossAbortsDialOnBothSides = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(GuestAcceptV(1)).then(HostSessionLoss(1))
    .expect(Safety and hf.get(1).st == "closed" and toGuestAbort == Set() and dl.res.get(1) == "NoSession" and not(dl.adm.contains(1)))
    .then(GuestSessionLossObserved(1)).expect(Safety and gf.get(1).st == "closed")
  // quiescence aborts host-opened flows and sends Abort on the live session; admission is refused while quiesced
  run QuiesceAbortsDialAndSendsAbort = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(GuestAcceptV(1)).then(Quiesce)
    .expect(Safety and hf.get(1).st == "closed" and toGuestAbort == Set(1) and dl.res.get(1) == "Quiesced" and not(dl.adm.contains(1)))
    .then(GuestRecvAbort(1)).expect(Safety and gf.get(1).st == "closed")
  // L-9: teardown ends a dial in flight; the dial reports NotServing (the admission variant of teardown), V_h is closed
  // and its V_g is torn down by the host's close (K-A4)
  run TeardownEndsPendingDialWithNotServing = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(GuestAcceptV(1)).then(Teardown)
    .expect(Safety and alloc == "Retiring" and hf.get(1).st == "closed" and dl.res.get(1) == "NotServing" and not(dl.adm.contains(1)))
    .then(GuestSeesHostClose(1)).expect(Safety and gf.get(1).st == "closed")
  // L-7 (dial row 5): session loss ends a pending dial with NoSession and closes its vsock; after the guest reconnects,
  // a new dial pairs (instance flows_two: two dial slots)
  run DialAfterReconnectPairs = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(HostSessionLoss(1))
    .expect(Safety and hf.get(1).st == "closed" and dl.res.get(1) == "NoSession" and not(sessLive))
    .then(GuestQueueReset(1)).then(GuestSessionLossObserved(1)).expect(Safety and gSess == 0 and gf.get(1).st == "reset")
    .then(GuestOpenSession(1)).then(HostAcceptSession(1)).then(HostReadOpen(1)).expect(Safety and sessLive and admissible)
    .then(DialOpen(2)).then(GuestAcceptV(2)).then(GuestPairOrRefuseOk(2, true)).then(HostRecvPaired(2))
    .expect(Safety and hf.get(2).st == "paired" and dl.res.get(2) == "Ok" and dl.res.get(1) == "NoSession" and not(resurrected))
  // DG-5-F: the session is lost; the guest owner aborts EVERY flow of it in the one dispatch that observes the
  // loss. Until then `lostSessionFlowOpen` holds, which disables GuestOpenSession (no reconnect precedes the
  // abort: GuestFlowsDoNotOutliveSession); then it reconnects
  run SessionLossAbortsAllFlowsBeforeReconnect = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(GuestAcceptV(1)).then(GuestPairOrRefuseOk(1, true))
    .then(DialOpen(2)).then(GuestAcceptV(2))
    .expect(Safety and gf.get(1).st == "attached" and gf.get(2).st == "acc" and gf.get(1).c == 1 and gf.get(2).c == 1)
    .then(HostSessionLoss(1)).expect(Safety and sess.get(1).st == "closed" and not(sessLive) and lostSessionFlowOpen
                                     and gf.get(1).st == "attached" and gf.get(2).st == "acc")
    .then(GuestSessionLossObserved(1)).expect(Safety and gf.get(1).st == "closed" and gf.get(2).st == "closed" and not(lostSessionFlowOpen))
    .then(GuestOpenSession(1)).then(HostAcceptSession(1)).then(HostReadOpen(1))
    .expect(Safety and sess.get(1).st == "live" and claim == 1 and sessLive)
  // DG-5-E: the host closes V_h while the guest's V_g is accepted and unattached (ReadHangup): the guest ends
  // the flow with no message; the flow is Closed, so it can neither pair nor refuse afterwards (U-1)
  run AcceptedVgHostCloseEndsFlowSilently = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(GuestAcceptV(1)).expect(Safety and gf.get(1).st == "acc")
    .then(HostAbortAccept(1)).expect(Safety and hf.get(1).st == "closed" and toGuestAbort == Set(1))
    .then(GuestSeesHostClose(1)).expect(Safety and gf.get(1).st == "closed" and toHostAbort == Set() and toHostPaired == Set() and toHostRefused == Set())
    .then(GuestRecvAbort(1)).expect(Safety and gf.get(1).st == "closed" and not(phantom) and toHostAbort == Set())
  // DG-5-E: an error on the accepted, unattached V_g (Hangup { Error }): the flow ends and the guest sends
  // Abort on the live session; the host aborts the flow; a pairing after the error finds the flow Closed (U-1, U-5)
  run AcceptedVgErrorAbortsWithAbort = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(GuestAcceptV(1))
    .then(GuestAcceptedError(1)).expect(Safety and gf.get(1).st == "closed" and toHostAbort == Set(1) and hf.get(1).st == "req")
    .then(HostRecvAbort(1)).expect(Safety and hf.get(1).st == "closed" and dl.res.get(1) == "Unreachable:ECONNRESET"
                                  and toHostAbort == Set() and toGuestAbort == Set() and not(resurrected))
  // INB-1: the guest's own connect to the application exceeds GUEST_INBOUND_CONNECT_TIMEOUT: TimedOut reaches the
  // dialer and the dial returns the typed error
  run DialTimedOutReturnsTypedError = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(GuestAcceptV(1)).then(GuestConnectTimesOut(1))
    .expect(Safety and gf.get(1).st == "closed" and dl.tout == Set(1) and dl.res.get(1) == "pending")
    .then(HostRecvTimedOut(1)).expect(Safety and hf.get(1).st == "closed" and dl.res.get(1) == "TimedOut" and dl.tout == Set())
  // INB-1: a dial to an address that is not an Active VM lease (before activation), or to an undeclared port, is
  // NotServing: a typed result and no state change
  run DialNotServingIsATypedResultWithNoStateChange = InitWith("g")
    .then(Provision).then(VmStart).then(DialNotServing(1, true))
    .expect(Safety and dl.res.get(1) == "NotServing" and hf.get(1).st == "none" and gf.get(1).st == "none" and toGuestAbort == Set())
    .then(GuestOpenSession(1)).then(HostAcceptSession(1)).then(HostReadOpen(1)).then(Activate)
    .then(DialNotServing(1, false)).expect(Safety and dl.res.get(1) == "NotServing" and hf.get(1).st == "none")
  // INB-1 / D15: while the node is quiesced a dial is Quiesced, a typed result with no state change; after restore it opens
  run DialWhileQuiescedIsATypedResultWithNoStateChange = ControlSessionOpensAndCidActivates
    .then(Quiesce).then(DialQuiesced(1))
    .expect(Safety and dl.res.get(1) == "Quiesced" and hf.get(1).st == "none" and toGuestAbort == Set())
    .then(Restore).then(DialOpen(1)).expect(Safety and hf.get(1).st == "req" and dl.res.get(1) == "pending")
  // INB-1: the allocation is Active and not quiesced but its control session is not live (it was lost): the dial is
  // the typed result NoSession, immediate, with no state change; once the guest re-opens a session the dial opens
  run DialWithoutLiveSessionIsATypedResultWithNoStateChange = ControlSessionOpensAndCidActivates
    .then(HostSessionLoss(1))
    .expect(Safety and alloc == "Active" and not(sessLive) and not(quiesced))
    .then(DialNoSession(1))
    .expect(Safety and dl.res.get(1) == "NoSession" and hf.get(1).st == "none" and gf.get(1).st == "none"
            and toGuestAbort == Set() and toHostAbort == Set())
    .expect(Safety and not(canDialOpen(1)))
  // INB-1: the VMM is gone while the lease is still Active and the owner has not yet observed the session's loss
  // (the session is `dead`, still live to the owner): step 3 passes, the vsock connect fails, the dial is
  // Unreachable, no state change; NoSession is not the answer while the loss is unobserved
  run DialToAGoneVmIsUnreachable = ControlSessionOpensAndCidActivates
    .then(VmStop)
    .expect(Safety and alloc == "Active" and not(vmUp) and sess.get(1).st == "dead" and sessLive and not(quiesced))
    .then(DialUnreachable(1))
    .expect(Safety and dl.res.get(1) == "Unreachable:ENODEV" and dl.adm == Set(1) and hf.get(1).st == "none" and gf.get(1).st == "none"
            and toGuestAbort == Set() and toHostAbort == Set())
    .expect(Safety and not(canDialNoSession(1)))
  // INB-1: the VMM is gone and the owner has observed the session's loss: step 3 answers NoSession (the owner
  // cannot observe vmUp; it never reaches the vsock connect), immediate, no state change; Unreachable is not the answer
  run DialAfterObservedSessionLossWithVmGoneReturnsNoSession = ControlSessionOpensAndCidActivates
    .then(VmStop).then(HostDeadSessionLoss(1))
    .expect(Safety and alloc == "Active" and not(vmUp) and sess.get(1).st == "closed" and not(sessLive) and not(quiesced))
    .then(DialNoSession(1))
    .expect(Safety and dl.res.get(1) == "NoSession" and hf.get(1).st == "none" and gf.get(1).st == "none"
            and toGuestAbort == Set() and toHostAbort == Set())
    .expect(Safety and not(canDialUnreachable(1)))
  // INB-1: the owner's pairing deadline is enabled only once the guest has replied or timed out (or cannot reply):
  // it cannot fire while the guest can still answer; after the guest's TimedOut the deadline fires, and the
  // late TimedOut that arrives afterwards is discarded
  run PairingDeadlineWaitsForTheGuestAnswer = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(GuestAcceptV(1))
    .expect(Safety and not(canHostDialDeadline(1)))
  run PairingDeadlineFiresOnlyAfterGuestTimeoutAndLateReplyIsDiscarded = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(GuestAcceptV(1)).then(GuestConnectTimesOut(1))
    .then(HostDialDeadline(1))
    .expect(Safety and hf.get(1).st == "closed" and dl.res.get(1) == "PairingDeadline" and not(dl.earlyDl) and not(dl.lateReply))
    .then(HostRecvTimedOut(1)).expect(Safety and dl.res.get(1) == "PairingDeadline" and dl.tout == Set() and hf.get(1).st == "closed")
  // H-1 (r1): the VMM exits while the dial awaits its pairing reply: the device-release orphan sweep resets V_h
  // (ECONNRESET) and the dial returns Unreachable { ECONNRESET } at once — not PairingDeadline; no message is sent
  run VmExitDuringDialReturnsUnreachableReset = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(GuestAcceptV(1)).then(VmStop)
    .expect(Safety and not(vmUp) and hf.get(1).st == "closed" and dl.res.get(1) == "Unreachable:ECONNRESET"
            and not(dl.adm.contains(1)) and toGuestAbort == Set() and gf.get(1).st == "closed")
  run VmExitDuringDialLeavesNoPairingDeadline = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(VmStop).expect(Safety and not(canHostDialDeadline(1)))
  // INB-1: a live VM whose guest never replies (its dial handler stalls): the pairing deadline ends the dial
  // (PairingDeadline), V_h is closed and Abort is sent on the live session
  run PairingDeadlineEndsDialToALiveVmWhoseGuestNeverReplies = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(GuestAcceptV(1)).then(GuestNeverReplies(1))
    .expect(Safety and vmUp and sessLive and not(canStillReply(1)))
    .then(HostDialDeadline(1))
    .expect(Safety and vmUp and hf.get(1).st == "closed" and dl.res.get(1) == "PairingDeadline" and not(dl.earlyDl)
            and toGuestAbort == Set(1))
  // INB-1 cancellation (r1 M-4 / L-9): the dialer drops the dial future; the owner ends the dial as at the deadline:
  // V_h closed, Abort on the live session; the guest's late Paired is discarded (U-5) and its half ends on the Abort
  run DroppedDialFutureEndsLikeThePairingDeadline = ControlSessionOpensAndCidActivates
    .then(DialOpen(1)).then(GuestAcceptV(1)).then(DialCancel(1))
    .expect(Safety and hf.get(1).st == "closed" and dl.res.get(1) == "Cancelled" and toGuestAbort == Set(1))
    .then(GuestPairOrRefuseOk(1, true)).then(HostRecvPaired(1))
    .expect(Safety and hf.get(1).st == "closed" and dl.res.get(1) == "Cancelled" and not(resurrected) and not(badPaired))
    .then(GuestRecvAbort(1)).expect(Safety and gf.get(1).st == "closed" and not(phantom))
  // ---- beacon (VM driver, port 1234; ADR-0157, ATTR-TOKEN, ATTR-LIVE-CLAIM), instance flows_beacon
  // happy path: the driver's launch is live from driver.start; READY is read, compared with the live token
  // and releases the beacon claim; driver.stop closes it and releases the claim; the CID is released
  run BeaconReadyReleasedThenDriverStopReleasesClaim = InitWith("g")
    .then(Provision).then(VmStart).expect(BeaconSafety and drvLive and vmGen == 1)
    .then(GuestOpenBeacon(1)).then(HostAcceptBeacon(1)).expect(BeaconSafety and bcn.get(1).st == "reading")
    .then(HostReadReady(1)).expect(BeaconSafety and bcn.get(1).st == "live" and bclaim == 1 and readyLive)
    .then(DriverStop).expect(BeaconSafety and bcn.get(1).st == "closed" and bclaim == 0 and not(drvLive))
    .then(VmStop).expect(BeaconSafety and not(vmUp) and not(drvLive) and bclaim == 0)
  // NoLiveLaunch: a beacon connection accepted after driver.stop is closed at accept
  run BeaconAfterDriverStopIsRefusedAtAccept = InitWith("g")
    .then(Provision).then(VmStart).then(GuestOpenBeacon(1)).then(DriverStop)
    .then(HostAcceptBeacon(1)).expect(BeaconSafety and bcn.get(1).st == "closed" and bclaim == 0)
  // ATTR-LIVE-CLAIM for the beacon: driver.stop between accept and READY read
  // closes it at the read with no claim; the next launch's READY is not refused SessionHeld
  run DriverStopBetweenAcceptAndReadyClosesWithoutClaim = InitWith("g")
    .then(Provision).then(VmStart).then(GuestOpenBeacon(1)).then(HostAcceptBeacon(1))
    .then(DriverStop).expect(BeaconSafety and bcn.get(1).st == "reading")
    .then(HostReadReady(1)).expect(BeaconSafety and bcn.get(1).st == "closed" and bclaim == 0)
    .then(VmStop).then(Provision).then(VmStart)
    .then(GuestOpenBeacon(2)).then(HostAcceptBeacon(2)).then(HostReadReady(2))
    .expect(BeaconSafety and bcn.get(2).st == "live" and bclaim == 2 and not(bHeldByOther))
  // K-A2': a beacon connection of a dead launch stays queued, is accepted while the next launch is live,
  // and is closed at its READY read on the stale token; the live launch's own READY is released
  run StaleLaunchBeaconIsClosedAtReadyAndDoesNotPoisonClaim = InitWith("g")
    .then(Provision).then(VmStart).then(GuestOpenBeacon(1)).then(DriverStop).then(VmStop)
    .then(Provision).then(VmStart).then(HostAcceptBeacon(1)).expect(BeaconSafety and bcn.get(1).st == "reading" and bcn.get(1).gen == 1 and vmGen == 2)
    .then(HostReadReady(1)).expect(BeaconSafety and bcn.get(1).st == "closed" and bclaim == 0 and not(readyCross))
    .then(GuestOpenBeacon(2)).then(HostAcceptBeacon(2)).then(HostReadReady(2))
    .expect(BeaconSafety and bcn.get(2).st == "live" and bclaim == 2 and not(readyCross) and not(bHeldByOther))
  // two beacon connections of the same launch: one takes the claim, the other is closed (SessionHeld)
  run SecondBeaconConnectionOfSameLaunchIsHeld = InitWith("g")
    .then(Provision).then(VmStart).then(GuestOpenBeacon(1)).then(GuestOpenBeacon(2))
    .then(HostAcceptBeacon(1)).then(HostAcceptBeacon(2)).then(HostReadReady(1)).then(HostReadReady(2))
    .expect(BeaconSafety and bcn.get(1).st == "live" and bcn.get(2).st == "closed" and bclaim == 1 and not(bHeldByOther))
  // D-UP-9: the VMM exits while its READY is released: the VM driver's one per-CID step ends the launch's liveness,
  // closes its beacon session and releases the claim; the next launch's READY takes it
  run VmExitEndsLaunchAndReleasesBeaconClaim = InitWith("g")
    .then(Provision).then(VmStart).then(GuestOpenBeacon(1)).then(HostAcceptBeacon(1)).then(HostReadReady(1))
    .expect(BeaconSafety and bclaim == 1 and drvLive)
    .then(VmStop).expect(BeaconSafety and not(drvLive) and bclaim == 0 and bcn.get(1).st == "closed")
    .then(Provision).then(VmStart).then(GuestOpenBeacon(2)).then(HostAcceptBeacon(2)).then(HostReadReady(2))
    .expect(BeaconSafety and bclaim == 2 and readyLive and not(bHeldByOther))
  // the VMM dies while a beacon connection is still being read: it is closed with no effect
  run VmDeathClosesUnreadBeaconConnection = InitWith("g")
    .then(Provision).then(VmStart).then(GuestOpenBeacon(1)).then(HostAcceptBeacon(1)).then(VmStop)
    .expect(BeaconSafety and bcn.get(1).st == "closed" and bclaim == 0)

  // non-vacuity witness kept: no named run reaches the phantom connect (violated: the state is reachable)
  val WitnessNoPhantomConnect: bool = not(phantom)
}


// ------------------------------------------------------------------ design instances
// Model-checking, liveness and hazard instances pass REC = false: `conf_step` stays at its initial value (D-UP-1).
// the design: one CID reused by 2 allocations, 2 control connections, 2 guest-opened flows, 1 host-opened flow
module flows_ok { import owner_flows(OFF = Set(), CFG = "core", REC = false).* from "./owner_flows" }
// the design, core, without guest-opened flows and dials: the instance of the SessionBecomesLive liveness check
module flows_sess { import owner_flows(OFF = Set(), CFG = "sess", REC = false).* from "./owner_flows" }
// the design, core, one guest-opened flow and one host-opened flow: the instance of the GuestHalfEnds liveness check
// (the property mentions the host-opened flow only; TLC's liveness mode on `flows_ok` took 53 min and 10 GB)
module flows_one { import owner_flows(OFF = Set(), CFG = "one", REC = false).* from "./owner_flows" }
// the design, core, two host-opened flows (two dial slots): the instance of the bounded Safety check of two concurrent dials
module flows_two { import owner_flows(OFF = Set(), CFG = "core2", REC = false).* from "./owner_flows" }
// the design, beacon: one CID reused by 2 allocations, VM start / exit and driver.stop, 2 beacon connections
module flows_beacon { import owner_flows(OFF = Set(), CFG = "beacon", REC = false).* from "./owner_flows" }

// ------------------------------------------------------------------ conformance mains (D-UP-1)
// The only instances that record `conf_step`; only named runs (evaluator) are checked on them. The host and guest
// owners' drivers name `flows_conf` (the core with two host-opened flows, so every core run replays on it); the VM
// driver's names `flows_beacon_conf` (D-UP-20: its own enabled actions).
module flows_conf { import owner_flows(OFF = Set(), CFG = "core2", REC = true).* from "./owner_flows" }
module flows_beacon_conf { import owner_flows(OFF = Set(), CFG = "beacon", REC = true).* from "./owner_flows" }
