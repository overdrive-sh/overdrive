# Slice 03 — Per-node intermediate CA, pathLen-constrained, signed by root

**Job**: J-SEC-001 | **Feature**: built-in-ca (GH #28) | **Story**: US-CA-03
**Walking skeleton**: part of the skeleton (the middle tier)

## Goal (one sentence)

Mint a Node Intermediate CA — signed by the Root CA, with
`basicConstraints` pathLen=0 so it can issue leaf SVIDs but cannot mint
further intermediates — and prove it chain-verifies back to the root.

## IN scope

- `Ca` trait gains `issue_intermediate(node_id) -> IntermediateCaMaterial`
  (signed by the held root key). Behaviour pinned in the trait docstring.
- Host adapter: intermediate params
  `IsCa::Ca(BasicConstraints::Constrained(0))` (research Finding 4),
  `KeyUsagePurpose::{KeyCertSign, CrlSign, DigitalSignature}` critical, signed
  by the root via `signed_by(&key, &root_cert, &root_key)`.
- Single-node Phase 2.6: the one co-located node gets **exactly one**
  intermediate (see feature-delta § multi-node prerequisite analysis — no
  node-registration verb exists in this phase; the node is implicit).
- Acceptance test: `openssl verify -CAfile <root> <intermediate>` accepts the
  intermediate; an attempt to use the intermediate to sign a *further CA* is
  rejected by chain validation (pathLen=0 enforced).

## OUT scope

- Workload SVID (leaf) → Slice 04.
- Intermediate *rotation* / re-signing on a schedule (research Finding 6, Gap
  4: 24h intermediate TTL with renewal at 50%) → rotation workflow, GH #40.
  This slice issues the intermediate once at bootstrap; the scheduled
  re-sign is #40.
- URI name constraints on the intermediate (research Finding 2/4 — root MAY
  constrain intermediates to the trust domain) → defer unless trivial; note
  as a hardening follow-up, not a forward-pointer with a fake issue.
- Multi-node: per-node intermediates, node attestation at bootstrap
  (research Finding 5/13) → owned by **#36 [2.14]** node enrollment /
  admission handler (already `Depends on #28`).

## Learning hypothesis

- **Disproves if it fails**: "the platform can issue a correctly-constrained
  intermediate that chains to the root, bounding node-compromise blast radius
  by construction (pathLen=0)." If the constraint or the chain does not hold,
  the defense-in-depth claim (research Finding 4 — a compromised node cannot
  mint further intermediates) is false.
- **Confirms if it succeeds**: the two-tier chain (root → intermediate) is
  real and correctly constrained; the leaf tier can build on it.

## Acceptance criteria

- [ ] Intermediate cert has `CA:TRUE`, `pathLenConstraint=0`, `keyCertSign` set, `keyUsage` critical.
- [ ] `openssl verify -CAfile <root.pem> <intermediate.pem>` succeeds. (Production-data AC.)
- [ ] A constructed chain where the intermediate signs a *further* CA fails verification (pathLen=0 is enforced, not merely set).
- [ ] Intermediate signing failure (root key unavailable / rcgen error) surfaces a typed error; node bootstrap fails loudly rather than running workloads it cannot issue identities for.
- [ ] DST: intermediate issuance against the (fixture-rooted) sim adapter is deterministic at a seed.

## Dependencies

- Slice 02 (persistent root key the adapter can sign with).

## Effort estimate

~1 day (≤6h). Reference class: `signed_by` is already used in
`tls_bootstrap.rs` for the server/client leaves; this is the same call with
CA params.

## Pre-slice SPIKE

Not needed.

## Taste-test note

Single new behaviour on an existing trait + adapter pair. Production data
(real chain verification). Disproves a real pre-commitment (bounded blast
radius). Distinct from Slice 04 (leaf, CA:FALSE, URI SAN) — not a scale-clone.
