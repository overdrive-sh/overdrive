// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — the forwarder's pair lifecycle at its real lock and RCU boundaries
 * (EVMOD-SAFE), and the wakeups that make backpressure resume.
 *
 * Design artifact. Models the design text, not an implementation. NOT a conformance target: every rule here is
 * internal to the C module `ovd_fwd` (ADR-0204 § Consequences: "The rules are internal to the C module; no port or
 * wire format carries them"); its evidence is the DELIVER review against the source and V-EV-SAFE (A-45).
 *
 * Source of truth: docs/feature/netns-density-295/feature-delta.md § "[REF] vsock data path — the in-kernel
 * event-driven forwarder (EVMOD)": "Forwarding, per direction" (Trigger, Run, Why it is bounded), "Order of a pair's
 * end", "Stale invocations" rules 1-4 (EVMOD-SAFE, ADR-0204), "Locking on the callback paths", "Watch",
 * "A watch ended by the owner" (WATCH-ABORT), the EVMOD-END table's sweep row (ADR-0180), EVMOD-REC (ADR-0202).
 *
 * Why a sibling of fwd_pair: fwd_pair runs every callback, forwarder run and owner call as ONE atomic step (its
 * abstraction of § Forwarding), which is sound for what it checks (byte delivery, classification, records) but
 * cannot see a late callback, a freed pair or a lost wakeup: those live between the sub-steps. This module splits
 * the actors at the points where another actor can enter, for ONE pair (or one watch) and ONE forwarded direction.
 *
 * Abstraction (one pair, or one watch, of one context):
 *   - Halves. kind "pair": s = the vsock half (the source: V_h of a host pair, guest→host direction) and t = the TCP
 *     half (the target D, toward the far reader). kind "watch": s = the dialled V_h only. The other direction of a
 *     pair runs the same code; fwd_pair checks both.
 *   - Per half: `cb` the callback pointer the kernel reads at its call site ("ovr" the override | "orig" the saved
 *     original; the four kinds share one pointer here — the end restores them together, rule 2) and `ud` its
 *     sk_user_data ("P" the pair | "null").
 *   - A callback invocation on half x (`inv`) is four steps: the kernel reaches the call site (an event: Write = a
 *     vsock packet enqueued then sk_data_ready, PeerFin = a 0x2 then sk_state_change, TcpRx = TCP input calling an
 *     override of the target (its sk_data_ready, the other direction's trigger), SwReport = the orphan sweep's
 *     sk_error_report); Dispatch reads the pointer; Load =
 *     rcu_read_lock + smp_load_acquire(sk_user_data) (rule 1); Use (pair found: record which direction needs a run,
 *     queue the work item; a watch's override queues its record first) or Fallback (null: call the socket's current
 *     callback, READ_ONCE, rule 2), each ending with rcu_read_unlock. One invocation per half at a time: a second
 *     concurrent invocation on the same half is another RCU reader of the same shape and adds no new interleaving
 *     of the rules checked here.
 *   - lock_sock(s) (`lk`): the vsock receive path holds it from the enqueue through its callback (Write, PeerFin;
 *     virtio_transport_common.c:1661, :1705 @ v7.0), and every step of the forwarder's run that reads s takes it
 *     (its peek is a recvmsg, af_vsock.c:2518). The orphan sweep holds only vsock_table_lock (af_vsock.c:458, :471)
 *     and NO socket lock (drivers/vhost/vsock.c:725-728): its stores and its callback interleave with the run.
 *   - The sweep (drivers/vhost/vsock.c:746-750 @ v7.0, plain stores, no barrier) runs inside one hold of
 *     vsock_table_lock (af_vsock.c:458, :471; `tl`, derived from `sw`): SwState (SOCK_DONE, peer_shutdown =
 *     SHUTDOWN_MASK, sk_state) and SwErr (sk_err = ECONNRESET) in EITHER order (no barrier: a weakly ordered CPU may
 *     show the run any subset of the stores, EVMOD-SWEEP), then SwReport (sk_error_report: the err invocation), then
 *     SwUnlock (the walk leaves the lock once the invocation returned).
 *   - EVMOD-SWEEP (F-S1, DR2-CLOSE): a run that reads an end (end-of-stream or an error) from the vsock half whose
 *     class is not recorded takes vsock_table_lock (spin_lock_bh, nested inside lock_sock, the kernel's own order)
 *     before acting (`wrun` "lock"; WRecheck is enabled only when the sweep does not hold it), then reads the recorded
 *     class again and acts on it: the sweep's sk_error_report override recorded SocketError (`cls` "reset"). Only an
 *     end-of-stream still unrecorded after the re-check is Unclassified (the alarm; no kernel writer produces it).
 *   - The work item (`wq`): PENDING (`pend`), running (`wrun`) and disabled (`dis`, set by cancel_work_sync). queue_work
 *     sets PENDING unless it is set or the item is disabled (kernel/workqueue.c:2415, :2097); the worker clears PENDING
 *     BEFORE it calls the function (:3254 before :3288), so a queue_work during a run makes one more run; an item
 *     queued while running runs again after, never concurrently (:1212-1214). A run moves one unit per step (the
 *     peek → send → consume of fwd_pair, atomic here: the work item is the only consumer of s and the only producer
 *     of t); on a full target it stops (EAGAIN sets SOCK_NOSPACE, net/core/stream.c:175-181) and on an empty source
 *     it stops; it never polls.
 *   - Wakeups: a TCP target calls sk_write_space on freed space only while SOCK_NOSPACE is set
 *     (include/net/tcp.h:770-776, net/ipv4/tcp_input.c:6118-6122 @ v7.0), modelled by `nospace`; vsock calls
 *     sk_data_ready per enqueued packet (virtio_transport_common.c:1432-1433, af_vsock.c:1062-1068).
 *   - The write-space trigger (round 16): sk_write_space is NEVER overridden. The
 *     socket's own sk_write_space (kernel text) wakes the socket's wait queue with EPOLLOUT; the forwarder's entry on
 *     that queue (`wqe`, added at attach) runs its function only while the waker holds the queue head's lock wqlk(t)
 *     with interrupts disabled (kernel/sched/wait.c:104-108, :118-127 @ v7.0). Read (the reader frees target space)
 *     starts the wake `ws` "called" (sock_wfree calls sk_write_space with no socket lock, no RCU section
 *     and preemption enabled, net/core/sock.c:2675-2693 @ v7.0, so the wake may run while the pair ends);
 *     WsEnter takes wqlk(t) and enters the entry function if the entry is present ("entry"); WsLeave runs
 *     it (record the run, queue_work: a touch of the pair) and drops wqlk(t). The pair's end removes the entry
 *     (`wt`, remove_wait_queue under lock_sock(t), kernel/sched/wait.c:69-76) before rule 2's restore and clear of t
 *     and before the free work is queued; it takes wqlk(t), so it waits while a wake holds it. Only t's entry is
 *     modelled: the source's write-space wake belongs to the other direction, which runs the same code. lock_sock(t)
 *     excludes no modelled caller of t (none is a socket-lock owner of t), so it is not modelled.
 *   - A pair's end (`wprog`, run inside its own work item, § "Order of a pair's end"): mark ended; restore cb then
 *     clear ud on s, then remove t's wait-queue entry, restore cb and clear ud on t (rule 2; the abort actions, whose wakeups reach the restored originals, are fwd_pair's);
 *     remove the id and queue the record in ONE section under the record-queue lock (rule 4, EVMOD-REC; no record at
 *     the context's release); queue the free work with queue_rcu_work (rule 3) and touch nothing more.
 *   - The free (`fr`): the RCU grace period (RcuGp: every RCU reader inside at queue time has left), then the free
 *     work: cancel_work_sync = disable + drop PENDING (FreeCancel), wait for the running item (FreeFlush, re-enable on
 *     return — kernel/workqueue.c:4444-4466, :4495-4498), then kfree (FreeDone).
 *   - Control calls by id (rule 4): OVD_FWD_ABORT_ALL acts on each live id of the context exactly as OVD_FWD_ABORT
 *     does (§ Control interface), so with one pair or watch it is CtlAbort. OVD_FWD_ABORT (pair: mark the abort requested and queue_work; watch: remove the id
 *     and queue the record, then unhook in process context, WATCH-ABORT), OVD_FWD_PAIR_STATE (copy counters), each
 *     ONE step under the record-queue lock; the context's release (pair: request the end with no record; watch: remove
 *     the id, no record, unhook). A watch's end belongs to the path that removes its id.
 *   - Attach is not modelled: it holds lock_sock, sk_callback_lock and (vsock) vsock_table_lock (§ "Attach order"),
 *     which exclude every callback of the half while it installs (fwd_pair's F9); the pair starts attached.
 *   - Not modelled: the module's unload and the lifetime of its text (the appliance image has CONFIG_MODULE_UNLOAD off,
 *     ADR-0068 § 7; EVMOD-LIFE, ADR-0198), and the classes of the callers that reach an override: the model checks the
 *     pair's state, not the override's text.
 *
 * Ghost (model-only) state: uaf (some actor touched freed pair state: an override's use, a run's start, a control
 * call, a queue_work), recursed (an override called itself), unclassified (the
 * unclassified_eof alarm fired), delivered, nfree.
 *
 * ASSUMPTIONS (kernel behaviour, not modelled mechanisms; discharged in README.md):
 *   S1  RCU: a grace period that starts after an RCU reader entered ends after it left; a reader that is not waited
 *       for sees every store before call_rcu (queue_rcu_work, kernel/workqueue.c:2649-2664 @ v7.0) ..... source; V-EV-SAFE
 *   S2  smp_store_release / smp_load_acquire order the restore before the clear for an override that reads NULL;
 *       rcu_assign_pointer(p, NULL) is a plain WRITE_ONCE (include/linux/rcupdate.h:575-576 @ v7.0) ..... source; V-EV-SAFE
 *   S3  Workqueue: PENDING cleared before the function runs (kernel/workqueue.c:3254, :3288); non-reentrant
 *       (:1212-1214); cancel_work_sync disables the item, drops a pending run and waits for a running one, and a
 *       queue_work on a disabled item is dropped (:4444-4466, :2415); an enqueue after it returns is not covered
 *       (:4481-4484) ......................................................................... source; V-EV-SAFE
 *   S4  The vsock receive path runs the callbacks under lock_sock (virtio_transport_common.c:1661, :1705); the orphan
 *       sweep stores SOCK_DONE, peer_shutdown, sk_state, sk_err in that order without the socket lock and then calls
 *       sk_error_report (drivers/vhost/vsock.c:725-750); a recvmsg that reads peer_shutdown & SEND_SHUTDOWN before
 *       sk_err returns 0 (af_vsock.c:2326-2328, :2439-2442) .............................. source; V-EV-SAFE (finding F-S1)
 *   S5  A TCP target wakes sk_write_space after an EAGAIN send once space frees (net/core/stream.c:175-181,
 *       include/net/tcp.h:770-776, net/ipv4/tcp_input.c:6118-6122), and the socket's own sk_write_space wakes its wait
 *       queue with EPOLLOUT (net/core/stream.c:34-51, net/core/sock.c:3617-3660); a vsock enqueue calls sk_data_ready
 *       (virtio_transport_common.c:1432-1433, af_vsock.c:1062-1068) .............. V-EV-SAFE full-target resume cases
 *   S6  The wait-queue function runs only with the head's lock held and interrupts disabled, and remove_wait_queue
 *       takes that lock (kernel/sched/wait.c:69-76, :104-108, :118-127 @ v7.0); sock_wfree calls sk_write_space with
 *       no socket lock and no RCU section unless SOCK_USE_WRITE_QUEUE (net/core/sock.c:2675-2693 @ v7.0)
 *       .............................................................. source; V-EV-SAFE full-target resume cases
 *
 * OFF names the rules switched off (teeth):
 *   rcuAssignNull      (alternative (a)) sk_user_data cleared by rcu_assign_pointer(…, NULL) and read by
 *                      rcu_dereference: the clear may be seen before the restore, and the override's later read of
 *                      the callback pointer is not ordered after its load (rule 2 off)
 *   noGracePeriod      the free work is queued with queue_work, not queue_rcu_work (rule 3: no grace period)
 *   ctlQueueOutsideRcu (alternative (d)) a control call looks the id up under RCU, leaves the read section, then queues
 *   watchEndNoIdCheck  (rule 4, watch) the watch's override queues its record without checking the id under the lock
 *   noWriteSpace       the target's write-space wait-queue entry is not added (the wake reaches nothing of ours)
 *   removeWaitAfterFreeQueued (the write-space entry's removal order) the entry is removed by the free work after its
 *                      cancel_work_sync, i.e. after the free work was queued, not by the end before it
 *   noDataReady        sk_data_ready of the source is not overridden
 *   noErrorReport      sk_error_report of the source is not overridden (the sweep's error queues no run)
 *   lateClearPending   the run's "needs a run" flag is cleared when the run returns, not before it starts
 *   ctlAbortNoQueue    OVD_FWD_ABORT marks the abort requested without queueing the work item
 *   endSplitNoRequeue  the pair's end stops after unhooking and leaves the record and the free to a later run it
 *                      does not queue
 *   noSweepRecheck     (EVMOD-SWEEP, F-S1) the run acts on an end it read with no recorded class without the
 *                      vsock_table_lock re-check: an end-of-stream read in the sweep's window aborts Unclassified
 */
module fwd_safe {
  const OFF: Set[str]
  // "pair" | "watch" (safety) | "plive" | "wlive" (liveness) | "runs" (the named runs: both kinds)
  const CFG: str

  // ------------------------------------------------------------------ parameters
  val KINDS: Set[str] =
    if (CFG == "pair" or CFG == "plive") Set("pair") else if (CFG == "watch" or CFG == "wlive") Set("watch")
    else Set("pair", "watch")
  val C: int = 1     // the vsock credit of s (units queued at the source)
  val W: int = 1     // the TCP target's space (units)
  // units the vsock peer writes
  val NMAX: int = if (CFG == "runs") 3 else 2
  // faults and control calls per trace: SwState, CtlAbort, CtlState, CtxRel, CtlLookup
  val FMAX: int = if (CFG == "runs") 100 else if (CFG == "pair" or CFG == "watch") 2 else 1

  val BUG_RCU_NULL: bool = OFF.contains("rcuAssignNull")
  val BUG_NO_GP: bool = OFF.contains("noGracePeriod")
  val BUG_CTL_OUTSIDE: bool = OFF.contains("ctlQueueOutsideRcu")
  val BUG_W_NO_ID: bool = OFF.contains("watchEndNoIdCheck")
  val BUG_NO_WSPACE: bool = OFF.contains("noWriteSpace")
  val BUG_RW_LATE: bool = OFF.contains("removeWaitAfterFreeQueued")
  val BUG_NO_DREADY: bool = OFF.contains("noDataReady")
  val BUG_NO_EREPORT: bool = OFF.contains("noErrorReport")
  val BUG_LATE_CLEAR: bool = OFF.contains("lateClearPending")
  val BUG_CTL_NO_QUEUE: bool = OFF.contains("ctlAbortNoQueue")
  val BUG_END_SPLIT: bool = OFF.contains("endSplitNoRequeue")
  val BUG_NO_RECHECK: bool = OFF.contains("noSweepRecheck")

  // ------------------------------------------------------------------ types
  // ph: none | called (the kernel reached the call site) | ovr (in the override, before its load) | found (pair found,
  // inside rcu_read_lock) | nul (sk_user_data NULL, inside rcu_read_lock); ev: data | state | err | space;
  type Inv = { ph: str, ev: str }
  // pst: live | ending (marked ended; its end in progress) | ended (the free work queued) | freed
  // cause: none | Completed | HalfClose | SocketError | Unclassified | Detached | Release
  // rec: none | queued | read; fr: none | gp | queued | flush | flushed | done (kfree'd)
  // lk: none | rx (the vsock receive path holds lock_sock(s))
  // wrun: idle | top | lock (an unclassified end read, waiting for vsock_table_lock) | ret | end
  // cls: none | clean (the peer's 0x2, sk_state_change) | reset (sk_err, recorded by the sk_error_report override)
  // cq (ctlQueueOutsideRcu): none | held (a control call inside rcu_read_lock with the pointer) | out (left it, pointer kept)
  // sw: the orphan sweep's progress: none | stores (vsock_table_lock held, stores in progress) | report (its
  // sk_error_report running, lock held) | done (lock released); swPs: its peer_shutdown store is visible
  // rawEnd: what the run's read returned when it found an unclassified end: none | eof | err
  // wqe: t's write-space wait-queue entry is on the queue; ws: the write-space wake: none | called (the socket's own
  // sk_write_space) | entry (wqlk(t) held, in the entry function)
  type St = {
    kind: str, cb: str -> str, ud: str -> str, inv: str -> Inv, lk: str,
    pend: bool, dis: bool, wrun: str, wprog: List[str], oprog: List[str],
    pst: str, cause: str, cls: str, abortReq: bool, relReq: bool,
    id: bool, rec: str, nrec: int, fr: str, gpw: Set[str], nfree: int,
    sq: int, tq: int, wr: int, got: int, nospace: bool, fin: bool, peerFin: bool,
    ps: Set[str], err: bool, sw: str, swPs: bool, rawEnd: str, cq: str, faults: int, needUnhook: bool,
    wqe: bool, ws: str,
    uaf: bool, recursed: bool, unclassified: bool, delivered: int,
  }

  var st: St

  // ------------------------------------------------------------------ helpers
  val HALVES: Set[str] = Set("s", "t")
  def halves(t: St): Set[str] = if (t.kind == "pair") Set("s", "t") else Set("s")
  val inv0: Inv = { ph: "none", ev: "none" }
  def canFault(t: St): bool = t.faults < FMAX
  def fault(t: St): St = { ...t, faults: t.faults + 1 }
  // an actor touches the pair's (watch's) state
  def touch(t: St): St = if (t.pst == "freed") { ...t, uaf: true } else t
  // queue_work on the pair's work item (the work_struct is inside the freed state)
  def queueWork(t: St): St =
    if (t.pst == "freed") { ...t, uaf: true }
    else if (t.dis or t.pend) t
    else { ...t, pend: true }
  // the readers inside rcu_read_lock now (a grace period queued now waits for them)
  def readers(t: St): Set[str] =
    HALVES.filter(x => t.inv.get(x).ph.in(Set("found", "nul")))
      .union(if (t.cq == "held") Set("c") else Set())
  // an invocation on half x returns (rcu_read_unlock; the receive path then releases lock_sock(s))
  def endInv(t: St, x: str): St =
    { ...t, inv: t.inv.set(x, inv0), gpw: t.gpw.exclude(Set(x)), lk: if (x == "s" and t.lk == "rx") "none" else t.lk }
  def startInv(t: St, x: str, ev: str): St = { ...t, inv: t.inv.set(x, { ph: "called", ev: ev }) }
  // the pair's end, in its work item (§ "Order of a pair's end"); rcuAssignNull: each clear may be seen before its
  // restore
  // "wt": the removal of t's write-space wait-queue entry, before t's restore and clear and before the free work
  // is queued (removeWaitAfterFreeQueued: not here, the free work removes it)
  val WT: List[str] = if (BUG_RW_LATE) List() else ["wt"]
  val END_PAIR: List[str] =
    if (BUG_RCU_NULL) ["us", "rs"].concat(WT).concat(["ut", "rt", "idrec", "free"])
    else ["rs", "us"].concat(WT).concat(["rt", "ut", "idrec", "free"])
  // a watch's unhook, by the path that removed its id (or its work item, when that path was the override)
  val UNHOOK_W: List[str] = if (BUG_RCU_NULL) ["us", "rs", "free"] else ["rs", "us", "free"]
  // one step of an end program (it reads the saved originals from the pair's state: a touch)
  // "wt" takes wqlk(t): it waits while a wake holds it (remove_wait_queue)
  def lockOk(t: St, step: str): bool = step != "wt" or t.ws != "entry"
  def doStep(t0: St, step: str, rest: List[str]): St =
    val t = touch(t0)
    if (step == "wt") { ...t, wqe: false }
    else if (step == "rs") { ...t, cb: t.cb.set("s", "orig") }
    else if (step == "us") { ...t, ud: t.ud.set("s", "null") }
    else if (step == "rt") { ...t, cb: t.cb.set("t", "orig") }
    else if (step == "ut") { ...t, ud: t.ud.set("t", "null") }
    else if (step == "idrec")
      // one section under the record-queue lock: remove the id and queue the one record (none at release)
      if (t.relReq) { ...t, id: false }
      else { ...t, id: false, rec: "queued", nrec: t.nrec + 1 }
    // "free": queue_rcu_work (noGracePeriod: queue_work, which runs at once); a second free of the same state is
    // counted (OneFree)
    else if (t.fr != "none") { ...t, nfree: t.nfree + 1 }
    else { ...t, pst: "ended", nfree: t.nfree + 1, fr: if (BUG_NO_GP) "queued" else "gp", gpw: readers(t) }

  // ------------------------------------------------------------------ init
  action InitWith(k: str): bool = all {
    KINDS.contains(k),
    st' = {
      kind: k,
      cb: HALVES.mapBy(x => if (k == "pair" or x == "s") "ovr" else "none"),
      ud: HALVES.mapBy(x => if (k == "pair" or x == "s") "P" else "none"),
      inv: HALVES.mapBy(x => inv0), lk: "none",
      pend: false, dis: false, wrun: "idle", wprog: List(), oprog: List(),
      pst: "live", cause: "none", cls: "none", abortReq: false, relReq: false,
      id: true, rec: "none", nrec: 0, fr: "none", gpw: Set(), nfree: 0,
      sq: 0, tq: 0, wr: 0, got: 0, nospace: false, fin: false, peerFin: false,
      ps: Set(), err: false, sw: "none", swPs: false, rawEnd: "none", cq: "none", faults: 0, needUnhook: false,
      // the pair's target carries its write-space entry (attach adds it; noWriteSpace: not added)
      wqe: k == "pair" and not(BUG_NO_WSPACE), ws: "none",
      uaf: false, recursed: false, unclassified: false, delivered: 0,
    },
  }
  action init = { nondet k = KINDS.oneOf() InitWith(k) }

  // ------------------------------------------------------------------ environment: events at the call sites
  // the vsock peer's packet: enqueued under lock_sock(s), then sk_data_ready (S4, S5)
  def canWrite(t: St): bool = and {
    t.kind == "pair", t.sw == "none", not(t.peerFin), t.wr < NMAX, t.sq < C, t.inv.get("s").ph == "none",
    t.lk == "none",
  }
  action Write: bool = all {
    canWrite(st),
    st' = startInv({ ...st, sq: st.sq + 1, wr: st.wr + 1, lk: "rx" }, "s", "data"),
  }
  // the vsock peer's 0x2: peer_shutdown updated under lock_sock(s), then sk_state_change (fwd_pair's F2)
  action PeerFin: bool = all {
    st.sw == "none", not(st.peerFin), st.inv.get("s").ph == "none", st.lk == "none",
    st' = startInv({ ...st, ps: st.ps.union(Set("S")), peerFin: true, lk: "rx" }, "s", "state"),
  }
  // the far reader reads from the target; freed space after an EAGAIN send wakes sk_write_space (S5, S6); the socket's
  // own sk_write_space wakes the wait queue
  def canRead(t: St): bool =
    t.kind == "pair" and t.tq > 0 and (not(t.nospace) or (t.inv.get("t").ph == "none" and t.ws == "none"))
  action Read: bool = all {
    canRead(st),
    val t1 = { ...st, tq: st.tq - 1, got: st.got + 1, nospace: false }
    st' = if (not(st.nospace)) t1 else { ...t1, ws: "called" },
  }
  // the wake takes wqlk(t) and enters the entry's function if the entry is on the queue (else it wakes nothing of ours)
  def canWsEnter(t: St): bool = t.ws == "called"
  action WsEnter: bool = all {
    canWsEnter(st),
    st' = if (st.wqe) { ...st, ws: "entry" } else { ...st, ws: "none" },
  }
  // the entry's function: the pair is reached through the entry itself (a touch); record the run, queue_work; then
  // wake_up drops wqlk(t)
  def canWsLeave(t: St): bool = t.ws == "entry"
  action WsLeave: bool = all { canWsLeave(st), st' = { ...queueWork(touch(st)), ws: "none" } }
  // TCP input calls an override of the target (its sk_data_ready: the other direction's trigger; an environment
  // event, bounded by FMAX)
  action TcpRx: bool = all {
    st.kind == "pair", canFault(st), st.inv.get("t").ph == "none",
    st' = startInv(fault(st), "t", "rx"),
  }
  // the orphan sweep (S4): no socket lock, one hold of vsock_table_lock; the first store finds the lock taken by the
  // walk (held from before the socket's first store); the two visible stores in either order (EVMOD-SWEEP)
  def tl(t: St): bool = t.sw.in(Set("stores", "report"))
  def sweepStart(t: St): St = if (t.sw == "none") fault({ ...t, sw: "stores" }) else t
  // the sweep's continuation (fair once started) and its start (not fair)
  def canSwStateCont(t: St): bool = t.sw == "stores" and not(t.swPs)
  def canSwState(t: St): bool = (t.sw == "none" and canFault(t)) or canSwStateCont(t)
  action SwState: bool = all {
    canSwState(st),
    st' = { ...sweepStart(st), swPs: true, ps: Set("S", "R") },
  }
  def canSwErrCont(t: St): bool = t.sw == "stores" and not(t.err)
  def canSwErr(t: St): bool = (t.sw == "none" and canFault(t)) or canSwErrCont(t)
  action SwErr: bool = all { canSwErr(st), st' = { ...sweepStart(st), err: true } }
  def canSwReport(t: St): bool = t.sw == "stores" and t.swPs and t.err and t.inv.get("s").ph == "none"
  action SwReport: bool = all { canSwReport(st), st' = startInv({ ...st, sw: "report" }, "s", "err") }
  // the walk moves on and leaves vsock_table_lock once sk_error_report returned
  def canSwUnlock(t: St): bool = t.sw == "report" and t.inv.get("s").ph == "none"
  action SwUnlock: bool = all { canSwUnlock(st), st' = { ...st, sw: "done" } }
  // ------------------------------------------------------------------ the callback invocation (rules 1, 2)
  // the kernel reads the socket's callback pointer (noWriteSpace / noDataReady: that kind is not overridden; a watch
  // overrides only sk_state_change and sk_error_report; sk_write_space is never overridden)
  def overridden(t: St, x: str, ev: str): bool = and {
    ev != "space", not(BUG_NO_DREADY and ev == "data"),
    not(BUG_NO_EREPORT and ev == "err"),
    t.kind == "pair" or ev.in(Set("state", "err")),
  }
  def canDispatch(t: St, x: str): bool = t.inv.get(x).ph == "called"
  action Dispatch(x: str): bool = all {
    canDispatch(st, x),
    val i = st.inv.get(x)
    st' = if (st.cb.get(x) == "ovr" and overridden(st, x, i.ev)) { ...st, inv: st.inv.set(x, { ...i, ph: "ovr" }) }
          else endInv({ ...st, delivered: st.delivered + 1 }, x),
  }
  // rcu_read_lock; smp_load_acquire(&sk->sk_user_data) (rule 1)
  def canLoad(t: St, x: str): bool = t.inv.get(x).ph == "ovr"
  action Load(x: str): bool = all {
    canLoad(st, x),
    val i = st.inv.get(x)
    st' = { ...st, inv: st.inv.set(x, { ...i, ph: if (st.ud.get(x) == "P") "found" else "nul" }) },
  }
  // the pair found: process the event (a spurious run is harmless) and queue the work item; the originals of
  // sk_state_change / sk_error_report / sk_write_space are called first (§ Forwarding, Trigger). A watch's override
  // queues the watch's record under the record-queue lock, if it still finds the id, and then calls the original.
  def watchEnd(t: St, cause: str): St =
    if (t.id or BUG_W_NO_ID)
      { ...t, id: false, rec: "queued", nrec: t.nrec + 1, cause: cause, pst: if (t.pst == "live") "ending" else t.pst,
        needUnhook: true }
    else t
  def canUse(t: St, x: str): bool = t.inv.get(x).ph == "found"
  action Use(x: str): bool = all {
    canUse(st, x),
    val ev = st.inv.get(x).ev
    val t0 = touch(st)
    val t1 = if (ev.in(Set("data", "rx"))) t0 else { ...t0, delivered: t0.delivered + 1 }
    val t2 =
      if (t1.kind == "watch")
        val w = watchEnd(t1, if (ev == "err") "SocketError" else "HalfClose")
        if (w.nrec > t1.nrec) queueWork(w) else w
      else
        // EVMOD-END: a newly set SEND_SHUTDOWN without RCV_SHUTDOWN is a clean half-close; sk_err reported by
        // sk_error_report records SocketError (error precedence)
        val c = if (ev == "err") "reset"
                else if (ev == "state" and t1.cls == "none" and t1.ps.contains("S") and not(t1.ps.contains("R"))) "clean"
                else t1.cls
        queueWork({ ...t1, cls: c })
    st' = endInv(t2, x),
  }
  // sk_user_data NULL: call the socket's current callback of the same kind (READ_ONCE): the original once rule 2
  // held (rcuAssignNull: the read is not ordered after the load and may return the override already called)
  def canFallback(t: St, x: str): bool = t.inv.get(x).ph == "nul"
  action Fallback(x: str): bool = all {
    canFallback(st, x),
    val cur = if (BUG_RCU_NULL) "ovr" else st.cb.get(x)
    val t1 =
      if (cur == "ovr") { ...st, recursed: true }
      else { ...st, delivered: st.delivered + 1 }
    st' = endInv(t1, x),
  }

  // ------------------------------------------------------------------ the work item (Run, Order of a pair's end)
  def canWStart(t: St): bool = t.pend and not(t.dis) and t.wrun == "idle"
  action WStart: bool = all {
    canWStart(st),
    // the worker clears PENDING before it calls the function (S3; lateClearPending: when the run returns)
    st' = touch({ ...st, pend: BUG_LATE_CLEAR, wrun: "top" }),
  }
  // what the run finds at the top of the pair's function, reading s under lock_sock(s): an end to carry out, else a
  // unit to move. The run reads sk_err, then (recvmsg) the queue and peer_shutdown. An end it reads from the half
  // whose class is not recorded (an error, or an end-of-stream) is "Recheck" (EVMOD-SWEEP): the run takes
  // vsock_table_lock before acting (noSweepRecheck: it acts on the read, and an unclassified end-of-stream is the
  // unclassified_eof alarm).
  def readEnd(t: St): str = if (t.err) "err" else if (t.sq == 0 and t.ps.contains("S")) "eof" else "none"
  def endCause(t: St): str =
    if (t.relReq) "Release"
    else if (t.abortReq) "Detached"
    else if (t.cls == "reset") "SocketError"
    else if (readEnd(t) == "err" and t.cls == "clean") "SocketError"
    else if (readEnd(t) == "eof" and t.cls == "clean") "Completed"
    else if (readEnd(t) != "none" and t.cls == "none")
      (if (not(BUG_NO_RECHECK)) "Recheck" else if (readEnd(t) == "err") "SocketError" else "Unclassified")
    else "none"
  // the class recorded once the run holds vsock_table_lock, for the end it read: the sweep's override recorded
  // SocketError; an end-of-stream still unrecorded is the alarm
  def recheckCause(t: St): str =
    if (t.cls == "reset") "SocketError"
    else if (t.rawEnd == "eof" and t.cls == "clean") "Completed"
    else if (t.rawEnd == "err") "SocketError"
    else "Unclassified"
  def atTop(t: St): bool = t.wrun == "top" and t.lk == "none"
  // the pair has ended (or the watch has no unhook to do): the run does nothing
  def canWNoop(t: St): bool = atTop(t) and (if (t.kind == "pair") t.pst != "live" else not(t.needUnhook))
  action WNoop: bool = all { canWNoop(st), st' = { ...st, wrun: "ret" } }
  // a clean end is forwarded as a FIN (SHUT_WR) on the target after its last unit (ADR-0183)
  def endWith(t: St, c: str): St =
    { ...t, pst: "ending", cause: c, fin: c == "Completed", unclassified: t.unclassified or c == "Unclassified",
      wprog: if (BUG_END_SPLIT) ["rs", "us", "rt", "ut"] else END_PAIR, wrun: "end" }
  def canWEnd(t: St): bool = atTop(t) and t.kind == "pair" and t.pst == "live" and endCause(t) != "none"
  action WEnd: bool = all {
    canWEnd(st),
    val c = endCause(st)
    st' = if (c == "Recheck") { ...st, wrun: "lock", rawEnd: readEnd(st) } else endWith(st, c),
  }
  // EVMOD-SWEEP: spin_lock_bh(vsock_table_lock), taken only when the sweep does not hold it, then the recorded class
  // read again; the run acts on it and drops the lock in the same step
  def canWRecheck(t: St): bool = t.wrun == "lock" and not(tl(t))
  action WRecheck: bool = all { canWRecheck(st), st' = endWith(touch(st), recheckCause(st)) }
  def canMoveCore(t: St): bool = atTop(t) and t.kind == "pair" and t.pst == "live" and endCause(t) == "none"
  def canWMove(t: St): bool = canMoveCore(t) and t.sq > 0 and t.tq < W
  action WMove: bool = all { canWMove(st), st' = { ...st, sq: st.sq - 1, tq: st.tq + 1 } }
  // the target is full: the send returns EAGAIN (SOCK_NOSPACE set) and the run stops
  def canWBlock(t: St): bool = canMoveCore(t) and t.sq > 0 and t.tq == W
  action WBlock: bool = all { canWBlock(st), st' = { ...st, nospace: true, wrun: "ret" } }
  // the source is empty: the run stops
  def canWDrained(t: St): bool = canMoveCore(t) and t.sq == 0
  action WDrained: bool = all { canWDrained(st), st' = { ...st, wrun: "ret" } }
  // a watch's work item, queued by the override that ended it, removes its overrides and queues its free
  def canWUnhook(t: St): bool = atTop(t) and t.kind == "watch" and t.needUnhook
  action WUnhook: bool = all {
    canWUnhook(st),
    st' = { ...st, needUnhook: false, wprog: UNHOOK_W, wrun: "end" },
  }
  // the function returns (lateClearPending: the flag is cleared now)
  def canWRet(t: St): bool = t.wrun == "ret"
  action WRet: bool = all { canWRet(st), st' = { ...st, wrun: "idle", pend: if (BUG_LATE_CLEAR) false else st.pend } }
  // one step of the end; after the last (the free work queued) the run touches nothing more and returns
  def canWStep(t: St): bool = t.wrun == "end" and t.wprog.length() > 0 and lockOk(t, t.wprog.head())
  action WStep: bool = all {
    canWStep(st),
    val t1 = doStep(st, st.wprog.head(), st.wprog.tail())
    st' = { ...t1, wprog: st.wprog.tail(), wrun: if (st.wprog.length() == 1) "idle" else "end" },
  }
  // the owner's process-context steps of a watch's end (abort, release)
  def canOStep(t: St): bool = t.oprog.length() > 0 and lockOk(t, t.oprog.head())
  action OStep: bool = all {
    canOStep(st),
    st' = { ...doStep(st, st.oprog.head(), st.oprog.tail()), oprog: st.oprog.tail() },
  }

  // ------------------------------------------------------------------ the free (rule 3)
  def canRcuGp(t: St): bool = t.fr == "gp" and t.gpw == Set()
  action RcuGp: bool = all { canRcuGp(st), st' = { ...st, fr: "queued" } }
  // cancel_work_sync: disable, drop a pending run (S3)
  def canFreeCancel(t: St): bool = t.fr == "queued"
  action FreeCancel: bool = all { canFreeCancel(st), st' = { ...st, dis: true, pend: false, fr: "flush" } }
  // ... wait for the running item, re-enable, return
  def canFreeFlush(t: St): bool = t.fr == "flush" and t.wrun == "idle"
  action FreeFlush: bool = all { canFreeFlush(st), st' = { ...st, dis: false, fr: "flushed" } }
  // removeWaitAfterFreeQueued: the free work removes t's entry after its cancel_work_sync returned, then frees
  def canFreeRemoveWait(t: St): bool = BUG_RW_LATE and t.fr == "flushed" and t.wqe and t.ws != "entry"
  action FreeRemoveWait: bool = all { canFreeRemoveWait(st), st' = { ...touch(st), wqe: false } }
  def canFreeDone(t: St): bool =
    t.fr == "flushed" and not(BUG_RW_LATE and t.wqe)
  action FreeDone: bool = all { canFreeDone(st), st' = { ...st, pst: "freed", fr: "done" } }
  // ------------------------------------------------------------------ control calls by id (rule 4) and the owner
  // OVD_FWD_ABORT, in one section under the record-queue lock: an id not live → ENOENT. Pair: the abort requested and
  // the work item queued (ctlAbortNoQueue: not queued). Watch (WATCH-ABORT): the id removed and the record queued, then
  // the unhook in process context.
  action CtlAbort: bool = all {
    canFault(st), not(BUG_CTL_OUTSIDE and st.kind == "pair"),
    val t0 = fault(st)
    st' =
      if (not(st.id)) t0
      else if (st.kind == "pair")
        val t1 = touch({ ...t0, abortReq: true })
        if (BUG_CTL_NO_QUEUE) t1 else queueWork(t1)
      else if (st.oprog.length() > 0) t0
      else
        { ...t0, id: false, rec: "queued", nrec: st.nrec + 1, pst: "ending", cause: "Detached", oprog: UNHOOK_W },
  }
  // OVD_FWD_PAIR_STATE: the counters copied under the lock
  action CtlState: bool = all {
    canFault(st),
    st' = if (st.id) touch(fault(st)) else fault(st),
  }
  // alternative (d) (ctlQueueOutsideRcu): the lookup under rcu_read_lock, then the queue after rcu_read_unlock
  action CtlLookup: bool = all {
    BUG_CTL_OUTSIDE, st.kind == "pair", st.cq == "none", canFault(st), st.id,
    st' = fault({ ...st, cq: "held" }),
  }
  def canCtlUnlock(t: St): bool = t.cq == "held"
  action CtlUnlock: bool = all {
    canCtlUnlock(st),
    st' = { ...st, cq: "out", gpw: st.gpw.exclude(Set("c")) },
  }
  def canCtlQueue(t: St): bool = t.cq == "out"
  action CtlQueue: bool = all {
    canCtlQueue(st),
    st' = { ...queueWork(touch({ ...st, abortReq: true })), cq: "none" },
  }
  // the context's release (the owner's death): pair — the end requested with no record; watch — the id removed, no
  // record, the unhook in process context
  action CtxRel: bool = all {
    not(st.relReq), canFault(st),
    val t0 = fault({ ...st, relReq: true })
    st' =
      if (not(st.id)) t0
      else if (st.kind == "pair") queueWork(touch(t0))
      else if (st.oprog.length() > 0) t0
      else { ...t0, id: false, pst: "ending", cause: "Release", oprog: UNHOOK_W },
  }
  // the owner reads the end record (a released context's record is never read)
  def canTakeEnd(t: St): bool = t.rec == "queued" and not(t.relReq)
  action TakeEnd: bool = all { canTakeEnd(st), st' = { ...st, rec: "read" } }

  // ------------------------------------------------------------------ next
  action step = {
    nondet x = HALVES.oneOf()
    any {
      Read, WsEnter, WsLeave, TcpRx,
      Write, PeerFin, SwState, SwErr, SwReport, SwUnlock,
      Dispatch(x), Load(x), Use(x), Fallback(x),
      WStart, WNoop, WEnd, WRecheck, WMove, WBlock, WDrained, WUnhook, WRet, WStep, OStep,
      RcuGp, FreeCancel, FreeFlush, FreeRemoveWait, FreeDone,
      CtlAbort, CtlState, CtlLookup, CtlUnlock, CtlQueue, CtxRel, TakeEnd,
    }
  }

  // ------------------------------------------------------------------ invariants (EVMOD-SAFE, EVMOD-REC, EVMOD-END)
  // rules 1, 3, 4: no override, run, queue_work or control call touches freed pair state
  val NoTouchAfterFree: bool = not(st.uaf)
  // rule 2: an override never calls itself
  val NoOverrideCallsItself: bool = not(st.recursed)
  // EVMOD-REC: at most one end record per id, and the id leaves the table only with its record (none at release)
  val OneEndRecord: bool = st.nrec <= 1
  val RecordAtIdRemoval: bool = not(st.id) implies (st.rec != "none" or st.relReq)
  // rule 3: one free, and the state is freed only after its work item completed (nothing pending or running) and no
  // RCU reader still holds it
  val OneFree: bool = st.nfree <= 1
  val FreeAfterWorkCompletes: bool = st.pst == "freed" implies (st.wrun == "idle" and not(st.pend))
  val FreeAfterReaders: bool =
    st.pst == "freed" implies (HALVES.forall(x => st.inv.get(x).ph != "found") and st.cq == "none")
  // rule 4: no live id names freed (or freeing) state
  val NoIdNamesFreedState: bool = st.id implies st.fr.in(Set("none", "gp"))
  // EVMOD-END: an end is clean only after the peer's 0x2 (the sweep's state never reads as a clean end)
  val CleanEndOnlyAfterPeerFin: bool = st.cause.in(Set("Completed", "HalfClose")) implies st.peerFin
  // EVMOD-END / EVMOD-SWEEP: the unclassified_eof alarm never fires, including for a run that reads the half inside
  // the orphan sweep's stores (F-S1, resolved by the vsock_table_lock re-check; hazard noSweepRecheck)
  val NoUnclassifiedEnd: bool = not(st.unclassified)
  val Safety: bool = and {
    NoTouchAfterFree, NoOverrideCallsItself, OneEndRecord, RecordAtIdRemoval, OneFree,
    FreeAfterWorkCompletes, FreeAfterReaders, NoIdNamesFreedState, CleanEndOnlyAfterPeerFin, NoUnclassifiedEnd,
  }

  // ------------------------------------------------------------------ liveness (TLC)
  // Fairness of the system: every callback step, every work-item and free step, the sweep once started, a control
  // call once it holds a pointer, the owner's steps and record read, and the far reader (it reads what it is given).
  // NOT the vsock peer's writes and FIN, the sweep's start, the control calls' start or the release. Every action is
  // taken only finitely often on the liveness instances (bounded units, events, faults), so every behaviour ends in
  // stuttering and weak fairness is "eventually, always, none of them is enabled" (as in fwd_pair).
  val sysEnabled: bool = or {
    HALVES.exists(x => canDispatch(st, x) or canLoad(st, x) or canUse(st, x) or canFallback(st, x)),
    canWStart(st), canWNoop(st), canWEnd(st), canWRecheck(st), canWMove(st), canWBlock(st), canWDrained(st), canWUnhook(st),
    canWRet(st), canWStep(st), canOStep(st),
    canRcuGp(st), canFreeCancel(st), canFreeFlush(st), canFreeRemoveWait(st), canFreeDone(st),
    canWsEnter(st), canWsLeave(st),
    canSwStateCont(st), canSwErrCont(st), canSwReport(st), canSwUnlock(st), canCtlUnlock(st), canCtlQueue(st), canTakeEnd(st), canRead(st),
  }
  temporal fair = eventually(always(not(sysEnabled)))
  // the flow was cut by an abort (its queued units are discarded with it)
  val cut: bool = st.cause.in(Set("SocketError", "Unclassified", "Detached", "Release"))
  val inflight: int = st.sq + st.tq
  // Forwarding / ADR-0183: every unit the peer wrote is eventually read by the far reader, unless the flow is cut
  temporal EveryUnitDelivered = fair.implies(always((inflight > 0).implies(eventually(inflight == 0 or cut))))
  // Forwarding, "Why it is bounded": a pair stalled on a full target resumes once the reader reads (sk_write_space),
  // (its wait-queue entry), and a pair whose source received data runs (sk_data_ready) — both sides never wait for ever
  temporal StalledPairResumes = fair.implies(always((st.sq > 0 and st.tq == W).implies(eventually(st.sq == 0 or cut))))
  // EVMOD-REC: an end eventually produces its record (none at release), and the requested abort takes effect
  temporal EndProducesRecord = fair.implies(always((st.pst != "live").implies(eventually(st.rec != "none" or st.relReq))))
  // EVMOD-REC / EVMOD-END: an end that reaches the socket — the peer's 0x2 (sk_state_change) or the orphan sweep
  // (sk_error_report) — eventually yields the pair's (watch's) end record, unless the context was released
  temporal SocketEndYieldsRecord =
    fair.implies(always((st.peerFin or st.sw.in(Set("report", "done"))).implies(eventually(st.rec != "none" or st.relReq))))
  temporal RequestedAbortEnds = fair.implies(always(st.abortReq.implies(eventually(st.pst != "live"))))
  // EVMOD-SAFE rule 3: an ended pair (watch) is eventually freed
  temporal EndedStateFreed = fair.implies(always((st.pst != "live").implies(eventually(st.pst == "freed"))))

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // happy path with backpressure: the peer's unit kicks the run (sk_data_ready); a second unit finds the target full
  // (EAGAIN, SOCK_NOSPACE) and the run stops; the reader's read wakes sk_write_space, which wakes the wait queue and
  // runs the forwarder's entry; the pair resumes
  run TargetStallResumesOnWriteSpace = InitWith("pair")
    .then(Write).then(Dispatch("s")).then(Load("s")).then(Use("s")).expect(Safety and st.pend and st.lk == "none")
    .then(WStart).then(WMove).then(WDrained).then(WRet).expect(Safety and st.tq == 1 and st.sq == 0)
    .then(Write).then(Dispatch("s")).then(Load("s")).then(Use("s"))
    .then(WStart).then(WBlock).then(WRet).expect(Safety and st.nospace and st.sq == 1 and st.tq == 1 and not(st.pend))
    .then(Read).expect(st.ws == "called" and st.inv.get("t").ph == "none")
    .then(WsEnter).expect(st.ws == "entry").then(WsLeave).expect(Safety and st.pend and st.ws == "none")
    .then(WStart).then(WMove).then(WDrained).then(WRet).then(Read)
    .expect(Safety and st.got == 2 and inflight == 0)
  // the reader's wakeup lands while the run that found the target full is still running: PENDING was cleared before
  // the run, so the wakeup queues one more run and nothing is lost
  run WakeupDuringRunIsNotLost = InitWith("pair")
    .then(Write).then(Dispatch("s")).then(Load("s")).then(Use("s")).then(WStart).then(WMove).then(WDrained).then(WRet)
    .then(Write).then(Dispatch("s")).then(Load("s")).then(Use("s")).then(WStart).then(WBlock)
    .then(Read).then(WsEnter).then(WsLeave).expect(Safety and st.pend and st.wrun == "ret")
    .then(WRet).then(WStart).then(WMove).expect(Safety and st.sq == 0 and st.tq == 1)
  // a clean end: the peer's 0x2 is classified in sk_state_change; the run forwards the FIN, ends the pair (restore,
  // clear, id + record, free work), the grace period passes, the free cancels and frees; the owner reads one record
  run CleanEndProducesOneRecordAndFrees = InitWith("pair")
    .then(PeerFin).then(Dispatch("s")).then(Load("s")).then(Use("s")).expect(Safety and st.cls == "clean" and st.pend)
    .then(WStart).then(WEnd).expect(Safety and st.cause == "Completed" and st.fin)
    .then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep)
    .expect(Safety and st.pst == "ended" and not(st.id) and st.rec == "queued" and st.fr == "gp" and st.wrun == "idle")
    .then(RcuGp).then(FreeCancel).then(FreeFlush).then(FreeDone).then(TakeEnd)
    .expect(Safety and st.pst == "freed" and st.nrec == 1 and st.rec == "read" and not(st.uaf))
  // a TCP input callback entered the target's override as the pair's end restores and clears:
  // it reads NULL and calls the restored original, never itself (rule 2)
  run LateCallbackAfterEndCallsOriginal = InitWith("pair")
    .then(Write).then(Dispatch("s")).then(Load("s")).then(Use("s")).then(WStart).then(WMove).then(WDrained).then(WRet)
    .then(Write).then(Dispatch("s")).then(Load("s")).then(Use("s")).then(WStart).then(WBlock).then(WRet)
    .then(CtlAbort).then(TcpRx).then(Dispatch("t")).expect(st.inv.get("t").ph == "ovr")
    .then(WStart).then(WEnd).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).expect(st.ud.get("t") == "null")
    .then(Load("t")).then(Fallback("t"))
    .expect(Safety and not(st.recursed) and st.cause == "Detached")
  // an override found the pair before the clear and still holds it when the end queues the free: the grace period
  // waits for it; its queue_work lands before the free work, which cancels it; nothing touches freed state (rule 3)
  run CallbackHoldingPairDelaysFree = InitWith("pair")
    .then(Write).then(Dispatch("s")).then(Load("s")).then(Use("s")).then(WStart).then(WMove).then(WDrained).then(WRet)
    .then(Write).then(Dispatch("s")).then(Load("s")).then(Use("s")).then(WStart).then(WBlock).then(WRet)
    .then(CtlAbort).then(TcpRx).then(Dispatch("t")).then(Load("t")).expect(st.inv.get("t").ph == "found")
    .then(WStart).then(WEnd).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep)
    .expect(Safety and st.fr == "gp" and st.gpw == Set("t") and not(canRcuGp(st)))
    .then(Use("t")).expect(Safety and st.pend)
    .then(RcuGp).then(FreeCancel).expect(not(st.pend))
    .then(FreeFlush).then(FreeDone).expect(Safety and st.pst == "freed" and not(st.uaf))
  // OVD_FWD_PAIR_STATE on an id whose end has left the record-queue section: ENOENT, the pair untouched (rule 4)
  run ControlCallAfterIdRemovalIsEnoent = InitWith("pair")
    .then(CtlAbort).then(WStart).then(WEnd).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep)
    .expect(not(st.id) and st.rec == "queued")
    .then(CtlState).then(WStep).then(RcuGp).then(FreeCancel).then(FreeFlush).then(FreeDone).then(CtlState)
    .expect(Safety and st.nrec == 1 and not(st.uaf))
  // OVD_FWD_ABORT lands while the pair's own clean end runs, before its id leaves the table: it queues the work item,
  // which the free's cancel_work_sync drops; one record (Completed)
  run AbortRacingEndIsCancelledByFree = InitWith("pair")
    .then(PeerFin).then(Dispatch("s")).then(Load("s")).then(Use("s")).then(WStart).then(WEnd)
    .then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(CtlAbort).expect(st.pend and st.abortReq)
    .then(WStep).then(WStep).then(RcuGp).then(FreeCancel).then(FreeFlush).then(FreeDone)
    .expect(Safety and st.cause == "Completed" and st.nrec == 1 and not(st.uaf))
  // the orphan sweep's three steps: its sk_error_report finds the pair and the run aborts it SocketError
  run SweepReportedEndAbortsSocketError = InitWith("pair")
    .then(SwState).then(SwErr).then(SwReport).then(Dispatch("s")).then(Load("s")).then(Use("s")).then(SwUnlock)
    .then(WStart).then(WEnd).expect(Safety and st.cause == "SocketError" and not(st.unclassified))
  // EVMOD-SWEEP (F-S1): a run already queued reads V_h between the sweep's peer_shutdown store and its sk_err store
  // (no socket lock): recvmsg returns end-of-stream with no class recorded, so the run waits for vsock_table_lock,
  // which the sweep holds until its sk_error_report returned; the class it then reads is SocketError, no alarm fires
  // (noSweepRecheck: the run aborts Unclassified at once and the alarm fires)
  run SweepStoresBeforeErrReadRechecksUnderTableLock = InitWith("pair")
    .then(Write).then(Dispatch("s")).then(Load("s")).then(Use("s")).expect(st.pend)
    .then(SwState).then(WStart).then(WMove).then(WEnd)
    .expect(Safety and st.wrun == "lock" and st.rawEnd == "eof" and st.pst == "live" and not(st.unclassified))
    .then(SwErr).then(SwReport).then(Dispatch("s")).then(Load("s")).then(Use("s"))
    .expect(Safety and st.cls == "reset" and not(canWRecheck(st)))
    .then(SwUnlock).then(WRecheck)
    .expect(Safety and st.cause == "SocketError" and not(st.unclassified) and st.wrun == "end")
  // no barrier: the sweep's sk_err store is visible before its peer_shutdown store; the run reads -ECONNRESET with no
  // class recorded (sk_err is read before the queue, so the queued unit is not moved) and waits for the lock in the
  // same way
  run SweepErrVisibleFirstRechecksUnderTableLock = InitWith("pair")
    .then(Write).then(Dispatch("s")).then(Load("s")).then(Use("s"))
    .then(SwErr).then(WStart).then(WEnd)
    .expect(Safety and st.wrun == "lock" and st.rawEnd == "err" and st.sq == 1)
    .then(SwState).then(SwReport).then(Dispatch("s")).then(Load("s")).then(Use("s")).then(SwUnlock).then(WRecheck)
    .expect(Safety and st.cause == "SocketError" and not(st.unclassified))
  // the context's release ends a pair with no record
  run ReleaseEndsPairWithoutRecord = InitWith("pair")
    .then(CtxRel).then(WStart).then(WEnd).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep)
    .then(RcuGp).then(FreeCancel).then(FreeFlush).then(FreeDone)
    .expect(Safety and st.cause == "Release" and st.rec == "none" and st.nrec == 0 and st.pst == "freed")
  // a watch's override takes its end (the guest's 0x2): record under the lock, then the original; an owner abort that
  // follows finds no id (ENOENT); the watch's work item unhooks and queues the free; one record
  run WatchOverrideEndThenOwnerAbortIsEnoent = InitWith("watch")
    .then(PeerFin).then(Dispatch("s")).then(Load("s")).then(Use("s"))
    .expect(Safety and st.cause == "HalfClose" and st.rec == "queued" and not(st.id) and st.pend)
    .then(CtlAbort).expect(st.nrec == 1 and st.oprog == List())
    .then(WStart).then(WUnhook).then(WStep).then(WStep).then(WStep).then(RcuGp).then(FreeCancel).then(FreeFlush)
    .then(FreeDone).then(TakeEnd)
    .expect(Safety and st.pst == "freed" and st.nrec == 1 and st.rec == "read")
  // the owner aborts a watch while the sweep's sk_error_report is inside the override and still finds the watch: the
  // abort owns the end (id removed, record Detached, unhook); the override finds no id and only calls the original;
  // the grace period waits for it
  run WatchOwnerAbortRacingSweepOneRecord = InitWith("watch")
    .then(SwState).then(SwErr).then(SwReport).then(Dispatch("s")).then(CtlAbort).then(Load("s"))
    .expect(st.inv.get("s").ph == "found")
    .then(OStep).then(OStep).then(OStep).expect(Safety and st.fr == "gp" and not(canRcuGp(st)))
    .then(Use("s")).then(RcuGp).then(FreeCancel).then(FreeFlush).then(FreeDone)
    .expect(Safety and st.cause == "Detached" and st.nrec == 1 and st.pst == "freed")

  // the write-space closure (round 16): a waker (sock_wfree from the vhost worker: no socket lock, no RCU section,
  // preemptible) holds wqlk(t) inside the entry's function as the pair's end reaches the entry's removal: the removal
  // waits for wqlk(t); the wake queues one more run (dropped by the free's cancel_work_sync); the pair ends and the
  // free runs
  run WfreeWakeFinishesBeforeRemoveWait = InitWith("pair")
    .then(Write).then(Dispatch("s")).then(Load("s")).then(Use("s")).then(WStart).then(WMove).then(WDrained).then(WRet)
    .then(Write).then(Dispatch("s")).then(Load("s")).then(Use("s")).then(WStart).then(WBlock).then(WRet)
    .then(Read).then(WsEnter).expect(st.ws == "entry" and st.wqe)
    .then(CtxRel).then(WStart).then(WEnd).then(WStep).then(WStep)
    .expect(Safety and st.wprog.head() == "wt" and not(canWStep(st)))
    .then(WsLeave).expect(Safety and st.pend and canWStep(st))
    .then(WStep).expect(not(st.wqe)).then(WStep).then(WStep).then(WStep).then(WStep)
    .expect(Safety and st.fr == "gp" and st.wrun == "idle" and not(st.id))
    .then(RcuGp).then(FreeCancel).expect(not(st.pend))
    .then(FreeFlush).then(FreeDone)
    .expect(Safety and st.pst == "freed" and not(st.uaf))

}

// design instances
module safe_pair { import fwd_safe(OFF = Set(), CFG = "pair").* from "./fwd_safe" }
module safe_watch { import fwd_safe(OFF = Set(), CFG = "watch").* from "./fwd_safe" }
module safe_plive { import fwd_safe(OFF = Set(), CFG = "plive").* from "./fwd_safe" }
module safe_wlive { import fwd_safe(OFF = Set(), CFG = "wlive").* from "./fwd_safe" }
// the named runs (both kinds, faults unbounded); not a conformance main (the module is C, ADR-0204)
module safe_runs { import fwd_safe(OFF = Set(), CFG = "runs").* from "./fwd_safe" }
