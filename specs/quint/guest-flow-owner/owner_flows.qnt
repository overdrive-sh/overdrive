// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — module A: control sessions, flow admission, and the
 * host-opened (TcpAccept) pairing exchange.
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 *
 * Source of truth: docs/feature/netns-density-295/feature-delta.md § "vsock Attachment Replacement
 * DESIGN" (revision 5): § Control session (D18a / ADR-0166), § Per-kind total orders (TcpConnect
 * steps 3-6, Datagram step 3 — D18 / ADR-0158, H-1), § Owner and provisioner (provision / activate /
 * teardown / quiesce / restore, M-7 per-allocation order), § Lifecycle, § Composition (teardown
 * order), ADR-0160 (stop aborts flows); and the rulings of 2026-10-06 that followed the first model
 * check:
 *   U-1  Flow admission is one step with flow-table registration; a step-6 continuation stops for
 *        a closed flow (TcpConnect host step 6; TcpAccept guest step 5).
 *   U-5  (confirmed 2026-10-06 after the second model check) A `Paired`, `Refused` or other
 *        control message for an unknown or closed flow id is discarded and counted, with no reply,
 *        by either side — except a late `Paired` for a released UDP slot, answered with `Abort`
 *        (SLOT-ABORT, module B). The side that closes a flow before `Paired` sends `Abort` while
 *        the session is live; session loss aborts every flow on both sides. The discard counter is
 *        telemetry and is not modelled. U-5 rests on assumption K-A4 below.
 *
 * Abstraction:
 *   - ONE guest CID. Successive allocations that lease the CID are distinguished by a generation
 *     `gen` (the CID is reused after `pool.release`, ADR-0156). The host attributes connections by
 *     peer CID only (P-17) — it cannot see `gen`; the model can (ghost).
 *   - CONNS: guest control-session connection attempts to host port 1243.
 *   - FLOW_IDS: guest-opened flow connections to host 1240/1241 (TcpConnect / datagram legs share
 *     the acceptor order: read the 16-byte request in userspace, THEN install).
 *   - HFLOWS: host-opened TcpAccept flows (host `hf`, guest `gf`): the host connects V_h to the
 *     guest (CID, 1242) and writes the request; the guest accepts V_g, connects C to the
 *     application, and answers `Paired` (step 5) or `Refused(ApplicationNotListening)`. Control
 *     messages travel on the CID's control session (sets per direction; lost with the session).
 *     Host flow ids are never reused within the model (FlowId space not exhausted in a pairing
 *     window, A-FID).
 *   - Per-CID serialization (M-7): provision, activate, teardown, quiesce, restore, session
 *     open/loss are each ONE atomic action. Flow events are NOT in that order (the design lists
 *     only those six), so each flow step is its own action and interleaves with them.
 *   - `quiesced` is the node admission latch as flows see it: under D8a-HOLD (ADR-0169, revision 8)
 *     it is the projection "the holder set is non-empty"; who may quiesce and restore, and the
 *     named holders, are modelled in module D (steering.qnt).
 *   - Time is abstract: deadlines (FLOW_REQUEST_DEADLINE, FLOW_PAIRING_DEADLINE) are nondeterministic
 *     failure actions.
 *
 * ASSUMPTIONS (kernel behaviour, not modelled mechanisms) — each is named in the findings with the
 * validation item that discharges it:
 *   K-A1  Peer-CID attribution is authentic (vhost stamps the device CID) ............ V-10 (P-17)
 *   K-A2  When a VM's vhost device is released (VMM exit), every host vsock connection from that
 *         VM — including ones still in a host listener's accept queue — is reset before the CID can
 *         be reassigned. ENV_STALE_CONN_SURVIVES=true drops this assumption ......... NO V-item
 *   K-A3  A flow socket installed before its request is read hands the request bytes to the
 *         verdict, which drops them on the route miss (why H-1 exists) ................ P-23 / R5-9
 *   K-A4  The host's close of V_h of an aborted or session-lost TcpAccept reliably tears down the
 *         guest-side connection V_g — an accepted V_g observes the reset; a V_g still in the
 *         guest's accept queue is either removed from it (GuestQueueReset) or, if the guest accepts
 *         it after the close, observed as ended (GuestSeesHostClose). The reset may race the
 *         guest's accept; it still arrives. ENV_GUEST_MISSES_HOST_CLOSE=true drops this
 *         assumption ................................................................... V-24
 *
 * BUG_* constants each switch ONE design rule off (teeth). The design model sets all to false.
 */
