//! Family `guest-flow-forwarding` (DDD-8): the `ovd_fwd` forwarder's data path under a flow the
//! guest flow owner paired, as `overdrive-init` runs it in a workload VM.
//!
//! Each replicate boots one fresh guest through the guest kit (the production `overdrive-init` as
//! PID 1) and runs the guest peer's `forwarding-bench` subcommand in it. The program pairs one
//! flow through the owner's vsock wire seam, drives unrecorded and timed 64-byte closed-loop round
//! trips, and returns them on the console under the strict program-return grammar. This host half
//! parses the return, reports the guest view and records the series. Every failure, from an
//! unprepared kit to a corrupt series block, fails the replicate; none is recorded as a metric.

use std::fmt::Display;
use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use overdrive_bench::{
    GuestArtifacts, GuestMemoryMib, GuestView, HostValue, Replicate, ReplicateFailure,
};
use overdrive_testing::guest_app::{
    FORWARDING_BENCH_METRIC, GUEST_PEER_GUEST_PATH, GuestReading, ProgramReturnError,
    parse_program_return, subcommand,
};
use overdrive_testing::guest_kit::cid_bands::GUEST_BENCHMARK;
use overdrive_testing::guest_kit::{GuestImageSpec, GuestKit, GuestKitError, WorkDir};
use overdrive_testing::guest_lane::{
    LANE_GUEST_MEMORY_MIB, LANE_GUEST_VCPUS, LANE_LAUNCH_TOKEN, lane_workload_addr,
    run_in_guest_program,
};
use overdrive_testing::vm_fixture::default_staging_root;
use overdrive_testing_macros::overdrive_bench;
use sha2::{Digest, Sha256};

/// Unrecorded closed-loop round trips before the timed phase.
const WARMUP_ROUND_TRIPS: usize = 200;
/// Timed closed-loop round trips: the length of the returned series.
const TIMED_ROUND_TRIPS: usize = 5_000;
/// The application listener's port inside the guest.
const APPLICATION_PORT: u16 = 8080;
/// How long one guest program run may take from its command to its exit status.
const PROGRAM_BOUND: Duration = Duration::from_secs(120);

/// Replicates this process has started; picks each replicate's CID inside the band.
static REPLICATES_STARTED: AtomicU32 = AtomicU32::new(0);
/// The provisioned guest kit, kept once a provision succeeded.
static KIT: OnceLock<GuestKit> = OnceLock::new();
/// The staged artifacts' digests, computed once per run.
static ARTIFACTS: OnceLock<GuestArtifacts> = OnceLock::new();

#[overdrive_bench(
    family = "guest-flow-forwarding",
    substrate = guest,
    arms = ["forwarding"],
    replicates = 30,
    metric(name = "forwarded_round_trip_ns", unit = ns, table = "forwarded-path", series),
    clock(guest = guest_monotonic),
    smoke(guest = "cgroup-default", profile = "nested-launch"),
)]
fn guest_flow_forwarding(rep: &mut Replicate) -> Result<(), ReplicateFailure> {
    let kit = provisioned(&KIT, || GuestKit::provision(&default_staging_root()))?;
    let cid = GUEST_BENCHMARK.cid(next_cid_index(&REPLICATES_STARTED));
    let work = WorkDir::create(&default_staging_root(), "guest-flow-forwarding")
        .map_err(|error| fail(&boot_reason(&error)))?;
    let program = program_argv(lane_workload_addr(cid.get()));

    let run = run_in_guest_program(
        kit,
        &GuestImageSpec::benchmark(),
        &program,
        cid,
        work.path(),
        PROGRAM_BOUND,
    )
    .map_err(|error| fail(&boot_reason(&error)))?;
    rep.retain_console(&run.console)?;
    let status = run.exit.map_err(|error| fail(&run_reason(&error)))?;
    if status != 0 {
        return Err(fail(&exit_reason(status)));
    }

    let returned = parse_program_return(&run.console, FORWARDING_BENCH_METRIC, TIMED_ROUND_TRIPS)
        .map_err(|error| fail(&return_reason(&error)))?;
    let artifacts = cached_artifacts(&ARTIFACTS, || staged_artifact_digests(kit));
    rep.report_guest_view(GuestView {
        vcpus: LANE_GUEST_VCPUS,
        memory_mib: GuestMemoryMib::new(LANE_GUEST_MEMORY_MIB),
        kernel: host_value(returned.kernel),
        clocksource: host_value(returned.clocksource),
        artifacts,
    })?;

    #[allow(clippy::cast_precision_loss, reason = "a round trip's nanoseconds are far below 2^52")]
    let values: Vec<f64> = returned.values.iter().map(|value| *value as f64).collect();
    rep.record_series(FORWARDING_BENCH_METRIC, &values)
}

