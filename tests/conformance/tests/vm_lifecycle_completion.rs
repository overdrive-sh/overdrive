//! VM lifecycle completion over repeated, sequential and concurrent launches.
//!
//! Every VM workload an operator launches completes its lifecycle: it reaches
//! Running (or runs to its own end), stops cleanly, raises every lifecycle
//! event, is never force-killed, and leaves nothing behind on the host. The
//! contract spans the VM driver, the control plane and its reconcilers, and is
//! observed through the production server's public API, so it has no single
//! crate as its owner.
//!
//! Latency of the lifecycle stages is not asserted here: the `vm-lifecycle`
//! benchmark family measures it.
//!
//! Run through `cargo xtask metal run -- cargo nextest run -p
//! overdrive-system-conformance --features integration-tests`: the body runs in
//! a disposable `cgroup-default` guest with nested KVM and launches the
//! workload VMs as nested guests.

#![cfg(feature = "integration-tests")]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    reason = "conformance bodies carry CONTRACT_SHAPE markers and the whole Given/When/Then narrative"
)]

use overdrive_system_conformance::TraceHistory;
use overdrive_system_conformance::vm_lifecycle::{Launch, Profile, VmLifecycleWorld};
use overdrive_testing::overdrive_test;

/// Launches run one after another, per profile.
const SEQUENTIAL_LAUNCHES: usize = 3;

/// Launches run together as one cohort of public operator requests, per profile.
const CONCURRENT_LAUNCHES: usize = 10;

/// An operator launches workloads of each lifecycle shape, one after another
/// and together, and every launch completes its lifecycle.
///
/// Given the production composition serves a VM guest image
/// When the operator launches, for each of a long-lived Job, a finite Job and a
///   cooperative Service, three workloads one after another and then ten at once
///  And stops the long-lived Job and the Service
/// Then every launch reaches Running (or its own end) and the terminal state
///   its shape defines: operator-stopped for the stopped workloads, a process
///   exit of zero for the finite Job, with no restart
///  And every launch left its lifecycle events, exited its VMM normally and
///   never by a signal, and holds no VMM process
///  And the driver's run directory, cgroup scope and rootfs clone of every
///   allocation are gone
///  And the host's links, namespaces, scopes, run directories and clone staging
///   are as they were before the server started.
///
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch-rustc")]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn native_metal_kvm_every_vm_launch_completes_its_lifecycle_cleanly_sequentially_and_concurrently()
 {
    let trace = TraceHistory::install_global();
    let world = VmLifecycleWorld::boot(&trace).await;

    let mut launches: Vec<Launch> = Vec::new();
    for profile in Profile::ALL {
        launches.extend(world.launch_sequentially(profile, 0, SEQUENTIAL_LAUNCHES).await);
        launches.extend(
            world.launch_concurrently(profile, SEQUENTIAL_LAUNCHES, CONCURRENT_LAUNCHES).await,
        );
    }

    let per_profile = SEQUENTIAL_LAUNCHES + CONCURRENT_LAUNCHES;
    assert_eq!(
        launches.len(),
        Profile::ALL.len() * per_profile,
        "every scheduled launch is reported"
    );
    for profile in Profile::ALL {
        let ordinals = launches
            .iter()
            .filter(|launch| launch.profile == profile)
            .map(|launch| launch.ordinal)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            ordinals,
            (0..per_profile).collect(),
            "{profile:?}: each sequential and concurrent launch is represented exactly once"
        );
    }
    let allocations = launches
        .iter()
        .filter_map(|launch| launch.alloc.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(allocations.len(), launches.len(), "every launch ran in its own allocation");

    let mut departures = launches
        .iter()
        .flat_map(|launch| {
            launch.failures.iter().map(|failure| format!("{}: {failure}", launch.workload_id))
        })
        .collect::<Vec<_>>();
    departures.extend(world.audit_events(&launches));

    let torn_down = world.tear_down().await;
    assert!(
        departures.is_empty(),
        "launches departed from a completed lifecycle:\n{departures:#?}"
    );
    torn_down.expect("the graceful shutdown converges and the host is restored");
}
