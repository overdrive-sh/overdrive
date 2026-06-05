<!-- markdownlint-disable MD013 MD024 MD033 -->
# Test Scenarios — built-in-ca (GH #28 · roadmap Phase 2.6)

**Wave**: DISTILL (wave 5 of 6) · **Agent**: Quinn (nw-acceptance-designer)
· **Density**: `lean` (Tier-1 `[REF]` only)

> **SPECIFICATION ONLY — not executed.** Per `.claude/rules/testing.md`
> § "No `.feature` files anywhere", this project has **no** Gherkin runner
> (no cucumber-rs, no pytest-bdd). The GIVEN/WHEN/THEN blocks below are the
> human-readable *specification companion* for the Rust scaffolds; they are
> **never parsed or executed**. The executable SSOT is the set of eight Rust
> `#[test]` scaffold files (one row of the "Scaffold" column per scenario),
> already authored and wired into their crate `tests/*.rs` entrypoints. The
> crafter implements the GREEN bodies of those Rust scaffolds in DELIVER; this
> doc is the contract those bodies must satisfy.
>
> **Crypto backend authority.** ADR-0063 (Accepted 2026-06-05, the latest
> wave artifact) fixes the crypto backend as **`ring`** (rcgen 0.14.8,
> `features = ["ring", "pem"]`, P-256). ADR-0039's intended `aws-lc-rs` switch
> is **unimplemented** and deferred to **#204**; FIPS 140-3 (Cert #4816) is
> contingent on #204. Earlier feature-delta DISCUSS prose that says
> `aws_lc_rs`/`rcgen 0.13` is **superseded** by ADR-0063 (documented in its
> changelog). The scaffolds encode `ring`; this doc follows ADR-0063.

---

## How to read this document

- Scenarios are grouped by the **5 slices** (Slice 01 → Slice 05), which map
  1:1 to the **5 user stories** (US-CA-01 → US-CA-05) and to the linear
  trust-hierarchy dependency chain (root → persist → intermediate → SVID →
  bundle/audit/re-issue).
- Each scenario carries an **ID** (`S-01-NN` … `S-05-NN`, slice-scoped), the
  **scaffold function** it specifies, its **layer** (per the
  `nw-test-design-mandates` Layered Test Discipline table), its **tags**, and
  its **trace** to the originating user story / KPI / journey / ADR decision.
- The **S-NN tags inside the scaffolds** (`@S-01` … `@S-05`) denote the
  *slice* a scenario belongs to (one scaffold tag = one slice). They are NOT
  per-scenario unique IDs; the per-scenario unique ID is the `S-0S-NN`
  assigned in this document.
- **Universe** (per Mandate 8): for state-mutating scenarios, the
  port-exposed observable surface the test promises to track. For this Rust
  workspace the `assert_state_delta` Python port is not used (see
  `docs/architecture/atdd-infrastructure-policy.md` § Mandate-8 mapping); the
  universe-bound discipline is satisfied natively by asserting **exact
  equality over the trait-accessor / observation-row / byte-scan surface** and
  fail-closed on any unexpected extra (e.g. an unexpected extra SAN, an extra
  audit row, a plaintext key byte present where none is expected).

### Layer legend (per `nw-test-design-mandates`)

| Layer | What | Files in this feature |
|---|---|---|
| **L1** Unit / pure | <1ms, no I/O, PBT-full at GREEN (Mandate 9) | `ca_cert_spec_policy.rs` (core `CertSpec` policy) |
| **L1/L2** sim adapter | ~10ms, in-memory sim doubles, example-only (Mandate 9 — layer 2) | `sim_ca_deterministic.rs` (`SimCa`) |
| **L1** archive (default lane) | pure rkyv, no I/O; golden-bytes (testing.md § schema-evolution) | `schema_evolution/root_ca_key.rs`, `schema_evolution/issued_certificate_row.rs` |
| **L3** real-io / integration | ~100ms–s, real crypto + `openssl verify` + real stores + keyring, gated `integration-tests`, Lima; example-only sad paths (Mandate 11) | `rcgen_ca_chain_verify.rs`, `rcgen_ca_root_key_envelope.rs`, `ca_equivalence.rs`, `ca_boot_and_audit.rs` |

### Walking skeleton

Per Mandate 5 / DISCUSS § Story Map, the walking skeleton is realised across
Slices 01→04 (generate root → persist → issue intermediate → issue SVID →
**full chain verifies**). The single scenario tagged `@walking_skeleton` is
**S-04-07** (`rcgen_full_svid_chain_verifies_root_intermediate_svid`): the
headline end-to-end proof that a real workload SVID chain-verifies Root →
Intermediate → SVID via `openssl verify`. Litmus (Dim 5): Sam the security
engineer runs `openssl verify` himself and confirms the workload identity
validates to the root — the genuine user-observable outcome. **No operator
CLI verb exists this phase** (D-CA-4); `openssl verify` is the honest external
entry point.

---

## Slice 01 — Root CA generation behind the `Ca` port trait (US-CA-01)

**Story**: US-CA-01 · **Job**: J-SEC-001 · **KPI**: K1 (chain-verify), K5 (DST)
**Hypothesis**: rcgen + `ring` can mint a SPIFFE-hierarchy root behind the
`Ca` port trait, DST-deterministic via the sim adapter.

### S-01-01 — Root profile is a self-signed CA (pure policy)

