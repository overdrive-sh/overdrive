# ADR-0122 — Use one typed guest-network operation error family

## Status

**Accepted — user-approved and approved by system design review iteration 5 on 2026-09-16.**
GH #295 DESIGN stage 1. This records ERR-295-A. The user-authorized solution-
review F-02 remediation amends only B1 visibility: the sibling simulation
adapter needs the same error/plan/port contract across a crate boundary.
The user-authorized S2-F01 remediation adds one source-less boot-precondition
variant for a non-BootClosed EXEC gate and one transparent control-plane boot
wrapper; runtime and allocation disposition are unchanged.

## Context

The shared-bridge path deletes `NetSlotExhausted` and netns/veth-specific
provision errors. Existing placement `NoCapacity` occurs before dispatch and
cannot represent below-cap allocator drift. Shared TAP, netlink, nft, endpoint
map, TCX, pin, and cleanup effects also need their original typed causes.

Splitting each cause into a new public action-shim variant would widen the shim
surface repeatedly and duplicate operation classification.

## Decision

Create one public operation discriminator and one public `GuestNetworkError`
family. It distinguishes address-pool exhaustion, netlink/bridge-guard, BPF
map, BPF program, bpffs pin, ordinary I/O, and successful-read/wrong-
postcondition failures. Postcondition mismatch carries closed structured
expected and observed TAP identity/type/persistence/VMM-owner, bridge identity/
type/gateway-prefix, link master/up, TCX attachment, endpoint/counter-map
identity/capacity, endpoint-map key/value, pinned-link, bridge-guard, or cleanup-
complement facts. The async `GuestNetworkProvisioner` and opaque/read-only plan
are `#[doc(hidden)] pub` solely for the existing `overdrive-sim` dependency;
the host implementation, plan construction, address pool, and production
dispatch stay control-plane-private. Both adapters return this family through
one module-local public result alias. Exact visibility, accessors, and the two
test-gated production-owner seams live only in the feature delta.

`ShimError` gains exactly one transparent guest-network wrapper. Only that outer
conversion is automatic. Adapter sites attach the precise operation and typed
source explicitly; no cause is formatted into a string or flattened. Pool
exhaustion remains a non-terminal infrastructure-drift refusal with degraded
node health, never an allocation failure.

Exact Rust variants, fields, visibility, and conversions live exclusively in
the #295 feature delta.

IP-family shared rules/sets/elements are worker-owned and use the separate
closed `InterceptError` vocabulary. They are deliberately absent from
`GuestNetworkOperation`, preventing two owners for the same effect.

## Alternatives considered

### Crate-private error plus decomposed public shim variants

Rejected. It avoids a public aggregate error but adds six public shim variants
and repeats the same operation/source taxonomy at the orchestration boundary.

### Reuse `PlacementError`, `NetSlotExhausted`, or `VethProvisionError`

Rejected. Their owners and meanings are respectively pre-dispatch scheduling,
deleted slot capacity, and deleted netns/veth topology.

## Consequences

Positive: one cause-preserving error path covers assignment, async provision,
teardown, runtime convergence, and cleanup without compatibility branches.
Negative: the public error and operation enums become a contract that must
remain exhaustive and source-honest as shared-network effects evolve; semantic
postcondition facts add a second, source-less error class that adapters must
populate exactly.
