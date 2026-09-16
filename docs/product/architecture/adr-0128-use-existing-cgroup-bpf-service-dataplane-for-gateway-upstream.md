# ADR-0128 — Use the existing cgroup-BPF Service dataplane for gateway upstream

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

After public Route matching, the gateway has one exact Service Frontend but must
not become a second backend load balancer. The host-originated socket is already
inside the ancestor scope of `cgroup_connect4_service`; forcing it through XDP
would require a gateway-only netns/veth packet-entry and reverse-path lifecycle.

## Decision

Send the gateway's host-originated upstream connect to the exact Service
Frontend through the existing ancestor-attached `cgroup_connect4_service` and
existing Service-map Maglev/`BACKEND_MAP` data. Demand-gated ADR-0053 TEACH makes
the selected frontend's Path-A backends eligible. BPF selects and rewrites the
destination; the gateway never reads `ServiceBackendRow`, enumerates candidates,
selects a backend or mutates raw maps. Existing XDP wire ingress stays unchanged.

This ADR decides only the System packet-entry topology. ADR-0118 owns
Application backend selection; ADR-0133 owns demand-gated teaching; ADR-0134
owns the transient receipt; ADR-0135 owns committed identity publication.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Gateway-only netns/veth into XDP | Adds interfaces, routes and reverse-path lifecycle solely to reach an ingress hook when the existing cgroup hook already covers `serve`. |
| Userspace backend selection | Duplicates BPF load balancing and can authenticate a different backend from the one carrying the request. |
| External proxy forwarding | Creates another deployment and forwarding owner, contrary to the embedded gateway boundary. |

## Consequences

- ServiceLifecycle remains the sole Backend Eligibility publisher and
  ServiceMapHydrator/Dataplane remain the only backend-application owners.
- Unregistered cgroup connects and the XDP wire path preserve their existing
  behavior.
- The cgroup program grows and remains subject to the repository's real-kernel
  verifier and performance gates.

## Links

- [ADR-0118](adr-0118-existing-cgroup-bpf-gateway-backend-selection.md)
- [ADR-0133](adr-0133-demand-gated-path-a-service-map-teach.md)
- [ADR-0134](adr-0134-gateway-connect-selected-backend-receipt.md)
- [ADR-0135](adr-0135-commit-gated-backend-identity-publication.md)
- [Receipt ownership ADR-0124](adr-0124-dataplane-selection-receipt-ownership.md)
- [Applied-identity ownership ADR-0143](adr-0143-dataplane-applied-backend-identity-association.md)
- [Exact Application dataplane contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Canonical C4](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- ADR-0040, ADR-0042 and ADR-0053
