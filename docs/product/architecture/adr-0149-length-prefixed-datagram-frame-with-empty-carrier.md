# ADR-0149 — Every forwarded datagram carries an 8-byte magic-and-length frame inside the transport; empty datagrams travel as a nonempty carrier; the payload bound is 59,000 bytes

## Status

**Accepted.** Decision D5 approved by the user 2026-10-05; the frame carried
on one vsock STREAM each way and framed and unframed by the in-kernel
forwarder (ADR-0201, EVMOD-DGRAM) approved 2026-10-17. GH #295. Recorded in
the #295 feature delta, § *[REF] Wire contracts* (byte layout) and § *[REF]
vsock data path — the in-kernel event-driven forwarder (EVMOD)*, "Datagram
mode" (the forwarder's handling). Datagrams above 59,000 bytes are **#309**
(user ruling D15, 2026-10-05). Not yet implemented.

## Context

A vsock STREAM has no datagram boundaries, and a zero-length datagram must
still be carried as something. Framing every datagram with an 8-byte prefix
was byte-exact at sizes 0, 1, 64, 1,431 and 59,000 bytes, including a payload
that is itself a valid header (`spike/aya-zero-udp-findings.md`).

V-EV-DGRAM (`spike/findings-ev-dgram.md`, P-72, Linux 7.0) measured the frame
on one vsock STREAM each way, framed and unframed by the forwarder: 0 to
59,000 B datagrams, empties included, delivered once and in order both ways;
6,079 partial-send resumes under a 64 KiB credit window with 0 lost,
duplicated or corrupt; oversize consumed and counted with no desync.

## Decision

- Every datagram on a datagram association, in both directions, is framed as a
  4-byte magic followed by a big-endian `u32` logical payload length. A
  zero-length datagram is the 8-byte header alone.
- The logical payload bound is **59,000 bytes**. A larger datagram from either
  side is consumed whole where it is first framed, with a typed counter. It is
  never truncated or split.
- **A frame is valid only when its magic matches and its declared length is
  at most 59,000.** A frame that fails either check aborts its association
  (`FrameInvalid`); a frame left incomplete at a clean-class end of the
  peer's direction aborts it likewise (ADR-0201).
- The frame is added and removed only by the forwarder. It never reaches an
  application: the forwarder frames each datagram from its UDP half into the
  vsock half and unframes each frame from the vsock half into one datagram on
  its UDP half, on both sides.
- Host-side datagrams are IPv4. IPv6 is #308.
- The magic, the header length and the bound are one SSOT shared by the
  module's UAPI header, the host control plane and `overdrive-init`.
- The frame is part of the guest-neutral host-side protocol (ADR-0207): the
  host frames and unframes it the same way for every guest.

## Alternatives considered

- **Carry the peer address in the frame.** Needed only to multiplex peers
  behind one association (inbound UDP service, #310). Rejected for this
  feature.
- **A SEQPACKET connection from the guest, using its record boundaries.**
  Empty datagrams cannot be represented without a frame, and a non-blocking
  SEQPACKET send under credit pressure emits a prefix and later resends the
  whole record (P-3, P-53), so the receiver would still need a length check
  and a drop rule. Rejected (ADR-0201).
- **A sequence number or checksum in the header.** On one STREAM framed and
  unframed by one forwarder per side nothing splits or merges frames; a
  sequence number guards nothing reachable. Rejected.
- **The full 65,507-byte UDP maximum.** Not measured. Deferred to #309.

## Consequences

- A workload that sends a datagram over 59,000 bytes sees it dropped and
  counted (`dgram_oversize`). Operator-visible counters name the bound.
- An invalid frame is a defect of the side that framed it and ends the
  association, never a silent drop.
- The spike magic `ZUD1` is replaced by the production magic pinned in the
  feature delta; validations run against the production bytes, with
  variable-size datagrams.
