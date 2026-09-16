# ADR-0123 — Use a dedicated Gateway Identity Slot and lifecycle

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

The gateway needs internal identity
`spiffe://overdrive.local/gateway/<node-id>` but is not an Allocation. It needs
one current ephemeral SVID, auditable issue/reissue, renewal, restart recovery,
drop semantics and exactly one authorized gateway-client consumer without
changing Allocation identity meaning.

## Decision

Extend Workload Identity with one in-process **Gateway Identity Slot** per node
and a sibling desired-versus-actual **Gateway Identity Lifecycle**. Reuse
`ca_issuance::issue_and_audit`, the issued-certificate audit row, existing
single-URI-SAN SVID profile, and workload `SvidLifecycle` retry/currentness
policy. Create a dedicated single-slot holder and lifecycle because existing
holders/actions are AllocationId-keyed.

Enabled+empty issues, near-expiry reissues, restart reissues from the empty
volatile slot, and disable/shutdown drops only after admission stops and users
drain. Audit must persist before hold. Only gateway-client mTLS may consume an
opaque use capability; generic allocation `IdentityRead`, public TLS, Route,
handlers and status cannot read private material.

## Lifecycle Gate Ownership

**Gateway Identity Current** is owned by Gateway Identity Slot/Lifecycle. It
gates initial public bind and new gateway-client handshakes; absence, expiry or
terminal issue failure stops new admission/handshakes while reissue policy runs.
Disable/shutdown stops admission and drains authorized users before drop.
Allocation SVIDs, public-key usability, Service/Allocation states, Backend
Eligibility and BPF Hydrated are unaffected.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Fabricate a synthetic `AllocationId` | Viable with current actions/holder, but falsely binds gateway identity to Allocation Running/stop/audit semantics. |
| Broaden `IdentityMgr`/`SvidLifecycle` to `{Allocation, Gateway}` keys | Viable with one lock/reconciler family, but expands every holder/read/action path and mixes one node-infrastructure slot with per-Running-allocation desired state. |

## Consequences

- Allocation identity APIs and transparent mTLS lifecycle remain unchanged.
- Gateway issue/hold/use/reissue/drop has one exact owner and no persisted
  private key.
- Gateway Identity Current is distinct from audit presence and gates new public
  admission/upstream handshakes without redefining workload states.

## Links

- [Domain identity aggregate/lifecycle](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-aggregate-contracts)
- [Exact Application identity contracts](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-identity-and-lifecycle-integration)
- [Application Gateway SVID lifecycle](adr-0119-dedicated-gateway-svid-identity-lifecycle.md)
- [Application exact-peer mTLS](adr-0136-exact-peer-gateway-client-mtls.md)
- [System trust-boundary decision](adr-0130-separate-public-tls-from-gateway-svid-workload-mtls.md)
- [Route/domain context map](c4-diagrams.md#public-ingress-gateway-route-domain-context)
- ADR-0063 and ADR-0067
