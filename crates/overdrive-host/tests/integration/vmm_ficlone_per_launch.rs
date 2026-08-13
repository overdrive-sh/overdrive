//! Tier-3 acceptance — S-VM-94: the per-launch `FICLONE` clone fails
//! closed on a REAL non-reflink target (self-application of the boot
//! probe's own rule, ADR-0082 §D5).
//!
//! Gated `integration-tests,kvm-tests` (see `tests/integration.rs`).
//! `@mandatory:mutation_target` — the `FICLONE` call in
//! `crates/overdrive-host/src/vmm.rs::ficlone_rootfs` is the mandatory
//! mutation target: a mutant substituting a full-copy fallback (e.g.
//! `std::fs::copy`) must be killed by this test's "no full-copy
//! fallback" assertion. Mutation testing itself is an end-of-DELIVER
//! gate, not run per-step (wave-decisions DWD-19) — this comment tags
//! the target for that later pass.

use std::num::NonZeroU8;
use std::path::{Path, PathBuf};

use overdrive_core::AllocationId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::traits::vmm::{Vmm, VmmError};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, KernelCmdline, KernelImage, MemoryPlan, RootfsPlan,
    VmConfig, VmConfinement, VmRunDir, VmmIdentity,
};
use overdrive_host::CloudHypervisorVmm;
use overdrive_testing::vm_fixture::{VmFixture, default_staging_root};

fn read_kernel_header(path: &Path) -> Vec<u8> {
    use std::io::Read;
    let file = std::fs::File::open(path).expect("open staged kernel for header read");
    let mut buf = Vec::new();
    file.take(KERNEL_MAGIC_WINDOW as u64).read_to_end(&mut buf).expect("read kernel header");
    buf
}

/// `true` iff `dir` genuinely does NOT support `FICLONE` — an EXECUTED
/// clone attempt against a real written file, never an `fstype` string
/// comparison (ADR-0082 §D5's own discipline, applied here to the TEST
/// fixture's own assumption rather than production code).
fn confirm_non_reflink(dir: &Path) -> bool {
    let probe = dir.join(".non-reflink-probe");
    let clone = dir.join(".non-reflink-probe.clone");
    let _ = std::fs::remove_file(&probe);
    let _ = std::fs::remove_file(&clone);
    let write_ok = std::fs::write(&probe, [0xAB_u8; 4096]).is_ok();
    let reflink_result = (|| -> std::io::Result<()> {
        let src = std::fs::File::open(&probe)?;
        let dst = std::fs::File::options().write(true).create_new(true).open(&clone)?;
        rustix::fs::ioctl_ficlone(&dst, &src).map_err(std::io::Error::from)
    })();
    let _ = std::fs::remove_file(&probe);
    let _ = std::fs::remove_file(&clone);
    write_ok && reflink_result.is_err()
}

#[tokio::test]
async fn ficlone_fails_closed_on_a_real_non_reflink_target() {
    // /dev/shm is unambiguously tmpfs on every Linux distro (unlike
    // `/tmp`, whose backing depends on distro config) -- tmpfs has never
    // implemented FICLONE (no `->remap_file_range`), so this is a REAL,
    // un-injected non-reflink substrate, per S-VM-94's driving port
    // ("no SimVmm").
    let non_reflink_dir = PathBuf::from("/dev/shm/overdrive-vmm-ficlone-test");
    let _ = std::fs::remove_dir_all(&non_reflink_dir);
    std::fs::create_dir_all(&non_reflink_dir).expect("create /dev/shm test dir");
    assert!(
        confirm_non_reflink(&non_reflink_dir),
        "test fixture assumption violated: /dev/shm unexpectedly supports FICLONE on this host"
    );

    let staging_root = default_staging_root();
    let fixture = VmFixture::provision(&staging_root).expect("fixture provisions (real kernel/rootfs/CH)");

    let master = non_reflink_dir.join("master.img");
    std::fs::copy(&fixture.rootfs_path, &master).expect("stage a master rootfs copy onto tmpfs");
    let master_bytes = std::fs::metadata(&master).expect("stat staged master").len();

    let alloc = AllocationId::new("ficlone-neg").expect("valid alloc id");
    let rootfs = RootfsPlan::for_alloc(master.clone(), master_bytes, &alloc);
    let clone_dest = rootfs.clone_dest().to_path_buf();
    assert_eq!(
        clone_dest.parent(),
        Some(non_reflink_dir.as_path()),
        "clone_dest must sit beside the master (FICLONE is intra-filesystem) -- precondition \
         for this test to actually exercise a non-reflink target"
    );

    let run_root = non_reflink_dir.join("run");
    let run_dir = VmRunDir::for_alloc(&run_root, &alloc);
    std::fs::create_dir_all(run_dir.path()).expect("create run dir");

    let header = read_kernel_header(&fixture.kernel_path);
    let kernel = KernelImage::validate(fixture.kernel_path.clone(), HostArch::X86_64, &header)
        .expect("fixture-staged kernel validates for x86_64");

    let config = VmConfig {
        alloc: alloc.clone(),
        kernel,
        rootfs,
        cmdline: KernelCmdline::platform_default(HostArch::X86_64),
        memory: MemoryPlan::derive(128 * 1024 * 1024),
        vcpus: NonZeroU8::new(1).expect("1 is nonzero"),
        run_dir,
        confinement: VmConfinement::confined(
            VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
            1024,
        ),
        netns: None,
        cgroup_scope: CgroupPath::for_alloc(&alloc),
    };

    let vmm = CloudHypervisorVmm::new();
    let result = vmm.create(&config).await;

    let err = result.expect_err("create must fail closed on a real non-reflink target");
    match &err {
        VmmError::Io(io_err) => {
            let errno = io_err.raw_os_error();
            assert!(
                errno == Some(libc::EOPNOTSUPP) || errno == Some(libc::EXDEV),
                "expected the typed FICLONE errno EOPNOTSUPP or EXDEV, got {errno:?} ({io_err})"
            );
        }
        VmmError::Create { .. } => {
            panic!("expected VmmError::Io carrying the typed FICLONE errno, got {err:?}")
        }
    }

    // No full-copy fallback -- @mandatory:mutation_target. The
    // destination is empty or absent, NEVER a byte-for-byte copy of the
    // master image (which would be `master_bytes` long).
    match std::fs::metadata(&clone_dest) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Ok(meta) => assert_eq!(
            meta.len(),
            0,
            "no full-copy fallback may occur: the destination must be empty or absent, never a \
             copy of the {master_bytes}-byte master"
        ),
        Err(e) => panic!("unexpected error stat-ing clone_dest: {e}"),
    }

    // No hypervisor process was spawned: `create` returns before
    // `Command::spawn` is ever reached on this path, which is what the
    // code path itself proves by construction (the FICLONE step runs to
    // completion, in error, before any argv is rendered).

    let _ = std::fs::remove_dir_all(&non_reflink_dir);
}
