# ADR-0118 — Guest address lease replaces `NetSlot` as network ownership

## Status

**Accepted — user-approved and approved by system design review iteration 5 on 2026-09-16.**
GH #295 DESIGN stage 1. This records D-295-4 and C-295-E; ADR-0122 owns the
typed exhaustion/error family.

**Amended 2026-09-24 by ADR-0132 and ADR-0133 (#295 D-295-R6/R7, accepted
with the correctness-recovery replacement DESIGN; the D-295-R7 counting policy
is a user ruling of the same date).** This decision is operative in code
committed at HEAD `db3af700` on the #295 feature branch: a process-global pool
(`static ACTION_POOL`, `guest_network.rs:523`) with exactly `assign`, `release`,
and `snapshot` (`:452`, `:509`, `:518`); not merged to `main`, no persisted
state. The amendment makes the lease pool the admission linearization point:
each lease carries an Admitted or Retiring state, and both count against the
cap until cleanup finishes. One pool exists per server instance, built in the
composition root, replacing the process-global static. The Decision's sentence
that the pool "exposes only assignment, release, and snapshot semantics" is
amended accordingly: the pool also exposes retirement (Admitted to Retiring)
and a read-only occupancy observation (ADR-0134). Address derivation,
release-last, and the no-adoption boot rule recorded here are unaffected.

## Context

The current `NetSlot` is simultaneously an address carve, interface/netns name,
capacity ceiling, and recovery key. Those meanings exist only because one
allocation owns two `/30`s plus a netns/veth/TAP cell. A shared bridge still
needs one collision-free IP, MAC, and TAP identity per allocation, but it does
not need a `/30` or a slot-shaped master key.

## Decision

One internal, process-held guest-address pool owns leases by `AllocationId` and
exposes only assignment, release, and snapshot semantics *(amended 2026-09-24:
also retirement and a read-only occupancy observation, one pool per server;
see Status)*. The exact
implementation-facing operations are pinned in the feature delta. Assignment
chooses the smallest free guest IPv4 address from the node-owned prefix,
excluding network, broadcast, and bridge-gateway addresses. Derive the
IFNAMSIZ-safe TAP name from the address's 16-bit host offset
(`ovd-tp-<4hex>`) and derive the locally administered unicast MAC as
`02:00:<four IPv4 octets>`. `workload_addr` remains the persisted observed IP.

No public numeric slot exists. Delete `NetSlot`, `NetSlotAllocator`,
`NetSlotExhausted`, slot-to-address inverse cleanup, and slot-derived naming.
Teardown removes classifier membership and the TAP before releasing the lease.
At boot, existing VM reclamation kills unsupervised VMs first; the switch owner
then sweeps prior-epoch TAP/map residue before initializing the new held set.
It does not adopt a surviving VMM or reconstruct a slot map.

The removed `NetSlotExhausted -> WorkloadNetnsProvisionFailed` disposition is
therefore deleted, not translated. Node admission caps active guests below the
address-pool limit (ADR-0117; since the 2026-09-24 amendment enforced at
assignment over held leases, ADR-0132 and ADR-0133); an allocator exhaustion inside that admitted
envelope is a typed, non-terminal infrastructure-drift refusal, not a permanent
allocation failure that consumes restart budget.

## Alternatives considered

### Rename `NetSlot` and keep its numeric index

Rejected. It preserves the forbidden master-key coupling and invites `/30` and
name derivations to survive under a new label.

### Hash `AllocationId` directly into TAP/MAC/IP

Rejected. A bounded hash makes collision unlikely rather than unrepresentable,
and collision resolution would still require an allocator.

### Persist a new lease table

Rejected for the proposed single-node lifecycle. Boot reclamation removes all
unsupervised VMs before switch sweep, and `workload_addr` already persists the
operator-visible fact. A second durable ownership store adds recovery and
schema machinery without a surviving resource to adopt.

## Consequences

Positive: the address is the only allocation key, name/MAC uniqueness is
structural inside the node prefix, and the obsolete exhaustion cause disappears.
Negative: the single internal address pool is process-held and relies on boot
reclamation plus a complete switch sweep; its assignment/release/snapshot
semantics and cleanup complements require explicit verification.
