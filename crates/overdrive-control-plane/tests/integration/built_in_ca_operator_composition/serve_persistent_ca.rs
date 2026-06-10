//! Integration — `overdrive serve` persistent-CA boot composition per
//! `built-in-ca-operator-composition` Slice ② (folds GH #215 boot-side; closes
//! D-CA-4). DELIVER (step 02-03) — scaffolds ACTIVATED.
//!
//! Layer 3 (real `RcgenCa` + injected `SimKek` boot KEK + real redb
//! `LocalIntentStore`; the WIRED `run_server` composition root is the driving
//! port, run under Lima per `.claude/rules/testing.md`). Per Mandate 11 these
//! layer-3 sad paths are EXAMPLE-ONLY (one example per failure mode); no PBT
//! machinery.
//!
//! Settled design (feature-delta.md D-OC-4/5/6): `run_server` replaces the
//! ephemeral `RcgenCa::new` + `root()` + `issue_intermediate()` block with the
//! already-implemented, already-probing `boot_ca` + `bootstrap_node_intermediate`
//! path (KEK-resolve probe (a) → envelope decrypt-probe (b) → adopt-or-refuse).
//! `ControlPlaneError::CaBoot(#[from] CaBootError)` is the dedicated typed
//! variant so the distinct `CaError` cause (`WrongKek` vs `TamperedEnvelope`,
//! already-split Display) survives to the operator's stderr.
//!
//! Why `run_server` is the driving port (NOT a forked `overdrive` binary):
//! `overdrive-cli`'s CLAUDE.md forbids subprocess-spawning the binary in tests,
//! and the `server_lifecycle.rs` precedent drives `run_server` in-process with
//! an injected `SimKek::for_boot()` + real redb + `RealCgroupFs` under Lima.
//! `run_server` IS the wired composition root the production `serve` verb calls
//! (`overdrive_cli::commands::serve::run_inner` → `run_server`); driving it
//! directly exercises the same `boot_ca`/`bootstrap_node_intermediate` wiring
//! the operator hits. LITMUS: delete the L1653 `boot_ca` call in `run_server`
//! and every refusal scenario below goes GREEN-when-it-should-be-RED (the boot
//! no longer refuses), so these tests are pinned to the production wiring, not a
//! fixture.
//!
//! EDD: S-OC-06/07 capture D01 (root key never plaintext at rest — on-disk
//! byte-scan); S-OC-08a/b/c/d + S-OC-09 capture O04 (refuse-to-start — one
//! scenario per cause: wrong-KEK / tampered-envelope / absent-KEK, plus a
//! pairwise-distinct-stderr contract — and no silent re-mint). The in-tree boot
//! tests in `ca_boot_and_audit.rs` (S-02-06/07) prove refuse-to-start at the
//! `boot_ca` seam; these scenarios prove the SAME behaviour through the WIRED
//! `run_server` composition root (the prior ephemeral path probed nothing).

#![allow(clippy::expect_used, clippy::unwrap_used)]
// Doc comments on these test helpers name protocol identifiers (AES-256-GCM,
// IntentStore, TamperedEnvelope, …) prose-style; back-ticking each is churn with
// no reader benefit in a test file. `single_match_else` flags the
// `assert_refused_cause` match whose Ok arm is a panic and whose two Err arms
// are distinct — the match form reads clearer than an if-let-else chain here.
#![allow(clippy::doc_markdown, clippy::single_match_else)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::{ServerConfig, ServerHandle, ca_boot, run_server};
use overdrive_core::ca::kek::KEK_LEN;
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::SimKek;
use overdrive_sim::adapters::dataplane::SimDataplane;
use tempfile::TempDir;

/// The canonical single-node boot KEK identity the persistent CA seals under.
fn boot_kek_id() -> KekId {
    ca_boot::root_kek_id()
}

/// A `SimKek` that resolves the boot KEK to the canonical fixture material — the
/// CORRECT KEK (`SimKek::for_boot()` registers `overdrive-ca-root → [0x5a; 32]`).
fn correct_kek() -> Arc<dyn overdrive_core::ca::kek::Kek> {
    Arc::new(SimKek::for_boot())
}

/// A `SimKek` that resolves the boot KEK to DIFFERENT material — the envelope
/// AEAD-opens with the wrong key → `CaError::WrongKek` → `CaBootError::EnvelopeDecrypt`.
fn wrong_kek() -> Arc<dyn overdrive_core::ca::kek::Kek> {
    Arc::new(SimKek::new().with(&boot_kek_id(), [0x22; KEK_LEN]))
}

