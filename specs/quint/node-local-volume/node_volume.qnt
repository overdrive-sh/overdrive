/**
 * Node-local persistent volume protocol (GH #348; decisions VD-2 (ADR-0311), VD-3 (ADR-0312, backing choice: no
 * protocol rule), VD-8 (ADR-0317), VD-9 (ADR-0318), SD-1 to SD-5 (ADR-0325, one decision)).
 *
 * ONE node, ONE workload (`replicas = 1`) with one declared volume, NA allocation ids (each minted once).
 * The volume size is abstracted away: "free >= size" is the boolean `spaceFree`.
 *
 * Owners and what they decide:
 *   volume store             at most one logical volume for the workload: `lv` (exists), `active` (activated: false
 *                            after creation until `ensure` activates it, and after a node reboot), identity `vid`,
 *                            the guest's data as `dv` (model-only `data_version`, bumped by guest writes);
 *                            `storeUp` (store composed or absent), `spaceFree` (free-extent availability).
 *   kernel claim (VD-8)      `claim`: 0 free, a > 0 held by the VMM of allocation a. A killed VMM's claim lingers
 *                            until `Drain` (spike B2b): the claim outlives the process. `claim_state` is the arbiter.
 *   launch path (VD-8, VD-9) per allocation, control-plane memory (`ph`, `waited`): supervision claimed, then
 *                            `ensure` = create-if-absent (one effect) then activate (a second effect), a control-plane
 *                            crash can interrupt after each; then poll the claim up to BOUND poll steps, then attach by
 *                            spawning the VMM, which takes the claim. A failed `ensure` is a typed VolumeUnavailable;
 *                            the bound passing is a typed VolumeHeld; both are ordinary start failures that charge the
 *                            restart budget (LIMIT attempts, then BackoffExhausted).
 *   workload-lifecycle       mints a successor only when no allocation row is non-terminal (ADR-0109); it does not wait
 *                            for cleanup or claim release (ADR-0106). Operator stop, `workload restart`.
 *
 * Lane signal (SD-1..SD-5, ADR-0325): a PENDING allocation stands for "a launch holds the lane" (production writes no row
 * before `driver.start` returns). `sup[a]` is raised by OpStop, WorkloadRestart (on a PENDING allocation) and Shutdown
 * (`down`); a launch that observes it abandons (`LaunchSuperseded`): no row result, no charge, the VMM terminated, the
 * volume kept. `ensure` (`LaunchEnsureActivate`) and the VMM create are never interrupted. VmmSpawn leaves the allocation
 * PENDING (LP_BOOT); GuestReady writes the Running row. A restart commit is the signal's producer and nothing more: this
 * spec has NO restart generation. That a superseded or lost launch is followed by exactly one placement for the
 * generation is assumption A10, owned by specs/quint/restart-generation/; here the successor of an abandoned launch is
 * placed by `WlStart`'s ordinary rule.
 *
 * Environment actions (faults, each draws on the budget FB): VmmSigkill, DeviceSlow (the dead VMM's in-flight I/O
 * outlasts the launch bound), LaunchCmdFail (an `lvm` command fails), CpCrash (control plane crash: in-flight launch
 * state is lost, an already-spawned VMM keeps running until `Reclaim`), NodeReboot (volume inactive, every VMM gone,
 * claims released, launch state lost). StoreAbsentBoot and PoolFull are boot-time substrate facts (OPS). `Shutdown` is
 * the node beginning graceful shutdown (an operator-gated event); `Boot` is its next boot. `dstart` bounds `WlStart`
 * while `down` to the one evaluation admitted before the close.
 *
 * Instances: vol_ok (exhaustive safety), vol_conf (named runs, REC=true), vol_live_start (liveness), hazards under hazard/.
 *
 * Assumptions: see README.md (A1..A9 of the feature delta, and the modelling assumptions M1..M7, M9).
 */
module node_volume_conf {
  type ConfStep =
    | Init
    | Declare | OpStop | OpResume | StoreAbsentBoot | PoolFull
    | Shutdown | Boot
    | WorkloadRestart({ a: int }) | WlStart({ a: int }) | StopAlloc({ a: int })
    | LaunchEnsureCreate({ a: int }) | LaunchEnsureActivate({ a: int }) | LaunchUnavailable({ a: int })
    | LaunchCmdFail({ a: int }) | LaunchCheck({ a: int }) | LaunchPoll({ a: int }) | LaunchHeldFail({ a: int })
    | LaunchSuperseded({ a: int }) | GuestReady({ a: int }) | VmmSpawn({ a: int }) | VmmSpawnFail({ a: int })
    | GuestWrite | VmmSigkill({ a: int }) | DeviceSlow | Drain | Reclaim({ a: int }) | CpCrash | NodeReboot
}

module node_volume {
  import node_volume_conf.ConfStep from "./node_volume"
  const OFF: Set[str]   // rules off: "noWait" (VD-8) "recreate" (VD-9) "stopDeletes" (VD-2) "noInterrupt" (SD-1..SD-5)
  const REC: bool       // true on the runs' main: conf_step records the action taken
  const OPS: Set[str]   // enabled operator / boot-time actions: "stop" "restart" "shutdown" "storeAbsent" "poolFull"
  const NA: int         // allocation ids 1..NA (an id is minted once)
  const FB: int         // fault budget (VmmSigkill, DeviceSlow, LaunchCmdFail, CpCrash, NodeReboot together)
  const LIMIT: int      // restart budget: failed attempts before BackoffExhausted (design: 5; abstracted)
  const DVMAX: int      // bound on guest writes (data_version)

  val AL = 1.to(NA)
  val BOUND = 2         // polls of the launch wait before VolumeHeld (time abstracted: 30 s)

  // allocation rows
  val UNMINTED = 0
  val PENDING = 1
  val RUNNING = 2
  val FAILED = 3
  val TERMINATED = 4
  // why an allocation ended
  val W_NONE = 0
  val W_CRASH = 1       // VMM killed, reclaimed, or lost to a node reboot
  val W_HELD = 2        // VolumeHeld
  val W_UNAVAIL = 3     // VolumeUnavailable
  val W_OPENLOST = 4    // the VMM's own open lost
  val W_STOP = 5
  val W_RESTART = 6
  val W_LOST = 7        // launch lost to a control-plane crash
  val W_SUPERSEDED = 8  // the launch observed the lane signal and abandoned; no row result, no charge
  // launch phase (control-plane memory)
  val LP_NONE = 0
  val LP_SUP = 1        // supervision claimed
  val LP_CREATED = 2    // ensure: create-if-absent done
  val LP_ENSURED = 3    // ensure: activated
  val LP_FREE = 4       // claim_state reported Free
  val LP_BOOT = 5       // VMM spawned, claim held, guest not ready, no row

