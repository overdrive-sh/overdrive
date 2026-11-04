// -*- mode: Bluespec; -*-
/**
 * The beacon report channel (service-allocation-readiness-and-ending D13 items 1, 3 and 10; ADR-0268, ADR-0272). An ordered,
 * single-writer, crash-sensitive protocol between the guest's workload listen events, init's one beacon writer, and the host's
 * beacon reader, reported-set board and exit report. The guest-signal spike measured the hazard: 185 of 300 boots delivered a
 * LISTEN after EXIT when init did not drain the kernel's listen events before writing EXIT.
 *
 * Variables (the design's names):
 *   wl        the workload: running | exited (the model starts with the workload running, hooks attached: see README finding)
 *   ring      the kernel's listen events not yet taken by the writer (a sequence over {tcp80, udp53, other}); RCAP = 1
 *   wire      what init has written to the beacon, in order: a listen event is its pair name, EXIT is "exit"
 *   rd        how many wire entries the host reader has consumed
 *   rset      the board's reported set (GuestListenerReports::reported); the design's `set` (a Quint built-in name)
 *   wait      the host's guest-report wait: waiting | ended
 *   exitrep   the exit report: unset | set (the ExitEvent on the exit receiver)
 * Model-only ghosts: produced (events the hook enqueued, each with a dropped marker for a full-ring event), eof (the beacon
 * closed), endset (the set when the wait ended: NoLateAccept), junk (garbage lines delivered, bounded), wapp (wire appends made
 * by a writer action: OneWriter).
 *
 * Declared pairs DECL = {tcp80, udp53}; `other` is an undeclared pair (a duplicate or undeclared pair is a no-op on the set).
 *
 * OFF names the rules switched off (hazards):
 *   noDrainBeforeExit (D13 item 1)   WriterExit is enabled without the ring being empty (the drain removed)
 */
module guest_report_conf {
  type ConfStep =
    | init
    | WorkloadListen({ p: str })
    | WorkloadExit
    | WriterForward
    | WriterDrain
    | WriterExit
    | BeaconClose
    | HostRead
    | HostGarbage
    | Idle
}

module guest_report {
  import guest_report_conf.ConfStep from "./guest_report"
  const OFF: Set[str]
  const REC: bool

  val DECL: Set[str] = Set("tcp80", "udp53")
  val ALPHA: Set[str] = Set("tcp80", "udp53", "other")
  val RCAP: int = 1     // ring capacity: a listen event with the ring full is dropped
  val PMAX: int = 3     // bound on events the workload produces
  val JMAX: int = 2     // bound on garbage lines
  val BUG_NODRAIN: bool = OFF.contains("noDrainBeforeExit")

  type Ev = { p: str, dropped: bool }

  var wl: str
  var ring: List[str]
  var wire: List[str]
  var rd: int
  var rset: Set[str]
  var wait: str
  var exitrep: str
  var produced: List[Ev]
  var eof: bool
  var endset: Set[str]
  var junk: int
  var wapp: int
  var conf_step: ConfStep

  action init = all {
    wl' = "running", ring' = List(), wire' = List(), rd' = 0, rset' = Set(), wait' = "waiting", exitrep' = "unset",
    produced' = List(), eof' = false, endset' = Set(), junk' = 0, wapp' = 0,
    conf_step' = variant("init", ()),
  }

  def exitOnWire: bool = wire.indices().exists(i => wire.nth(i) == "exit")
  def exitRead: bool = wire.indices().exists(i => i < rd and wire.nth(i) == "exit")
  def nonDropped: int = produced.select(e => not(e.dropped)).length()

  // the workload listens (kernel hook): the event is enqueued, or dropped and counted when the ring is full
  action WorkloadListen(p: str): bool = all {
    wl == "running", produced.length() < PMAX, ALPHA.contains(p),
    ring' = if (ring.length() < RCAP) ring.append(p) else ring,
    produced' = produced.append({ p: p, dropped: ring.length() >= RCAP }),
    wl' = wl, wire' = wire, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, eof' = eof, endset' = endset,
    junk' = junk, wapp' = wapp,
    conf_step' = if (REC) variant("WorkloadListen", { p: p }) else conf_step,
  }

  action WorkloadExit = all {
    wl == "running",
    wl' = "exited",
    ring' = ring, wire' = wire, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, produced' = produced, eof' = eof,
    endset' = endset, junk' = junk, wapp' = wapp,
    conf_step' = if (REC) variant("WorkloadExit", ()) else conf_step,
  }