module owner_flows {
  // ------------------------------------------------------------------ parameters
  const CONNS: Set[int]
  const FLOW_IDS: Set[int]
  const MAXGEN: int

  const BUG_SESSION_SPLIT_CLAIM: bool   // L-3 violated: "no live claim?" checked, then claimed later
  const BUG_INSTALL_BEFORE_READ: bool   // H-1 violated: acceptor installs V_h before the full request
  const BUG_ADMIT_SPLIT: bool           // admission check done, flow registered in the CID table later
  const BUG_PAIRED_NO_RECHECK: bool     // step 6 continuation ignores that the flow was aborted
  const BUG_NO_ABORT_ON_LOSS: bool      // control-session loss does not abort the CID's flows
  const BUG_TEARDOWN_SKIPS_FLOWS: bool  // teardown closes listeners/session but not the flows
  const ENV_STALE_CONN_SURVIVES: bool   // drop kernel assumption K-A2
  const HFLOWS: Set[int]
  const BUG_LATE_PAIRED_RESURRECTS: bool // U-5 off: the host acts on a Paired for a flow it closed
  const ENV_GUEST_MISSES_HOST_CLOSE: bool // drop K-A4: the guest never observes the host's close of V_h
  const ALT_GUEST_ABORT_TOMBSTONE: bool  // candidate: the guest keeps an Abort for a host-origin id it
                                         // has not accepted yet, and closes that flow at accept

  // ------------------------------------------------------------------ types
  // st: none | pending (in host accept queue) | checked (bug only) | live | dead (peer gone, loss
  //     not yet processed by the owner) | closed
  type Sess = { st: str, gen: int }
  // st: none | queued (accept queue) | checked (bug only) | accepted | read | installed |
  //     connecting | paired | closed
  // req: request bytes read in userspace: 0, 1 (partial), 2 (all 16 bytes)
  // inFlight: a non-blocking connect was issued and its completion has not been processed yet
  type Flow = { st: str, gen: int, req: int, inFlight: bool }

  // ------------------------------------------------------------------ state
  var alloc: str           // Absent | Provisioned | Active | Retiring   (owner CID table)
  var gen: int             // generation of the allocation currently leasing the CID
  var vmGen: int           // generation of the last VM launched on the CID
  var vmUp: bool
  var quiesced: bool       // node admission latch
  var claim: bool          // ClaimSet<GuestCid> membership for the CID
  var sess: int -> Sess
  var flows: int -> Flow
  var badPaired: bool      // ghost: a Paired was emitted while its admission conditions did not hold
  // host-opened TcpAccept flows
  // hf.st: none | req (V_h connected, request written, awaiting Paired / Refused) | paired | closed
  var hf: int -> { st: str, gen: int }
  // gf.st: none | tomb (ALT only: Abort seen before accept) | acc (V_g accepted, request read,
  //        installed) | routed (C connected, V_g routed, Paired sent) | closed | reset (K-A4: V_g
  //        torn down while still in the guest's accept queue; never accepted)
  // gf.c: the guest's live control connection when it accepted V_g (0 = none); its loss aborts the flow
  var gf: int -> { st: str, gen: int, c: int }
  var toHostPaired: Set[int]
  var toHostRefused: Set[int]
  var toGuestAbort: Set[int]
  var resurrected: bool    // ghost: a host flow left `closed` (Closed is terminal)
  var abortDiscarded: Set[int] // ghost: Aborts the guest discarded because the flow was unknown to it
  var phantom: bool        // ghost: the guest connected the application for a flow the host had
                           //        already closed and whose Abort it discarded as unknown

