//! GH #295 S-ND295-40 — the VMM adapter reports every TAP queue-attach
//! failure in its own terms, observed through the PUBLIC `Vmm::create`
//! contract (FD § "Core `VmmError` additions"), never a private mapping
//! function.
//!
//! `CloudHypervisorVmm::create` attaches one queue to the down persistent TAP
//! it was configured with (D-295-R2, DELIVER step 05-03) and maps each
//! `overdrive_netlink::TapQueueError` one-for-one onto a public `VmmError`:
//! the four sourced stages onto `VmmError::TapQueue { stage, .. }`, and the
//! two checked postconditions onto `VmmError::TapQueuePostcondition`. This
//! body drives `create` against real kernel TAPs and asserts the mapped
//! `VmmError` for every failure a real kernel state can actually produce:
//!
//! - `TapQueueStage::Attach` — a queue is already held on the single-queue
//!   TAP, so the adapter's attach gets `EBUSY`;
//! - `TapQueueViolation::Flags` — an absent name makes `TUNSETIFF` create a
//!   fresh non-persistent device whose read-back flags omit `IFF_PERSIST`;
//! - `TapQueueViolation::NotDown` — the persistent TAP is administratively up
//!   at attach.
//!
//! `TapQueueStage::{Open, FlagsReadBack, AdminStateReadBack}` are NOT
//! asserted: opening `/dev/net/tun`, `TUNGETIFF` on the freshly opened queue,
//! and `SIOCGIFFLAGS` on the named TAP all succeed given a real kernel and an
//! open descriptor, so no real kernel state produces those three sourced
//! stages. Their mapping is covered by the adapter's own unit mapping, not by
//! this contract body.
//!
//! Gated `integration-tests,kvm-tests` (see `tests/integration.rs`): `create`
//! must pass its rootfs-clone / hypervisor-present / confined-paths stages —
//! which need the real staged kernel + rootfs + `cloud-hypervisor` the
//! `VmFixture` provides — before it reaches the queue attach, so only the TAP
//! is wrong and the failure is attributable to the queue attach alone. Runs
//! as root on scratch TAPs named outside the production `ovd-tp-` family, each
//! deleted by RAII; the `overdrive-host` integration binary is in the
//! `host-kernel-shared` nextest group.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::missing_const_for_fn,
    clippy::panic,
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::unwrap_used,
    reason = "Tier-3 native fixtures fail fast, and CONTRACT_SHAPE lines use exact mandated tokens"
)]

use std::num::NonZeroU8;
use std::path::Path;

use overdrive_core::AllocationId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::traits::vmm::{TapQueueStage, TapQueueViolation, Vmm, VmmError};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, KernelCmdline, KernelImage, MemoryPlan, RootfsPlan,
    VmConfig, VmConfinement, VmNetworkAttachment, VmRunDir, VmmIdentity,
};
use overdrive_host::CloudHypervisorVmm;
use overdrive_netlink::{
    Client, TapLinkState, TapQueue, attach_tap_queue, block_on_host_netlink, create_persistent_tap,
};
use overdrive_testing::vm_fixture::{VmFixture, default_staging_root};
use tempfile::TempDir;

/// `IFF_PERSIST`: the bit a fresh non-persistent device read-back is missing.
const IFF_PERSIST: u16 = 0x0800;

/// Fail closed unless the test process is root (the metal runner's uid); the
/// TAP creation and queue attach both need `CAP_NET_ADMIN`.
fn require_root() {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    let euid = unsafe { libc::geteuid() };
    assert_eq!(euid, 0, "this test creates kernel TAPs and must run as root (cargo xtask metal run)");
}

/// A scratch-TAP name unique to this process and tag, outside `ovd-tp-`.
fn scratch_name(tag: &str) -> String {
    let name = format!("nd40{tag}{:05}", std::process::id() % 100_000);
    assert!(name.len() < libc::IFNAMSIZ, "scratch TAP name {name} fits IFNAMSIZ");
    name
}

/// The persistent-TAP state `name` reads back through rtnetlink, on a fresh
/// OS thread so it does not nest a runtime inside this `#[tokio::test]`.
fn tap_state(name: &str) -> TapLinkState {
    let owned = name.to_owned();
    std::thread::spawn(move || {
        block_on_host_netlink(|| async move { Client::new()?.observe_persistent_tap(&owned).await })
    })
    .join()
    .expect("netlink observe thread")
    .unwrap_or_else(|error| panic!("reading back TAP state: {error}"))
}

/// Raise `name` administratively (fresh OS thread; see [`tap_state`]).
fn raise(name: &str) {
    let owned = name.to_owned();
    std::thread::spawn(move || {
        block_on_host_netlink(|| async move { Client::new()?.set_link_up(&owned).await })
    })
    .join()
    .expect("netlink raise thread")
    .unwrap_or_else(|error| panic!("raising TAP {name:?}: {error}"));
}

/// Delete `name` if present (fresh OS thread; failure logged, never absorbed).
fn delete_link(name: &str) {
    let owned = name.to_owned();
    let deleted = std::thread::spawn(move || {
        block_on_host_netlink(|| async move { Client::new()?.del_link(&owned).await })
    })
    .join()
    .expect("netlink delete thread");
    if let Err(error) = deleted {
        eprintln!("cleanup: deleting scratch link failed: {error}");
    }
}

/// A persistent single-queue TAP owned by uid 0, created down, deleted on drop.
struct ScratchTap {
    name: String,
}

