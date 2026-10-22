//! Runs the existing `overdrive-fwd` integration scenarios inside disposable
//! `GuestKit` Cloud Hypervisor guests. Test-support executable only; never
//! shipped by a production crate.
//!
//! Prepare and run it on the qualified metal launcher without making the
//! physical kernel the module under test:
//!
//! ```text
//! cargo xtask metal run -- cargo xtask guest-kit-prepare
//! cargo xtask metal run -- cargo run -p overdrive-testing \
//!   --bin overdrive-fwd-guest-suite --features integration-tests
//! ```
//!
//! `OVERDRIVE_FWD_GUEST_GROUP` selects one fail-closed isolation group from
//! `probe`, `core`, `unload`, `ulp`, `stress`, and `nested-boundary`. Omitting
//! it runs every group in that order; each destructive group gets a fresh
//! guest boot.

#![cfg(target_os = "linux")]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "a fail-closed native test runner reports each guest's evidence"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use overdrive_testing::guest_app::{GUEST_PEER_GUEST_PATH, subcommand};
use overdrive_testing::guest_kit::cid_bands::FWD_GUEST_BOUNDARY;
use overdrive_testing::guest_kit::{
    GuestFile, GuestImageSpec, GuestInit, GuestKit, GuestKitError, LOOPBACK_MODULE_GUEST_PATH,
    ScriptedBootExtra, WorkDir, host_module_file,
};
use overdrive_testing::guest_lane::{CELL_BOUND, IN_GUEST_TESTS_PATH, InGuestTests};
use overdrive_testing::vm_fixture::default_staging_root;

const GUEST_MIB: u32 = 4_096;
const TEST_THREADS: &str = "--test-threads=1";
const NO_CAPTURE: &str = "--nocapture";
const ABSENT_LOOPBACK: &str = "integration::probe_gold::an_absent_vsock_loopback_transport_is_refused_naming_stream_round_trip";
const HEALTHY_PROBE: &str = "integration::probe_gold::the_probe_passes_and_leaves_no_residue_on_a_correctly_provisioned_node";
const ULP: &str =
    "integration::unload_in_guest::a_ulp_socket_keeps_the_module_for_the_boot_in_a_real_guest";
const STRESS: &str = "integration::unload_in_guest::the_unload_stress_never_faults_in_a_guest";
const NESTED_REFLINK_IMAGE: &str = "/var/lib/overdrive-testing/reflink.xfs";
const NESTED_PREBUILT_INIT: &str = "/opt/overdrive-testing/prebuilt-overdrive-init";

struct GuestRun {
    guest: overdrive_testing::guest_kit::ScriptedGuest,
    module_paths: Vec<String>,
    _work: WorkDir,
}

impl GuestRun {
    fn boot(
        kit: &GuestKit,
        tests: &InGuestTests,
        name: &str,
        slot: u32,
        extra: Vec<GuestFile>,
        module_paths: Vec<String>,
    ) -> Result<Self, GuestKitError> {
        Self::boot_with_vcpus(kit, tests, name, slot, extra, module_paths, 2)
    }

    fn boot_with_vcpus(
        kit: &GuestKit,
        tests: &InGuestTests,
        name: &str,
        slot: u32,
        mut extra: Vec<GuestFile>,
        module_paths: Vec<String>,
        vcpus: u8,
    ) -> Result<Self, GuestKitError> {
        let staging = default_staging_root();
        let work = WorkDir::create(&staging, &format!("fwd-suite-{name}"))?;
        extra.insert(
            0,
            GuestFile {
                host: tests.binary.clone(),
                guest: PathBuf::from(IN_GUEST_TESTS_PATH),
                with_shared_libraries: true,
            },
        );
        let image = kit.image(
            work.path(),
            name,
            &GuestImageSpec {
                init: GuestInit::ScriptedPeer,
                forwarder_module: true,
                loopback_module: true,
                extra,
            },
        )?;
        let guest = kit.boot_scripted_with(
            &image,
            FWD_GUEST_BOUNDARY.raw(slot),
            &work.path().join("run"),
            &ScriptedBootExtra {
                cmdline: Vec::new(),
                memory_mib: Some(GUEST_MIB),
                vcpus: Some(vcpus),
            },
        )?;
        Ok(Self { guest, module_paths, _work: work })
    }

