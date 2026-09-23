# ADR-0115 — TCX endpoint classification feeds the existing transparent-mTLS core

## Status

**Accepted — the current #295 contract is user-approved and independently
approved through D-295-DISTILL-9 at review iteration 12 on 2026-09-17.**
The user-directed D-295-DELIVER-04-01 evidence-boundary correction on
2026-09-23 requires no further review and changes no product mechanism.
The approved compound decision is D-295-2: TCX/SCHED_CLS is the primary
microVM TAP endpoint classifier; nftables remains the IP TPROXY/output socket
delivery mechanism and supplies only the minimum bridge fail-closed guard for
an absent TCX link. ADR-0124 owns runtime recovery and ADR-0125 owns constant
IP nft rules/shared elements. The internal shared-switch owner is exposed only
through the doc-hidden control-plane application port needed by the sibling sim
adapter; no low-level netlink/nft/BPF port is added.

## Context

Part A proved a shared-bridge/TAP catch through nftables bridge classification.
Part C then replaced that classifier with a real Rust/aya-rs
`BPF_PROG_TYPE_SCHED_CLS` program attached through TCX ingress to both real
microVM TAPs. The classifier reproduced bridge-local delivery, survived the
loader through pinned links/maps, was independently adopted and removed, and
fed the production TLS 1.3/kTLS/splice path. It failed closed on endpoint-map
miss, source-MAC spoof, source-IP spoof, and direct non-TCP guest-to-guest
bypass. No bridge-family nft classifier existed in the successful run.
The retained evidence is committed at
`7a00464969e98fe00764e0ab707428e8cd82a026`.

Part C did not prove safe traffic behavior after a deliberate TCX link removal:
once no program runs, program-level map-miss/drop logic cannot execute. A small
bridge-hook guard must therefore fail closed without duplicating endpoint
classification.

## Decision

Use one aya-rs SCHED_CLS endpoint program and one shared endpoint map. Attach
the program with TCX ingress and first ordering to every managed guest TAP. Key
the endpoint map by ingress ifindex; each value holds expected source IPv4,
expected source MAC, and the node bridge MAC.

`overdrive-dataplane::guest_tcx` owns the aya boundary. It maps aya's ingress,
egress, and custom-parent attachment values one-for-one into the semantic
`TcxAttachPoint` fact and retains raw map/program/pin failures behind one
source-bearing `GuestTcxError`. `overdrive-control-plane::guest_network`
re-exports those exact types for its facts/errors and sibling sim consumer; no
raw aya type enters control-plane facts or `overdrive-core`, and no duplicate
TCX error taxonomy exists. Exact type signatures live only in the #295 feature
delta.

The same dataplane boundary owns semantic attachment queries, exact pinned-link
detach, endpoint presence/removal, and counter reads. It keeps endpoint/counter
ABI and raw aya types private, returns sorted program identities, and preserves
map/program/pin/link/I/O sources through the canonical TCX error. Production
audit/teardown and the external double-loss fixture use these same high-level
operations; no subprocess or test-only host-owner fault hook exists.

The independent bridge safeguard is owned through the semantic
`overdrive-netlink::nft::bridge` adapter. Its read-only, generation-bracketed
observation preserves actual family/table identity, actual kernel rule order,
duplicate owned rule occurrences, expected members, and every owned or foreign
child inside the candidate table. Every chain occurrence appears exactly once
as base with only observed base attributes, regular without fabricated hook/
priority/policy/type, or unsupported. Every other child belongs to exactly one
typed or unsupported-child collection. Rule occurrences carry the canonical
adapter-owned semantic expression program, retaining wrong value/order,
duplicates and ordered unknown expressions without exposing raw nft ABI; the
same facts form guest-network expected and observed postconditions. The adapter
classifies the complete inventory as absent, exact or conflict without
mutation; non-repairing owner audit consumes that same result. Granular setup
and reverse cleanup remain staged and idempotent. The aggregate deletion used
by production cleanup and the external double-loss fixture may delete only an
exact table containing exclusively the owned chain, set, ordered rules and
expected members; every wrong identity, duplicate, partial or foreign child
refuses while outside objects remain unchanged. Specification and member
inputs are validated before I/O. Existing IP-family TPROXY rules and their
adapter API are unaffected.

