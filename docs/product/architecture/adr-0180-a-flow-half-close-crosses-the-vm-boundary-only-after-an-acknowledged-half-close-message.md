# ADR-0180 — A flow's half-close crosses the VM boundary only after an acknowledged half-close message on the control session

## Status

**Accepted — decision HALFCLOSE-SIGNAL approved under the user's 2026-10-09
remediation directive** ("if design needs changing, then you remediate, and
then proceed to DISTILL"), answering the falsification of validation item
V-13 (feature delta P-42, `spike/findings-v13.md`). GH #295. Recorded in the
#295 feature delta, § *[REF] vsock Attachment Replacement DESIGN*
(§ *Spike remediation decisions, round 2 — 2026-10-09*). Depends on
ADR-0166 (the control session carries the messages) and ADR-0160 (stop
aborts with reset; half-close waits for the drain signal).

## Context

A paired TCP flow crosses the VM boundary on one vsock connection per flow
(V_g in the guest, V_h on the host). Each owner propagates a half-close: when
its outer TCP socket reads EOF it waits for the drain signal and then shuts
down its vsock for writing, and the other owner, seeing the vsock's
end-of-stream (`EPOLLRDHUP`), drains and shuts down its own outer socket.

When one owner dies, the kernel closes its sockets. The dying owner's outer
sockets carry `SO_LINGER{1,0}` and reset their far ends. Its vsock
connections, however, end with an ordinary end-of-stream on the surviving
side. V-13 measured, on native metal (7.0.0-29 host and guest, CH fork
`41a619d19`, 110 observers per cell):

- far ends of the dying owner's sockets observed a reset in every cell
  (110/110, kTLS included);
- far ends of the survivor's sockets observed EOF in most trials (98/110,
  103/110, 4/110, 4/110, 49/110 per cell): the survivor saw the flow vsock's
  `EPOLLRDHUP` before control-session loss in 415 of 415 deaths (host survivor
  p50 9.1 ms ahead, guest survivor p50 58 µs ahead), treated it as a clean
  half-close, drained in 0.1–0.2 ms and sent a FIN; the abort that
  control-session loss triggers arrived after the FIN, and a reset after a
  FIN reaches the sender as `EPIPE`, never as an error on a read that already
  returned 0;
- `EPOLLHUP` never accompanied the end-of-stream (79/79 events were
  `EPOLLRDHUP` alone), so poll flags cannot tell a crash from a clean
  half-close;
- an explicit half-close message on the control session sent before the
  vsock shutdown, with a bare end-of-stream held until the message arrives
  and control loss aborting, gave a reset at every observer (360/360), and
  clean closes stayed byte-exact (80/80).

A vsock end-of-stream therefore does not state why the peer ended the
stream. That fact is known only to the peer owner, and the control session
is the channel that carries owner-to-owner facts.

## Decision

- **A flow vsock's end-of-stream is never by itself a half-close.** An owner
  shuts down its outer socket for writing (sends a FIN toward the
  application or remote) only for a direction whose half-close the peer
  owner announced and this owner acknowledged.
- **The half-close handshake.** The owner whose outer socket reads EOF waits
  for the drain signal toward its vsock (ADR-0160), sends `HalfClose` for the
  flow on the control session, waits for the peer's `HalfCloseAck`, and only
  then shuts down its vsock for writing. The peer answers every `HalfClose`
  for a flow it holds with `HalfCloseAck` at once (it waits on nothing), and
  records the direction as announced.
- **An unannounced end is a lost peer.** An end-of-stream on a flow vsock for
  a direction the receiving owner has not acknowledged a `HalfClose` for is
  treated as the peer owner's loss: the flow is aborted with reset
  semantics (ADR-0160), at once — not when the control session's loss
  arrives later.
- **An early half-close waits for pairing.** An outer socket that reads EOF
  before the flow is paired is a half-close, recorded when observed and
  carried out through the handshake once the flow is paired; only an error
  or reset before pairing aborts the flow.
- **Pending only on the announcing side.** Between `HalfClose` and
  `HalfCloseAck` the announcing owner holds its vsock open; that wait ends by
  the acknowledgement, an `Abort`, control-session loss, a flow error, or
  the flow's teardown or quiescence. No timer ends it. The acknowledging side
  never waits.
- **Datagram associations** have no half-close and are unaffected.

## Alternatives considered

- **Treat a vsock end-of-stream as a half-close (as before).** Falsified by
  V-13 for every far end the surviving owner holds. Rejected.
- **Treat `EPOLLHUP` as an abort.** Never delivered with the end-of-stream
  (V-13 variant B). Rejected.
- **A delay between the end-of-stream and the outer shutdown**, hoping the
  control-session loss arrives first. A timing heuristic over a fact the
  peer knows exactly; the measured lead of the end-of-stream reached 19.9 ms.
  Rejected (`.claude/rules/design.md`, fallbacks).
- **`HalfClose` without acknowledgement, the receiver holding a bare
  end-of-stream pending until the message or control loss arrives** (V-13
  variant C, which passed). The message and the end-of-stream travel on two
  connections, so a clean half-close can show its end-of-stream first; the
  receiver must then wait, and a peer that shut the vsock without the
  message while its session lives holds the flow pending with nothing but a
  timer to end it (the variant used 10 s). The acknowledgement orders the two
  causally — the end-of-stream of a clean half-close cannot exist before the
  receiver holds the message — so the receiver decides at once and without a
  timer. Rejected in favour of the acknowledged handshake; its evidence
  carries over (the kernel facts are the same, the ordering is the
  owners').
- **Carry the half-close in band on the flow vsock.** Owners write nothing on
  a flow connection after its request (ADR-0158, H-1), and payload never
  crosses userspace. Rejected.

## Consequences

- A survivor resets the far end of every flow whose peer owner died, before
  any FIN — the guest application when the host owner dies, the remote
  destination and inbound client when the guest owner dies — independent of
  how late the control-session loss is observed.
- Linger on the dying owner's own sockets (ADR-0160) stays necessary: it
  resets the far ends that owner holds.
- A clean half-close costs one control-session round trip before the FIN.
- The control message set grows by two message types (ADR-0166); a
  `HalfClose` or `HalfCloseAck` for an unknown or closed flow is discarded
  under U-5.
- A peer owner that never acknowledges with its session live is an Overdrive
  defect (both owners ship in one image); its exposure is that one direction
  of one flow holds its FIN until the application closes, the flow is
  aborted, or the session ends — within that VM's quota.
- The handshake is part of the guest-flow owner's formal model (ADR-0168,
  `specs/quint/guest-flow-owner/`), whose round 9 adds the vsock end-of-stream
  reaching the survivor before control-session loss as an action.
- The kernel facts the rule rests on are measured (V-13); the production
  Tier-3 matrix of both owner deaths through `overdrive serve` and the
  clean-close regression are DISTILL scenarios.
