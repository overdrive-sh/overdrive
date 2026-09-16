# ADR-0133 — Demand-gate Path-A Service-map teaching for public ingress

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

Current local Path-A Service backends populate `LOCAL_BACKEND_MAP`, while a
gateway upstream must traverse the Service-map Maglev path. Teaching every
Path-A frontend unconditionally would change unrelated local traffic and make
gateway demand indistinguishable from Service desired state.

## Decision

Teach a Path-A Service Frontend into the Service-map Dataplane only while the
active `GatewayApplicationOwner` publishes exact live demand for it. Demand is
the distinct frontend union of Staged, Current and Draining generations;
`ServiceMapHydrator` consumes it and acknowledges the exact revision before
promotion. Demand remains derived, in-process state rather than persisted
truth.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Teach every Path-A Service into `SERVICE_MAP` | It changes unrelated local Services and broadens the feature beyond actual gateway consumers. |
| Let the gateway program Service maps directly | It duplicates `ServiceMapHydrator` and raw-map ownership. |
| Persist Gateway Frontend Demand | It creates a second desired-state source beside Route/Gateway Application ownership. |

## Consequences

- A replacement frontend is taught before admission changes, while draining
  generations keep their frontend taught until retained users leave.
- Existing undemanded Path-A behavior remains unchanged.
- Backend selection remains ADR-0118; demand lifecycle vocabulary remains
  ADR-0125.

## Links

- [Exact demand contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Canonical C4 demand path](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [BPF selection ADR-0118](adr-0118-existing-cgroup-bpf-gateway-backend-selection.md)
- [Domain demand ADR-0125](adr-0125-derived-gateway-frontend-demand-lifecycle.md)
- [System packet-entry ADR-0128](adr-0128-use-existing-cgroup-bpf-service-dataplane-for-gateway-upstream.md)
