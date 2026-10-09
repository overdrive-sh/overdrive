// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — module A: control sessions, flow admission, launch
 * attribution (session, flows and the VM driver's beacon), the host-opened (TcpAccept) pairing
 * exchange, and the flow half-close handshake.
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 *
 * Source of truth: docs/feature/netns-density-295/feature-delta.md § "vsock Attachment Replacement
 * DESIGN": § Control session (D18a / ADR-0166), § Wire contracts → Attribution (ATTR-TOKEN /
 * ADR-0177), § Per-kind total orders (TcpConnect steps 3-6, Datagram step 3 — D18 / ADR-0158, H-1;
 * TcpAccept steps 1-6, TCPACCEPT-END), § Owner and provisioner (provision / activate / teardown /
 * quiesce / restore, M-7 per-allocation order), § Composition (teardown, reclaim and stop: the
 * session claim released by teardown, the beacon claim by driver.stop, both before pool.release),
 * § Driven port — VM driver beacon (ADR-0157), § Flow end (half-close, ADR-0180), § Spike
 * remediation decisions — 2026-10-09 (ATTR-TOKEN) and round 2 (ATTR-LIVE-CLAIM, HALFCLOSE-SIGNAL),
 * § Formal protocol model (round 9 items 1-4 and 8), ADR-0160 (stop aborts flows); and the rulings
 * of 2026-10-06:
 *   U-1  Flow admission is one step with flow-table registration; a step-6 continuation stops for
 *        a closed flow (TcpConnect host step 6; TcpAccept guest step 5).
 *   U-5  A `Paired`, `Refused` or other control message for an unknown or closed flow id is
 *        discarded and counted, with no reply, by either side — except SLOT-ABORT (module B). U-5
 *        rests on assumption K-A4 below.
 *   ATTR-TOKEN (2026-10-09) Every guest-opened host connection is attributed by peer CID AND the
 *        launch token its first record carries. Flow: check-and-register (U-1), read the request,
 *        compare the token; a mismatch closes, unregisters and sends nothing. pool.release follows
 *        teardown and driver.stop.
 *   ATTR-LIVE-CLAIM (2026-10-09, answering the round-9 first-run counterexample) A CID has a LIVE
 *        launch token only while its launch is live — owner: the CID Provisioned or Active; VM
 *        driver: from driver.start until driver.stop. A session claim (owner, port 1243) or a beacon
 *        claim (driver, port 1234) is taken only in one per-CID serialized step after the
 *        connection's first record (ControlOpen / READY) has been read, which requires together
 *        that the record's token equals the CID's live launch token and that the claim is free.
 *        Teardown (-> Retiring) and driver.stop end the live token in the step that closes the
 *        launch's session and releases its claim; neither touches connections whose first record is
 *        unread (each is closed at its read). The accept-time state check only closes early. Every
 *        flow token comparison uses the live token.
 *   HALFCLOSE-SIGNAL (2026-10-09, ADR-0180) A flow vsock's end-of-stream is never by itself a
 *        half-close. The owner whose outer leg reads EOF (before or after Paired; before it, the EOF
 *        is recorded and carried out at Paired) drains, sends HalfClose on the control session, and
 *        shuts its vsock down for writing only after HalfCloseAck; the peer acknowledges at once
 *        (U-5 discard for a closed flow) and shuts its outer leg only for an announced direction; a
 *        vsock end for an unannounced direction aborts at once with reset (UnannouncedVsockEnd).
 *        The announcing wait ends by the acknowledgement, Abort, session loss, a flow error,
 *        teardown or quiescence; no timer.
 *
 * Abstraction:
 *   - CFG selects the instance: "core" (sessions, guest-opened flows, host-opened TcpAccept flows,
 *     the allocation lifecycle), "beacon" (the allocation lifecycle, VM start / exit, driver.stop and
 *     the beacon; ADR-0157) or "half" (one paired TCP flow between the guest owner and the host owner
 *     with the half-close handshake, owner death per side and session loss; the opener is chosen at
 *     init: guest = TcpConnect, host = TcpAccept). The parts share nothing else, so each is checked
 *     on its own instance; the state of the parts an instance switches off stays constant.
 *   - ONE guest CID. Successive allocations (leases) that lease the CID are distinguished by `gen`;
 *     one launch per lease (VmStart once per gen), so the launch token is the lease's `gen`
 *     (abstractly). The owner's live token is `gen` while alloc is Provisioned or Active; the
 *     driver's live token is `vmGen` while `drvLive`. Each connection carries the launch that
 *     opened it (`gen` field); its first record (ControlOpen / READY / request) is buffered at
 *     connect.
 *   - CONNS: guest control-session connections to host port 1243; BCONNS: guest beacon connections
 *     to host port 1234. A closed connection slot may be reused by a new connect (the guest
 *     reconnects).
 *   - FLOW_IDS: guest-opened flow connections to host 1240/1241 (TcpConnect / datagram legs share
 *     the acceptor order: check-and-register, read the 32-byte request in userspace, compare the
 *     token, THEN install).
 *   - HFLOWS: host-opened TcpAccept flows (host `hf`, guest `gf`), unchanged from round 8. The
 *     host opens V_h only to the CID's current launch (vmGen == gen), and a V_g dies with its VM,
 *     so the guest's own token check on a TcpAccept request (ATTR-TOKEN) never fails in this
 *     abstraction and is not modelled; TCPACCEPT-END (the guest ends a flow whose V_g reads EOF
 *     after the request) is the GuestSeesHostClose / GuestQueueReset pair.
 *   - Half-close (`hc`, CFG "half", "half_host", "half_guest"): per side ("g" guest owner, "h" host owner) the outer leg and
 *     its far end (the guest application; the destination or inbound client), the flow vsock and
 *     the control session. The flow vsock's end-of-stream (`vEnd`) and the control session's
 *     messages (`ch`, FIFO per direction) are independent channels with no order between them.
 *     A dying owner's own far end is reset by linger; its in-flight control messages are still
 *     delivered before the survivor observes the session's end; its vsock end reaches the survivor
 *     as its own action (VsockEndReachesSurvivor), unordered against that loss (K-B4). Drains are
 *     one step (FLOW_DRAIN_DEADLINE is a flow error). Teardown, quiescence and every flow error are
 *     one fault action per side (HcFlowError).
 *   - Per-CID serialization (M-7): provision, activate, teardown, quiesce, restore, driver.stop,
 *     session / beacon claim and loss are each ONE atomic action. The read of a first record is
 *     its own step and interleaves with them.
 *   - `quiesced` is the node admission latch as flows see it (holders: module `quiescence`).
 *   - Time is abstract: deadlines are nondeterministic failure actions. A live guest writes its
 *     first record right after connecting, well within FLOW_REQUEST_DEADLINE, so a read with
 *     nothing buffered fails only once the connection's VM is gone (the deadline is not modelled
 *     against a live writer).
 *   - Not modelled: flow quotas; the stale-entry 4-tuple refusal of a new connect (P-37) — a
 *     refused connect is a connect the guest retries, which the reconnect already covers; the
 *     drain counters (K2) and the linger clearing on a clean close.
 *
 * ASSUMPTIONS (kernel behaviour, not modelled mechanisms; discharged in README.md):
 *   K-A1  Peer-CID attribution is authentic (vhost stamps the device CID) ............ V-10 (P-17)
 *   K-A2′ (measured, P-37 / V-22) At VMM death, the host connections of that VM that are already
 *         accepted fail at their next I/O (VmStop: a live session becomes `dead`; an accepted
 *         connection whose first record is unread loses it); a connection still in a host accept
 *         queue stays queued and is accepted later — after the launch's teardown and the CID's
 *         next lease and launch included — with the CID as peer, delivering its buffered first
 *         record if it had one, and failing its first read otherwise.
 *   K-A3  A flow socket installed before its request is read hands the request bytes to the
 *         verdict, which drops them on the route miss (why H-1 exists) ................ P-23 / R5-9
 *   K-A4  The host's close of V_h of an aborted or session-lost TcpAccept reliably tears down the
 *         guest-side connection V_g — accepted, or still in the guest's accept queue (after the
 *         request read V_g reads EOF / POLLRDHUP, first write EPIPE). OFF "K-A4" drops it .. V-24 (P-38)
 *   K-B4  When an owner dies, the surviving owner observes the end of each of its flow vsocks — in
 *         the measured case before the control-session loss — and the far ends the dying owner
 *         held are reset by linger (HcOwnerDies, VsockEndReachesSurvivor) ........... V-13 (P-42)
 *
 * OFF names the rules switched off (teeth), or a candidate alternative switched on; the design
 * instance switches none off.
 */
