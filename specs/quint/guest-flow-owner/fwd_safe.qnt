// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — the forwarder's pair lifecycle at its real lock and RCU boundaries
 * (EVMOD-SAFE), and the wakeups that make backpressure resume.
 *
 * Design artifact. Models the design text, not an implementation. NOT a conformance target: every rule here is
 * internal to the C module `ovd_fwd` (ADR-0204 § Consequences: "The rules are internal to the C module; no port or
 * wire format carries them"); its evidence is the DELIVER review against the source and V-EV-SAFE (A-45, KASAN).
 *
 * Source of truth: docs/feature/netns-density-295/feature-delta.md § "[REF] vsock data path — the in-kernel
 * event-driven forwarder (EVMOD)": "Forwarding, per direction" (Trigger, Run, Why it is bounded), "Order of a pair's
 * end", "Stale invocations" rules 1-5 (EVMOD-SAFE, ADR-0204; rule 5 MODULE-REFCOUNT 2026-10-20), "Module placement and
 * lifetime" (EVMOD-LIFE as amended, ADR-0198), "Locking on the callback paths", "Watch",
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
 *   - The write-space trigger (round 16, rule 5's write-space closure): sk_write_space is NEVER overridden. The
 *     socket's own sk_write_space (kernel text) wakes the socket's wait queue with EPOLLOUT; the forwarder's entry on
 *     that queue (`wqe`, added at attach) runs its function only while the waker holds the queue head's lock wqlk(t)
 *     with interrupts disabled (kernel/sched/wait.c:104-108, :118-127 @ v7.0). Read (the reader frees target space)
 *     starts the wake `ws` "called" by a waker of class `wsCl` "bh" or "wfree" (nondeterministic: sock_wfree calls
 *     sk_write_space with no socket lock, no RCU section and preemption enabled, net/core/sock.c:2675-2693 @ v7.0, and
 *     only TCP sets SOCK_USE_WRITE_QUEUE; modelling the class on the one abstract target is the conservative choice);
 *     WsEnter takes wqlk(t) and enters the entry function if the entry is present ("entry", module text); WsLeave runs
 *     it (record the run, queue_work: a touch of the pair) and drops wqlk(t). The pair's end removes the entry
 *     (`wt`, remove_wait_queue under lock_sock(t), kernel/sched/wait.c:69-76) before rule 2's restore and clear of t
 *     and before the free work is queued; it takes wqlk(t), so it waits while a wake holds it. Only t's entry is
 *     modelled: the source's write-space wake belongs to the other direction, which runs the same code. lock_sock(t)
 *     excludes no modelled caller of t (none is a socket-lock owner of t), so it is not modelled.
 *   - A pair's end (`wprog`, run inside its own work item, § "Order of a pair's end"): mark ended; restore cb then
 *     clear ud on s, then remove t's wait-queue entry, restore cb and clear ud on t (rules 2 and 5; the abort actions, whose wakeups reach the restored originals, are fwd_pair's);
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
 *   - The module's lifetime (EVMOD-LIFE as amended by MODULE-REFCOUNT 2026-10-20, ADR-0198; rule 5 of EVMOD-SAFE,
 *     ADR-0204): `modref` counts the module references — the modelled context's (`ctx`, the misc fops owner, taken at
 *     open, released by __fput after the release returned and the id table is empty), another context's (`octx`,
 *     CtxOpen / CtxOtherClose), and one per hooked half (`sref`, taken at attach, released by `module_put` as the free
 *     work's last statement; a half with a ULP installed at its removal keeps it for the boot). `Unload`
 *     (delete_module) is enabled only at modref = 0 and completes only once module_exit's destroy_workqueue drained the
 *     module's own workqueue (no pending or running pair work item or free work); it sets `unloaded`.
 *   - Callers' classes (rule 5): every invocation records its class `cl`: "lock" (a caller that owns the socket lock:
 *     the vsock receive path, Write and PeerFin on s), "bh" (the delta's "rcu" class: a region a vanilla grace period
 *     waits for from its call site on — TCP input on the target, TcpRx; the orphan sweep's sk_error_report; a ULP's
 *     call through its saved copy), "wfree" (a write-space waker with no socket lock, no RCU section and preemption
 *     enabled: the vhost worker, the guest's virtio-vsock transmit work, a NIC driver's skb free; it reaches only the
 *     wait-queue entry, which wqlk(t) closes, never an override; writeSpaceOverride puts it back on an override). The third class, the forwarder's own
 *     work item, is `wrun`. A vanilla grace period waits for a "bh" caller from the call site on (S7); a "lock" caller
 *     is an RCU reader only inside rcu_read_lock (Load .. Use / Fallback). TCP backlog processing in release_sock is a
 *     "lock" caller of t; it is not modelled separately (rule 5 excludes it exactly as it excludes the vsock receive
 *     path on s, which carries the teeth).
 *   - Rule 5's removal: rule 2's restore and clear of a half run under lock_sock of that half (`lk` "rm" from the
 *     half's first removal step to its second): a removal step is enabled only when no "lock" caller holds it.
 *   - A ULP (UlpInstall, an environment fault: an owner defect ADR-0200 rules out, kept to give the ULP check teeth) on
 *     the TCP half saves a copy of the override (kTLS: tls_sw_strparser_arm saves sk_data_ready,
 *     net/tls/tls_sw.c:2693-2694 @ v7.0); UlpCall is the ULP calling that copy from a "bh" caller, whatever the
 *     socket's callback pointer is.
 *
 * Ghost (model-only) state: uaf (some actor touched freed pair state: an override's use, a run's start, a control
 * call, a queue_work), recursed (an override called itself), lost (an event reached no callback), unclassified (the
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
 *   (MODULE-REFCOUNT, numbered as in feature-delta.md § "Formal protocol model", round 15; the delta's S5..S8 are
 *    M5..M8 here, since S5 above is the wakeup fact)
 *   M5  delete_module refuses while any module reference is held and calls exit only at zero
 *       (kernel/module/main.c:730-751, :828 @ v7.0) ........................................ source; V-EV-UNLOAD
 *   M6  destroy_workqueue drains every pending and running item of the module's own workqueue
 *       (kernel/workqueue.c:5979, :5962, :4183-4194 @ v7.0) ........................................ source; V-EV-UNLOAD
 *   M7  a vanilla RCU grace period waits for bottom-half-disabled regions (include/linux/rcupdate.h:671-675 @ v7.0)
 *       .......................................................................................... source; V-EV-SAFE
 *   M8  every override invocation comes from a socket-lock owner, a bottom-half-disabled region, or the forwarder's own
 *       work item ...................................................... DELIVER source review; V-EV-SAFE, V-EV-UNLOAD
 *
 * OFF names the rules switched off (teeth), or the rejected alternative switched on:
 *   rcuAssignNull      (alternative (a)) sk_user_data cleared by rcu_assign_pointer(…, NULL) and read by
 *                      rcu_dereference: the clear may be seen before the restore, and the override's later read of
 *                      the callback pointer is not ordered after its load (rule 2 off)
 *   noFallback         (alternative (b)) an override that finds no pair returns without calling anything
 *   noGracePeriod      the free work is queued with queue_work, not queue_rcu_work (rule 3: no grace period)
 *   noCancelSync       the free work frees without cancel_work_sync on the pair's work item (rule 3: no completion)
 *   ctlQueueOutsideRcu (alternative (d)) a control call looks the id up under RCU, leaves the read section, then queues
 *   ctlQueueInsideRcu  (alternative (d) read the other way) the same lookup, queueing before rcu_read_unlock
 *   watchFreeBeforeId  (rule 4 order, watch) the owner's abort of a watch queues the free before it removes the id
 *   watchEndNoIdCheck  (rule 4, watch) the watch's override queues its record without checking the id under the lock
 *   noWriteSpace       the target's write-space wait-queue entry is not added (the wake reaches nothing of ours)
 *   writeSpaceOverride (rule 5's write-space closure off) the write-space trigger is an sk_write_space override again,
 *                      entered by the waker outside wqlk(t), with no wait-queue entry
 *   removeWaitAfterFreeQueued (rule 5's write-space closure order) the entry is removed by the free work after its
 *                      cancel_work_sync, i.e. after the free work was queued, not by the end before it
 *   noDataReady        sk_data_ready of the source is not overridden
 *   noStateChange      sk_state_change of the source is not overridden (the peer's 0x2 queues no run)
 *   noErrorReport      sk_error_report of the source is not overridden (the sweep's error queues no run)
 *   lateClearPending   the run's "needs a run" flag is cleared when the run returns, not before it starts
 *   ctlAbortNoQueue    OVD_FWD_ABORT marks the abort requested without queueing the work item
 *   endSplitNoRequeue  the pair's end stops after unhooking and leaves the record and the free to a later run it
 *                      does not queue
 *   noSweepRecheck     (EVMOD-SWEEP, F-S1) the run acts on an end it read with no recorded class without the
 *                      vsock_table_lock re-check: an end-of-stream read in the sweep's window aborts Unclassified
 *   refDroppedAtUnhook (rule 5) a half's module_put at its removal (its sk_user_data clear), before the grace period
 *   removeWithoutSockLock (rule 5) the removal of a half does not take its socket lock
 *   ulpRefReleased     (rule 5) the free work drops the reference of a half that had a ULP at its removal
 *   putOnSystemQueue   (rule 5 / EVMOD-LIFE) the free work runs on a system workqueue: module_exit's drain does not
 *                      wait for its return after its module_put
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
  val BUG_NO_FALLBACK: bool = OFF.contains("noFallback")
  val BUG_NO_GP: bool = OFF.contains("noGracePeriod")
  val BUG_NO_CANCEL: bool = OFF.contains("noCancelSync")
  val BUG_CTL_OUTSIDE: bool = OFF.contains("ctlQueueOutsideRcu")
  val ALT_CTL_INSIDE: bool = OFF.contains("ctlQueueInsideRcu")
  val CTL_RCU: bool = BUG_CTL_OUTSIDE or ALT_CTL_INSIDE
  val BUG_W_FREE_FIRST: bool = OFF.contains("watchFreeBeforeId")
  val BUG_W_NO_ID: bool = OFF.contains("watchEndNoIdCheck")
  val BUG_NO_WSPACE: bool = OFF.contains("noWriteSpace")
  val BUG_WS_OVR: bool = OFF.contains("writeSpaceOverride")
  val BUG_RW_LATE: bool = OFF.contains("removeWaitAfterFreeQueued")
  val BUG_NO_DREADY: bool = OFF.contains("noDataReady")
  val BUG_NO_SCHANGE: bool = OFF.contains("noStateChange")
  val BUG_NO_EREPORT: bool = OFF.contains("noErrorReport")
  val BUG_LATE_CLEAR: bool = OFF.contains("lateClearPending")
  val BUG_CTL_NO_QUEUE: bool = OFF.contains("ctlAbortNoQueue")
  val BUG_END_SPLIT: bool = OFF.contains("endSplitNoRequeue")
  val BUG_NO_RECHECK: bool = OFF.contains("noSweepRecheck")
  val BUG_PUT_AT_UNHOOK: bool = OFF.contains("refDroppedAtUnhook")
  val BUG_NO_SOCK_LOCK: bool = OFF.contains("removeWithoutSockLock")
  val BUG_ULP_PUT: bool = OFF.contains("ulpRefReleased")
  val BUG_SYS_QUEUE: bool = OFF.contains("putOnSystemQueue")

  // ------------------------------------------------------------------ types
  // ph: none | called (the kernel reached the call site) | ovr (in the override, before its load) | found (pair found,
  // inside rcu_read_lock) | nul (sk_user_data NULL, inside rcu_read_lock); ev: data | state | err | space;
  // cl (rule 5): none | lock (the caller owns the socket lock) | bh (waited for by a grace period from its call site) |
  // wfree (no lock, no RCU section, preemptible; reaches an override only under writeSpaceOverride)
  type Inv = { ph: str, ev: str, cl: str }
  // pst: live | ending (marked ended; its end in progress) | ended (the free work queued) | freed
  // cause: none | Completed | HalfClose | SocketError | Unclassified | Detached | Release
  // rec: none | queued | read; fr: none | gp | queued | flush | flushed | done (kfree'd) | put (module_put done, the
  // free work not yet returned) | freed (returned)
  // lk: none | rx (a "lock" caller holds lock_sock(s)) | rm (rule 5: a removal holds lock_sock(s))
  // wrun: idle | top | lock (an unclassified end read, waiting for vsock_table_lock) | ret | end
  // cls: none | clean (the peer's 0x2, sk_state_change) | reset (sk_err, recorded by the sk_error_report override)
  // cq (alternative (d)): none | held (a control call inside rcu_read_lock with the pointer) | out (left it, pointer kept)
  // sw: the orphan sweep's progress: none | stores (vsock_table_lock held, stores in progress) | report (its
  // sk_error_report running, lock held) | done (lock released); swPs: its peer_shutdown store is visible
  // rawEnd: what the run's read returned when it found an unclassified end: none | eof | err
  // wqe: t's write-space wait-queue entry is on the queue; ws: the write-space wake: none | called (the socket's own
  // sk_write_space, kernel text) | entry (wqlk(t) held, in the entry function: module text); wsCl: its waker's class
  type St = {
    kind: str, cb: str -> str, ud: str -> str, inv: str -> Inv, lk: str,
    pend: bool, dis: bool, wrun: str, wprog: List[str], oprog: List[str],
    pst: str, cause: str, cls: str, abortReq: bool, relReq: bool,
    id: bool, rec: str, nrec: int, fr: str, gpw: Set[str], nfree: int,
    sq: int, tq: int, wr: int, got: int, nospace: bool, fin: bool, peerFin: bool,
    ps: Set[str], err: bool, sw: str, swPs: bool, rawEnd: str, cq: str, faults: int, needUnhook: bool,
    wqe: bool, ws: str, wsCl: str,
    uaf: bool, recursed: bool, lost: bool, unclassified: bool, delivered: int,
    // MODULE-REFCOUNT: the module references and the unload
    ctx: bool, octx: bool, sref: str -> bool, ulp: str -> bool, modref: int, unloaded: bool,
  }

  var st: St

  // ------------------------------------------------------------------ helpers
  val HALVES: Set[str] = Set("s", "t")
  def halves(t: St): Set[str] = if (t.kind == "pair") Set("s", "t") else Set("s")
  val inv0: Inv = { ph: "none", ev: "none", cl: "none" }
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
  // (M7: a "bh" caller is waited for from its call site on; a "lock" caller only inside rcu_read_lock)
  def readers(t: St): Set[str] =
    HALVES.filter(x => t.inv.get(x).ph.in(Set("found", "nul")) or (t.inv.get(x).cl == "bh" and t.inv.get(x).ph != "none"))
      .union(if (t.cq == "held") Set("c") else Set())
  // an invocation on half x returns (rcu_read_unlock; the receive path then releases lock_sock(s))
  def endInv(t: St, x: str): St =
    { ...t, inv: t.inv.set(x, inv0), gpw: t.gpw.exclude(Set(x)), lk: if (x == "s" and t.lk == "rx") "none" else t.lk }
  def startInv(t: St, x: str, ev: str, cl: str): St = { ...t, inv: t.inv.set(x, { ph: "called", ev: ev, cl: cl }) }
  // the pair's end, in its work item (§ "Order of a pair's end"); rcuAssignNull: each clear may be seen before its
  // restore
  // "wt": rule 5's removal of t's write-space wait-queue entry, before t's restore and clear and before the free work
  // is queued (removeWaitAfterFreeQueued: not here, the free work removes it)
  val WT: List[str] = if (BUG_RW_LATE) List() else ["wt"]
  val END_PAIR: List[str] =
    if (BUG_RCU_NULL) ["us", "rs"].concat(WT).concat(["ut", "rt", "idrec", "free"])
    else ["rs", "us"].concat(WT).concat(["rt", "ut", "idrec", "free"])
  // a watch's unhook, by the path that removed its id (or its work item, when that path was the override)
  val UNHOOK_W: List[str] = if (BUG_RCU_NULL) ["us", "rs", "free"] else ["rs", "us", "free"]
  // one step of an end program (it reads the saved originals from the pair's state: a touch)
  // rule 5: a removal step of s holds lock_sock(s) from the half's first removal step to its second ("rm"); it is
  // enabled only when no "lock" caller holds the lock (removeWithoutSockLock: it ignores the lock)
  def sStep(step: str): bool = step.in(Set("rs", "us"))
  // "wt" takes wqlk(t): it waits while a wake holds it (remove_wait_queue)
  def lockOk(t: St, step: str): bool =
    (not(sStep(step)) or BUG_NO_SOCK_LOCK or t.lk != "rx") and (step != "wt" or t.ws != "entry")
  // the lock is held from the half's first removal step to its last (rest: the program after this step)
  def sockLock(t: St, step: str, rest: List[str]): St =
    if (not(sStep(step)) or BUG_NO_SOCK_LOCK) t
    else { ...t, lk: if (rest.select(x => sStep(x)).length() > 0) "rm" else "none" }
  // refDroppedAtUnhook: the half's module_put at its sk_user_data clear (the ULP check kept)
  def putHalf(t: St, x: str): St =
    if (t.sref.get(x) and (not(t.ulp.get(x)) or BUG_ULP_PUT)) { ...t, sref: t.sref.set(x, false), modref: t.modref - 1 }
    else t
  def earlyPut(t: St, x: str): St = if (BUG_PUT_AT_UNHOOK) putHalf(t, x) else t
  def doStep(t0: St, step: str, rest: List[str]): St =
    val t = sockLock(touch(t0), step, rest)
    if (step == "wt") { ...t, wqe: false }
    else if (step == "rs") { ...t, cb: t.cb.set("s", "orig") }
    else if (step == "us") earlyPut({ ...t, ud: t.ud.set("s", "null") }, "s")
    else if (step == "rt") { ...t, cb: t.cb.set("t", "orig") }
    else if (step == "ut") earlyPut({ ...t, ud: t.ud.set("t", "null") }, "t")
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
      // the pair's target carries its write-space entry (attach adds it; noWriteSpace: not added; writeSpaceOverride:
      // the trigger is an override, no entry)
      wqe: k == "pair" and not(BUG_NO_WSPACE) and not(BUG_WS_OVR), ws: "none", wsCl: "none",
      uaf: false, recursed: false, lost: false, unclassified: false, delivered: 0,
      // the context is open and every hooked half holds its reference (attach takes it, __module_get)
      ctx: true, octx: false,
      sref: HALVES.mapBy(x => k == "pair" or x == "s"), ulp: HALVES.mapBy(x => false),
      modref: 1 + (if (k == "pair") 2 else 1), unloaded: false,
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
    st' = startInv({ ...st, sq: st.sq + 1, wr: st.wr + 1, lk: "rx" }, "s", "data", "lock"),
  }
  // the vsock peer's 0x2: peer_shutdown updated under lock_sock(s), then sk_state_change (fwd_pair's F2)
  action PeerFin: bool = all {
    st.sw == "none", not(st.peerFin), st.inv.get("s").ph == "none", st.lk == "none",
    st' = startInv({ ...st, ps: st.ps.union(Set("S")), peerFin: true, lk: "rx" }, "s", "state", "lock"),
  }
  // the far reader reads from the target; freed space after an EAGAIN send wakes sk_write_space (S5), called by a
  // waker of class c ("bh" or "wfree", S6); the socket's own sk_write_space wakes the wait queue (writeSpaceOverride:
  // it is the override, dispatched like any other)
  def canRead(t: St): bool =
    t.kind == "pair" and t.tq > 0 and (not(t.nospace) or (t.inv.get("t").ph == "none" and t.ws == "none"))
  action ReadBy(c: str): bool = all {
    canRead(st), c.in(Set("bh", "wfree")),
    val t1 = { ...st, tq: st.tq - 1, got: st.got + 1, nospace: false }
    st' =
      if (not(st.nospace)) t1
      else if (BUG_WS_OVR) startInv(t1, "t", "space", c)
      else { ...t1, ws: "called", wsCl: c },
  }
  // the named runs' read: a softirq waker
  action Read: bool = ReadBy("bh")
  // the wake takes wqlk(t) and enters the entry's function if the entry is on the queue (else it wakes nothing of ours)
  def canWsEnter(t: St): bool = t.ws == "called"
  action WsEnter: bool = all {
    canWsEnter(st),
    st' = if (st.wqe) { ...st, ws: "entry" } else { ...st, ws: "none", wsCl: "none" },
  }
  // the entry's function: the pair is reached through the entry itself (a touch); record the run, queue_work; then
  // wake_up drops wqlk(t)
  def canWsLeave(t: St): bool = t.ws == "entry"
  action WsLeave: bool = all { canWsLeave(st), st' = { ...queueWork(touch(st)), ws: "none", wsCl: "none" } }
  // TCP input calls an override of the target (its sk_data_ready: the other direction's trigger) from a region a
  // grace period waits for (rule 5's "rcu" class; an environment event, bounded by FMAX)
  action TcpRx: bool = all {
    st.kind == "pair", canFault(st), st.inv.get("t").ph == "none",
    st' = startInv(fault(st), "t", "rx", "bh"),
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
  action SwReport: bool = all { canSwReport(st), st' = startInv({ ...st, sw: "report" }, "s", "err", "bh") }
  // the walk moves on and leaves vsock_table_lock once sk_error_report returned
  def canSwUnlock(t: St): bool = t.sw == "report" and t.inv.get("s").ph == "none"
  action SwUnlock: bool = all { canSwUnlock(st), st' = { ...st, sw: "done" } }
  // a ULP installed on the TCP half after attach saves a copy of the override (an owner defect, ADR-0200; teeth for
  // rule 5's ULP check)
  action UlpInstall(x: str): bool = all {
    st.kind == "pair", x == "t", canFault(st), st.cb.get(x) == "ovr", not(st.ulp.get(x)), not(st.unloaded),
    st' = fault({ ...st, ulp: st.ulp.set(x, true) }),
  }
  // the ULP calls its saved copy of the override from a "bh" caller, whatever the socket's pointer now is
  action UlpCall(x: str): bool = all {
    st.ulp.get(x), canFault(st), st.inv.get(x).ph == "none",
    st' = fault({ ...st, inv: st.inv.set(x, { ph: "ovr", ev: "data", cl: "bh" }) }),
  }

  // ------------------------------------------------------------------ the callback invocation (rules 1, 2)
  // the kernel reads the socket's callback pointer (noWriteSpace / noDataReady: that kind is not overridden; a watch
  // overrides only sk_state_change and sk_error_report)
  def overridden(t: St, x: str, ev: str): bool = and {
    ev != "space" or BUG_WS_OVR, not(BUG_NO_DREADY and ev == "data"),
    not(BUG_NO_SCHANGE and ev == "state"), not(BUG_NO_EREPORT and ev == "err"),
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
  // held (rcuAssignNull: the read is not ordered after the load and may return the override already called;
  // noFallback: nothing is called)
  def canFallback(t: St, x: str): bool = t.inv.get(x).ph == "nul"
  action Fallback(x: str): bool = all {
    canFallback(st, x),
    val cur = if (BUG_RCU_NULL) "ovr" else st.cb.get(x)
    val t1 =
      if (BUG_NO_FALLBACK) { ...st, lost: true }
      else if (cur == "ovr") { ...st, recursed: true }
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
  def canFreeCancel(t: St): bool = t.fr == "queued" and not(BUG_NO_CANCEL)
  action FreeCancel: bool = all { canFreeCancel(st), st' = { ...st, dis: true, pend: false, fr: "flush" } }
  // ... wait for the running item, re-enable, return
  def canFreeFlush(t: St): bool = t.fr == "flush" and t.wrun == "idle"
  action FreeFlush: bool = all { canFreeFlush(st), st' = { ...st, dis: false, fr: "flushed" } }
  // removeWaitAfterFreeQueued: the free work removes t's entry after its cancel_work_sync returned, then frees
  def canFreeRemoveWait(t: St): bool = BUG_RW_LATE and t.fr == "flushed" and t.wqe and t.ws != "entry"
  action FreeRemoveWait: bool = all { canFreeRemoveWait(st), st' = { ...touch(st), wqe: false } }
  def canFreeDone(t: St): bool =
    (t.fr == "flushed" and not(BUG_RW_LATE and t.wqe)) or (BUG_NO_CANCEL and t.fr == "queued")
  action FreeDone: bool = all { canFreeDone(st), st' = { ...st, pst: "freed", fr: "done" } }
  // rule 5: module_put once per hooked half, as the free work's last statement (a half with a ULP keeps its reference
  // for the boot; ulpRefReleased: dropped anyway; refDroppedAtUnhook: already dropped at the removal)
  def canFreePut(t: St): bool = t.fr == "done"
  action FreePut: bool = all {
    canFreePut(st),
    st' = { ...putHalf(putHalf(st, "s"), "t"), fr: "put" },
  }
  // the free work returns (still module text after its module_put: module_exit's drain waits for it)
  def canFreeRet(t: St): bool = t.fr == "put"
  action FreeRet: bool = all { canFreeRet(st), st' = { ...st, fr: "freed" } }

  // ------------------------------------------------------------------ control calls by id (rule 4) and the owner
  // OVD_FWD_ABORT, in one section under the record-queue lock: an id not live → ENOENT. Pair: the abort requested and
  // the work item queued (ctlAbortNoQueue: not queued). Watch (WATCH-ABORT): the id removed and the record queued, then
  // the unhook in process context (watchFreeBeforeId: the free queued before the id is removed).
  action CtlAbort: bool = all {
    st.ctx, canFault(st), not(CTL_RCU and st.kind == "pair"),
    val t0 = fault(st)
    st' =
      if (not(st.id)) t0
      else if (st.kind == "pair")
        val t1 = touch({ ...t0, abortReq: true })
        if (BUG_CTL_NO_QUEUE) t1 else queueWork(t1)
      else if (st.oprog.length() > 0) t0
      else if (BUG_W_FREE_FIRST)
        { ...t0, pst: "ending", cause: "Detached", oprog: ["rs", "us", "free", "idrec"] }
      else
        { ...t0, id: false, rec: "queued", nrec: st.nrec + 1, pst: "ending", cause: "Detached", oprog: UNHOOK_W },
  }
  // OVD_FWD_PAIR_STATE: the counters copied under the lock
  action CtlState: bool = all {
    st.ctx, canFault(st),
    st' = if (st.id) touch(fault(st)) else fault(st),
  }
  // alternative (d): the lookup under rcu_read_lock, then the queue (inside or outside the read section)
  action CtlLookup: bool = all {
    CTL_RCU, st.ctx, st.kind == "pair", st.cq == "none", canFault(st), st.id,
    st' = fault({ ...st, cq: "held" }),
  }
  def canCtlUnlock(t: St): bool = t.cq == "held" and (BUG_CTL_OUTSIDE or t.abortReq)
  action CtlUnlock: bool = all {
    canCtlUnlock(st),
    st' = { ...st, cq: if (BUG_CTL_OUTSIDE) "out" else "none", gpw: st.gpw.exclude(Set("c")) },
  }
  def canCtlQueue(t: St): bool = (BUG_CTL_OUTSIDE and t.cq == "out") or (ALT_CTL_INSIDE and t.cq == "held" and not(t.abortReq))
  action CtlQueue: bool = all {
    canCtlQueue(st),
    val t1 = queueWork(touch({ ...st, abortReq: true }))
    st' = if (BUG_CTL_OUTSIDE) { ...t1, cq: "none" } else t1,
  }
  // the context's release (the owner's death): pair — the end requested with no record; watch — the id removed, no
  // record, the unhook in process context
  action CtxRel: bool = all {
    st.ctx, not(st.relReq), canFault(st),
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

  // ------------------------------------------------------------------ module references and the unload (rule 5)
  // __fput's fops_put: after the context's release returned, which waits until the id table is empty
  def canCtxClose(t: St): bool = t.ctx and t.relReq and not(t.id)
  action CtxClose: bool = all { canCtxClose(st), st' = { ...st, ctx: false, modref: st.modref - 1 } }
  // another context opened (misc_open's try_module_get, which fails once the module is going) and closed
  action CtxOpen: bool = all {
    not(st.octx), not(st.unloaded), canFault(st),
    st' = fault({ ...st, octx: true, modref: st.modref + 1 }),
  }
  action CtxOtherClose: bool = all { st.octx, st' = { ...st, octx: false, modref: st.modref - 1 } }
  // destroy_workqueue's drain (M6): no pair work item pending or running, no free work queued to or running on the
  // module's own workqueue (putOnSystemQueue: the free work is on a system queue, so the drain does not see it). A free
  // work still in its RCU callback ("gp") is not on the workqueue yet.
  def drained(t: St): bool =
    not(t.pend) and t.wrun == "idle" and (BUG_SYS_QUEUE or not(t.fr.in(Set("queued", "flush", "flushed", "done", "put"))))
  // delete_module (M5): refused while any reference is held; exit only at zero, completing after the drain
  def canUnload(t: St): bool = t.modref == 0 and not(t.unloaded) and drained(t)
  action Unload: bool = all { canUnload(st), st' = { ...st, unloaded: true } }

  // ------------------------------------------------------------------ next
  action step = {
    nondet x = HALVES.oneOf()
    any {
      ReadBy("bh"), ReadBy("wfree"), WsEnter, WsLeave, TcpRx,
      Write, PeerFin, SwState, SwErr, SwReport, SwUnlock,
      Dispatch(x), Load(x), Use(x), Fallback(x),
      WStart, WNoop, WEnd, WRecheck, WMove, WBlock, WDrained, WUnhook, WRet, WStep, OStep,
      RcuGp, FreeCancel, FreeFlush, FreeRemoveWait, FreeDone,
      CtlAbort, CtlState, CtlLookup, CtlUnlock, CtlQueue, CtxRel, TakeEnd,
      UlpInstall(x), UlpCall(x), FreePut, FreeRet, CtxClose, CtxOpen, CtxOtherClose, Unload,
    }
  }

  // ------------------------------------------------------------------ invariants (EVMOD-SAFE, EVMOD-REC, EVMOD-END)
  // rules 1, 3, 4: no override, run, queue_work or control call touches freed pair state
  val NoTouchAfterFree: bool = not(st.uaf)
  // rule 2: an override never calls itself
  val NoOverrideCallsItself: bool = not(st.recursed)
  // rule 2 (alternative (b) rejected): an event that reaches an override of an ended pair still reaches the socket's
  // callback
  val NoEventLost: bool = not(st.lost)
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
  // MODULE-REFCOUNT (EVMOD-LIFE, ADR-0198; EVMOD-SAFE rule 5, ADR-0204). The actors executing module text: an override
  // from its dispatch to its return, the write-space entry's function (under wqlk(t)), the pair's (watch's) work item,
  // the free work from its start to its return, a control call
  def overridesInText(t: St): Set[str] = HALVES.filter(x => t.inv.get(x).ph.in(Set("ovr", "found", "nul")))
  def inText(t: St): Set[str] =
    overridesInText(t)
      .union(if (t.wrun != "idle") Set("work") else Set())
      .union(if (t.fr.in(Set("flush", "flushed", "done", "put"))) Set("free") else Set())
      .union(if (t.cq != "none") Set("ctl") else Set())
      .union(if (t.ws == "entry") Set("entry") else Set())
  // what can still enter module text later: a half whose pointer is an override, a ULP's saved copy, the write-space
  // entry on the target's wait queue, a pending work item, a free work waiting for its grace period or queued
  def reachable(t: St): bool = or {
    HALVES.exists(x => t.cb.get(x) == "ovr" or t.ulp.get(x)), t.wqe, t.pend, t.fr.in(Set("gp", "queued")),
  }
  // nothing runs, or can be entered, in the module's text once it is unloaded
  val NoTextRunAfterUnload: bool = st.unloaded implies (inText(st) == Set() and not(reachable(st)))
  // while an override is in the text, a hooked half still carries an override or a ULP copy, a work item or free work
  // before its module_put is pending or running, or a context is open, a module reference is held (the free work's
  // return after its own module_put is covered by the drain, not by a reference)
  val RefHeldWhileReachable: bool =
    (overridesInText(st) != Set() or st.ws == "entry" or reachable(st) or st.wrun != "idle" or st.cq != "none"
     or st.ctx or st.octx
     or st.fr.in(Set("flush", "flushed", "done")))
      implies st.modref > 0
  val Safety: bool = and {
    NoTouchAfterFree, NoOverrideCallsItself, NoEventLost, OneEndRecord, RecordAtIdRemoval, OneFree,
    FreeAfterWorkCompletes, FreeAfterReaders, NoIdNamesFreedState, CleanEndOnlyAfterPeerFin, NoUnclassifiedEnd,
    NoTextRunAfterUnload, RefHeldWhileReachable,
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
    canRcuGp(st), canFreeCancel(st), canFreeFlush(st), canFreeRemoveWait(st), canFreeDone(st), canFreePut(st),
    canFreeRet(st), canWsEnter(st), canWsLeave(st),
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
  // a TCP input callback (rule 5's "rcu" class) entered the target's override as the pair's end restores and clears:
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

  // MODULE-REFCOUNT (EVMOD-LIFE, ADR-0198; EVMOD-SAFE rule 5, ADR-0204): the pair is aborted and freed while the
  // context stays open; its reference keeps the module loaded (delete_module refused)
  run UnloadRefusedWhileContextOpen = InitWith("pair")
    .then(CtlAbort).then(WStart).then(WEnd).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep)
    .then(RcuGp).then(FreeCancel).then(FreeFlush).then(FreeDone).then(FreePut).then(FreeRet).then(TakeEnd)
    .expect(Safety and st.ctx and st.modref == 1 and st.pst == "freed" and not(canUnload(st)))
  // the context is released and closed (its id table empty) while the pair's halves still hold their references: the
  // unload is refused until the free work's module_put
  run UnloadRefusedWhileSocketHooked = InitWith("pair")
    .then(CtxRel).then(WStart).then(WEnd).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep)
    .then(CtxClose).expect(Safety and not(st.ctx) and st.modref == 2 and not(canUnload(st)))
    .then(WStep).then(RcuGp).then(FreeCancel).then(FreeFlush).then(FreeDone)
    .expect(Safety and st.pst == "freed" and st.modref == 2 and not(canUnload(st)))
  // ... and succeeds once the free work dropped both references and returned (module_exit's drain waits for the return)
  run UnloadSucceedsAfterReleaseAndFree = InitWith("pair")
    .then(CtxRel).then(WStart).then(WEnd).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep)
    .then(CtxClose).then(WStep).then(RcuGp).then(FreeCancel).then(FreeFlush).then(FreeDone).then(FreePut)
    .expect(Safety and st.modref == 0 and st.fr == "put" and not(canUnload(st)))
    .then(FreeRet).then(Unload)
    .expect(Safety and st.unloaded and inText(st) == Set() and not(reachable(st)))
  // a vsock receive-path caller owns lock_sock(s) and has loaded the override's pointer (not yet an RCU reader) as the
  // pair's end reaches its removal: the removal waits for the socket lock, the caller finds the pair and returns;
  // only then are the overrides removed, and the module unloads after the free with no caller in its text
  run LateLockedCallbackFinishesBeforeRemoval = InitWith("pair")
    .then(CtlAbort).then(WStart).then(WEnd).then(Write).then(Dispatch("s"))
    .expect(st.inv.get("s").ph == "ovr" and st.inv.get("s").cl == "lock" and st.lk == "rx" and not(canWStep(st)))
    .then(Load("s")).then(Use("s")).expect(Safety and st.lk == "none" and canWStep(st))
    .then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep)
    .then(RcuGp).then(FreeCancel).then(FreeFlush).then(FreeDone).then(FreePut).then(FreeRet)
    .then(TakeEnd).then(CtxRel).then(CtxClose).then(Unload)
    .expect(Safety and st.unloaded and not(st.pend))
  // a ULP installed on the TCP half saved a copy of its override: at the removal the half keeps its reference for the
  // boot, so the module stays loaded after the context closes, and the ULP's later call of its copy runs loaded text
  // (it finds no pair and calls the restored original)
  run UlpCopyKeepsModuleLoaded = InitWith("pair")
    .then(UlpInstall("t")).then(CtxRel).then(WStart).then(WEnd)
    .then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep).then(WStep)
    .then(RcuGp).then(FreeCancel).then(FreeFlush).then(FreeDone).then(FreePut).then(FreeRet).then(CtxClose)
    .expect(Safety and st.modref == 1 and st.sref.get("t") and not(st.sref.get("s")) and not(canUnload(st)))
    .then(UlpCall("t")).then(Load("t")).then(Fallback("t"))
    .expect(Safety and not(st.unloaded) and not(st.recursed) and not(st.uaf))
  // rule 5's write-space closure (round 16): a "wfree" waker (sock_wfree from the vhost worker: no socket lock, no RCU
  // section, preemptible) holds wqlk(t) inside the entry's function as the pair's end reaches the entry's removal: the
  // removal waits for wqlk(t); the wake queues one more run (dropped by the free's cancel_work_sync); the pair ends,
  // the free runs, the module unloads and no text runs after it
  run WfreeWakeFinishesBeforeRemoveWait = InitWith("pair")
    .then(Write).then(Dispatch("s")).then(Load("s")).then(Use("s")).then(WStart).then(WMove).then(WDrained).then(WRet)
    .then(Write).then(Dispatch("s")).then(Load("s")).then(Use("s")).then(WStart).then(WBlock).then(WRet)
    .then(ReadBy("wfree")).then(WsEnter).expect(st.ws == "entry" and st.wsCl == "wfree" and st.wqe)
    .then(CtxRel).then(WStart).then(WEnd).then(WStep).then(WStep)
    .expect(Safety and st.wprog.head() == "wt" and not(canWStep(st)))
    .then(WsLeave).expect(Safety and st.pend and canWStep(st))
    .then(WStep).expect(not(st.wqe)).then(WStep).then(WStep).then(WStep).then(WStep)
    .expect(Safety and st.fr == "gp" and st.wrun == "idle" and not(st.id))
    .then(RcuGp).then(FreeCancel).expect(not(st.pend))
    .then(FreeFlush).then(FreeDone).then(FreePut).then(FreeRet).then(CtxClose).then(Unload)
    .expect(Safety and st.unloaded and inText(st) == Set() and not(reachable(st)) and not(st.uaf))

  // ------------------------------------------------------------------ non-vacuity witnesses (expected violated)
  val WitnessNoLateFallback: bool = not(st.inv.get("t").ph == "nul" or (st.inv.get("s").ph == "nul" and st.kind == "pair"))
  val WitnessNoReaderHoldsAtFree: bool = not(st.fr == "gp" and st.gpw != Set())
  val WitnessNoFreed: bool = st.pst != "freed"
  // MODULE-REFCOUNT: the module is unloaded on some behaviour
  val WitnessNoUnload: bool = not(st.unloaded)
}

// design instances
module safe_pair { import fwd_safe(OFF = Set(), CFG = "pair").* from "./fwd_safe" }
module safe_watch { import fwd_safe(OFF = Set(), CFG = "watch").* from "./fwd_safe" }
module safe_plive { import fwd_safe(OFF = Set(), CFG = "plive").* from "./fwd_safe" }
module safe_wlive { import fwd_safe(OFF = Set(), CFG = "wlive").* from "./fwd_safe" }
// the named runs (both kinds, faults unbounded); not a conformance main (the module is C, ADR-0204)
module safe_runs { import fwd_safe(OFF = Set(), CFG = "runs").* from "./fwd_safe" }
