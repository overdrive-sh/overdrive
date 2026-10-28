//! `RedbViewStore` integration tests — real fs round-trip per ADR-0035
//! § Earned Trust + §4.
//!
//! These scenarios exercise the production adapter against a real redb
//! file backed by a `tempfile::TempDir`. They MUST run on Lima on macOS
//! per `.claude/rules/testing.md` § "Running tests on macOS — Lima VM"
//! because the underlying `Database::create` path goes through real
//! `pwrite`/`fsync`. The integration-tests entrypoint already gates the
//! whole binary; no per-file `#[cfg]` needed.

use std::collections::BTreeMap;

use overdrive_control_plane::view_store::redb::RedbViewStore;
use overdrive_control_plane::view_store::{ProbeError, ViewStore, ViewStoreExt};
use overdrive_core::reconcilers::TargetResource;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct DemoView {
    counter: u64,
    label: String,
}

/// Reconciler names as `&'static str` literals — the `ViewStore` byte
/// surface requires `&'static` per the
/// `refactor-reconciler-static-name` RCA.
const N_JOB: &str = "workload-lifecycle";
const N_NODE: &str = "node-drainer";

fn target(s: &str) -> TargetResource {
    TargetResource::new(s).expect("valid target resource")
}

/// Headline scenario for AC#1, #2, #3, #4, #6: write a view, drop the
/// store (fsync+close), reopen against the same path, `bulk_load`
/// returns the written view byte-equal across reopen.
#[tokio::test]
async fn redb_view_store_roundtrips_views_across_reopens_with_durable_fsync() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let t = target("workload/payments");
    let v = DemoView { counter: 7, label: "first".into() };

    {
        let store = RedbViewStore::open(tmp.path()).expect("open store");
        store.write_through(N_JOB, &t, &v).await.expect("durable write");
        // Drop the store to release the redb file lock and ensure
        // commit fsync hit disk before reopen.
    }

    let store = RedbViewStore::open(tmp.path()).expect("reopen store");
    let loaded: BTreeMap<TargetResource, DemoView> =
        store.bulk_load(N_JOB).await.expect("bulk_load after reopen");
    assert_eq!(loaded.get(&t), Some(&v), "view must round-trip byte-equal across reopens");
}

/// AC#6 explicit — same shape, single explicit close/reopen cycle.
#[tokio::test]
async fn redb_view_store_persists_across_reopen() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let t = target("node/n-1");
    let v = DemoView { counter: 99, label: "persist".into() };

    {
        let store = RedbViewStore::open(tmp.path()).expect("open store");
        store.write_through(N_NODE, &t, &v).await.expect("write ok");
    }

    let store = RedbViewStore::open(tmp.path()).expect("reopen store");
    let loaded: BTreeMap<TargetResource, DemoView> =
        store.bulk_load(N_NODE).await.expect("read after reopen");
    assert_eq!(loaded.get(&t), Some(&v));
}

/// AC#7: writing under reconciler A's table does not surface in
/// `bulk_load(B)`.
#[tokio::test]
async fn redb_view_store_per_reconciler_table_isolation() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store = RedbViewStore::open(tmp.path()).expect("open");

    let t = target("workload/payments");
    let v_a = DemoView { counter: 1, label: "a".into() };

    store.write_through(N_JOB, &t, &v_a).await.expect("write a");

    let loaded_b: BTreeMap<TargetResource, DemoView> =
        store.bulk_load(N_NODE).await.expect("read b");
    assert!(
        loaded_b.is_empty(),
        "reconciler B's table must not see reconciler A's rows: {loaded_b:?}"
    );

    let loaded_a: BTreeMap<TargetResource, DemoView> =
        store.bulk_load(N_JOB).await.expect("read a");
    assert_eq!(loaded_a.get(&t), Some(&v_a));
}

/// AC#5 (healthy fs): `probe()` succeeds; leaves no observable residue.
#[tokio::test]
async fn redb_view_store_probe_succeeds_on_healthy_fs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store = RedbViewStore::open(tmp.path()).expect("open");

    store.probe().await.expect("probe ok on healthy fs");

    // No reconciler should have residual rows after probe — probe
    // writes to a dedicated `__probe__` table that is invisible to
    // `bulk_load(name)` for any user reconciler name.
    let loaded: BTreeMap<TargetResource, Vec<u8>> =
        store.bulk_load_bytes_for_test(N_JOB).await.expect("bulk_load_bytes");
    assert!(loaded.is_empty(), "probe must leave no rows under user reconcilers");
}

