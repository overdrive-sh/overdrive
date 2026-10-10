//! `prefix_landing` conformance on the real kernel (netns-density-295,
//! QPROJ-8).
//!
//! Spec, referenced in place: `specs/quint/guest-flow-owner/prefix_landing.qnt`,
//! module `prefix_landing`, definition `lands(net, client, port)`. The module
//! is stateless, so there is no trace to replay: for every reachable
//! configuration, every client class and every probed port, the landing the
//! kernel produces must equal `lands` evaluated over the `Net` the kernel
//! reports. This is the one driver that stays in `tests/conformance`: the
//! landing decision belongs to no single owner — the kernel arbitrates over the
//! steering (`overdrive-dataplane`), the constant prefix rules (the mTLS
//! worker), the route and the intake listeners (the host guest-flow owner).
//!
//! Invariants it defends at the kernel boundary (the models that import
//! `prefix_landing` state them over `lands`): `shared_table` invariant 11
//! (`NoWildcardReached`: no connection to the prefix reaches a host service),
//! `prefix_boot`'s steering-before-route ordering, and `intake`'s
//! "an intake listener is reached only through its steering entry"
//! (INTAKE-BIND, K-L10). Assumptions it discharges at Tier 3: K-D1, K-D2,
//! K-D4, K-L1, K-L2, K-L5, K-L10 and A-33 (spec README).
//!
//! Configurations (QPROJ-8), all reached through production paths only:
//! never booted; serving with the guest listening (workload `listening`) and
//! with the guest not listening (workload `silent`); `serve` killed; `serve`
//! restarted and serving; `serve` stopped gracefully. `assignRemote` is not
//! reachable through production and stays covered by the model alone.
//!
//! Lane: native `x86_64` metal (real microVM guests), root, `quint` on PATH,
//! node-global kernel state → the binary is in the `host-kernel-shared`
//! nextest group. Run through `tests/conformance/run-native-metal.sh` under
//! `cargo xtask metal run --`.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, reason = "conformance fixtures fail loudly on a broken precondition")]

use std::collections::BTreeSet;
use std::time::Duration;

use overdrive_system_conformance::prefix_landing::{
    ComparisonRecord, Configuration, LandingNode, LandingWorkload, RemoteClient, Target,
    UNLEASED_PREFIX_ADDRESS, WildcardSentinel, clear_node_guest_prefix_state,
    read_intake_listeners, stage_guest_rootfs,
};
use overdrive_system_conformance::{assert_native_metal, wait_until};
use overdrive_testing::vm_fixture::{VmFixture, default_staging_root};

/// Declared by `listening`, whose guest application listens on it.
const PORT_LISTENING: u16 = 18_961;
/// Declared by `silent`, whose guest application listens on nothing.
const PORT_SILENT: u16 = 18_962;
/// Declared by no workload.
const PORT_UNDECLARED: u16 = 18_963;

const RUNNING_BOUND: Duration = Duration::from_secs(120);
const STEERED_BOUND: Duration = Duration::from_secs(60);

fn listening() -> LandingWorkload {
    LandingWorkload {
        id: "landing-listening",
        declared: BTreeSet::from([PORT_LISTENING]),
        listens: true,
    }
}

fn silent() -> LandingWorkload {
    LandingWorkload {
        id: "landing-silent",
        declared: BTreeSet::from([PORT_SILENT]),
        listens: false,
    }
}