/// A `SimKek` that resolves NOTHING — every `resolve` is `KekError::NotFound` →
/// `CaBootError::KekUnavailable` (refused before any issuance, no throwaway KEK).
fn absent_kek() -> Arc<dyn overdrive_core::ca::kek::Kek> {
    Arc::new(SimKek::new())
}

/// A control-plane data directory + operator-config directory under one tempdir.
/// The redb `IntentStore` (`intent.redb`) lands under `data`. Kept alive by the
/// caller holding the returned `TempDir`.
struct ServeDirs {
    _tmp: TempDir,
    data_dir: std::path::PathBuf,
    operator_config_dir: std::path::PathBuf,
}

impl ServeDirs {
    fn new() -> Self {
        let tmp = TempDir::new().expect("serve tempdir");
        let data_dir = tmp.path().join("data");
        let operator_config_dir = tmp.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");
        Self { _tmp: tmp, data_dir, operator_config_dir }
    }

    /// The on-disk redb IntentStore path `run_server` opens under `data_dir`
    /// (ADR-0013 §5 storage root). This is the file D01 byte-scans and the file
    /// the O04 refusal stderr must name.
    fn intent_redb(&self) -> std::path::PathBuf {
        self.data_dir.join("intent.redb")
    }
}

/// Attempt a `run_server` boot against `dirs` with the supplied `kek`, returning
/// the `Result` so refusal scenarios can assert the typed `Err` and happy
/// scenarios can drive the returned `ServerHandle`.
///
/// Injects `SimDataplane` (no XDP attach) + the shared `lo`/`lo` dataplane
/// config (so `host_ipv4` resolves on a VM with no provisioned veth) per the
/// `server_lifecycle.rs` precedent. The KEK is the SUT — it is threaded through
/// `ServerConfig::new(kek)` exactly as the production `serve` verb threads
/// `SystemdCredsKeyring`.
async fn boot_attempt(
    dirs: &ServeDirs,
    kek: Arc<dyn overdrive_core::ca::kek::Kek>,
) -> Result<ServerHandle, ControlPlaneError> {
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir: dirs.data_dir.clone(),
        operator_config_dir: dirs.operator_config_dir.clone(),
        dataplane_override: Some(Arc::new(SimDataplane::new())),
        dataplane: Some(super::super::dataplane_lo::lo_dataplane_config()),
        ..ServerConfig::new(kek)
    };
    run_server(config, Arc::new(RealCgroupFs::new())).await
}

/// Read the bytes of the on-disk redb IntentStore. The file MUST exist after a
/// boot that persisted the root.
fn read_intent_bytes(dirs: &ServeDirs) -> Vec<u8> {
    std::fs::read(dirs.intent_redb()).unwrap_or_else(|e| {
        panic!("read on-disk IntentStore at {}: {e}", dirs.intent_redb().display())
    })
}

/// D01 guardrail (disk-observable half): the on-disk IntentStore MUST NOT carry
/// any plaintext PKCS#8 private-key PEM. The sealed root key is AEAD ciphertext;
/// the PEM armor markers (`-----BEGIN ... PRIVATE KEY-----`) of the plaintext
/// key can only appear if the key was written un-sealed. A byte-scan for the
/// armor markers is the honest, key-material-free disk scan (the sealed PEM
/// plaintext the boot path round-trips IS armored, so its markers would show
/// through if it were ever persisted un-sealed).
fn assert_no_plaintext_private_key_on_disk(bytes: &[u8]) {
    // The two armor markers of a PKCS#8 / SEC1 private-key PEM. The boot path
    // seals the root key in PEM form (see `ca_boot_and_audit::
    // persisted_root_key_envelope_seals_pem_not_der`), so if the plaintext ever
    // reached disk these markers would appear verbatim in the redb bytes.
    const BEGIN_MARKER: &[u8] = b"-----BEGIN";
    const PRIVATE_KEY_MARKER: &[u8] = b"PRIVATE KEY-----";
    let has_begin = bytes.windows(BEGIN_MARKER.len()).any(|w| w == BEGIN_MARKER);
    let has_private_key = bytes.windows(PRIVATE_KEY_MARKER.len()).any(|w| w == PRIVATE_KEY_MARKER);
    assert!(
        !(has_begin && has_private_key),
        "K3 guardrail violated: the on-disk IntentStore contains a plaintext PRIVATE KEY PEM \
         (BEGIN marker present={has_begin}, PRIVATE KEY marker present={has_private_key}); the \
         sealed root key must be AEAD ciphertext, never armored plaintext"
    );
}

