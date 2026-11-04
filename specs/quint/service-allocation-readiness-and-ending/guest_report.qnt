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
 * closed), junk (garbage lines delivered, bounded).
 *
 * Conformance: the full module is not a conformance target (the guest half has no writer seam; coverage split in the README). The
 * host half is the module guest_report_host below. GR-4 (workload created directly in the cgroup, CLONE_INTO_CGROUP): the V42 row
 * of the feature delta is the single home of that claim.
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
  var junk: int
  var conf_step: ConfStep

  action init = all {
    wl' = "running", ring' = List(), wire' = List(), rd' = 0, rset' = Set(), wait' = "waiting", exitrep' = "unset",
    produced' = List(), eof' = false, junk' = 0,
    conf_step' = variant("init", ()),
  }

  def exitOnWire: bool = wire.indices().exists(i => wire.nth(i) == "exit")
  def nonDropped: int = produced.select(e => not(e.dropped)).length()

  // the workload listens (kernel hook): the event is enqueued, or dropped and counted when the ring is full
  action WorkloadListen(p: str): bool = all {
    wl == "running", produced.length() < PMAX, ALPHA.contains(p),
    ring' = if (ring.length() < RCAP) ring.append(p) else ring,
    produced' = produced.append({ p: p, dropped: ring.length() >= RCAP }),
    wl' = wl, wire' = wire, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, eof' = eof,
    junk' = junk,
    conf_step' = if (REC) variant("WorkloadListen", { p: p }) else conf_step,
  }

  action WorkloadExit = all {
    wl == "running",
    wl' = "exited",
    ring' = ring, wire' = wire, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, produced' = produced, eof' = eof,
    junk' = junk,
    conf_step' = if (REC) variant("WorkloadExit", ()) else conf_step,
  }

  // the one writer moves the head of the ring to the beacon
  action WriterForward = all {
    not(eof), ring != List(),
    wire' = wire.append(ring.head()), ring' = ring.tail(),
    wl' = wl, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, produced' = produced, eof' = eof,
    junk' = junk,
    conf_step' = if (REC) variant("WriterForward", ()) else conf_step,
  }

  // after the workload exits, the writer forwards everything left in the ring
  action WriterDrain = all {
    wl == "exited", not(eof), ring != List(),
    wire' = wire.concat(ring), ring' = List(),
    wl' = wl, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, produced' = produced, eof' = eof,
    junk' = junk,
    conf_step' = if (REC) variant("WriterDrain", ()) else conf_step,
  }

  // EXIT is written once, only after the workload exited and the ring is empty (the drain rule)
  action WriterExit = all {
    wl == "exited", not(eof), BUG_NODRAIN or ring == List(), not(exitOnWire),
    wire' = wire.append("exit"),
    wl' = wl, ring' = ring, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, produced' = produced, eof' = eof,
    junk' = junk,
    conf_step' = if (REC) variant("WriterExit", ()) else conf_step,
  }

  // guest or VMM gone: the stream reaches EOF after the entries already written
  action BeaconClose = all {
    not(eof),
    eof' = true,
    wl' = wl, ring' = ring, wire' = wire, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, produced' = produced,
    junk' = junk,
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
        wl' = wl, ring' = ring, wire' = wire, produced' = produced, eof' = eof, junk' = junk,
        conf_step' = if (REC) variant("HostRead", ()) else conf_step,
      },
      all {
        eof, rd == n, wait == "waiting",
        wait' = "ended",
        wl' = wl, ring' = ring, wire' = wire, rd' = rd, rset' = rset, exitrep' = exitrep, produced' = produced, eof' = eof,
        junk' = junk,
        conf_step' = if (REC) variant("HostRead", ()) else conf_step,
      },
    }
  }

  // an unparseable line reaches the reader (the scripted line source): no effect on set, wait or exitrep
  action HostGarbage = all {
    junk < JMAX,
    junk' = junk + 1,
    wl' = wl, ring' = ring, wire' = wire, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, produced' = produced,
    eof' = eof,
    conf_step' = if (REC) variant("HostGarbage", ()) else conf_step,
  }

  action Idle = all {
    wl' = wl, ring' = ring, wire' = wire, rd' = rd, rset' = rset, wait' = wait, exitrep' = exitrep, produced' = produced,
    eof' = eof, junk' = junk,
    conf_step' = if (REC) variant("Idle", ()) else conf_step,
  }

  action step = {
    nondet p = ALPHA.oneOf()
    any { WorkloadListen(p), WorkloadExit, WriterForward, WriterDrain, WriterExit, BeaconClose, HostRead, HostGarbage, Idle }
  }

  // ------------------------------------------------------------------ invariants
  // D13 item 1 ListenBeforeExit: in the wire, every listen precedes exit
  val ListenBeforeExit: bool = wire.indices().forall(i => (wire.nth(i) == "exit") implies (i == wire.length() - 1))
  // D13 item 1 DeliveredBeforeExit: when exit is on the wire, every produced, non-dropped event is on the wire before it
  val DeliveredBeforeExit: bool = exitOnWire implies (wire.length() - 1 == nonDropped)
  val Safety: bool = and { ListenBeforeExit, DeliveredBeforeExit }

  // D13 item 3 ExitEventuallyReported: a workload that exits with the beacon open ends the guest-report wait
  temporal ExitEventuallyReported =
    and {
      weakFair(WriterForward, (wl, ring, wire, rd, rset, wait, exitrep, produced, eof, junk, conf_step)),
      weakFair(WriterDrain, (wl, ring, wire, rd, rset, wait, exitrep, produced, eof, junk, conf_step)),
      weakFair(WriterExit, (wl, ring, wire, rd, rset, wait, exitrep, produced, eof, junk, conf_step)),
      weakFair(HostRead, (wl, ring, wire, rd, rset, wait, exitrep, produced, eof, junk, conf_step)),
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

  // a declared event dropped on a full ring: the set stays partial, the wait still ends on exit
  run ring_drop_leaves_set_partial = init
    .then(WorkloadListen("tcp80")).then(WriterForward)
    .then(WorkloadListen("other")).then(WorkloadListen("udp53"))
    .expect(produced.length() == 3 and nonDropped == 2 and ring == List("other"))
    .then(WorkloadExit).then(WriterDrain).then(WriterExit)
    .then(HostRead).then(HostRead).then(HostRead)
    .expect(rset == Set("tcp80") and wait == "ended" and exitrep == "rset" and Safety)
}

