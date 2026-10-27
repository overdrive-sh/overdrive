// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — the per-allocation flow quota counts half-closed flows, and the host
 * releases a forwarded flow's place when it reads the flow's end record (ADR-0147; ADR-0183: a slow reader is
 * backpressure; EVMOD-AB, PROPOSED 2026-10-16).
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 *
 * The rule: a half-close has no deadline — the forwarder completes it after the last byte the target accepted, and
 * a reader that stops reading (or shuts down only its read side) holds the flow open for as long as it likes. The
 * only bound on that is the per-allocation flow quota, and it bounds only if the quota counts the half-closed (not
 * yet ended) flows too: a half-closed flow holds its quota place exactly like an open flow, so a guest that withholds
 * reads holds at most its own quota — exactly what an untrusted guest can already hold with open flows. Under the
 * forwarder (EVMOD-AB) a flow's place is released when the host owner reads the flow's end record (the pair
 * completed both ways, or aborted); no host place waits for the guest's final close. No claim is made about other
 * allocations' admissions beyond the quota (node capacity may be below the sum of quotas).
 *
 * Abstraction:
 *   - ALLOCS: allocations (VMs). SLOTS: the node's flow places (node flow slots), CAP = |SLOTS|. The model does NOT
 *     assume CAP >= the sum of the quotas: here CAP (2) is below it (3 * Q), so an admission may be refused for node
 *     capacity; that refusal is not a quota refusal.
 *   - A place is free, active (a pair forwarding both ways), half (one direction half-closed, the pair still
 *     forwarding the other), or ended (the pair's end record is queued and not yet read by the owner). Admission is
 *     refused (the typed capacity outcome) when the allocation holds its quota; a refused admission changes nothing.
 *   - Admission, a half-close, the pair's completion (both directions ended cleanly), its abort (a peer abort, a
 *     socket error, the owner's abort: quiescence, teardown, control-session loss) and the owner's read of the end
 *     record are each one atomic step. A reader that never reads is the absence of Complete; no fairness is assumed
 *     for it.
 *   - M-1: the end records are queued per context in the order the pairs ended (`endq`, the places by the order of
 *     their records) and the owner reads them in that order (`take_end` returns the head of the queue): ReadEnd(s)
 *     reads the record of place s only when it is at the head.
 *
 * D-UP-1 (conformance step record): `conf_step` records the action that produced the state, as a variant of
 * `flow_quota_conf::ConfStep` whose tag is the action's name and whose value is the record of its parameters by
 * parameter name (`init`: the unit value). quint-connect reads the action and its picks from it
 * (`Config::nondet`). Model-only.
 *
 * OFF names the rules switched off (teeth; instances in hazard/):
 *   countActiveOnly   the quota counts active flows only; a half-closed or ended flow keeps its place but not its count
 *   nodeWideQuota     one quota for the node instead of one per allocation
 */
// D-UP-1: the variants of `conf_step` (a module of its own: a constructor would clash with the action of the same
// name, so the actions build their variant with the builtin `variant`)
module flow_quota_conf {
  type ConfStep =
    | init
    | Admit({ a: int })
    | Refuse({ a: int })
    | HalfClose({ s: int })
    | Complete({ s: int })
    | Abort({ s: int })
    | ReadEnd({ s: int })
}

module flow_quota {
  import flow_quota_conf.ConfStep from "./flow_quota"
  const OFF: Set[str]
  // D-UP-1: true only on the module's conformance main, where `conf_step` records the action taken; every
  // model-checking, liveness and hazard instance passes false and keeps `conf_step` at its initial value.
  const REC: bool

  val ALLOCS: Set[int] = Set(1, 2, 3)
  val Q: int = 1
  val SLOTS: Set[int] = Set(1, 2)
  val CAP: int = 2

  val BUG_COUNT_ACTIVE_ONLY: bool = OFF.contains("countActiveOnly")
  val BUG_NODE_WIDE: bool = OFF.contains("nodeWideQuota")

  // own: the allocation holding the place (0 = free); st: free | active | half | ended;
  // fwd: the place's pair is still forwarding (attached and not ended) — model-only, for the clash ghost
  type Slot = { own: int, st: str, fwd: bool }
  type Ghost = {
    refusedUnder: bool,   // an admission was refused at the quota check while the allocation was below its own quota and a place was free
    clash: bool,          // a flow was admitted onto a place whose previous pair was still forwarding
    refused: bool,        // an admission was refused at the quota (witness)
    otherAdmitted: bool,  // an allocation was admitted while another held a half-closed flow (witness)
  }

  var slot: int -> Slot
  var endq: List[int]       // M-1: the places whose end records are queued, in queue order (take_end reads the head)
  var g: Ghost
  var conf_step: ConfStep   // D-UP-1: the action that produced this state (model-only)

  val free0: Slot = { own: 0, st: "free", fwd: false }
  val g0: Ghost = { refusedUnder: false, clash: false, refused: false, otherAdmitted: false }

  def heldBy(a: int): int = SLOTS.filter(s => slot.get(s).own == a).size()
  def activeBy(a: int): int = SLOTS.filter(s => slot.get(s).own == a and slot.get(s).st == "active").size()
  val totalHeld: int = SLOTS.filter(s => slot.get(s).own != 0).size()
  val freeSlots: Set[int] = SLOTS.filter(s => slot.get(s).own == 0)
  // the quota check of the design: the allocation's places, half-closed and ended (unread) flows included
  def withinQuota(a: int): bool =
    if (BUG_NODE_WIDE) totalHeld < Q
    else if (BUG_COUNT_ACTIVE_ONLY) activeBy(a) < Q
    else heldBy(a) < Q
  val minFree: int = freeSlots.fold(CAP + 1, (m, s) => if (s < m) s else m)
  val halfClosed: bool = SLOTS.exists(s => slot.get(s).fwd and slot.get(s).st != "active")

  action init = all { slot' = SLOTS.mapBy(s => free0), endq' = List(), g' = g0, conf_step' = variant("init", ()) }
  // a pair's end queues its record
  def queueEnd(s: int): List[int] = endq.append(s)

  // admission: within the quota and a free place exists → the flow takes the place (its pair is attached)
  action Admit(a: int): bool = all {
    withinQuota(a), freeSlots != Set(),
    val s = minFree
    all {
      slot' = slot.set(s, { own: a, st: "active", fwd: true }),
      endq' = endq,
      g' = { ...g, clash: g.clash or slot.get(s).fwd,
             otherAdmitted: g.otherAdmitted or SLOTS.exists(t => slot.get(t).own != a and slot.get(t).st == "half") },
    },
    conf_step' = if (REC) variant("Admit", { a: a }) else conf_step,
  }
  // the typed capacity outcome: nothing changes (a refusal below the allocation's own quota is the ghost)
  action Refuse(a: int): bool = all {
    not(withinQuota(a)) or freeSlots == Set(),
    slot' = slot, endq' = endq,
    g' = { ...g, refused: true, refusedUnder: g.refusedUnder or (heldBy(a) < Q and freeSlots != Set()) },
    conf_step' = if (REC) variant("Refuse", { a: a }) else conf_step,
  }
  // a direction of the flow half-closes: the forwarder carries the half-close after the last accepted byte; the pair
  // keeps forwarding the other direction, and the flow keeps its place
  action HalfClose(s: int): bool = all {
    slot.get(s).st == "active",
    slot' = slot.set(s, { ...slot.get(s), st: "half" }),
    endq' = endq, g' = g, conf_step' = if (REC) variant("HalfClose", { s: s }) else conf_step,
  }
  // the other direction ends cleanly too: the pair completes and queues its end record
  action Complete(s: int): bool = all {
    slot.get(s).fwd, slot.get(s).st == "half",
    slot' = slot.set(s, { ...slot.get(s), st: "ended", fwd: false }),
    endq' = queueEnd(s), g' = g, conf_step' = if (REC) variant("Complete", { s: s }) else conf_step,
  }
  // the pair aborts (a peer abort, a socket error, the owner's abort): it stops forwarding and queues its end record
  action Abort(s: int): bool = all {
    slot.get(s).fwd,
    slot' = slot.set(s, { ...slot.get(s), st: "ended", fwd: false }),
    endq' = queueEnd(s), g' = g, conf_step' = if (REC) variant("Abort", { s: s }) else conf_step,
  }
  // the host owner reads the next end record (the head of the queue, M-1) and releases its flow's place (EVMOD-AB)
  def canReadEnd(s: int): bool =
    slot.get(s).st == "ended" and endq.length() > 0 and endq.head() == s
  action ReadEnd(s: int): bool = all {
    canReadEnd(s),
    slot' = slot.set(s, free0), endq' = endq.tail(), g' = g, conf_step' = if (REC) variant("ReadEnd", { s: s }) else conf_step,
  }

  action step = {
    nondet a = ALLOCS.oneOf()
    nondet s = SLOTS.oneOf()
    any { Admit(a), Refuse(a), HalfClose(s), Complete(s), Abort(s), ReadEnd(s) }
  }

  // ------------------------------------------------------------------ invariants
  // QuotaBound (ADR-0147): an allocation holds at most its quota of places, half-closed and unread ended flows included
  val QuotaBound: bool = ALLOCS.forall(a => heldBy(a) <= Q)
  // QuotaRefusalOnlyAtOwnQuota: the quota never refuses an admission while the allocation is below its own quota and
  // a place is free — what a guest holds, half-closed flows included, counts against its own quota only (a refusal
  // for node capacity is a different outcome)
  val QuotaRefusalOnlyAtOwnQuota: bool = not(g.refusedUnder)
  // DrainingFlowHoldsItsSlot (EVMOD-AB): a flow whose pair still forwards — a half-closed flow included — holds its
  // place until its end record is read, and no flow is ever admitted onto a place whose pair still forwards
  val DrainingFlowHoldsItsSlot: bool = and {
    SLOTS.forall(s => slot.get(s).fwd implies slot.get(s).own != 0),
    not(g.clash),
  }
  val Safety: bool = and { QuotaBound, QuotaRefusalOnlyAtOwnQuota, DrainingFlowHoldsItsSlot }

  // ------------------------------------------------------------------ named runs (critical scenarios)
  def st(s: int): str = slot.get(s).st
  // admission up to the allocation's quota; the next admission of that allocation is refused (the typed
  // capacity outcome, no state change); another allocation is still admitted
  run AdmissionStopsAtTheAllocationQuota = init
    .then(Admit(1)).expect(Safety and st(1) == "active" and slot.get(1).own == 1)
    .expect(not(withinQuota(1)))
    .then(Refuse(1)).expect(Safety and g.refused and heldBy(1) == 1 and totalHeld == 1)
    .then(Admit(2)).expect(Safety and slot.get(2).own == 2)
  // a half-closed flow keeps its place and still counts against the quota
  run DrainingFlowStillCountsAgainstTheQuota = init
    .then(Admit(1)).then(HalfClose(1))
    .expect(Safety and st(1) == "half" and slot.get(1).fwd and heldBy(1) == 1)
    .expect(not(withinQuota(1)))
    .then(Refuse(1)).expect(Safety and g.refused and slot.get(1).fwd and totalHeld == 1)
  // ADR-0183: a guest that withholds reads holds a half-closed flow at most within its own quota; the other
  // allocation is admitted, completes and is released meanwhile
  run WithholdingGuestHoldsAtMostItsOwnQuota = init
    .then(Admit(1)).then(HalfClose(1)).then(Refuse(1))
    .then(Admit(2)).expect(Safety and slot.get(2).own == 2 and g.otherAdmitted)
    .then(HalfClose(2)).then(Complete(2)).then(ReadEnd(2))
    .expect(Safety and st(1) == "half" and slot.get(1).fwd and st(2) == "free")
    .then(Admit(2)).expect(Safety and slot.get(2).own == 2 and not(g.clash) and not(g.refusedUnder))
  // EVMOD-AB: the pair completes both ways and queues its end record; the place is held until the owner reads it,
  // then released and reused without a clash
  run CompletedPairReleasesTheSlotAtItsEndRecord = init
    .then(Admit(1)).then(HalfClose(1)).then(Complete(1))
    .expect(Safety and st(1) == "ended" and not(slot.get(1).fwd) and heldBy(1) == 1 and not(withinQuota(1)))
    .then(ReadEnd(1)).expect(Safety and st(1) == "free" and totalHeld == 0)
    .then(Admit(1)).expect(Safety and st(1) == "active" and not(g.clash))
  // EVMOD-AB: an abort of a half-closed flow ends its pair; the place is released at the owner's read of the end record
  run AbortOfHalfClosedFlowReleasesTheSlotAtItsEndRecord = init
    .then(Admit(1)).then(HalfClose(1)).then(Abort(1))
    .expect(Safety and st(1) == "ended" and not(slot.get(1).fwd) and heldBy(1) == 1)
    .then(ReadEnd(1)).expect(Safety and st(1) == "free" and totalHeld == 0)
    .then(Admit(2)).expect(Safety and slot.get(1).own == 2 and not(g.clash))

  // M-1: two pairs end; their records are read in the order they were queued, and the later one is not readable first
  run EndRecordsAreReadInQueueOrder = init
    .then(Admit(1)).then(Admit(2)).then(Abort(2)).then(Abort(1))
    .expect(Safety and endq == [2, 1] and st(1) == "ended" and st(2) == "ended")
    .expect(not(canReadEnd(1)))
  run EndRecordsAreReleasedInQueueOrder = init
    .then(Admit(1)).then(Admit(2)).then(Abort(2)).then(Abort(1))
    .then(ReadEnd(2)).expect(Safety and st(2) == "free" and st(1) == "ended" and endq == [1])
    .then(ReadEnd(1)).expect(Safety and totalHeld == 0 and endq == [])

}

// the design: three allocations, quota 1, two places (node capacity below the sum of the quotas)
module quota_ok { import flow_quota(OFF = Set(), REC = false).* from "./flow_quota" }
// conformance main (D-UP-1): `quota_ok` recording `conf_step`; only the named runs are checked on it
module quota_conf { import flow_quota(OFF = Set(), REC = true).* from "./flow_quota" }