  // ------------------------------------------------------------------ helpers
  def registered(st: str): bool = st.in(Set("accepted", "read", "installed", "connecting", "paired"))
  def sessUp(st: str): bool = st.in(Set("live", "dead"))
  val liveCount: int = CONNS.filter(c => sessUp(sess.get(c).st)).size()
  val sessLive: bool = liveCount > 0
  // control session accept: the CID is Provisioned or Active and holds no live claim
  val sessionAcceptable: bool = alloc.in(Set("Provisioned", "Active")) and not(claim)
  val admissible: bool = alloc == "Active" and sessLive and not(quiesced)
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

  // ------------------------------------------------------------------ init
  action init = all {
    alloc' = "Absent",
    gen' = 0,
    vmGen' = 0,
    vmUp' = false,
    quiesced' = false,
    claim' = false,
    sess' = CONNS.mapBy(c => { st: "none", gen: 0 }),
    flows' = FLOW_IDS.mapBy(f => { st: "none", gen: 0, req: 0, inFlight: false }),
    badPaired' = false,
    hf' = HFLOWS.mapBy(h => { st: "none", gen: 0 }),
    gf' = HFLOWS.mapBy(h => { st: "none", gen: 0, c: 0 }),
    toHostPaired' = Set(), toHostRefused' = Set(), toGuestAbort' = Set(),
    resurrected' = false, abortDiscarded' = Set(), phantom' = false,
  }