module gr_ok { import guest_report(OFF = Set(), REC = false).* from "./guest_report" }
module gr_conf { import guest_report(OFF = Set(), REC = true).* from "./guest_report" }

/**
 * guest_report_host: the conformance projection of the beacon report protocol (D13 items 3 and 10; feature delta § Formal
 * protocol model, D13 to D16). The host reader's half only: the input is the scripted line stream, the guest's drain rule is
 * not a host concern, so appends are unordered with respect to each other and `listen` can follow `exit` (the reader's own rule
 * drops it). It is the only conformance target of the report protocol; the full guest_report module is not one.
 *
 * State: rset (GuestListenerReports::reported(alloc)), exitrep (the ExitEvent on the receiver from take_exit_receiver): none |
 * guest-reported | no-report, moved off none only by VmmExit; the ExitEvent kind and exit_code give the value.
 * Model-only, never projected: gstat (the reader holds a guest status, set when it reads `exit`), wait (the guest-report wait), lines (the scripted input not yet consumed: listen(p), exit, junk),
 * eof. The declared set DECL is a constant; a valid line's pair ranges over DECL and one undeclared pair.
 *
 * Actions to driving port (the BeaconIo seam, SimBeaconIo): HostRead = release_read; AppendListen(p), AppendExit, HostGarbage =
 * guest_write of a valid LISTEN line, an EXIT line, an unparseable line; BeaconClose = guest_close;
 * VmmExit = SimVmm::exit_process plus the exit-gate release.
 * No hazard and no invariant of its own: the HostRead guards keep rset within DECL and growing only; the decision's hazard is
 * gr-hazard-no-drain-before-exit in the full module.
 */
module guest_report_host_conf {
  type HostConfStep =
    | init
    | HostRead
    | AppendListen({ p: str })
    | AppendExit
    | HostGarbage
    | BeaconClose
    | VmmExit
    | Idle
}

module guest_report_host {
  import guest_report_host_conf.HostConfStep from "./guest_report"
  const REC: bool

  val DECL: Set[str] = Set("tcp80", "udp53")
  val ALPHA: Set[str] = Set("tcp80", "udp53", "other")
  val LMAX: int = 4     // bound on the scripted lines not yet consumed

  var rset: Set[str]
  var exitrep: str
  var gstat: bool       // model-only: the reader holds a guest status (it has read `exit`)
  var wait: str         // model-only
  var lines: List[{ k: str, p: str }]  // model-only: k is "listen" (with pair p), "exit" or "junk"
  var eof: bool         // model-only
  var conf_step: HostConfStep

  action init = all {
    rset' = Set(), exitrep' = "none", gstat' = false, wait' = "waiting", lines' = List(), eof' = false,
    conf_step' = variant("init", ()),
  }

  // the real reader consumes the next scripted line and routes it, or sees EOF on an empty closed stream
  action HostRead = any {
    all {
      lines != List(),
      lines' = lines.tail(),
      rset' = if (lines.head().k == "listen" and wait == "waiting" and DECL.contains(lines.head().p)) rset.union(Set(lines.head().p)) else rset,
      gstat' = if (lines.head().k == "exit") true else gstat,
      exitrep' = exitrep,
      wait' = if (lines.head().k == "exit") "ended" else wait,
      eof' = eof,
      conf_step' = if (REC) variant("HostRead", ()) else conf_step,
    },
    all {
      lines == List(), eof, wait == "waiting",
      wait' = "ended",
      rset' = rset, exitrep' = exitrep, gstat' = gstat, lines' = lines, eof' = eof,
      conf_step' = if (REC) variant("HostRead", ()) else conf_step,
    },
  }

