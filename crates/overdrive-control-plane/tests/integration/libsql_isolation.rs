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
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_control_plane::{job_lifecycle, noop_heartbeat};
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
/// looks like (`ReconcilerName`'s regex is the primary guard; the check
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
/// `SQLite` reports `no such table: canary` to beta.
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

/// A `data_dir` whose parent does not exist is an error. The
/// provisioner is permitted to create the `data_dir` itself (so
/// `canonicalize` succeeds), but it must not fabricate arbitrary
/// paths under `/nonexistent`.
///
/// Ignored: the test premise is "a normal user cannot create
/// `/nonexistent/...`", which is false under root. The Lima dev
/// VM and Tier-3 CI both run integration tests with `sudo` so
/// cgroup writes succeed; in that context `create_dir_all` happily
/// fabricates the path and the assertion fires. Pre-existing test
/// from step 04-03 (origin/main); kept marked but not executed.
#[tokio::test]
#[ignore = "fails under root (Lima sudo / Tier-3 CI); premise assumes unprivileged user"]
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

// ---------------------------------------------------------------------------
// Step 01-02 — eager LibsqlHandle::open at register time
// ---------------------------------------------------------------------------

/// `ReconcilerRuntime::register` opens a real `LibsqlHandle` eagerly via
/// the `libsql_provisioner`-derived path; the registry exposes the
/// resulting handle to downstream callers (handlers.rs, the convergence
/// tick loop, `exit_observer`) via `libsql_handle(&ReconcilerName)`.
///
/// Positive path: a fresh `data_dir`, register `noop-heartbeat`, then
/// fetch the handle by name and exercise it through `connection()`. The
/// connection MUST be usable — proving the eager-open went all the way
/// through `Builder::new_local(path).build()`. A registry that returned
/// `None` here would silently regress the §18 hydrate contract: the
/// runtime would re-open per tick, drift from "single connection per
/// reconciler", and miss the failure-at-boot guarantee.
#[tokio::test]
async fn register_exposes_libsql_handle_via_accessor() {
    let tmp = TempDir::new().expect("tempdir");
    let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("new runtime");

    runtime.register(noop_heartbeat()).await.expect("register noop");

    let noop_name = name("noop-heartbeat");
    let handle = runtime.libsql_handle(&noop_name).expect("handle for noop");

    // Exercise the connection — `CREATE TABLE` proves the underlying
    // file-backed connection is usable, distinguishing a real
    // `LibsqlHandle::open(path)` from any future regression that
    // accidentally returned a stale or in-memory shortcut.
    let conn = handle.connection();
    conn.execute("CREATE TABLE step_01_02 (x INTEGER)", ())
        .await
        .expect("create table on accessor-returned handle");

    // Unregistered name → None.
    let unknown = name("not-registered");
    assert!(runtime.libsql_handle(&unknown).is_none(), "unregistered name must return None");
}

/// Both Phase 1 reconcilers (`noop-heartbeat`, `job-lifecycle`) get
/// independent libSQL handles at register time. Pins that the registry
/// stores one handle per reconciler — not a shared singleton — and the
/// per-reconciler isolation invariant from `provision_db_path` carries
/// through eager `LibsqlHandle::open`.
#[tokio::test]
async fn register_opens_distinct_handles_per_reconciler() {
    let tmp = TempDir::new().expect("tempdir");
    let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("new runtime");

    runtime.register(noop_heartbeat()).await.expect("register noop");
    runtime.register(job_lifecycle()).await.expect("register job");

    let noop = runtime.libsql_handle(&name("noop-heartbeat")).expect("handle for noop");
    let job = runtime.libsql_handle(&name("job-lifecycle")).expect("handle for job");

    // Per-reconciler isolation: a table created on `noop`'s connection
    // is invisible to `job`'s connection, because they sit on different
    // on-disk DB files.
    noop.connection()
        .execute("CREATE TABLE noop_only (x INTEGER)", ())
        .await
        .expect("create on noop");

    let result = job.connection().query("SELECT x FROM noop_only", ()).await;
    let err_msg = match result {
        Ok(mut rows) => match rows.next().await {
            Err(e) => format!("{e}"),
            Ok(Some(_) | None) => {
                panic!("job-lifecycle handle must not see noop-heartbeat's table");
            }
        },
        Err(e) => format!("{e}"),
    };
    assert!(
        err_msg.to_lowercase().contains("no such table"),
        "expected 'no such table', got: {err_msg}",
    );
}

/// Eager-open failure at register time bubbles as
/// `ControlPlaneError::Internal`, NOT deferred to first tick (per
/// research §9.2 failure modes).
///
/// We force the failure by pre-creating a *directory* at the exact
/// path `LibsqlHandle::open` will target: `<data_dir>/reconcilers/
/// noop-heartbeat/memory.db`. libSQL's `Builder::new_local(path).build()`
/// then cannot create / open `memory.db` — the path already exists as
/// a directory, not a file. This survives running as root (Lima dev
/// VM, Tier-3 CI) where filesystem permission tricks (chmod 0o555)
/// silently no-op.
///
/// Asserting on `ControlPlaneError::Internal` rather than a more
/// specific variant matches today's `ControlPlaneError` enum (no
/// dedicated `Libsql` variant) and the `Internal` constructor is the
/// idiomatic surface for "library X failed during boot".
#[tokio::test]
async fn register_fails_internally_when_libsql_open_rejects_path() {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path();

    // Pre-create a directory at the expected memory.db location. This
    // makes `Builder::new_local(...).build()` fail because the path is
    // not a file; libsql cannot open it.
    let canon = std::fs::canonicalize(data_dir).expect("canon");
    let conflicting_dir = canon.join("reconcilers").join("noop-heartbeat").join("memory.db");
    std::fs::create_dir_all(&conflicting_dir).expect("create blocking directory");
    assert!(conflicting_dir.is_dir(), "blocking path must be a dir, not a file");

    let mut runtime = ReconcilerRuntime::new(data_dir).expect("new runtime");
    let result = runtime.register(noop_heartbeat()).await;

    assert!(
        matches!(result, Err(ControlPlaneError::Internal(_))),
        "expected ControlPlaneError::Internal from libsql open failure, got {result:?}",
    );

    // The failed register must not leak into the registry — a future
    // recovery attempt (or a same-name retry under different conditions)
    // sees an empty slot.
    assert!(
        runtime.libsql_handle(&name("noop-heartbeat")).is_none(),
        "failed register must not insert a handle into the registry",
    );
    assert!(
        !runtime.registered().contains(&name("noop-heartbeat")),
        "failed register must not list the reconciler in registered()",
    );
}
