/**
 * Node-local persistent volume protocol (GH #348; decisions VD-2 (ADR-0297), VD-3 (ADR-0298, backing choice: no
 * protocol rule), VD-8 (ADR-0303), VD-9 (ADR-0304)).
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
 * Environment actions (faults, each draws on the budget FB): VmmSigkill, DeviceSlow (the dead VMM's in-flight I/O
 * outlasts the launch bound), LaunchCmdFail (an `lvm` command fails), CpCrash (control plane crash: in-flight launch
 * state is lost, an already-spawned VMM keeps running until `Reclaim`), NodeReboot (volume inactive, every VMM gone,
 * claims released, launch state lost). StoreAbsentBoot and PoolFull are boot-time substrate facts (OPS).
 *
 * Instances: vol_ok (exhaustive safety), vol_conf (named runs, REC=true), vol_live_start (liveness), hazards under
 * hazard/.
 *
 * Assumptions: see README.md (A1..A6 of the feature delta, and the modelling assumptions M1..M5).
 */
module node_volume_conf {
  type ConfStep =
    | Init
    | Declare | OpStop | OpResume | StoreAbsentBoot | PoolFull
    | WorkloadRestart({ a: int }) | WlStart({ a: int }) | StopAlloc({ a: int })
    | LaunchEnsureCreate({ a: int }) | LaunchEnsureActivate({ a: int }) | LaunchUnavailable({ a: int })
    | LaunchCmdFail({ a: int }) | LaunchCheck({ a: int }) | LaunchPoll({ a: int }) | LaunchHeldFail({ a: int })
    | LaunchDiscard({ a: int }) | VmmSpawn({ a: int }) | VmmSpawnFail({ a: int })
    | GuestWrite | VmmSigkill({ a: int }) | DeviceSlow | Drain | Reclaim({ a: int }) | CpCrash | NodeReboot
}

module node_volume {
  import node_volume_conf.ConfStep from "./node_volume"
  const OFF: Set[str]   // rules off: "noWait" (VD-8) "recreate" (VD-9) "stopDeletes" (VD-2)
  const REC: bool       // true on the runs' main: conf_step records the action taken
  const OPS: Set[str]   // enabled operator / boot-time actions: "stop" "restart" "storeAbsent" "poolFull"
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
  // launch phase (control-plane memory)
  val LP_NONE = 0
  val LP_SUP = 1        // supervision claimed
  val LP_CREATED = 2    // ensure: create-if-absent done
  val LP_ENSURED = 3    // ensure: activated
  val LP_FREE = 4       // claim_state reported Free

