# Slice 01 — Root CA generation behind the `Ca` port trait

**Job**: J-SEC-001 | **Feature**: built-in-ca (GH #28) | **Story**: US-CA-01
**Walking skeleton**: seed (first vertical cut)

## Goal (one sentence)

Generate a self-signed P-256 Root CA through a new `Ca` port trait (core
trait + host adapter over rcgen/aws-lc-rs + sim adapter), producing a root
certificate that verifies as a valid self-signed CA with a standard tool.

## IN scope

- `Ca` port trait in `overdrive-core` (core class, no I/O), minimal surface:
  `generate_root() -> RootCaMaterial` (PEM/DER cert + opaque in-memory key
  handle). Trait docstring pins observable behaviour per `.claude/rules/
  development.md` § "Trait definitions specify behavior, not just signature".
- Host adapter (`RcgenCa` or equivalent, `adapter-host`) over rcgen 0.14 with
  the `aws_lc_rs` feature (ADR-0039 alignment; research Finding 3): root
  params `IsCa::Ca(BasicConstraints::Unconstrained)`,
  `KeyUsagePurpose::{KeyCertSign, CrlSign, DigitalSignature}`, P-256.
- Sim adapter (`SimCa`, `adapter-sim`) that loads a pre-generated fixture
  root key from PEM (research Finding 11 — key generation is non-injectable;
  DST uses fixture keys). Returns deterministic material under a seed.
- Acceptance test (host adapter): the generated root cert chain-verifies as a
  self-signed CA via `openssl verify` (or rustls/webpki against itself).

## OUT scope

- Root key persistence / encryption at rest → Slice 02.
- Any intermediate or leaf cert → Slices 03/04.
- SPIFFE URI SAN on the root (the root SHOULD have no path component —
  research Finding 2; trust-domain-only SAN handling lands when needed).
- Operator CLI verb (there is none in this phase).

## Learning hypothesis

- **Disproves if it fails**: "rcgen + aws-lc-rs can mint a SPIFFE-hierarchy
  root behind our `Ca` port trait, with a sim adapter that keeps DST
  deterministic." If the host/sim equivalence cannot hold, the whole
  port-trait approach for the CA is wrong and must be reconsidered before any
  tier is built.
- **Confirms if it succeeds**: the crypto stack and the port-trait seam are
  sound; every subsequent tier is additive on this surface.

## Acceptance criteria

- [ ] `Ca::generate_root()` exists in `overdrive-core` with a behaviour-pinning docstring; core class compiles clean under dst-lint (no banned APIs).
- [ ] Host adapter produces a root cert with `CA:TRUE`, `keyCertSign` set, and `keyUsage` marked critical.
- [ ] `openssl verify` (or webpki) accepts the root as a valid self-signed CA. (Production-data AC — real cert, not a mock.)
- [ ] Sim adapter returns bit-identical material across two runs at the same seed (DST determinism).
- [ ] `cargo xtask lima run -- cargo nextest run -p <crate> --features integration-tests` green for the new acceptance test.

## Dependencies

- None outside the workspace. rcgen + rustls + aws-lc-rs already declared
  (brief § 10; ADR-0039). Confirm `rcgen` carries the `aws_lc_rs` feature
  (research Gap 3) during implementation.

## Effort estimate

~1 day (≤6h crafter). Reference class: the existing `tls_bootstrap.rs`
`mint_ephemeral_ca` already exercises these exact rcgen APIs — this slice
re-shapes that proven code behind a port trait, it does not discover new
crypto.

## Pre-slice SPIKE

Not needed — the research (Findings 1–4) + existing `tls_bootstrap.rs` remove
the uncertainty. One open confirm: the `rcgen` aws_lc_rs feature flag
(research Gap 3) — verify at first compile, not as a separate spike.

## Taste-test note

Ships 3 components (trait + 2 adapters). This is at the "thin" boundary but
justified: the project's port-trait discipline (`.claude/rules/development.md`
§ "Port-trait dependencies") requires BOTH a host and a sim adapter for any
core trait, or DST cannot exercise it. The trait carries real behaviour
(root generation), so this is NOT the "ship an empty abstraction first"
anti-pattern.
