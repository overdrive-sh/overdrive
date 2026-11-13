/**
 * Shared-guest owner protocol (feature metal-test-placement-attribute; DDD-8 owner uniqueness and
 * lifetime, DDD-9 serving order, DDD-10 no reuse after failure).
 *
 * ONE guest key, three clients (nextest test processes), one anchor (the nextest process), one run.
 *
 * Decisions:
 *   D-OWNER  first-arrival election under an exclusive lock; at most one live owner.
 *   D-LIFE   the guest never outlives its owner (assumption A-1); the owner exits when the anchor is gone;
 *            run teardown is the backstop.
 *   D-SERIAL the owner serves attached clients FIFO, one exec in flight.
 *   D-DETACH a disconnect never cancels an in-guest exec: queued -> removed; running -> detached, the exec
 *            runs to its bound, its result is discarded, the next client waits.
 *   D-TAINT  a failed or lost guest is discarded; the next attach boots a replacement (generation + 1).
 *
 * Assumptions (README maps each to its discharging validation item):
 *   A-1 owner death kills the guest (owner is init of its PID namespace). Modelled as a rule of OwnerCrash
 *       and OwnerExit; hazard `orphan` removes it.
 *   A-2 client processes are children of the anchor: once the anchor is Gone they take no action.
 *   A-3 the exec/boot/lost outcomes are environment choices; a test exec that fails is a nonzero exit.
 *   A-4 a nextest retry is a NEW client (a fresh client id), not a re-run of a finished one.
 *   A-5 Fault budget: OwnerCrash and GuestLost together occur at most FAULTS times (finite faults are the
 *       fairness precondition of the progress property).
 *
 * Model-only (not projectable from OwnerCore::observe()): lock, torn, bad, execgen, execs, seq, nseq, lastb,
 * fifobad, faults, anchor, phase of clients that are not in the queue (the clients' own outcomes), conf_step.
 */
module shared_guest_conf {
  type ConfStep =
    | Init
    | Arrive({ c: int }) | ElectBegin({ c: int }) | OwnerStarted({ c: int }) | Connect({ c: int })
    | ElectorDies({ c: int }) | ClientDisconnect({ c: int })
    | BeginBoot | BootOk | BootFail | BeginExec | ExecDonePass | ExecDoneFail | GuestLost
    | OwnerCrash | AnchorGone | OwnerExit | RunTeardown
}

module shared_guest {
  import shared_guest_conf.ConfStep from "./shared_guest"
  const OFF: Set[str]   // rules switched off: "noLock" "noSerial" "noTaint" "orphan" "cancelOnDisconnect"
  const REC: bool       // true only on the conformance main: conf_step records the action taken

  val CL = Set(1, 2, 3)
  val FAULTS = 2
  // client phases
  val IDLE = 0
  val ARRIVING = 1
  val STARTING = 2   // holds the election lock, starting the owner
  val QUEUED = 3
  val RUNNING = 4
  val DETACHED = 5   // disconnected while its exec runs in the guest
  val GONE = 6       // disconnected, no result owed
  val PASS = 10
  val FAIL = 11
  val LOST = 12      // GuestLost
  val BOOTFAIL = 13
  val OWNERLOST = 14

  type S = {
    anchor: bool, torn: bool, lock: int, owners: int, guest: int, gen: int, inflight: int,
    execgen: int, execs: int, queue: List[int], phase: int -> int, bad: Set[int],
    seq: int -> int, nseq: int, lastb: int, fifobad: bool, faults: int
  }

  var anchor: bool          // nextest process alive
  var torn: bool            // run teardown has run
  var lock: int             // election lock holder, 0 free
  var owners: int           // live owner processes
  var guest: int            // 0 NoGuest, 1 Booting, 2 Ready
  var gen: int              // generation of the current / last guest
  var inflight: int         // client whose exec the owner holds in flight, 0 none
  var execgen: int          // ghost: generation the in-flight exec runs on
  var execs: int            // ghost: execs actually running in the guest
  var queue: List[int]      // attached clients waiting, FIFO
  var phase: int -> int
  var bad: Set[int]         // ghost: generations that failed or were lost
  var seq: int -> int       // ghost: enqueue order
  var nseq: int
  var lastb: int            // ghost: seq of the last client whose exec began
  var fifobad: bool         // ghost: an exec began out of queue order
  var faults: int           // fault budget left
  var conf_step: ConfStep   // model-only

