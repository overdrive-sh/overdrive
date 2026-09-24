# ADR-0132 — Linearize node-wide guest-attachment admission at guest-address assignment

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R6. Proposed 2026-09-23 and revised 2026-09-24; reviewed by
independent DESIGN review rounds 3, 4, and 5 and the round-5 verification
(`arch_rev_20260924_netns295_r5_verify`); accepted by the user on 2026-09-24
with the replacement DESIGN. It supersedes the location of the cap in
ADR-0121, which is the placement decision; ADR-0121 states this explicitly. The cap's value is a fixed placeholder (ADR-0117,
ADR-0133). Exact signatures live only in the #295 feature delta.

## Context

ADR-0121 enforces the fixed cap in the placement decision. A seeded proof
(`netns_density_node_admission`, seeds `186055177052160001` and `295032`)
admitted one attachment more than the cap in four cases: placement at the cap,
placement with one admission in flight, crash replacement, and operator resume.
Four facts explain it:

- The scheduler receives only the target workload's own rows, so the cap check
  sees zero or one row.
- Up to eight evaluations for different workloads run concurrently, and no lock
  spans hydration through dispatch.
- `RestartAllocation`, which serves both crash replacement and operator resume,
  never calls the scheduler.
- An in-flight admission already holds a lease but has no row yet, so even a
  node-wide count of Running rows is too late.

The only node-wide atomic claim that covers every attachment class is
`GuestAddressPool::assign`. It takes one mutex, is idempotent per allocation,
and is taken by both the start path and the restart-successor path. Today the
pool is a process-global static (`guest_network.rs:525-548`, with a hardcoded
prefix), so an in-process server restart inherits the previous server's
leases.

Mature orchestrators linearize admission the same way: optimistic placement
over a snapshot, then one serialized node authority that may refuse. The
Kubernetes kubelet re-runs the scheduler's fit check at admission and rejects
with `OutOfpods`; Nomad's plan applier re-verifies fit for every plan and may
reject part of it. No surveyed system uses the address allocator itself as the
count gate: Kubernetes keeps `maxPods` separate from IPAM, and IP exhaustion
then surfaces after placement. (Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Findings 4.1, 4.2 and 4.3.)

## Decision

`GuestAddressPool::assign` is the sole linearization point for the node's
guest-attachment cap. Under its single mutex it refuses a new lease with a typed
refusal when the held population, Admitted plus Retiring (ADR-0133), has
reached the cap.

Every attachment-creating path passes through that one claim:
`StartAllocation`, the `RestartAllocation` successor, and any future path that
provisions a guest network. A restart admits its successor through the same
`assign`; its predecessor's lease keeps counting until the predecessor's
cleanup finishes (ADR-0133).

One pool exists per server instance. The composition root constructs it from
the node's guest prefix and injects it into the action shim and into the
placement read-port (ADR-0134). No process-global pool exists.

An admission refusal is non-terminal infrastructure pressure. It writes no
allocation row and records no failure. Any placement-time check is advisory
only (ADR-0134).

## Alternatives considered

### Scheduler over node-wide rows with serialized placement

Rejected. It cannot see in-flight admissions, which hold a lease but no row. It
does not cover `RestartAllocation`, and serializing all placement would remove
the runtime's cross-workload concurrency.

### A separate node admission counter checked before `assign`

Rejected. It is the Kubernetes shape (`maxPods` beside IPAM), but in #295 every
attachment's kernel identity (address, TAP name, MAC) is derived from its
lease, so the lease set already is the attachment set. A second counter would
duplicate it as another node-wide source of truth, and every path would have to
keep the two consistent. Fusing the count into the allocator has no direct
precedent; it is chosen because the lease is the attachment.

### Keep the placement-time cap and patch hydration only

Rejected. It leaves the in-flight, concurrency, and restart undercounts in
place.

### An atomic predecessor-to-successor slot handover at restart

Rejected. Under ADR-0133 a retiring predecessor still counts, so handing its
slot over at retirement frees nothing. The slot frees when the predecessor's
cleanup finishes and its lease is released.

### A process-global pool

Rejected. An in-process server restart would inherit leases from a server that
no longer owns their kernel state, and the read-port would reach a global
instead of an injected port.

## Consequences

Positive: one atomic check-and-claim covers every attachment class. The proof's
undercount becomes structurally impossible.

Negative: the pool becomes the admission owner, which is a new responsibility
for it. Admission refusal becomes a dispatch-time outcome, so a placement-time
advisory is needed to avoid a retry loop (ADR-0134). `PoolExhausted` remains a
distinct infrastructure-drift outcome; it is unreachable while the cap is below
the prefix's usable address count.
