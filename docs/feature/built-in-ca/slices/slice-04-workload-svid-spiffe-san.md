# Slice 04 — Workload SVID with single SPIFFE URI SAN, signed by intermediate

**Job**: J-SEC-001 | **Feature**: built-in-ca (GH #28) | **Story**: US-CA-04
**Walking skeleton**: COMPLETION — this closes the full 3-tier chain (D2)

## Goal (one sentence)

Mint a short-lived workload SVID — a leaf certificate carrying exactly one
SPIFFE URI SAN, `CA:FALSE`, `keyUsage=digitalSignature` (critical), and a
CSPRNG serial drawn through the `Entropy` port — signed by the node
intermediate, completing the Root → Intermediate → SVID chain.

## IN scope

- `Ca` trait gains `issue_svid(spiffe_id: &SpiffeId, ttl) -> SvidMaterial`
  (signed by the node intermediate). Behaviour pinned in the docstring,
  including the **single-URI-SAN invariant** (research Finding 2).
- Host adapter: leaf params `IsCa::NoCa`, exactly one
  `SanType::URI(spiffe://overdrive.local/job/<name>/alloc/<id>)`,
  `keyUsage=digitalSignature` (critical, MUST NOT include keyCertSign/cRLSign),
  `extendedKeyUsage` = ServerAuth + ClientAuth, 1h TTL (research Finding
  2/6).
- Serial number via `Entropy::fill(&mut [u8; …])` (research Finding 10/11) —
  64-bit CSPRNG floor; `OsEntropy` in production, `SeededEntropy` under DST,
  so serials are deterministic in DST and cryptographically strong in
  production. Wrap in the existing `CertSerial` newtype.
- Reuse the existing `SpiffeId` newtype (overdrive-core) — it already
  validates the `spiffe://overdrive.local/...` shape and lowercases.
- Acceptance test: the full chain `openssl verify -CAfile <root> -untrusted
  <intermediate> <svid>` succeeds; a leaf with 0 or ≥2 URI SANs is rejected
  by the issuer before it is handed out.

## OUT scope

- SVID *rotation / renewal* (mint-fresh-before-expiry on a schedule) →
  rotation workflow, GH #40 (depends on #39). NOTE: re-issue-on-demand (mint a
  fresh SVID when asked, no restart) lands in Slice 05; the *scheduled*
  renewal that drives it is #40.
- SVID *distribution* to the running workload (vsock for microVM, fs mount
  for exec/wasm — research Gap 1) → out of scope for the CA engine; a
  consumer feature.
- SPIFFE Workload API (Unix-socket gRPC, research Gap 1) → Phase 7+, explicit
  non-goal.
- Trust-bundle composition + audit row → Slice 05.

## Learning hypothesis

- **Disproves if it fails**: "the platform mints a SPIFFE-X.509-SVID-spec-
  compliant leaf that validates through the full Root → Intermediate → SVID
  chain, with DST-deterministic serials." If the leaf is non-compliant (wrong
  SAN cardinality, CA bit, keyUsage) or the chain does not verify, the
  identity foundation the billing/policy/mTLS pillars depend on is broken.
- **Confirms if it succeeds**: the walking skeleton is COMPLETE — every
  workload can be given a forgery-proof, spec-compliant, chain-verifiable
  identity. This is the D2 walking-skeleton definition realised end to end.

## Acceptance criteria

- [ ] SVID has `CA:FALSE`, exactly ONE `URI` SAN equal to the requested SpiffeId, `keyUsage=digitalSignature` marked critical, and NO `keyCertSign`/`cRLSign`.
- [ ] Issuing with a SpiffeId that would yield 0 or ≥2 URI SANs is rejected before any cert is produced (single-URI-SAN invariant enforced by construction).
- [ ] Full chain verifies: `openssl verify -CAfile <root> -untrusted <intermediate> <svid>` exits 0. (Production-data AC — the headline walking-skeleton proof.)
- [ ] Serial is ≥64 bits of CSPRNG output via the `Entropy` port; two DST runs at the same seed produce identical serials; two production mints produce distinct serials.
- [ ] DST: SVID issuance against the fixture-keyed sim adapter is deterministic and the chain-shape invariant is asserted.

## Dependencies

- Slice 03 (node intermediate to sign with).
- `Entropy` port (exists: `overdrive-core/src/traits/entropy.rs`, `fill`).
- `SpiffeId`, `CertSerial` newtypes (exist: `overdrive-core/src/id.rs`).

## Effort estimate

~1 day (≤6h). Reference class: leaf signing mirrors the client-leaf path in
`tls_bootstrap.rs`; the new parts are the URI SAN + the Entropy-sourced
serial + the single-URI-SAN guard.

## Pre-slice SPIKE

Not needed — `SanType::URI` is confirmed in research Finding 1 and the
newtypes/Entropy port already exist.

## Taste-test note

This is the highest-value slice (completes the chain) but NOT the riskiest —
Slice 01 carried the crypto-stack risk. Production data, disproves a real
pre-commitment (SPIFFE-spec compliance + chain validity), distinct from every
other slice (leaf tier, URI SAN, Entropy serial).