  val cur: S = {
    anchor: anchor, torn: torn, lock: lock, owners: owners, guest: guest, gen: gen, inflight: inflight,
    execgen: execgen, execs: execs, queue: queue, phase: phase, bad: bad, seq: seq, nseq: nseq,
    lastb: lastb, fifobad: fifobad, faults: faults
  }

  def ph(n: S, c: int, v: int): S = { ...n, phase: n.phase.set(c, v) }
  def enq(n: S, c: int): S = {
    { ...n, phase: n.phase.set(c, QUEUED), queue: n.queue.append(c),
      seq: n.seq.set(c, n.nseq), nseq: n.nseq + 1 }
  }
  // the owner and (if kill) its guest are gone: clients that were attached to the owner lose it
  def ownerDown(n: S, kill: bool): S = {
    val left = n.owners - 1
    if (left > 0) { ...n, owners: left }
    else {
      ...n, owners: 0, inflight: 0, queue: List(),
      guest: if (kill) 0 else n.guest, execs: if (kill) 0 else n.execs,
      phase: CL.mapBy(c => {
        val p = n.phase.get(c)
        if (p == QUEUED or p == RUNNING) OWNERLOST else if (p == DETACHED) GONE else p
      })
    }
  }
  // the in-flight exec ended: its client gets `res` if still attached, else the result is discarded
  def execEnds(n: S, res: int): S = {
    val c = n.inflight
    val p = n.phase.get(c)
    { ...n, inflight: 0, execs: if (n.execs > 0) n.execs - 1 else 0,
      phase: n.phase.set(c, if (p == RUNNING) res else if (p == DETACHED) GONE else p) }
  }

  action commit(n: S, t: ConfStep): bool = all {
    anchor' = n.anchor, torn' = n.torn, lock' = n.lock, owners' = n.owners, guest' = n.guest,
    gen' = n.gen, inflight' = n.inflight, execgen' = n.execgen, execs' = n.execs, queue' = n.queue,
    phase' = n.phase, bad' = n.bad, seq' = n.seq, nseq' = n.nseq, lastb' = n.lastb,
    fifobad' = n.fifobad, faults' = n.faults,
    conf_step' = if (REC) t else conf_step,
  }

  // a separate action for init: TLA+ cannot share one action between init and step
  action commitInit(n: S, t: ConfStep): bool = all {
    anchor' = n.anchor, torn' = n.torn, lock' = n.lock, owners' = n.owners, guest' = n.guest,
    gen' = n.gen, inflight' = n.inflight, execgen' = n.execgen, execs' = n.execs, queue' = n.queue,
    phase' = n.phase, bad' = n.bad, seq' = n.seq, nseq' = n.nseq, lastb' = n.lastb,
    fifobad' = n.fifobad, faults' = n.faults,
    conf_step' = t,
  }

  action init = commitInit({
    anchor: true, torn: false, lock: 0, owners: 0, guest: 0, gen: 0, inflight: 0, execgen: 0, execs: 0,
    queue: List(), phase: CL.mapBy(c => IDLE), bad: Set(), seq: CL.mapBy(c => 0), nseq: 1, lastb: 0,
    fifobad: false, faults: FAULTS
  }, variant("Init", ()))

  // ---------------------------------------------------------------- clients
  action Arrive(c: int): bool = all {
    cur.anchor, cur.phase.get(c) == IDLE,
    commit(ph(cur, c, ARRIVING), variant("Arrive", { c: c })),
  }
  def canElect(c: int): bool = and {
    cur.anchor, cur.phase.get(c) == ARRIVING, cur.owners == 0,
    OFF.contains("noLock") or cur.lock == 0,
  }
  // D-OWNER: first arrival takes the exclusive lock (no live owner, lock free)
  action ElectBegin(c: int): bool = all {
    canElect(c),
    commit({ ...ph(cur, c, STARTING), lock: if (OFF.contains("noLock")) cur.lock else c },
      variant("ElectBegin", { c: c })),
  }
  // the elected client has started the detached owner, releases the lock and connects
  action OwnerStarted(c: int): bool = all {
    cur.anchor, cur.phase.get(c) == STARTING,
    commit(enq({ ...cur, owners: cur.owners + 1, lock: if (cur.lock == c) 0 else cur.lock }, c),
      variant("OwnerStarted", { c: c })),
  }
  // a nextest kill of the electing client before the owner exists: the kernel frees the lock
  action ElectorDies(c: int): bool = all {
    cur.anchor, cur.phase.get(c) == STARTING,
    commit({ ...ph(cur, c, GONE), lock: if (cur.lock == c) 0 else cur.lock }, variant("ElectorDies", { c: c })),
  }
  // a later arrival finds the live owner and connects
  action Connect(c: int): bool = all {
    cur.anchor, cur.phase.get(c) == ARRIVING, cur.owners > 0,
    commit(enq(cur, c), variant("Connect", { c: c })),
  }
  // D-DETACH
  action ClientDisconnect(c: int): bool = all {
    cur.anchor, Set(QUEUED, RUNNING).contains(cur.phase.get(c)),
    if (cur.phase.get(c) == QUEUED)
      commit({ ...ph(cur, c, GONE), queue: cur.queue.select(x => x != c) }, variant("ClientDisconnect", { c: c }))
    else if (OFF.contains("cancelOnDisconnect"))
      // hazard: the owner treats the disconnect as a cancel and frees the slot; the exec still runs
      commit({ ...ph(cur, c, GONE), inflight: 0 }, variant("ClientDisconnect", { c: c }))
    else
      commit(ph(cur, c, DETACHED), variant("ClientDisconnect", { c: c })),
  }

