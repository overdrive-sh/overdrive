//! E-G6 spike: the `guest-flow-forwarding` body mirrored from the family, declared in a spike
//! crate to prove the guest substrate runs through `--spike` (DDD-11).

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
    replicates = 9,
    metric(name = "forwarded_round_trip_ns", unit = ns, table = "forwarded-path", series),
    clock(guest = guest_monotonic),
    smoke(none),
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
