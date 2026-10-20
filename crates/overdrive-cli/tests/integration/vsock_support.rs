//! Shared support of the vsock native-metal lanes driven through
//! `overdrive serve` + `overdrive deploy` (GH #295): the walking skeleton
//! (`vsock_walking_skeleton`, P6-S13), the native fault matrix
//! (`vsock_fault_matrix`, P6-S14) and the density and fail-closed cells
//! (`vsock_density`, P6-S15 / V-9).
//!
//! **Production composition, no test wiring.** Every lane boots the
//! in-process `overdrive serve` through `serve_fixture::serve` (`DataplaneChoice::Host`) — the same
//! `run_server` composition `overdrive serve` runs, the real `EbpfDataplane`
//! and the mTLS and guest-network owners composed, only the KEK injected
//! (`SimKek`; the kernel keyring refuses a cold boot) — and submits VM
//! workloads through the public deploy handler. Nothing here installs a
//! rule, binds a listener, programs a map or supplies an address that
//! `run_server` does not (CLAUDE.md § "Build vertical slices through
//! production entry points").
//!
//! **Lane setup.** Run `cargo xtask guest-kit-prepare` (under `cargo xtask
//! metal run --`) before these lanes: it builds `ovd_fwd.ko` for the running
//! kernel and the static guest program. `GuestKit::provision` builds
//! nothing; it refuses `LaneNotPrepared`, naming that command, when they are
//! missing.
//!
//! **Guests.** Each VM spec names the guest kit's kernel and a guest-kit root
//! filesystem (`overdrive_testing::guest_kit`, EVMOD-GUEST-KIT: the kit's
//! `overdrive-init` and its `ovd_fwd.ko`); the operator command is the guest
//! application `overdrive-guest-peer` (`overdrive_testing::guest_app`), an
//! ordinary identity-unaware program that opens plain TCP and UDP sockets.
//!
//! **The far side.** A `RemotePeer`: TCP and UDP servers in their own
//! network namespace on a leased test `/24`, off-host from the node's point
//! of view; the source address of every connection it accepts is the
//! node's veth address, which proves the guest's bytes crossed the host's
//! forwarder.
//!
//! **Observation.** Operator surfaces (`overdrive workload describe`), the
//! remote peer's own sockets, the guest console log the VMM writes for the
//! allocation (the guest application's stdout), and the kernel's process
//! table (the VMM process, its `--vsock` argument, `/dev/vhost-vsock`
//! holders).

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(dead_code, reason = "shared by three kvm-gated lane modules")]