  // ---------------------------------------------------------------- owner: guest and exec (D-SERIAL, D-TAINT)
  action BeginBoot = all {
    cur.owners > 0, cur.guest == 0, cur.queue.length() > 0,
    commit({ ...cur, guest: 1, gen: cur.gen + 1 }, variant("BeginBoot", ())),
  }
  action BootOk = all { cur.guest == 1, commit({ ...cur, guest: 2 }, variant("BootOk", ())) }
  // typed outcome BootFailed to the head client; the owner stays alive with no guest
  action BootFail = all {
    cur.guest == 1, cur.queue.length() > 0,
    commit({ ...ph(cur, cur.queue.head(), BOOTFAIL), guest: 0, queue: cur.queue.tail() }, variant("BootFail", ())),
  }
  def canBeginExec: bool = and {
    cur.owners > 0, cur.guest == 2, cur.queue.length() > 0,
    OFF.contains("noSerial") or cur.inflight == 0,
  }
  action BeginExec = all {
    canBeginExec,
    val c = cur.queue.head()
    commit({ ...ph(cur, c, RUNNING), queue: cur.queue.tail(), inflight: c, execgen: cur.gen,
             execs: cur.execs + 1, lastb: if (cur.seq.get(c) > cur.lastb) cur.seq.get(c) else cur.lastb,
             fifobad: cur.fifobad or cur.seq.get(c) < cur.lastb }, variant("BeginExec", ())),
  }
  action ExecDonePass = all {
    cur.inflight != 0, cur.guest == 2,
    commit(execEnds(cur, PASS), variant("ExecDonePass", ())),
  }
  // nonzero exit: the guest is discarded (D-TAINT); hazard noTaint keeps it Ready
  action ExecDoneFail = all {
    cur.inflight != 0, cur.guest == 2,
    commit({ ...execEnds(cur, FAIL), bad: cur.bad.union(Set(cur.execgen)),
             guest: if (OFF.contains("noTaint")) 2 else 0 }, variant("ExecDoneFail", ())),
  }
  // the guest kernel crashed or the VMM exited (environment fault)
  action GuestLost = all {
    cur.owners > 0, cur.guest == 2, cur.faults > 0,
    val after = if (cur.inflight != 0) execEnds(cur, LOST) else cur
    commit({ ...after, guest: 0, execs: 0, bad: cur.bad.union(Set(cur.gen)), faults: cur.faults - 1 },
      variant("GuestLost", ())),
  }

  // ---------------------------------------------------------------- environment: owner death, anchor, teardown (D-LIFE)
  action OwnerCrash = all {
    cur.owners > 0, cur.faults > 0,
    commit({ ...ownerDown(cur, not(OFF.contains("orphan"))), faults: cur.faults - 1 }, variant("OwnerCrash", ())),
  }
  action AnchorGone = all { cur.anchor, commit({ ...cur, anchor: false }, variant("AnchorGone", ())) }
  action OwnerExit = all {
    not(cur.anchor), cur.owners > 0,
    commit(ownerDown(cur, not(OFF.contains("orphan"))), variant("OwnerExit", ())),
  }
  // backstop: removes every owner and guest of the run
  action RunTeardown = all {
    not(cur.anchor), not(cur.torn),
    commit({ ...ownerDown({ ...cur, owners: 1 }, true), torn: true, lock: 0 }, variant("RunTeardown", ())),
  }

