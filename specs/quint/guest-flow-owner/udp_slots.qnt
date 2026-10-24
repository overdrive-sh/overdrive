// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — guest UDP slot pool and datagram associations.
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 *
 * Source of truth: feature-delta.md § Per-kind total orders ("Datagram association", "UDP slot
 * release"), § Guest adaptation contract (D25), user ruling SLOT-ABORT
 * (late Paired for a released slot → Abort), § Control session (Paired / Refused / Abort), spike
 * evidence P-35 (slot recycling hazard); and the user decision of 2026-10-06 that followed the first
 * model check (spike/quint-owner-findings.md U-2):
 *   U-2 After `Refused` / `Abort` the slot's parked frames are discarded; the slot stays owned by
 *          its (socket, destination) and does not re-associate for that association — only a new
 *          datagram from the application starts a new association (A-18).
 * and the datagram mode of the in-kernel forwarder (EVMOD-DGRAM, ADR-0201; PROPOSED 2026-10-16), which replaces
 * the framing and reassembly cells, their generations and CELL-RECREATE:
 *   EVMOD-DGRAM An association is one vsock STREAM (kind 2) whose pair the guest owner attaches to the slot
 *          socket S on `Paired`; datagrams the application sends before that wait in S's own receive queue (the
 *          only queue) and the attach-time run forwards them. Every end of an association (slot release, U-2,
 *          the pairing deadline) aborts the pair and DISCARDS S's receive queue without reading payload
 *          (recvmsg(MSG_DONTWAIT | MSG_TRUNC) until EAGAIN); the slot release then disconnects S and returns
 *          the slot.
 *
 * Abstraction:
 *   - SOCKS: application UDP sockets (by cookie; a cookie is never reused, so a closed socket never
 *     reopens). DESTS: original destinations. SLOTS: the guest slot pool (1,024 in the design).
 *   - `kernelSlots` is the capture program's QPROJ-7 claim, owned by ONE (socket, destination) as soon as the
 *     first datagram is redirected. `slots` is the owner's QPROJ-5 view, which publishes that claim only when
 *     `SlotReady` is consumed. Datagrams waiting in the slot socket's receive queue are a set of
 *     (socket, destination) tags (`parked`); a datagram is "delivered" when the association's pair is attached
 *     (`attached`) and the host association forwards it to ITS destination.
 *   - `fid`: the association's flow id (guest origin, allocated monotonically, never reused).
 *     host: the host's view of association `fid` (req → paired | closed) with ghost (sock, dest)
 *     and the slot index it was opened from.
 *   - Control messages are per-flow-id sets (order between different flow ids does not matter;
 *     each flow id gets at most one Paired/Refused).
 *   - Time is abstract. The guest owner's audit tick (GUEST_OWNER_AUDIT_PERIOD, DG-5-A) is ONE action,
 *     GuestAudit: the release audit, then the idle release of every slot idle for GUEST_DATAGRAM_SLOT_IDLE
 *     since the tick that last saw a change (a nondeterministic subset `idle`: which slots have reached the
 *     idle deadline is a matter of time), then the quarantine and `listener_map` retries (not modelled).
 *     There is no separate idle-release action. Each timer action (GuestAudit, GuestPairingTimeout) fires its
 *     own deadline only, one deadline per step; the order of simultaneously due deadlines (earliest first) is
 *     the driver's clock and is not modelled.
 *   - D-UP-20: the idle release applies only to an attached (Paired) slot: a slot awaiting Paired reaches its 5 s
 *     pairing deadline (FLOW_PAIRING_DEADLINE) long before its 120 s idle (GUEST_DATAGRAM_SLOT_IDLE). No
 *     condition is placed on the other slots. The orphan release (the cookie is gone) stays in the same tick.
 *   - D-UP-1 (conformance step record): on the conformance main `slots_conf` (REC = true), `conf_step` records the action
 *     that produced the state, as a variant of `udp_slots_conf::ConfStep` whose tag is the action's name and whose
 *     value is the record of its parameters by parameter name (`init`: the unit value). The wrappers
 *     `dispatchReady`, `GuestAudit` and `releaseAny` record the action they take (`GuestReady`, `GuestAuditIdle`,
 *     `GuestReleaseSock`). quint-connect reads the action and its picks from it (`Config::nondet`). Model-only; on
 *     the model-checking instances it stays `init` (see `rec`).
 *   - MAXFID only disables minting a new association id (DG-5-B); see its definition.
 *   - Readiness: the capture program's atomic claim and queued datagram precede the owner's observation.
 *     Association set-up (Datagram step 2, `GuestReady`) consumes that readiness, publishes the owner and
 *     starts the association in one dispatch. An attached slot socket is the forwarder's
 *     (its sk_data_ready is overridden), so the owner sees no readiness for it.
 *   - The end of an association as the guest learns it — the `Abort` / `Refused` control message, or the
 *     guest pair's end record (the host's abort reaches the guest forwarder as a peer abort) — is one
 *     handling (`GuestRecvEnd`); whichever arrives second finds no association and changes nothing.
 *
 * ASSUMPTIONS:
 *   K-B1  Closing T_g / Q_g is observed by the host as the association's end, even when the host
 *         accepts them after the guest closed them ......................... P-22; P-37
 *   K-B2  The guest `sock_release` hook reports a released socket, or the report is lost and the
 *         level-triggered audit (sock_diag) finds slots whose cookie no longer exists ....... R5-14
 *   K-B4  Draining a slot socket with recvmsg(MSG_DONTWAIT | MSG_TRUNC) until EAGAIN leaves its receive
 *         queue empty: a UDP socket has no other queue a datagram can come back from (udp.c @ v7.0) ... V-EV-DGRAM
 *
 * OFF names the rules switched off (teeth); the design instances switch none off.
 */
