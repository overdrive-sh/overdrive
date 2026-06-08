//! Integration — CA boot wiring, refuse-to-start, persistence, audit row
//! (DISTILL RED scaffolds, built-in-ca / GH #28).
//!
//! Layer 3 (gated `integration-tests`, runs via Lima — exercises the real
//! `IntentStore` (`LocalStore` over redb), the `ObservationStore`, and the
//! Earned-Trust composition-root probes). These prove the CONSUMER wiring:
//! how the control-plane boot path generates-or-loads the root, refuses to
//! start on probe failure, persists across restart, and writes the audit
//! row on issuance.
//!
//! Per Mandate 11 these layer-3 tests are EXAMPLE-ONLY; each failure mode
//! from the SSOT journey `error_paths` and ADR-0063 § "Earned Trust" gets
//! exactly one named example.
//!
//! Earned-Trust probe contract (ADR-0063 D8): wire -> probe -> use. On boot
//! the CA adapter probes (a) KEK present in keyring, (b) persisted envelope
//! decrypts, (c) systemd-creds credential present — BEFORE the control
//! plane accepts traffic; a probe failure refuses startup with
//! `health.startup.refused`, never a silent fallback / silent re-mint.
//!
//! Scenarios trace to: US-CA-02 (persistence reuse, refuse-to-start),
//! US-CA-03 (intermediate signing failure), US-CA-05 (audit row, no silent
//! issuance), SSOT journey `error_paths` steps 1-3.
//! Tags: `@real-io` `@adapter-integration` `@S-NN` · `+ @error` (failure paths).
//!
//! RED scaffold convention: self-contained `panic!` under
//! `#[should_panic(expected = "RED scaffold")]`; no import of unbuilt CA
//! wiring. DELIVER replaces with real boot/issuance assertions.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use overdrive_control_plane::ca_boot::{self, CaBootError};
use overdrive_control_plane::ca_issuance::{self, CaIssuanceError};
use overdrive_core::ca::kek::KEK_LEN;
use overdrive_core::ca::root_key_envelope::RootCaKeyRecord;
use overdrive_core::ca::{SKEW_TOLERANCE, WORKLOAD_SVID_TTL};
use overdrive_core::traits::ca::{
    Ca, CaError, IntermediateHandle, RootCaHandle, SvidMaterial, SvidRequest, TrustBundle,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::intent_store::{IntentStore, IntentStoreError};
use overdrive_core::traits::observation_store::{ObservationStore, ObservationStoreError};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{NodeId, SpiffeId};
use overdrive_host::OsEntropy;
use overdrive_host::ca::{RcgenCa, RootKeyAeadCodec, SystemdCredsKeyring};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalObservationStore;
use serial_test::serial;
use tempfile::TempDir;

/// Trust-domain subject the test root is minted for.
fn trust_domain_subject() -> SpiffeId {
    SpiffeId::new("spiffe://overdrive.local/overdrive/ca").expect("trust-domain SpiffeId parses")
}

/// A host `RcgenCa` over real OS entropy and the trust-domain subject.
fn host_ca() -> RcgenCa {
    RcgenCa::new(Arc::new(OsEntropy), trust_domain_subject())
}

/// Stage a 32-byte systemd-creds credential file named for the boot KEK id
/// under `dir`, so `SystemdCredsKeyring::with_credentials_dir(dir)` resolves
/// the KEK with no environment dependency.
fn stage_kek_credential(dir: &TempDir, byte: u8) {
    let kek_id = ca_boot::root_kek_id();
    std::fs::write(dir.path().join(kek_id.as_str()), [byte; KEK_LEN])
        .expect("write systemd-creds KEK credential");
}

/// The on-disk redb path the `intent_store` helper opens. Kept in ONE place
/// alongside `intent_store` so the `intent.redb` filename cannot drift between
/// the store opener and the `boot_ca` `redb_path` argument.
fn intent_redb_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("intent.redb")
}

/// Open a `LocalIntentStore` at `intent.redb` under `dir` as an
/// `Arc<dyn IntentStore>`.
fn intent_store(dir: &TempDir) -> Arc<dyn IntentStore> {
    Arc::new(
        overdrive_store_local::LocalIntentStore::open(intent_redb_path(dir))
            .expect("LocalIntentStore::open"),
    )
}

