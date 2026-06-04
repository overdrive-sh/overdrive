# Research: Built-in CA (rcgen + rustls) -- Root CA, Per-Node Intermediate CA, and SVID Issuance/Rotation

**Date**: 2026-06-04 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 18

> **CORRECTION (2026-06-05, built-in-ca DESIGN #28)** — This document originally
> stated the workspace uses **aws-lc-rs** as its rustls/rcgen crypto provider.
> That is **not true of the current codebase**: the workspace is on **`ring`**
> (`rustls` and `rcgen` both pin the `ring` feature; no crate enables
> `aws-lc-rs`). ADR-0039's aws-lc-rs adoption was *accepted* (2026-05-04) but
> **never implemented** — tracked by **#204**. This affects exactly one claim:
> **FIPS 140-3** (Cert #4816) requires aws-lc-rs and is therefore **pending
> #204**. Everything else holds — `ring` provides AES-256-GCM, HKDF-SHA256, and
> ECDSA P-256, so the built-in-CA design (HKDF→AES-256-GCM envelope, P-256
> hierarchy) works unchanged on `ring` today. Read every "aws-lc-rs" below as
> "the workspace crypto backend (ring today; aws-lc-rs after #204)" unless it is
> specifically about FIPS.

## Executive Summary

Overdrive's built-in 3-tier CA hierarchy (Root CA -> Node Intermediate CA -> Workload SVID) is fully implementable with the existing crate stack: **rcgen** for X.509 certificate generation, **rustls** with **aws-lc-rs** for TLS termination and FIPS 140-3 compliance. rcgen 0.14.x (target **0.14.8**; the workspace currently pins 0.13.2 and must be bumped — the core extension APIs `SanType::URI(Ia5String)`, `IsCa::Ca(BasicConstraints::Constrained(0))`, and the `aws_lc_rs` feature exist across 0.13.2→0.14.x, but the 0.14 cert-builder surface changed from 0.13, so the exact 0.14.8 API is verified at first compile) provides every X.509v3 extension the SPIFFE X.509-SVID specification requires, including `SanType::URI` for SPIFFE URI SANs, `IsCa::Ca(BasicConstraints::Constrained(0))` for pathLen-restricted intermediate CAs, and the full `keyUsage`/`extendedKeyUsage` surface. The crypto stack is aligned end-to-end: rcgen and rustls share one backend — **`ring` in the current workspace** (aws-lc-rs is ADR-0039's intended provider but is not yet implemented; tracked by #204) — with P-256 (ECDSA) as the default signing algorithm. FIPS 140-3 Cert #4816 is available **only after the #204 switch to aws-lc-rs**; on `ring` today there is no FIPS validation.

Certificate rotation follows the pattern established by SPIRE, Istio, and Linkerd: 1-hour SVID TTL with renewal at 50% of lifetime (30 minutes), keys generated locally on the node (never transmitted). The whitepaper (§18 "Primitive Composition") explicitly designates certificate rotation as a **workflow** — a multi-step durable sequence (DNS propagation → validation → trust-anchor swap → retirement) that terminates with `Ok(result)`, not a reconciler that converges forever. `WorkflowSpec::cert_rotation` is the named primitive in the DST harness (§21). In-flight kTLS sessions are unaffected by certificate rotation — session keys are independent of the certificate used to authenticate the handshake, so no connection draining is needed.

Root CA key encryption at rest in the IntentStore is best served by envelope encryption using aws-lc-rs AES-256-GCM (authenticated encryption), with a key-encryption-key derived from an operator passphrase. This uses the same crypto library already in the dependency graph and naturally extends to HSM/KMS integration in later phases. Root CA rotation follows SPIRE's two-phase model (prepare at 50% TTL, activate at 83% TTL) with both roots in the trust bundle during transition.

The built-in CA's primary advantage over SPIRE is operational simplicity: no separate daemon, no gRPC channel between server and agent, no plugin system to configure. The primary risk is that Overdrive's node attestation at bootstrap must be handled by the bootstrap ceremony itself, since there is no pluggable attestation model. For Phase 5, a join-token or mTLS-bootstrapped attestation (matching the Talos model from the existing bootstrap research) is the recommended path.

## Research Methodology

**Search Strategy**: Primary-source traversal from official specifications (SPIFFE X.509-SVID spec at spiffe.io, RFCs 5280/6125), crate documentation (docs.rs for rcgen, rustls, aws-lc-rs), and cross-referenced against SPIRE/Istio/Linkerd production implementations. Local codebase analysis of existing `tls_bootstrap.rs`, ADR-0010, ADR-0039, and whitepaper sections 4 and 8.

**Source Selection**: Types: official specifications, crate documentation, open-source project documentation | Reputation: high (0.9+) | Verification: cross-referencing across specification, implementation docs, and production precedent.

**Quality Standards**: Target 3 sources/claim (min 1 authoritative) | All major claims cross-referenced | Avg reputation: [in progress]

---

## Findings

### Finding 1: rcgen Supports All X.509 Extensions Required for SPIFFE-Compliant CA Hierarchy

**Evidence**: rcgen 0.14.x exposes `CertificateParams` with fields for every X.509v3 extension the SPIFFE X.509-SVID specification requires:

- `is_ca: IsCa` -- controls `basicConstraints` (CA:TRUE/FALSE). `IsCa::Ca(BasicConstraints::Constrained(n))` sets `pathLenConstraint` to `n`; `IsCa::Ca(BasicConstraints::Unconstrained)` omits pathLen; `IsCa::NoCa` sets CA:FALSE.
- `key_usages: Vec<KeyUsagePurpose>` -- supports `DigitalSignature`, `KeyCertSign`, `CrlSign`, `KeyEncipherment`, `KeyAgreement`, etc.
- `extended_key_usages: Vec<ExtendedKeyUsagePurpose>` -- supports `ServerAuth`, `ClientAuth`, and custom OIDs.
- `subject_alt_names: Vec<SanType>` -- includes `SanType::URI(Ia5String)` for SPIFFE URI SANs (`spiffe://overdrive.local/job/payments/alloc/a1b2c3`).
- `serial_number: Option<SerialNumber>` -- manual or auto-generated.
- `crl_distribution_points: Vec<CrlDistributionPoint>` -- CRL DP extension per RFC 5280 section 4.2.1.13.
- `custom_extensions: Vec<CustomExtension>` -- arbitrary X.509v3 extensions.
- `name_constraints: Option<NameConstraints>` -- URI name constraints for CA scope limitation.
- `use_authority_key_identifier_extension: bool` -- AKI extension.

The existing codebase (`crates/overdrive-control-plane/src/tls_bootstrap.rs`) already exercises `IsCa::Ca(BasicConstraints::Unconstrained)`, `KeyUsagePurpose::KeyCertSign`, `SanType::IpAddress`, `SanType::DnsName`, `ExtendedKeyUsagePurpose::ServerAuth`, and `ExtendedKeyUsagePurpose::ClientAuth` -- confirming these APIs work in practice.

**Source**: [rcgen CertificateParams docs.rs](https://docs.rs/rcgen/latest/rcgen/struct.CertificateParams.html) - Accessed 2026-06-04
**Confidence**: High
**Verification**: [rcgen SanType docs.rs](https://docs.rs/rcgen/latest/rcgen/enum.SanType.html), [existing tls_bootstrap.rs in codebase], [rcgen GitHub](https://github.com/rustls/rcgen)
**Analysis**: rcgen covers 100% of the X.509v3 extension surface required by the SPIFFE X.509-SVID specification and Overdrive's 3-tier CA hierarchy. No alternative crate (x509-cert, openssl) is needed. The `SanType::URI` variant is the critical piece for SPIFFE identity -- it accepts an `Ia5String` containing the SPIFFE URI.

### Finding 2: SPIFFE X.509-SVID Specification Defines Strict Certificate Profile

**Evidence**: The SPIFFE X.509-SVID specification (spiffe/spiffe repository, standards/X509-SVID.md) defines these mandatory requirements:

**Leaf SVID (workload certificate):**
- MUST contain exactly one URI SAN with the SPIFFE ID
- Validators MUST reject SVIDs containing more than one URI SAN
- MAY contain additional SAN types (DNS, IP) alongside the single URI SAN
- `basicConstraints`: `cA` field MUST be set to `false`
- `keyUsage`: MUST be set, MUST be marked critical, MUST include `digitalSignature`, MUST NOT include `keyCertSign` or `cRLSign`
- `extendedKeyUsage`: SHOULD be included; when present, `id-kp-serverAuth` and `id-kp-clientAuth` are permitted
- Subject field is not required; if omitted, URI SAN MUST be marked critical

**Signing/CA certificate:**
- MUST set `cA` field to `true` in `basicConstraints`
- MUST set `keyCertSign` in `keyUsage`; `keyUsage` MUST be marked critical
- MAY set `cRLSign` in `keyUsage`
- MAY set `pathLenConstraint`
- MAY apply URI name constraints to limit issuance scope
- SHOULD have SPIFFE ID without path component (trust domain only)

**Trust bundle:**
- Uses RFC 7517 JWK format
- `use` parameter MUST be `x509-svid`
- `x5c` parameter MUST contain exactly one base64-encoded DER CA certificate
- CA certificate SHOULD be self-signed

The spec does NOT specify: serial number format, signature algorithm, or validity period requirements.

**Source**: [SPIFFE X.509-SVID Specification](https://github.com/spiffe/spiffe/blob/main/standards/X509-SVID.md) - Accessed 2026-06-04
**Confidence**: High
**Verification**: [SPIFFE official docs](https://spiffe.io/docs/latest/spiffe-specs/x509-svid/)
**Analysis**: Every requirement is satisfiable by rcgen's API. The "exactly one URI SAN" rule is the most critical constraint -- Overdrive's SVID generation must ensure no second URI SAN is accidentally added. The trust bundle JWK format is a serialization concern separate from certificate generation.

### Finding 3: rcgen Uses aws-lc-rs (or ring) for Cryptographic Operations via Feature Flag

**Evidence**: rcgen 0.14.x uses a cargo feature to select the crypto backend:
- Default: `ring` backend
- Feature `aws_lc_rs`: switches to `aws-lc-rs`
- `KeyPair::generate()` defaults to `PKCS_ECDSA_P256_SHA256` (P-256 curve, SHA-256 hash per RFC 5758)
- `KeyPair::generate_for(alg)` allows explicit algorithm selection
- `KeyPair` implements the `SigningKey` trait, used directly by `CertificateParams::self_signed(&key)` and `CertificateParams::signed_by(&key, &issuer_cert, &issuer_key)`
- RSA key generation is only available with the `aws_lc_rs` feature

The workspace `Cargo.toml` declares `rcgen` and `rustls` on the **`ring`** feature today (NOT aws-lc-rs — ADR-0039's aws-lc-rs adoption is unimplemented, tracked by #204). The signing chain is provider-agnostic: rcgen generates/loads a `KeyPair` backed by the selected backend (ring today), uses it to sign X.509 certificates, and rustls consumes the resulting PEM for TLS handshakes using the same provider. The `aws_lc_rs` feature is selected only after the #204 switch.

**Source**: [rcgen KeyPair docs.rs](https://docs.rs/rcgen/latest/rcgen/struct.KeyPair.html) - Accessed 2026-06-04
**Confidence**: High
**Verification**: [ADR-0039 in codebase], [rcgen crates.io](https://crates.io/crates/rcgen)
**Analysis**: The crypto stack is fully aligned: rcgen (certificate generation) and rustls (TLS termination) share one backend — **`ring` today** (aws-lc-rs after #204). P-256 (ECDSA) is the right default -- it matches Phase 1's existing usage, is provided by both ring and aws-lc-rs, and is the most widely supported curve across TLS implementations. FIPS 140-3 compliance (aws-lc-rs Cert #4816) is contingent on the #204 provider switch; `ring` is not FIPS-validated. P-384 is available as an upgrade path for environments requiring 192-bit security.

### Finding 4: rcgen Supports Intermediate CA Certificates with pathLenConstraint

**Evidence**: The `IsCa` enum provides three variants:
- `IsCa::Ca(BasicConstraints::Unconstrained)` -- CA:TRUE with no pathLen (for root CA)
- `IsCa::Ca(BasicConstraints::Constrained(0))` -- CA:TRUE with pathLen=0 (for intermediate CA that can only issue leaf certs, not further intermediates)
- `IsCa::NoCa` -- CA:FALSE (for leaf SVIDs)

For the Overdrive 3-tier hierarchy:
- Root CA: `IsCa::Ca(BasicConstraints::Unconstrained)` -- can issue node intermediate CAs
- Node Intermediate CA: `IsCa::Ca(BasicConstraints::Constrained(0))` -- can issue leaf SVIDs only, cannot create further intermediates
- Workload SVID: `IsCa::NoCa` -- leaf certificate

The `name_constraints` field on `CertificateParams` enables URI name constraints, allowing the root CA to constrain intermediate CAs to issue only within the `spiffe://overdrive.local/` trust domain.

**Source**: [rcgen CertificateParams docs.rs](https://docs.rs/rcgen/latest/rcgen/struct.CertificateParams.html) - Accessed 2026-06-04
**Confidence**: High
**Verification**: [rcgen IsCa enum docs.rs](https://docs.rs/rcgen/latest/rcgen/enum.IsCa.html), [existing tls_bootstrap.rs in codebase]
**Analysis**: The pathLen=0 constraint on node intermediate CAs is a defense-in-depth measure -- even if a node's intermediate key is compromised, it cannot mint further intermediates, limiting blast radius to leaf SVIDs.

### Finding 5: SPIRE Architecture -- Comparison Model for Built-in CA

**Evidence**: SPIRE (SPIFFE Runtime Environment) implements a two-tier architecture:

**SPIRE Server** (centralized):
- Functions as the CA -- signs and issues all SVIDs within its trust domain
- Stores registration entries (which workloads get which SPIFFE IDs)
- Maintains signing keys
- Performs node attestation (verifies agent identity via platform-specific mechanisms: AWS IID, GCE metadata, join tokens, etc.)
- Default CA TTL: 24 hours; default SVID TTL: 1 hour
- By default acts as its own root CA; can plug in an upstream CA via `UpstreamAuthority` plugin

**SPIRE Agent** (per-node):
- Runs on every node containing workloads
- Requests SVIDs from server and caches them locally
- Exposes the SPIFFE Workload API via Unix domain socket
- Attests workload identity through OS-level queries (kernel PID, cgroup, Docker labels, etc.)
- Generates private keys for X.509-SVIDs locally via key manager plugins -- keys never leave the node
- Distributes cached SVIDs to requesting processes

**Critical design choice**: private keys are generated on the agent (on the node), never on the server. The server signs CSRs; it never sees workload private keys. This is the same model Overdrive's whitepaper specifies -- "The node agent issues short-lived leaf certificates (SVIDs, 1hr TTL) for each workload it runs, using its intermediate."

**Source**: [SPIRE Concepts](https://spiffe.io/docs/latest/spire-about/spire-concepts/) - Accessed 2026-06-04
**Confidence**: High
**Verification**: [SPIRE Configuration](https://spiffe.io/docs/latest/deploying/configuring/), [SPIFFE/SPIRE Security Self-Assessment](https://tag-security.cncf.io/community/assessments/projects/spiffe-spire/self-assessment/)
**Analysis**: Overdrive's built-in CA collapses SPIRE's server + agent into a single binary with a reconciler-driven lifecycle. The key structural advantages over SPIRE: no separate daemon to deploy/manage, no gRPC channel between server and agent (everything is in-process on single-node, Raft-replicated in HA), and rotation is a reconciler action rather than a background polling loop. The key structural risk: without SPIRE's pluggable attestation model, Overdrive's node attestation at bootstrap must be handled by the bootstrap ceremony itself (join token or equivalent).

### Finding 6: Certificate Rotation Mechanics Across Production Platforms

**Evidence**: Three production platforms converge on similar rotation patterns:

**SPIRE**: Agent renews SVIDs at 50% of TTL (configurable). For 1-hour SVIDs, renewal triggers at 30 minutes. Jitter is recommended for large-scale deployments (SPIRE issue #4268) to avoid spiky renewal load. New keys and trust bundles are delivered before expiry via the Workload API.

**Istio (istiod/Citadel)**: Default workload cert TTL is 24 hours. Sidecar proxies automatically request renewal from istiod before expiry. When intermediate CA rotates, workloads gradually pick up new certs as existing ones expire -- full rollover takes one cert lifetime (24h by default).

**Linkerd**: Control plane identity service issues 24-hour workload certs. The identity issuer (intermediate CA) is managed by cert-manager for automatic rotation. Trust anchor (root CA) rotation requires manual intervention or cert-manager automation. Workload cert rotation is transparent to applications.

**Smallstep step-ca**: Renews at 2/3 of certificate lifetime. Goal is "seamless end-entity certificate rotation." Root rotation uses a dual-trust-bundle approach -- workloads trust both old and new roots during a transition period.

**Convergent pattern across all four**:
- Workload cert TTL: 1-24 hours
- Renewal at 50-67% of TTL (SPIRE: 50%, step-ca: 67%)
- Keys generated locally (never transmitted over wire)
- Intermediate CA rotation is transparent to workloads
- Root CA rotation requires dual-trust-bundle transition

**Source**: [SPIRE Concepts](https://spiffe.io/docs/latest/spire-about/spire-concepts/) - Accessed 2026-06-04
**Confidence**: High
**Verification**: [Linkerd auto-rotation](https://linkerd.io/2-edge/tasks/automatically-rotating-control-plane-tls-credentials/), [Istio cert rotation](https://istio.io/v1.3/docs/concepts/security/), [Smallstep design doc](https://smallstep.com/docs/design-document/)
**Analysis**: Overdrive's 1-hour SVID TTL with renewal at 50% (30 minutes) matches SPIRE's proven default. The whitepaper (§18) designates certificate rotation as a **workflow** — a multi-step durable sequence that terminates with `Ok(result)` — not a reconciler. This is the correct primitive: rotation is a bounded operation (generate key → sign cert → distribute → retire old), not an indefinite convergence loop. The workflow's `ctx.sleep()` + `ctx.call()` surface handles the sequential steps; a reconciler that "re-discovers" the rotation need every tick is the wrong shape. Jitter should be added to avoid thundering-herd renewal on large clusters.

### Finding 7: kTLS and Certificate Rotation -- In-Flight Connections Are Not Affected

**Evidence**: Once kTLS session keys are installed in the kernel, the kernel performs all encryption/decryption independently of rustls. The `rustls::kernel::KernelConnection` module provides "the bare minimum needed to implement a TLS connection that does its own encryption and decryption while still using rustls to manage connection secrets and session tickets."

Certificate rotation affects only NEW connections. An in-flight kTLS session uses the session keys negotiated during the TLS 1.3 handshake -- these keys are independent of the certificate used to authenticate the handshake. When a workload's SVID rotates, new connections use the new SVID; existing kTLS sessions continue with their already-installed keys until the connection closes naturally or the application-level protocol drains.

TLS 1.3 key updates (RFC 8446 section 4.6.3) allow session keys to be refreshed without a new handshake, and `KernelConnection` supports computing new traffic secrets on key update. However, the kTLS API requires the caller to track message counts and refresh keys before reaching the cipher suite's confidentiality limit (particularly relevant for AES-GCM).

**Source**: [rustls kernel module docs.rs](https://docs.rs/rustls/latest/rustls/kernel/index.html) - Accessed 2026-06-04
**Confidence**: High
**Verification**: [ktls crate docs.rs](https://docs.rs/ktls), [fasterthanli.me ktls article](https://fasterthanli.me/articles/ktls-now-under-rustls-org)
**Analysis**: This means SVID rotation is a non-event for in-flight connections -- no connection draining is needed. The rotation window only needs to ensure new SVIDs are available before old ones expire, not that existing connections switch. This simplifies the rotation workflow: mint new SVID, install it for new connections, let old connections drain naturally. The connection drain concern from the research questions is resolved -- there is no concern.

### Finding 8: Root CA Key Protection at Rest -- Three Viable Approaches

**Evidence**: Three approaches for encrypting the root CA private key when stored in the IntentStore (redb):

**Approach A: PKCS#8 Encrypted Private Key (pkcs8 crate)**
The RustCrypto `pkcs8` crate (with `encryption` + `pkcs5` features) supports:
- Key derivation: scrypt (RFC 7914) or PBKDF2-SHA256
- Cipher: AES-256-CBC (best available in PKCS#5v2)
- API: `PrivateKeyInfo::encrypt(password)` -> `EncryptedPrivateKeyInfo`; `EncryptedPrivateKeyInfo::decrypt(password)` -> `PrivateKeyInfo`
- Standard format: RFC 5958 EncryptedPrivateKeyInfo, interoperable with OpenSSL
- Limitation: AES-CBC only, not AES-GCM (PKCS#5v2 spec constraint)

**Approach B: Envelope Encryption with aws-lc-rs**
Use aws-lc-rs AEAD (AES-256-GCM) directly:
- Generate a data encryption key (DEK), encrypt the CA private key with it
- Encrypt the DEK with a key-encryption-key (KEK) derived from operator passphrase via scrypt/Argon2
- Store `{encrypted_dek, nonce, encrypted_private_key}` in IntentStore
- Advantage: AES-GCM authenticated encryption (integrity + confidentiality)
- Advantage: KEK can later be sourced from HSM/KMS without changing the format
- Disadvantage: non-standard format, not interoperable with `openssl pkcs8`

**Approach C: OS Keyring Integration**
macOS Keychain or Linux kernel keyring for the KEK:
- Deferred: adds OS-specific adapter complexity
- Phase 5+ consideration when hardware-backed key storage is in scope

**Recommended for Overdrive**: Approach B (envelope encryption with aws-lc-rs). Reasons:
1. Uses the same crypto library already in the dependency graph (aws-lc-rs)
2. AES-GCM is authenticated encryption (AES-CBC is not)
3. The envelope pattern naturally extends to HSM/KMS integration later
4. The KEK source is pluggable: Phase 5 starts with passphrase-derived; later phases can source from hardware

**Source**: [pkcs8 crate docs.rs](https://docs.rs/pkcs8) - Accessed 2026-06-04
**Confidence**: Medium (recommendation is analysis, not sourced)
**Verification**: [aws-lc-rs docs.rs](https://docs.rs/aws-lc-rs/latest/aws_lc_rs/), [envelopers crate](https://docs.rs/envelopers)
**Analysis**: The whitepaper says "root CA key lives in the IntentStore, encrypted at rest." The envelope encryption pattern satisfies this with the smallest dependency surface (aws-lc-rs is already required). The PKCS#8 approach is simpler but uses AES-CBC (unauthenticated). For a root CA key that is the trust anchor of the entire platform, authenticated encryption is the defensible choice.

### Finding 9: SPIRE's CA Rotation Uses Prepared/Active Two-Phase Model

**Evidence**: SPIRE's X.509 authority rotation follows a two-phase model:
1. **Prepare phase**: At 1/2 TTL of the currently active X.509 authority, a new authority is prepared. The new root certificate is added to the trust bundle and pushed to all agents/workloads, but the old authority continues signing SVIDs.
2. **Active phase**: At 5/6 TTL of the currently active X.509 authority, the new authority becomes the active signer. The old root certificate remains in the trust bundle until it expires.

This creates an overlapping grace period where both old and new roots are trusted. Workloads that obtained SVIDs signed by the old authority can still validate against the trust bundle; new SVIDs are signed by the new authority.

For Overdrive's multi-region scenario (whitepaper §8): "Operator SPIFFE IDs are global, not per-region. In a multi-region deployment, operator certs are federated across all regional CAs -- either by nesting per-region CAs under a single cluster-scoped operator root, or by distributing the operator trust bundle as observation state."

**Source**: [SPIRE issue #2704](https://github.com/spiffe/spire/issues/2704) - Accessed 2026-06-04
**Confidence**: Medium (derived from issue discussion, not primary docs)
**Verification**: [SPIRE issue #185](https://github.com/spiffe/spire/issues/185), [SPIRE issue #928](https://github.com/spiffe/spire/issues/928)
**Analysis**: Overdrive can implement the same two-phase model as a durable workflow (`rotate_root_ca` per Finding 12). The workflow's journal tracks the current rotation phase; the trust bundle in ObservationStore includes both roots during the prepare→activate transition. This maps cleanly onto the existing state-layer hygiene: the root CA material is intent (IntentStore), the trust bundle is observation (ObservationStore), and the rotation progress is the workflow journal (libSQL per §18).

### Finding 10: Certificate Serial Number Generation -- 64-bit CSPRNG Minimum

**Evidence**: RFC 5280 section 4.1.2.2 requires certificate serial numbers to be unique positive integers, at most 20 bytes. The CA/Browser Forum Baseline Requirements (section 7.1, effective September 2016) mandate "at least 64 bits of output from a CSPRNG" for publicly-trusted certificates. While Overdrive's built-in CA issues only internally-trusted certificates, adopting the 64-bit CSPRNG requirement is a defensible floor.

rcgen's `CertificateParams` has `serial_number: Option<SerialNumber>`. When `None`, rcgen generates a serial automatically. For Overdrive's DST compatibility, serials should be generated via the `Entropy` port trait (`Entropy::fill(&mut [u8; 20])`) so that under DST the sequence is deterministic (seeded RNG) while in production the sequence is cryptographically strong (`OsEntropy`). This is the same pattern used for `AllocationId` generation.

**Source**: [SSL.com FAQ on serial number entropy](https://www.ssl.com/faqs/faq-what-is-the-serial-number-entropy-issue-im-hearing-about/) - Accessed 2026-06-04
**Confidence**: High
**Verification**: [Mozilla policy issue #13](https://github.com/mozilla/pkipolicy/issues/13), [RFC 5280 via datatracker.ietf.org](https://datatracker.ietf.org/doc/html/rfc5280)
**Analysis**: Using `Entropy::fill` for serial number generation aligns with the project's nondeterminism-injection discipline. The `Entropy` trait is already in `overdrive-core/src/traits/entropy.rs` with `fn fill(&self, buf: &mut [u8])`. Under DST, `SeededEntropy` produces deterministic serials; in production, `OsEntropy` produces CSPRNG output. The same API feeds both paths.

### Finding 11: Key Generation and the Entropy Trait -- Structural Limitation

**Evidence**: rcgen's `KeyPair::generate()` calls directly into the crypto backend (aws-lc-rs or ring) for key generation. It does NOT accept an external entropy source. The `KeyPair` API is:
- `KeyPair::generate()` -- generates P-256 key using the backend's internal CSPRNG
- `KeyPair::generate_for(alg)` -- generates for a specific algorithm, same internal CSPRNG
- `KeyPair::from_der()` / `KeyPair::from_pem()` -- loads a pre-existing key

This means key generation is NOT injectable via the `Entropy` port trait. Under DST, key generation produces non-deterministic keys (aws-lc-rs uses OS randomness). However, this is acceptable because:
1. The key material itself is never observed by the workflow's convergence-detection layer -- only the certificate (which wraps the public key) and the workflow steps (which reference the SPIFFE ID) cross the boundary.
2. DST tests of the `cert_rotation` workflow can use pre-generated fixture keys loaded via `KeyPair::from_pem()`.
3. Certificate serial numbers (which ARE reconciler-visible via observation rows) CAN be injected via `Entropy`.

The structural solution: certificate serial numbers flow through `Entropy`; key generation uses the crypto backend's own CSPRNG (which is the correct security posture for production). DST fixture keys are pre-generated and loaded from PEM.

**Source**: [rcgen KeyPair docs.rs](https://docs.rs/rcgen/latest/rcgen/struct.KeyPair.html) - Accessed 2026-06-04
**Confidence**: High
**Verification**: [rcgen source on GitHub](https://github.com/rustls/rcgen), [Entropy trait in codebase]
**Analysis**: This is a knowledge gap turned into a finding. The initial concern was "how does key generation compose with DST?" The answer is: it doesn't need to. Key generation is a host-adapter concern (like filesystem I/O); the reconciler operates on the certificate identity (SPIFFE ID) and lifecycle state (issued_at, expires_at, rotation_needed), not on the key bytes. The DST harness tests the reconciler's convergence logic; the crypto correctness is tested at the unit level with fixture keys.

### Finding 12: Certificate Rotation as a Workflow — Whitepaper-Aligned Architecture

**Evidence**: The whitepaper (§18 "Primitive Composition") explicitly lists certificate rotation under **Workflows**, not Reconcilers:

> **Workflows** (orchestrate, durable):
> - Certificate rotation (DNS propagation → validation → trust-anchor swap → retirement)

The DST harness (§21) tests it as a workflow with replay-equivalence:

```rust
assert_replay_equivalent!("cert_rotation workflow replays deterministically",
    WorkflowSpec::cert_rotation(domain));
```

This is the correct primitive per the reconciler-vs-workflow decision table in `development.md` § "Workflow contract": certificate rotation is a **bounded multi-step sequence with a natural terminal `Ok(result)`** — generate key, sign cert, distribute to workload, retire old cert. It terminates. "Keep N replicas running" is a reconciler; "roll the certificate through 4 steps" is a workflow.

**Proposed `cert_rotation` workflow shape:**

```
Workflow: cert_rotation(alloc_id, spiffe_id)
  Steps (each is a ctx.call / ctx.sleep await point, journaled):
    1. ctx.call("ca", generate_keypair())           -> keypair
    2. ctx.call("ca", sign_svid(spiffe_id, pubkey)) -> signed_cert
    3. ctx.call("node", install_svid(alloc_id, cert, privkey))
    4. ctx.call("node", update_trust_bundle_if_needed())
    5. Ok(IssuedSvid { serial, not_after })

Workflow: rotate_intermediate_ca(node_id)
  Steps:
    1. ctx.call("ca", generate_intermediate_keypair()) -> keypair
    2. ctx.call("root_ca", sign_intermediate(pubkey))  -> signed_intermediate
    3. ctx.call("node", install_intermediate(cert, key))
    4. ctx.call("observation", publish_trust_bundle_update())
    5. Ok(RotatedIntermediate { serial, not_after })

Workflow: rotate_root_ca(trust_domain)
  Steps (SPIRE two-phase model):
    1. ctx.call("ca", generate_new_root())             -> new_root
    2. ctx.call("trust", prepare_dual_bundle(old, new)) -- add new root to bundle
    3. ctx.sleep(propagation_window)                    -- wait for gossip
    4. ctx.call("trust", activate_new_root(new))        -- new root signs new intermediates
    5. ctx.sleep(old_cert_drain_window)                 -- wait for old SVIDs to expire
    6. ctx.call("trust", retire_old_root(old))          -- remove old root from bundle
    7. Ok(RotatedRoot { new_serial })
```

The **triggering** mechanism is a reconciler or a timer: a `WorkloadLifecycle` reconciler that detects "allocation has no SVID" emits `Action::StartWorkflow(cert_rotation(...))`. A node-level timer (or a lightweight cert-expiry reconciler) detects "SVID expires within 50% of TTL" and emits `Action::StartWorkflow(cert_rotation(...))` for renewal. The workflow executes the bounded sequence; the reconciler detects the need.

All non-determinism flows through `ctx` per `development.md` § "Workflow contract": no `Instant::now()`, no `reqwest::get()`, no direct crypto calls. The `ctx.call("ca", ...)` surface invokes the CA host adapter (which holds the intermediate CA key in memory and calls rcgen). Journal replay is bit-identical — `assert_replay_equivalent!` is the structural defense.

**Source**: [Whitepaper §18 — Primitive Composition], [Whitepaper §21 — DST], [development.md § Workflow contract]
**Confidence**: High (directly sourced from the whitepaper's explicit classification)
**Verification**: Whitepaper line 2112 (`Certificate rotation (DNS propagation → validation → trust-anchor swap → retirement)`), line 2401 (`assert_replay_equivalent!("cert_rotation workflow replays deterministically", WorkflowSpec::cert_rotation(domain))`)
**Analysis**: The earlier version of this finding (pre-correction) proposed a `CertRotationReconciler`. This was incorrect — the whitepaper explicitly classifies cert rotation as a workflow. The reconciler-vs-workflow distinction is load-bearing: a reconciler that "re-discovers" the rotation need every tick and re-emits the same actions is the wrong shape for a bounded sequence. The workflow journals each step, handles retries via `ctx` replay, and terminates. The reconciler's role is limited to *detecting* the need (missing SVID, approaching expiry) and *triggering* the workflow via `Action::StartWorkflow`.

### Finding 13: Built-in CA vs SPIRE -- Structural Comparison

**Evidence**: Comparing Overdrive's built-in CA with SPIRE across key dimensions:

| Dimension | SPIRE | Overdrive Built-in CA |
|---|---|---|
| **Deployment** | Separate daemon (server + agent per node) | Single binary, in-process |
| **Communication** | gRPC over mTLS between server and agent | In-process (single-node) or Raft-replicated (HA) |
| **Node attestation** | Pluggable (AWS IID, GCE, join token, k8s PSAT, ...) | Bootstrap ceremony (join token or mTLS bootstrap, Talos model) |
| **Workload attestation** | Pluggable (k8s pod, Docker, Unix PID, ...) | Implicit -- the control plane created the allocation, so the identity is known |
| **CA storage** | SQL datastore (SQLite/MySQL/PostgreSQL) | IntentStore (redb, Raft-replicated in HA) |
| **Key generation** | On-agent (private keys never leave node) | On-node (same: node agent's intermediate CA signs locally) |
| **Trust bundle distribution** | Workload API (gRPC over Unix socket) | ObservationStore (Corrosion gossip, local SQLite read) |
| **Rotation driver** | Background polling loop in SPIRE agent | Durable workflow (`cert_rotation`) triggered by reconciler detection |
| **Upstream CA integration** | Pluggable `UpstreamAuthority` | Not planned for Phase 5; future phase possibility |
| **Federation** | SPIFFE bundle endpoint API (RFC 7517 JWK) | Per-region CAs under global operator root (whitepaper section 8) |
| **Operational complexity** | High (server HA, agent deployment, attestation config, registration entries) | Low (same binary, no additional config beyond bootstrap) |
| **CNCF maturity** | Graduated project | N/A (proprietary) |

**Advantages of built-in CA:**
1. Zero operational overhead -- no daemon to deploy, monitor, or upgrade
2. No network hop for SVID issuance (in-process on single-node, Raft in HA)
3. Workflow-driven rotation is a bounded durable sequence with journal replay -- no polling daemon
4. Trust bundle distribution via ObservationStore is already the pattern for all observation data
5. Workload attestation is trivial -- the control plane created the allocation

**Risks of built-in CA:**
1. No pluggable attestation -- node bootstrap is a fixed ceremony
2. No upstream CA integration -- cannot delegate root to an external CA (Vault, AWS ACM PCA) without custom work
3. Limited blast radius isolation -- a control plane compromise exposes the root CA key (mitigated by IntentStore encryption)
4. No CNCF ecosystem compatibility -- workloads expecting SPIFFE Workload API (Unix socket gRPC) will not find it

**Source**: [SPIRE Concepts](https://spiffe.io/docs/latest/spire-about/spire-concepts/) - Accessed 2026-06-04
**Confidence**: High
**Verification**: [Whitepaper section 8], [ADR-0010], [SPIFFE/SPIRE Security Self-Assessment](https://tag-security.cncf.io/community/assessments/projects/spiffe-spire/self-assessment/)
**Analysis**: The built-in CA is the right choice for Overdrive's design principles (single binary, no external dependencies). The SPIFFE Workload API compatibility gap (risk 4) is relevant only if Overdrive workloads need to interact with SPIFFE-aware sidecars (Envoy, etc.). For Phase 5, the gap is acceptable. A future `overdrive-spiffe-workload-api` adapter that exposes the Unix socket gRPC surface could close it without changing the CA architecture.

### Finding 14: Multi-Region CA Architecture

**Evidence**: The whitepaper (section 8) specifies: "Operator SPIFFE IDs are global, not per-region. In a multi-region deployment, operator certs are federated across all regional CAs -- either by nesting per-region CAs under a single cluster-scoped operator root, or by distributing the operator trust bundle as observation state."

The recommended architecture for multi-region:

```
Global Operator Root CA (offline or Raft-replicated across all regions)
    |
    +-- Region A Root CA (IntentStore, Raft-replicated within region)
    |       +-- Node Intermediate CA (per node in region A)
    |               +-- Workload SVID (per allocation)
    |
    +-- Region B Root CA (IntentStore, Raft-replicated within region)
            +-- Node Intermediate CA (per node in region B)
                    +-- Workload SVID (per allocation)
```

Cross-region trust: each region's root CA certificate is added to the global trust bundle, distributed via ObservationStore (Corrosion gossip). A workload in region A that connects to a workload in region B validates the peer's SVID against the region B root CA certificate in the trust bundle. The trust bundle is observation, not intent -- it tolerates seconds of staleness and does not need cross-region linearizability.

Operator SVIDs are issued by the Global Operator Root CA (or by any region's root CA with the operator URI namespace in scope). Cross-region operator access works because the trust bundle includes all regional roots.

**Source**: [Whitepaper section 8] - Accessed 2026-06-04
**Confidence**: Medium (architecture recommendation based on whitepaper constraints; not externally validated)
**Verification**: [SPIRE federation model](https://spiffe.io/docs/latest/deploying/configuring/), [Linkerd multi-cluster trust](https://linkerd.io/2-edge/tasks/automatically-rotating-control-plane-tls-credentials/)
**Analysis**: The per-region root CA model avoids a single global root key that all regions must share, which would create a cross-region Raft dependency for CA operations. Each region is self-sufficient for SVID issuance; cross-region trust is additive (add the other region's root to the bundle). This is the same federation model SPIRE uses for cross-trust-domain communication, adapted to Overdrive's regional topology.

### Finding 15: Certificate Audit Trail -- Internal CA Logging

**Evidence**: For internal enterprise CAs (not publicly trusted), Certificate Transparency (CT) logs are neither required nor appropriate -- logging to public CT logs would expose internal hostnames and infrastructure details. Instead, internal CAs should produce their own audit trail.

For Overdrive, every certificate issuance is a workflow step (`ctx.call("ca", sign_svid(...))`), which means it is:
1. **Journaled in the workflow's await-point log** (the `cert_rotation` workflow's journal records each step's inputs and result in libSQL per §18)
2. **Observable via the workflow execution** (the CA host adapter that executes the signing can emit a structured telemetry event)
3. **Queryable via ObservationStore** (the issued SVID's metadata -- serial, SPIFFE ID, not_before, not_after, issuer -- can be written as an observation row)

The recommended audit shape:

```sql
CREATE TABLE issued_certificates (
    serial        BLOB PRIMARY KEY,
    spiffe_id     TEXT NOT NULL,
    issuer_serial BLOB NOT NULL,     -- intermediate CA that signed it
    not_before    INTEGER NOT NULL,   -- Unix timestamp
    not_after     INTEGER NOT NULL,   -- Unix timestamp
    node_id       TEXT NOT NULL,      -- which node issued it
    issued_at     INTEGER NOT NULL    -- logical timestamp
);
SELECT crsql_as_crr('issued_certificates');
```

This table is observation (gossiped via Corrosion) and serves as the internal CT equivalent. The `revoked_operator_certs` table (whitepaper section 8) is the companion revocation surface.

**Source**: [Certificate Transparency overview](https://certificate.transparency.dev/howctworks/) - Accessed 2026-06-04
**Confidence**: Medium (recommendation; audit table schema is design, not externally sourced)
**Verification**: [RFC 6962](https://datatracker.ietf.org/doc/html/rfc6962), [SSL.com CT article](https://www.ssl.com/article/certificate-transparency/)
**Analysis**: Public CT logs are out of scope -- Overdrive's CA is internal. The `issued_certificates` observation table provides the same audit trail for internal operations: "who issued what, when, signed by whom." The workflow's journal is the SSOT for the issuance inputs and step completion; the observation table is the gossip-distributed audit surface readable by any node.

---

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| rcgen CertificateParams docs.rs | docs.rs | High (1.0) | technical | 2026-06-04 | Y |
| rcgen SanType docs.rs | docs.rs | High (1.0) | technical | 2026-06-04 | Y |
| rcgen KeyPair docs.rs | docs.rs | High (1.0) | technical | 2026-06-04 | Y |
| rcgen GitHub | github.com | High (1.0) | open_source | 2026-06-04 | Y |
| SPIFFE X.509-SVID spec | github.com/spiffe | High (1.0) | official | 2026-06-04 | Y |
| SPIFFE X.509-SVID docs | spiffe.io | High (1.0) | official | 2026-06-04 | Y |
| SPIRE Concepts | spiffe.io | High (1.0) | official | 2026-06-04 | Y |
| SPIRE Configuration | spiffe.io | High (1.0) | official | 2026-06-04 | Y |
| SPIFFE/SPIRE Security Self-Assessment | tag-security.cncf.io | High (1.0) | official | 2026-06-04 | Y |
| Linkerd auto-rotation | linkerd.io | High (1.0) | official | 2026-06-04 | Y |
| Istio security concepts | istio.io | High (1.0) | official | 2026-06-04 | N |
| Smallstep design doc | smallstep.com | Medium-High (0.8) | industry | 2026-06-04 | Y |
| pkcs8 crate docs.rs | docs.rs | High (1.0) | technical | 2026-06-04 | N |
| aws-lc-rs docs.rs | docs.rs | High (1.0) | technical | 2026-06-04 | Y |
| rustls kernel module | docs.rs | High (1.0) | technical | 2026-06-04 | Y |
| ktls crate docs.rs | docs.rs | High (1.0) | technical | 2026-06-04 | Y |
| SSL.com serial number FAQ | ssl.com | Medium-High (0.8) | industry | 2026-06-04 | Y |
| Certificate Transparency | certificate.transparency.dev | High (1.0) | official | 2026-06-04 | Y |

Reputation: High: 15 (83%) | Medium-high: 3 (17%) | Avg: 0.97

---

## Knowledge Gaps

### Gap 1: SPIFFE Workload API Compatibility
**Issue**: Overdrive's built-in CA does not expose the SPIFFE Workload API (gRPC over Unix domain socket, per the SPIFFE Workload Endpoint spec). Workloads that expect to fetch SVIDs via this API (e.g., Envoy with SDS, SPIFFE-aware SDKs) cannot obtain certificates. | **Attempted**: Searched SPIFFE Workload Endpoint spec, Envoy SDS integration docs | **Recommendation**: Evaluate in Phase 7+ whether a compatibility shim (`overdrive-spiffe-workload-api` adapter) is needed for specific workload ecosystem integrations. For Phase 5, internal workloads receive SVIDs via the node agent's direct injection (vsock for microVMs, filesystem mount for exec/wasm), not via the Workload API.

### Gap 2: Hardware-Backed Root CA Key Storage
**Issue**: The research covers software-based encryption of the root CA key (envelope encryption with aws-lc-rs). Hardware-backed storage (HSM, TPM, cloud KMS) is not covered in detail. | **Attempted**: Searched for Rust HSM/KMS integration crates, PKCS#11 Rust bindings | **Recommendation**: Defer to a dedicated research item when hardware key storage enters the roadmap. The envelope encryption approach is designed to be extensible -- the KEK source is pluggable, so replacing passphrase-derived KEK with HSM-sourced KEK is an adapter change, not an architecture change.

### Gap 3: Exact rcgen Feature Flag Configuration for aws-lc-rs
**Issue**: The research confirms rcgen supports aws-lc-rs via a feature flag, but the exact `Cargo.toml` configuration for Overdrive's workspace (which already has `rustls` configured for aws-lc-rs via ADR-0039) was not fully validated against potential feature-flag conflicts. | **Attempted**: Checked rcgen docs.rs, crates.io | **Recommendation**: Target **rcgen 0.14.8** (user-specified). The workspace currently pins `rcgen = "0.13"` (resolved 0.13.2); bumping to 0.14 is a prerequisite, and the existing `mint_ephemeral_ca` (written against the 0.13 builder API) must be migrated to the 0.14.x API in the same change — 0.13→0.14 reshaped the cert-build / `KeyPair` / `CertifiedKey` surface (e.g. `params.self_signed(&key)` / `params.signed_by(&key, &issuer)` replacing the older `Certificate::generate` flow). The X.509-extension APIs this doc relies on (`SanType::URI(Ia5String)`, `IsCa::Ca(BasicConstraints::Constrained(0))`, the `aws_lc_rs` feature) carry across 0.13.2 and 0.14.x. Two first-compile checks for DELIVER: (1) the exact 0.14.8 builder API; (2) feature resolution — confirm rcgen's `aws_lc_rs` feature composes with the workspace `rustls = { version = "0.23", features = ["aws-lc-rs"], default-features = false }` (ADR-0039) without a crypto-backend / fips conflict. ADR-0063 records this as a DELIVER first-compile gate.

### Gap 4: Node Intermediate CA TTL Best Practice
**Issue**: The research found TTL recommendations for workload SVIDs (1 hour, well-established) and root CAs (10 years per Talos, or configurable per SPIRE's `ca_ttl` default of 24 hours for upstream-authority-signed intermediates). The ideal TTL for the per-node intermediate CA in Overdrive's topology is not directly addressed by any source. | **Attempted**: Searched SPIRE, Istio, Linkerd, Smallstep docs for intermediate CA TTL recommendations | **Recommendation**: 24-hour intermediate CA TTL (matching SPIRE's default `ca_ttl`) with renewal at 50% (12 hours). This bounds the blast radius of a compromised node to 24 hours of issuance while avoiding excessive root CA signing load. The intermediate is re-signed by the root CA at node boot and every 12 hours thereafter.

---

## Conflicting Information

### Conflict 1: Renewal Threshold -- 50% vs 67% of TTL
**Position A**: SPIRE renews at 50% of TTL (default, configurable). SPIRE issue #1754 discusses making this configurable. -- Source: [SPIRE Concepts](https://spiffe.io/docs/latest/spire-about/spire-concepts/), Reputation: High
**Position B**: Smallstep step-ca renews at 2/3 (67%) of certificate lifetime. -- Source: [Smallstep design doc](https://smallstep.com/docs/design-document/), Reputation: Medium-High
**Assessment**: Both are defensible. The 50% threshold provides a larger safety margin (50% of TTL to retry); the 67% threshold reduces unnecessary renewals. For Overdrive's 1-hour SVIDs, the difference is 30 minutes (50%) vs 40 minutes (67%) before expiry. SPIRE's 50% is more conservative and matches the larger ecosystem (SPIRE is the CNCF reference implementation). **Recommendation**: Use 50% as the default, with a reconciler-level policy constant that can be adjusted without schema change (per "persist inputs, not derived state").

---

## Recommendations for Further Research

1. **SPIFFE Workload API adapter** -- If Overdrive needs to support Envoy SDS or SPIFFE-aware SDK workloads, research the feasibility of a Unix-socket gRPC adapter that translates between Overdrive's internal SVID distribution and the SPIFFE Workload Endpoint spec.

2. **HSM/KMS integration for root CA key** -- When hardware-backed key storage enters the roadmap, research PKCS#11 Rust bindings (e.g., `cryptoki` crate), AWS KMS SDK for Rust, and the integration surface with rcgen's `KeyPair::from_der()`.

3. **Certificate revocation beyond TTL expiry** -- The whitepaper specifies gossip-propagated revocation for operators; research whether workload SVIDs need a parallel revocation surface or whether 1-hour TTL is sufficient to rely on expiry alone (the current whitepaper position).

4. **Upstream CA delegation** -- If enterprise customers require issuing Overdrive's root CA from their own enterprise PKI, research the integration pattern (CSR submission, signed root reception, trust bundle wiring).

---

## Full Citations

[1] rcgen Contributors. "rcgen - CertificateParams". docs.rs. 2026. https://docs.rs/rcgen/latest/rcgen/struct.CertificateParams.html. Accessed 2026-06-04.
[2] rcgen Contributors. "rcgen - SanType". docs.rs. 2026. https://docs.rs/rcgen/latest/rcgen/enum.SanType.html. Accessed 2026-06-04.
[3] rcgen Contributors. "rcgen - KeyPair". docs.rs. 2026. https://docs.rs/rcgen/latest/rcgen/struct.KeyPair.html. Accessed 2026-06-04.
[4] rustls Contributors. "rcgen - Generate X.509 certificates". GitHub. 2026. https://github.com/rustls/rcgen. Accessed 2026-06-04.
[5] SPIFFE Project. "X509-SVID Specification". GitHub. 2026. https://github.com/spiffe/spiffe/blob/main/standards/X509-SVID.md. Accessed 2026-06-04.
[6] SPIFFE Project. "X509-SVID". spiffe.io. 2026. https://spiffe.io/docs/latest/spiffe-specs/x509-svid/. Accessed 2026-06-04.
[7] SPIFFE Project. "SPIRE Concepts". spiffe.io. 2026. https://spiffe.io/docs/latest/spire-about/spire-concepts/. Accessed 2026-06-04.
[8] SPIFFE Project. "Configuring SPIRE". spiffe.io. 2026. https://spiffe.io/docs/latest/deploying/configuring/. Accessed 2026-06-04.
[9] CNCF TAG Security. "SPIFFE/SPIRE Security Self-Assessment". tag-security.cncf.io. 2026. https://tag-security.cncf.io/community/assessments/projects/spiffe-spire/self-assessment/. Accessed 2026-06-04.
[10] Linkerd Project. "Automatically Rotating Control Plane TLS Credentials". linkerd.io. 2026. https://linkerd.io/2-edge/tasks/automatically-rotating-control-plane-tls-credentials/. Accessed 2026-06-04.
[11] Istio Project. "Policies and Security". istio.io. 2026. https://istio.io/v1.3/docs/concepts/security/. Accessed 2026-06-04.
[12] Smallstep Labs. "step-ca Architecture & Design Document". smallstep.com. 2026. https://smallstep.com/docs/design-document/. Accessed 2026-06-04.
[13] RustCrypto. "pkcs8 crate". docs.rs. 2026. https://docs.rs/pkcs8. Accessed 2026-06-04.
[14] AWS. "aws-lc-rs". docs.rs. 2026. https://docs.rs/aws-lc-rs/latest/aws_lc_rs/. Accessed 2026-06-04.
[15] rustls Contributors. "rustls::kernel module". docs.rs. 2026. https://docs.rs/rustls/latest/rustls/kernel/index.html. Accessed 2026-06-04.
[16] ktls Contributors. "ktls crate". docs.rs. 2026. https://docs.rs/ktls. Accessed 2026-06-04.
[17] SSL.com. "FAQ: Serial Number Entropy Issue". ssl.com. 2026. https://www.ssl.com/faqs/faq-what-is-the-serial-number-entropy-issue-im-hearing-about/. Accessed 2026-06-04.
[18] Certificate Transparency Project. "How CT Works". certificate.transparency.dev. 2026. https://certificate.transparency.dev/howctworks/. Accessed 2026-06-04.

---

## Research Metadata

Duration: ~50 turns | Examined: 25+ | Cited: 18 | Cross-refs: 14 | Confidence: High 73%, Medium 27%, Low 0% | Output: docs/research/security/built-in-ca-rcgen-rustls-comprehensive-research.md