  // the one writer moves the head of the ring to the beacon
  action WriterForward = all {
    not(eof), ring != List(),
    wire' = wire.append(ring.head()), ring' = ring.tail(), wapp' = wapp + 1,
    wl' = wl, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, produced' = produced, eof' = eof, endset' = endset,
    junk' = junk,
    conf_step' = if (REC) variant("WriterForward", ()) else conf_step,
  }

  // after the workload exits, the writer forwards everything left in the ring
  action WriterDrain = all {
    wl == "exited", not(eof), ring != List(),
    wire' = wire.concat(ring), ring' = List(), wapp' = wapp + ring.length(),
    wl' = wl, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, produced' = produced, eof' = eof, endset' = endset,
    junk' = junk,
    conf_step' = if (REC) variant("WriterDrain", ()) else conf_step,
  }

  // EXIT is written once, only after the workload exited and the ring is empty (the drain rule)
  action WriterExit = all {
    wl == "exited", not(eof), BUG_NODRAIN or ring == List(), not(exitOnWire),
    wire' = wire.append("exit"), wapp' = wapp + 1,
    wl' = wl, ring' = ring, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, produced' = produced, eof' = eof,
    endset' = endset, junk' = junk,
    conf_step' = if (REC) variant("WriterExit", ()) else conf_step,
  }

  // guest or VMM gone: the stream reaches EOF after the entries already written
  action BeaconClose = all {
    not(eof),
    eof' = true,
    wl' = wl, ring' = ring, wire' = wire, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, produced' = produced,
    endset' = endset, junk' = junk, wapp' = wapp,
    conf_step' = if (REC) variant("BeaconClose", ()) else conf_step,
  }

  // the host reader consumes one wire entry, or reaches EOF
  action HostRead = {
    val n = wire.length()
    any {
      all {
        rd < n,
        rd' = rd + 1,
        rset' = if (wire.nth(rd) != "exit" and wait == "waiting" and DECL.contains(wire.nth(rd))) rset.union(Set(wire.nth(rd))) else rset,
        exitrep' = if (wire.nth(rd) == "exit") "rset" else exitrep,
        wait' = if (wire.nth(rd) == "exit") "ended" else wait,
        endset' = if (wire.nth(rd) == "exit" and wait == "waiting") rset else endset,
        wl' = wl, ring' = ring, wire' = wire, produced' = produced, eof' = eof, junk' = junk, wapp' = wapp,
        conf_step' = if (REC) variant("HostRead", ()) else conf_step,
      },
      all {
        eof, rd == n, wait == "waiting",
        wait' = "ended", endset' = rset,
        wl' = wl, ring' = ring, wire' = wire, rd' = rd, rset' = rset, exitrep' = exitrep, produced' = produced, eof' = eof,
        junk' = junk, wapp' = wapp,
        conf_step' = if (REC) variant("HostRead", ()) else conf_step,
      },
    }
  }

  // an unparseable line reaches the reader (the scripted line source): no effect on set, wait or exitrep
  action HostGarbage = all {
    junk < JMAX,
    junk' = junk + 1,
    wl' = wl, ring' = ring, wire' = wire, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, produced' = produced,
    eof' = eof, endset' = endset, wapp' = wapp,
    conf_step' = if (REC) variant("HostGarbage", ()) else conf_step,
  }

  action Idle = all {
    wl' = wl, ring' = ring, wire' = wire, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, produced' = produced,
    eof' = eof, endset' = endset, junk' = junk, wapp' = wapp,
    conf_step' = if (REC) variant("Idle", ()) else conf_step,
  }

  action step = {
    nondet p = ALPHA.oneOf()
    any { WorkloadListen(p), WorkloadExit, WriterForward, WriterDrain, WriterExit, BeaconClose, HostRead, HostGarbage, Idle }
  }

  // ------------------------------------------------------------------ invariants
  // D13 item 1 OneWriter: every wire entry was appended by the writer's actions (kept as a check on the ghost)
  val OneWriter: bool = wapp == wire.length()
  // D13 item 1 ListenBeforeExit: in the wire, every listen precedes exit
  val ListenBeforeExit: bool = wire.indices().forall(i => (wire.nth(i) == "exit") implies (i == wire.length() - 1))
  // D13 item 1 DeliveredBeforeExit: when exit is on the wire, every produced, non-dropped event is on the wire before it
  val DeliveredBeforeExit: bool = exitOnWire implies (wire.length() - 1 == nonDropped)
  // D13 item 3 NoLateAccept: the reported set does not change after the wait ended
  val NoLateAccept: bool = (wait == "ended") implies (rset == endset)
  // D13 item 3 ExitNotMasked: the exit report is set iff an exit entry was read
  val ExitNotMasked: bool = (exitrep == "rset") == exitRead
  // D13 item 10 SetWithinProduced: every pair in the set was produced by the workload (and reached the wire)
  val SetWithinProduced: bool = rset.forall(p => produced.indices().exists(i => produced.nth(i).p == p and not(produced.nth(i).dropped)))
  val Safety: bool = and { OneWriter, ListenBeforeExit, DeliveredBeforeExit, NoLateAccept, ExitNotMasked, SetWithinProduced }

