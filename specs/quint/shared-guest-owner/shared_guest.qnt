/**
 * Shared-guest owner protocol (feature metal-test-placement-attribute; DDD-8 owner uniqueness and
 * lifetime, DDD-9 serving order, DDD-10 no reuse after failure).
 *
 * ONE guest key, three clients (nextest test processes), one run.
 *
 * Decisions:
 *   D-OWNER  first-arrival election under an exclusive lock; at most one live owner.
 *   D-LIFE   the guest never outlives its owner (assumption A-1).
 *   D-SERIAL the owner serves attached clients FIFO, one exec in flight.
 *   D-DETACH a disconnect never cancels an in-guest exec: queued -> removed; running -> detached, the exec
 *            runs to its bound, its result is discarded, the next client waits.
 *   D-TAINT  a failed, lost or detached-from exec taints its guest generation: the guest is discarded and the
 *            next attach boots a replacement (generation + 1).
 *   D-BOUND  an exec that does not finish is ended by the exec bound: the guest is discarded and the client
 *            gets the typed outcome BOUND (a hung exec must not hold the slot for ever).
 *   D-IDLE   the owner exits only when idle: no client queued, none running, none detached-running. Time is
 *            abstracted: IdleElapsed may fire at any moment its guard holds. A too-early idle exit only costs
 *            a reboot of the guest for the next client, never a wrong result.
 *
 * The owner is reactive, as OwnerCore::handle(Event) -> Vec<Effect> is: every owner event is ONE action and
 * ends with `drive` (if no exec is in flight and the queue is non-empty: Ready guest -> begin the exec of the
 * queue head, NoGuest -> begin a boot).
 *
 * Two kinds of action.
 *   Owner events (the conformance instance, ENV=false), each one OwnerCore event:
 *     Attached{c} ClientGone{c} BootFinished{ok} ExecFinished{pass} GuestLost ExecBound IdleElapsed
 *   Environment actions (ENV=true only; performed by the environment, not by OwnerCore):
 *     Arrive{c} ElectBegin{c} OwnerStarted{c} OwnerStartFails{c} OwnerCrash
 *   OwnerStarted also enqueues the elector (as Attached does) and drives.
 *
 * Instances: guest_ok (ENV=true, REC=false) exhaustive checks; guest_env (ENV=true, REC=true) environment
 * named runs; guest_conf (ENV=false, REC=true) owner-driven named runs, the `#[quint_test]` pins. In
 * guest_conf the owner is already started (owners = 1, lock free, clients IDLE) and no environment action is
 * enabled; `owners` is then 1 or 0 and equals not(OwnerObservation.stopped).
 *
 * Assumptions (README maps each to its discharging validation item):
 *   A-1 owner death kills the guest (owner is init of its PID namespace). Modelled as a rule of OwnerCrash
 *       and IdleElapsed; hazard `orphan` removes it.
 *   A-3 the exec/boot/lost outcomes are environment choices; a test exec that fails is a nonzero exit.
 *   A-4 a nextest retry is a NEW client (a fresh client id), not a re-run of a finished one.
 *   A-5 Fault budget: OwnerCrash and GuestLost together occur at most FAULTS times (finite faults are the
 *       fairness precondition of the progress property).
 *   A-6 election, owner start and attach are atomic w.r.t. the owner's own state (OwnerStarted registers the
 *       owner and enqueues the electing client in one step).
 *   A-8 a client connecting at the instant the owner exits idle sees the connection fail and re-arrives once
 *       through the election (a second cut is a GuestExecution failure at stage owner); the model makes
 *       Attached and IdleElapsed atomic. Discharged by a deterministic client-side test (design DDD-17).
 *
 * Projectable from OwnerCore::observe(): guest (NoGuest/Booting/Ready), gen (generation), inflight, queue,
 * and, in the ENV=false instance, owners via `stopped`.
 * Model-only: owners (for election: I_OWNER_UNIQUE; Tier-3 proves the real lock), lock, bad, execgen,
 * execs, faults, idleLoss, conf_step, and the per-client outcome phases (the driver
 * reads them from the Reply effect).
 */
