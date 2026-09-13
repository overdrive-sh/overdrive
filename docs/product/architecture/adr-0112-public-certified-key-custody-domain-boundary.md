# ADR-0112 — Keep Public Certified-Key Custody as a distinct domain boundary

## Status

Proposed — 2026-09-13.

## Context

The public origin certificate/private key is production Web-PKI material,
distinct from operator HTTPS and internal SPIFFE. Manual files are the first
acquisition input; GH #57 must later be able to produce the same logical
Certified Key without changing Route or listener consumption. Certificate
rotation must not rewrite public-exposure intent.

## Decision

Create the supporting **Public Certified-Key Custody** bounded context and its
state-based **Public Certified Key** aggregate. It owns one producer-neutral,
protected current generation; complete hostname/profile/key validation;
expected-generation preserve-old replacement; validity/expiry currentness;
redaction; and opaque resolver grants. Reuse existing KEK/typed-codec and rustls
resolver patterns, but keep a distinct Web-PKI record, key domain and owner.

Manual paths and plaintext key bytes stop at ingestion. The Route stores only
`PublicCertifiedKeyId`. Manual is the sole first-slice producer; ACME/TCP/80
remain outside this feature and GH #57 may later call the same custody command.
There is no first-slice operator/public certified-key withdrawal command or
custody `withdraw` method. Route deletion withdraws Route intent only and never
deletes protected certified-key custody. GH #57 likewise installs/replaces; it
does not gain implied withdrawal.

## Lifecycle Gate Ownership

**Certified Key Usable** is owned only by Public Certified-Key Custody. It
gates application of a generation and new public TLS admission; absent,
invalid, not-yet-valid or expired material has a cause and no fallback CA or
plaintext. Route acceptance, Service/Allocation state, Backend Eligibility,
BPF Hydrated and Gateway Identity Current are unaffected.

A missing, invalid, stale or currently unusable manual refresh changes only the
redacted install-failure status and never erases a still-usable current
generation. The current generation remains active until a valid replacement or
until live-clock policy makes it Unusable/expired. At that point custody retains
the protected record for a later valid replacement while application/listener
status follows the active fail-closed contract.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Store protected certificate/key material inside Route | Viable for one transactional Route+key update, but makes every rotation a routing mutation and future ACME a Route writer. |
| Generalize allocation `IdentityMgr` for Public Certified Keys | Viable by adding another material/key variant, but conflates ephemeral AllocationId-keyed SPIFFE state with restart-recoverable hostname-bound Web-PKI custody. |

## Consequences

- Public Web-PKI, operator HTTPS and internal SPIFFE remain non-substitutable.
- Manual and later workflow producers share one consumption boundary.
- Neither Route deletion nor failed refresh deletes the protected generation;
  only valid replacement changes it in this slice.
- One additional supporting context/aggregate is justified; no second TLS
  resolver or certificate store is introduced.

## Links

- [Domain aggregate contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-aggregate-contracts)
- [Exact Application custody contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-persistence-custody-and-redaction)
- [Application custody decision](adr-0107-public-certified-key-custody-and-preserve-old-replacement.md)
- [System acquisition/consumption decision](adr-0117-separate-public-certificate-acquisition-from-runtime-consumption.md)
- [Route/domain context map](c4-diagrams.md#public-ingress-gateway-route-domain-context)
- [GitHub #57](https://github.com/overdrive-sh/overdrive/issues/57)