    fn exec(
        &mut self,
        test_args: &[&str],
        expected_passed: usize,
        bound: Duration,
    ) -> Result<(), GuestKitError> {
        let console_before = self.guest.vm.console().len();
        let mut command = vec![GUEST_PEER_GUEST_PATH, subcommand::RUN_WITH_MODULES];
        command.push(LOOPBACK_MODULE_GUEST_PATH);
        command.extend(self.module_paths.iter().map(String::as_str));
        command.extend(["--", IN_GUEST_TESTS_PATH]);
        command.extend_from_slice(test_args);
        let code = self.guest.peer().exec(&command, bound)?;
        let console = self.guest.vm.console();
        let run_console = console.get(console_before..).unwrap_or(&console);
        if code != 0 {
            return Err(GuestKitError::Staging {
                detail: format!("guest test argv {test_args:?} exited {code}"),
                source: std::io::Error::other(run_console.to_owned()),
            });
        }
        let summary = format!("test result: ok. {expected_passed} passed; 0 failed;");
        if !run_console.contains(&summary) {
            return Err(GuestKitError::Staging {
                detail: format!(
                    "guest test argv {test_args:?} exited zero without the required {summary:?}"
                ),
                source: std::io::Error::other(run_console.to_owned()),
            });
        }
        Ok(())
    }

    fn command_with_modules(
        &mut self,
        modules: &[String],
        program: &str,
        args: &[&str],
    ) -> Result<(), GuestKitError> {
        let mut command = vec![GUEST_PEER_GUEST_PATH, subcommand::RUN_WITH_MODULES];
        command.extend(modules.iter().map(String::as_str));
        command.extend(["--", program]);
        command.extend_from_slice(args);
        self.command(&command)
    }

    fn command(&mut self, argv: &[&str]) -> Result<(), GuestKitError> {
        let code = self.guest.peer().exec(argv, Duration::from_secs(30))?;
        if code != 0 {
            return Err(GuestKitError::Staging {
                detail: format!("guest evidence command {argv:?} exited {code}"),
                source: std::io::Error::other(self.guest.vm.console_tail(4096)),
            });
        }
        Ok(())
    }

    fn finish(mut self, name: &str) -> Result<(), GuestKitError> {
        let console = self.guest.vm.console();
        println!("===== disposable guest {name} =====\n{console}");
        let _ = self.guest.take_peer().power_off();
        let exited = self.guest.vm.wait_exit(Duration::from_secs(30))?;
        if exited.is_none() {
            return Err(GuestKitError::Wait {
                cid: self.guest.vm.cid(),
                source: std::io::Error::other("the disposable guest did not power off"),
            });
        }
        Ok(())
    }
}

fn tool(name: &str) -> GuestFile {
    let host = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"]
        .into_iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("required guest-suite tool {name} is not installed"));
    GuestFile { host, guest: Path::new("/usr/bin").join(name), with_shared_libraries: true }
}

fn base_tools() -> Vec<GuestFile> {
    ["cat", "cp", "echo", "ip", "nft", "sleep", "ss"].into_iter().map(tool).collect()
}