  // D13 item 3 ExitEventuallyReported: a workload that exits with the beacon open ends the guest-report wait
  temporal ExitEventuallyReported =
    and {
      weakFair(WriterForward, (wl, ring, wire, rd, rset, wait, exitrep, produced, eof, endset, junk, wapp, conf_step)),
      weakFair(WriterDrain, (wl, ring, wire, rd, rset, wait, exitrep, produced, eof, endset, junk, wapp, conf_step)),
      weakFair(WriterExit, (wl, ring, wire, rd, rset, wait, exitrep, produced, eof, endset, junk, wapp, conf_step)),
      weakFair(HostRead, (wl, ring, wire, rd, rset, wait, exitrep, produced, eof, endset, junk, wapp, conf_step)),
    }.implies(always((wl == "exited" and not(eof)).implies(eventually(wait == "ended"))))

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // happy path and the 185-of-300 race: a LISTEN produced just before exit is on the wire before exit
  run listen_then_exit_drains_first = init
    .then(WorkloadListen("tcp80")).then(WorkloadExit)
    .expect(ring == List("tcp80") and wire == List())
    .then(WriterDrain).then(WriterExit)
    .expect(wire == List("tcp80", "exit") and Safety)
    .then(HostRead).then(HostRead)
    .expect(rset == Set("tcp80") and exitrep == "rset" and wait == "ended" and Safety)

  // the set reaches the declared pair (the input to Pass) while the wait is still open; exit then ends the wait
  run all_declared_reported = init
    .then(WorkloadListen("tcp80")).then(WriterForward).then(WorkloadListen("udp53")).then(WriterForward)
    .then(HostRead).then(HostRead)
    .expect(rset == DECL and wait == "waiting" and exitrep == "unset" and Safety)
    .then(WorkloadExit).then(WriterExit).then(HostRead)
    .expect(rset == DECL and wait == "ended" and exitrep == "rset" and Safety)

  // an unparseable line mid-stream is dropped; later listen and exit lines still take effect
  run garbage_line_is_dropped = init
    .then(WorkloadListen("tcp80")).then(WriterForward).then(HostRead)
    .then(HostGarbage)
    .expect(rset == Set("tcp80") and wait == "waiting" and exitrep == "unset" and Safety)
    .then(WorkloadListen("udp53")).then(WriterForward).then(HostGarbage).then(HostRead)
    .expect(rset == DECL and wait == "waiting" and Safety)
    .then(WorkloadExit).then(WriterExit).then(HostRead)
    .expect(wait == "ended" and exitrep == "rset" and rset == DECL and Safety)

  // a declared event dropped on a full ring: the set stays partial, the wait still ends on exit
  run ring_drop_leaves_set_partial = init
    .then(WorkloadListen("tcp80")).then(WriterForward)
    .then(WorkloadListen("other")).then(WorkloadListen("udp53"))
    .expect(produced.length() == 3 and nonDropped == 2 and ring == List("other"))
    .then(WorkloadExit).then(WriterDrain).then(WriterExit)
    .then(HostRead).then(HostRead).then(HostRead)
    .expect(rset == Set("tcp80") and wait == "ended" and exitrep == "rset" and Safety)

  // EOF with the set partial: the wait ends, the set is frozen, no exit report
  run beacon_closes_with_partial_set = init
    .then(WorkloadListen("tcp80")).then(WriterForward).then(HostRead)
    .then(WorkloadListen("udp53")).then(BeaconClose)
    .expect(wait == "waiting" and rset == Set("tcp80"))
    .then(HostRead)
    .expect(wait == "ended" and rset == Set("tcp80") and exitrep == "unset" and Safety)
    .then(Idle)
    .expect(rset == Set("tcp80") and wait == "ended" and Safety)
}

module gr_ok { import guest_report(OFF = Set(), REC = false).* from "./guest_report" }
module gr_conf { import guest_report(OFF = Set(), REC = true).* from "./guest_report" }
