//! S-VM-85 (step 03-09, DWD-26 / ADR-0083 §§D3f-D3h, GH #42) — the
//! clone-index link outlives the clone it points at, on every
//! interleaving. `VmDriver`'s component-scope acceptance suite for the
//! clone-index ordering invariant, against `SimVmm` over a REAL
//! filesystem.
//!
//! Component scope, the same carve-out ADR-0082 §D4 already justifies for
//! S-VM-76 (`vm_driver_stop_totality.rs`): `SimVmm` is injected at the
//! `Vmm` port boundary and real `tempfile::TempDir`s supply the
//! clone-index directory and the operator rootfs-master directory as two
//! distinct real directories. There is NO guest boot — nothing spawns
//! cloud-hypervisor — so this runs under Lima in the default lane, exactly
//! like `vm_driver_stop_totality.rs`. It is registered in
//! `crates/overdrive-worker/tests/acceptance.rs`.
//!
//! ## The invariant, and why it is the mutation target
//!
//! ADR-0083 §D3f: the link is created BEFORE the clone (on the start path)
//! and removed AFTER the clone (on stop / cleanup). Therefore at every
//! instant a clone that exists has a link that exists — contrapositive
//! *no link ⇒ no clone* — so enumerating links enumerates a SUPERSET of
//! live clones and the reclamation sweep cannot miss one. A mutation that
//! swaps EITHER ordering reopens exactly the invisible-orphan leak S-VM-84
//! closes, and MUST be killed here.
//!
//! Both halves of the ordering are guarded DIRECTLY, because neither is a
//! `cargo-mutants` mutant: there is no statement-reorder operator, and the
//! whole-body→`()` mutant is already killed by the quiescent end-state
//! checks (Parts B/D). ADR-0083 § Consequences designates this test as the
//! sole guard of the ordering precisely because a comment would not hold
//! it, so each half is pinned by a deterministic witness:
//!
//! - **create-before** — Part A's `RecordsFsAtCreate` witness observes the
//!   filesystem at the instant `Vmm::create` (the FICLONE) is entered and
//!   asserts the link already exists while the clone does not. Creating the
//!   clone first would find the link absent there.
//! - **remove-after** — the dedicated
//!   `stop_keeps_the_index_link_when_the_clone_removal_fails` test
//!   interrupts the stop sequence at its first removal by making the clone
//!   un-removable (a directory `unlink(2)` cannot delete), and asserts the
//!   index link SURVIVES. Reversing the two removals drops the link first —
//!   before the clone is gone — leaving the "clone without a link" orphan
//!   §D3f marks *unreachable by construction*; that test goes RED on
//!   exactly the reversal (Part D's both-gone end state cannot, being
//!   order-independent).

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::SpiffeId;
use overdrive_core::guest_transport::ClaimedGuestCid;
use overdrive_core::id::AllocationId;
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::{AllocationSpec, Driver, DriverPayload, Resources, VmPayload};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmProbeError,
};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, RootfsPlan, VmConfig, VmConfinement, VmmIdentity,
};
use overdrive_host::RealVmHostState;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm};
use overdrive_worker::VmDriver;
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::vm_driver::VmHostLayout;
use tempfile::TempDir;

use super::support::BeaconHarness;

// ---------------------------------------------------------------------
// Fixtures — a per-test operator rootfs-master dir + a distinct
// platform-owned clone-index dir, both real directories on disk.
// ---------------------------------------------------------------------

const CGROUP_ROOT: &str = "/does-not-need-to-exist-for-the-clone-surface";

fn probe_runner() -> Arc<ProbeRunner> {
    Arc::new(ProbeRunner::new(
        Arc::new(SimTcpProber::new()),
        Arc::new(SimHttpProber::new()),
        Arc::new(SimClock::new()),
        Arc::new(SimObservationStore::single_peer(
            NodeId::new("vm-clone-index").expect("valid node ID"),
            0,
        )) as Arc<dyn ObservationStore>,
    ))
}

