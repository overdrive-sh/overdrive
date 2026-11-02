# ADR-0246 — Splice out of a kernel-TLS socket only into an empty private pipe of at least 64 KiB

## Status

Proposed — set by user ruling G1 (2026-11-02); pending independent review.
User rulings 2026-11-02: the invariant stays until the kernel 7.3 upgrade
[GH #339](https://github.com/overdrive-sh/overdrive/issues/339) lands (Q-1, R-17);
holding the pipe until connection close is not adopted (P-16, R-16).

## Context

`tls_sw_splice_read` at v7.0 computes `chunk = min(rxm->full_len, len)` and
splices `copied = skb_splice_bits(..., chunk, ...)` (`net/tls/tls_sw.c:2316-2317 @ v7.0`).
Two branches follow:

- `chunk < full_len`: the record is advanced by `len`, not by `copied`
  (`net/tls/tls_sw.c:2321-2325 @ v7.0`); octets the pipe did not map are skipped.
- `chunk == full_len` but `copied < chunk`: the record falls through to
  `consume_skb` (`net/tls/tls_sw.c:2327 @ v7.0`) and the unmapped tail is dropped.

Either way the loss is silent: `splice` returns success and there is no error on
either side. The cause of the second branch is INFERENCE, not a run: the clear-text
skb is built by `tls_alloc_clrtxt_skb` with a non-zero page order
(`net/tls/tls.h:44`, `net/tls/tls_sw.c:1476-1496 @ v7.0`), so one record can span
non-contiguous fragments, and a pipe with one free slot maps only the first.

Bounds that make 64 KiB sufficient (each read in the v7.0 source): one record
carries at most `TLS_MAX_PAYLOAD_SIZE` = 16 KiB of plaintext
(`include/net/tls.h:55`); `skb_splice_bits` maps at most `MAX_SKB_FRAGS` spans per
call (`net/core/skbuff.c:3258-3263`) and merges only adjacent spans of one page
(`spd_can_coalesce`, `net/core/skbuff.c:3119-3127`); `splice_to_pipe` stops when the
pipe is full and reports a short count (`fs/splice.c:215`). An empty pipe of at
least 64 KiB holds 16 slots of one page each, so one record's span fits whole.
The pipe size is set and read back because the kernel default is not reliable:
`fs/pipe.c:809-811 @ v7.0` shrinks it to `PIPE_MIN_DEF_BUFFERS` (2) for an
unprivileged user over the soft limit.

The v7.3 source advances by `copied` (`net/tls/tls_sw.c:2051-2060 @ v7.3`); this is
informational provenance, not the design basis. Spike
[findings (a)](../../feature/public-ingress-gateway/spike/findings-ktls-splice-relay.md)
observed the loss on the pinned kernel line (7.0.0-38) and none with an empty pipe
of at least 64 KiB.

## Decision

Every gateway relay splice out of a kernel-TLS socket reads into a pipe that is
private to that receive half, whose capacity has been set and read back as at
least 64 KiB, and that is empty at the moment of the call, with a length no
larger than the pipe's capacity and no larger than the octets the message still
needs. The relay splices out of the pipe until it is empty before the next
splice into it. A pipe that cannot be set to 64 KiB, or a call that would violate
any other clause, fails the relay with a typed error before the splice; no octet
is moved. This holds until the appliance kernel carries the v7.3 fix, which
arrives with the kernel 7.3 upgrade tracked in
[GH #339](https://github.com/overdrive-sh/overdrive/issues/339); the invariant is
kept until that issue lands.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Rely on the kernel's default pipe and a short-count check after each splice | The loss is silent: `splice` reports success, so there is nothing to check afterwards. |
| Require an appliance kernel with the `copied` fix | The pinned kernel line is 7.0 (ADR-0068); advancing it is an image decision tracked by [GH #339](https://github.com/overdrive-sh/overdrive/issues/339), not a gateway decision. The invariant costs one pipe per receive half and stays correct after the kernel is advanced. |
| Hold each read half's pipe until the connection closes | Saves pipe create/close churn per request, at 256 descriptors and 8 MiB per 128 idle connections that each relayed a body. Rejected by ruling P-16. |
| Copy through userspace (`recv`/`send`) | Same CPU, but every body octet passes through gateway memory; G1 moved the body path into the kernel. |

## Consequences

- One pipe (two descriptors, 64 KiB of kernel buffer) per receive half that
  relays at least one body; created lazily by the half's first relay call and
  released when the half starts reading its next message head or is dropped, so
  a connection waiting for its next request holds no pipe. No explicit
  per-exchange release operation exists.
- The invariant rests on the source reading above and one kernel build, not a proof.
- Backpressure comes from the destination socket: the splice into the destination
  returns a short count or `EAGAIN` when its send buffer is full
  (`fs/splice.c:874-888 @ v7.0`; non-blocking only through the socket's own
  `O_NONBLOCK`, not `SPLICE_F_NONBLOCK`), and the relay does not splice in again
  until the pipe is empty.

## Links

- [Kernel relay contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)
- [ADR-0239](adr-0239-stream-public-proxy-bodies-with-backpressure.md), [ADR-0244](adr-0244-gateway-legs-use-kernel-tls-after-a-userspace-rustls-handshake.md)
- [Spike findings](../../feature/public-ingress-gateway/spike/findings-ktls-splice-relay.md)