- **Scaffold**: `ca_cert_spec_policy.rs::root_spec_is_self_signed_ca_with_key_cert_sign_and_crl_sign`
- **Layer**: L1 (pure) · **Tags**: `@in-memory @S-01` · **Trace**: US-CA-01 AC1/AC2, ADR-0063 D5
- **Universe**: `CertSpec` port-exposed surface — `{role, is_ca, key_usages, key_usage_critical, path_len, subject}` (never internal builder fields)

```gherkin
Scenario: The root cert profile is a self-signed CA with cert-signing authority
  Given a request to build the root certificate profile for the trust domain
  When the platform constructs the root CertSpec
  Then the profile is CA:TRUE
  And it carries keyCertSign and cRLSign key usages
  And keyUsage is marked critical
  And it carries NO pathLen constraint
  And the subject is the trust domain only, with no path component
```

### S-01-02 — Root generation is bit-identical under the seeded harness

- **Scaffold**: `sim_ca_deterministic.rs::sim_ca_root_is_bit_identical_across_two_runs_at_same_seed`
- **Layer**: L2 (sim) · **Tags**: `@in-memory @S-01` · **Trace**: US-CA-01 AC4, KPI K5, journey integration_validation (serial via Entropy)
- **Universe**: `SimCa::root()` observable issuance bytes (PEM/DER), compared for exact byte-equality across two same-seed runs

```gherkin
Scenario: Root generation is deterministic under the simulation harness
  Given the seeded DST harness with the sim CA adapter and a fixture root key
  And a fixed seed of 0x5EED
  When the platform generates the root twice at that same seed
  Then both runs produce bit-identical root material
```

### S-01-03 — Root profile equivalence across host and sim adapters

- **Scaffold**: `ca_equivalence.rs::ca_equivalence_root_profile_matches_across_host_and_sim`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-01` · **Trace**: US-CA-01, ADR-0063 D8, development.md § "DST equivalence test is the structural guard"
- **Universe**: contract-observable root profile via trait accessors — `{is_ca, key_usages, key_usage_critical, subject}` (key/serial bytes EXCLUDED — sim fixture key differs from host-generated key by construction, research Finding 11; equivalence is over the profile, not the material)

```gherkin
Scenario: The root profile is identical whether produced by the host or sim adapter
  Given the host CA adapter (real rcgen/ring) and the sim CA adapter (fixture key)
  When each adapter produces its root
  Then both roots are CA:TRUE
  And both carry keyCertSign and cRLSign with keyUsage critical
  And both carry a trust-domain-only subject
  And the contract-observable profile is equivalent across the two adapters
```

### S-01-04 — Real root self-verifies with `openssl verify` (KPI K1)

- **Scaffold**: `rcgen_ca_chain_verify.rs::rcgen_root_is_a_valid_self_signed_ca_via_openssl_verify`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-01` · **Trace**: US-CA-01 AC3, KPI K1, journey step 1
- **Universe**: `openssl verify` exit code (0) + x509-parser-observed extensions `{is_ca, keyCertSign, keyUsage_critical}` on the real cert bytes

```gherkin
Scenario: The platform produces a valid self-signed root CA
  Given a freshly initialised control plane with no existing CA material
  When the platform generates its root certificate authority via the host adapter
  Then "openssl verify -CAfile root.pem root.pem" exits 0
  And the real cert carries CA:TRUE, keyCertSign, and keyUsage marked critical
```

### S-01-05 — `CertSpecError` variants are distinct per failure mode (sad path)

- **Scaffold**: `ca_cert_spec_policy.rs::cert_spec_error_variants_are_distinct_per_failure_mode`
- **Layer**: L1 (pure) · **Tags**: `@in-memory @error @S-01 @S-04` · **Trace**: US-CA-01 + US-CA-04, ADR-0063 D5, development.md § "Distinct failure modes get distinct error variants"
- **Universe**: the `CertSpecError` variant returned per failure mode — an invalid SAN cardinality surfaces `CertSpecError::InvalidSan`, DISTINCT from any other validation failure (guards against a single `Internal(String)` catch-all swallowing the load-bearing single-URI signal)
- **Note**: this scaffold carries both `@S-01` and `@S-04` tags (it guards the cross-role error taxonomy); it is filed under Slice 01 as the policy-taxonomy guard but also serves the K2 single-URI invariant (Slice 04).

```gherkin
Scenario: Certificate-spec error variants are distinct, not a catch-all
  Given the CertSpec policy with multiple distinguishable validation failures
  When an invalid SAN cardinality is submitted
  Then it surfaces CertSpecError::InvalidSan as a distinct variant
  And that variant is not flattened into a generic Internal(String) catch-all
```

---

## Slice 02 — Root CA key envelope-encrypted at rest (US-CA-02)

**Story**: US-CA-02 · **Job**: J-SEC-001 · **KPI**: K3 (root key never plaintext)
**Hypothesis**: the root survives restart with its key protected by
authenticated encryption (HKDF-SHA256 → AES-256-GCM), using only `ring`.

### S-02-01 — Envelope seal/open round-trips under the same KEK

- **Scaffold**: `rcgen_ca_root_key_envelope.rs::root_key_envelope_seals_and_opens_round_trip_under_same_kek`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-02` · **Trace**: US-CA-02 AC1, ADR-0063 D4, journey step 1
- **Universe**: recovered key DER bytes, compared for exact byte-equality against the original

```gherkin
Scenario: The root key round-trips through the AEAD envelope under the same KEK
  Given a root private key and a 256-bit KEK
  When the platform seals the key via HKDF-SHA256 then AES-256-GCM
  And then opens the sealed record under the same KEK
  Then the recovered key DER is byte-identical to the original
