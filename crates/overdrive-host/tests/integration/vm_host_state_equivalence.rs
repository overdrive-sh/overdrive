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

#![allow(
    clippy::doc_markdown,
    reason = "per-test CONTRACT_SHAPE declarations are machine-readable repository contracts"
)]

use std::collections::BTreeSet;
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};

use overdrive_core::AllocationId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::guest_transport::{GUEST_CID_BASE, GuestCid};
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_core::traits::vmm::Vmm;
use overdrive_core::vm::config::{
    HostArch, KERNEL_MAGIC_WINDOW, KernelCmdline, KernelImage, MemoryPlan, RootfsPlan, VmConfig,
    VmConfinement, VmRunDir, clone_index_dir,
};
use overdrive_host::{CloudHypervisorVmm, RealVmHostState};
use overdrive_sim::SimVmHostState;
use overdrive_testing::overdrive_test;
use overdrive_testing::vm_fixture::{VmFixture, default_staging_root};
use serial_test::serial;

/// Small guest RAM — keeps the real boot in this suite light. Matches
/// `vmm_equivalence.rs`'s own choice.
const GUEST_BYTES: u64 = 128 * 1024 * 1024;

/// The real cgroupfs root every workload scope resolves under.
const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// The leased guest CID `GUEST_CID_BASE + offset` of one launch (CID-NOCLAIM):
/// the value `Vmm::create` puts on the `--vsock` argument. No claim is taken;
/// the VMM opens the device itself.
fn cid(offset: u32) -> GuestCid {
    GuestCid::new(GUEST_CID_BASE + offset).expect("an in-range guest CID")
}

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

fn sample_confinement() -> VmConfinement {
    VmConfinement::confined(super::production_vmm_identity(), 1024)
}

