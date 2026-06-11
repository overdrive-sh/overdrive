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
//! variant so the distinct boot cause CLASS survives to the operator's stderr.
//! The three operationally-real, pairwise-distinct Phase-1 refusal causes are
//! `{ decrypt-auth-failure, decode-malformed, KEK-unavailable }` (ADR-0063 D4 §
//! "Honest decrypt-failure cause taxonomy"). AES-256-GCM CANNOT distinguish
//! wrong KEK *material* (matching id) from a tampered ciphertext (matching id)
//! — both are one opaque `CaError::EnvelopeAuthFailed`; only the `kek_id`
//! plaintext mismatch yields `CaError::WrongKek`, which is the
//! Phase-1-unreachable rotation guard (hardcoded id) and is NOT exercised here.
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
//! scenario per cause CLASS: wrong-KEK-material (auth-failure) / corrupted
//! envelope (decode-malformed) / absent-KEK, plus a pairwise-distinct-cause-class
//! contract — and no silent re-mint). The in-tree boot
//! tests in `ca_boot_and_audit.rs` (S-02-06/07) prove refuse-to-start at the
//! `boot_ca` seam; these scenarios prove the SAME behaviour through the WIRED
//! `run_server` composition root (the prior ephemeral path probed nothing).

#![allow(clippy::expect_used, clippy::unwrap_used)]
// Doc comments on these test helpers name protocol identifiers (AES-256-GCM,
// IntentStore, EnvelopeAuthFailed, …) prose-style; back-ticking each is churn
// with no reader benefit in a test file. `single_match_else` flags the
// `refused_cause` match whose Ok arm is a panic and whose Err arm is the typed
// cause — the match form reads clearer than an if-let-else chain here.
#![allow(clippy::doc_markdown, clippy::single_match_else)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::ca_boot::CaBootError;
use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::{ServerConfig, ServerHandle, ca_boot, run_server};
use overdrive_core::ca::kek::KEK_LEN;
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_core::codec::EnvelopeError;
use overdrive_core::traits::ca::CaError;
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

