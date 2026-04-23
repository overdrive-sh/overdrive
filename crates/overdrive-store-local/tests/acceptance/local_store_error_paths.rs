//! Acceptance scenarios for US-03 §4.3 — `LocalStore` error boundaries.
//!
//! Translates `docs/feature/phase-1-foundation/distill/test-scenarios.md`
//! §4.3 (corrupted-snapshot / disk-write-failure) into Rust
//! `#[tokio::test]` bodies. The `@us-03 @us-04` §4.4 type-level
//! separation scenario lives alongside the `overdrive-core` trybuild
//! harness — this module covers only the runtime error-path scenarios.
//!
//! Port-to-port discipline: every assertion drives the `IntentStore`
//! trait surface that `LocalStore` implements. Corruption is injected
//! at the byte-slice layer (the only surface `bootstrap_from` consumes);
//! the failed-write case injects failure by mutating filesystem
//! permissions on the backing directory. No internal types are
//! inspected.
//!
//! Strategy C per DWD-01: real redb, `tempfile::TempDir` backing path.
//!
//! `#[cfg(unix)]` gate: the read-only-directory scenario chmods the
//! backing directory to `0o555`. Windows does not model POSIX permission
//! bits in a way `std::fs::set_permissions` exercises, so the scenario
//! is gated to Unix per the roadmap note.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]
// `println!` appears once in the read-only-directory skip-path to surface
// a "filesystem ignored chmod" diagnostic through `cargo test --nocapture`.
// The workspace denies `print_stdout` in production code; this file is
// `tests/`-only and compiled under `cfg(test)`.
#![allow(clippy::print_stdout)]

use bytes::Bytes;
use overdrive_core::traits::intent_store::{IntentStore, IntentStoreError, StateSnapshot};
use overdrive_store_local::{LocalStore, snapshot_frame};
use tempfile::TempDir;

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Build a source `LocalStore` populated with a small known entry set
/// and return its export. The entries are deliberately non-trivial so
/// the payload has enough bytes for the bit-flip scenario to hit rkyv
/// structure (not just a pad byte).
async fn build_populated_snapshot() -> StateSnapshot {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalStore::open(tmp.path().join("intent.redb")).expect("open src");
    store.put(b"jobs/payments", b"spec-payments-v1").await.expect("put payments");
    store.put(b"jobs/auth", b"spec-auth-v1").await.expect("put auth");
    store.put(b"jobs/frontend", b"spec-frontend-v1").await.expect("put frontend");
    store.export_snapshot().await.expect("export snapshot")
}

/// Open a fresh target `LocalStore` on a new `TempDir` and return
/// `(TempDir, LocalStore)`. The `TempDir` is returned so the caller
/// keeps the backing directory alive for the duration of the test.
fn fresh_target() -> (TempDir, LocalStore) {
    let tmp = TempDir::new().expect("target temp dir");
    let store = LocalStore::open(tmp.path().join("intent.redb")).expect("open target");
    (tmp, store)
}

/// Build a snapshot whose canonical byte slice is byte-for-byte empty —
/// what a freshly-opened store exports if `export_snapshot` produced no
/// header at all. Used to compare against the target store's export after
/// a failed `bootstrap_from`. The real assertion is structural: the
/// target's post-failure export must equal the export it would have
/// produced had `bootstrap_from` never been called.
async fn fresh_target_reference_bytes() -> Vec<u8> {
    let (_tmp, store) = fresh_target();
    store.export_snapshot().await.expect("export fresh target").bytes().to_vec()
}

// -----------------------------------------------------------------------------
// §4.3 — Truncated snapshot
// -----------------------------------------------------------------------------

