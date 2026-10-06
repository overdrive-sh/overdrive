# ADR-0165 — The host removes the guest datagram frame in its verdict for every non-empty datagram, and a fragment-aware, tuple-gated egress program on every host interface removes the frames the verdict keeps

## Status

**Proposed — decision D5a (= hybrid) approved by user 2026-10-06; its
attachment-set detail approved by user 2026-10-06; pending independent DESIGN
review.** GH #295. Split out of ADR-0149 (frame format, D5) so that each ADR
records one decision. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*, which holds the
forwarder contract.

## Context

ADR-0149 frames every guest datagram inside the transport. Guest→host
datagrams leave the host toward host-local and off-host destinations, so the
frame must be removed somewhere in the host kernel.

The off-host spike (`spike/v11-vip-v14-findings.md`; qualified metal;
destinations behind veth at MTU 1,500 and through the physical NIC) compared
removal points:

- **Tuple-gated unframe on egress TC only.** Works up to the egress MTU. Above
  it the datagram is lost whole: IP fragmentation runs before TC egress, the
  first fragment fails the program's length check and is dropped, and
  `ip_do_fragment()` then stops sending the remaining fragments. A NIC capture
  of a 4,096-byte datagram showed no packet at all.
- **Fragment-aware egress TC only.** Works for every size (first fragment
  shortened by 8 bytes; later fragments' offsets moved back by one 8-byte
  unit; wire capture confirmed). If an egress interface lacks the program,
  every datagram through it reaches the peer 8 bytes too long.
- **Removal in the host verdict only.** Works for every non-empty size with no
  TC. An empty datagram cannot be redirected: a zero-length redirect makes the
  psock disable TX, after which every later datagram of the association is
  lost. Empties must therefore be dropped and counted.
- **Hybrid.** The verdict removes the frame from every non-empty datagram and
  keeps it only for an empty datagram or a payload that itself parses as a
  frame; a fragment-aware TC on each egress interface removes a valid frame on
  a registered tuple and passes everything else. All of 0, 1, 1,431, 4,096,
  4,097 and 59,000 bytes passed, connected and unconnected; frame-shaped
  payloads arrived byte-exact; a non-owner frame-shaped datagram was
  untouched; unregistered traffic cost about 270 ns per packet. On an
  interface lacking the program only empty and frame-shaped datagrams arrived
  in their 8-byte-longer form.
- **Tuple identity.** A tuple registered from the requested destination
  missed every datagram whose destination `connect4` rewrote (a service VIP,
  ADR-0164): the frame reached the backend. A tuple registered from the
  connected host socket's kernel peer was correct.

## Decision

- **Verdict strip.** The host verdict removes the frame from every non-empty
  guest→host datagram. It keeps the frame only when the datagram is empty, or
  when the payload itself parses as a valid frame (the escape rule, so a
  payload is never mistaken for a frame).
- **Egress unframe.** A fragment-aware, tuple-gated TC egress program on host
  `lo` and on every interface of the host's root network namespace removes a
  valid frame from a datagram on a registered tuple — including from the first
  fragment of a fragmented datagram, with the later fragments' offsets moved
  back to match — and passes every other packet unchanged. It never drops for
  bad magic.
- **Tuples come from the kernel.** A tuple is registered from the connected
  host socket's kernel-reported local address and peer (after any `connect4`
  rewrite), never from the requested destination.
- **Attachment set (user ruling 2026-10-06).** The program is installed on
  every interface in the host root network namespace. The set is refreshed at
  forwarder start, on each link-appearance notification, and at the audit
  cadence. A failed attachment is counted with its cause and retried at the
  next refresh; it never quiesces forwarding and is never classified as audit
  damage. Only the `lo` attachment gates startup and counts as damage when
  missing.

## Alternatives considered

- **Tuple-gated unframe on egress TC only, not fragment-aware.** Loses every
  datagram above the egress MTU. Rejected.
- **Fragment-aware egress TC only.** A missing or late attachment silently
  corrupts every datagram through that interface. Rejected in favour of hybrid,
  which confines that failure to empty and frame-shaped datagrams.
- **Removal in the host verdict only, empties dropped.** Drops every empty
  datagram (a D15 regression). Rejected.
- **Treat a failed non-`lo` attachment as audit damage.** Would quiesce every
  flow on the node for a gap that affects only empty and frame-shaped
  datagrams through one interface. Rejected.

## Consequences

- Non-empty datagrams leave the host independent of routing, interface set and
  MTU; only empty and frame-shaped datagrams depend on the egress program.
- On an interface not yet carrying the program, an empty datagram reaches its
  peer as 8 bytes and a frame-shaped payload in its framed form. Counters, the
  failure record and the audit's interface fact make the gap observable.
- A datagram encrypted by policy-based IPsec (xfrm without an interface)
  never crosses a TC egress hook in clear; the same gap applies to it. This is
  a validation item, not an observed fact.
- The host carries one TC egress program per root-namespace interface plus a
  fragment-tracking map; it is a no-op for unregistered tuples.
- Untested, and therefore validation items in the feature delta: NICs with UDP
  segmentation offload, several egress interfaces, policy routing, runtime
  interfaces, interfaces owned by other software, xfrm paths, and a
  frame-shaped payload above the MTU.
