# ADR-0063: Built-in CA — `Ca` Port Trait, 3-Tier Hierarchy, and Root-Key Protection (keyring + systemd-creds + rkyv envelope)

## Status

Accepted (2026-06-05). Supersedes **ADR-0010 for *workload identity* only** —
ADR-0010's ephemeral CA (`tls_bootstrap.rs`) continues to serve the
control-plane-HTTPS / operator-CLI consumer unchanged (see § Consequences
and D-CA-5 in the feature delta). This ADR governs the persistent
workload-identity trust hierarchy (GH #28, roadmap Phase 2.6).

## Context

The whitepaper's structural-security promise (§4, §8) — "every packet carries
cryptographic workload identity" — rests on a persistent X.509 trust hierarchy
the platform mints itself, with no external PKI (SPIRE / cert-manager / Vault)
to operate. Phase 1 shipped only an *ephemeral* in-process CA (ADR-0010): a
self-signed root + server/client leaf, re-minted every `serve` boot, key in
process memory, CN-only identity. That is the wrong shape for workload
identity — it dies on restart (orphaning every issued identity), carries no
SPIFFE SAN, and has no intermediate tier.

This feature builds the real hierarchy:

```
Root CA (self-signed, P-256, CA:TRUE, keyCertSign|cRLSign)
  └── Node Intermediate CA (signed by root, pathLen=0, one per node)
        └── Workload SVID (leaf, exactly ONE spiffe:// URI SAN, CA:FALSE,
                           keyUsage=digitalSignature critical, 1h TTL)
```

**Quality drivers** (in priority order, from the feature delta KPIs K1–K5):

1. **Security / integrity (K1, K2, K3)** — every SVID chain-verifies to the
   root; SVIDs are SPIFFE-spec-compliant (exactly one URI SAN); the root key
   is never observable in plaintext at rest. This is the dominant driver — the
   root key is the trust anchor of the entire platform.
2. **Testability (K5)** — the CA composes deterministically under DST
   (seeded). Certificate *correctness* is host-adapter-tested with real
   `openssl verify`; *issuance logic* is DST-tested with seeded serials.
3. **Operational simplicity (K4)** — zero external identity components; the CA
   ships inside the one binary.

**Constraints** (locked, from guided Q&A 2026-06-05 + research + project
rules):

- **Single-node (Phase 2.6)** — one co-located node, exactly one intermediate.
  Multi-node per-node intermediates + node attestation are owned by **#36
  [2.14]** (node enrollment / admission handler, already `Depends on #28`).
- **Crypto backend = `ring`** (the workspace crypto provider today — `rustls`
  and `rcgen` both pin the `ring` feature; ADR-0039's intended switch to
  `aws-lc-rs` is **unimplemented**, tracked by **#204**). `ring` provides the
  primitives this design needs — ECDSA **P-256** signing, AES-256-GCM AEAD,
  HKDF-SHA256 — so the hierarchy and root-key envelope work on `ring` today.
  `rcgen` for X.509; ECDSA **P-256**. **FIPS 140-3 (Cert #4816) is contingent
  on #204** (the aws-lc-rs switch) — `ring` is not FIPS-validated; the `fips`
  feature is unavailable until #204 lands.
- **OOP / ports-and-adapters** — the established project paradigm. This is a
  port trait mirroring `Clock`/`Transport`/`Entropy`/`Driver`/`Dataplane`.
- **`rcgen` + `ring` (the crypto backend) MUST NOT enter `overdrive-core`'s compile graph** —
  both pull entropy (`rand`-equivalent) and FFI; the dst-lint gate
  (`xtask/src/dst_lint.rs`) rejects `rand::*` / FFI on any `crate_class =
  "core"` compile path. Verified: `overdrive-core/Cargo.toml` declares
  `crate_class = "core"` and carries no such deps today.

## Decision

### D1 — `Ca` is a port trait in `overdrive-core`, with host + sim adapters

A pure `Ca` **trait** lives in `overdrive-core/src/traits/ca.rs` (no impl, no
`rcgen` types). A **host adapter** (`RcgenCa`) in `overdrive-host`
(`adapter-host`) owns *all* `rcgen` / crypto-backend usage (`ring` today; see
Constraints). A **sim adapter**
(`SimCa`) in `overdrive-sim` (`adapter-sim`) loads pre-generated fixture keys
for DST. Consumers (control-plane boot, node bootstrap, workload-lifecycle on
alloc-start, and later the #40 rotation workflow) take `Arc<dyn Ca>` as a
**required constructor parameter** — never defaulted to a production binding
(per `.claude/rules/development.md` § "Port-trait dependencies").

**The trait surface speaks project newtypes + typed DER/PEM byte newtypes,
never `rcgen` types.** Inputs are `SpiffeId` / `CertSerial` / `NodeId` and a
pure `CertSpec` (see D5); outputs are typed cert/key/bundle byte newtypes
(`CaCertPem`, `SvidMaterial`, `TrustBundlePem`, …). This is what keeps `rcgen`
out of core's compile graph while still letting core own the *decision* of
what each certificate carries.

Method surface (full rustdoc contract in the trait source; signatures here):

```rust
pub trait Ca: Send + Sync {
    /// Generate or load the persistent self-signed root CA.
    fn root(&self) -> Result<RootCaHandle, CaError>;

    /// Issue (or re-issue) the node intermediate CA, pathLen=0, signed
    /// by the root. Single-node: one node → one intermediate.
    fn issue_intermediate(&self, node: &NodeId) -> Result<IntermediateHandle, CaError>;

    /// Mint a workload SVID: exactly ONE spiffe:// URI SAN, CA:FALSE,
    /// keyUsage=digitalSignature (critical), CSPRNG serial via Entropy.
    /// Re-issue for an existing SpiffeId yields a fresh cert (distinct
    /// serial, new validity window).
    fn issue_svid(&self, req: &SvidRequest) -> Result<SvidMaterial, CaError>;

    /// Compose the trust bundle a relying party verifies an SVID against
    /// (root anchor; intermediate as untrusted chain material).
    fn trust_bundle(&self) -> Result<TrustBundle, CaError>;
}
```

`RootCaHandle` / `IntermediateHandle` expose the cert PEM/DER and a
sign-capability handle; the private key never crosses the trait boundary as
raw bytes for issued material (only the root/intermediate adapters hold signing
keys in memory, mirroring SPIRE's "keys never leave the signer" — research
Finding 5).

**`issue_svid` honors the single-URI-SAN invariant *by construction*, not by a
runtime cardinality guard.** A `SvidRequest { spiffe_id: SpiffeId }` carries
exactly one validated identity, so a zero-or-≥2-SAN request is *unrepresentable*
at the adapter boundary — there is no `CaError::InvalidSan` branch inside
`issue_svid` to reach (the request type already parsed the cardinality). The
single fallible parse is the pure-core `CertSpec::svid(Vec<SpiffeId>)` policy
(D5), which stays fallible and rejects 0/≥2 with `CertSpecError`; it is
DST-testable and dst-lint-clean. The SPIFFE-spec-mandated *runtime* reject
(X.509-SVID §5.2) lives at the relying-party verifier (#26), not the issuer.
See the three-layer enforcement-location note under D5 for the full rationale
([research][svid-cardinality]). This is the SPIFFE spec's hardest rule
(X.509-SVID §2/§5.2) and the highest-value invariant in the feature (KPI K2).

[svid-cardinality]: ../../research/security/svid-request-cardinality-enforcement-research.md

### D2 — Root key at rest: rkyv versioned envelope per ADR-0048, in the IntentStore

The root CA private key is persisted as a typed **`RootCaKeyEnvelope`** (rkyv
versioned envelope per ADR-0048) in the **IntentStore** (redb; Raft-replicated
in HA). CA material is **intent** (linearizable), **never observation** —
whitepaper §4. The envelope follows ADR-0048 discipline exactly:

- Public alias-to-payload: `pub type RootCaKeyRecord = RootCaKeyRecordV1`.
- Codec-internal envelope enum `RootCaKeyEnvelope { V1(RootCaKeyRecordV1) }`,
  NOT re-exported from `lib.rs`; writers go through
  `RootCaKeyEnvelope::latest(...)`.
- Golden-bytes schema-evolution fixture obligation: a `FIXTURE_V1` pinning the
  V1 archived bytes under
  `crates/overdrive-core/tests/schema_evolution/root_ca_key.rs` (flagged for
  DISTILL/DELIVER — see § Consequences).
- Persist *inputs, not derived state*: the record stores the
  envelope-encrypted key material + the AEAD parameters (salt, nonce, info,
  tag, `kek_id`) — never any decoded/derived form. The plaintext key is
  recomputed (decrypted) on read, held only in adapter memory.

The **payload `RootCaKeyRecordV1`** carries the AEAD envelope fields from D4.
The *typed codec* (`RootCaKeyRecord::archive_for_store` /
`from_store_bytes`) co-locates on the payload per ADR-0048 § 4b; decode
failure emits `health.startup.refused` and surfaces
`IntentStoreError::Envelope` (intent fail-fast).

### D3 — KEK runtime holder = Linux kernel keyring; delivery at boot = systemd-creds

The **key-encryption-key (KEK)** that protects the root key is a raw 256-bit
key held in **kernel space** via the Linux kernel keyring (`add_key` /
`keyctl`, `user`-type key in the service's session/user keyring) — not in the
process heap. **systemd-creds** (`LoadCredentialEncrypted`, host-key/TPM-backed)
delivers the KEK to the service at boot; the service loads it into the keyring
on startup. Kernel keyrings are volatile across reboots, so systemd-creds is
the per-boot root-of-trust that re-supplies the KEK on every boot.

The keyring/systemd-creds plumbing is a **host-adapter concern**
(`overdrive-host`): the `Ca` trait knows nothing about keyrings; it asks a
`Kek` provider port (see D6) for the KEK bytes, and the host wires that port to
`SystemdCredsKeyring`. The sim adapter wires a fixture KEK.

**Boot flow:**

- **First boot** (no `RootCaKeyRecord` in IntentStore):
  1. systemd-creds delivers the KEK → load into kernel keyring.
  2. `Ca::root()` generates a fresh self-signed P-256 root (crypto-backend
     CSPRNG — `ring` today; see Constraints).
  3. HKDF-derive a per-use subkey from the keyring KEK (D4); AES-256-GCM-encrypt
     the root private key.
  4. Wrap as `RootCaKeyEnvelope::latest(RootCaKeyRecordV1 { … })`; persist to
     IntentStore.
- **Subsequent boot** (record present):
  1. systemd-creds → kernel keyring (KEK re-supplied).
  2. Read `RootCaKeyRecord` from IntentStore; HKDF-derive the subkey from the
     keyring KEK using the record's `salt` + `info`; AES-256-GCM-decrypt.
  3. **Decrypt failure (wrong KEK, tampered ciphertext) → refuse to start**
     with a typed `CaError` + `health.startup.refused`. **Never silently
     re-mint** — a re-mint orphans every issued identity. AEAD authentication
     distinguishes "tampered envelope" from "wrong KEK" (distinct error
     variants).

**Dev / non-systemd fallback:** an `OVERDRIVE_CA_KEK` environment variable
supplies the KEK for local dev and non-systemd environments. It is **dev-only**
— gated and logged as such; production refuses to use it unless explicitly
opted in. (This is the pluggable-KEK-source seam the research Finding 8
Approach C / Gap 2 anticipated; HSM/KMS is a later-phase KEK provider.)

### D4 — AEAD shape: HKDF-derive a per-use subkey from the keyring KEK, then AES-256-GCM

**(Reconciliation A — resolved.)** The DISCUSS-era envelope sketch
`{ciphertext, nonce, salt, kdf_params}` assumed a passphrase + KDF. The KEK is
now a *raw* 256-bit key from the keyring (D3), so a passphrase KDF (scrypt /
argon2) no longer applies and is **dropped**.

Decision: **HKDF-SHA256-derive a per-use encryption subkey from the keyring
KEK**, then AES-256-GCM-encrypt the root private key under that subkey.

```
subkey   = HKDF-SHA256-Expand(
             HKDF-SHA256-Extract(salt, KEK),
             info = "overdrive/ca/root-key/v1",
             L = 32)
ciphertext, tag = AES-256-GCM-Seal(subkey, nonce, root_key_der, aad = kek_id)

RootCaKeyRecordV1 {
    kek_id:      KekId,        // which KEK this was sealed under (key rotation)
    salt:        [u8; 32],     // HKDF salt, random per seal
    info:        Vec<u8>,      // HKDF info / domain-separation label
    nonce:       [u8; 12],     // AES-GCM nonce, random per seal
    ciphertext:  Vec<u8>,      // sealed root private key (DER)
    aead_tag:    [u8; 16],     // GCM auth tag (may be appended to ciphertext)
}
```

**Rationale (why HKDF-from-KEK over direct-AEAD-under-KEK):** HKDF buys two
properties essentially for free (the crypto backend ships HKDF — `ring`
provides HKDF-SHA256 today; one extract + one expand call):

1. **Domain separation** — the same keyring KEK can protect future distinct
   secrets (operator-key material, a future signing-key cache) by varying
   `info`, with no key-reuse across domains.
2. **A clean key-rotation seam** — `kek_id` + per-seal `salt` mean KEK rotation
   (re-seal under a new KEK) is a re-encrypt of the record, not a format change.
   This is the seam #40 (rotation) and a future HSM KEK provider build on.

Direct AES-256-GCM under the raw KEK would be simpler and *sufficient* for
Phase 2.6's single secret — but the HKDF cost is negligible and the
domain-separation + rotation properties are exactly what the deferred work
(#40, HSM) will need, so paying it now avoids a format migration later. AAD =
`kek_id` binds the ciphertext to the KEK identity (defends against
KEK-confusion).

### D5 — Pure `CertSpec` builder in `overdrive-core`; host adapter translates to `rcgen::CertificateParams`

**(Reconciliation B — resolved.)** The *decision* of which X.509 extensions and
constraints each cert role carries is **pure policy** and lives in
`overdrive-core` as a `CertSpec` builder that speaks newtypes (no `rcgen`). The
*rcgen call* (`CertSpec → rcgen::CertificateParams → self_signed/signed_by`) is
the **host adapter**.

```rust
// overdrive-core — pure, no rcgen, dst-lint-clean
pub enum CertRole { Root, Intermediate { path_len: u8 }, Svid }

pub struct CertSpec {
    role:        CertRole,
    subject:     SpiffeId,            // SVID: the workload id; CA: trust-domain only
    serial:      CertSerial,          // drawn via Entropy
    not_before:  UnixInstant,
    not_after:   UnixInstant,
    // ... derived key-usage / basic-constraints per role
}

impl CertSpec {
    /// SVID constructor — enforces the single-URI-SAN invariant and the
    /// CA:FALSE / keyUsage=digitalSignature profile. Rejects 0 or ≥2 URI SANs.
    pub fn svid(...) -> Result<Self, CertSpecError> { ... }
    pub fn root(...) -> Self { ... }            // CA:TRUE, keyCertSign|cRLSign, no pathLen
    pub fn intermediate(...) -> Self { ... }    // CA:TRUE, pathLen=0
}
```

**Rationale:** the single-URI-SAN rejection (K2) and the role→extension mapping
are the highest-value invariants in the feature; putting them in core makes
them DST-testable and dst-lint-clean, and gives the sim adapter the *same*
policy surface as the host adapter (so the DST equivalence test exercises real
policy, not a divergent sim shortcut). The host adapter's job shrinks to a pure
translation + the `rcgen` signing call — `rcgen` never appears in core.

**Enforcement location of the single-URI-SAN invariant (three layers, distinct
roles).** The "exactly one `spiffe://` URI SAN ⇔ exactly one SPIFFE ID"
invariant is the SPIFFE domain invariant (X.509-SVID spec §2: *"An X.509 SVID
MUST contain exactly one URI SAN, and by extension, exactly one SPIFFE ID"*).
It is enforced at three semantically-distinct layers, each answering a
different question — NOT by a runtime cardinality guard inside `Ca::issue_svid`:

1. **The request *type* makes ≠1 unrepresentable.** `SvidRequest { spiffe_id:
   SpiffeId }` carries exactly one validated identity by construction. There is
   no `URISANs: Vec<…>` field; an adapter physically cannot be handed a
   zero-or-multiple-SAN request. This is "make illegal states unrepresentable"
   ([research][1] SQ3; Minsky/King) and is the same shape the SPIFFE reference
   implementation (SPIRE) chose — its signer parameter
   `WorkloadX509SVIDParams.SPIFFEID` is a single `spiffeid.ID`, not a slice
   ([research][1] SQ2).
2. **The pure parse `CertSpec::svid(Vec<SpiffeId>)` is the single fallible
   boundary.** The *one* place a raw `Vec` projection of identities is parsed
   into a validated single-identity leaf profile is the pure-core `CertSpec`
   policy (D5). It stays fallible and rejects 0 or ≥2 with `CertSpecError`
   ("parse, don't validate" — parse once at the boundary, trust the refined
   `SpiffeId` thereafter). This is DST-testable and dst-lint-clean, and is
   tested green at L1 by **S-04-02**
   (`svid_spec_rejects_zero_or_multiple_uri_sans_before_any_cert`).
3. **The relying-party verifier is the SPIFFE-spec-mandated *runtime* reject.**
   X.509-SVID spec §5.2 places the binding MUST-reject at the *validator*, not
   the issuer: *"Validators encountering an SVID containing more than one URI
   SAN MUST reject the SVID."* That runtime reject lives at the peer
   authenticator — the future #26 sockops/kTLS mTLS verifier — **not** inside
   `Ca::issue_svid`. It is the genuine defense-in-depth boundary (a distinct
   trust boundary that must reject any non-compliant cert regardless of which CA
   issued it), and it is out of this feature's scope (owned by #26).

The adapter does **not** runtime-reject SAN cardinality — it cannot receive a
bad one (layer 1), the only fallible parse is the pure policy (layer 2), and
the spec's runtime MUST-reject lives at the verifier (layer 3). A runtime guard
inside `issue_svid` for a state the request type already forbids would be dead
code in the same component, not defense-in-depth ([research][1] SQ3/SQ5; this
is the internal-CA / "no attacker-controlled issuance boundary" case, D-CA-4).

[1]: ../../research/security/svid-request-cardinality-enforcement-research.md

### D6 — Audit trail: `issued_certificates` ObservationStore row

Every issuance writes an `issued_certificates` **observation** row (single-node
= local SQLite; gossiped when multi-node #36 lands). Columns per research
Finding 15: `serial`, `spiffe_id`, `issuer_serial`, `not_before`, `not_after`,
`node_id`, `issued_at`. This is the internal-CT-equivalent audit surface,
readable via the existing `alloc status` observation path. It is **observation,
never intent** — the CA *material* is intent (D2); the *record of what was
issued* is observation. The row is a rkyv versioned envelope
(`IssuedCertificateRowEnvelope`) mirroring `AllocStatusRow` / `NodeHealthRow`.

**Issuance is never silent:** an issuance whose audit row cannot be written
surfaces a `CaError` rather than handing out an unaudited certificate (KPI/AC,
US-CA-05).

### D7 — Serials via the `Entropy` port; key generation via the crypto backend CSPRNG

Certificate **serial numbers** flow through the existing `Entropy` port
(`Entropy::fill`, ≥64-bit per research Finding 10 / CA/B Forum floor):
`OsEntropy` in production, `SeededEntropy` under DST → issuance is
DST-deterministic. **Key generation** uses the crypto backend's own CSPRNG
(`ring` today via `rcgen`'s `KeyPair::generate`; see Constraints) and is
**NOT injectable** — acceptable
per research Finding 11 (the correct production security posture; DST uses
pre-generated fixture keys loaded via PEM in the sim adapter). dst-lint stays
satisfied because key generation never enters a core compile path.

### D8 — Architecture-rule enforcement

- **dst-lint** (existing) enforces the crate-boundary: no `rand::*` / FFI /
  `tokio::net` on the `overdrive-core` compile path → `rcgen` and the crypto
  backend (`ring` today; see Constraints) cannot leak into core.
- **`tests/integration/ca_equivalence.rs`** — a DST equivalence test drives
  `RcgenCa` (host) and `SimCa` (sim) through the same call sequence and asserts
  observable equivalence (per `development.md` § "Trait definitions specify
  behavior" → "The DST equivalence test is the structural guard"). This is the
  enforcement for the `Ca` trait contract.
- **Earned Trust probe** — the root-key path has a composition-root invariant:
  *wire then probe then use*. On boot the CA adapter probes that the keyring KEK
  is present and the persisted envelope decrypts BEFORE the control plane
  accepts traffic; a probe failure refuses startup with `health.startup.refused`
  (see § Earned Trust below).

## Alternatives Considered

### A1 — CA as a free-function module in `overdrive-host` (no core trait)

Put the whole CA (params + signing + key protection) in `overdrive-host` as
plain functions, like today's `tls_bootstrap.rs`. **Rejected:** it cannot be
DST-tested (no sim seam), it puts the single-URI-SAN policy where the sim path
can't share it, and it breaks the project's uniform port-trait discipline
(`Clock`/`Transport`/`Driver`/`Dataplane` are all traits). The whole point of
this feature over ADR-0010 is a DST-honest, swappable CA.

### A2 — Whole cert-param construction in the host adapter (rcgen-shaped policy)

Keep `CertSpec` out of core; let the host adapter own both *what* a cert
carries and *how* it's built. **Rejected (reconciliation B):** the
single-URI-SAN rejection and role→extension mapping are the highest-value
invariants; burying them in the host adapter makes them untestable under DST
and lets the sim adapter diverge on policy. Core owns the decision; the adapter
owns the rcgen call.

### A3 — Root key protection: PKCS#8 encrypted private key (AES-256-CBC)

RustCrypto `pkcs8` with scrypt-derived key + AES-256-CBC (research Finding 8
Approach A). **Rejected:** AES-CBC is *unauthenticated* — for the platform's
trust anchor, authenticated encryption (AES-GCM, integrity + confidentiality)
is the defensible floor. AEAD lets us distinguish "tampered envelope" from
"wrong key" as distinct errors (an AC). PKCS#8's only advantage —
`openssl pkcs8` interop — is irrelevant for an internal key never exported.

### A4 — Root key protection: passphrase-derived KEK (the DISCUSS sketch)

scrypt/argon2 KEK from an operator passphrase + AES-256-GCM (research Finding 8
Approach B). **Superseded by D3/D4:** the locked decision moved the KEK to the
kernel keyring delivered by systemd-creds (host-key/TPM-backed) — a stronger,
operator-friendlier root-of-trust than a typed passphrase, and the path HSM/KMS
extends. The passphrase KDF is dropped; HKDF-from-keyring-KEK replaces it.

### A5 — Direct AES-256-GCM under the raw keyring KEK (no HKDF)

Encrypt the root key directly under the raw KEK, envelope =
`{version, ciphertext, nonce, kek_id, aead_tag}`. **Rejected (reconciliation
A):** simpler and sufficient for one secret, but HKDF-derive costs one
extract+expand and buys domain separation (reuse the KEK for future secrets via
`info`) and a clean rotation seam (`kek_id` + `salt`) — exactly what #40 and a
future HSM provider need. Paying the negligible HKDF cost now avoids a format
migration later.

### A6 — Audit trail in the workflow journal only (no observation row)

Rely on the (future #40) rotation workflow's await-point journal as the sole
audit surface. **Rejected:** issuance happens on alloc-start *before* any
rotation workflow exists; the journal is per-workflow and not gossip-readable.
The `issued_certificates` observation row is queryable by any node via the
existing observation path and is the internal-CT equivalent (research
Finding 15). The two are complementary (journal = workflow step inputs;
observation row = gossip-distributed audit) but the row is the SSOT for "what
was issued."

## Consequences

### Positive

- **Zero external PKI** (K4): the CA ships inside the one binary — no SPIRE,
  cert-manager, or Vault. Operational simplicity is the headline advantage over
  SPIRE (research Finding 13).
- **DST-honest** (K5): serials via `Entropy`, fixture keys in the sim adapter,
  the `ca_equivalence` test → issuance logic reproduces bit-identically from a
  seed.
- **Trust anchor protected at rest** (K3): root key never plaintext on disk;
  KEK in kernel space (not heap); AEAD authentication; systemd-creds/TPM
  root-of-trust.
- **Refuse-to-start over silent re-mint**: a decrypt failure refuses startup
  rather than orphaning every issued identity.
- **Clean extension seams**: KEK-source pluggable (env → systemd-creds → future
  HSM); rotation seam (`kek_id`/`salt`/HKDF) ready for #40; multi-node
  intermediate shape ready for #36.
- **Reuse, not reinvention**: `SpiffeId` / `CertSerial` / `NodeId` / `Entropy` /
  `IntentStore` / `ObservationStore` / `VersionedEnvelope` are all reused
  as-is (see brief § Reuse Analysis). The proven `rcgen` usage in
  `mint_ephemeral_ca` (P-256, `self_signed`, `signed_by`, `SanType`,
  `KeyUsagePurpose`, `IsCa`) carries forward and de-risks the crypto.

### Negative / costs

- **Linux-keyring + systemd-creds coupling** in the host adapter — non-systemd
  / non-Linux dev paths need the `OVERDRIVE_CA_KEK` fallback. Mitigated: the
  KEK source is a port (`Kek` provider) so the coupling is one adapter, and the
  fallback is gated dev-only. (Overdrive is Linux-only in production per user
  memory `no_cfg_target_os_linux`, so this is the production path, not a
  special case.)
- **Two new rkyv envelopes** (`RootCaKeyEnvelope`, `IssuedCertificateRowEnvelope`)
  each carry the ADR-0048 golden-bytes fixture obligation + the
  empirically-pinned `discriminant_offset_from_end`. Flagged for DISTILL/DELIVER
  (this is real work, not free).
- **Target is `rcgen` 0.14.8; the workspace currently pins 0.13.2 — the bump is
  a DELIVER first-compile gate** — `Cargo.toml` pins `rcgen = "0.13"` (lockfile
  0.13.2) today; the target pin is `rcgen = { version = "0.14",
  default-features = false, features = ["ring", "pem"] }` (resolving to 0.14.8,
  MSRV 1.88), so the pin must be bumped as a DELIVER prerequisite. The `ring`
  feature matches the workspace crypto provider (ADR-0039's `aws-lc-rs` switch
  is unimplemented; #204). The X.509-extension APIs are stable across
  0.13.2→0.14.x
  (`IsCa::Ca(BasicConstraints::Constrained(0))`, `SanType::URI(Ia5String)`,
  `KeyUsagePurpose` all exist in both), but the 0.14
  cert-builder API changed (e.g. the `params.self_signed(&key)` /
  `params.signed_by(&key, &issuer)` flow), so `mint_ephemeral_ca` — written
  against the 0.13 builder API — must migrate to 0.14.x in the same change.
  The bump de-risks nothing on the builder calls; confirm the builder surface
  + extension APIs at first compile (research Gap 3 + version delta).
- **`rcgen` `ring` feature** must be confirmed non-conflicting with the
  workspace's `rustls`/`ring` (research Gap 3) — first-compile check in
  Slice 01. (When #204 lands the aws-lc-rs switch, this `rcgen` feature flips
  to `aws_lc_rs` in lockstep with the workspace provider.)

### Earned Trust (probe contract)

Every dependency the CA boot path leans on that *could lie* gets probed before
the system accepts traffic — *wire then probe then use*:

- **KEK present in keyring** — probe `keyctl`/`add_key` round-trips the KEK
  before any decrypt; a missing/empty KEK refuses startup
  (`health.startup.refused`), not a panic mid-issuance.
- **Persisted envelope decrypts** — on subsequent boot, the adapter performs a
  trial HKDF-derive + AES-GCM-open of the persisted `RootCaKeyRecord` at
  composition time; an auth failure (tampered) or wrong-KEK failure refuses
  startup with the *distinct* typed error.
- **systemd-creds delivery** — the host adapter treats an absent
  `LoadCredentialEncrypted` credential (and the absence of the dev
  `OVERDRIVE_CA_KEK` opt-in) as a refuse-to-start, not a silent fallback to a
  generated KEK (which would make the at-rest encryption meaningless).

These probes are the composition-root invariant; the `ca_equivalence` DST test
plus host-adapter fault-injection (tampered ciphertext, wrong KEK, absent
credential) exercise the substrate lies. Fault-injection scenarios are flagged
for DISTILL.

## References

- GH #28 [2.6] — Built-in CA primitive (this feature).
- Feature delta: `docs/feature/built-in-ca/feature-delta.md` (DISCUSS + DESIGN).
- Research: `docs/research/security/built-in-ca-rcgen-rustls-comprehensive-research.md`
  (Findings 1–15; Approach B/C; Gaps 2/3).
- ADR-0010 — Phase-1 TLS bootstrap (superseded for *workload identity* only).
- ADR-0039 — rustls + aws-lc-rs + FIPS provider (ADR-0039 **unimplemented** —
  workspace remains on `ring`; aws-lc-rs switch + FIPS posture tracked by **#204**).
- ADR-0048 — rkyv versioned envelope (the `RootCaKeyEnvelope` /
  `IssuedCertificateRowEnvelope` discipline).
- Whitepaper §4 (state layers; CA material is intent), §8 (security),
  §18 (rotation is a workflow → #40), §21 (DST).
- Deferrals: #40 [3.3] rotation (needs #39 [3.2] workflow primitive),
  #36 [2.14] multi-node CA / node attestation, #104 [7.1] / #83 [5.17]
  multi-region, #81 operator-cert minting (Phase 5), Phase 5 gossip-revocation,
  Phase 7 SPIFFE Workload API.

## Changelog

- 2026-06-06 — **D5 enforcement-location clarification (DELIVER-surfaced;
  Option A ratified). No decision reversed.** DELIVER step 04 surfaced a
  contract contradiction: the original D1 prose and the `Ca::issue_svid` rustdoc
  claimed the adapter "rejects zero or two-or-more URI SANs with
  `CaError::InvalidSan` before any cert" — a rejection the request type
  (`SvidRequest { spiffe_id: SpiffeId }`, one validated identity by
  construction) makes **unreachable**. That was an aspirational-doc bug
  (`development.md` § "No aspirational docs" / "Never document behaviour that is
  not implemented"). The user ratified **Option A — type-enforced** (2026-06-06)
  on the strength of
  `docs/research/security/svid-request-cardinality-enforcement-research.md`
  (committed `b6a5278b`; SPIFFE X.509-SVID §2/§5.2 + SPIRE reference impl +
  "parse, don't validate"). This amendment adds the three-layer
  enforcement-location note under D5 — (1) the request *type* makes ≠1
  unrepresentable; (2) the pure `CertSpec::svid(Vec<SpiffeId>)` parse is the
  single fallible boundary (rejects 0/≥2, tested green by S-04-02 at L1); (3)
  the relying-party verifier (#26) is the SPIFFE-spec-mandated runtime reject —
  and corrects the D1 prose to state the invariant is honored *by construction*,
  not by an adapter cardinality guard. **D5 itself is unchanged** — policy was
  already in core; this amendment only pins *where the runtime reject lives*
  (the verifier, not `issue_svid`) and retires the type-unreachable claim. The
  two DISTILL scenarios that tested the unreachable adapter path — S-04-09
  (`rcgen_svid_request_with_bad_san_cardinality_is_rejected_pre_issuance`) and
  S-04-10 (`ca_equivalence_bad_san_request_rejected_identically_by_both`) — are
  retired (redundant under Option A: S-04-08 already asserts the host leaf
  carries exactly one URI SAN, S-04-06 already asserts cross-adapter SVID-profile
  equivalence including SAN cardinality, and S-04-02 tests the live `CertSpec`
  parse reject). The crafter applies the corrected `issue_svid` rustdoc and
  retires the two scaffolds. — Morgan.
- 2026-06-05 — Provider-attribution + FIPS-contingency correction. The
  workspace crypto provider is **`ring`** today (`rustls` and `rcgen` both pin
  the `ring` feature); ADR-0039's intended switch to `aws-lc-rs` is
  **unimplemented**, tracked by **#204**. The crypto *design* is unchanged —
  `ring` provides P-256, AES-256-GCM, and HKDF-SHA256, so the 3-tier hierarchy
  and HKDF→AES-256-GCM root-key envelope work on `ring` today. The only changed
  claim is **FIPS 140-3 (Cert #4816), now contingent on #204** (`ring` is not
  FIPS-validated). The `rcgen` 0.14.8 pin uses `features = ["ring", "pem"]`
  (was stated as `aws_lc_rs`). No architecture decision altered (`Ca` trait,
  HKDF AEAD, keyring/systemd-creds, envelope, audit row, single-node all
  intact).