impl ScratchTap {
    fn create(tag: &str) -> Self {
        let name = scratch_name(tag);
        assert_eq!(tap_state(&name), TapLinkState::Absent, "scratch TAP {name} must not pre-exist");
        create_persistent_tap(&name, 0)
            .unwrap_or_else(|error| panic!("creating scratch TAP {name}: {error}"));
        Self { name }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for ScratchTap {
    fn drop(&mut self) {
        delete_link(&self.name);
    }
}

/// Deletes a name that must stay absent, should a failing attach have left a
/// fresh non-persistent device behind.
struct AbsentNameGuard(String);

impl Drop for AbsentNameGuard {
    fn drop(&mut self) {
        delete_link(&self.0);
    }
}

fn read_kernel_header(path: &Path) -> Vec<u8> {
    use std::io::Read;
    let file = std::fs::File::open(path).expect("open staged kernel for header read");
    let mut buf = Vec::new();
    file.take(KERNEL_MAGIC_WINDOW as u64).read_to_end(&mut buf).expect("read kernel header");
    buf
}

/// A fully valid `VmConfig` whose only wrong element is the named TAP, so a
/// `create` failure is attributable to the queue attach. Mirrors
/// `vmm_ficlone_per_launch`'s staging.
fn vm_config_for(fixture: &VmFixture, staging: &Path, tag: &str, tap: &str) -> VmConfig {
    let master = staging.join(format!("{tag}-master.img"));
    std::fs::copy(&fixture.rootfs_path, &master).expect("stage a master rootfs copy");
    let master_bytes = std::fs::metadata(&master).expect("stat staged master").len();

    let alloc = AllocationId::new(&format!("nd295-40-{tag}")).expect("valid alloc id");
    let rootfs = RootfsPlan::for_alloc(
        master,
        master_bytes,
        &alloc,
        staging,
        Path::new("/run/overdrive/vm/clone-index"),
    );
    let run_dir = VmRunDir::for_alloc(&staging.join("run"), &alloc);
    std::fs::create_dir_all(run_dir.path()).expect("create run dir");

    let header = read_kernel_header(&fixture.kernel_path);
    let kernel = KernelImage::validate(fixture.kernel_path.clone(), HostArch::X86_64, &header)
        .expect("fixture-staged kernel validates for x86_64");

    VmConfig {
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
        network: Some(VmNetworkAttachment {
            tap: tap.to_owned(),
            mac: [0x02, 0x00, 0x00, 0x00, 0x00, 0x2a],
        }),
        cgroup_scope: CgroupPath::for_alloc(&alloc),
    }
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-40 — The VMM adapter launches from the queue and reports queue failures in their own terms
/// CONTRACT_SHAPE: bounded-change.
///
/// Each queue-attach failure a real kernel state can produce surfaces through
/// `Vmm::create` as its pinned `VmmError` value (`Attach`, `Flags`,
/// `NotDown`). `Open`, `FlagsReadBack`, and `AdminStateReadBack` are not
/// producible from a real kernel state (see the module docs) and are not
/// asserted.
#[tokio::test]
#[ignore = "pending DELIVER step 05-03 (S-ND295-40)"]
async fn every_tap_queue_error_maps_to_its_vmm_queue_error() {
    require_root();
    let staging_root = default_staging_root();
    let fixture =
        VmFixture::provision(&staging_root).expect("fixture provisions (real kernel/rootfs/CH)");
    let vmm = CloudHypervisorVmm::new();

    // NotDown: a persistent single-queue TAP raised administratively.
    {
        let staging = TempDir::new_in(&staging_root).expect("staging dir");
        let tap = ScratchTap::create("nd");
        raise(tap.name());
        let config = vm_config_for(&fixture, staging.path(), "notdown", tap.name());
        let err = vmm.create(&config).await.expect_err("an up TAP refuses the queue attach");
        assert!(
            matches!(
                &err,
                VmmError::TapQueuePostcondition {
                    tap: reported,
                    violation: TapQueueViolation::NotDown,
                } if reported == tap.name()
            ),
            "an administratively-up TAP maps to TapQueuePostcondition::NotDown, got {err:?}"
        );
    }

    // Flags: an absent name makes the attach create a fresh non-persistent
    // device whose read-back flags omit IFF_PERSIST.
    {
        let staging = TempDir::new_in(&staging_root).expect("staging dir");
        let absent = scratch_name("fl");
        assert_eq!(tap_state(&absent), TapLinkState::Absent, "the Flags name must not pre-exist");
        let _guard = AbsentNameGuard(absent.clone());
        let config = vm_config_for(&fixture, staging.path(), "flags", &absent);
        let err = vmm.create(&config).await.expect_err("a fresh non-persistent device is refused");
        match &err {
            VmmError::TapQueuePostcondition {
                tap: reported,
                violation: TapQueueViolation::Flags { observed },
            } => {
                assert_eq!(reported, &absent, "the violation names the attach target");
                assert_eq!(
                    observed & IFF_PERSIST,
                    0,
                    "the fresh device is not persistent: {observed:#06x}"
                );
            }
            other => panic!("an absent name maps to TapQueuePostcondition::Flags, got {other:?}"),
        }
    }

    // Attach: a queue is already held on the single-queue TAP, so the
    // adapter's attach gets EBUSY.
    {
        let staging = TempDir::new_in(&staging_root).expect("staging dir");
        let tap = ScratchTap::create("bu");
        let _held: TapQueue = attach_tap_queue(tap.name()).expect("hold the single queue");
        let config = vm_config_for(&fixture, staging.path(), "attach", tap.name());
        let err = vmm.create(&config).await.expect_err("a busy TAP refuses a second queue");
        assert!(
            matches!(
                &err,
                VmmError::TapQueue { tap: reported, stage: TapQueueStage::Attach, .. }
                    if reported == tap.name()
            ),
            "an already-attached queue maps to TapQueue {{ stage: Attach }}, got {err:?}"
        );
    }
}
