//! S-PIG-132 — a failed in-place `BACKEND_MAP` restore during the rollback of a
//! pre-commit `update_service` failure is returned as
//! `DataplaneError::ServiceApplyRollback` (R-60), never only logged.
//!
//! Owner path: `Dataplane::update_service` on the production [`EbpfDataplane`]
//! (real aya maps, built through `EbpfDataplane::new_with_pin_dir`), reaching
//! `rollback_service_apply`. The fault is a call-ordinal stimulus on the
//! crate-private `BACKEND_MAP` seam ([`BackendMapOps`]): this module's decorator
//! wraps the dataplane's real handle, delegates every operation to it, and fails
//! chosen `insert` calls. A map-level fault could not fail only the restore: the
//! forward in-place write and its restore are the same operation on the same key.
//! Only the call ordinal separates them.
//!
//! Evidence tier T3 (real kernel maps); `SimDataplane` does not model rollback.
//! The lane runs as root through `cargo xtask metal run --` (bpffs, cgroup v2, a
//! veth pair for the loader's XDP attach).
//!
//! The decorator is test support. It lives only in this
//! `#[cfg(all(test, feature = "integration-tests"))]` module; nothing in
//! production reads the arming state.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    reason = "a Tier-3 scenario asserts its own preconditions; protocol names are prose"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use aya::maps::MapError;
use overdrive_core::dataplane::{Proto, ServiceFrontend};
use overdrive_core::id::{BackendId, ServiceVip, SpiffeId};
use overdrive_core::traits::dataplane::{
    Backend, Dataplane, DataplaneError, ServiceApplyRollbackFailure, ServiceApplyStage,
};
use overdrive_netlink::{Client, NetlinkError, block_on_host_netlink};
use parking_lot::Mutex;

use crate::EbpfDataplane;
use crate::backend_map::BackendMapOps;
use crate::gateway_connect::BackendIdentityState;
use crate::maps::wire::ServiceKey;
use crate::maps::{BackendEntryPod, BackendKeyPod};

/// Port of the Service frontend.
const FRONTEND_PORT: u16 = 8080;

// ---------------------------------------------------------------------
// Test support: the fault decorator over the `BACKEND_MAP` seam.
// ---------------------------------------------------------------------

/// One `insert` seen while the decorator was armed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InsertAttempt {
    id: u32,
    failed: bool,
}

/// Test-side arming state. Nothing in production reads it.
#[derive(Default)]
struct FaultPlan {
    armed: bool,
    /// 1-based ordinals of the `insert` calls since arming that fail.
    fail_inserts: BTreeSet<u32>,
    inserts_seen: u32,
    attempts: Vec<InsertAttempt>,
}

/// The test's handle on the decorator it installed.
#[derive(Clone)]
struct Faults(Arc<Mutex<FaultPlan>>);

impl Faults {
    /// Fail the `n`th `insert` after this call for every `n` in `fail_inserts`.
    fn arm(&self, fail_inserts: &[u32]) {
        let mut plan = self.0.lock();
        plan.armed = true;
        plan.fail_inserts = fail_inserts.iter().copied().collect();
        plan.inserts_seen = 0;
        plan.attempts.clear();
    }

    fn disarm(&self) {
        self.0.lock().armed = false;
    }

    fn insert_attempts(&self) -> Vec<InsertAttempt> {
        self.0.lock().attempts.clone()
    }
}

/// The failure the decorator injects: the shape of a rejected `bpf(2)` update.
fn injected_failure() -> MapError {
    MapError::SyscallError(aya::sys::SyscallError {
        call: "bpf_map_update_elem",
        io_error: std::io::Error::from_raw_os_error(libc::ENOSPC),
    })
}

/// Delegates every operation to the real `BACKEND_MAP` handle and fails the
/// `insert` calls the armed plan names.
struct FailingBackendMap {
    inner: Box<dyn BackendMapOps + Send>,
    plan: Arc<Mutex<FaultPlan>>,
}

impl BackendMapOps for FailingBackendMap {
    fn get(&self, id: u32) -> Result<BackendEntryPod, MapError> {
        self.inner.get(id)
    }

    fn insert(&mut self, id: u32, entry: BackendEntryPod) -> Result<(), MapError> {
        let fail = {
            let mut plan = self.plan.lock();
            if plan.armed {
                plan.inserts_seen += 1;
                let failed = plan.fail_inserts.contains(&plan.inserts_seen);
                plan.attempts.push(InsertAttempt { id, failed });
                failed
            } else {
                false
            }
        };
        if fail { Err(injected_failure()) } else { self.inner.insert(id, entry) }
    }

    fn remove(&mut self, id: u32) -> Result<(), MapError> {
        self.inner.remove(id)
    }