// ---------------------------------------------------------------------------
// US-CA-02 / S-02-05 — Persistent root reused across restart (happy path)
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-02` — persistence: first boot
/// generates + envelope-encrypts + persists the root to the `IntentStore`;
/// second boot (same KEK) decrypts and REUSES the same root identity (same
/// public key / same cert). This is what supersedes ADR-0010's ephemerality.
#[tokio::test]
async fn root_ca_is_reused_across_control_plane_restart() {
    // GIVEN a persisted intent store, a staged KEK credential, and the codec.
    let store_dir = TempDir::new().expect("intent-store tempdir");
    let creds_dir = TempDir::new().expect("creds tempdir");
    stage_kek_credential(&creds_dir, 0x11);
    let kek = SystemdCredsKeyring::with_credentials_dir(creds_dir.path());
    let codec = RootKeyAeadCodec::new();
    let kek_id = ca_boot::root_kek_id();

    // Each boot opens its OWN handle on the SAME redb file — a genuine restart
    // (the first store is dropped before the second opens), so reuse is proven
    // through on-disk persistence, not in-process caching.
    let redb_path = intent_redb_path(&store_dir);
    let first = {
        let intent = intent_store(&store_dir);
        ca_boot::boot_ca(&host_ca(), &kek, &kek_id, &codec, &intent, &redb_path)
            .await
            .expect("first boot generates + persists the root")
    };

    let second = {
        let intent = intent_store(&store_dir);
        ca_boot::boot_ca(&host_ca(), &kek, &kek_id, &codec, &intent, &redb_path)
            .await
            .expect("second boot decrypts + reuses the persisted root")
    };

    // THEN the second boot reuses the SAME root identity: byte-identical public
    // cert (PEM + DER) and serial. A fresh `ca.root()` on the second boot would
    // mint a new keypair → different cert; equality proves reuse-from-disk.
    assert_eq!(
        first.cert_pem(),
        second.cert_pem(),
        "second boot must present the byte-identical root cert PEM (same public key)"
    );
    assert_eq!(
        first.cert_der(),
        second.cert_der(),
        "second boot must present the byte-identical root cert DER"
    );
    assert_eq!(first.serial(), second.serial(), "second boot must present the same root serial");
}

// ---------------------------------------------------------------------------
// Regression — issued chain anchors on the PERSISTED root after restart
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-02` `@error` — chain-to-persisted-root
/// regression guard (built-in-ca / GH #28).
///
/// The bug: `RcgenCa` holds its root signing key ONLY in an in-memory
/// `OnceLock`. After a control-plane restart a FRESH `RcgenCa` is constructed
/// with an empty cache. `load_persistent_root` decrypts the persisted root key
/// and rebuilds a `RootCaHandle`, but never feeds it back into the adapter — so
/// the adapter's first signing call lazily mints a BRAND-NEW ephemeral root.
/// Anything signed under it (the node intermediate, every SVID) does NOT chain
/// to the persisted root that relying parties anchor on → broken chain after
/// every restart.
///
/// This pins the contract end-to-end: after a genuine restart (the first store
/// handle is dropped before the second opens), a workload SVID issued through
/// the freshly-constructed adapter MUST chain to the FIRST boot's persisted
/// root. The discriminating proof is `openssl verify` — the ephemeral and
/// persisted roots share the same subject DN (trust domain), so only crypto
/// verification (not byte-equality) detects the signature mismatch. Without the
/// `adopt_persisted_root` fix the restart intermediate is signed by an ephemeral
/// root and `openssl verify` against the persisted root FAILS (non-zero); with
/// the fix it PASSES (exit 0).
#[tokio::test]
async fn issued_chain_anchors_on_persisted_root_after_restart() {
    // GIVEN a first boot that generates + persists the root, then bootstraps the
    // node intermediate under it. Capture the FIRST boot's persisted root cert
    // PEM — the relying-party anchor every later restart must chain to.
    let store_dir = TempDir::new().expect("intent-store tempdir");
    let creds_dir = TempDir::new().expect("creds tempdir");
    stage_kek_credential(&creds_dir, 0x11);
    let kek = SystemdCredsKeyring::with_credentials_dir(creds_dir.path());
    let codec = RootKeyAeadCodec::new();
    let kek_id = ca_boot::root_kek_id();
    let node = issuing_node();

    let redb_path = intent_redb_path(&store_dir);
    let first_root_pem = {
        let intent = intent_store(&store_dir);
        let root = ca_boot::boot_ca(&host_ca(), &kek, &kek_id, &codec, &intent, &redb_path)
            .await
            .expect("first boot generates + persists the root");
        root.cert_pem().as_pem().to_owned()
    };

    // WHEN a fresh process restarts: a NEW `RcgenCa` (empty OnceLock), boot_ca
    // (loads the persisted root AND adopts it into the adapter), THEN bootstrap
    // the node intermediate, THEN issue a workload SVID. ORDER MATTERS: boot_ca
    // must adopt the persisted root before any signing call, so the intermediate
    // is signed under the persisted root, not a freshly-minted ephemeral one.
    let (restart_inter_pem, restart_svid_pem) = {
        let restart_ca = host_ca();
        let intent = intent_store(&store_dir);
        ca_boot::boot_ca(&restart_ca, &kek, &kek_id, &codec, &intent, &redb_path)
            .await
            .expect("restart boot loads + adopts the persisted root");
        let inter = ca_boot::bootstrap_node_intermediate(
            &restart_ca,
            &node,
            &intent,
            &kek,
            &kek_id,
            &codec,
            &redb_path,
        )
        .await
        .expect("restart node bootstrap signs the intermediate under the adopted root");
        let svid =
            restart_ca.issue_svid(&workload_request()).expect("restart issues a workload SVID");
        (inter.cert_pem().as_pem().to_owned(), svid.cert_pem().as_pem().to_owned())
    };

    // THEN `openssl verify -CAfile <FIRST-boot persisted root.pem> -untrusted
    // <restart intermediate.pem> <restart svid.pem>` exits 0 — the full restart
    // chain anchors on the PERSISTED root. Under the bug the restart intermediate
    // is signed by an ephemeral root and this verification FAILS.
    let dir = TempDir::new().expect("pem tempdir");
    let root_pem_path = dir.path().join("root.pem");
    let inter_pem_path = dir.path().join("intermediate.pem");
    let svid_pem_path = dir.path().join("svid.pem");
    std::fs::write(&root_pem_path, first_root_pem.as_bytes()).expect("write root.pem");
    std::fs::write(&inter_pem_path, restart_inter_pem.as_bytes()).expect("write intermediate.pem");
    std::fs::write(&svid_pem_path, restart_svid_pem.as_bytes()).expect("write svid.pem");

    let output = std::process::Command::new("openssl")
        .arg("verify")
        .arg("-CAfile")
        .arg(&root_pem_path)
        .arg("-untrusted")
        .arg(&inter_pem_path)
        .arg(&svid_pem_path)
        .output()
        .expect("invoke openssl verify");
    assert!(
        output.status.success(),
        "the restart-issued chain must verify against the FIRST boot's persisted root \
         (chain-to-persisted-root after restart): stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

// ---------------------------------------------------------------------------
// Regression — pre-restart SVID stays verifiable against the POST-restart
// trust bundle (node intermediate survives restart; built-in-ca / GH #28)
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-02` `@error` — node-intermediate
/// continuity-across-restart regression guard (built-in-ca / GH #28).
///
/// The bug: the node intermediate was EPHEMERAL per process. Only its public
/// cert material was persisted (`ca/node/intermediate-material/v1`); its
/// PRIVATE key was never sealed, and `bootstrap_node_intermediate`
/// unconditionally called `issue_intermediate`, which on a fresh `RcgenCa`
/// (empty `intermediate_material` `OnceLock`) minted a BRAND-NEW intermediate
/// (new key + new cert) on every restart. The post-restart `trust_bundle()`
/// then carried that fresh intermediate, so every SVID signed under the
/// PREVIOUS boot's intermediate — still inside its 1-hour validity window —
/// failed to chain-verify against the refreshed bundle. This is the exact
/// chain-break class `adopt_persisted_root` (ADR-0063 D3) closed for the
/// root, left open for the intermediate.
///
/// This pins the contract: an SVID minted on boot 1 MUST still verify against
/// the trust bundle the control plane presents AFTER a genuine restart (the
/// first store handle is dropped before the second opens). The discriminating
/// proof is `openssl verify` against the POST-restart bundle: under the bug
/// the boot-1 SVID was signed by the lost ephemeral intermediate, so it does
/// NOT chain to the restart bundle's freshly-minted intermediate and
/// verification FAILS (non-zero). With the fix — the intermediate key is
/// sealed + persisted on boot 1, and the restart bootstraps by decrypting +
/// adopting it before any issuance — the restart bundle carries the SAME
/// intermediate the boot-1 SVID was signed under and verification PASSES.
#[tokio::test]
async fn pre_restart_svid_verifies_against_post_restart_trust_bundle() {
    // GIVEN a first boot that generates + persists the root, bootstraps the
    // node intermediate under it (sealing + persisting the intermediate key),
    // and mints a workload SVID. Capture the boot-1 SVID PEM — the leaf a
    // relying party holds for its full ~1h validity window across a restart.
    let store_dir = TempDir::new().expect("intent-store tempdir");
    let creds_dir = TempDir::new().expect("creds tempdir");
    stage_kek_credential(&creds_dir, 0x11);
    let kek = SystemdCredsKeyring::with_credentials_dir(creds_dir.path());
    let codec = RootKeyAeadCodec::new();
    let kek_id = ca_boot::root_kek_id();
    let node = issuing_node();
    let redb_path = intent_redb_path(&store_dir);

    let boot1_svid_pem = {
        let ca = host_ca();
        let intent = intent_store(&store_dir);
        ca_boot::boot_ca(&ca, &kek, &kek_id, &codec, &intent, &redb_path)
            .await
            .expect("first boot generates + persists the root");
        ca_boot::bootstrap_node_intermediate(
            &ca, &node, &intent, &kek, &kek_id, &codec, &redb_path,
        )
        .await
        .expect("first boot bootstraps + persists the node intermediate");
        let svid = ca.issue_svid(&workload_request()).expect("boot-1 mints a workload SVID");
        svid.cert_pem().as_pem().to_owned()
    };

    // WHEN a fresh process restarts: a NEW `RcgenCa` (empty OnceLocks),
    // boot_ca (adopts the persisted root), bootstrap_node_intermediate (must
    // decrypt + adopt the persisted intermediate, NOT mint a fresh one), then
    // read the trust bundle the restarted control plane now presents.
    let restart_bundle_pem = {
        let restart_ca = host_ca();
        let intent = intent_store(&store_dir);
        ca_boot::boot_ca(&restart_ca, &kek, &kek_id, &codec, &intent, &redb_path)
            .await
            .expect("restart boot loads + adopts the persisted root");
        ca_boot::bootstrap_node_intermediate(
            &restart_ca,
            &node,
            &intent,
            &kek,
            &kek_id,
            &codec,
            &redb_path,
        )
        .await
        .expect("restart boot loads + adopts the persisted node intermediate");
        restart_ca
            .trust_bundle()
            .expect("restart composes its trust bundle")
            .bundle_pem()
            .as_pem()
            .to_owned()
    };

    // THEN `openssl verify -CAfile <POST-restart bundle.pem> <boot-1 svid.pem>`
    // exits 0 — the bundle carries the SAME node intermediate the boot-1 SVID
    // was signed under (root anchor + adopted intermediate), so the still-valid
    // pre-restart leaf chains to the post-restart bundle. Under the bug the
    // restart minted a fresh ephemeral intermediate and this verification FAILS.
    let dir = TempDir::new().expect("pem tempdir");
    let bundle_pem_path = dir.path().join("bundle.pem");
    let svid_pem_path = dir.path().join("svid.pem");
    std::fs::write(&bundle_pem_path, restart_bundle_pem.as_bytes()).expect("write bundle.pem");
    std::fs::write(&svid_pem_path, boot1_svid_pem.as_bytes()).expect("write svid.pem");

    let output = std::process::Command::new("openssl")
        .arg("verify")
        .arg("-CAfile")
        .arg(&bundle_pem_path)
        .arg(&svid_pem_path)
        .output()
        .expect("invoke openssl verify");
    assert!(
        output.status.success(),
        "a boot-1 SVID must verify against the POST-restart trust bundle (the node intermediate \
         must survive restart, not be re-minted ephemerally): stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

// ---------------------------------------------------------------------------
// Regression — durable at-rest format: the sealed root key is PEM, not DER
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-02` — durable-format regression
/// guard (`aead_codec` param/doc said "DER" while the boot path seals PEM).
///
/// The at-rest envelope format is a durable protocol: a maintainer who reads
/// "DER" in the codec/field docs and "corrects" `generate_and_persist_root`
/// to seal `signing_key().as_der()` instead of `as_pem()` would make every
/// PREVIOUSLY-persisted record fail to decrypt on upgrade (the load path
/// parses the decrypted bytes as UTF-8 PEM). This pins the contract at the
/// boot seam: the plaintext recovered from a freshly-persisted envelope MUST
/// be PEM-armored, not DER. Swap the seal to DER and this goes RED — binary
/// DER is not valid UTF-8 and carries no PEM armor.
#[tokio::test]
async fn persisted_root_key_envelope_seals_pem_not_der() {
    // GIVEN a first boot that generates + envelope-encrypts + persists the root.
    let store_dir = TempDir::new().expect("intent-store tempdir");
    let creds_dir = TempDir::new().expect("creds tempdir");
    stage_kek_credential(&creds_dir, 0x11);
    let kek = SystemdCredsKeyring::with_credentials_dir(creds_dir.path());
    let codec = RootKeyAeadCodec::new();
    let kek_id = ca_boot::root_kek_id();

    let redb_path = intent_redb_path(&store_dir);
    let intent = intent_store(&store_dir);
    ca_boot::boot_ca(&host_ca(), &kek, &kek_id, &codec, &intent, &redb_path)
        .await
        .expect("first boot generates + persists the sealed root-key envelope");

    // WHEN the persisted envelope is read back and opened under the same KEK.
    let envelope_bytes = intent
        .get(b"ca/root/key-envelope/v1")
        .await
        .expect("intent store get")
        .expect("the first boot persisted a root-key envelope");
    let record = RootCaKeyRecord::from_store_bytes(
        &envelope_bytes,
        std::path::Path::new("<intent-store>"),
        Some("ca/root/key-envelope/v1"),
    )
    .expect("persisted envelope decodes into a RootCaKeyRecord");
    let recovered = codec.open(&kek, &kek_id, &record).expect("envelope opens under the KEK");

    // THEN the sealed plaintext is the PEM form of the signing key — NOT DER.
    // DER is binary (would fail this UTF-8 decode); PEM is valid UTF-8 + armored.
    let pem = std::str::from_utf8(&recovered)
        .expect("sealed root key must be PEM (valid UTF-8); a DER payload would be binary");
    assert!(
        pem.contains("-----BEGIN") && pem.contains("PRIVATE KEY-----"),
        "the sealed root-key plaintext must be PEM-armored, not DER; got a non-PEM payload"
    );
}

// ---------------------------------------------------------------------------
// US-CA-02 / S-02-06 — Refuse-to-start on decrypt failure (Earned Trust)
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-02` `@error` — SSOT journey
/// `error_paths` step 1: a tampered/undecryptable persisted envelope makes
/// the boot-time Earned-Trust probe FAIL; the control plane REFUSES to
/// start with a typed `CaError` + `health.startup.refused`, and does NOT
/// silently re-mint a new root (which would orphan every issued identity).
#[tokio::test]
async fn boot_refuses_to_start_on_envelope_decrypt_failure_without_remint() {
    // GIVEN a control plane that persisted its root under the correct KEK.
    let store_dir = TempDir::new().expect("intent-store tempdir");
    let creds_dir = TempDir::new().expect("creds tempdir");
    stage_kek_credential(&creds_dir, 0x11);
    let kek = SystemdCredsKeyring::with_credentials_dir(creds_dir.path());
    let codec = RootKeyAeadCodec::new();
    let kek_id = ca_boot::root_kek_id();

    let redb_path = intent_redb_path(&store_dir);
    let first = {
        let intent = intent_store(&store_dir);
        ca_boot::boot_ca(&host_ca(), &kek, &kek_id, &codec, &intent, &redb_path)
            .await
            .expect("first boot persists the root under the correct KEK")
    };

    // WHEN the second boot opens with the WRONG KEK (a different staged
    // credential), the persisted envelope cannot AES-GCM-open under it.
    let wrong_creds = TempDir::new().expect("wrong-creds tempdir");
    stage_kek_credential(&wrong_creds, 0x22);
    let wrong_kek = SystemdCredsKeyring::with_credentials_dir(wrong_creds.path());

    // Scoped so the redb file lock is released before the recovery boot opens
    // its own handle on the same file (redb is single-writer-exclusive).
    let result = {
        let intent = intent_store(&store_dir);
        ca_boot::boot_ca(&host_ca(), &wrong_kek, &kek_id, &codec, &intent, &redb_path).await
    };

    // THEN the boot REFUSES to start with the typed envelope-decrypt error
    // (Earned-Trust probe (b) failed). `health.startup.refused` is emitted by
    // the boot path before this return (asserted structurally by the variant).
    assert!(
        matches!(result, Err(CaBootError::EnvelopeDecrypt { .. })),
        "undecryptable envelope must refuse startup with EnvelopeDecrypt, got {result:?}"
    );

    // AND no NEW root was silently minted: the persisted public cert material
    // is byte-identical to the first boot's (a re-mint would have overwritten
    // it). Re-opening with the CORRECT KEK still recovers the ORIGINAL root.
    let recovered = {
        let intent = intent_store(&store_dir);
        ca_boot::boot_ca(&host_ca(), &kek, &kek_id, &codec, &intent, &redb_path)
            .await
            .expect("the original root is intact and re-openable under the correct KEK")
    };
    assert_eq!(
        first.cert_der(),
        recovered.cert_der(),
        "the refused boot must NOT have re-minted or overwritten the persisted root"
    );
}

/// `@real-io` `@adapter-integration` `@S-02` `@error` — regression guard for
/// the hardcoded-placeholder-path bug (operator-actionability of the
/// corrupt-envelope refusal).
///
/// The bug: `load_persistent_root` hardcoded
/// `std::path::Path::new("<intent-store>")` and threaded that placeholder into
/// `RootCaKeyRecord::from_store_bytes`, which surfaces it in TWO operator-facing
/// places on an undecodable envelope — the `health.startup.refused` tracing
/// event and the `IntentStoreError::Envelope` `Display` remediation hint
/// ("delete <intent-store> and restart"). The placeholder makes both
/// unactionable: an operator cannot tell which redb file to inspect or delete.
///
/// This pins the contract at the boot seam: when the persisted root-key envelope
/// is undecodable, the surfaced `IntentStoreError::Envelope` MUST carry the REAL
/// on-disk redb path (`<store>/intent.redb`), never the placeholder. The KEK
/// probe PASSES (correct KEK), so execution reaches `from_store_bytes` and the
/// failure is the envelope DECODE — not a KEK decrypt.
#[tokio::test]
async fn boot_envelope_decode_failure_surfaces_real_redb_path() {
    // GIVEN a first boot that generates + persists the root under the correct
    // KEK (scoped so the redb write-lock releases before the recovery boot).
    let store_dir = TempDir::new().expect("intent-store tempdir");
    let creds_dir = TempDir::new().expect("creds tempdir");
    stage_kek_credential(&creds_dir, 0x11);
    let kek = SystemdCredsKeyring::with_credentials_dir(creds_dir.path());
    let codec = RootKeyAeadCodec::new();
    let kek_id = ca_boot::root_kek_id();
    let redb_path = intent_redb_path(&store_dir);

    {
        let intent = intent_store(&store_dir);
        ca_boot::boot_ca(&host_ca(), &kek, &kek_id, &codec, &intent, &redb_path)
            .await
            .expect("first boot generates + persists the root");
    }

    // WHEN the persisted root-key envelope is CORRUPTED to non-decodable bytes
    // (scoped so the write-lock releases before the recovery boot opens).
    {
        let intent = intent_store(&store_dir);
        intent
            .put(b"ca/root/key-envelope/v1", b"not-a-valid-rkyv-envelope")
            .await
            .expect("overwrite envelope with garbage");
    }

    // WHEN a second boot runs under the CORRECT KEK — the KEK probe passes, so
    // execution reaches `from_store_bytes` and fails on the envelope DECODE.
    let result = {
        let intent = intent_store(&store_dir);
        ca_boot::boot_ca(&host_ca(), &kek, &kek_id, &codec, &intent, &redb_path).await
    };

    // THEN the boot refuses with the typed envelope-decode error carrying the
    // REAL redb path — the operator-actionable signal the placeholder destroyed.
    let reported = match result {
        Err(CaBootError::Intent(IntentStoreError::Envelope { redb_path, .. })) => redb_path,
        other => panic!(
            "undecodable envelope must refuse startup with Intent(Envelope {{ .. }}), got {other:?}"
        ),
    };
    assert_eq!(
        reported, redb_path,
        "the refusal must report the REAL on-disk redb path so the remediation hint is actionable"
    );
    assert_ne!(
        reported.as_path(),
        std::path::Path::new("<intent-store>"),
        "the hardcoded placeholder path must never reappear (regression guard)"
    );
}

/// `@real-io` `@adapter-integration` `@S-02` `@error` — Earned-Trust KEK
/// probe: an absent/empty keyring KEK (and no dev `OVERDRIVE_CA_KEK`
/// opt-in) refuses startup BEFORE any issuance, rather than panicking
/// mid-issuance or silently generating a throwaway KEK (which would make
/// at-rest encryption meaningless).
#[tokio::test]
#[serial(env)]
async fn boot_refuses_to_start_when_kek_absent_from_keyring() {
    // GIVEN an EMPTY credentials directory (no KEK credential staged) and no
    // dev OVERDRIVE_CA_KEK opt-in in the environment.
    let store_dir = TempDir::new().expect("intent-store tempdir");
    let empty_creds = TempDir::new().expect("empty-creds tempdir");
    // SAFETY: `#[serial(env)]` guarantees exclusive access to the process
    // environment for the duration of this test, so removing the dev-fallback
    // vars cannot race another test.
    unsafe {
        std::env::remove_var("OVERDRIVE_CA_KEK");
        std::env::remove_var("OVERDRIVE_CA_KEK_DEV_OPT_IN");
    }
    let kek = SystemdCredsKeyring::with_credentials_dir(empty_creds.path());
    let codec = RootKeyAeadCodec::new();
    let kek_id = ca_boot::root_kek_id();

    // WHEN the control plane attempts to start.
    let redb_path = intent_redb_path(&store_dir);
    let intent = intent_store(&store_dir);
    let result = ca_boot::boot_ca(&host_ca(), &kek, &kek_id, &codec, &intent, &redb_path).await;

    // THEN it refuses to start with the typed KEK-unavailable error, BEFORE any
    // issuance (no throwaway KEK minted).
    assert!(
        matches!(result, Err(CaBootError::KekUnavailable { .. })),
        "absent KEK (no dev opt-in) must refuse startup with KekUnavailable, got {result:?}"
    );

    // AND nothing was persisted — no root envelope, no throwaway KEK material —
    // because the KEK probe failed before generate-or-persist.
    let persisted = intent.get(b"ca/root/key-envelope/v1").await.expect("intent store get");
    assert!(
        persisted.is_none(),
        "no root key envelope must be persisted when the KEK probe refuses startup"
    );
}

// ---------------------------------------------------------------------------
// US-CA-03 / S-03 — Intermediate signing failure fails loudly
// ---------------------------------------------------------------------------

/// A `Ca` whose `issue_intermediate` always fails with a signing error,
/// modelling "root key unavailable (decrypt failed upstream)" — the exact
/// sad path node bootstrap must fail loudly on. `root` succeeds (the root
/// was already composed at control-plane boot); only the per-node
/// intermediate signing step is unable to reach the root key.
struct RootKeyUnavailableCa;

impl Ca for RootKeyUnavailableCa {
    fn root(&self) -> Result<RootCaHandle, CaError> {
        host_ca().root()
    }

    fn issue_intermediate(&self, _node: &NodeId) -> Result<IntermediateHandle, CaError> {
        // Upstream decrypt failed: the root signing key is unavailable, so the
        // intermediate cannot be signed by the root.
        Err(CaError::signing_failed("root key unavailable (decrypt failed upstream)"))
    }

    fn issue_svid(&self, _req: &SvidRequest) -> Result<SvidMaterial, CaError> {
        unreachable!("issue_svid is not exercised by the node-bootstrap sad path")
    }

    fn trust_bundle(&self) -> Result<TrustBundle, CaError> {
        unreachable!("trust_bundle is not exercised by the node-bootstrap sad path")
    }
}

/// `@real-io` `@adapter-integration` `@S-03` `@error` — SSOT journey
/// `error_paths` step 2: when the root key is unavailable at node bootstrap
/// (decrypt failed upstream), `issue_intermediate` surfaces a typed
/// `CaError`; node bootstrap fails loudly rather than running workloads it
/// cannot issue identities for (no half-provisioned state).
#[tokio::test]
async fn intermediate_signing_failure_fails_node_bootstrap_loudly() {
    // GIVEN a persisted intent store and a `Ca` whose root key is unavailable
    // for intermediate signing (decrypt failed upstream). The KEK is staged so
    // the Earned-Trust KEK probe passes and execution reaches the issuance step
    // — the failure under test is the intermediate SIGNING, not KEK resolution.
    let store_dir = TempDir::new().expect("intent-store tempdir");
    let creds_dir = TempDir::new().expect("creds tempdir");
    stage_kek_credential(&creds_dir, 0x11);
    let kek = SystemdCredsKeyring::with_credentials_dir(creds_dir.path());
    let codec = RootKeyAeadCodec::new();
    let kek_id = ca_boot::root_kek_id();
    let redb_path = intent_redb_path(&store_dir);
    let intent = intent_store(&store_dir);
    let ca = RootKeyUnavailableCa;
    let node = NodeId::new("overdrive-node-0").expect("valid NodeId");

    // WHEN node bootstrap issues the single node intermediate through the
    // node-bootstrap driving port.
    let result = ca_boot::bootstrap_node_intermediate(
        &ca, &node, &intent, &kek, &kek_id, &codec, &redb_path,
    )
    .await;

    // THEN node bootstrap fails loudly with the typed CA error surfaced from
    // `issue_intermediate` (no panic, no silent skip). `health.startup.refused`
    // is emitted by the boot path before this return (structural to the variant).
    assert!(
        matches!(result, Err(CaBootError::Ca { .. })),
        "intermediate signing failure must fail node bootstrap loudly with a typed CaError, got \
         {result:?}"
    );

    // AND no half-provisioned state is left behind: the intermediate was never
    // persisted (reconciler-discipline — no adopt-and-skip of a partial; the
    // node does not run workloads it cannot identify).
    let persisted =
        intent.get(b"ca/node/intermediate-material/v1").await.expect("intent store get");
    assert!(
        persisted.is_none(),
        "no node-intermediate material must be persisted when issuance fails (no half-provisioned \
         state)"
    );
}

// ---------------------------------------------------------------------------
// US-CA-05 / S-05 — Audit row written; no silent issuance; re-issue
// ---------------------------------------------------------------------------

/// A fixed-time [`Clock`] test double — `unix_now` returns a constant the test
/// pins, so the audit row's `issued_at` / validity window are deterministic.
struct FixedClock {
    unix: Duration,
    monotonic: Instant,
}

impl FixedClock {
    fn at_unix_secs(secs: u64) -> Self {
        Self { unix: Duration::from_secs(secs), monotonic: Instant::now() }
    }
}

#[async_trait]
impl Clock for FixedClock {
    fn now(&self) -> Instant {
        self.monotonic
    }

    fn unix_now(&self) -> Duration {
        self.unix
    }

    async fn sleep(&self, _duration: Duration) {
        // Not exercised by the issuance path under test; a no-op keeps the
        // fixed clock fully deterministic.
    }
}

/// Open a real `LocalObservationStore` at `obs.redb` under `dir` — the
/// production `ObservationStore` the issuance seam writes the
/// `issued_certificates` audit row through (`ObservationRow::IssuedCertificate`,
/// ADR-0063 D6). Returned as `Arc<dyn ObservationStore>` so the issuance seam
/// receives the PORT, never an inherent-method concrete binding.
fn audit_store(dir: &TempDir) -> Arc<dyn ObservationStore> {
    Arc::new(
        LocalObservationStore::open(dir.path().join("obs.redb"))
            .expect("LocalObservationStore::open"),
    )
}

/// The node whose intermediate issues workload SVIDs in these tests.
fn issuing_node() -> NodeId {
    NodeId::new("overdrive-node-0").expect("valid NodeId")
}

/// A workload SVID request for the dns-resolver identity, over a validity
/// window straddling the current wall-clock (ADR-0063 rev 2 amendment: the
/// window rides on the request). `not_before` 60 s in the past, `not_after`
/// ~1 h in the future, so a directly-`issue_svid`'d leaf is valid *now* under
/// the restart-chain `openssl verify`. NOTE: `issue_and_audit` IGNORES this
/// window and builds its own from the injected clock (the clock is the single
/// window SSOT) — this window is consumed only by the direct `issue_svid`
/// callers in the restart-chain tests.
fn workload_request() -> SvidRequest {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("wall-clock after epoch");
    let not_before = UnixInstant::from_unix_duration(now.saturating_sub(Duration::from_secs(60)));
    let not_after = not_before + Duration::from_secs(3600);
    SvidRequest::new(
        SpiffeId::new("spiffe://overdrive.local/overdrive/workload/dns-resolver")
            .expect("valid workload SpiffeId"),
        not_before,
        not_after,
    )
}

/// `@real-io` `@adapter-integration` `@S-05` — every issuance writes an
/// `issued_certificates` observation row through the `ObservationStore` port
/// (`ObservationRow::IssuedCertificate`); a test reads it back via the
/// `ObservationStore::issued_certificate_rows` read surface — the same way it
/// reads an `alloc_status` row — and asserts serial + `spiffe_id` +
/// `issuer_serial` match the minted cert (the internal-CT-equivalent audit
/// surface).
#[tokio::test]
async fn issuance_writes_issued_certificates_row_matching_the_minted_cert() {
    // GIVEN a host CA, a real observation-store binding (the PORT), and a fixed
    // clock.
    let obs_dir = TempDir::new().expect("obs-store tempdir");
    let ca = host_ca();
    let audit = audit_store(&obs_dir);
    let clock = FixedClock::at_unix_secs(1_700_000_005);
    let node = issuing_node();
    let request = workload_request();

    // WHEN the workload-start path issues the SVID through the issuance seam,
    // which writes the audit row through the `ObservationStore` port.
    let svid = ca_issuance::issue_and_audit(&ca, audit.as_ref(), &clock, &node, &request)
        .await
        .expect("issuance + audit write succeeds");

    // AND the issuer serial the audit row should carry is the node
    // intermediate's serial (the chain link recorded on the row).
    let issuer_serial = ca.issue_intermediate(&node).expect("intermediate").serial().clone();

    // THEN reading the audit surface back via the ObservationStore read path
    // yields exactly one row whose serial + spiffe_id + issuer_serial match the
    // minted cert's FAITHFUL accessors (not the cert bytes).
    let rows = audit.issued_certificate_rows().await.expect("read back audit rows");
    assert_eq!(rows.len(), 1, "exactly one issued_certificates row must be written per issuance");
    let row = &rows[0];
    assert_eq!(
        &row.serial,
        svid.serial(),
        "audit row serial must match the minted SVID serial (faithful accessor)"
    );
    assert_eq!(
        &row.spiffe_id,
        svid.spiffe_id(),
        "audit row spiffe_id must match the minted SVID identity (faithful accessor)"
    );
    assert_eq!(
        row.issuer_serial, issuer_serial,
        "audit row issuer_serial must match the node intermediate's serial"
    );

    // AND the consistency invariant the ADR-0063 rev 2 amendment pins: the
    // minted leaf's `not_after` EQUALS the audit row's `not_after` by
    // construction (one clock read, one window value, threaded into the leaf via
    // the windowed `SvidRequest` AND recorded on the row — ADR-0067 rev 3 D8).
    // This is the behavioral acceptance of the window-threading change: before
    // it, the leaf's window was a separate `SystemTime::now()` read in `RcgenCa`
    // that could not equal the audit row's clock-derived window.
    assert_eq!(
        svid.not_after(),
        row.not_after,
        "minted SVID not_after must equal the issued_certificates row not_after (single window SSOT)"
    );
}

/// `@real-io` `@adapter-integration` `@S-05` — regression (built-in-ca review):
/// the `issued_certificates` audit window must FAITHFULLY mirror the window the
/// host issuer actually signs the leaf with. The issuer sets `not_before = now
/// − SKEW_TOLERANCE` and `not_after = not_before + WORKLOAD_SVID_TTL`
/// (`overdrive-host` `RcgenCa`); the auditor must record the SAME shape from the
/// SAME `overdrive_core::ca` constants. Before the fix the auditor recorded
/// `not_before = issued_at` (omitting the skew back-off), so the audit row
/// claimed the leaf was invalid for the first 60 s of its real validity — a
/// systematic, drift-prone discrepancy. This test pins the back-off; it fails
/// RED against the pre-fix `not_before = issued_at` computation.
#[tokio::test]
async fn audit_window_mirrors_the_issued_leaf_window_with_skew_backoff() {
    // GIVEN a host CA, a real observation-store binding (the PORT), and a clock
    // pinned to a fixed Unix second so the recorded window is deterministic.
    const ISSUED_AT_SECS: u64 = 1_700_000_005;
    let obs_dir = TempDir::new().expect("obs-store tempdir");
    let ca = host_ca();
    let audit = audit_store(&obs_dir);
    let clock = FixedClock::at_unix_secs(ISSUED_AT_SECS);
    let node = issuing_node();
    let request = workload_request();

    // WHEN the workload-start path issues the SVID through the issuance seam.
    ca_issuance::issue_and_audit(&ca, audit.as_ref(), &clock, &node, &request)
        .await
        .expect("issuance + audit write succeeds");

    // THEN the single audit row's window is reconstructed from the SHARED
    // constants: `issued_at` is the clock snapshot, `not_before` is backed off
    // by `SKEW_TOLERANCE`, and the window width is exactly `WORKLOAD_SVID_TTL`.
    let rows = audit.issued_certificate_rows().await.expect("read back audit rows");
    assert_eq!(rows.len(), 1, "exactly one issued_certificates row must be written per issuance");
    let row = &rows[0];

    let issued_at = UnixInstant::from_unix_duration(Duration::from_secs(ISSUED_AT_SECS));
    let expected_not_before = UnixInstant::from_unix_duration(
        issued_at.as_unix_duration().saturating_sub(SKEW_TOLERANCE),
    );
    let expected_not_after = expected_not_before + WORKLOAD_SVID_TTL;

    assert_eq!(row.issued_at, issued_at, "audit issued_at must be the clock snapshot, unchanged");
    assert_eq!(
        row.not_before, expected_not_before,
        "audit not_before must back off by SKEW_TOLERANCE to mirror the issued leaf — not start at issued_at"
    );
    assert_eq!(
        row.not_after, expected_not_after,
        "audit not_after must be not_before + WORKLOAD_SVID_TTL (window width = the leaf TTL)"
    );
    // The regression guard: the pre-fix code recorded not_before = issued_at.
    // Pin that it no longer does, so a future revert is caught loud.
    assert_ne!(
        row.not_before, issued_at,
        "regression: audit not_before must NOT equal issued_at — that omits the 60s skew back-off the leaf is signed with"
    );
}

/// `@real-io` `@adapter-integration` `@S-05` `@error` — no silent issuance
/// (US-CA-05 AC + SSOT journey): an issuance whose `issued_certificates`
/// audit row cannot be written surfaces a `CaError` rather than handing out
/// an unaudited certificate (issuance + audit are observable together).
///
/// The fault is injected through the `ObservationStore` PORT — a
/// `SimObservationStore` with a queued write failure (`inject_write_failure`).
/// Because the audit write now flows through `ObservationStore::write`, the
/// DST sim adapter IS the audit path, so the no-silent-issuance bind is
/// exercised against the same port the production `LocalObservationStore`
/// implements.
#[tokio::test]
async fn issuance_that_cannot_write_audit_row_surfaces_an_error() {
    // GIVEN a host CA but an observation store (the PORT) whose NEXT write fails
    // — the audit-write fault injected through the sim adapter's queue.
    let ca = host_ca();
    let obs = SimObservationStore::single_peer(issuing_node(), 0xCA_05_04);
    obs.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
        "audit store unavailable (injected)",
    )));
    let clock = FixedClock::at_unix_secs(1_700_000_005);
    let node = issuing_node();
    let request = workload_request();

    // WHEN issuance is attempted.
    let result = ca_issuance::issue_and_audit(&ca, &obs, &clock, &node, &request).await;

    // THEN the issuance is REFUSED with a typed audit error — NO SvidMaterial is
    // returned, so no unaudited certificate escapes (issuance is never silent).
    assert!(
        matches!(result, Err(CaIssuanceError::Audit { .. })),
        "an audit-write failure must refuse the issuance with CaIssuanceError::Audit and hand out \
         NO certificate, got {result:?}"
    );

    // AND no audit row was recorded — the failed write mutated no observed state,
    // so the audit surface is empty (the cert and its row are observable together
    // or not at all).
    let rows = obs.issued_certificate_rows().await.expect("read back audit rows");
    assert!(
        rows.is_empty(),
        "a refused issuance must leave NO audit row behind, got {} rows",
        rows.len()
    );
}