/// Build a real, fully-resolved `VmConfig` sharing `fixture`'s staged
/// kernel/rootfs, for allocation `vmhosteq-a` — mirrors
/// `vmm_equivalence.rs::sample_vm_config`.
fn sample_vm_config(fixture: &VmFixture, run_root: &Path, index_dir: &Path) -> VmConfig {
    let alloc = AllocationId::new("vmhosteq-a").expect("valid alloc id");
    let master_bytes = std::fs::metadata(&fixture.rootfs_path).expect("stat staged rootfs").len();
    VmConfig {
        volume: None,
        alloc: alloc.clone(),
        kernel: validated_kernel(fixture),
        rootfs: RootfsPlan::for_alloc(
            fixture.rootfs_path.clone(),
            master_bytes,
            &alloc,
            // Stage the clone on the master's own filesystem (`run_root` is
            // under the fixture's reflink staging root) — ADR-0082 2026-08-18
            // fourth amendment (B1 fix): a platform dir, never beside the master.
            run_root,
            index_dir,
        ),
        cmdline: KernelCmdline::platform_default(HostArch::X86_64),
        memory: MemoryPlan::derive(GUEST_BYTES),
        vcpus: NonZeroU8::new(1).expect("1 is nonzero"),
        run_dir: VmRunDir::for_alloc(run_root, &alloc),
        confinement: sample_confinement(),
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

#[overdrive_test(guest = "cgroup-default")]
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

/// `CONTRACT_SHAPE`: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[tokio::test]
#[serial(vm_host_state_real_cgroup)]
async fn vm_host_state_equivalence_real() {
    let staging_root = default_staging_root();
    let fixture = VmFixture::provision(&staging_root).expect("fixture provisions on this host");
    let run_root = staging_root.join("vm-host-state-eq-run-root");
    std::fs::create_dir_all(&run_root).expect("create run root");

    // The clone-index dir is a real, platform-owned directory distinct
    // from the operator's rootfs-master dir (where the clone lands) —
    // `RealVmHostState` enumerates the LINKS here and resolves each to its
    // clone via `read_link` (ADR-0083 §§D3f-D3h).
    let index_dir = run_root.join("clone-index");
    let config = sample_vm_config(&fixture, &run_root, &index_dir);
    std::fs::create_dir_all(config.run_dir.path()).expect("create run dir");

    // Boot a real Cloud Hypervisor VMM inside a real cgroup v2 scope, so
    // `kill_scope`'s settle postcondition is proven against a genuinely live
    // process, not a synthetic stand-in. `create` joins a scope that exists;
    // making it is the DRIVER's job (`brief.md` §108's effect-isolation
    // table), so this test plays that role first. `create_dir_all` on a
    // mounted cgroup2 filesystem is exactly `mkdir -p`. The scope is removed
    // by `kill_scope`, below, which is the assertion under test.
    let scope_path = config.cgroup_scope.resolve(Path::new(CGROUP_ROOT));
    tokio::fs::create_dir_all(&scope_path).await.expect("create real cgroup v2 scope");
    let vmm = CloudHypervisorVmm::default();
    vmm.probe().await.expect("Vmm::probe succeeds on this host");
    let proc = vmm.create(&config, cid(0x0320)).await.expect("Vmm::create boots a real VMM");

    // The clone-index link is `VmDriver`'s to create at launch; this test
    // drives the `Vmm` port directly (no `VmDriver` in `overdrive-host`),
    // so it stands in for that one step — record the just-cloned rootfs in
    // the durable index exactly as `VmDriver::start` does BEFORE the
    // FICLONE (ADR-0083 §D3f). `observe` then reports the clone by
    // RESOLVING this link, and `discard_artifacts` removes target-then-link.
    std::fs::create_dir_all(&index_dir).expect("create the clone-index dir");
    std::os::unix::fs::symlink(config.rootfs.clone_dest(), config.rootfs.index_link())
        .expect("record the per-launch clone in the platform-owned index");

    let host =
        RealVmHostState::new(PathBuf::from(CGROUP_ROOT), run_root.clone(), index_dir.clone());

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
        staging_root.join("vm-host-state-eq-never-created-index-dir"),
    );
    empty_host.probe().await.expect("an absent run/index root is Ok, never a refusal");
}

// ---------------------------------------------------------------------
// S-VM-91 extension (step 03-09, DWD-26 / ADR-0083 §§D3f-D3h) — RED
// scaffold: after the per-launch clone moves to the operator's directory
// and `RealVmHostState` enumerates it through the platform-owned
// clone-index symlinks, the two adapters must STILL observe the same
// clone surface.
//
// Shape per `.claude/rules/testing.md` § "RED scaffolds and
// intentionally-failing commits": `#[should_panic(expected = "RED
// scaffold")]` plus a panic body naming the scenario, discoverable via
// `grep -rn 'should_panic.*RED scaffold' crates/`. `#[test]` (sync) TODAY
// because a body that is a single panic awaits nothing and boots no VMM;
// the activated form carries whatever `assert_observe_then_kill_then_
// discard` needs (`#[tokio::test]`, and `#[serial(vm_host_state_real_
// cgroup)]` if it folds into the real-adapter half). The two activated
// equivalence tests above are untouched — this is an ADDITION, not a
// rewrite.
// ---------------------------------------------------------------------

/// S-VM-91 extension / `@example` (Mandate 9, layer 3+) — the clone
/// surface stays adapter-equivalent after the DWD-26 relocation.
///
/// After ADR-0083 §§D3f-D3h the clone no longer sits in a single
/// node-level staging directory the sweep watches: `RootfsPlan::for_alloc`
/// reflinks it beside the OPERATOR's `[vm] rootfs` master (§D3a/§D3b), and
/// `RealVmHostState` learns of it through a platform-owned symlink index —
///
/// ```text
/// clone_index_dir(data_dir)/.overdrive-vm-rootfs-<alloc>.img  ->  <clone beside master>
/// ```
///
/// Two `RealVmHostState` method bodies change (§D3h), NO trait signature
/// does:
///   * `observe_clones` walks `index_dir` and takes the mapped path from
///     `read_link(entry.path())` instead of `entry.path()` — a dangling
///     link still yields an entry (§D3f's crash table);
///   * `discard_artifacts` stops re-deriving the clone path: it
///     `read_link`s the index entry, removes the TARGET first, then the
///     LINK, both `NotFound`-tolerant.
///
/// `RealVmHostState::new`'s third argument (today `config.rootfs.
/// clone_dest().parent()`, the staging surface) becomes
/// `clone_index_dir(&config.data_dir)`, renamed `index_dir`.
///
/// ## What the activated equivalence must assert
///
/// Drive the SAME shared sequence this file already owns —
/// [`assert_observe_then_kill_then_discard`] — against a `RealVmHostState`
/// constructed with a real `index_dir` that holds a symlink to a clone
/// staged beside an operator-chosen master (two distinct real directories,
/// NEITHER a platform staging dir NOR `/run`). Prove:
///   * `observe()` reports the clone by RESOLVING the symlink, so the
///     surface is populated even though no clone sits in `index_dir`
///     itself;
///   * `discard_artifacts` removes the clone via the link (target then
///     link), leaving neither behind, idempotently;
///   * `SimVmHostState` — whose `set_clone` already models the observable
///     clone surface directly — observes the byte-identical
///     observe/kill/discard shape, so the two adapters stay equivalent
///     across the new indirection.
///
/// This does NOT restate the pre-index observe/kill/discard assertions the
/// two activated tests above already make against a clone that lives in
/// the enumerated directory itself — those stand unchanged. What is new is
/// the ONE indirection DWD-26 introduces: the clone lives in the
/// operator's directory and is reached through the index link.
///
/// After the DWD-26 relocation the two adapters must STILL observe the
/// same clone surface — `RealVmHostState` through a platform-owned index
/// symlink resolving to a clone beside an operator-chosen master;
/// `SimVmHostState` through its in-memory clone map. Needs no guest boot
/// and no cgroup: it exercises only the clone indirection, over real
/// tempdirs and a real symlink.
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test]
async fn index_backed_clone_surface_stays_equivalent_across_adapters() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let alloc = AllocationId::new("vmhosteq-index-a").expect("valid alloc id");

    // Two DISTINCT real directories: the operator's rootfs-master dir
    // (where the clone lands beside the master) and the platform-owned
    // clone-index dir under a stand-in data_dir. NEITHER is a platform
    // staging dir NOR `/run`.
    let operator_dir = tmp.path().join("operator-rootfs");
    let data_dir = tmp.path().join("node-data");
    let index_dir = clone_index_dir(&data_dir);
    std::fs::create_dir_all(&operator_dir).expect("create operator rootfs dir");
    std::fs::create_dir_all(&index_dir).expect("create clone-index dir");

    // A real clone beside the operator master and a real index symlink
    // resolving to it — the on-disk shape `VmDriver::start` records
    // (link -> clone; ADR-0083 §D3f).
    // Synthetic observe/discard fixture: the clone location is arbitrary (a
    // per-test dir here) because VmHostState follows the INDEX LINK, not the
    // clone path. Staging it under `operator_dir` keeps the fixture compact.
    let plan = RootfsPlan::for_alloc(
        operator_dir.join("master.img"),
        0,
        &alloc,
        &operator_dir,
        &index_dir,
    );
    std::fs::write(plan.clone_dest(), b"clone-bytes").expect("stage the synthetic clone");
    std::os::unix::fs::symlink(plan.clone_dest(), plan.index_link())
        .expect("record the clone in the index");
    // Structural: the clone is NOT inside the index dir — a re-derivation
    // `index_dir.join(name)` would target the LINK, and the real clone
    // would leak. Reading the link is the only correct resolution.
    assert_ne!(plan.clone_dest().parent(), Some(index_dir.as_path()));

    // --- Real adapter: observe RESOLVES the link; discard removes
    // target-then-link, idempotently. ---
    let run_root = tmp.path().join("run");
    std::fs::create_dir_all(&run_root).expect("create run root");
    let real = RealVmHostState::new(PathBuf::from(CGROUP_ROOT), run_root, index_dir.clone());

    let obs = real.observe().await.expect("real observe");
    assert_eq!(
        obs.clones.get(&alloc),
        Some(&plan.clone_dest().to_path_buf()),
        "observe must report the clone the index link RESOLVES to (beside the operator master), \
         never the link's own path"
    );

    real.discard_artifacts(&alloc).await.expect("real discard");
    assert!(
        !plan.clone_dest().exists(),
        "discard must remove the clone the link resolves to (in the operator dir) — proof it read \
         the link rather than re-deriving a platform path"
    );
    assert!(!plan.index_link().exists(), "discard must remove the index link too");
    real.discard_artifacts(&alloc).await.expect("real discard is idempotent");
    assert!(
        !real.observe().await.expect("real observe").clones.contains_key(&alloc),
        "the real clone surface is empty after discard"
    );

    // --- Sim adapter: its in-memory clone map models the observable
    // surface directly; the observe/discard shape is byte-identical. ---
    let sim = SimVmHostState::new();
    sim.set_clone(alloc.clone(), plan.clone_dest().to_path_buf());
    assert!(
        sim.observe().await.expect("sim observe").clones.contains_key(&alloc),
        "sim observe reports the seeded clone"
    );
    sim.discard_artifacts(&alloc).await.expect("sim discard");
    assert!(
        !sim.observe().await.expect("sim observe").clones.contains_key(&alloc),
        "sim discard removes the clone — equivalent to the real adapter across the new indirection"
    );
    sim.discard_artifacts(&alloc).await.expect("sim discard is idempotent");
}

