/**
 * Existing Metal admission/completion owner protocol (P2, P5).
 * Two invocations, two possible checkouts, one exclusive maintainer.
 * Phase: 0 absent; 1 registered; 2 shared barrier held; 3 checkout held;
 * 4 command running; 5 main ended but unit live; 6 unit stopped.
 * StopOwner is the blocking systemd owner operation: native evidence discharges
 * drain-before-finalizer and return-after-control-group-stop. It is not a new
 * runtime operation or recovery protocol. Record validation/atomic publication,
 * correct identity/placement and kernel namespace isolation are explicit inputs,
 * not kernel facts proved by this state machine (README and feature delta).
 */
module runner_protocol {
  const OFF: Set[str]
  const REC: bool
  val IDS = Set(1, 2)
  type Run = { p: int, ctl: bool, checked: bool, ever: bool, starts: int,
               term: int, desc: bool, roots: bool, receipt: int }
  type ConfStep = { tag: str, value: { actor: int, value: int } }
  var runs: int -> Run
  var second_tree: int
  var maintenance: int // 0 waiting, 1 exclusive barrier, 2 mutating, 3 resolved
  var source_revision: int -> int
  var observed: int -> int // model-only collection history, -2 = no terminal answer
  var conf_step: ConfStep  // model-only replay action, never projected from driver state

  val blank: Run = { p: 0, ctl: true, checked: false, ever: false, starts: 0,
                    term: -1, desc: false, roots: false, receipt: 0 }
  def tree(r: int): int = if (r == 1) 1 else second_tree
  def shared(r: int): bool = Set(2, 3, 4).contains(runs.get(r).p)
  def checkout(r: int): bool = Set(3, 4).contains(runs.get(r).p)
  def liveAdmitted(r: int): bool = runs.get(r).ever and runs.get(r).p < 6
  def canBarrier(r: int): bool = and {
    runs.get(r).p == 1, not(Set(1, 2).contains(maintenance)),
    not(OFF.contains("ordinaryExclusive")) or not(IDS.exists(a => shared(a))),
  }
  def canCheckout(r: int): bool = and {
    runs.get(r).p == 2,
    not(IDS.exists(a => a != r and checkout(a) and tree(a) == tree(r))),
  }
  def canSync(c: int): bool = not(IDS.exists(a => checkout(a) and tree(a) == c))
  val canMaintenanceBegin = maintenance == 1 and not(IDS.exists(r => liveAdmitted(r)))
  def result(r: int): int = {
    val x = runs.get(r)
    if (x.term == -1) 70
    else if (x.receipt != 1) 74
    else x.term
  }

