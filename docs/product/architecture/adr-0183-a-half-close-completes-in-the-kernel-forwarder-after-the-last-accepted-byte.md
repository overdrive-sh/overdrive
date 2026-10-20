# ADR-0183 — A half-close completes in the kernel forwarder after the last accepted byte, with no deadline

## Status

**Accepted — decision R2-DRAIN approved by the user 2026-10-11, carried out
by the event-driven in-kernel forwarder of ADR-0197 (EVMOD-ADOPT, approved
2026-10-16).** GH #295. Not yet implemented. Recorded in the #295 feature
delta, § *[REF] vsock data path — the in-kernel event-driven forwarder
(EVMOD)*. How an end is classified as a clean half-close is ADR-0180; stop
and abort semantics are ADR-0160; the per-allocation flow quota is ADR-0208
and its release point ADR-0209. The pair end record that tells the owner a
flow is over is ADR-0202.

## Context

When one direction of a flow ends cleanly — a FIN on its TCP source, or a
clean half-close of its vsock source (ADR-0180) — the target of that
direction must be shut down for writing, and not before the target has
accepted every byte of the direction. Drain probe T1
(`spike/findings-drain-t1-t6.md`) measured how wrong the first wake is as
that moment: at the first `EPOLLRDHUP` the target had not yet sent every
byte in 8–97 % of TCP-source directions, and half-closing then passed only
345, 216 and 11 of 10,000 1 MiB flows at 0, 300 and 3,000 ms reader pauses.

Under the forwarder of ADR-0197 a byte leaves the source only after the
target accepted it, and the forwarder learns the source's end only by
reading it: a TCP source returns end-of-stream once its FIN is received and
every byte before it consumed (`net/ipv4/tcp_input.c:4953-4954`,
`net/ipv4/tcp.c:2794-2795 @ v7.0`), and a vsock source once its peer's
`SEND_SHUTDOWN` is recorded and its receive queue is empty
(`net/vmw_vsock/af_vsock.c:2326-2328 @ v7.0`). When the forwarder reads that
end, every byte of the direction has already been accepted by the target.
End-of-stream alone does not prove the end was clean: `tcp_recvmsg` tests
`SOCK_DONE` before `sk_err` (`net/ipv4/tcp.c:2794`, `:2797 @ v7.0`), so a TCP
peer that sends its FIN and then a reset (a server closing with unread data)
still reads as end-of-stream; the reset is seen through `sk_error_report`
(`net/ipv4/tcp_input.c:4890-4900 @ v7.0`), and the forwarder aborts the
pair on it while any direction is open (ADR-0180).
A vsock `shutdown(SHUT_WR)` never overtakes accepted bytes (drain probe T3:
0 early EOF in 42,000 trials, credit exhausted in 4,201).

A forwarder that moves bytes into a queue toward a target that does not
read can grow that queue without bound: guest-slot spike V-3 measured it for
a BPF sockmap redirect (guest OOM in about 1 s). The evmod spike measured
the forwarder's queue flat across 3 s and 10 s stalls, the sender blocked for
the whole stall, in both directions and on host and guest
(`spike/findings-evmod.md`, `spike/findings-evmod-followup.md`).

## Decision

- **The forwarder propagates a clean end.** When it reads a clean end of a
  direction's source it calls `shutdown(SHUT_WR)` on the direction's target.
  Because it consumes only what the target accepted, that shutdown follows
  the direction's last byte; no goal, counter, record or owner step decides
  it.
- **No deadline.** A half-closed direction waits until its target accepts
  its last byte, or the flow is aborted. A slow reader is backpressure: the
  bytes a stalled flow holds are bounded by its sockets' buffers and do not
  grow with time (ADR-0197).
- **The flow ends once both directions have ended cleanly**, or at the
  first abort. The pair's owner learns which from the pair's end record and
  only then releases the flow (ADR-0202).
- **The per-allocation flow quota counts half-closed flows** (ADR-0208,
  4,096): a flow holds its quota place until its pair has ended (ADR-0209),
  so a guest
  that withholds credit or an application that stops reading holds at most
  its allocation's quota of node flow capacity.
- **Control-session loss** aborts every flow of the session (DG-5-F,
  ADR-0166); no half-close depends on the session.

## Alternatives considered

- **A drain cell per direction, completed by kernel counters and a ring
  record** (two `fexit(skb_send_sock)` programs, an `fentry(sock_poll)`
  backstop, recursion-miss reads at every dispatch and audit tick). Needed
  only by a forwarder that reports nothing when its queue empties, such as a
  sockmap redirect, which the user rejected for #295 (ADR-0197). Rejected.
- **Half-close at the first `EPOLLRDHUP`.** Truncates (T1). Rejected.
- **A drain deadline that aborts.** Turns a slow reader into an abort and
  bounds nothing the quota does not already bound. Rejected.

## Consequences

- No drain state, program or counter exists on either side; the owner has
  no drain step and no counter to read on a period.
- A half-close completes as soon as the target accepted the last byte. A
  target application that never reads keeps the direction open with its
  bytes in the target socket's buffers (2.6–8 MiB measured with autotuned
  buffers), until it reads, closes or resets, or the flow is aborted.
- A peer that only shuts down its read side and keeps its socket open sends
  nothing (`net/ipv4/tcp.c:3113 @ v7.0`) and looks like a stalled reader;
  the flow waits as it would for a direct TCP connection to that peer.
