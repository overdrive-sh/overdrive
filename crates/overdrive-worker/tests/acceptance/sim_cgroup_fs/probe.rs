//! Class B `probe` scenarios for `SimCgroupFs` per ADR-0054
//! § Sim adapter (step 01-03).

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use overdrive_core::traits::{CgroupFs, ProbeError};
use overdrive_sim::{SimCgroupFs, SimOp};

fn fresh() -> (Arc<SimCgroupFs>, Arc<dyn CgroupFs>) {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();
    (sim, fs)
}

#[tokio::test]
async fn b_probe_happy() {
    let (_sim, fs) = fresh();
    fs.probe().await.expect("structural probe succeeds");
}

#[tokio::test]
async fn b_probe_leaves_state_empty() {
    let (sim, fs) = fresh();
    fs.probe().await.expect("probe succeeds");
    let snapshot = sim.snapshot();
    assert!(
        snapshot.is_empty(),
        "probe postcondition: all scratch artifacts removed, but state contains: {snapshot:?}"
    );
}

#[tokio::test]
async fn b_probe_injected_substrate_error() {
    let (sim, fs) = fresh();
    sim.inject_error(
        SimOp::Probe,
        PathBuf::from("/sim-probe-root"),
        io::ErrorKind::PermissionDenied,
    );

    let err = fs.probe().await.expect_err("injected substrate error fires");
    match err {
        ProbeError::Substrate { source } => {
            assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
        }
        ProbeError::RoundTripMismatch { .. } => {
            panic!("expected ProbeError::Substrate, got RoundTripMismatch")
        }
        ProbeError::SubstrateCorrupt { .. } => {
            panic!("expected ProbeError::Substrate, got SubstrateCorrupt")
        }
    }
}

#[tokio::test]
async fn b_probe_round_trip_mismatch() {
    let (sim, fs) = fresh();
    sim.inject_round_trip_mismatch();

    let err = fs.probe().await.expect_err("round-trip mismatch fires");
    match err {
        ProbeError::RoundTripMismatch { wrote, read } => {
            assert_eq!(wrote, b"probe\n".to_vec());
            assert_ne!(read, wrote, "read bytes must differ from written");
        }
        ProbeError::Substrate { .. } => {
            panic!("expected ProbeError::RoundTripMismatch, got Substrate")
        }
        ProbeError::SubstrateCorrupt { .. } => {
            panic!("expected ProbeError::RoundTripMismatch, got SubstrateCorrupt")
        }
    }
}

#[tokio::test]
async fn b_probe_write_error_cleans_up_probe_root() {
    let (sim, fs) = fresh();
    sim.inject_error(
        SimOp::Write,
        PathBuf::from("/sim-probe-root/probe-file"),
        io::ErrorKind::PermissionDenied,
    );

    let err = fs.probe().await.expect_err("write-step error fires");
    match err {
        ProbeError::Substrate { source } => {
            assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
        }
        other => panic!("expected ProbeError::Substrate, got {other:?}"),
    }

    assert!(sim.snapshot().is_empty(), "probe root must be cleaned up after write-step failure");
}

#[tokio::test]
async fn b_probe_create_dir_error_preserves_round_trip_mismatch_flag() {
    let (sim, fs) = fresh();

    // Inject both: a CreateDir failure AND a round-trip mismatch.
    sim.inject_error(
        SimOp::CreateDir,
        PathBuf::from("/sim-probe-root"),
        io::ErrorKind::PermissionDenied,
    );
    sim.inject_round_trip_mismatch();

    // First probe: CreateDir fails → ProbeError::Substrate.
    let err = fs.probe().await.expect_err("create_dir error fires");
    assert!(matches!(err, ProbeError::Substrate { .. }), "expected Substrate, got {err:?}");

    // Second probe: the mismatch flag must NOT have been consumed by
    // the first call — it should fire now.
    let err = fs.probe().await.expect_err("round-trip mismatch fires on retry");
    assert!(
        matches!(err, ProbeError::RoundTripMismatch { .. }),
        "expected RoundTripMismatch, got {err:?}",
    );
}
