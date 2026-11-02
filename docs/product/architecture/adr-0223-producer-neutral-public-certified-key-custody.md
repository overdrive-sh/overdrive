# ADR-0223 — Use producer-neutral Public Certified-Key Custody

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Ratified by the user 2026-09-13.

## Context

Public TLS consumes a Web-PKI certificate chain and matching origin private
key. The public origin certificate/private key is production Web-PKI material,
distinct from operator HTTPS and internal SPIFFE. Manual operator-supplied
files are the first-slice acquisition source, while GitHub #57 may later
produce the same logical credential through a Workflow. Listener and request
code must not become acquisition-specific secret owners, and neither Route nor
an allocation-identity holder may own public certificate material.

## Decision

Create the supporting **Public Certified-Key Custody** bounded context and its
**Public Certified Key** aggregate, owned by one `PublicCertifiedKeyCustody`
owner. It is the producer-neutral boundary between credential acquisition and
runtime consumption for protected public-certified-key generations, and is
referenced from Route only by `PublicCertifiedKeyId`.

- Manual operator input is the only first-slice producer. A later ACME or
  Workflow producer must publish through the same custody boundary and must
  not change listener, resolver, Route or Service-forwarding ownership.
- Filesystem paths and plaintext key material stop before runtime consumers,
  which receive only an opaque current resolver snapshot.
- The protected origin key is sealed with the node KEK-backed
  `PublicCertifiedKeyAeadCodec`, supplied by the composition root
  ([ADR-0219](adr-0219-run-server-receives-its-driven-adapters-from-the-composition-root.md)).
- Existing protection/holder patterns are reused without placing the aggregate
  in Route, operator HTTPS, internal SPIFFE or allocation `IdentityMgr`.

Validation profile, protected representation, validity policy, redaction,
resolver access and the exact mutation set are owned by the feature contract.
Preserve-old replacement ordering is an independent decision (ADR-0231).

## Lifecycle Gate Ownership

Not applicable: this ADR chooses a bounded-context/aggregate owner and the
acquisition/consumption boundary; the custody-usable gate is G-2 in the
feature delta.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Let the listener read PEM paths directly | It couples every connection to manual acquisition and spreads path/key handling into runtime code. |
| Give manual and Workflow producers separate managers | Two credential authorities could disagree about the current generation and force listener redesign. |
| Reuse the operator HTTPS CA or internal SPIFFE CA | Neither credential domain represents a public Web-PKI origin identity. |
| Store Public Certificate/key material inside Route | Viable for one transactional aggregate, but makes credential ownership and future producers part of public-exposure intent. |
| Generalize allocation `IdentityMgr` for Public Certified Keys | Viable with another material/key variant, but conflates AllocationId-keyed SPIFFE identity with hostname-bound public Web-PKI. |
| Require ACME in the first slice | Expands the usable gateway into an unrelated durable acquisition workflow already owned by issue #57. |

## Consequences

- Public credential acquisition and runtime consumption remain independently
  evolvable; manual and future ACME producers converge on one logical runtime
  input.
- Paths and plaintext private-key material do not enter Route, status or
  request state.
- Public Web-PKI, operator HTTPS and internal SPIFFE remain non-substitutable;
  Route and allocation identity remain non-owners of Public Certified Key.
- TCP/80, challenge state and ACME order/renewal are absent from the first
  slice.
- One additional supporting context/aggregate is justified; its implementation
  lifecycle stays independently reversible.

## Links

- [Exact custody contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-persistence-custody-and-redaction)
- [Aggregate contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-aggregate-contracts)
- [Canonical C4 custody boundary](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [Route/domain context map](c4-diagrams.md#public-ingress-gateway-route-domain-context)
- [Replacement transaction ADR-0231](adr-0231-preserve-last-usable-public-certified-key-replacement.md)
- [GitHub #57](https://github.com/overdrive-sh/overdrive/issues/57)
