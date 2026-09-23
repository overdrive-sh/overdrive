# ADR-0122 — Use one typed guest-network operation error family

## Status

**Accepted — the current #295 contract is user-approved and independently
approved through D-295-DISTILL-9 at review iteration 12 on 2026-09-17;
D-295-DISTILL-11's sourced component audit wrapper is autonomously authorized
and pending the trusted-checkpoint review. Amended 2026-09-23 by explicit user
direction with no review cycle to carry deferred TAP activation through this
same error family.**

## Context

The shared-bridge path deletes `NetSlotExhausted` and netns/veth-specific
provision errors. Existing placement `NoCapacity` occurs before dispatch and
cannot represent below-cap allocator drift. Shared TAP, netlink, nft, endpoint
map, TCX, pin, and cleanup effects also need their original typed causes.

Splitting each cause into a new public action-shim variant would widen the shim
surface repeatedly and duplicate operation classification.

## Decision

`overdrive-control-plane::guest_network` owns one public operation
discriminator and one public `GuestNetworkError` family beside the opaque plan
and both application ports. It distinguishes address-pool exhaustion,
netlink/bridge-guard, canonical TCX adapter, ordinary I/O, and
successful-read/wrong-postcondition failures. Postcondition mismatch carries closed structured
expected and observed TAP identity/type/persistence/VMM-owner, bridge identity/
type/gateway-prefix, link master/up, TCX attachment, endpoint/counter-map
identity/capacity, endpoint-map key/value, pinned-link, bridge-guard, or cleanup-
complement facts. Allocation cleanup retains its existing allocation-scoped
fact. Startup scratch cleanup uses a distinct complement covering the owned
bridge/TAP, endpoint and counter maps, endpoint entries, TCX program/link,
separate map/link pins, and bridge-guard table/chain/set/rules/members. The async `GuestNetworkProvisioner` and opaque/read-only plan
are `#[doc(hidden)] pub` solely for the existing `overdrive-sim` dependency;
the host implementation, plan construction, address pool, and production
dispatch stay control-plane-private. Both adapters return this family through
one module-local public result alias. Exact visibility, accessors, and the two
test-gated production-owner seams live only in the feature delta.

Netlink failures embed the canonical `overdrive-netlink::NetlinkError`
directly. TCX failures embed one
`overdrive-dataplane::guest_tcx::GuestTcxError`, whose Map/Program/Pin/Link and
I/O variants retain the exact lower source. The outer guest-network error adds
only the operation context; it does not repeat those adapter variants. Dataplane
also projects aya attachment values into `TcxAttachPoint`, which replaces raw
`aya::programs::TcAttachType` in control-plane facts. Core carries neither the
orchestration error/facts nor any adapter type.

Operation discriminators distinguish bridge deletion; guard table, chain, set,
and rule creation/deletion; TCX load; and endpoint-map, counter-map, and TCX-
link pin/adopt/unpin effects. There is no ambiguous generic link-pin operation
or compatibility variant. Complete scratch-count query failure remains the
cleanup-complement operation because the corresponding unavailable field names
the exact resource family.

`ShimError` gains exactly one transparent guest-network wrapper. Only that outer
conversion is automatic. Adapter sites attach the precise operation and typed
source explicitly; no cause is formatted into a string or flattened. Pool
exhaustion remains a non-terminal infrastructure-drift refusal with degraded
node health, never an allocation failure.

Exact Rust variants, fields, visibility, and conversions live exclusively in
the #295 feature delta.

Deferred TAP activation adds no error, fact, or operation variant. The existing
`TapSetUp`, `TapSetDown`, `TapObserve`, `BridgeObserve`,
`GuardMemberInsert`, `EndpointMapObserve`, `TcxQuery`, and `TcxLinkPin`
discriminators plus `Tap`/`LinkMaster`/existing protection facts already cover
every mutation and read-back. The sole new public method is the awaited
`GuestNetworkProvisioner::activate(&GuestNetworkPlan) -> Result<()>`; the same
concrete `SharedGuestNetworkOwner` implements it. A lower set-up failure keeps
its canonical source, while a successful operation followed by wrong read-back
remains source-less `PostconditionMismatch`.

The durable action-shim failure reuses the already-shipped
`WorkloadNetnsProvisionFailed { stage, detail }` payload with closed stage
`guest_network_activate`. The legacy variant name is not ideal, but adding a
second public `TransitionReason` or flattening `GuestNetworkError` would be a
larger contract change. The obsolete `NetSlotExhausted` mapping stays deleted;
pool exhaustion remains non-terminal drift.