module shared_guest_conf {
  type ConfStep =
    | Init
    | Arrive({ c: int }) | ElectBegin({ c: int }) | OwnerStarted({ c: int })
    | OwnerStartFails({ c: int }) | OwnerCrash
    | Attached({ c: int }) | ClientGone({ c: int }) | BootFinished({ ok: bool })
    | ExecFinished({ pass: bool }) | GuestLost | ExecBound | IdleElapsed
}

module shared_guest {
  import shared_guest_conf.ConfStep from "./shared_guest"
  const OFF: Set[str]   // rules off: "noLock" "noSerial" "noTaint" "orphan" "cancelOnDisconnect" "noBound" "idleAnyTime"
  const REC: bool       // true on the runs' mains: conf_step records the action taken
  const ENV: bool       // true: environment actions enabled; false: the conformance instance (owner already started)

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
  val STARTFAIL = 15 // the elector got a typed owner-start failure (owner binary missing)
  val BOUND = 16     // the exec bound ended the exec

  type S = {
    lock: int, owners: int, guest: int, gen: int, inflight: int,
    execgen: int, execs: int, queue: List[int], phase: int -> int, bad: Set[int],
    faults: int, idleLoss: bool
  }

  var lock: int             // election lock holder, 0 free
  var owners: int           // live owner processes
  var guest: int            // 0 NoGuest, 1 Booting, 2 Ready
  var gen: int              // generation of the current / last guest
  var inflight: int         // client whose exec the owner holds in flight, 0 none
  var execgen: int          // ghost: generation the in-flight exec runs on
  var execs: int            // ghost: execs actually running in the guest
  var queue: List[int]      // attached clients waiting, FIFO
  var phase: int -> int
  var bad: Set[int]         // ghost: generations that failed, were lost or were detached from
  var faults: int           // fault budget left
  var idleLoss: bool        // ghost: an idle exit dropped a queued, running or detached client
  var conf_step: ConfStep   // model-only

  val cur: S = {
    lock: lock, owners: owners, guest: guest, gen: gen, inflight: inflight,
    execgen: execgen, execs: execs, queue: queue, phase: phase, bad: bad,
    faults: faults, idleLoss: idleLoss
  }

