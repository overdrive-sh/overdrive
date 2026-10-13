# ADR-0180 — A flow's end is classified from the kernel's vsock SHUTDOWN flags, read by a raw-tracepoint program on both sides

## Status

**Accepted — decision R2-END approved by user 2026-10-11.** GH #295. Recorded
in the #295 feature delta, § *[REF] vsock Attachment Replacement DESIGN*
(§ *[REF] Wire contracts* for the flow-event record, § *[REF] Driven port —
host forwarder `overdrive-dataplane::guest_flow`* for the classifier and the
owner's event dispatch). How a clean half-close is completed once classified
is ADR-0183; stop and abort semantics are ADR-0160.

## Context

A TCP flow crosses the VM boundary on one vsock STREAM (ADR-0147). Each owner
— the host guest-flow owner and `overdrive-init` in the guest — propagates the
end of each direction across the boundary: a clean end (FIN) as a clean
half-close, a reset or a crash as an abort with reset semantics. The owner
that sees a flow vsock end must know which of the two the peer did.

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
  look alike. V-13 saw `EPOLLRDHUP` alone in 79 of 79 end events, and a
  surviving owner that took that end as a half-close sent its far end a FIN
  instead of a reset.

The kernel does carry the fact on the wire:

- `shutdown(SHUT_WR)` sends a SHUTDOWN op with the SEND flag only, `0x2`
  (`net/vmw_vsock/af_vsock.c:1179-1222`,
  `net/vmw_vsock/virtio_transport_common.c:1084-1096`,
  `include/uapi/linux/virtio_vsock.h:94-95 @ v7.0`).
- `close()` — including the close of every open file at process exit — sends
  SHUTDOWN with `RCV|SEND`, `0x3`, or an RST when the peer had already shut
  down both directions (`net/vmw_vsock/virtio_transport_common.c:1288-1295
  @ v7.0`).
- The tracepoint `vsock:virtio_transport_recv_pkt`
  (`include/trace/events/vsock_virtio_transport_common.h:89 @ v7.0`) fires
  for every received packet, with its op and flags, before the socket lookup
  (`net/vmw_vsock/virtio_transport_common.c:1622`, lookup at `:1639 @ v7.0`).

Measured on 7.0.0-29 host and guest with the vendored Cloud Hypervisor fork:

- T5 (`spike/findings-drain-t1-t6.md`): 0 misclassified in 660 microVM trials
  per side and 900 loopback trials — kill and `close()` gave `0x3`,
  `SHUT_WR` gave `0x2` — and the ring record preceded the `EPOLLRDHUP` wake
  by 11–287 µs.
- V-31 (`spike/findings-v31-r2-halfclose.md`): with the classifier, 15,000 of
  15,000 clean microVM half-closes, 0 clean EOF after a reset, every far end
  of a killed owner (guest, host or `serve`-side) reset. A perf-attached
  tracepoint program is skipped when `bpf_prog_active` is already raised on
  the CPU (`kernel/trace/bpf_trace.c:109-127 @ v7.0`; skip at `:115-126`) —
  by a running kprobe, perf-tracepoint, kprobe-multi or perf-event program
  (`kernel/trace/bpf_trace.c:2571`, `kernel/events/core.c:10596 @ v7.0`) or
  by a `bpf()` map operation (`include/linux/bpf.h:2547-2551`,
  `kernel/bpf/syscall.c:270`, `:317`, `:1862`, `:1974 @ v7.0`) — and 7 of
  1,173 loopback repetitions stranded a flow that way; a raw tracepoint runs
  regardless of `bpf_prog_active` (`kernel/trace/bpf_trace.c:1568 @ v7.0`)
  and is skipped only on recursion of the same program on one CPU, counted in
  the program's `recursion_misses` (`:2080-2083`); 0 of 1,312 repetitions
  (about 393,600 trials) stranded. T5's "record before the wake" holds for the
  raw form only.
- V-31: an unclassified TCP end-of-stream taken as clean returned a clean EOF
  after a destination reset in 6 of 12,000 trials; the TCP-source check below
  took that to 0.

## Decision

- **The classifier (A5).** A BPF program attached as a raw tracepoint (`raw_tp` or
  `tp_btf`, never a perf-attached tracepoint) on
  `vsock:virtio_transport_recv_pkt`, held by the host owner and by
  `overdrive-init`, writes one record to its side's flow-event ring for every
  received SHUTDOWN or RST op on a flow vsock. The record names the flow cell,
  carries the cell epoch and the flags, and is discarded by the owner when the
  epoch is not the cell's current one.
- **Classification of the peer's direction:**
  - SHUTDOWN `0x2` is a clean half-close of the peer's direction;
  - SHUTDOWN `0x3` while any direction of the flow is still open is an abort;
  - RST is an abort.
- **Unannounced end (A4).** An end-of-stream, `EPOLLRDHUP` or `EPOLLHUP` on a
  flow vsock for which the owner holds no classification record is an abort
  with reset semantics. Because the raw record precedes the wake, the owner
  reads its ring before acting on the wake; an end it cannot classify is
  never taken as clean.
- **TCP-source end check (A6).** An end-of-stream on an owner's TCP socket is
  a clean half-close only with a FIN in sequence. It is an abort when the
  socket is in `CLOSE` while the owner's write side toward it is still open,
  when the bytes forwarded from it equal `tcpi_bytes_received` (no FIN was
  counted), or when they exceed it.
- **Propagation.** An abort closes the flow with reset semantics
  (ADR-0160): TCP far ends are reset, and the flow vsock is closed while a
  direction is open, which the peer owner classifies as an abort. A clean
  half-close is propagated with `shutdown(SHUT_WR)` on the target once the
  direction has drained (ADR-0183); on a vsock target that sends `0x2`.
- **Dialled flows.** The host end of a flow the owner opens for a host
  platform client (ADR-0173) is not in the forwarding maps; the owner applies
  the same classifier to it by socket cookie and gives the dialer the
  classified end of the guest's direction. The dialer closing the stream
  while it is open sends `0x3` (the guest aborts the application connection);
  shutting down its write side sends `0x2`. For leg-S, the temporary bridge
  until #303, this carries reset fidelity both ways: a TLS peer's reset makes
  leg-S close the stream while open, so the guest resets the application
  connection; a guest application's reset makes the guest owner close its
  vsock while open, the stream reports an abort and leg-S resets the TLS
  client with `SO_LINGER{1,0}`; FIN maps to `0x2` both ways.
- **Owner death, by side.**
  - The guest owner is PID 1. Its death panics the guest before its files are
    closed (`kernel/exit.c:923-930`, files at `:971 @ v7.0`), so no SHUTDOWN
    is sent. The VMM then exits (ADR-0182), and the host flows of that guest
    are reset by the vhost-vsock device release, which marks every connected
    socket whose guest is gone `ECONNRESET`
    (`drivers/vhost/vsock.c:721-751`, reset at `:746-750`, release at `:753-768 @ v7.0`); `vsock_poll` reports
    that as `EPOLLERR` (`net/vmw_vsock/af_vsock.c:1243-1245 @ v7.0`), and the
    host owner aborts. The sweep runs at the last close of the device, so it
    relies on the VMM being the only holder of its vhost-vsock descriptor
    (ADR-0146).
  - When `overdrive serve` dies, closing its files sends `0x3` on every host
    flow vsock; the guest classifies each as an abort and resets the
    application connections. The guest stays up until boot reclamation of
    the next `serve` ends it (ADR-0159).
- **Datagram associations** have no half-close: any end of an association's
  vsock ends the association.
- **No control message carries an end.** No flow's end classification
  depends on the control session. Session loss still aborts every flow of the
  session (DG-5-F, ADR-0166).

## Alternatives considered

- **`HalfClose` / `HalfCloseAck` messages on the control session**, with an
  unacknowledged end-of-stream taken as an abort. Correct, but carries a
  fact the kernel already puts in the SHUTDOWN flags, costs one control round
  trip per half-close and two message types, and holds the announcing side in
  a pending state that only an acknowledgement or a session event ends.
  Rejected.
- **Treat every vsock end-of-stream as a clean half-close.** A reset becomes
  silent truncation (V-28: 192 of 192). Rejected.
- **Treat `EPOLLHUP` as an abort.** Never delivered with the end-of-stream
  (V-13). Rejected.
- **A perf-attached tracepoint.** Skipped while another BPF program runs on
  the CPU; a skipped record stranded flows (V-31). Rejected.
- **Discriminate by a zero-length send.** Misclassified clean half-closes
  (V-28: 192, 192 and 480 bad). Rejected.
- **A delay between the end-of-stream and acting on it.** A timing heuristic
  over a fact the kernel records exactly. Rejected
  (`.claude/rules/design.md`, fallbacks).
- **An in-band abort marker on the flow vsock.** Owners write nothing on a
  flow vsock after its request (ADR-0158), and payload does not cross
  userspace. Rejected.

## Consequences

- A clean half-close costs no control-session message; an abort reaches the
  far ends of both owners as a reset, whichever owner dies or resets.
- Each side runs one more BPF program, on every received vsock packet, and
  writes a ring record per SHUTDOWN or RST. A record lost to a full ring is a
  counted loss the owner answers by aborting the flow (ADR-0183, A7), never by
  a clean end.
- The host relies on the vhost-vsock release sweep for a guest crash; that is
  why no Overdrive process keeps a copy of a VMM's vhost-vsock descriptor
  (ADR-0146) and why a guest reset ends the VMM (ADR-0182).
- **A5 assumption.** The raw tracepoint is skipped only on recursion of the
  same program on one CPU; the skip increments the classifier's
  `recursion_misses` (`kernel/trace/bpf_trace.c:2080-2083 @ v7.0`). A
  SHUTDOWN or RST whose record is skipped leaves its end unclassified, so the
  owner takes it as an unannounced end (A4) and aborts the flow: a clean flow
  aborts only on a fault, a classifier skip included; a skip never strands a
  flow and never yields a clean end, and every such skip is counted.
- What held `bpf_prog_active` in V-31's skipped runs is not identified; the
  raw form removes the dependency on it. Every V-31 microVM run used the
  perf-attached build, so one microVM batch on the raw build is an open
  validation item in the feature delta's validation plan. It is not
  contract-shaping: a failure changes the program, not a pinned contract.
- The rules are covered by the feature delta's § *Formal protocol model*.