fn make_reflink_volume(dir: &Path) -> Result<PathBuf, GuestKitError> {
    let image = dir.join("nested-reflink.xfs");
    std::fs::File::create(&image).and_then(|file| file.set_len(768 * 1024 * 1024)).map_err(
        |source| GuestKitError::Staging {
            detail: format!("creating nested reflink volume at {}", image.display()),
            source,
        },
    )?;
    let output = Command::new("mkfs.xfs")
        .args(["-f", "-q", "-m", "reflink=1"])
        .arg(&image)
        .output()
        .map_err(|source| GuestKitError::Staging {
            detail: "spawning mkfs.xfs for the nested GuestKit volume".to_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(GuestKitError::Staging {
            detail: format!("mkfs.xfs exited {:?}", output.status.code()),
            source: std::io::Error::other(String::from_utf8_lossy(&output.stderr).into_owned()),
        });
    }
    Ok(image)
}

const fn exact(name: &str) -> [&str; 4] {
    [TEST_THREADS, NO_CAPTURE, "--exact", name]
}

fn run_probe_isolation(kit: &GuestKit, tests: &InGuestTests) -> Result<(), GuestKitError> {
    let mut guest = GuestRun::boot(kit, tests, "probe-isolation", 7, base_tools(), Vec::new())?;
    guest.command(&["/usr/bin/echo", "FWD-SUITE-BEFORE-REFS"])?;
    guest.command(&["/usr/bin/cat", "/sys/module/ovd_fwd/refcnt"])?;
    guest.command(&["/usr/bin/cat", "/sys/module/vsock_loopback/refcnt"])?;
    guest.command(&["/usr/bin/ss", "-H", "-a", "-f", "vsock"])?;
    guest.exec(&exact(ABSENT_LOOPBACK), 1, CELL_BOUND)?;
    guest.exec(&exact(HEALTHY_PROBE), 1, CELL_BOUND)?;
    guest.command(&["/usr/bin/echo", "FWD-SUITE-AFTER-REFS"])?;
    guest.command(&["/usr/bin/cat", "/sys/module/ovd_fwd/refcnt"])?;
    guest.command(&["/usr/bin/cat", "/sys/module/vsock_loopback/refcnt"])?;
    guest.command(&["/usr/bin/ss", "-H", "-a", "-f", "vsock"])?;
    guest.finish("probe-isolation")
}

fn run_core(kit: &GuestKit, tests: &InGuestTests) -> Result<(), GuestKitError> {
    let staging = default_staging_root();
    let modules = WorkDir::create(&staging, "fwd-suite-core-modules")?;
    let module_specs = [
        ("net/netfilter/nfnetlink", "/lib/overdrive-testing/modules/nfnetlink.ko"),
        ("net/netfilter/nf_tables", "/lib/overdrive-testing/modules/nf_tables.ko"),
        ("net/ipv4/netfilter/nf_reject_ipv4", "/lib/overdrive-testing/modules/nf_reject_ipv4.ko"),
        ("net/ipv6/netfilter/nf_reject_ipv6", "/lib/overdrive-testing/modules/nf_reject_ipv6.ko"),
        ("net/netfilter/nft_reject", "/lib/overdrive-testing/modules/nft_reject.ko"),
        ("net/netfilter/nft_reject_inet", "/lib/overdrive-testing/modules/nft_reject_inet.ko"),
        ("net/tls/tls", "/lib/overdrive-testing/modules/tls.ko"),
    ];
    let mut extra = base_tools();
    let mut module_paths = Vec::new();
    for (relative, guest_path) in module_specs {
        extra.push(host_module_file(kit, modules.path(), relative, Path::new(guest_path))?);
        module_paths.push(guest_path.to_owned());
    }
    let mut guest = GuestRun::boot(kit, tests, "core", 8, extra, module_paths)?;
    guest.exec(
        &[
            TEST_THREADS,
            NO_CAPTURE,
            "--skip",
            ABSENT_LOOPBACK,
            "--skip",
            "integration::guest_boundary::",
            "--skip",
            "integration::unload::",
            "--skip",
            "integration::unload_in_guest::",
        ],
        86,
        Duration::from_secs(900),
    )?;
    guest.finish("core")
}

fn run_unload(kit: &GuestKit, tests: &InGuestTests) -> Result<(), GuestKitError> {
    let workspace_module = GuestFile {
        host: kit.forwarder_module.clone(),
        guest: PathBuf::from(format!(
            "/home/ubuntu/overdrive/target/kmod/{}/ovd_fwd.ko",
            kit.kernel_release
        )),
        with_shared_libraries: false,
    };
    let mut guest = GuestRun::boot(kit, tests, "unload", 9, vec![workspace_module], Vec::new())?;
    guest.exec(&[TEST_THREADS, NO_CAPTURE, "integration::unload::"], 4, CELL_BOUND)?;
    guest.finish("unload")
}

fn run_ulp(kit: &GuestKit, tests: &InGuestTests) -> Result<(), GuestKitError> {
    let staging = default_staging_root();
    let work = WorkDir::create(&staging, "fwd-suite-tls-module")?;
    let tls = host_module_file(
        kit,
        work.path(),
        "net/tls/tls",
        Path::new("/lib/overdrive-testing/modules/tls.ko"),
    )?;
    let mut guest = GuestRun::boot(
        kit,
        tests,
        "ulp",
        10,
        vec![tls],
        vec!["/lib/overdrive-testing/modules/tls.ko".to_owned()],
    )?;
    guest.exec(&exact(ULP), 1, CELL_BOUND)?;
    drop(work);
    guest.finish("ulp")
}

fn run_stress(kit: &GuestKit, tests: &InGuestTests) -> Result<(), GuestKitError> {
    let mut guest =
        GuestRun::boot_with_vcpus(kit, tests, "stress", 11, Vec::new(), Vec::new(), 16)?;
    guest.exec(&exact(STRESS), 1, CELL_BOUND)?;
    guest.finish("stress")
}

fn run_nested_boundary(kit: &GuestKit, tests: &InGuestTests) -> Result<(), GuestKitError> {
    let staging = default_staging_root();
    let modules = WorkDir::create(&staging, "fwd-suite-nested-modules")?;
    let mut extra = base_tools();
    extra.extend(
        [
            "losetup",
            "chmod",
            "mkdir",
            "mkfs.ext4",
            "mknod",
            "mount",
            "prlimit",
            "setpriv",
            "stat",
            "true",
            "uname",
        ]
        .into_iter()
        .map(tool),
    );
    let reflink = make_reflink_volume(modules.path())?;
    extra.push(GuestFile {
        host: reflink,
        guest: PathBuf::from(NESTED_REFLINK_IMAGE),
        with_shared_libraries: false,
    });
    extra.push(GuestFile {
        host: kit.init_bin.clone(),
        guest: PathBuf::from(NESTED_PREBUILT_INIT),
        with_shared_libraries: false,
    });
    extra.push(GuestFile {
        host: kit.kernel_path.clone(),
        guest: PathBuf::from(format!("/boot/vmlinuz-{}", kit.kernel_release)),
        with_shared_libraries: false,
    });
    let xfs = host_module_file(
        kit,
        modules.path(),
        "fs/xfs/xfs",
        Path::new("/lib/overdrive-testing/modules/xfs.ko"),
    )?;
    extra.push(xfs);
    let nested_modules = [
        ("drivers/crypto/ccp/ccp", "/lib/overdrive-testing/modules/ccp.ko"),
        ("virt/lib/irqbypass", "/lib/overdrive-testing/modules/irqbypass.ko"),
        ("arch/x86/kvm/kvm", "/lib/overdrive-testing/modules/kvm.ko"),
        ("arch/x86/kvm/kvm-amd", "/lib/overdrive-testing/modules/kvm-amd.ko"),
        ("drivers/vhost/vhost_iotlb", "/lib/overdrive-testing/modules/vhost_iotlb.ko"),
        ("drivers/vhost/vhost", "/lib/overdrive-testing/modules/vhost.ko"),
        ("drivers/vhost/vhost_vsock", "/lib/overdrive-testing/modules/vhost_vsock.ko"),
    ];
    let mut module_paths = Vec::new();
    for (relative, guest_path) in nested_modules {
        extra.push(host_module_file(kit, modules.path(), relative, Path::new(guest_path))?);
        module_paths.push(guest_path.to_owned());
    }
    extra.push(GuestFile {
        host: kit.cloud_hypervisor_bin.clone(),
        guest: PathBuf::from("/usr/bin/cloud-hypervisor"),
        with_shared_libraries: true,
    });
    extra.push(tool("systemd-detect-virt"));
    let mut guest = GuestRun::boot(kit, tests, "nested-boundary", 12, extra, module_paths)?;
    guest.command(&["/usr/bin/mkdir", "-p", "/srv/vm"])?;
    guest.command_with_modules(
        &["/lib/overdrive-testing/modules/xfs.ko".to_owned()],
        "/usr/bin/losetup",
        &["/dev/loop0", NESTED_REFLINK_IMAGE],
    )?;
    guest.command(&["/usr/bin/mount", "-t", "xfs", "/dev/loop0", "/srv/vm"])?;
    guest.command(&["/usr/bin/cp", GUEST_PEER_GUEST_PATH, "/srv/vm/reflink-probe"])?;
    guest.command(&[
        "/usr/bin/cp",
        "--reflink=always",
        "/srv/vm/reflink-probe",
        "/srv/vm/reflink-probe.clone",
    ])?;
    let nested_modules = std::mem::take(&mut guest.module_paths);
    guest.command_with_modules(&nested_modules, "/usr/bin/true", &[])?;
    // A minimal devtmpfs has no udev rules, so both devices appear 0600.
    // Reproduce the qualified host's 0660 owner/group access: the production
    // confined identity keeps uid/gid 4200 and the actual device-owner group
    // (gid 0 in this disposable guest) remains its supplementary group.
    guest.command(&["/usr/bin/chmod", "0660", "/dev/kvm", "/dev/vhost-vsock"])?;
    guest.command(&[
        "/usr/bin/stat",
        "-c",
        "NESTED-DEVICE %n uid=%u gid=%g mode=%a",
        "/dev/kvm",
        "/dev/vhost-vsock",
    ])?;
    guest.exec(
        &[TEST_THREADS, NO_CAPTURE, "integration::guest_boundary::"],
        7,
        Duration::from_secs(600),
    )?;
    guest.finish("nested-boundary")
}

fn main() {
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("the prepared GuestKit provisions on metal");
    let tests = InGuestTests::prepared("overdrive-fwd", "integration")
        .expect("cargo xtask guest-kit-prepare staged the integration binary");
    let selected = std::env::var("OVERDRIVE_FWD_GUEST_GROUP").unwrap_or_else(|_| "all".to_owned());
    let runs = [
        ("probe", run_probe_isolation as fn(&GuestKit, &InGuestTests) -> _),
        ("core", run_core),
        ("unload", run_unload),
        ("ulp", run_ulp),
        ("stress", run_stress),
        ("nested-boundary", run_nested_boundary),
    ];
    let mut matched = false;
    for (name, run) in runs {
        if selected == "all" || selected == name {
            matched = true;
            run(&kit, &tests)
                .unwrap_or_else(|error| panic!("disposable guest group {name}: {error:?}"));
        }
    }
    assert!(matched, "unknown OVERDRIVE_FWD_GUEST_GROUP={selected:?}");
}