  action Progress = {
    nondet c = CL.oneOf()
    any { Arrive(c), ElectBegin(c), OwnerStarted(c), Connect(c), BeginBoot, BootOk, BootFail,
          BeginExec, ExecDonePass, ExecDoneFail, OwnerExit, RunTeardown }
  }
  action step = {
    nondet c = CL.oneOf()
    any { Arrive(c), ElectBegin(c), OwnerStarted(c), ElectorDies(c), Connect(c), ClientDisconnect(c),
          BeginBoot, BootOk, BootFail, BeginExec, ExecDonePass, ExecDoneFail, GuestLost,
          OwnerCrash, AnchorGone, OwnerExit, RunTeardown }
  }

  // ---------------------------------------------------------------- invariants (each named after its decision)
  val I_OWNER_UNIQUE = owners <= 1                                                   // D-OWNER
  val I_SERIAL = execs <= 1                                                          // D-SERIAL
  val I_DETACH_NO_CANCEL = (execs > 0).implies(inflight != 0)                        // D-DETACH
  val I_NO_REUSE = (inflight != 0).implies(not(bad.contains(execgen)))               // D-TAINT
  val I_NO_ORPHAN = (guest != 0).implies(owners > 0)                                 // D-LIFE (A-1)
  val I_TEARDOWN_CLEAN = (not(anchor) and torn).implies(and { owners == 0, guest == 0, execs == 0 })  // D-LIFE
  val I_FIFO = not(fifobad)                                                          // D-SERIAL
  val I_DELIVER_ONLY_ATTACHED = and {                                                // D-DETACH
    queue.indices().forall(i => phase.get(queue[i]) == QUEUED),
    CL.forall(c => Set(RUNNING, DETACHED).contains(phase.get(c)).implies(inflight == c)),
  }
  val Safety = and { I_OWNER_UNIQUE, I_SERIAL, I_DETACH_NO_CANCEL, I_NO_REUSE, I_NO_ORPHAN,
                     I_TEARDOWN_CLEAN, I_FIFO, I_DELIVER_ONLY_ATTACHED }

  // progress: with the anchor alive, every attached client gets a terminal outcome (A-5 bounds the faults)
  temporal FairProgress = weakFair(Progress, (anchor, torn, lock, owners, guest, gen, inflight, execgen, execs,
    queue, phase, bad, seq, nseq, lastb, fifobad, faults, conf_step))
  temporal EveryClientTerminates = (FairProgress and always(anchor)).implies(CL.forall(c =>
    always(Set(ARRIVING, STARTING, QUEUED, RUNNING, DETACHED).contains(phase.get(c)).implies(eventually(phase.get(c) >= GONE)))))

  // ---------------------------------------------------------------- named runs (one per critical scenario)
  // common prefix: client 1 elected and owner started, clients 2 and 3 connected
  action up3 = init.then(Arrive(1)).then(Arrive(2)).then(Arrive(3)).then(ElectBegin(1)).then(OwnerStarted(1))
    .then(Connect(2)).then(Connect(3))

  run happy_path_three_clients_share_one_guest = up3
    .then(BeginBoot).then(BootOk)
    .then(BeginExec).then(ExecDonePass).then(BeginExec).then(ExecDonePass).then(BeginExec).then(ExecDonePass)
    .expect(gen == 1 and owners == 1 and CL.forall(c => phase.get(c) == PASS) and Safety)

  run concurrent_first_arrival_one_owner = init.then(Arrive(1)).then(Arrive(2)).then(ElectBegin(1))
    .expect(lock == 1 and not(canElect(2)) and Safety)
    .then(OwnerStarted(1)).expect(owners == 1 and lock == 0 and not(canElect(2)) and Safety)
    .then(Connect(2)).expect(owners == 1 and queue == List(1, 2) and Safety)

  run failure_then_replacement_guest = up3
    .then(BeginBoot).then(BootOk).then(BeginExec).then(ExecDoneFail)
    .expect(phase.get(1) == FAIL and guest == 0 and bad.contains(1) and Safety)
    .then(BeginBoot).then(BootOk).then(BeginExec)
    .expect(execgen == 2 and inflight == 2 and Safety)
    .then(ExecDonePass).then(BeginExec).then(ExecDonePass)
    .expect(gen == 2 and phase.get(2) == PASS and phase.get(3) == PASS and Safety)

