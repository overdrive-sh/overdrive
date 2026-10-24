//! The guest-kit Cloud Hypervisor microVM fixture (GH #295, T-1, user ruling
//! 2026-10-18; EVMOD-GUEST-KIT, ADR-0206).
//!
//! A **guest kit** is one kernel image, the `ovd_fwd.ko` built against that
//! kernel, and the guest programs that run on it. For the native-metal
//! Tier-3 lane the kernel is the metal host's own stock Linux 7.0 kernel —
//! the one [`crate::vm_fixture::VmFixture`] stages — and the module built for
//! it by EVMOD-BUILD (`<workspace>/target/kmod/<kernel-release>/ovd_fwd.ko`).
//!
//! **Lane setup.** This fixture builds nothing. The lane's declared setup
//! step, `cargo xtask guest-kit-prepare` (run before `nextest` under
//! `cargo xtask metal run --`), builds the module, the static scripted peer
//! and the in-guest test binaries and leaves them at the contract paths its
//! module documents; [`GuestKit::provision`] and
//! [`crate::guest_lane::InGuestTests::prepared`] only check that each is
//! there and refuse with [`GuestKitError::LaneNotPrepared`] naming that
//! command when one is not. A nested `cargo` inside a test body would make a
//! build failure a test failure and contend for the target-dir lock the
//! running `nextest` holds.
//!
//! This module provides three things. The microVM is launched through the
//! production `overdrive-host` `CloudHypervisorVmm` (`Vmm::create` over a
//! `VmConfig`, U-4 (a)): that adapter is the one renderer of the
//! cloud-hypervisor command line, so the kit's VM runs under the production
//! launch argument (`cid=<cid>,backend=vhost-kernel`, the VMM opening
//! `/dev/vhost-vsock` itself, § *VMM backend contract*), confinement and
//! exit semantics, and the kit carries no cloud-hypervisor argv of its own.
//!
//! - [`GuestKit::provision`]: the kernel, the module prepared for it, the
//!   static `overdrive-init` and the prepared static scripted peer
//!   `overdrive-guest-peer` (this crate's `bin/guest_peer.rs`).
//! - [`GuestKit::image`]: an ext4 root filesystem with the test-only scripted
//!   peer as the module-loading PID 1. For [`GuestInit::OverdriveInit`] it
//!   loads the stock dummy-link fixture prerequisite and `exec`s the staged
//!   production init, which remains PID 1 and performs its own forwarder
//!   module validation/load. [`GuestInit::ScriptedPeer`] stays in the peer.
//!   Images also carry the guest program, vsock modules, stock dummy module,
//!   and — when asked — `ovd_fwd.ko` and the vsock loopback transport.
//! - [`GuestKit::boot`] / [`GuestKit::boot_scripted`]: a VMM at a CID from a
//!   registered [`cid_bands`] band, its serial console captured to a file,
//!   ended by [`GuestKitVm::kill`] (the VMM's exit is the vhost device's
//!   release, `drivers/vhost/vsock.c:721-750 @ v7.0`) or on drop.
//!
//! # Lane
//!
//! `integration-tests,kvm-tests` on the native x86_64 metal host through
//! `cargo xtask metal run --`, as root (`mknod`, the VMM, the vhost device).
//! Consumers run in the `host-kernel-shared` nextest group (the VMM, the
//! vhost device and the guest CIDs are node-global). The host vsock
//! listeners these lanes bind are kernel-assigned (PORTS-CONFIG, ADR-0218),
//! so they never collide with another lane's.
//!
//! Every step that can fail returns a named [`GuestKitError`]; nothing is
//! skipped silently and no partially staged image is returned.

#![cfg(target_os = "linux")]
#![allow(clippy::doc_markdown, reason = "kernel and file names in prose")]

use std::fs;
use std::io;
use std::num::NonZeroU8;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use overdrive_core::AllocationId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::guest_transport::{GUEST_FORWARDER_MODULE_PATH, GuestCid};
use overdrive_core::traits::vmm::{VmControl, Vmm, VmmExit};
use overdrive_core::vm::config::{
    Gid, HostArch, KernelCmdline, KernelImage, MemoryPlan, OVERDRIVE_VMM_UID, RootfsPlan, VmConfig,
    VmConfinement, VmRunDir, VmmIdentity,
};
use overdrive_host::CloudHypervisorVmm;

use crate::guest_app::GUEST_PEER_GUEST_PATH;
use crate::guest_peer::{CONTROL_PORT_PARAM, GuestPeer, GuestPeerControl, GuestPeerError};
use crate::vm_fixture::{self, VmFixture, VmFixtureError};

/// Where an image carries the vsock loopback transport module when asked
/// ([`GuestImageSpec::loopback_module`]). Production guests do not load it;
/// the in-guest test lane's launcher does (`guest_lane`).
pub const LOOPBACK_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/vsock_loopback.ko";
/// The stock kernel dummy-link driver required before an owner cell invokes
/// production guest transport setup.
pub const DUMMY_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/dummy.ko";
/// The stock PRIO scheduler used by the scripted-guest Fast Open fixture.
pub const PRIO_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/sch_prio.ko";
/// The stock NETEM scheduler used by the scripted-guest Fast Open fixture.
pub const NETEM_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/sch_netem.ko";
/// The host's `tc` binary staged into scripted guest images with its dynamic
/// library closure.
pub const TC_GUEST_PATH: &str = "/usr/sbin/tc";
/// The production init staged behind the guest-kit's module-loading PID 1.
pub const OVERDRIVE_INIT_GUEST_PATH: &str = "/lib/overdrive/overdrive-init";
/// Kernel parameter asking the test-only PID 1 to load fixture prerequisites
/// and replace itself with production `overdrive-init`.
pub const TEST_INIT_PARAM: &str = "overdrive.testinit=";

/// The name of the scripted peer binary target of this crate.
pub const GUEST_PEER_BIN: &str = "overdrive-guest-peer";

