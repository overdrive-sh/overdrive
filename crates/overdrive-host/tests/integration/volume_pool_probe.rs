//! Node-local persistent volume (GH #348, ADR-0313, ADR-0314): the boot probe of
//! the production volume pool adapter, against a real guest kernel.
//!
//! Driving port: `VolumePoolProbe::probe` on `LvmVolumePoolProbe`. A guest of the
//! base `cgroup-default` profile is provisioned WITHOUT a volume pool (no
//! partition labelled `overdrive-volumes`), so it is the soft case: no pool
//! composed, startup continues (`volume_backing.not_composed`). The healthy-pool
//! and broken-pool scenarios, the `claim_state` pairing and the `lvm` contract
//! live in `volume_pool/` and run in the `volume-pool` guest profile.
//!
//! Evidence tier: Tier 3, real guest kernel, example-only (Mandate 11).

#![allow(clippy::doc_markdown, reason = "the CONTRACT_SHAPE line is machine-read verbatim")]

use overdrive_core::traits::volume_backing::VolumePoolProbe;
use overdrive_host::LvmVolumePoolProbe;
use overdrive_testing::overdrive_test;

/// Boundary 6 (feature disabled), adapter side: a node with no labelled
/// partition composes no volume pool and the probe does not refuse startup.
///
/// This is a GUARD, not a pending test: the scaffold already reports "no pool"
/// (so every existing boot is unchanged), and the real probe must keep doing so
/// on a node without the labelled partition. It holds before and after DELIVER.
/// CONTRACT_SHAPE: pure-function.
#[overdrive_test(guest = "cgroup-default")]
#[test]
fn guard_a_node_without_a_labelled_partition_composes_no_volume_pool() {
    let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
    let probed = runtime
        .block_on(LvmVolumePoolProbe::new().probe())
        .expect("an absent pool is the soft case, not an error");
    assert!(probed.is_none(), "no partition is labelled overdrive-volumes on this guest");
}
