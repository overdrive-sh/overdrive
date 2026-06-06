# Slice 05 — Trust bundle composition, issued-cert audit row, re-issue on demand

**Job**: J-SEC-001 | **Feature**: built-in-ca (GH #28) | **Story**: US-CA-05
**Release**: Release 1 (first enhancement past the walking skeleton)

## Goal (one sentence)

Compose a trust bundle that an SVID validates against, write an
`issued_certificates` observation row for every issuance (the internal CT
equivalent), and let the platform re-issue a fresh valid SVID on demand
without a control-plane restart.

## IN scope

- `Ca` trait gains `trust_bundle() -> TrustBundle` composing the root (and
  the node intermediate where chain assembly needs it — research Finding 2:
  the bundle anchor is the self-signed root). Behaviour pinned in docstring.
- `issued_certificates` observation row (research Finding 15): `{serial,
  spiffe_id, issuer_serial, not_before, not_after, node_id, issued_at}`,
  written via the `ObservationStore` on each SVID/intermediate issuance.
  State-layer: observation (gossiped), NOT intent — the CA *material* is
  intent, the *audit of what was issued* is observation.
- Re-issue-on-demand: calling `issue_svid` again for the same SpiffeId mints
  a fresh leaf (new serial, new validity window) with no restart — the
  engine's "re-issue" capability that the rotation workflow (#40) will later
  drive on a schedule.
- Acceptance test: a freshly re-issued SVID chain-verifies against the
  composed `trust_bundle()`; the `issued_certificates` row is readable and
  matches the minted cert's serial + SPIFFE ID.

## OUT scope

- The scheduled rotation workflow that *decides when* to re-issue (50%-of-TTL
  renewal trigger, research Finding 6) → GH #40 (depends on workflow
  primitive #39). This slice provides the re-issue *mechanism*; #40 is the
  *policy/driver*.
- Gossip-propagated *revocation* (`revoked_operator_certs`, whitepaper §8) →
  Phase 5. SVID revocation-by-expiry is the model here (1h TTL); no CRL/OCSP.
- Multi-region trust-bundle federation (research Finding 14) → later phase.
- Trust-bundle *distribution* to workloads / the mTLS consumer → separate
  feature.

## Learning hypothesis

- **Disproves if it fails**: "an SVID validates against the platform-composed
  trust bundle, issuance is auditable as observation, and re-issuance works
  without a restart." If the bundle does not anchor the chain or re-issue
  requires a restart, the rotation workflow (#40) has no sound mechanism to
  build on.
- **Confirms if it succeeds**: the CA engine is feature-complete for #28 —
  issue, re-issue, verify-against-bundle, audit — and #40 can layer scheduled
  rotation on top with no engine changes.

## Acceptance criteria

- [ ] `trust_bundle()` returns material such that a Slice-04 SVID verifies against it with a standard tool (no external CA file needed beyond the bundle). (Production-data AC.)
- [ ] Every issuance writes an `issued_certificates` observation row; a test reads it back and asserts serial + spiffe_id + issuer_serial match the minted cert.
- [ ] Re-issuing for an existing SpiffeId yields a fresh cert (distinct serial, new validity window) and the control plane is NOT restarted.
- [ ] No silent issuance: an issuance that cannot write its audit row surfaces an error (issuance + audit are observable together).
- [ ] DST: bundle composition + re-issue + audit-row write are exercised against `SimObservationStore` and asserted deterministic at a seed.

## Dependencies

- Slice 04 (SVID issuance) and Slice 03 (intermediate) — the bundle anchors
  the chain those produce.
- `ObservationStore` + `SimObservationStore` (exist).

## Effort estimate

~1 day (≤6h). Reference class: observation-row writes mirror the existing
`alloc_status` / `node_health` row plumbing (brief § 6); bundle composition is
PEM concatenation + verification wiring.

## Pre-slice SPIKE

Not needed.

## Taste-test note

Three small additive behaviours on existing surfaces (bundle, audit row,
re-issue) that together deliver one outcome (auditable, repeatable,
bundle-verifiable issuance). Production data. Disproves a real pre-commitment
(the mechanism #40 depends on). Distinct from prior slices.
