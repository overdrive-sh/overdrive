# ADR-0114 — Keep selection receipt and applied backend identity in Service Dataplane

## Status

Proposed — 2026-09-13.

## Context

The gateway resolves only an exact Service Frontend. BPF must remain the sole
backend selector, and gateway-client mTLS must authenticate the workload that
BPF actually selected. Address-based rereads can race selection, while moving
backend rows/candidates into the gateway creates a second load balancer.

## Decision

Extend the existing **Service Dataplane** context to own transient Gateway
Connect Intent `(Socket Cookie, ServiceKey)`, cgroup-BPF Selection Receipt
`Selected(BackendId) | NoBackend`, and a process-lifetime immutable Applied
Backend Identity Association `BackendId → exact applied Backend.alloc`.

ServiceMapHydrator remains the sole `ServiceBackendRow` consumer and uses
demand-gated ADR-0053 TEACH for demanded Path-A frontends. The existing
ancestor-attached `cgroup_connect4_service` selects through the shared Maglev/
BACKEND maps. The association is installed from the exact applied Backend value
at the Dataplane commit boundary; old IDs are not rebound during the process.
The gateway connector owns the socket but never backend candidates or raw maps.
Existing XDP wire forwarding is unchanged.

## Lifecycle Gate Ownership

**Gateway Connect Path Trusted** is a Dataplane-owned boot gate proving the
registered Selected/NoBackend receipt and committed identity paths before
public bind. Per request, `Selected` may proceed only after exact identity
resolution; `NoBackend` yields 503, while missing/mismatched receipt or identity
fails closed as 502. Neither gate changes Allocation Running, Service Stable,
Backend Eligibility, BPF ownership, unregistered cgroup behavior or XDP.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Gateway reads `ServiceBackendRow` and selects in userspace | Viable as an ordinary proxy pool, but duplicates BPF selection and cannot prove the chosen peer matches the BPF path. |
| Resolve selected identity by backend address through `BackendIndex` | Viable as an asynchronous lookup, but can observe a different generation from the already-selected socket. |
| Create a gateway netns/veth solely to enter XDP | Viable literal XDP ingress, but duplicates topology/lifecycle although the existing cgroup hook already covers `serve`. |

## Consequences

- Selection and selected-identity publication stay with one Dataplane owner.
- Transient intent/receipt cleanup and conservative identity retention add
  bounded process-local state but no new durable store.
- Unregistered cgroup behavior and XDP wire forwarding remain unchanged; the
  gateway never becomes a backend-row consumer.

## Links

- [Domain selection/identity contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-event-model-and-lifecycle-gates)
- [Exact Application Dataplane contracts](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Application Dataplane decision](adr-0108-demand-gated-cgroup-bpf-selection-and-committed-backend-identity.md)
- [System packet-entry decision](adr-0118-use-existing-cgroup-bpf-service-dataplane-for-gateway-upstream.md)
- [System consistency decision](adr-0119-keep-gateway-and-service-dataplane-generations-independent.md)
- [Route/domain context map](c4-diagrams.md#public-ingress-gateway-route-domain-context)
- ADR-0040, ADR-0042 and ADR-0053
