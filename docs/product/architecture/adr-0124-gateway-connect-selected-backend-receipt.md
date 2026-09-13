# ADR-0124 — Correlate each gateway connect with its selected BackendId receipt

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

The cgroup-BPF Dataplane selects and rewrites a gateway socket below the HTTP
runtime. The connector needs the identity of the backend actually selected for
that connect; rereading current backend rows or inferring from the frontend
cannot prove which Maglev result the socket encountered.

## Decision

Register each gateway upstream socket by `(SO_COOKIE, ServiceKey)` before
connect and consume one matching transient Dataplane receipt afterward. The
receipt is exactly `Selected(BackendId)` or `NoBackend`, is correlated by the
same socket cookie, and is cleaned on every connector return. It is never a
persisted backend cache or userspace selection input.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Infer the backend from the connected address | DNAT hides the selection authority and address reuse can identify the wrong workload instance. |
| Reread backend observations after connect | The observed set may change and cannot identify the BPF choice for this socket. |
| Persist receipts for later lookup | Receipts are per-connect correlation facts; persistence adds stale identity risk without recovery value. |

## Consequences

- The connector can distinguish canonical `NoBackend` from a selected backend
  without enumerating candidates.
- Cookie intent and receipt cleanup become part of connection ownership.
- Backend identity publication is independently governed by ADR-0125.

## Links

- [Exact connect/receipt contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Canonical C4 receipt path](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [BPF selection ADR-0108](adr-0108-existing-cgroup-bpf-gateway-backend-selection.md)
- [Identity publication ADR-0125](adr-0125-commit-gated-backend-identity-publication.md)
- [Domain receipt ADR-0114](adr-0114-dataplane-selection-receipt-ownership.md)
- [System generation ADR-0119](adr-0119-keep-gateway-and-service-dataplane-generations-independent.md)
