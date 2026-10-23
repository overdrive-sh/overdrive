// -*- mode: Bluespec; -*-
/**
 * TCP-CORRELATION (netns-density-295) — shared pending-place ownership between an accepted intake child that is
 * still waiting for correlation and the CapturedConn emitted for that same child.
 *
 * Design artifact. This is a pure protocol model, not a model of BPF option bytes, callbacks, maps, sockets or
 * syscalls. The production GUEST_TCP_PENDING_MAX of 4,096 is symmetry-reduced to two abstract places. Four abstract
 * children are enough to fill both places, witness an overflow reset, release one place and reuse it.
 *
 * The rule:
 *   - Accept charges one free place. A Pending child keeps that place when it becomes Ready and is emitted.
 *   - Ready is consumed once. The emitted handle keeps the same place until attach, abort or close consumes it.
 *   - missing/refused correlation and a Pending child close release the charged place without an event, owner flow
 *     or control traffic.
 *   - overflow resets only when both places are held. Correlation storage introduces no refusal below that bound.
 *   - every release clears the child phase and makes its place available to another accepted child.
 *
 * OFF names the one TCP-CORRELATION decision weakening used by hazard/tcp_capture_pending_hazards.qnt:
 *   capacityRefusal  a correlated child is refused for correlation capacity while a pending place is free.
 */
module tcp_capture_pending {
  const OFF: Set[str]

  val CHILDREN: Set[int] = Set(1, 2, 3, 4)
  val PLACES: Set[int] = Set(1, 2)
  val LIMIT: int = 2

  val BUG_CAPACITY_REFUSAL: bool = OFF.contains("capacityRefusal")

  type Ghost = {
    overflowUnderBound: bool,
    capacityRefusedUnderBound: bool,
  }

  // phase is exactly free | correlation_pending | emitted. `place == 0` means uncharged.
  // row abstracts only the child-correlation classification visible to the adapter: missing | pending | ready |
  // refused. BPF representation and callback mechanics are outside this model.
  var phase: int -> str
  var place: int -> int
  var acceptedPlace: int -> int
  var row: int -> str
  var attempted: Set[int]
  var charged: Set[int]
  var released: Set[int]
  var rejected: Set[int]
  var overflowed: Set[int]
  var capacityRefused: Set[int]
  var eventCount: int -> int
  var ownerFlow: Set[int]
  var controlTraffic: Set[int]
  var g: Ghost

  val g0: Ghost = { overflowUnderBound: false, capacityRefusedUnderBound: false }

  def live(c: int): bool = phase.get(c) != "free"
  val held: Set[int] = CHILDREN.filter(c => live(c))
  val usedPlaces: Set[int] = held.map(c => place.get(c))
  val freePlaces: Set[int] = PLACES.exclude(usedPlaces)
  val minFree: int = freePlaces.fold(LIMIT + 1, (m, p) => if (p < m) p else m)

  action init = all {
    phase' = CHILDREN.mapBy(c => "free"),
    place' = CHILDREN.mapBy(c => 0),
    acceptedPlace' = CHILDREN.mapBy(c => 0),
    row' = CHILDREN.mapBy(c => "missing"),
    attempted' = Set(), charged' = Set(), released' = Set(), rejected' = Set(),
    overflowed' = Set(), capacityRefused' = Set(),
    eventCount' = CHILDREN.mapBy(c => 0),
    ownerFlow' = Set(), controlTraffic' = Set(), g' = g0,
  }

  // Kernel-side correlation facts are environment inputs. They may be prepared only before this abstract child is
  // accepted. "missing" needs no preparation because it is the initial row.
  action PreparePending(c: int): bool = all {
    CHILDREN.contains(c), not(attempted.contains(c)), phase.get(c) == "free", row.get(c) == "missing",
    row' = row.set(c, "pending"),
    phase' = phase, place' = place, acceptedPlace' = acceptedPlace,
    attempted' = attempted, charged' = charged, released' = released, rejected' = rejected,
    overflowed' = overflowed, capacityRefused' = capacityRefused, eventCount' = eventCount,
    ownerFlow' = ownerFlow, controlTraffic' = controlTraffic, g' = g,
  }

