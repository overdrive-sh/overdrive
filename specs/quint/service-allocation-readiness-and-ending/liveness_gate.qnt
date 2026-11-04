// -*- mode: Bluespec; -*-
/**
 * Liveness supervision begins at Stable; the probe runner owns the gate (service-allocation-readiness-and-ending D9, ADR-0254).
 *
 * One allocation. Record `lv`:
 *   rw       the allocation's row is Running (a crash or a stop clears it)
 *   sup      the probe supervisor is present (set by on_alloc_running, removed by on_alloc_terminal)
 *   ret      the supervisor retains the liveness descriptors without having spawned them
 *   role     the liveness role: absent | live
 *   committed  the allocation's row carries the Stable claim (ADR-0258: ServiceAllocFact.stable_committed, hydrated from the row)
 *   swr      outcome of the last Stable write: none | landed | failed
 *   hook     Driver::on_alloc_stable fired (the shim fires it after the Stable row is committed)
 *   ended    on_alloc_terminal ran
 *   streak   the producer-counted consecutive liveness failures, capped at LT;  since: liveness rows written since Stable
 * Actors: shim (OnRunning, OnStable, OnTerminal), ServiceLifecycle (StableCommit, LivenessStop), the probe
 * task (LivenessRow), the environment (Crash). StableCommit(w) is the emission of Stable and the shim's row write; w is the
 * write outcome landed | failed (B1, ADR-0258). The startup condition (a startup pass, or the opt-out first Running) is
 * abstracted to "the allocation is Running".
 *
 * OFF names the rules switched off (hazards): spawnAtRunning (the pre-D9 shape: on_alloc_running spawns the liveness
 * tasks); stampAtEmission (B1: emission is guarded by a record that a Stable write was attempted, set at emission, so a
 * Stable whose write failed is never emitted again; expressed on the write-outcome variable swr).
 *
 * ASSUMPTIONS (each discharged by the item after it):
 *   LG-1  Stable is committed before on_alloc_stable fires, on every Stable path including the opt-out first Running
 *         (V19). Discharged by: V19, S-31.
 *   LG-2  start_liveness is atomic with stop_alloc: both take the supervisors lock, so either the liveness tasks start
 *         and a later stop cancels them, or the allocation is gone and nothing starts (V20). Discharged by: V20, S-31.
 *   LG-3  The first liveness row lands one interval after the role goes live (tick-then-sleep), and its streak is 1.
 *         Discharged by: S-31, S-32.
 *   LG-4  A committed Stable claim is never dropped while the allocation is Running (no later writer of a Running row
 *         drops it), so `committed` is monotone and D12 can read it. The model does not model eligibility.
 *         Discharged by: V31, S-43.
 *
 * D-UP-1: `conf_step` as in probe-count. Conformance projection: role is ProbeRunner::is_role_live(alloc, Liveness)
 * through Driver::on_alloc_running / on_alloc_stable / on_alloc_terminal; sup is ProbeRunner::active_alloc_count();
 * liveness_stop enabled is the StopAllocation { Stopped { LivenessProbe } } action of ServiceLifecycleReconciler::reconcile;
 * committed is the committed Stable row (ServiceAllocFact.stable_committed hydrated from the allocation's row; the
 * presence of FinalizeFailed { Stable } in the returned actions is the emission); swr is the write outcome the driver scripts on
 * SimObservationStore; streak, since, ret, hook, ended, run are model-only or inputs the driver supplies.
 */
module liveness_gate_conf {
  type ConfStep =
    | init
    | OnRunning
    | StableCommit({ w: str })
    | OnStable
    | LivenessRow({ res: str })
    | LivenessStop
    | Crash
    | OnTerminal
    | Idle
}

module liveness_gate {
  import liveness_gate_conf.ConfStep from "./liveness_gate"
  const OFF: Set[str]
  const REC: bool

  val LT: int = 2       // liveness failure_threshold
  val SMAX: int = 4     // bound on liveness rows
  val BUG_SPAWN: bool = OFF.contains("spawnAtRunning")
  val BUG_STAMP: bool = OFF.contains("stampAtEmission")