  type S = {
    declared: bool, stopped: bool, storeUp: bool, spaceFree: bool,
    lv: bool, active: bool, vid: int, dv: int, maxDv: int, nCreated: int,
    claim: int,
    as: int -> int, alive: int -> bool, orphan: int -> bool, why: int -> int, ran: int -> bool,
    ph: int -> int, waited: int -> int,
    charged: int, faults: int, slow: bool,
    drainFail: bool,      // ghost: a launch failed (charged) before the bound although only a draining claim was in the way
    sup: int -> bool,     // the lane signal for allocation a's launch (PENDING stands for "a launch holds the lane")
    afterSup: int -> int, // ghost: steps of a launch that continued a wait or produced a result after sup[a] was true
    delivered: int -> bool, // ghost: a Running row was written for a
    chg: int -> int,      // ghost: budget charges attributable to a
    down: bool,           // SD-5: the node began shutdown (the lane table's shutdown token is cancelled)
    dstart: int           // SD-5: placements taken while `down` (bounded to the one evaluation admitted before the close)
  }

  var st: S
  var conf_step: ConfStep   // model-only

  val cur: S = st

  action commit(n: S, t: ConfStep): bool = all {
    st' = n,
    conf_step' = if (REC) t else conf_step,
  }

  action init = all {
    st' = {
      declared: false, stopped: false, storeUp: true, spaceFree: true,
      lv: false, active: false, vid: 0, dv: 0, maxDv: 0, nCreated: 0, claim: 0,
      as: AL.mapBy(a => UNMINTED), alive: AL.mapBy(a => false), orphan: AL.mapBy(a => false),
      why: AL.mapBy(a => W_NONE), ran: AL.mapBy(a => false), ph: AL.mapBy(a => LP_NONE), waited: AL.mapBy(a => 0),
      charged: 0, faults: FB, slow: false, drainFail: false,
      sup: AL.mapBy(a => false), afterSup: AL.mapBy(a => 0), delivered: AL.mapBy(a => false), chg: AL.mapBy(a => 0),
      down: false, dstart: 0
    },
    conf_step' = variant("Init", ()),
  }

  // ---------------------------------------------------------------- helpers
  def nonTerminal(s: S): bool = AL.exists(a => s.as.get(a) == PENDING or s.as.get(a) == RUNNING)
  def deadHolder(s: S): bool = s.claim > 0 and not(s.alive.get(s.claim))
  def isPending(s: S, a: int): bool = s.as.get(a) == PENDING
  def exhausted(s: S): bool = s.charged >= LIMIT and not(nonTerminal(s))

  // a failed launch: the allocation is Pending (a launch holds the lane) and the failure is charged to the budget. A
  // superseded launch never reaches here: its guards require the signal down (superseded wins at the same point).
  def failLaunch(s: S, a: int, why: int, drain: bool): S =
    { ...s, as: s.as.set(a, FAILED), why: s.why.set(a, why), charged: s.charged + 1,
      chg: s.chg.set(a, s.chg.get(a) + 1),
      ph: s.ph.set(a, LP_NONE), drainFail: s.drainFail or drain }
  // SD observation guard: a launch step that continues a wait or produces a result needs the signal down. Hazard
  // "noInterrupt" removes the guard.
  def sg(s: S, a: int): bool = not(s.sup.get(a)) or OFF.contains("noInterrupt")
  // ghost: a step taken while the signal is up counts against I_SUPERSEDE_ABANDONS_WAITS
  def bump(s: S, a: int): int -> int = s.afterSup.set(a, s.afterSup.get(a) + (if (s.sup.get(a)) 1 else 0))
  def earlyDrain(s: S, a: int): bool = deadHolder(s) and not(s.slow) and s.waited.get(a) < BOUND

  // ---------------------------------------------------------------- operator / boot-time facts
  action Declare = all {
    not(cur.declared),
    commit({ ...cur, declared: true }, variant("Declare", ())),
  }
  action OpStop = all {
    OPS.contains("stop"), cur.declared, not(cur.stopped),
    commit({ ...cur, stopped: true, sup: AL.mapBy(a => cur.sup.get(a) or cur.as.get(a) == PENDING) }, variant("OpStop", ())),
  }
  // `workload restart` of a stopped workload: only clears the stop (this spec has no generation)
  action OpResume = all {
    OPS.contains("stop"), cur.stopped,
    commit({ ...cur, stopped: false }, variant("OpResume", ())),
  }
  // a node booted without the labelled partition: only before anything was created
  action StoreAbsentBoot = all {
    OPS.contains("storeAbsent"), cur.storeUp, cur.nCreated == 0, AL.forall(a => cur.as.get(a) == UNMINTED),
    commit({ ...cur, storeUp: false }, variant("StoreAbsentBoot", ())),
  }
  // the volume pool has no free extent: only before the volume was created
  action PoolFull = all {
    OPS.contains("poolFull"), cur.spaceFree, cur.nCreated == 0, AL.forall(a => cur.as.get(a) == UNMINTED),
    commit({ ...cur, spaceFree: false }, variant("PoolFull", ())),
  }

  // ---------------------------------------------------------------- workload-lifecycle
  def canStart(s: S, a: int): bool = and {
    s.declared, not(s.stopped), s.charged < LIMIT, not(nonTerminal(s)),
    // M7: the boot sweep reclaims a VMM that survived a control-plane crash before any launch starts (feature delta,
    // boundary scenario 5); a Running orphan already blocked minting through its non-terminal row, a booting one does not
    AL.forall(x => not(s.orphan.get(x))),
    s.as.get(a) == UNMINTED, AL.forall(x => s.as.get(x) != UNMINTED or a <= x),
  }
  // the successor is minted as soon as the predecessor row is terminal: no wait for cleanup or claim release. SD-5: the
  // evaluation's lane opens here; a lane opened after shutdown began is born raised (sup = down). Stays enabled while
  // `down`: it stands for an evaluation admitted before admission closed whose tick had not opened its lane, bounded to
  // one such evaluation by `dstart`.
  def canWlStart(s: S, a: int): bool = canStart(s, a) and (not(s.down) or s.dstart < 1)
  action WlStart(a: int): bool = all {
    canWlStart(cur, a),
    commit({ ...cur, as: cur.as.set(a, PENDING), ph: cur.ph.set(a, LP_SUP), sup: cur.sup.set(a, cur.down),
             dstart: cur.dstart + (if (cur.down) 1 else 0) }, variant("WlStart", { a: a })),
  }
  // operator stop ends a Running allocation (production emits StopAllocation only for Running rows); the VMM dies, the
  // claim lingers until Drain. The volume stays (VD-2). A Pending allocation (a launch holds the lane) is ended by its
  // own launch observing the signal, not here.
  action StopAlloc(a: int): bool = all {
    cur.as.get(a) == RUNNING, cur.stopped,
    commit({ ...cur, as: cur.as.set(a, TERMINATED), why: cur.why.set(a, W_STOP), alive: cur.alive.set(a, false),
             orphan: cur.orphan.set(a, false),
             lv: cur.lv and not(OFF.contains("stopDeletes")), active: cur.active and not(OFF.contains("stopDeletes")),
             dv: if (OFF.contains("stopDeletes")) 0 else cur.dv }, variant("StopAlloc", { a: a })),
  }
  // `workload restart`. A Running allocation ends (the shim stops it, ADR-0106). For a Pending one the handler raises the
  // lane signal and changes no row: the launch abandons itself (LaunchSuperseded).
  action WorkloadRestart(a: int): bool = all {
    OPS.contains("restart"), cur.declared, not(cur.stopped),
    any {
      all {
        cur.as.get(a) == RUNNING,
        commit({ ...cur, as: cur.as.set(a, TERMINATED), why: cur.why.set(a, W_RESTART), alive: cur.alive.set(a, false),
                 orphan: cur.orphan.set(a, false) }, variant("WorkloadRestart", { a: a })),
      },
      all {
        cur.as.get(a) == PENDING,
        commit({ ...cur, sup: cur.sup.set(a, true) }, variant("WorkloadRestart", { a: a })),
      },
    },
  }
  // SD-5: node shutdown. One atomic action: the single cancel() of the parent token, so no interleaving separates "which
  // launches are pending" from "signal them". Nothing else changes (rows, volume, claim and budget untouched). An
  // environment event gated by OPS; the next boot is `Boot` (or CpCrash / NodeReboot, which also clear it).
  action Shutdown = all {
    OPS.contains("shutdown"), not(cur.down),
    commit({ ...cur, down: true,
             sup: AL.mapBy(a => cur.sup.get(a) or cur.as.get(a) == PENDING), dstart: 0 },
      variant("Shutdown", ())),
  }
  // the next boot after a graceful shutdown (weakly fair): `down` ends, the lane signals end, and every launch the shutdown left
  // unfinished is lost exactly as for a control-plane crash (the CpCrash loss, without a fault)
  action Boot = all {
    cur.down,
    commit({ ...crashLoss(cur), down: false, sup: AL.mapBy(a => false), dstart: 0 }, variant("Boot", ())),
  }


  // ---------------------------------------------------------------- launch path (VD-9 ensure, VD-8 claim wait)
  // ensure, step 1: create-if-absent (observe, then create only when missing). Hazard "recreate" (VD-9): creates
  // unconditionally, replacing an existing volume with a blank one.
  action LaunchEnsureCreate(a: int): bool = all {
    cur.ph.get(a) == LP_SUP, sg(cur, a), cur.storeUp, or { cur.lv, cur.spaceFree, OFF.contains("recreate") },
    val fresh = not(cur.lv) or OFF.contains("recreate")
    commit({ ...cur, lv: true, active: if (fresh) false else cur.active,
             vid: if (fresh) cur.nCreated + 1 else cur.vid, dv: if (fresh) 0 else cur.dv,
             nCreated: if (fresh) cur.nCreated + 1 else cur.nCreated, ph: cur.ph.set(a, LP_CREATED),
             afterSup: bump(cur, a) },
      variant("LaunchEnsureCreate", { a: a })),
  }
  // ensure, step 2: activate. Unguarded by the signal: ensure runs to its end (observation point 2 follows it).
  action LaunchEnsureActivate(a: int): bool = all {
    cur.ph.get(a) == LP_CREATED, cur.lv,
    commit({ ...cur, active: true, ph: cur.ph.set(a, LP_ENSURED) }, variant("LaunchEnsureActivate", { a: a })),
  }
  // typed VolumeUnavailable: no store composed, no free space for a missing volume, or the volume vanished (NotFound
  // maps to a command failure). Nothing is created.
  action LaunchUnavailable(a: int): bool = all {
    sg(cur, a),
    or {
      and { cur.ph.get(a) == LP_SUP, or { not(cur.storeUp), and { not(cur.lv), not(cur.spaceFree), not(OFF.contains("recreate")) } } },
      and { cur.ph.get(a) == LP_CREATED or cur.ph.get(a) == LP_ENSURED, not(cur.lv) },
    },
    commit({ ...failLaunch(cur, a, W_UNAVAIL, false), afterSup: bump(cur, a) }, variant("LaunchUnavailable", { a: a })),
  }
  // environment: an `lvm` command fails mid-ensure (typed VolumeUnavailable); an already created volume stays
  action LaunchCmdFail(a: int): bool = all {
    cur.ph.get(a) == LP_SUP or cur.ph.get(a) == LP_CREATED, sg(cur, a), cur.faults > 0,
    commit({ ...failLaunch(cur, a, W_UNAVAIL, false), faults: cur.faults - 1 }, variant("LaunchCmdFail", { a: a })),
  }
  // claim_state reports Free. Hazard "noWait" (VD-8): the launch treats the VMM process exit as the claim free.
  def checkPasses(s: S, a: int): bool = and {
    s.ph.get(a) == LP_ENSURED, s.lv, s.active,
    s.claim == 0 or (OFF.contains("noWait") and deadHolder(s)),
  }
  action LaunchCheck(a: int): bool = all {
    checkPasses(cur, a),
    sg(cur, a),
    commit({ ...cur, ph: cur.ph.set(a, LP_FREE), afterSup: bump(cur, a) }, variant("LaunchCheck", { a: a })),
  }
  // the claim is held: poll. Time passes only when the holder is not a draining dead VMM (a healthy drain finishes
  // within the bound, M3), or after DeviceSlow
  def canPoll(s: S, a: int): bool = and {
    s.ph.get(a) == LP_ENSURED, s.lv, s.claim != 0, s.waited.get(a) < BOUND, s.slow or not(deadHolder(s)),
  }
  action LaunchPoll(a: int): bool = all {
    canPoll(cur, a), sg(cur, a), not(OFF.contains("noWait")),
    commit({ ...cur, waited: cur.waited.set(a, cur.waited.get(a) + 1), afterSup: bump(cur, a) }, variant("LaunchPoll", { a: a })),
  }
  // VolumeHeld: the bound passed. M4: a launch that waited the whole bound ends the stuck-drain condition.
  action LaunchHeldFail(a: int): bool = all {
    cur.ph.get(a) == LP_ENSURED, cur.lv, cur.claim != 0, cur.waited.get(a) >= BOUND, sg(cur, a),
    commit({ ...failLaunch(cur, a, W_HELD, earlyDrain(cur, a)), slow: false, afterSup: bump(cur, a) }, variant("LaunchHeldFail", { a: a })),
  }
  // SD-1 / SD-3: the launch observed the lane signal at an observation point (points 0, 1, 2, 3, 4, 5, 6) and abandons:
  // the VMM, if spawned, is terminated; its claim lingers until Drain; no row result, no budget charge. Not at
  // LP_CREATED: ensure is one uninterruptible call (the signal is observed after it returns, point 2).
  def canSupersede(s: S, a: int): bool = and {
    isPending(s, a), s.sup.get(a), not(OFF.contains("noInterrupt")),
    s.ph.get(a) == LP_SUP or s.ph.get(a) == LP_ENSURED or s.ph.get(a) == LP_FREE or s.ph.get(a) == LP_BOOT,
  }
  action LaunchSuperseded(a: int): bool = all {
    canSupersede(cur, a),
    commit({ ...cur, as: cur.as.set(a, TERMINATED), why: cur.why.set(a, W_SUPERSEDED), ph: cur.ph.set(a, LP_NONE),
             sup: cur.sup.set(a, false), alive: cur.alive.set(a, false), orphan: cur.orphan.set(a, false) },
      variant("LaunchSuperseded", { a: a })),
  }
  // attach: spawn the VMM, which takes the claim (Cloud Hypervisor's O_EXCL open is the atomic verdict)
  def canSpawn(s: S, a: int): bool = and {
    isPending(s, a), s.ph.get(a) == LP_FREE, s.lv, s.active, s.claim == 0, sg(s, a),
  }
  def canGuestReady(s: S, a: int): bool = and { isPending(s, a), s.ph.get(a) == LP_BOOT, s.alive.get(a), sg(s, a) }
  // the allocation stays Pending (no row): the VMM is spawned and holds the claim, the guest is booting (LP_BOOT)
  action VmmSpawn(a: int): bool = all {
    canSpawn(cur, a),
    commit({ ...cur, claim: a, alive: cur.alive.set(a, true), ran: cur.ran.set(a, true),
             ph: cur.ph.set(a, LP_BOOT), afterSup: bump(cur, a) }, variant("VmmSpawn", { a: a })),
  }
  action VmmSpawnFail(a: int): bool = all {
    isPending(cur, a), cur.ph.get(a) == LP_FREE, sg(cur, a), or { not(cur.lv), not(cur.active), cur.claim != 0 },
    commit({ ...failLaunch(cur, a, W_OPENLOST, earlyDrain(cur, a)), afterSup: bump(cur, a) }, variant("VmmSpawnFail", { a: a })),
  }
  // the guest reported ready (accept_ready selected the boot race before the signal was observed): the Running row is
  // written. Superseded wins at the poll, so a signalled launch never gets here.
  action GuestReady(a: int): bool = all {
    canGuestReady(cur, a),
    commit({ ...cur, as: cur.as.set(a, RUNNING), delivered: cur.delivered.set(a, true), ph: cur.ph.set(a, LP_NONE),
             afterSup: bump(cur, a) }, variant("GuestReady", { a: a })),
  }

  // ---------------------------------------------------------------- guest and kernel
  // the guest writes (or a dead VMM's in-flight write lands, spike B2b) while a claim is held
  action GuestWrite = all {
    cur.lv, cur.claim > 0, cur.maxDv < DVMAX,
    commit({ ...cur, dv: cur.dv + 1, maxDv: cur.maxDv + 1 }, variant("GuestWrite", ())),
  }
  // environment: the VMM is killed; the allocation is Failed at once, the claim lingers (A3)
  action VmmSigkill(a: int): bool = all {
    cur.as.get(a) == RUNNING, cur.alive.get(a), cur.faults > 0,
    commit({ ...cur, as: cur.as.set(a, FAILED), why: cur.why.set(a, W_CRASH), alive: cur.alive.set(a, false),
             orphan: cur.orphan.set(a, false), charged: cur.charged + 1, chg: cur.chg.set(a, cur.chg.get(a) + 1),
             faults: cur.faults - 1 },
      variant("VmmSigkill", { a: a })),
  }
  // environment: the dead VMM's in-flight I/O outlasts the launch bound
  action DeviceSlow = all {
    cur.faults > 0, not(cur.slow), deadHolder(cur),
    commit({ ...cur, slow: true, faults: cur.faults - 1 }, variant("DeviceSlow", ())),
  }
  // the in-flight I/O drained: the claim is released
  action Drain = all {
    deadHolder(cur),
    commit({ ...cur, claim: 0, slow: false }, variant("Drain", ())),
  }
  // the boot reclamation sweep (M7) kills a VMM that survived a control-plane crash; its claim drains. A Running row fails
  // (charged). A booting VMM (LP_BOOT) whose launch was lost to the crash is marked Failed(W_LOST) with no row and no
  // charge (M3): the sweep only kills it (DiscardStrandedArtifacts), so it charges nothing either.
  action Reclaim(a: int): bool = all {
    cur.alive.get(a), cur.orphan.get(a),
    any {
      all {
        cur.as.get(a) == RUNNING,
        commit({ ...cur, as: cur.as.set(a, FAILED), why: cur.why.set(a, W_CRASH), alive: cur.alive.set(a, false),
                 orphan: cur.orphan.set(a, false), charged: cur.charged + 1, chg: cur.chg.set(a, cur.chg.get(a) + 1) },
          variant("Reclaim", { a: a })),
      },
      all {
        cur.as.get(a) != RUNNING,
        commit({ ...cur, alive: cur.alive.set(a, false), orphan: cur.orphan.set(a, false) }, variant("Reclaim", { a: a })),
      },
    },
  }
  // environment: control plane crash. Every in-flight launch is lost (a Pending allocation ends Failed(W_LOST)), a spawned
  // VMM keeps running (orphan) until Reclaim; the volume and its data persist.
  // the loss a control-plane death inflicts on in-flight launches (shared with `Boot`). A lost launch has NO row in
  // production, so it charges nothing (review finding M3): `charged` and `chg` are unchanged.
  def crashLoss(s: S): S =
    { ...s, ph: AL.mapBy(a => LP_NONE),
      as: AL.mapBy(a => if (s.as.get(a) == PENDING) FAILED else s.as.get(a)),
      why: AL.mapBy(a => if (s.as.get(a) == PENDING) W_LOST else s.why.get(a)),
      orphan: AL.mapBy(a => s.alive.get(a) or s.orphan.get(a)) }
  action CpCrash = all {
    cur.faults > 0,
    commit({ ...crashLoss(cur), sup: AL.mapBy(a => false), down: false, dstart: 0, faults: cur.faults - 1 },
      variant("CpCrash", ())),
  }

  // environment: node reboot. The volume is inactive, every VMM is gone and its claim released, launch state is lost,
  // the allocation that was Running fails (W_CRASH, a Failed row: charged); one that was Pending is a lost launch (W_LOST, no
  // row: not charged, M3).
  action NodeReboot = all {
    cur.faults > 0,
    commit({ ...cur, active: false, claim: 0, slow: false, ph: AL.mapBy(a => LP_NONE),
             alive: AL.mapBy(a => false), orphan: AL.mapBy(a => false),
             as: AL.mapBy(a => if (cur.as.get(a) == PENDING or cur.as.get(a) == RUNNING) FAILED else cur.as.get(a)),
             why: AL.mapBy(a => if (cur.as.get(a) == PENDING) W_LOST else if (cur.as.get(a) == RUNNING) W_CRASH else cur.why.get(a)),
             charged: cur.charged + AL.filter(a => cur.as.get(a) == RUNNING).size(),
             chg: AL.mapBy(a => cur.chg.get(a) + (if (cur.as.get(a) == RUNNING) 1 else 0)),
             sup: AL.mapBy(a => false), down: false, dstart: 0, faults: cur.faults - 1 }, variant("NodeReboot", ())),
  }

  // ---------------------------------------------------------------- composition
  // everything the system does on its own (not the operator's actions, not the guest, not faults)
  action Progress = {
    nondet a = AL.oneOf()
    any { WlStart(a), StopAlloc(a), LaunchEnsureCreate(a), LaunchEnsureActivate(a), LaunchUnavailable(a),
          LaunchCheck(a), LaunchPoll(a), LaunchHeldFail(a), LaunchSuperseded(a), VmmSpawn(a), GuestReady(a),
          VmmSpawnFail(a), Reclaim(a), Drain, Boot }
  }
  action step = {
    nondet a = AL.oneOf()
    any { Declare, OpStop, OpResume, Shutdown, StoreAbsentBoot, PoolFull, WorkloadRestart(a), GuestWrite, VmmSigkill(a),
          DeviceSlow, LaunchCmdFail(a), CpCrash, NodeReboot, Progress }
  }

  // ---------------------------------------------------------------- invariants (each named after its decision)
  def vmms(s: S): Set[int] = AL.filter(a => s.alive.get(a))
  // VD-8: never two VMMs attached to the volume; the kernel claim is the arbiter (a live VMM holds the claim)
  val I_SINGLE_WRITER = and {
    vmms(st).size() <= 1,
    vmms(st).forall(a => st.claim == a),
  }
  // VD-2 / VD-9: the volume an allocation attaches is the same volume, holding the latest data, as before the previous
  // allocation ended: across crash, stop, restart and node reboot ensure never replaces it with a blank one
  val I_DATA_SURVIVES_RESTART = and {
    st.lv implies and { st.vid == 1, st.dv == st.maxDv },
    vmms(st).forall(a => st.lv and st.active),
  }
  // VD-9: a crash after create and before attach leaves a volume the next launch reuses: never a second volume
  val I_ENSURE_IDEMPOTENT_UNDER_CRASH = st.nCreated <= 1
  // VD-8: a launch whose only obstacle is a claim that drains within the bound fails neither typed nor charged
  val I_NO_DRAIN_FAILURE_CHARGES_BUDGET = not(st.drainFail)
  // VD-2: stop (and everything else in this slice) never deletes the volume
  val I_STOP_KEEPS_VOLUME = st.nCreated > 0 implies st.lv
  // SD-1: once the lane signal is up a launch takes no wait-continuing or result-producing step (zero by the guards)
  val I_SUPERSEDE_ABANDONS_WAITS = AL.forall(a => st.afterSup.get(a) == 0)
  // SD-3: a superseded launch delivers nothing: no Running row, no charge, no phase left, no VMM alive
  val I_SUPERSEDED_LAUNCH_DELIVERS_NOTHING = AL.forall(a =>
    st.why.get(a) == W_SUPERSEDED implies
      and { not(st.delivered.get(a)), st.chg.get(a) == 0, st.ph.get(a) == LP_NONE, not(st.alive.get(a)),
            st.as.get(a) == TERMINATED })
  // SD-1: a signal belongs to one allocation's launch and never outlives it (a stale signal would abandon a successor)
  val I_SIGNAL_NEVER_LEAKS_ACROSS_ALLOCATIONS = AL.forall(a => st.sup.get(a) implies st.as.get(a) == PENDING)
  val Safety = and { I_SINGLE_WRITER, I_DATA_SURVIVES_RESTART, I_ENSURE_IDEMPOTENT_UNDER_CRASH,
                     I_NO_DRAIN_FAILURE_CHARGES_BUDGET, I_STOP_KEEPS_VOLUME,
                     I_SUPERSEDE_ABANDONS_WAITS, I_SUPERSEDED_LAUNCH_DELIVERS_NOTHING,
                     I_SIGNAL_NEVER_LEAKS_ACROSS_ALLOCATIONS }
  // hazard target (VD-9): data and identity survive, one volume
  val I_VD9 = and { I_DATA_SURVIVES_RESTART, I_ENSURE_IDEMPOTENT_UNDER_CRASH }

  // ---------------------------------------------------------------- liveness (weak fairness of every system action
  // family, finite faults)
  action fWlStart = { nondet a = AL.oneOf() WlStart(a) }
  action fStopAlloc = { nondet a = AL.oneOf() StopAlloc(a) }
  action fEnsureCreate = { nondet a = AL.oneOf() LaunchEnsureCreate(a) }
  action fEnsureActivate = { nondet a = AL.oneOf() LaunchEnsureActivate(a) }
  action fUnavailable = { nondet a = AL.oneOf() LaunchUnavailable(a) }
  action fCheck = { nondet a = AL.oneOf() LaunchCheck(a) }
  action fPoll = { nondet a = AL.oneOf() LaunchPoll(a) }
  action fHeldFail = { nondet a = AL.oneOf() LaunchHeldFail(a) }
  action fSuperseded = { nondet a = AL.oneOf() LaunchSuperseded(a) }
  action fGuestReady = { nondet a = AL.oneOf() GuestReady(a) }
  action fSpawn = { nondet a = AL.oneOf() VmmSpawn(a) }
  action fSpawnFail = { nondet a = AL.oneOf() VmmSpawnFail(a) }
  action fReclaim = { nondet a = AL.oneOf() Reclaim(a) }
  temporal FairProgress = and {
    weakFair(fWlStart, (st, conf_step)), weakFair(fStopAlloc, (st, conf_step)),
    weakFair(fEnsureCreate, (st, conf_step)), weakFair(fEnsureActivate, (st, conf_step)),
    weakFair(fUnavailable, (st, conf_step)), weakFair(fCheck, (st, conf_step)), weakFair(fPoll, (st, conf_step)),
    weakFair(fHeldFail, (st, conf_step)), weakFair(fSuperseded, (st, conf_step)), weakFair(fGuestReady, (st, conf_step)),
    weakFair(fSpawn, (st, conf_step)),
    weakFair(fSpawnFail, (st, conf_step)), weakFair(fReclaim, (st, conf_step)), weakFair(Drain, (st, conf_step)),
    weakFair(Boot, (st, conf_step)),
  }
  val hasRunning = AL.exists(a => st.as.get(a) == RUNNING)
  // a declared volume with a composed store and free space eventually has a Running allocation
  temporal StartLiveness = FairProgress.implies(always(and { st.declared, st.storeUp, st.spaceFree }.implies(
    eventually(hasRunning))))
  // a replacement attaches after the predecessor's claim drains: whenever an allocation of the declared, unstopped
  // workload has ended, a Running allocation eventually exists again
  temporal ReplacementLiveness = FairProgress.implies(always(and {
    st.declared, not(st.stopped), AL.exists(a => st.as.get(a) == FAILED or st.as.get(a) == TERMINATED)
  }.implies(eventually(hasRunning))))
  temporal EveryStartLiveness = StartLiveness and ReplacementLiveness

  // ---------------------------------------------------------------- named runs (one per critical scenario)
  // prefix: allocation 1 launched up to the point its ensure is done
  action ensured1 = init.then(Declare).then(WlStart(1)).then(LaunchEnsureCreate(1)).then(LaunchEnsureActivate(1))
  // prefix: allocation 1 running with one guest write
  action running1 = ensured1.then(LaunchCheck(1)).then(VmmSpawn(1)).then(GuestReady(1)).then(GuestWrite)

  run happy_path_deploy_to_running = init.then(Declare)
    .expect(not(nonTerminal(st)) and not(st.lv))
    .then(WlStart(1))
    .expect(st.as.get(1) == PENDING and st.ph.get(1) == LP_SUP and not(st.lv))
    .then(LaunchEnsureCreate(1))
    .expect(st.lv and not(st.active) and st.nCreated == 1 and st.vid == 1 and st.claim == 0)
    .then(LaunchEnsureActivate(1))
    .expect(st.active and checkPasses(st, 1))
    .then(LaunchCheck(1)).then(VmmSpawn(1))
    .expect(st.as.get(1) == PENDING and st.ph.get(1) == LP_BOOT and st.claim == 1 and st.alive.get(1)
            and not(st.delivered.get(1)) and Safety)
    .then(GuestReady(1))
    .expect(st.as.get(1) == RUNNING and st.delivered.get(1) and st.claim == 1 and st.alive.get(1) and st.charged == 0
            and Safety)
    .then(GuestWrite)
    .expect(st.dv == 1 and st.maxDv == 1 and Safety)

  // the VMM is killed, the successor is minted at once (no wait for the claim) and waits for the drain with no failure
  // and no budget charge, then attaches the same volume with its data
  run restart_waits_for_drain_then_attaches = running1
    .then(VmmSigkill(1))
    .expect(st.as.get(1) == FAILED and st.claim == 1 and not(st.alive.get(1)) and st.charged == 1 and Safety)
    .then(WlStart(2)).then(LaunchEnsureCreate(2)).then(LaunchEnsureActivate(2))
    .expect(st.as.get(2) == PENDING and st.nCreated == 1 and not(checkPasses(st, 2)) and not(canPoll(st, 2)) and Safety)
    .then(GuestWrite)
    .then(Drain)
    .expect(checkPasses(st, 2))
    .then(LaunchCheck(2)).then(VmmSpawn(2)).then(GuestReady(2))
    .expect(st.as.get(2) == RUNNING and st.claim == 2 and st.dv == 2 and st.maxDv == 2 and st.nCreated == 1
            and st.why.get(2) == W_NONE and st.charged == 1 and not(st.drainFail) and Safety)

  run stop_keeps_volume_and_restart_reattaches = running1
    .then(OpStop).then(StopAlloc(1)).then(Drain)
    .expect(st.as.get(1) == TERMINATED and st.lv and st.nCreated == 1 and st.claim == 0 and st.dv == 1
            and not(canStart(st, 2)) and st.charged == 0 and Safety)
    .then(OpResume).then(WlStart(2)).then(LaunchEnsureCreate(2)).then(LaunchEnsureActivate(2))
    .then(LaunchCheck(2)).then(VmmSpawn(2)).then(GuestReady(2))
    .expect(st.as.get(2) == RUNNING and st.dv == 1 and st.vid == 1 and st.nCreated == 1 and Safety)

  // the control plane crashed after create and before attach: the next launch's ensure reuses the volume
  run crash_after_create_before_attach_reuses_volume = init.then(Declare).then(WlStart(1)).then(LaunchEnsureCreate(1))
    .expect(st.lv and not(st.active) and st.nCreated == 1 and st.ph.get(1) == LP_CREATED and Safety)
    .then(CpCrash)
    .expect(st.lv and st.nCreated == 1 and st.as.get(1) == FAILED and st.why.get(1) == W_LOST
            and st.ph.get(1) == LP_NONE and st.charged == 0 and st.chg.get(1) == 0 and Safety)
    .then(WlStart(2)).then(LaunchEnsureCreate(2))
    .expect(st.nCreated == 1 and st.vid == 1 and Safety)
    .then(LaunchEnsureActivate(2)).then(LaunchCheck(2)).then(VmmSpawn(2)).then(GuestReady(2))
    .expect(st.as.get(2) == RUNNING and st.nCreated == 1 and st.vid == 1 and st.active and Safety)

  // node reboot: the volume is inactive, the VMM is gone, the next launch's ensure activates the same volume and its data
  run node_reboot_reactivates_and_data_survives = running1
    .then(NodeReboot)
    .expect(st.lv and not(st.active) and st.claim == 0 and st.as.get(1) == FAILED and not(st.alive.get(1))
            and st.dv == 1 and st.charged == 1 and Safety)
    .then(WlStart(2)).then(LaunchEnsureCreate(2))
    .expect(st.nCreated == 1 and not(st.active))
    .then(LaunchEnsureActivate(2))
    .expect(st.active and checkPasses(st, 2))
    .then(LaunchCheck(2)).then(VmmSpawn(2)).then(GuestReady(2))
    .expect(st.as.get(2) == RUNNING and st.dv == 1 and st.maxDv == 1 and st.nCreated == 1 and Safety)

  // the dead VMM's I/O outlasts the bound: VolumeHeld, charged, no VMM attached; the next successor attaches after the drain
  run launch_bound_exceeded_fails_typed_and_charges_budget = running1
    .then(VmmSigkill(1)).then(WlStart(2)).then(LaunchEnsureCreate(2)).then(LaunchEnsureActivate(2)).then(DeviceSlow)
    .then(LaunchPoll(2)).then(LaunchPoll(2))
    .expect(st.as.get(2) == PENDING and st.waited.get(2) == BOUND and not(canPoll(st, 2)) and Safety)
    .then(LaunchHeldFail(2))
    .expect(st.as.get(2) == FAILED and st.why.get(2) == W_HELD and st.charged == 2 and not(st.drainFail)
            and not(st.ran.get(2)) and not(st.alive.get(2)) and st.claim == 1 and Safety)
    .then(Drain).then(WlStart(3)).then(LaunchEnsureCreate(3)).then(LaunchEnsureActivate(3))
    .then(LaunchCheck(3)).then(VmmSpawn(3)).then(GuestReady(3))
    .expect(st.as.get(3) == RUNNING and st.dv == 1 and st.nCreated == 1 and Safety)

  // no free extent: each launch fails typed and charges the budget until BackoffExhausted; nothing is created or attached
  run no_space_fails_typed_and_never_attaches = init.then(Declare).then(PoolFull)
    .then(WlStart(1)).then(LaunchUnavailable(1))
    .expect(st.as.get(1) == FAILED and st.why.get(1) == W_UNAVAIL and st.charged == 1 and not(st.lv)
            and st.nCreated == 0 and st.claim == 0 and not(st.ran.get(1)) and Safety)
    .then(WlStart(2)).then(LaunchUnavailable(2)).then(WlStart(3)).then(LaunchUnavailable(3))
    .expect(st.charged == LIMIT and exhausted(st) and AL.forall(a => not(canStart(st, a)))
            and AL.forall(a => not(st.ran.get(a))) and not(st.lv) and st.nCreated == 0 and Safety)

  // no volume store composed: typed failure, no volume, no VMM
  run store_absent_fails_typed_and_never_attaches = init.then(Declare).then(StoreAbsentBoot)
    .then(WlStart(1)).then(LaunchUnavailable(1))
    .expect(st.as.get(1) == FAILED and st.why.get(1) == W_UNAVAIL and not(st.lv) and st.nCreated == 0
            and st.charged == 1 and st.claim == 0 and not(st.ran.get(1)) and Safety)

  // SD-1 / SD-3 / SD-4: `workload restart` while a launch waits on the claim. The handler raises the signal and changes
  // no row; the poll ends at once (no poll taken), no row result, no charge, no VMM; the successor attaches after the
  // drain.
  run superseded_launch_result_discarded = running1
    .then(VmmSigkill(1)).then(WlStart(2)).then(LaunchEnsureCreate(2)).then(LaunchEnsureActivate(2))
    .expect(st.as.get(2) == PENDING and st.ph.get(2) == LP_ENSURED and st.claim == 1 and not(st.alive.get(1))
            and st.waited.get(2) == 0 and not(st.sup.get(2)) and Safety)
    .then(WorkloadRestart(2))
    .expect(st.sup.get(2) and st.as.get(2) == PENDING and st.ph.get(2) == LP_ENSURED and not(canPoll(st, 2))
            and Safety)
    .then(LaunchSuperseded(2))
    .expect(st.as.get(2) == TERMINATED and st.why.get(2) == W_SUPERSEDED and st.waited.get(2) == 0 and st.charged == 1
            and not(st.ran.get(2)) and not(st.delivered.get(2)) and st.claim == 1 and not(st.sup.get(2))
            and st.ph.get(2) == LP_NONE and Safety)
    .then(WlStart(3)).then(LaunchEnsureCreate(3)).then(LaunchEnsureActivate(3))
    .then(Drain)
    .then(LaunchCheck(3)).then(VmmSpawn(3)).then(GuestReady(3))
    .expect(st.as.get(3) == RUNNING and st.claim == 3 and not(st.ran.get(2)) and st.dv == 1 and st.nCreated == 1
            and vmms(st).size() == 1 and Safety)

  // SD-1 / SD-3 / SD-4: an operator stop while the guest boots (point 5): the VMM is terminated, its claim lingers until
  // the drain, the volume is kept, no Running row, no charge; resume reattaches the same volume
  run stop_during_boot_wait_terminates_vmm_and_keeps_volume = ensured1
    .then(LaunchCheck(1)).then(VmmSpawn(1))
    .expect(st.ph.get(1) == LP_BOOT and st.alive.get(1) and st.as.get(1) == PENDING and st.claim == 1 and Safety)
    .then(OpStop)
    .expect(st.sup.get(1) and canSupersede(st, 1) and not(canGuestReady(st, 1)) and Safety)
    .then(LaunchSuperseded(1))
    .expect(not(st.alive.get(1)) and st.claim == 1 and st.as.get(1) == TERMINATED and st.why.get(1) == W_SUPERSEDED
            and not(st.delivered.get(1)) and st.charged == 0 and st.lv and st.nCreated == 1 and not(st.sup.get(1))
            and Safety)
    .then(Drain)
    .expect(st.claim == 0 and st.lv and Safety)
    .then(OpResume).then(WlStart(2)).then(LaunchEnsureCreate(2)).then(LaunchEnsureActivate(2))
    .then(LaunchCheck(2)).then(VmmSpawn(2)).then(GuestReady(2))
    .expect(st.as.get(2) == RUNNING and st.vid == 1 and st.nCreated == 1 and st.charged == 0 and Safety)

  // SD-1 / SD-4 (point 2): a stop while `ensure` is in flight: ensure is never interrupted, so the launch cannot abandon
  // until ensure returns; then it abandons. The volume is created and active and stays.
  run stop_during_ensure_lets_ensure_finish_then_abandons = init.then(Declare).then(WlStart(1)).then(LaunchEnsureCreate(1))
    .then(OpStop)
    .expect(st.sup.get(1) and st.ph.get(1) == LP_CREATED and not(canSupersede(st, 1)) and Safety)
    .then(LaunchEnsureActivate(1))
    .expect(st.lv and st.active and st.sup.get(1) and canSupersede(st, 1) and Safety)
    .then(LaunchSuperseded(1))
    .expect(st.as.get(1) == TERMINATED and st.why.get(1) == W_SUPERSEDED and st.lv and st.active and st.nCreated == 1
            and st.charged == 0 and not(st.ran.get(1)) and st.claim == 0 and Safety)

  // SD-5: node shutdown raises the signal for every pending launch, and a lane opened after it is born raised. The
  // launch waiting on the claim and the launch whose lane opens after the shutdown both abandon: no row, no charge, the
  // volume kept; the next boot (`Boot`) clears the shutdown, then a launch attaches. The late launch is minted after the
  // first one ends: the model's one lane per workload (canStart, ADR-0109) does not allow two Pending allocations.
  run shutdown_abandons_every_launch_including_one_admitted_late = running1
    .then(VmmSigkill(1)).then(WlStart(2)).then(LaunchEnsureCreate(2)).then(LaunchEnsureActivate(2))
    .expect(st.as.get(2) == PENDING and st.claim == 1 and st.waited.get(2) == 0 and not(st.down) and Safety)
    .then(Shutdown)
    .expect(st.down and st.sup.get(2) and st.as.get(1) == FAILED and st.as.get(2) == PENDING and st.charged == 1
            and st.claim == 1 and st.lv and Safety)
    .then(LaunchSuperseded(2))
    .expect(st.as.get(2) == TERMINATED and st.why.get(2) == W_SUPERSEDED and st.waited.get(2) == 0
            and not(st.ran.get(2)) and st.charged == 1 and st.claim == 1 and st.down and Safety)
    .then(WlStart(3))
    .expect(st.sup.get(3) and st.as.get(3) == PENDING and st.ph.get(3) == LP_SUP and Safety)
    .then(LaunchSuperseded(3))
    .expect(st.as.get(3) == TERMINATED and st.why.get(3) == W_SUPERSEDED and not(st.ran.get(3)) and st.charged == 1
            and st.claim == 1 and st.lv and st.nCreated == 1 and not(st.delivered.get(2)) and not(st.delivered.get(3))
            and Safety)
    .then(Boot)
    .expect(not(st.down) and AL.forall(a => not(st.sup.get(a))) and st.charged == 1 and st.claim == 1 and Safety)
    .then(Drain)
    .then(WlStart(4)).then(LaunchEnsureCreate(4)).then(LaunchEnsureActivate(4))
    .then(LaunchCheck(4)).then(VmmSpawn(4)).then(GuestReady(4))
    .expect(st.as.get(4) == RUNNING and st.vid == 1 and st.dv == 1 and st.nCreated == 1 and not(st.sup.get(4))
            and Safety)
}

// exhaustive safety: one workload, allocation ids for several attempts, every fault and operator action enabled
module vol_ok {
  import node_volume(OFF = Set(), REC = false,
    OPS = Set("stop", "restart", "shutdown", "storeAbsent", "poolFull"), NA = 4, FB = 2,
    LIMIT = 3, DVMAX = 2).* from "./node_volume"
}
// the named runs; the #[quint_test] pins
module vol_conf {
  import node_volume(OFF = Set(), REC = true,
    OPS = Set("stop", "restart", "shutdown", "storeAbsent", "poolFull"), NA = 4, FB = 2,
    LIMIT = 3, DVMAX = 2).* from "./node_volume"
}
// liveness: never stopped or restarted, faults finite (FB < LIMIT)
module vol_live_start {
  import node_volume(OFF = Set(), REC = false, OPS = Set(), NA = 4, FB = 2, LIMIT = 3, DVMAX = 1).* from "./node_volume"
}