fn fail(reason: &str) -> ReplicateFailure {
    ReplicateFailure::fixture(reason)
}

/// The kit held in `cell`, provisioning it on first success. A failed provision is not kept, so
/// an unprepared kit fails every replicate before any guest launches.
fn provisioned(
    cell: &OnceLock<GuestKit>,
    provision: impl FnOnce() -> Result<GuestKit, GuestKitError>,
) -> Result<&GuestKit, ReplicateFailure> {
    if let Some(kit) = cell.get() {
        return Ok(kit);
    }
    let kit = provision().map_err(|error| fail(&boot_reason(&error)))?;
    Ok(cell.get_or_init(|| kit))
}

/// The artifact digests held in `cell`, computed by `compute` on first use only.
fn cached_artifacts(
    cell: &OnceLock<GuestArtifacts>,
    compute: impl FnOnce() -> GuestArtifacts,
) -> GuestArtifacts {
    cell.get_or_init(compute).clone()
}

/// The index of the next replicate inside the CID band: this process's count of replicates
/// started, modulo the band width, so the seventeenth replicate reuses the first CID.
fn next_cid_index(started: &AtomicU32) -> u32 {
    started.fetch_add(1, Ordering::Relaxed) % GUEST_BENCHMARK.width
}

/// The guest program's command: the peer's `forwarding-bench` subcommand with the workload
/// address, the launch token, the application port and the round-trip counts.
fn program_argv(workload: Ipv4Addr) -> Vec<String> {
    vec![
        GUEST_PEER_GUEST_PATH.to_owned(),
        subcommand::FORWARDING_BENCH.to_owned(),
        workload.to_string(),
        LANE_LAUNCH_TOKEN.to_owned(),
        APPLICATION_PORT.to_string(),
        WARMUP_ROUND_TRIPS.to_string(),
        TIMED_ROUND_TRIPS.to_string(),
    ]
}

/// The digests of the three staged files the kit copies into the guest image.
fn staged_artifact_digests(kit: &GuestKit) -> GuestArtifacts {
    GuestArtifacts {
        overdrive_init: file_digest(&kit.init_bin),
        overdrive_guest_peer: file_digest(&kit.peer_bin),
        ovd_fwd: file_digest(&kit.forwarder_module),
    }
}