/// How long a scripted guest has to boot and dial its control port.
pub const SCRIPTED_BOOT_BOUND: Duration = Duration::from_secs(60);

/// Every failure of the guest kit, by step.
#[derive(Debug, thiserror::Error)]
pub enum GuestKitError {
    /// The shared VM fixture (KVM, the VMM binary, the kernel, the base
    /// rootfs) failed.
    #[error(transparent)]
    Fixture(#[from] VmFixtureError),
    /// The kit's provisioning lock could not be taken.
    #[error("could not take the guest kit lock {path}: {source}")]
    Lock {
        /// The lock file.
        path: PathBuf,
        /// The error.
        #[source]
        source: io::Error,
    },
    /// A build artefact the lane's setup step produces is missing: the
    /// lane was not prepared (or was prepared for another kernel).
    #[error(
        "the guest-kit lane is not prepared: no {artifact} at {path}; run `cargo xtask \
         guest-kit-prepare` (under `cargo xtask metal run --`) before the lane"
    )]
    LaneNotPrepared {
        /// The artefact.
        artifact: String,
        /// Where the setup step leaves it.
        path: PathBuf,
    },
    /// A file could not be staged into an image.
    #[error("staging the guest image: {detail}: {source}")]
    Staging {
        /// What was being staged.
        detail: String,
        /// The error.
        #[source]
        source: io::Error,
    },
    /// A dynamic binary's shared libraries could not be listed.
    #[error("listing the shared libraries of {binary}: {detail}")]
    SharedLibraries {
        /// The binary.
        binary: PathBuf,
        /// Why.
        detail: String,
    },
    /// `mkfs.ext4` failed.
    #[error("mkfs.ext4 of {image} failed (exit {status:?}): {stderr}")]
    ImageBuild {
        /// The image.
        image: PathBuf,
        /// The exit status.
        status: Option<i32>,
        /// Its stderr.
        stderr: String,
    },
    /// The VMM could not be spawned.
    #[error("spawning cloud-hypervisor: {source}")]
    Launch {
        /// The error.
        #[source]
        source: io::Error,
    },
    /// The VMM exited before the guest was usable.
    #[error(
        "the VMM at CID {cid} exited ({status}) before the guest was usable; console tail:\n{console_tail}\nVMM stderr:\n{stderr}"
    )]
    VmmExited {
        /// The CID.
        cid: u32,
        /// The exit status.
        status: ExitStatus,
        /// The guest console's tail.
        console_tail: String,
        /// The VMM's stderr.
        stderr: String,
    },
    /// A scripted guest's control port could not be bound.
    #[error("binding the scripted guest's control port {port}: {source}")]
    ControlBind {
        /// The port.
        port: u32,
        /// The error.
        #[source]
        source: io::Error,
    },
    /// A scripted guest did not dial its control port in time.
    #[error(
        "the scripted guest at CID {cid} never dialled its control port; console tail:\n{console_tail}"
    )]
    PeerNeverConnected {
        /// The CID.
        cid: u32,
        /// The guest console's tail.
        console_tail: String,
    },
    /// Talking to a scripted guest failed.
    #[error(transparent)]
    Peer(#[from] GuestPeerError),
    /// Waiting for, or killing, the VMM failed.
    #[error("waiting for the VMM at CID {cid}: {source}")]
    Wait {
        /// The CID.
        cid: u32,
        /// The error.
        #[source]
        source: io::Error,
    },
}

/// Registered CID bands of the guest-kit lanes: one per consuming test
/// file, so no two files ever launch a VMM on one CID, and none collides
/// with the CIDs a booted `overdrive serve` assigns itself.
///
/// A booted `serve` leases next-fit from `GUEST_CID_BASE` (M-9); the
/// production band below `PRODUCTION_BAND_WIDTH` is reserved for it (the
/// `overdrive-host` `kvm-tests` files use offsets `0x0300..=0x0405` inside it,
/// never alongside a booted `serve`). Adding a file = adding a band here.
pub mod cid_bands {
    use overdrive_core::guest_transport::{GUEST_CID_BASE, GUEST_CID_SPAN, GuestCid};

    /// Offsets `0..PRODUCTION_BAND_WIDTH` belong to `serve`'s own leases.
    pub const PRODUCTION_BAND_WIDTH: u32 = 0x1000;