/// A `SimKek` that resolves the boot KEK under the SAME id (`overdrive-ca-root`)
/// but with the WRONG key material — so the `kek_id` plaintext check PASSES
/// (this is NOT `WrongKek`, which is an id mismatch) and the AES-GCM open then
/// fails authentication → `CaError::EnvelopeAuthFailed` → `CaBootError::EnvelopeDecrypt`.
/// AEAD cannot distinguish this wrong-material case from a tampered ciphertext;
/// both are one opaque auth failure (ADR-0063 D4).
fn wrong_kek_material() -> Arc<dyn overdrive_core::ca::kek::Kek> {
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

    // AND the sealed root-key envelope actually exists in the IntentStore — a
    // non-empty file alone could be redb metadata; this asserts the SEALED
    // ENVELOPE itself was persisted under the boot key (S-OC-06 medium fix).
    assert!(
        persisted_root_key_envelope_exists(&dirs).await,
        "first boot must persist the sealed root-key envelope (not merely create a non-empty store)"
    );

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
// refuse-to-start on the WRONG KEK MATERIAL (same id): a persisted root whose
// envelope opens under a KEK with the matching id but WRONG material → the
// AES-GCM open fails authentication → CaError::EnvelopeAuthFailed wrapped in
// CaBootError::EnvelopeDecrypt. The control plane does NOT begin serving; the
// typed cause CLASS is EnvelopeAuthFailed and the rendered stderr names BOTH
// possibilities (wrong material OR tamper, indistinguishable under AEAD) + the
// IntentStore path. EDD O04 sub-claim 1. Universe: the boot Err outcome + its
// cause CLASS + the rendered cause string.
#[tokio::test]
async fn serve_refuses_on_wrong_kek_material() {
    let dirs = ServeDirs::new();
    // GIVEN a control plane that persisted its root under the CORRECT KEK.
    boot_attempt(&dirs, correct_kek())
        .await
        .expect("first boot persists the root under the correct KEK")
        .shutdown(Duration::from_secs(2))
        .await;

    // WHEN it restarts under a KEK with the SAME id but WRONG material — the id
    // check passes (NOT WrongKek), then the AES-GCM open fails authentication.
    let cause =
        refused_cause(boot_attempt(&dirs, wrong_kek_material()).await, "wrong-KEK-material");

    // THEN the refusal's cause CLASS is EnvelopeAuthFailed — asserted on the
    // typed cause, not a string. (A WrongKek here would be the bug the taxonomy
    // correction exists to catch: the id matches, so this is auth-failure.)
    assert_eq!(
        cause_class(&cause),
        RefusalCauseClass::EnvelopeAuthFailed,
        "wrong KEK MATERIAL (matching id) must surface as the EnvelopeAuthFailed cause class, \
         not WrongKek (id mismatch) nor a decode failure; got {cause:?}"
    );

    // AND the rendered stderr names the AES-GCM auth failure + BOTH possibilities
    // (wrong material OR tamper) + the IntentStore path the operator must inspect.
    let message = cause.to_string();
    assert!(
        message.contains("failed AES-GCM authentication"),
        "the auth-failure refusal must name the AES-GCM auth failure; got: {message}"
    );
    assert!(
        message.contains("KEK material is wrong OR") && message.contains("tampered/corrupted"),
        "the auth-failure refusal must name BOTH possibilities (wrong material OR tamper); \
         got: {message}"
    );
    assert!(
        message.contains(&dirs.intent_redb().display().to_string()),
        "the auth-failure refusal must name the IntentStore path {}; got: {message}",
        dirs.intent_redb().display()
    );
}

// S-OC-08b `@integration @real-io @error @driving_port @slice-2 @edd:O04` —
// refuse-to-start on a STRUCTURALLY CORRUPTED envelope: a persisted root whose
// bytes no longer deserialize into a valid RootCaKeyRecord → decode fails BEFORE
// crypto → EnvelopeError::Malformed wrapped in CaBootError::Envelope. The control
// plane does NOT begin serving; the cause CLASS is DecodeFailure (a distinct
// class from S-OC-08a's auth-failure) and the rendered stderr names the decode
// failure + the IntentStore path. EDD O04 sub-claim 2. Universe: the boot Err
// outcome + its cause CLASS + the rendered cause string.
#[tokio::test]
async fn serve_refuses_on_corrupted_envelope() {
    let dirs = ServeDirs::new();
    // GIVEN a control plane that persisted its root under the correct KEK.
    boot_attempt(&dirs, correct_kek())
        .await
        .expect("first boot persists the root")
        .shutdown(Duration::from_secs(2))
        .await;

    // WHEN the persisted root-key envelope is STRUCTURALLY corrupted (truncated
    // so it no longer deserializes), then it restarts under the CORRECT KEK —
    // decode fails before crypto runs.
    corrupt_root_key_envelope_structurally(&dirs).await;
    let cause = refused_cause(boot_attempt(&dirs, correct_kek()).await, "corrupted-envelope");

    // THEN the refusal's cause CLASS is DecodeFailure — a distinct cause class
    // from the auth-failure of S-OC-08a (this fails at decode, before crypto).
    assert_eq!(
        cause_class(&cause),
        RefusalCauseClass::DecodeFailure,
        "a structurally-corrupted envelope must surface as the DecodeFailure cause class \
         (decode fails before crypto), distinct from the auth-failure class; got {cause:?}"
    );

    // AND the rendered stderr names the decode/Malformed cause + the IntentStore
    // path the operator must inspect.
    let message = cause.to_string();
    let lowered = message.to_lowercase();
    assert!(
        lowered.contains("decode") || lowered.contains("malformed"),
        "the corrupted-envelope refusal must name a decode/Malformed cause; got: {message}"
    );
    assert!(
        message.contains(&dirs.intent_redb().display().to_string()),
        "the corrupted-envelope refusal must name the IntentStore path {}; got: {message}",
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
    let cause = refused_cause(result, "absent-KEK");
    assert_eq!(
        cause_class(&cause),
        RefusalCauseClass::KekUnavailable,
        "an absent KEK must surface as the KekUnavailable cause class; got {cause:?}"
    );
    let message = cause.to_string();
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
// three refusal causes are PAIRWISE-DISTINCT cause CLASSES: an operator can tell
// auth-failure (wrong KEK material OR tamper), decode-malformed (corrupted
// envelope), and KEK-unavailable apart by the typed cause CLASS — NOT by
// incidental rendered-string inequality (three strings can differ while still
// failing to discriminate the operator-meaningful cause; the contract is the
// cause CLASS). EDD O04 sub-claims 1–3 (cross-cause contract). Universe: the
// three refusal cause classes { EnvelopeAuthFailed, DecodeFailure,
// KekUnavailable }, compared for pairwise distinctness.
#[tokio::test]
async fn serve_refusal_causes_are_pairwise_distinct() {
    // Capture the auth-failure cause (wrong KEK material under the matching id).
    let auth_failed = {
        let dirs = ServeDirs::new();
        boot_attempt(&dirs, correct_kek())
            .await
            .expect("seed root")
            .shutdown(Duration::from_secs(2))
            .await;
        cause_class(&refused_cause(
            boot_attempt(&dirs, wrong_kek_material()).await,
            "wrong-KEK-material",
        ))
    };

    // Capture the decode-malformed cause (structurally-corrupted envelope).
    let decode_malformed = {
        let dirs = ServeDirs::new();
        boot_attempt(&dirs, correct_kek())
            .await
            .expect("seed root")
            .shutdown(Duration::from_secs(2))
            .await;
        corrupt_root_key_envelope_structurally(&dirs).await;
        cause_class(&refused_cause(boot_attempt(&dirs, correct_kek()).await, "corrupted-envelope"))
    };

    // Capture the KEK-unavailable cause (absent KEK).
    let kek_unavailable = {
        let dirs = ServeDirs::new();
        cause_class(&refused_cause(boot_attempt(&dirs, absent_kek()).await, "absent-KEK"))
    };

    // THEN each is the EXPECTED cause class — the system genuinely discriminates
    // these three (ADR-0063 D4), so the operator can triage from the cause alone.
    assert_eq!(
        auth_failed,
        RefusalCauseClass::EnvelopeAuthFailed,
        "wrong-KEK-material must classify as EnvelopeAuthFailed"
    );
    assert_eq!(
        decode_malformed,
        RefusalCauseClass::DecodeFailure,
        "corrupted-envelope must classify as DecodeFailure"
    );
    assert_eq!(
        kek_unavailable,
        RefusalCauseClass::KekUnavailable,
        "absent-KEK must classify as KekUnavailable"
    );

    // AND the three classes are PAIRWISE DISTINCT — the cross-cause contract.
    assert_ne!(
        auth_failed, decode_malformed,
        "auth-failure and decode-malformed must be distinct classes"
    );
    assert_ne!(
        auth_failed, kek_unavailable,
        "auth-failure and KEK-unavailable must be distinct classes"
    );
    assert_ne!(
        decode_malformed, kek_unavailable,
        "decode-malformed and KEK-unavailable must be distinct classes"
    );
}

// S-OC-09 `@integration @real-io @error @driving_port @slice-2 @edd:O04` — a
// boot that REFUSED (wrong KEK material → auth-failure) does NOT silently
// re-mint: re-supplying the correct KEK and starting again adopts the SAME
// original root (byte-identical
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

    // WHEN a boot REFUSES under the wrong KEK material (auth-failure).
    let _ = refused_cause(boot_attempt(&dirs, wrong_kek_material()).await, "wrong-KEK-material");

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

/// STRUCTURALLY corrupt the persisted root-key envelope so the record no longer
/// DESERIALIZES into a valid `RootCaKeyRecord` — truncate the stored rkyv bytes
/// to half their length, then write them back. Truncation reliably breaks the
/// rkyv framing, so a subsequent boot fails at `from_store_bytes` decode
/// (EnvelopeError::Malformed) BEFORE any crypto — the distinct decode/Malformed
/// cause class. (A byte-flip that left the record decodable would instead surface
/// as auth-failure, the SAME class as S-OC-08a — that would not exercise a
/// distinct cause, so the corruption MUST be structural per ADR-0063 D4.)
async fn corrupt_root_key_envelope_structurally(dirs: &ServeDirs) {
    use overdrive_core::traits::intent_store::IntentStore as _;
    let store = open_persisted_intent_store(dirs).await;
    let bytes = store
        .get(ROOT_KEY_ENVELOPE_KEY)
        .await
        .expect("IntentStore get envelope for corruption")
        .expect("the boot persisted a root-key envelope to corrupt")
        .to_vec();
    assert!(bytes.len() >= 2, "envelope must have bytes to truncate");
    // Truncate to half length — guaranteed to break the rkyv archived framing,
    // so decode fails before crypto runs.
    let truncated = &bytes[..bytes.len() / 2];
    store.put(ROOT_KEY_ENVELOPE_KEY, truncated).await.expect("write corrupted envelope back");
}

/// Assert a `run_server` boot result is a typed `ControlPlaneError::CaBoot`
/// refusal (NO `ServerHandle` returned — no serving), returning the **typed**
/// `CaBootError` so callers can match on the cause CLASS (not just the rendered
/// string). `label` names the scenario for the panic message.
fn refused_cause(result: Result<ServerHandle, ControlPlaneError>, label: &str) -> CaBootError {
    match result {
        Ok(_handle) => {
            panic!("{label}: the control plane must REFUSE to start, but run_server returned Ok")
        }
        Err(ControlPlaneError::CaBoot(cause)) => cause,
        Err(other) => panic!(
            "{label}: the refusal must be the typed ControlPlaneError::CaBoot, got {other:?}"
        ),
    }
}

/// The cause CLASS of a refusal — the operator-meaningful taxonomy ADR-0063 D4
/// pins. Distinct classes are what S-OC-08d asserts pairwise-distinct; rendered
/// string inequality is incidental and NOT the contract.
#[derive(Debug, PartialEq, Eq)]
enum RefusalCauseClass {
    /// AES-GCM auth failure under a matching `kek_id` — wrong KEK material OR a
    /// tampered (still-decodable) ciphertext, indistinguishable under AEAD.
    EnvelopeAuthFailed,
    /// The persisted bytes did not deserialize into a valid record — fails at
    /// DECODE, BEFORE any crypto. Both `EnvelopeError::Malformed` (rkyv bytecheck
    /// rejects) and `EnvelopeError::UnknownVersion` (the pre-decode version-tag
    /// probe rejects) are this one operator-meaningful class: "the bytes are not
    /// a decodable record." Which decode variant a given structural corruption
    /// trips is an implementation detail of where the broken bytes land relative
    /// to the rkyv framing — the operator-facing cause is "decode failure."
    DecodeFailure,
    /// The KEK could not be resolved at all — refused before any issuance.
    KekUnavailable,
    /// Any other `CaBootError` shape — surfaced so an unexpected class fails
    /// loud rather than silently collapsing into one of the three above.
    Other,
}

/// Classify a refusal by its typed cause CLASS (ADR-0063 D4 honest taxonomy):
/// `EnvelopeAuthFailed` (auth-failure, wrong material OR tamper) vs
/// `DecodeFailure` (structural decode failure, before crypto) vs
/// `KekUnavailable` (absent KEK). This is a `matches!` on the typed cause, not
/// a string compare.
const fn cause_class(cause: &CaBootError) -> RefusalCauseClass {
    use overdrive_core::traits::intent_store::IntentStoreError;
    match cause {
        CaBootError::KekUnavailable { .. } => RefusalCauseClass::KekUnavailable,
        CaBootError::EnvelopeDecrypt { source: CaError::EnvelopeAuthFailed { .. }, .. } => {
            RefusalCauseClass::EnvelopeAuthFailed
        }
        // A structurally-corrupt envelope fails at decode in
        // `RootCaKeyRecord::from_store_bytes`, which returns
        // `IntentStoreError::Envelope { source: EnvelopeError::{Malformed |
        // UnknownVersion} }` — propagated through `CaBootError::Intent` (the `?`
        // in `load_persistent_root`), BEFORE crypto. Both decode variants are the
        // one operator-meaningful "decode failure" class (see RefusalCauseClass
        // docs); which one a given corruption trips depends on where the broken
        // bytes land relative to the rkyv version-tag, not on operator intent.
        CaBootError::Intent(IntentStoreError::Envelope {
            source: EnvelopeError::Malformed { .. } | EnvelopeError::UnknownVersion { .. },
            ..
        }) => RefusalCauseClass::DecodeFailure,
        _ => RefusalCauseClass::Other,
    }
}
