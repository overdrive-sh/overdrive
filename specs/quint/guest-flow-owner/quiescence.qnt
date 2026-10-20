// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — forwarding quiescence as a counted, move-only hold (QHOLD,
 * ADR-0169), the per-component recoveries of the runtime supervisor (ADR-0124 bounded recovery, UP-7)
 * and the boot write of the shared mTLS nft table that refuses boot on failure (ADR-0184).
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 * Source of truth: feature-delta.md § "vsock Attachment Replacement DESIGN" — § Owner and provisioner
 * (`quiesce_forwarding`, `restore_forwarding`, `QuiescenceHold`), ADR-0169 (QHOLD), ADR-0184 (a failed
 * boot write of the shared table refuses boot), ADR-0124 (1 s audit, bounded attempts, fail-stop).
 *
 * The rules modelled:
 *   - `quiesce_forwarding()` increments the hold count and returns a move-only `QuiescenceHold`; the first
 *     hold performs the quiesce (admission latch, abort every open flow). `restore_forwarding(hold)`
 *     consumes the hold, decrements, and reopens admission only when the count reaches zero. A boolean
 *     latch is rejected: the first recovery's restore would reopen while a second recovery is still repairing.
 *   - D-UP-3: a hold dropped without a restore keeps the count (fail closed) and increments the owner's
 *     leaked-hold count (no I/O in Drop). The stimulus is the component-free environment action `LeakHold`:
 *     a `QuiescenceHold` taken from the owner and dropped without restore (in production only an Overdrive
 *     defect produces it; the driver calls `owner.quiesce()` and drops the hold). It creates no recovery
 *     episode and changes none. The runtime supervisor (ADR-0124) reads only the owner's leaked-hold count:
 *     above zero, its next audit tick fail-stops serve (cause `LeakedQuiescenceHold`), whatever any
 *     component's recovery is doing; the supervisor restart converges on boot.
 *   - D-UP-4 (PD-1): holds are issued and redeemed by the owner's own `QuiescenceLedger`. A `restore` with a hold
 *     another ledger issued returns `QuiescenceRestoreError::ForeignHold` and changes nothing in this owner: the
 *     count, the latch and its leaked-hold count are unchanged (fail closed). MODEL-ONLY CLAIM: the foreign hold,
 *     dropped unredeemed, counts as leaked at the ledger that issued it. That ledger is another owner's; no variable
 *     here represents it, no invariant checks the claim, and no conformance driver can project it from this owner.
 *     Its discharge is a sim/unit scenario outside this model: two ledgers, the owner refuses a hold the second
 *     ledger issued (`ForeignHold`), the hold is dropped, and the second ledger's `leaked()` rises by one while
 *     this owner's is unchanged. The stimulus is the component-free
 *     environment action `ForeignRestore` (in production only a composition defect produces it: one supervisor is
 *     composed with one owner; the driver issues a hold from a second `QuiescenceLedger` and calls
 *     `owner.restore(hold)`). The supervisor that received `ForeignHold` fail-stops serve (cause
 *     `ForeignQuiescenceHold`); the restart converges on boot.
 *   - each recovery: the audit finds component c damaged → quiesce, holding → repair c → audit → restore
 *     with its hold. A failed repair (`RepFail`, counted as an attempt, the recovery keeps its hold) or a
 *     failed post-repair audit is a failed attempt (UP-7); the budget is per component recovery; when spent,
 *     serve fail-stops.
 *   - boot step 8 writes the shared nft table in one transaction. A failed write refuses boot (typed,
 *     before forwarding opens); the supervisor restart retries it (converge-on-boot). Nothing leaves the
 *     table absent while serving, so no component recovery covers it.
 *
 * Abstraction:
 *   - Two generic runtime components C1 and C2 (a component damage is a task exit, recurring at any time,
 *     unbounded). `flowsOpen` stands for the open flows the first hold aborts.
 *   - Crash / restart: process-scoped state (the count, the holds, the leaked-hold count, the latch, the
 *     flows) is lost; the table persists across the crash. Boot step 8 either writes the table (then serve
 *     runs) or fails (serve refuses to start; the supervisor tries again).
 *   - One leaked hold per process lifetime (MAXLEAK) and one foreign restore per process lifetime (the supervisor
 *     fail-stops after it) keep the instance finite. `foreign` is process-scoped: the supervisor received
 *     `ForeignHold` and has not yet fail-stopped.
 *   - D-UP-1 (conformance step record): `conf_step` records the action that produced the state, as a variant of
 *     `quiescence_conf::ConfStep` whose tag is the action's name and whose value is the record of its
 *     parameters by parameter name (parameterless actions: the unit value). quint-connect reads the action and
 *     its picks from it (`Config::nondet`). Model-only.
 *
 * OFF names design rules switched off (hazard/quiescence_hazards.qnt):
 *   boolLatch       QHOLD: a boolean latch — any restore reopens and ends every hold
 *   dropDecrements  QHOLD: a dropped hold releases the count (the guard's Drop decrements, besides counting the leak)
 *   repairNoHold    a recovery repairs without holding quiescence
 *   redoNoCount     UP-7: a failed post-repair audit is not a failed attempt
 *   leakIgnored     ADR-0124 (D-UP-3): the supervisor does not consult the leaked-hold count (only a spent attempt
 *                   budget fail-stops), so a leaked hold keeps forwarding quiesced for good
 *   bootContinues   ADR-0184: boot continues with serve running after a failed table write
 *   restoreClearsLatch QHOLD: a restore reopens admission (clears the latch) whatever the remaining count
 *   quiesceSkipsFlows  QHOLD: the first hold latches admission but does not abort the open flows
 *   foreignDecrements  D-UP-4: a restore with a foreign hold is redeemed as if it were the owner's own (the count is
 *                   decremented and admission reopens at zero)
 *   foreignIgnored  D-UP-4 (PD-1): the supervisor does not fail-stop on `ForeignHold` (it drops the result)
 *
 * ASSUMPTIONS (each is an input the model takes as given; the discharging item follows the statement):
 *   Q-1  A task exit of a runtime component is detected by the 1 s audit; a repair that returns `Err` is a failed attempt
 *         Discharged by: OPEN validation item: DST invariant under UP-7 (ADR-0124)
 */
