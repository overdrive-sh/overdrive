# ADR-0115 — TCX endpoint classification feeds the existing transparent-mTLS core

## Status

**Accepted — user-approved and approved by system design review iteration 5 on 2026-09-16.**
The approved compound decision is D-295-2: TCX/SCHED_CLS is the primary
microVM TAP endpoint classifier; nftables remains the IP TPROXY/output socket
delivery mechanism and supplies only the minimum bridge fail-closed guard for
an absent TCX link. ADR-0124 owns runtime recovery and ADR-0125 owns constant
IP nft rules/shared elements.

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
host-originated leg-B-to-leg-C delivery, and leg-S exemptions. ADR-0125 amends
that delivery storage from per-allocation/per-port rules to three shared element
sets and eight constant IP rules while retaining the existing install-method
surface. `HostMtlsEnforcement` remains unchanged for TLS 1.3, kTLS TX/RX, and
kernel splice pumps; connection scaling belongs to
[GH #300](https://github.com/overdrive-sh/overdrive/issues/300), not #295.

### Ownership and order

- The node shared-switch owner owns the program, endpoint/counter maps, bpffs
  hierarchy, managed-TAP nft set, and the three-rule bridge guard.
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
capacity proof.
