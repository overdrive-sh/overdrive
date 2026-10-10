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
 * and the round-2 remediation decision of 2026-10-09 (§ Spike remediation decisions, round 2):
 *   CELL-RECREATE Every end of an association (slot release and U-2) discards the parked frames by
 *          RECREATING the slot's framing and reassembly cells (a new cell generation), never by
 *          emptying them; overdrive-init dispatches each readiness event by its registration's
 *          identity (slot index, cell generation), never by raw descriptor: an event of an older
 *          generation is discarded.
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
 *   - Time is abstract. The guest owner's audit tick (GUEST_OWNER_AUDIT_PERIOD, DG-5-A) is ONE action,
 *     GuestAudit: the release audit, then the idle release of every slot idle for GUEST_DATAGRAM_SLOT_IDLE
 *     since the tick that last saw a change (a nondeterministic subset of the owned slots with nothing
 *     parked: which have reached the idle deadline is a matter of time), then the quarantine and
 *     `listener_map` retries (not modelled). There is no separate idle-release action. Each timer action
 *     (GuestAudit, GuestPairingTimeout) fires its own deadline only, one deadline per step; the order of
 *     simultaneously due deadlines (earliest first) is the driver's clock and is not modelled.
 *   - MAXFID only disables minting a new association id (DG-5-B); see its definition.
 *   - Cells (CELL-RECREATE): each slot carries a cell generation, recreated (advanced) at every end
 *     of association; every datagram parked in a cell queues a readiness event registered for the
 *     slot's generation then current, in `ready`. Generations only grow and dispatch compares an
 *     event's generation only for equality with its slot's current one, so an event is kept as
 *     (slot, current?) — "earlier generation" is `false` — which keeps the instance finite for TLC.
 *     Association set-up (Datagram step 2) is triggered by consuming one event and trusts it (a
 *     readiness event means frames). The
 *     step order of an end of association (cut the reassembly output → close the legs → recreate →
 *     disconnect → remove the unframe tuple) concerns framing, which the model does not carry
 *     (R5-14, Tier-3). The quarantine of a slot whose new cell cannot be created is not modelled.
 *
 * ASSUMPTIONS:
 *   K-B1  Closing T_g / Q_g is observed by the host as the association's end, even when the host
 *         accepts them after the guest closed them ......................... P-22 / V-2 (bounded)
 *   K-B2  The guest `sock_release` hook reports a released socket, or the report is lost and the
 *         level-triggered audit (sock_diag) finds slots whose cookie no longer exists ....... R5-14
 *   K-B3  Recreating a slot's framing and reassembly cells discards every parked frame; EMPTYING
 *         them does not (sender-side queue / psock backlog residue refills the cell and re-triggers
 *         readiness: OFF "cellsEmptiedOnly") ................ V-5(c) (P-41: recreated validated, emptied falsified)
 *
 * OFF names the rules switched off (teeth); the design instances switch none off.
 */
module udp_slots {
  // OFF: design rules switched off (hazard instances in hazard/udp_slots_hazards.qnt); CFG: "ok" (two
  // destinations, three association ids), "live" (one destination: small enough for TLC) or "conf" (as "ok"
  // with MAXFID above any driver trace: the conformance instance, DG-5-B)
  const OFF: Set[str]
  const CFG: str

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
  val BUG_RELEASE_ONE_SLOT: bool = OFF.contains("releaseOneSlot")       // closing a socket releases only one of its slots (no check: the audit masks it)
  val ENV_REPORT_LOSS: bool = true                                       // sock_release reports may be lost (audit is the backstop)
  val BUG_KEEP_FRAMES_ON_END: bool = OFF.contains("keepFramesOnEnd")    // U-2 off: Refused / Abort leaves the parked frames in the cell
  val BUG_CELLS_EMPTIED_ONLY: bool = OFF.contains("cellsEmptiedOnly")   // CELL-RECREATE off: the cells are emptied, not recreated; the
                                                                         // residue refills them and re-triggers readiness (K-B3 as falsified)
  val BUG_FD_DISPATCH: bool = OFF.contains("fdDispatch")                // CELL-RECREATE off: readiness dispatched by slot (raw descriptor)
                                                                         // only, ignoring the generation

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
  var ready: Set[(int, bool)]           // readiness events not yet dispatched: (slot, registered for its CURRENT
                                        //   cell generation?) — `false`: an earlier generation (CELL-RECREATE)
  var residue: int -> Set[Tag]          // cellsEmptiedOnly only: frames left in the sender-side queue / psock backlog
  var staleGenStart: bool               // ghost: an association started from an event of an earlier generation

