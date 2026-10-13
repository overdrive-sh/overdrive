# ADR-0148 — Datagram flows use STREAM toward the guest and SEQPACKET from the guest on one vsock device; the guest reassembles, the host never sends on SEQPACKET

## Status

**Accepted — decision D4 approved by user 2026-10-05; keeping SEQPACKET from
the guest with its sender, buffer and linearisation rules (UDP-SEQ) approved
by user 2026-10-11.** GH #295. Recorded in the #295 feature delta, § *[REF]
vsock Attachment Replacement DESIGN* (§ *[REF] Wire contracts*, § *[REF]
Guest adaptation contract*).

## Context

A nonblocking vsock SEQPACKET `send()` under credit pressure emits a prefix of
the message without end-of-record and returns `-EAGAIN`; the caller's retry
sends the whole message again (`net/vmw_vsock/af_vsock.c:2206-2272`, `:2289-2296 @ v7.0`;
first reproduced without BPF on stock 7.0.0-29,
`spike/kernel-vs-aya-benchmark.md`, increment-l). The sockmap backlog retries
on `-EAGAIN` with `MSG_DONTWAIT` fixed (`net/core/skbuff.c:3322 @ v7.0`), so a
SEQPACKET redirect *target* emits such prefixes.

V-32 (`spike/findings-v32-udp-seqpacket.md`, 7.0.0-29 host and guest, real
microVM, 3,520,000 datagrams) measured the guest→host SEQPACKET leg as
designed:

- 406,799 prefix packets were emitted under forced credit pressure; each
  reached the host verdict as its own short skb, because the host verdict is
  fed one packet at a time (`net/vmw_vsock/virtio_transport_common.c:1751-1773
  @ v7.0`). ADR-0149's length check dropped every one; 0 datagrams were
  duplicated or lost.
- Without the length check the same regime delivered 6 of 20,000 datagrams.
- `virtio_transport_seqpacket_enqueue` refuses a message longer than
  `min(peer_buf_alloc, buf_alloc)` with `-EMSGSIZE`
  (`net/vmw_vsock/virtio_transport_common.c:825-846 @ v7.0`). With a 32 KiB
  host buffer and 59,008-byte frames the first send failed that way, the
  psock stopped transmitting and 0 of 300 datagrams arrived.
- A paged skb redirected without `bpf_skb_pull_data` leaves as several
  records (one `sendmsg` per head piece and fragment, `net/core/skbuff.c:3311-3345 @ v7.0`); without it 2.4–100 % of
  datagrams were lost.
- A STREAM leg both ways with a host strparser also delivered, but only with
  `TCP_NODELAY` on a second strparser cell and linearisation before it, and
  it repeats the parser flaw below on that second cell.

The guest-capture spike (`spike/guest-vsock-capture-findings.md`) established
the host→guest side: the guest's virtio-vsock RX buffers are 4 KiB, vhost
splits host data to fit, and `read_skb` hands the guest verdict one skb at a
time. A 59,000-byte host→guest frame arrived as about 15 guest skbs; a guest
TCP strparser cell reassembled frames correctly at 4,097 and 59,000 bytes.

## Decision

- A datagram association uses two vsock connections on the guest's CID:
  - a **STREAM**, used on the host only as the redirect *target* for
    host→guest frames;
  - a **SEQPACKET**, used on the host only as the redirect *source* for
    guest→host frames.
- **The host never writes on a SEQPACKET socket**, neither payload nor
  control. This is a host rule. Every flow control message travels on the
  control session (ADR-0158, ADR-0166).
- **The guest sender is the strparser cell.** In the guest, each datagram
  reaches the SEQPACKET from the association's strparser cell, whose verdict
  linearises the whole frame (`bpf_skb_pull_data`) and redirects it. The
  record may be preceded by a prefix emitted under credit pressure, which the
  host's length check (ADR-0149) drops and counts.
- **Buffer invariant.** On both ends of every SEQPACKET leg,
  `min(peer_buf_alloc, buf_alloc)` is at least the largest frame, 59,008
  bytes (the 59,000-byte bound plus the 8-byte header, ADR-0149).
- **The guest reassembles** host→guest frames in the kernel before delivery:
  it does not assume one guest skb is one frame. Each reassembled frame is
  delivered as exactly one datagram.
- A TCP flow uses one STREAM in both directions.

## Alternatives considered

- **SEQPACKET both ways.** The host would write on a SEQPACKET target and emit
  prefixes the guest would have to filter as well. Rejected.
- **STREAM both ways, with a host strparser recovering guest→host
  boundaries.** Works only with a second strparser cell carrying the same
  parser flaw, linearisation before it and `TCP_NODELAY` on its sender, and
  fixes no duplicate, merge or loss the SEQPACKET leg shows on the sockmap
  host (V-32). Rejected.
- **Assume one guest skb per host frame.** Falsified at 4 KiB. Rejected.
- **Patch the kernel's SEQPACKET partial-send handling.** Forbidden by the
  appliance kernel constraint. Rejected.

## Consequences

- An association costs two vsock connections per side, plus guest framing and
  reassembly cells (ADR-0147).
- Credit pressure costs bandwidth, not datagrams: at a 64 KiB host buffer
  almost every 59,000-byte datagram was preceded by a dropped prefix and the
  run took twice as long as at 256 KiB, where none were. The host buffer size
  is a tuning value in the feature delta, bounded below by the buffer
  invariant.
- A reader that consumes SEQPACKET with `recv()` concatenates records up to
  end-of-record (`net/vmw_vsock/virtio_transport_common.c:723-788 @ v7.0`) and
  would see merged records; the host never reads the leg that way.
