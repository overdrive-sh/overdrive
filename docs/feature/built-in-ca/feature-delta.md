<!-- markdownlint-disable MD024 -->
# Feature Delta — built-in-ca (GH #28 · roadmap Phase 2.6)

**Wave**: DISCUSS (wave 2 of 6) · **Agent**: Luna (nw-product-owner) · **Density**: `lean` + `ask-intelligent` (DISCUSS hard default)

This is the single narrative artifact for the built-in-ca feature. All DISCUSS
content lives here under `## Wave: DISCUSS / [REF|WHY|HOW] <Section>` headings.
Tier-1 `[REF]` sections are emitted (lean default); no Tier-2 expansions were
auto-rendered — two triggers fired and are reported to the orchestrator rather
than auto-expanded (see § Wave: DISCUSS / [REF] Density & Triggers).

---

## Wave: DISCUSS / [REF] Feature Summary

**What**: The platform's built-in X.509 Certificate Authority — the persistent
trust hierarchy that supersedes the Phase-1 ephemeral in-process CA (ADR-0010).
Three tiers: **Root CA** (self-signed, P-256, key envelope-encrypted at rest in
the IntentStore) → **per-node Intermediate CA** (signed by root, pathLen=0,
issued at node bootstrap) → **Workload SVID** (leaf, single SPIFFE URI SAN, 1h
TTL, signed by the node intermediate). Exposed behind a `Ca` **port trait**
(core trait + rcgen/aws-lc-rs host adapter + sim adapter for DST), matching the
project's `Clock`/`Transport`/`Entropy` pattern.

**Why** (J-SEC-001): so the whitepaper's structural-security promise (design
principle 3 — "every packet carries cryptographic workload identity") rests on
a real, persistent trust hierarchy; so the SPIFFE-identity-on-every-flow-event
billing pillar (vision.md pillar 2) and the FIPS/HSM enterprise tier have a CA
primitive to build on; and so adopting Overdrive does not mean operating a
second identity stack (SPIRE + cert-manager + Vault) beside it.

**Feature type**: Cross-cutting (security primitive spanning `overdrive-core`
[port trait + reuse of `SpiffeId`/`CertSerial`/`Entropy`], a CA host adapter
[rcgen/aws-lc-rs], `overdrive-store-local` [IntentStore for the encrypted root
key], `overdrive-sim` [sim adapter + DST], and `overdrive-control-plane`/node
bootstrap [where root + intermediate are wired in]).

**Evidence base**: `docs/research/security/built-in-ca-rcgen-rustls-comprehensive-research.md`
(18 sources, confidence High). Brownfield: `crates/overdrive-control-plane/src/
tls_bootstrap.rs` (`mint_ephemeral_ca`) already exercises the exact rcgen APIs
this feature needs — proving the crypto stack works in practice.

---

## Wave: DISCUSS / [REF] Persona

- **`sam-platform-security-engineer`** (Sam Okafor) — platform/security engineer
  who builds AND operates Overdrive's identity layer; has run SPIRE + Vault and
  hated it; threat-models by default; verifies chains with `openssl verify`
  rather than trusting the platform's word. SSOT:
  `docs/product/personas/sam-platform-security-engineer.yaml`. New persona —
  prior personas (priya-evaluator, maya-agent-developer) are docs-consumers; the
  platform-internal security archetype was unrepresented.

---

## Wave: DISCUSS / [REF] JTBD One-liner

**J-SEC-001** — *"Give every workload a forgery-proof cryptographic identity the
platform mints itself, with no external PKI to operate."*

> When I run a workload on the platform — and its traffic, billing record, and
> policy decision all hinge on "which workload is this, really" — and I have no
> separate SPIRE/cert-manager/Vault, **I want** the control plane to BE the CA
> (generate + protect a root, issue each node an intermediate at bootstrap, mint
> a short-lived SPIFFE-SAN SVID per workload, all chain-verifiable to the root),
> **so** structural security, SPIFFE-billing, and the FIPS tier have a real
> trust hierarchy — and adoption does not mean operating a second identity stack.

Full job (functional/emotional/social dimensions + four forces) is in the SSOT:
`docs/product/jobs.yaml` § `J-SEC-001`. Distilled from whitepaper §4/§8/§19 per
the jobs.yaml header precedent (whitepaper, not interviews).

---

## Wave: DISCUSS / [REF] Brownfield Evaluation (Walking Skeleton — D2)

**The Phase-1 ephemeral CA (ADR-0010) is the brownfield context, and it is a
*different consumer*, not a competing implementation.**