// ---------------------------------------------------------------------
// Node-local persistent volume (GH #348, ADR-0349) — `kill_scope`'s busy
// and refusal contract on both adapters.
//
// The real adapter takes its cgroup root as `RealVmHostState::new`'s
// first argument, so the refusal clauses run over a tempdir: no real
// scope, no guest, no VMM. A real BUSY scope (the last task held in exit)
// needs a held device and is `vm_host_state_scope_kill.rs`'s fixture; the
// busy clauses here run on `SimVmHostState` only.
//
// Not authored over a tempdir, and why: a `kill_scope` that returns `Ok`
// over a scope directory removes the directory, but the adapter's own
// `cgroup.kill` write creates a regular file in a tempdir scope, so the
// removal meets a non-empty directory; a regular file is not pollable
// for `cgroup.events` changes either. "Removal only after the `0`
// reading" and "a scope with no `cgroup.events` is removed" are therefore
// shown on the kernel by `vm_host_state_scope_kill.rs`.
// ---------------------------------------------------------------------

/// The longest a refusal may take before it counts as retried. The settle bound is
/// `KILL_SCOPE_SETTLE_BOUND` (3 s); a refusal that returns "at once" is far under it.
const AT_ONCE: std::time::Duration = std::time::Duration::from_secs(1);