The control-plane host owner's module-private scratch I/O invokes this same
dataplane adapter for TCX load/verifier, map/link pin/adopt/query, endpoint
mutation, detach, cleanup, and inventory effects. It receives the canonical
typed source and never reimplements aya operations. The host owner—not the I/O
adapter—orders the classifier/original-destination/detached-link stages and
constructs the cleanup aggregate.

The program validates endpoint registration and source identity. Valid ARP and
validated non-TCP traffic addressed to the bridge receive an **accepted mark**.
All validated TCP receives an **intercept mark**, bridge destination-MAC
rewrite, `PACKET_HOST`, and `TC_ACT_OK`, preserving original IP/port for host IP
TPROXY. Map miss, malformed input, source spoof, and non-TCP traffic addressed
directly to another guest return `TC_ACT_SHOT`. The eight Part-C counter classes
remain the observable verdict vocabulary.

Valid ARP is not an offset-only sender-IP check. It requires a complete
Ethernet/IPv4 ARP header, Ethernet+IPv4 types, 6/4 address lengths, request or
reply opcode, Ethernet source and ARP sender hardware address equal to the
registered MAC, and sender protocol address equal to the registered IPv4.
Header/opcode/truncation failures count malformed; either sender-MAC mismatch
counts MAC spoof; sender-IP mismatch counts IP/ARP spoof. The exact byte/counter
contract and negative probe partition live in the feature delta.

This tightens one scratch-only Part-C branch: its bounded program passed every
bridge-MAC-destined packet before protocol classification. Production must
intercept bridge-MAC-destined TCP as well, because off-subnet mesh frontends
use the guest's gateway MAC and still require leg-F resolution. Part C proves
the TCX mutation/local-delivery primitive, not this expanded input population;
the production probe must include both peer-MAC and gateway-MAC TCP.

Use `0x295a` as the intercept mark and `0x295b` as the accepted mark. The
bridge-family safeguard is exactly one managed-interface set and one
`type filter hook prerouting priority -300; policy accept` chain containing
three ordered rules:

1. `iifname @managed_taps meta mark 0x295a accept` — preserve intercept;
2. `iifname @managed_taps meta mark 0x295b meta mark set 0 accept` — clear the
   accepted proof mark before ordinary delivery;
3. `iifname @managed_taps counter drop` — fail closed and expose the
   deliberate-link-loss oracle.

The guard checks only platform-managed TAP membership and the proof mark
authored by TCX. It does not parse Ethernet/IP identity, choose a route, select
a backend, or decide whether traffic is mesh. It is therefore a fail-closed
attachment safeguard, not a second endpoint classifier. If a pinned link is
intentionally or accidentally absent, an unmarked guest packet reaches rule 3
and cannot fall through into ordinary bridge forwarding.