fn operator_rootfs_path(tmp: &TempDir) -> PathBuf {
    tmp.path().join("operator-rootfs").join("master.img")
}

fn kernel_path(tmp: &TempDir) -> PathBuf {
    tmp.path().join("vmlinuz")
}

fn clone_index_dir(tmp: &TempDir) -> PathBuf {
    // Under a stand-in durable data_dir — deliberately NOT `/run` and NOT
    // the operator dir where the clone lands.
    tmp.path().join("node-data").join("vm").join("clone-index")
}

fn stage_artifacts(tmp: &TempDir) {
    let master = operator_rootfs_path(tmp);
    std::fs::create_dir_all(master.parent().unwrap()).expect("create operator rootfs dir");
    std::fs::write(&master, b"deterministic-fixture-rootfs-bytes").expect("write master rootfs");
    let mut header = vec![0u8; KERNEL_MAGIC_WINDOW];
    header[..4].copy_from_slice(b"\x7fELF");
    std::fs::write(kernel_path(tmp), &header).expect("stage the synthetic kernel");
}

fn build_layout(tmp: &TempDir) -> VmHostLayout {
    stage_artifacts(tmp);
    VmHostLayout {
        cgroup_root: tmp.path().join("cgroup"),
        run_dir_root: tmp.path().join("run"),
        clone_index_dir: clone_index_dir(tmp),
        clone_staging_dir: tmp.path().join("clone-staging"),
        arch: HostArch::X86_64,
        confinement: VmConfinement::confined(
            VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
            1024,
        ),
    }
}

fn build_spec(alloc: &AllocationId, tmp: &TempDir, cid_offset: u32) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/workload/clone-index-test/alloc/x")
            .expect("valid spiffe id"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/sbin/init".to_owned(),
            args: vec![],
            kernel: kernel_path(tmp),
            rootfs: operator_rootfs_path(tmp),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        transport: Some(super::support::transport(
            "100.96.0.6".parse().expect("valid fixture guest address"),
            "100.96.0.5".parse().expect("valid fixture gateway"),
            cid_offset,
        )),
        service_ports: Vec::new(),
    }
}

/// A `VmDriver` over `vmm` whose beacon intake is a [`BeaconHarness`]
/// coupled to `sim` (the VMM `vmm` delegates to).
async fn build_driver(
    sim: &SimVmm,
    vmm: Arc<dyn Vmm>,
    layout: VmHostLayout,
) -> (VmDriver, SimClock, BeaconHarness) {
    let clock = SimClock::new();
    let fs: Arc<dyn overdrive_core::traits::CgroupFs> = Arc::new(SimCgroupFs::new());
    let cgroup_accounting: Arc<dyn overdrive_core::traits::cgroup_accounting::CgroupAccounting> =
        Arc::new(SimCgroupAccounting::new());
    let wiring =
        overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(clock.clone()));
    assert!(wiring.supervisor().open_after_boot());
    let beacon = BeaconHarness::new(sim);
    let driver = VmDriver::new(
        beacon.vmm(vmm),
        Arc::new(clock.clone()),
        fs,
        cgroup_accounting,
        probe_runner(),
        wiring.gate(),
        layout,
        beacon.intake().await,
    );
    (driver, clock, beacon)
}

/// Spawn `start`, the guest connects with `READY` once its VMM exists,
/// await `start` to `Ok`. The guest connection closes on return, as the
/// dropped stream did.
async fn start_with_beacon(driver: &VmDriver, spec: &AllocationSpec, beacon: &BeaconHarness) {
    let driver = driver.clone();
    let spec_owned = spec.clone();
    let start_task = tokio::spawn(async move { driver.start(&spec_owned).await });

    let _guest = beacon.dial_ready(spec).await;

    start_task.await.expect("start task did not panic").expect("start resolves Ok on beacon-win");
}

