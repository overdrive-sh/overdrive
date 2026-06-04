//! D1 — Real and Sim adapters honor the same byte-store contract.
//!
//! Equivalence proptest per ADR-0054 § D5 step 7. Generates a sequence
//! of `Op`s over a bounded path alphabet; applies each op to both
//! `RealCgroupFs` (rooted at a `tempfile::TempDir`) AND `SimCgroupFs`;
//! asserts (a) per-op `Ok`/`Err` verdict matches, (b) on `Err` the
//! `io::ErrorKind` matches, and (c) post-sequence file bytes match.
//!
//! Per `.claude/rules/testing.md` § "Property-based testing (proptest)"
//! the default case count is 1024. Default proptest `#![proptest_config]`
//! is honored — no `with_cases(...)` override here.
//!
//! # LIMITATION (per ADR-0054 § D3 — Non-replacement contract)
//!
//! Validates the **BYTE-STORE** contract only. Kernel-side effects are
//! EXPLICITLY OUT OF SCOPE for this test:
//!
//!   * `cgroup.kill` mass-kill (writing `1\n` does NOT terminate
//!     processes under a tempdir parent — the regular filesystem has
//!     no kernel-side concept of cgroup processes).
//!   * `cgroup.subtree_control` controller-enablement EBUSY (the
//!     kernel cgroup-v2 contract is unique to real cgroupfs; tempdirs
//!     accept arbitrary writes).
//!   * Pseudo-file synthesis (`cgroup.procs`, `cgroup.subtree_control`,
//!     `cgroup.events` appear automatically under real cgroup dirs;
//!     tempdirs do not synthesise them).
//!   * `EINVAL` from malformed controller-list writes.
//!
//! Those scenarios live in Class C of `docs/feature/cgroup-fs-port/
//! distill/test-scenarios.md` and ship in step 01-08 (Tier 3, real
//! `/sys/fs/cgroup` via `cargo xtask lima run --`). `RealCgroupFs` in
//! this test operates against a tempdir root — NOT `/sys/fs/cgroup`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_host::RealCgroupFs;
use overdrive_sim::{SimCgroupFs, SimEntry};
use proptest::collection::vec;
use proptest::prelude::*;

/// Bounded path alphabet. Selected to provide:
///   * shared-prefix paths (forcing parent-exists ordering interactions)
///   * one nested 3-level path (so `remove_dir` can hit
///     `DirectoryNotEmpty`-style conditions on either side)
///   * leaf paths suitable for `write` (the parent of every entry
///     can be created via `CreateDir` of the prefix).
const PATH_POOL: &[&str] = &["a", "a/0", "a/1", "b", "b/0", "c", "c/0", "c/0/inner"];

fn path_at(idx: usize) -> PathBuf {
    PathBuf::from(PATH_POOL[idx % PATH_POOL.len()])
}

/// The op-space: every variant carries an index into `PATH_POOL`.
/// `Write` additionally carries a small byte payload (≤16 bytes).
#[derive(Clone, Debug)]
enum Op {
    CreateDir(usize),
    Write(usize, Vec<u8>),
    RemoveDir(usize),
}

fn op_strategy() -> impl Strategy<Value = Op> {
    let idx = 0usize..PATH_POOL.len();
    prop_oneof![
        idx.clone().prop_map(Op::CreateDir),
        (idx.clone(), vec(any::<u8>(), 0..=16)).prop_map(|(i, b)| Op::Write(i, b)),
        idx.prop_map(Op::RemoveDir),
    ]
}

/// Apply `op` to both adapters and return the two `Result`s as their
/// `Ok(())`-or-`io::ErrorKind` verdicts. The `tokio::Runtime` is shared
/// across ops within a single proptest case (one runtime per case).
///
/// Path mapping:
///   * Real side — `<tempdir>/<rel>`. The tempdir always exists as a
///     directory; `<tempdir>` is the implicit "parent root" against
///     which all operations are scoped.
///   * Sim side — `<rel>` (relative, no leading `/`). Sim's parent
///     check skips when the parent path is the empty string, which
///     matches Real's "parent always exists" baseline since the
///     tempdir IS the de-facto root. Operations on multi-component
///     paths (`a/0`, `c/0/inner`) still exercise sim's parent
///     existence check.
async fn apply_op(
    real_fs: &Arc<dyn CgroupFs>,
    sim_fs: &Arc<dyn CgroupFs>,
    real_root: &Path,
    op: &Op,
) -> (Result<(), std::io::ErrorKind>, Result<(), std::io::ErrorKind>) {
    let kind_map = |r: std::io::Result<()>| r.map_err(|e| e.kind());
    match op {
        Op::CreateDir(i) => {
            let relative = path_at(*i);
            let real_path = real_root.join(&relative);
            (
                kind_map(real_fs.create_dir(&real_path).await),
                kind_map(sim_fs.create_dir(&relative).await),
            )
        }
        Op::Write(i, bytes) => {
            let relative = path_at(*i);
            let real_path = real_root.join(&relative);
            (
                kind_map(real_fs.write(&real_path, bytes).await),
                kind_map(sim_fs.write(&relative, bytes).await),
            )
        }
        Op::RemoveDir(i) => {
            let relative = path_at(*i);
            let real_path = real_root.join(&relative);
            (
                kind_map(real_fs.remove_dir(&real_path).await),
                kind_map(sim_fs.remove_dir(&relative).await),
            )
        }
    }
}

