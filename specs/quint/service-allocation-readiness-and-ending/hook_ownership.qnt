// -*- mode: Bluespec; -*-
/**
 * Probe-supervisor ownership (service-allocation-readiness-and-ending): D1 (the exit observer fires on_alloc_terminal per consumed
 * ExitEvent, before and regardless of the row write), D3 (the shim registers supervision before it releases the
 * exit-event gate); D2 (the hook is row-independent and idempotent) and D4 (the shim stop and finalize hooks stay,
 * cleanup_restart_predecessor gains none) are contract and non-change decisions, covered by the named runs.
 *
 * Two allocation ids: 1 (the first attempt) and 2 (the fresh-id successor, started only after id 1 has a terminal row,
 * the RestartAllocation precondition). Per id (record Alloc):
 *   started / regd / rel   StartAllocation ran / on_alloc_running ran (supervisor registered) / release_for_exit_emission
 *                          ran (the gate send)
 *   dead                   the guest died naturally (the watcher knows; it may emit only after the gate is released)
 *   ev                     ExitEvent: none | queued | consumed
 *   vmstop / srw           the shim StopAllocation effects ran (VmDriver::stop, mTLS stop, network teardown) / the stop's
 *                          terminal row write landed (B2, ADR-0259: the arm's order is effects, row write, hook)
 *   stopped / fin          the shim StopAllocation hook (after the row write) / FinalizeFailed hook ran (D4)
 *   sup                    the probe supervisor is present
 *   row                    a terminal row exists (the observer wrote it, or the shim stop / finalize arm did)
 *   late                   ghost: on_alloc_running registered a supervisor for an id whose end was already claimed
 *
 * Actors: shim start (Start, Register, Release), the watcher (Die, Emit), the observer (Consume), shim stop (StopEffects,
 * StopRow, StopHook), shim finalize (Finalize). The observation write outcome is the parameter `wrote` of Consume (ok | exhausted/NoWrite).
 *
 * OFF names the rules switched off (hazards):
 *   hookGatedOnRowWrite   D1: the observer fires the hook only when the row write succeeded
 *   gateBeforeRegistration D3: the shim releases the gate first and registers afterwards (the pre-D3 order)
 *
 * ASSUMPTIONS (each discharged by the item after it):
 *   HO-1  Release (the gate send) is the last effect of release_for_exit_emission, so the watcher can emit an ExitEvent
 *         only after the gate is released (V5). Discharged by: V5, S-4.
 *   HO-2  VmDriver::stop emits no ExitEvent (the watcher's try_begin_ending returns None) (V1). Discharged by: V1, S-6.
 *   HO-3  The hook is infallible, synchronous, idempotent and a no-op for an unknown id (D2, V2). Discharged by: V2, S-7.
 *   HO-4  A registered supervisor implies a held VM slot until the hook runs, except the state a failed stop row write
 *         creates (vacant slot, supervisor registered, row Running), which has no host residue and is retried by the next
 *         probe row; so reclamation (which needs a vacant slot) never ends a supervised id (V7). Discharged by: V7.
 *   HO-5  The weak upgrade of the observer's driver succeeds while the process runs; at shutdown the runner is gone with
 *         the driver. Discharged by: exit_observer.rs:262-265 (D1 Q2).
 *
 * D-UP-1 (conformance step record): `conf_step` records the action that produced each state, a variant of
 * `hook_ownership_conf::ConfStep` whose tag is the action name and whose value is the record of its parameters.
 * Model-only and write-only. Conformance projection: sup (per id) is `ProbeRunner::active_alloc_count()` /
 * `is_role_live`; row is `ObservationStore::alloc_status_row(alloc)`; started/regd/rel/dead/ev/stopped/fin/late are
 * model-only ghosts the driver drives through Driver::on_alloc_running, release_for_exit_emission, on_alloc_terminal and
 * the ExitEvent receiver.
 */
module hook_ownership_conf {
  type ConfStep =
    | init
    | Start({ a: int })
    | Register({ a: int })
    | Release({ a: int })
    | Die({ a: int })
    | Emit({ a: int })
    | Consume({ a: int, wrote: bool })
    | StopEffects({ a: int })
    | StopRow({ a: int, wrote: bool })
    | StopHook({ a: int })
    | Finalize({ a: int })
    | Idle
}

module hook_ownership {
  import hook_ownership_conf.ConfStep from "./hook_ownership"
  const OFF: Set[str]
  const REC: bool

  val IDS: Set[int] = Set(1, 2)
  val BUG_GATED: bool = OFF.contains("hookGatedOnRowWrite")
  val BUG_ORDER: bool = OFF.contains("gateBeforeRegistration")

  type Alloc = {
    started: bool, regd: bool, rel: bool, dead: bool, ev: str, vmstop: bool, srw: bool, stopped: bool, fin: bool,
    sup: bool, row: bool, late: bool,
  }

  var al: int -> Alloc
  var conf_step: ConfStep