/// A `Vmm` decorator that captures, at the instant `Vmm::create` (the
/// FICLONE) is ENTERED, whether the clone-index link already exists and
/// whether the clone does NOT yet — the deterministic witness of the
/// link-before-clone creation ordering (ADR-0083 §D3f). Correct code
/// creates the link BEFORE `vmm.create`, so at entry the link exists and
/// the clone does not; a mutation that creates the clone first would find
/// the link absent here.
#[derive(Clone)]
struct RecordsFsAtCreate {
    inner: SimVmm,
    at_create: Arc<Mutex<Option<(bool, bool)>>>,
}

#[async_trait]
impl Vmm for RecordsFsAtCreate {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    async fn probe(&self) -> VmmResult<(), VmmProbeError> {
        self.inner.probe().await
    }

    async fn create(&self, config: &VmConfig, vsock: ClaimedGuestCid) -> VmmResult<VmProcess> {
        let link_exists = std::fs::symlink_metadata(config.rootfs.index_link()).is_ok();
        let clone_exists = config.rootfs.clone_dest().exists();
        *self.at_create.lock().expect("recorder mutex not poisoned") =
            Some((link_exists, clone_exists));
        self.inner.create(config, vsock).await
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

fn real_host(layout: &VmHostLayout) -> RealVmHostState {
    RealVmHostState::new(
        PathBuf::from(CGROUP_ROOT),
        layout.run_dir_root.clone(),
        layout.clone_index_dir.clone(),
    )
}

/// `true` iff a symlink exists at `p`, whether or not its target does.
/// `Path::exists()` FOLLOWS the link, so a DANGLING link (target not yet
/// created, or already removed) reads as absent — the index link is
/// deliberately dangling at two of the invariant's interruption points
/// (link-before-clone, and clone-removed-before-link), so the link's
/// presence must be checked with `symlink_metadata`, which stats the link
/// itself.
fn link_present(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

// ---------------------------------------------------------------------
// S-VM-85 — `no link ⇒ no clone` at every interruption point.
// ---------------------------------------------------------------------

/// S-VM-85 / `@ac-08` `@real-io` `@mandatory:mutation_target` — the
/// clone-index link's lifetime CONTAINS the clone's.
#[tokio::test]
async fn no_clone_index_link_implies_no_clone_at_every_interruption_point() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let index_dir = layout.clone_index_dir.clone();
    let staging_dir = layout.clone_staging_dir.clone();
    let master = operator_rootfs_path(&tmp);
    let master_bytes = std::fs::metadata(&master).expect("stat master").len();

    // === Part A — CREATION ordering: link strictly precedes the clone. ===
    let recorder: Arc<Mutex<Option<(bool, bool)>>> = Arc::new(Mutex::new(None));
    let sim = SimVmm::new();
    let vmm = RecordsFsAtCreate { inner: sim.clone(), at_create: recorder.clone() };
    let (driver, clock, beacon) = build_driver(&sim, Arc::new(vmm), layout.clone()).await;

    let alloc = AllocationId::new("alloc-clone-index-a").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp, 0);
    start_with_beacon(&driver, &spec, &beacon).await;

    let (link_at_create, clone_at_create) =
        recorder.lock().expect("recorder mutex").take().expect("Vmm::create was entered");
    assert!(
        link_at_create,
        "the clone-index link MUST exist at the instant the FICLONE (Vmm::create) is entered — the \
         link is created BEFORE the clone (§D3f)"
    );
    assert!(
        !clone_at_create,
        "the clone MUST NOT exist yet at FICLONE entry — the link strictly precedes the clone"
    );

    // The running state: both the clone and its index link exist, and the
    // link RESOLVES to the clone beside the operator master — NOT into the
    // index dir (so a re-derivation would target the wrong path).
    let plan =
        RootfsPlan::for_alloc(master.clone(), master_bytes, &alloc, &staging_dir, &index_dir);
    assert!(plan.clone_dest().exists(), "the per-launch clone exists while running");
    assert!(link_present(plan.index_link()), "the clone-index link exists while running");
    assert_eq!(
        std::fs::read_link(plan.index_link()).expect("read the index link"),
        plan.clone_dest(),
        "the index link resolves to the clone beside the operator master"
    );
    assert_ne!(
        plan.clone_dest().parent(),
        Some(index_dir.as_path()),
        "the clone lives beside the operator master, NOT in the platform index dir"
    );

    // === Part B — reclamation over the running residue: observe resolves
    // the link; discard removes the clone (via read_link, in the operator
    // dir) then the link, idempotently. ===
    let host = real_host(&layout);
    let obs = host.observe().await.expect("observe");
    assert_eq!(
        obs.clones.get(&alloc),
        Some(&plan.clone_dest().to_path_buf()),
        "observe reports the clone the link RESOLVES to, not the link path"
    );
    host.discard_artifacts(&alloc).await.expect("discard");
    assert!(
        !plan.clone_dest().exists(),
        "discard removed the clone the link resolved to (operator dir) — it read the link, it did \
         not re-derive a platform path"
    );
    assert!(!link_present(plan.index_link()), "discard removed the index link too");
    host.discard_artifacts(&alloc).await.expect("discard is idempotent");

    // === Part C — the dangling-link residue (crash after clone removal,
    // before link removal): observe still yields an entry; discard
    // disposes it idempotently, never leaving a clone without a link. ===
    let dangling = AllocationId::new("alloc-clone-index-dangling").expect("valid alloc id");
    let dangling_plan =
        RootfsPlan::for_alloc(master.clone(), master_bytes, &dangling, &staging_dir, &index_dir);
    std::fs::create_dir_all(&index_dir).expect("index dir");
    std::fs::write(dangling_plan.clone_dest(), b"x").expect("stage a clone");
    std::os::unix::fs::symlink(dangling_plan.clone_dest(), dangling_plan.index_link())
        .expect("link -> clone");
    std::fs::remove_file(dangling_plan.clone_dest()).expect("simulate 'clone removed, link not'");
    // Invariant at this residue: a clone does NOT exist without a link —
    // here the clone is gone and only the (dangling) link remains.
    assert!(!dangling_plan.clone_dest().exists(), "residue: the clone is gone");
    assert!(link_present(dangling_plan.index_link()), "residue: a dangling link remains");
    let obs_dangling = host.observe().await.expect("observe over the dangling residue");
    assert!(
        obs_dangling.clones.contains_key(&dangling),
        "a dangling index link MUST still yield an observe entry (§D3f crash table)"
    );
    host.discard_artifacts(&dangling).await.expect("discard the dangling residue");
    assert!(!link_present(dangling_plan.index_link()), "discard disposes the dangling link");
    host.discard_artifacts(&dangling).await.expect("discard is idempotent over an absent residue");

    // === Part D — the stop path removes the clone AND its link; the
    // post-stop residue is nothing (§D3f point 4). ===
    let alloc_stop = AllocationId::new("alloc-clone-index-stop").expect("valid alloc id");
    let spec_stop = build_spec(&alloc_stop, &tmp, 1);
    start_with_beacon(&driver, &spec_stop, &beacon).await;
    let stop_plan =
        RootfsPlan::for_alloc(master.clone(), master_bytes, &alloc_stop, &staging_dir, &index_dir);
    assert!(
        stop_plan.clone_dest().exists() && link_present(stop_plan.index_link()),
        "running: both present"
    );

    let handle =
        overdrive_core::traits::driver::AllocationHandle { alloc: alloc_stop.clone(), pid: None };
    // `stop` writes SHUTDOWN then awaits `clock.sleep(SHUTDOWN_DEADLINE)`
    // on the injected `SimClock`; drive it in a task and tick the clock so
    // the deadline elapses (mirrors `vm_driver_stop_totality.rs`).
    let driver_for_stop = driver.clone();
    let handle_owned = handle.clone();
    let stop_task = tokio::spawn(async move { driver_for_stop.stop(&handle_owned).await });
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    clock.tick(Duration::from_secs(2));
    stop_task.await.expect("stop task did not panic").expect("stop returns Ok");
    assert!(!stop_plan.clone_dest().exists(), "stop removed the clone");
    assert!(!link_present(stop_plan.index_link()), "stop removed the index link");
}

// ---------------------------------------------------------------------
// S-VM-85 (remove-after half) — the index link OUTLIVES a clone whose
// removal fails. This is the guard Part D above cannot be: Part D's
// both-gone end state is ORDER-INDEPENDENT, so reversing the two removals
// still leaves both gone and Part D stays GREEN. Here the clone is made
// un-removable, so the two orderings diverge at a quiescent end state:
//   clone-first (correct):  clone removal fails -> link KEPT  (clone ⇒ link)
//   link-first  (reversed): link removed first  -> clone with NO link (orphan)
// The assertion is RED on exactly the reversed order — it is the sole
// guard of the remove-after half of §D3f's ordering.
// ---------------------------------------------------------------------

/// S-VM-85 / `@ac-08` `@real-io` `@mandatory:mutation_target` — `stop` must
/// remove the clone BEFORE its index link, so a clone that cannot be
/// removed (operator rootfs dir gone read-only / `EROFS` / `EACCES` between
/// the FICLONE and stop) keeps its index entry rather than becoming the
/// invisible orphan the reclamation sweep can never enumerate.
#[tokio::test]
async fn stop_keeps_the_index_link_when_the_clone_removal_fails() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let index_dir = layout.clone_index_dir.clone();
    let staging_dir = layout.clone_staging_dir.clone();
    let master = operator_rootfs_path(&tmp);
    let master_bytes = std::fs::metadata(&master).expect("stat master").len();
    let sim = SimVmm::new();
    let (driver, clock, beacon) = build_driver(&sim, Arc::new(sim.clone()), layout.clone()).await;

