// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — flow end: how one paired TCP flow ends across the VM
 * boundary (R2-END, ADR-0180) and how a half-close drain completes (R2-DRAIN, ADR-0183).
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 *
 * ONE paired TCP flow between the guest owner (g) and the host owner (h), two directions. Direction
 * d_x is the data that flows from the far end of x's outer leg to the far end of the other side's
 * outer leg. For d_x the source owner x reads TCP and the kernel redirects into x's flow vsock
 * (target cell V_x, kernel quantity `sent` = vsock tx_cnt); the other owner y receives from the
 * vsock and the kernel redirects into y's outer TCP leg (target cell O_y, `sent` = TCP write_seq).
 * The four cells Vg, Vh, Og, Oh are the (flow, direction, target) cells of the design.
 *
 * Rules modelled:
 *   R2-END   The end of a direction is classified by the kernel record of the peer's vsock SHUTDOWN
 *            flags, written by a raw-tracepoint program on `vsock:virtio_transport_recv_pkt` before
 *            the EOF wake: 0x2 = clean half-close of the peer's direction; 0x3 while any direction
 *            is open, or RST = abort. A vsock end with no record = unannounced end, an abort (A4).
 *            An unclassified TCP-source EOF is an abort when it is a destination reset (A6).
 *            An abort closes the vsock (the peer sees 0x3) and resets the far end.
 *   R2-DRAIN Per (flow, direction) one cell (sent, goal, armed, fired). The owner stores goal, then
 *            armed, then reads sent; the kernel (two independent fexit programs, each reading the
 *            CUMULATIVE kernel quantity) stores sent then reads armed/goal; whoever wins fired
 *            reports (the kernel by a ringbuf record carrying the epoch). The goal is
 *            bytes_received (TCP source) or the owner's own forwarded count (vsock source). A call
 *            both programs miss is covered by the level backstop (fentry(sock_poll) re-read), run
 *            by the owner on a rise of recursion_misses and once at arm time on a short read. No
 *            deadline: a slow reader is backpressure.
 *   H-3 (r1) The kernel raises no event for a rise of recursion_misses, a ring drop or a cell's lost
 *            count. The owner (guest and host alike) READS these counters at every flow-event record
 *            dispatch AND on a fixed period equal to the ADR-0124 1 s audit tick (OwnAuditTick, weakly
 *            fair). This is a counter read, not a drain deadline: a direction completes only on its record
 *            or its backstop. A both-skip or a lost record is detected within one audit period.
 *   A2  records of another epoch are discarded.        A3  a refused redirect writes one record.
 *   A7  a record the full ring dropped raises the lost counter; the owner aborts the flow on the rise
 *       it reads.
 *   H-2 (r1) A dropped record increments exactly one per-cell counter by class: `ring_lost` counts a lost
 *       flow-end / drain / redirect-refused record (this module's `lost`: the owner aborts every flow whose
 *       count rose); `empty_lost` counts a lost EmptyDatagram notice (module udp_empty's `lost`: the owner
 *       compensates the live flow of the current epoch and never aborts). The two classes are modelled in their
 *       own modules; no record increments both, so a lost empty notice never aborts an association.
 *   DG-5-C  the clean close happens in the step that completes the second direction.
 *   DG-5-F  control-session loss aborts the flow at both owners.
 *   INB-1   for a host-opened flow (opener h) the host side is the dialer's GuestStream: no cells,
 *           no drain race; the dialer shuts down and reads directly.
 *
 * Bytes are counted: each direction carries TOT bytes then a FIN. Time is abstract; there is no
 * deadline anywhere. Faults are environment actions with a budget of one use each.
 *
 * ASSUMPTIONS (kernel behaviour, discharged in README.md):
 *   A1  each fexit program may miss a call; the other program's cumulative read covers it ... V-31 item 4
 *   A5  (raw tracepoint) the record precedes the EOF wake and is not skipped for another program's
 *       activity; it is skipped only on the same program's recursion on one CPU
 *       (kernel/trace/bpf_trace.c:2080-2083 @ v7.0), counted in the classifier's recursion_misses; such a
 *       skip is an A4 unannounced-end abort (EnvClassifierSkip, a fault) ....... V-31, T5 (raw form), V-31-MV
 *   A6  the TCP-source end check classifies a destination reset ................... V-31 item 2b
 *   A8  at a classified 0x2 the survivor has forwarded every byte the peer sent .. T4 (findings-drain-t1-t6)
 *   A9  the fentry(sock_poll) backstop runs in the owner's own syscall and is not skipped ... V-31 item 4
 *
 * OFF names the rules switched off (teeth); the design instances switch none off.
 */
module flow_end {
  const OFF: Set[str]
  const CFG: str    // "all" | "clean" | "dv" | "do" | "abort" | "ring" | "two" — the byte count and the enabled environment faults

  // Instances. "all": one byte, every fault (the named runs). "skip": the classifier skip only, both directions active
  // (M-2: AbortOnlyOnFault and EveryEndResolves with the A5 fault). "clean": no fault (exhaustive checks and
  // progress of the happy path). "dv" / "do": the kernel-side faults (A1 misses, A7 ring full, A2 stale records, A3 target
  // gone) on the drain toward the vsock (guest side, direction d_g) / toward the outer leg (host side, same direction).
  // "abort": the abort-side faults (far-end reset, owner death, PID-1 crash).
  // "ring": a full ring, a session drop, an unannounced end. In every instance but "clean" and "abort" only the guest's
  // far end writes and ends (the host's far end is passive); "clean" exercises both directions exhaustively. "two": two bytes, fexit misses only (the cumulative-quantity and arm-order rules).
  val TOT: int = if (CFG == "two") 2 else 1
  // the far ends that write and end their direction (a far end's reset is a fault, not an activity)
  val ACTIVE: Set[str] = if (CFG == "clean" or CFG == "abort" or CFG == "all" or CFG == "skip") Set("g", "h") else Set("g")
  // an enabled fault is a pair (kind, target): a side for the per-side faults, a cell for the per-cell faults
  val FAULTS: Set[(str, str)] =
    if (CFG == "clean") Set()
    else if (CFG == "skip") Set(("cskip", "g"), ("cskip", "h"))
    else if (CFG == "dv") Set(("m1", "g"), ("m2", "g"), ("full", "g"), ("stale", "Vg"), ("gone", "Vg"))
    else if (CFG == "do") Set(("m1", "h"), ("m2", "h"), ("full", "h"), ("stale", "Oh"), ("gone", "Oh"))
    else if (CFG == "abort") Set(("rst", "g"), ("rst", "h"), ("hostdie", "h"), ("pid1", "g"))
    else if (CFG == "ring") Set(("full", "g"), ("full", "h"), ("drop", "g"), ("unann", "h"), ("stale2", "g"), ("stale2", "h"))
    else if (CFG == "two") Set(("m1", "g"), ("m2", "g"), ("m1", "h"), ("m2", "h"))
    else Set(("m1", "g"), ("m2", "g"), ("m1", "h"), ("m2", "h"), ("full", "g"), ("full", "h"), ("stale", "Vg"), ("stale", "Oh"),
             ("stale2", "g"), ("stale2", "h"), ("gone", "Vg"), ("gone", "Oh"), ("rst", "g"), ("rst", "h"), ("hostdie", "h"),
             ("pid1", "g"), ("drop", "g"), ("unann", "h"), ("cskip", "g"), ("cskip", "h"))

  val SIDES: Set[str] = Set("g", "h")
  val CELLS: Set[str] = Set("Vg", "Vh", "Og", "Oh")