| Aspect | Phase-1 `tls_bootstrap.rs` (ADR-0010) | This feature (#28) |
|---|---|---|
| Purpose | TLS cert for the **control-plane HTTPS endpoint** (operator-CLI trust over `:7001`) | **Workload identity** hierarchy (SVIDs for allocations) |
| Lifetime | Ephemeral — re-minted every `serve` boot, key in process memory only | **Persistent** — root key envelope-encrypted at rest, reused across restarts |
| Identity format | CN (`overdrive-control-plane`, `local-operator`) | **SPIFFE URI SAN** (`spiffe://overdrive.local/job/.../alloc/...`) |
| Hierarchy | Root → server/client leaf (2 tiers) | Root → **node intermediate** → workload SVID (3 tiers) |
| Key protection | None (memory only, discarded on stop) | **AES-256-GCM envelope encryption** in IntentStore |

**Verdict**: the proven rcgen API usage in `mint_ephemeral_ca` (root self-sign,
`signed_by` for leaves, `SanType`, `KeyUsagePurpose`, P-256) **carries forward
and de-risks the crypto**; the *structure* (persistence, SPIFFE SAN,
intermediate tier, envelope encryption) is net-new. The two coexist —
`tls_bootstrap.rs` keeps serving the control-plane endpoint; #28 builds the
workload-identity hierarchy. This feature does **not** delete or refactor
`tls_bootstrap.rs` (its consumer — operator CLI mTLS — is Phase 5 work).

**Walking skeleton** (D2: "root CA → one node intermediate CA → one workload
SVID, signed and chain-verifiable"): realised across **Slices 01–04**. The
thinnest end-to-end cut that touches all activities — generate root (01) →
protect it so it persists (02) → issue intermediate (03) → issue SVID and
verify the full chain (04). Slice 05 is the first enhancement (bundle + audit +
re-issue).

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate — Phase 1.5)

**Verdict: PASS — right-sized as ONE feature, sliced into 5 thin vertical cuts.**

Oversized-signal check (oversized = any 2+ firing):

| Signal | Threshold | This feature | Fires? |
|---|---|---|---|
| User stories | >10 | 5 (US-CA-01..05) | No |
| Bounded contexts | >3 | 1 (identity/CA) — touches ~4 crates but one context | No |
| WS integration points | >5 | ~4 (root→persist→intermediate→leaf→verify) | No |
| Estimated effort | >2 weeks | ~5 days (5 × ≤1-day slices) | No |
| Independent shippable outcomes | multiple | 1 coherent outcome (a workload identity) | No |

Zero signals fire decisively. The feature is one coherent capability (a
platform-minted workload identity) and is **not** split into multiple features.
It IS sliced thinly (carpaccio) internally — 5 slices, each ≤1 day, each with a
learning hypothesis (see § Story Map). All five carpaccio taste tests pass
(documented per-slice in the slice briefs; Slice 01's 3-component count is at
the "thin" boundary but justified by the project's mandatory host+sim adapter
discipline).

---

## Wave: DISCUSS / [REF] Story Map

**Persona**: Sam (platform/security engineer) · **Goal**: every workload gets a
platform-minted, chain-verifiable cryptographic identity.

### Backbone (user/platform activities, left → right)

| A. Establish a root of trust | B. Delegate signing to a node | C. Identify a workload | D. Verify & audit |
|---|---|---|---|
| Generate root CA (S01) | Issue node intermediate, pathLen=0 (S03) | Mint SVID, single URI SAN (S04) | Compose trust bundle (S05) |
| Protect root key at rest (S02) | | Re-issue on demand (S05) | Write issued-cert audit row (S05) |

### Walking Skeleton (thinnest end-to-end, all activities)

**Slices 01 → 02 → 03 → 04**: generate root → envelope-encrypt + persist →
issue pathLen=0 intermediate → mint single-URI-SAN SVID → **full chain
verifies** (`openssl verify -CAfile root -untrusted intermediate svid` → exit 0).

### Release 1 (first enhancement past the skeleton)

**Slice 05**: trust-bundle composition + `issued_certificates` audit row +
re-issue-on-demand. Targets the auditability and repeatability outcomes; gives
the rotation workflow (#40) a sound mechanism to build on.

### Slice list (each = one story = one ≤1-day cut)

| Slice | Story | Learning hypothesis (disproves X if it fails) | Brief |
|---|---|---|---|
| 01 | US-CA-01 | rcgen+aws-lc-rs can mint a SPIFFE-hierarchy root behind our `Ca` port trait, DST-deterministic via sim adapter | `slices/slice-01-root-ca-behind-port-trait.md` |
| 02 | US-CA-02 | the root survives restart with its key protected by authenticated encryption, using only aws-lc-rs | `slices/slice-02-root-key-envelope-encrypted-at-rest.md` |
| 03 | US-CA-03 | the platform issues a pathLen=0 intermediate that chains to the root, bounding node-compromise blast radius | `slices/slice-03-per-node-intermediate-ca.md` |
| 04 | US-CA-04 | the platform mints a SPIFFE-spec-compliant SVID that validates through the full 3-tier chain, DST-deterministic serials | `slices/slice-04-workload-svid-spiffe-san.md` |
| 05 | US-CA-05 | an SVID validates against the platform-composed bundle, issuance is auditable, re-issue works without restart | `slices/slice-05-trust-bundle-audit-and-reissue.md` |

---

## Wave: DISCUSS / [REF] Priority Rationale

Execution order = **learning leverage first** (highest-uncertainty slices early,
so failures cost least) then **dependency chain** then **dogfood cadence**.

| Order | Slice | Why this position |
|---|---|---|
| 1 | S01 | **Riskiest assumption** — does the crypto stack work behind our port trait at all? If rcgen/aws-lc-rs/sim-equivalence fails, everything downstream is moot. Cheapest place to learn it. |
| 2 | S02 | Strict dependency on S01 (needs a root to persist). Resolves the persistence/key-protection risk — the reason this feature exists (supersede ADR-0010 ephemerality). |
| 3 | S03 | Depends on S02 (needs a persistent root key to sign with). Middle tier; lower uncertainty than 01/02 (`signed_by` already proven in `tls_bootstrap.rs`). |
| 4 | S04 | Depends on S03. **Completes the walking skeleton** — the headline dogfood moment ("a workload has a verifiable identity"). High value, moderate risk. |
| 5 | S05 | Depends on S04+S03. Enhancement; the mechanism the rotation workflow (#40) will drive. Lowest uncertainty — additive plumbing on proven surfaces. |

Dependency chain is linear (S01→S02→S03→S04→S05) — inherent to a layered trust
hierarchy (you cannot sign a leaf without an intermediate without a root). No
parallelism available; the order above is both the dependency order and the
risk-retirement order, which is the ideal alignment.

---

## Wave: DISCUSS / [REF] System Constraints (cross-cutting)

These apply to every story; stated once here rather than repeated per story.

- **Crypto backend**: rcgen MUST use the `aws_lc_rs` feature so the CA shares
  the workspace rustls crypto provider (ADR-0039, FIPS 140-3 Cert #4816).
  Confirm the feature flag at first compile (research Gap 3).
- **DST discipline**: certificate **serial numbers** flow through the `Entropy`
  port (`OsEntropy` prod, `SeededEntropy` DST — research Finding 10).
  **Key generation** is a host-adapter concern and is NOT injectable (research
  Finding 11); DST uses pre-generated fixture keys loaded via PEM. Core-class
  code stays free of banned APIs (dst-lint).
- **State-layer hygiene**: CA *material* (root key, certs) is **intent**
  (IntentStore, redb) and deliberately never written to the ObservationStore
  (whitepaper §4). The *audit of what was issued* (`issued_certificates`) is
  **observation** (gossiped). These never merge.
- **Port-trait shape**: the `Ca` trait lives in `overdrive-core`; a host adapter
  (rcgen) and a sim adapter (fixture keys) both implement it; the dependency is
  required (constructor parameter), not defaulted (`.claude/rules/development.md`
  § "Port-trait dependencies"). A DST equivalence test drives both adapters
  through the same calls.
- **Persist inputs, not derived state**: store the envelope-encrypted key
  material, not any decoded/derived form; recompute on read.
- **No operator CLI verb in this phase**: SVID issuance is an internal platform
  mechanism triggered when the platform runs a workload — there is **no**
  `overdrive` subcommand to "issue an SVID". The operator-visible surfaces are
  the control plane starting cleanly with a persistent CA and the
  `issued_certificates` audit row (readable via the existing `alloc status`
  observation path). Do NOT invent a CLI verb. (And per CLAUDE.md, the workload
  verb is `overdrive deploy <SPEC>`, never `job submit`.)
- **Single-node (Phase 2.6)**: the one co-located node gets exactly one
  intermediate; no node-registration verb exists. Multi-node per-node
  intermediates + node attestation at bootstrap are owned by **#36 [2.14]**
  (node enrollment / admission handler, which already `Depends on #28`).

---

## Wave: DISCUSS / [REF] User Stories

Every story traces to `job_id: J-SEC-001`. Every story has an Elevator Pitch.
ACs are embedded and derived from the UAT scenarios. None are `@infrastructure`
(each delivers a verifiable security property — see Elevator Pitches).

> **Elevator-Pitch "After" caveat**: this is a security *primitive* with no
> operator CLI verb (see System Constraints). Each pitch's "After" references a
> real, executable verification entry point — `openssl verify` on the minted
> material and the `issued_certificates` observation surface — which is the
> honest user-invocable observable output for this feature, not an invented
> subcommand. The DECISION enabled is the security reviewer's / operator's
> trust decision, which is the genuine J-SEC-001 connection.

### US-CA-01 — Root CA generation behind the `Ca` port trait

**Problem**: Sam, a platform/security engineer, has no platform-minted root of
trust for workload identity — the Phase-1 CA is ephemeral and dies on restart.
He finds it untenable to bolt on SPIRE/Vault just to get a root CA.

**Who**: Platform/security engineer | building the identity layer | wants a root
of trust the platform owns.

**Solution**: A `Ca` port trait (core) with a rcgen/aws-lc-rs host adapter and a
fixture-keyed sim adapter; `generate_root()` mints a self-signed P-256 root CA.

#### Elevator Pitch

- **Before**: there is no persistent, platform-minted root of trust for workload identity; the only CA is ephemeral (ADR-0010).
- **After**: the platform generates a self-signed root CA, and `openssl verify -CAfile root.pem root.pem` → exits 0 (`OK`) — a valid self-signed CA.
- **Decision enabled**: Sam decides the crypto stack + port-trait seam are sound enough to build the rest of the hierarchy on (or stops here if the root is malformed).

#### Domain Examples

1. **Happy path** — The control plane on Sam's single-node host calls `Ca::generate_root()`; rcgen (aws-lc-rs backend) produces a P-256 root with `CA:TRUE`, `keyCertSign`, `keyUsage` critical. `openssl verify` accepts it.
2. **DST determinism** — Under the seeded harness, `SimCa::generate_root()` loads fixture key `ca-fixture-p256.pem`; two runs at seed `0x5EED` produce bit-identical material.
3. **Boundary** — `Ca::generate_root()` on a host where the aws-lc-rs backend is unavailable surfaces a typed error (not a panic); the control plane does not proceed with no root.

#### UAT Scenarios (BDD)

##### Scenario: The platform produces a valid self-signed root CA
Given a freshly initialised control plane with no existing CA material
When the platform generates its root certificate authority
Then the root certificate is a valid self-signed CA (CA:TRUE, keyCertSign, keyUsage critical)
And `openssl verify` accepts it as a self-signed CA

##### Scenario: Root generation is deterministic under the simulation harness
Given the seeded DST harness with the sim CA adapter and a fixture root key
When the platform generates the root twice at the same seed
Then both runs produce bit-identical root material

#### Acceptance Criteria

- [ ] `Ca::generate_root()` exists in `overdrive-core` with a behaviour-pinning docstring; passes dst-lint (no banned APIs in core).
- [ ] Host adapter produces a root with `CA:TRUE`, `keyCertSign` set, `keyUsage` marked critical, P-256.
- [ ] `openssl verify -CAfile root.pem root.pem` exits 0.
- [ ] Sim adapter is deterministic at a fixed seed.

#### Technical Notes

- Re-shapes the proven `mint_ephemeral_ca` rcgen usage behind a port trait. Confirm `rcgen` carries `aws_lc_rs` (research Gap 3).

---

### US-CA-02 — Root CA key envelope-encrypted at rest

**Problem**: Sam cannot trust a root whose key dies on restart or sits in
plaintext. He needs the root to persist with its private key protected by
authenticated encryption.

**Who**: Platform/security engineer | operating a cluster that must keep a
stable trust anchor | wants the root key safe at rest.

**Solution**: Envelope-encrypt the root private key (AES-256-GCM DEK,
passphrase-derived KEK) and store only the ciphertext in the IntentStore;
decrypt + reuse on subsequent boots.

#### Elevator Pitch

- **Before**: the root key is in memory only and is discarded on restart; there is no key-protected persistent trust anchor.
- **After**: restart the control plane → it reuses the **same** root (same identity), and inspecting the IntentStore file shows only the AES-256-GCM-encrypted blob, never the plaintext key.
- **Decision enabled**: Sam decides the root key is safe enough at rest to defend in a security review (or refuses to ship if plaintext key bytes are present on disk).

#### Domain Examples

1. **Happy path** — First boot generates + envelope-encrypts + persists the root. Second boot decrypts with the operator passphrase and reuses the identical root; existing SVIDs (later slices) stay valid.
2. **Tamper detection** — An attacker flips a byte in the encrypted blob on disk; next boot's AES-GCM authentication fails → the control plane refuses to start with a "corrupt/tampered envelope" error, distinct from a "wrong passphrase" error.
3. **Wrong passphrase** — Operator supplies the wrong passphrase; boot refuses to start with a "bad passphrase" error and does NOT re-mint a new root (which would orphan every issued identity).

#### UAT Scenarios (BDD)

##### Scenario: The root CA survives a restart with its key protected
Given the control plane has generated and persisted its root CA
And only the envelope-encrypted key blob is on disk (no plaintext key bytes)
When the control plane restarts with the correct operator passphrase
Then it reuses the same root CA identity (same public key)

##### Scenario: A tampered root key blob refuses to start, distinctly from a wrong passphrase
Given the persisted encrypted root key blob has been tampered with on disk
When the control plane attempts to start
Then it refuses to start with an actionable error naming a corrupt/tampered envelope
And the error is distinguishable from a wrong-passphrase error
And no new root CA is silently minted

#### Acceptance Criteria

- [ ] First boot persists; second boot decrypts + reuses the same root identity across restart.
- [ ] A test asserts plaintext private-key bytes do NOT appear in the IntentStore file.
- [ ] AES-256-GCM authentication: tampered ciphertext → distinct error from wrong passphrase.
- [ ] Decryption failure → control plane refuses to start (`health.startup.refused`), does NOT re-mint.

#### Technical Notes

- aws-lc-rs AEAD (research Finding 8 Approach B). Confirm the passphrase-KDF crate (scrypt/argon2) in the workspace graph. Root rotation (dual-bundle) is OUT — that is GH #40.

---

### US-CA-03 — Per-node intermediate CA, pathLen-constrained

**Problem**: Sam needs the node to sign workload identities locally, but an
unbounded intermediate (one that can mint further CAs) is an unacceptable
blast radius on node compromise.

**Who**: Platform/security engineer | running workloads on a node | wants local
signing power that is bounded by construction.

**Solution**: Mint a Node Intermediate CA signed by the root with
`basicConstraints` pathLen=0 — issues leaves only, no further intermediates.

#### Elevator Pitch

- **Before**: there is no node-level signing authority; and naively a node CA could mint further CAs (unbounded blast radius).
- **After**: the node gets an intermediate, and `openssl verify -CAfile root.pem intermediate.pem` → exits 0, while a chain where that intermediate signs a *further CA* → fails verification (pathLen=0 enforced).
- **Decision enabled**: Sam decides node-compromise blast radius is acceptably bounded (the intermediate cannot escalate to mint further CAs).

#### Domain Examples

1. **Happy path** — At node bootstrap the platform mints an intermediate signed by the root, pathLen=0; `openssl verify -CAfile root.pem intermediate.pem` succeeds.
2. **Constraint enforced** — A constructed chain `root → intermediate → another-CA` fails verification because pathLen=0 forbids the intermediate from being a CA issuer.
3. **Signing failure** — The root key is unavailable at node bootstrap (decrypt failed upstream); intermediate signing surfaces a typed error and the node does not run workloads it cannot identify.

#### UAT Scenarios (BDD)

##### Scenario: The node intermediate chains to the root and is pathLen-constrained
Given a persistent root CA
When the platform issues a node intermediate CA at bootstrap
Then the intermediate has CA:TRUE and pathLenConstraint=0
And `openssl verify -CAfile root.pem intermediate.pem` exits 0

##### Scenario: The intermediate cannot mint a further CA
Given a node intermediate CA with pathLen=0
When a chain is constructed in which that intermediate signs a further CA certificate
Then chain verification fails

#### Acceptance Criteria

- [ ] Intermediate has `CA:TRUE`, `pathLenConstraint=0`, `keyCertSign`, `keyUsage` critical.
- [ ] `openssl verify -CAfile root.pem intermediate.pem` exits 0.
- [ ] A chain where the intermediate signs a further CA fails verification (constraint enforced, not merely set).
- [ ] Intermediate signing failure → typed error; node bootstrap fails loudly.

#### Technical Notes

- `IsCa::Ca(BasicConstraints::Constrained(0))` (research Finding 4). Single-node: one node → one intermediate. URI name-constraints on the intermediate are an optional hardening, not in scope. Scheduled re-signing → GH #40.

---

### US-CA-04 — Workload SVID with single SPIFFE URI SAN

**Problem**: Sam needs each workload to carry a forgery-proof, SPIFFE-compliant
identity that validates through the full chain — and the SPIFFE spec's hardest
rule (exactly one URI SAN) must be enforced, not hoped for.

**Who**: Platform/security engineer | whose workloads' billing/policy/mTLS all
key off identity | wants spec-compliant, chain-verifiable SVIDs.

**Solution**: The node intermediate signs a short-lived leaf with exactly one
SPIFFE URI SAN, `CA:FALSE`, `keyUsage=digitalSignature` (critical), and a
CSPRNG serial drawn through the `Entropy` port.

#### Elevator Pitch

- **Before**: workloads have no platform-minted SPIFFE identity; nothing enforces the single-URI-SAN spec rule.
- **After**: the platform mints an SVID, and `openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem` → exits 0; the leaf carries exactly one `URI:spiffe://overdrive.local/job/.../alloc/...` SAN and `CA:FALSE`.
- **Decision enabled**: Sam decides workload identity is real and spec-compliant — the foundation billing/policy/mTLS depend on — or rejects a leaf with the wrong SAN cardinality.

#### Domain Examples

1. **Happy path** — The platform starts allocation `a1b2c3` of job `payments`; the node intermediate signs a leaf with one SAN `URI:spiffe://overdrive.local/job/payments/alloc/a1b2c3`, CA:FALSE, 1h TTL. Full chain verifies.
2. **Single-URI invariant** — A request that would yield two URI SANs (or zero) is rejected before any cert is produced.
3. **DST serial determinism** — Under seed `0x5EED`, the SVID serial drawn via `Entropy::fill` is identical across two runs; in production two mints produce distinct ≥64-bit CSPRNG serials.

#### UAT Scenarios (BDD)

##### Scenario: A workload SVID validates through the full Root → Intermediate → SVID chain
Given a persistent root, a node intermediate, and a request to identify allocation a1b2c3 of job payments
When the platform mints the workload SVID
Then the leaf carries exactly one URI SAN equal to spiffe://overdrive.local/job/payments/alloc/a1b2c3
And the leaf is CA:FALSE with keyUsage=digitalSignature marked critical
And `openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem` exits 0

##### Scenario: An SVID that would carry zero or multiple URI SANs is rejected
Given a request whose SpiffeId would yield zero or more than one URI SAN
When the platform attempts to mint the SVID
Then issuance is rejected before any certificate is produced

##### Scenario: SVID serial numbers are CSPRNG and DST-deterministic
Given the seeded DST harness with the sim CA adapter
When the platform mints an SVID twice at the same seed
Then both serials are identical and at least 64 bits

#### Acceptance Criteria

- [ ] SVID has `CA:FALSE`, exactly ONE `URI` SAN equal to the requested SpiffeId, `keyUsage=digitalSignature` critical, NO `keyCertSign`/`cRLSign`.
- [ ] Issuing a SpiffeId yielding 0 or ≥2 URI SANs is rejected before any cert is produced.
- [ ] `openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem` exits 0.
- [ ] Serial ≥64-bit CSPRNG via `Entropy`; DST-deterministic at a seed; distinct in production.

#### Technical Notes

- `SanType::URI` (research Finding 1); reuse `SpiffeId` + `CertSerial` newtypes and the `Entropy` port (all exist). 1h TTL (research Finding 6). Rotation/distribution/Workload-API are OUT (GH #40 / consumer feature / Phase 7).

---

### US-CA-05 — Trust bundle, issued-cert audit, re-issue on demand

**Problem**: Sam needs an SVID to validate against a platform-composed trust
bundle (not a hand-assembled CA file), needs to see what was issued (audit),
and needs re-issuance to work without bouncing the control plane — the
mechanism a future rotation workflow will drive.

**Who**: Platform/security engineer | auditing issuance + relying on a bundle |
wants verifiable, auditable, repeatable issuance.

**Solution**: Compose a trust bundle; write an `issued_certificates` observation
row per issuance; re-issue a fresh SVID on demand with no restart.

#### Elevator Pitch

- **Before**: there is no platform trust bundle to verify against, no record of what was issued, and re-issuing requires a restart.
- **After**: a re-issued SVID verifies against the platform's `trust_bundle()`, and `overdrive alloc status --job <id>` (the existing observation surface) shows the `issued_certificates` row (serial, SPIFFE ID, issuer, validity) for the workload.
- **Decision enabled**: Sam decides issuance is auditable and repeatable enough to rely on — and the rotation workflow (#40) has a sound mechanism to build on.

#### Domain Examples

1. **Happy path** — The platform composes a bundle anchored on the root; a Slice-04 SVID verifies against it. Each issuance writes an `issued_certificates` row; Sam reads it back and the serial matches the minted cert.
2. **Re-issue without restart** — The platform re-issues an SVID for `spiffe://overdrive.local/job/payments/alloc/a1b2c3`; a fresh leaf (new serial, new validity window) is produced and the control plane is not restarted.
3. **Audit-write failure** — If the `issued_certificates` row cannot be written, the issuance surfaces an error (issuance + audit are observable together; no silent issuance).

#### UAT Scenarios (BDD)

##### Scenario: A re-issued SVID verifies against the platform trust bundle and is audited
Given a workload with an existing SVID and a platform-composed trust bundle
When the platform re-issues a fresh SVID for that workload without restarting
Then the new SVID verifies against the trust bundle
And an issued_certificates row records the new serial, SPIFFE ID, issuer serial, and validity window

##### Scenario: Issuance is never silent
Given an issuance whose audit row cannot be written
When the platform attempts to mint the certificate
Then the issuance surfaces an error rather than handing out an unaudited certificate

#### Acceptance Criteria

- [ ] `trust_bundle()` returns material such that a Slice-04 SVID verifies against it with a standard tool.
- [ ] Every issuance writes an `issued_certificates` observation row; a test reads it back and matches serial + spiffe_id + issuer_serial.
- [ ] Re-issuing for an existing SpiffeId yields a fresh cert (distinct serial, new validity) with no control-plane restart.
- [ ] Issuance that cannot write its audit row surfaces an error (no silent issuance).

#### Technical Notes

- `issued_certificates` observation row (research Finding 15) mirrors `alloc_status`/`node_health` plumbing. Revocation (CRL/OCSP/`revoked_operator_certs`) is OUT — SVID revocation-by-expiry (1h TTL) is the model; gossip revocation is Phase 5. The scheduled renewal *trigger* is GH #40.

---

## Wave: DISCUSS / [REF] Outcome KPIs

### Objective

By the end of #28, every workload the platform runs can be given a
forgery-proof, SPIFFE-compliant, chain-verifiable identity minted by the
platform itself — with zero external PKI components to operate.

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| K1 | Platform-issued workload SVIDs | chain-verify Root → Intermediate → SVID with a standard tool | 100% of issued SVIDs verify (`openssl verify` exits 0) | 0% (no SVID hierarchy exists; Phase-1 CA is 2-tier, CN-only) | Acceptance test asserting `openssl verify` over the minted chain (Slices 04/05) | Leading |
| K2 | Issued SVIDs | carry a SPIFFE-spec-compliant SAN | 100% carry exactly one URI SAN; 0% carry 0 or ≥2 (rejected at issuance) | n/a (no SVIDs today) | Cert-inspection test + the single-URI-SAN rejection test (Slice 04) | Leading |
| K3 | The root CA private key | is never observable in plaintext at rest | 0 plaintext key bytes in the IntentStore file across the full lifecycle | n/a (Phase-1 key is in memory, never persisted) | Test scanning the IntentStore file for plaintext key material (Slice 02) | Guardrail |
| K4 | External identity components Sam must deploy/operate | to give workloads cryptographic identity | 0 (no SPIRE / cert-manager / Vault) | 3+ in a comparable SPIFFE stack | Architecture review — the feature ships entirely inside the one binary | Lagging |
| K5 | The CA primitive | composes deterministically under DST | 100% of CA DST scenarios reproduce bit-identically from a seed | n/a | Seeded DST runs (serials via Entropy; fixture keys) reproduce identically (all slices) | Guardrail |

### Metric hierarchy

- **North Star**: K1 — % of issued SVIDs that chain-verify to the root. This is
  the single signal that the trust hierarchy is real.
- **Leading indicators**: K2 (spec compliance) predicts K1 (a non-compliant
  leaf may still "verify" loosely but fails the spec gate).
- **Guardrails**: K3 (root key never plaintext at rest) and K5 (DST
  determinism) must NOT degrade as slices land.

### Measurement plan

| KPI | Data source | Collection method | Frequency | Owner |
|---|---|---|---|---|
| K1, K2 | Host-adapter acceptance tests | `openssl verify` + cert inspection in CI (integration-tests feature, via Lima) | Per PR | crafter / CI |
| K3 | IntentStore file scan test | Byte-scan assertion in the Slice-02 acceptance test | Per PR | crafter / CI |
| K4 | Dependency + architecture review | Manual review at DESIGN/handoff | Once at handoff | architect |
| K5 | Seeded DST harness | `cargo dst` twin-run identity | Per PR | crafter / CI |

### Hypothesis

We believe that a built-in `Ca` port trait + 3-tier hierarchy for the platform
will achieve K1 (100% chain-verifiable SVIDs) and K4 (0 external identity
components). We will know this is true when 100% of issued SVIDs verify with
`openssl verify` and the feature ships entirely inside the one binary.

---

## Wave: DISCUSS / [REF] Out-of-scope (explicit non-goals)

Each non-goal cites its owning issue/phase. No hand-wavy forward pointers.

| Non-goal | Owner | Note |
|---|---|---|
| Certificate **rotation lifecycle** (scheduled renewal: detect-expiry → mint-fresh → swap → retire) | **GH #40** [3.3], depends on workflow primitive **GH #39** [3.2] + this #28 | The engine here can *issue and re-issue on demand* (Slice 05); the durable **workflow** that drives scheduled renewal is #40 (research Finding 12 — rotation is a workflow, not a reconciler). |
| Root CA **rotation** (SPIRE two-phase dual-bundle) | **GH #40** | Research Finding 9. Single persistence + reuse is in scope (Slice 02); rotation is not. |
| Operator cert minting (`overdrive op create`), operator SPIFFE IDs, OIDC enrolment, Biscuit delegation | **Phase 5 / Phase 7** (whitepaper §8 "Deferred"; user memory `project_cli_auth`; GH #81 for `cluster init`/`op create`/`op revoke`) | This feature is workload identity only. |
| Gossip-propagated revocation (`revoked_operator_certs`, CRL/OCSP) | **Phase 5** (whitepaper §8) | SVID revocation-by-expiry (1h TTL) is the model here. |
| mTLS handshake + kTLS session-key install (the CA's *consumer*) | **Separate feature** (whitepaper §8 Kernel mTLS) | This feature mints identities; it does not perform handshakes. |
| Multi-region CA federation | **#104 [7.1]** (multi-region federation) + **#83 [5.17]** (operator trust-bundle federation across regions) (research Finding 14) | Per-region roots under a global operator root — not in single-node Phase 2.6. |
| SVID distribution to workloads (vsock / fs mount) + SPIFFE Workload API (Unix-socket gRPC) | **Consumer feature / Phase 7+** (research Gap 1) | The CA engine produces SVID material; delivery to the running workload is a separate concern. |
| Multi-node per-node intermediates + node attestation at bootstrap | **#36 [2.14]** node enrollment / admission handler (research Finding 5/13) — `Depends on #28` | Single-node gets exactly one intermediate; multi-node is NOT a prerequisite for #28. #36 is the verified home: "first-boot exchange issues SVID + ... returns trust bundle and node-intermediate CA material to the enrolling agent; accepts optional TPM attestation." |
| HSM / KMS / OS-keyring KEK source | **Later phase** (research Finding 8 Approach C, Gap 2) | The KEK source is pluggable by construction; passphrase-derived only here. |

---

## Wave: DISCUSS / [REF] Driving Ports & Pre-requisites

**Driving ports (inbound surfaces that trigger CA behaviour)**:
- Control-plane **bootstrap** path → triggers root CA generate-or-load (US-CA-01/02).
- Node **bootstrap** path → triggers intermediate issuance (US-CA-03).
- Workload **start** path (the existing allocation lifecycle) → triggers SVID issuance (US-CA-04/05).
- **No operator CLI verb** — by design (see System Constraints). The only
  operator-observable read surface is the `issued_certificates` observation row
  via the existing `alloc status` path.

**Pre-requisites (all satisfied today)**:
- `overdrive-core`: `SpiffeId`, `CertSerial` newtypes; `Entropy` port (`fill`). ✓ confirmed present.
- `overdrive-store-local`: `LocalStore` (IntentStore) for the encrypted root blob. ✓
- `overdrive-control-plane` / `overdrive-sim`: ObservationStore + SimObservationStore for the audit row. ✓
- rcgen + rustls + aws-lc-rs in the workspace graph (ADR-0039; brief §10). ✓ (confirm `rcgen` `aws_lc_rs` feature — research Gap 3).

---

## Wave: DISCUSS / [REF] Definition of Ready (9-item gate)

| # | DoR Item | Status | Evidence |
|---|---|---|---|
| 1 | Problem statement clear, domain language | ✅ PASS | Each US has a Problem in security-engineer domain language; J-SEC-001 frames the job. |
| 2 | User/persona with specific characteristics | ✅ PASS | `sam-platform-security-engineer` persona (10+ yr platform/security, threat-models, verifies with openssl). |
| 3 | 3+ domain examples with real data | ✅ PASS | Each US has 3 examples with real data (`spiffe://overdrive.local/job/payments/alloc/a1b2c3`, seed `0x5EED`, P-256, AES-256-GCM). |
| 4 | UAT in Given/When/Then (3-7 scenarios) | ✅ PASS | 2–3 scenarios per story, 12 total across 5 stories (within range per story; happy + boundary + error coverage). |
| 5 | AC derived from UAT | ✅ PASS | Each US's AC checklist maps 1:1 to its scenarios. |
| 6 | Right-sized (1-3 days, 3-7 scenarios) | ✅ PASS | 5 slices, each ≤1 day, 2–3 scenarios each (slice briefs). |
| 7 | Technical notes: constraints/dependencies | ✅ PASS | § System Constraints + per-story Technical Notes. |
| 8 | Dependencies resolved or tracked | ✅ PASS | Pre-reqs all present (newtypes/Entropy/stores confirmed); non-goals cite #36/#40/#39/#83/#104/#81/Phase 5/7. Multi-node framing RESOLVED — single-node confirmed by user; multi-node owned by existing #36 [2.14] (no new issue). |
| 9 | Outcome KPIs defined with measurable targets | ✅ PASS | K1–K5 with numeric targets + measurement method. |

**DoR verdict: PASS (9/9)** — item-8 multi-node framing is RESOLVED: the user
confirmed single-node scope (2026-06-05), and the multi-node extension is owned
by the existing **#36 [2.14]** node enrollment / admission handler (already
`Depends on #28`). No new issue, no duplicate.

---

## Wave: DISCUSS / [REF] Density & Triggers

**Resolved density**: `lean` + `ask-intelligent` (DISCUSS hard default; the
project `des-config.json` rigor is `inherit`, which does not override the wave
default). Tier-1 `[REF]` sections emitted; no Tier-2 expansions auto-rendered.

**Triggers evaluated (`ask-intelligent` mode)** — two fired; reported here
rather than auto-expanded (per the lean discipline):

| Trigger | Fired? | Detail | Suggested expansion (NOT auto-applied) |
|---|---|---|---|
| AC ambiguity | No | ACs are crisp crypto invariants (single URI SAN, pathLen=0, chain verifies) — no reasonable-reader disagreement. | — |
| Cross-context complexity | **YES** | ≥3 distinct technologies: rcgen X.509, aws-lc-rs AEAD envelope encryption, redb/IntentStore persistence, the Entropy port. (One bounded context, but the tech surface is broad.) | `alternatives-considered` (e.g. envelope-encryption Approach A vs B vs C — research Finding 8) |
| Multi-stakeholder need | No | One persona (Sam). | — |
| Compliance / regulatory | **YES** | ACs reference encryption-at-rest, audit (`issued_certificates`), FIPS alignment (aws-lc-rs Cert #4816). | `journey-deep-dive` (CA capability path with error states) |
| WS strategy = D (Configurable) | No | Normal vertical walking skeleton (Slices 01–04). | — |

The orchestrator may request `--expand alternatives-considered` (envelope-
encryption tradeoffs) and/or `--expand journey-deep-dive` (full error-path map)
if the downstream DESIGN wave would benefit. The research doc already covers the
envelope-encryption alternatives (Finding 8) and the SSOT journey
(`docs/product/journeys/issue-workload-identity.yaml`) covers the error paths —
so the lean default is defensible and expansion is optional.

---

## Wave: DISCUSS / [REF] Wave Decisions

### Key decisions

- **[D-CA-1]** Feature is right-sized as ONE feature, sliced into 5 thin vertical
  cuts (not split into multiple features). Rationale: scope assessment — zero
  oversized signals fire; one coherent outcome (a workload identity). (See §
  Scope Assessment.)
- **[D-CA-2]** The `Ca` capability is a **port trait** (core + host + sim),
  matching `Clock`/`Transport`/`Entropy`. Rationale: research Finding 11 + project
  port-trait discipline; makes the CA DST-honest (fixture keys + Entropy serials).
- **[D-CA-3]** Certificate **rotation is OUT of scope** and belongs to GH #40 (a
  workflow), depending on workflow primitive GH #39. The engine provides
  *issue + re-issue on demand* (Slice 05); the scheduled-renewal *driver* is #40.
  Rationale: research Finding 12 (corrected) — rotation is a workflow.
- **[D-CA-4]** **No operator CLI verb** in this phase. SVID issuance is internal
  platform mechanism; the only operator-observable surface is the
  `issued_certificates` audit row. Rationale: System Constraints + CLAUDE.md
  (the workload verb is `overdrive deploy`, not an invented CA verb).
- **[D-CA-5]** This feature **supersedes ADR-0010's ephemeral CA** for *workload
  identity* but does NOT delete `tls_bootstrap.rs` (which serves the distinct
  control-plane-HTTPS consumer; its replacement is Phase 5 operator-mTLS work).
- **[D-CA-6]** **Single-node (Phase 2.6)** — *confirmed by the user 2026-06-05*:
  one co-located node gets exactly one intermediate. Multi-node per-node
  intermediates + node attestation are NOT a prerequisite for #28; they are
  owned by **#36 [2.14]** (node enrollment / admission handler), which already
  `Depends on #28`. No new follow-up issue is created — #36 is the verified,
  scope-matching home.

### Requirements summary

- Primary job: J-SEC-001 — a platform-minted, forgery-proof, SPIFFE-compliant
  workload identity with no external PKI to operate.
- Walking skeleton: Slices 01–04 (root → persist → intermediate → SVID, chain
  verifies). Release 1: Slice 05 (bundle + audit + re-issue).
- Feature type: cross-cutting security primitive.

### Constraints established

See § System Constraints (crypto backend = aws-lc-rs; serials via Entropy / keys
via fixtures under DST; CA material = intent, audit = observation; port-trait
shape; persist inputs not derived state; no CLI verb; single-node).

### Upstream changes

- None to DISCOVER/DIVERGE (no DIVERGE artifacts exist for this feature — the
  orchestrator confirmed research is complete and the job is authored fresh).
- SSOT additions (this wave): `docs/product/jobs.yaml` (+ J-SEC-001 + changelog),
  `docs/product/personas/sam-platform-security-engineer.yaml` (new),
  `docs/product/journeys/issue-workload-identity.yaml` (new).

---

## Wave: DISCUSS / [REF] Open Questions / BLOCKERS for the orchestrator

> Surfaced per the project rule: a subagent cannot create GH issues or message
> the user. These are relayed for the orchestrator to put to the user.

1. **[RESOLVED 2026-06-05] Multi-node framing — single-node confirmed; multi-node
   tracked by existing #36.** Phase 1 was single-node (one co-located node; no
   node-registration verb — user memory `feedback_phase1_single_node_scope`).
   #28 is Phase 2.6. The honest reading — **confirmed by the user**: per-node
   intermediate CA does NOT require multi-node node-registration. The single
   co-located node gets exactly one intermediate, mirroring how the single-node
   dataplane (ADR-0061) collapses three roles onto one host. The CA hierarchy
   (root → intermediate → SVID) is fully exercisable single-node. The
   *multi-node* shape (per-node intermediates, node attestation at bootstrap —
   research Finding 5/13) is owned by the **existing #36 [2.14]** (node
   enrollment / admission handler — "first-boot exchange issues SVID + ...
   returns trust bundle and node-intermediate CA material to the enrolling
   agent; accepts optional TPM attestation"), which already `Depends on #28`.
   **No new follow-up issue was created** — #36 is the verified, scope-matching
   home (per CLAUDE.md deferral discipline: cite the existing issue, never
   duplicate). Slices 03–05 and D-CA-6 bake in the single-node scope.

2. **[NON-BLOCKING — confirm at implementation, no issue needed] rcgen
   `aws_lc_rs` feature flag (research Gap 3).** The workspace declares rcgen and
   rustls(aws-lc-rs), but the research could not fully validate that rcgen
   carries the `aws_lc_rs` feature without conflict. Resolution is a first-compile
   check in Slice 01, not a separate spike or issue.

No deferral in this feature requires a NEW GH issue: every non-goal maps to an
EXISTING issue (#36 multi-node CA, #40 rotation, #39 workflow primitive, #83 /
#104 multi-region, #81 cluster-init/op-create) or a named phase (5/7). **No
invented issue numbers, no hand-wavy forward pointers, no duplicates.**

---

## Wave: DISCUSS / [REF] SSOT Artifacts Produced

| Artifact | Path | Change |
|---|---|---|
| Job register | `docs/product/jobs.yaml` | + J-SEC-001 (full JTBD) + changelog entry |
| Persona | `docs/product/personas/sam-platform-security-engineer.yaml` | NEW |
| Journey | `docs/product/journeys/issue-workload-identity.yaml` | NEW (product-level summary) |
| Slice briefs | `docs/feature/built-in-ca/slices/slice-0{1..5}-*.md` | NEW (5 briefs) |
| Feature delta | `docs/feature/built-in-ca/feature-delta.md` | THIS file |

---

## Wave: DISCUSS / [REF] Handoff

- **To DESIGN (nw-solution-architect)**: full artifact set — this feature-delta
  (stories + ACs + story map + system constraints + KPIs) + the 5 slice briefs +
  the SSOT job/persona/journey. Key DESIGN questions: the `Ca` port-trait
  surface shape; the envelope-encryption KEK derivation + IntentStore key shape;
  where root/intermediate wire into control-plane and node bootstrap; the
  `issued_certificates` observation-row schema.
- **To DEVOPS (nw-platform-architect)**: the Outcome KPIs (K1–K5) — instrument
  chain-verify rate, spec-compliance, the no-plaintext-key guardrail, DST
  determinism.
- **To DISTILL (nw-acceptance-designer)**: the UAT scenarios (embedded above —
  no standalone `.feature` file per `.claude/rules/testing.md`), the error paths
  (SSOT journey), and the KPIs. Note: the test surface for this feature crosses
  Tier-1 (DST, sim CA adapter, deterministic serials) and host-adapter
  acceptance tests (`openssl verify` over real rcgen output, gated behind
  `integration-tests`, run via Lima).

---

## Wave: DISCUSS / [REF] Review Record

- **Reviewer**: nw-product-owner-reviewer (Eclipse) — 2026-06-05
- **Verdict**: **APPROVED** — 0 blocking, 0 high, 0 medium, 1 non-blocking nit.
- **Scope of review**: feature-delta + 5 slice briefs + SSOT (J-SEC-001, persona,
  journey). DoR 9/9 PASS; slice-composition hard gate PASS (no infra-only slice);
  elevator-pitch gate PASS (all 5 stories observable); zero antipatterns;
  deferral discipline assessed "immaculate".
- **Non-blocking nit (applied)**: journey steps 2–3 given clarifying
  parentheticals for non-PKI readers.
- **Post-review correction (orchestrator)**: multi-node deferral references
  re-pointed from vague "Phase 5+" to the verified existing **#36 [2.14]** (node
  enrollment / admission handler, `Depends on #28`); multi-region made precise
  (#104 [7.1] / #83 [5.17]). User confirmed single-node framing for #28
  (2026-06-05). No new issue created — #36 is the scope-matching home.

---

# DESIGN Wave (wave 4 of 6) · Agent: Morgan (nw-solution-architect) · Mode: GUIDE · Density: `lean`

DESIGN content appends below under `## Wave: DESIGN / [REF] <Section>` headings.
Tier-1 `[REF]` sections only (lean default); Tier-2 expansions are *recommended*
to DISTILL/DELIVER, not auto-rendered. Mode = GUIDE: the user made the key
decisions in guided Q&A (2026-06-05); this wave writes to them. The two
explicit reconciliation points (A) and (B) are resolved here with rationale.

**SSOT for the architecture**: `docs/product/architecture/adr-0063-built-in-ca-port-trait-and-root-key-protection.md`,
`docs/product/architecture/brief.md` § "built-in-ca extension",
`docs/product/architecture/c4-diagrams.md` § "Built-in CA". This feature-delta
section is the navigable summary; the brief + ADR are authoritative.

---

## Wave: DESIGN / [REF] DDD — Bounded Context & Tactical Shape

One bounded context: **workload identity / CA**, spanning ~4 crates but one
ubiquitous language (root / intermediate / SVID / trust bundle / SPIFFE ID).
It is a *supporting* security primitive (the *core* domain it serves is
structural security — every flow carrying cryptographic identity). No
sub-context split is warranted (DISCUSS scope assessment: 1 context, 0
oversized signals).

**Aggregates / tactical types** (all speaking project newtypes):
- **Certificate roles** as a sum type — `CertRole { Root, Intermediate {
  path_len }, Svid }` — making invalid role/extension combinations
  unrepresentable (per `development.md` § "Type-driven design").
- **`CertSpec`** — the pure aggregate that encodes the cert profile decision
  (which extensions/constraints a role carries). The single-URI-SAN invariant
  lives here as a constructor precondition, not a runtime hope.
- **`RootCaKeyRecordV1`** — the intent aggregate for the root key at rest
  (rkyv envelope payload; persists *inputs*: ciphertext + AEAD params).
- **`IssuedCertificateRowV1`** — the observation row aggregate (audit).

**Context map**: the CA is an *Open Host Service* to its consumers
(control-plane boot, node bootstrap, workload-lifecycle, future #40 rotation
workflow) via the `Ca` port trait. The `tls_bootstrap.rs` ephemeral CA is a
*separate* context (control-plane HTTPS) with no shared model — they coexist.

---

## Wave: DESIGN / [REF] Component Decomposition (crate → responsibility)

See `brief.md` § "built-in-ca extension" → "Component decomposition" for the
full table. Summary:

- **`overdrive-core`** (class `core`, no rcgen): `Ca` trait, `CertSpec`
  builder, `RootCaKeyEnvelope`, `IssuedCertificateRowEnvelope`, `Kek` provider
  port. dst-lint boundary verified — `overdrive-core/Cargo.toml` declares
  `crate_class = "core"`; the gate bans `rand::*`/FFI on its compile path, so
  `rcgen`/`ring` (both pull entropy + FFI) cannot enter.
- **`overdrive-host`** (class `adapter-host`): `RcgenCa` (all rcgen 0.14.8 +
  `ring` HKDF-SHA256/AES-256-GCM), root-key AEAD codec, `SystemdCredsKeyring`
  (`Kek`).
- **`overdrive-sim`** (class `adapter-sim`): `SimCa` (fixture P-256 keys),
  fixture `Kek`.
- **`overdrive-control-plane` / `overdrive-worker`**: boot/issuance wiring
  (`root()` at control-plane boot; `issue_intermediate(node)` at node
  bootstrap; `issue_svid(req)` + audit-row write at workload start).

---

## Wave: DESIGN / [REF] Driving & Driven Ports

**Driving** (inbound triggers): control-plane bootstrap → `root()`; node
bootstrap → `issue_intermediate(node)`; workload-start → `issue_svid(req)`.
**No operator CLI verb** (D-CA-4 preserved); the only operator-observable read
is `issued_certificates` via the existing `alloc status` path. Future #40
rotation workflow is a deferred driving caller.

**Driven** (outbound dependencies, all required constructor params, never
defaulted): `IntentStore` (`RootCaKeyEnvelope`), `ObservationStore`
(`issued_certificates`), `Kek` provider → kernel keyring + systemd-creds,
`Entropy` (serials).

### `Ca` trait surface

```rust
pub trait Ca: Send + Sync {
    fn root(&self) -> Result<RootCaHandle, CaError>;
    fn issue_intermediate(&self, node: &NodeId) -> Result<IntermediateHandle, CaError>;
    fn issue_svid(&self, req: &SvidRequest) -> Result<SvidMaterial, CaError>;
    fn trust_bundle(&self) -> Result<TrustBundle, CaError>;
}
```

Trait-contract rigor (per `development.md` § "Trait definitions specify
behavior"): every method's rustdoc pins preconditions / postconditions /
edge cases / observable invariants. Load-bearing edges to specify:
`issue_svid` single-URI-SAN invariant enforced **by construction**, NOT a
runtime cardinality guard (Option A, ratified 2026-06-06) — `SvidRequest
{ spiffe_id: SpiffeId }` carries exactly one validated identity, so a
zero-or-≥2-SAN request is *unrepresentable* at the adapter (no
`CaError::InvalidSan` branch inside `issue_svid` to reach); the single fallible
parse is the pure-core `CertSpec::svid(Vec<SpiffeId>)` (rejects 0/≥2 with
`CertSpecError`, tested at L1 by S-04-02), and the SPIFFE-spec-mandated runtime
reject (X.509-SVID §5.2) lives at the relying-party verifier (#26), not the
issuer (research:
`docs/research/security/svid-request-cardinality-enforcement-research.md`);
re-issue idempotency (same `SpiffeId` → fresh serial + new validity, distinct
cert); `issue_intermediate` pathLen guarantee (=0, enforced not merely set);
`trust_bundle` composition (root anchor + intermediate as untrusted chain).
**Enforcement**: `crates/<crate>/tests/integration/ca_equivalence.rs` — DST
equivalence test driving `RcgenCa` (host) and `SimCa` (sim) through the same
call sequence, asserting observable equivalence (SVID profile incl. SAN
cardinality covered by S-04-06).

---

## Wave: DESIGN / [REF] Technology Choices

See `brief.md` § "built-in-ca extension" → "Technology choices". All OSS,
all already in-graph: `rcgen` 0.14.8 (`features = ["ring", "pem"]`, MSRV 1.88
— bumped from 0.13.2 and committed in `35958ecb`), crypto provider = **`ring`**
(the workspace provider today; ADR-0039's intended `aws-lc-rs` switch is
**unimplemented**, and FIPS 140-3 Cert #4816 is **contingent on #204** — `ring`
is not FIPS-validated), Linux kernel keyring, systemd-creds, `rkyv` (existing
envelope machinery). No proprietary tech. P-256 (ECDSA) is the research default;
`ring` provides ECDSA P-256 + AES-256-GCM + HKDF-SHA256, so the hierarchy and
root-key envelope work on `ring` today. **Version note**: the `rcgen` 0.14.8
pin is already in place and `mint_ephemeral_ca` is already migrated to the 0.14
builder API (`Issuer::from_params(&params, &key)` + 2-arg `signed_by(&key,
&issuer)`), so there is **no rcgen bump step** in DELIVER. `RcgenCa` confirms
`IsCa::Ca(BasicConstraints::Constrained(0))` / `SanType::URI(Ia5String)` /
`KeyUsagePurpose` against 0.14.8 (all present) at first compile; low risk since
`mint_ephemeral_ca` already compiles the adjacent 0.14.8 APIs.

---

## Wave: DESIGN / [REF] Decisions Table

| # | Decision | Rationale | Source |
|---|---|---|---|
| DES-CA-1 | `Ca` = port trait (core) + `RcgenCa` (host) + `SimCa` (sim) | Mirrors `Clock`/`Transport`/`Driver`; DST-honest; rcgen out of core | ADR-0063 D1; locked Q1 |
| DES-CA-2 | Root key at rest = rkyv `RootCaKeyEnvelope` (ADR-0048) in IntentStore | CA material is intent (linearizable), never observation (whitepaper §4) | ADR-0063 D2; locked Q2 |
| DES-CA-3 | KEK in Linux kernel keyring; systemd-creds delivers per-boot | KEK in kernel space not heap; TPM/host-key root-of-trust; keyrings volatile | ADR-0063 D3; locked Q1b/Q4 |
| DES-CA-4 (**recon A**) | HKDF-SHA256-from-KEK subkey → AES-256-GCM | Passphrase-KDF dropped (KEK is raw); HKDF buys domain-separation + rotation seam for #40/HSM at negligible cost | ADR-0063 D4 |
| DES-CA-5 (**recon B**) | Pure `CertSpec` builder in core; host adapter → `rcgen::CertificateParams` | Single-URI-SAN rejection (K2) becomes DST-testable; sim shares policy; rcgen stays out of core | ADR-0063 D5 |
| DES-CA-6 | Serials via `Entropy`; key-gen via backend CSPRNG (not injectable) | DST-deterministic issuance; key-gen non-injectability acceptable (research F11) | ADR-0063 D7; locked default |
| DES-CA-7 | `issued_certificates` ObservationStore audit row | Internal-CT equivalent; gossiped when #36 lands; no silent issuance | ADR-0063 D6; locked Q5 |
| DES-CA-8 | Refuse-to-start on decrypt failure (typed + `health.startup.refused`) | Silent re-mint orphans every issued identity | ADR-0063 D3; locked default |
| DES-CA-9 | Single-node = exactly one intermediate | Phase 2.6 scope; multi-node owned by #36 | ADR-0063 Context; D-CA-6 |
| DES-CA-10 | Earned-Trust probe: KEK-present + envelope-decrypt + credential-present | wire→probe→use; refuse-to-start on probe failure | ADR-0063 D8 / § Earned Trust |

---

## Wave: DESIGN / [REF] Reuse Analysis (HARD GATE)

Full table with evidence in `brief.md` § "built-in-ca extension" → "Reuse
Analysis". Verdict: **6 REUSE-AS-IS** (`IntentStore`, `ObservationStore`,
`Entropy`, `SpiffeId`, `CertSerial`/`NodeId`, `VersionedEnvelope`/
`codec::envelope`) · **1 REUSE-proven-via-new-adapter** (the rcgen usage in
`mint_ephemeral_ca` de-risks `RcgenCa` — same rcgen 0.14.8 APIs, already
migrated to the 0.14 `Issuer` builder; new structure) ·
**1 LEAVE-AS-IS-distinct-consumer** (`tls_bootstrap.rs` — serves control-plane
HTTPS, NOT workload identity; D-CA-5; Phase 5 / #81 replaces it) ·
**8 CREATE-NEW (justified)** (`Ca` trait, `CertSpec`, `RcgenCa`, `SimCa`,
`Kek` port, `RootCaKeyEnvelope`, `IssuedCertificateRowEnvelope`, AEAD codec —
no existing alternative for any). Reuse-heavy profile is expected: the crypto
stack, state layers, newtypes, and envelope machinery all pre-exist; the
feature is the *composition* behind a new port trait.

Codebase evidence verified this wave (Grep/Read):
- `overdrive-core/Cargo.toml` → `crate_class = "core"`; no rcgen/`ring`/rand
  in `[dependencies]`. dst-lint (`xtask/src/dst_lint.rs`) bans `rand::*` /
  `tokio::net::*` / FFI on core. Boundary constraint confirmed.
- `id.rs` → `SpiffeId` (canonical-lowercase + trust-domain/path accessors),
  `CertSerial(String)` (hex, max-bytes bounded), `NodeId` (validated). All
  reusable.
- `traits/entropy.rs` → `Entropy::fill(&mut [u8])` + `u64()`. Reusable for
  serials.
- `traits/intent_store.rs` → docstring already names "certificates" as
  intent; `IntentStoreError::Envelope` (intent fail-fast) present.
- `codec/envelope.rs` → `VersionedEnvelope`, `decode_envelope_bytes`,
  `probe_known_variant`, `EnvelopeError`. Reusable for both new envelopes.
- `traits/observation_store.rs` → `AllocStatusRow`/`NodeHealthRow`
  alias-to-payload + `…V1` envelope pattern to mirror for
  `issued_certificates`.
- `tls_bootstrap.rs` → `mint_ephemeral_ca` exercises rcgen 0.14.8 `IsCa`,
  `SanType`, `KeyUsagePurpose`, `self_signed` / `Issuer::from_params` + 2-arg
  `signed_by`, P-256 — proven (already migrated to the 0.14 builder API,
  committed `35958ecb`).

---

## Wave: DESIGN / [REF] Wave Decisions Summary (DESIGN)

- **[DES-D-1]** Both reconciliation points resolved in-wave with rationale
  (recon A = HKDF-from-KEK + AES-256-GCM; recon B = pure `CertSpec` in core +
  host translates). No punt.
- **[DES-D-2]** ADR-0063 written (next free number — verified `ls adr-*.md`
  max is 0062; duplicate-numbered files exist at 0054–0058 but 0062 is the
  unambiguous max). Single ADR chosen over multiple: the `Ca` trait, the
  3-tier hierarchy, and the root-key protection scheme are one cohesive
  architecture; splitting would fragment the supersession-of-ADR-0010 record.
- **[DES-D-3]** C4 L1+L2+L3 produced (Mermaid) — L3 warranted by the
  trait→host/sim→IntentStore/ObservationStore/keyring/Entropy complexity.
- **[DES-D-4]** brief.md `## built-in-ca extension` + ADR index row 0063 +
  changelog entry + Status-table row written (architect-only per
  delegate-to-architect rule).
- **[DES-D-5]** No roadmap created — DELIVER owns that.
- **[DES-D-6]** All deferrals cite EXISTING issues (#40/#39, #36, #104/#83,
  #81, Phase 5/7). No inventions. No NEW deferral requiring a new issue
  surfaced.

---

## Wave: DESIGN / [REF] Open Questions → DISTILL / DELIVER

1. **rkyv envelope obligations** (both `RootCaKeyEnvelope`,
   `IssuedCertificateRowEnvelope`): golden-bytes `FIXTURE_V1` +
   empirically-pinned `discriminant_offset_from_end` per ADR-0048 / testing.md
   § "Archive schema-evolution roundtrip". Real DELIVER work.
2. **Earned-Trust fault-injection scenario set** for the probes (tampered
   ciphertext, wrong KEK, absent systemd credential) — DISTILL authors these
   as graduating `O`/`E` expectations + Tier-3 tests.
3. **rcgen 0.14.8 builder/extension API already in place** — the `rcgen = "0.14"`
   (`features = ["ring", "pem"]`) bump and the `mint_ephemeral_ca` migration to
   the 0.14 `Issuer` builder API are **already committed** (`35958ecb`), so there
   is **no rcgen-bump step**. `RcgenCa` confirms the extension APIs
   (`IsCa::Ca(BasicConstraints::Constrained(0))`, `SanType::URI(Ia5String)`,
   `KeyUsagePurpose` — all present in 0.14.8) at first compile. Provider = `ring`
   (aws-lc-rs/FIPS → #204).
4. **`ca_equivalence` DST test** design — the trait-contract enforcement;
   DISTILL specifies the call sequence and observable-equivalence assertions.
5. **Tier-2 expansion recommendations** (NOT auto-expanded; lean default):
   `alternatives-considered` for the AEAD shape (ADR-0063 A1–A6 already cover
   it) and `journey-deep-dive` for the boot error-path map — both optional;
   the ADR + SSOT journey carry the substance.

---

## Wave: DESIGN / [REF] Handoff

- **To DISTILL (nw-acceptance-designer)**: the `Ca` trait contract (pre/post/
  edge/invariants to pin in rustdoc), the `ca_equivalence` DST equivalence
  test, the Earned-Trust fault-injection scenarios, the two rkyv golden-bytes
  fixture obligations, and the KPI mapping (K1 `openssl verify`, K2
  single-URI-SAN rejection, K3 no-plaintext-key byte-scan, K5 DST
  determinism). Operator-surface scenarios (`openssl verify`,
  `issued_certificates` row) graduate to `verification/expectations/`.
- **To DELIVER (nw-software-crafter, OOP)**: ADR-0063 + brief § "built-in-ca
  extension" + C4 L1/L2/L3. Component decomposition is per-crate; the slice
  order (S01→S05) from DISCUSS holds (linear dependency chain). The rcgen 0.14.8
  (`features = ["ring", "pem"]`) pin + the `mint_ephemeral_ca` 0.14 `Issuer`-API
  migration are **already committed** (`35958ecb`) — no rcgen-bump step; provider
  is `ring` (aws-lc-rs/FIPS → #204). `RcgenCa` confirms the 0.14.8 extension
  APIs at first compile in Slice 01.
- **To DEVOPS (nw-platform-architect)**: KPIs K1–K5; the keyring +
  systemd-creds boot dependency (operationalize the per-boot KEK delivery +
  the `OVERDRIVE_CA_KEK` dev-only gate); the refuse-to-start
  (`health.startup.refused`) signal. **No external third-party API
  integrations** in this feature → no consumer-driven contract tests needed
  (the keyring/systemd-creds are OS ABIs probed via Earned-Trust, not network
  services).

---

# DISTILL Wave (wave 5 of 6) · Agent: Quinn (nw-acceptance-designer) · Mode: lean · Density: `lean`

DISTILL content appends below under `## Wave: DISTILL / [REF] <Section>`
headings. Tier-1 `[REF]` sections only (lean default); no Tier-2 expansions
auto-rendered. The executable scenario SSOT is the eight Rust `#[test]`
scaffold files (already authored + wired); the GIVEN/WHEN/THEN specification
companion is `docs/feature/built-in-ca/distill/test-scenarios.md` (spec-only,
never parsed — `.claude/rules/testing.md` § "No `.feature` files anywhere").

**Crypto-backend authority note**: ADR-0063 (Accepted 2026-06-05, the latest
artifact) fixes the backend as **`ring`** (rcgen 0.14.8, `features = ["ring",
"pem"]`, P-256); the `aws-lc-rs` switch + FIPS are deferred to **#204**.
Earlier DISCUSS prose saying `aws_lc_rs`/`rcgen 0.13` is **superseded** by
ADR-0063 (documented in its changelog). The scaffolds + this DISTILL section
follow ADR-0063. This is a recorded supersession, not an unresolved
reconciliation contradiction (see § Wave: DISTILL / [REF] Reconciliation).

---

## Wave: DISTILL / [REF] Reconciliation (Pre-Scenario HARD GATE)

Wave-decisions are embedded in this single-file delta (DISCUSS § Wave
Decisions D-CA-1..6; DESIGN § Wave Decisions Summary DES-D-1..6 + Decisions
Table DES-CA-1..10; ADR-0063 D1..D8). No standalone `wave-decisions.md` files
exist. No DEVOPS wave section exists (single-file model — KPI instrumentation
lives in DISCUSS § Outcome KPIs) → **WARN, proceed**.

**Contradiction scan — DISCUSS ↔ DESIGN ↔ (no DEVOPS):**

| Decision area | DISCUSS | DESIGN / ADR-0063 | Verdict |
|---|---|---|---|
| Crypto backend | `aws_lc_rs` / `rcgen 0.13` (§ System Constraints) | `ring` / `rcgen 0.14.8`, aws-lc-rs→#204 (ADR-0063 Constraints + changelog) | **Documented supersession**, not a live contradiction — ADR-0063's changelog explicitly records the correction ("No architecture decision altered"). Scaffolds encode `ring`. |
| Single-node scope | D-CA-6 single-node, one intermediate | DES-CA-9 single-node, one intermediate | CONSISTENT |
| No operator CLI verb | D-CA-4 (no verb; audit row only) | DESIGN Driving Ports (no verb; `issued_certificates` only) | CONSISTENT |
| State layers | CA material = intent; audit = observation | D2 (intent) + D6 (observation) | CONSISTENT |
| Rotation OUT | D-CA-3 → #40 (needs #39) | DES-CA-1 / ADR refs → #40/#39 | CONSISTENT |

**Result: Reconciliation passed — 0 unresolved contradictions** (1 documented
supersession on the crypto backend). Scenario writing proceeded.

---

## Wave: DISTILL / [REF] Scenario List with Tags

37 scenarios across the 5 slices (was 39; S-04-09 + S-04-10 RETIRED 2026-06-06
under Option A — see the Slice-04 note) (= 5 user stories = the linear
trust-hierarchy dependency chain). Full GIVEN/WHEN/THEN + Universe +
per-scenario trace: `distill/test-scenarios.md`. IDs are `S-0S-NN`
(slice-scoped). The `@S-0S` tags *inside* the scaffolds denote the owning slice
(one tag = one slice), not per-scenario unique IDs.

### Slice 01 — Root CA behind the `Ca` port trait (US-CA-01) — `ca_cert_spec_policy.rs` + `sim_ca_deterministic.rs` + `ca_equivalence.rs` + `rcgen_ca_chain_verify.rs`

| ID | Scaffold fn | Layer | Tags |
|---|---|---|---|
| S-01-01 | `root_spec_is_self_signed_ca_with_key_cert_sign_and_crl_sign` | L1 pure | `@in-memory @S-01` |
| S-01-02 | `sim_ca_root_is_bit_identical_across_two_runs_at_same_seed` | L2 sim | `@in-memory @S-01` |
| S-01-03 | `ca_equivalence_root_profile_matches_across_host_and_sim` | L3 | `@real-io @adapter-integration @S-01` |
| S-01-04 | `rcgen_root_is_a_valid_self_signed_ca_via_openssl_verify` | L3 | `@real-io @adapter-integration @S-01` |
| S-01-05 | `cert_spec_error_variants_are_distinct_per_failure_mode` | L1 pure | `@in-memory @error @S-01 @S-04` |

### Slice 02 — Root key envelope-encrypted at rest (US-CA-02) — `rcgen_ca_root_key_envelope.rs` + `ca_boot_and_audit.rs` + `schema_evolution/root_ca_key.rs`

| ID | Scaffold fn | Layer | Tags |
|---|---|---|---|
| S-02-01 | `root_key_envelope_seals_and_opens_round_trip_under_same_kek` | L3 | `@real-io @adapter-integration @S-02` |
| S-02-02 | `root_key_envelope_contains_no_plaintext_key_bytes` | L3 | `@real-io @adapter-integration @S-02` |
| S-02-03 | `root_key_envelope_tampered_ciphertext_fails_distinct_from_wrong_kek` | L3 | `@real-io @adapter-integration @S-02 @error` |
| S-02-04 | `root_key_envelope_wrong_kek_fails_distinct_from_tampered` | L3 | `@real-io @adapter-integration @S-02 @error` |
| S-02-05 | `root_ca_is_reused_across_control_plane_restart` | L3 | `@real-io @adapter-integration @S-02` |
| S-02-06 | `boot_refuses_to_start_on_envelope_decrypt_failure_without_remint` | L3 | `@real-io @adapter-integration @S-02 @error` |
| S-02-07 | `boot_refuses_to_start_when_kek_absent_from_keyring` | L3 | `@real-io @adapter-integration @S-02 @error` |
| S-02-08 | `root_ca_key_envelope_v1_golden_bytes_roundtrip` | L1 archive | `@property @S-02` |
| S-02-09 | `root_ca_key_envelope_discriminant_offset_triangulates` | L1 archive | `@property @S-02` |
| S-02-10 | `root_ca_key_envelope_unknown_version_probe_surfaces_error` | L1 archive | `@property @S-02 @error` |

### Slice 03 — Per-node intermediate CA, pathLen=0 (US-CA-03) — `ca_cert_spec_policy.rs` + `sim_ca_deterministic.rs` + `ca_equivalence.rs` + `rcgen_ca_chain_verify.rs` + `ca_boot_and_audit.rs`

| ID | Scaffold fn | Layer | Tags |
|---|---|---|---|
| S-03-01 | `intermediate_spec_is_ca_true_with_path_len_zero_and_key_cert_sign` | L1 pure | `@in-memory @S-03` |
| S-03-02 | `sim_ca_intermediate_is_deterministic_and_chains_to_fixture_root` | L2 sim | `@in-memory @S-03` |
| S-03-03 | `ca_equivalence_intermediate_profile_matches_across_host_and_sim` | L3 | `@real-io @adapter-integration @S-03` |
| S-03-04 | `rcgen_intermediate_chains_to_root_via_openssl_verify` | L3 | `@real-io @adapter-integration @S-03` |
| S-03-05 | `rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced` | L3 | `@real-io @adapter-integration @S-03 @error` |
| S-03-06 | `intermediate_signing_failure_fails_node_bootstrap_loudly` | L3 | `@real-io @adapter-integration @S-03 @error` |

### Slice 04 — Workload SVID, single URI SAN (US-CA-04) — `ca_cert_spec_policy.rs` + `sim_ca_deterministic.rs` + `ca_equivalence.rs` + `rcgen_ca_chain_verify.rs`

| ID | Scaffold fn | Layer | Tags |
|---|---|---|---|
| S-04-01 | `svid_spec_carries_exactly_one_uri_san_and_leaf_key_usage` | L1 pure (PBT) | `@in-memory @property @S-04` |
| S-04-02 | `svid_spec_rejects_zero_or_multiple_uri_sans_before_any_cert` | L1 pure (PBT) | `@in-memory @property @S-04 @error` |
| S-04-03 | `svid_spec_subject_uri_equals_requested_spiffe_id` | L1 pure | `@in-memory @S-04` |
| S-04-04 | `sim_ca_svid_serial_is_deterministic_and_at_least_64_bits` | L2 sim | `@in-memory @S-04` |
| S-04-05 | `sim_ca_svid_carries_single_uri_san_and_is_not_a_ca` | L2 sim | `@in-memory @S-04` |
| S-04-06 | `ca_equivalence_svid_profile_matches_across_host_and_sim` | L3 | `@real-io @adapter-integration @S-04` |
| S-04-07 | `rcgen_full_svid_chain_verifies_root_intermediate_svid` | L3 | `@real-io @adapter-integration @walking_skeleton @S-04` |
| S-04-08 | `rcgen_svid_leaf_carries_exactly_one_uri_san_and_leaf_profile` | L3 | `@real-io @adapter-integration @S-04` |
| ~~S-04-09~~ | ~~`rcgen_svid_request_with_bad_san_cardinality_is_rejected_pre_issuance`~~ | ~~L3~~ | **RETIRED 2026-06-06** |
| ~~S-04-10~~ | ~~`ca_equivalence_bad_san_request_rejected_identically_by_both`~~ | ~~L3~~ | **RETIRED 2026-06-06** |

> **S-04-09 / S-04-10 RETIRED (2026-06-06; Option A — type-enforced).** Both
> tested the adapter rejecting a bad-SAN-cardinality `SvidRequest` — a path the
> request type (`SvidRequest { spiffe_id: SpiffeId }`, one validated identity by
> construction) makes **unreachable**. Under the ratified Option A
> (enforcement-location pinned in ADR-0063 D5: type makes ≠1 unrepresentable →
> `CertSpec::svid` is the single fallible parse → verifier #26 owns the runtime
> reject per SPIFFE X.509-SVID §5.2), these are redundant: **S-04-08** already
> asserts the host leaf carries exactly one URI SAN, **S-04-06** already asserts
> cross-adapter SVID-profile equivalence including SAN cardinality, and
> **S-04-02** tests the live `CertSpec::svid` 0/≥2 reject at L1. The
> spec-mandated runtime reject is at the relying-party verifier (#26), out of
> this feature's scope. Research:
> `docs/research/security/svid-request-cardinality-enforcement-research.md`
> (SPIFFE §2/§5.2; SPIRE single-`spiffeid.ID` reference impl; "parse, don't
> validate"). The crafter retires the two scaffold fns; the rows are kept struck
> for traceability, not deleted.

### Slice 05 — Trust bundle, audit, re-issue (US-CA-05) — `sim_ca_deterministic.rs` + `ca_equivalence.rs` + `ca_boot_and_audit.rs` + `schema_evolution/issued_certificate_row.rs`

| ID | Scaffold fn | Layer | Tags |
|---|---|---|---|
| S-05-01 | `sim_ca_reissue_for_same_spiffe_id_yields_a_fresh_distinct_leaf` | L2 sim | `@in-memory @S-05` |
| S-05-02 | `ca_equivalence_trust_bundle_shape_matches_across_host_and_sim` | L3 | `@real-io @adapter-integration @S-05` |
| S-05-03 | `issuance_writes_issued_certificates_row_matching_the_minted_cert` | L3 | `@real-io @adapter-integration @S-05` |
| S-05-04 | `issuance_that_cannot_write_audit_row_surfaces_an_error` | L3 | `@real-io @adapter-integration @S-05 @error` |
| S-05-05 | `svid_is_reissued_on_demand_without_control_plane_restart` | L3 | `@real-io @adapter-integration @S-05` |
| S-05-06 | `issued_certificate_row_envelope_v1_golden_bytes_roundtrip` | L1 archive | `@property @S-05` |
| S-05-07 | `issued_certificate_row_envelope_discriminant_offset_triangulates` | L1 archive | `@property @S-05` |
| S-05-08 | `issued_certificate_row_envelope_unknown_version_probe_surfaces_error` | L1 archive | `@property @S-05 @error` |

**Coverage profile**: **37 total** (was 39; S-04-09 + S-04-10 RETIRED 2026-06-06
under Option A — see the Slice-04 note above) · **13 `@error` (35.1%)** (was
15/39 = 38.5%; both retired scenarios were `@error`, so the ratio drops to
13/37 — a **non-gating DISTILL metric**, accepted as a consequence of the
type-honest design: the bad-cardinality path the two `@error` scenarios tested
is unrepresentable, so the honest scenario count is lower) · 1
`@walking_skeleton` · 8 `@property` · by layer: L1 pure 6, L2 sim 5, L1 archive
6, **L3 real-io 20** (was 22). (The Slice-01 table lists 5 rows because
`cert_spec_error_variants...` carries `@S-01 @S-04` — it is filed under Slice 01
as the policy-taxonomy guard and also serves the K2 single-URI invariant. It is
one of the 13 `@error` scenarios, not a 14th.)

---

## Wave: DISTILL / [REF] Walking Skeleton Strategy

**Per-project Architecture of Reference + Project Infrastructure Policy** (not
a per-feature A/B/C/D choice — the 4-way strategy is retired). The walking
skeleton is realised across **Slices 01→04** (root → persist → intermediate →
SVID → **chain verifies**), per DISCUSS § Story Map. The single
`@walking_skeleton`-tagged scenario is **S-04-07**
(`rcgen_full_svid_chain_verifies_root_intermediate_svid`): `openssl verify
-CAfile root.pem -untrusted intermediate.pem svid.pem` → exit 0.

**Litmus (Dim 5)**: Sam the security engineer runs `openssl verify` himself
and sees the workload identity validate to the root — a genuine user-observable
outcome, framed as a user goal (not "all layers connect"). The
walking-skeleton scenario uses **real adapters** (real `ring`/rcgen crypto,
real `openssl verify` subprocess), gated `integration-tests`, run via Lima per
`.claude/rules/testing.md` — i.e. `@real-io`, not `@in-memory`.

**No operator CLI verb** (D-CA-4): there is no `overdrive` subcommand to
"issue an SVID". `openssl verify` over the minted material is the honest
external entry point this phase (per the DISCUSS elevator-pitch caveat). The
only operator read surface is the `issued_certificates` observation row via the
existing `alloc status` path (S-05-03).

---

## Wave: DISTILL / [REF] Adapter Coverage Table (Mandate 6)

Every driven adapter / driven port has at least one real-I/O scenario (or, for
the in-memory sim tier, the DST-determinism + equivalence coverage that proves
the sim honours the same `Ca` trait contract). No `NO — MISSING` rows.

| Adapter / driven port | Real-I/O coverage | Covered by |
|---|---|---|
| `RcgenCa` (host adapter, real `ring`/rcgen X.509) | YES (`@real-io`) | S-01-04, S-03-04, S-03-05, S-04-07, S-04-08 (`rcgen_ca_chain_verify.rs` via `openssl verify`; S-04-09 RETIRED 2026-06-06 — the bad-cardinality adapter path is type-unreachable under Option A) |
| Root-key AEAD codec (real HKDF-SHA256 + AES-256-GCM via `ring`) | YES (`@real-io`) | S-02-01..S-02-04 (`rcgen_ca_root_key_envelope.rs`, byte-scan + tamper/wrong-KEK) |
| `SimCa` (sim adapter, fixture P-256 keys + `SeededEntropy`) | in-memory (`@in-memory`) + cross-adapter equivalence | S-01-02, S-03-02, S-04-04, S-04-05, S-05-01 (`sim_ca_deterministic.rs`); equivalence S-01-03/S-03-03/S-04-06/S-05-02 (S-04-10 RETIRED 2026-06-06 — SAN-cardinality equivalence covered by S-04-06 under Option A) |
| `IntentStore` (`LocalStore` over redb — root-key envelope persistence) | YES (`@real-io`) | S-02-05 (reuse across restart), S-02-06 (refuse-to-start on decrypt failure) (`ca_boot_and_audit.rs`) |
| `ObservationStore` (`issued_certificates` audit row) | YES (`@real-io`) | S-05-03 (read-back match), S-05-04 (no silent issuance) (`ca_boot_and_audit.rs`) |
| `Kek` provider → kernel keyring + systemd-creds (Earned-Trust probe) | YES (`@real-io`) | S-02-07 (absent KEK refuses startup) (`ca_boot_and_audit.rs`) |
| `Entropy` port (serials) | exercised indirectly | S-04-04 (serial determinism via `SeededEntropy`) — `Entropy` is a pre-existing reused port, not a new adapter |
| rkyv envelopes (`RootCaKeyEnvelope`, `IssuedCertificateRowEnvelope`) | default-lane archive | S-02-08..10, S-05-06..08 (`schema_evolution/*.rs`, golden-bytes per ADR-0048) |

**`Ca` trait-contract enforcement** (development.md § "DST equivalence test is
the structural guard"): `ca_equivalence.rs` drives `RcgenCa` (host) and `SimCa`
(sim) through the same call sequence and asserts observable equivalence via
trait accessors (S-01-03, S-03-03, S-04-06, S-05-02; S-04-10 RETIRED
2026-06-06 — SAN-cardinality equivalence is covered by S-04-06's
SVID-profile equivalence under Option A). This is the central guard that the
sim adapter does not diverge on policy from the host adapter — the
highest-value structural test in the feature.

---

## Wave: DISTILL / [REF] Scaffolds (RED-ready)

Eight Rust scaffold files, all RED-at-the-bar via
`#[should_panic(expected = "RED scaffold")]` (`.claude/rules/testing.md` §
"RED scaffolds" — the project's Rust convention; the Python `__SCAFFOLD__`
marker is N/A here). Each scaffold body is a self-contained `panic!` that names
the scenario + the DELIVER GREEN target; **no scaffold imports unbuilt
production types**, so nextest reports PASS (expected panic), clippy is clean,
and lefthook needs no `--no-verify`.

| Scaffold file | Crate · class | Layer · lane | Scenarios |
|---|---|---|---|
| `tests/acceptance/ca_cert_spec_policy.rs` | overdrive-core · core | L1 pure · default | S-01-01, S-03-01, S-04-01/02/03, + error-variant distinctness |
| `tests/acceptance/sim_ca_deterministic.rs` | overdrive-sim · adapter-sim | L2 sim · default | S-01-02, S-03-02, S-04-04/05, S-05-01 |
| `tests/integration/rcgen_ca_chain_verify.rs` | overdrive-host · adapter-host | L3 · `integration-tests` | S-01-04, S-03-04/05, S-04-07/08 (S-04-09 scaffold fn retired by crafter 2026-06-06) |
| `tests/integration/rcgen_ca_root_key_envelope.rs` | overdrive-host · adapter-host | L3 · `integration-tests` | S-02-01/02/03/04 |
| `tests/integration/ca_equivalence.rs` | overdrive-control-plane | L3 · `integration-tests` | S-01-03, S-03-03, S-04-06, S-05-02 (S-04-10 scaffold fn retired by crafter 2026-06-06) |
| `tests/integration/ca_boot_and_audit.rs` | overdrive-control-plane | L3 · `integration-tests` | S-02-05/06/07, S-03-06, S-05-03/04/05 |
| `tests/schema_evolution/root_ca_key.rs` | overdrive-core · core | L1 archive · default | S-02-08/09/10 |
| `tests/schema_evolution/issued_certificate_row.rs` | overdrive-core · core | L1 archive · default | S-05-06/07/08 |

**Wired entrypoints** (already modified, verified this wave):
`overdrive-core/tests/acceptance.rs` (`mod ca_cert_spec_policy`),
`overdrive-sim/tests/acceptance.rs` (`mod sim_ca_deterministic`),
`overdrive-core/tests/schema_evolution.rs` (`mod root_ca_key`, `mod
issued_certificate_row`), `overdrive-control-plane/tests/integration.rs` (`mod
ca_boot_and_audit`, `mod ca_equivalence`). `overdrive-host` newly gained
`integration-tests = []` + `tests/integration.rs` + `tests/integration/`
(`mod rcgen_ca_chain_verify`, `mod rcgen_ca_root_key_envelope`).

---

## Wave: DISTILL / [REF] Test Placement

Rust convention per `.claude/rules/testing.md` § "Layout" + ADR-0005, with the
precedent justification already inline in each entrypoint:

- **L1 pure / archive** (default lane, no `integration-tests`): `overdrive-core
  tests/acceptance/ca_cert_spec_policy.rs` and `tests/schema_evolution/*.rs`.
  `CertSpec` is pure core policy (dst-lint-clean), so its tests run in the
  default lane; the schema-evolution golden-bytes tests are pure in-memory rkyv
  (testing.md § "Archive schema-evolution roundtrip" mandates one
  golden-fixture per rkyv envelope).
- **L2 sim** (default lane): `overdrive-sim tests/acceptance/
  sim_ca_deterministic.rs` — `SimCa` is in-process, no real I/O.
- **L3 real-io** (`integration-tests` feature, Lima): host-adapter crypto +
  `openssl verify` (`overdrive-host tests/integration/`); boot/issuance wiring
  + equivalence (`overdrive-control-plane tests/integration/`). The
  `ca_equivalence` test lives in `overdrive-control-plane` because it is the
  **only** crate that dev-deps BOTH `overdrive-host` (`RcgenCa`) and
  `overdrive-sim` (`SimCa`) — host and sim do not depend on each other (the
  sim/host split is load-bearing per CLAUDE.md), so the equivalence harness has
  no other natural home (justification inline in `integration.rs`).

---

## Wave: DISTILL / [REF] Driving Adapter Coverage

**No operator CLI verb, no HTTP endpoint, no hook adapter** for CA issuance
this phase (D-CA-4 / DESIGN Driving Ports). The driving *ports* are internal:
control-plane bootstrap → `root()`; node bootstrap → `issue_intermediate(node)`;
workload-start → `issue_svid(req)`. These are exercised via the in-process
boot/issuance wiring in `ca_boot_and_audit.rs` (real `IntentStore` /
`ObservationStore` / keyring through the composition root), not via a subprocess
CLI invocation — because there is no CLI surface to invoke. The
RCA-fix "every CLI/endpoint/hook in DESIGN has a subprocess/HTTP/hook scenario"
gate is **vacuously satisfied** (zero such surfaces in DESIGN).

The single user-observable *external* entry point is `openssl verify` (the
walking-skeleton S-04-07 + S-01-04/S-03-04 run it as a real subprocess) and the
`issued_certificates` observation row via the existing `alloc status` path
(S-05-03; graduated to verification expectation O-CA-04). These are the honest
operator/security-reviewer surfaces and are covered by `@real-io` scenarios.

---

## Wave: DISTILL / [REF] Pre-requisites

- **DESIGN driving ports**: `Ca` trait (`root`/`issue_intermediate`/
  `issue_svid`/`trust_bundle`), `CertSpec` pure builder, `RootCaKeyEnvelope` /
  `IssuedCertificateRowEnvelope` (ADR-0063 D1/D2/D5/D6) — all CREATE-NEW in
  DELIVER; the scaffolds specify their observable surface.
- **Reused (present today)**: `SpiffeId`, `CertSerial`, `NodeId`, `Entropy`
  port, `IntentStore` (`LocalStore`), `ObservationStore` /
  `SimObservationStore`, `VersionedEnvelope` / `codec::envelope` (DISCUSS
  pre-reqs + DESIGN Reuse Analysis — all confirmed in-graph).
- **Crypto stack**: `rcgen` (bump 0.13.2 → 0.14.8, `features = ["ring", "pem"]`
  — a DELIVER first-compile gate per ADR-0063 Consequences), `ring` (workspace
  provider today), Linux kernel keyring + systemd-creds (host adapter,
  Linux-only production path).
- **DEVOPS environment matrix**: none authored (single-file model). The
  effective test environment is the project default — Lima VM for all `@real-io`
  scenarios (`cargo xtask lima run -- cargo nextest run ... --features
  integration-tests`); default lane (macOS host or Linux) for L1/L2 scenarios.
  Tampered-ciphertext / wrong-KEK / absent-credential are the in-test
  fault-injection equivalents of an environment matrix (Mandate 4 environmental
  realism), enumerated as example-based sad paths (Mandate 11).
- **rcgen `ring` feature non-conflict** with workspace `rustls`/`ring`:
  first-compile check in DELIVER Slice 01 (research Gap 3), not a spike.

---

## Wave: DISTILL / [REF] Mandate Compliance Evidence

- **CM-A (Mandate 1 — hexagonal boundary)**: tests enter through the `Ca`
  driving port (and the boot/issuance composition root), never internal
  components. The scaffolds import no unbuilt internals; at GREEN, L1 tests
  call the pure `CertSpec` public API (its own driving port per
  nw-tdd-methodology), L2/L3 call `SimCa`/`RcgenCa` via the `Ca` trait.
- **CM-B (Mandate 2 / Pillar 1 — business language)**: scenario titles + the
  GIVEN/WHEN/THEN companion speak the security domain (root, intermediate,
  SVID, SAN, trust bundle, chain verify). Technical detail (rcgen, AES-GCM,
  redb) is confined to scaffold bodies / Universe notes, not scenario surfaces.
- **CM-C (Mandate 3 — journey completeness)**: each scenario is a complete
  unit of behaviour with an observable outcome (a cert that verifies, a refused
  startup, an audit row, a rejected request) — not an isolated technical op.
  WS counts: 1 walking skeleton + 36 focused (37 total; was 1 + 38 before the
  2026-06-06 S-04-09/S-04-10 retirement) (within the 2-3 WS / 15-20 focused
  guidance, scaled to a 5-slice security primitive).
- **CM-E (Mandate 8 — Universe)**: every state-mutating scenario declares its
  port-exposed Universe in `test-scenarios.md`; the Rust workspace satisfies
  the universe-bound discipline natively via exact-equality / byte-scan /
  set-equality fail-closed assertions (per `docs/architecture/
  atdd-infrastructure-policy.md` § Mandate-8 mapping — no Python
  `assert_state_delta` port).
- **CM-F (Mandate 9 — layer-dependent PBT)**: PBT-full (`proptest!`) at GREEN
  is confined to L1 pure scenarios (S-04-01/02 single-URI-SAN property +
  rejection property; the schema-evolution `@property` tags denote
  golden-bytes invariants, not generative PBT). All L3 scenarios are
  example-only.
- **CM-G (Mandate 10 — two-tier)**: **Tier A only**. No Tier B
  state-machine PBT file. Rationale: the CA is a config/issuance-shaped
  primitive — each slice's journey is 1-2 chained scenarios over a hierarchy,
  not a ≥3-chained rich-input journey with a state-machine *model*. The
  `ca_equivalence` cross-adapter contract test covers the "do both adapters
  agree" space that a Tier-B exploration would otherwise probe.
- **CM-H (Mandate 11 — example-based sad paths at L3)**: all 11 L3 `@error`
  scenarios (was 13 before the 2026-06-06 S-04-09/S-04-10 retirement) are named
  example-based tests (tampered ciphertext, wrong KEK, absent KEK, pathLen
  escalation, audit-write failure, decrypt-failure refuse-to-start), one example
  per failure mode from the SSOT journey `error_paths` + ADR-0063 Earned-Trust.
  No PBT machinery at L3. The bad-SAN-cardinality sad path is **not** an L3
  example under Option A — it is type-unreachable at the adapter; the live
  reject is the L1 `CertSpec::svid` parse (S-04-02, `@property`), and the
  spec-mandated runtime reject is at the verifier (#26).

---

## Wave: DISTILL / [REF] Verification Catalogue (EDD graduation)

Operator/qualitative-surface scenarios graduate to
`verification/expectations/` (per `.claude/rules/verification.md`). In-process
logic stays in the test tiers (not duplicated). Four expectations authored
(`pending` — evidence captured in DELIVER/DEVOPS against the built binary in
Lima):

| ID | Surface | Anchor | What it pins |
|---|---|---|---|
| `E03-ca-full-chain-verifies` | E (end-to-end) | S-04-07, ADR-0063 D1, KPI K1 | The full Root → Intermediate → SVID chain verifies under `openssl verify` |
| `O04-ca-refuse-to-start-actionable-error` | O (operator CLI) | S-02-06/07, ADR-0063 D3/Earned-Trust, journey error_paths step 1 | The control plane refuses to start on root-key decrypt failure with an *actionable* cause-distinct error (not a cryptic panic), and does not silently re-mint |
| `O05-ca-issued-certificates-audit-row` | O (operator CLI) | S-05-03/04, ADR-0063 D6, journey step 4 | The `issued_certificates` audit row is observable via the existing `alloc status` path (serial / spiffe_id / issuer); no silent issuance |
| `D01-ca-root-key-never-plaintext-at-rest` | D (kernel/disk-observable) | S-02-02, ADR-0063 D2/D4, KPI K3 | The persisted root-key blob contains zero plaintext private-key bytes (byte-scan of the IntentStore file) |

These four are the operator/qualitative slice the four test tiers under-serve;
the in-process logic (CertSpec policy, sim determinism, envelope roundtrip,
equivalence) stays in the tiers and is NOT duplicated as expectations.

---

## Wave: DISTILL / [REF] Outcomes Registered

Per `nw-distill` § "Register Outcomes" (D-5 per-typed-contract grain). The
registry at `docs/product/outcomes/registry.yaml` was created this wave (did
not exist). Five OUT rows for the feature's new typed contract surface:

| OUT id | kind | Contract |
|---|---|---|
| OUT-CA-ROOT | operation | `Ca::root() -> RootCaHandle` |
| OUT-CA-INTERMEDIATE | operation | `Ca::issue_intermediate(&NodeId) -> IntermediateHandle` |
| OUT-CA-SVID | operation | `Ca::issue_svid(&SvidRequest) -> SvidMaterial` |
| OUT-CA-TRUST-BUNDLE | operation | `Ca::trust_bundle() -> TrustBundle` |
| OUT-CA-SINGLE-URI-SAN | invariant | every SVID carries exactly one `spiffe://` URI SAN, by construction (`SvidRequest { spiffe_id: SpiffeId }`); the single fallible parse is `CertSpec::svid(Vec<SpiffeId>)` (rejects 0/≥2 with `CertSpecError`); the runtime reject is at the relying-party verifier (#26, SPIFFE X.509-SVID §5.2). (Option A — clarified 2026-06-06.) |

`nwave-ai outcomes check-delta` clean (no collisions against the fresh
registry).

---

## Wave: DISTILL / [REF] Self-Review

Against the DISTILL self-review checklist + critique Dimensions 1-9:

- **WS strategy declared** (this section) + WS scenario tagged `@walking_skeleton`
  with `@real-io` (S-04-07) — Dim 9a/9b PASS.
- **Every driven adapter has real-I/O (or sim-equivalence) coverage** — adapter
  table, zero `MISSING` — Dim 9c PASS.
- **Scaffolds RED-not-BROKEN** — all 37 (was 39 before the 2026-06-06
  S-04-09/S-04-10 retirement) `#[should_panic(expected = "RED scaffold")]`, none
  import unbuilt types; nextest PASS on default-lane, L3 compile-clean —
  Mandate 7 PASS.
- **Business-language purity** (Dim 3 / Pillar 1) — scenario surfaces carry no
  raw HTTP/JSON/DB jargon; crypto domain terms (SAN, CA, pathLen) are the
  ubiquitous language, not implementation leakage — PASS.
- **Observable-behavior assertions** (Dim 7) — Universes are port-exposed
  (trait accessors, observation rows, `openssl verify` exit, byte-scan), never
  internal struct fields — PASS.
- **Traceability** (Dim 8) — every US-CA-0N has ≥4 scenarios; K1-K5 mapped
  (K4 = architecture-review, no executable scenario) — Check A PASS; no DEVOPS
  environments file (Check B → fault-injection sad paths substitute) — WARN.
- **Deferrals cite real issues** — #40/#39 (rotation/workflow), #36 (multi-node),
  #204 (aws-lc-rs/FIPS), #104/#83 (multi-region), #81/Phase 5/7. No inventions.

**Finding (reported, not auto-fixed)**: `@error` ratio = **13/37 = 35.1%**
(was 15/39 = 38.5% before the 2026-06-06 S-04-09/S-04-10 retirement; both
retired scenarios were `@error`), under the 40% Dim-1 target. This is a
**non-gating DISTILL metric**, and the drop is an accepted consequence of the
type-honest Option-A design: the bad-SAN-cardinality path the two `@error`
scenarios tested is *unrepresentable* at the adapter, so removing them makes the
scenario count honest rather than padding it with a dead-code test. The eight
scaffold files remain the authored SSOT (two fns retired by the crafter).
Surfaced to the orchestrator. (Doc-completeness note: an earlier draft mapped
only 38 of the then-39 scaffolds; the cross-role `cert_spec_error_variants`
guard is documented as S-01-05 — it is one of the 13 surviving `@error`
scenarios.)

**Verdict**: DISTILL artifacts complete; scenarios sound and traceable;
scaffolds RED-at-the-bar; outcomes + verification expectations authored. Ready
for the final 4-reviewer gate (orchestrator-dispatched) and DELIVER handoff.

---

## Wave: DISTILL / [REF] Handoff

- **To DELIVER (nw-software-crafter, OOP)**: 37 RED scaffolds across 8 files
  (was 39; S-04-09 + S-04-10 RETIRED 2026-06-06 under Option A — the crafter
  retires those two scaffold fns) (the executable SSOT) +
  `distill/test-scenarios.md` (the GWT + Universe contract). Implement GREEN
  bottom-up along the linear chain S-01 → S-05. **`issue_svid` contract (Option
  A)**: the single-URI-SAN invariant is honored by construction — `SvidRequest`
  carries one validated `SpiffeId`, so there is NO `CaError::InvalidSan` branch
  inside `issue_svid`; the fallible parse is `CertSpec::svid` (L1, S-04-02), and
  the runtime reject is the verifier (#26). Apply the corrected `Ca::issue_svid`
  rustdoc (Preconditions/Postconditions/Edge-cases/Observable-invariants
  specified in the architect handoff). The `rcgen` 0.14.8 (`features = ["ring",
  "pem"]`) pin + `mint_ephemeral_ca` 0.14-builder migration are already
  committed (`35958ecb`) — no rcgen-bump step. Two rkyv envelopes carry the
  golden-bytes `FIXTURE_V1` + `discriminant_offset_from_end` obligation
  (ADR-0048). PBT-full (`proptest!`) only at L1 (S-04-01/02). All L3 sad paths
  example-based.
- **To DEVOPS (nw-platform-architect)**: capture the four EDD expectations
  (E04, O04, O05, D04) against the built binary in Lima; instrument K1
  (`openssl verify` rate), K2 (single-URI-SAN rejection), K3 (no-plaintext-key
  byte-scan), K5 (DST determinism). The keyring + systemd-creds per-boot KEK
  delivery + the `OVERDRIVE_CA_KEK` dev gate are the boot dependencies.
- **Reviewer gate**: orchestrator dispatches the final 4-reviewer parallel gate
  (Eclipse/Architect/Forge/Sentinel) against the full feature-delta; this
  DISTILL section + the scaffolds + `test-scenarios.md` are Sentinel's scope.

---

# DELIVER Wave (wave 6 of 6) · Agent: Morgan (nw-solution-architect) · back-propagation pass

DELIVER content that feeds back into the SSOT appears under
`## Wave: DELIVER / [WHY] <Section>` headings. This is a contract
back-propagation pass: DELIVER revealed a contradiction in the `issue_svid`
contract; the user ratified the resolution (Option A); this section records WHY
and what was retired, so the next reader does not re-open the decision.

---

## Wave: DELIVER / [WHY] Upstream Issues

### U-CA-1 — `issue_svid` single-URI-SAN: type-unreachable runtime guard (RESOLVED, Option A, 2026-06-06)

**The contradiction DELIVER surfaced.** While implementing Slice 04, the crafter
hit a contract contradiction between the artifacts and the type. The
`Ca::issue_svid` rustdoc (in `crates/overdrive-core/src/traits/ca.rs`), the
brief `Ca` trait-contract prose, and the DESIGN `Ca` trait-surface note all
claimed `issue_svid` "rejects zero or two-or-more URI SANs with
`CaError::InvalidSan` before any cert is produced." But the request type is
`SvidRequest { spiffe_id: SpiffeId }` — exactly **one** validated identity by
construction. There is no way to hand `issue_svid` a zero-or-≥2-SAN request, so
the documented `CaError::InvalidSan` branch is **unreachable** — an
aspirational-doc bug (`development.md` § "No aspirational docs. Never document
behaviour that is not implemented"). Two DISTILL scenarios were written to test
this unreachable path:
- **S-04-09** `rcgen_svid_request_with_bad_san_cardinality_is_rejected_pre_issuance`
  (host adapter, L3, `@error`)
- **S-04-10** `ca_equivalence_bad_san_request_rejected_identically_by_both`
  (cross-adapter equivalence, L3, `@error`)

Neither can be written without first *widening* the request type to re-admit the
invalid cardinality — i.e. they presuppose the very design (Option B) the user
did not choose.

**The user's decision (APPROVED 2026-06-06): Option A — type-enforced.** The
single-URI-SAN invariant is enforced at three semantically-distinct layers, none
of which is a runtime cardinality guard inside `issue_svid`:
1. the request **type** (`SvidRequest { spiffe_id: SpiffeId }`) makes ≠1
   unrepresentable;
2. the pure **parse** `CertSpec::svid(Vec<SpiffeId>)` is the single fallible
   boundary (stays fallible, rejects 0/≥2 with `CertSpecError` — tested green at
   L1 by **S-04-02**);
3. the **relying-party verifier** (#26 sockops/kTLS) is the SPIFFE-spec-mandated
   runtime reject (X.509-SVID §5.2 places the MUST-reject at the validator, not
   the issuer), out of this feature's scope.

The adapter does not (cannot) runtime-reject cardinality. The `Ca` trait
**signature is unchanged** — `SvidRequest { spiffe_id: SpiffeId }` is correct
under Option A; no widening.

**Evidence.**
`docs/research/security/svid-request-cardinality-enforcement-research.md`
(committed `b6a5278b`, confidence High, 13 sources). Key facts: SPIFFE
X.509-SVID **§2** ("An X.509 SVID MUST contain exactly one URI SAN, and by
extension, exactly one SPIFFE ID") + **§5.2** (the MUST-reject is at the
relying party); **SPIRE** (the reference impl) already implements Option A —
`WorkloadX509SVIDParams` carries a single `spiffeid.ID` (no URI-SAN slice), the
cert template writes exactly one URI by construction, and the runtime reject is
at the go-spiffe verifier; "Parse, Don't Validate" (Alexis King) + RFC 5280
§4.2.1.6 (issuer owns SAN *correctness*; relying party owns *rejection*); the
internal consumer survey (every `issue_svid` consumer — #35/#36/#40/#26/#80/#81/
#100/#89 + whitepaper §8 — wants exactly one identity; the DNS-SAN/ACME lane is
a separate `instant-acme` path, not this port; D-CA-4 confirms no
attacker-controlled issuance boundary).

**Resolution (this back-propagation pass).**
- **S-04-09 and S-04-10 are RETIRED** (decision: retire, not convert). They are
  redundant under Option A: **S-04-08** already asserts the host leaf carries
  exactly one URI SAN; **S-04-06** already asserts cross-adapter SVID-profile
  equivalence including SAN cardinality; **S-04-02** tests the live
  `CertSpec::svid` 0/≥2 reject at L1; and the spec-mandated runtime reject is at
  the verifier (#26), out of scope here. The rows are kept struck-through (not
  silently deleted) in `test-scenarios.md` § Slice 04 and the DISTILL
  Slice-04 scenario table for traceability; the crafter retires the two scaffold
  fns.
- **The `issue_svid` contract is corrected.** ADR-0063 D5 gains a three-layer
  enforcement-location note + an Amendments changelog entry; the brief `Ca`
  trait-surface prose and the DESIGN `Ca` trait-surface note are corrected to
  the by-construction framing; the crafter applies the corrected
  `Ca::issue_svid` rustdoc (specified in the architect handoff). See **ADR-0063
  D5** and its **2026-06-06 Amendments entry**.
- **Coverage metric moves (non-gating).** built-in-ca scenario count 39 → 37;
  `@error` ratio 15/39 (38.5%) → 13/37 (35.1%). The drop is the accepted,
  honest consequence of removing two dead-code tests, not a coverage
  regression. DISTILL `@error`-ratio is a non-gating metric.

**No decision reversed.** D5 already located the single-URI-SAN policy in core;
this pass only pins *where the runtime reject lives* (the verifier, not
`issue_svid`) and retires the aspirational claim. **No new GitHub issue** — this
is an in-scope contract fix, not a deferral.
