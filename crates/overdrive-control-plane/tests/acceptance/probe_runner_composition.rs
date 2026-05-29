//! GAP-4-AT-01 — production `ExecDriver` carries a wired `ProbeRunner`.
//!
//! Pre-patch (commit `004032b8` and earlier) the binary-composition
//! root constructed `Arc<ProbeRunner>` then immediately discarded it
//! into a `let _probe_runner = probe_runner;` binding, while the
//! production `ExecDriver` was built BEFORE the runner existed and
//! consequently carried `probe_runner: None`. Every
//! `ExecDriver::on_alloc_running` / `on_alloc_terminal` invocation in
//! production took the trait-default no-op path — the probe
//! subsystem was structurally dead despite shipping green tests
//! against the `with_probe_runner`-equipped fixture path.
//!
//! This acceptance test pins the post-patch composition shape by
//! driving the production helper
//! [`overdrive_control_plane::compose_production_driver`] with Sim
//! prober adapters (matching the
//! [`probe_runner_boot_gate`](super::probe_runner_boot_gate) AT's
//! convention) and asserting on the observable supervisor lifecycle
//! through the resulting `Arc<dyn Driver>` and `Arc<ProbeRunner>`
//! pair. The structural defense is the helper itself: `run_server`
//! is the sole production caller of `compose_production_driver`, so
//! exercising the helper through its public driving port pins the
//! exact composition `run_server` performs at boot.
//!
//! Companion structural defense — [`xtask::dst_lint`] gains a
//! `BannedKind::UnderscoreBindingProbeRunner` clause that rejects
//! `let _probe_runner = ...` / `let _ = probe_runner` patterns in
//! `crates/overdrive-control-plane/src/`, so a future copy-paste
//! cannot re-introduce GAP-5 silently. See
//! `.context/01-03-structural-gap-audit.md`.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_control_plane::compose_production_driver;
use overdrive_core::SpiffeId;
use overdrive_core::aggregate::probe_descriptor::ProbeDescriptor;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{AllocationSpec, Resources};
use overdrive_sim::adapters::SimCgroupFs;
use overdrive_sim::adapters::probers::{SimExecProber, SimHttpProber, SimTcpProber};

/// Stub clock — `compose_production_driver` plumbs the clock into
/// `ExecDriver::new`; the driver only consults it from `Driver::stop`
/// which this AT does not exercise. The synthetic implementation is
/// load-bearing only for satisfying the mandatory port-trait
/// dependency at construction per `.claude/rules/development.md`
/// § "Port-trait dependencies".
struct StubClock;

#[async_trait]
impl Clock for StubClock {
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }
    fn unix_now(&self) -> Duration {
        Duration::ZERO
    }
    async fn sleep(&self, _: Duration) {}
}

fn sample_spec(alloc_id: &AllocationId) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc_id.clone(),
        identity: SpiffeId::from_str("spiffe://overdrive.local/test/wl").expect("valid SpiffeId"),
        command: "/bin/true".to_owned(),
        args: vec![],
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::<ProbeDescriptor>::new(),
    }
}

/// GAP-4-AT-01 — the production composition helper threads the
/// Earned-Trust-vetted `Arc<ProbeRunner>` through into the resulting
/// `Arc<dyn Driver>` such that `Driver::on_alloc_running` increments
/// the runner's `active_alloc_count` and `Driver::on_alloc_terminal`
/// decrements it back to zero.
///
/// Observable: `runner.active_alloc_count()` transitions
/// 0 → 1 → 0 across the driver's lifecycle hooks. If `run_server`
/// had constructed the driver without `.with_probe_runner(...)` — as
/// it did pre-patch — the first transition would still read 0
/// because the trait-default hook is a no-op when `probe_runner` is
/// `None`.
///
/// The test deliberately enters through the same production helper
/// `run_server` calls; the helper is the single composition site,
/// so the post-boot object graph it produces IS the production
/// object graph (modulo the Sim probers, which only affect the
/// Earned-Trust gate verdict, not the driver-to-runner threading).
#[tokio::test]
async fn production_driver_lifecycle_hooks_drive_wired_probe_runner_supervisor() {
    let tcp = Arc::new(SimTcpProber::new()); // empty queue → Pass
    let http = Arc::new(SimHttpProber::new());
    let exec = Arc::new(SimExecProber::new());

    let (driver, runner) = compose_production_driver(
        tcp,
        http,
        exec,
        // Cgroup path is exercised only on `Driver::start` (which
        // this AT does not call); any path is acceptable here.
        PathBuf::from("/tmp/overdrive-test-composition"),
        Arc::new(StubClock),
        // `CgroupFs` substrate is stored on the driver and consulted
        // only from `Driver::start`; this AT exercises the lifecycle
        // hooks, not start, so the Sim adapter satisfies the mandatory
        // port-trait dependency without affecting the assertion.
        Arc::new(SimCgroupFs::new()),
    )
    .await
    .expect("Earned-Trust gate passes with default Sim probers");

    let alloc_id = AllocationId::new("alloc-composition-1").expect("valid AllocationId");
    let spec = sample_spec(&alloc_id);

    assert_eq!(
        runner.active_alloc_count(),
        0,
        "post-boot ProbeRunner must hold zero supervisors (no allocs have run yet)"
    );

    driver.on_alloc_running(&spec);
    assert_eq!(
        runner.active_alloc_count(),
        1,
        "production composition gap (GAP-4 + GAP-5): on_alloc_running must register a \
         supervisor on the wired ProbeRunner. Pre-patch the production driver was built \
         without `.with_probe_runner(...)` and the trait-default no-op fired — see \
         `.context/01-03-structural-gap-audit.md`"
    );

    driver.on_alloc_terminal(&alloc_id);
    assert_eq!(
        runner.active_alloc_count(),
        0,
        "on_alloc_terminal must cancel the wired supervisor; failure here means the \
         driver→runner threading was lost between `on_alloc_running` and `on_alloc_terminal`"
    );
}