  val BUG_NO_TCP_END_CHECK: bool = OFF.contains("noTcpEndCheck")          // A6 off: every TCP-source EOF is a clean half-close
  val BUG_EOF_IS_HALF: bool = OFF.contains("eofIsHalfClose")              // R2-END off: a vsock end is a half-close whatever its class
  val BUG_ANY_SHUTDOWN_ABORTS: bool = OFF.contains("anyShutdownAborts")   // 0x3 aborts even when no direction is open
  val BUG_NO_UNANNOUNCED_ABORT: bool = OFF.contains("noUnannouncedAbort") // A4 off: an end with no record is ignored
  val BUG_PERF_TRACEPOINT: bool = OFF.contains("perfTracepoint")          // A5 off: a SHUTDOWN record can be skipped silently
  val BUG_WAKE_BEFORE_RECORD: bool = OFF.contains("wakeBeforeRecord")     // A5 off: the wake can precede the record
  val BUG_NO_EPOCH: bool = OFF.contains("noEpoch")                        // A2 off: a record of another epoch is acted on
  val BUG_NO_REDIRECT_NOTICE: bool = OFF.contains("noRedirectNotice")     // A3 off: a refused redirect writes no record
  val BUG_IGNORE_RING_DROP: bool = OFF.contains("ignoreRingDrop")         // A7 off: the owner ignores a raised lost counter
  val BUG_NO_BACKSTOP: bool = OFF.contains("noBackstop")                  // A1: no level backstop
  val BUG_PER_CALL_COUNTER: bool = OFF.contains("perCallCounter")         // A1: the cell counts calls, not the cumulative quantity
  val BUG_ARM_FIRST: bool = OFF.contains("armFirst")                      // owner arms before it stores the goal
  val BUG_READ_BEFORE_ARM: bool = OFF.contains("readBeforeArm")           // owner reads sent before it arms
  val BUG_ABORT_KEEPS_ARMED: bool = OFF.contains("abortKeepsArmed")       // an abort leaves the cells armed
  val BUG_SEPARATE_CLOSE: bool = OFF.contains("separateClose")            // DG-5-C off: the clean close is a later step
  val BUG_NO_PERIODIC_READ: bool = OFF.contains("noPeriodicRead")         // H-3 off: the counters are read only at a record dispatch

  type Msg = { k: str, c: str, ep: int }           // k: drain | end2 | end3 | refused; epoch 1 is the flow's
  type Cell = { sent: int, goal: int, armed: bool, fired: bool }
  type F = {
    opener: str,                       // g = TcpConnect, h = TcpAccept (host side = the dialer's GuestStream)
    paired: str -> bool, pairMsg: bool,
    open: str -> bool, dead: str -> bool, loss: str -> bool, ctlDown: bool,
    rcv: str -> int,                   // d_x bytes received at x's outer leg (tcpi_bytes_received, FIN not counted)
    fin: str -> bool, rst: str -> bool,// the far end of x ended d_x with a FIN / reset the connection
    eofSeen: str -> bool, cls: str -> str,  // owner x saw its outer leg end; none | clean | abort
    sent: str -> int,                  // per cell: the target's cumulative kernel quantity
    cell: str -> Cell, kchk: str -> bool,
    dr: str -> str,                    // per cell: idle | goal | arm0 | armed | rdarm | waiting | done
    shut: str -> bool, shutAt: str -> int,
    gone: str -> bool, refusedOnce: str -> bool,
    vdead: str -> bool,                // s's flow vsock is dead (reset by a transport reset / orphan sweep): nothing more crosses it
    armShort: str -> bool, seen: str -> int,
    dlv: str -> int,                   // d_x bytes delivered from x's vsock to the other side's redirect
    wire: str -> List[str],            // vsock ops in flight toward side y: s2 | s3
    ring: str -> List[Msg], lateRec: str -> List[Msg],
    gotShut: str -> bool,              // y consumed a valid 0x2 record: d_other(y) is half-closed
    uWake: str -> bool,                // an end of y's flow vsock with no record
    skip1: str -> bool, skip2: str -> bool, miss: str -> int,
    failNext: str -> bool, lost: str -> int, seenLost: str -> int, perfSkip: str -> bool,
    obsMiss: str -> int, obsLost: str -> int, // the owner's last read of recursion_misses and of the lost count (H-3)
    used: Set[(str, str)],
    eofDeliv: str -> bool,             // ghost, per target side y: y's far end read EOF for d_other(y)
    farReset: str -> bool,             // ghost: x's far end was reset by an abort
    srcAborted: str -> bool,           // ghost: x aborted before its 0x2 was sent
    aborted: bool, faulted: bool,      // ghosts
  }

  var f: F

  // ------------------------------------------------------------------ helpers
  def other(s: str): str = if (s == "g") "h" else "g"
  def vk(s: str): str = if (s == "g") "Vg" else "Vh"
  def ck(s: str): str = if (s == "g") "Og" else "Oh"
  def sideOf(c: str): str = if (c == "Vg" or c == "Og") "g" else "h"
  def isV(c: str): bool = c == "Vg" or c == "Vh"
  def srcOf(c: str): str = if (isV(c)) sideOf(c) else other(sideOf(c))
  def isDirect(x: F, c: str): bool = x.opener == "h" and sideOf(c) == "h"
  def avail(x: F, c: str): int = if (isV(c)) x.rcv.get(sideOf(c)) else x.dlv.get(other(sideOf(c)))
  def goalNow(x: F, c: str): int = avail(x, c)
  def openA(x: F, s: str): bool = x.open.get(s) and not(x.dead.get(s))
  def maxi(a: int, b: int): int = if (a > b) a else b
  def push(x: F, y: str, op: str): F =
    if (x.dead.get(y)) x else { ...x, wire: x.wire.set(y, x.wire.get(y).append(op)) }
  val cell0: Cell = { sent: 0, goal: 0, armed: false, fired: false }

  // a ring record at side y (the producer's output fails when the ring is full: lost counter rises, and a
  // lost end record leaves a vsock end with no record)
  def emit(x: F, y: str, r: Msg, endEv: bool): F =
    if (x.failNext.get(y))
      { ...x, failNext: x.failNext.set(y, false), lost: x.lost.set(y, x.lost.get(y) + 1),
        uWake: if (endEv) x.uWake.set(y, true) else x.uWake }
    else { ...x, ring: x.ring.set(y, x.ring.get(y).append(r)) }

  // the kernel's check after a store of `sent`: armed and the goal reached; the winner of fired reports
  def fireIfMet(x: F, c: str): F =
    val cl = x.cell.get(c)
    if (cl.armed and not(cl.fired) and cl.sent >= cl.goal)
      emit({ ...x, cell: x.cell.set(c, { ...cl, fired: true }) }, sideOf(c), { k: "drain", c: c, ep: 1 }, false)
    else x

  // abort at s with reset semantics: remove the pair, reset the far end, close the vsock (the peer sees 0x3
  // while it has an open direction), disarm both cells. `echo` is false when the abort answers the peer's 0x3.
  def abortAt(x: F, s: str, echo: bool): F =
    if (not(x.open.get(s))) x
    else {
      val y = other(s)
      val x1 = { ...x, open: x.open.set(s, false), farReset: x.farReset.set(s, true),
                 srcAborted: x.srcAborted.set(s, x.srcAborted.get(s) or not(x.shut.get(vk(s)))),
                 aborted: true,
                 dr: if (BUG_ABORT_KEEPS_ARMED) x.dr else x.dr.set(vk(s), "idle").set(ck(s), "idle"),
                 cell: if (BUG_ABORT_KEEPS_ARMED) x.cell
                       else x.cell.set(vk(s), { ...x.cell.get(vk(s)), armed: false }).set(ck(s), { ...x.cell.get(ck(s)), armed: false }) }
      if (echo) push(x1, y, "s3") else x1
    }

  // DG-5-C: the clean close is part of the step that completes the second direction
  def doneAt(x: F, s: str): bool = x.shut.get(vk(s)) and x.shut.get(ck(s))
  def closeIfDone(x: F, s: str): F =
    if (not(BUG_SEPARATE_CLOSE) and x.open.get(s) and doneAt(x, s)) push({ ...x, open: x.open.set(s, false) }, other(s), "s3")
    else x