#[tokio::test]
async fn bootstrapping_from_a_truncated_snapshot_fails_without_writing_state() {
    // Given a valid snapshot whose bytes have been truncated by one byte.
    let snap = build_populated_snapshot().await;
    let mut truncated_bytes = snap.bytes().to_vec();
    let original_len = truncated_bytes.len();
    assert!(original_len > snapshot_frame::HEADER_LEN, "precondition: snapshot has a payload");
    truncated_bytes.pop();
    let truncated = StateSnapshot::from_parts(snap.version, snap.entries.clone(), truncated_bytes);

    // When Ana bootstraps a freshly constructed LocalStore from the
    // truncated bytes.
    let (_target_tmp, target) = fresh_target();
    let reference_empty = fresh_target_reference_bytes().await;
    let outcome = target.bootstrap_from(truncated).await;

    // Then Ana receives a snapshot-corrupt error whose `offset` names the
    // byte position where corruption was detected.
    let err = outcome.expect_err("truncated bootstrap must fail");
    match err {
        IntentStoreError::SnapshotCorrupt { offset } => {
            assert!(
                offset >= snapshot_frame::HEADER_LEN,
                "truncation within the payload; offset must be at or after the header boundary \
                 (got {offset}, header = {})",
                snapshot_frame::HEADER_LEN,
            );
            assert!(
                offset <= original_len,
                "offset must lie within the snapshot byte range (got {offset}, original_len = \
                 {original_len})",
            );
        }
        other => panic!("expected SnapshotCorrupt, got {other:?}"),
    }

    // And exporting the target store produces an empty snapshot — the
    // target must look exactly as if bootstrap_from had never been
    // invoked.
    let post_failure = target.export_snapshot().await.expect("export after failed bootstrap");
    assert_eq!(
        post_failure.bytes(),
        reference_empty.as_slice(),
        "target must be byte-identical to a fresh, never-bootstrapped store",
    );
    assert!(post_failure.entries.is_empty(), "no entries must have been written");
}

// -----------------------------------------------------------------------------
// §4.3 — Single-bit-flipped payload
// -----------------------------------------------------------------------------

#[tokio::test]
async fn bootstrapping_from_a_bit_flipped_snapshot_fails_without_writing_state() {
    // Given a valid snapshot whose bytes have one bit flipped inside the
    // payload (not the header — the magic/version path is exercised by
    // the snapshot-frame unit tests and must not be confused with
    // payload-corruption scenarios).
    //
    // rkyv lays its root pointer + length at the *tail* of the archive;
    // that word controls how the reader interprets the rest of the
    // buffer, so a bit flip there is guaranteed to trip `bytecheck`
    // validation. A flip in the middle of a Vec<u8> payload byte can
    // land on a valid byte value that passes validation (bytecheck only
    // rejects *malformed* shapes, not perturbed payload bytes), so we
    // do not use a mid-payload flip as the canonical corruption seed.
    let snap = build_populated_snapshot().await;
    let mut flipped = snap.bytes().to_vec();
    // Flip a high-order bit in the last byte of the frame — that byte
    // is always part of the rkyv archive's trailing length/pointer
    // word, and a single flip there reliably trips `bytecheck`.
    let target_index = flipped.len() - 1;
    assert!(
        target_index >= snapshot_frame::HEADER_LEN,
        "precondition: flip target lies inside the rkyv payload",
    );
    flipped[target_index] ^= 0b1000_0000;
    let flipped_snap = StateSnapshot::from_parts(snap.version, snap.entries.clone(), flipped);

    // When Ana bootstraps a freshly constructed LocalStore from the
    // corrupted bytes.
    let (_target_tmp, target) = fresh_target();
    let reference_empty = fresh_target_reference_bytes().await;
    let outcome = target.bootstrap_from(flipped_snap).await;

    // Then Ana receives a snapshot-corrupt error.
    let err = outcome.expect_err("bit-flipped bootstrap must fail");
    match err {
        IntentStoreError::SnapshotCorrupt { offset } => {
            assert!(
                offset >= snapshot_frame::HEADER_LEN,
                "corruption is in the payload; offset must be at or after the header boundary \
                 (got {offset}, header = {})",
                snapshot_frame::HEADER_LEN,
            );
        }
        other => panic!("expected SnapshotCorrupt, got {other:?}"),
    }

    // And exporting the target store produces an empty snapshot — no
    // partial write leaked from the failed decode path.
    let post_failure = target.export_snapshot().await.expect("export after failed bootstrap");
    assert_eq!(
        post_failure.bytes(),
        reference_empty.as_slice(),
        "target must be byte-identical to a fresh, never-bootstrapped store",
    );
    assert!(post_failure.entries.is_empty(), "no entries must have been written");
}

