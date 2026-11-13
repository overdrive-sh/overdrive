# ADR-0221 — Use a singleton Public Route Set aggregate

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13.
Wording correction approved by the user 2026-11-06: the cardinality and
conflict mechanism is the single owner task serializing get, decide, put, not
one conditional aggregate write (`IntentStore` has no value compare-and-set).
The decision itself is unchanged.

## Context

The first public-ingress slice permits at most one Route. Public exposure must
remain independent from Service/Allocation lifecycle, while competing Route
IDs must not both claim the singleton slot. Route intent must reference one
stable Service Listener rather than persist a derived frontend or backend.

## Decision

Model one state-based **Public Route Set** aggregate with a fixed gateway-wide
identity and an optional top-level Route entity. `RouteId` identifies that
entity. The single Gateway Application owner task, the sole Route writer,
owns empty/occupied cardinality, same-ID replacement, different-ID conflict
and withdrawal by serializing get, decide, put on that aggregate;
`IntentStore` has no value compare-and-set.

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
Eligibility or BPF state. The failure and late-result scenarios are in the
feature delta's Domain event and lifecycle section.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Independent per-Route aggregates plus a global hostname/slot claim | Viable for multi-Route concurrency, but requires coordination or compensation across two consistency boundaries and recreates the singleton with another failure mode. |
| Embed PublicExposure in the Service aggregate | Viable for transactional listener/exposure changes, but couples public host/path/key withdrawal to workload and Backend Eligibility lifecycle. |
| Event-source the Route (and the other public-ingress aggregates), or add a CQRS command bus with separate projections | The gateway asks only current-state questions (which Route/key/application/SVID is current). Event sourcing needs secret handling, receipt retention/GC, cross-owner ordering and upcasting, and cannot restore a live socket from events; CQRS adds projection lag and stale/unavailable states for a one-node/one-Route workload. State-based aggregates over the existing IntentStore, custody, ObservationStore and reconciler View layers (persist inputs, not derived deadlines — `development.md`) need no new persistence subsystem. The ES/CQRS assessment is in the feature delta. |

## Consequences

- The single owner task, the sole Route writer, enforces first-slice
  cardinality and conflict by serializing get, decide, put; `IntentStore` has
  no value compare-and-set.
- Public exposure can be replaced/withdrawn without mutating Service intent.
- Multi-Route cardinality requires a later explicit decision rather than dormant
  global-index machinery now.
- One Route per node is a limitation of this slice. [GitHub #352](https://github.com/overdrive-sh/overdrive/issues/352)
  ("serve several Routes and hostnames on one listener") tracks lifting it.

## Links

- [Domain aggregate and lifecycle contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-aggregate-contracts)
- [Exact Route/owner contracts](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Application ownership decision](adr-0222-public-ingress-route-and-gateway-application-owner.md)
- [Route/domain architecture summary](brief.md#public-ingress-gateway-route-domain-model)
- [Route/domain context map](c4-diagrams.md#public-ingress-gateway-route-domain-context)
