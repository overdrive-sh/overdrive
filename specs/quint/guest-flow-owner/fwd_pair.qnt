// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — the in-kernel event-driven forwarder (EVMOD): one guest flow end to
 * end across the VM boundary, forwarded on each side by the forwarder `ovd_fwd`.
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 *
 * Source of truth: docs/feature/netns-density-295/feature-delta.md § "[REF] vsock data path — the in-kernel
 * event-driven forwarder (EVMOD)": EVMOD-ADOPT (approved 2026-10-16, ADR-0197) and the PROPOSED contracts
 * EVMOD-END (end classification in the forwarder's callback), EVMOD-PAIR (attach at pairing), EVMOD-CTL
 * (context, its lifetime and recovery, ADR-0199), EVMOD-REC (one lossless end record per id, ADR-0202),
 * EVMOD-STREAM-END (how a GuestStream resolves an end; split from EVMOD-REC 2026-10-17, no ADR; under the approved
 * WATCH-ABORT rule), EVMOD-STREAM-ABORT, EVMOD-ATTACH-END, EVMOD-DGRAM (datagram mode), EVMOD-AB (release at the
 * end record), with the 2026-10-17 revisions (U-EV-1 .. U-EV-11, U-EV3-1 .. U-EV3-7); R2-END,
 * END-IDEMPOTENT, CLEAN-FULL-END, ADR-0183 (a clean end is forwarded as SHUT_WR after the last accepted byte;
 * a slow reader is backpressure), D20.
 *
 * Abstraction:
 *   - Two sides, g (guest) and h (host); two directions, gh (written at g, read at h) and hg. Each side holds
 *     one forwarder context (`ctx`) and at most one pair or watch (`pr`), named by the side.
 *   - mode "stream" (TcpConnect, host and guest pairs): the guest application's TCP socket ↔ [guest pair: I ↔
 *     V_g] ↔ vsock ↔ [host pair: V_h ↔ D] ↔ the far peer's TCP socket. Each direction is three queues of
 *     numbered units: q0 the source TCP socket's receive queue at the source side (window W), q1 the vsock
 *     (credit C: units sent and not yet consumed by the peer forwarder), q2 the target TCP socket toward the
 *     reader (window W). A forwarder run moves ONE unit: it peeks, sends if the target has space, and consumes
 *     only what was accepted (one atomic step per unit). W = C = 1 so a direction holds at most 3 units and a
 *     writer of 4 units is blocked by a stopped reader.
 *   - mode "watch" (a dial, TcpAccept): the host side holds no pair; the dialer (leg-S or the probe runner)
 *     reads and writes V_h directly (q1), and the forwarder WATCHES V_h: it classifies the guest's direction
 *     and queues the watch's end record before the end is readable. The guest side is the TcpAccept pair
 *     V_g ↔ C.
 *   - mode "dgram" (a datagram association, EVMOD-DGRAM): UDP halves. Datagram k of direction d has size
 *     sz ∈ {0, 1, 2}; its frame is the items 10k (the header, ADR-0149) and 10k+1 .. 10k+sz. dq0 the source UDP
 *     socket's receive queue (WU datagrams: a full queue drops), q1 the vsock carrying frame items (credit CD,
 *     at least the largest frame), dq2 the far receiver's queue (WU; a full queue drops, as for any UDP
 *     socket); the target UDP socket's send may block (`blk`, the environment's send buffer).
 *   - The vsock carries data (q1) and, per direction, an ordered list of SHUTDOWN / RST packets (`ctl`, flag
 *     sets {S} = 0x2, {S, R} = 0x3 or RST). Data a side sent before a packet is already in the peer's receive
 *     queue when the packet is processed (one virtqueue, in order). Processing a packet (VCtl) is the receiving
 *     side's `sk_state_change`: it records `peer_shutdown` (vs.ps) and, for a live pair or watch, classifies
 *     the transition in the callback (EVMOD-END table): S newly set without R → clean half-close of the
 *     incoming direction (forwarded as EOF once the vsock queue is empty); S newly set with R → abort; R newly
 *     set while this side still sends → abort; R newly set after this side shut its send → none
 *     (CLEAN-FULL-END); any transition after the pair's end → none (END-IDEMPOTENT).
 *   - A TCP half needs no classifier: its FIN is end-of-stream after its queue (HopEnd0); a far peer's reset is one
 *     action, TcpRst(s, after_fin) (T-1): it sets `tcpErr` (EPIPE in CLOSE_WAIT, else ECONNRESET) and runs the half's
 *     sk_error_report in the same step, which aborts a live pair SocketError { side, errno }. A UDP half's error is
 *     `terr`, found by the next run (FwdErr). The host vsock's device-release error at VMM exit is `vs.err`.
 *   - Abort (EVMOD-CTL): a pair aborts on a socket error, a peer abort, an invalid frame, the owner's abort
 *     (Detached) or its context's release (no record). The abort resets the TCP half's far end (tcp_abort) and
 *     shuts the vsock half (SHUT_RDWR, a {S, R} packet). A completed pair (both directions clean) and an aborted
 *     pair each queue exactly one end record; the owner reads it (TakeEnd), then closes its descriptors
 *     (OwnerClose: a vsock close sends {S, R}); a trailing RST may follow (TrailingRst).
 *   - Owner sides attach in the pinned order (EVMOD-PAIR): the host first (before `Paired`), the guest on
 *     `Paired`; in mode watch the watch exists from the start (the dial's V_h connects before the request is
 *     written) and the dialer acts only once the guest pair is attached (it gets its stream at `Paired`). Bytes
 *     and an end that reach a half before attach wait in its own queue; the attach-time run forwards them.
 *     Attach reads the half's state and installs the callbacks in ONE step (the locked section of § Forwarding,
 *     "Attach order": lock_sock, sk_callback_lock and, for vsock, vsock_table_lock), so a transition is either in
 *     the attach-time state or seen by a callback. An end present at attach is classified there, once, by the
 *     table with this side's send open (§ End classification, "Ends present at attach"): {S} only → clean, any R
 *     → abort, so a 0x2-then-0x3 before attach is an abort. The ends that reach a half before attach: the guest
 *     owner's close of a flow it has not attached (CloseUnattached: a pending intake child that reset, the pairing
 *     deadline, session loss — {S, R}), the host's forwarded FIN (0x2) and then its abort (0x3) before the guest
 *     TcpConnect attaches, and an application FIN before Paired. A vsock end-of-stream whose class was recorded
 *     neither at attach nor by a callback aborts the pair and counts `unclassified_eof` (UnclassifiedEof; never
 *     enabled under the design's rules — NoUnclassifiedEof).
 *   - A TCP half (EVMOD-END, "TCP halves"): sk_err on it aborts the pair while any direction of the pair is open,
 *     whatever its recvmsg returned — a far peer's FIN then a reset reads as end-of-stream and the reset is seen
 *     only by sk_error_report, in TcpRst's own step.
 *   - Ended halves at attach (EVMOD-ATTACH-END, T-3, T-4): attach accepts TCP ESTABLISHED and CLOSE_WAIT and a vsock
 *     ESTABLISHED half with any peer bits; it refuses a half that has ended — `tcpErr` set, the vsock's `err` (the
 *     sweep) or `st = closing` (a packet left peer_shutdown == SHUTDOWN_MASK with the queue empty) — with HalfEnded:
 *     no pair, no record (`ph = refused`, `attRes`, `attEnded`); in the same owner step the owner closes the TCP half
 *     with SO_LINGER{1,0} and the vsock half, and the host writes Abort { flow } (`guestAbortSent`), which the guest
 *     owner takes (GuestAbort).
 *   - Context lifetime (EVMOD-CTL, § Context lifetime and owner failure): each owner holds exactly one context
 *     (`gen` numbers the host owner's contexts, `oldCtx` is the state of the one it replaced); the host's in-process
 *     `Forwarder` recovery runs abort-all (Abort) → close the old context (CtxRelease) → open a new one (Reopen),
 *     at most once per trace; a record queued in the old context is never read. The guest owner is PID 1 and is
 *     never replaced (its death is the VM's end, VmmExit). D-5 (architect 2026-10-18): a CtxRelease(h) that finds the
 *     host pair or watch live is the host owner's death and is final (no Reopen follows); one that finds none live is
 *     the recovery's close, and the recovery's step 1 has closed the descriptors of a flow whose record is unread.
 *     U-R1 (architect 2026-10-18, DR2-CLOSE): the recovery's step 1 also ends a held GuestStream whose watch has
 *     already ended (its record queued or read) as an owner abort: `strmAbort` is set and Abort { flow } is written
 *     while the session is live; what the stream already returned to the holder and its reported end (`hres`) stay
 *     as they were, and later calls resolve the owner's own end first.
 *     D-4: the host owner reads a record and closes the flow's descriptors in one dispatch (TakeEnd(h) carries
 *     OwnerClose's effect outside watch mode); OwnerClose is the guest's own dispatch.
 *   - A watch ended by the owner (WATCH-ABORT, EVMOD-STREAM-ABORT, T-5): Abort of the watch queues its record,
 *     restores the callbacks, drops the module's reference and changes nothing of V_h; in the same owner step the
 *     stream is marked ended by the owner (`strmAbort`: every later call returns ConnectionAborted) and Abort { flow }
 *     is written while the session is live. Only the context's release shuts a watched V_h (SHUT_RDWR, an {S, R}
 *     packet; no record). The holder's drop (HolderDrop, T-6) aborts the watch and closes V_h in one step.
 *   - The holder's end observation (HolderEnd, F11 corrected): what its read returns at the end of V_h — Eof (0) or
 *     Err (a read error). After the release's SHUT_RDWR either, at once (0 while V_h is ESTABLISHED, af_vsock.c:2545-2548;
 *     ENOTCONN once it left ESTABLISHED, vsock_shutdown having cleared SOCK_DONE, :1220, :2522-2531 @ v7.0); after a
 *     guest SHUTDOWN processed by the watch's callback, Eof once the queue is empty; after the orphan sweep (VmmExit
 *     sets sk_err and SS_UNCONNECTED, which is TCP_ESTABLISHED's value, drivers/vhost/vsock.c:746-750), Err
 *     (ECONNRESET) once the queue is empty — and the sweep's state is readable BEFORE its sk_error_report (FwdErr)
 *     runs: the end is two steps, VmmExit then FwdErr (no separate sweep actions, U-EV3-7). The GuestStream's
 *     reported end (ReadEof on the dialer, `hres`: HalfClose = Ok(0), GuestAbort = Err(ConnectionReset), OwnerAbort =
 *     Err(ConnectionAborted)) is resolved from the watch, never from the observation (EVMOD-STREAM-END): the owner's own end
 *     of the watch; else the watch's record; else it waits for the record in flight.
 *   - Watch record before wake (EVMOD-STREAM-END; EV-END change 3): the watch's callbacks (VCtl, FwdErr) queue the record and then call
 *     the original callback (the holder's wakeup) in the same step (`gh.woken`).
 *   - Writers (Write, Fin), readers (Read, ReadEof, ShutRd) and resets (TcpRst, HolderDrop, VmmExit, UdpErr,
 *     UdpBlock) are the environment. Readers are NOT fair: a reader may stop for ever or shut only its read side.
 *   - Instances (CFG): "ok" (stream; one unit each way), "watch" (a dial; one unit each way),
 *     "live" (stream; one unit each way: the liveness instance), "stall" (stream; gh 4 units — more than the pipeline
 *     holds, so a stopped reader blocks its writer — hg none, no fault), "dgram" (one datagram each way), "dlive" (gh 1
 *     datagram, hg none, no send-buffer toggles: the datagram liveness instance), "wlive" (a dial; gh 1 unit, hg none:
 *     the watch liveness instance of T-5 and T-6) and "conf" (every mode; 4 units and 3 datagrams each
 *     way: the conformance instance, checked only along the named runs). On every model-checking instance a trace
 *     carries at most ONE environment fault or owner abort (TcpRst, HolderDrop, VmmExit, UdpErr, Abort,
 *     CtxRelease, CloseUnattached, DgHalfClose: `FMAX`); the named runs on `pair_conf` combine them freely. Two faults meeting in one trace (a
 *     reset racing an owner's abort) are therefore checked only along the runs, not exhaustively.
 *   - D-UP-1 (conformance step record): on the conformance main `pair_conf` (REC = true), `conf_step` records the
 *     action that produced the state, as a variant of `fwd_pair_conf::ConfStep` whose tag is the action's name
 *     and whose value is the record of its parameters by parameter name (`init` records `InitWith`). quint-connect
 *     reads it through `Config::nondet = &["pair_conf::fwd_pair::conf_step"]`. Model-only.
 *
 * Ghost (model-only) state: vs.sshut (this side's sk_shutdown & SEND_SHUTDOWN as the forwarder reads it under the
 * socket lock: set in the step that sends the SHUTDOWN, CLEAN-FULL-END), vs.misc (a transition classified otherwise
 * than the table gives with the socket's true send state), gh.hobs (the holder's end observation), gh.vhTouched (an
 * owner abort of a watch changed V_h), gh.dropped (the holder dropped its stream), guestAbortSent, gh.woken (a holder woken by a watch's end),
 * dir.loss (the stream of a direction was cut — an abort, a reset, an owner's death or
 * the VMM's exit — before its writer's FIN), dir.lost (units consumed and not accepted), vs.tcls (the class of
 * the transition that ended the incoming direction, by the EVMOD-END table, whatever the forwarder did),
 * vs.ucls (the class the forwarder acted on), vs.att (an end was present when the callbacks were installed),
 * vs.clsAt / vs.ncls (where, and how many times, the end of the incoming direction was classified), vs.snap (the
 * state read by a non-atomic attach, checkThenHook only), gh.fwdAfterEnd, gh.fwdOld (a pair of a replaced
 * context forwarded), gh.unclassified (the unclassified_eof alarm), gh.rstOpen (sides whose TCP half was reset
 * while their pair was open), gh.holderReadAfterAbort, dg.sdrop / dg.rdrop.
 *
 * ASSUMPTIONS (kernel behaviour, not modelled mechanisms; discharged in README.md):
 *   F1  A peek returns no vsock credit and no TCP window; only a dequeue does (virtio_transport_common.c:806-809,
 *       :536, :583, :654-655 @ v7.0) ................................................ P-67, P-68, V-EV-PAIR
 *   F2  The `sk_state_change` callback runs per received SHUTDOWN / RST, with the flags already in
 *       `peer_shutdown`, before end-of-stream is readable (virtio_transport_common.c:1443-1464, :1244-1253 @ v7.0;
 *       af_vsock.c:2326-2328) ................................................... proven, P-69 (V-EV-END)
 *   F3  A TCP reset is reported by sk_error_report (tcp_input.c:4890-4902 @ v7.0), and the next recvmsg / sendmsg
 *       returns it only while the forwarder still reads that half (P-69, hazard 16); a FIN is end-of-stream after
 *       every queued byte (tcp_input.c:4953-4954, tcp.c:2794-2795) ........................... P-69, P-68
 *   F4  A vsock SHUT_WR never overtakes accepted bytes (virtio_transport_common.c:1084-1097 @ v7.0) ..... T3 (P-56)
 *   F5  tcp_abort sends a reset (tcp.c:5087-5141 @ v7.0) .......................... P-68 (rst / kill shapes)
 *   F6  The release of a context (the last close of /dev/ovd_fwd, or its process's exit) aborts every pair of
 *       the context before it returns, and an end record is never dropped ............ proven, P-70 (V-EV-CTL)
 *   F7  The VMM's exit sets sk_err (ECONNRESET) on every host vsock to the guest (drivers/vhost/vsock.c:746-750 @
 *       v7.0), reported through sk_error_report ..................................... V-30 Q2c (K-C4)
 *   F8  A UDP socket's full receive queue drops (udp.c:1721-1731 @ v7.0); a frame of at most 59,008 B fits the vsock
 *       buffer because datagram-mode attach refuses a buffer_size below MAX_GUEST_DATAGRAM_FRAME (BufferTooSmall;
 *       default 256 KiB, af_vsock.c:199) ................................................. P-72 D3 (V-EV-DGRAM)
 *   F9  Attach reads the half's state and installs its callbacks in one section: every transition that changes
 *       that state runs under a lock the section holds (vsock receive under lock_sock,
 *       virtio_transport_common.c:1661, :1705; TCP input backlogged while owned, tcp_ipv4.c:2318-2321; the orphan
 *       sweep under vsock_table_lock, af_vsock.c:453-471 @ v7.0), so none falls between the read and the
 *       install. The end present at attach is classified by the table with this side's send open ({S} only →
 *       clean, any R → abort): an owner never shuts a socket before attaching it ...... source reading; P-71
 *       (h_noseed); the widened-window stress is a module-suite check
 *   F10 A TCP reset received after a FIN leaves recvmsg returning end-of-stream (SOCK_DONE is tested before
 *       sk_err, tcp.c:2794, :2797 @ v7.0) and is reported by sk_error_report (tcp_input.c:4890-4900) ... proven, P-69
 *   F11 (corrected) A vsock socket whose own receive side is shut returns 0 from recvmsg at once only while it is
 *       ESTABLISHED (af_vsock.c:2545-2548 @ v7.0); once it left ESTABLISHED without SOCK_DONE (vsock_shutdown clears
 *       it, :1220) it returns -ENOTCONN (:2522-2531). After the orphan sweep queued bytes are read and then the
 *       error is returned (:2326-2328, :2439-2440) ................................................. proven, P-70
 *   F12 The datagram source forwarder sends nothing on a vsock it shut (EPIPE); the only datagram-mode 0x2 is a peer
 *       outside the design (DgHalfClose: no Overdrive owner half-closes a datagram vsock, U-EV-5; P-72 § 8 reached
 *       the incomplete-frame end with an independent peer) ................................. P-72 § 8 (V-EV-DGRAM)
 *   F13 The forwarder's abort of a watch sends nothing on the vsock and changes no socket state (the delta's third-
 *       iteration "F12") ........................... module code; P-70 § 3.8 (the spike's ABORT); the module suite
 *   F14 A vsock CLOSING is set only on a connected socket, when a packet leaves peer_shutdown == SHUTDOWN_MASK with the
 *       receive queue empty (virtio_transport_common.c:1448-1452, :1463-1464, :1249-1252 @ v7.0); the sweep's sk_err is
 *       set before its state write (drivers/vhost/vsock.c:746-750) (the delta's "F13") ... source reading; P-70 § 3.6, P-71
 *
 * OFF names the rules switched off (teeth):
 *   consumeOnShortSend   a short send consumes the whole peek; the unaccepted unit is lost
 *   eofAsFin             every vsock end is forwarded as a FIN (no abort class)
 *   shutdownOnCallback   the target is shut in the callback, before the vsock queue is empty
 *   releaseKeepsPairs    the context's release leaves its pairs forwarding
 *   lossyEndRecord       an end record may be dropped
 *   consumePartialFrame  a datagram is consumed when the vsock accepted part of its frame
 *   checkThenHook        attach reads the half's state, and installs the callbacks in a later step: a transition
 *                        between them is recorded by neither (AttachRead / AttachHook)
 *   holderTrustsRead     the GuestStream reports the holder's read of V_h (Eof → HalfClose, Err → GuestAbort), not the
 *                        watch's, and passes reads to V_h after the owner ended it
 *   endedAttachGracefulClose (T-4) on HalfEnded the owner closes the surviving TCP half without SO_LINGER{1,0}
 *   watchAbortShutsVh    (T-5, the rejected EVMOD-WATCH-SHUT) the owner's abort of a watch shuts V_h
 *   dropWithoutAbort     (T-6) the holder's drop only closes V_h; the forwarder's reference keeps it open
 */
// D-UP-1: the variants of `conf_step` (a module of its own: a constructor would clash with the action of the same
// name, so the actions build their variant with the builtin `variant`)
module fwd_pair_conf {
  type ConfStep =
    | InitWith({ m: str })
    | Attach({ s: str })
    | Write({ d: str })
    | Fin({ d: str })
    | Read({ d: str })
    | ReadEof({ d: str })
    | ShutRd({ d: str })
    | TcpRst({ s: str, after_fin: bool })
    | HolderDrop
    | Hop0({ d: str })
    | Hop1({ d: str })
    | HopEnd0({ d: str })
    | HopEnd1({ d: str })
    | VCtl({ d: str })
    | FwdErr({ s: str })
    | Abort({ s: str })
    | CtxRelease({ s: str })
    | TakeEnd({ s: str })
    | OwnerClose({ s: str })
    | TrailingRst({ s: str })
    | VmmExit
    | SendDg({ d: str, sz: int })
    | DRead({ d: str })
    | UdpBlock({ d: str })
    | UdpUnblock({ d: str })
    | UdpErr({ s: str })
    | DropRecord({ s: str })
    | AttachRead({ s: str })
    | AttachHook({ s: str })
    | CloseUnattached({ s: str })
    | Reopen({ s: str })
    | UnclassifiedEof({ d: str })
    | HolderEnd({ k: str })
    | WatchRecord({ s: str })
    | DgHalfClose({ s: str })
    | GuestAbort({ s: str })
}

module fwd_pair {
  import fwd_pair_conf.ConfStep from "./fwd_pair"
  // OFF: design rules switched off (hazard instances in hazard/fwd_pair*_hazards.qnt); CFG: "ok" | "watch" | "live" |
  // "stall" | "dgram" | "dlive" | "mid" | "dhaz" | "conf"
  const OFF: Set[str]
  const CFG: str
  // D-UP-1: true only on the conformance main `pair_conf`; every model-checking and hazard instance passes false
  // and keeps `conf_step` at its initial value
  const REC: bool

  // ------------------------------------------------------------------ parameters
  val SIDES: Set[str] = Set("g", "h")
  val DIRS: Set[str] = Set("gh", "hg")
  val MODES: Set[str] =
    if (CFG == "ok" or CFG == "live" or CFG == "stall") Set("stream")
    else if (CFG == "watch" or CFG == "wlive") Set("watch")
    else if (CFG == "dgram" or CFG == "dlive") Set("dgram")
    else if (CFG == "mid") Set("stream", "dgram")
    else if (CFG == "dhaz") Set("dgram")
    else Set("stream", "watch", "dgram")
  val W: int = 1    // a TCP socket's queue (units)
  val C: int = 1    // the vsock credit in stream mode (units)
  val CD: int = 3   // the vsock credit in datagram mode (frame items): at least the largest frame (F8)
  val WU: int = 1   // a UDP socket's receive queue (datagrams)
  val SIZES: Set[int] = Set(0, 1, 2)
  // units each writer may write: "ok", "live" and "watch" 1 gh / 1 hg (one unit per direction), "stall" 4 gh (more than
  // the pipeline holds: a stopped reader blocks the writer) / 0 hg, "wlive" 1 gh / 0 hg, "mid" 2 gh / 0 hg, "conf" 4 / 4
  def nmax(d: str): int =
    if (CFG == "conf") 4
    else if (CFG == "stall") (if (d == "gh") 4 else 0)
    else if (CFG == "wlive") (if (d == "gh") 1 else 0)
    else if (CFG == "mid") (if (d == "gh") 2 else 0)
    else 1
  // environment faults per trace (EpRst, DialerDrop, VmmExit, UdpErr) and owner aborts (Abort, CtxRelease): one on
  // the model-checking instances, which keeps them finite and small; none on "stall"; unbounded on the
  // conformance main
  val FMAX: int = if (CFG == "conf") 1000 else if (CFG == "stall") 0 else 1
  // datagrams each source may send: "dgram" 1 gh / 1 hg (one datagram per direction), "conf" 3 / 3,
  // "dlive" (the datagram liveness instance) 1 / 0, "mid" 2 / 0, "dhaz" 3 / 0
  def dnmax(d: str): int =
    if (CFG == "conf") 3 else if (CFG == "dgram") 1
    else if (CFG == "dlive") (if (d == "gh") 1 else 0)
    else if (CFG == "mid") (if (d == "gh") 2 else 0)
    else if (CFG == "dhaz") (if (d == "gh") 3 else 0) else 0

  val BUG_CONSUME_ON_SHORT_SEND: bool = OFF.contains("consumeOnShortSend")
  val BUG_EOF_AS_FIN: bool = OFF.contains("eofAsFin")
  val BUG_SHUT_ON_CALLBACK: bool = OFF.contains("shutdownOnCallback")
  val BUG_RELEASE_KEEPS: bool = OFF.contains("releaseKeepsPairs")
  val BUG_LOSSY_RECORD: bool = OFF.contains("lossyEndRecord")
  val BUG_CONSUME_PARTIAL: bool = OFF.contains("consumePartialFrame")
  val BUG_CHECK_THEN_HOOK: bool = OFF.contains("checkThenHook")
  val BUG_WATCH_SHUTS_VH: bool = OFF.contains("watchAbortShutsVh")
  val BUG_ENDED_GRACEFUL: bool = OFF.contains("endedAttachGracefulClose")
  val BUG_DROP_WITHOUT_ABORT: bool = OFF.contains("dropWithoutAbort")
  val BUG_HOLDER_TRUSTS_READ: bool = OFF.contains("holderTrustsRead")

  // ------------------------------------------------------------------ types
  // ph: unattached | attaching (checkThenHook only) | live | ended | refused (attach returned HalfEnded: no pair was
  // created and the owner ended the flow, EVMOD-ATTACH-END) | gone (the VM is gone with the side's state)
  // rec: none | queued | read | lost (lossyEndRecord only); out: none | completed | aborted | halfclose (watch)
  // cause (aborted): PeerAbort | SocketError | FrameInvalid | Detached | Unclassified (the unclassified_eof
  // alarm: AbortCause::Unclassified, record cause 5, U-EV-7, EVMOD-END) | HalfEnded (the owner's end of a refused flow; no
  // record); errno: the SocketError errno of a TCP reset (ECONNRESET | EPIPE, T-1), else none; byRel: ended with no
  // record — by its context's release, by the owner's close of a flow it never attached, or by a refused attach (no
  // id exists); closed: the owner closed its descriptors; nrec: end records produced; cgen: the context (by `gen`)
  // the pair was attached in
  type Pair = { ph: str, rec: str, out: str, cause: str, errno: str, byRel: bool, closed: bool, nrec: int, cgen: int }
  // Per direction d. Writer at srcOf(d): wr units written, wfin its FIN (in mode watch, hg's writer is the dialer:
  // it writes q1 directly). q0 / q1 / q2 as above.
  // srcEnd / srcShut: at the source side's pair, the TCP source's end ("none" | "clean") and the vsock target
  // shut (an {S} packet sent). dstEnd / dstShut: at the target side's pair, the vsock source's end and the TCP
  // target shut (a FIN toward the reader). Reader at dstOf(d): got the units read, eof end-of-stream read, rst a
  // reset observed, shutRd shutdown(SHUT_RD). Ghosts: loss, lost.
  type Dir = { wr: int, wfin: bool, q0: List[int], q1: List[int], q2: List[int],
               srcEnd: str, srcShut: bool, dstEnd: str, dstShut: bool,
               got: List[int], eof: bool, rst: bool, shutRd: bool, loss: bool, lost: int }
  // The vsock half of a side: ps = peer_shutdown (flags received), sent = flags this side sent, err = sk_err
  // (the device release), cls = the class the callback recorded for the incoming direction ("none" | "clean").
  // Ghosts: tcls, ucls ("none" | "clean" | "abort"); att ("none" before attach | "end" | "noend": whether an end
  // was present when the callbacks were installed); clsAt ("none" | "attach" | "callback" | "eof": where the end
  // of the incoming direction was first classified); ncls (how many times it was); snap (checkThenHook: the state
  // the attach read).
  // st (vsSt, T-4): "est" | "closing" — CLOSING is set when a packet leaves peer_shutdown == SHUTDOWN_MASK with the
  // receive queue empty (an RST, or a 0x3 with nothing queued); with units queued the socket stays ESTABLISHED.
  // err is vsSt's companion vsErr (the orphan sweep's sk_err). sshut is ownShut (T-8).
  type Vs = { ps: Set[str], sent: Set[str], err: bool, cls: str, tcls: str, ucls: str,
              att: str, clsAt: str, ncls: int, snap: Set[str], sshut: bool, misc: bool, st: str }
  // Datagram mode, per direction: n datagrams sent, sizes[k-1] the size of datagram k, dq0 the source UDP queue,
  // foff the current frame's items already accepted, blk the target's send would block, dq2 the far receiver's
  // queue (whole frames), got the frames read. Ghosts: sdrop (dropped at a source queue: full, or discarded at a
  // slot release), rdrop (dropped at the far receiver's full queue), fdrop (dropped by the forwarder).
  type Dg = { n: int, sizes: List[int], dq0: List[int], foff: int, blk: bool, dq2: List[List[int]],
              got: List[List[int]], sdrop: Set[int], rdrop: Set[int], fdrop: Set[int] }
  // hobs: the holder's end observation ("none" | "eof" | "err"); woken: a holder woken by a watch's end;
  // vhTouched: an owner abort of a watch changed V_h (T-5); dropped: the holder dropped
  // its GuestStream (T-6)
  // hostDead: the host owner died (a CtxRelease(h) that found its pair or watch live): final in the trace, no Reopen
  // hresCls: the class of the watch (watchClass) at the moment the GuestStream reported its end (hres)
  type Gh = { fwdAfterEnd: bool, fwdOld: bool, unclassified: bool, rstOpen: Set[str], holderReadAfterAbort: bool,
              trail: Set[str], faults: int, hobs: str, woken: bool, vhTouched: bool, dropped: bool,
              hostDead: bool, hresCls: str }
  type St = { mode: str, vmUp: bool, ctx: str -> str, gen: str -> int, oldCtx: str -> str, pr: str -> Pair,
              dir: str -> Dir, vs: str -> Vs, ctl: str -> List[Set[str]], terr: str -> bool, ep: str -> str,
              dg: str -> Dg, gh: Gh, tcpErr: str -> str, attRes: str -> str, attEnded: str -> int,
              strmAbort: bool, hres: str, guestAbortSent: bool }

  // ------------------------------------------------------------------ state
  var mode: str                   // stream | watch | dgram
  var vmUp: bool                  // the VM (and the guest side) is running
  var ctx: str -> str             // side -> the owner's current context: open | released | gone
  var gen: str -> int             // side -> the number of the owner's current context (0, or 1 after a recovery)
  var oldCtx: str -> str          // side -> the context the current one replaced: none | released | open (hazard)
  var pr: str -> Pair             // side -> its pair (mode watch: pr.get("h") is the watch)
  var dir: str -> Dir             // direction -> its queues, ends, writer and reader
  var vs: str -> Vs               // side -> its vsock half
  var ctl: str -> List[Set[str]]  // direction -> SHUTDOWN / RST packets from srcOf(d) not yet processed at dstOf(d)
  var terr: str -> bool           // side -> its UDP half has a pending error (datagram mode)
  var tcpErr: str -> str          // side -> its TCP half's pending reset error: "0" | "ECONNRESET" | "EPIPE" (T-1)
  var attRes: str -> str          // side -> the attach (watch) result its owner received: none | ok | halfEnded (T-4)
  var attEnded: str -> int        // side -> forwarder_attach_ended: the owner's count of HalfEnded refusals (T-4)
  var strmAbort: bool             // the dial's GuestStream was ended by the host owner (T-5)
  var hres: str                   // the outcome the GuestStream reported: none | HalfClose | GuestAbort | OwnerAbort
  var guestAbortSent: bool        // Abort { flow } written to the guest and not yet taken (model-only, T-5)
  var ep: str -> str              // side -> its far end (guest application / far peer / dialer): alive | dead | reset | gone
  var dg: str -> Dg               // direction -> datagram-mode state
  var gh: Gh                      // ghosts
  var conf_step: ConfStep         // D-UP-1: the action that produced this state (model-only)

  // ------------------------------------------------------------------ helpers
  pure def srcOf(d: str): str = if (d == "gh") "g" else "h"
  pure def dstOf(d: str): str = if (d == "gh") "h" else "g"
  pure def outof(s: str): str = if (s == "g") "gh" else "hg"
  pure def into(s: str): str = if (s == "g") "hg" else "gh"
  pure def other(s: str): str = if (s == "g") "h" else "g"
  pure def ids(l: List[int]): Set[int] = l.foldl(Set(), (acc, x) => if (x % 10 == 0) acc.union(Set(x / 10)) else acc)
  pure def frameIds(l: List[List[int]]): Set[int] =
    l.foldl(Set(), (acc, f) => if (f.length() > 0) acc.union(Set(f[0] / 10)) else acc)
  pure def lset(l: List[int]): Set[int] = l.foldl(Set(), (acc, x) => acc.union(Set(x)))
  pure val both: Set[str] = Set("S", "R")

  val cur: St = { mode: mode, vmUp: vmUp, ctx: ctx, gen: gen, oldCtx: oldCtx, pr: pr, dir: dir, vs: vs, ctl: ctl,
                  terr: terr, ep: ep, dg: dg, gh: gh, tcpErr: tcpErr, attRes: attRes, attEnded: attEnded,
                  strmAbort: strmAbort, hres: hres, guestAbortSent: guestAbortSent }
  action commit(n: St, c: ConfStep): bool = all {
    mode' = n.mode, vmUp' = n.vmUp, ctx' = n.ctx, gen' = n.gen, oldCtx' = n.oldCtx, pr' = n.pr, dir' = n.dir,
    vs' = n.vs, ctl' = n.ctl,
    terr' = n.terr, ep' = n.ep, dg' = n.dg, gh' = n.gh, tcpErr' = n.tcpErr, attRes' = n.attRes,
    attEnded' = n.attEnded, strmAbort' = n.strmAbort, hres' = n.hres, guestAbortSent' = n.guestAbortSent,
    conf_step' = if (REC) c else conf_step,
  }

  def sideUp(t: St, s: str): bool = s == "h" or t.vmUp
  // one more fault of the per-trace budget
  def canFault(t: St): bool = t.gh.faults < FMAX
  def fault(t: St): St = { ...t, gh: { ...t.gh, faults: t.gh.faults + 1 } }
  def watchSide(t: St, s: str): bool = t.mode == "watch" and s == "h"
  def live(t: St, s: str): bool = t.pr.get(s).ph == "live"
  // the pair of side s belongs to the owner's current, open context
  def pairCur(t: St, s: str): bool = t.pr.get(s).cgen == t.gen.get(s) and t.ctx.get(s) == "open"
  // the end of side s's incoming direction is classified, here (ghost)
  def noteCls(t: St, s: str, where: str): St =
    val v = t.vs.get(s)
    { ...t, vs: t.vs.set(s, { ...v, ncls: v.ncls + 1, clsAt: if (v.clsAt == "none") where else v.clsAt }) }
  def setDir(t: St, d: str, x: Dir): St = { ...t, dir: t.dir.set(d, x) }
  def setVs(t: St, s: str, v: Vs): St = { ...t, vs: t.vs.set(s, v) }
  def setPr(t: St, s: str, p: Pair): St = { ...t, pr: t.pr.set(s, p) }
  def setDg(t: St, d: str, x: Dg): St = { ...t, dg: t.dg.set(d, x) }
  // a packet from side s toward its peer
  // (CLEAN-FULL-END: this side's send-shut flag is set in the same step as the SHUTDOWN it sends — vsock_shutdown
  // sets sk_shutdown before the packet leaves, af_vsock.c:1203, :1216, :1221 @ v7.0)
  def send(t: St, s: str, pkt: Set[str]): St =
    val v = t.vs.get(s)
    val shutNow = pkt.contains("S")
    { ...t, ctl: t.ctl.set(outof(s), t.ctl.get(outof(s)).append(pkt)),
      vs: t.vs.set(s, { ...v, sent: v.sent.union(pkt), sshut: v.sshut or shutNow }) }
  // SHUT_RDWR / close of side s's vsock half: a {S, R} packet unless both flags were already sent
  def shutAll(t: St, s: str): St = if (t.vs.get(s).sent == both) t else send(t, s, both)
  // the stream of direction d is cut before its writer's FIN (ghost)
  def cutDir(t: St, d: str): St =
    setDir(t, d, { ...t.dir.get(d), loss: t.dir.get(d).loss or not(t.dir.get(d).wfin) })
  def cutBoth(t: St): St = cutDir(cutDir(t, "gh"), "hg")
  // a record queued for side s's pair or watch
  def ended(p: Pair, out: str, cause: str, byRel: bool): Pair =
    { ...p, ph: "ended", out: out, cause: cause, byRel: byRel,
      rec: if (byRel) "none" else "queued", nrec: if (byRel) p.nrec else p.nrec + 1 }

  // The EVMOD-END table, per transition: `nw` the flags newly set by the packet, `ps` the flags before it,
  // `sends` this side still sends (it has not shut its send direction).
  pure def classify(nw: Set[str], ps: Set[str], sends: bool): str =
    if (nw.contains("S") and not(nw.contains("R")) and not(ps.contains("R"))) "clean"
    else if (nw.contains("S")) "abort"
    else if (nw.contains("R") and sends) "abort"
    else "none"

  // The abort of side s's pair (or watch): tcp_abort resets the TCP half's far end (F5); SHUT_RDWR on the vsock
  // half; one record unless the context was released. A UDP half sends nothing. A context's release also ends a
  // flow its owner has not attached yet: the owner's own descriptors close, the TCP half with SO_LINGER{1,0} (D20)
  // — a reset — and the vsock half with {S, R}; no record (no id exists).
  def abortSide(t: St, s: str, cause: str, byRel: bool): St =
    if (not(live(t, s)) and not(byRel and t.pr.get(s).ph.in(Set("unattached", "attaching")))) t
    else if (watchSide(t, s))
      // WATCH-ABORT: the owner's abort of a watch queues its record, restores the callbacks and drops the module's
      // reference, and changes nothing of V_h (F13; watchAbortShutsVh: it shuts V_h, the
      // rejected EVMOD-WATCH-SHUT); only the context's release shuts a watched V_h (SHUT_RDWR: the guest sees 0x3)
      val w1 = cutBoth(setPr(t, s, ended(t.pr.get(s), "aborted", cause, byRel)))
      if (byRel or BUG_WATCH_SHUTS_VH) shutAll(w1, s) else w1
    else
      val hit = t.mode != "dgram" and t.ep.get(s) == "alive"
      val t1 = setPr(t, s, ended(t.pr.get(s), "aborted", cause, byRel))
      val t2 = if (hit) { ...t1, ep: t1.ep.set(s, "reset") } else t1
      val t3 = if (hit) setDir(t2, into(s), { ...t2.dir.get(into(s)), rst: true }) else t2
      val t4 = if (t.mode == "dgram") t3 else cutBoth(t3)
      shutAll(t4, s)
  def complete(t: St, s: str): St =
    if (live(t, s)) setPr(t, s, ended(t.pr.get(s), "completed", "none", false)) else t
  def completeIfDone(t: St, s: str): St =
    if (t.dir.get(into(s)).dstEnd == "clean" and t.dir.get(outof(s)).srcEnd == "clean") complete(t, s) else t
  // the callback recorded a clean half-close of the incoming direction at side s
  def markClean(t: St, s: str): St =
    val t1 = setVs(t, s, { ...t.vs.get(s), cls: "clean", ucls: "clean" })
    if (watchSide(t1, s)) setPr(t1, s, ended(t1.pr.get(s), "halfclose", "none", false))
    else if (t1.mode == "dgram") (if (t1.dir.get(into(s)).q1.length() == 0) complete(t1, s) else t1)
    else if (BUG_SHUT_ON_CALLBACK) setDir(t1, into(s), { ...t1.dir.get(into(s)), dstShut: true })
    else t1
  def withUcls(t: St, s: str, nw: Set[str]): St =
    if (nw.contains("S")) setVs(t, s, { ...t.vs.get(s), ucls: "abort" }) else t
  // a peer abort: a pair aborts; a watch only records it (the dialer holds V_h)
  def peerAbort(t: St, s: str): St =
    if (watchSide(t, s)) setPr(t, s, ended(t.pr.get(s), "aborted", "PeerAbort", false))
    else abortSide(t, s, "PeerAbort", false)

  // ------------------------------------------------------------------ init
  def dir0: Dir = { wr: 0, wfin: false, q0: List(), q1: List(), q2: List(),
                    srcEnd: "none", srcShut: false, dstEnd: "none", dstShut: false,
                    got: List(), eof: false, rst: false, shutRd: false, loss: false, lost: 0 }
  def pair0: Pair = { ph: "unattached", rec: "none", out: "none", cause: "none", errno: "none", byRel: false,
                      closed: false, nrec: 0, cgen: 0 }
  def vs0: Vs = { ps: Set(), sent: Set(), err: false, cls: "none", tcls: "none", ucls: "none",
                  att: "none", clsAt: "none", ncls: 0, snap: Set(), sshut: false, misc: false, st: "est" }
  def dg0: Dg = { n: 0, sizes: List(), dq0: List(), foff: 0, blk: false, dq2: List(), got: List(),
                  sdrop: Set(), rdrop: Set(), fdrop: Set() }
  action InitWith(m: str): bool = all {
    mode' = m, vmUp' = true,
    ctx' = SIDES.mapBy(s => "open"),
    gen' = SIDES.mapBy(s => 0),
    oldCtx' = SIDES.mapBy(s => "none"),
    pr' = SIDES.mapBy(s => if (m == "watch" and s == "h") { ...pair0, ph: "live" } else pair0),
    dir' = DIRS.mapBy(d => dir0),
    vs' = SIDES.mapBy(s => vs0),
    ctl' = DIRS.mapBy(d => List()),
    terr' = SIDES.mapBy(s => false),
    ep' = SIDES.mapBy(s => "alive"),
    dg' = DIRS.mapBy(d => dg0),
    gh' = { fwdAfterEnd: false, fwdOld: false, unclassified: false, rstOpen: Set(), holderReadAfterAbort: false,
            trail: Set(), faults: 0, hobs: "none", woken: false, vhTouched: false, dropped: false,
            hostDead: false, hresCls: "none" },
    tcpErr' = SIDES.mapBy(s => "0"),
    attRes' = SIDES.mapBy(s => if (m == "watch" and s == "h") "ok" else "none"),
    attEnded' = SIDES.mapBy(s => 0),
    strmAbort' = false, hres' = "none", guestAbortSent' = false,
    conf_step' = variant("InitWith", { m: m }),
  }
  action init = { nondet m = MODES.oneOf() InitWith(m) }

  // ------------------------------------------------------------------ owners (EVMOD-PAIR, EVMOD-CTL)
  // Attach at the pinned point: the host first (before Paired), the guest on Paired. The callbacks are installed
  // and the half's state `ps` read in one step (F9); an end present then is classified there, once, with this
  // side's send open ({S} only → clean, any R → abort), and the attach-time run forwards anything queued.
  // (checkThenHook: `ps` is the state read by an earlier AttachRead.)
  // The TCP half's state (tcpSt, T-3): "ended" (a pending reset error), "closeWait" (the half received its far
  // peer's FIN and the forwarder has not sent its own: CLOSE_WAIT), "est"; "none" for a UDP half or a watch. The
  // fourth state the design names, "defect" (never connected, listening, self-shut: ENOTCONN, an owner defect), has
  // no producing action in the model.
  def tcpSt(t: St, s: str): str =
    if (t.mode == "dgram" or watchSide(t, s)) "none"
    else if (t.tcpErr.get(s) != "0") "ended"
    else if (t.dir.get(outof(s)).wfin and not(t.dir.get(into(s)).dstShut)) "closeWait"
    else "est"
  // EVMOD-ATTACH-END: attach refuses a half that has ended — a pending sk_err on the TCP half (a reset) or on the
  // vsock half (the orphan sweep), or a vsock CLOSING; it accepts TCP ESTABLISHED and CLOSE_WAIT and a vsock
  // ESTABLISHED half with any peer bits
  def halfEnded(t: St, s: str): bool = or {
    tcpSt(t, s) == "ended",
    t.vs.get(s).err, t.vs.get(s).st == "closing",
  }
  // HalfEnded: no pair is created and no record queued; the owner, in the same dispatch, ends the flow by its
  // pre-Paired close — the TCP half with SO_LINGER{1,0} (a reset toward its far end; endedAttachGracefulClose: a
  // graceful close, a FIN), the vsock half closed ({S, R}); the host also writes Abort { flow } while the session
  // is live — and counts forwarder_attach_ended
  def refuseEff(t: St, s: str): St =
    val t0 = { ...t, attRes: t.attRes.set(s, "halfEnded"), attEnded: t.attEnded.set(s, t.attEnded.get(s) + 1) }
    val t1 = setPr(t0, s, { ...t0.pr.get(s), ph: "refused", out: "aborted", cause: "HalfEnded", byRel: true })
    val hit = t1.mode != "dgram" and t1.ep.get(s) == "alive"
    val t2 =
      if (not(hit)) t1
      else if (BUG_ENDED_GRACEFUL) setDir(t1, into(s), { ...t1.dir.get(into(s)), dstShut: true })
      else setDir({ ...t1, ep: t1.ep.set(s, "reset") }, into(s), { ...t1.dir.get(into(s)), rst: true })
    val t3 = shutAll(if (t2.mode == "dgram") t2 else cutBoth(t2), s)
    if (s == "h" and t3.vmUp) { ...t3, guestAbortSent: true } else t3
  def attachEff(t: St, s: str, ps: Set[str]): St =
    if (halfEnded(t, s)) refuseEff(t, s)
    else
    val v = t.vs.get(s)
    val t0 = setVs({ ...t, attRes: t.attRes.set(s, "ok") }, s, { ...v, att: if (v.ps == Set()) "noend" else "end" })
    val t1 = setPr(t0, s, { ...t0.pr.get(s), ph: "live", cgen: t0.gen.get(s) })
    if (ps == Set()) t1
    else if (BUG_EOF_AS_FIN) (if (ps.contains("S")) markClean(noteCls(t1, s, "attach"), s) else t1)
    else if (ps.contains("R")) abortSide(noteCls(t1, s, "attach"), s, "PeerAbort", false)
    else markClean(noteCls(t1, s, "attach"), s)
  // (the guest attaches on Paired: never after the host's attach was refused — the host then writes Abort { flow })
  def canAttach(t: St, s: str): bool = and {
    not(watchSide(t, s)), t.ctx.get(s) == "open", t.pr.get(s).ph == "unattached", sideUp(t, s),
    t.attRes.get(s) == "none",
    s == "h" or not(t.pr.get("h").ph.in(Set("unattached", "attaching", "refused"))),
  }
  action Attach(s: str): bool = all {
    not(BUG_CHECK_THEN_HOOK), canAttach(cur, s),
    commit(attachEff(cur, s, cur.vs.get(s).ps), variant("Attach", { s: s })),
  }
  // checkThenHook: the state is read in one step and the callbacks installed in a later one
  action AttachRead(s: str): bool = all {
    BUG_CHECK_THEN_HOOK, canAttach(cur, s),
    commit(setVs(setPr(cur, s, { ...cur.pr.get(s), ph: "attaching" }), s, { ...cur.vs.get(s), snap: cur.vs.get(s).ps }),
           variant("AttachRead", { s: s })),
  }
  def canAttachHook(t: St, s: str): bool = t.pr.get(s).ph == "attaching" and sideUp(t, s) and t.ctx.get(s) == "open"
  action AttachHook(s: str): bool = all {
    canAttachHook(cur, s),
    commit(attachEff(cur, s, cur.vs.get(s).snap), variant("AttachHook", { s: s })),
  }
  // the guest owner closes a flow it has not attached (a pending intake child that reset, the pairing deadline,
  // session loss): its TCP half with SO_LINGER{1,0} (D20, a reset toward the application) and V_g ({S, R}); no
  // record (no id exists)
  action CloseUnattached(s: str): bool = all {
    s == "g", cur.pr.get(s).ph == "unattached", sideUp(cur, s), cur.ctx.get(s) == "open", canFault(cur),
    commit(abortSide(fault(cur), s, "Detached", true), variant("CloseUnattached", { s: s })),
  }
  // the owner's abort (quiescence, teardown, control-session loss): Detached. On a watch (EVMOD-STREAM-ABORT) it is
  // one owner step: abort(watch) (record queued, V_h untouched), the stream marked ended by the owner, and
  // Abort { flow } written while the session is live
  def ownerAbort(t: St, s: str): St =
    val t1 = abortSide(fault(t), s, "Detached", false)
    if (not(watchSide(t, s))) t1
    else
      val touched = t1.vs.get("h") != t.vs.get("h") or t1.ctl.get("hg") != t.ctl.get("hg") or
                    t1.dir.get("gh").q1 != t.dir.get("gh").q1 or t1.dir.get("hg").q1 != t.dir.get("hg").q1
      { ...t1, strmAbort: true, guestAbortSent: t1.vmUp,
        gh: { ...t1.gh, vhTouched: t1.gh.vhTouched or touched } }
  action Abort(s: str): bool = all {
    live(cur, s), sideUp(cur, s), canFault(cur),
    commit(ownerAbort(cur, s), variant("Abort", { s: s })),
  }
  // the guest owner takes Abort { flow } (owner_flows' toGuestAbort): it aborts the flow's pair, or closes the flow it
  // has not attached (no record)
  def canGuestAbort(t: St): bool =
    t.guestAbortSent and t.vmUp and t.ctx.get("g") == "open" and t.pr.get("g").ph.in(Set("live", "unattached"))
  action GuestAbort(s: str): bool = all {
    s == "g", canGuestAbort(cur),
    val t1 = abortSide(cur, s, "Detached", cur.pr.get(s).ph == "unattached")
    commit({ ...t1, guestAbortSent: false }, variant("GuestAbort", { s: s })),
  }
  // The context's release aborts every pair of the context, with no record (F6). Side h, one meaning chosen by the
  // state (D-5, architect 2026-10-18): with the host pair or watch live it is the host owner's death — final in the
  // trace, no Reopen follows (production has no in-process successor); with none live it is the Forwarder recovery's
  // close of the old context, after its step 1 ended every flow in the owner's table (§ Context lifetime and owner
  // failure): a non-watch flow whose record is queued and unread has its descriptors closed then (OwnerClose's
  // effect; its record stays queued in the released context and is never read).
  // U-R1: a held GuestStream whose watch already ended is ended as an owner abort (its dials entry leaves): the
  // stream is marked ended by the owner and Abort { flow } is written while the session is live; hres stays
  def recoveryClose(t: St): St =
    val p = t.pr.get("h")
    if (not(watchSide(t, "h")) and p.ph == "ended" and p.rec == "queued" and not(p.closed))
      shutAll(setPr(t, "h", { ...p, closed: true }), "h")
    else if (watchSide(t, "h") and p.ph == "ended" and t.ep.get("h") == "alive" and not(t.strmAbort))
      { ...t, strmAbort: true, guestAbortSent: t.vmUp }
    else t
  action CtxRelease(s: str): bool = all {
    cur.ctx.get(s) == "open", sideUp(cur, s), canFault(cur),
    val death = live(cur, s)
    val t1 = fault({ ...cur, ctx: cur.ctx.set(s, "released"),
                     gh: { ...cur.gh, hostDead: cur.gh.hostDead or (s == "h" and death) } })
    val t2 = if (BUG_RELEASE_KEEPS and live(t1, s)) t1 else abortSide(t1, s, "Detached", true)
    commit(if (s == "h" and not(death)) recoveryClose(t2) else t2, variant("CtxRelease", { s: s })),
  }
  // the host's Forwarder recovery (EVMOD-CTL): after abort-all and the close of the old context, the owner opens
  // and probes a new one; at most once per trace, never after the host owner's death
  action Reopen(s: str): bool = all {
    s == "h", cur.gen.get(s) == 0, not(cur.gh.hostDead),
    cur.ctx.get(s) == "released",
    commit({ ...cur, ctx: cur.ctx.set(s, "open"), oldCtx: cur.oldCtx.set(s, cur.ctx.get(s)), gen: cur.gen.set(s, 1) },
           variant("Reopen", { s: s })),
  }
  // the owner reads the end record (of its current context only: a record queued in a replaced context is never
  // read)
  def canTakeEnd(t: St, s: str): bool =
    t.pr.get(s).rec == "queued" and sideUp(t, s) and t.ctx.get(s) == "open" and t.pr.get(s).cgen == t.gen.get(s)
  // after the record, the owner closes its descriptors: a completed TCP half's linger is cleared first (D20); the
  // vsock close sends {S, R}; in datagram mode the guest owner discards the slot socket's queue without reading it
  def closeEff(t: St, s: str): St =
    val t1 = shutAll(setPr(t, s, { ...t.pr.get(s), closed: true }), s)
    val x = t1.dg.get(outof(s))
    if (t1.mode == "dgram" and s == "g")
      setDg(t1, outof(s), { ...x, dq0: List(), foff: 0, sdrop: x.sdrop.union(lset(x.dq0)) })
    else t1
  // D-4 (architect 2026-10-18): the host owner reads the record and closes the flow's descriptors in ONE dispatch,
  // so outside watch mode TakeEnd(h) performs OwnerClose(h)'s effect in the same step (in watch mode the holder owns
  // V_h); the guest's close stays a dispatch of its own
  action TakeEnd(s: str): bool = all {
    canTakeEnd(cur, s),
    val t1 = setPr(cur, s, { ...cur.pr.get(s), rec: "read" })
    commit(if (s == "h" and not(watchSide(cur, s))) closeEff(t1, s) else t1, variant("TakeEnd", { s: s })),
  }
  action OwnerClose(s: str): bool = all {
    s == "g", cur.pr.get(s).rec == "read", not(cur.pr.get(s).closed), sideUp(cur, s),
    commit(closeEff(cur, s), variant("OwnerClose", { s: s })),
  }
  // the lossyEndRecord hazard: a queued end record is dropped
  action DropRecord(s: str): bool = all {
    BUG_LOSSY_RECORD, cur.pr.get(s).rec == "queued",
    commit(setPr(cur, s, { ...cur.pr.get(s), rec: "lost" }), variant("DropRecord", { s: s })),
  }

  // ------------------------------------------------------------------ the forwarder (EVMOD forwarding, EVMOD-END)
  // a run's send to the vsock fails once the peer shut its receive side (EPIPE)
  def sendErr(t: St, s: str): bool =
    t.mode != "dgram" and not(watchSide(t, s)) and t.vs.get(s).ps.contains("R") and
    t.dir.get(outof(s)).q0.length() > 0
  // A TCP half's reset is reported by its sk_error_report in the step that sets it (TcpRst, T-1), so no forwarder
  // run finds it. A UDP half's error (terr) is found by the next run.
  def errPending(t: St, s: str): bool = t.terr.get(s) or t.vs.get(s).err or sendErr(t, s)
  def fwdGhost(t: St, s: str): St =
    { ...t, gh: { ...t.gh, fwdAfterEnd: t.gh.fwdAfterEnd or not(pairCur(t, s)),
                  fwdOld: t.gh.fwdOld or t.pr.get(s).cgen != t.gen.get(s) } }

  // stream: the source side's run moves one unit from the TCP source (or its memory) to the vsock
  def canHop0S(t: St, d: str): bool =
    val s = srcOf(d)
    val x = t.dir.get(d)
    and {
      t.mode != "dgram", not(watchSide(t, s)), live(t, s), sideUp(t, s), not(errPending(t, s)),
      not(x.srcShut), x.q0.length() > 0,
      x.q1.length() < C or (BUG_CONSUME_ON_SHORT_SEND and x.q0.length() > 0),
    }
  def hop0S(t: St, d: str): St =
    val x = t.dir.get(d)
    val x2 =
      if (x.q1.length() < C) { ...x, q0: x.q0.tail(), q1: x.q1.append(x.q0.head()) }
      else { ...x, q0: x.q0.tail(), lost: x.lost + 1 }
    fwdGhost(setDir(t, d, x2), srcOf(d))
  // datagram: the source side's run sends the next item of the head datagram's frame; the datagram is consumed
  // only once its whole frame was accepted (consumePartialFrame: once the credit ran out mid-frame)
  def canHop0D(t: St, d: str): bool =
    val s = srcOf(d)
    and {
      t.mode == "dgram", live(t, s), sideUp(t, s), not(errPending(t, s)), not(t.vs.get(s).ps.contains("R")),
      not(t.vs.get(s).sent.contains("S")), t.dg.get(d).dq0.length() > 0, t.dir.get(d).q1.length() < CD,
    }
  def hop0D(t: St, d: str): St =
    val x = t.dir.get(d)
    val y = t.dg.get(d)
    val k = y.dq0.head()
    val sz = y.sizes[k - 1]
    val q1n = x.q1.append(10 * k + y.foff)
    val done = y.foff + 1 == sz + 1
    val cut = BUG_CONSUME_PARTIAL and not(done) and q1n.length() == CD
    val y2 = if (done or cut) { ...y, dq0: y.dq0.tail(), foff: 0 } else { ...y, foff: y.foff + 1 }
    fwdGhost(setDg(setDir(t, d, { ...x, q1: q1n }), d, y2), srcOf(d))
  action Hop0(d: str): bool = all {
    canHop0S(cur, d) or canHop0D(cur, d),
    commit(if (cur.mode == "dgram") hop0D(cur, d) else hop0S(cur, d), variant("Hop0", { d: d })),
  }

  // stream: the target side's run moves one unit from the vsock (or its memory) to the TCP target
  def canHop1S(t: St, d: str): bool =
    val s = dstOf(d)
    val x = t.dir.get(d)
    and {
      t.mode != "dgram", not(watchSide(t, s)), live(t, s), sideUp(t, s), not(errPending(t, s)),
      not(x.dstShut), x.q1.length() > 0,
      x.q2.length() < W or (BUG_CONSUME_ON_SHORT_SEND and x.q1.length() > 0),
    }
  def hop1S(t: St, d: str): St =
    val x = t.dir.get(d)
    val x2 =
      if (x.q2.length() < W) { ...x, q1: x.q1.tail(), q2: x.q2.append(x.q1.head()) }
      else { ...x, q1: x.q1.tail(), lost: x.lost + 1 }
    fwdGhost(setDir(t, d, x2), dstOf(d))
  // datagram: the target side's run takes the whole frame at the head of the vsock (a bad header aborts the pair,
  // FrameInvalid) and sends its payload as one datagram, or waits while the target's send would block
  def frameReady(t: St, d: str): bool =
    val x = t.dir.get(d)
    x.q1.length() > 0 and (x.q1.head() % 10 != 0 or x.q1.length() >= t.dg.get(d).sizes[x.q1.head() / 10 - 1] + 1)
  // the head frame is incomplete (its header is in the vsock, not all of its items)
  def headPartial(t: St, d: str): bool =
    val x = t.dir.get(d)
    x.q1.length() > 0 and x.q1.head() % 10 == 0 and not(frameReady(t, d))
  // EV-DGRAM D1: the vsock peer has ended its direction (peer_shutdown & SEND_SHUTDOWN, read before the queued-byte
  // count) while the head frame is incomplete: the run aborts FrameInvalid
  def truncAtEnd(t: St, d: str): bool =
    headPartial(t, d) and t.vs.get(dstOf(d)).ps.contains("S")
  def canHop1D(t: St, d: str): bool =
    val s = dstOf(d)
    and {
      t.mode == "dgram", live(t, s), sideUp(t, s), not(errPending(t, s)),
      or {
        truncAtEnd(t, d),
        frameReady(t, d) and (not(t.dg.get(d).blk) or t.dir.get(d).q1.head() % 10 != 0),
      },
    }
  def hop1D(t: St, d: str): St =
    val x = t.dir.get(d)
    val y = t.dg.get(d)
    val h = x.q1.head()
    if (h % 10 != 0 or truncAtEnd(t, d)) abortSide(t, dstOf(d), "FrameInvalid", false)
    else
      val k = h / 10
      val len = y.sizes[k - 1] + 1
      val fr = x.q1.slice(0, len)
      val t1 = setDir(t, d, { ...x, q1: x.q1.slice(len, x.q1.length()) })
      val y2 =
        if (y.blk) { ...y, fdrop: y.fdrop.union(Set(k)) }
        else if (y.dq2.length() < WU) { ...y, dq2: y.dq2.append(fr) }
        else { ...y, rdrop: y.rdrop.union(Set(k)) }
      // a classified clean end of the incoming direction completes the pair once its last frame is sent
      val t2 = fwdGhost(setDg(t1, d, y2), dstOf(d))
      if (t2.vs.get(dstOf(d)).cls == "clean" and t2.dir.get(d).q1.length() == 0) complete(t2, dstOf(d)) else t2
  action Hop1(d: str): bool = all {
    canHop1S(cur, d) or canHop1D(cur, d),
    commit(if (cur.mode == "dgram") hop1D(cur, d) else hop1S(cur, d), variant("Hop1", { d: d })),
  }

  // the TCP source's end-of-stream (its FIN, after every queued unit: F3) is forwarded as SHUT_WR on the vsock
  // after the last accepted unit (ADR-0183, F4)
  def canHopEnd0(t: St, d: str): bool =
    val s = srcOf(d)
    val x = t.dir.get(d)
    and {
      t.mode != "dgram", not(watchSide(t, s)), live(t, s), sideUp(t, s), not(errPending(t, s)),
      x.wfin, x.q0.length() == 0, x.srcEnd == "none",
    }
  action HopEnd0(d: str): bool = all {
    canHopEnd0(cur, d),
    val s = srcOf(d)
    val t1 = send(setDir(cur, d, { ...cur.dir.get(d), srcEnd: "clean", srcShut: true }), s, Set("S"))
    commit(completeIfDone(t1, s), variant("HopEnd0", { d: d })),
  }
  // the vsock source's end, classified clean in the callback, is read once its queue is empty and forwarded as a
  // FIN on the TCP target
  def canHopEnd1(t: St, d: str): bool =
    val s = dstOf(d)
    val x = t.dir.get(d)
    and {
      t.mode != "dgram", not(watchSide(t, s)), live(t, s), sideUp(t, s), not(errPending(t, s)),
      x.q1.length() == 0, x.dstEnd == "none", t.vs.get(s).cls == "clean",
    }
  action HopEnd1(d: str): bool = all {
    canHopEnd1(cur, d),
    val s = dstOf(d)
    val t0 = if (cur.vs.get(s).cls == "none") noteCls(cur, s, "eof") else cur
    val t1 = setVs(t0, s, { ...t0.vs.get(s), cls: "clean", ucls: "clean" })
    val t2 = setDir(t1, d, { ...t1.dir.get(d), dstEnd: "clean", dstShut: true })
    commit(completeIfDone(t2, s), variant("HopEnd1", { d: d })),
  }
  // a vsock end-of-stream whose class was recorded neither at attach nor by a callback: a broken invariant the
  // attach order does not allow; the forwarder aborts the pair and counts unclassified_eof
  def canUnclassifiedEof(t: St, d: str): bool =
    val s = dstOf(d)
    val x = t.dir.get(d)
    and {
      t.mode != "dgram", not(watchSide(t, s)), live(t, s), sideUp(t, s), not(errPending(t, s)),
      x.q1.length() == 0, x.dstEnd == "none", t.vs.get(s).ps.contains("S"), t.vs.get(s).cls == "none",
    }
  action UnclassifiedEof(d: str): bool = all {
    canUnclassifiedEof(cur, d),
    val s = dstOf(d)
    val t0 = noteCls({ ...cur, gh: { ...cur.gh, unclassified: true } }, s, "eof")
    val t1 = abortSide(setVs(t0, s, { ...t0.vs.get(s), ucls: "abort" }), s, "Unclassified", false)
    commit(t1, variant("UnclassifiedEof", { d: d })),
  }

  // the receiving side's sk_state_change for the next SHUTDOWN / RST packet of direction d (F2)
  def vctl(t: St, d: str): St =
    val s = dstOf(d)
    val pkt = t.ctl.get(d).head()
    val v = t.vs.get(s)
    val nw = pkt.exclude(v.ps)
    // "this side still sends": the forwarder reads its own sk_shutdown (sshut); the ghost `misc` compares the class
    // with the one the socket's true send state (the SHUTDOWN already sent) gives
    val sends = if (watchSide(t, s)) not(t.dir.get("hg").wfin) else not(v.sshut)
    val trueSends = if (watchSide(t, s)) not(t.dir.get("hg").wfin) else not(v.sent.contains("S"))
    val c = classify(nw, v.ps, sends)
    // vsSt: the packet that leaves peer_shutdown == SHUTDOWN_MASK with the receive queue empty sets CLOSING
    val closing = v.ps.union(pkt) == both and t.dir.get(d).q1.length() == 0
    val v1 = { ...v, ps: v.ps.union(pkt), tcls: if (nw.contains("S") and v.tcls == "none") c else v.tcls,
               misc: v.misc or (live(t, s) and c != classify(nw, v.ps, trueSends)),
               st: if (closing) "closing" else v.st }
    val t1 = setVs({ ...t, ctl: t.ctl.set(d, t.ctl.get(d).tail()) }, s, v1)
    val r =
      if (not(live(t, s))) t1                      // unattached: recorded (F9); ended: ignored (END-IDEMPOTENT)
      else if (BUG_EOF_AS_FIN) (if (nw.contains("S")) markClean(noteCls(t1, s, "callback"), s) else t1)
      else if (c == "clean") markClean(noteCls(t1, s, "callback"), s)
      else if (c == "abort") peerAbort(withUcls(if (nw.contains("S")) noteCls(t1, s, "callback") else t1, s, nw), s)
      else t1
    // a watch's callback queued its record and then called the original callback: the holder is woken
    if (watchSide(t, s) and live(t, s) and r.pr.get(s).ph == "ended") { ...r, gh: { ...r.gh, woken: true } } else r
  def canVCtl(t: St, d: str): bool = t.ctl.get(d).length() > 0 and sideUp(t, dstOf(d))
  action VCtl(d: str): bool = all {
    canVCtl(cur, d),
    commit(vctl(cur, d), variant("VCtl", { d: d })),
  }
  // a run (or sk_error_report) finds a socket error: SocketError (a watch records it)
  // On a watch this is its sk_error_report: the record is queued and then the original callback wakes the holder
  def canFwdErr(t: St, s: str): bool = live(t, s) and sideUp(t, s) and errPending(t, s)
  def watchErr(t: St, s: str): St =
    val t1 = setPr(t, s, ended(t.pr.get(s), "aborted", "SocketError", false))
    { ...t1, gh: { ...t1.gh, woken: true } }
  action FwdErr(s: str): bool = all {
    canFwdErr(cur, s),
    val t1 = abortSide(cur, s, "SocketError", false)
    commit(if (watchSide(cur, s)) watchErr(cur, s) else t1, variant("FwdErr", { s: s })),
  }

  // ------------------------------------------------------------------ environment: writers, readers, resets
  // the guards of a TCP writer's write and a TCP reader's read (also used by the named runs)
  def canWriteS(t: St, d: str): bool = and {
    t.ep.get(srcOf(d)) == "alive", sideUp(t, srcOf(d)), not(t.dir.get(d).wfin), t.dir.get(d).wr < nmax(d),
    t.dir.get(d).q0.length() < W,
  }
  def canReadS(t: St, d: str): bool = and {
    t.ep.get(dstOf(d)) == "alive", sideUp(t, dstOf(d)), not(t.dir.get(d).shutRd), t.dir.get(d).q2.length() > 0,
  }
  // the writer of d writes its next unit (stream: into the source TCP socket's queue; the dialer: into V_h)
  def dialerSide(t: St, d: str, side: str): bool = t.mode == "watch" and side == "h"
  // a write by the dialer fails once V_h is shut (its own SHUT_RDWR at the watch's end: EPIPE)
  def canWriteAny(t: St, d: str): bool = and {
    t.mode != "dgram", t.ep.get(srcOf(d)) == "alive", sideUp(t, srcOf(d)),
    not(t.dir.get(d).wfin), t.dir.get(d).wr < nmax(d),
    if (dialerSide(t, d, srcOf(d)))
      and { t.attRes.get("g") == "ok", not(t.strmAbort), t.dir.get(d).q1.length() < C, not(t.vs.get("h").err),
            not(t.vs.get("h").ps.contains("R")), not(t.vs.get("h").sent.contains("S")) }
    else canWriteS(t, d),
  }
  action Write(d: str): bool = all {
    canWriteAny(cur, d),
    val x = cur.dir.get(d)
    val x2 = if (dialerSide(cur, d, srcOf(d))) { ...x, wr: x.wr + 1, q1: x.q1.append(x.wr + 1) }
             else { ...x, wr: x.wr + 1, q0: x.q0.append(x.wr + 1) }
    commit(setDir(cur, d, x2), variant("Write", { d: d })),
  }
  // the writer's FIN (the dialer's shutdown_write sends {S} on V_h itself)
  action Fin(d: str): bool = all {
    cur.mode != "dgram", cur.ep.get(srcOf(d)) == "alive", sideUp(cur, srcOf(d)), not(cur.dir.get(d).wfin),
    if (dialerSide(cur, d, srcOf(d)))
      all {
        cur.attRes.get("g") == "ok", not(cur.strmAbort), not(cur.vs.get("h").sent.contains("S")),
        commit(send(setDir(cur, d, { ...cur.dir.get(d), wfin: true, srcEnd: "clean", srcShut: true }), "h", Set("S")),
               variant("Fin", { d: d })),
      }
    else commit(setDir(cur, d, { ...cur.dir.get(d), wfin: true }), variant("Fin", { d: d })),
  }
  // the reader of d reads its next unit (the dialer reads through its GuestStream, until V_h's own receive side is
  // shut: F11; once the owner ended the stream every call returns ConnectionAborted — holderTrustsRead: the stream
  // passes the read to V_h whatever the owner did)
  def canReadAny(t: St, d: str): bool = and {
    t.mode != "dgram", t.ep.get(dstOf(d)) == "alive", sideUp(t, dstOf(d)),
    if (dialerSide(t, d, dstOf(d)))
      and { t.attRes.get("g") == "ok", not(t.strmAbort) or BUG_HOLDER_TRUSTS_READ, t.dir.get(d).q1.length() > 0,
            not(t.vs.get("h").sent.contains("R")) }
    else canReadS(t, d),
  }
  action Read(d: str): bool = all {
    canReadAny(cur, d),
    val x = cur.dir.get(d)
    val dial = dialerSide(cur, d, dstOf(d))
    val t1 = setDir(cur, d, if (dial) { ...x, q1: x.q1.tail(), got: x.got.append(x.q1.head()) }
                            else { ...x, q2: x.q2.tail(), got: x.got.append(x.q2.head()) })
    val late = dial and cur.strmAbort
    commit({ ...t1, gh: { ...t1.gh, holderReadAfterAbort: t1.gh.holderReadAfterAbort or late } }, variant("Read", { d: d })),
  }
  // The holder's end observation on V_h (F11 corrected): what its read returns at the end — Eof (0) or Err (a read
  // error): after its own SHUT_RDWR (the watch's end by the owner) either, queued bytes or not; after a guest
  // SHUTDOWN, Eof once the queue is empty; after the orphan sweep, Err (ECONNRESET) once the queue is empty, the sweep's
  // callback (FwdErr) run or not
  def holderReadable(t: St): Set[str] =
    if (not(and { t.mode == "watch", t.ep.get("h") == "alive", t.attRes.get("g") == "ok", t.gh.hobs == "none" }))
      Set()
    else if (t.vs.get("h").sent.contains("R")) Set("eof", "err")
    else if (t.dir.get("gh").q1.length() > 0) Set()
    else if (t.vs.get("h").err) Set("err")
    else if (t.vs.get("h").ps.contains("S")) Set("eof")
    else Set()
  def canHolderEnd(t: St): bool = holderReadable(t) != Set()
  action HolderEnd(k: str): bool = all {
    holderReadable(cur).contains(k),
    commit({ ...cur, gh: { ...cur.gh, hobs: k } }, variant("HolderEnd", { k: k })),
  }
  // The GuestStream's resolution of its end (EVMOD-STREAM-END, EVMOD-STREAM-ABORT): (1) the owner's own end of the watch —
  // its abort (strmAbort) or its context's release — → OwnerAbort (Err(ConnectionAborted)); (2) the watch's record —
  // HalfClose → HalfClose (Ok(0)), Aborted → GuestAbort (Err(ConnectionReset)); (3) otherwise the record is in flight
  // (only the orphan sweep's end is readable before its callback) and the stream waits. holderTrustsRead: from the
  // holder's read of V_h (Eof → HalfClose).
  def watchOutcome(t: St): str =
    val p = t.pr.get("h")
    if (BUG_HOLDER_TRUSTS_READ) (if (t.gh.hobs == "eof") "HalfClose" else "GuestAbort")
    else if (t.strmAbort or (p.ph == "ended" and p.byRel)) "OwnerAbort"
    else if (p.ph == "ended" and p.rec.in(Set("queued", "read"))) (if (p.out == "halfclose") "HalfClose" else "GuestAbort")
    else "wait"
  // the reader reaches the end of the stream: end-of-stream after a FIN; the dialer's GuestStream reports the
  // resolved end — at once once the owner ended the stream, else of what its holder observed
  def canReadEof(t: St, d: str): bool = and {
    t.mode != "dgram", t.ep.get(dstOf(d)) == "alive", sideUp(t, dstOf(d)),
    not(t.dir.get(d).eof), not(t.dir.get(d).rst),
    if (dialerSide(t, d, dstOf(d)))
      and {
        t.attRes.get("g") == "ok",
        (t.strmAbort and not(BUG_HOLDER_TRUSTS_READ)) or (t.gh.hobs != "none" and watchOutcome(t) != "wait"),
      }
    else and { not(t.dir.get(d).shutRd), t.dir.get(d).q2.length() == 0, t.dir.get(d).dstShut },
  }
  action ReadEof(d: str): bool = all {
    canReadEof(cur, d),
    val x = cur.dir.get(d)
    val dial = dialerSide(cur, d, dstOf(d))
    val o = if (dial) watchOutcome(cur) else "HalfClose"
    val t1 = setDir(cur, d, if (o == "HalfClose") { ...x, eof: true } else { ...x, rst: true })
    commit(if (dial) { ...t1, hres: o, gh: { ...t1.gh, hresCls: watchClass(cur) } } else t1, variant("ReadEof", { d: d })),
  }
  // the reader shuts down its read side only and keeps its socket open: it sends nothing (tcp.c:3113-3114 @ v7.0)
  action ShutRd(d: str): bool = all {
    cur.mode != "dgram", not(dialerSide(cur, d, dstOf(d))), cur.ep.get(dstOf(d)) == "alive", sideUp(cur, dstOf(d)),
    not(cur.dir.get(d).shutRd),
    commit(setDir(cur, d, { ...cur.dir.get(d), shutRd: true }), variant("ShutRd", { d: d })),
  }
  // T-1 (EV-END change 1): the far TCP peer at side s resets, after its FIN or before it (`after_fin`, the half's
  // received FIN). The half's sk_err is ECONNRESET, or EPIPE when the half is in CLOSE_WAIT (it received the FIN and
  // the forwarder has not sent its own; tcp_reset, net/ipv4/tcp_input.c:4918-4930 @ v7.0), and its sk_error_report
  // runs in the same step: a live pair aborts SocketError { side, errno } (a live pair always has a direction open;
  // once both ended the pair is complete and the report is ignored). An unattached half is refused at attach (T-4).
  action TcpRst(s: str, after_fin: bool): bool = all {
    cur.mode != "dgram", not(watchSide(cur, s)), cur.ep.get(s) == "alive", sideUp(cur, s), canFault(cur),
    after_fin == cur.dir.get(outof(s)).wfin,
    val e = if (tcpSt(cur, s) == "closeWait") "EPIPE" else "ECONNRESET"
    val open = cur.pr.get(s).ph.in(Set("unattached", "attaching", "live"))
    val t1 = fault(cutBoth({ ...cur, ep: cur.ep.set(s, "dead"), tcpErr: cur.tcpErr.set(s, e) }))
    val t2 = { ...t1, gh: { ...t1.gh, rstOpen: if (open) t1.gh.rstOpen.union(Set(s)) else t1.gh.rstOpen } }
    val t3 =
      if (live(t2, s))
        val a = abortSide(t2, s, "SocketError", false)
        setPr(a, s, { ...a.pr.get(s), errno: e })
      else t2
    commit(t3, variant("TcpRst", { s: s, after_fin: after_fin })),
  }
  // T-6 (EV-CTL 4.4): the holder drops its GuestStream: the stream aborts its watch (record queued, the module's
  // reference dropped) and then closes V_h — the guest sees 0x3 — in one step; a stream the owner already ended only
  // closes V_h. dropWithoutAbort: the drop only closes the descriptor; the forwarder's reference keeps V_h open
  // (P-70 § 4.4), so nothing is sent and the watch stays live.
  action HolderDrop: bool = all {
    cur.mode == "watch", cur.ep.get("h") == "alive", canFault(cur),
    val t1 = fault(cutBoth({ ...cur, ep: cur.ep.set("h", "dead"), gh: { ...cur.gh, dropped: true } }))
    val t2 =
      if (live(t1, "h") and BUG_DROP_WITHOUT_ABORT) t1
      else if (live(t1, "h")) shutAll(setPr(t1, "h", ended(t1.pr.get("h"), "aborted", "Detached", false)), "h")
      else shutAll(t1, "h")
    commit(t2, variant("HolderDrop", ())),
  }
  // the VMM exits (a guest reset included, RESET-EXIT): the guest side is gone; the host vsock gets sk_err (F7)
  action VmmExit: bool = all {
    cur.vmUp, canFault(cur),
    val pg = cur.pr.get("g")
    val t1 = { ...fault(cur), vmUp: false, ctx: cur.ctx.set("g", "gone"),
               pr: cur.pr.set("g", if (pg.ph.in(Set("ended", "refused"))) pg else { ...pg, ph: "gone" }),
               ep: cur.ep.set("g", "gone"), vs: cur.vs.set("h", { ...cur.vs.get("h"), err: true }) }
    commit(if (cur.mode == "dgram") t1 else cutBoth(t1), variant("VmmExit", ())),
  }
  // the owner of the far end at side s closes (after the pair's end): its peer's kernel answers with an RST,
  // which reaches side s after its pair ended (CLEAN-FULL-END)
  action TrailingRst(s: str): bool = all {
    cur.pr.get(s).closed, not(cur.gh.trail.contains(s)), sideUp(cur, other(s)),
    val t1 = { ...cur, ctl: cur.ctl.set(into(s), cur.ctl.get(into(s)).append(both)),
               gh: { ...cur.gh, trail: cur.gh.trail.union(Set(s)) } }
    commit(t1, variant("TrailingRst", { s: s })),
  }

  // ------------------------------------------------------------------ environment: datagram mode
  // the source application (guest) or the destination (host) sends datagram n+1 of size sz to the source UDP
  // socket; a full queue drops it (F8)
  action SendDg(d: str, sz: int): bool = all {
    cur.mode == "dgram", sideUp(cur, srcOf(d)), cur.dg.get(d).n < dnmax(d), SIZES.contains(sz),
    val y = cur.dg.get(d)
    val k = y.n + 1
    val y2 = if (y.dq0.length() < WU) { ...y, n: k, sizes: y.sizes.append(sz), dq0: y.dq0.append(k) }
             else { ...y, n: k, sizes: y.sizes.append(sz), sdrop: y.sdrop.union(Set(k)) }
    commit(setDg(cur, d, y2), variant("SendDg", { d: d, sz: sz })),
  }
  // the far receiver reads a datagram
  action DRead(d: str): bool = all {
    cur.mode == "dgram", sideUp(cur, dstOf(d)), cur.dg.get(d).dq2.length() > 0,
    val y = cur.dg.get(d)
    commit(setDg(cur, d, { ...y, dq2: y.dq2.tail(), got: y.got.append(y.dq2.head()) }), variant("DRead", { d: d })),
  }
  // the target UDP socket's send would block (EAGAIN: its send buffer is full) / no longer
  action UdpBlock(d: str): bool = all {
    cur.mode == "dgram", CFG != "dlive", not(cur.dg.get(d).blk),
    commit(setDg(cur, d, { ...cur.dg.get(d), blk: true }), variant("UdpBlock", { d: d })),
  }
  action UdpUnblock(d: str): bool = all {
    cur.mode == "dgram", cur.dg.get(d).blk,
    commit(setDg(cur, d, { ...cur.dg.get(d), blk: false }), variant("UdpUnblock", { d: d })),
  }
  // the connected UDP socket at side s reports an error (ECONNREFUSED from ICMP, or the D26 rule's EPERM)
  action UdpErr(s: str): bool = all {
    cur.mode == "dgram", sideUp(cur, s), not(cur.terr.get(s)), canFault(cur),
    commit(fault({ ...cur, terr: cur.terr.set(s, true) }), variant("UdpErr", { s: s })),
  }
  // a peer outside the design shuts the write side of side s's datagram vsock (a 0x2) — no Overdrive owner or
  // forwarder half-closes a datagram vsock (U-EV-5); the stimulus by which EV-DGRAM D1 is checked (F12)
  action DgHalfClose(s: str): bool = all {
    cur.mode == "dgram", live(cur, s), sideUp(cur, s), not(cur.vs.get(s).sent.contains("S")), canFault(cur),
    commit(send(fault(cur), s, Set("S")), variant("DgHalfClose", { s: s })),
  }

  // ------------------------------------------------------------------ next
  action step = {
    nondet d = DIRS.oneOf()
    nondet s = SIDES.oneOf()
    nondet sz = SIZES.oneOf()
    nondet k = Set("eof", "err").oneOf()
    any {
      Attach(s), AttachRead(s), AttachHook(s), CloseUnattached(s), Abort(s), CtxRelease(s), Reopen(s), TakeEnd(s),
      OwnerClose(s), DropRecord(s),
      Hop0(d), Hop1(d), HopEnd0(d), HopEnd1(d), UnclassifiedEof(d), VCtl(d), FwdErr(s),
      Write(d), Fin(d), Read(d), ReadEof(d), ShutRd(d), TcpRst(s, cur.dir.get(outof(s)).wfin), HolderDrop, VmmExit,
      TrailingRst(s), GuestAbort(s),
      SendDg(d, sz), DRead(d), UdpBlock(d), UdpUnblock(d), UdpErr(s),
      HolderEnd(k), DgHalfClose(s),
    }
  }

  // ------------------------------------------------------------------ invariants (stream and watch)
  def inflight(d: str): int =
    val x = dir.get(d)
    x.q0.length() + x.q1.length() + x.q2.length()
  // each reader's units are exactly 1..k of its writer's: no loss, duplicate or reorder
  val DeliveredIsInOrderPrefix: bool = DIRS.forall(d => dir.get(d).got == range(1, dir.get(d).got.length() + 1))
  // every unit written is in a queue or read
  val UnitsConserved: bool = DIRS.forall(d => dir.get(d).wr == inflight(d) + dir.get(d).got.length())
  // no unit is off its source and not accepted by its target, outside one run
  val ConsumeOnlyAfterAccept: bool =
    DIRS.forall(d => dir.get(d).lost == 0)
  // every queue within its window or credit; nothing else holds units
  val QueuesWithinBounds: bool = DIRS.forall(d => and {
    dir.get(d).q0.length() <= W, dir.get(d).q1.length() <= (if (mode == "dgram") CD else C),
    dir.get(d).q2.length() <= W,
    dg.get(d).dq0.length() <= WU, dg.get(d).dq2.length() <= WU,
  })
  // a target is shut only after its source's clean end and the acceptance of its last unit (ADR-0183)
  val NoEarlyShutdown: bool = DIRS.forall(d => and {
    dir.get(d).srcShut implies (dir.get(d).srcEnd == "clean" and dir.get(d).q0.length() == 0),
    dir.get(d).dstShut implies (dir.get(d).dstEnd == "clean" and dir.get(d).q1.length() == 0),
  })
  // a reader sees end-of-stream only after its writer ended cleanly and every unit was delivered (R2-END)
  val AbortNeverCleanEof: bool =
    DIRS.forall(d => dir.get(d).eof implies (dir.get(d).wfin and dir.get(d).got.length() == dir.get(d).wr))
  // an owner's death, the VMM's exit or a reset is never seen as end-of-stream
  val NoEofOnPeerLoss: bool = DIRS.forall(d => dir.get(d).eof implies not(dir.get(d).loss))
  // each vsock end's class is the class of the transition that ended it, not of the accumulated flags (EVMOD-END)
  // (and no transition is classified otherwise than the table gives with the socket's true send state:
  // CLEAN-FULL-END's "this side shut its send")
  val ClassFixedAtTransition: bool = SIDES.forall(s =>
    ((vs.get(s).ucls != "none" and vs.get(s).tcls != "none") implies vs.get(s).ucls == vs.get(s).tcls)
    and not(vs.get(s).misc))
  // exactly one end record per pair or watch, unless its context was released (EVMOD-CTL)
  val OneEndRecordPerPair: bool = SIDES.forall(s => and {
    pr.get(s).nrec <= 1,
    (pr.get(s).ph == "ended" and not(pr.get(s).byRel)) implies pr.get(s).rec.in(Set("queued", "read")),
  })
  // no unit is forwarded by a pair whose context is released
  val NoForwardAfterEnd: bool = not(gh.fwdAfterEnd)
  // after a context's release no pair of it forwards: a live pair belongs to its owner's current, open context
  val ReleasedContextHoldsNoPair: bool = SIDES.forall(s => live(cur, s) implies pairCur(cur, s))
  // no unit is forwarded by a pair of a context the owner replaced (EVMOD-CTL recovery)
  val NoPairOfReleasedContextForwards: bool = not(gh.fwdOld)
  // an owner never holds two open contexts (EVMOD-CTL: the recovery closes the old before it opens the new)
  val OneContextPerOwner: bool = SIDES.forall(s => not(ctx.get(s) == "open" and oldCtx.get(s) == "open"))
  // the guest side's context is gone exactly when the VM is down (round 14, the init driver's ctx(g) classification):
  // VmmExit is the only writer of gone and the only writer of vmUp = false; nothing reopens side g or restarts the VM,
  // so ctx(g) = gone is stimulus-determined and never projected from a driver flag
  val CtxGoneIffVmDown: bool = (ctx.get("g") == "gone") iff not(vmUp)
  // an end present when the callbacks were installed is classified exactly once, at attach (EVMOD-PAIR, U-EV-2);
  // no end of a half is classified twice
  val AttachTimeEndClassifiedOnce: bool = SIDES.forall(s => and {
    vs.get(s).ncls <= 1, vs.get(s).att == "end" implies vs.get(s).clsAt == "attach",
  })
  // the unclassified_eof alarm never fires (EVMOD-END)
  val NoUnclassifiedEof: bool = not(gh.unclassified)
  // a pair whose TCP half was reset while the pair was open never completes, whatever its recvmsg returned
  // (EVMOD-END, TCP halves: a FIN then a reset aborts)
  val ResetNeverEndsClean: bool = SIDES.forall(s => gh.rstOpen.contains(s) implies pr.get(s).out != "completed")
  // the holder of a stream its owner ended reads no further unit through it (EVMOD-STREAM-ABORT: every call returns
  // ConnectionAborted)
  val NoHolderReadAfterWatchAbort: bool = not(gh.holderReadAfterAbort)
  // the GuestStream's reported end (hres) is OwnerAbort iff the owner ended the watch (its abort, or its context's
  // release), else the class of the watch's record — whatever the holder's read returned (EVMOD-STREAM-END, F11, T-5);
  // judged when the stream reports it: a later owner end (the recovery's, U-R1) leaves the reported end as it was
  def watchClass(t: St): str =
    val p = t.pr.get("h")
    if (t.strmAbort or (p.ph == "ended" and p.byRel)) "OwnerAbort"
    else if (p.out == "halfclose") "HalfClose" else if (p.out == "aborted") "GuestAbort" else "none"
  val HolderOutcomeFromWatch: bool = (mode == "watch" and hres != "none") implies hres == gh.hresCls
  // EVMOD-CTL recovery step 1 (U-R1): after the host Forwarder recovery's close (a release that found no live watch,
  // not the owner's death), no GuestStream the holder still holds is left un-ended by its owner
  val RecoveryEndsHeldStream: bool =
    (mode == "watch" and not(gh.hostDead) and (ctx.get("h") == "released" or gen.get("h") == 1) and ep.get("h") == "alive")
      implies strmAbort
  // T-1: no pair is live once its TCP half's reset was reported (a live pair always has a direction open)
  val TcpResetEndsPair: bool = SIDES.forall(s => tcpErr.get(s) != "0" implies not(live(cur, s)))
  // T-1: a reset's record carries EPIPE iff the reset half was in CLOSE_WAIT (it had received its far peer's FIN
  // and the forwarder had not sent its own), else ECONNRESET
  val ResetErrnoMatchesState: bool = SIDES.forall(s =>
    pr.get(s).errno != "none" implies
      ((pr.get(s).errno == "EPIPE") == (dir.get(outof(s)).wfin and not(dir.get(into(s)).dstShut))))
  // T-3: a pair or a flow ends aborted only after some fault or abort action of the trace (TcpRst, HolderDrop,
  // VmmExit, UdpErr, Abort, CtxRelease, CloseUnattached, DgHalfClose — every action that counts in `faults`)
  val AbortOnlyAfterFault: bool = SIDES.forall(s => pr.get(s).out == "aborted" implies gh.faults > 0)
  // T-4: an attach that returned HalfEnded created no pair and no end record
  val RefusedAttachLeavesNoPair: bool = SIDES.forall(s =>
    attRes.get(s) == "halfEnded" implies and { not(live(cur, s)), pr.get(s).nrec == 0, pr.get(s).rec == "none" })
  // T-4: after a HalfEnded refusal no far end of the flow reads end-of-stream
  val EndedHalfNeverEndsClean: bool =
    SIDES.exists(s => attRes.get(s) == "halfEnded") implies DIRS.forall(d => not(dir.get(d).eof))
  // T-5 (WATCH-ABORT): the owner's abort of a watch changes nothing of V_h — its shut flags, the flags it sent, its
  // queues
  val WatchAbortLeavesHolderSocket: bool = not(gh.vhTouched)
  // T-6: after the holder's drop no watch of the dial is live (the forwarder holds no reference to V_h)
  val NoLiveWatchAfterDrop: bool = (mode == "watch" and gh.dropped) implies not(live(cur, "h"))
  // T-8 (CLEAN-FULL-END): whenever this side's 0x2 is in flight or delivered, its own send-shut flag is set
  val OwnShutBeforeShutdownSent: bool = SIDES.forall(s => vs.get(s).sent.contains("S") implies vs.get(s).sshut)
  // a holder woken by a watch's end finds the watch's record queued (EVMOD-STREAM-END, EV-END change 3: record, then the original
  // callback, in sk_state_change and sk_error_report)
  val WatchRecordBeforeWake: bool =
    (mode == "watch" and gh.woken) implies (pr.get("h").ph == "ended" and pr.get("h").rec != "none")
  val Safety: bool = and {
    DeliveredIsInOrderPrefix, UnitsConserved, ConsumeOnlyAfterAccept, QueuesWithinBounds, NoEarlyShutdown,
    AbortNeverCleanEof, NoEofOnPeerLoss, ClassFixedAtTransition, OneEndRecordPerPair, NoForwardAfterEnd,
    ReleasedContextHoldsNoPair, NoPairOfReleasedContextForwards, OneContextPerOwner, AttachTimeEndClassifiedOnce,
    NoUnclassifiedEof, ResetNeverEndsClean, NoHolderReadAfterWatchAbort, HolderOutcomeFromWatch,
    WatchRecordBeforeWake, TcpResetEndsPair, ResetErrnoMatchesState, AbortOnlyAfterFault, RefusedAttachLeavesNoPair,
    EndedHalfNeverEndsClean, WatchAbortLeavesHolderSocket, NoLiveWatchAfterDrop, OwnShutBeforeShutdownSent,
    RecoveryEndsHeldStream, CtxGoneIffVmDown,
  }

  // ------------------------------------------------------------------ invariants (datagram mode, EVMOD-DGRAM)
  // a whole frame of one datagram: 10k, 10k+1, .., 10k+sz(k)
  def frameOk(d: str, f: List[int]): bool = and {
    f.length() > 0, f[0] % 10 == 0, f[0] / 10 >= 1, f[0] / 10 <= dg.get(d).n,
    f.indices().forall(i => f[i] == f[0] + i),
    f.length() == dg.get(d).sizes[f[0] / 10 - 1] + 1,
  }
  def strictlyIncreasing(l: List[int]): bool = l.indices().forall(i => i == 0 or l[i - 1] < l[i])
  // every datagram, an empty one included, is delivered at most once, whole and in order
  val DatagramsOnceInOrder: bool = DIRS.forall(d =>
    val g = dg.get(d).got
    and {
      g.indices().forall(i => frameOk(d, g[i])),
      strictlyIncreasing(g.foldl(List(), (acc, f) => acc.append(if (f.length() > 0) f[0] / 10 else 0))),
    })
  // no delivered (or deliverable) datagram carries items of two datagrams
  pure def oneDatagram(f: List[int]): bool = f.length() == 0 or f.indices().forall(i => f[i] / 10 == f[0] / 10)
  val FrameNeverSplitAcrossDatagrams: bool =
    DIRS.forall(d => dg.get(d).got.indices().forall(i => oneDatagram(dg.get(d).got[i])) and
                     dg.get(d).dq2.indices().forall(i => oneDatagram(dg.get(d).dq2[i])))
  // every datagram sent is queued, in flight, delivered, or dropped at a UDP receive queue (the source socket's,
  // full or discarded at a slot release, or the far receiver's) — never inside the forwarder or the vsock
  val DropsOnlyAtSourceQueue: bool = DIRS.forall(d =>
    val y = dg.get(d)
    and {
      y.fdrop == Set(),
      1.to(y.n).forall(k => or {
        y.sdrop.contains(k), y.rdrop.contains(k), lset(y.dq0).contains(k), ids(dir.get(d).q1).contains(k),
        frameIds(y.dq2).contains(k), frameIds(y.got).contains(k),
      }),
    })
  // EV-DGRAM D1: a live pair whose vsock peer ended its direction while the head frame is incomplete is aborted
  // FrameInvalid by its next run (no such pair waits for the rest of the frame); an error pending aborts it first
  val IncompleteFrameAtEndAborts: bool = SIDES.forall(s =>
    (mode == "dgram" and live(cur, s) and sideUp(cur, s) and vs.get(s).ps.contains("S") and headPartial(cur, into(s)))
    implies (truncAtEnd(cur, into(s)) or errPending(cur, s)))
  val DgramSafety: bool = and {
    DatagramsOnceInOrder, FrameNeverSplitAcrossDatagrams, DropsOnlyAtSourceQueue, QueuesWithinBounds,
    OneEndRecordPerPair, ReleasedContextHoldsNoPair, NoForwardAfterEnd, NoPairOfReleasedContextForwards,
    OneContextPerOwner, AttachTimeEndClassifiedOnce, IncompleteFrameAtEndAborts, AbortOnlyAfterFault,
    RefusedAttachLeavesNoPair, OwnShutBeforeShutdownSent, CtxGoneIffVmDown,
  }
  // the instance "mid" carries stream and datagram mode in one search: the trace's mode picks its property set (the
  // stream invariants count units in q1; in datagram mode q1 holds frame items)
  val MidSafety: bool = if (mode == "dgram") DgramSafety else Safety

  // ------------------------------------------------------------------ liveness (TLC, instance "live")
  // Weak fairness of the system: the owners' attach and record reads, every forwarder run and callback, and the
  // readers' reads — NOT a reader's choice to stop (a reader that shut its read side is a stall the design allows).
  // Every action of the model can be taken only finitely often on these instances (bounded units, packets, faults
  // and records; the datagram send-buffer toggles are not on them), so every behaviour ends in stuttering, and weak
  // fairness of the system actions is then equivalent to the state formula "eventually, always, none of them is
  // enabled". `sysEnabled` is the disjunction of the guards the actions themselves are built on. (Stated through
  // `weakFair`, TLC evaluates the enabledness of every action per state in its liveness mode, about 1,000 states a
  // minute here.)
  val sysEnabled: bool = or {
    SIDES.exists(s => canAttach(cur, s) or canAttachHook(cur, s) or canTakeEnd(cur, s) or canFwdErr(cur, s)),
    canGuestAbort(cur),
    DIRS.exists(d => or {
      canHop0S(cur, d), canHop0D(cur, d), canHop1S(cur, d), canHop1D(cur, d), canHopEnd0(cur, d), canHopEnd1(cur, d),
      canUnclassifiedEof(cur, d),
      canVCtl(cur, d), canReadAny(cur, d), canReadEof(cur, d), canHolderEnd(cur),
    }),
  }
  temporal fair = eventually(always(not(sysEnabled)))
  // ... and the writers writing as fast as they can
  temporal fairWrite = eventually(always(not(sysEnabled or DIRS.exists(d => canWriteS(cur, d)))))
  val stalled: bool = DIRS.exists(d => dir.get(d).shutRd)
  def cutOff(d: str): bool = or {
    stalled, dir.get(d).rst, dir.get(d).loss, ep.get(dstOf(d)) != "alive", not(vmUp),
    SIDES.exists(s => pr.get(s).out == "aborted"),
  }
  def resolved(s: str): bool = or {
    pr.get(s).rec == "read", pr.get(s).byRel, pr.get(s).ph.in(Set("gone", "refused")), ctx.get(s) != "open",
    pr.get(s).cgen != gen.get(s),
  }
  // (datagram mode: the vsock peer ended the incoming direction — EV-DGRAM D1's end)
  def endDue(s: str): bool = or {
    DIRS.forall(d => dir.get(d).wfin), errPending(cur, s), pr.get(s).ph == "ended", DIRS.exists(d => dir.get(d).loss),
    mode == "dgram" and vs.get(s).ps.contains("S"),
  }
  // every unit written is eventually delivered, unless the flow is cut or stalled
  temporal EveryUnitDelivered = fair.implies(and {
    always((inflight("gh") > 0).implies(eventually(inflight("gh") == 0 or cutOff("gh")))),
    always((inflight("hg") > 0).implies(eventually(inflight("hg") == 0 or cutOff("hg")))),
  })
  // every pair ends — after both clean ends or at an abort — and its owner reads its record (or the context is
  // released), unless a reader stalls
  temporal EveryEndResolves = fair.implies(and {
    always(endDue("g").implies(eventually(resolved("g") or stalled))),
    always(endDue("h").implies(eventually(resolved("h") or stalled))),
  })
  // a writer's FIN reaches its reader as end-of-stream, unless the flow is cut or stalled
  temporal CleanHalfCloseCompletes = fair.implies(and {
    always(dir.get("gh").wfin.implies(eventually(dir.get("gh").eof or cutOff("gh")))),
    always(dir.get("hg").wfin.implies(eventually(dir.get("hg").eof or cutOff("hg")))),
  })
  // T-4: a flow whose attach was refused HalfEnded is closed at both owners
  temporal EndedHalfFlowResolves = fair.implies(
    always(SIDES.exists(s => attRes.get(s) == "halfEnded").implies(eventually(resolved("g") and resolved("h")))))
  // the guest pair has ended (or the VM is gone, or its attach was refused)
  val guestEnded: bool = pr.get("g").ph.in(Set("ended", "refused", "gone"))
  // T-5: after the owner's abort of a watched stream the guest pair ends (by Abort { flow } while the session is
  // live, else by the holder's close)
  temporal GuestPairEndsAfterOwnerAbort = fair.implies(always(strmAbort.implies(eventually(guestEnded))))
  // T-6: after the holder's drop the guest pair ends
  temporal GuestPairEndsAfterHolderDrop = fair.implies(always(gh.dropped.implies(eventually(guestEnded))))
  // while a reader has stopped, its writer — writing as fast as it can — is eventually unable to send
  def canWrite(d: str): bool = canWriteS(cur, d)
  temporal StalledSenderIsBlocked = fairWrite.implies(and {
    always(dir.get("gh").shutRd.implies(eventually(not(canWrite("gh"))))),
    always(dir.get("hg").shutRd.implies(eventually(not(canWrite("hg"))))),
  })

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // an action's guard, for a run to assert that the action is disabled in the current state
  def enHop0(d: str): bool = canHop0S(cur, d) or canHop0D(cur, d)
  def enHop1(d: str): bool = canHop1S(cur, d) or canHop1D(cur, d)
  def canRead(d: str): bool = canReadS(cur, d)
  // Deterministic scenarios on the conformance main `pair_conf`, each with Safety (DgramSafety in mode dgram)
  // checked along the way. The prefix `Paired` attaches both sides in the pinned order.
  run Paired = InitWith("stream").then(Attach("h")).then(Attach("g")).expect(Safety and live(cur, "g") and live(cur, "h"))
  // happy path: one unit each way, a FIN each way; both pairs complete, each owner reads its record and closes
  // (the close's {S, R} reaches an ended pair and changes nothing)
  run CleanBothWaysCompletes = Paired
    .then(Write("gh")).then(Hop0("gh")).then(Hop1("gh")).then(Read("gh"))
    .then(Write("hg")).then(Hop0("hg")).then(Hop1("hg")).then(Read("hg"))
    .expect(Safety and dir.get("gh").got == [1] and dir.get("hg").got == [1])
    .then(Fin("gh")).then(HopEnd0("gh")).expect(Safety and dir.get("gh").srcShut and ctl.get("gh") == [Set("S")])
    .then(VCtl("gh")).expect(Safety and vs.get("h").cls == "clean" and vs.get("h").tcls == "clean")
    .then(HopEnd1("gh")).then(ReadEof("gh")).expect(Safety and dir.get("gh").eof and live(cur, "h"))
    .then(Fin("hg")).then(HopEnd0("hg")).expect(Safety and pr.get("h").out == "completed" and pr.get("h").rec == "queued")
    .then(VCtl("hg")).then(HopEnd1("hg")).expect(Safety and pr.get("g").out == "completed")
    .then(ReadEof("hg")).then(TakeEnd("g")).then(TakeEnd("h")).then(OwnerClose("g"))
    .then(VCtl("gh")).then(VCtl("hg"))
    .expect(Safety and dir.get("gh").eof and dir.get("hg").eof and not(dir.get("gh").rst) and not(dir.get("hg").rst)
            and pr.get("g").out == "completed" and pr.get("h").out == "completed" and pr.get("g").nrec == 1
            and pr.get("h").nrec == 1 and ep.get("g") == "alive" and ep.get("h") == "alive")
  // a stopped reader stops the forwarder (the target is full) and then the writer (its window is full); nothing
  // grows; when the reader resumes, everything moves again
  run TargetStallBlocksSenderThenResumes = Paired
    .then(Write("gh")).then(Hop0("gh")).then(Hop1("gh")).expect(Safety and dir.get("gh").q2 == [1])
    .then(Write("gh")).then(Hop0("gh")).expect(Safety and dir.get("gh").q1 == [2])
    .expect(not(enHop1("gh")))
    .then(Write("gh")).expect(Safety and dir.get("gh").q0 == [3] and not(canWrite("gh")) and not(enHop0("gh")))
    .then(Read("gh")).then(Hop1("gh")).then(Hop0("gh")).then(Write("gh"))
    .expect(Safety and dir.get("gh").wr == 4 and dir.get("gh").got == [1] and dir.get("gh").q0 == [4])
  // EVMOD-PAIR: bytes that arrive before attach — the guest application's write before Paired, the far peer's
  // server-first banner before the host attached — wait in their source's own queue; the attach-time runs
  // forward them
  run EarlyBytesBeforeAttachDelivered = InitWith("stream")
    .then(Write("gh")).then(Write("hg")).expect(Safety and dir.get("gh").q0 == [1] and dir.get("hg").q0 == [1])
    .then(Attach("h")).then(Hop0("hg")).expect(Safety and dir.get("hg").q1 == [1] and pr.get("g").ph == "unattached")
    .then(Attach("g")).then(Hop0("gh")).then(Hop1("hg")).then(Hop1("gh")).then(Read("gh")).then(Read("hg"))
    .expect(Safety and dir.get("gh").got == [1] and dir.get("hg").got == [1])
  // EVMOD-PAIR: the guest application writes and sends its FIN before Paired; both are carried out after attach
  run EarlyFinBeforeAttachCarriedOut = InitWith("stream")
    .then(Write("gh")).then(Fin("gh")).then(Attach("h")).then(Attach("g"))
    .then(Hop0("gh")).then(HopEnd0("gh")).then(VCtl("gh")).then(Hop1("gh")).then(HopEnd1("gh"))
    .then(Read("gh")).then(ReadEof("gh"))
    .expect(Safety and dir.get("gh").got == [1] and dir.get("gh").eof and live(cur, "g") and live(cur, "h"))
  // R2-END: the guest application resets; the guest TCP half's sk_error_report aborts the guest pair in the same
  // step (ECONNRESET) and shuts V_g; the host's callback classifies the 0x3 abort and resets the far peer — never an
  // EOF
  run GuestAppResetReachesRemoteAsReset = Paired
    .then(Write("gh")).then(Hop0("gh")).then(Hop1("gh"))
    .then(TcpRst("g", false))
    .expect(Safety and pr.get("g").cause == "SocketError" and pr.get("g").errno == "ECONNRESET" and ctl.get("gh") == [both])
    .then(VCtl("gh"))
    .expect(Safety and pr.get("h").out == "aborted" and pr.get("h").cause == "PeerAbort" and dir.get("gh").rst
            and not(dir.get("gh").eof) and ep.get("h") == "reset" and vs.get("h").tcls == "abort")
  // R2-END, the other way: the far peer resets; the guest application sees a reset
  run RemoteResetReachesGuestAppAsReset = Paired
    .then(Write("hg")).then(Hop0("hg")).then(Hop1("hg"))
    .then(TcpRst("h", false)).expect(Safety and pr.get("h").cause == "SocketError" and pr.get("h").errno == "ECONNRESET")
    .then(VCtl("hg"))
    .expect(Safety and pr.get("g").out == "aborted" and pr.get("g").cause == "PeerAbort" and dir.get("hg").rst
            and not(dir.get("hg").eof) and ep.get("g") == "reset")
  // CLEAN-FULL-END: the peer's 0x2 and later its close (0x3, R newly set after this side shut its send) reach the
  // guest before it drained the direction: the half-close stays clean and the guest pair completes
  run HalfCloseThenPeerCloseIsClean = Paired
    .then(Fin("gh")).then(HopEnd0("gh")).then(VCtl("gh")).then(HopEnd1("gh")).then(ReadEof("gh"))
    .then(Write("hg")).then(Fin("hg")).then(Hop0("hg")).then(HopEnd0("hg"))
    .expect(Safety and pr.get("h").out == "completed" and dir.get("hg").q1 == [1])
    .then(TakeEnd("h")).expect(Safety and ctl.get("hg") == [Set("S"), both])
    .then(VCtl("hg")).then(VCtl("hg")).expect(Safety and live(cur, "g") and vs.get("g").cls == "clean")
    .then(Hop1("hg")).then(HopEnd1("hg")).then(Read("hg")).then(ReadEof("hg"))
    .expect(Safety and pr.get("g").out == "completed" and dir.get("hg").eof and not(dir.get("hg").rst))
  // R2-END: the host closes with the guest's incoming direction still open (a 0x3 with no 0x2 before it): the
  // guest aborts and resets its application, although the other direction had ended cleanly
  run PeerCloseWithDirectionOpenAborts = Paired
    .then(Fin("gh")).then(HopEnd0("gh")).then(VCtl("gh")).then(HopEnd1("gh")).then(ReadEof("gh"))
    .then(Abort("h")).then(VCtl("hg"))
    .expect(Safety and pr.get("g").out == "aborted" and pr.get("g").cause == "PeerAbort" and dir.get("hg").rst
            and dir.get("gh").eof and vs.get("g").tcls == "abort")
  // a far peer that shuts down only its read side looks like a stalled reader: the writer is blocked, nothing
  // grows, and nothing aborts (owner policy: none)
  run ReadShutdownPeerStallsLikeSlowReader = Paired
    .then(ShutRd("gh"))
    .then(Write("gh")).then(Hop0("gh")).then(Hop1("gh")).then(Write("gh")).then(Hop0("gh")).then(Write("gh"))
    .expect(Safety and not(canWrite("gh")) and dir.get("gh").q2 == [1] and live(cur, "g") and live(cur, "h")
            and not(canRead("gh")))
  // the owner's abort resets both far ends: the host's TCP half by tcp_abort, the guest's through the 0x3
  run OwnerAbortResetsBothFarEnds = Paired
    .then(Abort("h")).expect(Safety and pr.get("h").cause == "Detached" and pr.get("h").rec == "queued" and dir.get("gh").rst)
    .then(VCtl("hg"))
    .expect(Safety and pr.get("g").cause == "PeerAbort" and dir.get("hg").rst and ep.get("g") == "reset" and ep.get("h") == "reset")
  // the owner's death: the context's release aborts its pair with no record; both far ends see resets
  run ContextReleaseResetsEveryPair = Paired
    .then(Write("hg")).then(Hop0("hg"))
    .then(CtxRelease("g")).expect(Safety and pr.get("g").ph == "ended" and pr.get("g").rec == "none" and dir.get("hg").rst)
    .then(VCtl("gh")).expect(Safety and pr.get("h").out == "aborted" and dir.get("gh").rst and not(dir.get("gh").eof))
  // the VMM exits: the host vsock's error aborts the host pair and resets the far peer, never an EOF
  run VmmExitAbortsHostHalf = Paired
    .then(Write("gh")).then(Fin("gh")).then(Hop0("gh"))
    .then(VmmExit).then(FwdErr("h"))
    .expect(Safety and pr.get("h").cause == "SocketError" and dir.get("gh").rst and not(dir.get("gh").eof)
            and pr.get("g").ph == "gone")
  // END-IDEMPOTENT: an RST after the pair completed changes nothing
  run TrailingRstAfterCompletionIsInert = CleanBothWaysCompletes
    .then(TrailingRst("g")).then(VCtl("hg"))
    .expect(Safety and pr.get("g").out == "completed" and pr.get("g").nrec == 1 and dir.get("hg").eof and ep.get("g") == "alive")
  // a dial: the guest application resets; the watch classifies the guest pair's 0x3 as an abort before the end is
  // readable, so the dialer's GuestStream returns Err(ConnectionReset), not Ok(0)
  run WatchedStreamSeesGuestResetAsAbort = InitWith("watch")
    .then(Attach("g")).then(Write("gh")).then(Hop0("gh")).then(Read("gh"))
    .then(TcpRst("g", false)).then(VCtl("gh"))
    .expect(Safety and pr.get("h").out == "aborted" and pr.get("h").cause == "PeerAbort" and pr.get("h").rec == "queued"
            and gh.woken)
    .then(HolderEnd("eof")).then(ReadEof("gh"))
    .expect(Safety and dir.get("gh").rst and not(dir.get("gh").eof) and dir.get("gh").got == [1] and hres == "GuestAbort")
  // ---- ends present at attach (EVMOD-PAIR, U-EV-2)
  // the host forwards the destination's FIN as 0x2 and then aborts (0x3) before the guest TcpConnect attaches on
  // Paired: the guest's attach finds both flags and classifies the end once, at attach, as an abort — never the
  // clean half-close the first packet alone would be
  run HostHalfCloseThenAbortBeforeGuestAttachIsAbort = InitWith("stream").then(Attach("h"))
    .then(Write("hg")).then(Fin("hg")).then(Hop0("hg")).then(HopEnd0("hg"))
    .expect(Safety and ctl.get("hg") == [Set("S")] and pr.get("g").ph == "unattached")
    .then(Abort("h")).expect(Safety and ctl.get("hg") == [Set("S"), both] and pr.get("h").cause == "Detached")
    .then(VCtl("hg")).then(VCtl("hg"))
    .expect(Safety and vs.get("g").ps == both and vs.get("g").ncls == 0 and pr.get("g").ph == "unattached")
    .then(Attach("g"))
    .expect(Safety and pr.get("g").out == "aborted" and pr.get("g").cause == "PeerAbort" and vs.get("g").att == "end"
            and vs.get("g").clsAt == "attach" and vs.get("g").ncls == 1 and dir.get("hg").rst
            and not(dir.get("hg").eof) and ep.get("g") == "reset" and not(canReadEof(cur, "hg")))
  // the guest owner closes a flow before the host attached (a pending intake child that reset, the pairing
  // deadline), its 0x3 still in flight when the host attaches: the attach finds V_h ESTABLISHED with no end and is
  // accepted; the callback then classifies the 0x3 as an abort and the far peer sees a reset (T-4: the branch in
  // which the end is not yet in V_h's state at attach — see README, "Underspecified points", U-EV3-1)
  run GuestCloseBeforeHostAttachAborts = InitWith("stream")
    .then(Write("hg")).then(CloseUnattached("g"))
    .expect(Safety and pr.get("g").byRel and pr.get("g").rec == "none" and ctl.get("gh") == [both] and ep.get("g") == "reset")
    .then(Attach("h")).expect(Safety and attRes.get("h") == "ok" and live(cur, "h") and vs.get("h").att == "noend")
    .then(VCtl("gh"))
    .expect(Safety and pr.get("h").out == "aborted" and pr.get("h").cause == "PeerAbort" and pr.get("h").rec == "queued"
            and vs.get("h").clsAt == "callback" and dir.get("gh").rst and not(dir.get("gh").eof) and ep.get("h") == "reset")
  // T-4: the same close processed before the host attaches: V_h is CLOSING (the 0x3 found its queue empty); the
  // attach is refused HalfEnded, no pair or record is created, the owner closes D with SO_LINGER{1,0} (the far peer
  // sees a reset) and V_h, counts forwarder_attach_ended and writes Abort { flow }
  run GuestCloseWithEmptyQueueBeforeHostAttachIsRefusedAndFlowAborts = InitWith("stream")
    .then(Write("hg")).then(CloseUnattached("g"))
    .then(VCtl("gh")).expect(Safety and pr.get("h").ph == "unattached" and vs.get("h").ps == both and vs.get("h").st == "closing")
    .then(Attach("h"))
    .expect(Safety and attRes.get("h") == "halfEnded" and attEnded.get("h") == 1 and pr.get("h").ph == "refused"
            and pr.get("h").nrec == 0 and pr.get("h").rec == "none" and ep.get("h") == "reset" and dir.get("gh").rst
            and not(dir.get("gh").eof) and vs.get("h").sent == both and guestAbortSent)
    .then(VCtl("hg")).expect(Safety and pr.get("g").ph == "ended" and pr.get("g").nrec == 0)
  // T-4 (the spike's eg-drst): the destination resets before the host attaches: the attach is refused HalfEnded; the
  // host closes V_h and writes Abort { flow }; the guest owner, holding the flow unattached, closes it (its
  // application sees a reset)
  run DestinationResetBeforeHostAttachIsRefusedAndFlowAborts = InitWith("stream")
    .then(TcpRst("h", false)).expect(Safety and tcpErr.get("h") == "ECONNRESET" and pr.get("h").ph == "unattached")
    .then(Attach("h"))
    .expect(Safety and attRes.get("h") == "halfEnded" and attEnded.get("h") == 1 and pr.get("h").ph == "refused"
            and pr.get("h").nrec == 0 and ctl.get("hg") == [both] and guestAbortSent and not(canAttach(cur, "g")))
    .then(GuestAbort("g"))
    .expect(Safety and pr.get("g").ph == "ended" and pr.get("g").byRel and pr.get("g").nrec == 0 and ep.get("g") == "reset"
            and dir.get("hg").rst and not(guestAbortSent))
  // T-4 (the spike's eg-rst): the guest application's intake child resets before Paired: the guest's attach on
  // Paired is refused HalfEnded; the guest closes I and V_g; the host pair classifies the 0x3 as an abort and resets
  // the far peer
  run IntakeChildResetBeforePairedIsRefusedAndFlowAborts = InitWith("stream")
    .then(Attach("h")).then(TcpRst("g", false)).expect(Safety and tcpErr.get("g") == "ECONNRESET" and live(cur, "h"))
    .then(Attach("g"))
    .expect(Safety and attRes.get("g") == "halfEnded" and attEnded.get("g") == 1 and pr.get("g").ph == "refused"
            and pr.get("g").nrec == 0 and ctl.get("gh") == [both])
    .then(VCtl("gh"))
    .expect(Safety and pr.get("h").out == "aborted" and pr.get("h").cause == "PeerAbort" and ep.get("h") == "reset"
            and dir.get("gh").rst and not(dir.get("gh").eof))
  // T-2 (the spike's eg-sffin): the destination's banner and the host's 0x2 reach V_g before the guest TcpConnect
  // attaches: the attach seeds a clean half-close; the banner and the end are delivered and both pairs complete
  run EarlyVsockHalfCloseBeforeGuestAttachCompletes = InitWith("stream").then(Attach("h"))
    .then(Write("hg")).then(Fin("hg")).then(Hop0("hg")).then(HopEnd0("hg")).then(VCtl("hg"))
    .expect(Safety and vs.get("g").ps == Set("S") and vs.get("g").st == "est" and pr.get("g").ph == "unattached")
    .then(Attach("g"))
    .expect(Safety and attRes.get("g") == "ok" and vs.get("g").cls == "clean" and vs.get("g").clsAt == "attach")
    .then(Hop1("hg")).then(HopEnd1("hg")).then(Read("hg")).then(ReadEof("hg"))
    .then(Fin("gh")).then(HopEnd0("gh")).then(VCtl("gh")).then(HopEnd1("gh")).then(ReadEof("gh"))
    .expect(Safety and pr.get("g").out == "completed" and pr.get("h").out == "completed" and dir.get("hg").got == [1]
            and dir.get("hg").eof and dir.get("gh").eof and not(gh.unclassified))
  // T-3 (the spike's eg-wf): the guest application writes and sends its FIN on the intake child before Paired; the
  // guest attaches its half in CLOSE_WAIT; every unit is delivered, the far end reads EOF, the records are Completed
  run AppWriteAndFinBeforePairedAttachesInCloseWaitAndCompletes = InitWith("stream")
    .then(Write("gh")).then(Fin("gh")).then(Attach("h")).expect(Safety and tcpSt(cur, "g") == "closeWait")
    .then(Attach("g")).expect(Safety and attRes.get("g") == "ok" and live(cur, "g"))
    .then(Hop0("gh")).then(HopEnd0("gh")).then(VCtl("gh")).then(Hop1("gh")).then(HopEnd1("gh"))
    .then(Read("gh")).then(ReadEof("gh"))
    .then(Fin("hg")).then(HopEnd0("hg")).then(VCtl("hg")).then(HopEnd1("hg")).then(ReadEof("hg"))
    .then(TakeEnd("g")).then(TakeEnd("h"))
    .expect(Safety and pr.get("g").out == "completed" and pr.get("h").out == "completed" and dir.get("gh").got == [1]
            and dir.get("gh").eof and dir.get("hg").eof and gh.faults == 0)
  // T-8 (CLEAN-FULL-END, crossing half-closes): each side sends its 0x2 before it processes the other's; the host
  // completes and closes (0x3); the guest, which shut its send in the step that sent its 0x2, classifies the 0x3 as
  // none: the guest pair completes and both far ends read EOF
  run PeerCloseRacingOwnHalfCloseIsClean = Paired
    .then(Fin("hg")).then(HopEnd0("hg")).then(Fin("gh")).then(HopEnd0("gh"))
    .expect(Safety and vs.get("g").sshut and ctl.get("hg") == [Set("S")] and ctl.get("gh") == [Set("S")])
    .then(VCtl("gh")).then(HopEnd1("gh")).expect(Safety and pr.get("h").out == "completed")
    .then(ReadEof("gh")).then(TakeEnd("h")).expect(Safety and ctl.get("hg") == [Set("S"), both])
    .then(VCtl("hg")).then(VCtl("hg")).expect(Safety and live(cur, "g") and vs.get("g").ps == both)
    .then(HopEnd1("hg")).then(ReadEof("hg"))
    .expect(Safety and pr.get("g").out == "completed" and dir.get("gh").eof and dir.get("hg").eof
            and not(dir.get("hg").rst))
  // ---- a TCP reset after a FIN (EVMOD-END, TCP halves)
  // the far peer half-closes (its FIN is forwarded and read by the guest application as end-of-stream) and then
  // resets with the guest→remote direction open: the host half's error aborts the pair; the guest application
  // sees a reset, and the unit the peer never read is lost — never a clean end
  run RemoteFinThenResetAbortsPair = Paired
    .then(Write("gh")).then(Hop0("gh")).then(Hop1("gh"))
    .then(Fin("hg")).then(HopEnd0("hg")).then(VCtl("hg")).then(HopEnd1("hg")).then(ReadEof("hg"))
    .expect(Safety and dir.get("hg").eof and live(cur, "h") and live(cur, "g"))
    .then(TcpRst("h", true))
    .expect(Safety and pr.get("h").out == "aborted" and pr.get("h").cause == "SocketError" and pr.get("h").errno == "EPIPE"
            and ctl.get("hg") == [both])
    .then(VCtl("hg"))
    .expect(Safety and pr.get("g").out == "aborted" and pr.get("g").cause == "PeerAbort" and ep.get("g") == "reset"
            and dir.get("gh").loss and dir.get("gh").got == [])
  // T-1 (the spike's g-rst-hg-only): the guest application half-closes (its FIN is forwarded) and then resets with
  // the host→guest direction open: its intake child is in CLOSE_WAIT, so the guest pair aborts SocketError EPIPE in
  // the step of the reset; the host classifies the 0x3 after the 0x2 as an abort (its send was open)
  run GuestAppFinThenResetAbortsPairWithEpipe = Paired
    .then(Write("hg")).then(Hop0("hg")).then(Hop1("hg"))
    .then(Fin("gh")).then(HopEnd0("gh")).expect(Safety and tcpSt(cur, "g") == "closeWait")
    .then(TcpRst("g", true))
    .expect(Safety and pr.get("g").out == "aborted" and pr.get("g").cause == "SocketError" and pr.get("g").errno == "EPIPE"
            and ctl.get("gh") == [Set("S"), both])
    .then(VCtl("gh")).then(VCtl("gh"))
    .expect(Safety and pr.get("h").out == "aborted" and pr.get("h").cause == "PeerAbort" and ep.get("h") == "reset"
            and dir.get("gh").rst and not(dir.get("gh").eof))
  // ---- the host's Forwarder recovery (EVMOD-CTL, § Context lifetime and owner failure)
  // abort-all (the owner's abort: record queued, far ends reset), then the old context closed, then a new one
  // opened: no pair of the old context forwards, the old record is never read, one context at a time
  run ForwarderRecoveryAbortsThenReopens = Paired
    .then(Write("gh")).then(Hop0("gh"))
    .then(Abort("h")).expect(Safety and pr.get("h").cause == "Detached" and pr.get("h").rec == "queued" and ep.get("h") == "reset")
    .then(CtxRelease("h"))
    .expect(Safety and ctx.get("h") == "released" and pr.get("h").nrec == 1 and pr.get("h").rec == "queued"
            and pr.get("h").closed and not(gh.hostDead))
    .then(Reopen("h"))
    .expect(Safety and ctx.get("h") == "open" and gen.get("h") == 1 and oldCtx.get("h") == "released"
            and pr.get("h").ph == "ended" and not(canTakeEnd(cur, "h")) and not(enHop1("gh")))
    .then(VCtl("hg"))
    .expect(Safety and pr.get("g").out == "aborted" and pr.get("g").cause == "PeerAbort" and ep.get("g") == "reset"
            and dir.get("hg").rst)
  // D-5: a release that finds the host pair live is the host owner's death: the pair ends with no record and no
  // recovery reopen follows (production has no in-process successor: a new serve process reclaims the VM)
  run HostOwnerDeathIsFinal = Paired
    .then(CtxRelease("h"))
    .expect(Safety and gh.hostDead and pr.get("h").byRel and pr.get("h").rec == "none" and ep.get("h") == "reset")
    .then(Reopen("h")).fail()
  // ---- a watch ended by its owner (WATCH-ABORT, EVMOD-STREAM-ABORT, T-5) or dropped by its holder (T-6)
  // the owner aborts the watched stream of a dial: the watch's record is queued and V_h is untouched (no flag sent,
  // its queue as it was); the stream is ended by the owner — the holder can neither read the queued unit nor write,
  // and its next call reports OwnerAbort (Err(ConnectionAborted)) at once; the owner wrote Abort { flow }, which the
  // guest owner takes by aborting its pair (the guest application sees a reset)
  run OwnerAbortOfWatchLeavesVhAndStreamReportsAborted = InitWith("watch")
    .then(Attach("g")).then(Write("gh")).then(Hop0("gh")).expect(Safety and dir.get("gh").q1 == [1])
    .then(Abort("h"))
    .expect(Safety and pr.get("h").out == "aborted" and pr.get("h").cause == "Detached" and pr.get("h").rec == "queued"
            and vs.get("h").sent == Set() and ctl.get("hg") == [] and dir.get("gh").q1 == [1] and ep.get("h") == "alive"
            and strmAbort and guestAbortSent and not(canReadAny(cur, "gh")) and not(canWriteAny(cur, "hg")))
    .then(ReadEof("gh"))
    .expect(Safety and hres == "OwnerAbort" and dir.get("gh").rst and not(dir.get("gh").eof) and dir.get("gh").got == [])
    .then(GuestAbort("g"))
    .expect(Safety and pr.get("g").out == "aborted" and pr.get("g").cause == "Detached" and ep.get("g") == "reset"
            and ctl.get("gh") == [both] and not(guestAbortSent))
    .then(VCtl("gh")).expect(Safety and pr.get("h").cause == "Detached" and pr.get("h").nrec == 1)
  // the guest pair has already ended on Abort { flow } when the holder drops its stream: the drop only closes V_h,
  // whose 0x3 reaches an ended guest pair and changes nothing (END-IDEMPOTENT)
  run OwnerAbortSendsGuestAbortThenHolderDropIsInert = InitWith("watch")
    .then(Attach("g")).then(Abort("h")).then(GuestAbort("g"))
    .expect(Safety and pr.get("g").out == "aborted" and pr.get("g").nrec == 1 and pr.get("h").nrec == 1)
    .then(HolderDrop).expect(Safety and ctl.get("hg") == [both] and gh.dropped and not(live(cur, "h")))
    .then(VCtl("hg")).then(VCtl("gh"))
    .expect(Safety and pr.get("g").out == "aborted" and pr.get("g").cause == "Detached" and pr.get("g").nrec == 1
            and pr.get("h").cause == "Detached" and pr.get("h").nrec == 1)
  // the context's release shuts the watched V_h (SHUT_RDWR, the guest sees 0x3) with no record; the holder's read
  // fails ENOTCONN (V_h left ESTABLISHED, F11 corrected) and its GuestStream reports the owner's own end
  run ContextReleaseShutsWatchedVhAndHolderReadsNotConnected = InitWith("watch")
    .then(Attach("g")).then(Write("gh")).then(Hop0("gh")).then(Read("gh"))
    .then(CtxRelease("h"))
    .expect(Safety and pr.get("h").byRel and pr.get("h").rec == "none" and vs.get("h").sent == both and ctl.get("hg") == [both])
    .then(HolderEnd("err")).then(ReadEof("gh"))
    .expect(Safety and gh.hobs == "err" and hres == "OwnerAbort" and dir.get("gh").rst and dir.get("gh").got == [1])
    .then(VCtl("hg")).expect(Safety and pr.get("g").cause == "PeerAbort" and ep.get("g") == "reset")
  // the context's release ends a watch with no record; the GuestStream resolves the holder's end (here its read
  // returned 0) from the owner's own end of the watch: OwnerAbort
  run ReleasedWatchWithNoRecordReadsAsAbort = InitWith("watch")
    .then(Attach("g")).then(Write("gh")).then(Hop0("gh")).then(Read("gh"))
    .then(CtxRelease("h"))
    .expect(Safety and pr.get("h").byRel and pr.get("h").rec == "none" and vs.get("h").sent == both and ep.get("h") == "alive")
    .then(HolderEnd("eof")).then(ReadEof("gh"))
    .expect(Safety and dir.get("gh").rst and not(dir.get("gh").eof) and dir.get("gh").got == [1] and hres == "OwnerAbort")
    .then(VCtl("hg")).expect(Safety and pr.get("g").cause == "PeerAbort")
  // U-R1 (EVMOD-CTL recovery step 1): the guest application resets, the watch records the abort and the holder's
  // stream reports it (GuestAbort); then the host Forwarder recovery closes the context: the held stream is ended
  // as an owner abort (its dials entry leaves, Abort { flow } written), while what it reported stays GuestAbort
  run RecoveryEndsHeldStreamWhoseWatchEnded = WatchedStreamSeesGuestResetAsAbort
    .then(CtxRelease("h"))
    .expect(Safety and strmAbort and hres == "GuestAbort" and guestAbortSent and not(gh.hostDead) and ep.get("h") == "alive"
            and ctx.get("h") == "released")
    .then(Reopen("h")).expect(Safety and gen.get("h") == 1 and strmAbort and hres == "GuestAbort")
  // T-6: the holder drops its stream: the watch is aborted (record queued, no live watch) and V_h closed; the guest's
  // callback classifies the 0x3 as an abort and resets the guest application
  run DroppedStreamAbortsWatchThenGuestSeesAbort = InitWith("watch")
    .then(Attach("g")).then(Write("gh")).then(Hop0("gh")).then(Read("gh"))
    .then(HolderDrop)
    .expect(Safety and gh.dropped and not(live(cur, "h")) and pr.get("h").cause == "Detached" and pr.get("h").rec == "queued"
            and ctl.get("hg") == [both])
    .then(VCtl("hg"))
    .expect(Safety and pr.get("g").out == "aborted" and pr.get("g").cause == "PeerAbort" and ep.get("g") == "reset")
    .then(TakeEnd("h")).expect(Safety and pr.get("h").rec == "read")
  // the VMM exits with a dial live: the orphan sweep makes V_h's end readable before its sk_error_report runs; the
  // holder reads the error first, finds no record and no end of its owner, and waits; the callback queues the
  // record and then wakes it; the GuestStream reports the record's abort
  run VmmExitWithWatchRecordInFlightResolvesFromRecord = InitWith("watch")
    .then(Attach("g")).then(Write("gh")).then(Hop0("gh")).then(Read("gh"))
    .then(VmmExit).expect(Safety and vs.get("h").err and live(cur, "h") and holderReadable(cur) == Set("err"))
    .then(HolderEnd("err")).expect(Safety and live(cur, "h") and watchOutcome(cur) == "wait" and not(canReadEof(cur, "gh")))
    .then(FwdErr("h"))
    .expect(Safety and pr.get("h").out == "aborted" and pr.get("h").cause == "SocketError" and pr.get("h").rec == "queued"
            and gh.woken)
    .then(ReadEof("gh"))
    .expect(Safety and dir.get("gh").rst and not(dir.get("gh").eof) and dir.get("gh").got == [1] and hres == "GuestAbort")
  // ---- datagram mode (EVMOD-DGRAM)
  run DgramPaired = InitWith("dgram").then(Attach("h")).then(Attach("g")).expect(DgramSafety)
  // datagrams of sizes 1, 0 (empty) and 2 are delivered once, whole and in order
  run DatagramsInOrderWithEmpty = DgramPaired
    .then(SendDg("gh", 1)).then(Hop0("gh")).then(Hop0("gh")).then(Hop1("gh")).then(DRead("gh"))
    .then(SendDg("gh", 0)).then(Hop0("gh")).then(Hop1("gh")).then(DRead("gh"))
    .then(SendDg("gh", 2)).then(Hop0("gh")).then(Hop0("gh")).then(Hop0("gh")).then(Hop1("gh")).then(DRead("gh"))
    .expect(DgramSafety and dg.get("gh").got == [[10, 11], [20], [30, 31, 32]])
  // the vsock accepts only the header of datagram 2's frame; the datagram stays queued; after the credit returns
  // the run resumes at the next item — nothing duplicated or lost
  run PartialFrameResumedNotDuplicated = DgramPaired
    .then(SendDg("gh", 1)).then(Hop0("gh")).then(Hop0("gh"))
    .then(SendDg("gh", 2)).then(Hop0("gh"))
    .expect(DgramSafety and dir.get("gh").q1 == [10, 11, 20] and dg.get("gh").dq0 == [2] and dg.get("gh").foff == 1)
    .expect(not(enHop0("gh")))
    .then(Hop1("gh")).then(DRead("gh")).then(Hop0("gh")).then(Hop0("gh")).then(Hop1("gh"))
    .expect(DgramSafety and dg.get("gh").got == [[10, 11]] and dg.get("gh").dq2 == [[20, 21, 22]])
  // the target's send blocks: the vsock fills, the forwarder stops, the source queue fills and the next datagram
  // is dropped there — and only there; when the send unblocks, the queued datagrams move on in order
  run StalledUdpTargetDropsAtSource = DgramPaired
    .then(UdpBlock("gh"))
    .then(SendDg("gh", 2)).then(Hop0("gh")).then(Hop0("gh")).then(Hop0("gh"))
    .expect(not(enHop1("gh")))
    .then(SendDg("gh", 0)).expect(not(enHop0("gh")))
    .then(SendDg("gh", 1)).expect(DgramSafety and dg.get("gh").sdrop == Set(3) and dg.get("gh").dq0 == [2])
    .then(UdpUnblock("gh")).then(Hop1("gh")).then(DRead("gh")).then(Hop0("gh")).then(Hop1("gh")).then(DRead("gh"))
    .expect(DgramSafety and dg.get("gh").got == [[10, 11, 12], [20]] and dg.get("gh").fdrop == Set())
  // EV-DGRAM D1: the vsock carries only the header of datagram 1 when its peer ends the direction (a 0x2): the
  // callback classifies a clean half-close, and the run, finding the head frame incomplete with the peer's
  // direction ended, aborts the pair FrameInvalid instead of waiting for the rest
  run TruncatedFrameAtPeerEndAbortsFrameInvalid = DgramPaired
    .then(SendDg("gh", 2)).then(Hop0("gh")).expect(DgramSafety and dir.get("gh").q1 == [10] and dg.get("gh").foff == 1)
    .then(DgHalfClose("g")).expect(DgramSafety and ctl.get("gh") == [Set("S")] and not(enHop0("gh")))
    .then(VCtl("gh")).expect(DgramSafety and live(cur, "h") and vs.get("h").cls == "clean" and headPartial(cur, "gh"))
    .then(Hop1("gh"))
    .expect(DgramSafety and pr.get("h").out == "aborted" and pr.get("h").cause == "FrameInvalid"
            and pr.get("h").rec == "queued" and ctl.get("hg") == [both] and dg.get("gh").got == [])

  // non-vacuity witness kept: no named run ends a watch in a half-close (violated: the end is reachable)
  val WitnessNoWatchHalfClose: bool = not(pr.get("h").out == "halfclose")
  // the VMM exits (the true side of CtxGoneIffVmDown is reached)
  val WitnessNoVmmExit: bool = vmUp
}

// ------------------------------------------------------------------ design instances
// stream mode: 1 unit gh, 1 hg, one fault per trace
module pair_ok { import fwd_pair(OFF = Set(), CFG = "ok", REC = false).* from "./fwd_pair" }
// watch mode (a dial): 1 unit gh, 1 hg, one fault per trace
module pair_watch { import fwd_pair(OFF = Set(), CFG = "watch", REC = false).* from "./fwd_pair" }
// stream mode for the liveness checks: 1 unit each way, one fault per trace (TLC's liveness mode evaluates the
// fairness action's enabledness per state, which makes a larger instance take hours)
module pair_live { import fwd_pair(OFF = Set(), CFG = "live", REC = false).* from "./fwd_pair" }
// stream mode for StalledSenderIsBlocked: 4 units gh (more than the pipeline holds), none hg, no fault
module pair_stall { import fwd_pair(OFF = Set(), CFG = "stall", REC = false).* from "./fwd_pair" }
// watch mode for the liveness checks of WATCH-ABORT and the holder's drop (T-5, T-6): 1 unit gh, none hg, one fault per
// trace
module pair_wlive { import fwd_pair(OFF = Set(), CFG = "wlive", REC = false).* from "./fwd_pair" }
// datagram mode: 1 datagram gh, 1 hg, one fault per trace
module pair_dgram { import fwd_pair(OFF = Set(), CFG = "dgram", REC = false).* from "./fwd_pair" }
// datagram mode for the liveness check of EV-DGRAM D1: 1 datagram gh, none hg, one fault per trace, no send-buffer
// toggles
module pair_dlive { import fwd_pair(OFF = Set(), CFG = "dlive", REC = false).* from "./fwd_pair" }
// stream and datagram mode, 2 units / 2 datagrams gh and none hg, one fault per trace: the multi-unit and multi-datagram
// instance (a reset, an exit or a send block while the second unit or datagram is queued or mid-frame)
module pair_mid { import fwd_pair(OFF = Set(), CFG = "mid", REC = false).* from "./fwd_pair" }
// conformance main (D-UP-1): every mode, recording `conf_step`; only the named runs are checked on it
module pair_conf { import fwd_pair(OFF = Set(), CFG = "conf", REC = true).* from "./fwd_pair" }