    /// A named run of CID offsets.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CidBand {
        /// The consuming file.
        pub name: &'static str,
        /// The first offset from `GUEST_CID_BASE`.
        pub first_offset: u32,
        /// How many CIDs.
        pub width: u32,
    }

    impl CidBand {
        /// The raw CID of the band's `n`-th slot; panics past the band.
        #[must_use]
        pub const fn raw(&self, n: u32) -> u32 {
            assert!(n < self.width, "past the end of the CID band");
            GUEST_CID_BASE + self.first_offset + n
        }

        /// The band's `n`-th CID as a `GuestCid`.
        #[must_use]
        pub fn cid(&self, n: u32) -> GuestCid {
            GuestCid::new(self.raw(n))
                .unwrap_or_else(|e| panic!("a registered band is inside the leased range: {e}"))
        }
    }

    /// `overdrive-fwd/tests/integration/guest_boundary.rs`.
    pub const FWD_GUEST_BOUNDARY: CidBand =
        CidBand { name: "overdrive-fwd guest_boundary", first_offset: 0x2000, width: 16 };
    /// `overdrive-control-plane` `guest_network::flow_io::accept_path_tier3`.
    pub const HOST_FLOW_ACCEPT_PATH: CidBand = CidBand {
        name: "overdrive-control-plane accept_path_tier3",
        first_offset: 0x2010,
        width: 16,
    };
    /// `overdrive-host/tests/integration/vsock_beacon_accept_path.rs`.
    pub const HOST_BEACON_ACCEPT_PATH: CidBand = CidBand {
        name: "overdrive-host vsock_beacon_accept_path",
        first_offset: 0x2020,
        width: 16,
    };
    /// `overdrive-init/tests/integration/in_guest_lane.rs`.
    pub const INIT_IN_GUEST_LANE: CidBand =
        CidBand { name: "overdrive-init in_guest_lane", first_offset: 0x2030, width: 16 };
    /// `overdrive-fwd/tests/integration/unload_in_guest.rs` (V-EV-UNLOAD's
    /// ULP control and unload stress, U-EV-UNLOAD).
    pub const FWD_UNLOAD_IN_GUEST: CidBand =
        CidBand { name: "overdrive-fwd unload_in_guest", first_offset: 0x2040, width: 16 };

    /// Every registered band.
    pub const REGISTERED: [CidBand; 5] = [
        FWD_GUEST_BOUNDARY,
        HOST_FLOW_ACCEPT_PATH,
        HOST_BEACON_ACCEPT_PATH,
        INIT_IN_GUEST_LANE,
        FWD_UNLOAD_IN_GUEST,
    ];

    /// The guard the registry carries: bands are pairwise disjoint, above
    /// the production band, and inside the leased range.
    #[must_use]
    pub fn registry_is_sound(bands: &[CidBand]) -> bool {
        let inside = bands.iter().all(|b| {
            b.width > 0
                && b.first_offset >= PRODUCTION_BAND_WIDTH
                && b.first_offset.checked_add(b.width).is_some_and(|end| end <= GUEST_CID_SPAN)
        });
        let disjoint = bands.iter().enumerate().all(|(i, a)| {
            bands[i + 1..].iter().all(|b| {
                a.first_offset + a.width <= b.first_offset
                    || b.first_offset + b.width <= a.first_offset
            })
        });
        inside && disjoint
    }
}

/// PID 1 of a guest image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestInit {
    /// The production `overdrive-init` (it needs a host beacon, and its
    /// guest owner a host control listener).
    OverdriveInit,
    /// The scripted peer (`overdrive-guest-peer`): no Overdrive owner.
    ScriptedPeer,
}

/// A host file to place in a guest image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestFile {
    /// The file on the host.
    pub host: PathBuf,
    /// Its absolute path in the guest.
    pub guest: PathBuf,
    /// Also stage its ELF interpreter and shared libraries at their host
    /// paths (a dynamically linked test binary).
    pub with_shared_libraries: bool,
}

/// What a guest image carries besides PID 1, the guest program and the
/// vsock transport modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestImageSpec {
    /// PID 1 behavior after fixture module loading.
    pub init: GuestInit,
    /// `ovd_fwd.ko` at `GUEST_FORWARDER_MODULE_PATH` (EVMOD-GUEST-KIT).
    /// Without it the production `overdrive-init` refuses before READY with
    /// `ForwarderModuleMissing` — the image for that refusal's cell.
    pub forwarder_module: bool,
    /// `vsock_loopback.ko` at [`LOOPBACK_MODULE_GUEST_PATH`].
    pub loopback_module: bool,
    /// Further files.
    pub extra: Vec<GuestFile>,
}

impl GuestImageSpec {
    /// The image a `[vm]` spec names: the production `overdrive-init` as
    /// PID 1 and the kit's module.
    #[must_use]
    pub const fn workload() -> Self {
        Self {
            init: GuestInit::OverdriveInit,
            forwarder_module: true,
            loopback_module: false,
            extra: Vec::new(),
        }
    }

    /// A scripted guest with the kit's module (its forwarder usable).
    #[must_use]
    pub const fn scripted() -> Self {
        Self {
            init: GuestInit::ScriptedPeer,
            forwarder_module: true,
            loopback_module: false,
            extra: Vec::new(),
        }
    }
}

/// A built guest root filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestImage {
    /// The ext4 image.
    pub rootfs: PathBuf,
    /// Which PID 1 behavior this image carries.
    pub init: GuestInit,
}

/// A provisioned guest kit.
#[derive(Debug, Clone)]
pub struct GuestKit {
    /// The kernel image (the metal host's own 7.0 kernel, staged).
    pub kernel_path: PathBuf,
    /// Its release (`uname -r`).
    pub kernel_release: String,
    /// `ovd_fwd.ko` built for that release.
    pub forwarder_module: PathBuf,
    /// The confirmed `cloud-hypervisor` binary (the fork, with the
    /// vhost-kernel vsock backend; ADR-0161).
    pub cloud_hypervisor_bin: PathBuf,
    /// The static `overdrive-init`.
    pub init_bin: PathBuf,
    /// The static `overdrive-guest-peer`.
    pub peer_bin: PathBuf,
}

#[clippy::msrv = "1.89"]
fn take_lock(path: &Path) -> Result<fs::File, GuestKitError> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|source| GuestKitError::Lock { path: path.to_path_buf(), source })?;
    file.lock().map_err(|source| GuestKitError::Lock { path: path.to_path_buf(), source })?;
    Ok(file)
}

fn staging(detail: impl Into<String>) -> impl FnOnce(io::Error) -> GuestKitError {
    let detail = detail.into();
    move |source| GuestKitError::Staging { detail, source }
}

impl GuestKit {
    /// Provisions (or re-verifies) the kit under `staging_root`: the shared
    /// VM fixture's preflight and kernel, the static `overdrive-init`, and —
    /// checked, not built — the module prepared for that kernel and the
    /// static scripted peer the lane's setup step left. Concurrency-safe
    /// across processes (one lock for the whole call).
    ///
    /// # Errors
    ///
    /// The first failing step's [`GuestKitError`];
    /// [`GuestKitError::LaneNotPrepared`] when the setup step's artefacts
    /// are missing.
    pub fn provision(staging_root: &Path) -> Result<Self, GuestKitError> {
        let fixture = VmFixture::provision(staging_root)?;
        let _lock = take_lock(&staging_root.join(".guest-kit-lock"))?;
        let kernel_release = vm_fixture::host_kernel_release()?;
        let forwarder_module = prepared_forwarder_module(&kernel_release)?;
        let init_bin = vm_fixture::build_overdrive_init_static()?;
        let peer_bin = prepared_guest_peer()?;
        Ok(Self {
            kernel_path: fixture.kernel_path,
            kernel_release,
            forwarder_module,
            cloud_hypervisor_bin: fixture.cloud_hypervisor_bin,
            init_bin,
            peer_bin,
        })
    }