proptest! {
    /// Per-op Ok/Err verdict + ErrorKind agreement between RealCgroupFs
    /// (against a tempdir) and SimCgroupFs (in-memory BTreeMap).
    ///
    /// Post-sequence: for every File entry in SimCgroupFs's snapshot,
    /// the corresponding real-side path exists and its bytes match.
    /// For every Dir entry, the real-side path exists as a directory.
    #[test]
    fn real_and_sim_byte_store_equivalence(ops in vec(op_strategy(), 0..32)) {
        let real_root = tempfile::TempDir::new().expect("tempdir");
        let real_fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
        // SimCgroupFs::clone() shares the underlying Arc<Mutex<...>>
        // state. Keep one concrete handle for snapshot() inspection
        // and erase the other to the trait surface for op dispatch.
        let sim_concrete = SimCgroupFs::new();
        let sim_fs: Arc<dyn CgroupFs> = Arc::new(sim_concrete.clone());

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        for op in &ops {
            let (real_verdict, sim_verdict) =
                rt.block_on(apply_op(&real_fs, &sim_fs, real_root.path(), op));
            prop_assert_eq!(
                real_verdict.is_ok(),
                sim_verdict.is_ok(),
                "Ok/Err disagreement on op {:?}: real={:?}, sim={:?}",
                op,
                real_verdict,
                sim_verdict,
            );
            if let (Err(real_kind), Err(sim_kind)) = (&real_verdict, &sim_verdict) {
                prop_assert_eq!(
                    real_kind,
                    sim_kind,
                    "ErrorKind disagreement on op {:?}: real={:?}, sim={:?}",
                    op,
                    real_kind,
                    sim_kind,
                );
            }
        }

        // Post-sequence snapshot equivalence. For every File entry on
        // the sim side, the real-side path must exist with byte-equal
        // contents. For every Dir entry, the real-side path must exist
        // as a directory.
        let snap = sim_concrete.snapshot();
        rt.block_on(async {
            for (sim_path, (entry, bytes)) in &snap {
                // Sim paths are relative — no leading `/`. Real paths
                // join the relative onto the tempdir.
                let real_path = real_root.path().join(sim_path);
                match entry {
                    SimEntry::File => {
                        let real_bytes = tokio::fs::read(&real_path)
                            .await
                            .map_err(|e| TestCaseError::fail(
                                format!("real-side read failed at {real_path:?}: {e}")
                            ))?;
                        prop_assert_eq!(
                            &real_bytes,
                            bytes,
                            "byte mismatch at {:?}",
                            sim_path,
                        );
                    }
                    SimEntry::Dir => {
                        let meta = tokio::fs::metadata(&real_path)
                            .await
                            .map_err(|e| TestCaseError::fail(
                                format!("real-side metadata failed at {real_path:?}: {e}")
                            ))?;
                        prop_assert!(
                            meta.is_dir(),
                            "real-side path {:?} expected to be a directory",
                            real_path,
                        );
                    }
                }
            }
            Ok::<_, TestCaseError>(())
        })?;

        // Inverse direction: for every regular file the real adapter
        // wrote, the sim must agree on its bytes. (Dir-only entries
        // are skipped — `tokio::fs::walk_dir`-style traversal is more
        // expensive than the value at the default-case-count budget.)
        let real_files = rt.block_on(async {
            let mut acc: BTreeSet<PathBuf> = BTreeSet::new();
            collect_files(real_root.path(), real_root.path(), &mut acc).await;
            acc
        });
        for relative in &real_files {
            let real_path = real_root.path().join(relative);
            // Sim keys are stored as the relative path verbatim.
            let real_bytes = rt.block_on(tokio::fs::read(&real_path))
                .map_err(|e| TestCaseError::fail(
                    format!("real-side read failed at {real_path:?}: {e}")
                ))?;
            let entry = snap.get(relative);
            prop_assert!(
                entry.is_some(),
                "real has file at {:?} but sim has no entry",
                relative,
            );
            let (sim_entry, sim_bytes) = entry.expect("checked above");
            prop_assert_eq!(
                *sim_entry,
                SimEntry::File,
                "real has file at {:?} but sim entry is {:?}",
                relative,
                sim_entry,
            );
            prop_assert_eq!(
                &real_bytes,
                sim_bytes,
                "byte mismatch at {:?} (real → sim)",
                relative,
            );
        }
    }
}

/// Recursively collect every regular-file path under `dir` as a path
/// RELATIVE to `base`. Async by way of `tokio::fs::read_dir`.
fn collect_files<'a>(
    base: &'a Path,
    dir: &'a Path,
    acc: &'a mut BTreeSet<PathBuf>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        let Ok(mut rd) = tokio::fs::read_dir(dir).await else { return };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let p = entry.path();
            let Ok(meta) = tokio::fs::metadata(&p).await else { continue };
            if meta.is_dir() {
                collect_files(base, &p, acc).await;
            } else if meta.is_file()
                && let Ok(relative) = p.strip_prefix(base)
            {
                acc.insert(relative.to_path_buf());
            }
        }
    })
}