module owner_flows {
  // ------------------------------------------------------------------ parameters
  const OFF: Set[str]
  const CFG: str            // "core" | "beacon" | "half" | "half_host" | "half_guest"

  val CORE: bool = CFG == "core"
  val BEACON: bool = CFG == "beacon"
  // `half` is the design instance. `half_host` / `half_guest` (S-4) are the conformance instances, one
  // per owner under test: `HcPeerDefectEnd(s)` is a defect of `s`'s own code, which a production owner
  // under test does not exhibit, so it is enabled only for the side NOT under test (the stimulus).
  val HALF: bool = CFG == "half" or CFG == "half_host" or CFG == "half_guest"
  val UNDER: str = if (CFG == "half_host") "h" else if (CFG == "half_guest") "g" else "none"

  val CONNS: Set[int] = Set(1, 2)
  val BCONNS: Set[int] = Set(1, 2)
  val FLOW_IDS: Set[int] = Set(1, 2)
  val MAXGEN: int = 2
  val HFLOWS: Set[int] = Set(1)
  val SIDES: Set[str] = Set("g", "h")

  val BUG_SESSION_SPLIT_CLAIM: bool = OFF.contains("sessionSplitClaim")     // L-3 violated: "no live claim?" checked, then claimed later
  val BUG_INSTALL_BEFORE_READ: bool = OFF.contains("installBeforeRead")     // H-1 violated: acceptor installs V_h before the full request
  val BUG_ADMIT_SPLIT: bool = OFF.contains("admitSplit")                    // admission check done, flow registered in the CID table later
  val BUG_PAIRED_NO_RECHECK: bool = OFF.contains("pairedNoRecheck")         // step 6 continuation ignores that the flow was aborted
  val BUG_NO_ABORT_ON_LOSS: bool = OFF.contains("noAbortOnLoss")            // control-session loss does not abort the CID's flows
  val BUG_TEARDOWN_SKIPS_FLOWS: bool = OFF.contains("teardownSkipsFlows")   // teardown closes listeners/session but not the flows
  val BUG_LATE_PAIRED_RESURRECTS: bool = OFF.contains("latePairedResurrects") // U-5 off: the host acts on a Paired for a flow it closed
  val BUG_PEER_CID_ONLY: bool = OFF.contains("peerCidOnly")                 // ATTR-TOKEN off: attribution by peer CID alone (round 8)
  val BUG_CLAIM_BEFORE_TOKEN: bool = OFF.contains("claimBeforeToken")       // ATTR-LIVE-CLAIM order off: the session claim is taken at
                                                                            // accept, before the ControlOpen is read
  val BUG_NO_CLAIM_RECHECK: bool = OFF.contains("noClaimRecheck")           // ATTR-LIVE-CLAIM off (the first round-9 design): the claim
                                                                            // step compares with the LEASE token (Retiring included);
                                                                            // the CID state is checked at accept only
  val BUG_BEACON_NO_LIVE_RECHECK: bool = OFF.contains("beaconNoLiveRecheck") // ATTR-LIVE-CLAIM off for the beacon: READY compared with
                                                                            // the lease's token, without the live-launch check
  val BUG_BEACON_PEER_CID_ONLY: bool = OFF.contains("beaconPeerCidOnly")    // ATTR-TOKEN off for the beacon: READY attributed by peer CID
  val BUG_RDHUP_IS_HALFCLOSE: bool = OFF.contains("rdhupIsHalfClose")       // HALFCLOSE-SIGNAL off (round 8): every vsock end is a half-close
  val BUG_NO_ACK: bool = OFF.contains("noAck")                              // HalfClose sent and the vsock shut down at once; the receiver
                                                                            // still aborts on a bare vsock end
  val BUG_ACK_WAITS: bool = OFF.contains("ackWaits")                        // the acknowledging side waits (acks only after its own outer
                                                                            // EOF) instead of acknowledging at once
  val BUG_EARLY_EOF_ABORTS: bool = OFF.contains("earlyEofAborts")           // an outer EOF before Paired aborts the flow
  val ENV_GUEST_MISSES_HOST_CLOSE: bool = OFF.contains("K-A4")              // drop K-A4: the guest never observes the host's close of V_h
  val ALT_GUEST_ABORT_TOMBSTONE: bool = OFF.contains("altTombstone")        // candidate: the guest keeps an Abort for a host-origin id it
                                                                            // has not accepted yet, and closes that flow at accept
  val ALT_TEARDOWN_CLOSES_READING: bool = OFF.contains("altTeardownClosesReading") // rejected alternative to ATTR-LIVE-CLAIM: teardown
                                                                            // also closes connections in `reading`, and the claim step
                                                                            // compares with the lease token (no live check)
  val ALT_PENDING_WAIT: bool = OFF.contains("altPendingWait")               // rejected alternative to the acknowledgement (V-13 variant C):
                                                                            // HalfClose without acknowledgement; a bare vsock end is held
                                                                            // until the message or session loss

  // ------------------------------------------------------------------ types
  // st: none | pending (host accept queue) | reading (accepted, CID check passed, first record not
  //     yet read) | checked (bug only) | live | dead (peer gone, loss not yet processed by the
  //     owner) | closed
  // gen: the launch that opened it. Its first record is buffered at connect (a live guest writes it
  // at once). A queued connection with nothing buffered fails its first read and is closed with no
  // effect — under claimBeforeToken it releases the claim exactly as a token mismatch does — so it
  // adds no behaviour and is not modelled.
  type Sess = { st: str, gen: int }
  // st: none | queued (accept queue) | checked (bug only) | accepted (registered, request not yet
  //     fully read and compared) | read (token matched: registration complete) | installed |
  //     connecting | paired | closed
  // req: request bytes read in userspace: 0, 1 (partial), 2 (all of it)
  // inFlight: a non-blocking connect was issued and its completion has not been processed yet
  // A flow's request is buffered at connect (a live guest writes it at once). A queued flow whose
  // VM died keeps it (K-A2′); one with no buffered request would fail its first read and close with
  // no effect, which HostFlowEnds already covers.
  type Flow = { st: str, gen: int, req: int, inFlight: bool }
  // One paired TCP flow, per side s ∈ {g, h} (CFG "half"). Direction s is the data from s's far end
  // toward the other side's far end.
  type HC = {
    opener: str,                // g = TcpConnect, h = TcpAccept
    paired: str -> bool,        // s considers the flow Paired (acceptor: at sending Paired; opener: at receiving it)
    outEof: str -> bool,        // s's outer leg read EOF (recorded when observed, before or after Paired)
    farHc: str -> bool,         // ghost: s's far end half-closed direction s
    ann: str -> str,            // s's announcement of direction s: none | pending (HalfClosing) | done (vsock shut down)
    acked: str -> bool,         // s acknowledged the peer's HalfClose: direction other(s) is announced at s
    ackDue: str -> bool,        // ackWaits only: an acknowledgement s has not sent yet
    outShut: str -> bool,       // s shut its outer leg down for writing (a FIN toward its far end)
    open: str -> bool,          // the flow is open at s (not aborted, not closed cleanly)
    far: str -> str,            // ghost: the first terminal event s's far end observes: none | eof | reset
    dead: str -> bool,          // s's owner died
    loss: str -> bool,          // s observed the control session's loss
    ctlDown: bool,              // the control session ended (dropped, or an owner died)
    ch: str -> List[str],       // control messages in flight from s: Paired | HalfClose | Ack | Abort (FIFO)
    vEnd: str -> bool,          // an end-of-stream of s's flow vsock in flight toward the other side
    vEndSent: str -> bool,      // s's flow vsock has ended (shutdown, abort or death)
    held: str -> bool,          // altPendingWait only: s holds a bare vsock end pending
    earlyShut: bool,            // ghost: a vsock was shut down before the peer's acknowledgement
    faulted: bool,              // ghost: a fault happened (owner death, session drop, flow error, outer reset, defect)
  }

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
  // gf.st: none | tomb (ALT only) | acc | routed | closed | reset (K-A4: V_g torn down while still
  //        in the guest's accept queue)
  // gf.c: the guest's live control connection when it accepted V_g (0 = none)
  var gf: int -> { st: str, gen: int, c: int }
  var toHostPaired: Set[int]
  var toHostRefused: Set[int]
  var toGuestAbort: Set[int]
  var resurrected: bool    // ghost: a host flow left `closed`
  var abortDiscarded: Set[int] // ghost: Aborts the guest discarded because the flow was unknown to it
  var phantom: bool        // ghost: the guest connected the application for a flow the host had
                           //        already closed and whose Abort it discarded as unknown
  // beacon (VM driver, port 1234; CFG "beacon")
  var drvLive: bool        // the driver's launch (vmGen) is live: started, not stopped
  var bclaim: int          // the CID's beacon claim (ClaimSet<GuestCid>): the connection holding it (0 = none)
  var bcn: int -> Sess     // beacon connections: none | pending | reading | live (claim held: READY released) | closed
  var readyCross: bool     // ghost: a READY of one launch was released while another launch is the CID's
  var bHeldByOther: bool   // ghost: a beacon connection was refused because one of another launch held the claim
  // half-close (CFG "half")
  var hc: HC