/// A tempdir cgroup root and the scope directory of `alloc` under it, as
/// `RealVmHostState` resolves it. Returns the root guard, the host and the
/// scope directory.
fn real_host_over_tempdir(
    alloc: &AllocationId,
) -> (tempfile::TempDir, RealVmHostState, std::path::PathBuf) {
    let root = tempfile::TempDir::new().expect("tempdir");
    let scope_dir = CgroupPath::for_alloc(alloc).resolve(root.path());
    std::fs::create_dir_all(&scope_dir).expect("create the scope directory");
    let host = RealVmHostState::new(
        root.path().to_path_buf(),
        root.path().join("run"),
        root.path().join("clone-index"),
    );
    (root, host, scope_dir)
}

/// ADR-0349 item 4 ("a non-`ResourceBusy` error is returned at once"; "`Err` kinds other than
/// busy are refusals"), the cgroup.kill write refused. Owner path: `VmHostState::kill_scope` then
/// `observe`. Fault: `cgroup.kill` is a directory on the real adapter (the write fails with a
/// non-`NotFound` kind); a scripted `PermissionDenied` on the sim. Oracle: the kind is not busy,
/// the call took far less than the settle bound, and the scope is still observed.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(host_pure)]
#[tokio::test]
async fn a_refused_cgroup_kill_write_returns_its_kind_at_once_and_leaves_the_scope_observed() {
    // --- Real adapter: the write to `cgroup.kill` fails. ---
    let alloc = AllocationId::new("vmhosteq-refuse-a").expect("valid alloc id");
    let (_root, real, scope_dir) = real_host_over_tempdir(&alloc);
    std::fs::create_dir(scope_dir.join("cgroup.kill")).expect("make cgroup.kill unwritable");

    let started = std::time::Instant::now();
    let refusal = real
        .kill_scope(&CgroupPath::for_alloc(&alloc))
        .await
        .expect_err("a refused cgroup.kill write must surface as Err");
    let real_elapsed = started.elapsed();

    assert_ne!(
        refusal.kind(),
        std::io::ErrorKind::ResourceBusy,
        "a refused write is a refusal, never the retry-later busy kind: {refusal:?}"
    );
    assert!(
        real_elapsed < AT_ONCE,
        "a refused cgroup.kill write must return at once, took {real_elapsed:?}"
    );
    assert!(
        real.observe().await.expect("real observe").scopes.contains_key(&alloc),
        "a refused kill leaves the scope observed"
    );

    // --- Sim adapter: the same contract from a scripted refusal. ---
    let sim = SimVmHostState::new();
    sim.set_scope(alloc.clone(), BTreeSet::from([4242]));
    sim.script_kill_scope_failure(std::io::ErrorKind::PermissionDenied);

    let sim_refusal = sim
        .kill_scope(&CgroupPath::for_alloc(&alloc))
        .await
        .expect_err("a scripted refusal must surface as Err");

    assert_eq!(sim_refusal.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(
        sim.observe().await.expect("sim observe").scopes.contains_key(&alloc),
        "a refused kill leaves the scope observed"
    );
    sim.kill_scope(&CgroupPath::for_alloc(&alloc))
        .await
        .expect("the refusal was not retried inside the call: the next call proceeds and removes");
    assert!(!sim.observe().await.expect("sim observe").scopes.contains_key(&alloc));
}

/// ADR-0349 item 3 ("a removal that fails is a real error returned at once", replacing today's
/// retry of whatever the error). Owner path: `RealVmHostState::kill_scope` over a tempdir root.
/// Fault: the scope directory holds a child directory, so its removal fails `ENOTEMPTY`, a
/// non-busy kind; the scope has no `cgroup.events`, so there is no wait. Oracle: the kind is not
/// busy, the call returns at once (not after the settle retries), the scope is still observed.
/// PENDING until the settle loop stops retrying every error kind.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(host_pure)]
#[tokio::test]
async fn a_failed_removal_with_a_non_busy_kind_returns_at_once_without_retry() {
    let alloc = AllocationId::new("vmhosteq-remove-a").expect("valid alloc id");
    let (_root, real, scope_dir) = real_host_over_tempdir(&alloc);
    std::fs::create_dir(scope_dir.join("child")).expect("make the removal fail non-empty");

    let started = std::time::Instant::now();
    let failure = real
        .kill_scope(&CgroupPath::for_alloc(&alloc))
        .await
        .expect_err("a non-empty scope directory cannot be removed");
    let elapsed = started.elapsed();

    assert_ne!(
        failure.kind(),
        std::io::ErrorKind::ResourceBusy,
        "a removal that fails with ENOTEMPTY is not the busy kind: {failure:?}"
    );
    assert!(
        elapsed < AT_ONCE,
        "a removal failure that is not busy must return at once, never retried: took {elapsed:?}"
    );
    assert!(
        real.observe().await.expect("real observe").scopes.contains_key(&alloc),
        "the scope whose removal failed is still observed"
    );
}

/// ADR-0349 item 4 ("an absent scope is `Ok`"). Owner path: `VmHostState::kill_scope`. Oracle: `Ok`
/// and an unchanged observation, whether the whole slice is absent or only this scope is.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(host_pure)]
#[tokio::test]
async fn a_kill_of_a_scope_that_is_not_there_is_ok_and_changes_nothing_on_both_adapters() {
    let alloc = AllocationId::new("vmhosteq-absent-a").expect("valid alloc id");
    let other = AllocationId::new("vmhosteq-absent-other").expect("valid alloc id");

    // Real: first the workloads slice itself does not exist, then it exists with only `other`.
    let root = tempfile::TempDir::new().expect("tempdir");
    let real = RealVmHostState::new(
        root.path().to_path_buf(),
        root.path().join("run"),
        root.path().join("clone-index"),
    );
    real.kill_scope(&CgroupPath::for_alloc(&alloc)).await.expect("no slice at all is Ok");
    std::fs::create_dir_all(CgroupPath::for_alloc(&other).resolve(root.path()))
        .expect("create the other scope");
    let before = real.observe().await.expect("real observe");
    real.kill_scope(&CgroupPath::for_alloc(&alloc)).await.expect("an absent scope is Ok");
    assert_eq!(
        real.observe().await.expect("real observe"),
        before,
        "killing an absent scope changes nothing, the other scope included"
    );

    // Sim: the same, with `other` seeded.
    let sim = SimVmHostState::new();
    sim.set_scope(other.clone(), BTreeSet::new());
    let sim_before = sim.observe().await.expect("sim observe");
    sim.kill_scope(&CgroupPath::for_alloc(&alloc)).await.expect("an absent scope is Ok");
    assert_eq!(sim.observe().await.expect("sim observe"), sim_before);
}

/// ADR-0349 item 5 (sim only; the real busy scope is `vm_host_state_scope_kill.rs`'s fixture).
/// Owner path: `kill_scope` over a held scope. Fault: the scope is held. Oracle: `Err(ResourceBusy)`
/// leaves the scope observed with its pids; once the hold ends, `Ok` implies the scope is absent
/// from `observe`; a second kill is idempotent.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(host_pure)]
#[tokio::test]
async fn a_busy_scope_stays_observed_until_the_kill_succeeds_and_ok_means_absent_sim() {
    let sim = SimVmHostState::new();
    let alloc = AllocationId::new("vmhosteq-busy-a").expect("valid alloc id");
    let scope = CgroupPath::for_alloc(&alloc);
    sim.set_scope(alloc.clone(), BTreeSet::from([4242]));
    sim.hold_scope_while(&alloc, || true);

    let busy = sim.kill_scope(&scope).await.expect_err("a held scope is busy");

    assert_eq!(busy.kind(), std::io::ErrorKind::ResourceBusy);
    let observed = sim.observe().await.expect("sim observe");
    assert_eq!(
        observed.scopes.get(&alloc).map(|facts| facts.pids.clone()),
        Some(BTreeSet::from([4242])),
        "Err(ResourceBusy) leaves the scope observed with its pids"
    );

    sim.release_scope_hold(&alloc);
    sim.kill_scope(&scope).await.expect("the hold ended: the kill succeeds");
    assert!(
        !sim.observe().await.expect("sim observe").scopes.contains_key(&alloc),
        "Ok from kill_scope implies the scope is absent from observe"
    );
    sim.kill_scope(&scope).await.expect("a second kill of the now-absent scope is Ok");
}