/// The SHA-256 of the file at `path`, absent when it does not exist and unreadable otherwise.
fn file_digest(path: &Path) -> HostValue {
    match std::fs::read(path) {
        Ok(bytes) => HostValue::Text(hex_digest(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            HostValue::Absent { absent: format!("{} does not exist", path.display()) }
        }
        Err(error) => HostValue::Unreadable { unreadable: format!("{}: {error}", path.display()) },
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn host_value(reading: GuestReading) -> HostValue {
    match reading {
        GuestReading::Text(text) => HostValue::Text(text),
        GuestReading::Absent(reason) => HostValue::Absent { absent: reason },
        GuestReading::Unreadable(error) => HostValue::Unreadable { unreadable: error },
    }
}

fn boot_reason(cause: &dyn Display) -> String {
    format!("guest boot: {cause}")
}

fn run_reason(cause: &dyn Display) -> String {
    format!("guest run: {cause}")
}

fn exit_reason(status: i32) -> String {
    format!("the in-guest program exited {status}")
}

fn return_reason(error: &ProgramReturnError) -> String {
    match error {
        ProgramReturnError::ReadingMissing { .. }
        | ProgramReturnError::ReadingRepeated { .. }
        | ProgramReturnError::ReadingMalformed { .. } => {
            format!("the guest returned no intact readings: {error}")
        }
        ProgramReturnError::BlockMissing
        | ProgramReturnError::BlockRepeated
        | ProgramReturnError::BlockMalformed
        | ProgramReturnError::CountMismatch { .. }
        | ProgramReturnError::ChecksumMismatch => {
            format!("the guest returned no intact {FORWARDING_BENCH_METRIC} block: {error}")
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, reason = "tests assert their own preconditions")]
mod tests {
    //! The host half's decisions that need no guest: the CID a replicate takes, the digests'
    //! one-time computation, the program's command and the pinned failure reasons.

    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;

    use overdrive_testing::placement;

    use super::*;

    placement!(host_pure);

    #[test]
    fn the_seventeenth_replicate_reuses_the_first_cid() {
        let started = AtomicU32::new(0);
        let width = GUEST_BENCHMARK.width;
        let indexes: Vec<u32> = (0..=width).map(|_| next_cid_index(&started)).collect();
        assert_eq!(indexes[..width as usize], (0..width).collect::<Vec<_>>()[..]);
        assert_eq!(indexes[width as usize], 0, "replicate {} wraps to the first CID", width + 1);
        assert_eq!(width, 16, "the band is sixteen CIDs wide");
        assert_eq!(started.load(Ordering::Relaxed), width + 1);
    }

    #[test]
    fn the_artifact_digests_are_computed_once_per_run() {
        let cell: OnceLock<GuestArtifacts> = OnceLock::new();
        let computed = AtomicUsize::new(0);
        let digest = || {
            computed.fetch_add(1, Ordering::Relaxed);
            GuestArtifacts {
                overdrive_init: HostValue::Text("a".to_owned()),
                overdrive_guest_peer: HostValue::Text("b".to_owned()),
                ovd_fwd: HostValue::Text("c".to_owned()),
            }
        };
        let first = cached_artifacts(&cell, digest);
        let second = cached_artifacts(&cell, digest);
        assert_eq!(first, second);
        assert_eq!(computed.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn the_program_command_carries_the_flow_the_token_and_the_round_trip_counts() {
        let argv = program_argv(Ipv4Addr::new(10, 200, 0, 9));
        assert_eq!(
            argv,
            [
                "/sbin/overdrive-guest-peer",
                "forwarding-bench",
                "10.200.0.9",
                LANE_LAUNCH_TOKEN,
                "8080",
                "200",
                "5000",
            ]
        );
    }

    #[test]
    fn a_reading_error_names_the_readings_and_a_block_error_names_the_block() {
        for error in [
            ProgramReturnError::ReadingMissing { key: "kernel".to_owned() },
            ProgramReturnError::ReadingRepeated { key: "kernel".to_owned() },
            ProgramReturnError::ReadingMalformed { key: "clocksource".to_owned() },
        ] {
            assert_eq!(
                return_reason(&error),
                format!("the guest returned no intact readings: {error}")
            );
        }
        for error in [
            ProgramReturnError::BlockMissing,
            ProgramReturnError::BlockRepeated,
            ProgramReturnError::BlockMalformed,
            ProgramReturnError::CountMismatch { expected: 5_000, found: 4_990 },
            ProgramReturnError::ChecksumMismatch,
        ] {
            assert_eq!(
                return_reason(&error),
                format!("the guest returned no intact forwarded_round_trip_ns block: {error}")
            );
        }
    }

    #[test]
    fn an_unprepared_kit_fails_every_replicate_before_any_guest_launches() {
        let cell: OnceLock<GuestKit> = OnceLock::new();
        let attempts = AtomicUsize::new(0);
        for _ in 0..2 {
            let failure = provisioned(&cell, || {
                attempts.fetch_add(1, Ordering::Relaxed);
                Err(GuestKitError::LaneNotPrepared {
                    artifact: "overdrive-guest-peer".to_owned(),
                    path: PathBuf::from("/prepared/overdrive-guest-peer"),
                })
            })
            .expect_err("an unprepared kit provides no kit");
            let reason = failure.to_string();
            assert!(reason.contains("guest boot: "), "{reason}");
            assert!(reason.contains("cargo xtask guest-kit-prepare"), "{reason}");
        }
        assert_eq!(attempts.load(Ordering::Relaxed), 2, "no failed provision is kept");
        assert!(cell.get().is_none());
    }

    #[test]
    fn a_launch_failure_a_run_failure_and_a_non_zero_exit_carry_their_pinned_reasons() {
        let cause = std::io::Error::other("no vmm");
        assert_eq!(boot_reason(&cause), "guest boot: no vmm");
        assert_eq!(run_reason(&cause), "guest run: no vmm");
        assert_eq!(exit_reason(40), "the in-guest program exited 40");
    }
}