  // ------------------------------------------------------------------ helpers
  // registered in the CID's flow table (U-1), including a registration awaiting its token check
  def registered(st: str): bool = st.in(Set("accepted", "read", "installed", "connecting", "paired"))
  // registration completed (the request's token matched)
  def attributed(st: str): bool = st.in(Set("read", "installed", "connecting", "paired"))
  def sessUp(st: str): bool = st.in(Set("live", "dead"))
  val liveCount: int = CONNS.filter(c => sessUp(sess.get(c).st)).size()
  val sessLive: bool = liveCount > 0
  // the CID's state accepts a control session (Accept step: an early close only)
  val cidAcceptsSession: bool = alloc.in(Set("Provisioned", "Active"))
  // the CID's LIVE launch token (ATTR-LIVE-CLAIM): `gen` while the CID is Provisioned or Active
  def tokenMatches(g: int): bool = cidAcceptsSession and g == gen
  // the CID's lease token (the first round-9 design; noClaimRecheck / altTeardownClosesReading)
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
  // a host-side abort before Paired sends Abort only while the session is live
  def abortMsgs(liveAfter: bool): Set[int] = if (liveAfter) toGuestAbort.union(openH) else Set()
  action keepH = all {
    hf' = hf, gf' = gf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused,
    toGuestAbort' = toGuestAbort, resurrected' = resurrected, abortDiscarded' = abortDiscarded,
    phantom' = phantom,
  }
  action keepGhostsH = all { resurrected' = resurrected, abortDiscarded' = abortDiscarded, phantom' = phantom }
  action keepAttr = all { crossMsg' = crossMsg, heldByOther' = heldByOther }
  action keepB = all {
    drvLive' = drvLive, bclaim' = bclaim, bcn' = bcn, readyCross' = readyCross, bHeldByOther' = bHeldByOther,
  }
  // the beacon and the half-close state, unchanged
  action keepX = all { keepB, hc' = hc }

  // ------------------------------------------------------------------ init
  action init = all {
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
    toHostPaired' = Set(), toHostRefused' = Set(), toGuestAbort' = Set(),
    resurrected' = false, abortDiscarded' = Set(), phantom' = false,
    drvLive' = false, bclaim' = 0, bcn' = BCONNS.mapBy(b => { st: "none", gen: 0 }),
    readyCross' = false, bHeldByOther' = false,
    nondet o = (if (HALF) Set("g", "h") else Set("g")).oneOf()
    hc' = {
      opener: o,
      paired: SIDES.mapBy(x => false), outEof: SIDES.mapBy(x => false), farHc: SIDES.mapBy(x => false),
      ann: SIDES.mapBy(x => "none"), acked: SIDES.mapBy(x => false), ackDue: SIDES.mapBy(x => false),
      outShut: SIDES.mapBy(x => false), open: SIDES.mapBy(x => true), far: SIDES.mapBy(x => "none"),
      dead: SIDES.mapBy(x => false), loss: SIDES.mapBy(x => false), ctlDown: false,
      ch: SIDES.mapBy(x => List()), vEnd: SIDES.mapBy(x => false), vEndSent: SIDES.mapBy(x => false),
      held: SIDES.mapBy(x => false), earlyShut: false, faulted: false,
    },
  }

  // ------------------------------------------------------------------ allocation lifecycle (owner, per-CID order)
  // pool.assign + provision: a new lease (and its launch token) for the CID
  action Provision = all {
    alloc == "Absent", gen < MAXGEN,
    alloc' = "Provisioned", gen' = gen + 1,
    vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim, sess' = sess, flows' = flows,
    badPaired' = badPaired, keepAttr, keepH, keepX,
  }
  // driver.start: CH launched with the leased CID and the lease's launch token (once per lease); the
  // driver's launch becomes live (only the beacon instance tracks it)
  action VmStart = all {
    alloc == "Provisioned", not(vmUp), vmGen < gen,
    vmUp' = true, vmGen' = gen, drvLive' = BEACON,
    alloc' = alloc, gen' = gen, quiesced' = quiesced, claim' = claim, sess' = sess, flows' = flows,
    badPaired' = badPaired, keepAttr, keepH,
    bclaim' = bclaim, bcn' = bcn, readyCross' = readyCross, bHeldByOther' = bHeldByOther, hc' = hc,
  }
  // VMM exit (driver.stop's kill, or a crash). K-A2′: connections already accepted fail at their next
  // I/O (a live session becomes `dead`; an accepted session or beacon connection whose first record
  // is unread, and an accepted flow whose request is not fully read, fail their read and are closed,
  // with no other effect; under claimBeforeToken such a session's claim is released by the close);
  // queued connections stay queued with their buffered first record. A live beacon session keeps its
  // claim until driver.stop.
  action VmStop = all {
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
      if (bcn.get(b).gen == vmGen and bcn.get(b).st == "reading") { ...bcn.get(b), st: "closed" } else bcn.get(b)),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, quiesced' = quiesced, claim' = claim,
    badPaired' = badPaired, keepAttr,
    gf' = gf.keys().mapBy(h => if (gf.get(h).gen == vmGen and gf.get(h).st != "none") { ...gf.get(h), st: "closed" } else gf.get(h)),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    keepGhostsH,
    drvLive' = drvLive, bclaim' = bclaim, readyCross' = readyCross, bHeldByOther' = bHeldByOther, hc' = hc,
  }
  // activate: the only Provisioned -> Active edge; requires a live session; QuiescenceLatched otherwise
  action Activate = all {
    alloc == "Provisioned", sessLive, not(quiesced),
    alloc' = "Active",
    gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim, sess' = sess,
    flows' = flows, badPaired' = badPaired, keepAttr, keepH, keepX,
  }
  // teardown: Retiring — which ends the CID's live launch token — closes intake listeners and the
  // control session and releases its claim, in one step; aborts every flow of the CID (registrations
  // awaiting their token check included, U-1). It does not touch connections in `reading`
  // (ATTR-LIVE-CLAIM; the rejected alternative altTeardownClosesReading closes them too).
  action Teardown = all {
    alloc.in(Set("Provisioned", "Active")),
    alloc' = "Retiring",
    sess' = sess.keys().mapBy(c =>
      val s = sess.get(c)
      if (s.st.in(Set("live", "dead", "checked")) or (ALT_TEARDOWN_CLOSES_READING and s.st == "reading"))
        { ...s, st: "closed" }
      else s),
    claim' = 0,
    flows' = if (BUG_TEARDOWN_SKIPS_FLOWS) flows else abortAll,
    gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, badPaired' = badPaired, keepAttr,
    hf' = abortAllH, gf' = gf, toHostPaired' = Set(), toHostRefused' = Set(), toGuestAbort' = Set(),
    keepGhostsH, keepX,
  }
  // pool.release after driver.stop and teardown (ADR-0133): the CID may be leased again
  action Release = all {
    alloc == "Retiring", not(vmUp), not(drvLive),
    alloc' = "Absent",
    gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim, sess' = sess,
    flows' = flows, badPaired' = badPaired, keepAttr, keepH, keepX,
  }
  // quiesce_forwarding: latch, abort every live flow (listeners: module `intake`)
  action Quiesce = all {
    not(quiesced),
    quiesced' = true, flows' = abortAll,
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, claim' = claim, sess' = sess,
    badPaired' = badPaired, keepAttr, hf' = abortAllH, gf' = gf, toHostPaired' = toHostPaired,
    toHostRefused' = toHostRefused, toGuestAbort' = abortMsgs(sessLive), keepGhostsH, keepX,
  }
  action Restore = all {
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
  // the guest connects to (2, 1243) and writes its ControlOpen (its own launch token) — two first
  // connects may race (R5-18, L-3) — and reconnects after its connection ended (refusal or loss)
  def guestHasConn: bool = CONNS.exists(x =>
    sess.get(x).gen == vmGen and sess.get(x).st.in(Set("pending", "reading", "checked", "live", "dead")))
  action GuestOpenSession(c: int): bool = all {
    vmUp, sess.get(c).st == "none" or (sess.get(c).st == "closed" and not(guestHasConn)),
    sess' = sess.set(c, { st: "pending", gen: vmGen }),
    claim' = claim, heldByOther' = heldByOther, keepSessRest,
  }
  // Accept: an early close for a CID not Provisioned or Active; it admits nothing. (claimBeforeToken:
  // the claim is also taken here, before the ControlOpen is read; a held claim closes the connection.)
  action HostAcceptSession(c: int): bool = all {
    sess.get(c).st == "pending",
    if (not(cidAcceptsSession)) all {
      sess' = sess.set(c, { ...sess.get(c), st: "closed" }), claim' = claim, heldByOther' = heldByOther,
    } else if (BUG_CLAIM_BEFORE_TOKEN and claim != 0) all {
      sess' = sess.set(c, { ...sess.get(c), st: "closed" }), claim' = claim,
      heldByOther' = (heldByOther or sess.get(claim).gen != sess.get(c).gen),
    } else all {
      sess' = sess.set(c, { ...sess.get(c), st: "reading" }),
      claim' = if (BUG_CLAIM_BEFORE_TOKEN) c else claim, heldByOther' = heldByOther,
    },
    keepSessRest,
  }
  // Read the ControlOpen and, in the same per-CID step (ATTR-LIVE-CLAIM), compare its token with the
  // CID's LIVE launch token and take the claim (ClaimSet): a mismatch — another launch, or a CID no
  // longer Provisioned or Active — closes the connection with no claim (StaleLaunch); a held claim
  // closes it (SessionHeld).
  action HostReadOpen(c: int): bool = all {
    sess.get(c).st == "reading",
    val s = sess.get(c)
    val tokOk = BUG_PEER_CID_ONLY or
                (if (BUG_NO_CLAIM_RECHECK or ALT_TEARDOWN_CLOSES_READING) leaseTokenMatches(s.gen)
                 else tokenMatches(s.gen))
    val upSt = if (writerUp(s.gen)) "live" else "dead"
    if (BUG_CLAIM_BEFORE_TOKEN) all {
      // the claim is already this connection's (taken at accept)
      sess' = sess.set(c, { ...s, st: if (tokOk) upSt else "closed" }),
      claim' = if (tokOk) claim else (if (claim == c) 0 else claim),
      heldByOther' = heldByOther,
    } else if (not(tokOk)) all {
      sess' = sess.set(c, { ...s, st: "closed" }), claim' = claim, heldByOther' = heldByOther,
    } else if (claim != 0) all {
      sess' = sess.set(c, { ...s, st: "closed" }), claim' = claim,
      heldByOther' = (heldByOther or sess.get(claim).gen != s.gen),
    } else if (BUG_SESSION_SPLIT_CLAIM) all {
      sess' = sess.set(c, { ...s, st: "checked" }), claim' = claim, heldByOther' = heldByOther,
    } else all {
      sess' = sess.set(c, { ...s, st: upSt }), claim' = c, heldByOther' = heldByOther,
    },
    keepSessRest,
  }
  // BUG_SESSION_SPLIT_CLAIM: ... then claim, without re-checking.
  action HostClaimSession(c: int): bool = all {
    BUG_SESSION_SPLIT_CLAIM, sess.get(c).st == "checked",
    sess' = sess.set(c, { ...sess.get(c), st: if (writerUp(sess.get(c).gen)) "live" else "dead" }), claim' = c,
    heldByOther' = heldByOther, keepSessRest,
  }
  // Loss (peer EOF / reset, or a malformed message): release the claim, abort every flow of the VM.
  action HostSessionLoss(c: int): bool = all {
    sessUp(sess.get(c).st),
    sess' = sess.set(c, { ...sess.get(c), st: "closed" }),
    claim' = if (claim == c) 0 else claim,
    flows' = if (BUG_NO_ABORT_ON_LOSS) flows else abortAll,
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced,
    badPaired' = badPaired, keepAttr, hf' = abortAllH, gf' = gf, toHostPaired' = Set(), toHostRefused' = Set(), toGuestAbort' = Set(),
    keepGhostsH, keepX,
  }
  // the owner processes the loss of a session whose VM is gone (fair in SessionBecomesLive)
  action HostDeadSessionLoss(c: int): bool = all { sess.get(c).st == "dead", HostSessionLoss(c) }

  // ------------------------------------------------------------------ guest-opened flows (TcpConnect / datagram legs)
  action keepFlowRest = all {
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, heldByOther' = heldByOther, keepH, keepX,
  }
  // the guest connects and writes its request (with its launch token)
  action GuestOpenFlow(f: int): bool = all {
    vmUp, flows.get(f).st == "none",
    flows' = flows.set(f, { st: "queued", gen: vmGen, req: 0, inFlight: false }),
    badPaired' = badPaired, crossMsg' = crossMsg, keepFlowRest,
  }
  // step 3: accept V_h; check Active + live session + not quiesced (+ quota); register in the
  // CID's flow table — one owner step (U-1). Otherwise close, no message.
  action HostAcceptFlow(f: int): bool = all {
    not(BUG_ADMIT_SPLIT), flows.get(f).st == "queued",
    flows' = flows.set(f, { ...flows.get(f), st: if (admissible) "accepted" else "closed" }),
    badPaired' = badPaired, crossMsg' = crossMsg, keepFlowRest,
  }
  action HostCheckFlow(f: int): bool = all {
    BUG_ADMIT_SPLIT, flows.get(f).st == "queued",
    flows' = flows.set(f, { ...flows.get(f), st: if (admissible) "checked" else "closed" }),
    badPaired' = badPaired, crossMsg' = crossMsg, keepFlowRest,
  }
  action HostRegisterFlow(f: int): bool = all {
    BUG_ADMIT_SPLIT, flows.get(f).st == "checked",
    flows' = flows.set(f, { ...flows.get(f), st: "accepted" }),
    badPaired' = badPaired, crossMsg' = crossMsg, keepFlowRest,
  }
  // step 3: read the request in userspace (in one or two pieces); once all of it is read, compare
  // its launch token with the CID's live launch token: a mismatch closes V_h and removes the
  // registration, with no control message (StaleLaunch); a match completes the registration.
  action HostReadReq(f: int): bool = all {
    flows.get(f).st == "accepted", flows.get(f).req < 2,
    nondet n = Set(1, 2).oneOf()
    val x = flows.get(f)
    val r = if (x.req + n > 2) 2 else x.req + n
    val done = r == 2
    val tokOk = BUG_PEER_CID_ONLY or tokenMatches(x.gen)
    flows' = flows.set(f, { ...x, req: r,
                            st: if (not(done)) "accepted" else if (tokOk) "read" else "closed" }),
    badPaired' = badPaired, crossMsg' = crossMsg, keepFlowRest,
  }
  // step 3: only then install V_h (no route yet)
  action HostInstall(f: int): bool = all {
    flows.get(f).st == "read" or (BUG_INSTALL_BEFORE_READ and flows.get(f).st == "accepted"),
    flows' = flows.set(f, { ...flows.get(f), st: "installed" }),
    badPaired' = badPaired, crossMsg' = crossMsg, keepFlowRest,
  }
  // steps 4-5: policy, single resolution, arm D, issue a NON-BLOCKING connect
  action HostStartConnect(f: int): bool = all {
    flows.get(f).st == "installed",
    flows' = flows.set(f, { ...flows.get(f), st: "connecting", inFlight: true }),
    badPaired' = badPaired, crossMsg' = crossMsg, keepFlowRest,
  }
  // step 6: the connect completed: confirm D installed, route V_h -> D, send Paired on the session.
  action HostConnectDone(f: int): bool = all {
    flows.get(f).inFlight,
    flows.get(f).st == "connecting" or (BUG_PAIRED_NO_RECHECK and flows.get(f).st == "closed"),
    flows' = flows.set(f, { ...flows.get(f), st: "paired", inFlight: false }),
    badPaired' = (badPaired or not(admissible and flows.get(f).gen == gen)),
    crossMsg' = (crossMsg or crossFor(flows.get(f).gen)),
    keepFlowRest,
  }
  // the in-flight completion of an aborted flow is discarded
  action HostConnectDiscard(f: int): bool = all {
    flows.get(f).inFlight, flows.get(f).st == "closed", not(BUG_PAIRED_NO_RECHECK),
    flows' = flows.set(f, { ...flows.get(f), inFlight: false }),
    badPaired' = badPaired, crossMsg' = crossMsg, keepFlowRest,
  }
  // refusals, request / pairing deadlines, connect failure, peer EOF/reset: the flow closes. A flow
  // whose request was read and matched may be answered (Refused / Abort on the session); one not yet
  // attributed is closed with no message (its flow id is unknown or another launch's).
  action HostFlowEnds(f: int): bool = all {
    registered(flows.get(f).st) or flows.get(f).st == "checked",
    nondet msg = Set(true, false).oneOf()
    val sends = msg and attributed(flows.get(f).st)
    all {
      flows' = flows.set(f, { ...flows.get(f), st: "closed" }),
      crossMsg' = (crossMsg or (sends and crossFor(flows.get(f).gen))),
    },
    badPaired' = badPaired, keepFlowRest,
  }

  // ------------------------------------------------------------------ host-opened flows (TcpAccept)
  action keepCore = all {
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, flows' = flows, badPaired' = badPaired, heldByOther' = heldByOther, keepX,
  }
  action keepCoreMsg = all { keepCore, crossMsg' = crossMsg }
  def connLive(c: int): bool = CONNS.exists(x => x == c and sess.get(x).st == "live")
  // V_g is in the guest's accept queue: the host connected V_h to this VM, the guest has not accepted
  def queuedV(h: int): bool = and {
    gf.get(h).st.in(Set("none", "tomb")), hf.get(h).st != "none", hf.get(h).gen == vmGen, vmUp }
  def guestHalf(h: int): bool = gf.get(h).st.in(Set("acc", "routed")) or queuedV(h)
  val liveConn: int = CONNS.fold(0, (acc, c) => if (sess.get(c).st == "live" and sess.get(c).gen == vmGen) c else acc)
  // steps 1-2: a client reached a steered intake listener (serving ⇔ Active ∧ ¬Quiesced ∧ session
  // Live, module `intake`); the host connects V_h to (CID, 1242), writes the request with the lease token
  action HostOpenAccept(h: int): bool = all {
    hf.get(h).st == "none", admissible, vmUp, vmGen == gen,
    hf' = hf.set(h, { st: "req", gen: gen }),
    gf' = gf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    keepGhostsH, keepCoreMsg,
  }
  // step 3: the guest accepts V_g, reads the request (its own token), installs V_g.
  action GuestAcceptV(h: int): bool = all {
    hf.get(h).st != "none", hf.get(h).gen == vmGen, vmUp, gf.get(h).st.in(Set("none", "tomb")),
    gf' = gf.set(h, { st: if (gf.get(h).st == "tomb") "closed" else "acc", gen: vmGen, c: liveConn }),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    keepGhostsH, keepCoreMsg,
  }
  // steps 4-5: connect C to the application; refused → Refused(ApplicationNotListening); otherwise
  // (the flow is not Closed — U-1) route V_g → C and send Paired. Messages need the live session.
  action GuestPairOrRefuse(h: int): bool = all {
    gf.get(h).st == "acc", vmUp, connLive(gf.get(h).c),
    nondet ok = Set(true, false).oneOf()
    all {
      gf' = gf.set(h, { ...gf.get(h), st: if (ok) "routed" else "closed" }),
      toHostPaired' = if (ok) toHostPaired.union(Set(h)) else toHostPaired,
      toHostRefused' = if (ok) toHostRefused else toHostRefused.union(Set(h)),
      phantom' = (phantom or (ok and hf.get(h).st == "closed" and abortDiscarded.contains(h))),
    },
    hf' = hf, toGuestAbort' = toGuestAbort, resurrected' = resurrected, abortDiscarded' = abortDiscarded,
    keepCoreMsg,
  }
  // host step 6: Paired for an open flow → route; for a closed / unknown flow → discarded (U-5)
  action HostRecvPaired(h: int): bool = all {
    toHostPaired.contains(h),
    toHostPaired' = toHostPaired.exclude(Set(h)),
    if (hf.get(h).st == "req") all {
      hf' = hf.set(h, { ...hf.get(h), st: "paired" }), resurrected' = resurrected,
      badPaired' = (badPaired or not(admissible and hf.get(h).gen == gen)),
    } else if (BUG_LATE_PAIRED_RESURRECTS and hf.get(h).st == "closed") all {
      hf' = hf.set(h, { ...hf.get(h), st: "paired" }), resurrected' = true, badPaired' = badPaired,
    } else all {
      hf' = hf, resurrected' = resurrected, badPaired' = badPaired,
    },
    gf' = gf, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    abortDiscarded' = abortDiscarded, phantom' = phantom,
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, flows' = flows, crossMsg' = crossMsg, heldByOther' = heldByOther, keepX,
  }
  action HostRecvRefused(h: int): bool = all {
    toHostRefused.contains(h),
    toHostRefused' = toHostRefused.exclude(Set(h)),
    hf' = if (hf.get(h).st == "req") hf.set(h, { ...hf.get(h), st: "closed" }) else hf,
    gf' = gf, toHostPaired' = toHostPaired, toGuestAbort' = toGuestAbort, keepGhostsH, keepCoreMsg,
  }
  // the host closes an open host-opened flow before or after Paired (pairing deadline, peer error):
  // V_h closed with reset; Abort sent while the session is live
  action HostAbortAccept(h: int): bool = all {
    hOpen(hf.get(h).st),
    hf' = hf.set(h, { ...hf.get(h), st: "closed" }),
    toGuestAbort' = if (sessLive) toGuestAbort.union(Set(h)) else toGuestAbort,
    crossMsg' = (crossMsg or (sessLive and crossFor(hf.get(h).gen))),
    gf' = gf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, keepGhostsH, keepCore,
  }
  // the guest receives Abort: closes a known flow; an unknown one is discarded (U-5), or kept as a
  // tombstone (ALT)
  action GuestRecvAbort(h: int): bool = all {
    toGuestAbort.contains(h),
    toGuestAbort' = toGuestAbort.exclude(Set(h)),
    val st = gf.get(h).st
    all {
      gf' = if (st.in(Set("acc", "routed"))) gf.set(h, { ...gf.get(h), st: "closed" })
            else if (st == "none" and ALT_GUEST_ABORT_TOMBSTONE) gf.set(h, { st: "tomb", gen: vmGen, c: 0 })
            else gf,
      abortDiscarded' = if (st == "none" and not(ALT_GUEST_ABORT_TOMBSTONE)) abortDiscarded.union(Set(h)) else abortDiscarded,
    },
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused,
    resurrected' = resurrected, phantom' = phantom, keepCoreMsg,
  }
  // the guest observes the host's close of V_h (EOF / POLLRDHUP, TCPACCEPT-END) and ends its side (K-A4)
  action GuestSeesHostClose(h: int): bool = all {
    not(ENV_GUEST_MISSES_HOST_CLOSE), hf.get(h).st == "closed", gf.get(h).st.in(Set("acc", "routed")),
    gf' = gf.set(h, { ...gf.get(h), st: "closed" }),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    keepGhostsH, keepCoreMsg,
  }
  // the guest observes the loss of the session its flow belongs to and aborts every flow of it
  action GuestLossAbort(h: int): bool = all {
    gf.get(h).st.in(Set("acc", "routed")),
    not(connLive(gf.get(h).c)),
    gf' = gf.set(h, { ...gf.get(h), st: "closed" }),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    keepGhostsH, keepCoreMsg,
  }
  // K-A4: the host's close of V_h ends a V_g still in the guest's accept queue
  action GuestQueueReset(h: int): bool = all {
    not(ENV_GUEST_MISSES_HOST_CLOSE), hf.get(h).st == "closed", queuedV(h),
    gf' = gf.set(h, { st: "reset", gen: vmGen, c: 0 }),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    keepGhostsH, keepCoreMsg,
  }
  action guestCleanup = { nondet h = HFLOWS.oneOf() any { GuestRecvAbort(h), GuestSeesHostClose(h), GuestLossAbort(h), GuestQueueReset(h) } }

  // ------------------------------------------------------------------ beacon (VM driver, port 1234; ADR-0157, ATTR-TOKEN, ATTR-LIVE-CLAIM)
  action keepAllButB = all {
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, flows' = flows, badPaired' = badPaired, keepAttr, keepH, hc' = hc,
  }
  def guestHasBeacon: bool = BCONNS.exists(x =>
    bcn.get(x).gen == vmGen and bcn.get(x).st.in(Set("pending", "reading", "live")))
  // overdrive-init dials the beacon and (after the guest network) writes READY with its launch
  // token; it re-dials after a refusal within its attempt budget
  action GuestOpenBeacon(b: int): bool = all {
    BEACON, vmUp, bcn.get(b).st == "none" or (bcn.get(b).st == "closed" and not(guestHasBeacon)),
    bcn' = bcn.set(b, { st: "pending", gen: vmGen }),
    drvLive' = drvLive, bclaim' = bclaim, readyCross' = readyCross, bHeldByOther' = bHeldByOther,
    keepAllButB,
  }
  // accept: an early close when no live launch holds the CID (NoLiveLaunch); otherwise held
  // unclaimed while its first line is read
  action HostAcceptBeacon(b: int): bool = all {
    BEACON, bcn.get(b).st == "pending",
    bcn' = bcn.set(b, { ...bcn.get(b), st: if (drvLive) "reading" else "closed" }),
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
    drvLive' = drvLive, keepAllButB,
  }
  // driver.stop: ends the launch's liveness, closes its beacon session and releases the claim — one
  // step; it does not touch connections in `reading` (each is closed at its read). The VMM's exit
  // is VmStop, in either order.
  action DriverStop = all {
    BEACON, drvLive,
    drvLive' = false,
    bcn' = bcn.keys().mapBy(b => if (bcn.get(b).st == "live") { ...bcn.get(b), st: "closed" } else bcn.get(b)),
    bclaim' = 0, readyCross' = readyCross, bHeldByOther' = bHeldByOther,
    keepAllButB,
  }

  // ------------------------------------------------------------------ half-close (one paired TCP flow; HALFCLOSE-SIGNAL, ADR-0180)
  def other(s: str): str = if (s == "g") "h" else "g"
  // a control message from s (lost if the session has ended)
  def hcSend(x: HC, s: str, m: str): HC = if (x.ctlDown) x else { ...x, ch: x.ch.set(s, x.ch.get(s).append(m)) }
  // s's flow vsock ends (shutdown, abort or death): an end-of-stream travels to the other side
  def hcEndVsock(x: HC, s: str): HC =
    if (x.vEndSent.get(s)) x else { ...x, vEnd: x.vEnd.set(s, true), vEndSent: x.vEndSent.set(s, true) }
  // abort at s with reset semantics (ADR-0160): remove the pair, reset the far end, Abort to the peer
  // (when `msg` and the session lives), the vsock aborted (its end reaches the peer)
  def hcAbort(x: HC, s: str, msg: bool): HC =
    val x1 = { ...x, open: x.open.set(s, false), held: x.held.set(s, false),
               far: if (x.far.get(s) == "none") x.far.set(s, "reset") else x.far }
    hcEndVsock(if (msg) hcSend(x1, s, "Abort") else x1, s)
  def hcAlive(s: str): bool = hc.open.get(s) and not(hc.dead.get(s))
  // s shuts its outer leg down for writing (after its drain): its far end reads EOF
  def hcShutOuter(x: HC, s: str): HC =
    { ...x, outShut: x.outShut.set(s, true), held: x.held.set(s, false),
      far: if (x.far.get(s) == "none") x.far.set(s, "eof") else x.far }
  // s shuts its vsock down for writing: its announcement is done
  def hcShutVsock(x: HC, s: str): HC = hcEndVsock({ ...x, ann: x.ann.set(s, "done") }, s)
  action hcSet(n: HC): bool = all { hc' = n, keepB,
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, flows' = flows, badPaired' = badPaired, keepAttr, keepH,
  }

  // s's far end half-closes (writes, then EOF): s's outer leg reads EOF, recorded at once — before
  // Paired only on the opener's side (the acceptor's outer leg exists from its Paired on)
  action HcFarHalfClose(s: str): bool = all {
    HALF, hcAlive(s), not(hc.farHc.get(s)), hc.far.get(s) == "none", hc.paired.get(s) or s == hc.opener,
    val x1 = { ...hc, farHc: hc.farHc.set(s, true), outEof: hc.outEof.set(s, true) }
    hcSet(if (BUG_EARLY_EOF_ABORTS and not(hc.paired.get(s))) hcAbort(x1, s, true) else x1),
  }
  // the acceptor pairs (TcpConnect: host step 6; TcpAccept: guest step 5) and sends Paired
  action HcPair: bool = all {
    HALF,
    val a = other(hc.opener)
    all {
      hcAlive(a), not(hc.paired.get(a)), not(hc.loss.get(a)),
      hcSet(hcSend({ ...hc, paired: hc.paired.set(a, true) }, a, "Paired")),
    },
  }
  // the outer-leg-first path: EOF recorded and Paired → (drained) HalfClose; the vsock stays open
  // for writing until HalfCloseAck (noAck / altPendingWait: shut down at once)
  action HcAnnounce(s: str): bool = all {
    HALF, hcAlive(s), hc.paired.get(s), hc.outEof.get(s), hc.ann.get(s) == "none", not(hc.loss.get(s)),
    val x1 = hcSend({ ...hc, ann: hc.ann.set(s, "pending") }, s, "HalfClose")
    hcSet(if (BUG_NO_ACK or ALT_PENDING_WAIT) { ...hcShutVsock(x1, s), earlyShut: true } else x1),
  }
  // HalfClose for a flow s holds: acknowledge at once and record the direction as announced (U-5:
  // discarded for a closed flow)
  def hcOnHalfClose(x: HC, s: str): HC =
    if (ALT_PENDING_WAIT) (if (x.held.get(s)) hcShutOuter({ ...x, acked: x.acked.set(s, true) }, s)
                           else { ...x, acked: x.acked.set(s, true) })
    else if (BUG_ACK_WAITS) { ...x, acked: x.acked.set(s, true), ackDue: x.ackDue.set(s, true) }
    else hcSend({ ...x, acked: x.acked.set(s, true) }, s, "Ack")
  // the next control message from the other side is delivered at s (FIFO)
  action HcRecv(s: str): bool = all {
    HALF, not(hc.dead.get(s)), hc.ch.get(other(s)).length() > 0,
    val m = hc.ch.get(other(s)).head()
    val x0 = { ...hc, ch: hc.ch.set(other(s), hc.ch.get(other(s)).tail()) }
    hcSet(
      if (not(x0.open.get(s))) x0                                    // U-5: closed flow, discarded
      else if (m == "Paired") { ...x0, paired: x0.paired.set(s, true) }
      else if (m == "HalfClose") hcOnHalfClose(x0, s)
      else if (m == "Ack") (if (x0.ann.get(s) == "pending") hcShutVsock(x0, s) else x0)
      else hcAbort(x0, s, false)                                     // Abort
    ),
  }
  // ackWaits only: the deferred acknowledgement, once s's own outer leg read EOF
  action HcLateAck(s: str): bool = all {
    HALF, BUG_ACK_WAITS, hcAlive(s), hc.ackDue.get(s), hc.outEof.get(s),
    hcSet(hcSend({ ...hc, ackDue: hc.ackDue.set(s, false) }, s, "Ack")),
  }
  // the other side's vsock end-of-stream reaches s: for an announced direction a half-close (shut the
  // outer leg after the drain); otherwise the peer owner is lost — abort at once with reset
  // (UnannouncedVsockEnd)
  def hcOnVsockEnd(x: HC, s: str): HC =
    val x0 = { ...x, vEnd: x.vEnd.set(other(s), false) }
    if (not(x0.open.get(s))) x0
    else if (x0.acked.get(s) or BUG_RDHUP_IS_HALFCLOSE) hcShutOuter(x0, s)
    else if (ALT_PENDING_WAIT) { ...x0, held: x0.held.set(s, true) }
    else hcAbort(x0, s, true)
  // ... while the other owner is alive (a shutdown, an abort or a defect)
  action VsockEndArrives(s: str): bool = all {
    HALF, not(hc.dead.get(s)), hc.vEnd.get(other(s)), not(hc.dead.get(other(s))),
    hcSet(hcOnVsockEnd(hc, s)),
  }
  // ... from a dead owner: reaches the survivor as its own action, unordered against the survivor's
  // observation of the session's loss (K-B4; in the measured case before it)
  action VsockEndReachesSurvivor(s: str): bool = all {
    HALF, not(hc.dead.get(s)), hc.vEnd.get(other(s)), hc.dead.get(other(s)),
    hcSet(hcOnVsockEnd(hc, s)),
  }
  // clean close at s: both directions half-closed through the handshake
  action HcCleanClose(s: str): bool = all {
    HALF, hcAlive(s), hc.ann.get(s) == "done", hc.outShut.get(s),
    hcSet({ ...hc, open: hc.open.set(s, false) }),
  }
  // s observes the control session's end (after the dead peer's in-flight messages): every flow of
  // the VM is aborted
  action HcLossObserved(s: str): bool = all {
    HALF, hc.ctlDown, not(hc.dead.get(s)), not(hc.loss.get(s)), hc.ch.get(other(s)).length() == 0,
    val x1 = { ...hc, loss: hc.loss.set(s, true) }
    hcSet(if (x1.open.get(s)) hcAbort(x1, s, false) else x1),
  }
  // faults
  // s's far end errors or resets (before or after Paired): abort, Abort sent
  action HcFarReset(s: str): bool = all {
    HALF, hcAlive(s), hc.far.get(s) == "none",
    hcSet({ ...hcAbort(hc, s, true), faulted: true }),
  }
  // teardown, quiescence, a flow error or the drain deadline at s: abort, Abort sent
  action HcFlowError(s: str): bool = all {
    HALF, hcAlive(s),
    hcSet({ ...hcAbort(hc, s, true), faulted: true }),
  }
  // s's owner dies: linger resets the far end it holds; its vsock ends; its control session ends
  // (its in-flight messages are still delivered); messages toward it are lost
  action HcOwnerDies(s: str): bool = all {
    HALF, not(hc.dead.get(s)),
    val x1 = { ...hc, dead: hc.dead.set(s, true), open: hc.open.set(s, false), held: hc.held.set(s, false),
               far: if (hc.far.get(s) == "none") hc.far.set(s, "reset") else hc.far,
               ctlDown: true, ch: hc.ch.set(other(s), List()), faulted: true }
    hcSet(hcEndVsock(x1, s)),
  }
  // the session drops while both owners live (reset, malformed message): its messages are lost
  action HcSessionDrop: bool = all {
    HALF, not(hc.ctlDown),
    hcSet({ ...hc, ctlDown: true, ch: SIDES.mapBy(x => List()), faulted: true }),
  }
  // an Overdrive defect: s ends its flow vsock without the handshake while its session lives
  action HcPeerDefectEnd(s: str): bool = all {
    HALF, s != UNDER, hcAlive(s), not(hc.vEndSent.get(s)), not(hc.ctlDown),
    hcSet({ ...hcEndVsock(hc, s), faulted: true }),
  }

  // ------------------------------------------------------------------ next
  action lifeStep = any { Provision, VmStart, VmStop, Activate, Teardown, Release }
  action coreStep = {
    nondet c = CONNS.oneOf()
    nondet f = FLOW_IDS.oneOf()
    nondet h = HFLOWS.oneOf()
    any {
      HostOpenAccept(h), GuestAcceptV(h), GuestPairOrRefuse(h), HostRecvPaired(h), HostRecvRefused(h),
      HostAbortAccept(h), GuestRecvAbort(h), GuestSeesHostClose(h), GuestLossAbort(h), GuestQueueReset(h),
      Quiesce, Restore,
      GuestOpenSession(c), HostAcceptSession(c), HostReadOpen(c), HostClaimSession(c),
      HostSessionLoss(c),
      GuestOpenFlow(f), HostAcceptFlow(f), HostCheckFlow(f), HostRegisterFlow(f),
      HostReadReq(f), HostInstall(f), HostStartConnect(f), HostConnectDone(f),
      HostConnectDiscard(f), HostFlowEnds(f),
    }
  }
  action beaconStep = {
    nondet b = BCONNS.oneOf()
    any { GuestOpenBeacon(b), HostAcceptBeacon(b), HostReadReady(b), DriverStop }
  }
  action halfStep = {
    nondet s = SIDES.oneOf()
    any {
      HcFarHalfClose(s), HcPair, HcAnnounce(s), HcRecv(s), HcLateAck(s), VsockEndArrives(s),
      VsockEndReachesSurvivor(s), HcCleanClose(s), HcLossObserved(s),
      HcFarReset(s), HcFlowError(s), HcOwnerDies(s), HcSessionDrop, HcPeerDefectEnd(s),
    }
  }
  action step = any {
    all { not(HALF), lifeStep },
    all { CORE, coreStep },
    all { BEACON, beaconStep },
    all { HALF, halfStep },
  }

  // ------------------------------------------------------------------ invariants
  // (1) A CID has at most one live control session.
  val OneLiveSession: bool = liveCount <= 1
  // (3) No flow is installed before its full request has been read.
  val NoInstallBeforeRead: bool = FLOW_IDS.forall(f =>
    flows.get(f).st.in(Set("installed", "connecting", "paired")) implies flows.get(f).req == 2)
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
  val Safety: bool = and {
    OneLiveSession, NoInstallBeforeRead, NoAdmitWhenClosed, CurrentGeneration, ClosedIsTerminal,
    NoCrossLaunchMsg, NoCrossLaunchSessionHeld,
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
  val BeaconSafety: bool = and { BeaconCurrentLaunch, BeaconNoCrossLaunchReady, BeaconNoCrossLaunchHeld }

  // ---- half-close (HALFCLOSE-SIGNAL; CFG "half")
  // (13) a far end observes EOF for a direction only if that direction's far end (application or
  //      remote) half-closed it — never because an owner died
  val NoEofOnPeerLoss: bool = SIDES.forall(s => hc.far.get(s) == "eof" implies hc.farHc.get(other(s)))
  // (13) no owner shuts its flow vsock down before the peer acknowledged that direction's HalfClose
  val FinOnlyAfterAck: bool = not(hc.earlyShut)
  val HalfCloseSafety: bool = NoEofOnPeerLoss and FinOnlyAfterAck

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
                 abortDiscarded, phantom, drvLive, bclaim, bcn, readyCross, bHeldByOther, hc)
  val awaitingSession: bool = alloc == "Provisioned" and vmUp and vmGen == gen
  val launchLive: bool = CONNS.exists(c => sess.get(c).st == "live" and sess.get(c).gen == gen)
  temporal SessionBecomesLive =
    CONNS.forall(c => and {
      weakFair(GuestOpenSession(c), allVars), weakFair(HostAcceptSession(c), allVars),
      weakFair(HostReadOpen(c), allVars), weakFair(HostClaimSession(c), allVars),
      weakFair(HostDeadSessionLoss(c), allVars),
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

  // half-close progress: weak fairness of every owner step and channel delivery; none on the far
  // ends' choices or on faults
  temporal hcFair = and {
    weakFair(HcPair, allVars),
    SIDES.forall(s => and {
      weakFair(HcRecv(s), allVars), weakFair(HcAnnounce(s), allVars), weakFair(HcLateAck(s), allVars),
      weakFair(VsockEndArrives(s), allVars), weakFair(VsockEndReachesSurvivor(s), allVars),
      weakFair(HcLossObserved(s), allVars), weakFair(HcCleanClose(s), allVars),
    }),
  }
  def hcPending(s: str): bool = hc.open.get(s) and not(hc.dead.get(s)) and hc.ann.get(s) == "pending"
  // with no fault, an announced half-close reaches the other far end as EOF
  temporal HalfCloseCompletes =
    hcFair.implies(SIDES.forall(s =>
      always((hc.farHc.get(s) and not(hc.faulted)).implies(eventually(hc.far.get(other(s)) == "eof" or hc.faulted)))))
  // a HalfClosing wait ends (acknowledgement, Abort, session loss, a flow error, teardown or
  // quiescence) — faults included
  temporal AnnounceEnds =
    hcFair.implies(SIDES.forall(s => always(hcPending(s).implies(eventually(not(hcPending(s)))))))
  // an outer EOF recorded before Paired is carried out after Paired (never turned into an abort)
  temporal EarlyEofCarried =
    hcFair.implies(SIDES.forall(s =>
      always((hc.outEof.get(s) and not(hc.paired.get(s)) and not(hc.faulted))
        .implies(eventually(hc.ann.get(s) != "none" or hc.faulted)))))
  // altPendingWait: a held bare vsock end is eventually decided (the receiver never waits forever)
  temporal ReceiverPendingEnds =
    hcFair.implies(SIDES.forall(s => always(hc.held.get(s).implies(eventually(not(hc.held.get(s)))))))

  // ------------------------------------------------------------------ non-vacuity witnesses
  // Each is EXPECTED to be violated: the violation proves the behaviour is reachable.
  val WitnessNoPaired: bool = FLOW_IDS.forall(f => flows.get(f).st != "paired")
  val WitnessNoReuse: bool = gen < 2 or alloc != "Active"
  val WitnessNoQuiesceAbort: bool = not(quiesced and FLOW_IDS.exists(f => flows.get(f).st == "closed" and flows.get(f).req == 2))
  val WitnessNoLateCompletion: bool = not(FLOW_IDS.exists(f => flows.get(f).st == "closed" and flows.get(f).inFlight))
  val WitnessNoAcceptPaired: bool = HFLOWS.forall(h => hf.get(h).st != "paired")
  val WitnessNoPhantomConnect: bool = not(phantom)
  val WitnessNoQueueReset: bool = HFLOWS.forall(h => gf.get(h).st != "reset")
  // K-A2′: a dead launch's queued connection, its first record buffered, is accepted by the host
  // after the CID's next lease exists (a session being read, or a flow registered pending its check)
  val WitnessNoStaleAccept: bool = not(and {
    gen >= 2,
    or {
      CONNS.exists(c => sess.get(c).st == "reading" and sess.get(c).gen < gen),
      FLOW_IDS.exists(f => flows.get(f).st == "accepted" and flows.get(f).gen < gen),
    },
  })
  // the round-9 first-run shape: a connection accepted while the CID was Provisioned is read after
  // teardown (its CID Retiring) — the design then closes it at its read
  val WitnessNoReadAfterTeardown: bool = not(alloc == "Retiring" and CONNS.exists(c => sess.get(c).st == "reading"))
  // beacon: a launch's READY is released; a dead launch's beacon connection is read while the next
  // launch is live
  val WitnessNoReady: bool = not(readyLive)
  val WitnessNoStaleBeaconRead: bool = not(drvLive and BCONNS.exists(b => bcn.get(b).st == "reading" and bcn.get(b).gen < vmGen))
  // half-close: a clean half-close reaches a far end as EOF; a survivor aborts on a dead owner's
  // vsock end before it observes the session's loss (K-B4); an outer EOF before Paired
  val WitnessNoCleanEof: bool = SIDES.forall(s => hc.far.get(s) != "eof")
  val WitnessNoSurvivorAbortFirst: bool = not(SIDES.exists(s =>
    hc.dead.get(other(s)) and not(hc.dead.get(s)) and not(hc.loss.get(s)) and not(hc.open.get(s)) and hc.far.get(s) == "reset"))
  val WitnessNoEarlyEof: bool = not(SIDES.exists(s => hc.outEof.get(s) and not(hc.paired.get(s))))
}


// ------------------------------------------------------------------ design instances
// the design: one CID reused by 2 allocations, 2 control connections, 2 guest-opened flows, 1 host-opened flow
module flows_ok { import owner_flows(OFF = Set(), CFG = "core").* from "./owner_flows" }
// the design, beacon: one CID reused by 2 allocations, VM start / exit and driver.stop, 2 beacon connections
module flows_beacon { import owner_flows(OFF = Set(), CFG = "beacon").* from "./owner_flows" }
// the design, half-close: one paired TCP flow (TcpConnect or TcpAccept), both owners, faults per side
module flows_half { import owner_flows(OFF = Set(), CFG = "half").* from "./owner_flows" }
// conformance (S-4), the host owner under test: as `flows_half`, with the guest's defect enabled and the
// host's disabled. A closed flow's fields at a side are history: once `open(s)` is false the owner holds
// nothing of the flow and a driver compares nothing else of side `s`.
module flows_half_host { import owner_flows(OFF = Set(), CFG = "half_host").* from "./owner_flows" }
// conformance (S-4), the guest owner under test: the host's defect enabled, the guest's disabled
module flows_half_guest { import owner_flows(OFF = Set(), CFG = "half_guest").* from "./owner_flows" }