// D-UP-1: the variants of `conf_step` (a module of its own: a constructor would clash with the action of the same
// name, so the actions build their variant with the builtin `variant`)
module quiescence_conf {
  type ConfStep =
    | InitWith({ fl: int })
    | InitDown
    | Damage({ c: str })
    | OpenFlow
    | Crash
    | RestartWith({ writeOk: bool })
    | RecQuiesce({ c: str })
    | RecRepair({ c: str })
    | RepFail({ c: str })
    | RecRedo({ c: str })
    | RecRestore({ c: str })
    | LeakHold
    | FailStop({ c: str })
    | RecRepairNoHold({ c: str })
    | ForeignRestore
}

module quiescence {
  import quiescence_conf.ConfStep from "./quiescence"
  const OFF: Set[str]
  const CFG: str   // "ok" | "live" | "conf"
  // D-UP-1: true only on the module's conformance main, where `conf_step` records the action taken; every
  // model-checking, liveness and hazard instance passes false and keeps `conf_step` at its initial value.
  const REC: bool

  def bug(r: str): bool = OFF.contains(r)
  val COMPONENTS: Set[str] = Set("C1", "C2")
  val MAXATT: int = 2
  val MAXLEAK: int = 1                                  // leaked holds per process lifetime (finite instance)
  val ALLOW_CRASH: bool = CFG == "ok" or CFG == "conf"
  // -1: a repair may fail forever. The conformance main ("conf") has both the crash of "ok" and the unbounded repair
  // failures of "live" (only named runs and driver traces run on it, so it need not be finite)
  val REPAIR_FAILS: int = if (CFG == "ok") 1 else -1

  type Ghost = {
    reopenEarly: bool,   // forwarding reopened while some recovery had not repaired
    repairNoHold: bool,  // a repair ran without its recovery's hold
    overlapped: bool,    // two holds held at once (witness)
    stillQuiesced: bool, // a restore left the count above zero (witness)
    failStopped: bool,   // serve fail-stopped (witness)
    dropped: bool,       // a hold was dropped without a restore (witness)
    refusedBoot: bool,   // a boot attempt was refused (witness)
    foreignChanged: bool, // D-UP-4: a restore with a foreign hold changed the count, the latch or the leaked count
  }
  val g0: Ghost = { reopenEarly: false, repairNoHold: false, overlapped: false, stillQuiesced: false,
                    failStopped: false, dropped: false, refusedBoot: false,
                    foreignChanged: false }