// -----------------------------------------------------------------------------
// §4.3 — Read-only backing directory surfaces a typed I/O error
// -----------------------------------------------------------------------------

#[cfg(unix)]
#[tokio::test]
async fn a_put_against_a_read_only_backing_directory_surfaces_a_typed_io_error() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    // Given a LocalStore whose backing directory has been made read-only
    // after open. Opening first is deliberate — the roadmap note prefers
    // injecting a *known* failure mode rather than enumerating every
    // reason a put could fail. A chmod to `0o555` on the parent directory
    // blocks redb's per-write temp-file machinery and every other put
    // path without corrupting the already-opened file handle.
    let tmp = TempDir::new().expect("temp dir");
    let db_path = tmp.path().join("intent.redb");
    let store = LocalStore::open(&db_path).expect("open before chmod");

    // Pre-seed one value so a successful put has already been observed
    // through the same LocalStore instance. The post-failure read
    // assertion below then has a meaningful reference value: if a
    // partial write DID leak, `get` would return the attempted-but-failed
    // bytes instead.
    store.put(b"seed/key", b"seed-value").await.expect("seed put");

    // Chmod the directory to r-xr-xr-x. Guard the chmod behind a
    // reset-on-drop so even a panicking assertion returns the TempDir to
    // a permission set `tempfile` can clean up — otherwise the test
    // leaves a non-deletable directory on the filesystem.
    let dir_path = tmp.path().to_path_buf();
    let original_mode = fs::metadata(&dir_path).expect("metadata").permissions().mode();
    fs::set_permissions(&dir_path, fs::Permissions::from_mode(0o555)).expect("chmod ro");
    let _restore = RestorePermissions { path: dir_path.clone(), mode: original_mode };

    // When Ana writes a value under any key.
    let outcome = store.put(b"write/attempt", b"attempted-value").await;

    // Then Ana receives an intent-store I/O error.
    match outcome {
        Err(IntentStoreError::Io(_)) => {}
        Err(other) => panic!("expected IntentStoreError::Io, got {other:?}"),
        Ok(()) => {
            // The platform / filesystem permitted the write despite the
            // chmod (some tmpfs configurations honour `root` override).
            // Restore perms and skip the assertion set rather than flake.
            fs::set_permissions(&dir_path, fs::Permissions::from_mode(original_mode))
                .expect("restore for tmpfs-skip");
            // Test harness intentionally uses `println!` here to surface
            // a skipped-scenario note through `cargo test --nocapture`;
            // `eprintln!` is the banned surface in production code per
            // the workspace lints, but this file is compiled only in
            // the `test` cfg and will never run on an operator node.
            println!(
                "warning: filesystem ignored chmod 0o555 on tempdir; skipping read-only \
                 assertion on this runner"
            );
            return;
        }
    }

    // And no partial value is persisted — the read-after-failure returns
    // `None` for the attempted key, and the pre-seeded value is still
    // intact.
    //
    // Restore permissions first so the read can open its read transaction;
    // redb holds the database handle open from before the chmod, but
    // subsequent reads may need to touch the directory.
    fs::set_permissions(&dir_path, fs::Permissions::from_mode(original_mode)).expect("restore");

    let read_attempt = store.get(b"write/attempt").await.expect("get after failed put");
    assert_eq!(read_attempt, None, "failed put must not leave a partial value");

    let read_seed = store.get(b"seed/key").await.expect("get seed after failed put");
    assert_eq!(
        read_seed,
        Some(Bytes::copy_from_slice(b"seed-value")),
        "pre-existing state must be untouched by a failed put"
    );
}

/// Guard that resets a directory's mode on drop. Used by the
/// read-only-directory test so a panic partway through the assertions
/// does not leave a non-deletable `TempDir` on the filesystem.
#[cfg(unix)]
struct RestorePermissions {
    path: std::path::PathBuf,
    mode: u32,
}

#[cfg(unix)]
impl Drop for RestorePermissions {
    fn drop(&mut self) {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        // Best-effort — if this fails the `TempDir`'s own Drop will also
        // fail, which surfaces the problem to the operator.
        let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
    }
}