Startup probe failures preserve the existing typed lower-level source through
the applicable canonical TCX/netlink/I/O variant. Semantic classifier,
original-destination, and deliberate-detached-link guard mismatches use the
closed startup-probe stage vocabulary. Ordinary probe failure must clean its
scratch universe to zero. If cleanup itself fails or read-back remains
non-empty, the aggregate cleanup error retains the prior failure when one
exists, the direct cleanup error as its source, and the complete per-resource
scratch observation. Each family is either an observed count or unavailable;
unavailable is never empty. “Empty” requires every family to have been
observed as zero, and the complement has no default value that could fabricate
that state. A semantic non-empty cleanup result uses one source-less leaf cause
while the residue exists only in the aggregate observation. The primary and
cleanup fields are direct errors, never nested cleanup aggregates. Refusal and
non-publication remain mandatory; a cleanup failure never fabricates an empty
complement or overwrites the first diagnostic.

The private host scratch I/O returns only raw typed leaf sources, semantic
stage results, and individual resource counts. The host owner constructs every
public error and the complete complement, attempts the full reverse cleanup
after success or failure, continues after the first cleanup failure, and
queries all fifteen families. Public sim-owner scripting may return a completed
port result to test composition reaction, but it is not host cleanup evidence.

The bridge adapter distinguishes its own input validation from kernel I/O.
Invalid specification identifiers, fixed priority or marks, and invalid member
byte/NUL/IFNAMSIZ shape are typed validation failures produced before netlink
I/O. The host owner projects those closed caller/plan mismatches through the
existing source-less postcondition mismatch; it never fabricates a lower-level
cause. Only transport, malformed decode, ACK and kernel failures retain the
original operation-tagged `NetlinkError` source. Well-formed wrong family/
table/schema/hook/priority/program, duplicate owned occurrences, partial state,
unexpected members, and foreign children are absent/exact/conflict observation
outcomes; the host owner likewise projects conflict through the existing
source-less postcondition mismatch. The bridge-guard fact carries the
netlink-adapter-owned semantic rule facts, not normalized byte vectors. Its
expected side comes from the validated specification's canonical expected-rule
projection; its observed side comes from rule occurrences in kernel order.
Wrong values/order, duplicates and well-formed unknown expressions therefore
remain structured expected/observed evidence without control-plane raw-ABI
parsing or a copied expected program. No bridge-specific guest-network error,
raw nft type, or duplicate netlink/rule taxonomy exists.

Runtime audit adds no duplicate lower-level error family. One public audit
wrapper carries the closed `SharedGuestNetworkComponent` and retains the exact
existing `GuestNetworkError` as its source. The host owner tags the actual first
failed read-back; the reusable sim adapter can script each of the twelve closed
components and one exact next source. Standing sim failures use one structured
shared-component expected/observed fact and the shared-audit operation, rather
than formatting or inventing a kernel cause. The retained supervisor consumes
the component for recovery ordering and preserves the source for diagnostics.

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

### Extend the existing allocation cleanup complement

Rejected. Allocation teardown owns one allocation's TAP/link/pin/entry/member
universe while startup scratch cleanup also owns node-level bridge, maps,
program, and complete guard-program objects. One union fact would make ordinary
allocation cleanup appear responsible for shared-owner resources.

### Duplicate lower-level failures into a cleanup-only error enum

Rejected. It would duplicate the netlink/map/program/pin/I/O/postcondition
taxonomy and allow its source semantics to drift. The aggregate reuses the
direct `GuestNetworkError` source and forbids nested aggregates.

### Put the operation/fact/error family in `overdrive-core`

Rejected. The exact accepted sources are netlink and aya adapter errors; core
must not depend on either adapter crate. Mirroring them in core would either
duplicate the source taxonomy or flatten it.

### Expose raw aya types directly from control-plane

Rejected. It would give the orchestrator and sim-facing API a direct aya
dependency and bypass the existing high-level dataplane adapter home. The
dataplane-owned semantic attach projection and nested source error preserve all
information without leaking the library type.

### Move the whole guest-network error into dataplane

Rejected. The family also owns address-pool, bridge/TAP/netlink, cleanup, and
orchestration postconditions. Moving it would make dataplane depend on netlink
and absorb effects it does not own.

### Script the completed cleanup aggregate through the public owner port

Rejected as the host-algorithm test boundary. It proves the composition's
response to a valid typed owner result but lets the fixture author the primary,
cleanup source, and observation. The module-private raw-effect boundary makes
the real host owner produce those values instead.

## Consequences

Positive: one cause-preserving error path covers assignment, async provision,
activation, teardown, runtime convergence, and cleanup without compatibility branches.
Negative: the public error and operation enums become a contract that must
remain exhaustive and source-honest as shared-network effects evolve; semantic
postcondition facts add a second, source-less error class that adapters must
populate exactly. The startup cleanup aggregate uses boxed direct errors so the
primary failure and active cleanup source survive one refused boot without
string flattening, plus one explicit per-family observation whose unavailable
states prevent false zero-complement claims.
