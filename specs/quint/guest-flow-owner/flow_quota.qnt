// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — the per-allocation flow quota counts draining flows (R2-DRAIN,
 * ADR-0183; quota ADR-0147).
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 *
 * The rule: a half-close drain has no deadline, so a guest that withholds credit holds its draining
 * flow, and the flow's drain cell, for as long as it likes. The only bound on that is the
 * per-allocation flow quota, and it bounds only if the quota counts the draining (half-closed, not yet
 * completed) flows too: a draining flow holds its quota slot exactly like an open flow, so a guest that
 * withholds credit holds at most its own quota — exactly what an untrusted guest can already hold with
 * open flows; draining adds no new exposure class. A flow holds its flow slot (and the slot's drain
 * cells) from admission to the end of the flow; a half-close does not release it. No claim is made
 * about other allocations' admissions beyond the quota (node capacity may be below the sum of quotas).
 *
 * Abstraction:
 *   - ALLOCS: allocations (VMs). SLOTS: the node's flow slots (the index of the drain-cell array), CAP =
 *     |SLOTS|. The model does NOT assume CAP >= the sum of the quotas: here CAP (2) is below it (3 * Q),
 *     so an admission may be refused for node capacity; that refusal is not a quota refusal.
 *   - A slot is free, active (a flow forwarding), draining (a direction half-closed, its drain pending),
 *     or half (the drain completed, the other direction still open). Admission is refused (the typed
 *     capacity outcome) when the allocation holds its quota; a refused admission changes nothing.
 *   - Admission, half-close, the drain's completion, the end of the flow and an abort are each one
 *     atomic owner step. A drain that never completes (a withholding guest) is the absence of
 *     DrainDone; no fairness is assumed for it.
 *
 * OFF names the rules switched off (teeth; instances in hazard/):
 *   countActiveOnly   the quota counts active flows only; a draining flow keeps its slot but not its count
 *   nodeWideQuota     one quota for the node instead of one per allocation
 *   freeAtHalfClose   the slot is released at the half-close, while the drain is pending
 */
module flow_quota {
  const OFF: Set[str]

  val ALLOCS: Set[int] = Set(1, 2, 3)
  val Q: int = 1
  val SLOTS: Set[int] = Set(1, 2)
  val CAP: int = 2

  val BUG_COUNT_ACTIVE_ONLY: bool = OFF.contains("countActiveOnly")
  val BUG_NODE_WIDE: bool = OFF.contains("nodeWideQuota")
  val BUG_FREE_AT_HALF_CLOSE: bool = OFF.contains("freeAtHalfClose")

  // own: the allocation holding the slot (0 = free); st: free | active | draining | half;
  // pend: the slot's drain cell is armed for a drain that has not completed
  type Slot = { own: int, st: str, pend: bool }
  type Ghost = {
    refusedUnder: bool,   // an admission was refused at the quota check while the allocation was below its own quota and a slot was free
    clash: bool,          // a flow was admitted onto a slot whose drain cell was still pending
    refused: bool,        // an admission was refused at the quota (witness)
    otherAdmitted: bool,  // an allocation was admitted while another held a pending drain (witness)
  }

  var slot: int -> Slot
  var g: Ghost

  val free0: Slot = { own: 0, st: "free", pend: false }
  val g0: Ghost = { refusedUnder: false, clash: false, refused: false, otherAdmitted: false }

  def heldBy(a: int): int = SLOTS.filter(s => slot.get(s).own == a).size()
  def activeBy(a: int): int = SLOTS.filter(s => slot.get(s).own == a and slot.get(s).st == "active").size()
  val totalHeld: int = SLOTS.filter(s => slot.get(s).own != 0).size()
  val freeSlots: Set[int] = SLOTS.filter(s => slot.get(s).own == 0)
  // the quota check of the design: the allocation's slots, draining flows included
  def withinQuota(a: int): bool =
    if (BUG_NODE_WIDE) totalHeld < Q
    else if (BUG_COUNT_ACTIVE_ONLY) activeBy(a) < Q
    else heldBy(a) < Q
  val minFree: int = freeSlots.fold(CAP + 1, (m, s) => if (s < m) s else m)
  val pendingDrain: bool = SLOTS.exists(s => slot.get(s).pend)

  action init = all { slot' = SLOTS.mapBy(s => free0), g' = g0 }