  var up: bool
  var stop: str              // none | crash | failstop | refused
  var table: bool            // the shared nft table is written (persists across crash and restart)
  var cnt: int               // the hold count
  var latch: bool            // admission quiesced
  var rec: str -> str        // per component recovery: idle | held | repaired (D-UP-3: no dropped phase)
  var dmg: str -> bool       // per component damage
  var att: str -> int        // failed attempts of the current episode (process-scoped)
  var repLeft: int
  var flowsOpen: int         // open flows (0 or 1)
  var leaked: int            // D-UP-3: the owner's leaked-hold count (holds dropped without a restore; process-scoped)
  var foreign: bool          // D-UP-4: the supervisor's restore returned ForeignHold (process-scoped, until fail-stop)
  var g: Ghost
  var conf_step: ConfStep    // D-UP-1: the action that produced this state (model-only)

  val recIdle: str -> str = COMPONENTS.mapBy(c => "idle")
  val att0: str -> int = COMPONENTS.mapBy(c => 0)
  def dec(x: int): int = if (x < 0) x else x - 1
  // a recovery that has quiesced and not yet repaired
  def unrepaired(c: str): bool = rec.get(c) == "held"
  // a recovery that holds its `QuiescenceHold` (from its quiesce to its restore)
  def holding(c: str): bool = rec.get(c).in(Set("held", "repaired"))