    let alloc = AllocationId::new("alloc-clone-index-stuck").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp, 0);
    start_with_beacon(&driver, &spec, &beacon).await;
    let plan =
        RootfsPlan::for_alloc(master.clone(), master_bytes, &alloc, &staging_dir, &index_dir);
    assert!(
        plan.clone_dest().exists() && link_present(plan.index_link()),
        "running: both the clone and its index link exist"
    );

    // Make the clone un-removable by `unlink(2)`: replace the clone FILE
    // with a DIRECTORY at the same path, so `remove_file` returns `EISDIR`
    // — a non-`NotFound` error. The index link keeps pointing at that path.
    std::fs::remove_file(plan.clone_dest()).expect("remove the staged clone file");
    std::fs::create_dir(plan.clone_dest())
        .expect("stage a directory the clone removal cannot unlink");
    assert!(
        link_present(plan.index_link()),
        "precondition: the index link is still present before stop"
    );

    // Drive `stop` to completion (mirrors Part D — SHUTDOWN then a clock
    // tick past `VM_SHUTDOWN_REQUEST_DEADLINE`). `stop` stays best-effort:
    // the un-removable clone must not make it return `Err` or panic.
    let handle =
        overdrive_core::traits::driver::AllocationHandle { alloc: alloc.clone(), pid: None };
    let driver_for_stop = driver.clone();
    let handle_owned = handle.clone();
    let stop_task = tokio::spawn(async move { driver_for_stop.stop(&handle_owned).await });
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    clock.tick(Duration::from_secs(2));
    stop_task
        .await
        .expect("stop task did not panic")
        .expect("stop returns Ok despite the un-removable clone");

    // The clone (now an un-unlinkable directory) is still on disk, so its
    // index link MUST have outlived it — removing the link first (the
    // reversed ordering) would strand the clone as the invisible orphan
    // §D3f marks unreachable by construction.
    assert!(
        link_present(plan.index_link()),
        "the index link MUST outlive a clone whose removal failed (non-NotFound): removing it \
         first strands the clone as the invisible orphan §D3f forbids (§D3h 'no link ⇒ no clone')"
    );
    // Housekeeping: drop the stand-in directory so TempDir cleanup is clean.
    let _ = std::fs::remove_dir_all(plan.clone_dest());
}