  val FIDS: Set[int] = 1.to(MAXFID)
  val freeSlot: Slot = { owned: false, sock: 0, dest: 0, fid: 0, routed: false, parked: Set(), holders: 0 }
  def ownerOf(k: int, d: int): Set[int] = SLOTS.filter(s => slots.get(s).owned and slots.get(s).sock == k and slots.get(s).dest == d)
  def mismatch(tags: Set[Tag], a: Assoc): bool = tags.exists(t => t != (a.sock, a.dest))
  // release (correct): end the association (host observes it), discard parked frames, return the slot
  def released(x: Slot): Slot = { ...freeSlot, parked: parkedAfterEnd(x, true) }
  def endOf(ss: Set[int]): Set[int] = ss.filter(s => slots.get(s).fid != 0).map(s => slots.get(s).fid)
  // An end of association on slots `ss` (U-2 when `rel` is false, release when true). The design
  // recreates the cells: the parked frames go, the generation advances, and every event already
  // queued for the slot is of an earlier generation. keepFramesOnEnd / keepFrames: the cells are
  // left as they are — same generation, frames still there, the cell still readable (a level-
  // triggered event of the current generation). cellsEmptiedOnly: the cells are emptied — same
  // generation — and the frames survive as residue (Refill).
  def recreates(rel: bool): bool =
    not(BUG_CELLS_EMPTIED_ONLY) and not(if (rel) BUG_KEEP_FRAMES else BUG_KEEP_FRAMES_ON_END)
  def keepsFrames(rel: bool): bool = if (rel) BUG_KEEP_FRAMES else BUG_KEEP_FRAMES_ON_END
  def residueAfterEnd(ss: Set[int]): int -> Set[Tag] =
    residue.keys().mapBy(s => if (ss.contains(s) and BUG_CELLS_EMPTIED_ONLY) residue.get(s).union(slots.get(s).parked) else residue.get(s))
  def readyAfterEnd(ss: Set[int], rel: bool): Set[(int, bool)] =
    // recreating advances the slot's generation: every event already queued for it is now of an
    // earlier generation
    val aged = if (recreates(rel)) ready.map(e => if (ss.contains(e._1)) (e._1, false) else e) else ready
    if (keepsFrames(rel)) aged.union(ss.filter(s => slots.get(s).parked != Set()).map(s => (s, true))) else aged
  // the cell's frames when it is not recreated: kept, or emptied (cellsEmptiedOnly: into residue)
  def parkedAfterEnd(x: Slot, rel: bool): Set[Tag] = if (keepsFrames(rel)) x.parked else Set()

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
    ready' = Set(), residue' = SLOTS.mapBy(s => Set()), staleGenStart' = false,
  }
  action keepCells = all { ready' = ready, residue' = residue, staleGenStart' = staleGenStart }

  // ------------------------------------------------------------------ application sends
  // A datagram to (k, d) on the slot that already owns (k, d): delivered if routed, else parked.
  action AppSendOnSlot(k: int, d: int, s: int): bool = all {
    sockAlive.get(k), slots.get(s).owned, slots.get(s).sock == k, slots.get(s).dest == d,
    val x = slots.get(s)
    if (x.routed) all {
      misdelivered' = (misdelivered or (host.get(x.fid).st == "paired" and mismatch(Set((k, d)), host.get(x.fid)))),
      slots' = slots, ready' = ready,
    } else all {
      misdelivered' = misdelivered,
      slots' = slots.set(s, { ...x, parked: x.parked.union(Set((k, d))) }),
      ready' = ready.union(Set((s, true))),
    },
    residue' = residue, staleGenStart' = staleGenStart,
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
    ready' = ready.union(Set((s, true))), residue' = residue, staleGenStart' = staleGenStart,
    sockAlive' = sockAlive, releasePending' = releasePending, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    misdelivered' = misdelivered, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew.exclude(Set(s)), staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  action AppPick(k: int, d: int, s: int): bool = all {
    BUG_CLAIM_SPLIT, sockAlive.get(k), ownerOf(k, d) == Set(), not(slots.get(s).owned),
    not(picks.exists(p => p.k == k and p.d == d)),
    picks' = picks.union(Set({ k: k, d: d, s: s })), keepCells,
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
    ready' = ready.union(Set((s, true))), residue' = residue, staleGenStart' = staleGenStart,
    sockAlive' = sockAlive, releasePending' = releasePending, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    misdelivered' = misdelivered, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew.exclude(Set(s)), staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }

  // ------------------------------------------------------------------ association set-up
  // Datagram step 2: a readiness event of slot s is dispatched. The design dispatches by (slot, cell
  // generation): an event of an earlier generation is discarded. A current one, for an owned slot
  // with no association, claims a FlowId, opens T_g + Q_g and writes both requests (the event is
  // trusted: readiness means frames). fdDispatch: dispatched by slot only, whatever its generation.
  action GuestReady(s: int, cur: bool): bool = all {
    ready.contains((s, cur)),
    val x = slots.get(s)
    val stale = not(cur)
    if (stale and not(BUG_FD_DISPATCH)) all {
      // an event of an earlier cell generation: discarded
      slots' = slots, host' = host, nextFid' = nextFid, staleReassoc' = staleReassoc,
      endedNoNew' = endedNoNew, staleGenStart' = staleGenStart,
    } else if (x.owned and x.fid == 0 and not(x.routed) and nextFid <= MAXFID) all {
      slots' = slots.set(s, { ...x, fid: nextFid }),
      host' = host.set(nextFid, { st: "req", sock: x.sock, dest: x.dest, slot: s }),
      nextFid' = nextFid + 1,
      staleReassoc' = (staleReassoc or endedNoNew.contains(s)),
      endedNoNew' = endedNoNew.exclude(Set(s)),
      staleGenStart' = (staleGenStart or stale),
    } else all {
      // nothing to set up (an association is already pending or routed; the slot is free)
      slots' = slots, host' = host, nextFid' = nextFid, staleReassoc' = staleReassoc,
      endedNoNew' = endedNoNew, staleGenStart' = staleGenStart,
    },
    ready' = ready.exclude(Set((s, cur))), residue' = residue,
    sockAlive' = sockAlive, releasePending' = releasePending, picks' = picks,
    toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    misdelivered' = misdelivered, lateIgnored' = lateIgnored, guestEnded' = guestEnded,
  }
  // cellsEmptiedOnly: the residue refills the emptied cell and re-triggers readiness (P-41)
  action Refill(s: int): bool = all {
    residue.get(s) != Set(),
    slots' = slots.set(s, { ...slots.get(s), parked: slots.get(s).parked.union(residue.get(s)) }),
    residue' = residue.set(s, Set()),
    ready' = ready.union(Set((s, true))),
    staleGenStart' = staleGenStart,
    sockAlive' = sockAlive, releasePending' = releasePending, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, toHostEnd' = toHostEnd,
    misdelivered' = misdelivered, lateIgnored' = lateIgnored, endedNoNew' = endedNoNew,
    staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  // Datagram steps 3-4 (host): pair, or refuse (Policy / HostInternal / Capacity / ...).
  action HostPair(i: int): bool = all {
    host.get(i).st == "req",
    host' = host.set(i, { ...host.get(i), st: "paired" }),
    toGuestPaired' = toGuestPaired.union(Set(i)),
    keepCells, sockAlive' = sockAlive, releasePending' = releasePending, slots' = slots, picks' = picks,
    nextFid' = nextFid, toGuestRefused' = toGuestRefused, toGuestAbort' = toGuestAbort,
    toHostAbort' = toHostAbort, toHostEnd' = toHostEnd, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  action HostRefuse(i: int): bool = all {
    host.get(i).st == "req",
    host' = host.set(i, { ...host.get(i), st: "closed" }),
    toGuestRefused' = toGuestRefused.union(Set(i)),
    keepCells, sockAlive' = sockAlive, releasePending' = releasePending, slots' = slots, picks' = picks,
    nextFid' = nextFid, toGuestPaired' = toGuestPaired, toGuestAbort' = toGuestAbort,
    toHostAbort' = toHostAbort, toHostEnd' = toHostEnd, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, endedNoNew' = endedNoNew, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }
  // The host aborts a live association (error on H, quiescence, control-session loss, ...).
  action HostAbortAssoc(i: int): bool = all {
    host.get(i).st == "paired",
    host' = host.set(i, { ...host.get(i), st: "closed" }),
    toGuestAbort' = toGuestAbort.union(Set(i)),
    keepCells, sockAlive' = sockAlive, releasePending' = releasePending, slots' = slots, picks' = picks,
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
    toHostEnd' = toHostEnd, keepCells,
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
        if (ended.contains(s)) { ...slots.get(s), fid: 0, routed: false, parked: parkedAfterEnd(slots.get(s), false) }
        else slots.get(s)),
      endedNoNew' = endedNoNew.union(ended),
      residue' = residueAfterEnd(ended), ready' = readyAfterEnd(ended, false),
    },
    staleGenStart' = staleGenStart,
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
      slots' = slots.set(s, { ...x, fid: 0, parked: parkedAfterEnd(x, false) }),
      residue' = residueAfterEnd(Set(s)), ready' = readyAfterEnd(Set(s), false),
      staleGenStart' = staleGenStart,
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
    keepCells,
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
    keepCells,
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
    keepCells,
  }
  action releaseSet(ss: Set[int]): bool = all {
    slots' = slots.keys().mapBy(s => if (ss.contains(s)) released(slots.get(s)) else slots.get(s)),
    toHostEnd' = toHostEnd.union(endOf(ss)),
    endedNoNew' = endedNoNew.exclude(ss),
    residue' = residueAfterEnd(ss), ready' = readyAfterEnd(ss, true),
    staleGenStart' = staleGenStart,
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
  action GuestAuditIdle(idle: Set[int]): bool = all {
    val orphans = SLOTS.filter(s => slots.get(s).owned and not(sockAlive.get(slots.get(s).sock)))
    all {
      idle.forall(s => slots.get(s).owned and slots.get(s).parked == Set() and not(orphans.contains(s))),
      releaseSet(orphans.union(idle)),
    },
    sockAlive' = sockAlive, releasePending' = releasePending, picks' = picks, nextFid' = nextFid,
    host' = host, toGuestPaired' = toGuestPaired, toGuestRefused' = toGuestRefused,
    toGuestAbort' = toGuestAbort, toHostAbort' = toHostAbort, misdelivered' = misdelivered,
    lateIgnored' = lateIgnored, staleReassoc' = staleReassoc, guestEnded' = guestEnded,
  }

  action releaseAny = { nondet k = SOCKS.oneOf() GuestReleaseSock(k) }
  // overdrive-init's event loop takes one queued readiness event
  action dispatchReady = { nondet e = ready.oneOf() GuestReady(e._1, e._2) }
  action step = {
    nondet k = SOCKS.oneOf()
    nondet d = DESTS.oneOf()
    nondet s = SLOTS.oneOf()
    nondet i = FIDS.oneOf()
    any {
      AppSendOnSlot(k, d, s), AppClaim(k, d, s), AppPick(k, d, s), AppTake(k, d, s),
      dispatchReady, Refill(s), HostPair(i), HostRefuse(i), HostAbortAssoc(i), GuestRecvPaired(i),
      GuestRecvEnd(i), GuestPairingTimeout(s), HostRecvEnd(i), SockClose(k), SockCloseReportLost(k), GuestReleaseSock(k),
      GuestAudit,
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
  // (14, CELL-RECREATE) no association starts from a readiness event of an earlier cell generation
  val NoStaleGenerationStart: bool = not(staleGenStart)
  val Safety: bool = SlotHeldOnce and ReleasedSlotCarriesNoData and RouteCurrent and LatePairedAborted and
    NoReassociationWithoutNewDatagram and NoStaleGenerationStart

  // ------------------------------------------------------------------ liveness (TLC)
  def orphan(s: int): bool = slots.get(s).owned and not(sockAlive.get(slots.get(s).sock))
  val anyOrphan: bool = SLOTS.exists(s => orphan(s))
  // A slot held by a released socket eventually returns to the pool (becomes claimable).
  temporal ReleasedSlotBecomesFree =
    and { weakFair(releaseAny, slots), weakFair(GuestAudit, slots) }
      .implies(always(anyOrphan.implies(eventually(not(anyOrphan)))))

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // Deterministic scenarios on `slots_ok` (readiness events are dispatched with `GuestReady(slot,
  // current?)` rather than the nondeterministic `dispatchReady`), each ending in an `expect` of its
  // outcome with `Safety` checked along the way.
  // Datagram happy path: the first datagram claims a slot and parks; its readiness event opens the
  // association; the host pairs; Paired routes the parked frame to the association; later datagrams
  // are delivered directly
  run DatagramAssociationClaimsPairsAndRoutes = init
    .then(AppClaim(1, 1, 0)).expect(Safety and slots.get(0).owned and slots.get(0).parked == Set((1, 1)) and ready == Set((0, true)))
    .then(GuestReady(0, true)).expect(Safety and slots.get(0).fid == 1 and host.get(1).st == "req" and ready == Set())
    .then(HostPair(1)).then(GuestRecvPaired(1))
    .expect(Safety and slots.get(0).routed and slots.get(0).parked == Set() and host.get(1).st == "paired")
    .then(AppSendOnSlot(1, 1, 0)).expect(Safety and not(misdelivered) and slots.get(0).routed)
  // two sockets claim distinct slots; each slot has exactly one owner
  run TwoSocketsClaimDistinctSlots = init
    .then(AppClaim(1, 1, 0)).then(AppClaim(2, 1, 1))
    .expect(Safety and ownerOf(1, 1) == Set(0) and ownerOf(2, 1) == Set(1) and slots.get(0).holders == 1 and slots.get(1).holders == 1)
  // U-2 / A-18: Refused discards the parked frames (the cells are recreated), the slot stays owned and
  // does not re-associate; only a new datagram from the application starts a new association
  run RefusedDiscardsParkedFramesAndOnlyNewDatagramReassociates = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0, true)).then(HostRefuse(1)).then(GuestRecvEnd(1))
    .expect(Safety and slots.get(0).owned and slots.get(0).fid == 0 and slots.get(0).parked == Set() and endedNoNew == Set(0))
    .then(AppSendOnSlot(1, 1, 0)).expect(Safety and slots.get(0).parked == Set((1, 1)) and endedNoNew == Set())
    .then(GuestReady(0, true)).expect(Safety and slots.get(0).fid == 2 and host.get(2).st == "req" and not(staleReassoc))
  // a host abort of a live association: Abort received, the slot stays owned and unrouted
  run HostAbortOfLiveAssociationEndsItAndKeepsSlotOwned = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0, true)).then(HostPair(1)).then(GuestRecvPaired(1))
    .then(HostAbortAssoc(1)).then(GuestRecvEnd(1))
    .expect(Safety and slots.get(0).owned and not(slots.get(0).routed) and slots.get(0).fid == 0 and host.get(1).st == "closed")
    .then(AppSendOnSlot(1, 1, 0)).then(GuestReady(0, true)).expect(Safety and slots.get(0).fid == 2 and not(staleReassoc))
  // CELL-RECREATE: an event queued before the association ended is of an earlier generation and is
  // discarded: no association starts from it
  run StaleGenerationReadinessEventIsDiscarded = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0, true)).then(AppSendOnSlot(1, 1, 0))
    .expect(Safety and ready == Set((0, true)))
    .then(HostRefuse(1)).then(GuestRecvEnd(1)).expect(Safety and ready == Set((0, false)) and slots.get(0).parked == Set())
    .then(GuestReady(0, false)).expect(Safety and ready == Set() and slots.get(0).fid == 0 and not(staleGenStart) and not(staleReassoc))
  // SLOT-ABORT: the socket closes and its slot is released before the host's Paired arrives; the
  // late Paired is answered with Abort and routed nowhere
  run LatePairedForReleasedSlotIsAnsweredWithAbort = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0, true)).then(SockClose(1)).then(GuestReleaseSock(1))
    .expect(Safety and not(slots.get(0).owned) and toHostEnd == Set(1))
    .then(HostPair(1)).then(GuestRecvPaired(1))
    .expect(Safety and toHostAbort == Set(1) and not(lateIgnored) and not(slots.get(0).routed))
    .then(HostRecvEnd(1)).expect(Safety and host.get(1).st == "closed" and toHostAbort == Set() and toHostEnd == Set())
  // K-B2: the sock_release report is lost; the level-triggered audit finds the slot whose cookie no
  // longer exists and releases it
  run LostReleaseReportIsFoundByAudit = init
    .then(AppClaim(1, 1, 0)).then(SockCloseReportLost(1))
    .expect(Safety and slots.get(0).owned and not(sockAlive.get(1)) and not(releasePending.get(1)))
    .then(GuestAudit).expect(Safety and not(slots.get(0).owned) and slots.get(0).parked == Set())
  // closing a socket releases every slot it owns
  run SocketCloseReleasesEverySlotItOwns = init
    .then(AppClaim(1, 1, 0)).then(AppClaim(1, 2, 1)).then(SockClose(1)).then(GuestReleaseSock(1))
    .expect(Safety and not(slots.get(0).owned) and not(slots.get(1).owned))
  // GUEST_DATAGRAM_SLOT_IDLE: an idle routed association's slot is released by the audit tick (no timer of
  // its own, DG-5-A) and the host observes the end
  run IdleSlotReleaseEndsAssociation = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0, true)).then(HostPair(1)).then(GuestRecvPaired(1))
    .then(GuestAuditIdle(Set(0))).expect(Safety and not(slots.get(0).owned) and not(slots.get(0).routed) and toHostEnd == Set(1))
    .then(HostRecvEnd(1)).expect(Safety and host.get(1).st == "closed")
  // DG-5-A: one audit tick runs, in order, the release audit (the slot of a closed socket whose report was
  // lost) and then the idle release (an idle routed slot): both slots are released in that one step. The
  // orphan is released by the release audit, so it is not among the idle slots chosen; a slot with frames
  // parked is not idle
  run AuditTickReleasesOrphansAndIdleSlotsInOneStep = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0, true)).then(HostPair(1)).then(GuestRecvPaired(1))
    .then(AppClaim(2, 1, 1)).expect(Safety and slots.get(1).parked == Set((2, 1)))
    .then(GuestReady(1, true)).then(HostPair(2)).then(GuestRecvPaired(2))
    .then(SockCloseReportLost(1))
    .expect(Safety and slots.get(0).routed and slots.get(1).routed and not(sockAlive.get(1)) and sockAlive.get(2))
    .then(GuestAuditIdle(Set(1)))
    .expect(Safety and not(slots.get(0).owned) and not(slots.get(1).owned) and toHostEnd == Set(1, 2))
  // FLOW_PAIRING_DEADLINE: the guest ends an unpaired association itself and sends Abort; the slot stays
  // owned with its frames discarded; the host's late Paired is discarded without a second Abort (U-5)
  run PairingDeadlineAbortsAndLatePairedIsDiscarded = init
    .then(AppClaim(1, 1, 0)).then(GuestReady(0, true)).then(GuestPairingTimeout(0))
    .expect(Safety and slots.get(0).owned and slots.get(0).fid == 0 and slots.get(0).parked == Set() and toHostAbort == Set(1) and guestEnded == Set(1))
    .then(HostPair(1)).then(GuestRecvPaired(1))
    .expect(Safety and toHostAbort == Set(1) and toGuestPaired == Set() and not(slots.get(0).routed) and not(lateIgnored))
    .then(HostRecvEnd(1)).expect(Safety and host.get(1).st == "closed")
  // P-35: a slot recycled to a new owner never carries the previous owner's frames: its frames are
  // discarded at release, the old readiness event is of an earlier generation, and the new owner's
  // association delivers only its own frames to its own destination
  run RecycledSlotNeverDeliversPreviousOwnersFrames = init
    .then(AppClaim(1, 1, 0)).then(SockClose(1)).then(GuestReleaseSock(1))
    .expect(Safety and not(slots.get(0).owned) and slots.get(0).parked == Set() and ready == Set((0, false)))
    .then(AppClaim(2, 2, 0)).expect(Safety and slots.get(0).parked == Set((2, 2)))
    .then(GuestReady(0, false)).expect(Safety and slots.get(0).fid == 0 and not(staleGenStart))
    .then(GuestReady(0, true)).then(HostPair(1)).then(GuestRecvPaired(1))
    .expect(Safety and slots.get(0).routed and host.get(1).sock == 2 and host.get(1).dest == 2 and not(misdelivered))

  // ------------------------------------------------------------------ non-vacuity witnesses
  val WitnessNoLatePaired: bool = toHostAbort == Set()
  val WitnessNoRouted: bool = SLOTS.forall(s => not(slots.get(s).routed))
  val WitnessNoTwoSlotsOneSocket: bool = not(SOCKS.exists(k =>
    SLOTS.filter(s => slots.get(s).owned and slots.get(s).sock == k).size() >= 2))
  // CELL-RECREATE: an event of an earlier cell generation is queued (and will be discarded)
  val WitnessNoStaleEvent: bool = ready.forall(e => e._2)
  val WitnessNoReassociation: bool = not(SLOTS.exists(s => slots.get(s).owned and slots.get(s).fid >= 2 and
    host.get(slots.get(s).fid - 1).sock == slots.get(s).sock and host.get(slots.get(s).fid - 1).dest == slots.get(s).dest))
}


// ------------------------------------------------------------------ design instances
// the design: 2 sockets, 2 destinations, 2 slots, 3 association ids, lost release reports
module slots_ok { import udp_slots(OFF = Set(), CFG = "ok").* from "./udp_slots" }
// the design, one destination: small enough for TLC (exhaustive safety and liveness)
module slots_live { import udp_slots(OFF = Set(), CFG = "live").* from "./udp_slots" }
// conformance (DG-5-B): as `slots_ok`, with MAXFID above any trace of a driver's step bound; the runs are
// checked on it, the exhaustive checks on `slots_ok` / `slots_live`
module slots_conf { import udp_slots(OFF = Set(), CFG = "conf").* from "./udp_slots" }
