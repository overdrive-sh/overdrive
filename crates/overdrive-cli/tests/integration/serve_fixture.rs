//! The one composition every in-process `serve` test fixture of this crate
//! boots through (GH #295: NO-OVERRIDE-SEAMS, STORES-IN, PROBE-ROOT-ENV,
//! PORTS-CONFIG, FWD-PREREQ; feature-delta § *Composition*, "Driven adapters
//! of `run_server`" and "`overdrive serve` and the CLI entry points").
//!
//! **One entry point.** Every fixture calls
//! [`overdrive_cli::commands::serve::run_with_adapters`] — production `run`
//! is a call of it — with the real cgroupfs adapter and a builder that takes
//! the host bundle `NodeAdapters::host(config, fs, kek)` over the very
//! `&ServerConfig` the boot receives, so the stores, the CA and the
//! observation store are opened from the test's own `data_dir` and the
//! bundle cannot name another directory. The builder then replaces, by
//! name, only the fields the fixture simulates:
//!
//! - `boot.kek` is always a hermetic [`SimKek`] (the production
//!   `SystemdCredsKeyring` refuses a cold boot; C1-AMEND), passed to
//!   `NodeAdapters::host` directly;
//! - [`DataplaneChoice::Sim`] sets `boot.dataplane` to a
//!   `SimDataplaneComposer` over a `SimDataplane` with `host_ipv4`
//!   `127.0.0.1` — the replacement of the removed `dataplane_override`;
//!   [`DataplaneChoice::Host`] keeps the production `HostDataplaneComposer`
//!   (the former `run_with_kek` path);
//! - [`VmPorts`] replaces `vm.vmm` / `vm.beacon` (they travel together,
//!   DG-4) and sets `vm.capability` to `VmCapability::Required` — the
//!   replacement of the removed `vm_override`.
//!
//! No fixture sets a `ServerConfig` adapter or fault field (none exists).
//!
//! **Ports.** Every fixture's [`serve_args`] carries
//! `NodeVsockPortConfig::kernel_assigned()`, so the real beacon, flow and
//! control listeners bind kernel-assigned vsock ports and no `serve` test
//! collides with another on the host, whichever workspace runs it. A test
//! reads the bound ports back (`ServerHandle::vsock_ports`), never assumes a
//! number.
//!
//! **Lane prerequisite.** Every boot runs the real phase-1 forwarder probe,
//! so [`serve`] first checks that the loaded `ovd_fwd` is this workspace's
//! build (`overdrive_testing::forwarder_module`) and fails as a missing or
//! stale lane prerequisite naming the step — never as a boot regression.
//!
//! **Restart.** A fixture that boots twice over one `data_dir` shuts the
//! first server down and calls [`serve`] again: the builder runs
//! `NodeAdapters::host` afresh, so the stores are reopened from disk as a new
//! process would reopen them (their boot-validation walks run again) and the
//! CA gets a fresh `CaAdapters`. No fixture carries a `StoreAdapters` clone
//! across boots.

#![allow(dead_code, reason = "each lane module uses the subset it needs")]

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use overdrive_cli::commands::serve::{ServeArgs, ServeHandle, run_with_adapters};
use overdrive_cli::http_client::CliError;
use overdrive_control_plane::{NodeAdapters, VmCapability};
use overdrive_core::guest_transport::NodeVsockPortConfig;
use overdrive_core::traits::vmm::Vmm;
use overdrive_core::vm::beacon::BeaconIo;
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::{SimDataplaneComposer, SimKek};

/// Which dataplane composer the boot uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataplaneChoice {
    /// `SimDataplaneComposer` over a fresh `SimDataplane`, `host_ipv4`
    /// `127.0.0.1` (no XDP attach, no veth provisioning).
    Sim,
    /// The production `HostDataplaneComposer` from `NodeAdapters::host`: the
    /// real `EbpfDataplane` and its ADR-0061 veth pair.
    Host,
}

/// The VM driver's driven ports a fixture supplies in place of the host
/// pair (DG-4: the VMM and its beacon travel together).
pub struct VmPorts {
    /// The VMM.
    pub vmm: Arc<dyn Vmm>,
    /// The beacon intake's I/O paired with it.
    pub beacon: Arc<dyn BeaconIo>,
}

/// `ServeArgs` for an in-process test `serve`: the requested bind, the two
/// directories, and kernel-assigned vsock listener ports (PORTS-CONFIG).
pub const fn serve_args(bind: SocketAddr, data_dir: PathBuf, config_dir: PathBuf) -> ServeArgs {
    ServeArgs { bind, data_dir, config_dir, vsock_ports: NodeVsockPortConfig::kernel_assigned() }
}

/// Boots `serve` with `dataplane` and the host VM ports (`Optional`
/// capability: a node whose VMM probe fails composes no `Vm` driver, as
/// `overdrive serve` does).
///
/// # Errors
///
/// What `run_with_adapters` returns.
pub async fn serve(args: ServeArgs, dataplane: DataplaneChoice) -> Result<ServeHandle, CliError> {
    serve_composed(args, dataplane, None).await
}

/// Boots `serve` with `dataplane` and the fixture's VM ports, `Required`
/// capability: a failed VMM or cgroup-accounting probe refuses the boot
/// (`vmm.probe`).
///
/// # Errors
///
/// What `run_with_adapters` returns.
pub async fn serve_with_vm(
    args: ServeArgs,
    dataplane: DataplaneChoice,
    vm: VmPorts,
) -> Result<ServeHandle, CliError> {
    serve_composed(args, dataplane, Some(vm)).await
}

#[expect(
    clippy::result_large_err,
    reason = "the bundle builder returns the boot's typed `ControlPlaneError` unboxed, as `run_server` does"
)]
async fn serve_composed(
    args: ServeArgs,
    dataplane: DataplaneChoice,
    vm: Option<VmPorts>,
) -> Result<ServeHandle, CliError> {
    overdrive_testing::forwarder_module::expect_forwarder_module();
    run_with_adapters(args, Arc::new(RealCgroupFs::new()), move |config, fs| {
        let mut adapters = NodeAdapters::host(config, fs, Arc::new(SimKek::for_boot()))?;
        if dataplane == DataplaneChoice::Sim {
            adapters.boot.dataplane = Arc::new(SimDataplaneComposer::new(
                Arc::new(SimDataplane::new()),
                Ipv4Addr::LOCALHOST,
            ));
        }
        if let Some(VmPorts { vmm, beacon }) = vm {
            adapters.vm.vmm = vmm;
            adapters.vm.beacon = beacon;
            adapters.vm.capability = VmCapability::Required;
        }
        Ok(adapters)
    })
    .await
}