  action keepEnv = all { up' = up, stop' = stop, table' = table, dmg' = dmg, repLeft' = repLeft }

  action init = any { InitWith(0), InitWith(1), InitDown }
  // a fresh node before its first boot: serve is down and the shared table is not yet written
  action InitDown = all {
    up' = false, stop' = "crash", table' = false, cnt' = 0, latch' = false, rec' = recIdle,
    dmg' = COMPONENTS.mapBy(c => false), att' = att0, repLeft' = REPAIR_FAILS, flowsOpen' = 0, leaked' = 0, foreign' = false, g' = g0,
    conf_step' = variant("InitDown", ()),
  }
  // `fl`: whether a flow is open when the first hold is taken
  action InitWith(fl: int): bool = all {
    up' = true, stop' = "none", table' = true, cnt' = 0, latch' = false, rec' = recIdle,
    dmg' = COMPONENTS.mapBy(c => false), att' = att0, repLeft' = REPAIR_FAILS, flowsOpen' = fl, leaked' = 0, foreign' = false, g' = g0,
    conf_step' = if (REC) variant("InitWith", { fl: fl }) else variant("InitDown", ()),
  }

  // ------------------------------------------------------------------ environment
  // a component task exits (recurring)
  action Damage(c: str): bool = all {
    up, not(dmg.get(c)),
    dmg' = dmg.set(c, true), up' = up, stop' = stop, table' = table, repLeft' = repLeft,
    cnt' = cnt, latch' = latch, rec' = rec, att' = att, flowsOpen' = flowsOpen, leaked' = leaked, foreign' = foreign, g' = g,
    conf_step' = if (REC) variant("Damage", { c: c }) else conf_step,
  }
  // a new flow opens while admission is open
  action OpenFlow = all {
    up, not(latch), flowsOpen == 0,
    flowsOpen' = 1, cnt' = cnt, latch' = latch, rec' = rec, att' = att, leaked' = leaked, foreign' = foreign, keepEnv, g' = g,
    conf_step' = if (REC) variant("OpenFlow", ()) else conf_step,
  }
  action Crash = all {
    ALLOW_CRASH, up,
    up' = false, stop' = "crash", cnt' = 0, latch' = false, rec' = recIdle, att' = att0, flowsOpen' = 0, leaked' = 0, foreign' = false,
    dmg' = COMPONENTS.mapBy(c => false), table' = table, repLeft' = repLeft, g' = g,
    conf_step' = if (REC) variant("Crash", ()) else conf_step,
  }
  // boot step 8 writes the table in one transaction. It succeeds (serve runs, forwarding opens), or fails and
  // boot is refused: serve stays down and the supervisor tries again. bootContinues: serve runs anyway.
  action Restart = {
    nondet writeOk = Set(true, false).oneOf()
    RestartWith(writeOk)
  }
  action RestartWith(writeOk: bool): bool = all {
    not(up), stop != "none",
    up' = (writeOk or bug("bootContinues")), stop' = if (writeOk or bug("bootContinues")) "none" else "refused",
    table' = (table or writeOk),
    cnt' = cnt, latch' = latch, rec' = rec, att' = att, flowsOpen' = flowsOpen, dmg' = dmg, repLeft' = repLeft,
    leaked' = leaked, foreign' = foreign,
    g' = { ...g, refusedBoot: g.refusedBoot or not(writeOk) },
    conf_step' = if (REC) variant("RestartWith", { writeOk: writeOk }) else conf_step,
  }
  // D-UP-3: a `QuiescenceHold` taken from the owner (`quiesce_forwarding()`) and dropped without a restore — an
  // Overdrive defect, not tied to any component's recovery. The take is a quiesce (count + 1; the first hold
  // latches admission and aborts the flows); the Drop keeps the count (fail closed) and counts the leak. No
  // recovery episode is created and none changes. dropDecrements: the Drop also releases the count.
  action LeakHold = all {
    up, leaked < MAXLEAK,
    val taken = cnt + 1
    val after = if (bug("dropDecrements")) taken - 1 else taken
    all {
      cnt' = after,
      latch' = after > 0,
      flowsOpen' = if (cnt == 0 and not(bug("quiesceSkipsFlows"))) 0 else flowsOpen,
    },
    leaked' = leaked + 1, foreign' = foreign, rec' = rec, att' = att, keepEnv,
    g' = { ...g, dropped: true, overlapped: g.overlapped or cnt > 0 },
    conf_step' = if (REC) variant("LeakHold", ()) else conf_step,
  }

  // D-UP-4 (PD-1): the supervisor calls `restore` with a hold another `QuiescenceLedger` issued — a composition defect,
  // not tied to any component's recovery. The owner's ledger refuses to redeem it (`QuiescenceRestoreError::ForeignHold`)
  // and nothing in this owner changes: the count, the latch, the leaked-hold count, the flows. The supervisor holds the
  // `ForeignHold` result (`foreign`) and fail-stops at its next step (`FailStop`, cause `ForeignQuiescenceHold`).
  // foreignDecrements: the foreign hold is redeemed as the owner's own (count − 1 above zero; admission reopens at zero).
  action ForeignRestore = all {
    up, not(foreign),
    val newCnt = if (bug("foreignDecrements") and cnt > 0) cnt - 1 else cnt
    val newLatch = if (bug("foreignDecrements")) newCnt > 0 else latch
    val early = latch and not(newLatch) and COMPONENTS.exists(d => unrepaired(d))
    all {
      cnt' = newCnt,
      latch' = newLatch,
      g' = { ...g, reopenEarly: g.reopenEarly or early,
             foreignChanged: g.foreignChanged or newCnt != cnt or newLatch != latch },
    },
    foreign' = true, leaked' = leaked, rec' = rec, att' = att, flowsOpen' = flowsOpen, keepEnv,
    conf_step' = if (REC) variant("ForeignRestore", ()) else conf_step,
  }