// D-UP-1: the variants of `conf_step` (a module of its own: a constructor would clash with the action of the same
// name, so the actions build their variant with the builtin `variant`)
module udp_slots_conf {
  type ConfStep =
    | init
    | AppSendOnSlot({ k: int, d: int, s: int })
    | AppClaim({ k: int, d: int, s: int })
    | AppPick({ k: int, d: int, s: int })
    | AppTake({ k: int, d: int, s: int })
    | GuestReady({ s: int })
    | HostPair({ i: int })
    | HostRefuse({ i: int })
    | HostAbortAssoc({ i: int })
    | GuestRecvPaired({ i: int })
    | GuestRecvEnd({ i: int })
    | GuestPairingTimeout({ s: int })
    | HostRecvEnd({ i: int })
    | SockClose({ k: int })
    | SockCloseReportLost({ k: int })
    | GuestReleaseSock({ k: int })
    | GuestAuditIdle({ idle: Set[int] })
}

module udp_slots {
  import udp_slots_conf.ConfStep from "./udp_slots"
  // OFF: design rules switched off (hazard instances in hazard/udp_slots_hazards.qnt); CFG: "ok" (two
  // destinations, three association ids), "live" (one destination: small enough for TLC) or "conf" (as "ok"
  // with MAXFID above any driver trace: the conformance instance, DG-5-B)
  const OFF: Set[str]
  const CFG: str
  // D-UP-1: true only on the module's conformance main, where `conf_step` records the action taken; every
  // model-checking, liveness and hazard instance passes false and keeps `conf_step` at its initial value.
  const REC: bool

  val SOCKS: Set[int] = Set(1, 2)
  val DESTS: Set[int] = if (CFG == "live") Set(1) else Set(1, 2)
  val SLOTS: Set[int] = Set(0, 1)
  // MAXFID is a model-checking bound with no production counterpart (production allocates FlowIds
  // monotonically, skipping live ids; DG-5-B): it only disables minting (GuestReady starts no association
  // once nextFid > MAXFID). The conformance instance ("conf") sizes it above the reach of any trace within a
  // driver's step bound: every step mints at most one flow id, and CONF_STEP_BOUND exceeds the step bound of
  // the quint-connect drivers (`#[quint_run]` max steps 10 by default; a driver raising its bound beyond
  // CONF_STEP_BOUND raises this constant with it).
  val CONF_STEP_BOUND: int = 32
  val MAXFID: int = if (CFG == "live") 2 else if (CFG == "conf") CONF_STEP_BOUND + 1 else 3

  val BUG_CLAIM_SPLIT: bool = OFF.contains("claimSplit")                // slot claim: "is s free?" and "take s" are two steps
  val BUG_KEEP_FRAMES: bool = OFF.contains("keepFrames")                // P-35: release returns the slot with frames still parked
  val BUG_LATE_PAIRED_BY_SLOT: bool = OFF.contains("latePairedBySlot")  // late Paired matched to the slot it was opened from, not by fid
  val BUG_IGNORE_LATE_PAIRED: bool = OFF.contains("ignoreLatePaired")   // late Paired silently dropped instead of answered with Abort
  val ENV_REPORT_LOSS: bool = true                                       // sock_release reports may be lost (audit is the backstop)
  val BUG_KEEP_FRAMES_ON_END: bool = OFF.contains("keepFramesOnEnd")    // U-2 / EVMOD-DGRAM off: the slot socket's queue is not
                                                                         // discarded at an end of association (Refused / Abort / the pairing deadline)

  type Tag = (int, int)                // (socket, destination) of a frame
  type Slot = { owned: bool, sock: int, dest: int, fid: int, attached: bool, parked: Set[Tag], holders: int }
  type KernelSlot = { owned: bool, sock: int, dest: int }
  type Assoc = { st: str, sock: int, dest: int, slot: int }

  var sockAlive: int -> bool
  var releasePending: int -> bool       // a sock_release report is queued for overdrive-init
  // QPROJ-7: the capture program's atomic slot claim exists before the
  // owner consumes SlotReady and publishes it through QPROJ-5 `slots`.
  var kernelSlots: int -> KernelSlot
  var slots: int -> Slot
  var picks: Set[{ k: int, d: int, s: int }]   // BUG_CLAIM_SPLIT only
  var nextFid: int
  var host: int -> Assoc
  var toGuestPaired: Set[int]
  var toGuestRefused: Set[int]
  var toGuestAbort: Set[int]
  var toHostAbort: Set[int]
  var toHostEnd: Set[int]
  var misdelivered: bool                // ghost: a frame reached a host association of another (socket, destination)
  var lateIgnored: bool                 // ghost: a late Paired was neither attached nor answered with Abort
  var endedNoNew: Set[int]              // ghost: slots whose association ended by Refused / Abort with
                                        //        no datagram from the application since
  var staleReassoc: bool                // ghost: such a slot re-associated without a new datagram
  var guestEnded: Set[int]              // association ids the guest ended itself (pairing deadline)
  var conf_step: ConfStep               // D-UP-1: the action that produced this state (model-only)

