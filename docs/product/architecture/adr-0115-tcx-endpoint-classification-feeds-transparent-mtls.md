# ADR-0115 — TCX endpoint classification feeds the existing transparent-mTLS core

## Status

**Accepted — the current #295 contract is user-approved and independently
approved through D-295-DISTILL-9 at review iteration 12 on 2026-09-17.**
The user-directed D-295-DELIVER-04-01 evidence-boundary correction on
2026-09-23 was revised again after native capture proved a product-ordering
contradiction: guest-source ARP replies and TCP RST frames occurred before the
intercept-success receipt while the host TAP was already up. Under the user's
explicit no-review direction, the same D-295-DELIVER-04-01 amendment now also
defers the sole TAP activation until after that receipt and before EXEC release.
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

### Same-node protected-transport evidence boundary

For a same-node `Mesh` peer, the encrypted peer-facing boundary is not the
shared bridge. Outbound leg B is a host-local TCP socket to the selected
backend workload address and declared port. The constant `output` route-hook
divert marks that tuple; the accepted fwmark rule selects table 100's
`local 0.0.0.0/0 dev lo` route; and shared leg C accepts the connection before
ordinary bridge egress. Leg C then decrypts and the marked leg-S socket delivers
plaintext over the shared bridge to the destination TAP. The caller's
TAP/bridge path to leg F is likewise plaintext by design.

Native run `e72385d6` executed the prior ruling exactly. It uniquely correlated
live leg-B tuple `100.95.0.1:35260 → 100.95.0.2:18951`, TLS 1.3 kTLS TX/RX
state, socket inode, and sole in-process fd. A lossless loopback AF_PACKET
capture observed both tuple directions across 6,076 packets but reassembled
zero complete TLS `0x17` records. Loopback AF_PACKET bytes are therefore not an
authoritative ciphertext layer for this locally diverted production path.

S-ND295-01 instead joins three independent existing evidence layers:

- A live `ss -H -n -t -i -e` journal identifies exactly one leg-B tuple from
  the shared-bridge gateway address to the selected backend address and port.
  One record must report `tcp-ulp-tls`, TLS 1.3, TX configuration, RX
  configuration, and a nonzero inode; `/proc/self/fd` must map it to exactly one
  live in-process fd.
- The existing strace-style thread-group observer must show completed positive
  `splice(2)` calls with that exact fd as destination for the post-establishment
  request and source for its response, followed by the byte-exact guest reply.
  This ties kTLS state and bidirectional steady-state data movement to the same
  production socket without an adapter accessor or product hook.
- One loss-accounted all-interface AF_PACKET capture starts before caller-VMM
  release and retains actual ifindices. The exact leg-B tuple and reverse must
  appear only on loopback and never on a non-loopback interface. On the shared
  bridge and managed TAPs, plaintext markers are permitted only on the caller
  guest-address-to-frontend leg-F tuple/reverse and the unique non-kTLS leg-S
  node-gateway-to-selected-backend tuple/reverse. Every other observed
  non-loopback interface/tuple must contain zero markers, and direct caller-
  guest-to-Service-guest bypass must be absent.

Missing or ambiguous tuple/inode/fd ownership, a missing splice direction,
capture loss/truncation, leg-B on a non-loopback interface, plaintext on an
unapproved interface/tuple, or direct bypass fails closed. A future cross-host
physical-wire receipt must still prove TLS records (`0x17`) and no cleartext on
that real egress interface, but cross-host selection remains GH #298 and is not
an S-ND295-01 claim.

### S-ND295-01 intercept-live timing receipt

The existing production event `mtls.intercept.install.success` is the sole
timing authority for the zero-frame-before-intercept assertion. The action shim
emits it synchronously immediately after awaited
`mtls_lifecycle.start_alloc(&spec)` succeeds—therefore after allocation
elements are active and read back—then awaits the same guest-network owner's
exact TAP activation/read-back, and only afterward calls
`driver.release_for_exit_emission(handle)` to release guest EXEC.

