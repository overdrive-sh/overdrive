# ADR-0115 — Use a singleton Public Route Set aggregate

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN. The filename is retained for link compatibility after
the original domain-model bucket was split under `.claude/rules/design.md`.

## Context

The first public-ingress slice permits at most one Route. Public exposure must
remain independent from Service/Allocation lifecycle, while competing Route
IDs must not both claim the singleton slot. Route intent must reference one
stable Service Listener rather than persist a derived frontend or backend.

## Decision

Model one state-based **Public Route Set** aggregate with a fixed gateway-wide
identity and an optional top-level Route entity. `RouteId` identifies that
entity. One aggregate write owns empty/occupied cardinality, same-ID
replacement, different-ID conflict and withdrawal.

A Route contains one canonical Public Hostname, raw exact-or-segment-prefix
Path Match, exact `(WorkloadId, port, tcp)` Service Listener Reference and one
Public Certified-Key Reference. Resolution yields the current exact Service
Frontend; Route persists no `ServiceId`, VIP, backend, `BackendId`, Allocation
ID or SPIFFE identity.

## Lifecycle Gate Ownership

**Route References Resolved** is owned by Public Ingress and affects only
whether this Route generation may enter Gateway Application staging. Missing,
wrong-kind or unresolved target/key reference prevents application/listener
admission with a cause; it does not gate Service, Allocation, Backend
Eligibility or BPF state. The complete failure/late-result scenarios live in
the feature delta's Domain event/lifecycle section.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Independent per-Route aggregates plus a global hostname/slot claim | Viable for multi-Route concurrency, but requires coordination or compensation across two consistency boundaries and recreates the singleton with another failure mode. |
| Embed PublicExposure in the Service aggregate | Viable for transactional listener/exposure changes, but couples public host/path/key withdrawal to workload and Backend Eligibility lifecycle. |

## Consequences

- One conditional aggregate write enforces first-slice cardinality and conflict.
- Public exposure can be replaced/withdrawn without mutating Service intent.
- Multi-Route cardinality requires a later explicit decision rather than dormant
  global-index machinery now.

## Links

- [Domain aggregate and lifecycle contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-aggregate-contracts)
- [Exact Application Route/owner contracts](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Application ownership decision](adr-0116-public-ingress-route-and-gateway-application-owner.md)
- [Route/domain architecture summary](brief.md#public-ingress-gateway-route-domain-model)
- [Route/domain context map](c4-diagrams.md#public-ingress-gateway-route-domain-context)