  val FIDS: Set[int] = 1.to(MAXFID)
  val freeKernelSlot: KernelSlot = { owned: false, sock: 0, dest: 0 }
  val freeSlot: Slot = { owned: false, sock: 0, dest: 0, fid: 0, attached: false, parked: Set(), holders: 0 }
  def ownerOf(k: int, d: int): Set[int] = SLOTS.filter(s => slots.get(s).owned and slots.get(s).sock == k and slots.get(s).dest == d)
  def kernelOwnerOf(k: int, d: int): Set[int] = SLOTS.filter(s => kernelSlots.get(s).owned and kernelSlots.get(s).sock == k and kernelSlots.get(s).dest == d)
  def mismatch(tags: Set[Tag], a: Assoc): bool = tags.exists(t => t != (a.sock, a.dest))
  // release (correct): abort the association's pair (the host observes it), discard the slot socket's queue,
  // return the slot
  def released(x: Slot): Slot = { ...freeSlot, parked: parkedAfterEnd(x, true) }
  def endOf(ss: Set[int]): Set[int] = ss.filter(s => slots.get(s).fid != 0).map(s => slots.get(s).fid)
  // the slot socket's queue after an end of association (U-2 when `rel` is false, release when true): discarded,
  // or kept (keepFrames / keepFramesOnEnd)
  def keepsFrames(rel: bool): bool = if (rel) BUG_KEEP_FRAMES else BUG_KEEP_FRAMES_ON_END
  def parkedAfterEnd(x: Slot, rel: bool): Set[Tag] = if (keepsFrames(rel)) x.parked else Set()

  action init = all {
    sockAlive' = SOCKS.mapBy(k => true),
    releasePending' = SOCKS.mapBy(k => false),
    kernelSlots' = SLOTS.mapBy(s => freeKernelSlot),
    slots' = SLOTS.mapBy(s => freeSlot),
    picks' = Set(),
    nextFid' = 1,
    host' = FIDS.mapBy(i => { st: "none", sock: 0, dest: 0, slot: 0 }),
    toGuestPaired' = Set(), toGuestRefused' = Set(), toGuestAbort' = Set(),
    toHostAbort' = Set(), toHostEnd' = Set(),
    misdelivered' = false, lateIgnored' = false, endedNoNew' = Set(), staleReassoc' = false, guestEnded' = Set(),
    conf_step' = variant("init", ()),
  }

  // ------------------------------------------------------------------ application sends
  // A datagram to (k, d) on the slot that already owns (k, d): delivered if attached, else parked.
  action AppSendOnSlot(k: int, d: int, s: int): bool = all {
    conf_step' = if (REC) variant("AppSendOnSlot", { k: k, d: d, s: s }) else conf_step,
    sockAlive.get(k), kernelSlots.get(s).owned, kernelSlots.get(s).sock == k, kernelSlots.get(s).dest == d,
    val x = slots.get(s)
    if (x.attached) all {
      misdelivered' = (misdelivered or (host.get(x.fid).st == "paired" and mismatch(Set((k, d)), host.get(x.fid)))),
      slots' = slots,
    } else all {
      misdelivered' = misdelivered,
      slots' = slots.set(s, { ...x, parked: x.parked.union(Set((k, d))) }),
    },
    sockAlive' = sockAlive, releasePending' = releasePending, kernelSlots' = kernelSlots, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    lateIgnored' = lateIgnored, endedNoNew' = endedNoNew.exclude(Set(s)), staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  // First datagram to a new (k, d): the capture program atomically claims a
  // free kernel slot and queues SlotReady. QPROJ-5 remains free until the
  // owner consumes that event in GuestReady.
  action AppClaim(k: int, d: int, s: int): bool = all {
    conf_step' = if (REC) variant("AppClaim", { k: k, d: d, s: s }) else conf_step,
    not(BUG_CLAIM_SPLIT), sockAlive.get(k), kernelOwnerOf(k, d) == Set(), not(kernelSlots.get(s).owned),
    kernelSlots' = kernelSlots.set(s, { owned: true, sock: k, dest: d }),
    slots' = slots.set(s, { ...slots.get(s), parked: slots.get(s).parked.union(Set((k, d))) }),
    sockAlive' = sockAlive, releasePending' = releasePending, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    misdelivered' = misdelivered, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew.exclude(Set(s)), staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  action AppPick(k: int, d: int, s: int): bool = all {
    conf_step' = if (REC) variant("AppPick", { k: k, d: d, s: s }) else conf_step,
    BUG_CLAIM_SPLIT, sockAlive.get(k), ownerOf(k, d) == Set(), not(slots.get(s).owned),
    not(picks.exists(p => p.k == k and p.d == d)),
    picks' = picks.union(Set({ k: k, d: d, s: s })),
    slots' = slots, kernelSlots' = kernelSlots, sockAlive' = sockAlive, releasePending' = releasePending, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    misdelivered' = misdelivered, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  action AppTake(k: int, d: int, s: int): bool = all {
    conf_step' = if (REC) variant("AppTake", { k: k, d: d, s: s }) else conf_step,
    BUG_CLAIM_SPLIT, picks.contains({ k: k, d: d, s: s }),
    picks' = picks.exclude(Set({ k: k, d: d, s: s })),
    kernelSlots' = kernelSlots.set(s, { owned: true, sock: k, dest: d }),
    slots' = slots.set(s, { ...slots.get(s), owned: true, sock: k, dest: d,
                           parked: slots.get(s).parked.union(Set((k, d))),
                           holders: slots.get(s).holders + 1 }),
    sockAlive' = sockAlive, releasePending' = releasePending, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    misdelivered' = misdelivered, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew.exclude(Set(s)), staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }

