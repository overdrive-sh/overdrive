# ADR-0201 — A datagram association is one framed vsock stream, forwarded one datagram at a time

## Status

**Accepted — EVMOD-DGRAM, approved by the user 2026-10-17.** GH #295.
Follows from ADR-0197 (accepted 2026-10-16). It is the datagram path of
#295; ADR-0149 decides the frame format it carries. Its contract-shaping
validation item V-EV-DGRAM ran on Linux 7.0 and held (P-72,
`spike/findings-ev-dgram.md`; physical host `7.0.0-38`, VMs and guest
`7.0.0-29`). The frame both ways is the host-side protocol of every guest and
is guest-neutral: the host forwarder frames and unframes it the same way for
any guest kernel (a non-Linux guest's hop is
[GH #321](https://github.com/overdrive-sh/overdrive/issues/321)). Contract:
§ *[REF] vsock data path — the in-kernel event-driven forwarder (EVMOD)*,
"Datagram mode". The frame parser's placement (one pure C function, below) is
architect disposition FRAME-PARSE of 2026-10-18, requested by the user and
confirmed by adversarial review; it is not a user approval.

## Context

Under the sockmap redirect a datagram association needed: a SEQPACKET
connection from the guest, because a stream verdict cannot see record
boundaries; a guest strparser framing cell with `TCP_NODELAY` and a
reassembly cell, recreated at every association end because emptied cells
refilled (P-41); a host verdict that length-checks every SEQPACKET record
and strips its frame; an owner that sends empty datagrams itself, because a
zero-length skb can be neither passed nor redirected (P-50), which reorders
empties against their neighbours (D15-R4); a guest `lo` TC program that
strips the frame from host replies; and a validation item for an unbounded
queue toward a UDP target (V-3-DGRAM). A SEQPACKET send under credit
pressure emits a prefix and resends the whole record (P-3, P-53), which the
host length check had to absorb.

The forwarder of ADR-0197 reads its source with `recvmsg` and writes its
target with `sendmsg`. On a UDP socket a peek returns one datagram and,
with `MSG_TRUNC`, its full length; a UDP send of length 0 emits one empty
datagram; a UDP receive queue that is full drops the arriving datagram
(`net/ipv4/udp.c:1721-1731 @ v7.0`), which is UDP's own overload behaviour.

## Decision

A datagram association is one vsock STREAM connection (request kind
`DatagramStream`, on the flow stream port, default 1240; node configuration,
ADR-0218) carrying the existing length-prefixed frame
(ADR-0149) in both directions, and the forwarder joins it to the
association's UDP socket in datagram mode:

- **UDP to vsock:** the forwarder peeks one datagram with its length, sends
  the 8-byte frame header and the payload, resuming a partly sent frame where
  it stopped, and consumes the datagram only after its whole frame was
  accepted. A datagram above `MAX_GUEST_DATAGRAM_PAYLOAD` is consumed and
  counted, not sent.
- **vsock to UDP:** the forwarder peeks a frame header, waits until the whole
  frame is queued (the wait keys on the queued-byte count,
  `vsock_stream_has_data`, `net/vmw_vsock/af_vsock.c:1030-1037 @ v7.0`, so no
  payload peek repeats while a frame accumulates), sends the payload as one
  datagram (an empty one when the length is 0), and consumes the frame after
  the send. A frame with a bad magic or a length above the bound aborts the
  association (`FrameInvalid`).
- **The end of the peer's direction, with whole or partial frames
  queued.** The end's class is fixed first, by the vsock half's callback
  (ADR-0180). An abort-class end (`0x3` with a direction open, or an `RST`)
  aborts the pair `PeerAbort` in the callback, whatever is queued; every
  production end of a datagram vsock is of this class (the guest's slot
  release and the host's abort both shut the vsock both ways), so the run
  never meets an incomplete frame after a production end. A clean-class end
  (`0x2` only) is produced by no owner: the run then sends every whole frame
  still queued, in order, and the pair completes after the last one; if a
  frame is left incomplete (a partial header or a partial payload), the
  pair aborts `FrameInvalid` — the forwarder reads `peer_shutdown &
  SEND_SHUTDOWN` before the queued-byte count. This last branch has no
  production producer; it is the forwarder's one correct effect for that
  input, the alternatives being a pair that waits for ever for bytes that
  cannot come (reproduced with an independent peer, P-72 § 8) or a frame
  dropped while the association reports a clean end.
- **The vsock half's buffer holds the largest frame.** Datagram-mode attach
  refuses a vsock half whose `buffer_size` (`include/net/af_vsock.h:71 @
  v7.0`) is below `MAX_GUEST_DATAGRAM_FRAME` (59,008 B); the owners never
  lower `SO_VM_SOCKETS_BUFFER_SIZE` (default 256 KiB). A smaller buffer could
  never queue a large frame (reproduced with 32,768 B, P-72 § 8).
- **Slot sockets bind an explicit port, and the guest's release order is
  abort, discard, disconnect.** A port-0 bind loses its port at disconnect
  (`net/ipv4/udp.c:2209-2235`, `net/ipv4/af_inet.c:562-563 @ v7.0`), and the
  slot's address keys the capture programs' maps. Without the discard,
  1,018 queued datagrams of one association reached the next association's
  peer (P-72 § 7); with it, none.
- **No `ENOBUFS` handling and no `IP_RECVERR`.** A UDP send whose packet a
  qdisc drops returns success unless `IP_RECVERR` is set
  (`net/ipv4/udp.c:1195-1201 @ v7.0`; 1,086,869 qdisc drops, `err` 0, P-72
  § 8). The owners keep `IP_RECVERR` off, so a qdisc drop is UDP loss, not
  an association's end.
- The guest's UDP slot socket and the host's connected UDP socket are the
  pair's UDP halves; no SEQPACKET connection, strparser cell, reassembly
  cell or guest `lo` program exists.
- **One pure function interprets the frame.** On both sides the forwarder
  reads a frame header from the vsock half only through one pure C function
  in its own source file under `kmod/ovd_fwd/`. It takes a byte slice,
  allocates nothing, takes no lock, touches no socket, reads no byte past
  its input, checks the declared length against `MAX_GUEST_DATAGRAM_FRAME`
  and against the slice before using it, and returns a typed outcome: more
  bytes needed (with the total the frame needs, which is how a partly queued
  frame resumes), a whole frame of a given length, an oversize length, or an
  invalid frame. Nothing else in the module interprets bytes received from
  the peer, and TCP stream payload is never parsed. The same source file is
  compiled into a userspace Rust test crate, so the parser that ships is the
  parser that is tested, and it is checked differentially against the Rust
  codec of ADR-0149 and fuzzed (obligations in the feature delta, "Datagram
  mode", frame parser).

Measured (P-72): 0 to 59,000 B datagrams, empties included, delivered once
and in order both ways, connected and unconnected, over loopback vsock with
the module at both ends and over a real vhost device to an independent host
parser; 6,079 partial-send resumes under a 64 KiB credit window with 0
lost, duplicated or corrupt; oversize consumed and counted with no desync;
under a 10 s stall every datagram accounted for (sent = delivered + drops,
drops only at UDP receive queues) with kernel memory flat; ICMP refusal and
the D26 `EPERM` ended the association with `SocketError` (111, 1); the
`recvmsg4` rewrite restored the original reply source.

## Alternatives considered

- **Keep the sockmap redirect for datagrams only.** Two forwarding
  mechanisms, the whole framing-cell and verdict stack kept, and the
  unbounded queue of V-3-DGRAM still open with no valve. Rejected.
- **Keep SEQPACKET from the guest under the forwarder.** A non-blocking
  SEQPACKET send can emit a prefix and later resend the whole record (P-3),
  so the receiver would still need the length check and its drop rule.
  Rejected.
- **Move datagrams in the owners in userspace.** Copies every datagram
  through userspace; rejected with the userspace pump (ADR-0197).
- **Parse the frame inline in the datagram run.** The header checks would sit
  among socket calls and locking, could be exercised only through a real
  vsock at Tier 3, and could not be driven at arbitrary split points or
  checked for an out-of-bounds read under a sanitizer. Rejected for the one
  pure function.
- **Check the parser only at Tier 3, with the Rust codec as the oracle on the
  received datagrams.** Covers only the split points a real credit window
  happens to produce and cannot detect a read past the input. Kept as the
  composed check; rejected as the parser's only check.

## Consequences

- An empty datagram is delivered in order with its neighbours, as on an
  unforwarded socket; D15 has no residual difference for empty datagrams.
- A datagram association costs one vsock connection; there is no SEQPACKET
  request kind and no second flow port.
- When a target does not keep up, datagrams wait in the source UDP socket's
  receive queue and, once it is full, the kernel drops new ones — the
  overload behaviour of an unforwarded UDP socket. Nothing grows without
  bound.
- The UDP slot release has no cells to recreate: the guest owner aborts the
  pair and discards the slot socket's receive queue without reading its
  payload.
- The datagram mode is new kernel code; V-EV-DGRAM measured it on Linux 7.0
  with the host side over loopback vsock and the guest side over a real
  vhost device. The host side on a real vhost transport is exercised by the
  module's Tier-3 suite in DELIVER.
- The only module code that reads bytes from the peer is a pure function that
  also builds in userspace, so it gets the same differential proptest and
  coverage-guided fuzzing as Rust code, even though the module itself is
  outside cargo-mutants. The module source gains a second C file and a small
  header that must compile both in the kernel and in userspace.