```

### S-02-02 — Sealed record contains no plaintext key bytes (KPI K3 guardrail)

- **Scaffold**: `rcgen_ca_root_key_envelope.rs::root_key_envelope_contains_no_plaintext_key_bytes`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-02` · **Trace**: US-CA-02 AC2, KPI K3, journey step 1
- **Universe**: the serialized `RootCaKeyRecordV1` bytes (and the IntentStore blob wrapping them) byte-scanned for the known plaintext key DER — expected absence (fail-closed: any occurrence fails)

```gherkin
Scenario: The root key is never observable in plaintext at rest
  Given the root key has been sealed into the RootCaKeyRecord and persisted
  When the serialized record and its IntentStore blob are byte-scanned
  Then zero plaintext private-key bytes are present
```

### S-02-03 — Tampered ciphertext fails distinctly from a wrong KEK

- **Scaffold**: `rcgen_ca_root_key_envelope.rs::root_key_envelope_tampered_ciphertext_fails_distinct_from_wrong_kek`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-02 @error` · **Trace**: US-CA-02 AC3, ADR-0063 D4, journey error_paths step 1
- **Universe**: the `CaError` variant returned on open — expected a corrupt/tampered-envelope variant, DISTINCT from the wrong-KEK variant (the GCM auth tag distinguishes them)

```gherkin
Scenario: A tampered envelope fails AEAD open with a distinct error
  Given a sealed root-key record under a known KEK
  When one byte of the sealed ciphertext is flipped
  And the record is opened under the correct KEK
  Then the open fails with a corrupt/tampered-envelope error
  And that error is distinguishable from a wrong-KEK error
```

### S-02-04 — Wrong KEK fails distinctly from a tampered envelope

- **Scaffold**: `rcgen_ca_root_key_envelope.rs::root_key_envelope_wrong_kek_fails_distinct_from_tampered`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-02 @error` · **Trace**: US-CA-02 AC3, ADR-0063 D4 (AAD = kek_id), journey error_paths step 1
- **Universe**: the `CaError` variant returned on open under a different KEK — expected a wrong-KEK variant, DISTINCT from tampered-envelope

```gherkin
Scenario: Opening with the wrong KEK fails with a distinct wrong-KEK error
  Given a sealed root-key record under one KEK
  When the record is opened under a different KEK
  Then the open fails with a wrong-KEK error
  And that error is distinguishable from a tampered-envelope error
```

### S-02-05 — Persistent root reused across control-plane restart

- **Scaffold**: `ca_boot_and_audit.rs::root_ca_is_reused_across_control_plane_restart`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-02` · **Trace**: US-CA-02 AC1, ADR-0063 D2/D3, journey step 1 (supersedes ADR-0010 ephemerality)
- **Universe**: the root identity observable across two boots — same public key / same cert on second boot (exact equality)

```gherkin
Scenario: The root CA survives a restart with its key protected
  Given a control plane that has generated and persisted its root CA
  And only the envelope-encrypted key blob is on disk
  When the control plane restarts with the correct KEK
  Then it decrypts and reuses the SAME root CA identity (same public key)
```

### S-02-06 — Boot refuses to start on envelope decrypt failure, without re-mint

- **Scaffold**: `ca_boot_and_audit.rs::boot_refuses_to_start_on_envelope_decrypt_failure_without_remint`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-02 @error` · **Trace**: US-CA-02 AC4, ADR-0063 D3/Earned-Trust, journey error_paths step 1
- **Universe**: control-plane startup outcome (refused) + emitted `health.startup.refused` signal + typed `CaError`; AND the absence of any newly-minted root (no silent re-mint)

```gherkin
Scenario: A tampered/undecryptable envelope refuses startup without re-minting
  Given a persisted root-key envelope that cannot be decrypted
  When the control plane attempts to start
  Then the Earned-Trust probe fails
  And the control plane refuses to start with a typed error and health.startup.refused
  And no new root CA is silently minted
```

### S-02-07 — Boot refuses to start when the KEK is absent from the keyring

- **Scaffold**: `ca_boot_and_audit.rs::boot_refuses_to_start_when_kek_absent_from_keyring`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-02 @error` · **Trace**: US-CA-02 AC4, ADR-0063 D3/Earned-Trust (KEK probe), journey error_paths step 1
- **Universe**: control-plane startup outcome (refused before any issuance) + the absence of any silently-generated throwaway KEK

```gherkin
Scenario: An absent keyring KEK refuses startup before any issuance
  Given an empty keyring KEK and no dev OVERDRIVE_CA_KEK opt-in
  When the control plane attempts to start
  Then it refuses to start before any issuance
  And no throwaway KEK is silently generated
```

### S-02-08 — `RootCaKeyEnvelope` V1 golden-bytes roundtrip (schema evolution)

- **Scaffold**: `schema_evolution/root_ca_key.rs::root_ca_key_envelope_v1_golden_bytes_roundtrip`
- **Layer**: L1 (archive, default lane) · **Tags**: `@property @S-02` (S-EV-CA-01) · **Trace**: ADR-0063 D2, ADR-0048, testing.md § "Archive schema-evolution roundtrip"
- **Universe**: the canonical `Latest` projection after `into_latest()`, compared for exact equality against the hand-pinned canonical payload

```gherkin
Scenario: The persisted root-key envelope layout is byte-stable across versions
  Given the pinned V1 archived bytes of the RootCaKeyRecord envelope (FIXTURE_V1)
  When the bytes are rkyv-deserialised and projected via into_latest()
  Then the result equals the canonical Latest projection
  And any future field appended to V1 (rather than minting V2) breaks this test