  // ------------------------------------------------------------------ recovery of component c
  // the audit finds c damaged → quiesce_forwarding(): count + 1; the first hold latches admission and aborts the flows
  action RecQuiesce(c: str): bool = all {
    up, rec.get(c) == "idle", dmg.get(c), not(bug("repairNoHold")),
    cnt' = cnt + 1, latch' = true, rec' = rec.set(c, "held"), att' = att, leaked' = leaked, foreign' = foreign,
    flowsOpen' = if (cnt == 0 and not(bug("quiesceSkipsFlows"))) 0 else flowsOpen, keepEnv,
    g' = { ...g, overlapped: g.overlapped or cnt > 0 },
    conf_step' = if (REC) variant("RecQuiesce", { c: c }) else conf_step,
  }
  // the repair of c
  action RecRepair(c: str): bool = all {
    up, rec.get(c) == "held", att.get(c) < MAXATT,
    dmg' = dmg.set(c, false), rec' = rec.set(c, "repaired"),
    cnt' = cnt, latch' = latch, att' = att, flowsOpen' = flowsOpen, up' = up, stop' = stop, table' = table,
    repLeft' = repLeft, leaked' = leaked, foreign' = foreign, g' = g,
    conf_step' = if (REC) variant("RecRepair", { c: c }) else conf_step,
  }
  // D-UP-3 / UP-7: the repair returns Err — a failed attempt, counted; the recovery keeps its hold (the only cause of
  // a failed repair)
  action RepFail(c: str): bool = all {
    up, rec.get(c) == "held", att.get(c) < MAXATT, repLeft != 0,
    att' = att.set(c, att.get(c) + 1), repLeft' = dec(repLeft),
    cnt' = cnt, latch' = latch, rec' = rec, flowsOpen' = flowsOpen, up' = up, stop' = stop, dmg' = dmg, table' = table,
    leaked' = leaked, foreign' = foreign, g' = g,
    conf_step' = if (REC) variant("RepFail", { c: c }) else conf_step,
  }
  // the post-repair audit finds c damaged again: a failed attempt (UP-7); repair again under the hold
  action RecRedo(c: str): bool = all {
    up, rec.get(c) == "repaired", dmg.get(c),
    rec' = rec.set(c, "held"),
    att' = if (bug("redoNoCount")) att else att.set(c, att.get(c) + 1),
    cnt' = cnt, latch' = latch, flowsOpen' = flowsOpen, leaked' = leaked, foreign' = foreign, keepEnv, g' = g,
    conf_step' = if (REC) variant("RecRedo", { c: c }) else conf_step,
  }
  // clean audit → restore_forwarding(hold): the hold is consumed, the count decremented; admission reopens only
  // when the count reaches zero. boolLatch: the restore reopens and ends every hold.
  action RecRestore(c: str): bool = all {
    up, rec.get(c) == "repaired", not(dmg.get(c)),
    val newCnt = if (bug("boolLatch")) 0 else cnt - 1
    val reopens = newCnt == 0
    val early = reopens and COMPONENTS.exists(d => d != c and unrepaired(d))
    all {
      cnt' = newCnt,
      latch' = if (bug("restoreClearsLatch")) false else not(reopens),
      rec' = if (bug("boolLatch")) COMPONENTS.mapBy(d => if (d == c or rec.get(d) == "held") "idle" else rec.get(d))
             else rec.set(c, "idle"),
      att' = if (bug("boolLatch")) att0 else att.set(c, 0),
      g' = { ...g, reopenEarly: g.reopenEarly or early, stillQuiesced: g.stillQuiesced or not(reopens) },
    },
    flowsOpen' = flowsOpen, leaked' = leaked, foreign' = foreign, keepEnv,
    conf_step' = if (REC) variant("RecRestore", { c: c }) else conf_step,
  }
  // ADR-0124, the recovery supervisor's audit tick fail-stops serve: the attempt budget of c's unrepaired recovery is
  // spent, or (D-UP-3, cause LeakedQuiescenceHold) the owner's leaked-hold count is above zero, or (D-UP-4 / PD-1, cause
  // ForeignQuiescenceHold) a restore returned ForeignHold — the last two independent of any component's recovery (c then
  // only names the supervisor's step). leakIgnored: the leaked count is not consulted; foreignIgnored: nor ForeignHold.
  action FailStop(c: str): bool = all {
    up, or {
      unrepaired(c) and att.get(c) >= MAXATT,
      leaked > 0 and not(bug("leakIgnored")),
      foreign and not(bug("foreignIgnored")),
    },
    up' = false, stop' = "failstop", cnt' = 0, latch' = false, rec' = recIdle, att' = att0, flowsOpen' = 0, leaked' = 0, foreign' = false,
    dmg' = COMPONENTS.mapBy(d => false), table' = table, repLeft' = repLeft,
    g' = { ...g, failStopped: true },
    conf_step' = if (REC) variant("FailStop", { c: c }) else conf_step,
  }
  // repairNoHold: the recovery repairs without quiescing
  action RecRepairNoHold(c: str): bool = all {
    bug("repairNoHold"), up, rec.get(c) == "idle", dmg.get(c),
    dmg' = dmg.set(c, false), cnt' = cnt, latch' = latch, rec' = rec, att' = att, flowsOpen' = flowsOpen,
    up' = up, stop' = stop, table' = table, repLeft' = repLeft, leaked' = leaked, foreign' = foreign,
    g' = { ...g, repairNoHold: true },
    conf_step' = if (REC) variant("RecRepairNoHold", { c: c }) else conf_step,
  }

