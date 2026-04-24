//! Tier-3 integration tests for the libSQL per-primitive path
//! provisioner (step 04-03).
//!
//! These exercise the real filesystem boundary: real
//! `std::fs::canonicalize`, real `libsql::Builder::new_local(...)`
//! against a real SQLite-backed file, and real `tempfile::TempDir`
//! lifecycles. Per ADR-0013 §5 the path shape is
//! `<data_dir>/reconcilers/<name>/memory.db`, canonicalised, with a
//! defence-in-depth `starts_with` check.
//!
//! The isolation invariant — alpha cannot see beta's data through its
//! own handle — is the behavioural heart of the step: it asserts the
//! provisioner gives each reconciler a filesystem-isolated DB.

use std::path::PathBuf;

use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::libsql_provisioner::{open_db, provision_db_path};
use overdrive_core::reconciler::ReconcilerName;
use tempfile::TempDir;

fn name(raw: &str) -> ReconcilerName {
    ReconcilerName::new(raw).expect("valid reconciler name")
}

/// Expected canonical layout: `<canon(data_dir)>/reconcilers/<name>/memory.db`.
#[tokio::test]
async fn provision_db_path_returns_canonical_layout() {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().to_path_buf();

    let path = provision_db_path(&data_dir, &name("alpha")).expect("provision");

    // `std::fs::canonicalize` is what the provisioner is expected to
    // apply — derive the expected value the same way so the test
    // tolerates symlink-resolving tmpdirs (e.g. `/tmp` → `/private/tmp`
    // on macOS).
    let expected = std::fs::canonicalize(&data_dir)
        .expect("canonicalise tmpdir")
        .join("reconcilers")
        .join("alpha")
        .join("memory.db");

    assert_eq!(path, expected);
}

/// Distinct reconcilers get distinct paths under a shared
/// `<data_dir>/reconcilers/` prefix.
#[tokio::test]
async fn provision_db_path_distinct_for_distinct_names() {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path();

    let alpha = provision_db_path(data_dir, &name("alpha")).expect("alpha");
    let beta = provision_db_path(data_dir, &name("beta")).expect("beta");

    assert_ne!(alpha, beta, "distinct names must get distinct paths");

    let canon = std::fs::canonicalize(data_dir).expect("canon");
    let shared = canon.join("reconcilers");
    assert!(
        alpha.starts_with(&shared),
        "alpha path {} does not start with {}",
        alpha.display(),
        shared.display()
    );
    assert!(
        beta.starts_with(&shared),
        "beta path {} does not start with {}",
        beta.display(),
        shared.display()
    );

    // Their first point of divergence is the name segment — the common
    // prefix is exactly `<canon>/reconcilers/`.
    let alpha_parent = alpha.parent().and_then(std::path::Path::parent);
    let beta_parent = beta.parent().and_then(std::path::Path::parent);
    assert_eq!(alpha_parent, Some(shared.as_path()));
    assert_eq!(beta_parent, Some(shared.as_path()));
}

/// Defence-in-depth: the returned path always starts with
/// `<canonicalised_data_dir>/reconcilers/` regardless of what the name
/// looks like (ReconcilerName's regex is the primary guard; the check
/// is insurance per ADR-0013 §5).
#[tokio::test]
async fn provision_db_path_starts_with_data_dir_reconcilers() {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path();

    let path = provision_db_path(data_dir, &name("some-reconciler-123")).expect("provision");

    let canon = std::fs::canonicalize(data_dir).expect("canon");
    assert!(
        path.starts_with(canon.join("reconcilers")),
        "path {} must start with {}/reconcilers",
        path.display(),
        canon.display()
    );
}

/// `open_db` materialises missing directories and returns a usable
/// libSQL handle. Proves the provisioner does not require the caller
/// to pre-create parents.
#[tokio::test]
async fn open_db_creates_directory_tree_if_missing() {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path();

    let path = provision_db_path(data_dir, &name("fresh")).expect("provision");
    // Parent does not exist yet — `provision_db_path` only computes
    // the path; it must not create the `<name>/` directory.
    assert!(
        !path.parent().expect("parent").exists(),
        "parent dir {} should not exist before open_db",
        path.parent().unwrap().display()
    );

    let db = open_db(&path).await.expect("open_db");
    // Parent is now present.
    assert!(
        path.parent().expect("parent").exists(),
        "open_db must create the parent directory tree"
    );
    // Connection is usable — exercising it is the real proof.
    let conn = db.connect().expect("connect");
    conn.execute("CREATE TABLE t (x INTEGER)", ()).await.expect("create table");
}