```

### S-02-09 — `RootCaKeyEnvelope` discriminant offset triangulates

- **Scaffold**: `schema_evolution/root_ca_key.rs::root_ca_key_envelope_discriminant_offset_triangulates`
- **Layer**: L1 (archive, default lane) · **Tags**: `@property @S-02` · **Trace**: ADR-0048 (two-source discriminant pin)
- **Universe**: the independently-pinned discriminant offset vs `RootCaKeyEnvelope::discriminant_offset_from_end()` — exact agreement

```gherkin
Scenario: The envelope discriminant offset is pinned from two independent sources
  Given an independent pin of the V1 discriminant offset
  When it is compared against discriminant_offset_from_end()
  Then the two agree
  And neither pin can drift unilaterally without failing this test
```

### S-02-10 — Unknown root-key envelope version surfaces an error (intent fail-fast)

- **Scaffold**: `schema_evolution/root_ca_key.rs::root_ca_key_envelope_unknown_version_probe_surfaces_error`
- **Layer**: L1 (archive, default lane) · **Tags**: `@property @S-02 @error` · **Trace**: ADR-0048 (intent fail-fast), ADR-0063 D2
- **Universe**: the `EnvelopeError` surfaced by `probe_known_variant` (vs a silent garbage decode)

```gherkin
Scenario: An unknown root-key envelope version fails fast rather than decoding garbage
  Given an unknown/forward envelope version
  When the IntentStore decode path probes the variant
  Then it surfaces an EnvelopeError
  And it does not decode into garbage (the intent path will emit health.startup.refused)
```

---

## Slice 03 — Per-node intermediate CA, pathLen-constrained (US-CA-03)

**Story**: US-CA-03 · **Job**: J-SEC-001 · **KPI**: K1 (chain-verify)
**Hypothesis**: the platform issues a pathLen=0 intermediate that chains to
the root, bounding node-compromise blast radius.

### S-03-01 — Intermediate profile is CA:TRUE with pathLen=0 (pure policy)

- **Scaffold**: `ca_cert_spec_policy.rs::intermediate_spec_is_ca_true_with_path_len_zero_and_key_cert_sign`
- **Layer**: L1 (pure) · **Tags**: `@in-memory @S-03` · **Trace**: US-CA-03 AC1, ADR-0063 D5 (sum type `CertRole::Intermediate { path_len: 0 }`)
- **Universe**: `CertSpec` profile — `{role, is_ca, path_len, key_usages, key_usage_critical}`

```gherkin
Scenario: The intermediate cert profile is a pathLen-0 signing CA
  Given a request to build the node intermediate certificate profile
  When the platform constructs the intermediate CertSpec
  Then the profile is CA:TRUE with pathLenConstraint=0
  And it carries keyCertSign with keyUsage marked critical
```

### S-03-02 — Intermediate is deterministic and chains to the fixture root (sim)

- **Scaffold**: `sim_ca_deterministic.rs::sim_ca_intermediate_is_deterministic_and_chains_to_fixture_root`
- **Layer**: L2 (sim) · **Tags**: `@in-memory @S-03` · **Trace**: US-CA-03, KPI K5
- **Universe**: `SimCa::issue_intermediate(&node)` observable issuance bytes + chains-to-fixture-root linkage, byte-equal across two same-seed runs

```gherkin
Scenario: Intermediate issuance is deterministic and chains to the fixture root
  Given the seeded DST harness with the sim CA adapter at a fixed seed
  When the platform issues a node intermediate twice at that seed
  Then both runs produce the same intermediate material and serial
  And the intermediate chains to the fixture root
```

### S-03-03 — Intermediate profile equivalence across host and sim adapters

- **Scaffold**: `ca_equivalence.rs::ca_equivalence_intermediate_profile_matches_across_host_and_sim`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-03` · **Trace**: US-CA-03, ADR-0063 D8
- **Universe**: intermediate contract-observable profile via trait accessors — `{is_ca, path_len, chains_to_root, key_usages}`

```gherkin
Scenario: The intermediate profile is identical whether produced by host or sim
  Given the host CA adapter and the sim CA adapter
  When each issues its node intermediate
  Then both are CA:TRUE with pathLenConstraint=0
  And both carry an identical key-usage profile
  And each chains to its own root
```

### S-03-04 — Intermediate chains to root with `openssl verify` (KPI K1)

- **Scaffold**: `rcgen_ca_chain_verify.rs::rcgen_intermediate_chains_to_root_via_openssl_verify`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-03` · **Trace**: US-CA-03 AC2, KPI K1, journey step 2
- **Universe**: `openssl verify` exit (0) + x509-parser extensions `{is_ca, pathLen=0, keyCertSign, keyUsage_critical}` on real bytes

```gherkin
Scenario: The node intermediate chains to the root and is pathLen-constrained
  Given a persistent root CA
  When the platform issues a node intermediate CA at bootstrap
  Then "openssl verify -CAfile root.pem intermediate.pem" exits 0
  And the real cert carries CA:TRUE, pathLenConstraint=0, keyCertSign, keyUsage critical
```

### S-03-05 — pathLen=0 is enforced, not merely set (sad path)

- **Scaffold**: `rcgen_ca_chain_verify.rs::rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-03 @error` · **Trace**: US-CA-03 AC3, research Finding 4, journey step 2
- **Universe**: `openssl verify` exit on a constructed `root → intermediate → further-CA` chain — expected non-zero (verification fails)

```gherkin
Scenario: The intermediate cannot mint a further CA
  Given a node intermediate CA with pathLen=0
  When a chain is constructed in which that intermediate signs a further CA certificate
  Then chain verification fails