  action recStep(c: str): bool = any {
    RecQuiesce(c), RecRepair(c), RecRedo(c), RecRestore(c), FailStop(c), RecRepairNoHold(c),
  }
  action step = {
    nondet c = COMPONENTS.oneOf()
    any { recStep(c), RepFail(c), Damage(c), OpenFlow, LeakHold, ForeignRestore, Crash, Restart }
  }

  // ------------------------------------------------------------------ invariants
  // LatchIffCount (QHOLD): admission is quiesced exactly while the hold count is above zero
  val LatchIffCount: bool = latch == (cnt > 0)
  // CountCoversRecoveries (QHOLD, D-UP-3): the hold count is the number of recoveries holding a hold (from quiesce to
  // restore) plus the leaked holds
  val CountCoversRecoveries: bool = cnt == COMPONENTS.filter(c => holding(c)).size() + leaked
  // NoReopenBeforeRepair (QHOLD): forwarding never reopens while a recovery is between quiesce and repair
  val NoReopenBeforeRepair: bool = not(g.reopenEarly)
  // no repair runs without that recovery's hold
  val NoRepairWithoutHold: bool = not(g.repairNoHold)
  // quiescence aborted every flow, and none opens while quiesced
  val QuiescedHoldsNoFlow: bool = latch implies flowsOpen == 0
  // BootRefusedOnTableWriteFailure (ADR-0184): serve runs only with the shared table written
  val ServeRunsOnlyWithTable: bool = up implies table
  // LeakedHoldKeepsForwardingQuiesced (QHOLD, D-UP-3): while the leaked-hold count is above zero, forwarding stays
  // quiesced (the count covers the leaked holds); only the end of the process clears both
  val LeakedHoldKeepsForwardingQuiesced: bool = leaked > 0 implies (cnt >= leaked and latch)
  // ForeignRestoreChangesNothing (D-UP-4, PD-1): a restore with a hold another ledger issued changes neither the hold
  // count, nor the latch, nor the owner's leaked-hold count
  val ForeignRestoreChangesNothing: bool = not(g.foreignChanged)
  val Safety: bool = and {
    ForeignRestoreChangesNothing, LeakedHoldKeepsForwardingQuiesced, LatchIffCount, CountCoversRecoveries, NoReopenBeforeRepair, NoRepairWithoutHold, QuiescedHoldsNoFlow,
    ServeRunsOnlyWithTable,
  }

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // Deterministic scenarios (`InitWith` / `RestartWith` fix the open flow and boot step 8's outcome), each
  // ending in an `expect` of its outcome with `Safety` checked along the way. Runs on `quiesce_ok`.
  // QHOLD, one recovery: quiesce takes the first hold (latch, flows aborted), repair, restore reopens at zero
  run SingleRecoveryQuiescesRepairsRestores = InitWith(1)
    .expect(Safety and cnt == 0 and not(latch) and flowsOpen == 1)
    .then(Damage("C1")).then(RecQuiesce("C1"))
    .expect(Safety and cnt == 1 and latch and flowsOpen == 0 and rec.get("C1") == "held")
    .then(RecRepair("C1")).then(RecRestore("C1"))
    .expect(Safety and cnt == 0 and not(latch) and rec.get("C1") == "idle" and not(g.stillQuiesced))
    .then(OpenFlow).expect(Safety and flowsOpen == 1)
  // QHOLD: two recoveries overlap; the first restore leaves the count above zero and forwarding stays quiesced
  // until the second recovery has repaired and restored
  run TwoOverlappingRecoveriesKeepForwardingQuiescedUntilTheLastRestore = InitWith(1)
    .then(Damage("C1")).then(Damage("C2"))
    .then(RecQuiesce("C1")).then(RecQuiesce("C2"))
    .expect(Safety and cnt == 2 and latch and g.overlapped and flowsOpen == 0)
    .then(RecRepair("C1")).then(RecRestore("C1"))
    .expect(Safety and cnt == 1 and latch and g.stillQuiesced and not(g.reopenEarly) and rec.get("C2") == "held")
  // ... and no flow opens while the count is above zero
  run NoFlowOpensWhileCountedHoldRemains = TwoOverlappingRecoveriesKeepForwardingQuiescedUntilTheLastRestore
    .then(OpenFlow).fail()
  run TwoOverlappingRecoveriesReopenOnlyAtZero = TwoOverlappingRecoveriesKeepForwardingQuiescedUntilTheLastRestore
    .then(RecRepair("C2")).then(RecRestore("C2"))
    .expect(Safety and cnt == 0 and not(latch) and not(g.reopenEarly))
    .then(OpenFlow).expect(Safety and flowsOpen == 1)
  // QHOLD, D-UP-3: a hold dropped without a restore (LeakHold), here during a live recovery, keeps the count (fail
  // closed) and raises the leaked-hold count; the recovery's own restore leaves forwarding quiesced; the supervisor
  // fail-stops serve at its audit tick on a leaked-hold count above zero; the restart converges from a clean count
  run DroppedHoldKeepsForwardingQuiescedThenFailStops = InitWith(1)
    .then(Damage("C1")).then(RecQuiesce("C1")).then(LeakHold)
    .expect(Safety and cnt == 2 and latch and leaked == 1 and rec.get("C1") == "held" and g.dropped)
    .then(RecRepair("C1")).then(RecRestore("C1"))
    .expect(Safety and cnt == 1 and latch and leaked == 1 and rec.get("C1") == "idle" and flowsOpen == 0)
    .then(FailStop("C1")).expect(Safety and not(up) and stop == "failstop" and cnt == 0 and not(latch) and g.failStopped and leaked == 0)
    .then(RestartWith(true)).expect(Safety and up and stop == "none" and cnt == 0 and leaked == 0)
    .then(OpenFlow).expect(Safety and flowsOpen == 1)
  // D-UP-4 (PD-1): during a live recovery, a restore with a hold another ledger issued returns ForeignHold and changes
  // nothing (count, latch, leaked count, the recovery's own hold); the supervisor fail-stops serve (cause
  // ForeignQuiescenceHold); the restart converges from a clean count
  run ForeignHoldRestoreChangesNothingThenFailStops = InitWith(1)
    .then(Damage("C1")).then(RecQuiesce("C1"))
    .expect(Safety and cnt == 1 and latch and leaked == 0 and flowsOpen == 0 and not(foreign))
    .then(ForeignRestore)
    .expect(Safety and foreign and cnt == 1 and latch and leaked == 0 and rec.get("C1") == "held" and flowsOpen == 0)
    .then(FailStop("C1"))
    .expect(Safety and not(up) and stop == "failstop" and g.failStopped and not(foreign) and cnt == 0 and leaked == 0)
    .then(RestartWith(true)).expect(Safety and up and stop == "none" and cnt == 0 and not(latch) and not(foreign))
    .then(OpenFlow).expect(Safety and flowsOpen == 1)
  // UP-7: a repair that returns Err is a failed attempt; the recovery repairs again under its hold
  run RepairFailureCountsAttemptThenRepairs = InitWith(0)
    .then(Damage("C1")).then(RecQuiesce("C1")).then(RepFail("C1"))
    .expect(Safety and att.get("C1") == 1 and cnt == 1 and latch and dmg.get("C1"))
    .then(RecRepair("C1")).then(RecRestore("C1"))
    .expect(Safety and not(latch) and att.get("C1") == 0 and cnt == 0)
  // UP-7: the post-repair audit finds the component damaged again: a failed attempt, repaired again under the hold
  run PostRepairAuditFailureCountsAttempt = InitWith(0)
    .then(Damage("C2")).then(RecQuiesce("C2")).then(RecRepair("C2"))
    .then(Damage("C2")).expect(Safety and rec.get("C2") == "repaired" and dmg.get("C2"))
    .then(RecRedo("C2")).expect(Safety and rec.get("C2") == "held" and att.get("C2") == 1)
    .then(RecRepair("C2")).then(RecRestore("C2"))
    .expect(Safety and not(latch) and cnt == 0)
  // ADR-0124 (quiesce_live: a repair may fail forever): the budget is spent, serve fail-stops; the restart
  // converges again
  run SpentBudgetFailStopsAndRestartConverges = InitWith(0)
    .then(Damage("C1")).then(RecQuiesce("C1")).then(RepFail("C1")).then(RepFail("C1"))
    .expect(Safety and att.get("C1") == MAXATT and latch)
    .then(FailStop("C1"))
    .expect(Safety and not(up) and stop == "failstop" and cnt == 0 and not(latch) and g.failStopped)
    .then(RestartWith(true)).expect(Safety and up and cnt == 0)
  // crash during a recovery: the process-scoped hold vanishes with the process; the restart's boot step 8
  // writes the table again
  run CrashDuringRecoveryThenRestartRecovers = InitWith(1)
    .then(Damage("C1")).then(RecQuiesce("C1")).then(Crash)
    .expect(Safety and not(up) and stop == "crash" and cnt == 0 and not(latch))
    .then(RestartWith(true)).expect(Safety and up and cnt == 0 and not(latch))
    .then(OpenFlow).expect(Safety and flowsOpen == 1)
  // ADR-0184: boot step 8's write of the shared table fails: boot is refused, nothing opens, the supervisor
  // restart retries; the next write succeeds
  run FailedBootTableWriteRefusesBootThenRetrySucceeds = InitDown
    .then(RestartWith(false))
    .expect(Safety and not(up) and stop == "refused" and g.refusedBoot)
    .then(RestartWith(false)).expect(Safety and not(up) and stop == "refused")
    .then(RestartWith(true)).expect(Safety and up and stop == "none" and table)

