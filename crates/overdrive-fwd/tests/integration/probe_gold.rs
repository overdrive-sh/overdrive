//! The forwarder probe's gold test (§ *Earned Trust — the forwarder probe*,
//! the behavioural layer): each failure a stage exists to catch is injected,
//! and the probe must refuse naming that stage. A probe that passes under
//! its own failure fails the test.
//!
//! Injections are scoped to one thread's namespaces (support
//! `in_namespaces`) or restored on drop (`LoopbackTransportRemoved`), so
//! they never outlive the case.
//!
//! Not injected here: a module of another ABI. The `Version` stage checks
//! the ABI version only (MODULE-REFCOUNT, 2026-10-20: no permanence check),
//! and its verdict over hand-built inputs is `src/uapi.rs`'s
//! `the_version_stage_accepts_only_this_abi`.

use nix::libc;
use overdrive_core::guest_transport::forwarder::{
    FlowForwarder, ForwarderError, ForwarderProbeObservation, ForwarderProbeStage,
};
use overdrive_fwd::KernelFlowForwarder;

use super::support::{
    LoopbackTransportRemoved, clear_cloexec, in_namespaces, live, mount_raw, open_context, run,
};

/// The stage a refusal names, or a panic if the probe passed or failed
/// without naming one.
fn refused_stage(result: Result<(), ForwarderError>) -> (ForwarderProbeStage, ForwarderError) {
    match result {
        Ok(()) => panic!("the probe passed under its own injected failure"),
        Err(ForwarderError::Probe { stage, source }) => (stage, *source),
        Err(e @ ForwarderError::ProbeMismatch { stage, .. }) => (stage, e),
        Err(other) => panic!("a probe refusal must name its stage, got {other:?}"),
    }
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn the_probe_passes_and_leaves_no_residue_on_a_correctly_provisioned_node() {
    let fwd = open_context();
    fwd.probe().expect("the probe passes on a correctly provisioned node");
    // Cleanup: the probe's own pairs, watches and records are gone, and the
    // owner's context is as it was.
    assert_eq!(live(&fwd), (0, 0, 0));
    assert_eq!(fwd.take_end().expect("take_end"), None);
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn an_absent_device_is_refused_naming_device_open() {
    // The module absent: `/dev/ovd_fwd` does not exist in this thread's
    // mount namespace. The `DeviceOpen` stage's refusal is the open's own
    // `DeviceAbsent` (the variant `probe_startup` projects for this stage,
    // § *Failure projection*). A context opened before the device vanished
    // must not pass its probe either: the probe's `ContextRelease` stage
    // opens and closes a second context, so a probe that passes here did
    // not exercise the device it claims to have proved.
    let fwd = open_context();
    let (open_result, probe_result) = in_namespaces(libc::CLONE_NEWNS, move || {
        mount_raw(Some("tmpfs"), "/dev", Some("tmpfs"), 0).expect("hide /dev in this namespace");
        (KernelFlowForwarder::open(), fwd.probe())
    });
    assert!(
        matches!(open_result, Err(ForwarderError::DeviceAbsent { .. })),
        "open without the device must be DeviceAbsent, got {open_result:?}"
    );
    let (stage, _) = refused_stage(probe_result);
    assert!(
        matches!(stage, ForwarderProbeStage::DeviceOpen | ForwarderProbeStage::ContextRelease),
        "a probe without the device is refused by a stage that opens it, got {stage:?}"
    );
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_device_that_is_not_the_forwarder_is_refused_naming_version() {
    // `/dev/null` answers no forwarder ioctl: the `Version` stage, not a later
    // one, must refuse it.
    let (stage, _) = in_namespaces(libc::CLONE_NEWNS, || {
        mount_raw(Some("/dev/null"), "/dev/ovd_fwd", None, libc::MS_BIND)
            .expect("bind /dev/null over /dev/ovd_fwd in this namespace");
        let fwd = KernelFlowForwarder::open().expect("the substituted device opens");
        refused_stage(fwd.probe())
    });
    assert_eq!(stage, ForwarderProbeStage::Version);
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn an_inheritable_context_is_refused_naming_version() {
    let fwd = open_context();
    clear_cloexec(fwd.readiness_fd());
    match fwd.probe() {
        Err(ForwarderError::ProbeMismatch { stage, observed }) => {
            assert_eq!(stage, ForwarderProbeStage::Version);
            assert_eq!(observed, ForwarderProbeObservation::ContextInheritable);
        }
        other => panic!("a context without FD_CLOEXEC must be ContextInheritable, got {other:?}"),
    }
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn an_absent_vsock_loopback_transport_is_refused_naming_stream_round_trip() {
    let fwd = open_context();
    let removed = LoopbackTransportRemoved::remove();
    let result = fwd.probe();
    drop(removed);
    let (stage, _) = refused_stage(result);
    assert_eq!(stage, ForwarderProbeStage::StreamRoundTrip);
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_peer_reset_where_a_clean_end_is_expected_is_refused() {
    // In a private network namespace every TCP FIN on loopback is rejected
    // with a reset, so a TCP end the probe expects to be clean arrives as a
    // reset. The stages whose contract includes a clean TCP end are
    // `StreamRoundTrip` (if it ends its pairs cleanly) and `HalfClose`; the
    // refusal must name one of them, never a pass and never another stage.
    let fwd = open_context();
    let result = in_namespaces(libc::CLONE_NEWNET, move || {
        run("ip", &["link", "set", "lo", "up"]);
        run("nft", &["add", "table", "inet", "ovd_fwd_probe_gold"]);
        run(
            "nft",
            &[
                "add",
                "chain",
                "inet",
                "ovd_fwd_probe_gold",
                "out",
                "{ type filter hook output priority 0 ; }",
            ],
        );
        run(
            "nft",
            &[
                "add",
                "rule",
                "inet",
                "ovd_fwd_probe_gold",
                "out",
                "tcp",
                "flags",
                "&",
                "fin",
                "==",
                "fin",
                "reject",
                "with",
                "tcp",
                "reset",
            ],
        );
        fwd.probe()
    });
    let (stage, _) = refused_stage(result);
    assert!(
        matches!(stage, ForwarderProbeStage::StreamRoundTrip | ForwarderProbeStage::HalfClose),
        "a reset where a clean end is expected must be refused by a stage expecting a clean end, \
         got {stage:?}"
    );
}
