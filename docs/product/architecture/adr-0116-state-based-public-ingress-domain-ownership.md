# ADR-0116 — Use state-based aggregates, reconcilers and derived views for public ingress

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

Public ingress asks current-state questions: which Route/key/application/SVID is
current, which frontend is demanded, and what one BPF connect selected. Existing
IntentStore, protected custody, ObservationStore audit/status and reconciler
View memory already separate desired, actual, occurrence and retry state. No
temporal business query requires replaying full gateway history.

## Decision

Use state-based Public Route Set, Public Certified Key and Gateway Identity Slot
aggregates; dedicated reconcilers/owners for identity, application and BPF
convergence; and derived in-memory frontend-demand, applied-identity and
per-connect receipt views. Persist authoritative inputs and bounded issuance/
status facts only. Persist retry inputs, not derived deadlines. Do not add Event
Sourcing, a CQRS command bus, an event log or another datastore.

Transient demand, socket receipts and BackendId identity associations die at
restart and are rebuilt only where their owners can do so honestly. Issuance
audit is durable occurrence evidence but never substitutes for held/current
Gateway SVID state.

## Lifecycle Gate Ownership

Not applicable: this decision places state/history and introduces no new gate
or state transition. Gate owners and failure scenarios remain those recorded by
the decision-specific ADRs and feature delta.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Event-source Route, key, identity, application and connect lifecycle | Viable for complete temporal audit, but requires secret handling, receipt retention/GC, cross-owner ordering, upcasting/compaction and cannot restore a live socket from events. |
| CQRS command bus plus separate Route/key/identity/application projections | Viable for independently scaled reads, but adds projection lag, stale/unavailable states and another source for a one-node/one-Route workload. |

## Consequences

- Currentness remains a direct owner claim rather than a replay/projection
  inference.
- Historical detail is intentionally bounded to existing audit/status facts.
- The selected model reuses existing state layers and introduces no new
  persistence subsystem or deployment unit.

## Links

- [Domain state/event model](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-event-model-and-lifecycle-gates)
- [Domain reuse and ES/CQRS assessment](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-domain-reuse-and-escqrs-assessment)
- [Exact Application persistence contracts](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-persistence-custody-and-redaction)
- [Operator status decision](adr-0111-redacted-operator-gateway-status.md)
- [System consistency decision](adr-0119-keep-gateway-and-service-dataplane-generations-independent.md)
- [System listener-gate decision](adr-0121-gate-public-listener-without-redefining-workload-service-readiness.md)
- [Route/domain context map](c4-diagrams.md#public-ingress-gateway-route-domain-context)