    /// Builds the image `name` under `dir` (which should be on the
    /// reflink-capable staging root when a `serve` clones it).
    ///
    /// # Errors
    ///
    /// [`GuestKitError::Staging`], [`GuestKitError::SharedLibraries`] or
    /// [`GuestKitError::ImageBuild`].
    pub fn image(
        &self,
        dir: &Path,
        name: &str,
        spec: &GuestImageSpec,
    ) -> Result<GuestImage, GuestKitError> {
        let stage = dir.join(format!("{name}-stage"));
        if stage.exists() {
            fs::remove_dir_all(&stage).map_err(staging(format!("clearing {}", stage.display())))?;
        }
        for sub in [
            "proc",
            "sys",
            "dev",
            "tmp",
            "sbin",
            "etc",
            "run",
            "lib/overdrive",
            "lib/overdrive-testing/modules",
        ] {
            fs::create_dir_all(stage.join(sub)).map_err(staging(format!("creating /{sub}")))?;
        }
        let init = &self.peer_bin;
        install(&stage, init, Path::new(vm_fixture::GUEST_INIT_SBIN_PATH))?;
        install(&stage, init, Path::new(vm_fixture::GUEST_INIT_PATH))?;
        if spec.init == GuestInit::OverdriveInit {
            install(&stage, &self.init_bin, Path::new(OVERDRIVE_INIT_GUEST_PATH))?;
        }
        install(&stage, &self.peer_bin, Path::new(GUEST_PEER_GUEST_PATH))?;
        vm_fixture::stage_vsock_modules(&stage, &self.kernel_release)?;
        let dummy =
            host_module_file(self, dir, "drivers/net/dummy", Path::new(DUMMY_MODULE_GUEST_PATH))?;
        install(&stage, &dummy.host, &dummy.guest)?;
        if spec.init == GuestInit::ScriptedPeer {
            for (relative, guest) in [
                ("net/sched/sch_prio", PRIO_MODULE_GUEST_PATH),
                ("net/sched/sch_netem", NETEM_MODULE_GUEST_PATH),
            ] {
                let module = host_module_file(self, dir, relative, Path::new(guest))?;
                install(&stage, &module.host, &module.guest)?;
            }
            let tc = Path::new(TC_GUEST_PATH);
            install(&stage, tc, tc)?;
            for library in shared_libraries(tc)? {
                install(&stage, &library, &library)?;
            }
        }
        if spec.forwarder_module {
            install(&stage, &self.forwarder_module, Path::new(GUEST_FORWARDER_MODULE_PATH))?;
        }
        if spec.loopback_module {
            stage_loopback_module(&stage, &self.kernel_release)?;
        }
        for file in &spec.extra {
            install(&stage, &file.host, &file.guest)?;
            if file.with_shared_libraries {
                for library in shared_libraries(&file.host)? {
                    install(&stage, &library, &library)?;
                }
            }
        }
        vm_fixture::mknod_char_device(&stage.join("dev/console"), 5, 1, 0o600)?;
        vm_fixture::mknod_char_device(&stage.join("dev/null"), 1, 3, 0o666)?;
        let rootfs = dir.join(format!("{name}.ext4"));
        mkfs(&stage, &rootfs)?;
        Ok(GuestImage { rootfs, init: spec.init })
    }

    /// Launches a VMM over `boot.image` at `boot.cid`, through the
    /// production `CloudHypervisorVmm` (`Vmm::create` over a `VmConfig`).
    ///
    /// # Errors
    ///
    /// [`GuestKitError::Launch`] when the VMM cannot be launched.
    pub fn boot(&self, boot: &GuestBoot<'_>) -> Result<GuestKitVm, GuestKitError> {
        fs::create_dir_all(boot.run_dir)
            .map_err(staging(format!("creating {}", boot.run_dir.display())))?;
        let launch = |detail: String| GuestKitError::Launch { source: io::Error::other(detail) };
        let alloc = AllocationId::new(&format!("kit-{}", boot.cid))
            .map_err(|e| launch(format!("allocation id: {e}")))?;
        let cid = GuestCid::new(boot.cid).map_err(|e| launch(format!("guest CID: {e}")))?;
        // Cloud Hypervisor's pathname Unix socket is limited by `sun_path`.
        // Keep only per-launch runtime artifacts under this bounded path; the
        // rootfs clone and its index stay under the caller's reflink-capable
        // `boot.run_dir` below.
        let runtime = WorkDir::create(Path::new("/run"), "ovd-gk")?;
        let header = vm_fixture::read_header(&self.kernel_path)
            .map_err(staging(format!("reading {}", self.kernel_path.display())))?;
        let kernel = KernelImage::validate(self.kernel_path.clone(), HostArch::X86_64, &header)
            .map_err(|e| launch(format!("kernel image: {e}")))?;
        let rootfs_bytes = fs::metadata(&boot.image.rootfs)
            .map_err(staging(format!("reading {}", boot.image.rootfs.display())))?
            .len();
        let mut cmdline = KernelCmdline::platform_default(HostArch::X86_64);
        if boot.image.init == GuestInit::OverdriveInit {
            let token = format!("{TEST_INIT_PARAM}{OVERDRIVE_INIT_GUEST_PATH}");
            assert!(cmdline.append_platform_token(&token));
        }
        for token in &boot.cmdline_extra {
            assert!(
                cmdline.append_platform_token(token),
                "a kernel parameter has no whitespace: {token:?}"
            );
        }
        let vcpus =
            NonZeroU8::new(boot.vcpus).unwrap_or_else(|| panic!("a guest has at least one vCPU"));
        let config = VmConfig {
            alloc: alloc.clone(),
            kernel,
            // Keep the production FICLONE target on the caller's
            // reflink-capable staging filesystem; only Unix-socket/runtime
            // artifacts use the short directory above.
            rootfs: RootfsPlan::for_alloc(
                boot.image.rootfs.clone(),
                rootfs_bytes,
                &alloc,
                boot.run_dir,
                &boot.run_dir.join("clone-index"),
            ),
            cmdline,
            memory: MemoryPlan::derive(u64::from(boot.memory_mib) * 1024 * 1024),
            vcpus,
            run_dir: VmRunDir::for_alloc(runtime.path(), &alloc),
            confinement: VmConfinement::confined(confined_identity()?, 1024),
            cgroup_scope: CgroupPath::for_alloc(&alloc),
        };
        fs::create_dir_all(config.run_dir.path())
            .map_err(staging(format!("creating {}", config.run_dir.path().display())))?;
        let console = config.run_dir.console_log();
        let vmm = CloudHypervisorVmm::new().with_binary(self.cloud_hypervisor_bin.clone());
        let launcher = vmm.clone();
        let process = run_on_kit_runtime(async move { launcher.create(&config, cid).await })
            .map_err(|e| launch(e.to_string()))?;
        let exit = Arc::new(Mutex::new(None));
        let recorded = Arc::clone(&exit);
        let mut watch = process.exit;
        kit_runtime().spawn(async move {
            let ended = watch.recv().await.unwrap_or(VmmExit {
                exit_code: None,
                signal: None,
                stderr_tail: None,
            });
            *recorded.lock().unwrap_or_else(PoisonError::into_inner) = Some(ended);
        });
        Ok(GuestKitVm {
            vmm,
            control: Some(process.control),
            exit,
            cid: boot.cid,
            console,
            _runtime: runtime,
        })
    }