  type Lv = {
    rw: bool, started: bool, sup: bool, ret: bool, role: str, committed: bool, swr: str, hook: bool, ended: bool,
    streak: int, since: int, rows: int, stopBeforeStable: bool,
  }
  var lv: Lv
  var conf_step: ConfStep

  action init = all {
    lv' = { rw: false, started: false, sup: false, ret: false, role: "absent", committed: false, swr: "none", hook: false,
            ended: false, streak: 0, since: 0, rows: 0, stopBeforeStable: false },
    conf_step' = variant("init", ()),
  }

  // on_alloc_running: register the supervisor, retain the liveness descriptors. D9: the liveness tasks are not spawned.
  action OnRunning = all {
    not(lv.started),
    lv' = { ...lv, started: true, rw: true, sup: true, ret: true, role: if (BUG_SPAWN) "live" else "absent" },
    conf_step' = if (REC) variant("OnRunning", ()) else conf_step,
  }

  // Stable emission is enabled for a Running allocation whose startup condition holds. B1 (ADR-0258): keyed on the
  // committed row, so a Stable whose write failed is emitted again. The hazard emits only if no Stable write has been
  // attempted (the stamp-at-emission rule on the write-outcome variable).
  def emitEnabled: bool = lv.rw and (if (BUG_STAMP) lv.swr == "none" else not(lv.committed))
  // ServiceLifecycle emits Stable and the shim writes the row; the write lands or fails (w)
  action StableCommit(w: str): bool = all {
    emitEnabled,
    lv' = { ...lv, committed: lv.committed or w == "landed", swr: w },
    conf_step' = if (REC) variant("StableCommit", { w: w }) else conf_step,
  }

  // Driver::on_alloc_stable (fired after the Stable row is committed): retire the startup role, start liveness iff the
  // allocation is still supervised. Idempotent; a no-op for an ended allocation.
  action OnStable = all {
    lv.committed, not(lv.hook),
    lv' = { ...lv, hook: true, role: if (lv.sup and lv.ret) "live" else lv.role },
    conf_step' = if (REC) variant("OnStable", ()) else conf_step,
  }

  // a liveness probe row (only while the role is live)
  action LivenessRow(res: str): bool = all {
    lv.role == "live", lv.sup, lv.rows < SMAX,
    lv' = { ...lv, rows: lv.rows + 1, since: if (lv.committed) lv.since + 1 else lv.since,
            streak: if (res == "fail") (if (lv.streak < LT) lv.streak + 1 else LT) else 0 },
    conf_step' = if (REC) variant("LivenessRow", { res: res }) else conf_step,
  }

  def stopEnabled: bool = lv.rw and lv.streak >= LT
  // ServiceLifecycle's liveness stop: a Running allocation with a crossed streak
  action LivenessStop = all {
    stopEnabled,
    lv' = { ...lv, rw: false, stopBeforeStable: lv.stopBeforeStable or not(lv.committed) },
    conf_step' = if (REC) variant("LivenessStop", ()) else conf_step,
  }

  // the guest crashes
  action Crash = all {
    lv.rw,
    lv' = { ...lv, rw: false },
    conf_step' = if (REC) variant("Crash", ()) else conf_step,
  }

  // Driver::on_alloc_terminal (the observer after a crash, the shim after a stop): remove the supervisor
  action OnTerminal = all {
    lv.started, not(lv.rw), not(lv.ended),
    lv' = { ...lv, ended: true, sup: false, ret: false, role: "absent" },
    conf_step' = if (REC) variant("OnTerminal", ()) else conf_step,
  }

  action Idle = all { lv' = lv, conf_step' = if (REC) variant("Idle", ()) else conf_step }

  action step = {
    nondet res = Set("fail", "pass").oneOf()
    nondet w = Set("landed", "failed").oneOf()
    any { OnRunning, StableCommit(w), OnStable, LivenessRow(res), LivenessStop, Crash, OnTerminal, Idle }
  }

  // ------------------------------------------------------------------ invariants
  // D9 LivenessOnlyAfterStable (restated on the committed row, B1): the liveness role is live only for an id whose row
  // carries Stable
  val LivenessOnlyAfterStable: bool = lv.role == "live" implies lv.committed
  // D9 NoLivenessStopBeforeStable: a liveness stop is never enabled for an id whose row does not carry Stable
  val NoLivenessStopBeforeStable: bool = and { not(lv.stopBeforeStable), stopEnabled implies lv.committed }
  // D9 NoLivenessForEndedId: the liveness role is never live for an id whose supervisor was removed
  val NoLivenessForEndedId: bool = lv.role == "live" implies (lv.sup and not(lv.ended))
  // D9 LivenessStreakStartsAtStable: the streak is at most the number of liveness rows written since Stable
  val LivenessStreakStartsAtStable: bool = lv.streak <= lv.since
  // B1 StableEventuallyCommitted (progress under fairness): a Running allocation with its supervisor present and no
  // committed Stable is eventually committed (or stops being Running or supervised). Fairness: a Stable write that can
  // land lands.
  def pendingStable: bool = lv.rw and lv.sup and not(lv.committed)
  temporal StableEventuallyCommitted =
    weakFair(StableCommit("landed"), (lv, conf_step)).implies(always(pendingStable.implies(eventually(not(pendingStable)))))
  val Safety: bool = and { LivenessOnlyAfterStable, NoLivenessStopBeforeStable, NoLivenessForEndedId, LivenessStreakStartsAtStable }

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // D9: the liveness probe would have failed from the start; before Stable no liveness row exists, no stop is enabled and
  // the streak starts at Stable
  run liveness_failing_during_startup_is_not_stopped = init
    .then(OnRunning)
    .expect(lv.role == "absent" and lv.ret and lv.sup and not(stopEnabled) and Safety)
    .then(StableCommit("landed")).then(OnStable)
    .expect(lv.role == "live" and lv.streak == 0 and lv.since == 0 and not(stopEnabled) and Safety)
    .then(LivenessRow("fail")).expect(lv.streak == 1 and lv.since == 1 and not(stopEnabled) and Safety)

  // D9: liveness starts at Stable and stops the instance after the threshold of post-Stable failures
  run liveness_starts_at_stable_and_stops_on_failure = init
    .then(OnRunning).then(StableCommit("landed")).then(OnStable)
    .then(LivenessRow("fail")).then(LivenessRow("fail"))
    .expect(lv.streak == LT and stopEnabled and Safety)
    .then(LivenessStop).expect(not(lv.rw) and not(lv.stopBeforeStable) and Safety)
    .then(OnTerminal).expect(lv.role == "absent" and not(lv.sup) and Safety)

  // D9: the supervisor was removed before on_alloc_stable fired: it starts no liveness
  run stable_after_end_starts_no_liveness = init
    .then(OnRunning).then(StableCommit("landed")).then(Crash).then(OnTerminal)
    .then(OnStable)
    .expect(lv.role == "absent" and not(lv.sup) and lv.ended and Safety)

  // B1 (ADR-0258): the Stable write fails; emission is keyed on the committed row, so the next evaluation emits Stable again;
  // once the write lands the hook fires and liveness starts; a committed Stable is not emitted again
  run lost_stable_write_is_reannounced = init
    .then(OnRunning).then(StableCommit("failed"))
    .expect(not(lv.committed) and lv.swr == "failed" and emitEnabled and lv.role == "absent" and not(stopEnabled) and Safety)
    .then(StableCommit("landed"))
    .expect(lv.committed and lv.swr == "landed" and not(emitEnabled) and lv.role == "absent" and Safety)
    .then(OnStable)
    .expect(lv.hook and lv.role == "live" and not(emitEnabled) and Safety)

  // D9: an opt-out Service's first Running is its Stable, so its liveness starts at that moment (no special case)
  run opt_out_service_starts_liveness_at_first_running_stable = init
    .then(OnRunning).expect(lv.role == "absent")
    .then(StableCommit("landed")).then(OnStable)
    .expect(lv.role == "live" and lv.since == 0 and Safety)
}

module lg_ok { import liveness_gate(OFF = Set(), REC = false).* from "./liveness_gate" }
module lg_conf { import liveness_gate(OFF = Set(), REC = true).* from "./liveness_gate" }
