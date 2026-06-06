# Evolution — built-in-ca (GH #28 · roadmap Phase 2.6)

**Finalized**: 2026-06-06 · **Feature SHA**: `2f4eccd4` (`feat(built-in-ca):
X.509 CA root/intermediate/SVID hierarchy (#212)`) · **ADR**:
[ADR-0063](../product/architecture/adr-0063-built-in-ca-port-trait-and-root-key-protection.md)

## Summary

The platform's built-in X.509 Certificate Authority: a persistent three-tier
trust hierarchy that supersedes the Phase-1 ephemeral in-process CA (ADR-0010)
for **workload identity**.

```
Root CA (self-signed, P-256, key envelope-encrypted at rest in the IntentStore)
  └─ per-node Intermediate CA (signed by root, pathLen=0, minted at node bootstrap)
       └─ Workload SVID (leaf, exactly one SPIFFE URI SAN, ~1h validity, node-held key)
```

Delivered as a `Ca` **port trait** in `overdrive-core` with a production host
adapter (`RcgenCa`, real rcgen 0.14.8 / `ring` crypto) and a deterministic sim
adapter (`SimCa`, fixture-keyed, DST-reproducible). The root key is sealed with
HKDF→AES-256-GCM under a KEK held in the **Linux kernel keyring** (delivered per
boot by systemd-creds); the control plane **refuses to start** rather than
silently re-mint when the envelope cannot be decrypted (Earned-Trust probe).
Every issuance writes an `issued_certificates` ObservationStore audit row.

## Business context

- **Before**: the only CA was ephemeral (ADR-0010) — re-minted on every `serve`
  boot, key in process memory only, two-tier (root → leaf, CN-only). No
  persistent platform-minted root of trust for workload identity.
- **After**: a persistent root survives restarts (key never plaintext at rest),
  a SPIFFE-compliant three-tier hierarchy, and an auditable issuance trail —
  **zero external identity components** (no SPIRE / cert-manager / Vault); the
  capability ships entirely inside the one binary (K4).