/// Prove the booted server reached a SERVING state: its bound address is a live
/// listener (a fresh TCP connect succeeds), then shut it down.
async fn assert_serving_then_shutdown(handle: ServerHandle) {
    let bound = handle.local_addr().await.expect("bound addr");
    assert!(
        bound.port() > 0,
        "a serving control plane binds a non-zero ephemeral port, got {bound}"
    );
    let conn = tokio::net::TcpStream::connect(("127.0.0.1", bound.port())).await;
    assert!(
        conn.is_ok(),
        "the bound listener must accept a TCP connection (serving), got {conn:?}"
    );
    handle.shutdown(Duration::from_secs(2)).await;
}

// S-OC-06 `@integration @real-io @adapter-integration @driving_port @slice-2
// @edd:D01` — on a CLEAN IntentStore with a resolvable KEK, `run_server` FIRST
// boot generates a self-signed P-256 root + a node intermediate, persists the
// root as a KEK-sealed AES-256-GCM envelope in the IntentStore file, and reaches
// a serving state. EDD D01 sub-claims 1+2: the on-disk file carries the sealed
// envelope (non-empty) and ZERO plaintext root-key PEM (byte-scan). Universe:
// the serve startup outcome + the on-disk IntentStore file contents.
#[tokio::test]
async fn serve_first_boot_generates_seals_and_persists_root() {
    let dirs = ServeDirs::new();

    // WHEN the control plane boots on a CLEAN store with the correct boot KEK.
    let handle = boot_attempt(&dirs, correct_kek())
        .await
        .expect("first boot generates + KEK-seals + persists the root and reaches serving");

    // THEN it reached a serving state (the boot_ca probe + adopt succeeded and
    // the listener bound).
    assert_serving_then_shutdown(handle).await;

    // AND the on-disk IntentStore exists, is non-empty (the sealed envelope was
    // persisted), and carries NO plaintext private-key PEM (D01 sub-claims 1+2).
    let bytes = read_intent_bytes(&dirs);
    assert!(
        !bytes.is_empty(),
        "the IntentStore must be non-empty after persisting the sealed root"
    );
    assert_no_plaintext_private_key_on_disk(&bytes);
}

// S-OC-07 `@integration @real-io @adapter-integration @driving_port @slice-2
// @edd:D01` — a control plane that booted once and persisted a KEK-sealed root,
// restarted with the SAME KEK available, decrypts and ADOPTS the SAME root
// (identical root serial across the restart) and does NOT generate a new root.
// EDD D01 sub-claim 3: the on-disk file STILL contains no plaintext key bytes.
// Universe: the persisted root cert material (byte-identical across restart,
// proving adopt-not-remint) + the on-disk byte-scan after restart.
#[tokio::test]
async fn serve_restart_adopts_same_root_no_remint() {
    let dirs = ServeDirs::new();

    // First boot persists the root; capture the persisted public root cert
    // material bytes from the IntentStore (the adopt-vs-remint discriminator —
    // a re-mint would overwrite these with a new keypair's cert).
    let h1 = boot_attempt(&dirs, correct_kek()).await.expect("first boot persists the root");
    h1.shutdown(Duration::from_secs(2)).await;
    let root_material_first = persisted_root_cert_material(&dirs).await;

    // WHEN the control plane RESTARTS with the SAME KEK (a fresh run_server over
    // the SAME on-disk store — the first handle is shut down before the second
    // boots, so reuse is proven through on-disk persistence, not in-process
    // caching).
    let h2 = boot_attempt(&dirs, correct_kek())
        .await
        .expect("restart decrypts + adopts the persisted root and reaches serving");
    assert_serving_then_shutdown(h2).await;
    let root_material_second = persisted_root_cert_material(&dirs).await;

    // THEN the persisted root cert material is BYTE-IDENTICAL across the restart:
    // the restart adopted the SAME root (no new keypair, no overwrite). A fresh
    // `ca.root()` on restart would mint a new keypair → different persisted cert.
    assert_eq!(
        root_material_first, root_material_second,
        "restart must ADOPT the persisted root (byte-identical persisted cert material), not re-mint"
    );

    // AND the on-disk file STILL carries no plaintext key (the guardrail holds
    // across the lifecycle — D01 sub-claim 3).
    assert_no_plaintext_private_key_on_disk(&read_intent_bytes(&dirs));
}

