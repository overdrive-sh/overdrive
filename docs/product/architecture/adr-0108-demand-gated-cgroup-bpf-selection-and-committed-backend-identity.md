# ADR-0108 — Keep gateway backend selection and identity publication in the existing Dataplane owner

## Status

Proposed — 2026-09-13.

## Context

The gateway resolves only a Service Frontend. Existing BPF owns Service backend
selection, but current Path-A backends are gated from `SERVICE_MAP` and the
host-originated gateway socket needs the identity of the backend BPF actually
selected. Publishing an identity independently of fallible multi-map updates
would permit an identity to appear applied when no committed Service-map
generation can select it.

## Decision

Extend the existing ServiceMapHydrator/EbpfDataplane path with exact live
frontend demand and a registered `(SO_COOKIE, ServiceKey)` cgroup-BPF arm that
returns `Selected(BackendId)` or `NoBackend`. Serialize each affected
Service-map application behind one Dataplane commit guard: validate, reserve
the not-yet-readable identity association, stage all fallible BACKEND/inner/
reverse-NAT writes, make the atomic outer
`SERVICE_MAP` pointer swap the commit point, and expose the corresponding
BackendId-to-`Backend.alloc` identity only after that guarded commit is
complete. Pre-commit failure restores the captured prior state.

This is one decision about retaining selection plus selected-identity commit
authority in the existing Dataplane owner. It does not decide public Route,
gateway SVID or HTTP policy.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Watch `ServiceBackendRow` and round-robin in userspace | It creates a second selector that can disagree with BPF and cannot authenticate the backend BPF actually chose. |
| Create a gateway netns/veth solely to enter XDP | It duplicates network lifecycle even though the existing ancestor cgroup hook already covers `serve`. |
| Publish BackendId identity before fallible map application without a commit guard | Failure can leave an identity labelled applied but unselectable and poison later allocator use. |

## Consequences

- Gateway request code never enumerates backends or mutates raw BPF maps.
- The existing XDP wire path and unregistered cgroup behavior remain unchanged.
- The Dataplane owner gains a small transient cookie registry, a committed
  identity association and rollback bookkeeping under one serialized commit.
- Multi-map updates are not claimed kernel-atomic; the guard and rollback make
  userspace publication atomic at the only observable selected-identity seam.

## Links

- [Exact demand, receipt and transaction contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Dataplane component boundary](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-application-component-decomposition)
- [Canonical C4 selection path](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0114](adr-0114-dataplane-selection-receipt-and-applied-backend-identity.md)
- ADR-0040, ADR-0042 and ADR-0053
- [System packet-entry ADR-0118](adr-0118-use-existing-cgroup-bpf-service-dataplane-for-gateway-upstream.md)
- [System consistency ADR-0119](adr-0119-keep-gateway-and-service-dataplane-generations-independent.md)
