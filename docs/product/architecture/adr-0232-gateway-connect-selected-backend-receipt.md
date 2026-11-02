# ADR-0232 — Correlate each gateway connect with its selected BackendId receipt

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13; registration is re-pinned to the dataplane adapter by D5 (user-approved 2026-11-01, see [feature-delta § Reconstruction: user decisions](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-reconstruction-user-decisions-and-proposals)).

## Context

The registered `cgroup_connect4_service` arm ([ADR-0224](adr-0224-existing-cgroup-bpf-gateway-backend-selection.md))
selects and rewrites a gateway socket below the HTTP runtime. The connector
needs the identity of the backend actually selected for that connect; rereading
current backend rows or inferring from the frontend cannot prove which Maglev
result the socket encountered.

## Decision

Correlate each gateway upstream socket with its receipt by the kernel socket
cookie. The dataplane adapter registers `(SO_COOKIE, ServiceKey)` before
`connect(2)` ([ADR-0243](adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md)),
the hook writes exactly one outcome for that cookie, and the connector consumes
one matching transient receipt afterward. The receipt is exactly
`Selected(BackendId)` or `NoBackend`, is bound to its registered cookie and
`ServiceKey`, and is cleaned on every connector return. It is never a persisted
backend cache or userspace selection input. The intent and receipt maps live as
long as the `EbpfDataplane` instance and are not pinned.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Infer the backend from the connected address | DNAT hides the selection authority and address reuse can identify the wrong workload instance. |
| Reread backend observations after connect | The observed set may change and cannot identify the BPF choice for this socket. |
| Persist receipts for later lookup | Receipts are per-connect correlation facts; persistence adds stale identity risk without recovery value. |
| Socket-local storage keyed by the socket instead of cookie-keyed maps | Would remove the cleanup ledger; not adopted without a new spike (open proposal P-3 in the feature delta). |

## Consequences

- The connector can distinguish canonical `NoBackend` from a selected backend
  without enumerating candidates.
- Cookie intent and receipt cleanup become part of connection ownership, with a
  residue sweep at boot.
- Backend identity publication is independently governed by ADR-0233.

## Links

- [Receipt contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Backend programming and selection](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-backend-programming-and-selection-on-main-d1-d3)
- [Canonical C4 receipt path](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0224](adr-0224-existing-cgroup-bpf-gateway-backend-selection.md), [ADR-0233](adr-0233-commit-gated-backend-identity-publication.md), [ADR-0228](adr-0228-dataplane-selection-receipt-ownership.md), [ADR-0229](adr-0229-keep-gateway-and-service-dataplane-generations-independent.md), [ADR-0243](adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md)
