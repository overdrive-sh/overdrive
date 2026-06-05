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
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_control_plane::ca_boot::{self, CaBootError};
use overdrive_control_plane::ca_issuance::{self, CaIssuanceError, IssuedCertificateAudit};
use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::ca::kek::KEK_LEN;
use overdrive_core::traits::ca::{
    Ca, CaError, IntermediateHandle, RootCaHandle, SvidMaterial, SvidRequest, TrustBundle,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStoreError;
use overdrive_core::{NodeId, SpiffeId};
use overdrive_host::OsEntropy;
use overdrive_host::ca::{RcgenCa, RootKeyAeadCodec, SystemdCredsKeyring};
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

/// Open a `LocalIntentStore` at `intent.redb` under `dir` as an
/// `Arc<dyn IntentStore>`.
fn intent_store(dir: &TempDir) -> Arc<dyn IntentStore> {
    Arc::new(
        overdrive_store_local::LocalIntentStore::open(dir.path().join("intent.redb"))
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
    let first = {
        let intent = intent_store(&store_dir);
        ca_boot::boot_ca(&host_ca(), &kek, &kek_id, &codec, &intent)
            .await
            .expect("first boot generates + persists the root")
    };

    let second = {
        let intent = intent_store(&store_dir);
        ca_boot::boot_ca(&host_ca(), &kek, &kek_id, &codec, &intent)
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

    let first = {
        let intent = intent_store(&store_dir);
        ca_boot::boot_ca(&host_ca(), &kek, &kek_id, &codec, &intent)
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
        ca_boot::boot_ca(&host_ca(), &wrong_kek, &kek_id, &codec, &intent).await
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
        ca_boot::boot_ca(&host_ca(), &kek, &kek_id, &codec, &intent)
            .await
            .expect("the original root is intact and re-openable under the correct KEK")
    };
    assert_eq!(
        first.cert_der(),
        recovered.cert_der(),
        "the refused boot must NOT have re-minted or overwritten the persisted root"
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
    let intent = intent_store(&store_dir);
    let result = ca_boot::boot_ca(&host_ca(), &kek, &kek_id, &codec, &intent).await;

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
    // for intermediate signing (decrypt failed upstream).
    let store_dir = TempDir::new().expect("intent-store tempdir");
    let intent = intent_store(&store_dir);
    let ca = RootKeyUnavailableCa;
    let node = NodeId::new("overdrive-node-0").expect("valid NodeId");

    // WHEN node bootstrap issues the single node intermediate through the
    // node-bootstrap driving port.
    let result = ca_boot::bootstrap_node_intermediate(&ca, &node, &intent).await;

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

/// An [`IssuedCertificateAudit`] double whose `record` ALWAYS fails — injects
/// the audit-write failure for the no-silent-issuance sad path (S-05-04) at the
/// driven-port boundary, never inside the issuance logic.
struct FailingAudit;

#[async_trait]
impl IssuedCertificateAudit for FailingAudit {
    async fn record(&self, _row: IssuedCertificateRow) -> Result<(), ObservationStoreError> {
        Err(ObservationStoreError::Io(std::io::Error::other("audit store unavailable (injected)")))
    }

    async fn issued_certificate_rows(
        &self,
    ) -> Result<Vec<IssuedCertificateRow>, ObservationStoreError> {
        Ok(Vec::new())
    }
}

/// Open a real `LocalObservationStore` at `obs.redb` under `dir` as the
/// production [`IssuedCertificateAudit`] binding.
fn audit_store(dir: &TempDir) -> Arc<dyn IssuedCertificateAudit> {
    Arc::new(
        LocalObservationStore::open(dir.path().join("obs.redb"))
            .expect("LocalObservationStore::open"),
    )
}

/// The node whose intermediate issues workload SVIDs in these tests.
fn issuing_node() -> NodeId {
    NodeId::new("overdrive-node-0").expect("valid NodeId")
}

/// A workload SVID request for the dns-resolver identity.
fn workload_request() -> SvidRequest {
    SvidRequest::new(
        SpiffeId::new("spiffe://overdrive.local/overdrive/workload/dns-resolver")
            .expect("valid workload SpiffeId"),
    )
}

/// `@real-io` `@adapter-integration` `@S-05` — every issuance writes an
/// `issued_certificates` observation row; a test reads it back via the
/// `ObservationStore` and asserts serial + `spiffe_id` + `issuer_serial` match
/// the minted cert (the internal-CT-equivalent audit surface, readable via
/// the existing `alloc status` path).
#[tokio::test]
async fn issuance_writes_issued_certificates_row_matching_the_minted_cert() {
    // GIVEN a host CA, a real observation-store audit binding, and a fixed clock.
    let obs_dir = TempDir::new().expect("obs-store tempdir");
    let ca = host_ca();
    let audit = audit_store(&obs_dir);
    let clock = FixedClock::at_unix_secs(1_700_000_005);
    let node = issuing_node();
    let request = workload_request();

    // WHEN the workload-start path issues the SVID through the issuance seam.
    let svid = ca_issuance::issue_and_audit(&ca, &audit, &clock, &node, &request)
        .await
        .expect("issuance + audit write succeeds");

    // AND the issuer serial the audit row should carry is the node
    // intermediate's serial (the chain link recorded on the row).
    let issuer_serial = ca.issue_intermediate(&node).expect("intermediate").serial().clone();

    // THEN reading the audit surface back via the ObservationStore yields
    // exactly one row whose serial + spiffe_id + issuer_serial match the minted
    // cert's FAITHFUL accessors (not the cert bytes).
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
}

/// `@real-io` `@adapter-integration` `@S-05` `@error` — no silent issuance
/// (US-CA-05 AC + SSOT journey): an issuance whose `issued_certificates`
/// audit row cannot be written surfaces a `CaError` rather than handing out
/// an unaudited certificate (issuance + audit are observable together).
#[tokio::test]
async fn issuance_that_cannot_write_audit_row_surfaces_an_error() {
    // GIVEN a host CA but an audit store whose write ALWAYS fails (injected
    // fault at the driven-port boundary).
    let ca = host_ca();
    let audit: Arc<dyn IssuedCertificateAudit> = Arc::new(FailingAudit);
    let clock = FixedClock::at_unix_secs(1_700_000_005);
    let node = issuing_node();
    let request = workload_request();

    // WHEN issuance is attempted.
    let result = ca_issuance::issue_and_audit(&ca, &audit, &clock, &node, &request).await;

    // THEN the issuance is REFUSED with a typed audit error — NO SvidMaterial is
    // returned, so no unaudited certificate escapes (issuance is never silent).
    assert!(
        matches!(result, Err(CaIssuanceError::Audit { .. })),
        "an audit-write failure must refuse the issuance with CaIssuanceError::Audit and hand out \
         NO certificate, got {result:?}"
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
    let first = ca_issuance::issue_and_audit(&ca, &audit, &clock, &node, &request)
        .await
        .expect("first issuance succeeds");
    let second = ca_issuance::issue_and_audit(&ca, &audit, &clock, &node, &request)
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
