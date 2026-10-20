//! Shared `lo`-named `DataplaneConfig` for the simulated-dataplane fixtures.
//!
//! ADR-0061 § 1 (single-node-dataplane-wiring step 01-03): the default
//! `ServerConfig.dataplane` is the veth-named single-node shape
//! (`ovd-veth-cli` / `ovd-veth-bk`) the production `HostDataplaneComposer`
//! provisions. A fixture whose `BootAdapters.dataplane` is a
//! `SimDataplaneComposer` (GH #295 NO-OVERRIDE-SEAMS; the replacement of the
//! removed `dataplane_override`) provisions no pair and attaches no XDP; it
//! names `lo` / `lo` so its configuration describes interfaces that exist in
//! the test VM, matching the `host_ipv4` 127.0.0.1 its composer returns.
//!
//! This single helper is the SSOT for that `lo`/`lo` shape so the many
//! SimDataplane-override fixtures cannot drift. The prior `loopback()`
//! helper on `DataplaneConfig` was deleted in this step (single-cut,
//! no surviving production default of `lo`); this test-only helper
//! replaces it for the fixtures that genuinely need a loopback iface.
//!
//! Included into both the acceptance and integration test-binary roots
//! via `#[path]` (each `tests/*.rs` file is its own crate root, so the
//! shared source is `#[path]`-included rather than `mod`-resolved).

/// The `lo`/`lo` `DataplaneConfig` for SimDataplane-override fixtures.
///
/// `client_iface = backend_iface = "lo"` so
/// `resolve_host_ipv4_from_dataplane_config` resolves `host_ipv4` to
/// `127.0.0.1` without the serve-boot provisioner having created any
/// veth pair. This is NOT a default-veth shape, so the production boot
/// gate (`is_default_veth`) correctly skips provision for these
/// fixtures.
pub fn lo_dataplane_config() -> overdrive_control_plane::dataplane_config::DataplaneConfig {
    overdrive_control_plane::dataplane_config::DataplaneConfig {
        client_iface: "lo".to_owned(),
        backend_iface: "lo".to_owned(),
    }
}