- `tls_bootstrap.rs` (`mint_ephemeral_ca`) is **deliberately retained** — it
  serves the distinct control-plane-HTTPS concern, not workload identity (D-CA-5;
  replaced in Phase 5 / #81).

## Outcome KPIs

| KPI | Target | How proven |
|---|---|---|
| K1 (North Star) | 100% of issued SVIDs chain-verify (`openssl verify` exits 0) | `rcgen_full_svid_chain_verifies_root_intermediate_svid` (Lima) |
| K2 (leading) | 100% carry exactly one URI SAN; 0/≥2 rejected | `CertSpec::svid` PBT + leaf-profile inspection |
| K3 (guardrail) | 0 plaintext key bytes in the IntentStore across the lifecycle | `root_key_envelope_contains_no_plaintext_key_bytes` byte-scan |
| K4 (lagging) | 0 external identity components | architecture review — single binary |
| K5 (guardrail) | 100% of CA DST scenarios reproduce bit-identically from a seed | seeded DST (serials via `Entropy`, fixture keys) |

## Slices delivered (roadmap → execution-log, all 13 steps EXECUTED/PASS)

1. **Slice 01 — root CA behind the `Ca` port trait** (01-01..03): `CertSpec::root`
   core profile; `SimCa::root` deterministic; `RcgenCa::root` real self-signed
   P-256 CA, validated by `openssl verify`.
2. **Slice 02 — root key envelope-encrypted at rest** (02-01..03):
   `RootCaKeyRecordV1` rkyv versioned envelope (ADR-0048); HKDF→AES-256-GCM codec
   (`RootKeyAeadCodec`) with tampered-vs-wrong-KEK distinction; `SystemdCredsKeyring`
   KEK + CA-boot Earned-Trust probe + refuse-to-start.
3. **Slice 03 — per-node intermediate CA** (03-01..04): `CertSpec::intermediate`
   pathLen=0; both adapters chain to root; node bootstrap fails loudly on
   intermediate signing failure; pathLen=0 enforced (a further-CA fails `openssl
   verify`, not merely set).
4. **Slice 04 — workload SVID with SPIFFE SAN** (04-01..04): `CertSpec::svid`
   single-URI-SAN leaf profile (PBT); `RcgenCa::issue_svid` completes the
   walking skeleton (full chain verifies, leaf profile via x509-parser);
   host/sim SVID-profile equivalence.
5. **Slice 05 — trust bundle, audit, re-issue** (05-01..03): `IssuedCertificateRow`
   rkyv envelope + golden bytes; `trust_bundle` on both adapters; re-issue
   distinctness; `issue_and_audit` binds issuance to the `issued_certificates`
   audit row (no silent issuance).

## Key decisions (ADR-0063 D1–D9 + DISCUSS D-CA-1..6)

- **D1** — `Ca` is a port trait (core + host + sim); root/intermediate signing
  keys are CA-held, leaf key is node-held.
- **D2 / D4** — root key at rest = rkyv versioned envelope (ADR-0048) in the
  IntentStore; AEAD = HKDF-derived per-use subkey → AES-256-GCM.
- **D3 / Earned Trust** — KEK runtime holder is the Linux kernel keyring;
  delivery at boot is systemd-creds; boot **adopts** the persisted root and
  refuses to start on decrypt failure rather than re-minting (reconciled
  2026-06-06: `adopt_persisted_root`, verify-winner on lost-race adoption).
- **D5** — pure `CertSpec` builder in core; host adapter translates to
  `rcgen::CertificateParams`. Single-URI-SAN cardinality (K2) is enforced **by
  the type** (`SvidRequest { spiffe_id }`), not an adapter runtime guard
  (Option A, ratified 2026-06-06).
- **D6** — audit trail is an **additive** `issued_certificates` ObservationStore
  row (append-only enforced), routed through the ObservationStore port.
- **D7** — serials via the `Entropy` port; key generation via the crypto-backend
  CSPRNG (DST determinism, K5).
- **D9** — workload-SVID leaf private key is **node-held**: `issue_svid` returns
  cert **+ key** (added 2026-06-06; closes the orphaned-key gap).
- **D-CA-3** — certificate rotation is **out of scope** → GH #40 (needs #39).
- **D-CA-4** — **no operator CLI verb** this phase: SVID issuance is an internal
  platform mechanism; the only operator-observable read surface is the
  `issued_certificates` audit row.
- **D-CA-6** — single-node (Phase 2.6): exactly one intermediate; multi-node
  gossip of the audit trail is GH #36.

## Lessons & review findings (the fix-commit trail)

The headline `feat` landed, then a dense review/hardening pass — each commit is
a real defect or contract sharpening, not polish:

- **Fail loud, never silently degrade**: `boot_ca` refuses to start on decrypt
  failure with a cause-distinct, operator-actionable error carrying the real
  redb path (`e3077d5a`); lost-race ephemeral adoption verifies the winner
  rather than trusting it (`ade22762`); node bootstrap fails loudly on
  intermediate signing failure (`e68efd8e`).
- **Persist inputs / survive restart**: re-seed the CA adapter with the
  persisted root on restart (`9c684741`); persist + adopt the node intermediate
  key so the trust bundle survives restart (`b6e8e93c`); seal the payload as
  **PEM, not DER** (`90ef7021`).
- **Append-only audit**: `issued_certificates` writes are append-only enforced
  (`34039440`); the audit window faithfully mirrors the issued-leaf window with
  skew back-off (`f6823635`).
- **Secret hygiene**: private-key `Debug` is redacted; adoption-conflict is a
  typed variant; the sim guards cert/key shape (`b19f6c3a`).
- **Node-held custody**: `issue_svid` returns the node-held leaf key (D9,
  `7ec77639` / `76be4b5f`).
- **Mutation gaps closed**: byte-newtype accessor roundtrip tests
  (`232a3a8f`); `Ca::adopt_persisted_*` defaults marked equivalent mutants
  (`3521bcab`).

## Verification (EDD catalogue) — status at finalize

Four operator/end-to-end expectations were authored at DISTILL for this feature
and graduated into `verification/` (E03, O04, O05, D01). At finalize **all four
remain `pending`** — and this is **by design, not a gap**:

> The built-in CA is library-complete and proven by the gated `integration-tests`
> Rust tiers, but it is **intentionally not wired into the operator binary** this
> phase (D-CA-4: no operator verb; SVID issuance is internal). `overdrive serve`
> still boots the ADR-0010 ephemeral CA; no verb mints/exports SVIDs; `alloc
> status` renders no issued-certificates section. The catalogue is strictly
> black-box (drives the built binary), so there is no operator surface to capture
> against yet.

Each expectation was **executed** through the harness at SHA `2f4eccd4` (not
narrated) and self-reported `pending` — O04's `overdrive serve --help` built and
ran in Lima (exit 0) but exposes no CA boot surface; O05's `cluster status`
found no running control plane. The runners also carried a scaffold bug (missing
exec bit) fixed at finalize so they are actually runnable.

The **executed in-tree proof** that the behaviour is real lives in the gated
integration tests (run via Lima, `--features integration-tests`):

| Expectation | KPI | In-tree executed proof |
|---|---|---|
| E03 — full chain verifies under `openssl verify` | K1 | `crates/overdrive-host/tests/integration/rcgen_ca_chain_verify.rs::rcgen_full_svid_chain_verifies_root_intermediate_svid` (+ `ca_equivalence.rs`, `ca_boot_and_audit.rs::issued_chain_anchors_on_persisted_root_after_restart`) |
| O04 — refuse-to-start, cause-distinct, no re-mint | K3 | `crates/overdrive-control-plane/tests/integration/ca_boot_and_audit.rs::{boot_refuses_to_start_on_envelope_decrypt_failure_without_remint, boot_refuses_to_start_when_kek_absent_from_keyring}` |
| O05 — `issued_certificates` audit row, no silent issuance | K1 | `ca_boot_and_audit.rs::{issuance_writes_issued_certificates_row_matching_the_minted_cert, issuance_that_cannot_write_audit_row_surfaces_an_error}` |
| D01 — root key never plaintext at rest | K3 | `crates/overdrive-host/tests/integration/rcgen_ca_root_key_envelope.rs::root_key_envelope_contains_no_plaintext_key_bytes` |

**Unblocking** (tracked): these four become satisfiable when the built-in CA is
composed into the live operator surface — split across two issues:

- **#215** (boot-side) — wire `ca_boot::boot_ca` into `overdrive serve` so the
  persistent root is sealed + persisted + adopted, and the Earned-Trust
  refuse-to-start is observable from the binary → **D01**, **O04**.
- **#35** (consumer-side) — `IdentityMgr` drives SVID issuance on alloc-start so
  a deployed workload's SVID can be `openssl verify`'d and its
  `issued_certificates` row surfaces via `alloc status` → **E03**, **O05**.

That composition is future scope (no SVID consumer exists in Phase 2.6 — sockops
mTLS/kTLS is #26); the expectations are forward-looking design-time `why`, and
the gated tests are the `what, forever` until then.

## Pointers

- ADR: `docs/product/architecture/adr-0063-built-in-ca-port-trait-and-root-key-protection.md`
- Scenarios: `docs/scenarios/built-in-ca/` (migrated test-scenarios + slice specs)
- Feature workspace (preserved as history): `docs/feature/built-in-ca/`
- Verification: `verification/expectations/{E03,O04,O05,D01}-*` (status `pending`, by design)
- EDD composition follow-up: **#215** (boot-side: `boot_ca` → `serve`) + **#35**
  (consumer-side: SVID issuance on alloc-start)
- Feature issue: GH #28 (closed) · headline commit: #212 (`2f4eccd4`)
- Deferrals (existing issues): rotation #40/#39 · multi-node audit gossip #36 ·
  SVID consumer (sockops mTLS/kTLS) #26 · aws-lc-rs/FIPS #204 · operator auth
  Phase 5/#81
