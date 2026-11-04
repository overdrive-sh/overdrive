# ADR-0225 — Use a dedicated Gateway SVID identity slot and lifecycle

## Status

Proposed — pending independent review (2026-11-02). Ratified by the user
2026-09-13. User ruling 2026-11-02 (R-4): disable is a synchronous clear and
there is no drop Action.

## Context

The public client never uses SPIFFE, but the gateway needs an internal client
identity `spiffe://overdrive.local/gateway/<node-id>` for its workload hop.
Existing SVID holders and lifecycle targets are Allocation-shaped, while the
gateway is a node-owned, non-allocation application role. It needs one current
ephemeral SVID, auditable issue/reissue, renewal, restart recovery, clearing
on disable and exactly one authorized gateway-client consumer without changing
Allocation identity meaning. Main's workload-identity CA is persistent and
KEK-sealed ([ADR-0063](adr-0063-built-in-ca-port-trait-and-root-key-protection.md));
the gateway identity is issued from it and is distinct from the ephemeral
operator HTTPS CA.

## Decision

Give each enabled node one volatile in-process **Gateway Identity Slot** and one
node-targeted `GatewaySvidLifecycle` (a sibling desired-versus-actual
reconciler) for `spiffe://overdrive.local/gateway/<node-id>`. Reuse
`ca_issuance::issue_and_audit`, the issued-certificate audit row, the existing
single-URI-SAN SVID profile and workload `SvidLifecycle` retry/currentness
policy, without fabricating an AllocationId. A dedicated single-slot holder and
lifecycle exist because existing holders/actions are AllocationId-keyed.

- Enabled and empty issues; near-expiry reissues; restart reissues from the
  empty volatile slot. Disable or shutdown clears the slot synchronously, only
  after admission stops and users drain: the call that disables bumps the
  epoch and empties the slot together, with no reconciler round-trip, no wait
  and no drop Action. The design has no drop Action because no production
  caller needs one: the only disabling caller is the gateway shutdown stage.
- Audit must persist before hold. A checked desired epoch prevents a stale
  issue completion from re-holding identity after disable.
- Only gateway-client mTLS may consume an opaque use capability; generic
  allocation `IdentityRead`, public TLS, Route, handlers and status cannot read
  private material.
- Wakeup model (`.claude/rules/reconcilers.md`): the reconciler is host-backed.
  Its `actual` is the in-process slot and its own executor authors the
  `IssuedCertificate` rows, so it declares empty `interests()` and a
  `resync_schedule()` (`ResyncScope::LocalNode`) as its level-triggered
  backstop; the period is a DELIVER tuning constant bounded by the SVID
  reissue margin.

## Lifecycle Gate Ownership

**Gateway Identity Current** is owned by the Gateway Identity Slot/Lifecycle.
It gates initial public bind and new gateway-client handshakes; absence or
expiry stops new admission/handshakes while reissue policy runs (an issue
failure is retried, never terminal, and is recorded per ADR-0274). Disable/shutdown stops admission and drains authorized users
before the slot is cleared. Allocation SVIDs, public-key usability, Service/Allocation states,
Backend Eligibility and BPF Hydrated are unaffected.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Fabricate an AllocationId and reuse `IdentityMgr` | It corrupts Allocation identity and lifecycle meaning; viable with current actions/holder but falsely binds the gateway to Allocation Running/stop/audit semantics. |
| Broaden `IdentityMgr`/`SvidLifecycle` to `{Allocation, Gateway}` keys | Viable with one lock/reconciler family, but expands every holder/read/action path and mixes one node-infrastructure slot with per-Running-allocation desired state. |
| Reuse the public Web-PKI origin key | It crosses public and internal trust domains. |
| Run the gateway without an internal identity | The workload hop could not authenticate the gateway through the existing SPIFFE trust domain. |
| A waiting disable: bump the epoch, let the reconciler emit a clearing Action, and wait for the slot to empty | It adds an Action, a settle step and a timeout outcome for a caller (gateway shutdown) that needs the slot empty at once and has no live reconciler to rely on; the issuance audit row is already durable. Rejected by ruling R-4. |

## Consequences

- Allocation SVID ownership, identity APIs and transparent workload mTLS remain
  unchanged. Gateway issue/hold/use/reissue/clear has one exact owner and no
  persisted private key.
- Gateway Identity Current is distinct from audit presence and gates new public
  admission and upstream handshakes without redefining workload states.
- Leg-C accepts the gateway identity as a client without a change to its
  verifier: `tls_config::server_config` requires a client leaf that chains to
  the workload root and checks no SPIFFE-ID allowlist
  (`crates/overdrive-dataplane/src/mtls/tls_config.rs:249-269`). The gateway
  SVID therefore uses the same issuance profile as allocation SVIDs (one URI
  SAN, no extended-key-usage restriction that omits `clientAuth`).
- Exact workload-peer authentication is independently decided in ADR-0234,
  which also records that public TLS and the Gateway SVID are separate trust
  domains.
- Shutdown clears the volatile slot synchronously inside the gateway shutdown
  stage (feature delta, D5 section (c)); there is no reconciler round-trip at
  shutdown because the audit row is already durable. An Issue completion that
  arrives after the clear carries a stale epoch and is a no-op. Disable
  cannot time out and has no timeout outcome.

## Links

- [Gateway SVID lifecycle contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-identity-and-lifecycle-integration)
- [Domain identity aggregate/lifecycle](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-aggregate-contracts)
- [Canonical C4 identity path](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [Route/domain context map](c4-diagrams.md#public-ingress-gateway-route-domain-context)
- [ADR-0234](adr-0234-exact-peer-gateway-client-mtls.md)
- [ADR-0063](adr-0063-built-in-ca-port-trait-and-root-key-protection.md), [ADR-0067](adr-0067-workload-identity-manager-svid-lifecycle.md)
