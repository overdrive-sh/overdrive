# ADR-0112 — Keep Public Certified-Key Custody as a distinct domain boundary

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

The public origin certificate/private key is production Web-PKI material,
distinct from operator HTTPS and internal SPIFFE. Manual files are the first
acquisition input; GH #57 must later be able to produce the same logical
Certified Key without making Route or an allocation-identity holder own public
certificate material.

## Decision

Create the supporting **Public Certified-Key Custody** bounded context and its
**Public Certified Key** aggregate. This boundary owns the public
Web-PKI Certified Key as one concept and is referenced from Route only by
`PublicCertifiedKeyId`. Reuse existing protection/holder patterns without
placing the aggregate in Route, operator HTTPS, internal SPIFFE or allocation
`IdentityMgr`.

Validation profile, protected representation, replacement transaction,
validity policy, redaction, resolver access, producer calls and the exact
absence/presence of mutations are independently owned by the linked System/
Application decisions and feature contract; this Domain ADR does not restate
them.

## Lifecycle Gate Ownership

Not applicable: this ADR chooses a bounded-context/aggregate owner and does not
add or move a lifecycle gate. Certified Key Usable policy and its effects are
owned by the linked feature contract and decision-specific Application ADRs.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Store Public Certificate/key material inside Route | Viable for one transactional aggregate, but makes credential ownership and future producers part of public-exposure intent. |
| Generalize allocation `IdentityMgr` for Public Certified Keys | Viable with another material/key variant, but conflates AllocationId-keyed SPIFFE identity with hostname-bound public Web-PKI. |

## Consequences

- Public Web-PKI, operator HTTPS and internal SPIFFE remain non-substitutable.
- Route and allocation identity remain non-owners of Public Certified Key.
- One additional supporting context/aggregate is justified; its implementation
  lifecycle stays independently reversible.

## Links

- [Domain aggregate contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-aggregate-contracts)
- [Exact Application custody contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-persistence-custody-and-redaction)
- [Application custody ownership](adr-0107-producer-neutral-public-certified-key-custody.md)
- [Application replacement transaction](adr-0122-preserve-last-usable-public-certified-key-replacement.md)
- [System acquisition/consumption decision](adr-0117-separate-public-certificate-acquisition-from-runtime-consumption.md)
- [Route/domain context map](c4-diagrams.md#public-ingress-gateway-route-domain-context)
- [GitHub #57](https://github.com/overdrive-sh/overdrive/issues/57)