    /// Boots a [`GuestInit::ScriptedPeer`] image at `cid` and returns once
    /// its agent has dialled the control port.
    ///
    /// # Errors
    ///
    /// [`GuestKitError::ControlBind`], [`GuestKitError::VmmExited`] or
    /// [`GuestKitError::PeerNeverConnected`].
    pub fn boot_scripted(
        &self,
        image: &GuestImage,
        cid: u32,
        run_dir: &Path,
    ) -> Result<ScriptedGuest, GuestKitError> {
        self.boot_scripted_with(image, cid, run_dir, &ScriptedBootExtra::default())
    }

    /// [`Self::boot_scripted`] with further kernel parameters and the guest's
    /// RAM and vCPUs: the in-guest lanes whose cell reads its own parameters
    /// (a transport token, a script port) from `/proc/cmdline`.
    ///
    /// # Errors
    ///
    /// As [`Self::boot_scripted`].
    pub fn boot_scripted_with(
        &self,
        image: &GuestImage,
        cid: u32,
        run_dir: &Path,
        extra: &ScriptedBootExtra,
    ) -> Result<ScriptedGuest, GuestKitError> {
        // PORTS-CONFIG / GUEST-PORTS: the control listener binds a
        // kernel-assigned port and the guest learns it from its command line,
        // so no scripted guest of any workspace holds a fixed host port.
        let control = GuestPeerControl::bind(crate::vsock::VMADDR_PORT_ANY).map_err(|source| {
            GuestKitError::ControlBind { port: crate::vsock::VMADDR_PORT_ANY, source }
        })?;
        let port = control.port();
        let mut cmdline_extra = vec![format!("{CONTROL_PORT_PARAM}{port}")];
        cmdline_extra.extend(extra.cmdline.iter().cloned());
        let vm = self.boot(&GuestBoot {
            image,
            cid,
            cmdline_extra,
            memory_mib: extra.memory_mib.unwrap_or(256),
            vcpus: extra.vcpus.unwrap_or(2),
            run_dir,
        })?;
        match control.accept_peer(cid, SCRIPTED_BOOT_BOUND) {
            Ok(peer) => Ok(ScriptedGuest { peer: Some(peer), vm }),
            Err(GuestPeerError::NeverConnected { .. }) => {
                if let Some(status) = vm.try_exit() {
                    return Err(vm.exited_error(status));
                }
                Err(GuestKitError::PeerNeverConnected { cid, console_tail: vm.console_tail(4096) })
            }
            Err(other) => Err(other.into()),
        }
    }
}

/// What [`GuestKit::boot_scripted_with`] adds to a scripted boot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptedBootExtra {
    /// Kernel parameters after the scripted peer's own.
    pub cmdline: Vec<String>,
    /// Guest RAM; 256 MiB when `None`.
    pub memory_mib: Option<u32>,
    /// vCPUs; 2 when `None`.
    pub vcpus: Option<u8>,
}

/// One VMM launch.
#[derive(Debug, Clone)]
pub struct GuestBoot<'a> {
    /// The root filesystem (read-write; one boot per image).
    pub image: &'a GuestImage,
    /// The vsock CID (from a registered [`cid_bands`] band).
    pub cid: u32,
    /// Kernel parameters after the platform default
    /// (`KernelCmdline::platform_default`, which carries `panic=1`).
    pub cmdline_extra: Vec<String>,
    /// Guest RAM.
    pub memory_mib: u32,
    /// vCPUs.
    pub vcpus: u8,
    /// The reflink-capable image-work root: the rootfs clone and its index
    /// are created under it. The fixture keeps the API socket and other
    /// per-launch runtime artifacts on a separate bounded private path.
    pub run_dir: &'a Path,
}

/// A running VMM. Dropping it kills the VMM and reaps it.
pub struct GuestKitVm {
    vmm: CloudHypervisorVmm,
    /// `None` once [`Self::kill`] has reaped the VMM.
    control: Option<VmControl>,
    /// The VMM's ending, recorded by the watcher task when it happens.
    exit: Arc<Mutex<Option<VmmExit>>>,
    cid: u32,
    console: PathBuf,
    /// Owns the short runtime path until the VMM has been terminated.
    _runtime: WorkDir,
}