  run guest_lost_mid_exec_then_replacement = up3
    .then(BeginBoot).then(BootOk).then(BeginExec).then(GuestLost)
    .expect(phase.get(1) == LOST and guest == 0 and inflight == 0 and execs == 0 and bad.contains(1) and Safety)
    .then(BeginBoot).then(BootOk).then(BeginExec)
    .expect(execgen == 2 and inflight == 2 and Safety)
    .then(ExecDonePass).expect(phase.get(2) == PASS and Safety)

  run owner_crash_then_new_owner_elected = init.then(Arrive(1)).then(Arrive(2)).then(Arrive(3))
    .then(ElectBegin(1)).then(OwnerStarted(1)).then(Connect(2))
    .then(BeginBoot).then(BootOk).then(BeginExec).then(OwnerCrash)
    .expect(owners == 0 and guest == 0 and phase.get(1) == OWNERLOST and phase.get(2) == OWNERLOST
            and queue == List() and Safety)
    .then(ElectBegin(3)).then(OwnerStarted(3)).then(BeginBoot)
    .expect(gen == 2 and owners == 1 and Safety)
    .then(BootOk).then(BeginExec).then(ExecDonePass)
    .expect(phase.get(3) == PASS and Safety)

  run disconnect_while_running_next_waits = up3
    .then(BeginBoot).then(BootOk).then(BeginExec).then(ClientDisconnect(1))
    .expect(phase.get(1) == DETACHED and inflight == 1 and execs == 1 and not(canBeginExec) and Safety)
    .then(ExecDonePass)
    .expect(phase.get(1) == GONE and inflight == 0 and execs == 0 and canBeginExec and Safety)
    .then(BeginExec).then(ExecDonePass)
    .expect(phase.get(2) == PASS and Safety)

  run disconnect_while_queued_is_removed = up3
    .then(BeginBoot).then(BootOk).then(BeginExec).then(ClientDisconnect(2))
    .expect(phase.get(2) == GONE and queue == List(3) and inflight == 1 and Safety)
    .then(ExecDonePass).then(BeginExec)
    .expect(inflight == 3 and Safety)
    .then(ExecDonePass).expect(phase.get(3) == PASS and phase.get(2) == GONE and Safety)

  run boot_failure_then_retry_boot_on_next_attach = init.then(Arrive(1)).then(ElectBegin(1)).then(OwnerStarted(1))
    .then(BeginBoot).then(BootFail)
    .expect(phase.get(1) == BOOTFAIL and owners == 1 and guest == 0 and queue == List() and Safety)
    .then(Arrive(2)).then(Connect(2)).then(BeginBoot).expect(gen == 2 and Safety)
    .then(BootOk).then(BeginExec).then(ExecDonePass)
    .expect(phase.get(2) == PASS and Safety)

  run anchor_gone_ends_owner_and_guest = up3
    .then(BeginBoot).then(BootOk).then(BeginExec).then(AnchorGone)
    .expect(owners == 1 and not(canElect(1)) and Safety)
    .then(OwnerExit)
    .expect(owners == 0 and guest == 0 and execs == 0 and phase.get(1) == OWNERLOST and phase.get(2) == OWNERLOST and Safety)
    .then(RunTeardown).expect(torn and owners == 0 and guest == 0 and Safety)

  run run_teardown_removes_leftovers = init.then(Arrive(1)).then(ElectBegin(1)).then(OwnerStarted(1))
    .then(BeginBoot).then(BootOk).then(AnchorGone).then(RunTeardown)
    .expect(torn and owners == 0 and guest == 0 and Safety)

  run retry_of_failed_client_gets_fresh_guest = init.then(Arrive(1)).then(ElectBegin(1)).then(OwnerStarted(1))
    .then(BeginBoot).then(BootOk).then(BeginExec).then(ExecDoneFail)
    .expect(phase.get(1) == FAIL and guest == 0 and Safety)
    .then(Arrive(3)).then(Connect(3)).then(BeginBoot).then(BootOk).then(BeginExec)
    .expect(execgen == 2 and inflight == 3 and Safety)
    .then(ExecDonePass).expect(phase.get(3) == PASS and gen == 2 and Safety)

  run elector_killed_before_owner_starts = init.then(Arrive(1)).then(Arrive(2)).then(ElectBegin(1))
    .then(ElectorDies(1)).expect(lock == 0 and owners == 0 and phase.get(1) == GONE and canElect(2) and Safety)
    .then(ElectBegin(2)).then(OwnerStarted(2)).expect(owners == 1 and Safety)
}

module guest_ok { import shared_guest(OFF = Set(), REC = false).* from "./shared_guest" }
module guest_conf { import shared_guest(OFF = Set(), REC = true).* from "./shared_guest" }