  val blank: Alloc = {
    started: false, regd: false, rel: false, dead: false, ev: "none", vmstop: false, srw: false, stopped: false, fin: false,
    sup: false, row: false, late: false,
  }

  pure def ended(r: Alloc): bool = r.ev == "consumed" or r.stopped or r.fin

  action init = all {
    al' = IDS.mapBy(a => blank),
    conf_step' = variant("init", ()),
  }

  // StartAllocation / the restart successor: the VM is started, the gate is held. The successor is placed only after the
  // predecessor has a terminal row (RestartAllocation requires Failed | Terminated).
  action Start(a: int): bool = all {
    not(al.get(a).started),
    a == 1 or al.get(1).row,
    al' = al.set(a, { ...al.get(a), started: true }),
    conf_step' = if (REC) variant("Start", { a: a }) else conf_step,
  }

  // Driver::on_alloc_running: register the probe supervisor. D3: before the gate release. The pre-D3 hazard registers
  // after it.
  action Register(a: int): bool = {
    val r = al.get(a)
    all {
      r.started, not(r.regd),
      if (BUG_ORDER) r.rel else true,
      al' = al.set(a, { ...r, regd: true, sup: true, late: r.late or ended(r) }),
      conf_step' = if (REC) variant("Register", { a: a }) else conf_step,
    }
  }

  // Driver::release_for_exit_emission: the gate send. D3: after registration. The pre-D3 hazard releases first.
  def canRelease(a: int): bool = {
    val r = al.get(a)
    r.started and not(r.rel) and (if (BUG_ORDER) true else r.regd)
  }
  action Release(a: int): bool = {
    val r = al.get(a)
    all {
      canRelease(a),
      al' = al.set(a, { ...r, rel: true }),
      conf_step' = if (REC) variant("Release", { a: a }) else conf_step,
    }
  }

  // the guest dies by itself (before or after the gate release; the watcher cannot emit before the gate)
  def canDie(a: int): bool = {
    val r = al.get(a)
    r.started and not(r.dead) and not(r.vmstop)
  }
  action Die(a: int): bool = {
    val r = al.get(a)
    all {
      canDie(a),
      al' = al.set(a, { ...r, dead: true }),
      conf_step' = if (REC) variant("Die", { a: a }) else conf_step,
    }
  }

  // the watcher emits the ExitEvent: only after the gate is released (HO-1) and never after a driver stop (HO-2)
  def canEmit(a: int): bool = {
    val r = al.get(a)
    r.dead and r.rel and not(r.vmstop) and r.ev == "none"
  }
  action Emit(a: int): bool = {
    val r = al.get(a)
    all {
      canEmit(a),
      al' = al.set(a, { ...r, ev: "queued" }),
      conf_step' = if (REC) variant("Emit", { a: a }) else conf_step,
    }
  }

