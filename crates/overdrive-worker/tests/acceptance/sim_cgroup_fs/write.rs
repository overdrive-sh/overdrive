//! Class B `write` scenarios for `SimCgroupFs` per ADR-0054
//! § Sim adapter (step 01-03).
//!
//! Includes byte-honesty scenarios proving `SimCgroupFs` does NOT
//! model kernel side effects (process movement, mass-kill) — only
//! byte storage. Non-replacement contract per ADR-0054 § Scope.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_sim::{SimCgroupFs, SimEntry, SimOp};

fn fresh() -> (Arc<SimCgroupFs>, Arc<dyn CgroupFs>) {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();
    (sim, fs)
}

#[tokio::test]
async fn b_write_happy() {
    let (sim, fs) = fresh();
    let parent = Path::new("/sys/fs/cgroup/overdrive.slice");
    let file = parent.join("cgroup.subtree_control");

    fs.create_dir(parent).await.expect("create parent");
    fs.write(&file, b"+cpu +memory\n").await.expect("write Ok");

    let snap = sim.snapshot();
    let (entry, bytes) = snap.get(&file).expect("file persisted");
    assert_eq!(*entry, SimEntry::File);
    assert_eq!(bytes.as_slice(), b"+cpu +memory\n");
}

#[tokio::test]
async fn b_write_empty_bytes() {
    let (sim, fs) = fresh();
    let parent = Path::new("/sys/fs/cgroup/overdrive.slice");
    let file = parent.join("cgroup.subtree_control");

    fs.create_dir(parent).await.expect("create parent");
    fs.write(&file, b"").await.expect("empty write Ok");

    let snap = sim.snapshot();
    let (entry, bytes) = snap.get(&file).expect("file persisted");
    assert_eq!(*entry, SimEntry::File);
    assert!(bytes.is_empty(), "empty payload preserved verbatim");
}

#[tokio::test]
async fn b_write_missing_parent_returns_notfound() {
    let (_sim, fs) = fresh();
    let orphan = Path::new("/sys/fs/cgroup/overdrive.slice/orphan-file");

    let err = fs.write(orphan, b"payload").await.expect_err("missing parent must surface NotFound");
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
}

#[tokio::test]
async fn b_write_injected_other_returns() {
    let (sim, fs) = fresh();
    let parent = Path::new("/sys/fs/cgroup/overdrive.slice");
    let file = parent.join("cgroup.subtree_control");
    fs.create_dir(parent).await.expect("create parent");

    sim.inject_error(SimOp::Write, file.clone(), io::ErrorKind::Other);

    let err = fs.write(&file, b"+cpu").await.expect_err("injected Other fires");
    assert_eq!(err.kind(), io::ErrorKind::Other);

    let snap = sim.snapshot();
    assert!(!snap.contains_key(&file), "injected error must leave state unchanged");
}

#[tokio::test]
async fn b_write_respects_cgroup_procs_pid_payload() {
    // Byte-honesty: writing a PID payload to `cgroup.procs` stores the
    // exact bytes. SimCgroupFs does NOT simulate process movement —
    // that's a kernel-side effect exclusively covered at Tier 3 per
    // ADR-0054 § D3.
    let (sim, fs) = fresh();
    let parent = Path::new("/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-x.scope");
    let procs = parent.join("cgroup.procs");

    fs.create_dir(parent).await.expect("create parent");
    fs.write(&procs, b"12345\n").await.expect("write PID Ok");

    let snap = sim.snapshot();
    let (entry, bytes) = snap.get(&procs).expect("cgroup.procs persisted");
    assert_eq!(*entry, SimEntry::File);
    assert_eq!(
        bytes.as_slice(),
        b"12345\n",
        "PID payload stored byte-for-byte; no process movement simulated"
    );
}

#[tokio::test]
async fn b_write_cgroup_kill_stores_one_newline() {
    // Byte-honesty: writing "1\n" to `cgroup.kill` stores those exact
    // bytes. SimCgroupFs does NOT simulate the kernel's mass-kill
    // primitive — that's Tier 3.
    let (sim, fs) = fresh();
    let parent = Path::new("/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-y.scope");
    let kill = parent.join("cgroup.kill");

    fs.create_dir(parent).await.expect("create parent");
    fs.write(&kill, b"1\n").await.expect("write kill Ok");

    let snap = sim.snapshot();
    let (entry, bytes) = snap.get(&kill).expect("cgroup.kill persisted");
    assert_eq!(*entry, SimEntry::File);
    assert_eq!(
        bytes.as_slice(),
        b"1\n",
        "cgroup.kill stores literal bytes; no mass-kill simulated"
    );
}

#[tokio::test]
async fn b_write_then_snapshot_is_deterministic_across_runs() {
    // Two fresh SimCgroupFs instances given the same write sequence
    // produce bit-identical snapshots. F1 K3 sanity-check at single
    // seed — the full proptest lives in `k3_determinism.rs`.
    let ops: Vec<(PathBuf, Vec<u8>)> = vec![
        (PathBuf::from("/a/b/c"), b"alpha".to_vec()),
        (PathBuf::from("/a/b/d"), b"beta".to_vec()),
        (PathBuf::from("/a/x"), b"gamma".to_vec()),
    ];

    let sim_a = Arc::new(SimCgroupFs::new());
    let sim_b = Arc::new(SimCgroupFs::new());
    let fs_a: Arc<dyn CgroupFs> = sim_a.clone();
    let fs_b: Arc<dyn CgroupFs> = sim_b.clone();

    for (path, bytes) in &ops {
        let parent = path.parent().expect("non-root");
        fs_a.create_dir(parent).await.expect("create a");
        fs_b.create_dir(parent).await.expect("create b");
        fs_a.write(path, bytes).await.expect("write a");
        fs_b.write(path, bytes).await.expect("write b");
    }

    assert_eq!(
        sim_a.snapshot(),
        sim_b.snapshot(),
        "two fresh SimCgroupFs given the same op sequence produce identical snapshots"
    );
}