  action PrepareReady(c: int): bool = all {
    CHILDREN.contains(c), not(attempted.contains(c)), phase.get(c) == "free", row.get(c) == "missing",
    row' = row.set(c, "ready"),
    phase' = phase, place' = place, acceptedPlace' = acceptedPlace,
    attempted' = attempted, charged' = charged, released' = released, rejected' = rejected,
    overflowed' = overflowed, capacityRefused' = capacityRefused, eventCount' = eventCount,
    ownerFlow' = ownerFlow, controlTraffic' = controlTraffic, g' = g,
  }

  action PrepareRefused(c: int): bool = all {
    CHILDREN.contains(c), not(attempted.contains(c)), phase.get(c) == "free", row.get(c) == "missing",
    row' = row.set(c, "refused"),
    phase' = phase, place' = place, acceptedPlace' = acceptedPlace,
    attempted' = attempted, charged' = charged, released' = released, rejected' = rejected,
    overflowed' = overflowed, capacityRefused' = capacityRefused, eventCount' = eventCount,
    ownerFlow' = ownerFlow, controlTraffic' = controlTraffic, g' = g,
  }

  // Acceptance and immediate classification are one adapter step. The correct protocol has exactly three outcomes:
  // full -> reset without charging; correlated -> charge and hold/emit; missing/refused -> charge and immediately
  // release without an output. The hazard adds the forbidden fourth outcome: correlation-capacity refusal below the
  // pending bound.
  action Accept(c: int): bool = all {
    CHILDREN.contains(c), not(attempted.contains(c)), phase.get(c) == "free",
    if (freePlaces == Set()) all {
      phase' = phase, place' = place, acceptedPlace' = acceptedPlace, row' = row.set(c, "missing"),
      attempted' = attempted.union(Set(c)), charged' = charged, released' = released,
      rejected' = rejected.union(Set(c)), overflowed' = overflowed.union(Set(c)),
      capacityRefused' = capacityRefused, eventCount' = eventCount,
      ownerFlow' = ownerFlow, controlTraffic' = controlTraffic,
      g' = { ...g, overflowUnderBound: g.overflowUnderBound or held.size() < LIMIT },
    } else if (BUG_CAPACITY_REFUSAL and (row.get(c) == "pending" or row.get(c) == "ready")) all {
      phase' = phase, place' = place, acceptedPlace' = acceptedPlace, row' = row.set(c, "missing"),
      attempted' = attempted.union(Set(c)), charged' = charged, released' = released,
      rejected' = rejected.union(Set(c)), overflowed' = overflowed,
      capacityRefused' = capacityRefused.union(Set(c)), eventCount' = eventCount,
      ownerFlow' = ownerFlow, controlTraffic' = controlTraffic,
      g' = { ...g, capacityRefusedUnderBound: true },
    } else if (row.get(c) == "pending") all {
      phase' = phase.set(c, "correlation_pending"), place' = place.set(c, minFree),
      acceptedPlace' = acceptedPlace.set(c, minFree), row' = row,
      attempted' = attempted.union(Set(c)), charged' = charged.union(Set(c)), released' = released,
      rejected' = rejected, overflowed' = overflowed, capacityRefused' = capacityRefused,
      eventCount' = eventCount, ownerFlow' = ownerFlow, controlTraffic' = controlTraffic, g' = g,
    } else if (row.get(c) == "ready") all {
      phase' = phase.set(c, "emitted"), place' = place.set(c, minFree),
      acceptedPlace' = acceptedPlace.set(c, minFree), row' = row.set(c, "missing"),
      attempted' = attempted.union(Set(c)), charged' = charged.union(Set(c)), released' = released,
      rejected' = rejected, overflowed' = overflowed, capacityRefused' = capacityRefused,
      eventCount' = eventCount.set(c, eventCount.get(c) + 1),
      ownerFlow' = ownerFlow, controlTraffic' = controlTraffic, g' = g,
    } else all {
      // A missing or explicitly refused row is charged at acceptance and released by the same classification step.
      phase' = phase, place' = place, acceptedPlace' = acceptedPlace.set(c, minFree),
      row' = row.set(c, "missing"), attempted' = attempted.union(Set(c)), charged' = charged.union(Set(c)),
      released' = released.union(Set(c)), rejected' = rejected.union(Set(c)), overflowed' = overflowed,
      capacityRefused' = capacityRefused, eventCount' = eventCount,
      ownerFlow' = ownerFlow, controlTraffic' = controlTraffic, g' = g,
    },
  }

