//! Tier-3 acceptance — `VmHostState` adapter equivalence, including the
//! `kill_scope` settle postcondition (S-VM-91).
//!
//! Gated `integration-tests,kvm-tests` (see `tests/integration.rs`) — the
//! real-adapter half boots a real Cloud Hypervisor VMM (via the `Vmm` port
//! directly, the same pattern `vmm_equivalence.rs` uses — `overdrive-host`
//! does not depend on `overdrive-worker`, so there is no `VmDriver` here)
//! so `kill_scope`'s settle postcondition is exercised against a REAL
//! cgroup v2 scope holding a REAL live process, not a synthetic stand-in.
//! Run via:
//!
//! ```text
//! cargo xtask metal run -- cargo nextest run -p overdrive-host \
//!   --features integration-tests,kvm-tests -E 'test(vm_host_state_equivalence)'
//! ```
//!
//! Per Mandate 9 (`nw-tdd-methodology`): this is a FIXED, hand-enumerated
//! call sequence at layer 3+ — `@example`, not `@property`.

use std::collections::BTreeSet;
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};

use overdrive_core::AllocationId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_core::traits::vmm::Vmm;
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, KernelCmdline, KernelImage, MemoryPlan, RootfsPlan,
    VmConfig, VmConfinement, VmRunDir, VmmIdentity,
};
use overdrive_host::{CloudHypervisorVmm, RealVmHostState};
use overdrive_sim::SimVmHostState;
use overdrive_testing::vm_fixture::{VmFixture, default_staging_root};
use serial_test::serial;

/// Small guest RAM — keeps the real boot in this suite light. Matches
/// `vmm_equivalence.rs`'s own choice.
const GUEST_BYTES: u64 = 128 * 1024 * 1024;

/// The real cgroupfs root every workload scope resolves under.
const CGROUP_ROOT: &str = "/sys/fs/cgroup";

fn read_kernel_header(path: &Path) -> Vec<u8> {
    use std::io::Read;
    let file = std::fs::File::open(path).expect("open staged kernel for header read");
    let mut buf = Vec::new();
    file.take(KERNEL_MAGIC_WINDOW as u64).read_to_end(&mut buf).expect("read kernel header");
    buf
}

fn validated_kernel(fixture: &VmFixture) -> KernelImage {
    let header = read_kernel_header(&fixture.kernel_path);
    KernelImage::validate(fixture.kernel_path.clone(), HostArch::X86_64, &header)
        .expect("fixture-staged kernel validates for x86_64")
}

const fn sample_confinement() -> VmConfinement {
    VmConfinement::confined(
        VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
        1024,
    )
}

/// Build a real, fully-resolved `VmConfig` sharing `fixture`'s staged
/// kernel/rootfs, for allocation `vmhosteq-a` — mirrors
/// `vmm_equivalence.rs::sample_vm_config`.
fn sample_vm_config(fixture: &VmFixture, run_root: &Path) -> VmConfig {
    let alloc = AllocationId::new("vmhosteq-a").expect("valid alloc id");
    let master_bytes = std::fs::metadata(&fixture.rootfs_path).expect("stat staged rootfs").len();
    VmConfig {
        alloc: alloc.clone(),
        kernel: validated_kernel(fixture),
        rootfs: RootfsPlan::for_alloc(fixture.rootfs_path.clone(), master_bytes, &alloc),
        cmdline: KernelCmdline::platform_default(HostArch::X86_64),
        memory: MemoryPlan::derive(GUEST_BYTES),
        vcpus: NonZeroU8::new(1).expect("1 is nonzero"),
        run_dir: VmRunDir::for_alloc(run_root, &alloc),
        confinement: sample_confinement(),
        netns: None,
        cgroup_scope: CgroupPath::for_alloc(&alloc),
    }
}

/// The shared call sequence S-VM-91 drives against BOTH adapters: probe
/// (idempotent) -> observe (sees the seeded/booted allocation on all
/// three surfaces) -> `kill_scope` (settles -- the scope is gone
/// afterward, on BOTH adapters) -> `discard_artifacts` (the run dir and
/// clone are both gone afterward).
async fn assert_observe_then_kill_then_discard(host: &dyn VmHostState, alloc: &AllocationId) {
    host.probe().await.expect("probe succeeds");
    host.probe().await.expect("probe is idempotent -- second call also succeeds");

    let observation = host.observe().await.expect("observe succeeds");
    assert!(
        observation.scopes.contains_key(alloc),
        "observe must report the allocation's cgroup scope, got {:?}",
        observation.scopes.keys().collect::<Vec<_>>()
    );
    assert!(
        observation.run_dirs.contains(alloc),
        "observe must report the allocation's run directory"
    );
    assert!(observation.clones.contains_key(alloc), "observe must report the allocation's clone");

    let scope = CgroupPath::for_alloc(alloc);
    host.kill_scope(&scope).await.expect("kill_scope succeeds and settles");

    // SETTLE POSTCONDITION: immediately after kill_scope returns, the
    // scope is gone from a fresh observation -- not eventually, NOW.
    let after_kill = host.observe().await.expect("observe succeeds after kill_scope");
    assert!(
        !after_kill.scopes.contains_key(alloc),
        "kill_scope must not return until the scope is gone (settle postcondition)"
    );

    // kill_scope is idempotent -- an already-absent scope is Ok, not an
    // error.
    host.kill_scope(&scope).await.expect("kill_scope is idempotent against an absent scope");

    host.discard_artifacts(alloc).await.expect("discard_artifacts succeeds");
    let after_discard = host.observe().await.expect("observe succeeds after discard_artifacts");
    assert!(!after_discard.run_dirs.contains(alloc), "discard_artifacts must remove the run dir");
    assert!(!after_discard.clones.contains_key(alloc), "discard_artifacts must remove the clone");

    // discard_artifacts is idempotent -- absence of either is success.
    host.discard_artifacts(alloc).await.expect("discard_artifacts is idempotent");
}