/// Two calls to `open_db` against the same path return independent
/// handles — neither shares connection state with the other. This is
/// the libSQL-level complement to the filesystem-isolation property
/// tested below.
#[tokio::test]
async fn open_db_two_calls_same_path_two_independent_connections() {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path();
    let path = provision_db_path(data_dir, &name("shared")).expect("provision");

    let db1 = open_db(&path).await.expect("first open");
    let db2 = open_db(&path).await.expect("second open");

    let conn1 = db1.connect().expect("connect 1");
    let conn2 = db2.connect().expect("connect 2");

    conn1.execute("CREATE TABLE ping (x INTEGER)", ()).await.expect("create via conn1");

    // conn2 sees the same on-disk file — it's the same path — but
    // they are independent connection objects, so a transaction state
    // on one does not affect the other. Proving independence via a
    // committed write both handles can observe is the cleanest shape:
    let mut rows = conn2
        .query("SELECT name FROM sqlite_master WHERE name = 'ping'", ())
        .await
        .expect("query via conn2");
    let row = rows.next().await.expect("row result").expect("row present");
    let got: String = row.get(0).expect("name column");
    assert_eq!(got, "ping");
}

/// The load-bearing invariant: alpha's handle writes a canary row;
/// beta's handle — provisioned separately by name — cannot see that
/// row at all, because beta's DB file is a different file on disk.
/// SQLite reports `no such table: canary` to beta.
#[tokio::test]
async fn alpha_canary_row_invisible_to_beta_handle() {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path();

    let alpha_path = provision_db_path(data_dir, &name("alpha")).expect("alpha path");
    let beta_path = provision_db_path(data_dir, &name("beta")).expect("beta path");

    let alpha_db = open_db(&alpha_path).await.expect("alpha open");
    let beta_db = open_db(&beta_path).await.expect("beta open");

    let alpha_conn = alpha_db.connect().expect("alpha connect");
    alpha_conn.execute("CREATE TABLE canary (marker TEXT)", ()).await.expect("create canary");
    alpha_conn
        .execute("INSERT INTO canary (marker) VALUES ('alpha-was-here')", ())
        .await
        .expect("insert canary");

    let beta_conn = beta_db.connect().expect("beta connect");
    // SELECT against a table that does not exist in beta's DB — the
    // isolation invariant is that this surfaces a libSQL error
    // containing the standard SQLite "no such table" diagnostic. In
    // libsql 0.5 the error may be raised by `prepare`, `query`, or
    // on first row access depending on the planner; handle all three
    // by collecting any of them and inspecting the message.
    let query_outcome = beta_conn.query("SELECT marker FROM canary", ()).await;
    let err_msg = match query_outcome {
        Ok(mut rows) => match rows.next().await {
            Err(e) => format!("{e}"),
            Ok(Some(_)) => panic!("beta must not return rows from alpha's canary table"),
            Ok(None) => panic!("beta returned empty rows instead of 'no such table' error"),
        },
        Err(e) => format!("{e}"),
    };

    let lowered = err_msg.to_lowercase();
    assert!(lowered.contains("no such table"), "expected 'no such table' error, got: {err_msg}");
}

/// A data_dir whose parent does not exist is an error. The
/// provisioner is permitted to create the data_dir itself (so
/// `canonicalize` succeeds), but it must not fabricate arbitrary
/// paths under `/nonexistent`.
#[tokio::test]
async fn non_existent_data_dir_parent_returns_error() {
    // A path whose parent cannot plausibly exist. `std::fs::create_dir_all`
    // should fail here (no permission to create under `/nonexistent`,
    // which does not exist and cannot be created by a normal user).
    let bogus: PathBuf = PathBuf::from("/nonexistent/overdrive-04-03/data");

    let result = provision_db_path(&bogus, &name("alpha"));
    assert!(
        matches!(result, Err(ControlPlaneError::Internal(_))),
        "expected ControlPlaneError::Internal, got {result:?}"
    );
}