  // Fast Open: the callback changes only the correlation row. The adapter-held child keeps its exact place.
  action PendingBecomesReady(c: int): bool = all {
    phase.get(c) == "correlation_pending", row.get(c) == "pending",
    row' = row.set(c, "ready"),
    phase' = phase, place' = place, acceptedPlace' = acceptedPlace,
    attempted' = attempted, charged' = charged, released' = released, rejected' = rejected,
    overflowed' = overflowed, capacityRefused' = capacityRefused, eventCount' = eventCount,
    ownerFlow' = ownerFlow, controlTraffic' = controlTraffic, g' = g,
  }

  // The owner takes Ready once and emits the existing capture event. The same place transfers to the emitted handle.
  action EmitReady(c: int): bool = all {
    phase.get(c) == "correlation_pending", row.get(c) == "ready", eventCount.get(c) == 0,
    phase' = phase.set(c, "emitted"), row' = row.set(c, "missing"),
    place' = place, acceptedPlace' = acceptedPlace,
    eventCount' = eventCount.set(c, 1),
    attempted' = attempted, charged' = charged, released' = released, rejected' = rejected,
    overflowed' = overflowed, capacityRefused' = capacityRefused,
    ownerFlow' = ownerFlow, controlTraffic' = controlTraffic, g' = g,
  }

  // A held Pending child whose row disappears is reset and releases its place without an event.
  action Missing(c: int): bool = all {
    phase.get(c) == "correlation_pending", row.get(c) == "pending",
    phase' = phase.set(c, "free"), place' = place.set(c, 0), row' = row.set(c, "missing"),
    acceptedPlace' = acceptedPlace, attempted' = attempted, charged' = charged,
    released' = released.union(Set(c)), rejected' = rejected.union(Set(c)),
    overflowed' = overflowed, capacityRefused' = capacityRefused, eventCount' = eventCount,
    ownerFlow' = ownerFlow, controlTraffic' = controlTraffic, g' = g,
  }

  action Refused(c: int): bool = all {
    phase.get(c) == "correlation_pending", row.get(c) == "pending",
    phase' = phase.set(c, "free"), place' = place.set(c, 0), row' = row.set(c, "missing"),
    acceptedPlace' = acceptedPlace, attempted' = attempted, charged' = charged,
    released' = released.union(Set(c)), rejected' = rejected.union(Set(c)),
    overflowed' = overflowed, capacityRefused' = capacityRefused, eventCount' = eventCount,
    ownerFlow' = ownerFlow, controlTraffic' = controlTraffic, g' = g,
  }

  action ChildClose(c: int): bool = all {
    phase.get(c) == "correlation_pending",
    phase' = phase.set(c, "free"), place' = place.set(c, 0), row' = row.set(c, "missing"),
    acceptedPlace' = acceptedPlace, attempted' = attempted, charged' = charged,
    released' = released.union(Set(c)), rejected' = rejected.union(Set(c)),
    overflowed' = overflowed, capacityRefused' = capacityRefused, eventCount' = eventCount,
    ownerFlow' = ownerFlow, controlTraffic' = controlTraffic, g' = g,
  }