#[tokio::test]
async fn vm_host_state_equivalence_sim() {
    let sim = SimVmHostState::new();
    let alloc = AllocationId::new("vmhosteq-sim-a").expect("valid alloc id");

    sim.set_scope(alloc.clone(), BTreeSet::from([4242]));
    sim.set_run_dir(alloc.clone());
    sim.set_clone(
        alloc.clone(),
        PathBuf::from("/sim/staging/.overdrive-vm-rootfs-vmhosteq-sim-a.img"),
    );

    assert_observe_then_kill_then_discard(&sim, &alloc).await;

    // Fault injection -- probe surfaces the injected Substrate error.
    sim.inject_probe_fault(std::io::ErrorKind::PermissionDenied);
    let err = sim.probe().await.expect_err("an injected probe fault must surface");
    assert!(
        matches!(
            err,
            overdrive_core::traits::vm_host_state::VmHostStateProbeError::Substrate { .. }
        ),
        "expected Substrate, got {err:?}"
    );
    sim.probe().await.expect("the one-shot fault is consumed -- the next probe is healthy again");

    assert_eq!(sim.kind(), "overdrive_sim::SimVmHostState");
}

#[tokio::test]
#[serial(vm_host_state_real_cgroup)]
async fn vm_host_state_equivalence_real() {
    let staging_root = default_staging_root();
    let fixture = VmFixture::provision(&staging_root).expect("fixture provisions on this host");
    let run_root = staging_root.join("vm-host-state-eq-run-root");
    std::fs::create_dir_all(&run_root).expect("create run root");

    let config = sample_vm_config(&fixture, &run_root);
    std::fs::create_dir_all(config.run_dir.path()).expect("create run dir");

    // Boot a real Cloud Hypervisor VMM -- the process whose PID this test
    // enrolls into a real cgroup v2 scope, so `kill_scope`'s settle
    // postcondition is proven against a genuinely live process, not a
    // synthetic stand-in.
    let vmm = CloudHypervisorVmm::default();
    vmm.probe().await.expect("Vmm::probe succeeds on this host");
    let proc = vmm.create(&config).await.expect("Vmm::create boots a real VMM");

    // Cgroup enrolment is the DRIVER's job, not `create`'s (per
    // `brief.md` §108's effect-isolation table) -- this test plays that
    // role directly. `create_dir_all` on a mounted cgroup2 filesystem is
    // exactly `mkdir -p`; the kernel synthesises `cgroup.procs` /
    // `cgroup.kill` for the new leaf regardless of controller
    // delegation (delegation governs resource accounting, not base
    // membership/kill).
    let scope_path = config.cgroup_scope.resolve(Path::new(CGROUP_ROOT));
    tokio::fs::create_dir_all(&scope_path).await.expect("create real cgroup v2 scope");
    tokio::fs::write(scope_path.join("cgroup.procs"), proc.control.pid.to_string())
        .await
        .expect("enrol the VMM pid into the scope");

    let host = RealVmHostState::new(
        PathBuf::from(CGROUP_ROOT),
        run_root.clone(),
        config.rootfs.clone_dest().parent().expect("clone_dest has a parent").to_path_buf(),
    );

    assert_observe_then_kill_then_discard(&host, &config.alloc).await;

    // The VMM process is genuinely dead -- `kill_scope`'s settle
    // postcondition (the scope's rmdir succeeded) already proves this
    // structurally (a cgroup with a live process cannot be rmdir'd), but
    // assert it directly too for a belt-and-braces witness.
    assert!(
        !Path::new(&format!("/proc/{}", proc.control.pid)).exists(),
        "the real VMM process must be dead after kill_scope"
    );

    assert_eq!(host.kind(), "overdrive_host::RealVmHostState");

    // probe's absent-root tolerance -- a fresh alloc's never-created
    // roots are Ok, not a refusal.
    let empty_host = RealVmHostState::new(
        PathBuf::from(CGROUP_ROOT),
        staging_root.join("vm-host-state-eq-never-created-run-root"),
        staging_root.join("vm-host-state-eq-never-created-staging-root"),
    );
    empty_host.probe().await.expect("an absent run/staging root is Ok, never a refusal");
}
