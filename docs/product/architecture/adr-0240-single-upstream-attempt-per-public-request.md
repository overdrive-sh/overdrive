# ADR-0240 — Perform one upstream attempt per public request

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13.

## Context

BPF selects one backend for each registered upstream socket. Retrying,
replaying or hedging an HTTP request can duplicate a non-idempotent effect and
would open another socket whose independently selected backend may differ.

## Decision

Perform exactly one upstream connect/mTLS/request attempt for each accepted
public request. Do not retry, replay, hedge or open an alternate-backend
attempt. This includes backend unavailability: a `NoBackend` outcome is
returned to the client as 503 without a wait or second selection
([ADR-0242](adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md)).

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Retry all failed requests | Non-idempotent requests can execute more than once. |
| Retry only methods normally considered idempotent | It still creates a second BPF selection and needs a separate retry/deadline policy absent from this slice. |
| Hedge requests to multiple backends | It duplicates effects and introduces userspace multi-backend coordination. |

## Consequences

- BPF remains the only backend selector for the single socket attempt.
- Failure mapping is deterministic and request bodies are never replayed.
- The 503/502 and pre-header/post-header projections live in the feature
  contract rather than becoming retry decisions here.
- A future retry policy would require its own explicit decision and contract.

## Links

- [Request/error contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Consuming-port availability contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-consuming-port-availability-contract-d2)
- [Canonical C4 request path](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0224](adr-0224-existing-cgroup-bpf-gateway-backend-selection.md), [ADR-0232](adr-0232-gateway-connect-selected-backend-receipt.md), [ADR-0239](adr-0239-stream-public-proxy-bodies-with-backpressure.md), [ADR-0242](adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md)
