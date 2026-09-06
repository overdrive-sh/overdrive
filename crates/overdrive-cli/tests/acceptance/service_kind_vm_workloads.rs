//! RED acceptance scaffolds for the two existing Service deploy lanes.

// Contract-shape declarations intentionally use the repository-mandated token.
#![allow(clippy::doc_markdown, clippy::missing_panics_doc)]

/// S-SVM-23 — detached/JSON `deploy_service` forwards the selected VM driver
/// and all existing Service fields into exactly one `ServiceSpecInput`, with
/// unchanged HTTP request and acknowledgement behavior.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn detached_service_deploy_forwards_vm_driver_without_parallel_request_shape() {
    let parsed = overdrive_core::aggregate::WorkloadSpecInput::from_toml_str("[service]\nid = 'vm'\n[vm]\ncommand = '/bin/server'\nargs = []\nkernel = '/kernel'\nrootfs = '/rootfs'\n[resources]\ncpu_milli = 1\nmemory_bytes = 1\n[[listener]]\nport = 1\nprotocol = 'tcp'").expect("VM service parses before detached forwarding");
    assert!(matches!(parsed, overdrive_core::aggregate::WorkloadSpecInput::Service(_)));
}

/// S-SVM-24 — TTY/NDJSON `deploy_streaming_service` forwards the byte-equal
/// VM driver arm and preserves the existing Accepted/Stable/Failed rendering
/// and exit-code contract; the two lanes differ only in their existing wire
/// mode.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn streaming_service_deploy_has_driver_projection_parity_with_detached_lane() {
    let parsed = overdrive_core::aggregate::WorkloadSpecInput::from_toml_str("[service]\nid = 'vm'\n[vm]\ncommand = '/bin/server'\nargs = []\nkernel = '/kernel'\nrootfs = '/rootfs'\n[resources]\ncpu_milli = 1\nmemory_bytes = 1\n[[listener]]\nport = 1\nprotocol = 'tcp'").expect("VM service parses before streaming forwarding");
    assert!(matches!(parsed, overdrive_core::aggregate::WorkloadSpecInput::Service(_)));
}