  action initFor(c: int): bool = all {
    runs' = IDS.mapBy(r => blank), second_tree' = c, maintenance' = 0,
    source_revision' = IDS.mapBy(c => 0), observed' = IDS.mapBy(r => -2),
    conf_step' = { tag: "init", value: { actor: 0, value: c } },
  }
  action init = { nondet c = IDS.oneOf(); initFor(c) }
  action setRun(r: int, x: Run, op: str, value: int): bool = all {
    runs' = runs.set(r, x), second_tree' = second_tree, maintenance' = maintenance,
    source_revision' = source_revision, observed' = observed,
    conf_step' = if (REC) { tag: op, value: { actor: r, value: value } } else conf_step,
  }
  action Register(r: int): bool = all { runs.get(r).p == 0, setRun(r, { ...blank, p: 1 }, "Register", 0) }
  action Barrier(r: int): bool = all { canBarrier(r), setRun(r, { ...runs.get(r), p: 2 }, "Barrier", 0) }
  action Checkout(r: int): bool = all {
    canCheckout(r), setRun(r, { ...runs.get(r), p: 3, ever: true, roots: true }, "Checkout", 0),
  }
  action Prepare(r: int): bool = all {
    runs.get(r).p == 3, not(runs.get(r).checked),
    runs' = runs.set(r, { ...runs.get(r), checked: true }),
    source_revision' = source_revision.set(tree(r), 0), second_tree' = second_tree,
    maintenance' = maintenance, observed' = observed,
    conf_step' = if (REC) { tag: "Prepare", value: { actor: r, value: 0 } } else conf_step,
  }
  action Start(r: int): bool = all {
    runs.get(r).p == 3, runs.get(r).checked, runs.get(r).ctl,
    setRun(r, { ...runs.get(r), p: 4, starts: runs.get(r).starts + 1, desc: true }, "Start", 0),
  }
  action LoseController(r: int): bool = all {
    Set(1, 2, 3, 4).contains(runs.get(r).p), runs.get(r).ctl,
    setRun(r, if (runs.get(r).p == 4) { ...runs.get(r), ctl: false }
      else { ...runs.get(r), ctl: false, p: 5, term: 75 }, "LoseController", 0),
  }
  action AdmissionTimeout(r: int): bool = all {
    Set(1, 2).contains(runs.get(r).p),
    setRun(r, { ...runs.get(r), p: 5, term: 75 }, "AdmissionTimeout", 0),
  }
  action AbortPreparation(r: int, code: int): bool = all {
    runs.get(r).p == 3, Set(75, 78).contains(code),
    setRun(r, { ...runs.get(r), p: 5, term: code }, "AbortPreparation", code),
  }
  action MainExit(r: int, code: int): bool = all {
    runs.get(r).p == 4, Set(0, 255).contains(code),
    setRun(r, { ...runs.get(r), p: 5, term: code }, "MainExit", code),
  }
  action SupervisorLost(r: int): bool = all {
    Set(1, 2, 3, 4).contains(runs.get(r).p),
    setRun(r, { ...runs.get(r), p: 5, term: -1 }, "SupervisorLost", 0),
  }
  action StopOwner(r: int, outcome: int): bool = all {
    runs.get(r).p == 5, Set(0, 1, 2, 3).contains(outcome),
    setRun(r, { ...runs.get(r), p: 6, desc: false, receipt: (if (outcome == 3) 0 else outcome),
      roots: runs.get(r).roots and not(Set(1, 3).contains(outcome)) }, "StopOwner", outcome),
  }
  action Finish(r: int): bool = { nondet outcome = Set(0, 1, 2, 3).oneOf(); StopOwner(r, outcome) }
  action Collect(r: int): bool = {
    val early = OFF.contains("collectAtTerminal") and runs.get(r).p == 5 and runs.get(r).term != -1
    all {
      runs.get(r).p == 6 or early,
      observed' = observed.set(r, if (early) runs.get(r).term else result(r)),
      runs' = runs, second_tree' = second_tree, maintenance' = maintenance,
      source_revision' = source_revision,
      conf_step' = if (REC) { tag: "Collect", value: { actor: r, value: 0 } } else conf_step,
    }
  }
  action Sync(c: int): bool = all {
    canSync(c), source_revision.get(c) == 0,
    source_revision' = source_revision.set(c, 1), runs' = runs,
    second_tree' = second_tree, maintenance' = maintenance, observed' = observed,
    conf_step' = if (REC) { tag: "Sync", value: { actor: c, value: 1 } } else conf_step,
  }
  action NoSyncRefusal(r: int): bool = all {
    runs.get(r).p == 3, source_revision.get(tree(r)) != 0,
    setRun(r, { ...runs.get(r), p: 5, term: 78 }, "NoSyncRefusal", 0),
  }
  action Maintain(op: int): bool = all {
    if (op == 1) maintenance == 0 and not(IDS.exists(r => shared(r)))
    else if (op == 2) canMaintenanceBegin
    else if (op == 3) maintenance == 2
    else maintenance == 0,
    maintenance' = if (op == 0) 3 else op,
    runs' = runs, second_tree' = second_tree, observed' = observed,
    source_revision' = source_revision,
    conf_step' = if (REC) { tag: "Maintain", value: { actor: 0, value: op } } else conf_step,
  }
  action Idle = all {
    runs' = runs, second_tree' = second_tree, maintenance' = maintenance,
    source_revision' = source_revision, observed' = observed,
    conf_step' = if (REC) { tag: "Idle", value: { actor: 0, value: 0 } } else conf_step,
  }
  action step = {
    nondet r = IDS.oneOf()
    nondet outcome = Set(0, 1, 2, 3).oneOf()
    any { Register(r), Barrier(r), Checkout(r), Prepare(r), Start(r), LoseController(r),
      AdmissionTimeout(r), AbortPreparation(r, 75), AbortPreparation(r, 78), MainExit(r, 0), MainExit(r, 255),
      SupervisorLost(r), StopOwner(r, outcome), Collect(r), Sync(r), NoSyncRefusal(r),
      Maintain(0), Maintain(1), Maintain(2), Maintain(3), Idle }
  }
  val CheckoutExclusive = not(and { checkout(1), checkout(2), tree(1) == tree(2) })
  val OrdinaryDoesNotBlockOrdinary = IDS.forall(r =>
    (runs.get(r).p == 1 and not(Set(1, 2).contains(maintenance))).implies(canBarrier(r)))
  val MaintenanceExcludesOwners = (maintenance == 2).implies(not(IDS.exists(r => shared(r) or liveAdmitted(r))))
  val SourceStableDuringCommand = IDS.forall(r => (runs.get(r).p == 4).implies(source_revision.get(tree(r)) == 0))
  val AtMostOneStart = IDS.forall(r => runs.get(r).starts <= 1)
  val NoPrematureCommandCompletion = IDS.forall(r => Set(0, 255).contains(observed.get(r)).implies(and {
    runs.get(r).p == 6, runs.get(r).receipt == 1,
    not(runs.get(r).roots), not(runs.get(r).desc), observed.get(r) == runs.get(r).term,
  }))
  val NoFabricatedResult = IDS.forall(r => (runs.get(r).term == -1).implies(not(Set(0, 255).contains(observed.get(r)))))
  val Safety = and { CheckoutExclusive, OrdinaryDoesNotBlockOrdinary, MaintenanceExcludesOwners,
    SourceStableDuringCommand, AtMostOneStart, NoPrematureCommandCompletion, NoFabricatedResult }
  temporal FairOwners = IDS.forall(r => and {
    weakFair(AdmissionTimeout(r), (runs, second_tree, maintenance, source_revision, observed, conf_step)),
    weakFair(Barrier(r), (runs, second_tree, maintenance, source_revision, observed, conf_step)),
    weakFair(Checkout(r), (runs, second_tree, maintenance, source_revision, observed, conf_step)),
    weakFair(Prepare(r), (runs, second_tree, maintenance, source_revision, observed, conf_step)),
    weakFair(Start(r), (runs, second_tree, maintenance, source_revision, observed, conf_step)),
    weakFair(MainExit(r, 0), (runs, second_tree, maintenance, source_revision, observed, conf_step)),
    weakFair(Finish(r), (runs, second_tree, maintenance, source_revision, observed, conf_step)),
  })
  temporal FairMaintenance = and {
    weakFair(Maintain(1), (runs, second_tree, maintenance, source_revision, observed, conf_step)),
    weakFair(Maintain(2), (runs, second_tree, maintenance, source_revision, observed, conf_step)),
    weakFair(Maintain(3), (runs, second_tree, maintenance, source_revision, observed, conf_step)),
  }
  temporal OwnersEventuallyEnd = FairOwners.implies(IDS.forall(r => always((runs.get(r).p > 0).implies(eventually(runs.get(r).p == 6)))))
  temporal MaintenanceEventuallyResolves = (FairOwners and FairMaintenance).implies(eventually(maintenance == 3))