/// AC#5 (read-only fs): `probe()` returns `ProbeError::WriteFailed` when
/// the underlying storage is read-only. We construct the store on a writable
/// dir, close it, then make both the existing database file and its directory
/// read-only before reopening, so the next open or write transaction fails.
///
/// **Skipped under root** because root bypasses DAC permission bits;
/// `chmod 0o500` cannot model a read-only fs from root's perspective
/// — the kernel happily lets root write to a 0o500 dir. The Lima
/// integration runner defaults to root (per `cargo xtask lima run`'s
/// sudo wrapper) so this assertion is genuine only when the runner is
/// unprivileged. Non-root developer laptops on Linux exercise the
/// assertion; CI under LVH (also unprivileged) will too. The
/// alternative — a `tmpfs -o size=4k` disk-full simulation — was
/// deferred to a follow-up unit-test mock per upstream-issues.md.
#[cfg(unix)]
#[tokio::test]
async fn redb_view_store_probe_fails_on_readonly_fs() {
    use std::os::unix::fs::PermissionsExt;

    // SAFETY: `geteuid` is a pure read, no preconditions. Cannot fail.
    let euid = unsafe { libc::geteuid() };
    if euid == 0 {
        // Skipping: running as root; DAC bypass makes the readonly
        // assertion unfalsifiable. See test docstring above.
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");

    // Open once on a writable dir to materialise the redb file, then
    // close so the file lock releases.
    {
        let _store = RedbViewStore::open(tmp.path()).expect("open writable");
    }

    // Make both the existing database file and its directory read-only.
    // Directory-only chmod is not a read-only-filesystem model: an existing
    // writable file can still be reopened and updated. The probe contract is
    // about a storage surface that refuses writes, so pin both permission
    // checks before reopening. 0o400 = r--------; 0o500 = r-x------.
    let db_path = RedbViewStore::resolve_path(tmp.path());
    let dir_meta = std::fs::metadata(tmp.path()).expect("stat tempdir");
    let file_meta = std::fs::metadata(&db_path).expect("stat redb file");
    let original_dir_permissions = dir_meta.permissions();
    let original_file_permissions = file_meta.permissions();

    let mut file_readonly = original_file_permissions.clone();
    file_readonly.set_mode(0o400);
    std::fs::set_permissions(&db_path, file_readonly).expect("chmod redb file ro");
    let mut readonly = dir_meta.permissions();
    readonly.set_mode(0o500);
    std::fs::set_permissions(tmp.path(), readonly).expect("chmod ro");

    // Reopen on the read-only dir — open itself may succeed (the file
    // already exists and we can read it), but probe must fail because
    // the write transaction or commit cannot create the lock file
    // and/or rewrite the file metadata.
    //
    // If even open fails, that is also acceptable evidence the
    // adapter refuses to operate against a read-only fs. The probe
    // contract only needs a `ProbeError::WriteFailed` somewhere on the
    // boot path; a hard refusal at open is just a stronger guarantee.
    let result = RedbViewStore::open(tmp.path()).map(|store| async move { store.probe().await });

    let probe_outcome = match result {
        Ok(fut) => fut.await,
        Err(_open_err) => {
            // Restore the original permissions so tempdir can clean up, then
            // accept the open-side refusal as satisfying the AC.
            let _ = std::fs::set_permissions(tmp.path(), original_dir_permissions.clone());
            let _ = std::fs::set_permissions(&db_path, original_file_permissions.clone());
            return;
        }
    };

    // Restore permissions for tempdir cleanup regardless of outcome.
    std::fs::set_permissions(tmp.path(), original_dir_permissions)
        .expect("restore tempdir permissions");
    std::fs::set_permissions(&db_path, original_file_permissions)
        .expect("restore redb permissions");

    assert!(
        matches!(probe_outcome, Err(ProbeError::WriteFailed { .. })),
        "expected ProbeError::WriteFailed on read-only fs, got {probe_outcome:?}"
    );
}
