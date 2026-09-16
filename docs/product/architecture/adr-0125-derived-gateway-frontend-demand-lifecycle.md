# ADR-0125 — Model Gateway Frontend Demand as derived staged/current/draining state

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

Demand-gated ADR-0053 TEACH must know which exact Service Frontends are live
gateway consumers. A replacement frontend must be taught before admission
switches, while an old frontend must remain taught until connections retaining
its Gateway Application generation drain. Persisting demand would duplicate
Route/application truth.

## Decision

`GatewayApplicationOwner` derives one in-process **Gateway Frontend Demand**
set as the distinct-frontends union across three generation phases:

- **Staged** before the generation may become current;
- **Current** while new connections may retain it; and
- **Draining** after supersession/withdrawal while old connections retain it.

Publish Staged demand before the atomic application swap. Move superseded
Current to Draining, remove superseded Staged immediately, and withdraw a
Draining frontend only after its last retaining connection closes and no other
phase references the same frontend. Demand carries exact Service Frontends
only—never backend rows, candidates, health or BPF generation—and is rebuilt
from authoritative state after restart.

## Lifecycle Gate Ownership

**Gateway Frontend Demand Applied** is owned by ServiceMapHydrator/Dataplane and
allows only the corresponding Staged generation to become Current. Staged
demand must precede admission swap; Draining demand survives until the last
retaining connection closes. Missing/failed application blocks that generation
without changing ServiceLifecycle Backend Eligibility, Route/key state or the
independent BPF-generation meaning.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Demand only from the current Route/frontend | Viable and simple, but withdraws old Path-A state while retained connections still depend on it and cannot teach a new frontend before swap. |
| Persist a separate frontend-demand aggregate | Viable for replay/inspection, but duplicates Route/application truth and creates recovery reconciliation for derived state. |
| Have ServiceMapHydrator infer demand from all Routes | Viable at scale, but makes the hydrator a Route consumer and spreads application generation/drain ownership across contexts. |

## Consequences

- Application generations cannot race TEACH introduction/removal.
- Same-frontend replacements do not prematurely withdraw shared demand.
- One owner tracks connection retention; ServiceMapHydrator consumes only the
  exact derived frontend set and remains backend authority.

## Links

- [Domain demand lifecycle](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-event-model-and-lifecycle-gates)
- [Exact Application owner/demand contracts](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-application-component-decomposition)
- [Application Route/owner decision](adr-0116-public-ingress-route-and-gateway-application-owner.md)
- [Application demand-gated TEACH decision](adr-0133-demand-gated-path-a-service-map-teach.md)
- [System packet-entry decision](adr-0128-use-existing-cgroup-bpf-service-dataplane-for-gateway-upstream.md)
- [System consistency decision](adr-0129-keep-gateway-and-service-dataplane-generations-independent.md)
- [Route/domain context map](c4-diagrams.md#public-ingress-gateway-route-domain-context)
