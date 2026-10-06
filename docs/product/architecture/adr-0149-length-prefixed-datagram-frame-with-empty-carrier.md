# ADR-0149 — Every forwarded datagram carries an 8-byte magic-and-length frame inside the transport; empty datagrams travel as a nonempty carrier; the payload bound is 59,000 bytes

## Status

**Proposed — decision D5 approved by user 2026-10-05; pending independent
DESIGN review.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*, which holds the byte
layout and the forwarder contract.

Where the host removes the frame from guest→host datagrams is a separate
decision, D5a (ADR-0165). Datagrams above 59,000 bytes are deferred to
**#309** (user ruling D15, 2026-10-05).

## Context

A STREAM leg has no datagram boundaries (ADR-0148), and a zero-length skb
carries nothing to redirect. The correction framed every datagram with an
8-byte prefix and removed it in the kernel (`spike/aya-zero-udp-findings.md`):
the SK_SKB verdict adds room, writes the header and linearises; a TCX program
on `lo` egress removes the 8 bytes and fixes the IPv4 and UDP lengths. Sizes 0,
1, 64, 1,431 and 59,000 bytes were byte-exact, including a payload that is
itself a valid header. Malformed, short and length-mismatched frames, and route
or tuple misses, failed closed with counters.

The guest-capture spike used the same frame in the guest
(`spike/guest-vsock-capture-findings.md`):

- guest→host: the guest UDP intake's verdict frames each application datagram
  and a strparser cell feeds exactly one frame per verdict to the SEQPACKET;
- host→guest: a guest strparser cell reassembles frames split at 4 KiB, and a
  TCX program on guest `lo` strips the frame before the application receives
  it.

## Decision

- Every datagram on a datagram association, in both directions, is framed as a
  4-byte magic followed by a big-endian `u32` logical payload length. A
  zero-length datagram is the 8-byte header alone.
- The logical payload bound is **59,000 bytes**.
  - A larger datagram from either side is dropped whole where it is first
    framed, with a typed counter. It is never truncated or split.
  - A frame that declares more than 59,000 bytes, or a length other than its
    reassembled size minus 8, is dropped whole with a typed counter.
- The frame is added and removed only by kernel programs. It never reaches a
  guest application:
  - host→guest: framed by the host verdict; removed in the guest kernel before
    delivery to the application;
  - guest→host: framed in the guest kernel; removed in the host kernel before
    the datagram reaches its destination, at the points ADR-0165 sets.
- Host-side datagrams are IPv4 with UDP checksum 0. IPv6 is #308.
- The magic, the header length and the bound are one SSOT shared by the host
  programs, the guest programs, the host control plane and `overdrive-init`.

## Alternatives considered

- **Carry the peer address in the frame.** Needed only to multiplex peers
  behind one association (inbound UDP service, #310). Rejected for this
  feature.
- **SEQPACKET record boundaries alone, no frame.** Empty datagrams cannot be
  represented, and STREAM has no boundaries. Rejected.
- **The full 65,507-byte UDP maximum.** Not measured. Deferred to #309.

## Consequences

- A workload that sends a datagram over 59,000 bytes sees it dropped and
  counted. Operator-visible counters name the bound.
- The checksum-0 delivery is valid for IPv4 only.
- The spike magic `ZUD1` is replaced by the production magic pinned in the
  feature delta; validations re-run against the production bytes.
- Whether a frame can reach a remote peer depends on the removal points
  (ADR-0165).