  // admission: within the quota and a free slot exists → the flow takes the slot
  action Admit(a: int): bool = all {
    withinQuota(a), freeSlots != Set(),
    val s = minFree
    all {
      slot' = slot.set(s, { own: a, st: "active", pend: false }),
      g' = { ...g, clash: g.clash or slot.get(s).pend,
             otherAdmitted: g.otherAdmitted or SLOTS.exists(t => slot.get(t).own != a and slot.get(t).pend) },
    },
  }
  // the typed capacity outcome: nothing changes (a refusal below the allocation's own quota is the ghost)
  action Refuse(a: int): bool = all {
    not(withinQuota(a)) or freeSlots == Set(),
    slot' = slot,
    g' = { ...g, refused: true, refusedUnder: g.refusedUnder or (heldBy(a) < Q and freeSlots != Set()) },
  }
  // a direction of the flow half-closes: the drain is armed; the flow keeps its slot
  action HalfClose(s: int): bool = all {
    slot.get(s).st == "active",
    slot' = slot.set(s, if (BUG_FREE_AT_HALF_CLOSE) { own: 0, st: "free", pend: true }
                        else { ...slot.get(s), st: "draining", pend: true }),
    g' = g,
  }
  // the drain completes (the reader advanced): the cell is disarmed; the other direction stays open
  action DrainDone(s: int): bool = all {
    slot.get(s).pend,
    slot' = slot.set(s, { ...slot.get(s), st: if (slot.get(s).st == "draining") "half" else slot.get(s).st, pend: false }),
    g' = g,
  }
  // both directions are done: the flow ends and releases its slot
  action EndFlow(s: int): bool = all {
    slot.get(s).st == "half",
    slot' = slot.set(s, free0), g' = g,
  }
  // an abort (stop, quiescence, session loss, a failed redirect, an unannounced end) releases the slot and
  // disarms the cell
  action Abort(s: int): bool = all {
    slot.get(s).own != 0,
    slot' = slot.set(s, free0), g' = g,
  }

  action step = {
    nondet a = ALLOCS.oneOf()
    nondet s = SLOTS.oneOf()
    any { Admit(a), Refuse(a), HalfClose(s), DrainDone(s), EndFlow(s), Abort(s) }
  }

  // ------------------------------------------------------------------ invariants
  // QuotaBound (ADR-0147, R2-DRAIN): an allocation holds at most its quota of slots, draining flows included
  val QuotaBound: bool = ALLOCS.forall(a => heldBy(a) <= Q)
  // QuotaRefusalOnlyAtOwnQuota (R2-DRAIN): the quota never refuses an admission while the allocation is below its
  // own quota and a slot is free — what a guest holds, draining flows included, counts against its own quota
  // only (a refusal for node capacity is a different outcome)
  val QuotaRefusalOnlyAtOwnQuota: bool = not(g.refusedUnder)
  // DrainingFlowHoldsItsSlot (R2-DRAIN): a pending drain's cell belongs to a flow that holds its slot, and no
  // flow is ever admitted onto a slot whose drain cell is pending
  val DrainingFlowHoldsItsSlot: bool = and {
    SLOTS.forall(s => slot.get(s).pend implies slot.get(s).own != 0),
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
  // R2-DRAIN: a draining flow keeps its slot and still counts against the quota
  run DrainingFlowStillCountsAgainstTheQuota = init
    .then(Admit(1)).then(HalfClose(1))
    .expect(Safety and st(1) == "draining" and slot.get(1).pend and heldBy(1) == 1)
    .expect(not(withinQuota(1)))
    .then(Refuse(1)).expect(Safety and g.refused and slot.get(1).pend and totalHeld == 1)
  // R2-DRAIN: a draining flow holds its quota slot like an open flow; a guest that withholds credit holds at most its own quota; the other allocation is admitted, drains
  // and completes meanwhile
  run WithholdingGuestHoldsAtMostItsOwnQuota = init
    .then(Admit(1)).then(HalfClose(1)).then(Refuse(1))
    .then(Admit(2)).expect(Safety and slot.get(2).own == 2 and g.otherAdmitted)
    .then(HalfClose(2)).then(DrainDone(2)).then(EndFlow(2))
    .expect(Safety and st(1) == "draining" and slot.get(1).pend and st(2) == "free")
    .then(Admit(2)).expect(Safety and slot.get(2).own == 2 and not(g.clash) and not(g.refusedUnder))
  // the drain completes, the flow ends: the slot is released and reused without a clash
  run CompletedDrainAndEndReleaseTheSlot = init
    .then(Admit(1)).then(HalfClose(1)).then(DrainDone(1))
    .expect(Safety and st(1) == "half" and not(slot.get(1).pend) and heldBy(1) == 1)
    .then(EndFlow(1)).expect(Safety and st(1) == "free" and totalHeld == 0)
    .then(Admit(1)).expect(Safety and st(1) == "active" and not(g.clash))
  // an abort of a draining flow (a failed redirect, an unannounced end) releases the slot and disarms the cell
  run AbortOfDrainingFlowReleasesSlotAndCell = init
    .then(Admit(1)).then(HalfClose(1)).then(Abort(1))
    .expect(Safety and st(1) == "free" and not(slot.get(1).pend) and totalHeld == 0)
    .then(Admit(2)).expect(Safety and slot.get(1).own == 2 and not(g.clash))

  // ------------------------------------------------------------------ witnesses (expected violated)
  val WitnessNoRefusal: bool = not(g.refused)
  val WitnessNoPendingDrain: bool = not(pendingDrain)
  val WitnessNoCrossAllocationAdmission: bool = not(g.otherAdmitted)
}

// the design: three allocations, quota 1, two slots (node capacity below the sum of the quotas)
module quota_ok { import flow_quota(OFF = Set()).* from "./flow_quota" }
