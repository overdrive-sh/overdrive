# ADR-0165 — The host verdict strips the frame from every guest datagram; the guest-flow owner sends empty datagrams itself

## Status

**Accepted — decisions UDP-EMPTY and D15-R4 approved by user 2026-10-11.**
GH #295. Recorded in the #295 feature delta, § *[REF] vsock Attachment
Replacement DESIGN* (§ *[REF] Wire contracts* for the frame and the
flow-event record, § *[REF] Driven port — host forwarder
`overdrive-dataplane::guest_flow`* for the verdict). The frame format and its
length check are ADR-0149; the guest-side removal of the frame from host→guest
datagrams (TCX on guest `lo`) is ADR-0150 and is not part of this decision.

## Context

ADR-0149 frames every datagram of a datagram association with an 8-byte
magic-and-length header. Guest→host datagrams arrive on the host as records of
the association's SEQPACKET (ADR-0148). Before a datagram leaves the host
toward its destination, the frame must be removed in the host kernel, and an
empty datagram (declared length 0) must reach the destination as a real
zero-length UDP datagram (D15).

The verdict cannot hand an empty datagram to the destination socket:

- Redirecting a zero-length skb to an egress socket makes `skb_send_sock`
  return 0 (the head loop at `net/core/skbuff.c:3314` is skipped and `:3336-3338` returns at once, `@ v7.0`); the psock backlog
  treats that as a hard error, reports `EPIPE` and clears `TX_ENABLED`
  (`net/core/skmsg.c:730-732 @ v7.0`), so every later datagram of the
  association is lost (V-29 item 1b: `sent_zero` 1, then 5 of 6 data
  datagrams lost).
- Passing a zero-length skb (`SK_PASS`) into a psock makes every later
  `recv`/`recvmsg` on that socket return `-EFAULT`
  (`net/core/skmsg.c:444-445`, `:488-491 @ v7.0`) until the socket leaves the
  map, and the data queued behind it is lost (V-29 item 3, identical on two
  runs).

Two further facts shape the verdict:

- A vsock skb above about 32 KiB is paged
  (`include/linux/virtio_vsock.h:84-92 @ v7.0`), and `__skb_send_sock` issues
  one `sendmsg` per head piece and per fragment
  (`net/core/skbuff.c:3311-3345 @ v7.0`). Redirected to a UDP socket without
  linearising, such a datagram leaves as two to four datagrams (V-29 item 4:
  30/30 split at ≥ 32,768 B; with `bpf_skb_pull_data`
  (`net/core/filter.c:1907-1918 @ v7.0`) 65/65 arrived byte-exact up to
  65,507 B).
- `udp_bpf` replaces `recvmsg`, `close` and `ioctl` of a sockmap-held UDP
  socket but not `sendmsg` (`net/ipv4/udp_bpf.c:128-134 @ v7.0`), so the
  owner can send on the association's connected host UDP socket while that
  socket is a redirect target.

V-29 (`spike/findings-v29-udp-empty.md`, 7.0.0-29 host and guest, real
microVM) measured the owner-sent empty datagram: every one of 3,000 (and
about 98,000 across 35 phases) left exactly once as `UDP, length 0`, with no
error and no zero-length send on the psock. It also measured that an empty
datagram sent by the owner can overtake data datagrams the verdict queued
before it: the verdict only queues the skb and schedules the backlog
(`net/core/skmsg.c:989-990 @ v7.0`); the backlog defers while the socket is
not writable, i.e. while its write allocation is at least half its send
buffer (`net/core/skmsg.c:649-651`, `include/net/sock.h:2631-2634 @ v7.0`);
the owner's send needs only write allocation below the send buffer
(`net/core/sock.c:2983 @ v7.0`). Unloaded, 31–70 % of empties overtook their
predecessor; throttled to 5 Mbit, 98.8 %, by up to 2,423 datagrams. An owner
gate on the flow's sent count removed every early case but left a late tail of
0.1–57 %; no owner-side mechanism gives exact order.

The only mechanism that gives exact order is to keep the empty datagram's
frame through the verdict and remove it on egress, so that the empty rides
the same psock queue as its neighbours. That needs a fragment-aware,
tuple-gated TC program on host `lo` and on every root-namespace interface,
an attach set fixed at boot, a boot refusal for stacked interfaces, a
fragment record map, a ban on UDP segmentation offload on owner sockets and
an image requirement on interface configuration. D15-R4 admits that an
empty datagram may be reordered relative to its neighbours, so exact order
is not required.