  // ------------------------------------------------------------------ init
  action init = {
    nondet o = Set("g", "h").oneOf()
    InitWith(o)
  }
  action InitWith(o: str): bool = all {
    f' = {
      opener: o,
      paired: SIDES.mapBy(s => false), pairMsg: false, open: SIDES.mapBy(s => true), dead: SIDES.mapBy(s => false),
      loss: SIDES.mapBy(s => false), ctlDown: false,
      rcv: SIDES.mapBy(s => 0), fin: SIDES.mapBy(s => false), rst: SIDES.mapBy(s => false),
      eofSeen: SIDES.mapBy(s => false), cls: SIDES.mapBy(s => "none"),
      sent: CELLS.mapBy(c => 0), cell: CELLS.mapBy(c => cell0), kchk: CELLS.mapBy(c => false),
      dr: CELLS.mapBy(c => "idle"), shut: CELLS.mapBy(c => false), shutAt: CELLS.mapBy(c => 0),
      gone: CELLS.mapBy(c => false), refusedOnce: CELLS.mapBy(c => false), vdead: SIDES.mapBy(s => false),
      armShort: CELLS.mapBy(c => false), seen: CELLS.mapBy(c => 0),
      dlv: SIDES.mapBy(s => 0), wire: SIDES.mapBy(s => List()), ring: SIDES.mapBy(s => List()),
      lateRec: SIDES.mapBy(s => List()), gotShut: SIDES.mapBy(s => false), uWake: SIDES.mapBy(s => false),
      skip1: SIDES.mapBy(s => false), skip2: SIDES.mapBy(s => false), miss: SIDES.mapBy(s => 0),
      failNext: SIDES.mapBy(s => false), lost: SIDES.mapBy(s => 0), seenLost: SIDES.mapBy(s => 0),
      obsMiss: SIDES.mapBy(s => 0), obsLost: SIDES.mapBy(s => 0),
      perfSkip: SIDES.mapBy(s => false), used: Set(),
      eofDeliv: SIDES.mapBy(s => false), farReset: SIDES.mapBy(s => false), srcAborted: SIDES.mapBy(s => false),
      aborted: false, faulted: false,
    },
  }

  // ------------------------------------------------------------------ pairing
  // the acceptor pairs (TcpConnect: host step 6; TcpAccept: guest step 5) and sends Paired on the session
  action HcPair = all {
    val a = other(f.opener)
    all {
      openA(f, a), not(f.paired.get(a)), not(f.loss.get(a)),
      f' = { ...f, paired: f.paired.set(a, true), pairMsg: not(f.ctlDown) },
    },
  }
  // the opener receives Paired (a closed flow discards it, U-5)
  action HcRecvPaired = all {
    f.pairMsg, not(f.dead.get(f.opener)),
    f' = { ...f, pairMsg: false, paired: if (f.open.get(f.opener)) f.paired.set(f.opener, true) else f.paired },
  }

  // ------------------------------------------------------------------ far ends (environment)
  // a far end's outer leg exists for the opener from the start and for the acceptor from its Paired on
  def farUp(s: str): bool = openA(f, s) and (s == f.opener or f.paired.get(s)) and not(f.fin.get(s)) and not(f.rst.get(s))
  action FarWrite(s: str): bool = all {
    ACTIVE.contains(s), farUp(s), f.rcv.get(s) < TOT,
    // the dialer's bytes (opener h, side h) go straight into its flow vsock: no redirect, sent counts them
    f' = { ...f, rcv: f.rcv.set(s, f.rcv.get(s) + 1),
           sent: if (f.opener == "h" and s == "h") f.sent.set("Vh", f.sent.get("Vh") + 1) else f.sent },
  }
  action FarFin(s: str): bool = all {
    ACTIVE.contains(s), farUp(s), f.rcv.get(s) == TOT,
    f' = { ...f, fin: f.fin.set(s, true) },
  }
  // the far end of s resets its connection: s's source direction ends in an unclean EOF, and the redirect
  // target of the other direction leaves ESTABLISHED
  action FarRst(s: str): bool = all {
    FAULTS.contains(("rst", s)),
    farUp(s), not(f.used.contains(("rst", s))),
    f' = { ...f, rst: f.rst.set(s, true), gone: f.gone.set(ck(s), true), faulted: true, used: f.used.union(Set(("rst", s))) },
  }

  // ------------------------------------------------------------------ owner: end of a source direction (R2-END, A6)
  // the owner of s sees its outer leg end (FIN in sequence, or a reset that reads as EOF) and classifies it
  action OwnSeesEof(s: str): bool = all {
    openA(f, s), not(f.eofSeen.get(s)), f.fin.get(s) or f.rst.get(s),
    val direct = f.opener == "h" and s == "h"
    val isAbort = if (direct) f.rst.get(s) else (f.rst.get(s) and not(BUG_NO_TCP_END_CHECK))
    val x1 = { ...f, eofSeen: f.eofSeen.set(s, true), cls: f.cls.set(s, if (isAbort) "abort" else "clean") }
    f' = if (isAbort) abortAt(x1, s, true)
         else if (direct) { ...x1, dr: x1.dr.set(vk(s), "done") }      // the dialer shuts down by itself
         else x1,
  }

  // ------------------------------------------------------------------ owner: drain protocol (R2-DRAIN)
  def drainWanted(c: str): bool =
    val s = sideOf(c)
    if (isV(c)) f.eofSeen.get(s) and f.cls.get(s) == "clean" and f.paired.get(s)
    else f.gotShut.get(s) and f.paired.get(s)
  def drained(c: str): bool = f.dr.get(c) == "done"
  // store the goal, then (below) arm, then read
  action OwnGoal(c: str): bool = all {
    openA(f, sideOf(c)), not(isDirect(f, c)), drainWanted(c),
    f.dr.get(c) == "idle" or (BUG_ARM_FIRST and f.dr.get(c) == "arm0"),
    val cl = f.cell.get(c)
    f' = { ...f, cell: f.cell.set(c, { ...cl, goal: goalNow(f, c) }),
           dr: f.dr.set(c, if (f.dr.get(c) == "arm0") "armed" else "goal") },
  }
  action OwnArm(c: str): bool = all {
    openA(f, sideOf(c)), not(isDirect(f, c)), drainWanted(c),
    or {
      f.dr.get(c) == "goal",
      BUG_ARM_FIRST and f.dr.get(c) == "idle",
      BUG_READ_BEFORE_ARM and f.dr.get(c) == "rdarm",
    },
    val cl = f.cell.get(c)
    f' = { ...f, cell: f.cell.set(c, { ...cl, armed: true }),
           dr: f.dr.set(c, if (f.dr.get(c) == "goal") "armed" else if (f.dr.get(c) == "idle") "arm0" else "waiting") },
  }
  // read sent: the owner or the kernel reports (whoever wins fired); a short read waits for the record and
  // requests the arm-time backstop
  action OwnRead(c: str): bool = all {
    openA(f, sideOf(c)), not(isDirect(f, c)),
    f.dr.get(c) == "armed" or (BUG_READ_BEFORE_ARM and f.dr.get(c) == "goal"),
    val cl = f.cell.get(c)
    val met = cl.sent >= cl.goal
    val early = f.dr.get(c) == "goal"
    f' = if (met and not(cl.fired)) { ...f, cell: f.cell.set(c, { ...cl, fired: true }), dr: f.dr.set(c, "done") }
         else if (met) { ...f, dr: f.dr.set(c, "waiting") }
         else if (early) { ...f, dr: f.dr.set(c, "rdarm") }
         else { ...f, dr: f.dr.set(c, "waiting"), armShort: f.armShort.set(c, true) },
  }
  // the level backstop: a zero-event poll on the target runs the same cumulative read and check in the kernel
  action OwnBackstop(c: str): bool = all {
    not(BUG_NO_BACKSTOP), openA(f, sideOf(c)), not(isDirect(f, c)),
    f.dr.get(c) == "waiting", not(f.cell.get(c).fired),
    f.obsMiss.get(sideOf(c)) > f.seen.get(c) or f.armShort.get(c),
    val cl = f.cell.get(c)
    val x1 = { ...f, seen: f.seen.set(c, f.obsMiss.get(sideOf(c))), armShort: f.armShort.set(c, false),
               cell: f.cell.set(c, { ...cl, sent: if (BUG_PER_CALL_COUNTER) cl.sent else maxi(cl.sent, f.sent.get(c)) }) }
    f' = fireIfMet(x1, c),
  }
  // the shutdown that propagates the half-close, after the drain completed
  action OwnShut(c: str): bool = all {
    val s = sideOf(c)
    all {
      openA(f, s), f.dr.get(c) == "done", not(f.shut.get(c)),
      f' = if (not(isDirect(f, c)) and f.gone.get(c)) abortAt(f, s, true)    // shutdown on a socket that left ESTABLISHED fails
           else {
             val x1 = { ...f, shut: f.shut.set(c, true),
                        shutAt: f.shutAt.set(c, if (isDirect(f, c)) TOT else f.sent.get(c)),
                        cell: f.cell.set(c, { ...f.cell.get(c), armed: false }) }
             val x2 = if (isV(c)) push(x1, other(s), "s2") else { ...x1, eofDeliv: x1.eofDeliv.set(s, true) }
             closeIfDone(x2, s)
           },
    },
  }
  // DG-5-C off: the clean close as a step of its own
  action HcLateClose(s: str): bool = all {
    BUG_SEPARATE_CLOSE, openA(f, s), doneAt(f, s),
    f' = push({ ...f, open: f.open.set(s, false) }, other(s), "s3"),
  }

  // ------------------------------------------------------------------ owner: ring, lost counter, unannounced end
  def handle(x: F, s: str, r: Msg): F =
    if (r.k == "drain") (if (x.dr.get(r.c).in(Set("armed", "waiting"))) { ...x, dr: x.dr.set(r.c, "done") } else x)
    else if (r.k == "refused") abortAt(x, s, true)
    else if (r.k == "end2" or (BUG_EOF_IS_HALF and r.k == "end3")) {
      if (x.gotShut.get(s)) x
      else if (x.opener == "h" and s == "h")
        // the dialer reads the end of its GuestStream directly (INB-1): no drain race
        closeIfDone({ ...x, gotShut: x.gotShut.set(s, true), eofDeliv: x.eofDeliv.set(s, true),
                      shut: x.shut.set(ck(s), true), shutAt: x.shutAt.set(ck(s), TOT), dr: x.dr.set(ck(s), "done") }, s)
      else { ...x, gotShut: x.gotShut.set(s, true) }
    }
    else {
      // end3: an abort while any direction is open; benign once both directions are ended
      val benign = x.gotShut.get(s) and x.shut.get(vk(s))
      if (benign and not(BUG_ANY_SHUTDOWN_ABORTS)) x else abortAt(x, s, false)
    }
  action OwnRing(s: str): bool = all {
    f.ring.get(s).length() > 0,
    val r = f.ring.get(s).head()
    // H-3: the dispatch of a record also reads the counters
    val f0 = { ...f, ring: f.ring.set(s, f.ring.get(s).tail()),
               obsMiss: f.obsMiss.set(s, f.miss.get(s)), obsLost: f.obsLost.set(s, f.lost.get(s)) }
    f' = if (not(openA(f0, s))) f0
         else if (r.ep != 1 and not(BUG_NO_EPOCH)) f0
         else handle(f0, s, r),
  }
  // A7: a raised lost counter, as the owner read it, aborts the flow (a drain or classification record may have been lost)
  action OwnSeeLost(s: str): bool = all {
    not(BUG_IGNORE_RING_DROP), f.obsLost.get(s) > f.seenLost.get(s),
    f' = abortAt({ ...f, seenLost: f.seenLost.set(s, f.obsLost.get(s)) }, s, true),
  }
  // H-3: the owner's periodic counter read on the ADR-0124 1 s audit tick (weakly fair): it reads recursion_misses
  // and the lost count of side s. Enabled only when a read would observe a change (an unchanged read is a stutter).
  // noPeriodicRead: the counters are read only at a record dispatch.
  action OwnAuditTick(s: str): bool = all {
    not(BUG_NO_PERIODIC_READ), not(f.dead.get(s)),
    f.obsMiss.get(s) != f.miss.get(s) or f.obsLost.get(s) != f.lost.get(s),
    f' = { ...f, obsMiss: f.obsMiss.set(s, f.miss.get(s)), obsLost: f.obsLost.set(s, f.lost.get(s)) },
  }
  // A4: an end of the flow vsock with no classification record is an abort
  action OwnUnannounced(s: str): bool = all {
    not(BUG_NO_UNANNOUNCED_ABORT), f.uWake.get(s),
    f' = abortAt({ ...f, uWake: f.uWake.set(s, false) }, s, true),
  }
  // DG-5-F: the owner observes the session's loss and aborts the flow
  action HcLossObserved(s: str): bool = all {
    f.ctlDown, not(f.dead.get(s)), not(f.loss.get(s)),
    f' = abortAt({ ...f, loss: f.loss.set(s, true) }, s, true),
  }

  // ------------------------------------------------------------------ kernel
  // one redirected send into target c: the cumulative quantity advances; each fexit program may miss the call
  action KSend(c: str): bool = all {
    val s = sideOf(c)
    all {
      openA(f, s), not(isDirect(f, c)), f.paired.get(s), not(f.gone.get(c)), f.sent.get(c) < avail(f, c),
      not(isV(c) and f.vdead.get(s)),
      val n = f.sent.get(c) + 1
      val r1 = not(f.skip1.get(s))
      val r2 = not(f.skip2.get(s))
      val cl = f.cell.get(c)
      val ran = r1 or r2
      f' = { ...f, sent: f.sent.set(c, n),
             cell: f.cell.set(c, { ...cl, sent: if (not(ran)) cl.sent
                                                else if (BUG_PER_CALL_COUNTER) cl.sent + 1
                                                else maxi(cl.sent, n) }),
             kchk: f.kchk.set(c, f.kchk.get(c) or ran),
             skip1: f.skip1.set(s, false), skip2: f.skip2.set(s, false),
             miss: f.miss.set(s, f.miss.get(s) + (if (r1) 0 else 1) + (if (r2) 0 else 1)) },
    },
  }
  // the program's second half: read armed and goal; whoever wins fired reports through the ring
  action KCheck(c: str): bool = all {
    f.kchk.get(c),
    f' = fireIfMet({ ...f, kchk: f.kchk.set(c, false) }, c),
  }
  // A3: a redirect into a target that left ESTABLISHED is refused; the first refusal writes one record
  action KRefuse(c: str): bool = all {
    val s = sideOf(c)
    all {
      openA(f, s), not(isDirect(f, c)), f.paired.get(s), f.gone.get(c), not(f.refusedOnce.get(c)), f.sent.get(c) < avail(f, c),
      f' = if (BUG_NO_REDIRECT_NOTICE) { ...f, refusedOnce: f.refusedOnce.set(c, true) }
           else emit({ ...f, refusedOnce: f.refusedOnce.set(c, true) }, s, { k: "refused", c: c, ep: 1 }, false),
    },
  }
  // d_x bytes cross the vsock and are forwarded by the receiving side's redirect (in order)
  action KDeliver(x: str): bool = all {
    val y = other(x)
    all {
      openA(f, y), f.paired.get(y), not(f.vdead.get(y)), not(f.vdead.get(x)), f.dlv.get(x) < f.sent.get(vk(x)),
      f' = { ...f, dlv: f.dlv.set(x, f.dlv.get(x) + 1) },
    },
  }
  // the raw-tracepoint program writes the record of a received SHUTDOWN / RST before the EOF wake (A5).
  // 0x2 follows the data in order (A8).
  action KRecv(y: str): bool = all {
    val x = other(y)
    all {
      not(f.dead.get(y)), not(f.vdead.get(y)), f.wire.get(y).length() > 0,
      f.wire.get(y).head() == "s3" or f.dlv.get(x) == f.sent.get(vk(x)),
      val op = f.wire.get(y).head()
      val r: Msg = { k: if (op == "s2") "end2" else "end3", c: "", ep: 1 }
      val x0 = { ...f, wire: f.wire.set(y, f.wire.get(y).tail()) }
      f' = if (f.perfSkip.get(y)) { ...x0, perfSkip: x0.perfSkip.set(y, false), uWake: x0.uWake.set(y, true) }
           else if (BUG_WAKE_BEFORE_RECORD) { ...x0, uWake: x0.uWake.set(y, true), lateRec: x0.lateRec.set(y, x0.lateRec.get(y).append(r)) }
           else emit(x0, y, r, true),
    },
  }
  // hazard only (A5 off, wakeBeforeRecord): the record written after the wake. Not a design action.
  // hazard only (wakeBeforeRecord, A5 off): the record written after the EOF wake; enabled only under BUG_WAKE_BEFORE_RECORD
  action KLateRecord(y: str): bool = all {
    BUG_WAKE_BEFORE_RECORD, f.lateRec.get(y).length() > 0,
    val r = f.lateRec.get(y).head()
    f' = emit({ ...f, lateRec: f.lateRec.set(y, f.lateRec.get(y).tail()) }, y, r, false),
  }

  // ------------------------------------------------------------------ environment faults (one use each)
  action EnvMiss1(s: str): bool = all {
    FAULTS.contains(("m1", s)),
    not(f.used.contains(("m1", s))),
    f' = { ...f, skip1: f.skip1.set(s, true), used: f.used.union(Set(("m1", s))) },
  }
  action EnvMiss2(s: str): bool = all {
    FAULTS.contains(("m2", s)),
    not(f.used.contains(("m2", s))),
    f' = { ...f, skip2: f.skip2.set(s, true), used: f.used.union(Set(("m2", s))) },
  }
  action EnvRingFull(s: str): bool = all {
    FAULTS.contains(("full", s)),
    not(f.used.contains(("full", s))),
    f' = { ...f, failNext: f.failNext.set(s, true), faulted: true, used: f.used.union(Set(("full", s))) },
  }
  // a record of the cell's earlier incarnation (epoch 0) reaches the ring
  action EnvStaleDrain(c: str): bool = all {
    FAULTS.contains(("stale", c)),
    not(f.used.contains(("stale", c))),
    f' = { ...f, ring: f.ring.set(sideOf(c), f.ring.get(sideOf(c)).append({ k: "drain", c: c, ep: 0 })),
           used: f.used.union(Set(("stale", c))) },
  }
  action EnvStaleEnd(s: str): bool = all {
    FAULTS.contains(("stale2", s)),
    not(f.used.contains(("stale2", s))),
    f' = { ...f, ring: f.ring.set(s, f.ring.get(s).append({ k: "end2", c: "", ep: 0 })),
           used: f.used.union(Set(("stale2", s))) },
  }
  // the target of c left ESTABLISHED without any record (vsock_poll shows RDHUP only)
  action EnvTargetGone(c: str): bool = all {
    FAULTS.contains(("gone", c)),
    not(isDirect(f, c)), not(f.used.contains(("gone", c))),
    f' = { ...f, gone: f.gone.set(c, true), faulted: true, used: f.used.union(Set(("gone", c))) },
  }
  // the host's flow vsock ends with no SHUTDOWN / RST packet (the orphan sweep of a gone VMM; any record-less end):
  // nothing more crosses it. The guest side has no such end: a guest vsock ends only by a packet or with the VM.
  action EnvUnannounced(s: str): bool = all {
    FAULTS.contains(("unann", s)),
    not(f.used.contains(("unann", s))), not(f.dead.get(s)),
    f' = { ...f, uWake: f.uWake.set(s, true), vdead: f.vdead.set(s, true), faulted: true, used: f.used.union(Set(("unann", s))) },
  }
  // A5 (M-2): the raw-tracepoint classifier is skipped on the same program's recursion on one CPU
  // (kernel/trace/bpf_trace.c:2080-2083 @ v7.0): the next SHUTDOWN / RST record at s is not written; the end is then an
  // unannounced end and aborts (A4). A fault: a clean flow may abort on it.
  action EnvClassifierSkip(s: str): bool = all {
    FAULTS.contains(("cskip", s)),
    not(f.used.contains(("cskip", s))),
    f' = { ...f, perfSkip: f.perfSkip.set(s, true), faulted: true, used: f.used.union(Set(("cskip", s))) },
  }
  // hazard only (A5 off): the next SHUTDOWN record at s is skipped silently
  action EnvPerfSkip(s: str): bool = all {
    BUG_PERF_TRACEPOINT, not(f.used.contains(("perf", s))),
    f' = { ...f, perfSkip: f.perfSkip.set(s, true), used: f.used.union(Set(("perf", s))) },
  }
  // serve crashes: the host owner's sockets close (its far end is reset, the guest sees 0x3 for open directions)
  action HostOwnerDies = all {
    FAULTS.contains(("hostdie", "h")),
    not(f.dead.get("h")),
    // a dead owner holds nothing: its drain phases and armed cells vanish with the process
    f' = push({ ...f, dead: f.dead.set("h", true), open: f.open.set("h", false), farReset: f.farReset.set("h", true),
                dr: f.dr.set("Vh", "idle").set("Oh", "idle"),
                cell: f.cell.set("Vh", { ...f.cell.get("Vh"), armed: false }).set("Oh", { ...f.cell.get("Oh"), armed: false }),
                srcAborted: f.srcAborted.set("h", f.srcAborted.get("h") or not(f.shut.get("Vh"))),
                ctlDown: true, pairMsg: false, faulted: true, wire: f.wire.set("h", List()) }, "g", "s3"),
  }
  // the guest's PID 1 crashes: no SHUTDOWN is sent, the VMM exits (RESET-EXIT) and the orphan sweep ends the
  // host's flow vsock without a record
  action GuestPid1Crash = all {
    FAULTS.contains(("pid1", "g")),
    not(f.dead.get("g")),
    f' = { ...f, dead: f.dead.set("g", true), open: f.open.set("g", false),
           dr: f.dr.set("Vg", "idle").set("Og", "idle"),
           cell: f.cell.set("Vg", { ...f.cell.get("Vg"), armed: false }).set("Og", { ...f.cell.get("Og"), armed: false }),
           srcAborted: f.srcAborted.set("g", f.srcAborted.get("g") or not(f.shut.get("Vg"))),
           ctlDown: true, pairMsg: false, faulted: true, wire: f.wire.set("g", List()),
           uWake: f.uWake.set("h", true), vdead: f.vdead.set("h", true) },
  }
  action HcSessionDrop = all {
    FAULTS.contains(("drop", "g")),
    not(f.ctlDown),
    f' = { ...f, ctlDown: true, pairMsg: false, faulted: true },
  }

  // ------------------------------------------------------------------ next
  action step = {
    nondet s = SIDES.oneOf()
    nondet c = CELLS.oneOf()
    any {
      HcPair, HcRecvPaired, FarWrite(s), FarFin(s), FarRst(s), OwnSeesEof(s),
      OwnGoal(c), OwnArm(c), OwnRead(c), OwnBackstop(c), OwnShut(c), HcLateClose(s),
      OwnRing(s), OwnSeeLost(s), OwnAuditTick(s), OwnUnannounced(s), HcLossObserved(s),
      KSend(c), KCheck(c), KRefuse(c), KDeliver(s), KRecv(s), KLateRecord(s),
      EnvMiss1(s), EnvMiss2(s), EnvRingFull(s), EnvStaleDrain(c), EnvStaleEnd(s), EnvTargetGone(c),
      EnvUnannounced(s), EnvClassifierSkip(s), EnvPerfSkip(s), HostOwnerDies, GuestPid1Crash, HcSessionDrop,
    }
  }

  // ------------------------------------------------------------------ invariants
  // NoEarlyShutdown (R2-DRAIN): no outer leg or flow vsock is shut down for writing before every byte
  // forwarded toward it was sent
  val NoEarlyShutdown: bool = CELLS.forall(c =>
    (f.shut.get(c) and not(isDirect(f, c))) implies f.shutAt.get(c) == TOT)
  // NoEofOnPeerLoss (R2-END): a far end reads EOF for a direction only if the direction's far end half-closed
  // it — never because an owner died, a session dropped or a reset was read as EOF
  val NoEofOnPeerLoss: bool = SIDES.forall(y => f.eofDeliv.get(y) implies f.fin.get(other(y)))
  // AbortNeverCleanEof (R2-END): no clean EOF is delivered for a direction whose source aborted before it was
  // announced
  val AbortNeverCleanEof: bool = SIDES.forall(y => f.eofDeliv.get(y) implies not(f.srcAborted.get(other(y))))
  // an ended flow holds no armed cell and no waiting drain
  val EndedFlowHoldsNoArmedCell: bool = SIDES.forall(s =>
    not(f.open.get(s)) implies (not(f.cell.get(vk(s)).armed) and not(f.cell.get(ck(s)).armed)
                                and not(f.dr.get(vk(s)).in(Set("goal", "arm0", "armed", "rdarm", "waiting")))
                                and not(f.dr.get(ck(s)).in(Set("goal", "arm0", "armed", "rdarm", "waiting")))))
  // DG-5-C: a flow whose two directions are both shut down is closed in the step that completed the second
  val NoCompletedFlowStaysOpen: bool = SIDES.forall(s => doneAt(f, s) implies not(f.open.get(s)))
  // a clean flow aborts only on a fault, a classifier skip (A5) included: a clean half-close is never turned into an
  // abort without one
  val AbortOnlyOnFault: bool = f.aborted implies f.faulted
  val Safety: bool = and {
    NoEarlyShutdown, NoEofOnPeerLoss, AbortNeverCleanEof, EndedFlowHoldsNoArmedCell,
    NoCompletedFlowStaysOpen, AbortOnlyOnFault,
  }

  // ------------------------------------------------------------------ liveness (TLC)
  val allVars = f
  temporal hcFair = and {
    weakFair(HcPair, allVars), weakFair(HcRecvPaired, allVars),
    SIDES.forall(s => and {
      weakFair(OwnSeesEof(s), allVars), weakFair(OwnRing(s), allVars), weakFair(OwnSeeLost(s), allVars),
      weakFair(OwnAuditTick(s), allVars),
      weakFair(OwnUnannounced(s), allVars), weakFair(HcLossObserved(s), allVars), weakFair(HcLateClose(s), allVars),
      weakFair(KDeliver(s), allVars), weakFair(KRecv(s), allVars), weakFair(KLateRecord(s), allVars),
    }),
    CELLS.forall(c => and {
      weakFair(OwnGoal(c), allVars), weakFair(OwnArm(c), allVars), weakFair(OwnRead(c), allVars),
      weakFair(OwnBackstop(c), allVars), weakFair(OwnShut(c), allVars),
      weakFair(KSend(c), allVars), weakFair(KCheck(c), allVars), weakFair(KRefuse(c), allVars),
    }),
  }
  // d_x is resolved: the other far end read EOF, or the flow is gone at both owners
  def dirDone(x: str): bool = f.eofDeliv.get(other(x)) or (not(f.open.get(x)) and not(f.open.get(other(x))))
  def endPending(x: str): bool = f.fin.get(x) or f.rst.get(x)
  // EveryEndResolves (R2-DRAIN, R2-END): the end of a direction completes, or the flow aborts — no deadline
  temporal EveryEndResolves =
    hcFair.implies(SIDES.forall(x => always(endPending(x).implies(eventually(dirDone(x))))))
  // a clean half-close, with no fault, reaches the other far end as EOF
  temporal CleanHalfCloseCompletes =
    hcFair.implies(SIDES.forall(x => always((f.fin.get(x) and not(f.faulted)).implies(eventually(f.eofDeliv.get(other(x)) or f.faulted)))))

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // Deterministic scenarios on `end_all` (TOT = 1), each ending in an `expect` of its outcome with `Safety` checked along
  // the way. The first direction d_x of a TcpConnect flow, to its end at the far end of the other side:
  // happy path, guest-opened flow (TcpConnect, both owners forward): d_g half-closes, drains, announces by 0x2,
  // the host drains and shuts the destination; then d_h; the second direction closes each owner in its step
  run TcpConnectBothDirectionsHalfCloseThenCloseCleanly = InitWith("g")
    .then(HcPair).then(HcRecvPaired).expect(Safety and f.paired.get("g") and f.paired.get("h"))
    .then(FarWrite("g")).then(FarFin("g")).then(OwnSeesEof("g"))
    .expect(Safety and f.cls.get("g") == "clean" and f.dr.get("Vg") == "idle")
    .then(KSend("Vg")).then(KCheck("Vg"))
    .then(OwnGoal("Vg")).then(OwnArm("Vg")).expect(Safety and f.dr.get("Vg") == "armed" and f.cell.get("Vg").goal == 1)
    .then(OwnRead("Vg")).expect(Safety and f.dr.get("Vg") == "done" and f.cell.get("Vg").fired)
    .then(OwnShut("Vg")).expect(Safety and f.shut.get("Vg") and f.wire.get("h") == List("s2") and f.open.get("g"))
    .then(KDeliver("g")).then(KRecv("h"))
    .expect(Safety and f.ring.get("h") == List({ k: "end2", c: "", ep: 1 }))
    .then(OwnRing("h")).expect(Safety and f.gotShut.get("h") and f.dr.get("Oh") == "idle")
    .then(OwnGoal("Oh")).then(OwnArm("Oh")).then(KSend("Oh")).then(OwnRead("Oh"))
    .expect(Safety and f.dr.get("Oh") == "done" and f.cell.get("Oh").goal == 1 and not(f.eofDeliv.get("h")))
    .then(OwnShut("Oh")).expect(Safety and f.eofDeliv.get("h") and f.shut.get("Oh") and f.open.get("h") and not(f.faulted))
    .then(FarWrite("h")).then(FarFin("h")).then(OwnSeesEof("h")).then(KSend("Vh")).then(OwnGoal("Vh")).then(OwnArm("Vh")).then(OwnRead("Vh"))
    .then(OwnShut("Vh"))
    .expect(Safety and not(f.open.get("h")) and f.wire.get("g") == List("s2", "s3") and not(f.aborted))
    .then(KDeliver("h")).then(KRecv("g")).then(OwnRing("g")).expect(Safety and f.gotShut.get("g") and f.open.get("g"))
    .then(OwnGoal("Og")).then(OwnArm("Og")).then(KSend("Og")).then(OwnRead("Og"))
    .then(KRecv("g")).then(OwnRing("g")).expect(Safety and f.open.get("g") and not(f.aborted) and f.dr.get("Og") == "done")
    .then(OwnShut("Og"))
    .expect(Safety and not(f.open.get("g")) and not(f.open.get("h")) and f.eofDeliv.get("g") and f.eofDeliv.get("h")
            and not(f.aborted) and not(f.faulted))
  // happy path, host-opened flow (TcpAccept through the dialer): the host side has no cells; the dialer reads
  // the guest's end directly and shuts its own direction down by itself
  run TcpAcceptDialerBothDirectionsHalfCloseThenCloseCleanly = InitWith("h")
    .then(HcPair).then(HcRecvPaired).expect(Safety and f.paired.get("g") and f.paired.get("h"))
    .then(FarWrite("h")).then(FarFin("h")).then(OwnSeesEof("h"))
    .expect(Safety and f.cls.get("h") == "clean" and f.dr.get("Vh") == "done")
    .then(OwnShut("Vh")).expect(Safety and f.shut.get("Vh") and f.wire.get("g") == List("s2") and f.open.get("h"))
    .then(KDeliver("h")).then(KRecv("g")).then(OwnRing("g"))
    .expect(Safety and f.gotShut.get("g") and f.dr.get("Og") == "idle")
    .then(OwnGoal("Og")).then(OwnArm("Og")).then(KSend("Og")).then(OwnRead("Og")).then(OwnShut("Og"))
    .expect(Safety and f.eofDeliv.get("g") and f.open.get("g") and not(f.faulted))
    .then(FarWrite("g")).then(FarFin("g")).then(OwnSeesEof("g")).then(KSend("Vg")).then(OwnGoal("Vg")).then(OwnArm("Vg")).then(OwnRead("Vg"))
    .then(OwnShut("Vg")).expect(Safety and not(f.open.get("g")) and f.wire.get("h") == List("s2", "s3"))
    .then(KDeliver("g")).then(KRecv("h")).then(OwnRing("h"))
    .expect(Safety and f.eofDeliv.get("h") and f.shut.get("Oh") and not(f.open.get("h")) and not(f.aborted) and not(f.faulted))
    .then(KRecv("h")).then(OwnRing("h")).expect(Safety and not(f.open.get("h")) and not(f.aborted))
  // a clean close does not abort the peer: x's flow closes (0x3 after 0x2) while the peer's drain toward its
  // outer leg is still pending; the 0x3 is benign and the drain completes with a clean EOF
  run CleanCloseZeroX3IsBenignWhilePeerDrainPending = InitWith("g")
    .then(HcPair).then(HcRecvPaired)
    .then(FarWrite("g")).then(FarFin("g")).then(OwnSeesEof("g")).then(KSend("Vg")).then(OwnGoal("Vg")).then(OwnArm("Vg")).then(OwnRead("Vg")).then(OwnShut("Vg"))
    .then(FarWrite("h")).then(FarFin("h")).then(OwnSeesEof("h")).then(KSend("Vh")).then(OwnGoal("Vh")).then(OwnArm("Vh")).then(OwnRead("Vh")).then(OwnShut("Vh"))
    .then(KDeliver("g")).then(KRecv("h")).then(OwnRing("h")).then(OwnGoal("Oh")).then(OwnArm("Oh")).then(KSend("Oh")).then(OwnRead("Oh")).then(OwnShut("Oh"))
    .expect(Safety and not(f.open.get("h")) and f.wire.get("g") == List("s2", "s3") and f.eofDeliv.get("h"))
    .then(KDeliver("h")).then(KRecv("g")).then(KRecv("g"))
    .then(OwnRing("g")).then(OwnRing("g"))
    .expect(Safety and f.open.get("g") and not(f.aborted) and f.gotShut.get("g") and f.shut.get("Vg"))
    .then(OwnGoal("Og")).then(OwnArm("Og")).then(KSend("Og")).then(OwnRead("Og")).then(OwnShut("Og"))
    .expect(Safety and not(f.open.get("g")) and f.eofDeliv.get("g") and not(f.aborted) and not(f.faulted))
  // A1: both fexit programs miss the last send; the cell stays short; the owner's arm-time read is short and the
  // level backstop re-reads the cumulative quantity and completes the drain
  run RecursionMissOnLastCallIsCoveredByBackstop = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(FarWrite("g")).then(FarFin("g")).then(OwnSeesEof("g"))
    .then(EnvMiss1("g")).then(EnvMiss2("g")).then(KSend("Vg"))
    .expect(Safety and f.sent.get("Vg") == 1 and f.cell.get("Vg").sent == 0 and f.miss.get("g") == 2 and not(f.kchk.get("Vg")))
    .then(OwnGoal("Vg")).then(OwnArm("Vg")).then(OwnRead("Vg"))
    .expect(Safety and f.dr.get("Vg") == "waiting" and f.armShort.get("Vg"))
    .then(OwnBackstop("Vg")).expect(Safety and f.cell.get("Vg").fired and f.ring.get("g").length() == 1)
    .then(OwnRing("g")).expect(Safety and f.dr.get("Vg") == "done")
    .then(OwnShut("Vg")).expect(Safety and f.shut.get("Vg") and f.shutAt.get("Vg") == 1 and not(f.faulted))
  // A1: one program misses; the other covers the call; the kernel completes the drain by a ring record
  run SingleProgramMissIsCoveredByTheOtherAndKernelFires = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(FarWrite("g")).then(FarFin("g")).then(OwnSeesEof("g"))
    .then(OwnGoal("Vg")).then(OwnArm("Vg"))
    .then(EnvMiss1("g")).then(KSend("Vg")).expect(Safety and f.cell.get("Vg").sent == 1 and f.miss.get("g") == 1)
    .then(KCheck("Vg")).expect(Safety and f.cell.get("Vg").fired and f.ring.get("g").length() == 1)
    .then(OwnRead("Vg")).expect(Safety and f.dr.get("Vg") == "waiting" and f.cell.get("Vg").fired)
    .then(OwnRing("g")).expect(Safety and f.dr.get("Vg") == "done" and not(f.faulted))
  // (H-3) the lost count is a counter, not an event: nothing acts on its rise before the owner reads it
  run RingDropIsNotActedOnBeforeTheOwnerReadsTheCount = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(FarWrite("g")).then(FarFin("g")).then(OwnSeesEof("g"))
    .then(OwnGoal("Vg")).then(OwnArm("Vg")).then(EnvRingFull("g"))
    .then(KSend("Vg")).then(KCheck("Vg"))
    .expect(Safety and f.cell.get("Vg").fired and f.lost.get("g") == 1 and f.ring.get("g") == List())
    .then(OwnRead("Vg")).expect(Safety and f.dr.get("Vg") == "waiting" and f.obsLost.get("g") == 0)
    .then(OwnSeeLost("g")).fail()
  // A7: the full ring drops the completion record; the owner reads the raised lost count at the next audit tick (H-3)
  // and aborts the flow, resetting both far ends
  run RingDropAbortsTheFlowAndResetsBothFarEnds = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(FarWrite("g")).then(FarFin("g")).then(OwnSeesEof("g"))
    .then(OwnGoal("Vg")).then(OwnArm("Vg")).then(EnvRingFull("g"))
    .then(KSend("Vg")).then(KCheck("Vg")).then(OwnRead("Vg"))
    .then(OwnAuditTick("g")).expect(Safety and f.obsLost.get("g") == 1 and f.open.get("g"))
    .then(OwnSeeLost("g")).expect(Safety and not(f.open.get("g")) and f.farReset.get("g") and f.wire.get("h") == List("s3"))
    .then(KRecv("h")).then(OwnRing("h")).expect(Safety and not(f.open.get("h")) and f.farReset.get("h") and not(f.eofDeliv.get("h")))
  // H-3: both fexit programs miss the last send AFTER the owner's arm-time read: no record will come; the owner's audit
  // tick reads the recursion_misses rise and the backstop completes the drain (detected within one audit period)
  run BothSkipAfterArmIsFoundByTheAuditTick = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(FarWrite("g")).then(FarFin("g")).then(OwnSeesEof("g"))
    .then(OwnGoal("Vg")).then(OwnArm("Vg")).then(OwnRead("Vg"))
    .expect(Safety and f.dr.get("Vg") == "waiting" and f.armShort.get("Vg"))
    .then(OwnBackstop("Vg")).expect(Safety and not(f.armShort.get("Vg")) and not(f.cell.get("Vg").fired))
    .then(EnvMiss1("g")).then(EnvMiss2("g")).then(KSend("Vg"))
    .expect(Safety and f.miss.get("g") == 2 and f.obsMiss.get("g") == 0 and f.cell.get("Vg").sent == 0 and f.ring.get("g") == List())
    .then(OwnAuditTick("g")).expect(Safety and f.obsMiss.get("g") == 2)
    .then(OwnBackstop("Vg")).expect(Safety and f.cell.get("Vg").fired and f.ring.get("g").length() == 1)
    .then(OwnRing("g")).then(OwnShut("Vg")).expect(Safety and f.shut.get("Vg") and f.shutAt.get("Vg") == 1 and not(f.faulted))
  // A5 (M-2): the classifier is skipped on its own recursion for the peer's 0x2: the end has no record and is an A4
  // abort, with reset at both far ends; the classifier skip is a fault (AbortOnlyOnFault)
  run ClassifierSkipTurnsACleanEndIntoAnAbort = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(FarWrite("g")).then(FarFin("g")).then(OwnSeesEof("g"))
    .then(KSend("Vg")).then(OwnGoal("Vg")).then(OwnArm("Vg")).then(OwnRead("Vg")).then(OwnShut("Vg"))
    .then(EnvClassifierSkip("h")).then(KDeliver("g")).then(KRecv("h"))
    .expect(Safety and f.ring.get("h") == List() and f.uWake.get("h") and f.faulted and not(f.gotShut.get("h")))
    .then(OwnUnannounced("h")).expect(Safety and not(f.open.get("h")) and f.farReset.get("h") and f.aborted and not(f.eofDeliv.get("h")))
  // A2: a completion record of the cell's earlier incarnation arrives while the drain waits; it is discarded
  run StaleEpochRecordIsDiscarded = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(FarWrite("g")).then(FarFin("g")).then(OwnSeesEof("g"))
    .then(OwnGoal("Vg")).then(OwnArm("Vg")).then(OwnRead("Vg")).expect(Safety and f.dr.get("Vg") == "waiting")
    .then(EnvStaleDrain("Vg")).then(OwnRing("g")).expect(Safety and f.dr.get("Vg") == "waiting" and f.ring.get("g") == List())
    .then(KSend("Vg")).then(KCheck("Vg")).then(OwnRing("g")).expect(Safety and f.dr.get("Vg") == "done")
    .then(OwnShut("Vg")).expect(Safety and f.shutAt.get("Vg") == 1 and not(f.faulted))
  // A2: a stale classification record (0x2 of an earlier flow) is discarded; no half-close is invented
  run StaleEpochEndRecordIsDiscarded = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(EnvStaleEnd("h")).then(OwnRing("h"))
    .expect(Safety and not(f.gotShut.get("h")) and f.open.get("h") and f.ring.get("h") == List())
  // A3: the target left ESTABLISHED with a send pending; the refused redirect writes one record and the owner aborts
  run RefusedRedirectAbortsTheFlow = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(FarWrite("g")).then(FarFin("g")).then(OwnSeesEof("g"))
    .then(EnvTargetGone("Vg")).then(OwnGoal("Vg")).then(OwnArm("Vg")).then(OwnRead("Vg"))
    .then(KRefuse("Vg")).expect(Safety and f.refusedOnce.get("Vg") and f.ring.get("g").length() == 1)
    .then(OwnRing("g")).expect(Safety and not(f.open.get("g")) and f.farReset.get("g") and f.wire.get("h") == List("s3"))
    .then(KRecv("h")).then(OwnRing("h")).expect(Safety and not(f.open.get("h")) and f.farReset.get("h"))
  // A4: an end of the flow vsock with no record is an unannounced end: the owner aborts with reset
  run UnannouncedEndAbortsWithReset = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(EnvUnannounced("h")).then(OwnUnannounced("h"))
    .expect(Safety and not(f.open.get("h")) and f.farReset.get("h") and f.wire.get("g") == List("s3"))
    .then(KRecv("g")).then(OwnRing("g")).expect(Safety and not(f.open.get("g")) and f.farReset.get("g") and not(f.eofDeliv.get("g")))
  // A6: the TCP source's EOF is really a destination reset: classified an abort, never a clean EOF
  run TcpSourceResetIsAbortNotCleanEof = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(FarWrite("g")).then(FarRst("g")).then(OwnSeesEof("g"))
    .expect(Safety and f.cls.get("g") == "abort" and not(f.open.get("g")) and f.farReset.get("g") and f.wire.get("h") == List("s3"))
    .then(KRecv("h")).then(OwnRing("h")).expect(Safety and not(f.open.get("h")) and not(f.eofDeliv.get("h")) and not(f.eofDeliv.get("g")))
  // the far end of the acceptor resets after Paired (TcpAccept opener): both owners abort, both far ends are reset
  run FarEndResetAfterPairedAbortsBothOwners = InitWith("h")
    .then(HcPair).then(HcRecvPaired).then(FarRst("g")).then(OwnSeesEof("g"))
    .then(KRecv("h")).then(OwnRing("h"))
    .expect(Safety and not(f.open.get("g")) and not(f.open.get("h")) and f.farReset.get("g") and f.farReset.get("h")
            and not(f.eofDeliv.get("h")) and not(f.eofDeliv.get("g")))
  // serve crashes: the host owner's sockets close; the guest receives 0x3 for the open flow and aborts it with reset
  run HostOwnerDeathGivesGuestAnAbort = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(HostOwnerDies)
    .expect(Safety and f.dead.get("h") and not(f.open.get("h")) and f.farReset.get("h") and f.wire.get("g") == List("s3"))
    .then(KRecv("g")).then(OwnRing("g")).expect(Safety and not(f.open.get("g")) and f.farReset.get("g") and not(f.eofDeliv.get("g")))
    .then(HcLossObserved("g")).expect(Safety and f.loss.get("g") and not(f.open.get("g")))
  // the guest PID 1 crashes: no SHUTDOWN is sent; the VMM exits (RESET-EXIT) and the host's flow vsock ends with no
  // record: the host owner aborts it with reset
  run GuestPid1CrashEndsHostFlowWithNoClassificationRecord = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(GuestPid1Crash)
    .expect(Safety and f.dead.get("g") and f.wire.get("h") == List() and f.uWake.get("h"))
    .then(OwnUnannounced("h")).expect(Safety and not(f.open.get("h")) and f.farReset.get("h") and not(f.eofDeliv.get("h")))
    .then(HcLossObserved("h")).expect(Safety and f.loss.get("h"))
  // the control session drops while both owners live: each observes the loss and aborts the flow
  run SessionDropAbortsFlowAtBothOwners = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(HcSessionDrop)
    .then(HcLossObserved("g")).then(HcLossObserved("h"))
    .expect(Safety and not(f.open.get("g")) and not(f.open.get("h")) and f.farReset.get("g") and f.farReset.get("h")
            and not(f.eofDeliv.get("g")) and not(f.eofDeliv.get("h")))
  // an outer EOF before Paired is recorded and carried out after Paired (never turned into an abort)
  run EarlyOuterEofIsCarriedOutAfterPaired = InitWith("g")
    .then(FarWrite("g")).then(FarFin("g")).then(OwnSeesEof("g"))
    .expect(Safety and f.cls.get("g") == "clean" and f.open.get("g") and not(drainWanted("Vg")))
    .then(HcPair).then(HcRecvPaired).expect(Safety and drainWanted("Vg"))
    .then(KSend("Vg")).then(OwnGoal("Vg")).then(OwnArm("Vg")).then(OwnRead("Vg")).then(OwnShut("Vg"))
    .expect(Safety and f.shut.get("Vg") and f.wire.get("h") == List("s2") and not(f.aborted))
  // a ring-drop of the 0x3 record: the owner sees the end with no record and the lost counter, and aborts
  run LostAbortRecordStillEndsTheFlow = InitWith("g")
    .then(HcPair).then(HcRecvPaired).then(HostOwnerDies).then(EnvRingFull("g")).then(KRecv("g"))
    .expect(Safety and f.ring.get("g") == List() and f.lost.get("g") == 1 and f.uWake.get("g"))
    .then(OwnAuditTick("g")).then(OwnSeeLost("g")).expect(Safety and not(f.open.get("g")) and f.farReset.get("g"))

  // ------------------------------------------------------------------ witnesses (expected violated)
  val WitnessNoCleanEof: bool = SIDES.forall(y => not(f.eofDeliv.get(y)))
  val WitnessNoBothDirectionsClosed: bool = f.open.get("g") or f.open.get("h")
  val WitnessNoBackstopFire: bool = not(f.dr.get("Vg") == "waiting" and f.armShort.get("Vg"))
  val WitnessNoAbort: bool = not(f.aborted)
  val WitnessNoLost: bool = SIDES.forall(s => f.lost.get(s) == 0)
}

// every fault, one byte: the named runs and the bounded checks
module end_all { import flow_end(OFF = Set(), CFG = "all").* from "./flow_end" }
// the classifier skip only (A5, M-2), both directions active
module end_skip { import flow_end(OFF = Set(), CFG = "skip").* from "./flow_end" }
// no fault: exhaustive safety and the progress of the happy path
module end_clean { import flow_end(OFF = Set(), CFG = "clean").* from "./flow_end" }
// the kernel-side faults on the drain toward the vsock (guest side)
module end_dv { import flow_end(OFF = Set(), CFG = "dv").* from "./flow_end" }
// the kernel-side faults on the drain toward the outer leg (host side)
module end_do { import flow_end(OFF = Set(), CFG = "do").* from "./flow_end" }
// a full ring against owner death (A7 on the classification records)
module end_ring { import flow_end(OFF = Set(), CFG = "ring").* from "./flow_end" }
// the abort-side faults only
module end_abort { import flow_end(OFF = Set(), CFG = "abort").* from "./flow_end" }
// two bytes, fexit misses only
module end_two { import flow_end(OFF = Set(), CFG = "two").* from "./flow_end" }
