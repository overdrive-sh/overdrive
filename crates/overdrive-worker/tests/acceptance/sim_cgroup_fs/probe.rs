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
    }
}
