# ADR-0149 — Every forwarded datagram carries an 8-byte magic-and-length frame inside the transport; empty datagrams travel as a nonempty carrier; the payload bound is 59,000 bytes

## Status

**Accepted — decision D5 approved by user 2026-10-05; the normative length
check on every SEQPACKET record and its counters (UDP-SEQ) approved by user
2026-10-11.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN* (§ *[REF] Wire contracts*), which holds the
byte layout, the counters and the forwarder contract.

Where the host removes the frame from guest→host datagrams, and how an empty
datagram reaches its destination, is ADR-0165. Datagrams above 59,000 bytes
are **#309** (user ruling D15, 2026-10-05).

## Context

A STREAM leg has no datagram boundaries (ADR-0148), and a zero-length skb
carries nothing to redirect. Framing every datagram with an 8-byte prefix
added and removed in the kernel was byte-exact at sizes 0, 1, 64, 1,431 and
59,000 bytes, including a payload that is itself a valid header
(`spike/aya-zero-udp-findings.md`). Malformed, short and length-mismatched
frames failed closed with counters.

The guest-capture spike (`spike/guest-vsock-capture-findings.md`) used the
same frame in the guest: guest→host, the guest UDP intake's verdict frames
each application datagram and a strparser cell feeds one frame per verdict to
the SEQPACKET; host→guest, a guest strparser cell reassembles frames split at
4 KiB and a TCX program on guest `lo` strips the frame before the application
receives it.

V-32 (`spike/findings-v32-udp-seqpacket.md`, 3,520,000 datagrams on 7.0.0-29)
showed that the host's length check is what keeps the guest→host SEQPACKET
leg exact: every credit-pressure prefix (406,799) arrived as its own short
record and was dropped by the check, and every merged or mis-cut record the
guest produced was dropped by it. With the check off, 19,994 of 20,000
datagrams were lost or corrupted in the 64 KiB credit regime.

## Decision

- Every datagram on a datagram association, in both directions, is framed as a
  4-byte magic followed by a big-endian `u32` logical payload length. A
  zero-length datagram is the 8-byte header alone.
- The logical payload bound is **59,000 bytes**. A larger datagram from either
  side is dropped whole where it is first framed, with a typed counter. It is
  never truncated or split.
- **The host length check is normative on every SEQPACKET record.** A record is
  a frame only when its magic matches and its length minus 8 equals the
  declared length, which is at most 59,000. Every other record is dropped
  whole and counted by class:
  - `short` (shorter than declared): the expected prefix a guest send emits
    under credit pressure (ADR-0148); counted, not alerted;
  - `long` (longer than declared), `lt8` (shorter than a header) and
    `badmagic`: counted and alerted, as is the guest strparser cell's
    `strp_bad_frame`.
- The frame is added and removed only by kernel programs. It never reaches a
  guest application:
  - host→guest: framed by the host verdict; removed in the guest kernel before
    delivery to the application;
  - guest→host: framed in the guest kernel; removed by the host verdict
    (ADR-0165).
- Host-side datagrams are IPv4 with UDP checksum 0. IPv6 is #308.
- The magic, the header length and the bound are one SSOT shared by the host
  programs, the guest programs, the host control plane and `overdrive-init`.

## Alternatives considered

- **Carry the peer address in the frame.** Needed only to multiplex peers
  behind one association (inbound UDP service, #310). Rejected for this
  feature.
- **SEQPACKET record boundaries alone, no frame.** Empty datagrams cannot be
  represented, STREAM has no boundaries, and nothing would reject a prefix or
  merged record. Rejected.
- **A sequence number or checksum in the header.** Would also catch the
  residual below, which was never observed naturally. Rejected; the residual
  is accepted.
- **The full 65,507-byte UDP maximum.** Not measured. Deferred to #309.

## Consequences

- A workload that sends a datagram over 59,000 bytes sees it dropped and
  counted. Operator-visible counters name the bound.
- The `short` count is the credit-pressure signal; `long`, `lt8`, `badmagic`
  and `strp_bad_frame` indicate a defect and are alerted.
- **Residual:** a guest send split into two skbs without an `-EAGAIN` between
  them — credit returning between iterations of the vsock send loop
  (`net/vmw_vsock/af_vsock.c:2206-2272 @ v7.0`) — loses that datagram, and if
  the bytes at the split happen to form a valid frame, delivers a datagram the
  application never sent into the sender's own association. V-32 produced it
  only with synthetic records (0 natural occurrences in 3,520,000 datagrams
  with 406,799 forced prefixes). Accepted, not mitigated; a sequence number or
  checksum in this header is the mitigation if it ever matters.
- The checksum-0 delivery is valid for IPv4 only.
- The spike magic `ZUD1` is replaced by the production magic pinned in the
  feature delta; validations re-run against the production bytes, with
  variable-size datagrams (fixed sizes pass the guest cell by accident,
  ADR-0150).