fn unleased_target() -> Target {
    Target {
        label: "unleased",
        addr: UNLEASED_PREFIX_ADDRESS,
        declared: BTreeSet::new(),
        ports: vec![PORT_LISTENING, PORT_UNDECLARED],
    }
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER vsock new-step 5 and attachment cut (new-steps 9, 12, 13: guest intake \
            adaptation, guest-prefix steering with intake listeners, constant prefix rules, the prefix \
            route); then stays ignored: native metal only, run by run-native-metal.sh with quint \
            provisioned — on activation replace this reason, never remove the attribute"]
async fn native_metal_kvm_prefix_landing_matches_lands_for_every_client_class_and_reachable_configuration()
 {
    assert_native_metal();
    let fixture = VmFixture::provision(&default_staging_root()).expect("provision the VM fixture");
    let root = tempfile::Builder::new()
        .prefix("prefix-landing-")
        .tempdir_in(default_staging_root())
        .expect("scenario directory on the reflink-capable staging root");
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    std::fs::create_dir_all(&data_dir).expect("create the data dir");
    std::fs::create_dir_all(&config_dir).expect("create the config dir");
    let rootfs = stage_guest_rootfs(root.path(), &fixture);
    let mut record = ComparisonRecord::default();

    // GIVEN a node that never ran `serve`, a wildcard host service on every
    // probed port (bound before any boot, R5-1), and a client of each class.
    let precondition = clear_node_guest_prefix_state();
    assert!(
        precondition.failures.is_empty(),
        "reach the never-booted kernel state: {:?}",
        precondition.failures
    );
    for removed in precondition.removed {
        record.note(format!("precondition removed leftover {removed}"));
    }
    let _sentinels: Vec<WildcardSentinel> =
        [PORT_LISTENING, PORT_SILENT, PORT_UNDECLARED].map(WildcardSentinel::bind).into();
    let remote = RemoteClient::create();

    // never booted
    record.compare(Configuration::NeverBooted, &[unleased_target()], &remote).await;

    // WHEN `serve` boots and two workloads are deployed through the API
    let node = LandingNode::boot(&data_dir, &config_dir).await;
    for workload in [listening(), silent()] {
        node.api().submit_workload(workload.id, &workload.request(&fixture, &rootfs)).await;
    }
    let mut addrs = Vec::new();
    for workload in [listening(), silent()] {
        let mut addr = None;
        let deadline = tokio::time::Instant::now() + RUNNING_BOUND;
        while addr.is_none() && tokio::time::Instant::now() < deadline {
            addr = node.workload_addr(workload.id).await;
            if addr.is_none() {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        let addr = addr
            .unwrap_or_else(|| panic!("{} reaches Running with a workload address", workload.id));
        record.note(format!("{} runs at {addr}", workload.id));
        addrs.push((workload, addr));
    }
    let (listening_workload, listening_addr) = addrs[0].clone();
    wait_until(STEERED_BOUND, "the listening guest's intake listener is open", || {
        read_intake_listeners(listening_addr, &listening_workload.declared)
            .contains(&PORT_LISTENING)
    });
    let targets: Vec<Target> = addrs
        .iter()
        .map(|(workload, addr)| Target {
            label: workload.id,
            addr: *addr,
            declared: workload.declared.clone(),
            ports: workload.declared.iter().copied().chain([PORT_UNDECLARED]).collect(),
        })
        .chain([unleased_target()])
        .collect();

    // THEN every landing equals `lands` over the kernel's Net, serving …
    record.compare(Configuration::Serving, &targets, &remote).await;

    // … after `serve` dies …
    node.kill().await;
    record.compare(Configuration::ServeKilled, &targets, &remote).await;

    // … after it restarts and converges again …
    let node = LandingNode::boot(&data_dir, &config_dir).await;
    wait_until(STEERED_BOUND, "the restarted serve steers the listening guest again", || {
        read_intake_listeners(listening_addr, &listening_workload.declared)
            .contains(&PORT_LISTENING)
    });
    record.compare(Configuration::ServingAfterRestart, &targets, &remote).await;

    // … and after a graceful stop.
    for (workload, _) in &addrs {
        node.api().stop_workload(workload.id).await;
    }
    node.shutdown().await;
    record.compare(Configuration::ServeStoppedGracefully, &targets, &remote).await;

    // Cleanup complement of this scenario's boots (U-4: production keeps the
    // route, the pinned steering and the table across shutdown).
    let cleanup = clear_node_guest_prefix_state();
    for removed in &cleanup.removed {
        record.note(format!("cleanup removed {removed}"));
    }
    for failure in &cleanup.failures {
        record.note(format!("cleanup FAILED: {failure}"));
    }
    record.assert_conforms();
    assert!(cleanup.failures.is_empty(), "cleanup complement failed: {:?}", cleanup.failures);
}
