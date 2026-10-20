# ADR-0215 — Each kind of flow bounds its pairing wait at exactly one side

## Status

**Accepted — the per-kind wait bounds under D15-R3 = (b), approved by the
user 2026-10-06.** GH #295. Not yet implemented. The deadline values are in
the #295 feature delta, § *[REF] Core vocabulary* and § *[REF] Per-kind
total orders*.

Related decisions, each separately reversible: the non-mesh connect bound
itself (D15-R3, ADR-0162); the dial and its typed results (INB-1,
ADR-0173); out-of-band pairing (D18, ADR-0158); a pending flow's place in the
quota (ADR-0208).

## Context

A flow waits between its request and its `Paired` (ADR-0158). An unbounded
wait holds the flow's quota place and sockets for ever. Two waits on one
flow, at both sides, can end in either order: if the opener gives up first, a
later answer meets a flow that is gone; if two deadlines are equal, the
order is a race.

## Decision

- **Guest-opened TCP: no opener deadline.** The host answers once its own
  connect ends; a non-mesh connect is bounded by the host kernel's SYN
  retries (ADR-0162, D15-R3).
- **Datagram association: the opener's deadline.** The opener that receives
  neither `Paired` nor `Refused` within the pinned pairing deadline aborts
  the flow.
- **Host-opened flow: the guest's timeout inside the owner's deadline.** The
  guest bounds its connect to the application by its inbound connect timeout
  and answers `Paired`, `Refused` or a timeout; the owner's pairing deadline
  for that flow is strictly longer than the guest's timeout, so a guest
  answer always arrives before the owner gives up.

## Alternatives considered

- **A bounded host connect for guest-opened TCP** (3 s, the pairing
  deadline less the request deadline). Fails an unresponsive destination
  earlier than the kernel does today, a change the kernel does not force.
  Rejected by the user (D15-R3 = (b), ADR-0162).
- **Equal deadlines on both sides of a host-opened flow.** Which side gives
  up first is a race; a late `Paired` meets a dial already failed. Rejected.
- **No deadline on a datagram association.** A destination that never
  answers holds a slot and a quota place with no end. Rejected.

## Consequences

- Every wait ends with a typed result at the side that bounds it; the other
  side discards a late answer (ADR-0166, ADR-0213).
- The bounds are part of the host-side protocol of every guest (ADR-0207);
  in a Linux guest `overdrive-init` keeps the guest's side. A guest that runs
  no Linux kernel is
  [GH #321](https://github.com/overdrive-sh/overdrive/issues/321).