```

### S-03-06 — Intermediate signing failure fails node bootstrap loudly (sad path)

- **Scaffold**: `ca_boot_and_audit.rs::intermediate_signing_failure_fails_node_bootstrap_loudly`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-03 @error` · **Trace**: US-CA-03 AC4, journey error_paths step 2
- **Universe**: node-bootstrap outcome (fails loudly with a typed `CaError`) + absence of half-provisioned workload state

```gherkin
Scenario: Intermediate signing failure fails node bootstrap loudly
  Given the root key is unavailable at node bootstrap (decrypt failed upstream)
  When the platform attempts to issue the node intermediate
  Then issue_intermediate surfaces a typed CaError
  And node bootstrap fails loudly with no half-provisioned state
```

---

## Slice 04 — Workload SVID with single SPIFFE URI SAN (US-CA-04)

**Story**: US-CA-04 · **Job**: J-SEC-001 · **KPI**: K1 (chain-verify), K2
(single URI SAN), K5 (DST serials)
**Hypothesis**: the platform mints a SPIFFE-spec-compliant SVID that validates
through the full 3-tier chain, with DST-deterministic serials.

### S-04-01 — SVID profile carries exactly one URI SAN + leaf key usage (PROPERTY, K2)

- **Scaffold**: `ca_cert_spec_policy.rs::svid_spec_carries_exactly_one_uri_san_and_leaf_key_usage`
- **Layer**: L1 (pure, PBT-full at GREEN) · **Tags**: `@in-memory @property @S-04` · **Trace**: US-CA-04 AC1, KPI K2, ADR-0063 D5, research Finding 2
- **Universe**: `CertSpec` profile — `{role, san_uris, is_ca, key_usages, key_usage_critical}`
- **DELIVER note**: implement as `proptest!` over a `SpiffeId` strategy yielding exactly one `spiffe://` URI

```gherkin
Property: For any SpiffeId yielding exactly one URI SAN, the SVID profile is a spec-compliant leaf
  Given any SpiffeId whose SAN projection yields exactly one spiffe:// URI
  When the platform constructs the SVID CertSpec
  Then the profile is CA:FALSE
  And it carries exactly that one URI SAN
  And keyUsage is digitalSignature marked critical
  And it carries NO keyCertSign or cRLSign
```

### S-04-02 — SVID rejects 0 or ≥2 URI SANs before any cert (PROPERTY, K2 negative)

- **Scaffold**: `ca_cert_spec_policy.rs::svid_spec_rejects_zero_or_multiple_uri_sans_before_any_cert`
- **Layer**: L1 (pure, PBT-full at GREEN) · **Tags**: `@in-memory @property @S-04 @error` · **Trace**: US-CA-04 AC2, KPI K2, research Finding 2, Hebert ch.6 negative-testing
- **Universe**: the `Result` of `CertSpec::svid(..)` — expected `Err(CertSpecError::InvalidSan)` for every 0-or-≥2 input, and the absence of any partial spec escaping
- **DELIVER note**: `proptest!` over a strategy generating 0 and ≥2 URI-SAN inputs

```gherkin
Property: For any SAN projection yielding zero or two-or-more URI SANs, issuance is rejected before any cert
  Given any SAN projection that would yield zero or two-or-more spiffe:// URI SANs
  When the platform attempts to construct the SVID CertSpec
  Then it returns CertSpecError::InvalidSan
  And no partial certificate spec escapes
```

### S-04-03 — SVID subject URI equals the requested SpiffeId exactly

- **Scaffold**: `ca_cert_spec_policy.rs::svid_spec_subject_uri_equals_requested_spiffe_id`
- **Layer**: L1 (pure) · **Tags**: `@in-memory @S-04` · **Trace**: US-CA-04 AC1 (example-pinned readability companion to S-04-01)
- **Universe**: `CertSpec.san_uris` — exactly `[spiffe://overdrive.local/job/payments/alloc/a1b2c3]`

```gherkin
Scenario: The SVID subject URI equals the requested SpiffeId exactly
  Given a request to identify allocation a1b2c3 of job payments
  When the platform constructs the SVID CertSpec
  Then the sole URI SAN is spiffe://overdrive.local/job/payments/alloc/a1b2c3
  And the canonical-lowercase form is preserved through the SpiffeId newtype
```

### S-04-04 — SVID serial is deterministic and ≥64 bits (sim, KPI K5)

- **Scaffold**: `sim_ca_deterministic.rs::sim_ca_svid_serial_is_deterministic_and_at_least_64_bits`
- **Layer**: L2 (sim) · **Tags**: `@in-memory @S-04` · **Trace**: US-CA-04 AC4, KPI K5, research Finding 10 (CA/B Forum 64-bit floor)
- **Universe**: `SimCa::issue_svid(&req)` serial bytes (drawn via `SeededEntropy::fill`) — byte-equal across two same-seed runs AND `serial.len() * 8 >= 64`

```gherkin
Scenario: SVID serial numbers are CSPRNG and DST-deterministic
  Given the seeded DST harness with the sim CA adapter at seed 0x5EED
  When the platform mints an SVID twice at that seed
  Then both serials are byte-identical
  And each serial is at least 64 bits wide
```

### S-04-05 — Sim SVID carries single URI SAN and is not a CA (shared core policy)

