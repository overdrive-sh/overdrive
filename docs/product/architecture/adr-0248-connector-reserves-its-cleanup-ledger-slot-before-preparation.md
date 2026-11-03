# ADR-0248 — The connector reserves its cleanup-ledger slot before it asks for a prepared socket

## Status

Proposed — pending independent review. User ruling DG-11 (R-5, 2026-11-02).

## Context

[ADR-0243](adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md)
makes socket preparation one atomic dataplane operation whose result is a socket
with a registered intent. The connector keeps a bounded cleanup ledger so that
every registered intent can be cleaned up at shutdown or after a failure. If the
connector reserved the slot after preparation returned, a full ledger would leave
a registered intent with no slot for its cleanup.

## Decision

The connector's bounded cleanup ledger reserves one slot before it asks for a
prepared socket and binds the returned registered intent to that slot afterwards.
A full ledger refuses before any socket exists or any intent is registered; a slot
reserved for a preparation that fails is released; and no registered intent is
ever without a ledger slot for its cleanup.

The order is a rule of one sequential connector task. The seeded `overdrive-sim`
invariants listed in the feature delta cover it; no concurrent protocol is
modelled for it, so it has no Quint hazard.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Split preparation so the connector reserves its slot between the cookie read and the registration | Reintroduces a socket that exists with a cookie but no registration, and makes the port two operations again. Rejected by ruling DG-11 (R-5). |
| Reserve the slot after preparation and release the intent when the ledger is full | A full ledger would then perform a registration and an immediate cleanup for a request that is refused anyway; the refusal belongs before any kernel state exists. |

## Consequences

- A full ledger is refused with no kernel-side effect.
- The connector holds a slot for the duration of one preparation, including a
  preparation that fails.

## Links

- [Gateway upstream path into leg-C](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-upstream-path-into-leg-c-d3-d5-d)
- [ADR-0243](adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md), [ADR-0232](adr-0232-gateway-connect-selected-backend-receipt.md)
