# ADR-0176 — Declared TCP listener ports are admitted against the node's guest-prefix steering capacity, so a listener never fails to steer for lack of space

## Status

**Proposed — decision D8a-CAP approved by user 2026-10-08; pending
independent DESIGN review.** GH #295. Recorded in the #295 feature delta,
§ *[REF] vsock Attachment Replacement DESIGN — PROPOSED 2026-10-05*; the exact
contract is § *Core vocabulary* (`GUEST_INTAKE_STEERING_MAX`,
`GuestAddressPool::assign`) and § *Driven port — guest-prefix steering*.

## Context

Every steered intake listener holds one entry in the guest-prefix steering
map (ADR-0171), and a socket map has a fixed `max_entries`. An intake
listener exists only for a declared TCP port of a held lease (ADR-0173,
ADR-0163). Admission counts attachments (ADR-0121, placeholder 16,384) but
did not count declared ports, so a node could admit more listeners than the
map holds. A listener that found the map full could never be steered until
another closed: its port stayed refused, at activation the allocation failed
with an error indistinguishable from a bind failure, and the owner retried an
insert that could not succeed. The formal model showed the exposure
unbounded (`intake-steer-forever-unreachable`).

The number of declared ports is known when an allocation is admitted. An
entry is cheap: in the kernel's socket-hash layout one entry is a slab object
of about 64 bytes allocated at insert, and the map pre-allocates about
16 bytes per hash bucket at creation; a steered listener's psock exists
whether or not the map is large.

The bridge-era capacity contract (CAP-295-A) stated that any valid
distribution of listener ports remained supported product semantics, without
a node bound.

## Decision

- **Capacity sized from the admitted bound, with headroom.** The steering
  map's `max_entries` is `GUEST_INTAKE_STEERING_MAX = 262,144`: 16 declared
  TCP listener ports per attachment at the admission placeholder of 16,384,
  four times the T1-PORT4 profile (4 ports per attachment). Cost: about 4 MiB
  of buckets at creation and about 16 MiB of entries at full occupancy,
  measured by V-9.
- **Admission counts declared ports.** The pool's admission reserves an
  allocation's declared TCP listener ports together with its lease and
  releases them with the lease. An admission whose ports would take the
  node's reservations past the capacity is refused with a typed,
  non-terminal error before any effect, projected like the attachment cap.
  Reservations of Retiring leases count until release, because their
  listeners may still be open.
- **No capacity failure at steer time.** Because every open intake listener
  belongs to a held lease and declares one reserved port, the map always has
  room for every listener the owner steers; the steer path has no
  capacity-driven retry.
- **What remains retried.** A steer that fails because the kernel cannot
  allocate the entry (memory pressure) closes the listener and is retried
  while the port is wanted; the port is refused meanwhile. Any other steer
  failure is an Overdrive defect: it is reported with its cause and not
  retried.

## Alternatives considered

- **Keep the map at 65,536 entries and retry a full map.** A retry that
  cannot succeed until another listener closes compensates for a fact the
  design can know at admission. Rejected.
- **Size the map for every possible port of every attachment (16,384 ×
  64,510).** About 10^9 entries; the bucket array alone would be gigabytes.
  Rejected.
- **Count only ports the guest reports listening.** Unknown at admission;
  reintroduces a steer-time capacity failure. Rejected.
- **A per-spec port limit at deploy.** Every spec's distinct ports fit the
  node capacity (at most 64,510 of them), so a per-spec limit would refuse
  nothing the node bound does not already handle. Rejected.

## Consequences

- A node supports at most 262,144 declared VM TCP listener ports across its
  held leases. A deployment beyond it is refused at admission with a typed
  error, never admitted and left unreachable. This replaces CAP-295-A's
  "any valid distribution remains supported" for VM attachments.
- At up to 16 declared ports per attachment on average the full 16,384
  placeholder is admissible; workloads declaring more ports reach the port
  bound before the attachment bound. This is a workload-driven bound like the
  flow capacity (ADR-0155), not an attachment-topology ceiling.
- Activation and post-activation steering fail only under kernel memory
  pressure or an Overdrive defect, each reported with its cause.
- Rests on the kernel fact that closing a listener frees its map slot in the
  same step (K-L3, V-26).
