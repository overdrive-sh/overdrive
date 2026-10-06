# ADR-0148 — Datagram flows use STREAM toward the guest and SEQPACKET from the guest on one vsock device; the guest reassembles, the host never sends on SEQPACKET

## Status

**Proposed — decision D4 approved by user 2026-10-05; pending independent
DESIGN review.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*.

## Context

Stock vsock SEQPACKET has a partial-send defect: a nonblocking `send()` can
emit part of a message, return `EAGAIN`, and on retry emit the whole message
again. It was reproduced without BPF on stock 7.0.0-29
(`spike/kernel-vs-aya-benchmark.md`, increment-l). The sockmap backlog retries
on `EAGAIN`, so a SEQPACKET redirect target duplicates data. Fixing it needs a
kernel patch, which the constraints forbid.

The corrected arrangement — STREAM toward the guest, SEQPACKET from the guest,
both on one CID — passed six AB/BA/AB cohorts and 384 open-loop windows at
populations up to 16,384, with no duplicate emit.

The guest-capture spike (`spike/guest-vsock-capture-findings.md`) added a
guest-side fact: the guest's virtio-vsock RX buffers are 4 KiB, vhost splits
host data to fit, and `read_skb` hands the guest verdict one skb at a time. A
59,000-byte host→guest frame arrived as about 15 guest skbs. A TCP strparser
cell in the guest reassembled frames correctly at 4,097 and 59,000 bytes.
SEQPACKET from the guest delivered 59,008-byte frames intact after
`pull_data` linearisation on the host.

## Decision

- A datagram association uses two vsock connections on the guest's CID:
  - a **STREAM**, used on the host only as the redirect *target* for
    host→guest frames;
  - a **SEQPACKET**, used on the host only as the redirect *source* for
    guest→host frames.
- **The host never writes on a SEQPACKET socket**, neither payload nor
  control. Every flow control message travels on the control session
  (ADR-0158).
- **The guest reassembles** host→guest frames in the kernel before delivery:
  it does not assume one guest skb is one frame. Each reassembled frame is
  delivered as exactly one datagram.
- **The guest sends** each datagram as exactly one SEQPACKET record.
- A TCP flow uses one STREAM in both directions.

## Alternatives considered

- **SEQPACKET both ways.** Hits the stock duplicate-emit defect. Rejected.
- **STREAM both ways.** The host would need a strparser to recover guest→host
  boundaries, which was never measured on the host, and SEQPACKET already
  gives one record per message. Rejected for this feature.
- **Assume one guest skb per host frame.** Falsified at 4 KiB. Rejected.
- **Patch the kernel's SEQPACKET partial-send handling.** Forbidden.

## Consequences

- An association costs two vsock connections per side, plus guest framing and
  reassembly cells (ADR-0147).
- The partial-send defect and the 4 KiB split are observed on stock 7.0.0-29.
  The arrangement does not depend on either being present: the host never
  writes on SEQPACKET, and guest reassembly handles any split size.