  // Successful attachment is the only abstract step that creates the downstream owner-flow/control artifacts. The
  // adapter's place is released after attachment completes.
  action Attach(c: int): bool = all {
    phase.get(c) == "emitted",
    phase' = phase.set(c, "free"), place' = place.set(c, 0), row' = row,
    acceptedPlace' = acceptedPlace, attempted' = attempted, charged' = charged,
    released' = released.union(Set(c)), rejected' = rejected,
    overflowed' = overflowed, capacityRefused' = capacityRefused, eventCount' = eventCount,
    ownerFlow' = ownerFlow.union(Set(c)), controlTraffic' = controlTraffic.union(Set(c)), g' = g,
  }

  action Abort(c: int): bool = all {
    phase.get(c) == "emitted",
    phase' = phase.set(c, "free"), place' = place.set(c, 0), row' = row,
    acceptedPlace' = acceptedPlace, attempted' = attempted, charged' = charged,
    released' = released.union(Set(c)), rejected' = rejected,
    overflowed' = overflowed, capacityRefused' = capacityRefused, eventCount' = eventCount,
    ownerFlow' = ownerFlow, controlTraffic' = controlTraffic, g' = g,
  }

  action Close(c: int): bool = all {
    phase.get(c) == "emitted",
    phase' = phase.set(c, "free"), place' = place.set(c, 0), row' = row,
    acceptedPlace' = acceptedPlace, attempted' = attempted, charged' = charged,
    released' = released.union(Set(c)), rejected' = rejected,
    overflowed' = overflowed, capacityRefused' = capacityRefused, eventCount' = eventCount,
    ownerFlow' = ownerFlow, controlTraffic' = controlTraffic, g' = g,
  }

  action step = {
    nondet c = CHILDREN.oneOf()
    any {
      PreparePending(c), PrepareReady(c), PrepareRefused(c), Accept(c), PendingBecomesReady(c), EmitReady(c),
      Missing(c), Refused(c), ChildClose(c), Attach(c), Abort(c), Close(c),
    }
  }

  // ------------------------------------------------------------------ invariants
  val OnePhasePerChild: bool = CHILDREN.forall(c =>
    Set("free", "correlation_pending", "emitted").contains(phase.get(c)))

  // A live child owns exactly one unique place; released children own none. Pending -> emitted preserves the place
  // accepted for that child instead of taking a second charge.
  val OneSharedCharge: bool = and {
    CHILDREN.forall(c => live(c) iff and {
      charged.contains(c), not(released.contains(c)), PLACES.contains(place.get(c)),
      place.get(c) == acceptedPlace.get(c),
    }),
    CHILDREN.forall(c => not(live(c)) implies place.get(c) == 0),
    PLACES.forall(p => CHILDREN.filter(c => place.get(c) == p).size() <= 1),
    held.size() <= LIMIT,
  }

  // Overflow is the shared pending bound only. A correlation collection cannot impose an earlier refusal.
  val OverflowOnlyAtBound: bool = not(g.overflowUnderBound)
  val NoCorrelationCapacityRefusalBelowBound: bool =
    capacityRefused == Set() and not(g.capacityRefusedUnderBound)

  // A Ready row is taken once. Classification rejection never creates an event, owner flow or control traffic.
  val ReadyEmitsOnce: bool = CHILDREN.forall(c => and {
    eventCount.get(c) >= 0,
    eventCount.get(c) <= 1,
    phase.get(c) == "emitted" implies eventCount.get(c) == 1,
    (row.get(c) == "ready" and attempted.contains(c)) implies
      (phase.get(c) == "correlation_pending" and eventCount.get(c) == 0),
    eventCount.get(c) == 1 implies and {
      charged.contains(c),
      row.get(c) == "missing",
      phase.get(c) == "emitted" or released.contains(c),
    },
    ownerFlow.contains(c) implies eventCount.get(c) == 1,
    controlTraffic.contains(c) implies eventCount.get(c) == 1,
  })
  val RejectionCreatesNoOwnerOutput: bool = rejected.forall(c => and {
    eventCount.get(c) == 0,
    not(ownerFlow.contains(c)),
    not(controlTraffic.contains(c)),
  })

