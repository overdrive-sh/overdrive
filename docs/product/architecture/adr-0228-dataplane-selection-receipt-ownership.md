# ADR-0228 — Keep the gateway selection receipt in Service Dataplane

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13.
Reconstruction decisions D1–D5 approved by the user 2026-11-01 (D5 moves intent registration into the dataplane adapter; see [feature-delta § Reconstruction: user decisions](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-reconstruction-user-decisions-and-proposals)).

## Context

The gateway resolves an exact Service Frontend but BPF remains the sole backend
selector. The connector needs one trustworthy fact naming the selection outcome
for its own socket without reading backend candidates or inferring selection
from current observations.

## Decision

The existing **Service Dataplane** context owns the transient Gateway Connect
Intent and the matching BPF Selection Receipt for one gateway-connector-owned
socket. The Intent is created only by the dataplane adapter when it prepares the
upstream socket ([ADR-0243](adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md));
Public Ingress observes the receipt through the dataplane port, and neither it
nor the connector owns receipt state or backend choice.

The receipt is a per-connect correlation fact, never Route intent, a persisted
backend cache or a userspace selection input. Applied BackendId identity
publication is a separate decision in ADR-0241. Exact cookie/ServiceKey,
outcome and cleanup representation belongs to ADR-0232 and the linked feature
contract.

## Lifecycle Gate Ownership

**Gateway Selection Receipt Observed** is owned by Service Dataplane for one
registered connect. A matching `Selected` outcome may proceed to the separately
owned identity read; `NoBackend` is a cause-distinct result
([ADR-0242](adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md));
a missing, malformed or mismatched receipt fails the current connect closed.
Allocation Running, Service Stable, Backend Eligibility, BPF Hydrated,
unregistered cgroup behaviour and XDP forwarding are unaffected.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Infer selection from the connected address | Address reuse/rewrite does not provide the authoritative BPF-selected BackendId fact. |
| Reread `ServiceBackendRow` after connect | The current row may differ from the generation this socket encountered and makes the gateway a backend-row consumer. |
| Persist receipts for later lookup | Per-connect facts have no restart value and persistence creates stale correlation state. |

## Consequences

- The gateway can observe `Selected` versus `NoBackend` without selecting.
- Intent/receipt lifetime remains bounded to one connector-owned connect and
  to the `EbpfDataplane` instance (the maps are not pinned).
- Selected Backend Identity ownership can change independently without changing
  the receipt boundary.

## Links

- [Domain connect event model](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-event-model-and-lifecycle-gates)
- [Receipt contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [ADR-0232](adr-0232-gateway-connect-selected-backend-receipt.md), [ADR-0241](adr-0241-dataplane-applied-backend-identity-association.md), [ADR-0224](adr-0224-existing-cgroup-bpf-gateway-backend-selection.md), [ADR-0243](adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md)
- [Route/domain context map](c4-diagrams.md#public-ingress-gateway-route-domain-context)