    fn keys(&self) -> Box<dyn Iterator<Item = Result<u32, MapError>> + '_> {
        self.inner.keys()
    }
}

/// Occupies the dataplane's slot for the instant the real handle is taken out.
struct Vacant;

impl BackendMapOps for Vacant {
    fn get(&self, _id: u32) -> Result<BackendEntryPod, MapError> {
        panic!("the vacant BACKEND_MAP slot is never reached")
    }

    fn insert(&mut self, _id: u32, _entry: BackendEntryPod) -> Result<(), MapError> {
        panic!("the vacant BACKEND_MAP slot is never reached")
    }

    fn remove(&mut self, _id: u32) -> Result<(), MapError> {
        panic!("the vacant BACKEND_MAP slot is never reached")
    }

    fn keys(&self) -> Box<dyn Iterator<Item = Result<u32, MapError>> + '_> {
        panic!("the vacant BACKEND_MAP slot is never reached")
    }
}

/// Replace the dataplane's `BACKEND_MAP` handle with the decorator around it.
/// The decorator starts disarmed and delegates everything.
fn install_faults(dataplane: &mut EbpfDataplane) -> Faults {
    let plan = Arc::new(Mutex::new(FaultPlan::default()));
    let slot = dataplane.backend_map.get_mut();
    let real = std::mem::replace(slot, Box::new(Vacant));
    *slot = Box::new(FailingBackendMap { inner: real, plan: Arc::clone(&plan) });
    Faults(plan)
}

// ---------------------------------------------------------------------
// Test support: the kernel fixture (veth pair, bpffs pin directory).
// ---------------------------------------------------------------------

/// One ordinal per fixture built in the process, so veth names never collide.
static NEXT_ORDINAL: AtomicU64 = AtomicU64::new(0);

/// Run one netlink operation to completion through the crate's sync bridge.
fn netlink<T, F, Fut>(operation: F) -> Result<T, NetlinkError>
where
    F: FnOnce(Client) -> Fut + Send,
    Fut: std::future::Future<Output = Result<T, NetlinkError>>,
    T: Send,
{
    block_on_host_netlink(|| async move { operation(Client::new()?).await })
}

/// An RAII veth pair the loader attaches its XDP programs to.
struct Veth {
    host: String,
    peer: String,
}

impl Veth {
    fn create() -> Self {
        let ordinal = NEXT_ORDINAL.fetch_add(1, Ordering::Relaxed);
        let host = format!("rh{}x{ordinal}", std::process::id() % 100_000);
        let peer = format!("rp{}x{ordinal}", std::process::id() % 100_000);
        netlink(|client| {
            let host = host.clone();
            async move { client.del_link(&host).await }
        })
        .expect("remove a stale veth pair over netlink");
        netlink(|client| {
            let (host, peer) = (host.clone(), peer.clone());
            async move {
                client.add_veth_pair(&host, &peer).await?;
                client.set_link_up(&host).await?;
                client.set_link_up(&peer).await
            }
        })
        .expect("create the veth pair over netlink: needs CAP_NET_ADMIN (run as root)");
        Self { host, peer }
    }
}

impl Drop for Veth {
    fn drop(&mut self) {
        // Deleting one end of a veth pair reaps both; an absent link is a no-op.
        let host = self.host.clone();
        if let Err(error) = netlink(|client| async move { client.del_link(&host).await }) {
            let _reported =
                writeln!(std::io::stderr(), "could not remove veth {}: {error}", self.host);
        }
    }
}

/// The test-owned bpffs pin directory; removed on drop.
struct PinDir(PathBuf);

impl PinDir {
    fn create() -> Self {
        let path =
            PathBuf::from(format!("/sys/fs/bpf/overdrive-test-rollback-{}", std::process::id()));
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                panic!("remove the stale bpffs pin directory {}: {error}", path.display())
            }
        }
        std::fs::create_dir_all(&path).expect("create the bpffs pin directory");
        Self(path)
    }
}

impl Drop for PinDir {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            let _reported = writeln!(
                std::io::stderr(),
                "could not remove the bpffs pin directory {}: {error}",
                self.0.display()
            );
        }
    }
}

// ---------------------------------------------------------------------
// The scenario.
// ---------------------------------------------------------------------

fn frontend() -> ServiceFrontend {
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(198, 18, 251, 1))).expect("IPv4 VIP");
    ServiceFrontend::new(vip, NonZeroU16::new(FRONTEND_PORT).expect("nonzero port"), Proto::Tcp)
        .expect("IPv4 frontend")
}

fn service_key() -> ServiceKey {
    ServiceKey {
        vip_host: u32::from(frontend().vip_v4()),
        port_host: FRONTEND_PORT,
        proto: Proto::Tcp.as_u8(),
        _pad: 0,
    }
}

