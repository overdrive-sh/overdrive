# ADR-0156 — A VM's vsock CID is derived from its admission lease, not allocated separately

## Status

**Proposed — decision D16 approved by user 2026-10-05, with next-fit
assignment (independent DESIGN review finding M-9) confirmed by user
2026-10-06; pending independent DESIGN review.** GH #295.
Amends on acceptance: ADR-0118 (assignment order of the address pool).
Recorded in the #295 feature delta, § *[REF] vsock Attachment Replacement
DESIGN — PROPOSED 2026-10-05*. Depends on ADR-0146. How the derived CID is
made the lease's own on the host kernel is ADR-0170 (D16-CLAIM, approved by
user 2026-10-07; pending independent DESIGN review).

## Context

Kernel vhost-vsock requires every guest CID to be unique on the host
(`spike/ch-vhost-vsock-findings.md`, P5), and a CID is free again
immediately after the holding VMM exits (P7, 20/20). Today every VM uses
CID 3, which works only because each CH instance has its own Unix backend.

ADR-0132 and ADR-0133 make the guest-address lease the single admission
authority, held from assignment until cleanup finishes, including while it is
Retiring. One pool exists per server; nothing is persisted or adopted at boot
(ADR-0118).

## Decision

Each guest's CID is `GUEST_CID_BASE + lease_offset`, where `lease_offset` is
the offset of the allocation's `workload_addr` within the node's guest prefix.

- The CID is computed in exactly one place: the pool's `assign`.
- It is never persisted.
- The offset is free again only when the lease is released.
- **Next-fit assignment.** `assign` considers free offsets starting at an
  in-memory cursor that advances past each assigned offset and wraps. The
  cursor is not persisted; it starts at the first offset on each boot.
  Which of those offsets `assign` may take is decided by the kernel claim of
  ADR-0170.

Uniqueness among Overdrive's own VMs follows from lease uniqueness over
Admitted and Retiring leases; uniqueness against a CID still held by a VMM
the pool does not lease (one that survived a `serve` crash) is the kernel's
claim (ADR-0170).

## Alternatives considered

- **A separate CID allocator.** A second admission authority that can drift
  from the lease. Rejected.
- **A random CID with retry on collision.** Non-deterministic, breaking DST
  seed reproducibility. Rejected.
- **First-fit assignment.** Correct under ADR-0170, but reuses the lowest
  free address and CID immediately after release. Next-fit spreads reuse
  across the prefix. Rejected in favour of next-fit (M-9).

## Consequences

- One lease names two derived identities, `workload_addr` and CID, released
  together and last (ADR-0133).
- `workload_addr` reuse is spread across the prefix instead of always taking
  the lowest free address.
- The derivation is a pure function, pinned by a proptest; the assignment
  discipline is checked by the formal model (ADR-0168, module `cid_lease`) and
  a seeded-sim scenario.