use std::net::{SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::time::Duration;

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::ServeHandle;
use overdrive_cli::commands::workload::{DescribeArgs, describe};
use overdrive_control_plane::api::{AllocStateWire, AllocStatusRowBody};
use overdrive_core::id::AllocationId;
use overdrive_core::vm::config::VmRunDir;
use overdrive_testing::guest_app::GUEST_PEER_GUEST_PATH;
use overdrive_testing::guest_kit::{GuestImage, GuestImageSpec, GuestKit};
use tempfile::TempDir;

use super::vm_walking_skeleton::{
    config_path, shared_staging_root, spawn_vm_server_mtls_composed, write_toml,
};

/// Where the VM driver keeps each allocation's run directory (and the
/// console log the VMM writes there).
const VM_RUN_ROOT: &str = "/run/overdrive/vm";

/// Guest RAM of every lane VM (the kit's stock kernel boots in it).
pub(super) const LANE_VM_MEMORY_BYTES: u64 = 128 * 1024 * 1024;

/// One booted `serve` with the guest kit and a workload image.
pub(super) struct Lane {
    pub(super) kit: GuestKit,
    pub(super) image: GuestImage,
    handle: Option<ServeHandle>,
    pub(super) server_tmp: TempDir,
}

impl Lane {
    /// Provisions the guest kit, boots `serve` in its production
    /// composition, and builds the workload image `name` next to its data
    /// directory (same filesystem: the VM driver reflinks the rootfs master
    /// into the data directory).
    pub(super) async fn boot(name: &str) -> Self {
        let kit = GuestKit::provision(&shared_staging_root())
            .expect("precondition: the guest kit provisions (kernel, ovd_fwd.ko, guest programs)");
        let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
        let image = kit
            .image(server_tmp.path(), name, &GuestImageSpec::workload())
            .expect("the workload image");
        Self { kit, image, handle: Some(handle), server_tmp }
    }

    /// The operator config the deploy and describe handlers read.
    pub(super) fn cfg(&self) -> PathBuf {
        config_path(self.server_tmp.path())
    }

    /// Deploys a VM Job whose operator command is the guest application
    /// with `args`; returns the workload id.
    pub(super) async fn deploy_job(&self, id: &str, args: &[&str]) -> String {
        let args_toml = args.iter().map(|a| format!("\"{a}\"")).collect::<Vec<_>>().join(", ");
        let body = format!(
            "[job]\nid = \"{id}\"\n\n[vm]\ncommand = \"{GUEST_PEER_GUEST_PATH}\"\nargs = [{args_toml}]\n\
             kernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = 500\n\
             memory_bytes = {LANE_VM_MEMORY_BYTES}\n",
            self.kit.kernel_path.display(),
            self.image.rootfs.display(),
        );
        let spec = write_toml(self.server_tmp.path(), &format!("{id}.toml"), &body);
        deploy(DeployArgs { spec, config_path: self.cfg() })
            .await
            .unwrap_or_else(|e| panic!("deploy {id}: {e}"))
            .workload_id
    }

    /// Stops a workload through the operator verb.
    pub(super) async fn stop(&self, id: &str) {
        stop(StopArgs { id: id.to_owned(), config_path: self.cfg() })
            .await
            .unwrap_or_else(|e| panic!("stop {id}: {e}"));
    }

    /// The workload's allocation rows.
    pub(super) async fn rows(&self, id: &str) -> Vec<AllocStatusRowBody> {
        describe(DescribeArgs { id: id.to_owned(), config_path: self.cfg() })
            .await
            .unwrap_or_else(|e| panic!("describe {id}: {e}"))
            .snapshot
            .rows
    }

    /// Polls until `done` holds over the rows, or panics with the last rows.
    pub(super) async fn wait_rows(
        &self,
        id: &str,
        bound: Duration,
        what: &str,
        done: impl Fn(&[AllocStatusRowBody]) -> bool,
    ) -> Vec<AllocStatusRowBody> {
        let deadline = tokio::time::Instant::now() + bound;
        loop {
            let rows = self.rows(id).await;
            if done(&rows) {
                return rows;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "workload {id}: {what} not reached within {bound:?}; rows: {rows:#?}"
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// The first allocation of `id` once it is Running.
    pub(super) async fn running_alloc(&self, id: &str, bound: Duration) -> AllocationId {
        let rows = self
            .wait_rows(id, bound, "a Running allocation", |rows| {
                rows.iter().any(|r| r.state == AllocStateWire::Running)
            })
            .await;
        let row = rows.iter().find(|r| r.state == AllocStateWire::Running).expect("a Running row");
        AllocationId::new(&row.alloc_id).expect("a valid allocation id")
    }

    /// `serve` dies without a graceful drain (the serve lifetime port's
    /// killed mode): every in-process task, socket and descriptor of the
    /// owner is released, as the kernel releases them at a `SIGKILL`, and
    /// the VMMs survive.
    pub(super) async fn kill_serve(&mut self) {
        let handle = self.handle.take().expect("serve is running");
        super::serve_lifetime_support::kill_serve_owner(handle).await.expect("the killed mode");
    }

    /// Boots `serve` again on the same data and config directories (the
    /// supervisor's restart after a crash).
    pub(super) async fn reboot_serve(&mut self) {
        assert!(self.handle.is_none(), "serve is down");
        let args = super::serve_fixture::serve_args(
            "127.0.0.1:0".parse::<SocketAddr>().expect("bind addr"),
            self.server_tmp.path().join("data"),
            self.server_tmp.path().join("conf"),
        );
        let handle = super::serve_fixture::serve(args, super::serve_fixture::DataplaneChoice::Host)
            .await
            .expect("serve boots again on the same directories");
        self.handle = Some(handle);
    }

    /// Graceful shutdown.
    pub(super) async fn shutdown(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.shutdown().await.expect("clean shutdown");
        }
    }
}

/// A row's allocation id.
pub(super) fn alloc_of(row: &AllocStatusRowBody) -> AllocationId {
    AllocationId::new(&row.alloc_id).expect("a valid allocation id")
}

/// The pid and NUL-joined argv of the live `cloud-hypervisor` process of
/// `alloc` (matched by its run directory in the argv), or `None`.
pub(super) fn hypervisor_of(alloc: &AllocationId) -> Option<(u32, String)> {
    let needle =
        VmRunDir::for_alloc(Path::new(VM_RUN_ROOT), alloc).path().to_string_lossy().into_owned();
    for entry in std::fs::read_dir("/proc").expect("read /proc").flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
        let argv0 = cmdline.split(|b| *b == 0).next().unwrap_or(&[]);
        if !String::from_utf8_lossy(argv0).ends_with("cloud-hypervisor") {
            continue;
        }
        let argv = String::from_utf8_lossy(&cmdline).replace('\0', " ");
        if argv.contains(&needle) && process_alive(pid) {
            return Some((pid, argv));
        }
    }
    None
}

/// The CID of a VMM's one `--vsock cid=<cid>,backend=vhost-kernel` device.
pub(super) fn vsock_cid(argv: &str) -> u32 {
    let at = argv.find("cid=").unwrap_or_else(|| panic!("no --vsock cid= in {argv}"));
    argv[at + 4..]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|digits| digits.parse().ok())
        .unwrap_or_else(|| panic!("a decimal CID in {argv}"))
}

/// Whether `pid` is a live (not zombie) process.
pub(super) fn process_alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| stat.rsplit_once(") ").map(|(_, rest)| !rest.starts_with('Z')))
        .unwrap_or(false)
}

/// Waits up to `bound` for `pid` to end.
pub(super) async fn wait_process_gone(pid: u32, bound: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + bound;
    while process_alive(pid) {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    true
}

/// The allocation's guest console so far.
pub(super) fn console_of(alloc: &AllocationId) -> String {
    let path = VmRunDir::for_alloc(Path::new(VM_RUN_ROOT), alloc).console_log();
    std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

/// Waits up to `bound` for the allocation's console to satisfy `done`.
pub(super) async fn wait_console(
    alloc: &AllocationId,
    bound: Duration,
    done: impl Fn(&str) -> bool,
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + bound;
    loop {
        let console = console_of(alloc);
        if done(&console) {
            return Ok(console);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(console);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Formats an address for a guest application argument.
pub(super) fn arg(addr: SocketAddrV4) -> String {
    addr.to_string()
}