// S-OC-08a `@integration @real-io @error @driving_port @slice-2 @edd:O04` —
// refuse-to-start on the WRONG KEK: a persisted root whose envelope cannot be
// opened with the supplied KEK → CaError::WrongKek wrapped in
// CaBootError::EnvelopeDecrypt. The control plane does NOT begin serving; the
// typed error names the wrong-KEK cause + the IntentStore path. EDD O04
// sub-claim 1. Universe: the boot Err outcome + the wrong-KEK cause string.
#[tokio::test]
async fn serve_refuses_on_wrong_kek() {
    let dirs = ServeDirs::new();
    // GIVEN a control plane that persisted its root under the CORRECT KEK.
    boot_attempt(&dirs, correct_kek())
        .await
        .expect("first boot persists the root under the correct KEK")
        .shutdown(Duration::from_secs(2))
        .await;

    // WHEN it restarts under the WRONG KEK — the persisted envelope cannot
    // AES-GCM-open under it.
    let result = boot_attempt(&dirs, wrong_kek()).await;

    // THEN the boot REFUSES with the typed CaBoot(EnvelopeDecrypt) error naming
    // the IntentStore path — and NO ServerHandle is returned (no serving).
    let message = assert_refused_cause(result, "wrong-KEK");
    assert!(
        message.contains(&dirs.intent_redb().display().to_string()),
        "the wrong-KEK refusal must name the IntentStore path {}; got: {message}",
        dirs.intent_redb().display()
    );
}

// S-OC-08b `@integration @real-io @error @driving_port @slice-2 @edd:O04` —
// refuse-to-start on a TAMPERED envelope: a persisted root whose envelope bytes
// were mutated → CaError::TamperedEnvelope wrapped in EnvelopeDecrypt. The
// control plane does NOT begin serving; the stderr names the tampered-envelope
// cause + the IntentStore path. EDD O04 sub-claim 2. Universe: the boot Err
// outcome + the tampered-envelope cause string.
#[tokio::test]
async fn serve_refuses_on_tampered_envelope() {
    let dirs = ServeDirs::new();
    // GIVEN a control plane that persisted its root under the correct KEK.
    boot_attempt(&dirs, correct_kek())
        .await
        .expect("first boot persists the root")
        .shutdown(Duration::from_secs(2))
        .await;

    // WHEN the persisted root-key envelope's AEAD tag is corrupted (flip the
    // last byte of the sealed envelope value), then it restarts under the
    // CORRECT KEK — the KEK resolves but the AEAD open fails authentication.
    tamper_root_key_envelope(&dirs).await;
    let result = boot_attempt(&dirs, correct_kek()).await;

    // THEN the boot REFUSES with the typed CaBoot(EnvelopeDecrypt) error naming
    // the IntentStore path — and NO ServerHandle (no serving).
    let message = assert_refused_cause(result, "tampered-envelope");
    assert!(
        message.contains(&dirs.intent_redb().display().to_string()),
        "the tampered-envelope refusal must name the IntentStore path {}; got: {message}",
        dirs.intent_redb().display()
    );
}

// S-OC-08c `@integration @real-io @error @driving_port @slice-2 @edd:O04` —
// refuse-to-start when the KEK is ABSENT: NO KEK resolvable → KekUnavailable,
// refused BEFORE any issuance, and NO throwaway KEK is generated (no envelope
// persisted). The control plane does NOT begin serving; the error names the
// absent-KEK cause. EDD O04 sub-claim 3. Universe: the boot Err outcome + the
// absent-KEK cause string + the absence of a persisted envelope.
#[tokio::test]
async fn serve_refuses_on_absent_kek() {
    let dirs = ServeDirs::new();

    // WHEN the control plane boots on a CLEAN store with an ABSENT KEK.
    let result = boot_attempt(&dirs, absent_kek()).await;

    // THEN it refuses with the typed CaBoot(KekUnavailable) error BEFORE any
    // issuance — and NO ServerHandle (no serving).
    let message = assert_refused_cause(result, "absent-KEK");
    assert!(
        message.to_lowercase().contains("kek"),
        "the absent-KEK refusal must name the KEK as the cause; got: {message}"
    );

    // AND no throwaway KEK / root envelope was persisted: the IntentStore either
    // does not exist or carries no plaintext key (the probe refused before
    // generate-or-persist). The discriminating check is the absence of any
    // persisted root-key envelope.
    assert!(
        !persisted_root_key_envelope_exists(&dirs).await,
        "an absent-KEK refusal must persist NO root-key envelope (no throwaway KEK minted)"
    );
}