  // Every attempted child is exactly one of: charged, overflow-reset, or the forbidden hazard refusal. In the
  // design instance the last set is empty. Every charged child eventually released by a cleanup action is Free.
  val AdmissionAndReleaseAccounting: bool = and {
    attempted == charged.union(overflowed).union(capacityRefused),
    charged.intersect(overflowed) == Set(), charged.intersect(capacityRefused) == Set(),
    overflowed.intersect(capacityRefused) == Set(),
    released.forall(c => charged.contains(c) and phase.get(c) == "free" and place.get(c) == 0),
  }

  val Safety: bool = and {
    OnePhasePerChild,
    OneSharedCharge,
    OverflowOnlyAtBound,
    NoCorrelationCapacityRefusalBelowBound,
    ReadyEmitsOnce,
    RejectionCreatesNoOwnerOutput,
    AdmissionAndReleaseAccounting,
  }

  // ------------------------------------------------------------------ named runs (critical scenarios)
  run PendingToReadyTransfersTheSameCharge = init
    .then(PreparePending(1)).then(Accept(1))
    .expect(Safety and phase.get(1) == "correlation_pending" and place.get(1) == 1 and eventCount.get(1) == 0)
    .then(PendingBecomesReady(1))
    .expect(Safety and phase.get(1) == "correlation_pending" and place.get(1) == acceptedPlace.get(1))
    .then(EmitReady(1))
    .expect(Safety and phase.get(1) == "emitted" and place.get(1) == acceptedPlace.get(1) and eventCount.get(1) == 1)
    .then(Attach(1)).expect(Safety and phase.get(1) == "free" and freePlaces == PLACES)

  run ReadyAtAcceptEmitsOnceThenAbortReusesThePlace = init
    .then(PrepareReady(1)).then(Accept(1))
    .expect(Safety and phase.get(1) == "emitted" and place.get(1) == 1 and eventCount.get(1) == 1)
    .then(Abort(1)).expect(Safety and phase.get(1) == "free" and freePlaces == PLACES)
    .then(PrepareReady(2)).then(Accept(2))
    .expect(Safety and phase.get(2) == "emitted" and place.get(2) == 1 and eventCount.get(2) == 1)

  run MissingRefusedAndChildCloseReleaseWithoutOutputs = init
    .then(Accept(1))
    .expect(Safety and rejected.contains(1) and released.contains(1) and phase.get(1) == "free")
    .then(PrepareRefused(2)).then(Accept(2))
    .expect(Safety and rejected.contains(2) and released.contains(2) and phase.get(2) == "free")
    .then(PreparePending(3)).then(Accept(3)).then(ChildClose(3))
    .expect(Safety and rejected.contains(3) and released.contains(3) and freePlaces == PLACES)

  run OverflowResetsOnlyAtTheBoundThenAReleaseMakesRoom = init
    .then(PreparePending(1)).then(Accept(1)).then(PreparePending(2)).then(Accept(2))
    .expect(Safety and held.size() == LIMIT and freePlaces == Set())
    .then(PrepareReady(3)).then(Accept(3))
    .expect(Safety and overflowed.contains(3) and rejected.contains(3) and eventCount.get(3) == 0)
    .then(Refused(1)).expect(Safety and freePlaces == Set(1))
    .then(PrepareReady(4)).then(Accept(4))
    .expect(Safety and phase.get(4) == "emitted" and place.get(4) == 1 and held.size() == LIMIT)

  run AttachAbortAndCloseEachReleaseTheirPlace = init
    .then(PrepareReady(1)).then(Accept(1)).then(Attach(1))
    .expect(Safety and ownerFlow.contains(1) and controlTraffic.contains(1) and freePlaces == PLACES)
    .then(PrepareReady(2)).then(Accept(2)).then(Abort(2))
    .expect(Safety and not(ownerFlow.contains(2)) and freePlaces == PLACES)
    .then(PrepareReady(3)).then(Accept(3)).then(Close(3))
    .expect(Safety and not(ownerFlow.contains(3)) and freePlaces == PLACES)
}

module pending_ok { import tcp_capture_pending(OFF = Set()).* from "./tcp_capture_pending" }