- **Scaffold**: `sim_ca_deterministic.rs::sim_ca_svid_carries_single_uri_san_and_is_not_a_ca`
- **Layer**: L2 (sim) · **Tags**: `@in-memory @S-04` · **Trace**: US-CA-04 AC1, ADR-0063 D5 (sim shares core `CertSpec`)
- **Universe**: SimCa SVID trait-accessor surface — `{san_uris (cardinality + value), is_ca}`

```gherkin
Scenario: The sim SVID carries exactly one URI SAN and is not a CA
  Given the sim CA adapter sharing the core CertSpec policy
  When it issues an SVID for a SpiffeId
  Then the leaf carries exactly one URI SAN equal to that SpiffeId
  And the leaf is CA:FALSE
```

### S-04-06 — SVID profile equivalence across host and sim adapters (K2 shared contract)

- **Scaffold**: `ca_equivalence.rs::ca_equivalence_svid_profile_matches_across_host_and_sim`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-04` · **Trace**: US-CA-04, KPI K2, ADR-0063 D8 (proves sim does not diverge on policy)
- **Universe**: SVID contract-observable profile via trait accessors — `{is_ca, san_uris (cardinality + value), key_usages, issuer_linkage}`

```gherkin
Scenario: The SVID profile is identical whether produced by host or sim
  Given the host CA adapter and the sim CA adapter
  When each issues an SVID for the same SpiffeId
  Then both leaves are CA:FALSE
  And both carry exactly one URI SAN equal to that SpiffeId
  And both carry keyUsage=digitalSignature marked critical
```

### S-04-07 — Full Root → Intermediate → SVID chain verifies (WALKING SKELETON, K1)

- **Scaffold**: `rcgen_ca_chain_verify.rs::rcgen_full_svid_chain_verifies_root_intermediate_svid`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @walking_skeleton @S-04` · **Trace**: US-CA-04 AC3, KPI K1 (North Star), journey step 4, DISCUSS walking-skeleton (D2 completion)
- **Universe**: `openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem` exit code (0)

```gherkin
Scenario: A workload SVID validates through the full Root -> Intermediate -> SVID chain
  Given a persistent root, a node intermediate, and a request to identify allocation a1b2c3 of job payments
  When the platform mints the workload SVID via the host adapter
  Then "openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem" exits 0
```

### S-04-08 — Real SVID leaf carries exactly one URI SAN + leaf profile (K2 on real bytes)

- **Scaffold**: `rcgen_ca_chain_verify.rs::rcgen_svid_leaf_carries_exactly_one_uri_san_and_leaf_profile`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-04` · **Trace**: US-CA-04 AC1, KPI K2, research Finding 1/6 (1h TTL)
- **Universe**: x509-parser-observed extensions on real SVID bytes — `{san_uris (cardinality=1, value), is_ca=false, keyUsage=digitalSignature critical, no keyCertSign/cRLSign, validity ~1h}`

```gherkin
Scenario: The real SVID leaf is SPIFFE-spec-compliant
  Given a minted workload SVID for spiffe://overdrive.local/job/payments/alloc/a1b2c3
  When the real certificate bytes are inspected
  Then the leaf carries exactly one URI SAN equal to that SpiffeId
  And it is CA:FALSE with keyUsage=digitalSignature marked critical
  And it carries no keyCertSign or cRLSign
  And its validity window is approximately one hour
```

### ~~S-04-09~~ — **RETIRED 2026-06-06** (Option A — type-enforced)

- ~~**Scaffold**: `rcgen_ca_chain_verify.rs::rcgen_svid_request_with_bad_san_cardinality_is_rejected_pre_issuance`~~
- **RETIRED 2026-06-06.** This scenario tested the host adapter rejecting a
  bad-SAN-cardinality `SvidRequest` — a path the request type (`SvidRequest {
  spiffe_id: SpiffeId }`, exactly one validated identity by construction) makes
  **unreachable**. Under the user-ratified Option A (ADR-0063 D5
  enforcement-location amendment, 2026-06-06), there is no `CaError::InvalidSan`
  branch inside `issue_svid` to test: the type makes ≠1 unrepresentable, the
  single fallible parse is the pure-core `CertSpec::svid` (tested at L1 by
  **S-04-02**), and the SPIFFE-spec-mandated runtime reject (X.509-SVID §5.2) is
  at the relying-party verifier (#26), out of this feature's scope. **Redundant
  coverage**: **S-04-08** already asserts the host leaf carries exactly one URI
  SAN. The crafter retires this scaffold fn; the row is kept struck-through for
  traceability, not deleted. Research:
  `docs/research/security/svid-request-cardinality-enforcement-research.md`
  (SPIFFE §2/§5.2; SPIRE single-`spiffeid.ID` reference impl; "parse, don't
  validate").

### ~~S-04-10~~ — **RETIRED 2026-06-06** (Option A — type-enforced)

- ~~**Scaffold**: `ca_equivalence.rs::ca_equivalence_bad_san_request_rejected_identically_by_both`~~
- **RETIRED 2026-06-06.** This scenario asserted both adapters reject a
  bad-SAN-cardinality request identically — again a type-unreachable path under
  Option A (see S-04-09 above). The cross-adapter SAN-cardinality equivalence it
  was meant to guard is already covered by **S-04-06**
  (`ca_equivalence_svid_profile_matches_across_host_and_sim`), whose Universe
  includes `san_uris (cardinality + value)`. The crafter retires this scaffold
  fn; the row is kept struck-through for traceability. Research:
  `docs/research/security/svid-request-cardinality-enforcement-research.md`.

---

## Slice 05 — Trust bundle, issued-cert audit, re-issue on demand (US-CA-05)

**Story**: US-CA-05 · **Job**: J-SEC-001 · **KPI**: K1 (verify against bundle)
**Hypothesis**: an SVID validates against the platform-composed bundle,
issuance is auditable, and re-issue works without restart.

### S-05-01 — Re-issue under the sim adapter yields a fresh distinct leaf

- **Scaffold**: `sim_ca_deterministic.rs::sim_ca_reissue_for_same_spiffe_id_yields_a_fresh_distinct_leaf`
- **Layer**: L2 (sim) · **Tags**: `@in-memory @S-05` · **Trace**: US-CA-05 AC3, the #40 rotation re-issue mechanism
- **Universe**: two consecutive `SimCa::issue_svid` results for the SAME SpiffeId — expected DISTINCT serials / validity windows (determinism is per-call-sequence, not per-SpiffeId-cached)

```gherkin
Scenario: Re-issuing for the same SpiffeId yields a fresh, distinct leaf
  Given the sim CA adapter
  When it issues an SVID twice for the same SpiffeId in sequence
  Then each issuance yields a distinct serial and a new validity window
  And no per-SpiffeId cached leaf is reused