/// `@real-io` `@adapter-integration` `@S-05` — re-issue without restart: the
/// platform re-issues a fresh SVID for an existing `SpiffeId`; a new leaf
/// (distinct serial, new validity window) is produced and the control plane
/// is NOT restarted — the re-issue mechanism the #40 rotation workflow will
/// later drive on a schedule.
#[tokio::test]
async fn svid_is_reissued_on_demand_without_control_plane_restart() {
    // GIVEN a single host CA + audit store composed ONCE (the running control
    // plane); re-issue must work against this SAME composition, no restart.
    let obs_dir = TempDir::new().expect("obs-store tempdir");
    let ca = host_ca();
    let audit = audit_store(&obs_dir);
    let clock = FixedClock::at_unix_secs(1_700_000_005);
    let node = issuing_node();
    let request = workload_request();

    // WHEN the SAME identity is issued twice against the SAME running composition.
    let first = ca_issuance::issue_and_audit(&ca, audit.as_ref(), &clock, &node, &request)
        .await
        .expect("first issuance succeeds");
    let second = ca_issuance::issue_and_audit(&ca, audit.as_ref(), &clock, &node, &request)
        .await
        .expect("re-issue on the running control plane succeeds (no restart)");

    // THEN both leaves carry the SAME identity but the re-issue is a FRESH leaf:
    // a distinct serial (re-issue is not cached — Ca::issue_svid mints anew).
    assert_eq!(first.spiffe_id(), second.spiffe_id(), "re-issue is for the SAME workload identity");
    assert_ne!(
        first.serial(),
        second.serial(),
        "re-issue must yield a FRESH leaf with a DISTINCT serial (not a cached cert)"
    );
    assert_ne!(
        first.cert_der(),
        second.cert_der(),
        "the re-issued leaf must be byte-distinct from the first (fresh certificate)"
    );

    // AND both issuances were audited on the running store — two rows, one per
    // issuance, read back via the ObservationStore.
    let rows = audit.issued_certificate_rows().await.expect("read back audit rows");
    assert_eq!(
        rows.len(),
        2,
        "each on-demand re-issue writes its own issued_certificates row (no restart needed)"
    );
}
