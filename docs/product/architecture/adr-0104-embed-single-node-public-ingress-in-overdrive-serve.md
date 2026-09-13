# ADR-0104 — Embed the first public ingress gateway in `overdrive serve`

## Status

Proposed — 2026-09-13.

## Context

GitHub issue #54 requires a node-agent-embedded public gateway. The first slice
is one node and one public IPv4 TCP/443 listener. Placement determines the
deployment unit, failure domain and lifecycle owner independently of certificate
acquisition, Service forwarding, trust transition, protocol policy and status.

## Decision

Run the first public ingress gateway inside the existing `overdrive serve`
process on the selected node. The existing serve composition/`ServerHandle`
boundary owns construction, public-listener lifetime and shutdown. The gateway
is neither an Allocation nor a separate daemon or external proxy.

This ADR decides only node placement and the process failure domain.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| External reverse proxy | Adds another deployment, configuration, certificate and shutdown owner and conflicts with issue #54's node-agent-embedded intent. |
| Schedule the gateway as an Allocation | Incorrectly gives node infrastructure workload placement and lifecycle semantics. |
| Separate node-local gateway daemon | Preserves locality but adds a second supervisor and an inter-process failure boundary without first-slice value. |

## Consequences

- Gateway availability shares the selected node and `overdrive serve` process
  failure domain; the single-node slice claims no HA or failover.
- Existing serve composition and shutdown ordering extend instead of creating a
  second supervisor.
- Route/application ownership, certified-key custody, BPF forwarding, internal
  identity, HTTP policy and operator status remain independently reversible
  decisions.

## Links

- [System decisions](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-system-decisions)
- [Canonical C4](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0106](adr-0106-public-ingress-route-and-gateway-application-owner.md)
- [ADR-0117](adr-0117-separate-public-certificate-acquisition-from-runtime-consumption.md)
- [ADR-0118](adr-0118-use-existing-cgroup-bpf-service-dataplane-for-gateway-upstream.md)
- [ADR-0119](adr-0119-keep-gateway-and-service-dataplane-generations-independent.md)
- [ADR-0120](adr-0120-separate-public-tls-from-gateway-svid-workload-mtls.md)
- [ADR-0121](adr-0121-gate-public-listener-without-redefining-workload-service-readiness.md)