nftables IP prerouting/output remains responsible for mark-to-leg-F TPROXY,
host-originated leg-B-to-leg-C delivery, and leg-S exemptions. ADR-0125 defines
that delivery storage as three shared element sets and eight constant IP rules
while retaining the existing install-method
surface. `HostMtlsEnforcement` remains unchanged for TLS 1.3, kTLS TX/RX, and
kernel splice pumps; connection scaling belongs to
[GH #300](https://github.com/overdrive-sh/overdrive/issues/300), not #295.

### Same-node peer-facing byte boundary

For a same-node `Mesh` peer, the encrypted peer-facing boundary is not the
shared bridge. Outbound leg B is a host-local TCP socket to the selected
backend workload address and declared port. The constant `output` route-hook
divert marks that tuple; the accepted fwmark rule selects table 100's
`local 0.0.0.0/0 dev lo` route; and shared leg C accepts the connection before
ordinary bridge egress. The real leg-B/leg-C TCP segment is therefore observed
on loopback. Leg C then decrypts and the marked leg-S socket delivers plaintext
over the shared bridge to the destination TAP. The caller's TAP/bridge path to
leg F is likewise plaintext by design.

S-ND295-01 must bind its authoritative AF_PACKET TLS capture to the exact
loopback ifindex before the first dial. A live `ss -H -n -t -i -e` journal must
identify exactly one leg-B tuple from the shared-bridge gateway address to the
selected backend address and port whose single record reports TLS 1.3,
`tcp-ulp-tls`, TX configuration, and RX configuration. The tuple and exact
reverse must each reassemble at least one TLS application-data record (`0x17`)
with zero occurrence of either byte-distinct plaintext marker. Capture drops,
truncation, gaps, conflicting bytes, wrong-interface frames, a missing
direction, or zero/multiple correlating tuples fail closed.

Steady-state splice evidence is correlated to that same socket: the live
`ss -e` inode maps through `/proc/self/fd` to the in-process leg-B fd, and the
existing strace-style thread-group observation must show completed positive
`splice(2)` calls with that fd as request destination and response source for a
second post-establishment byte-distinct exchange. This is test-only observation
of the existing production process and kernel; it adds no adapter accessor or
product hook.

A separate loss-accounted shared-bridge/TAP capture is a positive plaintext
delivery oracle, not TLS evidence. It proves the guest-local flow toward leg F,
the unique non-kTLS leg-S tuple from node gateway address and ephemeral port to
selected backend address and declared port (plus its reverse) carrying the
request/reply markers, and the absence of a direct caller-guest-to-Service-
guest bypass. The caller guest-address-to-frontend tuple is the distinct leg-F
positive. Cross-host physical wire selection remains outside #295 and belongs
to GH #298.

### Ownership and order

- The node shared-switch owner owns the program, endpoint/counter maps, bpffs
  hierarchy, managed-TAP nft set, and the three-rule bridge guard.
- Its private host implementation and sibling sim implementation satisfy the
  same application-owner contract for startup probe, stale sweep, shared
  converge/audit, TAP quiescence, and inherited allocation provision/teardown.
  This is one owner boundary, not a second classifier or a low-level kernel
  fault abstraction.
- The application owner and its source-bearing orchestration error live in
  control-plane. Dataplane alone converts aya attachment/error types; netlink
  retains its existing canonical error. Core contains none of those adapter
  types.
- Per-allocation network provisioning first adds a down TAP to the managed set,
  then writes its endpoint-map value, attaches/pins TCX, queries the exact
  ifindex/program identity, and only then permits VMM attachment. Every partial
  state is fail-closed by the guard.
- Links and maps are pinned. Closing the loader does not detach them. Within one
  boot, the owner reopens/adopts pins for inspection, update, and teardown.
- On process boot, existing VM reclamation runs first. Prior-epoch pins are
  adopted only to verify their target/program and remove them with their dead
  TAP; no VMM or allocation is adopted.
- Teardown stops the VMM and mTLS owner, deletes the endpoint-map entry,
  unpins/detaches TCX, deletes the TAP while it is still in the managed set,
  then removes set membership and releases the guest address. A detach or TAP
  deletion failure leaves the bridge guard active.

The production bpffs hierarchy is
`/sys/fs/bpf/overdrive/mtls-endpoints/`: maps at
`maps/endpoints` and `maps/counters`, and per-TAP ingress links at
`links/<tap>-ingress`. This is ownership inventory, not operator API. A pin
outside this hierarchy is never adopted as an Overdrive endpoint link.

Cilium validates the lifecycle shape but is not copied as a complete answer.
In the user-supplied checkout at `/Users/marcus/git/cilium/cilium` commit
`e99150f8d8f403eca51ed82138d4ae20a265c8f3`, its endpoint loader attaches TCX
to each endpoint device and commits bpffs pins after attachment
([primary source](https://github.com/cilium/cilium/blob/e99150f8d8f403eca51ed82138d4ae20a265c8f3/pkg/datapath/loader/endpoint.go#L214-L257));
its TCX loader pins links, updates existing pins, recognizes defunct links, and
queries attachment state
([primary source](https://github.com/cilium/cilium/blob/e99150f8d8f403eca51ed82138d4ae20a265c8f3/pkg/datapath/loader/tcx.go#L34-L171));
and endpoint deletion removes pinned links before endpoint map/program state
([primary source](https://github.com/cilium/cilium/blob/e99150f8d8f403eca51ed82138d4ae20a265c8f3/pkg/datapath/loader/endpoint.go#L150-L168)).
Cilium also orders policy-program reachability before endpoint attachment
([primary source](https://github.com/cilium/cilium/blob/e99150f8d8f403eca51ed82138d4ae20a265c8f3/pkg/datapath/loader/endpoint.go#L214-L237)),
publishes endpoint-map state only after the endpoint datapath is realized
([primary source](https://github.com/cilium/cilium/blob/e99150f8d8f403eca51ed82138d4ae20a265c8f3/pkg/endpoint/bpf.go#L481-L506)),
and documents detach-before-map cleanup plus endpoint-map deletion as the point
where many packet paths begin dropping
([primary source](https://github.com/cilium/cilium/blob/e99150f8d8f403eca51ed82138d4ae20a265c8f3/pkg/endpoint/bpf.go#L867-L901)).
Cilium's program-level missing-policy tail call returns `DROP_EP_NOT_READY`
([primary source](https://github.com/cilium/cilium/blob/e99150f8d8f403eca51ed82138d4ae20a265c8f3/bpf/lib/local_delivery.h#L24-L56)),
but that cannot run when the endpoint entrypoint itself is detached. The
Overdrive bridge guard closes precisely that different gap; no claim is made
that Cilium supplies it.

## Alternatives considered

### Full nftables bridge classification

Rejected as the primary path after the user selected TCX. Part A proves it is
feasible, but it provides neither Part C's ifindex-keyed endpoint identity
validation nor its distinct map-miss/MAC-spoof/IP-spoof/direct-bypass counters.
nftables remains only for TPROXY/output delivery and the minimum proof-mark
guard.

### TCX with no bridge guard

Rejected. Pinned links survive loader exit and map miss fails closed, but a
deliberately detached link executes no classifier and can restore ordinary
bridge forwarding.

### Duplicate endpoint validation in nftables

Rejected. Rechecking source MAC/IP/protocol/destination in bridge rules would
create a second classifier and two policy sources. The guard accepts only the
two TCX proof marks and otherwise drops.

### Userspace/vhost-user switch

Rejected for current requirements. It puts guest packets through a userspace
switch, adds a daemon/shared-memory/reconnect lifecycle, and still needs kernel
socket injection to reuse kTLS/splice.

### Treat the shared bridge as the same-node peer-facing encrypted wire

Rejected. The bridge legitimately carries guest-local plaintext before leg F
and after leg S. The output divert and local policy route deliver the selected
leg-B socket to leg C before ordinary bridge egress, so absence of that exact
kTLS tuple from the bridge is expected rather than evidence of a product
failure.

### Let an unqualified all-interface capture choose the evidence boundary

Rejected as the authority. It is useful diagnostic evidence and may retain the
actual ifindex for every frame, but accepting whichever interface happens to
contain a same-port TLS-looking stream would leave the security boundary
ambiguous. The accepted local route pins loopback; exact tuple, direction,
kTLS, and splice correlation then select one socket fail-closed.

## Consequences

Positive: the primary classifier now has real-metal verifier, memory, attach,
negative-security, pin/adoption, wire, zero-copy, and cleanup evidence. One
shared program/map family provides per-endpoint identity validation and
explicit counters without userspace packet forwarding. Negative: every TAP
owns a pinned TCX link, the shared endpoint map and bpffs state need recovery,
and a small bridge nft guard becomes a second dependency—but not a second
classifier. Part C's 296 verified instructions, 4,096-byte program memlock,
4,208-byte bounded-probe map memlock, ~7.6–7.9 ms attach/pin, and ~50.8 ms
load/verifier are point measurements, not the pinned-kernel baseline or 16k
capacity proof. The S-ND295-01 clarification changes only evidence attribution:
loopback proves the same-node encrypted leg-B/leg-C segment, while bridge/TAP
capture proves the intentional plaintext leg-F/leg-S boundary and no bypass.