/// The endpoint of backend `octet`: `127.0.0.<octet>:<9000 + octet>`.
fn endpoint(octet: u8) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, octet), 9000 + u16::from(octet))
}

fn backend(octet: u8, healthy: bool) -> Backend {
    Backend {
        alloc: SpiffeId::new(&format!("spiffe://overdrive.local/workload/rollback/alloc/a{octet}"))
            .expect("allocation SpiffeId"),
        addr: SocketAddr::V4(endpoint(octet)),
        weight: 1,
        healthy,
    }
}

/// The four backends. Ascending `(ip, port)` is the planned order: A, B, C, X.
const A: u8 = 1;
const B: u8 = 2;
const C: u8 = 3;
const X: u8 = 9;

/// Everything the scenario's universe names, read through the existing
/// inspectors and the crate-private trackers.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Observed {
    /// `BACKEND_MAP`, by raw id.
    backend_map: BTreeMap<u32, BackendEntryPod>,
    /// `REVERSE_NAT_MAP` presence of each backend endpoint's key.
    reverse_nat: BTreeMap<SocketAddrV4, bool>,
    /// The inner-map value the `SERVICE_MAP` slot holds; `None` when unprogrammed.
    outer_slot: Option<Vec<u8>>,
    /// The service-backends tracker.
    service_backends: BTreeMap<ServiceKey, BTreeSet<u32>>,
    /// The reverse-NAT key tracker.
    service_reverse_nat_keys: BTreeMap<ServiceKey, BTreeSet<BackendKeyPod>>,
    /// Reverse-NAT keys awaiting a later delete.
    cleanup_debt: BTreeSet<BackendKeyPod>,
    /// `Applied` (true) or `Reserved` (false) identity, with its allocation, by id.
    identities: BTreeMap<u32, (bool, SpiffeId)>,
    /// The allocator's memo for each backend endpoint.
    memo: BTreeMap<SocketAddrV4, Option<u32>>,
}

fn observe(dataplane: &EbpfDataplane) -> Observed {
    let endpoints = [A, B, C, X].map(endpoint);
    let key = service_key();
    let mut key_bytes = Vec::with_capacity(8);
    key_bytes.extend_from_slice(&key.vip_host.to_ne_bytes());
    key_bytes.extend_from_slice(&key.port_host.to_ne_bytes());
    key_bytes.extend_from_slice(&[key.proto, 0]);
    Observed {
        backend_map: dataplane
            .backend_map_entries()
            .expect("read BACKEND_MAP")
            .into_iter()
            .collect(),
        reverse_nat: endpoints
            .iter()
            .map(|e| {
                let present = dataplane
                    .reverse_nat_map_has_backend_proto(*e.ip(), e.port(), Proto::Tcp)
                    .expect("read REVERSE_NAT_MAP");
                (*e, present)
            })
            .collect(),
        outer_slot: crate::sys::bpf::bpf_map_lookup_elem(
            dataplane.service_map.as_fd(),
            &key_bytes,
            std::mem::size_of::<u32>(),
        )
        .expect("read the SERVICE_MAP slot"),
        service_backends: dataplane.service_backends.lock().clone(),
        service_reverse_nat_keys: dataplane.service_reverse_nat_keys.lock().clone(),
        cleanup_debt: dataplane.service_cleanup_debt.lock().clone(),
        identities: dataplane
            .backend_identities
            .lock()
            .iter()
            .map(|(id, state)| {
                (
                    *id,
                    (
                        matches!(state, BackendIdentityState::Applied { .. }),
                        state.spiffe_id().clone(),
                    ),
                )
            })
            .collect(),
        memo: {
            let allocator = dataplane.backend_id_alloc.lock();
            endpoints
                .iter()
                .map(|e| {
                    let id = allocator
                        .existing_id(u32::from(*e.ip()), e.port(), Proto::Tcp.as_u8())
                        .map(BackendId::get);
                    (*e, id)
                })
                .collect()
        },
    }
}

/// The id the allocator holds for backend `octet`.
fn memo_id(observed: &Observed, octet: u8) -> Option<u32> {
    observed.memo[&endpoint(octet)]
}