```

### S-05-02 — Trust-bundle shape equivalence across host and sim adapters

- **Scaffold**: `ca_equivalence.rs::ca_equivalence_trust_bundle_shape_matches_across_host_and_sim`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-05` · **Trace**: US-CA-05 AC1, ADR-0063 D1 (`trust_bundle()` contract)
- **Universe**: `trust_bundle()` composition shape via trait accessors — `{root_anchor, intermediate_as_untrusted_chain}`; plus each adapter's own leaf verifies against its own bundle (cross-adapter mixing NOT asserted — different roots)

```gherkin
Scenario: The trust bundle composition shape is identical across host and sim
  Given the host CA adapter and the sim CA adapter
  When each composes its trust bundle
  Then both bundles are shaped as a root anchor plus intermediate as untrusted chain material
  And each adapter's own leaf verifies against its own bundle
```

### S-05-03 — Issuance writes an `issued_certificates` audit row matching the cert

- **Scaffold**: `ca_boot_and_audit.rs::issuance_writes_issued_certificates_row_matching_the_minted_cert`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-05` · **Trace**: US-CA-05 AC2, ADR-0063 D6, journey step 4
- **Universe**: the `issued_certificates` observation row read back via the ObservationStore — `{serial, spiffe_id, issuer_serial}` exactly match the minted cert

```gherkin
Scenario: Every issuance writes an issued-certificates audit row
  Given the platform mints a workload SVID
  When the issued_certificates observation row is read back
  Then the row's serial, spiffe_id, and issuer_serial match the minted cert
```

### S-05-04 — Issuance that cannot write its audit row surfaces an error (sad path)

- **Scaffold**: `ca_boot_and_audit.rs::issuance_that_cannot_write_audit_row_surfaces_an_error`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-05 @error` · **Trace**: US-CA-05 AC4, ADR-0063 D6 (no silent issuance), journey ("No silent issuance")
- **Universe**: issuance result when the audit-row write fails — expected `Err(CaError)` + absence of any handed-out unaudited certificate

```gherkin
Scenario: Issuance is never silent
  Given an issuance whose issued_certificates row cannot be written
  When the platform attempts to mint the certificate
  Then issuance surfaces a CaError
  And no unaudited certificate is handed out
```

### S-05-05 — SVID re-issued on demand without control-plane restart

- **Scaffold**: `ca_boot_and_audit.rs::svid_is_reissued_on_demand_without_control_plane_restart`
- **Layer**: L3 (real-io, gated) · **Tags**: `@real-io @adapter-integration @S-05` · **Trace**: US-CA-05 AC3, the #40 rotation mechanism
- **Universe**: the re-issued leaf (distinct serial, new validity) + the absence of any control-plane restart between issuances

```gherkin
Scenario: An SVID is re-issued on demand without restarting the control plane
  Given an existing SVID for spiffe://overdrive.local/job/payments/alloc/a1b2c3
  When the platform re-issues a fresh SVID for that SpiffeId
  Then a new leaf with a distinct serial and new validity window is produced
  And the control plane is not restarted
```

### S-05-06 — `IssuedCertificateRow` V1 golden-bytes roundtrip (schema evolution)

- **Scaffold**: `schema_evolution/issued_certificate_row.rs::issued_certificate_row_envelope_v1_golden_bytes_roundtrip`
- **Layer**: L1 (archive, default lane) · **Tags**: `@property @S-05` (S-EV-CA-02) · **Trace**: ADR-0063 D6, ADR-0048, research Finding 15
- **Universe**: the canonical `Latest` projection after `into_latest()`, exact-equality against the hand-pinned canonical row

```gherkin
Scenario: The issued-certificate audit row layout is byte-stable across versions
  Given the pinned V1 archived bytes of the IssuedCertificateRow envelope (FIXTURE_V1)
  When the bytes are rkyv-deserialised and projected via into_latest()
  Then the result equals the canonical Latest projection
  And any future field appended to V1 (rather than minting V2) breaks this test
```

### S-05-07 — `IssuedCertificateRow` discriminant offset triangulates

- **Scaffold**: `schema_evolution/issued_certificate_row.rs::issued_certificate_row_envelope_discriminant_offset_triangulates`
- **Layer**: L1 (archive, default lane) · **Tags**: `@property @S-05` · **Trace**: ADR-0048 (two-source discriminant pin)
- **Universe**: independent discriminant pin vs `IssuedCertificateRowEnvelope::discriminant_offset_from_end()` — exact agreement