  // a valid LISTEN line is written to the beacon (the guest side; nothing is appended once the beacon is closed)
  action AppendListen(p: str): bool = all {
    not(eof), lines.length() < LMAX, ALPHA.contains(p),
    lines' = lines.append({ k: "listen", p: p }),
    rset' = rset, exitrep' = exitrep, gstat' = gstat, wait' = wait, eof' = eof,
    conf_step' = if (REC) variant("AppendListen", { p: p }) else conf_step,
  }

  action AppendExit = all {
    not(eof), lines.length() < LMAX,
    lines' = lines.append({ k: "exit", p: "" }),
    rset' = rset, exitrep' = exitrep, gstat' = gstat, wait' = wait, eof' = eof,
    conf_step' = if (REC) variant("AppendExit", ()) else conf_step,
  }

  // an unparseable line: no effect on rset, wait or exitrep
  action HostGarbage = all {
    not(eof), lines.length() < LMAX,
    lines' = lines.append({ k: "junk", p: "" }),
    rset' = rset, exitrep' = exitrep, gstat' = gstat, wait' = wait, eof' = eof,
    conf_step' = if (REC) variant("HostGarbage", ()) else conf_step,
  }

  // EOF: the stream ends after the lines already written
  action BeaconClose = all {
    not(eof),
    eof' = true,
    rset' = rset, exitrep' = exitrep, gstat' = gstat, wait' = wait, lines' = lines,
    conf_step' = if (REC) variant("BeaconClose", ()) else conf_step,
  }

  // the VMM process exits (the exit gate releases): enabled once the guest-report wait has ended; the only action that moves
  // exitrep off none; guest-reported when the reader holds a guest status, no-report otherwise
  action VmmExit = all {
    wait == "ended", exitrep == "none",
    exitrep' = if (gstat) "guest-reported" else "no-report",
    rset' = rset, gstat' = gstat, wait' = wait, lines' = lines, eof' = eof,
    conf_step' = if (REC) variant("VmmExit", ()) else conf_step,
  }

  action Idle = all {
    rset' = rset, exitrep' = exitrep, gstat' = gstat, wait' = wait, lines' = lines, eof' = eof,
    conf_step' = if (REC) variant("Idle", ()) else conf_step,
  }

  action step = {
    nondet p = ALPHA.oneOf()
    any { HostRead, AppendListen(p), AppendExit, HostGarbage, BeaconClose, VmmExit, Idle }
  }

  // all declared pairs appended and read: rset reaches the declared set while the wait stays open
  run host_all_declared_reported = init
    .then(AppendListen("tcp80")).then(AppendListen("udp53"))
    .then(HostRead).then(HostRead)
    .expect(rset == DECL and wait == "waiting" and exitrep == "none")

  // an unparseable line mid-stream is dropped; the following listen and exit still take effect
  run host_garbage_line_is_dropped = init
    .then(AppendListen("tcp80")).then(HostGarbage).then(AppendListen("udp53")).then(AppendExit)
    .then(HostRead).then(HostRead)
    .expect(rset == Set("tcp80") and wait == "waiting" and exitrep == "none")
    .then(HostRead).then(HostRead)
    .expect(rset == DECL and wait == "ended" and exitrep == "none")
    .then(VmmExit)
    .expect(rset == DECL and wait == "ended" and exitrep == "guest-reported")

  // EOF with rset partial: the wait ends, rset is as it was, no exit report
  run host_beacon_closes_with_partial_set = init
    .then(AppendListen("tcp80")).then(HostRead).then(BeaconClose)
    .expect(wait == "waiting" and rset == Set("tcp80"))
    .then(HostRead)
    .expect(wait == "ended" and rset == Set("tcp80") and exitrep == "none")
    .then(VmmExit)
    .expect(rset == Set("tcp80") and wait == "ended" and exitrep == "no-report")
    .then(Idle)
    .expect(rset == Set("tcp80") and wait == "ended" and exitrep == "no-report")

  // an exit line then a listen line: the wait ends and rset is unchanged by the later line (the reader's rule); exitrep stays
  // none until the VMM exits, then guest-reported
  run host_listen_after_exit_is_ignored = init
    .then(AppendListen("tcp80")).then(AppendExit).then(AppendListen("udp53"))
    .then(HostRead).then(HostRead)
    .expect(rset == Set("tcp80") and exitrep == "none" and wait == "ended")
    .then(HostRead)
    .expect(rset == Set("tcp80") and exitrep == "none" and wait == "ended")
    .then(VmmExit)
    .expect(rset == Set("tcp80") and exitrep == "guest-reported" and wait == "ended")
}

module gr_host_conf { import guest_report_host(REC = true).* from "./guest_report" }