  run independent_worktrees = initFor(2).then(Register(1)).then(Register(2))
    .then(Barrier(1)).then(Barrier(2)).then(Checkout(1)).then(Checkout(2))
    .then(Prepare(1)).then(Prepare(2)).then(Start(1)).then(Start(2))
    .expect(runs.get(1).p == 4 and runs.get(2).p == 4 and Safety)
    .then(MainExit(1, 0)).then(StopOwner(1, 1)).then(Collect(1))
    .expect(runs.get(2).p == 4 and observed.get(1) == 0 and Safety)
  run same_checkout_excludes_sync = initFor(1).then(Register(1)).then(Register(2))
    .then(Barrier(1)).then(Barrier(2)).then(Checkout(1)).then(Prepare(1)).then(Start(1))
    .expect(not(canCheckout(2)) and not(canSync(1)) and Safety)
    .then(MainExit(1, 0)).then(StopOwner(1, 1)).then(Checkout(2)).expect(Safety)
  run controller_eof_before_start = initFor(2).then(Register(1)).then(Barrier(1))
    .then(Checkout(1)).then(Prepare(1)).then(LoseController(1)).then(StopOwner(1, 1)).then(Collect(1))
    .expect(observed.get(1) == 75 and runs.get(1).starts == 0 and Safety)
  run controller_loss_after_start = initFor(2).then(Register(1)).then(Barrier(1)).then(Checkout(1))
    .then(Prepare(1)).then(Start(1)).then(LoseController(1)).expect(runs.get(1).p == 4)
    .then(MainExit(1, 255)).then(StopOwner(1, 1)).then(Collect(1))
    .expect(observed.get(1) == 255 and Safety)
  run supervisor_loss_is_not_a_result = initFor(2).then(Register(1)).then(Barrier(1)).then(Checkout(1))
    .then(Prepare(1)).then(Start(1)).then(SupervisorLost(1)).then(StopOwner(1, 1)).then(Collect(1))
    .expect(observed.get(1) == 70 and Safety)
  run cleanup_failure_keeps_command = initFor(2).then(Register(1)).then(Barrier(1)).then(Checkout(1))
    .then(Prepare(1)).then(Start(1)).then(MainExit(1, 0)).then(StopOwner(1, 2)).then(Collect(1))
    .expect(observed.get(1) == 74 and runs.get(1).term == 0 and runs.get(1).roots and Safety)
  run missing_completion_refuses = initFor(2).then(Register(1)).then(Barrier(1)).then(Checkout(1))
    .then(Prepare(1)).then(Start(1)).then(MainExit(1, 0)).then(StopOwner(1, 3)).then(Collect(1))
    .expect(observed.get(1) == 74 and not(runs.get(1).roots) and Safety)
  run maintenance_waits_for_stop = initFor(2).then(Register(1)).then(Barrier(1)).then(Checkout(1))
    .then(Prepare(1)).then(Start(1)).then(MainExit(1, 0)).then(Maintain(1))
    .expect(not(canMaintenanceBegin) and Safety)
    .then(StopOwner(1, 1)).then(Maintain(2)).then(Maintain(3)).expect(Safety)
  run no_sync_mismatch_refuses = initFor(2).then(Sync(1)).then(Register(1)).then(Barrier(1))
    .then(Checkout(1)).then(NoSyncRefusal(1)).then(StopOwner(1, 1)).then(Collect(1))
    .expect(observed.get(1) == 78 and Safety)
  run admission_timeout = initFor(2).then(Register(1)).then(Barrier(1))
    .then(AdmissionTimeout(1)).then(StopOwner(1, 1)).then(Collect(1))
    .expect(observed.get(1) == 75 and runs.get(1).starts == 0 and Safety)
  run preflight_aborts_before_start = initFor(2).then(Register(1)).then(Barrier(1)).then(Checkout(1))
    .then(AbortPreparation(1, 78)).then(StopOwner(1, 1)).then(Collect(1))
    .expect(observed.get(1) == 78 and runs.get(1).starts == 0 and Safety)
  run missing_rsync_aborts_before_start = initFor(2).then(Register(1)).then(Barrier(1)).then(Checkout(1))
    .then(AbortPreparation(1, 75)).then(StopOwner(1, 1)).then(Collect(1))
    .expect(observed.get(1) == 75 and runs.get(1).starts == 0 and Safety)
  run repeat_collection_agrees = initFor(2).then(Register(1)).then(Barrier(1)).then(Checkout(1))
    .then(Prepare(1)).then(Start(1)).then(MainExit(1, 255)).then(StopOwner(1, 1))
    .then(Collect(1)).expect(observed.get(1) == 255).then(Collect(1))
    .expect(observed.get(1) == 255 and Safety)
}
module runner_ok { import runner_protocol(OFF = Set(), REC = false).* from "./runner_protocol" }
module runner_conf { import runner_protocol(OFF = Set(), REC = true).* from "./runner_protocol" }