  // ------------------------------------------------------------------ witnesses (expected violated)
  val WitnessNoOverlap: bool = not(g.overlapped)
  val WitnessNoStillQuiesced: bool = not(g.stillQuiesced)
  val WitnessNoFailStop: bool = not(g.failStopped)
  val WitnessNoDroppedHold: bool = not(g.dropped)
  val WitnessNoRefusedBoot: bool = not(g.refusedBoot)

  // ------------------------------------------------------------------ liveness (TLC)
  // fairness of component c's own recovery steps only (no fairness for any other recovery, for damage,
  // crash, restart, leak)
  temporal OwnFair(c: str): bool = and {
    weakFair(RecQuiesce(c), (rec, cnt, dmg)), weakFair(RecRepair(c), (rec, cnt, dmg)),
    weakFair(RecRedo(c), (rec, cnt, dmg)), weakFair(RecRestore(c), (rec, cnt, dmg)),
    weakFair(FailStop(c), (rec, cnt, dmg, up)),
  }
  // (QHOLD, ADR-0124, UP-7) a recovery episode ends — restored, or serve stopped — under the recovery's own
  // fairness, whatever the other recovery and recurring damage do
  temporal C1EpisodeEnds =
    OwnFair("C1").implies(always((rec.get("C1") != "idle").implies(eventually(rec.get("C1") == "idle" or not(up)))))
  temporal C2EpisodeEnds =
    OwnFair("C2").implies(always((rec.get("C2") != "idle").implies(eventually(rec.get("C2") == "idle" or not(up)))))
  // (QHOLD) a damaged component is repaired (or serve stops) under its own recovery's fairness only: no other
  // recovery can pre-empt the repair forever
  temporal C1Repaired =
    OwnFair("C1").implies(always(dmg.get("C1").implies(eventually(not(dmg.get("C1")) or not(up)))))
  // LeakedHoldEndsInFailStop (QHOLD, ADR-0124, D-UP-3): a leaked hold always ends in the supervisor's fail-stop
  // (serve stops), under the same fairness (the audit tick's fail-stop step is weakly fair)
  temporal LeakedHoldEndsInFailStop =
    OwnFair("C1").implies(always((leaked > 0).implies(eventually(not(up)))))
  // ForeignHoldEndsInFailStop (D-UP-4, PD-1, ADR-0124): a restore that returned ForeignHold always ends in the
  // supervisor's fail-stop (cause ForeignQuiescenceHold), under the same fairness
  temporal ForeignHoldEndsInFailStop =
    OwnFair("C1").implies(always(foreign.implies(eventually(not(up)))))
}

// ------------------------------------------------------------------ design instances
// crash + restart (boot step 8's write may fail and refuse boot), one repair failure, recurring damage
module quiesce_ok { import quiescence(OFF = Set(), CFG = "ok", REC = false).* from "./quiescence" }
// no crash; a repair may fail forever; recurring damage
module quiesce_live { import quiescence(OFF = Set(), CFG = "live", REC = false).* from "./quiescence" }
// conformance main (D-UP-1): crash + restart and a repair that may fail forever, recording `conf_step`; only the named
// runs are checked on it
module quiesce_conf { import quiescence(OFF = Set(), CFG = "conf", REC = true).* from "./quiescence" }
