//! The driven-adapter bundles this crate's `serve` fixtures boot over
//! (GH #295: NO-OVERRIDE-SEAMS full scope, STORES-IN, PORTS-CONFIG,
//! FWD-PREREQ; feature-delta § *Composition*, "Driven adapters of
//! `run_server`", and its replacement table).
//!
//! `run_server(config, adapters)` and the supplied-owner path
//! `run_server_with_obs_and_driver(s)(config, boot, ..)` construct no driven
//! adapter; `ServerConfig` carries configuration only. So every fixture
//! starts from the one list of host adapters, `NodeAdapters::host(&config,
//! fs, kek)` — the composition `overdrive serve` runs, opened over the
//! fixture's own `data_dir` (the stores, the observation store and the CA
//! from STORES-IN) — and replaces by name only the fields it simulates. The
//! two helpers here are the replacements every non-VM fixture made before
//! the seams were removed, in one place so the fixtures cannot drift:
//!
//! - `boot.kek` — a hermetic `SimKek` (C1-AMEND: the kernel keyring refuses
//!   a cold boot);
//! - `boot.dataplane` — a `SimDataplaneComposer` over a `SimDataplane` with
//!   `host_ipv4` `127.0.0.1` (the former `dataplane_override:
//!   Some(SimDataplane)`; the replacement table's first row);
//! - `boot.mtls.intercept` / `boot.guest_dns` — `SimMtlsIntercept` /
//!   `SimGuestDnsFactory` (the adapters the fixtures passed to the former
//!   `ServerConfig::new(kek, intercept, dns)`).
//!
//! Everything else stays the host adapter: the clock is `SystemClock` (the
//! former `ServerConfig::new` default), the guest-flow seam is
//! `GuestFlowAdapter::host()` (the real flow and control listeners and the
//! real forwarder probe), the VM ports are the host pair with `Optional`
//! capability. A fixture that needs another clock, a scripted adapter or a
//! fault replaces that field itself after calling a helper.
//!
//! **Ports.** [`test_config`] sets `NodeVsockPortConfig::kernel_assigned()`,
//! so the real listeners bind kernel-assigned vsock ports; a fixture reads
//! them back (`ServerHandle::vsock_ports`), never assumes a number.
//!
//! **Lane prerequisite.** [`node_adapters`] (the composed path, which runs
//! the real phase-1 forwarder probe) first checks the loaded `ovd_fwd` is
//! this workspace's build (`overdrive_testing::forwarder_module`), so a
//! missing or foreign module fails as a lane prerequisite naming its step,
//! never as a boot regression. [`boot_adapters`] (the supplied-owner path,
//! whose supplied owner is a simulation that runs no forwarder probe) does
//! not.
//!
//! **Restart.** A fixture that boots twice over one `data_dir` shuts the
//! first server down and calls a helper again: `NodeAdapters::host` then
//! reopens the stores from disk, as a new process would (their
//! boot-validation walks run again), with a fresh `CaAdapters`. No fixture
//! carries a `StoreAdapters` clone across boots.
//!
//! `#[path]`-included into both test-binary roots (each `tests/*.rs` file is
//! its own crate root).

#![allow(dead_code, reason = "each test binary uses the subset it needs")]

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use overdrive_control_plane::{BootAdapters, NodeAdapters, ServerConfig};
use overdrive_core::guest_transport::NodeVsockPortConfig;
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::vm_host_state::SimVmHostState;
use overdrive_sim::adapters::{SimDataplaneComposer, SimGuestDnsFactory, SimKek, SimMtlsIntercept};

/// The configuration every in-process fixture boots: `127.0.0.1:0`, the two
/// directories, kernel-assigned vsock listener ports, the `lo` / `lo`
/// dataplane interfaces (no veth pair is provisioned for them) and every
/// other field at `ServerConfig::new()`'s default.
#[must_use]
pub fn test_config(data_dir: PathBuf, operator_config_dir: PathBuf) -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1:0".parse::<SocketAddr>().expect("parse bind addr"),
        data_dir,
        operator_config_dir,
        dataplane: Some(super::dataplane_lo::lo_dataplane_config()),
        vsock_ports: NodeVsockPortConfig::kernel_assigned(),
        ..ServerConfig::new()
    }
}

/// The host bundle over `config` with the non-VM fixtures' simulations
/// (module docs), after the FWD-PREREQ check. For `run_server`.
///
/// # Panics
///
/// When the lane prerequisite does not hold, or the host bundle cannot open
/// its stores under `config.data_dir`.
#[must_use]
pub fn node_adapters(config: &ServerConfig) -> NodeAdapters {
    overdrive_testing::forwarder_module::expect_forwarder_module();
    let mut adapters = host_bundle(config);
    simulate_boot_edges(&mut adapters.boot);
    adapters
}

/// The same bundle as [`node_adapters`] but keeping the production
/// `HostDataplaneComposer` (the real `EbpfDataplane` and its veth pair) —
/// the former `dataplane_override: None` fixtures.
///
/// # Panics
///
/// As [`node_adapters`].
#[must_use]
pub fn node_adapters_with_host_dataplane(config: &ServerConfig) -> NodeAdapters {
    overdrive_testing::forwarder_module::expect_forwarder_module();
    let mut adapters = host_bundle(config);
    adapters.boot.mtls.intercept = Arc::new(SimMtlsIntercept::new());
    adapters.boot.guest_dns = Arc::new(SimGuestDnsFactory::default());
    adapters
}

/// The boot-wide group for the supplied-owner path
/// (`run_server_with_obs_and_driver(s)`): the host bundle's `boot` over
/// `config` with the non-VM fixtures' simulations and a `SimVmHostState`
/// (every supplied-owner fixture's boot reclamation read). A fixture that wires its
/// own observation store replaces `observation` (and must not have opened
/// the bundle's file itself: the bundle already opened it — use
/// `boot.observation` instead of wiring a second handle); the supplied
/// path's drivers and exec wiring are built over `boot.clock`.
///
/// # Panics
///
/// When the host bundle cannot open its stores under `config.data_dir`.
#[must_use]
pub fn boot_adapters(config: &ServerConfig) -> BootAdapters {
    let mut boot = host_bundle(config).boot;
    simulate_boot_edges(&mut boot);
    boot.vm_host_state = Arc::new(SimVmHostState::new());
    boot
}

fn host_bundle(config: &ServerConfig) -> NodeAdapters {
    NodeAdapters::host(config, Arc::new(RealCgroupFs::new()), Arc::new(SimKek::for_boot()))
        .expect("the host bundle opens its stores under the fixture's data_dir")
}

fn simulate_boot_edges(boot: &mut BootAdapters) {
    boot.dataplane =
        Arc::new(SimDataplaneComposer::new(Arc::new(SimDataplane::new()), Ipv4Addr::LOCALHOST));
    boot.mtls.intercept = Arc::new(SimMtlsIntercept::new());
    boot.guest_dns = Arc::new(SimGuestDnsFactory::default());
}

/// `run_server(config, node_adapters(&config))`: the composed boot of a
/// non-VM fixture.
///
/// # Errors
///
/// What `run_server` returns.
///
/// # Panics
///
/// As [`node_adapters`].
pub async fn boot(
    config: ServerConfig,
) -> Result<overdrive_control_plane::ServerHandle, overdrive_control_plane::error::ControlPlaneError>
{
    let adapters = node_adapters(&config);
    overdrive_control_plane::run_server(config, adapters).await
}
