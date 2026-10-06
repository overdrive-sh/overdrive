// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — module B: guest UDP slot pool and datagram associations.
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 *
 * Source of truth: feature-delta.md § Per-kind total orders ("Datagram association", "UDP slot
 * release"), § Guest adaptation contract (D25 guest intake model — PROPOSED), user ruling SLOT-ABORT
 * (late Paired for a released slot → Abort), § Control session (Paired / Refused / Abort), spike
 * evidence P-35 (slot recycling hazard); and the user decision of 2026-10-06 that followed the first
 * model check (spike/quint-owner-findings.md U-2):
 *   U-2 After `Refused` / `Abort` the slot's parked frames are discarded; the slot stays owned by
 *          its (socket, destination) and does not re-associate for that association — only a new
 *          datagram from the application starts a new association (A-18).
 *
 * Abstraction:
 *   - SOCKS: application UDP sockets (by cookie; a cookie is never reused, so a closed socket never
 *     reopens). DESTS: original destinations. SLOTS: the guest slot pool (1,024 in the design).
 *   - A slot is owned by ONE (socket, destination). Frames waiting in the slot's framing cell are a
 *     set of (socket, destination) tags (`parked`); a frame is "delivered" when the cell is routed to
 *     the association's Q_g and the host association forwards it to ITS destination.
 *   - `fid`: the association's flow id (guest origin, allocated monotonically, never reused).
 *     host: the host's view of association `fid` (req → paired | closed) with ghost (sock, dest)
 *     and the slot index it was opened from.
 *   - Control messages are per-flow-id sets (order between different flow ids does not matter;
 *     each flow id gets at most one Paired/Refused).
 *   - Time is abstract: GUEST_DATAGRAM_SLOT_IDLE (120 s) is a nondeterministic IdleRelease of a slot
 *     with nothing parked.
 *
 * ASSUMPTIONS:
 *   K-B1  Closing T_g / Q_g is observed by the host as the association's end, even when the host
 *         accepts them after the guest closed them ......................... P-22 / V-2 (bounded)
 *   K-B2  The guest `sock_release` hook reports a released socket, or the report is lost and the
 *         level-triggered audit (sock_diag) finds slots whose cookie no longer exists ....... R5-14
 *   K-B3  Emptying / recreating a slot's framing and reassembly cells discards every parked frame
 *         (no frame survives in a psock backlog)................................... NO V-item (R5-14 Tier-3 partially)
 *
 * BUG_* constants each switch ONE design rule off (teeth). The design model sets all to false.
 */
module udp_slots {
  const SOCKS: Set[int]
  const DESTS: Set[int]
  const SLOTS: Set[int]
  const MAXFID: int

  const BUG_CLAIM_SPLIT: bool          // slot claim: "is s free?" and "take s" are two steps
  const BUG_KEEP_FRAMES: bool          // P-35: release returns the slot with frames still parked
  const BUG_LATE_PAIRED_BY_SLOT: bool  // late Paired matched to the slot it was opened from, not by fid
  const BUG_IGNORE_LATE_PAIRED: bool   // late Paired silently dropped instead of answered with Abort
  const BUG_RELEASE_ONE_SLOT: bool     // closing a socket releases only one of its slots (no check: the audit masks it)
  const ENV_REPORT_LOSS: bool          // sock_release reports may be lost (audit is the backstop)
  const BUG_KEEP_FRAMES_ON_END: bool   // U-2 off: Refused / Abort leaves the parked frames in the cell

  type Tag = (int, int)                // (socket, destination) of a frame
  type Slot = { owned: bool, sock: int, dest: int, fid: int, routed: bool, parked: Set[Tag], holders: int }
  type Assoc = { st: str, sock: int, dest: int, slot: int }

  var sockAlive: int -> bool
  var releasePending: int -> bool       // a sock_release report is queued for overdrive-init
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
  var lateIgnored: bool                 // ghost: a late Paired was neither routed nor answered with Abort
  var endedNoNew: Set[int]              // ghost: slots whose association ended by Refused / Abort with
                                        //        no datagram from the application since
  var staleReassoc: bool                // ghost: such a slot re-associated without a new datagram
  var guestEnded: Set[int]              // association ids the guest ended itself (pairing deadline)

  val FIDS: Set[int] = 1.to(MAXFID)
  val freeSlot: Slot = { owned: false, sock: 0, dest: 0, fid: 0, routed: false, parked: Set(), holders: 0 }
  def ownerOf(k: int, d: int): Set[int] = SLOTS.filter(s => slots.get(s).owned and slots.get(s).sock == k and slots.get(s).dest == d)
  def mismatch(tags: Set[Tag], a: Assoc): bool = tags.exists(t => t != (a.sock, a.dest))
  // release (correct): end the association (host observes it), discard parked frames, return the slot
  def released(x: Slot): Slot = if (BUG_KEEP_FRAMES) { ...freeSlot, parked: x.parked } else freeSlot
  def endOf(ss: Set[int]): Set[int] = ss.filter(s => slots.get(s).fid != 0).map(s => slots.get(s).fid)

  action init = all {
    sockAlive' = SOCKS.mapBy(k => true),
    releasePending' = SOCKS.mapBy(k => false),
    slots' = SLOTS.mapBy(s => freeSlot),
    picks' = Set(),
    nextFid' = 1,
    host' = FIDS.mapBy(i => { st: "none", sock: 0, dest: 0, slot: 0 }),
    toGuestPaired' = Set(), toGuestRefused' = Set(), toGuestAbort' = Set(),
    toHostAbort' = Set(), toHostEnd' = Set(),
    misdelivered' = false, lateIgnored' = false, endedNoNew' = Set(), staleReassoc' = false, guestEnded' = Set(),
  }

  // ------------------------------------------------------------------ application sends
  // A datagram to (k, d) on the slot that already owns (k, d): delivered if routed, else parked.
  action AppSendOnSlot(k: int, d: int, s: int): bool = all {
    sockAlive.get(k), slots.get(s).owned, slots.get(s).sock == k, slots.get(s).dest == d,
    val x = slots.get(s)
    if (x.routed) all {
      misdelivered' = (misdelivered or (host.get(x.fid).st == "paired" and mismatch(Set((k, d)), host.get(x.fid)))),
      slots' = slots,
    } else all {
      misdelivered' = misdelivered,
      slots' = slots.set(s, { ...x, parked: x.parked.union(Set((k, d))) }),
    },
    sockAlive' = sockAlive, releasePending' = releasePending, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    lateIgnored' = lateIgnored, endedNoNew' = endedNoNew.exclude(Set(s)), staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  // First datagram to a new (k, d): claim a free slot — one atomic step (correct).
  action AppClaim(k: int, d: int, s: int): bool = all {
    not(BUG_CLAIM_SPLIT), sockAlive.get(k), ownerOf(k, d) == Set(), not(slots.get(s).owned),
    slots' = slots.set(s, { owned: true, sock: k, dest: d, fid: 0, routed: false,
                           parked: slots.get(s).parked.union(Set((k, d))), holders: 1 }),
    sockAlive' = sockAlive, releasePending' = releasePending, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    misdelivered' = misdelivered, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew.exclude(Set(s)), staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  action AppPick(k: int, d: int, s: int): bool = all {
    BUG_CLAIM_SPLIT, sockAlive.get(k), ownerOf(k, d) == Set(), not(slots.get(s).owned),
    not(picks.exists(p => p.k == k and p.d == d)),
    picks' = picks.union(Set({ k: k, d: d, s: s })),
    slots' = slots, sockAlive' = sockAlive, releasePending' = releasePending, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    misdelivered' = misdelivered, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  action AppTake(k: int, d: int, s: int): bool = all {
    BUG_CLAIM_SPLIT, picks.contains({ k: k, d: d, s: s }),
    picks' = picks.exclude(Set({ k: k, d: d, s: s })),
    slots' = slots.set(s, { ...slots.get(s), owned: true, sock: k, dest: d,
                           parked: slots.get(s).parked.union(Set((k, d))),
                           holders: slots.get(s).holders + 1 }),
    sockAlive' = sockAlive, releasePending' = releasePending, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    misdelivered' = misdelivered, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew.exclude(Set(s)), staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }

  // ------------------------------------------------------------------ association set-up
  // Datagram step 2: on framing-cell readiness, claim a FlowId, open T_g + Q_g, write both requests.
  action GuestAssociate(s: int): bool = all {
    val x = slots.get(s)
    all {
      x.owned, x.parked != Set(), x.fid == 0, not(x.routed), nextFid <= MAXFID,
      slots' = slots.set(s, { ...x, fid: nextFid }),
      host' = host.set(nextFid, { st: "req", sock: x.sock, dest: x.dest, slot: s }),
    },
    nextFid' = nextFid + 1,
    sockAlive' = sockAlive, releasePending' = releasePending, picks' = picks,
    toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    misdelivered' = misdelivered, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew.exclude(Set(s)), staleReassoc' = (staleReassoc or endedNoNew.contains(s)), guestEnded' = guestEnded,
  }
  // Datagram steps 3-4 (host): pair, or refuse (Policy / HostInternal / Capacity / ...).
  action HostPair(i: int): bool = all {
    host.get(i).st == "req",
    host' = host.set(i, { ...host.get(i), st: "paired" }),
    toGuestPaired' = toGuestPaired.union(Set(i)),
    sockAlive' = sockAlive, releasePending' = releasePending, slots' = slots, picks' = picks,
    nextFid' = nextFid, toGuestRefused' = toGuestRefused, toGuestAbort' = toGuestAbort,
    toHostAbort' = toHostAbort, toHostEnd' = toHostEnd, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  action HostRefuse(i: int): bool = all {
    host.get(i).st == "req",
    host' = host.set(i, { ...host.get(i), st: "closed" }),
    toGuestRefused' = toGuestRefused.union(Set(i)),
    sockAlive' = sockAlive, releasePending' = releasePending, slots' = slots, picks' = picks,
    nextFid' = nextFid, toGuestPaired' = toGuestPaired, toGuestAbort' = toGuestAbort,
    toHostAbort' = toHostAbort, toHostEnd' = toHostEnd, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  // The host aborts a live association (error on H, quiescence, control-session loss, ...).
  action HostAbortAssoc(i: int): bool = all {
    host.get(i).st == "paired",
    host' = host.set(i, { ...host.get(i), st: "closed" }),
    toGuestAbort' = toGuestAbort.union(Set(i)),
    sockAlive' = sockAlive, releasePending' = releasePending, slots' = slots, picks' = picks,
    nextFid' = nextFid, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toHostAbort' = toHostAbort, toHostEnd' = toHostEnd, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }

  // Datagram step 5 / SLOT-ABORT: Paired for i. Matched by flow id against a live association.
  action GuestRecvPaired(i: int): bool = all {
    toGuestPaired.contains(i),
    toGuestPaired' = toGuestPaired.exclude(Set(i)),
    val live = SLOTS.filter(s => slots.get(s).owned and slots.get(s).fid == i)
    val orig = host.get(i).slot
    if (live != Set()) all {
      // route the framing cell -> Q_g; parked frames flow to the host association's destination
      slots' = slots.keys().mapBy(s => if (live.contains(s)) { ...slots.get(s), routed: true, parked: Set() } else slots.get(s)),
      misdelivered' = (misdelivered or live.exists(s => mismatch(slots.get(s).parked, host.get(i)))),
      toHostAbort' = toHostAbort, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
    } else if (BUG_LATE_PAIRED_BY_SLOT and slots.get(orig).owned) all {
      slots' = slots.set(orig, { ...slots.get(orig), fid: i, routed: true, parked: Set() }),
      misdelivered' = (misdelivered or mismatch(slots.get(orig).parked, host.get(i))),
      toHostAbort' = toHostAbort, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
    } else if (BUG_IGNORE_LATE_PAIRED) all {
      slots' = slots, misdelivered' = misdelivered, toHostAbort' = toHostAbort, lateIgnored' = true, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
    } else if (guestEnded.contains(i)) all {
      // the guest ended this association itself (pairing deadline) and sent Abort: discard (U-5)
      slots' = slots, misdelivered' = misdelivered, toHostAbort' = toHostAbort,
      lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
    } else all {
      // the slot was released (association ended): answer with Abort, route nothing (SLOT-ABORT)
      slots' = slots, misdelivered' = misdelivered, toHostAbort' = toHostAbort.union(Set(i)),
      lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
    },
    sockAlive' = sockAlive, releasePending' = releasePending, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestRefused' = toGuestRefused, toGuestAbort' = toGuestAbort,
    toHostEnd' = toHostEnd,
  }
  // Refused / Abort received: the association closes on the guest side; the slot stays owned by its
  // (socket, destination); its parked frames are discarded (U-2) and only the next datagram
  // re-associates (A-18).
  action GuestRecvEnd(i: int): bool = all {
    toGuestRefused.contains(i) or toGuestAbort.contains(i),
    toGuestRefused' = toGuestRefused.exclude(Set(i)),
    toGuestAbort' = toGuestAbort.exclude(Set(i)),
    val ended = SLOTS.filter(s => slots.get(s).owned and slots.get(s).fid == i)
    all {
      slots' = slots.keys().mapBy(s =>
        if (ended.contains(s)) { ...slots.get(s), fid: 0, routed: false,
                                 parked: if (BUG_KEEP_FRAMES_ON_END) slots.get(s).parked else Set() }
        else slots.get(s)),
      endedNoNew' = endedNoNew.union(ended),
    },
    sockAlive' = sockAlive, releasePending' = releasePending, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    misdelivered' = misdelivered, lateIgnored' = lateIgnored, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  // FLOW_PAIRING_DEADLINE: the guest opener aborts an unpaired association and sends Abort; the
  // slot stays owned, its parked frames are discarded, it does not re-associate for them (U-2)
  action GuestPairingTimeout(s: int): bool = all {
    val x = slots.get(s)
    all {
      x.owned, x.fid != 0, not(x.routed),
      slots' = slots.set(s, { ...x, fid: 0, parked: if (BUG_KEEP_FRAMES_ON_END) x.parked else Set() }),
      toHostAbort' = toHostAbort.union(Set(x.fid)),
      guestEnded' = guestEnded.union(Set(x.fid)),
      endedNoNew' = endedNoNew.union(Set(s)),
    },
    sockAlive' = sockAlive, releasePending' = releasePending, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostEnd' = toHostEnd, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, staleReassoc' = staleReassoc,
  }
  // Host receives Abort, or observes T_h / Q_h EOF (K-B1): the association closes.
  action HostRecvEnd(i: int): bool = all {
    toHostAbort.contains(i) or toHostEnd.contains(i),
    toHostAbort' = toHostAbort.exclude(Set(i)),
    toHostEnd' = toHostEnd.exclude(Set(i)),
    host' = host.set(i, { ...host.get(i), st: if (host.get(i).st == "none") "none" else "closed" }),
    sockAlive' = sockAlive, releasePending' = releasePending, slots' = slots, picks' = picks,
    nextFid' = nextFid, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, misdelivered' = misdelivered, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }

  // ------------------------------------------------------------------ slot release (D25)
  action SockClose(k: int): bool = all {
    sockAlive.get(k),
    sockAlive' = sockAlive.set(k, false),
    releasePending' = releasePending.set(k, true),
    slots' = slots, picks' = picks.filter(p => p.k != k), nextFid' = nextFid, host' = host,
    toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused, toGuestAbort' = toGuestAbort,
    toHostAbort' = toHostAbort, toHostEnd' = toHostEnd, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  // the sock_release report is lost (K-B2): only the audit can find the slots
  action SockCloseReportLost(k: int): bool = all {
    ENV_REPORT_LOSS, sockAlive.get(k),
    sockAlive' = sockAlive.set(k, false),
    releasePending' = releasePending,
    slots' = slots, picks' = picks.filter(p => p.k != k), nextFid' = nextFid, host' = host,
    toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused, toGuestAbort' = toGuestAbort,
    toHostAbort' = toHostAbort, toHostEnd' = toHostEnd, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  action releaseSet(ss: Set[int]): bool = all {
    slots' = slots.keys().mapBy(s => if (ss.contains(s)) released(slots.get(s)) else slots.get(s)),
    toHostEnd' = toHostEnd.union(endOf(ss)),
    endedNoNew' = endedNoNew.exclude(ss),
  }
  // overdrive-init handles the report: every slot of the released socket (slot-ownership table)
  action GuestReleaseSock(k: int): bool = all {
    releasePending.get(k),
    releasePending' = releasePending.set(k, false),
    val mine = SLOTS.filter(s => slots.get(s).owned and slots.get(s).sock == k)
    val one = if (mine == Set()) Set() else Set(mine.fold(1000000, (a, b) => if (a < b) a else b))
    releaseSet(if (BUG_RELEASE_ONE_SLOT) one else mine),
    sockAlive' = sockAlive, picks' = picks, nextFid' = nextFid, host' = host,
    toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused, toGuestAbort' = toGuestAbort,
    toHostAbort' = toHostAbort, misdelivered' = misdelivered, lateIgnored' = lateIgnored, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  // level-triggered audit: slots whose socket cookie no longer exists
  action GuestAudit: bool = all {
    val orphans = SLOTS.filter(s => slots.get(s).owned and not(sockAlive.get(slots.get(s).sock)))
    all { orphans != Set(), releaseSet(orphans) },
    sockAlive' = sockAlive, releasePending' = releasePending, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  // GUEST_DATAGRAM_SLOT_IDLE elapsed with no datagram either way (nothing parked)
  action IdleRelease(s: int): bool = all {
    slots.get(s).owned, slots.get(s).parked == Set(),
    releaseSet(Set(s)),
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
      GuestAssociate(s), HostPair(i), HostRefuse(i), HostAbortAssoc(i), GuestRecvPaired(i),
      GuestRecvEnd(i), GuestPairingTimeout(s), HostRecvEnd(i), SockClose(k), SockCloseReportLost(k), GuestReleaseSock(k),
      GuestAudit, IdleRelease(s),
    }
  }

  // ------------------------------------------------------------------ invariants
  // (2a) A slot is held by at most one (socket, destination); one slot per (socket, destination).
  val SlotHeldOnce: bool = and {
    SLOTS.forall(s => slots.get(s).holders <= 1),
    SLOTS.forall(a => SLOTS.forall(b => (a != b and slots.get(a).owned and slots.get(b).owned) implies
      (slots.get(a).sock, slots.get(a).dest) != (slots.get(b).sock, slots.get(b).dest))),
  }
  // (2b) A released slot never carries data; an owned slot carries only its owner's frames; no frame
  //      ever reaches a host association of another (socket, destination).
  val ReleasedSlotCarriesNoData: bool = and {
    SLOTS.forall(s => not(slots.get(s).owned) implies
      (slots.get(s).parked == Set() and not(slots.get(s).routed) and slots.get(s).fid == 0)),
    SLOTS.forall(s => slots.get(s).owned implies
      slots.get(s).parked.forall(t => t == (slots.get(s).sock, slots.get(s).dest))),
    not(misdelivered),
  }
  // a routed slot routes to its own current association
  val RouteCurrent: bool = SLOTS.forall(s => slots.get(s).routed implies and {
    slots.get(s).owned, slots.get(s).fid != 0,
    host.get(slots.get(s).fid).sock == slots.get(s).sock,
    host.get(slots.get(s).fid).dest == slots.get(s).dest,
  })
  // (2c) A late Paired for a released slot is answered with Abort (never ignored, never routed).
  val LatePairedAborted: bool = not(lateIgnored)
  // U-2: after Refused / Abort a slot re-associates only for a new datagram from its application
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

  // ------------------------------------------------------------------ non-vacuity witnesses
  val WitnessNoLatePaired: bool = toHostAbort == Set()
  val WitnessNoRouted: bool = SLOTS.forall(s => not(slots.get(s).routed))
  val WitnessNoTwoSlotsOneSocket: bool = not(SOCKS.exists(k =>
    SLOTS.filter(s => slots.get(s).owned and slots.get(s).sock == k).size() >= 2))
  val WitnessNoReassociation: bool = not(SLOTS.exists(s => slots.get(s).owned and slots.get(s).fid >= 2 and
    host.get(slots.get(s).fid - 1).sock == slots.get(s).sock and host.get(slots.get(s).fid - 1).dest == slots.get(s).dest))
}


// ------------------------------------------------------------------ design instances
// the design: 2 sockets, 2 destinations, 2 slots, 3 association ids, lost release reports
module slots_ok {
  import udp_slots(SOCKS = Set(1, 2), DESTS = Set(1, 2), SLOTS = Set(0, 1), MAXFID = 3,
    BUG_CLAIM_SPLIT = false, BUG_KEEP_FRAMES = false, BUG_LATE_PAIRED_BY_SLOT = false,
    BUG_IGNORE_LATE_PAIRED = false, BUG_RELEASE_ONE_SLOT = false, ENV_REPORT_LOSS = true, BUG_KEEP_FRAMES_ON_END = false).* from "./udp_slots"
}
// the design, one destination: small enough for TLC (exhaustive safety and liveness)
module slots_live {
  import udp_slots(SOCKS = Set(1, 2), DESTS = Set(1), SLOTS = Set(0, 1), MAXFID = 2,
    BUG_CLAIM_SPLIT = false, BUG_KEEP_FRAMES = false, BUG_LATE_PAIRED_BY_SLOT = false,
    BUG_IGNORE_LATE_PAIRED = false, BUG_RELEASE_ONE_SLOT = false, ENV_REPORT_LOSS = true, BUG_KEEP_FRAMES_ON_END = false).* from "./udp_slots"
}