  type S = {
    declared: bool, stopped: bool, storeUp: bool, spaceFree: bool,
    lv: bool, active: bool, vid: int, dv: int, maxDv: int, nCreated: int,
    claim: int,
    as: int -> int, alive: int -> bool, orphan: int -> bool, why: int -> int, ran: int -> bool,
    ph: int -> int, waited: int -> int,
    charged: int, faults: int, slow: bool,
    drainFail: bool       // ghost: a launch failed (charged) before the bound although only a draining claim was in the way
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
      charged: 0, faults: FB, slow: false, drainFail: false
    },
    conf_step' = variant("Init", ()),
  }

  // ---------------------------------------------------------------- helpers
  def nonTerminal(s: S): bool = AL.exists(a => s.as.get(a) == PENDING or s.as.get(a) == RUNNING)
  def deadHolder(s: S): bool = s.claim > 0 and not(s.alive.get(s.claim))
  def isPending(s: S, a: int): bool = s.as.get(a) == PENDING
  def exhausted(s: S): bool = s.charged >= LIMIT and not(nonTerminal(s))

  // a failed launch: charged to the budget when the allocation row is still Pending; a superseded launch's result is
  // discarded (no row change, no charge). `drain` marks the ghost.
  def failLaunch(s: S, a: int, why: int, drain: bool): S =
    if (isPending(s, a))
      { ...s, as: s.as.set(a, FAILED), why: s.why.set(a, why), charged: s.charged + 1,
        ph: s.ph.set(a, LP_NONE), drainFail: s.drainFail or drain }
    else { ...s, ph: s.ph.set(a, LP_NONE) }
  def earlyDrain(s: S, a: int): bool = deadHolder(s) and not(s.slow) and s.waited.get(a) < BOUND

  // ---------------------------------------------------------------- operator / boot-time facts
  action Declare = all {
    not(cur.declared),
    commit({ ...cur, declared: true }, variant("Declare", ())),
  }
  action OpStop = all {
    OPS.contains("stop"), cur.declared, not(cur.stopped),
    commit({ ...cur, stopped: true }, variant("OpStop", ())),
  }
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
    s.as.get(a) == UNMINTED, AL.forall(x => s.as.get(x) != UNMINTED or a <= x),
  }
  // the successor is minted as soon as the predecessor row is terminal: no wait for cleanup or claim release
  action WlStart(a: int): bool = all {
    canStart(cur, a),
    commit({ ...cur, as: cur.as.set(a, PENDING), ph: cur.ph.set(a, LP_SUP) }, variant("WlStart", { a: a })),
  }
  // operator stop ends the non-terminal allocation; the VMM dies, the claim lingers until Drain. The volume stays (VD-2).
  action StopAlloc(a: int): bool = all {
    cur.as.get(a) == PENDING or cur.as.get(a) == RUNNING, cur.stopped,
    commit({ ...cur, as: cur.as.set(a, TERMINATED), why: cur.why.set(a, W_STOP), alive: cur.alive.set(a, false),
             orphan: cur.orphan.set(a, false),
             lv: cur.lv and not(OFF.contains("stopDeletes")), active: cur.active and not(OFF.contains("stopDeletes")),
             dv: if (OFF.contains("stopDeletes")) 0 else cur.dv }, variant("StopAlloc", { a: a })),
  }
  // `workload restart` (generation bump): the non-terminal allocation ends; a Pending one's launch is superseded and
  // runs on in the background; its result is discarded (reconcilers.md)
  action WorkloadRestart(a: int): bool = all {
    OPS.contains("restart"), cur.declared, not(cur.stopped),
    cur.as.get(a) == PENDING or cur.as.get(a) == RUNNING,
    commit({ ...cur, as: cur.as.set(a, TERMINATED), why: cur.why.set(a, W_RESTART), alive: cur.alive.set(a, false),
             orphan: cur.orphan.set(a, false) }, variant("WorkloadRestart", { a: a })),
  }

  // ---------------------------------------------------------------- launch path (VD-9 ensure, VD-8 claim wait)
  // ensure, step 1: create-if-absent (observe, then create only when missing). Hazard "recreate" (VD-9): creates
  // unconditionally, replacing an existing volume with a blank one.
  action LaunchEnsureCreate(a: int): bool = all {
    cur.ph.get(a) == LP_SUP, cur.storeUp, or { cur.lv, cur.spaceFree, OFF.contains("recreate") },
    val fresh = not(cur.lv) or OFF.contains("recreate")
    commit({ ...cur, lv: true, active: if (fresh) false else cur.active,
             vid: if (fresh) cur.nCreated + 1 else cur.vid, dv: if (fresh) 0 else cur.dv,
             nCreated: if (fresh) cur.nCreated + 1 else cur.nCreated, ph: cur.ph.set(a, LP_CREATED) },
      variant("LaunchEnsureCreate", { a: a })),
  }
  // ensure, step 2: activate
  action LaunchEnsureActivate(a: int): bool = all {
    cur.ph.get(a) == LP_CREATED, cur.lv,
    commit({ ...cur, active: true, ph: cur.ph.set(a, LP_ENSURED) }, variant("LaunchEnsureActivate", { a: a })),
  }
  // typed VolumeUnavailable: no store composed, no free space for a missing volume, or the volume vanished (NotFound
  // maps to a command failure). Nothing is created.
  action LaunchUnavailable(a: int): bool = all {
    or {
      and { cur.ph.get(a) == LP_SUP, or { not(cur.storeUp), and { not(cur.lv), not(cur.spaceFree), not(OFF.contains("recreate")) } } },
      and { cur.ph.get(a) == LP_CREATED or cur.ph.get(a) == LP_ENSURED, not(cur.lv) },
    },
    commit(failLaunch(cur, a, W_UNAVAIL, false), variant("LaunchUnavailable", { a: a })),
  }
  // environment: an `lvm` command fails mid-ensure (typed VolumeUnavailable); an already created volume stays
  action LaunchCmdFail(a: int): bool = all {
    cur.ph.get(a) == LP_SUP or cur.ph.get(a) == LP_CREATED, cur.faults > 0,
    commit({ ...failLaunch(cur, a, W_UNAVAIL, false), faults: cur.faults - 1 }, variant("LaunchCmdFail", { a: a })),
  }
  // claim_state reports Free. Hazard "noWait" (VD-8): the launch treats the VMM process exit as the claim free.
  def checkPasses(s: S, a: int): bool = and {
    s.ph.get(a) == LP_ENSURED, s.lv, s.active,
    s.claim == 0 or (OFF.contains("noWait") and deadHolder(s)),
  }
  action LaunchCheck(a: int): bool = all {
    checkPasses(cur, a),
    commit({ ...cur, ph: cur.ph.set(a, LP_FREE) }, variant("LaunchCheck", { a: a })),
  }
  // the claim is held: poll. Time passes only when the holder is not a draining dead VMM (a healthy drain finishes
  // within the bound, M3), or after DeviceSlow
  def canPoll(s: S, a: int): bool = and {
    s.ph.get(a) == LP_ENSURED, s.lv, s.claim != 0, s.waited.get(a) < BOUND, s.slow or not(deadHolder(s)),
  }
  action LaunchPoll(a: int): bool = all {
    canPoll(cur, a), not(OFF.contains("noWait")),
    commit({ ...cur, waited: cur.waited.set(a, cur.waited.get(a) + 1) }, variant("LaunchPoll", { a: a })),
  }
  // VolumeHeld: the bound passed. M4: a launch that waited the whole bound ends the stuck-drain condition.
  action LaunchHeldFail(a: int): bool = all {
    cur.ph.get(a) == LP_ENSURED, cur.lv, cur.claim != 0, cur.waited.get(a) >= BOUND,
    commit({ ...failLaunch(cur, a, W_HELD, earlyDrain(cur, a)), slow: false }, variant("LaunchHeldFail", { a: a })),
  }
  // a superseded launch's result is discarded: it never attaches
  action LaunchDiscard(a: int): bool = all {
    cur.ph.get(a) == LP_FREE, not(isPending(cur, a)),
    commit({ ...cur, ph: cur.ph.set(a, LP_NONE) }, variant("LaunchDiscard", { a: a })),
  }
  // attach: spawn the VMM, which takes the claim (Cloud Hypervisor's O_EXCL open is the atomic verdict)
  def canSpawn(s: S, a: int): bool = and {
    isPending(s, a), s.ph.get(a) == LP_FREE, s.lv, s.active, s.claim == 0,
  }
  action VmmSpawn(a: int): bool = all {
    canSpawn(cur, a),
    commit({ ...cur, claim: a, as: cur.as.set(a, RUNNING), alive: cur.alive.set(a, true), ran: cur.ran.set(a, true),
             ph: cur.ph.set(a, LP_NONE) }, variant("VmmSpawn", { a: a })),
  }
  action VmmSpawnFail(a: int): bool = all {
    isPending(cur, a), cur.ph.get(a) == LP_FREE, or { not(cur.lv), not(cur.active), cur.claim != 0 },
    commit(failLaunch(cur, a, W_OPENLOST, earlyDrain(cur, a)), variant("VmmSpawnFail", { a: a })),
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
             orphan: cur.orphan.set(a, false), charged: cur.charged + 1, faults: cur.faults - 1 },
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
  // the existing VmReclamation kills a VMM that survived a control-plane crash; its allocation fails, its claim drains
  action Reclaim(a: int): bool = all {
    cur.as.get(a) == RUNNING, cur.alive.get(a), cur.orphan.get(a),
    commit({ ...cur, as: cur.as.set(a, FAILED), why: cur.why.set(a, W_CRASH), alive: cur.alive.set(a, false),
             orphan: cur.orphan.set(a, false), charged: cur.charged + 1 }, variant("Reclaim", { a: a })),
  }
  // environment: control plane crash. Every in-flight launch is lost (a Pending allocation fails, charged), a spawned
  // VMM keeps running (orphan) until Reclaim; the volume and its data persist.
  action CpCrash = all {
    cur.faults > 0,
    commit({ ...cur, ph: AL.mapBy(a => LP_NONE),
             as: AL.mapBy(a => if (cur.as.get(a) == PENDING) FAILED else cur.as.get(a)),
             why: AL.mapBy(a => if (cur.as.get(a) == PENDING) W_LOST else cur.why.get(a)),
             orphan: AL.mapBy(a => cur.alive.get(a) or cur.orphan.get(a)),
             charged: cur.charged + AL.filter(a => cur.as.get(a) == PENDING).size(), faults: cur.faults - 1 },
      variant("CpCrash", ())),
  }
  // environment: node reboot. The volume is inactive, every VMM is gone and its claim released, launch state is lost,
  // the allocation that was Pending or Running fails (charged).
  action NodeReboot = all {
    cur.faults > 0,
    commit({ ...cur, active: false, claim: 0, slow: false, ph: AL.mapBy(a => LP_NONE),
             alive: AL.mapBy(a => false), orphan: AL.mapBy(a => false),
             as: AL.mapBy(a => if (cur.as.get(a) == PENDING or cur.as.get(a) == RUNNING) FAILED else cur.as.get(a)),
             why: AL.mapBy(a => if (cur.as.get(a) == PENDING or cur.as.get(a) == RUNNING) W_CRASH else cur.why.get(a)),
             charged: cur.charged + AL.filter(a => cur.as.get(a) == PENDING or cur.as.get(a) == RUNNING).size(),
             faults: cur.faults - 1 }, variant("NodeReboot", ())),
  }

  // ---------------------------------------------------------------- composition
  // everything the system does on its own (not the operator's actions, not the guest, not faults)
  action Progress = {
    nondet a = AL.oneOf()
    any { WlStart(a), StopAlloc(a), LaunchEnsureCreate(a), LaunchEnsureActivate(a), LaunchUnavailable(a),
          LaunchCheck(a), LaunchPoll(a), LaunchHeldFail(a), LaunchDiscard(a), VmmSpawn(a), VmmSpawnFail(a),
          Reclaim(a), Drain }
  }
  action step = {
    nondet a = AL.oneOf()
    any { Declare, OpStop, OpResume, StoreAbsentBoot, PoolFull, WorkloadRestart(a), GuestWrite, VmmSigkill(a),
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
  // a launch that fails typed never leaves a VMM attached
  val I_TYPED_FAILURE_NEVER_ATTACHES = AL.forall(a =>
    (st.why.get(a) == W_HELD or st.why.get(a) == W_UNAVAIL) implies
      and { not(st.ran.get(a)), not(st.alive.get(a)), st.claim != a })
  // restart budget bound
  val I_BUDGET_BOUND = st.charged <= LIMIT
  val Safety = and { I_SINGLE_WRITER, I_DATA_SURVIVES_RESTART, I_ENSURE_IDEMPOTENT_UNDER_CRASH,
                     I_NO_DRAIN_FAILURE_CHARGES_BUDGET, I_STOP_KEEPS_VOLUME, I_TYPED_FAILURE_NEVER_ATTACHES,
                     I_BUDGET_BOUND }
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
  action fDiscard = { nondet a = AL.oneOf() LaunchDiscard(a) }
  action fSpawn = { nondet a = AL.oneOf() VmmSpawn(a) }
  action fSpawnFail = { nondet a = AL.oneOf() VmmSpawnFail(a) }
  action fReclaim = { nondet a = AL.oneOf() Reclaim(a) }
  temporal FairProgress = and {
    weakFair(fWlStart, (st, conf_step)), weakFair(fStopAlloc, (st, conf_step)),
    weakFair(fEnsureCreate, (st, conf_step)), weakFair(fEnsureActivate, (st, conf_step)),
    weakFair(fUnavailable, (st, conf_step)), weakFair(fCheck, (st, conf_step)), weakFair(fPoll, (st, conf_step)),
    weakFair(fHeldFail, (st, conf_step)), weakFair(fDiscard, (st, conf_step)), weakFair(fSpawn, (st, conf_step)),
    weakFair(fSpawnFail, (st, conf_step)), weakFair(fReclaim, (st, conf_step)), weakFair(Drain, (st, conf_step)),
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
  action running1 = ensured1.then(LaunchCheck(1)).then(VmmSpawn(1)).then(GuestWrite)

  run happy_path_deploy_to_running = init.then(Declare)
    .expect(not(nonTerminal(st)) and not(st.lv))
    .then(WlStart(1))
    .expect(st.as.get(1) == PENDING and st.ph.get(1) == LP_SUP and not(st.lv))
    .then(LaunchEnsureCreate(1))
    .expect(st.lv and not(st.active) and st.nCreated == 1 and st.vid == 1 and st.claim == 0)
    .then(LaunchEnsureActivate(1))
    .expect(st.active and checkPasses(st, 1))
    .then(LaunchCheck(1)).then(VmmSpawn(1))
    .expect(st.as.get(1) == RUNNING and st.claim == 1 and st.alive.get(1) and st.charged == 0 and Safety)
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
    .then(LaunchCheck(2)).then(VmmSpawn(2))
    .expect(st.as.get(2) == RUNNING and st.claim == 2 and st.dv == 2 and st.maxDv == 2 and st.nCreated == 1
            and st.why.get(2) == W_NONE and st.charged == 1 and not(st.drainFail) and Safety)

  run stop_keeps_volume_and_restart_reattaches = running1
    .then(OpStop).then(StopAlloc(1)).then(Drain)
    .expect(st.as.get(1) == TERMINATED and st.lv and st.nCreated == 1 and st.claim == 0 and st.dv == 1
            and not(canStart(st, 2)) and st.charged == 0 and Safety)
    .then(OpResume).then(WlStart(2)).then(LaunchEnsureCreate(2)).then(LaunchEnsureActivate(2))
    .then(LaunchCheck(2)).then(VmmSpawn(2))
    .expect(st.as.get(2) == RUNNING and st.dv == 1 and st.vid == 1 and st.nCreated == 1 and Safety)

  // the control plane crashed after create and before attach: the next launch's ensure reuses the volume
  run crash_after_create_before_attach_reuses_volume = init.then(Declare).then(WlStart(1)).then(LaunchEnsureCreate(1))
    .expect(st.lv and not(st.active) and st.nCreated == 1 and st.ph.get(1) == LP_CREATED and Safety)
    .then(CpCrash)
    .expect(st.lv and st.nCreated == 1 and st.as.get(1) == FAILED and st.why.get(1) == W_LOST
            and st.ph.get(1) == LP_NONE and st.charged == 1 and Safety)
    .then(WlStart(2)).then(LaunchEnsureCreate(2))
    .expect(st.nCreated == 1 and st.vid == 1 and Safety)
    .then(LaunchEnsureActivate(2)).then(LaunchCheck(2)).then(VmmSpawn(2))
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
    .then(LaunchCheck(2)).then(VmmSpawn(2))
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
    .then(LaunchCheck(3)).then(VmmSpawn(3))
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

  // a restart bumps the generation while a launch waits: the stale launch runs on, its result is discarded and never
  // attaches; the successor's launch attaches after the drain
  run superseded_launch_result_discarded = running1
    .then(VmmSigkill(1)).then(WlStart(2)).then(LaunchEnsureCreate(2)).then(LaunchEnsureActivate(2))
    .then(WorkloadRestart(2))
    .expect(st.as.get(2) == TERMINATED and st.why.get(2) == W_RESTART and st.ph.get(2) == LP_ENSURED and Safety)
    .then(WlStart(3)).then(LaunchEnsureCreate(3)).then(LaunchEnsureActivate(3))
    .then(Drain)
    .then(LaunchCheck(2))
    .expect(not(canSpawn(st, 2)) and Safety)
    .then(LaunchDiscard(2))
    .expect(st.ph.get(2) == LP_NONE and not(st.ran.get(2)) and not(st.alive.get(2)) and st.claim == 0 and Safety)
    .then(LaunchCheck(3)).then(VmmSpawn(3))
    .expect(st.as.get(3) == RUNNING and st.claim == 3 and not(st.ran.get(2)) and st.dv == 1 and st.nCreated == 1
            and vmms(st).size() == 1 and Safety)
}

// exhaustive safety: one workload, allocation ids for several attempts, every fault and operator action enabled
module vol_ok {
  import node_volume(OFF = Set(), REC = false, OPS = Set("stop", "restart", "storeAbsent", "poolFull"), NA = 4, FB = 2,
    LIMIT = 3, DVMAX = 2).* from "./node_volume"
}
// the named runs; the #[quint_test] pins
module vol_conf {
  import node_volume(OFF = Set(), REC = true, OPS = Set("stop", "restart", "storeAbsent", "poolFull"), NA = 4, FB = 2,
    LIMIT = 3, DVMAX = 2).* from "./node_volume"
}
// liveness: never stopped or restarted, faults finite (FB < LIMIT)
module vol_live_start {
  import node_volume(OFF = Set(), REC = false, OPS = Set(), NA = 4, FB = 2, LIMIT = 3, DVMAX = 1).* from "./node_volume"
}