## Decision

- **The verdict strips every frame.** The host SEQPACKET verdict removes the
  8-byte frame from every guest→host datagram that passes ADR-0149's length
  check. There is no escape rule: every SEQPACKET record is a frame by
  construction, so a payload that itself parses as a frame is payload.
- **An empty datagram is sent by the owner.** A frame whose declared length
  is 0 is dropped (`SK_DROP`) and the verdict writes one notice — flow cell
  and cell epoch — to the owner's flow-event ring (layout in the feature
  delta). On the notice the guest-flow owner sends one zero-length datagram on
  the association's connected host UDP socket (`send(fd, NULL, 0)`). There is
  no ordering gate and no polling.
- **Stale notices are discarded.** A notice whose epoch is not the cell's
  current epoch is dropped without sending (the cell has been reused).
- **Verdict invariants, on every UDP and SEQPACKET path, host and guest:**
  1. a zero-length skb is never passed (`SK_PASS`) or redirected;
  2. `bpf_skb_pull_data` over the whole datagram runs before every redirect
     to a UDP or SEQPACKET target;
  3. the flow-event ring has a global drop counter, and each cell has two
     lost counts split by record class: `empty_lost` counts lost empty-datagram
     notices; `ring_lost` counts lost flow-end, drain and redirect-refused
     records (ADR-0183, A7). A failed ring write increments exactly one of
     them.
- **A lost empty notice is compensated, never aborted.** On a rise of a live
  association's `empty_lost` the owner sends the counted number of empty
  datagrams on that association; the association stays open. Only a rise of
  `ring_lost` aborts a flow (ADR-0183).
- **When the counts are read.** The kernel raises no event for a lost ring
  write, so the owner reads the counts at every flow-event dispatch and on
  the fixed one-second audit period of ADR-0124. A lost notice is therefore
  compensated within one audit period.
- **Lost counts belong to one flow.** A cell's lost counts are reset when the
  cell is initialised for a new flow (new epoch). Compensation applies only
  to the live association of the current epoch; removing an association reads
  nothing and sends nothing.
- **No egress program.** The host attaches no TC program to any interface for
  guest datagrams and keeps no per-tuple or per-fragment state for them.

## Alternatives considered

- **Keep the frame on empty and frame-shaped datagrams and remove it with a
  fragment-aware, tuple-gated egress TC program on every interface.** Exact
  order, at the cost of the per-interface mechanism listed in the Context.
  Rejected: D15-R4 admits the reordering it exists to prevent.
- **Owner gate on the flow's sent count before sending the empty.** Removes
  early overtaking but not late displacement (V-29: up to 57 % late at 100 µs
  pacing), needs a completion wake the forwarder does not have, and still
  cannot stop the successor's redirect running first. Rejected.
- **Redirect or pass the zero-length skb.** Disables the psock's TX or
  poisons the receiver with `-EFAULT` (Context). Rejected.
- **Drop empty datagrams, counted.** A D15 regression: an application that
  sends an empty datagram expects it delivered. Rejected.
- **An escape rule that keeps the frame on frame-shaped payloads.** Needed
  only when a later stage decides whether bytes are a frame; with every
  frame stripped in the verdict nothing downstream inspects payload.
  Rejected.

## Consequences

- An empty UDP datagram from a guest may arrive before or after neighbouring
  datagrams of the same association; the order of non-empty datagrams is
  unaffected (D15-R4, recorded beside D15-R1 to D15-R3 in ADR-0150's residual
  D15 set).
- Non-empty datagrams leave the host independent of routing, interface set,
  MTU and segmentation offload; no host interface carries a guest-datagram
  program, and the image carries no interface requirement for guest
  datagrams.
- Each empty datagram costs one ring record and one `send` system call in the
  owner. V-29 sized the record at 40 bytes; a full ring loses notices, which
  `empty_lost` turns into compensating sends (V-29 item 2: 1,000 empties
  into a 4,096-byte ring — 102 notices plus 898 counted, 1,000 delivered with
  compensation). Compensation restores the number of empty datagrams, not
  their positions, which D15-R4 already admits.
- Not measured by V-29: IPv6, `UDP_SEGMENT`, concurrent associations sending
  empties, and the epoch-mismatch notice path. The epoch and lost-count rules
  are covered by the feature delta's § *Formal protocol model*.
