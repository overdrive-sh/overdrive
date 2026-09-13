# ADR-0132 — Perform one upstream attempt per public request

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

BPF selects one backend for each registered upstream socket. Retrying,
replaying or hedging an HTTP request can duplicate a non-idempotent effect and
would open another socket whose independently selected backend may differ.

## Decision

Perform exactly one upstream connect/mTLS/request attempt for each accepted
public request. Do not retry, replay, hedge or open an alternate-backend
attempt.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Retry all failed requests | Non-idempotent requests can execute more than once. |
| Retry only methods normally considered idempotent | It still creates a second BPF selection and needs a separate retry/deadline policy absent from this slice. |
| Hedge requests to multiple backends | It duplicates effects and introduces userspace multi-backend coordination. |

## Consequences

- BPF remains the only backend selector for the single socket attempt.
- Failure mapping is deterministic and request bodies are never replayed.
- The exact existing 503/502 and pre-header/post-header projections remain in
  the feature implementation contract rather than becoming retry decisions
  here.
- A future retry policy would require its own explicit decision and contract.

## Links

- [Exact request/error contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Canonical C4 request path](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [BPF selection ADR-0108](adr-0108-existing-cgroup-bpf-gateway-backend-selection.md)
- [Receipt ADR-0124](adr-0124-gateway-connect-selected-backend-receipt.md)
- [Streaming ADR-0131](adr-0131-stream-public-proxy-bodies-with-backpressure.md)