The native test installs a tracing Layer before deployment. Its synchronous
`on_event` callback accepts exactly one event whose `alloc` field equals the
caller allocation ID and samples `clock_gettime(CLOCK_REALTIME)`. AF_PACKET
`SO_TIMESTAMPNS` is in the same realtime domain. Every guest-originated frame
on the caller TAP with a missing timestamp or timestamp less than or equal to
that barrier fails the zero-frame assertion; only a strictly later timestamp
is post-live. Event absence, duplication, wrong allocation, or capture loss
fails closed.

The event remains correctly named because it reports only that the mTLS
intercept installation and its `2 + P` elements succeeded. It does not report
TAP activation, EXEC release, or allocation completion. Keeping it before the
only host-TAP up transition is load-bearing: any newly enabled frame is then
strictly post-event. Moving the event after activation would create an
unobservable interval in which the first guest frame could precede the event.

The typed generation-bracketed shared-IP state remains mandatory evidence of
the complete constant program and exact `2 + P` member universe. Its userspace
poll-completion timestamp is not an ordering receipt and cannot replace or
move the event barrier; changing a polling interval cannot strengthen that
evidence. This uses the existing event and source order without a product hook,
event-field change, clock injection, or API.

Observed nft rule/set handles remain private kernel-assigned receipts. Tests
compare semantic identity and relative ownership/preservation only; they never
require a literal handle number such as `74`.

### Ownership and order

- The node shared-switch owner owns the program, endpoint/counter maps, bpffs
  hierarchy, managed-TAP nft set, and the three-rule bridge guard.
- Its private host implementation and sibling sim implementation satisfy the
  same application-owner contract for startup probe, stale sweep, shared
  converge/audit, TAP quiescence, and inherited allocation
  provision/activation/teardown.
  This is one owner boundary, not a second classifier or a low-level kernel
  fault abstraction.
- The application owner and its source-bearing orchestration error live in
  control-plane. Dataplane alone converts aya attachment/error types; netlink
  retains its existing canonical error. Core contains none of those adapter
  types.
- Per-allocation network provisioning first adds a down TAP to the managed set,
  then writes its endpoint-map value, attaches/pins TCX, queries the exact
  ifindex/program identity, and returns only after the exact TAP is read back
  still down. Cloud Hypervisor may then attach the persistent TAP but must not
  raise it through READY/Running. After the action shim's intercept-success
  event, the same owner re-reads guard/endpoint/link/pin/master identity, raises
  the TAP, and reads back exact up/master identity before EXEC release. Every
  partial state is fail-closed by the guard or administrative-down barrier.
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

### Require TLS `0x17` bytes from loopback AF_PACKET

Rejected by native falsifier `e72385d6`. The exact live TLS 1.3 kTLS TX/RX
socket was uniquely correlated and loopback capture was lossless in both
directions, yet it exposed no complete TLS record. Retaining the requirement
would reject the accepted real-kTLS mechanism because the chosen observation
layer cannot expose its ciphertext.

### Accept socket state without an interface escape audit

Rejected. `ss` plus same-fd splice proves the protected socket and its data
owner, but does not independently prove the output divert kept leg B off the
shared bridge or a physical uplink. The lossless all-interface capture is
required as a separate positive-loopback/negative-non-loopback oracle and must
confine plaintext to the two named guest-local tuple families.

### Keep provision-time TAP activation and allow pre-intercept control frames

Rejected by the native counterexample and ADR-0088's closed zero-frame
contract. The observed ARP replies and TCP RST were guest-originated before
EXEC; categorizing them as harmless would make the security barrier depend on
packet-type exceptions rather than exact ordering.

### Timestamp intercept-live after TAP activation

Rejected. TAP activation is the first operation that can admit a guest frame to
the bridge, so an event emitted afterward cannot prove that no frame preceded
intercept-live. The existing event correctly timestamps completed mTLS
installation; activation follows it and remains separately awaited/read back.

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
capacity proof. The original S-ND295-01 clarification changed evidence
attribution: exact kTLS socket state plus same-inode bidirectional splice proves
the same-node protected transport; lossless interface capture proves local
diversion, zero physical/ordinary-forwarding leg-B egress, intentional
plaintext confinement to leg F/leg S, and no bypass. Same-node AF_PACKET makes
no TLS-record byte claim. The later native frame-order counterexample changes
one product order: provision ends with TAP down and the same owner activates it
after the unchanged mTLS success receipt and before EXEC release.
