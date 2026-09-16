# ADR-0116 — Dial-by-name DNS is served on the shared bridge gateway

## Status

**Accepted — user-approved and approved by system design review iteration 5 on 2026-09-16.**
The approved D-295-6 decision keeps one shared-gateway in-agent userspace DNS
responder and rejects DNS synthesis in eBPF. ADR-0124 owns runtime DNS-loop
recovery/fail-stop.

## Context

ADR-0072's resolver logic is topology-neutral, but its bind fallback and guest
injection depend on one gateway per `NetSlot` and host `/etc/netns` resolver
files. Part A proved a guest on the shared bridge can query one UDP socket on
the bridge gateway without any host per-netns resolver path. Part B left the
production DNS adapter itself unchanged and therefore did not prove its final
constructor shape.

## Decision

Keep the in-agent `DnsResponder`, `NameIndex`, `FrontendAddrAllocator`, DNS wire
codec, `IP_PKTINFO` source pinning, negative-answer contract, and startup
List-seed probe. Re-home the listener to the shared bridge gateway named in the
guest network token.

At startup, bind `0.0.0.0:53` first. On `EADDRINUSE`, fall back to exactly one
socket at the shared gateway, not one socket per allocation. Delete the
`NetSlotAllocator` dependency and all per-gateway rebinding logic. A bind or
List-seed failure continues to refuse startup.

DNS stays in userspace because it is a variable-length protocol and a stateful
semantic boundary, not steady-state application payload forwarding. The owner
must parse questions, preserve transaction IDs, distinguish A/NODATA/NXDOMAIN,
encode SOA-based negative caching, handle malformed messages and EDNS/TCP
evolution, consult the live `NameIndex`, and source-pin replies with
`IP_PKTINFO`. The existing responder and wire codec already own those semantics;
an eBPF reimplementation would duplicate them under verifier constraints and
create two DNS truths. The shared gateway also reduces the socket count to
O(1), removing the topology-based reason to push name handling into the
endpoint program.

Cilium independently keeps the same responsibility split while using TCX for
endpoint programs: TCX is enabled by default for supported endpoint devices
([current config](https://github.com/cilium/cilium/blob/e99150f8d8f403eca51ed82138d4ae20a265c8f3/install/kubernetes/cilium/values.yaml#L742-L749)),
while its `DNSProxy` is a singleton userspace component inside the agent that
owns DNS servers, parsing, compression, endpoint/rule state, and replies
([primary source](https://github.com/cilium/cilium/blob/e99150f8d8f403eca51ed82138d4ae20a265c8f3/pkg/fqdn/dnsproxy/proxy.go#L57-L151)).
Its current configuration exposes one global in-agent DNS proxy port
([config](https://github.com/cilium/cilium/blob/e99150f8d8f403eca51ed82138d4ae20a265c8f3/install/kubernetes/cilium/values.yaml#L4236-L4259)),
and the implementation listens for both UDP and TCP before serving parsed DNS
messages
([listener](https://github.com/cilium/cilium/blob/e99150f8d8f403eca51ed82138d4ae20a265c8f3/pkg/fqdn/dnsproxy/proxy.go#L559-L693),
[request semantics](https://github.com/cilium/cilium/blob/e99150f8d8f403eca51ed82138d4ae20a265c8f3/pkg/fqdn/dnsproxy/proxy.go#L884-L1059)).
This is validating precedent, not a directive to copy Cilium's policy or
forwarding behavior into Overdrive.

## Alternatives considered

### Per-TAP DNS listeners

Rejected. It recreates per-workload socket lifecycle and makes DNS cost scale
with allocations despite one shared L2 gateway.

### In-kernel/eBPF DNS synthesis

Rejected. Variable-length parsing, compression, UDP/TCP handling, semantic
negative answers, cache TTLs, live name/backend state, and source-pinned replies
already have a correct userspace owner. Moving them into BPF would duplicate
the authoritative `NameIndex` and wire contract, add verifier complexity, and
optimize a control-plane query rather than the steady-state payload path.

### External DNS daemon

Rejected. It adds a process and a second backend-index copy. The accepted
in-agent responder already owns the contract.

## Consequences

Positive: DNS socket count becomes O(1), guest configuration is one stable
gateway, and all higher-level naming semantics are reused. Negative: the shared
gateway is a node startup dependency and must remain addressable before guests
reach READY.
