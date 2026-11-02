# ADR-0220 — Embed the first public ingress gateway in `overdrive serve`

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13.
Reconstruction decisions D1–D5 approved by the user 2026-11-01 (see [feature-delta § Reconstruction: user decisions](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-reconstruction-user-decisions-and-proposals)).

## Context

GitHub issue #54 requires a node-agent-embedded public gateway. The first slice
is one node and one public IPv4 TCP/443 listener. Placement determines the
deployment unit, failure domain and lifecycle owner independently of certificate
acquisition, Service forwarding, trust transition, protocol policy and status.

`overdrive serve` already owns the node's composition root: `run_server`
receives its driven adapters from the caller ([ADR-0219](adr-0219-run-server-receives-its-driven-adapters-from-the-composition-root.md))
and `ServerHandle` owns the ordered shutdown of every long-lived node task.

## Decision

Run the first public ingress gateway inside the existing `overdrive serve`
process on the selected node. The existing serve composition and `ServerHandle`
boundary own construction, public-listener lifetime and shutdown. The gateway
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
  second supervisor; the exact slot, boot position and shutdown stage are in
  [Composition-root gateway slot, boot and shutdown](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-composition-root-gateway-slot-boot-and-shutdown-d5).
- Route/application ownership, certified-key custody, BPF forwarding, internal
  identity, HTTP policy and operator status remain independently reversible
  decisions.

## Links

- [System decisions](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-system-decisions)
- [Canonical C4](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0219](adr-0219-run-server-receives-its-driven-adapters-from-the-composition-root.md)
- [ADR-0222](adr-0222-public-ingress-route-and-gateway-application-owner.md)
- [ADR-0223](adr-0223-producer-neutral-public-certified-key-custody.md)
- [ADR-0224](adr-0224-existing-cgroup-bpf-gateway-backend-selection.md)
- [ADR-0229](adr-0229-keep-gateway-and-service-dataplane-generations-independent.md)
- [ADR-0230](adr-0230-gate-public-listener-without-redefining-workload-service-readiness.md)
- [ADR-0234](adr-0234-exact-peer-gateway-client-mtls.md)