// S-OC-08d `@integration @real-io @error @driving_port @slice-2 @edd:O04` — the
// three refusal causes render PAIRWISE-DISTINCT stderr: an operator can tell
// wrong-KEK, tampered-envelope, and absent-KEK apart from the surfaced cause
// alone. EDD O04 sub-claims 1–3 (cross-cause contract). Universe: the three
// captured cause strings, compared for pairwise distinctness.
#[tokio::test]
async fn serve_refusal_causes_are_pairwise_distinct() {
    // Capture the wrong-KEK cause.
    let wrong = {
        let dirs = ServeDirs::new();
        boot_attempt(&dirs, correct_kek())
            .await
            .expect("seed root")
            .shutdown(Duration::from_secs(2))
            .await;
        assert_refused_cause(boot_attempt(&dirs, wrong_kek()).await, "wrong-KEK")
    };

    // Capture the tampered-envelope cause.
    let tampered = {
        let dirs = ServeDirs::new();
        boot_attempt(&dirs, correct_kek())
            .await
            .expect("seed root")
            .shutdown(Duration::from_secs(2))
            .await;
        tamper_root_key_envelope(&dirs).await;
        assert_refused_cause(boot_attempt(&dirs, correct_kek()).await, "tampered-envelope")
    };

    // Capture the absent-KEK cause.
    let absent = {
        let dirs = ServeDirs::new();
        assert_refused_cause(boot_attempt(&dirs, absent_kek()).await, "absent-KEK")
    };

    // THEN the three cause strings are PAIRWISE DISTINCT — the operator can
    // triage from stderr alone.
    assert_ne!(wrong, tampered, "wrong-KEK and tampered-envelope causes must be distinct");
    assert_ne!(wrong, absent, "wrong-KEK and absent-KEK causes must be distinct");
    assert_ne!(tampered, absent, "tampered-envelope and absent-KEK causes must be distinct");
}

// S-OC-09 `@integration @real-io @error @driving_port @slice-2 @edd:O04` — a
// boot that REFUSED (wrong KEK) does NOT silently re-mint: re-supplying the
// correct KEK and starting again adopts the SAME original root (byte-identical
// persisted cert material), and no new root envelope was written during the
// refused boot. EDD O04 sub-claim 4 — the load-bearing guardrail. Universe: the
// persisted root cert material before the refused boot, after it, and after the
// recovery boot (all identical).
#[tokio::test]
async fn refuse_to_start_does_not_remint_the_root() {
    let dirs = ServeDirs::new();

    // GIVEN a first boot that persists the root; capture its persisted cert
    // material.
    boot_attempt(&dirs, correct_kek())
        .await
        .expect("first boot persists the root")
        .shutdown(Duration::from_secs(2))
        .await;
    let material_before = persisted_root_cert_material(&dirs).await;

    // WHEN a boot REFUSES under the wrong KEK.
    assert_refused_cause(boot_attempt(&dirs, wrong_kek()).await, "wrong-KEK");

    // THEN the persisted root cert material is UNCHANGED by the refused boot (no
    // silent re-mint during refusal).
    assert_eq!(
        material_before,
        persisted_root_cert_material(&dirs).await,
        "a refused boot must NOT overwrite or re-mint the persisted root"
    );

    // AND re-supplying the CORRECT KEK adopts the SAME original root and reaches
    // serving — the persisted material is still byte-identical to the first
    // boot's (the original root survived the refusal intact).
    let recovered = boot_attempt(&dirs, correct_kek())
        .await
        .expect("recovery boot under the correct KEK adopts the original root and serves");
    assert_serving_then_shutdown(recovered).await;
    assert_eq!(
        material_before,
        persisted_root_cert_material(&dirs).await,
        "the recovery boot must adopt the SAME original root (no re-mint across the refused boot)"
    );
}

// ---------------------------------------------------------------------------
// IntentStore inspection helpers — open the persisted redb out of band to read
// the public root cert material / envelope, the same keys `ca_boot` persists
// under. These read the DRIVEN-port observable state `run_server` wrote.
// ---------------------------------------------------------------------------