```gherkin
Scenario: The audit-row envelope discriminant offset is pinned from two independent sources
  Given an independent pin of the V1 discriminant offset
  When it is compared against discriminant_offset_from_end()
  Then the two agree
```

### S-05-08 — Unknown audit-row version surfaces an error (observation log-and-skip)

- **Scaffold**: `schema_evolution/issued_certificate_row.rs::issued_certificate_row_envelope_unknown_version_probe_surfaces_error`
- **Layer**: L1 (archive, default lane) · **Tags**: `@property @S-05 @error` · **Trace**: ADR-0048 (asymmetric unknown handling — observation tolerates, intent refuses)
- **Universe**: the `EnvelopeError` surfaced by `probe_known_variant` (vs a garbage decode); observation path logs + skips the row, convergence proceeds for surviving rows

```gherkin
Scenario: An unknown audit-row version fails fast rather than decoding garbage
  Given an unknown/forward audit-row envelope version
  When the observation decode path probes the variant
  Then it surfaces an EnvelopeError
  And the row is logged-and-skipped (convergence proceeds for surviving rows)
```

---

## Traceability matrix (story → scenario IDs → KPI)

| Story | Slice | Scenario IDs | KPIs covered |
|---|---|---|---|
| US-CA-01 (root behind port trait) | 01 | S-01-01, S-01-02, S-01-03, S-01-04, S-01-05 | K1, K5 |
| US-CA-02 (root key envelope-encrypted) | 02 | S-02-01 … S-02-10 | K3, K5 |
| US-CA-03 (pathLen-0 intermediate) | 03 | S-03-01 … S-03-06 | K1 |
| US-CA-04 (workload SVID, single URI SAN) | 04 | S-04-01 … S-04-08 (S-04-09, S-04-10 RETIRED 2026-06-06 — Option A) | K1, K2, K5 |
| US-CA-05 (bundle, audit, re-issue) | 05 | S-05-01 … S-05-08 | K1 |

Every user story has ≥4 scenarios (Dim 8 Check A — zero untraceable stories;
zero stories with zero scenarios). K1–K5 all covered (K4 is an
architecture-review KPI — no executable scenario, asserted at handoff per
DISCUSS measurement plan).

## Coverage profile

- **Total scenarios**: **37** (was 39; **S-04-09 + S-04-10 RETIRED 2026-06-06**
  under Option A — type-enforced; see § Slice 04 retirement notes). One scaffold
  fn per scenario; the crafter retires the two retired scaffold fns, leaving 37
  across the 8 files.
- **Error / sad-path** (`@error`): **13 = 35.1%** (was 15/39 = 38.5%; both
  retired scenarios were `@error`). `cert_spec_error_variants` (S-01-05) is one
  of these 13. This is a **non-gating DISTILL metric** — see § Finding.
- **Walking skeleton** (`@walking_skeleton`): 1 (S-04-07)
- **Property** (`@property`, PBT-full at GREEN for L1): 8 (S-02-08/09/10,
  S-04-01/02, S-05-06/07/08)
- **By layer**: L1 pure 6 · L2 sim 5 · L1 archive 6 · **L3 real-io 20** (was 22)

## Finding (reported, not auto-fixed)

The `@error` ratio is **13/37 = 35.1%** (was 15/39 = 38.5% before the
2026-06-06 retirement of S-04-09 + S-04-10, both `@error`), under the Dim-1 /
BDD 40% target. This is a **non-gating DISTILL metric**, and the drop is the
**accepted, honest consequence of the type-honest Option-A design**: the
bad-SAN-cardinality path that S-04-09/S-04-10 tested is *unrepresentable* at the
adapter (the request type carries exactly one validated `SpiffeId`), so the two
`@error` scenarios were dead-code tests of a branch that cannot exist. Removing
them makes the scenario count honest rather than padding it. The live
single-URI-SAN guard is the L1 `CertSpec::svid` reject (S-04-02, `@property`);
the spec-mandated runtime reject is at the relying-party verifier (#26, SPIFFE
X.509-SVID §5.2). The eight scaffold files remain the **authored SSOT** (two
fns retired by the crafter). Surfaced for the orchestrator; not auto-expanded.

> **Documentation-completeness note**: an earlier draft of this document mapped
> only 38 of the then-39 scaffolds — the cross-role
> `cert_spec_error_variants_are_distinct_per_failure_mode` scaffold (tagged
> `@error @in-memory`, spanning `@S-01 @S-04`) was folded into the S-01
> narrative without its own row; it is documented as **S-01-05** and is one of
> the 13 surviving `@error` scenarios.

## Pre-DELIVER RED classification

All 37 surviving scaffolds (was 39; S-04-09 + S-04-10 retired 2026-06-06 under
Option A — the crafter removes those two scaffold fns) are RED-at-the-bar via
the project's `#[should_panic(expected = "RED scaffold")]` convention
(`.claude/rules/testing.md` § "RED scaffolds"): nextest reports PASS (the
expected panic fires), so the RED state is *discoverable and hook-compatible*
without `--no-verify`. DELIVER replaces each `panic!` body with the real
assertions (L1/L2 default-lane scaffolds) or real crypto/keyring calls (L3 gated
scaffolds), flipping `#[should_panic]` off as each goes GREEN. Classification:
**MISSING_FUNCTIONALITY** for all 37 (the production `Ca` trait / `CertSpec` /
`RcgenCa` / `SimCa` / envelopes do not exist yet) — none are
IMPORT_ERROR/FIXTURE_BROKEN (the scaffolds import no unbuilt production types
by design).
