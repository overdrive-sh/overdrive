# ADR-0114 — Keep the gateway selection receipt in Service Dataplane

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

The gateway resolves an exact Service Frontend but BPF remains the sole backend
selector. The connector needs one trustworthy fact naming the selection outcome
for its own socket without reading backend candidates or inferring selection
from current observations.

## Decision

The existing **Service Dataplane** context owns the transient Gateway Connect
Intent and matching BPF Selection Receipt for one gateway-connector-owned
socket. Public Ingress may submit the intent and observe the receipt, but
neither it nor the connector owns receipt state or backend choice.

The receipt is a per-connect correlation fact, never Route intent, a persisted
backend cache or a userspace selection input. Applied BackendId identity
publication is a separate Domain decision in ADR-0133. Exact cookie/ServiceKey,
outcome and cleanup representation belongs to ADR-0124 and the linked feature
contract.

## Lifecycle Gate Ownership

**Gateway Selection Receipt Observed** is owned by Service Dataplane for one
registered connect. A matching `Selected` outcome may proceed to the separately
owned identity read; `NoBackend` is a cause-distinct result; missing/malformed/
mismatched receipt fails the current connect closed. Allocation Running,
Service Stable, Backend Eligibility, BPF Hydrated, unregistered cgroup behavior
and XDP forwarding are unaffected.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Infer selection from the connected address | Address reuse/rewrite does not provide the authoritative BPF-selected BackendId fact. |
| Reread `ServiceBackendRow` after connect | The current row may differ from the generation this socket encountered and makes the gateway a backend-row consumer. |
| Persist receipts for later lookup | Per-connect facts have no restart value and persistence creates stale correlation state. |

## Consequences

- The gateway can observe `Selected` versus `NoBackend` without selecting.
- Intent/receipt lifetime remains bounded to one connector-owned connect.
- Selected Backend Identity ownership can change independently without changing
  the receipt boundary.

## Links

- [Domain connect event model](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-event-model-and-lifecycle-gates)
- [Exact Application receipt contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Application receipt decision](adr-0124-gateway-connect-selected-backend-receipt.md)
- [Domain applied-identity decision](adr-0133-dataplane-applied-backend-identity-association.md)
- [System packet-entry decision](adr-0118-use-existing-cgroup-bpf-service-dataplane-for-gateway-upstream.md)
- [Route/domain context map](c4-diagrams.md#public-ingress-gateway-route-domain-context)