  // ------------------------------------------------------------------ association set-up
  // Datagram step 2: the owner consumes SlotReady, publishes the kernel
  // claim through QPROJ-5, claims a FlowId, opens V_g (kind 2), and writes the
  // request in one dispatch. A slot with an empty queue gives no readiness.
  action GuestReady(s: int): bool = all {
    conf_step' = if (REC) variant("GuestReady", { s: s }) else conf_step,
    val x = slots.get(s)
    val claim = kernelSlots.get(s)
    all {
      kernelSlots.get(s).owned, x.fid == 0, not(x.attached), x.parked != Set(), nextFid <= MAXFID,
      slots' = slots.set(s, { ...x, owned: true, sock: claim.sock, dest: claim.dest,
                              fid: nextFid, holders: if (x.owned) x.holders else 1 }),
      host' = host.set(nextFid, { st: "req", sock: claim.sock, dest: claim.dest, slot: s }),
      nextFid' = nextFid + 1,
      staleReassoc' = (staleReassoc or endedNoNew.contains(s)),
      endedNoNew' = endedNoNew.exclude(Set(s)),
    },
    sockAlive' = sockAlive, releasePending' = releasePending, kernelSlots' = kernelSlots, picks' = picks,
    toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    misdelivered' = misdelivered, lateIgnored' = lateIgnored, guestEnded' = guestEnded,
  }
  // Datagram steps 3-4 (host): pair, or refuse (Policy / HostInternal / Capacity / ...).
  action HostPair(i: int): bool = all {
    conf_step' = if (REC) variant("HostPair", { i: i }) else conf_step,
    host.get(i).st == "req",
    host' = host.set(i, { ...host.get(i), st: "paired" }),
    toGuestPaired' = toGuestPaired.union(Set(i)),
    sockAlive' = sockAlive, releasePending' = releasePending, kernelSlots' = kernelSlots, slots' = slots, picks' = picks,
    nextFid' = nextFid, toGuestRefused' = toGuestRefused, toGuestAbort' = toGuestAbort,
    toHostAbort' = toHostAbort, toHostEnd' = toHostEnd, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  action HostRefuse(i: int): bool = all {
    conf_step' = if (REC) variant("HostRefuse", { i: i }) else conf_step,
    host.get(i).st == "req",
    host' = host.set(i, { ...host.get(i), st: "closed" }),
    toGuestRefused' = toGuestRefused.union(Set(i)),
    sockAlive' = sockAlive, releasePending' = releasePending, kernelSlots' = kernelSlots, slots' = slots, picks' = picks,
    nextFid' = nextFid, toGuestPaired' = toGuestPaired, toGuestAbort' = toGuestAbort,
    toHostAbort' = toHostAbort, toHostEnd' = toHostEnd, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  // The host aborts a live association (error on H, quiescence, control-session loss, ...).
  action HostAbortAssoc(i: int): bool = all {
    conf_step' = if (REC) variant("HostAbortAssoc", { i: i }) else conf_step,
    host.get(i).st == "paired",
    host' = host.set(i, { ...host.get(i), st: "closed" }),
    toGuestAbort' = toGuestAbort.union(Set(i)),
    sockAlive' = sockAlive, releasePending' = releasePending, kernelSlots' = kernelSlots, slots' = slots, picks' = picks,
    nextFid' = nextFid, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toHostAbort' = toHostAbort, toHostEnd' = toHostEnd, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }

  // Datagram step 5 / SLOT-ABORT: Paired for i. Matched by flow id against a live association.
  action GuestRecvPaired(i: int): bool = all {
    conf_step' = if (REC) variant("GuestRecvPaired", { i: i }) else conf_step,
    toGuestPaired.contains(i),
    toGuestPaired' = toGuestPaired.exclude(Set(i)),
    val live = SLOTS.filter(s => slots.get(s).owned and slots.get(s).fid == i)
    val orig = host.get(i).slot
    if (live != Set()) all {
      // attach the pair (S, V_g): the attach-time run forwards the queued datagrams to the host association's destination
      slots' = slots.keys().mapBy(s => if (live.contains(s)) { ...slots.get(s), attached: true, parked: Set() } else slots.get(s)),
      misdelivered' = (misdelivered or live.exists(s => mismatch(slots.get(s).parked, host.get(i)))),
      toHostAbort' = toHostAbort, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
    } else if (BUG_LATE_PAIRED_BY_SLOT and slots.get(orig).owned) all {
      slots' = slots.set(orig, { ...slots.get(orig), fid: i, attached: true, parked: Set() }),
      misdelivered' = (misdelivered or mismatch(slots.get(orig).parked, host.get(i))),
      toHostAbort' = toHostAbort, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
    } else if (BUG_IGNORE_LATE_PAIRED) all {
      slots' = slots, misdelivered' = misdelivered, toHostAbort' = toHostAbort, lateIgnored' = true, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
    } else if (guestEnded.contains(i)) all {
      // the guest ended this association itself (pairing deadline) and sent Abort: discard (U-5)
      slots' = slots, misdelivered' = misdelivered, toHostAbort' = toHostAbort,
      lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
    } else all {
      // the slot was released (association ended): answer with Abort, attach nothing (SLOT-ABORT)
      slots' = slots, misdelivered' = misdelivered, toHostAbort' = toHostAbort.union(Set(i)),
      lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
    },
    sockAlive' = sockAlive, releasePending' = releasePending, kernelSlots' = kernelSlots, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestRefused' = toGuestRefused, toGuestAbort' = toGuestAbort,
    toHostEnd' = toHostEnd,
  }
  // Refused / Abort received: the association closes on the guest side; the slot stays owned by its
  // (socket, destination); its parked frames are discarded (U-2) and only the next datagram
  // re-associates (A-18).
  action GuestRecvEnd(i: int): bool = all {
    conf_step' = if (REC) variant("GuestRecvEnd", { i: i }) else conf_step,
    toGuestRefused.contains(i) or toGuestAbort.contains(i),
    toGuestRefused' = toGuestRefused.exclude(Set(i)),
    toGuestAbort' = toGuestAbort.exclude(Set(i)),
    val ended = SLOTS.filter(s => slots.get(s).owned and slots.get(s).fid == i)
    all {
      slots' = slots.keys().mapBy(s =>
        if (ended.contains(s)) { ...slots.get(s), fid: 0, attached: false, parked: parkedAfterEnd(slots.get(s), false) }
        else slots.get(s)),
      endedNoNew' = endedNoNew.union(ended),
    },
    sockAlive' = sockAlive, releasePending' = releasePending, kernelSlots' = kernelSlots, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    misdelivered' = misdelivered, lateIgnored' = lateIgnored, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  // FLOW_PAIRING_DEADLINE: the guest opener aborts an unpaired association and sends Abort; the
  // slot stays owned, its parked frames are discarded, it does not re-associate for them (U-2)
  action GuestPairingTimeout(s: int): bool = all {
    conf_step' = if (REC) variant("GuestPairingTimeout", { s: s }) else conf_step,
    val x = slots.get(s)
    all {
      x.owned, x.fid != 0, not(x.attached),
      // The production clock settles every deadline due at this instant.
      // A conformance step therefore names one pending association; the
      // model-checking instances retain the full abstract-time interleaving.
      not(REC) or SLOTS.filter(other => slots.get(other).fid != 0 and not(slots.get(other).attached)) == Set(s),
      // A release audit whose cookie is already gone has an earlier deadline
      // and releases the orphan before this 5 s pairing deadline.
      not(SLOTS.exists(other => slots.get(other).owned and not(sockAlive.get(slots.get(other).sock)))),
      slots' = slots.set(s, { ...x, fid: 0, parked: parkedAfterEnd(x, false) }),
      toHostAbort' = toHostAbort.union(Set(x.fid)),
      guestEnded' = guestEnded.union(Set(x.fid)),
      endedNoNew' = endedNoNew.union(Set(s)),
    },
    sockAlive' = sockAlive, releasePending' = releasePending, kernelSlots' = kernelSlots, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostEnd' = toHostEnd, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, staleReassoc' = staleReassoc,
  }
  // Host receives Abort, or observes T_h / Q_h EOF (K-B1): the association closes.
  action HostRecvEnd(i: int): bool = all {
    conf_step' = if (REC) variant("HostRecvEnd", { i: i }) else conf_step,
    toHostAbort.contains(i) or toHostEnd.contains(i),
    toHostAbort' = toHostAbort.exclude(Set(i)),
    toHostEnd' = toHostEnd.exclude(Set(i)),
    host' = host.set(i, { ...host.get(i), st: if (host.get(i).st == "none") "none" else "closed" }),
    sockAlive' = sockAlive, releasePending' = releasePending, kernelSlots' = kernelSlots, slots' = slots, picks' = picks,
    nextFid' = nextFid, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, misdelivered' = misdelivered, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }

  // ------------------------------------------------------------------ slot release (D25)
  action SockClose(k: int): bool = all {
    conf_step' = if (REC) variant("SockClose", { k: k }) else conf_step,
    sockAlive.get(k),
    sockAlive' = sockAlive.set(k, false),
    releasePending' = releasePending.set(k, true),
    kernelSlots' = kernelSlots, slots' = slots, picks' = picks.filter(p => p.k != k), nextFid' = nextFid, host' = host,
    toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused, toGuestAbort' = toGuestAbort,
    toHostAbort' = toHostAbort, toHostEnd' = toHostEnd, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  // the sock_release report is lost (K-B2): only the audit can find the slots
  action SockCloseReportLost(k: int): bool = all {
    conf_step' = if (REC) variant("SockCloseReportLost", { k: k }) else conf_step,
    ENV_REPORT_LOSS, sockAlive.get(k),
    sockAlive' = sockAlive.set(k, false),
    releasePending' = releasePending,
    kernelSlots' = kernelSlots, slots' = slots, picks' = picks.filter(p => p.k != k), nextFid' = nextFid, host' = host,
    toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused, toGuestAbort' = toGuestAbort,
    toHostAbort' = toHostAbort, toHostEnd' = toHostEnd, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  action releaseSet(ss: Set[int]): bool = all {
    kernelSlots' = kernelSlots.keys().mapBy(s => if (ss.contains(s)) freeKernelSlot else kernelSlots.get(s)),
    slots' = slots.keys().mapBy(s => if (ss.contains(s)) released(slots.get(s)) else slots.get(s)),
    toHostEnd' = toHostEnd.union(endOf(ss)),
    endedNoNew' = endedNoNew.exclude(ss),
  }
  // overdrive-init handles the report: every slot of the released socket (slot-ownership table)
  action GuestReleaseSock(k: int): bool = all {
    conf_step' = if (REC) variant("GuestReleaseSock", { k: k }) else conf_step,
    releasePending.get(k),
    releasePending' = releasePending.set(k, false),
    val mine = SLOTS.filter(s => slots.get(s).owned and slots.get(s).sock == k)
    releaseSet(mine),
    sockAlive' = sockAlive, picks' = picks, nextFid' = nextFid, host' = host,
    toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused, toGuestAbort' = toGuestAbort,
    toHostAbort' = toHostAbort, misdelivered' = misdelivered, lateIgnored' = lateIgnored, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  // The guest owner's audit tick (DG-5-A: periodic at GUEST_OWNER_AUDIT_PERIOD, fixed rate) — ONE step, in
  // this order: the release audit (slots whose socket cookie no longer exists, K-B2), then the idle release
  // of the slots idle for GUEST_DATAGRAM_SLOT_IDLE since the tick that last saw a change (an owned slot with
  // nothing parked that is not an orphan just released; time is abstract, so WHICH idle slots have reached
  // the idle deadline is the nondeterministic subset `idle`), then the quarantine retry and the
  // `listener_map` retry (not modelled). Idle release has no timer of its own. The tick fires only its own
  // deadline: the pairing deadline is a separate action (GuestPairingTimeout), one deadline per step.
  action GuestAudit: bool = {
    nondet idle = SLOTS.powerset().oneOf()
    GuestAuditIdle(idle)
  }
  // D-UP-20: an idle slot is an attached (Paired) slot with nothing parked (a slot awaiting Paired reaches its pairing
  // deadline first)
  action GuestAuditIdle(idle: Set[int]): bool = all {
    conf_step' = if (REC) variant("GuestAuditIdle", { idle: idle }) else conf_step,
    val orphans = SLOTS.filter(s => slots.get(s).owned and not(sockAlive.get(slots.get(s).sock)))
    all {
      // In the conformance instance, omit audit stutters while an association
      // has the earlier 5 s pairing deadline. They change no compared state;
      // the model-checking instances retain them.
      not(REC) or orphans != Set()
        or not(SLOTS.exists(s => slots.get(s).fid != 0 and not(slots.get(s).attached))),
      idle.forall(s => slots.get(s).owned and slots.get(s).attached and slots.get(s).parked == Set() and not(orphans.contains(s))),
      releaseSet(orphans.union(idle)),
    },
    sockAlive' = sockAlive, releasePending' = releasePending, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }

  action releaseAny = { nondet k = SOCKS.oneOf() GuestReleaseSock(k) }
  action step = {
    nondet k = SOCKS.oneOf()
    nondet d = DESTS.oneOf()
    nondet s = SLOTS.oneOf()
    nondet i = FIDS.oneOf()
    any {
      AppSendOnSlot(k, d, s), AppClaim(k, d, s), AppPick(k, d, s), AppTake(k, d, s),
      GuestReady(s), HostPair(i), HostRefuse(i), HostAbortAssoc(i), GuestRecvPaired(i),
      GuestRecvEnd(i), GuestPairingTimeout(s), HostRecvEnd(i), SockClose(k), SockCloseReportLost(k), GuestReleaseSock(k),
      GuestAudit,
    }
  }

  // ------------------------------------------------------------------ invariants
  // (2a) A slot is held by at most one (socket, destination); one slot per (socket, destination).
  val SlotHeldOnce: bool = and {
    SLOTS.forall(s => slots.get(s).holders <= 1),
    SLOTS.forall(a => SLOTS.forall(b => (a != b and kernelSlots.get(a).owned and kernelSlots.get(b).owned) implies
      (kernelSlots.get(a).sock, kernelSlots.get(a).dest) != (kernelSlots.get(b).sock, kernelSlots.get(b).dest))),
    SLOTS.forall(s => slots.get(s).owned implies and {
      kernelSlots.get(s).owned,
      slots.get(s).sock == kernelSlots.get(s).sock,
      slots.get(s).dest == kernelSlots.get(s).dest,
    }),
  }
  // (2b) A released slot never carries data; an owned slot carries only its owner's frames; no frame
  //      ever reaches a host association of another (socket, destination).
  val ReleasedSlotCarriesNoData: bool = and {
    SLOTS.forall(s => not(kernelSlots.get(s).owned) implies and {
      not(slots.get(s).owned), slots.get(s).parked == Set(),
      not(slots.get(s).attached), slots.get(s).fid == 0,
    }),
    SLOTS.forall(s => kernelSlots.get(s).owned implies
      slots.get(s).parked.forall(t => t == (kernelSlots.get(s).sock, kernelSlots.get(s).dest))),
    not(misdelivered),
  }
  // an attached slot forwards to its own current association
  val RouteCurrent: bool = SLOTS.forall(s => slots.get(s).attached implies and {
    slots.get(s).owned, slots.get(s).fid != 0,
    host.get(slots.get(s).fid).sock == slots.get(s).sock,
    host.get(slots.get(s).fid).dest == slots.get(s).dest,
  })
  // (2c) A late Paired for a released slot is answered with Abort (never ignored, never attached).
  val LatePairedAborted: bool = not(lateIgnored)
  // U-2: after Refused / Abort a slot re-associates only for a new datagram from its application (its socket's queue
  // was discarded at the end)
  val NoReassociationWithoutNewDatagram: bool = not(staleReassoc)
  val Safety: bool = SlotHeldOnce and ReleasedSlotCarriesNoData and RouteCurrent and LatePairedAborted and
    NoReassociationWithoutNewDatagram

  // ------------------------------------------------------------------ liveness (TLC)
  def orphan(s: int): bool = slots.get(s).owned and not(sockAlive.get(slots.get(s).sock))
  val anyOrphan: bool = SLOTS.exists(s => orphan(s))
  // A slot held by a released socket eventually returns to the pool (becomes claimable).
  temporal ReleasedSlotBecomesFree =
    and { weakFair(releaseAny, slots), weakFair(GuestAudit, slots) }
      .implies(always(anyOrphan.implies(eventually(not(anyOrphan)))))

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // Deterministic scenarios on the conformance main `slots_conf`, each ending in an `expect` of its outcome with
  // `Safety` checked along the way. `readyFor(s)`: the guard of Datagram step 2 (the slot socket is readable and the
  // slot can start an association).
  def readyFor(s: int): bool = and {
    kernelSlots.get(s).owned, slots.get(s).fid == 0, not(slots.get(s).attached), slots.get(s).parked != Set(), nextFid <= MAXFID,
  }
  // Datagram happy path: the first datagram claims a slot and waits in its socket's queue; readiness opens the
  // association; the host pairs; on Paired the guest attaches the pair and the attach-time run forwards the
  // waiting datagram; later datagrams are forwarded directly
  run DatagramAssociationClaimsPairsAndRoutes = init
    .then(AppClaim(1, 1, 0)).expect(Safety and not(slots.get(0).owned) and kernelOwnerOf(1, 1) == Set(0)
                                        and slots.get(0).parked == Set((1, 1)) and readyFor(0))
    .then(GuestReady(0)).expect(Safety and slots.get(0).fid == 1 and host.get(1).st == "req" and not(readyFor(0)))
    .then(HostPair(1)).then(GuestRecvPaired(1))
    .expect(Safety and slots.get(0).attached and slots.get(0).parked == Set() and host.get(1).st == "paired")
    .then(AppSendOnSlot(1, 1, 0)).expect(Safety and not(misdelivered) and slots.get(0).attached)
  // two sockets claim distinct slots; each slot has exactly one owner
  run TwoSocketsClaimDistinctSlots = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0)).then(AppClaim(2, 1, 1)).then(GuestReady(1))
    .expect(Safety and ownerOf(1, 1) == Set(0) and ownerOf(2, 1) == Set(1) and slots.get(0).holders == 1 and slots.get(1).holders == 1)
  // U-2 / A-18: Refused discards the slot socket's queue, the slot stays owned and
  // does not re-associate; only a new datagram from the application starts a new association
  run RefusedDiscardsParkedFramesAndOnlyNewDatagramReassociates = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0)).then(HostRefuse(1)).then(GuestRecvEnd(1))
    .expect(Safety and slots.get(0).owned and slots.get(0).fid == 0 and slots.get(0).parked == Set() and endedNoNew == Set(0))
    .then(AppSendOnSlot(1, 1, 0)).expect(Safety and slots.get(0).parked == Set((1, 1)) and endedNoNew == Set())
    .then(GuestReady(0)).expect(Safety and slots.get(0).fid == 2 and host.get(2).st == "req" and not(staleReassoc))
  // a host abort of a live association: Abort received, the slot stays owned and unattached
  run HostAbortOfLiveAssociationEndsItAndKeepsSlotOwned = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0)).then(HostPair(1)).then(GuestRecvPaired(1))
    .then(HostAbortAssoc(1)).then(GuestRecvEnd(1))
    .expect(Safety and slots.get(0).owned and not(slots.get(0).attached) and slots.get(0).fid == 0 and host.get(1).st == "closed")
    .then(AppSendOnSlot(1, 1, 0)).then(GuestReady(0)).expect(Safety and slots.get(0).fid == 2 and not(staleReassoc))
  // EVMOD-DGRAM: a datagram queued in the slot socket while the association was pending is discarded with the queue
  // at the association's end, so the slot gives no readiness and starts no association until the application
  // sends a new datagram (replaces the cell-generation discard of CELL-RECREATE)
  run DatagramQueuedBeforeAnEndIsDiscardedWithTheQueue = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0)).then(AppSendOnSlot(1, 1, 0))
    .expect(Safety and slots.get(0).parked == Set((1, 1)) and not(readyFor(0)))
    .then(HostRefuse(1)).then(GuestRecvEnd(1))
    .expect(Safety and slots.get(0).parked == Set() and slots.get(0).fid == 0 and not(readyFor(0)) and not(staleReassoc))
    .then(AppSendOnSlot(1, 1, 0)).expect(Safety and readyFor(0) and endedNoNew == Set())
  // SLOT-ABORT: the socket closes and its slot is released before the host's Paired arrives; the
  // late Paired is answered with Abort and attached nowhere
  run LatePairedForReleasedSlotIsAnsweredWithAbort = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0)).then(SockClose(1)).then(GuestReleaseSock(1))
    .expect(Safety and not(slots.get(0).owned) and toHostEnd == Set(1))
    .then(HostPair(1)).then(GuestRecvPaired(1))
    .expect(Safety and toHostAbort == Set(1) and not(lateIgnored) and not(slots.get(0).attached))
    .then(HostRecvEnd(1)).expect(Safety and host.get(1).st == "closed" and toHostAbort == Set() and toHostEnd == Set())
  // K-B2: the sock_release report is lost; the level-triggered audit finds the slot whose cookie no
  // longer exists and releases it
  run LostReleaseReportIsFoundByAudit = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0)).then(SockCloseReportLost(1))
    .expect(Safety and slots.get(0).owned and not(sockAlive.get(1)) and not(releasePending.get(1)))
    .then(GuestAudit).expect(Safety and not(slots.get(0).owned) and slots.get(0).parked == Set())
  // closing a socket releases every slot it owns
  run SocketCloseReleasesEverySlotItOwns = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0)).then(AppClaim(1, 2, 1)).then(GuestReady(1))
    .then(SockClose(1)).then(GuestReleaseSock(1))
    .expect(Safety and not(slots.get(0).owned) and not(slots.get(1).owned))
  // GUEST_DATAGRAM_SLOT_IDLE: an idle attached association's slot is released by the audit tick (no timer of
  // its own, DG-5-A) and the host observes the end
  run IdleSlotReleaseEndsAssociation = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0)).then(HostPair(1)).then(GuestRecvPaired(1))
    .then(GuestAuditIdle(Set(0))).expect(Safety and not(slots.get(0).owned) and not(slots.get(0).attached) and toHostEnd == Set(1))
    .then(HostRecvEnd(1)).expect(Safety and host.get(1).st == "closed")
  // DG-5-A: one audit tick runs, in order, the release audit (the slot of a closed socket whose report was
  // lost) and then the idle release (an idle attached slot): both slots are released in that one step. The
  // orphan is released by the release audit, so it is not among the idle slots chosen; a slot with frames
  // parked is not idle
  run AuditTickReleasesOrphansAndIdleSlotsInOneStep = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0)).then(HostPair(1)).then(GuestRecvPaired(1))
    .then(AppClaim(2, 1, 1)).expect(Safety and slots.get(1).parked == Set((2, 1)))
    .then(GuestReady(1)).then(HostPair(2)).then(GuestRecvPaired(2))
    .then(SockCloseReportLost(1))
    .expect(Safety and slots.get(0).attached and slots.get(1).attached and not(sockAlive.get(1)) and sockAlive.get(2))
    .then(GuestAuditIdle(Set(1)))
    .expect(Safety and not(slots.get(0).owned) and not(slots.get(1).owned) and toHostEnd == Set(1, 2))
  // FLOW_PAIRING_DEADLINE: the guest ends an unpaired association itself and sends Abort; the slot stays
  // owned with its frames discarded; the host's late Paired is discarded without a second Abort (U-5)
  run PairingDeadlineAbortsAndLatePairedIsDiscarded = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0)).then(GuestPairingTimeout(0))
    .expect(Safety and slots.get(0).owned and slots.get(0).fid == 0 and slots.get(0).parked == Set() and toHostAbort == Set(1) and guestEnded == Set(1))
    .then(HostPair(1)).then(GuestRecvPaired(1))
    .expect(Safety and toHostAbort == Set(1) and toGuestPaired == Set() and not(slots.get(0).attached) and not(lateIgnored))
    .then(HostRecvEnd(1)).expect(Safety and host.get(1).st == "closed")
  // P-35: a slot recycled to a new owner never carries the previous owner's datagrams: its socket's queue is
  // discarded at release, and the new owner's association delivers only its own datagrams to its own destination
  run RecycledSlotNeverDeliversPreviousOwnersFrames = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0)).then(SockClose(1)).then(GuestReleaseSock(1))
    .expect(Safety and not(slots.get(0).owned) and slots.get(0).parked == Set() and not(readyFor(0)))
    .then(AppClaim(2, 2, 0)).expect(Safety and slots.get(0).parked == Set((2, 2)) and readyFor(0))
    .then(GuestReady(0)).then(HostPair(2)).then(GuestRecvPaired(2))
    .expect(Safety and slots.get(0).attached and host.get(2).sock == 2 and host.get(2).dest == 2 and not(misdelivered))

}


// ------------------------------------------------------------------ design instances
// the design: 2 sockets, 2 destinations, 2 slots, 3 association ids, lost release reports
module slots_ok { import udp_slots(OFF = Set(), CFG = "ok", REC = false).* from "./udp_slots" }
// the design, one destination: small enough for TLC (exhaustive safety and liveness)
module slots_live { import udp_slots(OFF = Set(), CFG = "live", REC = false).* from "./udp_slots" }
// conformance main (D-UP-1, DG-5-B): as `slots_ok`, with MAXFID above any trace of a driver's step bound, recording
// `conf_step`; the runs are checked on it, the exhaustive checks on `slots_ok` / `slots_live`
module slots_conf { import udp_slots(OFF = Set(), CFG = "conf", REC = true).* from "./udp_slots" }