impl std::fmt::Debug for GuestKitVm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestKitVm")
            .field("pid", &self.pid())
            .field("cid", &self.cid)
            .field("console", &self.console)
            .finish_non_exhaustive()
    }
}

impl GuestKitVm {
    /// The VMM's process id, while it is not reaped.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.control.as_ref().map(|control| control.pid)
    }

    /// The vsock CID.
    #[must_use]
    pub const fn cid(&self) -> u32 {
        self.cid
    }

    /// The guest console so far.
    #[must_use]
    pub fn console(&self) -> String {
        fs::read(&self.console)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    }

    /// The last `max` bytes of the console.
    #[must_use]
    pub fn console_tail(&self, max: usize) -> String {
        let console = self.console();
        let start = console.len().saturating_sub(max);
        console.get(start..).unwrap_or(&console).to_owned()
    }

    /// Waits up to `bound` for the console to contain `needle`.
    #[must_use]
    pub fn wait_console(&self, needle: &str, bound: Duration) -> bool {
        let deadline = Instant::now() + bound;
        loop {
            if self.console().contains(needle) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The VMM's exit status if it has exited.
    pub fn try_exit(&self) -> Option<ExitStatus> {
        self.control.as_ref().and_then(|_| self.recorded_exit()).map(|exit| status_of(&exit))
    }

    /// Waits up to `bound` for the VMM to exit on its own.
    ///
    /// # Errors
    ///
    /// `waitpid`'s error.
    pub fn wait_exit(&self, bound: Duration) -> Result<Option<ExitStatus>, GuestKitError> {
        let deadline = Instant::now() + bound;
        loop {
            if let Some(status) = self.try_exit() {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Kills the VMM (`SIGKILL`) and reaps it: the VMM's exit is the last
    /// close of its `/dev/vhost-vsock` file, whose release sweeps every host
    /// socket of the CID (`vhost_vsock_dev_release`,
    /// `drivers/vhost/vsock.c:721-750 @ v7.0`).
    ///
    /// # Errors
    ///
    /// `kill` or `waitpid`'s error.
    pub fn kill(&mut self) -> Result<ExitStatus, GuestKitError> {
        let cid = self.cid;
        let control = self.control.take().ok_or_else(|| GuestKitError::Wait {
            cid,
            source: io::Error::other("the VMM was already reaped"),
        })?;
        let vmm = self.vmm.clone();
        run_on_kit_runtime(async move { vmm.terminate(&control, Duration::ZERO).await })
            .map_err(|e| GuestKitError::Wait { cid, source: io::Error::other(e.to_string()) })?;
        // `terminate` returns once the reaper has the ending; the watcher
        // task records it a moment later.
        let deadline = Instant::now() + VMM_REAP_BOUND;
        loop {
            if let Some(exit) = self.recorded_exit() {
                return Ok(status_of(&exit));
            }
            if Instant::now() >= deadline {
                return Err(GuestKitError::Wait {
                    cid,
                    source: io::Error::other("the VMM's ending was not recorded"),
                });
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn recorded_exit(&self) -> Option<VmmExit> {
        self.exit.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn exited_error(&self, status: ExitStatus) -> GuestKitError {
        GuestKitError::VmmExited {
            cid: self.cid,
            status,
            console_tail: self.console_tail(4096),
            stderr: self.recorded_exit().and_then(|exit| exit.stderr_tail).unwrap_or_default(),
        }
    }
}

impl Drop for GuestKitVm {
    fn drop(&mut self) {
        if let Some(control) = self.control.take() {
            let vmm = self.vmm.clone();
            let _ =
                run_on_kit_runtime(async move { vmm.terminate(&control, Duration::ZERO).await });
        }
    }
}

/// How long the watcher task has to record a reaped VMM's ending.
const VMM_REAP_BOUND: Duration = Duration::from_secs(10);

/// The kit's own runtime: it drives the production launcher's reaper and the
/// exit watch behind the kit's synchronous surface (one worker thread).
fn kit_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("guest-kit-vmm")
            .enable_all()
            .build()
            .unwrap_or_else(|e| panic!("the guest kit's runtime builds: {e}"))
    })
}

/// Runs `future` on the kit's runtime and waits for it from any thread, in
/// or out of another runtime's context.
fn run_on_kit_runtime<T: Send + 'static>(
    future: impl std::future::Future<Output = T> + Send + 'static,
) -> T {
    let (sender, receiver) = std::sync::mpsc::channel();
    kit_runtime().spawn(async move {
        let _ = sender.send(future.await);
    });
    receiver.recv().unwrap_or_else(|_| panic!("the guest kit's runtime dropped a VMM task"))
}

/// A VMM's ending as the wait status a child process would report.
fn status_of(exit: &VmmExit) -> ExitStatus {
    let raw = match (exit.signal, exit.exit_code) {
        (Some(signal), _) => i32::from(signal),
        (None, Some(code)) => code << 8,
        (None, None) => 0,
    };
    ExitStatus::from_raw(raw)
}

/// The identity the production launch drops the VMM to: the production VMM
/// numeric uid/gid and the supplementary groups that own `/dev/kvm` and
/// `/dev/vhost-vsock` (the kernel's device nodes decide who may open them;
/// no device group is assumed).
fn confined_identity() -> Result<VmmIdentity, GuestKitError> {
    let group_of = |device: &str| {
        fs::metadata(device).map(|m| Gid::new(m.gid())).map_err(staging(format!("stat {device}")))
    };
    let kvm = group_of("/dev/kvm")?;
    let vhost = group_of("/dev/vhost-vsock")?;
    let mut supplementary = vec![kvm];
    if vhost != kvm {
        supplementary.push(vhost);
    }
    Ok(VmmIdentity { uid: OVERDRIVE_VMM_UID, gid: Gid::new(OVERDRIVE_VMM_UID), supplementary })
}

/// A scripted guest: its VMM and its agent's control connection.
#[derive(Debug)]
pub struct ScriptedGuest {
    peer: Option<GuestPeer>,
    /// The VMM.
    pub vm: GuestKitVm,
}

impl ScriptedGuest {
    /// The agent.
    ///
    /// # Panics
    ///
    /// After [`Self::take_peer`].
    pub fn peer(&mut self) -> &mut GuestPeer {
        self.peer.as_mut().unwrap_or_else(|| panic!("the scripted guest's agent was taken"))
    }

    /// Takes the agent (for the consuming calls `crash_kernel`, `power_off`).
    ///
    /// # Panics
    ///
    /// When already taken.
    pub fn take_peer(&mut self) -> GuestPeer {
        self.peer.take().unwrap_or_else(|| panic!("the scripted guest's agent was taken"))
    }
}

fn install(stage: &Path, host: &Path, guest: &Path) -> Result<(), GuestKitError> {
    let dest = stage.join(guest.strip_prefix("/").unwrap_or(guest));
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(staging(format!("creating {}", parent.display())))?;
    }
    fs::copy(host, &dest).map_err(staging(format!(
        "copying {} to {}",
        host.display(),
        guest.display()
    )))?;
    let executable = fs::metadata(host).map(|m| {
        use std::os::unix::fs::PermissionsExt as _;
        m.permissions().mode() & 0o111 != 0
    });
    if executable.map_err(staging(format!("reading {}", host.display())))?
        || guest.starts_with("/sbin")
    {
        vm_fixture::set_executable(&dest)?;
    }
    Ok(())
}

/// The running kernel's module `relative` (under
/// `/lib/modules/<release>/kernel/`, without the `.ko` suffix, e.g.
/// `net/tls/tls`) as an image file at `guest`.
///
/// It is decompressed into `dir` when the host ships it zstd-compressed (a
/// guest's `finit_module` takes it uncompressed). For an in-guest cell that needs a module the kit
/// does not stage (the ULP control's `tls`); the guest program's
/// `run-with-modules` loads it.
///
/// # Errors
///
/// [`GuestKitError::Staging`] naming the module when the host has neither
/// form, or when `zstd` fails.
pub fn host_module_file(
    kit: &GuestKit,
    dir: &Path,
    relative: &str,
    guest: &Path,
) -> Result<GuestFile, GuestKitError> {
    let base = format!("/lib/modules/{}/kernel/{relative}", kit.kernel_release);
    let compressed = PathBuf::from(format!("{base}.ko.zst"));
    let plain = PathBuf::from(format!("{base}.ko"));
    let host = if compressed.exists() {
        let name = relative.replace('/', "-");
        let out_path = dir.join(format!("{name}.ko"));
        let out = Command::new("zstd")
            .args(["-d", "-f", "-q"])
            .arg(&compressed)
            .arg("-o")
            .arg(&out_path)
            .output()
            .map_err(staging(format!("spawning zstd for {}", compressed.display())))?;
        if !out.status.success() {
            return Err(GuestKitError::Staging {
                detail: format!("zstd -d {}", compressed.display()),
                source: io::Error::other(String::from_utf8_lossy(&out.stderr).into_owned()),
            });
        }
        out_path
    } else if plain.exists() {
        plain
    } else {
        return Err(GuestKitError::Staging {
            detail: format!(
                "the running kernel ships no module {relative} (neither {} nor {})",
                compressed.display(),
                plain.display()
            ),
            source: io::Error::from(io::ErrorKind::NotFound),
        });
    };
    Ok(GuestFile { host, guest: guest.to_path_buf(), with_shared_libraries: false })
}

fn stage_loopback_module(stage: &Path, release: &str) -> Result<(), GuestKitError> {
    let dir = format!("/lib/modules/{release}/kernel/net/vmw_vsock");
    let dest = stage.join(LOOPBACK_MODULE_GUEST_PATH.trim_start_matches('/'));
    let compressed = PathBuf::from(format!("{dir}/vsock_loopback.ko.zst"));
    let plain = PathBuf::from(format!("{dir}/vsock_loopback.ko"));
    if compressed.exists() {
        let out = Command::new("zstd")
            .args(["-d", "-f", "-q"])
            .arg(&compressed)
            .arg("-o")
            .arg(&dest)
            .output()
            .map_err(staging("spawning zstd for vsock_loopback.ko"))?;
        if !out.status.success() {
            return Err(GuestKitError::Staging {
                detail: format!("zstd -d {}", compressed.display()),
                source: io::Error::other(String::from_utf8_lossy(&out.stderr).into_owned()),
            });
        }
        Ok(())
    } else {
        fs::copy(&plain, &dest).map(drop).map_err(staging(format!(
            "copying {} (the guest kernel needs CONFIG_VSOCKETS_LOOPBACK=m)",
            plain.display()
        )))
    }
}

/// The ELF interpreter and shared libraries `ldd` lists for `binary`.
fn shared_libraries(binary: &Path) -> Result<Vec<PathBuf>, GuestKitError> {
    let out =
        Command::new("ldd").arg(binary).output().map_err(|e| GuestKitError::SharedLibraries {
            binary: binary.to_path_buf(),
            detail: format!("spawning ldd: {e}"),
        })?;
    let text = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        return Err(GuestKitError::SharedLibraries {
            binary: binary.to_path_buf(),
            detail: format!("ldd exited {:?}: {text}", out.status.code()),
        });
    }
    let mut libraries = Vec::new();
    for line in text.lines() {
        // `libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x...)` or
        // `/lib64/ld-linux-x86-64.so.2 (0x...)`; `linux-vdso.so.1` has no file.
        let path =
            line.split("=>").nth(1).unwrap_or(line).split_whitespace().next().unwrap_or_default();
        if path.starts_with('/') {
            libraries.push(PathBuf::from(path));
        }
    }
    Ok(libraries)
}

fn tree_bytes(dir: &Path) -> Result<u64, GuestKitError> {
    let mut total = 0u64;
    for entry in fs::read_dir(dir).map_err(staging(format!("listing {}", dir.display())))? {
        let entry = entry.map_err(staging(format!("listing {}", dir.display())))?;
        let meta =
            entry.metadata().map_err(staging(format!("reading {}", entry.path().display())))?;
        total += if meta.is_dir() { tree_bytes(&entry.path())? } else { meta.len() };
    }
    Ok(total)
}

fn mkfs(stage: &Path, image: &Path) -> Result<(), GuestKitError> {
    const MIB: u64 = 1024 * 1024;
    let payload = tree_bytes(stage)?;
    let size = (payload * 2 + 64 * MIB).div_ceil(MIB) * MIB;
    if image.exists() {
        fs::remove_file(image).map_err(staging(format!("removing {}", image.display())))?;
    }
    fs::File::create(image)
        .and_then(|f| f.set_len(size))
        .map_err(staging(format!("sizing {}", image.display())))?;
    let out = Command::new("mkfs.ext4")
        .args(["-F", "-q", "-L", "ovd-guest-kit", "-d"])
        .arg(stage)
        .arg(image)
        .output()
        .map_err(staging("spawning mkfs.ext4"))?;
    if out.status.success() {
        Ok(())
    } else {
        let _ = fs::remove_file(image);
        Err(GuestKitError::ImageBuild {
            image: image.to_path_buf(),
            status: out.status.code(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// `path` if it is a file, else [`GuestKitError::LaneNotPrepared`].
pub(crate) fn prepared(path: PathBuf, artifact: &str) -> Result<PathBuf, GuestKitError> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(GuestKitError::LaneNotPrepared { artifact: artifact.to_owned(), path })
    }
}

/// The module the lane setup step built for `release` from this
/// workspace's source (EVMOD-BUILD, round 13: workspace-scoped,
/// `<workspace>/target/kmod/<release>/ovd_fwd.ko`, or under
/// `$OVERDRIVE_KMOD_DIR` in a mutation run; the
/// `xtask::guest_kit_prepare` contract path, see
/// [`crate::forwarder_module`]).
fn prepared_forwarder_module(release: &str) -> Result<PathBuf, GuestKitError> {
    let nested_prebuilt = PathBuf::from(GUEST_FORWARDER_MODULE_PATH);
    if nested_prebuilt.is_file() {
        return Ok(nested_prebuilt);
    }
    let module =
        crate::forwarder_module::built_module_path(&crate::forwarder_module::kmod_dir(), release);
    prepared(module, &format!("ovd_fwd.ko for kernel {release}"))
}

/// The static scripted peer the lane setup step built for the host's musl
/// target (`<target>/<musl-triple>/release/overdrive-guest-peer`).
fn prepared_guest_peer() -> Result<PathBuf, GuestKitError> {
    let nested_prebuilt = PathBuf::from(GUEST_PEER_GUEST_PATH);
    if nested_prebuilt.is_file() {
        return Ok(nested_prebuilt);
    }
    let target = vm_fixture::musl_target_triple()?;
    let root = vm_fixture::workspace_root();
    let peer =
        vm_fixture::cargo_target_dir(&root).join(target).join("release").join(GUEST_PEER_BIN);
    prepared(peer, &format!("static {GUEST_PEER_BIN} for {target}"))
}

/// A unique per-test directory under its caller-selected root, removed on
/// drop.
///
/// Image callers select the reflink-capable staging root; the guest kit's
/// private runtime caller selects `/run` only for non-disk artifacts.
#[derive(Debug)]
pub struct WorkDir {
    path: PathBuf,
}

impl WorkDir {
    /// Creates `<staging_root>/<prefix>-<pid>-<n>`.
    ///
    /// # Errors
    ///
    /// [`GuestKitError::Staging`] when it cannot be created.
    pub fn create(staging_root: &Path, prefix: &str) -> Result<Self, GuestKitError> {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = staging_root.join(format!("{prefix}-{}-{n}", std::process::id()));
        if path.exists() {
            fs::remove_dir_all(&path).map_err(staging(format!("clearing {}", path.display())))?;
        }
        fs::create_dir_all(&path).map_err(staging(format!("creating {}", path.display())))?;
        Ok(Self { path })
    }

    /// The directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Holders of `/dev/vhost-vsock` among live processes, by pid (the VMM is
/// the only holder of its device, CID-NOCLAIM): every process with an fd
/// whose link is `/dev/vhost-vsock`.
///
/// # Errors
///
/// A `/proc` read failure other than a process vanishing mid-scan.
pub fn vhost_vsock_holders() -> io::Result<Vec<u32>> {
    let mut holders = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let fds = match fs::read_dir(entry.path().join("fd")) {
            Ok(fds) => fds,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                ) =>
            {
                continue;
            }
            Err(e) => return Err(e),
        };
        for fd in fds.flatten() {
            if fs::read_link(fd.path()).is_ok_and(|target| target == Path::new("/dev/vhost-vsock"))
            {
                holders.push(pid);
                break;
            }
        }
    }
    Ok(holders)
}

#[cfg(test)]
mod tests {
    use super::cid_bands::{CidBand, PRODUCTION_BAND_WIDTH, REGISTERED, registry_is_sound};

    #[test]
    fn registered_cid_bands_are_pairwise_disjoint_above_the_production_band() {
        assert!(registry_is_sound(&REGISTERED));
        let overlapping = [
            CidBand { name: "a", first_offset: PRODUCTION_BAND_WIDTH, width: 8 },
            CidBand { name: "b", first_offset: PRODUCTION_BAND_WIDTH + 4, width: 8 },
        ];
        assert!(!registry_is_sound(&overlapping));
        let in_production = [CidBand { name: "c", first_offset: 0, width: 1 }];
        assert!(!registry_is_sound(&in_production));
    }
}
