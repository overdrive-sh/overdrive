# ADR-0108 — Keep gateway backend selection in the existing cgroup-BPF Dataplane

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

The gateway resolves one exact Service Frontend but must not enumerate or
choose its backends. The existing ancestor-attached cgroup connect hook and
Service Dataplane already own Maglev selection and destination rewrite for
host-originated sockets.

## Decision

Send registered gateway upstream connects through the existing
`cgroup_connect4_service` Service Dataplane so BPF alone selects and rewrites
the backend through `SERVICE_MAP`, its Maglev inner map and `BACKEND_MAP`.
Gateway request code supplies the exact Service Frontend and never implements
userspace backend selection. XDP wire ingress remains unchanged.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Watch backend rows and round-robin in the gateway | It creates a second selector that can disagree with BPF. |
| Create a gateway netns/veth solely to enter XDP | It adds network lifecycle although the existing cgroup hook already covers the socket. |
| Run an external proxy/load balancer | It adds another deployment and secret boundary while duplicating the existing Dataplane owner. |

## Consequences

- The gateway cannot list backend candidates or mutate raw BPF maps.
- Existing unregistered cgroup traffic and XDP wire forwarding keep their
  current behavior.
- Demand, selection receipt and applied-identity publication remain separate
  decisions in ADR-0123, ADR-0124 and ADR-0125.

## Links

- [Exact Dataplane contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Canonical C4 selection path](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [Demand ADR-0123](adr-0123-demand-gated-path-a-service-map-teach.md)
- [Receipt ADR-0124](adr-0124-gateway-connect-selected-backend-receipt.md)
- [Identity publication ADR-0125](adr-0125-commit-gated-backend-identity-publication.md)
- [System packet-entry ADR-0118](adr-0118-use-existing-cgroup-bpf-service-dataplane-for-gateway-upstream.md)
- ADR-0040, ADR-0042 and ADR-0053