/// The IntentStore key under which `ca_boot` persists the PUBLIC root cert
/// material (PEM + DER + serial). Mirrors `ca_boot::ROOT_CERT_MATERIAL_KEY` (a
/// private const in the crate; the literal is the stable persisted key).
const ROOT_CERT_MATERIAL_KEY: &[u8] = b"ca/root/cert-material/v1";

/// The IntentStore key under which `ca_boot` persists the SEALED root-key
/// envelope. Mirrors `ca_boot::ROOT_KEY_ENVELOPE_KEY`.
const ROOT_KEY_ENVELOPE_KEY: &[u8] = b"ca/root/key-envelope/v1";

/// Open the persisted IntentStore out of band, retrying with an ASYNC sleep
/// while the just-shut-down `run_server`'s background tasks finish releasing the
/// redb single-writer lock (`DatabaseAlreadyOpen` is transient post-shutdown —
/// the `AppState` store Arc drops as the joined task closures are reaped, which
/// needs the tokio runtime to keep progressing, so a BLOCKING sleep would
/// deadlock the release). Panics if the lock never releases within the window.
async fn open_persisted_intent_store(dirs: &ServeDirs) -> overdrive_store_local::LocalIntentStore {
    for _ in 0..100 {
        match overdrive_store_local::LocalIntentStore::open(dirs.intent_redb()) {
            Ok(store) => return store,
            Err(_) => {
                tokio::task::yield_now().await;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
    overdrive_store_local::LocalIntentStore::open(dirs.intent_redb())
        .expect("open persisted IntentStore (redb lock did not release after shutdown)")
}

/// Open the persisted IntentStore and read the public root cert material bytes
/// (the adopt-vs-remint discriminator). Panics if absent — every happy/refused
/// scenario that calls this has already persisted the root.
async fn persisted_root_cert_material(dirs: &ServeDirs) -> Vec<u8> {
    use overdrive_core::traits::intent_store::IntentStore as _;
    let store = open_persisted_intent_store(dirs).await;
    store
        .get(ROOT_CERT_MATERIAL_KEY)
        .await
        .expect("IntentStore get root cert material")
        .expect("the boot persisted public root cert material")
        .to_vec()
}

/// Whether the persisted IntentStore carries a root-key envelope at all (the
/// absent-KEK no-throwaway-KEK check). `false` when the store file does not
/// exist (the refusal fired before generate-or-persist) or the key is unset.
async fn persisted_root_key_envelope_exists(dirs: &ServeDirs) -> bool {
    use overdrive_core::traits::intent_store::IntentStore as _;
    if !dirs.intent_redb().exists() {
        return false;
    }
    let store = open_persisted_intent_store(dirs).await;
    store.get(ROOT_KEY_ENVELOPE_KEY).await.expect("IntentStore get envelope").is_some()
}

/// Corrupt the persisted root-key envelope's AEAD authentication by flipping the
/// last byte of the stored value, then write it back. A subsequent boot under
/// the CORRECT KEK resolves the KEK but fails the AEAD open → TamperedEnvelope.
async fn tamper_root_key_envelope(dirs: &ServeDirs) {
    use overdrive_core::traits::intent_store::IntentStore as _;
    let store = open_persisted_intent_store(dirs).await;
    let mut bytes = store
        .get(ROOT_KEY_ENVELOPE_KEY)
        .await
        .expect("IntentStore get envelope for tamper")
        .expect("the boot persisted a root-key envelope to tamper")
        .to_vec();
    let last = bytes.len().checked_sub(1).expect("envelope is non-empty");
    bytes[last] ^= 0xff;
    store.put(ROOT_KEY_ENVELOPE_KEY, &bytes).await.expect("write tampered envelope back");
}

/// Assert a `run_server` boot result is a typed `ControlPlaneError::CaBoot`
/// refusal (NO `ServerHandle` returned — no serving), returning the rendered
/// cause message (the operator-visible stderr the CLI surfaces). `label` names
/// the scenario for the panic message.
fn assert_refused_cause(result: Result<ServerHandle, ControlPlaneError>, label: &str) -> String {
    match result {
        Ok(_handle) => {
            panic!("{label}: the control plane must REFUSE to start, but run_server returned Ok")
        }
        Err(ControlPlaneError::CaBoot(cause)) => cause.to_string(),
        Err(other) => panic!(
            "{label}: the refusal must be the typed ControlPlaneError::CaBoot, got {other:?}"
        ),
    }
}
