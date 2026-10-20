# ADR-0180 — A flow's end is classified clean or abort in the forwarder's socket callbacks, from the kernel's own end facts

## Status

**Accepted.** One decision: a flow's end is classified from the facts the
kernel records for it — the vsock SHUTDOWN flags, an RST, a socket's
`sk_err` — and the classification is computed in the forwarder's own socket
callbacks (ADR-0197). The rule was approved by the user 2026-10-11 (R2-END)
and its computation in the forwarder's callbacks on 2026-10-17 (EVMOD-END);
it is written here as one current decision, not implemented. Two
consequences of the rule that guest-slot spike V-4 measured, that a trailing
end is a no-op (END-IDEMPOTENT) and that a clean full end is `0x2` each way
before close (CLEAN-FULL-END), were confirmed by the user 2026-10-15; they
follow from the classification table and are not separate choices. GH #295.
Contract: the #295 feature delta, § *[REF] vsock data path — the in-kernel
event-driven forwarder (EVMOD)*, "End classification" and "Ends present at
attach".

Related decisions, each separately reversible: a half that has already ended
when it is attached is refused and its flow aborted (ADR-0210); how a clean
half-close is completed once classified (ADR-0183); stop and abort semantics
(ADR-0160); how an owner learns a pair's end (ADR-0202); that the host
forwarder watches a dialled vsock with these classes (ADR-0216); how a
dialled stream's holder learns the class of its flow's end (the `GuestStream`
contract of the feature delta, EVMOD-STREAM-END) and what an owner's abort
of a watch does (ADR-0211); the host side of these classes for every guest
(ADR-0207; a guest that runs no Linux kernel is
[GH #321](https://github.com/overdrive-sh/overdrive/issues/321)).

## Context

A TCP flow crosses the VM boundary on one vsock STREAM (ADR-0147). Each side
propagates the end of each direction across the boundary: a clean end (FIN)
as a clean half-close, a reset or a crash as an abort with reset semantics.
The side that sees a flow vsock end must know which of the two the peer did.

The vsock socket API does not say:

- An established-state RST runs `virtio_transport_do_close`
  (`net/vmw_vsock/virtio_transport_common.c:1463-1464`, `:1244-1252 @ v7.0`),
  which sets `SOCK_DONE` and `peer_shutdown = SHUTDOWN_MASK` and never sets
  `sk_err`, so `read()` returns 0. V-28 measured a guest application's reset
  arriving at the TLS client as a clean EOF in 192 of 192 flows
  (`spike/findings-v28-legs-vsock.md`, Q2b).
- `vsock_poll` reports a peer's end only as `EPOLLRDHUP`, and `EPOLLHUP` only
  when this side has also shut down its send side
  (`net/vmw_vsock/af_vsock.c:1247-1259 @ v7.0`), so half-close and full close
  look alike (V-13: `EPOLLRDHUP` alone in 79 of 79 end events).

The kernel does carry the fact on the wire, and records it before the end is
readable:

- `shutdown(SHUT_WR)` sends a SHUTDOWN op with the SEND flag only, `0x2`
  (`net/vmw_vsock/af_vsock.c:1179-1222`,
  `net/vmw_vsock/virtio_transport_common.c:1084-1096`,
  `include/uapi/linux/virtio_vsock.h:94-95 @ v7.0`).
- `close()` — including the close of every open file at process exit — sends
  SHUTDOWN with `RCV|SEND`, `0x3`, or an RST when the peer had already shut
  down both directions (`net/vmw_vsock/virtio_transport_common.c:1288-1295
  @ v7.0`).
- The receiving socket's `sk_state_change` is called after every received
  SHUTDOWN, with its flags recorded in `peer_shutdown`
  (`virtio_transport_common.c:1443-1461 @ v7.0`), and after an RST
  (`:1463-1464` → `:1244-1253`), with the socket lock held (`:1661`, `:1705`).
  The vhost device release's orphan sweep sets `sk_err` and calls
  `sk_error_report` without a state change (`drivers/vhost/vsock.c:746-750 @
  v7.0`). It is the one writer that does not hold the socket lock: it stores
  `peer_shutdown = SHUTDOWN_MASK` (`:747`) before `sk_err = ECONNRESET`
  (`:749`) with no barrier, holding only `vsock_table_lock`
  (`net/vmw_vsock/af_vsock.c:458`, `:471 @ v7.0`), so a read between those
  stores returns end-of-stream with `sk_err` 0 before the class is recorded
  (`af_vsock.c:2326-2328`, `:2439-2442`; the `SS_UNCONNECTED` stored at
  `:748` is the value of `TCP_ESTABLISHED`). The guest transport's reset
  (`net/vmw_vsock/virtio_transport.c:413-423 @ v7.0`) is the same shape.
- A TCP half's end-of-stream does not prove a clean end: `tcp_recvmsg` tests
  `SOCK_DONE` before `sk_err` (`net/ipv4/tcp.c:2794`, `:2797 @ v7.0`), so a
  peer that sends a FIN and then a reset reads as end-of-stream; the reset is
  reported by `sk_error_report` (`net/ipv4/tcp_input.c:4890-4902 @ v7.0`).

The forwarder of ADR-0197 already overrides `sk_state_change` and
`sk_error_report` on both halves of every pair. Guest-slot spike V-4
(`spike/findings-v4-guest-slot.md`, 2,000 trials, ten pre-states) measured
that a guest `close()` of a flow vsock reaches the host as exactly one
SHUTDOWN `0x3`, whatever was exchanged before, and that only the "`0x2` each
way, then `close()`" history ends with no direction open.

## Decision

- **Classification of the peer's direction:**
  - SHUTDOWN `0x2` is a clean half-close of the peer's direction;
  - SHUTDOWN `0x3` while any direction of the flow is still open is an abort;
  - RST is an abort;
  - `sk_err` on a vsock half (the orphan sweep) is an abort.
- **Where it is computed.** In the forwarder's `sk_state_change` and
  `sk_error_report` overrides on every forwarded or watched vsock half, per
  received SHUTDOWN or RST, before the end is readable on the locked
  receive path. The forwarder keeps
  the peer bits it has already acted on, so `0x2` followed later by `0x3`
  and a direct `0x3` are told apart. "This side has shut its own send
  direction", which makes a later `0x3` the clean release of a flow ended
  both ways, is the vsock half's own `sk_shutdown & SEND_SHUTDOWN` read under
  its socket lock; `vsock_shutdown` sets it before the SHUTDOWN leaves, under
  the same lock (`net/vmw_vsock/af_vsock.c:1203`, `:1216`, `:1221 @ v7.0`).
- **An end already present when a still-connected half is attached** is read
  in the same locked section that installs the overrides and classified once,
  by the same table: an owner never shuts a socket's send direction before
  attaching it, so `SEND_SHUTDOWN` alone is a clean half-close and any
  `RCV_SHUTDOWN` or RST that left the socket connected is an abort (a `0x2`
  followed by `0x3` before attach is an abort, as a direct `0x3` is). A TCP
  half in `CLOSE_WAIT` with no `sk_err` is a clean end of the peer's
  direction.
- **TCP halves.** `sk_err` on a TCP half aborts the pair while any direction
  of the pair is open, whatever its `recvmsg` returned. A TCP reset's errno
  is `ECONNRESET`, or `EPIPE` when the half was in `CLOSE_WAIT`
  (`net/ipv4/tcp_input.c:4905-4930 @ v7.0`); both are a reset.
- **An end read before the sweep's class is recorded.** A forwarder run
  that reads an end (end-of-stream or an error) from a vsock half whose
  class is not recorded takes `vsock_table_lock` before acting, then reads
  the recorded class again. The sweep holds that lock across all its stores
  and its `sk_error_report` call, so the acquisition orders the run after a
  sweep in progress, and the run aborts the pair with the class the
  override recorded, `SocketError { ECONNRESET }`. Socket lock then table
  lock is the kernel's own nesting (`vsock_bind`,
  `net/vmw_vsock/af_vsock.c:1106-1107`, `:863 @ v7.0`).
- **An end that was never classified** — an end on a vsock half whose class
  is still not recorded after that re-check — aborts the pair with its own
  cause, `Unclassified`, and is counted as an alarm. Every locked path that
  sets the peer's SEND bit without `sk_err` calls `sk_state_change`
  (`virtio_transport_common.c:1253`, `:1461 @ v7.0`), and both unlocked
  writers end in `sk_error_report` under `vsock_table_lock`, so the kernel
  source does not produce it; only a defect in the forwarder's own attach or
  callbacks does (hazard `h_noseed`).
- **A transition after a pair's or watch's end changes nothing.** It is not
  reclassified, releases nothing a second time and increments no counter a
  second time (END-IDEMPOTENT); ids are never reused (ADR-0202), so no later
  flow can receive it.
- **A `close()` with any direction still open is an abort**, whatever was
  exchanged before; a clean full end is `0x2` each way, then close
  (CLEAN-FULL-END). An owner closes a flow's sockets only after the pair's
  end record.
- **No control message carries an end.** No flow's end classification
  depends on the control session. Session loss still aborts every flow of the
  session (DG-5-F, ADR-0166).

## Alternatives considered

- **A BPF raw-tracepoint classifier on `vsock:virtio_transport_recv_pkt`
  writing a flow-event ring, beside the forwarder.** Two mechanisms for one
  fact, a ring that drops records when full (and the loss count and abort
  rule that follow), reused slots needing an epoch per record, and a
  recursion skip that leaves an end unclassified. The forwarder's own
  callbacks see every transition with the socket lock held. Rejected.
- **`HalfClose` / `HalfCloseAck` messages on the control session**, with an
  unacknowledged end-of-stream taken as an abort. Carries a fact the kernel
  already puts in the SHUTDOWN flags and costs a control round trip per
  half-close. Rejected.
- **The guest sends `Abort` on the control session before closing.** The
  ordering of two connections at the host is unproven, and the end path
  would run through userspace. Rejected.
- **Forward every vsock end-of-stream as a FIN**, or **treat every vsock
  end-of-stream as a clean half-close.** A reset becomes a clean close or a
  silent truncation at the far end (V-28: 192 of 192; the spike's `rst-G`
  read EOF). Rejected.
- **Map an unclassified end to the peer's abort.** Hides a broken invariant
  behind a reachable cause. Rejected.
- **Take every vsock end with no recorded class as the sweep's reset**,
  without the lock. Correct for today's kernel source, but it infers the
  cause from the symptom and removes the alarm for the forwarder's own
  defects (`h_noseed` would read as resets). Rejected.
- **Re-read `sk_err` without the lock, or leave the end for the pending
  `sk_error_report` to finish.** The sweep's stores carry no barrier, so an
  unlocked re-read proves nothing on a weakly ordered CPU; leaving the end
  pending hangs the pair if no report follows. Rejected.
- **Treat `EPOLLHUP` as an abort.** Never delivered with the end-of-stream
  (V-13). Rejected.
- **Discriminate by a zero-length send** (V-28: 192, 192 and 480 bad) or **by
  a delay** (a timing heuristic over a fact the kernel records exactly).
  Rejected.

## Consequences

- A clean half-close costs no control-session message; an abort reaches the
  far ends of both sides as a reset, whichever side dies or resets. Measured
  on Linux 7.0 with the forwarder on both sides of a Cloud Hypervisor microVM
  (P-69): every abort reached the other far end as a reset and every clean
  end as EOF after the last byte, one record per id, `unclassified_eof` 0.
  With the attach-time classification removed, 289 to 386 of 500 ends went
  unclassified (hazard `h_noseed`, P-71); without the TCP `sk_error_report`
  rule a FIN-then-reset left the far end in `CLOSE_WAIT` (hazard 16, 20 of
  20, P-69); with "this side shut its send" published after the SHUTDOWN
  left, 270 of 300 clean flows aborted (hazard `h_lateshut`, P-71).
- No ring, epoch, loss count or recursion count exists for flow ends.
- Owner death is classified by this rule without the owner acting. A guest
  PID-1 death ends the VMM (ADR-0182), and the vhost-vsock device release
  marks every connected host socket of that guest `ECONNRESET`
  (`drivers/vhost/vsock.c:721-751 @ v7.0`), an abort here; the sweep runs at
  the device's last close, which is why no Overdrive process keeps a copy of
  a VMM's vhost-vsock descriptor (ADR-0146). A `serve` death releases its
  forwarder context, which aborts every host pair (ADR-0199): the guest sees
  `0x3` with its direction open, an abort here.
- A datagram association's vsock has no production half-close; every
  production end of it is abort-class (ADR-0201).
- The rules are model-checked in `specs/quint/guest-flow-owner` (module
  `fwd_pair`; feature delta, § *Formal protocol model*).