  // the exit observer consumes the event. D1: it fires the hook first, whatever the write outcome (`wrote`: the row
  // write succeeded; false = retries exhausted or NoWrite).
  action Consume(a: int, wrote: bool): bool = {
    val r = al.get(a)
    all {
      r.ev == "queued",
      al' = al.set(a, { ...r, ev: "consumed", row: r.row or wrote,
                        sup: if (BUG_GATED) (if (wrote) false else r.sup) else false }),
      conf_step' = if (REC) variant("Consume", { a: a, wrote: wrote }) else conf_step,
    }
  }

  // shim StopAllocation (operator or liveness stop), B2 (ADR-0259), in the arm's order: the stop effects, the terminal row
  // write, then the hook. No ExitEvent follows (HO-2). A failed row write leaves the supervisor registered; the retry
  // finds the guest stopped and writes the row.
  action StopEffects(a: int): bool = {
    val r = al.get(a)
    all {
      r.rel, not(r.dead), not(r.vmstop),
      al' = al.set(a, { ...r, vmstop: true }),
      conf_step' = if (REC) variant("StopEffects", { a: a }) else conf_step,
    }
  }
  action StopRow(a: int, wrote: bool): bool = {
    val r = al.get(a)
    all {
      r.vmstop, not(r.srw),
      al' = al.set(a, { ...r, srw: r.srw or wrote, row: r.row or wrote }),
      conf_step' = if (REC) variant("StopRow", { a: a, wrote: wrote }) else conf_step,
    }
  }
  action StopHook(a: int): bool = {
    val r = al.get(a)
    all {
      r.srw, not(r.stopped),
      al' = al.set(a, { ...r, stopped: true, sup: false }),
      conf_step' = if (REC) variant("StopHook", { a: a }) else conf_step,
    }
  }

  // shim FinalizeFailed: the hook and the terminal row; the VM may still be alive (no driver.stop)
  action Finalize(a: int): bool = {
    val r = al.get(a)
    all {
      r.rel, not(r.fin),
      al' = al.set(a, { ...r, fin: true, sup: false, row: true }),
      conf_step' = if (REC) variant("Finalize", { a: a }) else conf_step,
    }
  }

  action Idle = all { al' = al, conf_step' = if (REC) variant("Idle", ()) else conf_step }

  action step = {
    nondet a = IDS.oneOf()
    nondet wrote = Set(true, false).oneOf()
    any { Start(a), Register(a), Release(a), Die(a), Emit(a), Consume(a, wrote), StopEffects(a), StopRow(a, wrote), StopHook(a), Finalize(a), Idle }
  }

  // ------------------------------------------------------------------ invariants
  // D3 NoRegisterAfterEnd: a supervisor is never registered for an id after its end was claimed (the ExitEvent consumed,
  // or the shim stop / finalize hook ran)
  val NoRegisterAfterEnd: bool = IDS.forall(a => not(al.get(a).late))
  val Safety: bool = NoRegisterAfterEnd

  // D1 EndedImpliesEventuallyUnsupervised (progress under fairness): an id whose guest ended and whose event is out
  // (or consumed) is eventually unsupervised, whatever the write outcome. Fairness: the shim, the watcher and the
  // observer take their enabled steps.
  temporal EndedImpliesEventuallyUnsupervised =
    and {
      IDS.forall(a => weakFair(Register(a), (al, conf_step))),
      IDS.forall(a => weakFair(Release(a), (al, conf_step))),
      IDS.forall(a => weakFair(Emit(a), (al, conf_step))),
      IDS.forall(a => Set(true, false).forall(w => weakFair(Consume(a, w), (al, conf_step)))),
    }.implies(IDS.forall(a => always((al.get(a).ev != "none").implies(eventually(not(al.get(a).sup))))))

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // #334: a crash with no restart. The supervisor is gone as soon as the observer handled the event, with the row
  // written and no successor ever started.
  run crash_without_restart = init
    .then(Start(1)).then(Register(1)).then(Release(1)).expect(al.get(1).sup and Safety)
    .then(Die(1)).then(Emit(1)).expect(al.get(1).sup)
    .then(Consume(1, true))
    .expect(not(al.get(1).sup) and al.get(1).row and not(al.get(2).started) and Safety)

  // #318: a crash, then the fresh-id restart. The predecessor is unsupervised, the successor is supervised and untouched
  // by the predecessor's hook; the shim made no hook call for the successor (cleanup_restart_predecessor gains none).
  run crash_then_fresh_id_restart = init
    .then(Start(1)).then(Register(1)).then(Release(1))
    .then(Die(1)).then(Emit(1)).then(Consume(1, true))
    .then(Start(2)).then(Register(2)).then(Release(2))
    .expect(not(al.get(1).sup) and al.get(2).sup and Safety)
    .then(Die(2)).then(Emit(2)).then(Consume(2, true))
    .expect(not(al.get(1).sup) and not(al.get(2).sup) and Safety)

  // D1 outcome independence: the observation write is exhausted (and the NoWrite case is the same input): the
  // supervisor is gone all the same, and no row was written
  run write_exhaustion_still_stops_probes = init
    .then(Start(1)).then(Register(1)).then(Release(1)).then(Die(1)).then(Emit(1))
    .then(Consume(1, false))
    .expect(not(al.get(1).sup) and not(al.get(1).row) and al.get(1).ev == "consumed" and Safety)

  // D4: an operator or liveness stop (shim StopAllocation) stops the probes and emits no ExitEvent
  run operator_or_liveness_stop_emits_no_exit_event = init
    .then(Start(1)).then(Register(1)).then(Release(1))
    .then(StopEffects(1))
    .expect(al.get(1).sup and not(al.get(1).row) and Safety)
    .then(StopRow(1, false))
    .expect(al.get(1).sup and not(al.get(1).srw) and not(al.get(1).row) and not(al.get(1).stopped) and Safety)
    .then(StopRow(1, true)).then(StopHook(1))
    .expect(not(al.get(1).sup) and al.get(1).row and al.get(1).ev == "none" and Safety)
    .expect(not(canDie(1)) and not(canEmit(1)))

  // D4: FinalizeFailed on a VM that may still be alive stops the probes by the shim hook; no ExitEvent exists to do it
  run finalize_on_live_vm_stops_probes = init
    .then(Start(1)).then(Register(1)).then(Release(1))
    .then(Finalize(1))
    .expect(not(al.get(1).sup) and al.get(1).row and not(al.get(1).dead) and al.get(1).ev == "none" and Safety)

  // D2: double fire (the observer, then FinalizeFailed on the same id) and the hook on an id that never ran
  run double_fire_is_idempotent = init
    .then(Start(1)).then(Register(1)).then(Release(1)).then(Die(1)).then(Emit(1)).then(Consume(1, true))
    .expect(not(al.get(1).sup))
    .then(Finalize(1)).expect(not(al.get(1).sup) and al.get(1).row and Safety)
}

// Main instance: the design. All invariants hold.
module hooks_ok { import hook_ownership(OFF = Set(), REC = false).* from "./hook_ownership" }
// Conformance main: records conf_step; only the named runs are checked on it.
module hooks_conf { import hook_ownership(OFF = Set(), REC = true).* from "./hook_ownership" }