/// A failed in-place `BACKEND_MAP` restore during rollback is returned as
/// `ServiceApplyRollback { BackendRestore }`.
///
/// Generation 1 programs A (unhealthy) and X. Generation 2 names A (healthy, an
/// in-place update), X (identical), B and C (fresh). Armed `insert` ordinals 3
/// and 4 fail: the insert of C and the restore of A's prior value.
///
/// CONTRACT_SHAPE: bounded-change (declared mutation set = the `BACKEND_MAP`
/// entry of A).
#[test]
fn a_failed_in_place_backend_restore_during_rollback_is_returned_as_service_apply_rollback() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");
    let pins = PinDir::create();
    let veth = Veth::create();
    let mut dataplane = EbpfDataplane::new_with_pin_dir(
        &veth.host,
        &veth.peer,
        &pins.0,
        std::path::Path::new("/sys/fs/cgroup"),
    )
    .expect("EbpfDataplane::new_with_pin_dir with the cgroup attach");
    let faults = install_faults(&mut dataplane);

    // Setup: generation 1 is programmed through a successful update.
    runtime
        .block_on(dataplane.update_service(frontend(), vec![backend(A, false), backend(X, true)]))
        .expect("generation 1 is accepted");
    let before = observe(&dataplane);
    let a_id = memo_id(&before, A).expect("A holds an id after generation 1");
    let x_id = memo_id(&before, X).expect("X holds an id after generation 1");
    assert_eq!(
        before.backend_map.keys().copied().collect::<BTreeSet<_>>(),
        BTreeSet::from([a_id, x_id]),
        "generation 1 programs exactly A and X"
    );
    let a_gen1 = before.backend_map[&a_id];
    assert_eq!(a_gen1.healthy, 0, "A is unhealthy in generation 1");

    // Fault stimulus: armed immediately before the generation-2 update.
    faults.arm(&[3, 4]);
    let generation_2 = vec![backend(A, true), backend(X, true), backend(B, true), backend(C, true)];
    let result = runtime.block_on(dataplane.update_service(frontend(), generation_2.clone()));
    let after = observe(&dataplane);

    // The write order is the planned (ascending endpoint) order: #1 A in place,
    // #2 B insert, #3 C insert (failed), #4 the restore of A (failed).
    let attempts = faults.insert_attempts();
    assert_eq!(attempts.len(), 4, "four BACKEND_MAP inserts after arming: {attempts:?}");
    let (b_id, c_id) = (attempts[1].id, attempts[2].id);
    assert_eq!(
        attempts,
        vec![
            InsertAttempt { id: a_id, failed: false },
            InsertAttempt { id: b_id, failed: false },
            InsertAttempt { id: c_id, failed: true },
            InsertAttempt { id: a_id, failed: true },
        ],
        "the fail-set hits C's insert and A's restore"
    );
    assert!(![a_id, x_id].contains(&b_id) && ![a_id, x_id].contains(&c_id) && b_id != c_id);

    // (1) The typed failure names the restore, and only the restore.
    match result {
        Err(DataplaneError::ServiceApplyRollback { stage, rollback }) => {
            assert_eq!(stage, ServiceApplyStage::BackendMap);
            assert_eq!(
                rollback,
                vec![ServiceApplyRollbackFailure::BackendRestore(
                    BackendId::new(a_id).expect("BackendId")
                )],
                "B's delete succeeded: no BackendDelete and no ReverseNatDelete"
            );
        }
        other => panic!("expected ServiceApplyRollback, got {other:?}"),
    }

    // (2)-(5) Before/after equality over the whole universe, with one declared
    // change: A's BACKEND_MAP entry holds the generation-2 value because its
    // restore failed (the residual the typed failure reports). X is byte-equal,
    // B and C are absent; reverse NAT, the SERVICE_MAP slot (generation 1's inner
    // map), the trackers, the cleanup debt, the identities (B and C hold no
    // identity, A and X keep their Applied ones) and the allocator memos (B and C
    // released) equal generation 1.
    let mut expected = before;
    expected.backend_map.insert(a_id, BackendEntryPod { healthy: 1, ..a_gen1 });
    assert_eq!(after, expected, "only A's BACKEND_MAP entry differs from generation 1");
    assert!(after.outer_slot.is_some(), "the slot is still programmed");
    assert!(after.cleanup_debt.is_empty());
    assert_eq!(memo_id(&after, B), None);
    assert_eq!(memo_id(&after, C), None);

    // (6) Recovery: disarmed, the same update commits; ids are never recycled.
    faults.disarm();
    runtime
        .block_on(dataplane.update_service(frontend(), generation_2))
        .expect("the generation-2 update is accepted once the fault is disarmed");
    let recovered = observe(&dataplane);
    assert_eq!(recovered.backend_map[&a_id].healthy, 1, "A reads healthy = true");
    let (new_b, new_c) = (
        memo_id(&recovered, B).expect("B holds a fresh id"),
        memo_id(&recovered, C).expect("C holds a fresh id"),
    );
    assert!(![b_id, c_id].contains(&new_b) && ![b_id, c_id].contains(&new_c) && new_b != new_c);
    assert_eq!(memo_id(&recovered, A), Some(a_id));
    assert_eq!(memo_id(&recovered, X), Some(x_id));
    assert!(recovered.identities[&new_b].0 && recovered.identities[&new_c].0);
}