  def ph(n: S, c: int, v: int): S = { ...n, phase: n.phase.set(c, v) }
  def enq(n: S, c: int): S = {
    { ...n, phase: n.phase.set(c, QUEUED), queue: n.queue.append(c) }
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
  // begin the exec of the queue head on the current generation
  def beginExec(n: S): S = {
    val c = n.queue.head()
    { ...ph(n, c, RUNNING), queue: n.queue.tail(), inflight: c, execgen: n.gen, execs: n.execs + 1 }
  }
  // the reactive step every owner event ends with (D-SERIAL: only with no exec in flight)
  def drive(n: S): S = {
    val go = and { n.owners > 0, n.queue.length() > 0, OFF.contains("noSerial") or n.inflight == 0 }
    if (not(go)) n
    else if (n.guest == 2) beginExec(n)
    else if (n.guest == 0) { ...n, guest: 1, gen: n.gen + 1 }
    else n
  }

  action commit(n: S, t: ConfStep): bool = all {
    lock' = n.lock, owners' = n.owners, guest' = n.guest,
    gen' = n.gen, inflight' = n.inflight, execgen' = n.execgen, execs' = n.execs, queue' = n.queue,
    phase' = n.phase, bad' = n.bad,
    faults' = n.faults, idleLoss' = n.idleLoss,
    conf_step' = if (REC) t else conf_step,
  }

  // a separate action for init: TLA+ cannot share one action between init and step
  action commitInit(n: S, t: ConfStep): bool = all {
    lock' = n.lock, owners' = n.owners, guest' = n.guest,
    gen' = n.gen, inflight' = n.inflight, execgen' = n.execgen, execs' = n.execs, queue' = n.queue,
    phase' = n.phase, bad' = n.bad,
    faults' = n.faults, idleLoss' = n.idleLoss,
    conf_step' = t,
  }

  action init = commitInit({
    lock: 0, owners: if (ENV) 0 else 1, guest: 0, gen: 0, inflight: 0, execgen: 0, execs: 0,
    queue: List(), phase: CL.mapBy(c => IDLE), bad: Set(),
    faults: FAULTS, idleLoss: false
  }, variant("Init", ()))

  // ---------------------------------------------------------------- environment: election (D-OWNER)
  action Arrive(c: int): bool = all {
    ENV, cur.phase.get(c) == IDLE,
    commit(ph(cur, c, ARRIVING), variant("Arrive", { c: c })),
  }
  def canElect(c: int): bool = and {
    ENV, cur.phase.get(c) == ARRIVING, cur.owners == 0,
    OFF.contains("noLock") or cur.lock == 0,
  }
  // D-OWNER: first arrival takes the exclusive lock (no live owner, lock free)
  action ElectBegin(c: int): bool = all {
    canElect(c),
    commit({ ...ph(cur, c, STARTING), lock: if (OFF.contains("noLock")) cur.lock else c },
      variant("ElectBegin", { c: c })),
  }
  // the elected client has started the detached owner, releases the lock and connects: the owner handles the
  // elector's attach (enqueue) and drives
  action OwnerStarted(c: int): bool = all {
    ENV, cur.phase.get(c) == STARTING,
    commit(drive(enq({ ...cur, owners: cur.owners + 1, lock: if (cur.lock == c) 0 else cur.lock }, c)),
      variant("OwnerStarted", { c: c })),
  }
  // the electing client holding the lock fails (owner binary missing: typed LaneNotPrepared) or is killed
  // before the owner exists: lock released, no owner, nothing left running
  action OwnerStartFails(c: int): bool = all {
    ENV, cur.phase.get(c) == STARTING,
    commit({ ...ph(cur, c, STARTFAIL), lock: if (cur.lock == c) 0 else cur.lock }, variant("OwnerStartFails", { c: c })),
  }

  // ---------------------------------------------------------------- owner events (one OwnerCore event each)
  // a client connects to the live owner (ENV=false: the client is IDLE, it is just attached)
  action Attached(c: int): bool = all {
    cur.owners > 0,
    cur.phase.get(c) == ARRIVING or (not(ENV) and cur.phase.get(c) == IDLE),
    commit(drive(enq(cur, c)), variant("Attached", { c: c })),
  }
  // D-DETACH, D-TAINT
  action ClientGone(c: int): bool = all {
    Set(QUEUED, RUNNING).contains(cur.phase.get(c)),
    if (cur.phase.get(c) == QUEUED)
      commit(drive({ ...ph(cur, c, GONE), queue: cur.queue.select(x => x != c) }), variant("ClientGone", { c: c }))
    else if (OFF.contains("cancelOnDisconnect"))
      // hazard: the owner treats the disconnect as a cancel and frees the slot; the exec still runs
      commit(drive({ ...ph(cur, c, GONE), inflight: 0 }), variant("ClientGone", { c: c }))
    else
      // detach taints the generation: the guest is discarded when that exec ends
      commit(drive({ ...ph(cur, c, DETACHED), bad: cur.bad.union(Set(cur.execgen)) }), variant("ClientGone", { c: c })),
  }
  // the boot ends ok or failed; a failed boot answers the head client BootFailed, the owner stays with no guest
  action BootFinished(ok: bool): bool = all {
    cur.owners > 0, cur.guest == 1,
    if (ok) commit(drive({ ...cur, guest: 2 }), variant("BootFinished", { ok: ok }))
    else if (cur.queue.length() > 0)
      commit(drive({ ...ph(cur, cur.queue.head(), BOOTFAIL), guest: 0, queue: cur.queue.tail() }),
        variant("BootFinished", { ok: ok }))
    else commit({ ...cur, guest: 0 }, variant("BootFinished", { ok: ok })),
  }
  // the in-flight exec finished (pass or nonzero exit); a failed or detached exec taints the generation
  // (D-TAINT): the guest is discarded; hazard noTaint keeps it Ready
  action ExecFinished(pass: bool): bool = all {
    cur.owners > 0, cur.inflight != 0, cur.guest == 2,
    val ended = execEnds(cur, if (pass) PASS else FAIL)
    val bad1 = if (pass) cur.bad else cur.bad.union(Set(cur.execgen))
    val discard = bad1.contains(cur.execgen) and not(OFF.contains("noTaint"))
    commit(drive({ ...ended, bad: bad1, guest: if (discard) 0 else 2 }), variant("ExecFinished", { pass: pass })),
  }
  // the guest kernel crashed or the VMM exited (environment fault)
  action GuestLost = all {
    cur.owners > 0, cur.guest == 2, cur.faults > 0,
    val after = if (cur.inflight != 0) execEnds(cur, LOST) else cur
    commit(drive({ ...after, guest: 0, execs: 0, bad: cur.bad.union(Set(cur.gen)), faults: cur.faults - 1 }),
      variant("GuestLost", ())),
  }
  // D-BOUND: the exec bound ended a hung exec: the guest is discarded, the client gets BOUND
  action ExecBound = all {
    not(OFF.contains("noBound")),
    cur.owners > 0, cur.inflight != 0, cur.guest == 2,
    val ended = execEnds(cur, BOUND)
    commit(drive({ ...ended, guest: 0, execs: 0, bad: cur.bad.union(Set(cur.execgen)) }), variant("ExecBound", ())),
  }
  // D-IDLE: the owner exits only when no client is queued, running or detached-running; the guest dies with it
  def canIdle: bool = and {
    cur.owners > 0,
    OFF.contains("idleAnyTime") or and { cur.queue.length() == 0, cur.inflight == 0 },
  }
  action IdleElapsed = all {
    canIdle,
    val dropped = CL.exists(c => Set(QUEUED, RUNNING, DETACHED).contains(cur.phase.get(c)))
    commit({ ...ownerDown(cur, true), idleLoss: cur.idleLoss or (cur.owners == 1 and dropped) },
      variant("IdleElapsed", ())),
  }

  // ---------------------------------------------------------------- environment: owner crash (D-LIFE)
  action OwnerCrash = all {
    ENV, cur.owners > 0, cur.faults > 0,
    commit({ ...ownerDown(cur, not(OFF.contains("orphan"))), faults: cur.faults - 1 }, variant("OwnerCrash", ())),
  }

  // fairness set: no ExecFinished (a hung exec may never finish); ExecBound ends it
  action Progress = {
    nondet c = CL.oneOf()
    nondet b = Set(true, false).oneOf()
    any { Arrive(c), ElectBegin(c), OwnerStarted(c), Attached(c), BootFinished(b), ExecBound }
  }
  action step = {
    nondet c = CL.oneOf()
    nondet b = Set(true, false).oneOf()
    any { Arrive(c), ElectBegin(c), OwnerStarted(c), OwnerStartFails(c), Attached(c),
          ClientGone(c), BootFinished(b), ExecFinished(b), GuestLost, ExecBound, IdleElapsed,
          OwnerCrash }
  }

  // ---------------------------------------------------------------- invariants (each named after its decision)
  val I_OWNER_UNIQUE = owners <= 1                                                   // D-OWNER
  val I_SERIAL = execs <= 1                                                          // D-SERIAL
  val I_DETACH_NO_CANCEL = (execs > 0).implies(inflight != 0)                        // D-DETACH
  val I_NO_REUSE = (guest == 2 and inflight == 0).implies(not(bad.contains(gen)))    // D-TAINT
  val I_NO_ORPHAN = (guest != 0).implies(owners > 0)                                 // D-LIFE (A-1)
  val I_IDLE_NO_LOSS = not(idleLoss)                                                 // D-IDLE
  val I_DELIVER_ONLY_ATTACHED = and {                                                // D-DETACH
    queue.indices().forall(i => phase.get(queue[i]) == QUEUED),
    CL.forall(c => Set(RUNNING, DETACHED).contains(phase.get(c)).implies(inflight == c)),
  }
  val Safety = and { I_OWNER_UNIQUE, I_SERIAL, I_DETACH_NO_CANCEL, I_NO_REUSE, I_NO_ORPHAN,
                     I_IDLE_NO_LOSS, I_DELIVER_ONLY_ATTACHED }

  // progress (D-BOUND): every attached client gets a terminal outcome
  // (A-5 bounds the faults)
  temporal FairProgress = weakFair(Progress, (lock, owners, guest, gen, inflight, execgen, execs,
    queue, phase, bad, faults, idleLoss, conf_step))
  temporal EveryClientTerminates = FairProgress.implies(CL.forall(c =>
    always(Set(ARRIVING, STARTING, QUEUED, RUNNING, DETACHED).contains(phase.get(c)).implies(eventually(phase.get(c) >= GONE)))))

  // ---------------------------------------------------------------- named runs (one per critical scenario)
  // guest_conf runs (ENV=false): owner already started; common prefix: three clients attached
  action up3 = init.then(Attached(1)).then(Attached(2)).then(Attached(3))

  run happy_path_three_clients_share_one_guest = up3
    .expect(guest == 1 and gen == 1 and queue == List(1, 2, 3) and Safety)
    .then(BootFinished(true)).expect(inflight == 1 and queue == List(2, 3))
    .then(ExecFinished(true)).then(ExecFinished(true)).then(ExecFinished(true))
    .expect(gen == 1 and owners == 1 and guest == 2 and CL.forall(c => phase.get(c) == PASS) and Safety)

  run failure_then_replacement_guest = up3
    .then(BootFinished(true)).then(ExecFinished(false))
    .expect(phase.get(1) == FAIL and guest == 1 and gen == 2 and bad.contains(1) and inflight == 0 and Safety)
    .then(BootFinished(true))
    .expect(execgen == 2 and inflight == 2 and Safety)
    .then(ExecFinished(true)).then(ExecFinished(true))
    .expect(gen == 2 and phase.get(2) == PASS and phase.get(3) == PASS and Safety)

  run guest_lost_mid_exec_then_replacement = up3
    .then(BootFinished(true)).then(GuestLost)
    .expect(phase.get(1) == LOST and guest == 1 and gen == 2 and inflight == 0 and execs == 0 and bad.contains(1) and Safety)
    .then(BootFinished(true))
    .expect(execgen == 2 and inflight == 2 and Safety)
    .then(ExecFinished(true)).expect(phase.get(2) == PASS and inflight == 3 and Safety)

  run disconnect_while_running_next_waits = init.then(Attached(1)).then(BootFinished(true))
    .then(ClientGone(1))
    .expect(phase.get(1) == DETACHED and inflight == 1 and execs == 1 and bad.contains(1) and Safety)
    .then(Attached(2))
    .expect(queue == List(2) and inflight == 1 and execs == 1 and guest == 2 and Safety)
    .then(ClientGone(2))
    .expect(queue == List() and phase.get(2) == GONE and Safety)
    .then(ExecFinished(true))
    .expect(phase.get(1) == GONE and inflight == 0 and execs == 0 and guest == 0 and Safety)
    .then(Attached(3))
    .expect(guest == 1 and gen == 2 and Safety)
    .then(BootFinished(true)).then(ExecFinished(true))
    .expect(phase.get(3) == PASS and gen == 2 and Safety)

  run disconnect_while_queued_is_removed = up3
    .then(BootFinished(true)).then(ClientGone(2))
    .expect(phase.get(2) == GONE and queue == List(3) and inflight == 1 and Safety)
    .then(ExecFinished(true))
    .expect(inflight == 3 and Safety)
    .then(ExecFinished(true)).expect(phase.get(3) == PASS and phase.get(2) == GONE and Safety)

  run boot_failure_then_retry_boot_on_next_attach = init.then(Attached(1))
    .then(BootFinished(false))
    .expect(phase.get(1) == BOOTFAIL and owners == 1 and guest == 0 and queue == List() and Safety)
    .then(Attached(2)).expect(guest == 1 and gen == 2 and Safety)
    .then(BootFinished(true)).then(ExecFinished(true))
    .expect(phase.get(2) == PASS and Safety)

  run exec_bound_discards_guest_and_replies_bound = init.then(Attached(1)).then(BootFinished(true))
    .then(ExecBound)
    .expect(phase.get(1) == BOUND and guest == 0 and execs == 0 and inflight == 0 and bad.contains(1) and Safety)
    .then(Attached(2)).expect(guest == 1 and gen == 2 and Safety)
    .then(BootFinished(true)).then(ExecFinished(true))
    .expect(phase.get(2) == PASS and gen == 2 and Safety)

  run idle_exit_only_when_empty = init.then(Attached(1)).then(Attached(2))
    .expect(not(canIdle))
    .then(BootFinished(true)).expect(inflight == 1 and queue == List(2) and not(canIdle))
    .then(ExecFinished(true)).expect(inflight == 2 and queue == List() and not(canIdle))
    .then(ExecFinished(true)).expect(inflight == 0 and queue == List() and canIdle)
    .then(IdleElapsed)
    .expect(owners == 0 and guest == 0 and not(idleLoss) and Safety)

  // guest_env runs (ENV=true, REC=true): the environment actions
  run concurrent_first_arrival_one_owner = init.then(Arrive(1)).then(Arrive(2)).then(ElectBegin(1))
    .expect(lock == 1 and not(canElect(2)) and Safety)
    .then(OwnerStarted(1)).expect(owners == 1 and lock == 0 and queue == List(1) and guest == 1 and not(canElect(2)) and Safety)
    .then(Attached(2)).expect(owners == 1 and queue == List(1, 2) and Safety)

  run owner_crash_then_new_owner_elected = init.then(Arrive(1)).then(Arrive(2)).then(Arrive(3))
    .then(ElectBegin(1)).then(OwnerStarted(1)).then(Attached(2))
    .then(BootFinished(true)).then(OwnerCrash)
    .expect(owners == 0 and guest == 0 and phase.get(1) == OWNERLOST and phase.get(2) == OWNERLOST
            and queue == List() and Safety)
    .then(ElectBegin(3)).then(OwnerStarted(3))
    .expect(gen == 2 and guest == 1 and owners == 1 and Safety)
    .then(BootFinished(true)).then(ExecFinished(true))
    .expect(phase.get(3) == PASS and Safety)

  run owner_start_fails_then_next_elects = init.then(Arrive(1)).then(Arrive(2)).then(ElectBegin(1))
    .then(OwnerStartFails(1))
    .expect(lock == 0 and owners == 0 and phase.get(1) == STARTFAIL and canElect(2) and Safety)
    .then(ElectBegin(2)).then(OwnerStarted(2)).then(BootFinished(true)).then(ExecFinished(true))
    .expect(owners == 1 and phase.get(2) == PASS and Safety)
}

module guest_ok { import shared_guest(OFF = Set(), REC = false, ENV = true).* from "./shared_guest" }
module guest_env { import shared_guest(OFF = Set(), REC = true, ENV = true).* from "./shared_guest" }
module guest_conf { import shared_guest(OFF = Set(), REC = true, ENV = false).* from "./shared_guest" }