  // ------------------------------------------------------------------ allocation lifecycle (owner, per-CID order)
  action Provision = all {
    alloc == "Absent", gen < MAXGEN,
    alloc' = "Provisioned", gen' = gen + 1,
    vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim, sess' = sess, flows' = flows,
    badPaired' = badPaired, keepH,
  }
  // driver.start: CH launched with the leased CID (after provision, G-V4)
  action VmStart = all {
    alloc == "Provisioned", not(vmUp), vmGen < gen,
    vmUp' = true, vmGen' = gen,
    alloc' = alloc, gen' = gen, quiesced' = quiesced, claim' = claim, sess' = sess, flows' = flows,
    badPaired' = badPaired, keepH,
  }
  // VMM exit (driver.stop, or a crash). K-A2: the kernel resets every connection of that VM,
  // including accept-queue entries; established sessions become `dead` until the owner sees HUP.
  action VmStop = all {
    vmUp,
    vmUp' = false,
    sess' = sess.keys().mapBy(c =>
      val s = sess.get(c)
      if (s.gen == vmGen and s.st == "pending" and not(ENV_STALE_CONN_SURVIVES)) { ...s, st: "closed" }
      else if (s.gen == vmGen and s.st == "live") { ...s, st: "dead" }
      else s),
    flows' = flows.keys().mapBy(f =>
      val x = flows.get(f)
      if (x.gen == vmGen and x.st == "queued" and not(ENV_STALE_CONN_SURVIVES)) { ...x, st: "closed" }
      else x),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, quiesced' = quiesced, claim' = claim,
    badPaired' = badPaired, gf' = gf.keys().mapBy(h => if (gf.get(h).gen == vmGen and gf.get(h).st != "none") { ...gf.get(h), st: "closed" } else gf.get(h)),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    keepGhostsH,
  }
  // activate: the only Provisioned -> Active edge; requires a live session; QuiescenceLatched otherwise
  action Activate = all {
    alloc == "Provisioned", sessLive, not(quiesced),
    alloc' = "Active",
    gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim, sess' = sess,
    flows' = flows, badPaired' = badPaired, keepH,
  }
  // teardown: Retiring; close intake listeners and the control session; abort every flow of the CID
  action Teardown = all {
    alloc.in(Set("Provisioned", "Active")),
    alloc' = "Retiring",
    sess' = sess.keys().mapBy(c =>
      val s = sess.get(c)
      if (s.st.in(Set("live", "dead", "checked"))) { ...s, st: "closed" } else s),
    claim' = false,
    flows' = if (BUG_TEARDOWN_SKIPS_FLOWS) flows else abortAll,
    gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, badPaired' = badPaired, hf' = abortAllH, gf' = gf, toHostPaired' = Set(), toHostRefused' = Set(), toGuestAbort' = Set(),
    keepGhostsH,
  }
  // pool.release after driver.stop and teardown (ADR-0133): the CID may be leased again
  action Release = all {
    alloc == "Retiring", not(vmUp),
    alloc' = "Absent",
    gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim, sess' = sess,
    flows' = flows, badPaired' = badPaired, keepH,
  }
  // quiesce_forwarding: latch, abort every live flow (listeners: module D)
  action Quiesce = all {
    not(quiesced),
    quiesced' = true, flows' = abortAll,
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, claim' = claim, sess' = sess,
    badPaired' = badPaired, hf' = abortAllH, gf' = gf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused,
    toGuestAbort' = abortMsgs(sessLive), keepGhostsH,
  }
  action Restore = all {
    quiesced,
    quiesced' = false,
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, claim' = claim, sess' = sess,
    flows' = flows, badPaired' = badPaired, keepH,
  }

  // ------------------------------------------------------------------ control session (D18a)
  action GuestOpenSession(c: int): bool = all {
    vmUp, sess.get(c).st == "none",
    sess' = sess.set(c, { st: "pending", gen: vmGen }),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    flows' = flows, badPaired' = badPaired, keepH,
  }
  // Correct: the CID-state check and the ClaimSet claim are ONE operation.
  action HostAcceptSession(c: int): bool = all {
    not(BUG_SESSION_SPLIT_CLAIM), sess.get(c).st == "pending",
    sess' = sess.set(c, { ...sess.get(c), st: if (sessionAcceptable) "live" else "closed" }),
    claim' = (claim or sessionAcceptable),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, flows' = flows,
    badPaired' = badPaired, keepH,
  }
  // BUG_SESSION_SPLIT_CLAIM: check ...
  action HostCheckSession(c: int): bool = all {
    BUG_SESSION_SPLIT_CLAIM, sess.get(c).st == "pending",
    sess' = sess.set(c, { ...sess.get(c), st: if (sessionAcceptable) "checked" else "closed" }),
    claim' = claim,
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, flows' = flows,
    badPaired' = badPaired, keepH,
  }
  // ... then claim, without re-checking.
  action HostClaimSession(c: int): bool = all {
    BUG_SESSION_SPLIT_CLAIM, sess.get(c).st == "checked",
    sess' = sess.set(c, { ...sess.get(c), st: "live" }), claim' = true,
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, flows' = flows,
    badPaired' = badPaired, keepH,
  }
  // Loss (peer EOF / reset, or a malformed message): release the claim, abort every flow of the VM.
  action HostSessionLoss(c: int): bool = all {
    sessUp(sess.get(c).st),
    sess' = sess.set(c, { ...sess.get(c), st: "closed" }),
    claim' = false,
    flows' = if (BUG_NO_ABORT_ON_LOSS) flows else abortAll,
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced,
    badPaired' = badPaired, hf' = abortAllH, gf' = gf, toHostPaired' = Set(), toHostRefused' = Set(), toGuestAbort' = Set(),
    keepGhostsH,
  }

  // ------------------------------------------------------------------ guest-opened flows (TcpConnect / datagram legs)
  action GuestOpenFlow(f: int): bool = all {
    vmUp, flows.get(f).st == "none",
    flows' = flows.set(f, { st: "queued", gen: vmGen, req: 0, inFlight: false }),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, badPaired' = badPaired, keepH,
  }
  // step 3: accept V_h; check Active + live session + not quiesced (+ quota); register in the
  // CID's flow table — one owner step. Otherwise close (NotActive / Quiesced / NoControlSession).
  action HostAcceptFlow(f: int): bool = all {
    not(BUG_ADMIT_SPLIT), flows.get(f).st == "queued",
    flows' = flows.set(f, { ...flows.get(f), st: if (admissible) "accepted" else "closed" }),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, badPaired' = badPaired, keepH,
  }
  action HostCheckFlow(f: int): bool = all {
    BUG_ADMIT_SPLIT, flows.get(f).st == "queued",
    flows' = flows.set(f, { ...flows.get(f), st: if (admissible) "checked" else "closed" }),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, badPaired' = badPaired, keepH,
  }
  action HostRegisterFlow(f: int): bool = all {
    BUG_ADMIT_SPLIT, flows.get(f).st == "checked",
    flows' = flows.set(f, { ...flows.get(f), st: "accepted" }),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, badPaired' = badPaired, keepH,
  }
  // step 3: read the 16-byte request in userspace (possibly in pieces)
  action HostReadReq(f: int): bool = all {
    flows.get(f).st == "accepted", flows.get(f).req < 2,
    flows' = flows.set(f, { ...flows.get(f), req: flows.get(f).req + 1,
                            st: if (flows.get(f).req + 1 == 2) "read" else "accepted" }),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, badPaired' = badPaired, keepH,
  }
  // step 3: only then install V_h (no route yet)
  action HostInstall(f: int): bool = all {
    flows.get(f).st == "read" or (BUG_INSTALL_BEFORE_READ and flows.get(f).st == "accepted"),
    flows' = flows.set(f, { ...flows.get(f), st: "installed" }),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, badPaired' = badPaired, keepH,
  }
  // steps 4-5: policy, single resolution, arm D, issue a NON-BLOCKING connect
  action HostStartConnect(f: int): bool = all {
    flows.get(f).st == "installed",
    flows' = flows.set(f, { ...flows.get(f), st: "connecting", inFlight: true }),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, badPaired' = badPaired, keepH,
  }
  // step 6: the connect completed: confirm D installed, route V_h -> D, send Paired.
  // Design: an aborted flow is Closed (terminal) — the continuation finds it Closed and stops.
  action HostConnectDone(f: int): bool = all {
    flows.get(f).inFlight,
    flows.get(f).st == "connecting" or (BUG_PAIRED_NO_RECHECK and flows.get(f).st == "closed"),
    flows' = flows.set(f, { ...flows.get(f), st: "paired", inFlight: false }),
    badPaired' = (badPaired or not(admissible and flows.get(f).gen == gen)),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, keepH,
  }
  // the in-flight completion of an aborted flow is discarded (design reading of "Closed is terminal")
  action HostConnectDiscard(f: int): bool = all {
    flows.get(f).inFlight, flows.get(f).st == "closed", not(BUG_PAIRED_NO_RECHECK),
    flows' = flows.set(f, { ...flows.get(f), inFlight: false }),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, badPaired' = badPaired, keepH,
  }
  // refusals, request / pairing deadlines, connect failure, peer EOF/reset: the flow closes
  action HostFlowEnds(f: int): bool = all {
    registered(flows.get(f).st) or flows.get(f).st == "checked",
    flows' = flows.set(f, { ...flows.get(f), st: "closed" }),
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, badPaired' = badPaired, keepH,
  }

  // ------------------------------------------------------------------ host-opened flows (TcpAccept)
  action keepCore = all {
    alloc' = alloc, gen' = gen, vmGen' = vmGen, vmUp' = vmUp, quiesced' = quiesced, claim' = claim,
    sess' = sess, flows' = flows, badPaired' = badPaired,
  }
  val guestSessLive: bool = CONNS.exists(c => sess.get(c).st == "live" and sess.get(c).gen == vmGen)
  def connLive(c: int): bool = CONNS.exists(x => x == c and sess.get(x).st == "live")
  // V_g is in the guest's accept queue: the host connected V_h to this VM, the guest has not accepted
  def queuedV(h: int): bool = and {
    gf.get(h).st.in(Set("none", "tomb")), hf.get(h).st != "none", hf.get(h).gen == vmGen, vmUp }
  // a guest-side connection of this TcpAccept exists (queued or accepted, not yet closed)
  def guestHalf(h: int): bool = gf.get(h).st.in(Set("acc", "routed")) or queuedV(h)
  val liveConn: int = CONNS.fold(0, (acc, c) => if (sess.get(c).st == "live" and sess.get(c).gen == vmGen) c else acc)
  // steps 1-2: a client reached a bound intake listener (bound ⇔ Active ∧ ¬Quiesced ∧ session Live,
  // module D); the host claims a host-origin FlowId, connects V_h to (CID, 1242), writes the request
  action HostOpenAccept(h: int): bool = all {
    hf.get(h).st == "none", admissible, vmUp, vmGen == gen,
    hf' = hf.set(h, { st: "req", gen: gen }),
    gf' = gf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    keepGhostsH, keepCore,
  }
  // step 3: the guest accepts V_g, reads the request, installs V_g. With ALT_GUEST_ABORT_TOMBSTONE a
  // remembered Abort closes the flow here.
  action GuestAcceptV(h: int): bool = all {
    hf.get(h).st != "none", hf.get(h).gen == vmGen, vmUp, gf.get(h).st.in(Set("none", "tomb")),
    gf' = gf.set(h, { st: if (gf.get(h).st == "tomb") "closed" else "acc", gen: vmGen, c: liveConn }),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    keepGhostsH, keepCore,
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
    keepCore,
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
    sess' = sess, flows' = flows,
  }
  action HostRecvRefused(h: int): bool = all {
    toHostRefused.contains(h),
    toHostRefused' = toHostRefused.exclude(Set(h)),
    hf' = if (hf.get(h).st == "req") hf.set(h, { ...hf.get(h), st: "closed" }) else hf,
    gf' = gf, toHostPaired' = toHostPaired, toGuestAbort' = toGuestAbort, keepGhostsH, keepCore,
  }
  // the host closes an open host-opened flow before or after Paired (pairing deadline, peer error):
  // V_h closed with reset; Abort sent while the session is live
  action HostAbortAccept(h: int): bool = all {
    hOpen(hf.get(h).st),
    hf' = hf.set(h, { ...hf.get(h), st: "closed" }),
    toGuestAbort' = if (sessLive) toGuestAbort.union(Set(h)) else toGuestAbort,
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
    resurrected' = resurrected, phantom' = phantom, keepCore,
  }
  // the guest observes the host's close of V_h (reset / EOF) and closes its side (K-A4)
  action GuestSeesHostClose(h: int): bool = all {
    not(ENV_GUEST_MISSES_HOST_CLOSE), hf.get(h).st == "closed", gf.get(h).st.in(Set("acc", "routed")),
    gf' = gf.set(h, { ...gf.get(h), st: "closed" }),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    keepGhostsH, keepCore,
  }
  // the guest observes the loss of the session its flow belongs to (before it reconnects) and
  // aborts every flow of that session
  action GuestLossAbort(h: int): bool = all {
    gf.get(h).st.in(Set("acc", "routed")),
    not(connLive(gf.get(h).c)),
    gf' = gf.set(h, { ...gf.get(h), st: "closed" }),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    keepGhostsH, keepCore,
  }
  // K-A4: the host's close of V_h removes a V_g still in the guest's accept queue
  action GuestQueueReset(h: int): bool = all {
    not(ENV_GUEST_MISSES_HOST_CLOSE), hf.get(h).st == "closed", queuedV(h),
    gf' = gf.set(h, { st: "reset", gen: vmGen, c: 0 }),
    hf' = hf, toHostPaired' = toHostPaired, toHostRefused' = toHostRefused, toGuestAbort' = toGuestAbort,
    keepGhostsH, keepCore,
  }
  action guestCleanup = { nondet h = HFLOWS.oneOf() any { GuestRecvAbort(h), GuestSeesHostClose(h), GuestLossAbort(h), GuestQueueReset(h) } }

  action step = {
    nondet c = CONNS.oneOf()
    nondet f = FLOW_IDS.oneOf()
    nondet h = HFLOWS.oneOf()
    any {
      HostOpenAccept(h), GuestAcceptV(h), GuestPairOrRefuse(h), HostRecvPaired(h), HostRecvRefused(h),
      HostAbortAccept(h), GuestRecvAbort(h), GuestSeesHostClose(h), GuestLossAbort(h), GuestQueueReset(h),
      Provision, VmStart, VmStop, Activate, Teardown, Release, Quiesce, Restore,
      GuestOpenSession(c), HostAcceptSession(c), HostCheckSession(c), HostClaimSession(c),
      HostSessionLoss(c),
      GuestOpenFlow(f), HostAcceptFlow(f), HostCheckFlow(f), HostRegisterFlow(f), HostReadReq(f),
      HostInstall(f), HostStartConnect(f), HostConnectDone(f), HostConnectDiscard(f),
      HostFlowEnds(f),
    }
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
  // (extra, § Composition: "The CID is never freed while a flow, listener or control session of the
  //  allocation exists") — every live session / registered flow belongs to the CURRENT allocation.
  val CurrentGeneration: bool = and {
    CONNS.forall(c => sess.get(c).st.in(Set("live", "dead", "checked")) implies
      (sess.get(c).gen == gen and alloc != "Absent")),
    FLOW_IDS.forall(f => (registered(flows.get(f).st) or flows.get(f).st == "checked") implies
      (flows.get(f).gen == gen and alloc != "Absent")),
    HFLOWS.forall(h => hOpen(hf.get(h).st) implies (hf.get(h).gen == gen and alloc != "Absent")),
  }
  // U-5 / § Flow end: Closed is terminal — a late Paired never revives a host-opened flow
  val ClosedIsTerminal: bool = not(resurrected)
  val Safety: bool = OneLiveSession and NoInstallBeforeRead and NoAdmitWhenClosed and CurrentGeneration and
    ClosedIsTerminal

  // ------------------------------------------------------------------ liveness (TLC)
  // U-5 with K-A4: a guest-side TcpAccept connection whose host side is closed — accepted or still
  // in the guest's accept queue — is eventually torn down (no orphaned application connection),
  // under fair guest cleanup
  temporal GuestHalfEnds =
    weakFair(guestCleanup, gf).implies(HFLOWS.forall(h =>
      always((hf.get(h).st == "closed" and guestHalf(h)).implies(eventually(not(guestHalf(h)))))))

  // ------------------------------------------------------------------ non-vacuity witnesses
  // Each is EXPECTED to be violated: the violation proves the behaviour is reachable.
  val WitnessNoPaired: bool = FLOW_IDS.forall(f => flows.get(f).st != "paired")
  val WitnessNoReuse: bool = gen < 2 or alloc != "Active"
  val WitnessNoQuiesceAbort: bool = not(quiesced and FLOW_IDS.exists(f => flows.get(f).st == "closed" and flows.get(f).req == 2))
  val WitnessNoLateCompletion: bool = not(FLOW_IDS.exists(f => flows.get(f).st == "closed" and flows.get(f).inFlight))
  val WitnessNoAcceptPaired: bool = HFLOWS.forall(h => hf.get(h).st != "paired")
  // an Abort sent before the guest accepted V_g is discarded as unknown, and the guest then
  // connects the application for that already-closed flow (expected reachable under U-5)
  val WitnessNoPhantomConnect: bool = not(phantom)
  // K-A4 removes a queued V_g whose host side closed before the guest accepted it
  val WitnessNoQueueReset: bool = HFLOWS.forall(h => gf.get(h).st != "reset")
}


// ------------------------------------------------------------------ design instances
// the design: one CID reused by 2 allocations, 2 control connections, 2 guest-opened flows, 1 host-opened flow
module flows_ok {
  import owner_flows(CONNS = Set(1, 2), FLOW_IDS = Set(1, 2), MAXGEN = 2, BUG_SESSION_SPLIT_CLAIM = false, BUG_INSTALL_BEFORE_READ = false, BUG_ADMIT_SPLIT = false, BUG_PAIRED_NO_RECHECK = false, BUG_NO_ABORT_ON_LOSS = false, BUG_TEARDOWN_SKIPS_FLOWS = false, ENV_STALE_CONN_SURVIVES = false, HFLOWS = Set(1), BUG_LATE_PAIRED_RESURRECTS = false, ENV_GUEST_MISSES_HOST_CLOSE = false, ALT_GUEST_ABORT_TOMBSTONE = false).* from "./owner_flows"
}
