//! GH #295 shared mTLS owner acceptance bodies.

#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::os::fd::AsRawFd as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex as StdMutex};
use std::time::Duration;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::driver::{
    AllocationSpec, DriverPayload, GuestNetworkAssignment, Resources, VmPayload,
};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_netlink::nft::SharedIpInterceptIdentity;
use overdrive_sim::adapters::SimIdentityRead;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_sim::adapters::mtls_intercept::{SimInterceptFault, SimMtlsIntercept};
use overdrive_worker::mtls_intercept::{InterceptError, InterceptLeg, InterceptPostcondition};
use overdrive_worker::mtls_intercept_port::{InterceptGuard, MtlsIntercept};
use overdrive_worker::mtls_intercept_worker::{MtlsInterceptWorker, MtlsSharedOwnerError};
use parking_lot::Mutex;

fn worker(intercept: Arc<dyn MtlsIntercept>) -> Arc<MtlsInterceptWorker> {
    let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
    let enforcement: Arc<dyn MtlsEnforcement> =
        Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));
    let resolve: Arc<dyn MtlsResolve> = Arc::new(overdrive_sim::adapters::SimMtlsResolve::new(
        BTreeMap::new(),
        MtlsResolution::NonMesh,
    ));
    Arc::new(MtlsInterceptWorker::new(enforcement, resolve, Arc::new(SimClock::new()), intercept))
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn shared_owner_starts_once_audits_and_shutdown_drains_the_owner_tree() {
    let worker = worker(Arc::new(SimMtlsIntercept::new()));

    worker.start_shared_owner().await.expect("two listeners, two tasks, and one node guard start");
    worker.audit_shared_owner().await.expect("full listener/task/rule read-back succeeds");
    worker
        .start_shared_owner()
        .await
        .expect("repeated owner start is idempotent and adds no listener or task");
    worker.shutdown_owner().await.expect("owner shutdown drains the complete userspace tree");
    assert!(matches!(
        worker.audit_shared_owner().await,
        Err(MtlsSharedOwnerError::OwnerShutdown | MtlsSharedOwnerError::NotStarted)
    ));
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn initial_leg_f_bind_refusal_returns_to_absent_without_partial_publication() {
    for errno in [libc::EADDRINUSE, libc::EPERM, libc::EMFILE] {
        let intercept = Arc::new(SimMtlsIntercept::new());
        intercept.script_bind_fault(SimInterceptFault::TransparentListener { errno });
        let worker = worker(intercept);

        let error =
            worker.start_shared_owner().await.expect_err("standing bind fault refuses start");
        assert!(matches!(error, MtlsSharedOwnerError::ListenerBind { leg: InterceptLeg::F, .. }));
        assert!(matches!(worker.audit_shared_owner().await, Err(MtlsSharedOwnerError::NotStarted)));
        worker.shutdown_owner().await.expect("Absent owner shutdown is idempotent");
    }
}

struct InertGuard;
impl InterceptGuard for InertGuard {}

struct RecordingSharedIntercept {
    listener_clones: Mutex<Vec<TcpListener>>,
    listener_addresses: Mutex<Vec<SocketAddrV4>>,
    retain_listener_clones: bool,
    bind_calls: AtomicUsize,
    converge_calls: AtomicUsize,
    observe_calls: AtomicUsize,
    fail_bind_at: Option<usize>,
    fail_converge: AtomicBool,
    shared_observation: Mutex<Option<InterceptPostcondition>>,
    shared_guard_drops: Arc<AtomicUsize>,
    occupy_exact_rebind: AtomicBool,
    blockers: Mutex<Vec<TcpListener>>,
}

impl RecordingSharedIntercept {
    fn new() -> Self {
        Self {
            listener_clones: Mutex::new(Vec::new()),
            listener_addresses: Mutex::new(Vec::new()),
            retain_listener_clones: true,
            bind_calls: AtomicUsize::new(0),
            converge_calls: AtomicUsize::new(0),
            observe_calls: AtomicUsize::new(0),
            fail_bind_at: None,
            fail_converge: AtomicBool::new(false),
            shared_observation: Mutex::new(None),
            shared_guard_drops: Arc::new(AtomicUsize::new(0)),
            occupy_exact_rebind: AtomicBool::new(false),
            blockers: Mutex::new(Vec::new()),
        }
    }

    fn failing_bind(call: usize) -> Self {
        Self { retain_listener_clones: false, fail_bind_at: Some(call), ..Self::new() }
    }

    fn failing_converge() -> Self {
        Self { retain_listener_clones: false, fail_converge: AtomicBool::new(true), ..Self::new() }
    }

    fn listener_addresses(&self) -> Vec<SocketAddrV4> {
        self.listener_addresses.lock().clone()
    }

    fn shared_observation(&self) -> Option<InterceptPostcondition> {
        self.shared_observation.lock().clone()
    }

    fn replace_shared_observation(&self, observation: Option<InterceptPostcondition>) {
        *self.shared_observation.lock() = observation;
    }

    fn call_counts(&self) -> (usize, usize, usize) {
        (
            self.bind_calls.load(Ordering::SeqCst),
            self.converge_calls.load(Ordering::SeqCst),
            self.observe_calls.load(Ordering::SeqCst),
        )
    }

    fn shared_guard_drops(&self) -> usize {
        self.shared_guard_drops.load(Ordering::SeqCst)
    }

    fn terminate_listener_task(&self, index: usize) {
        let listener = self.listener_clones.lock().remove(index);
        // SAFETY: `listener` owns a live TCP socket. `shutdown` changes socket
        // state but does not steal fd ownership; dropping closes it once.
        let _ = unsafe { libc::shutdown(listener.as_raw_fd(), libc::SHUT_RDWR) };
        drop(listener);
    }

    fn occupy_next_exact_rebind(&self) {
        self.occupy_exact_rebind.store(true, Ordering::SeqCst);
    }
}

impl MtlsIntercept for RecordingSharedIntercept {
    fn bind_transparent(
        &self,
        addr: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<TcpListener> {
        let call = self.bind_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_bind_at == Some(call) {
            return Err(InterceptError::TransparentListener {
                addr,
                source: std::io::Error::from_raw_os_error(libc::EMFILE),
            });
        }
        if addr.port() != 0 && self.occupy_exact_rebind.swap(false, Ordering::SeqCst) {
            let blocker = TcpListener::bind(addr)
                .map_err(|source| InterceptError::TransparentListener { addr, source })?;
            self.blockers.lock().push(blocker);
        }
        let listener = TcpListener::bind(addr)
            .map_err(|source| InterceptError::TransparentListener { addr, source })?;
        let bound = match listener.local_addr().expect("listener address") {
            std::net::SocketAddr::V4(address) => address,
            std::net::SocketAddr::V6(_) => panic!("fixture binds IPv4"),
        };
        self.listener_addresses.lock().push(bound);
        if self.retain_listener_clones {
            self.listener_clones
                .lock()
                .push(listener.try_clone().expect("clone listener for real socket-state mutation"));
        }
        Ok(listener)
    }

    fn converge_shared(
        &self,
        _prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.converge_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_converge.load(Ordering::SeqCst) {
            return Err(InterceptError::NftRuleInstallFailed {
                op: "replace-shared",
                source: overdrive_worker::mtls_intercept::NetlinkError::nft(
                    "replace-shared",
                    std::io::Error::from_raw_os_error(libc::EBUSY),
                ),
            });
        }
        let (table_and_chains, sets, prerouting, output) =
            SharedIpInterceptIdentity::for_listener_ports(leg_f.port(), leg_c.port())
                .map_err(|source| InterceptError::NftRuleInstallFailed {
                    op: "shared-ip-expected",
                    source,
                })?
                .normalized_parts();
        *self.shared_observation.lock() = Some(InterceptPostcondition::ConstantRules {
            table_and_chains,
            sets,
            prerouting,
            output,
        });
        Ok(Box::new(DropCountGuard(Arc::clone(&self.shared_guard_drops))))
    }

    fn observe_shared(
        &self,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptPostcondition>> {
        self.observe_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.shared_observation.lock().clone())
    }

    fn install_outbound(
        &self,
        _host_veth: &str,
        _leg_f_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        Ok(Box::new(InertGuard))
    }

    fn install_inbound(
        &self,
        _virt: SocketAddrV4,
        _leg_c_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        Ok(Box::new(InertGuard))
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn leg_c_bind_refusal_closes_the_already_bound_leg_f_and_publishes_no_owner() {
    let intercept = Arc::new(RecordingSharedIntercept::failing_bind(2));
    let worker = worker(intercept.clone());
    let error = worker.start_shared_owner().await.expect_err("second bind refuses start");
    assert!(matches!(error, MtlsSharedOwnerError::ListenerBind { leg: InterceptLeg::C, .. }));
    let addresses = intercept.listener_addresses();
    assert_eq!(addresses.len(), 1, "only leg F was acquired before refusal");
    let rebound = TcpListener::bind(addresses[0]).expect("failed start closes the partial leg F");
    drop(rebound);
    assert!(matches!(worker.audit_shared_owner().await, Err(MtlsSharedOwnerError::NotStarted)));
    worker.shutdown_owner().await.expect("Absent owner shutdown is idempotent");
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn shared_rule_convergence_refusal_closes_both_sockets_and_publishes_no_tasks_or_guard() {
    let intercept = Arc::new(RecordingSharedIntercept::failing_converge());
    let worker = worker(intercept.clone());
    let error = worker.start_shared_owner().await.expect_err("shared rule convergence refuses");
    assert!(matches!(error, MtlsSharedOwnerError::Intercept { .. }));
    let addresses = intercept.listener_addresses();
    assert_eq!(addresses.len(), 2);
    for address in addresses {
        let rebound = TcpListener::bind(address).expect("failed start closes every partial socket");
        drop(rebound);
    }
    assert!(matches!(worker.audit_shared_owner().await, Err(MtlsSharedOwnerError::NotStarted)));
    worker.shutdown_owner().await.expect("Absent owner shutdown is idempotent");
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER step for GH #295 exact-port shared-listener recovery"]
async fn lost_leg_f_rebinds_the_recorded_nonzero_address_before_audit_succeeds() {
    let intercept = Arc::new(RecordingSharedIntercept::new());
    let worker = worker(intercept.clone());
    worker.start_shared_owner().await.expect("start the two-listener owner");
    let original = intercept.listener_addresses();
    assert_eq!(original.len(), 2);
    assert_ne!(original[0].port(), 0);
    intercept.terminate_listener_task(0);
    let failure = tokio::time::timeout(Duration::from_secs(2), worker.wait_shared_owner_failure())
        .await
        .expect("real listener task exit is observed");
    assert!(matches!(
        failure,
        MtlsSharedOwnerError::TaskReturned { leg: InterceptLeg::F }
            | MtlsSharedOwnerError::TaskFailed { leg: InterceptLeg::F, .. }
            | MtlsSharedOwnerError::TaskCancelled { leg: InterceptLeg::F }
    ));

    worker.converge_shared_owner().await.expect("exact-port recovery converges");
    worker.audit_shared_owner().await.expect("full post-recovery audit succeeds");
    let rebound = intercept.listener_addresses();
    assert_eq!(rebound.last(), Some(&original[0]), "recovery never selects another port");
    worker.shutdown_owner().await.expect("shared owner drains");
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER step for GH #295 occupied exact-port fail-stop path"]
async fn occupied_original_leg_f_address_refuses_recovery_without_selecting_another_port() {
    let intercept = Arc::new(RecordingSharedIntercept::new());
    let worker = worker(intercept.clone());
    worker.start_shared_owner().await.expect("start the two-listener owner");
    let original = intercept.listener_addresses();
    intercept.terminate_listener_task(0);
    let _ = tokio::time::timeout(Duration::from_secs(2), worker.wait_shared_owner_failure())
        .await
        .expect("listener failure is observed");
    intercept.occupy_next_exact_rebind();

    let error = worker
        .converge_shared_owner()
        .await
        .expect_err("EADDRINUSE on the recorded address refuses recovery");
    assert!(matches!(
        error,
        MtlsSharedOwnerError::ListenerBind {
            leg: InterceptLeg::F,
            requested,
            source: InterceptError::TransparentListener { source, .. },
        } if requested == original[0] && source.raw_os_error() == Some(libc::EADDRINUSE)
    ));
    assert_eq!(
        intercept.listener_addresses(),
        original,
        "failure does not bind a replacement address"
    );
    worker.shutdown_owner().await.expect("failed recovery still drains the owner tree");
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(
    clippy::too_many_lines,
    reason = "one published-owner narrative keeps exact targets, observe-only conflict, retained guard, and terminal drain together"
)]
async fn published_wrong_shared_target_is_observe_only_until_bounded_fail_stop() {
    let intercept = Arc::new(RecordingSharedIntercept::new());
    let worker = worker(intercept.clone());
    worker
        .start_shared_owner()
        .await
        .expect("publish two exact listeners, two tasks, and one node guard");
    worker.audit_shared_owner().await.expect("published owner begins healthy");

    let listener_addresses = intercept.listener_addresses();
    assert_eq!(listener_addresses.len(), 2);
    assert!(listener_addresses.iter().all(|address| address.port() != 0));
    let healthy_identity =
        intercept.shared_observation().expect("published owner retains one exact shared identity");
    let (expected_tables, expected_sets, expected_prerouting, expected_output) =
        SharedIpInterceptIdentity::for_listener_ports(
            listener_addresses[0].port(),
            listener_addresses[1].port(),
        )
        .expect("recorded non-zero listener addresses form one canonical identity")
        .normalized_parts();
    assert_eq!(
        healthy_identity,
        InterceptPostcondition::ConstantRules {
            table_and_chains: expected_tables,
            sets: expected_sets,
            prerouting: expected_prerouting,
            output: expected_output,
        },
        "published semantic identity names the two concrete listener targets"
    );
    let wrong_leg_f = SocketAddrV4::new(
        *listener_addresses[0].ip(),
        listener_addresses[0].port().checked_add(1).unwrap_or(1),
    );
    assert_ne!(wrong_leg_f, listener_addresses[0]);
    let (wrong_tables, wrong_sets, wrong_prerouting, wrong_output) =
        SharedIpInterceptIdentity::for_listener_ports(
            wrong_leg_f.port(),
            listener_addresses[1].port(),
        )
        .expect("different non-zero leg-F target remains canonical")
        .normalized_parts();
    let wrong_identity = InterceptPostcondition::ConstantRules {
        table_and_chains: wrong_tables,
        sets: wrong_sets,
        prerouting: wrong_prerouting,
        output: wrong_output,
    };
    intercept.replace_shared_observation(Some(wrong_identity.clone()));

    let calls_before_detection = intercept.call_counts();
    let detection = worker
        .audit_shared_owner()
        .await
        .expect_err("wrong published target is an immediate structured conflict");
    assert!(matches!(
        detection,
        MtlsSharedOwnerError::Intercept {
            source: InterceptError::PostconditionMismatch {
                expected,
                observed: Some(observed),
            },
        } if expected == healthy_identity && observed == wrong_identity
    ));

    let convergence_error = worker
        .converge_shared_owner()
        .await
        .expect_err("the production worker returns a present wrong-target conflict to its owner");
    assert!(matches!(
        convergence_error,
        MtlsSharedOwnerError::Intercept {
            source: InterceptError::PostconditionMismatch {
                expected: ref actual_expected,
                observed: Some(ref actual_observed),
            },
        } if actual_expected == &healthy_identity && actual_observed == &wrong_identity
    ));
    assert_eq!(
        intercept.listener_addresses(),
        listener_addresses,
        "the worker never binds port zero or substitutes an address"
    );
    assert_eq!(
        intercept.shared_observation().as_ref(),
        Some(&wrong_identity),
        "the worker never rewrites the published target"
    );
    assert_eq!(intercept.shared_guard_drops(), 0, "published guard stays retained");

    let calls_after_trigger = intercept.call_counts();
    assert_eq!(calls_after_trigger.0, calls_before_detection.0, "no runtime bind call");
    assert_eq!(
        calls_after_trigger.1, calls_before_detection.1,
        "runtime never calls the fresh-process converge_shared branch"
    );
    assert_eq!(
        calls_after_trigger.2,
        calls_before_detection.2 + 2,
        "detection plus the production worker convergence trigger are observe-only"
    );

    worker.shutdown_owner().await.expect("published owner drains after the bounded conflict");
    assert_eq!(
        intercept.shared_guard_drops(),
        0,
        "the private published-owner path relinquishes rather than drops the node guard"
    );
}

struct DropCountGuard(Arc<AtomicUsize>);
impl InterceptGuard for DropCountGuard {}
impl Drop for DropCountGuard {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct ActivationBarrierIntercept {
    shared: RecordingSharedIntercept,
    entered: AtomicBool,
    release: (StdMutex<bool>, Condvar),
    block_once: AtomicBool,
    guard_drops: Arc<AtomicUsize>,
}

impl ActivationBarrierIntercept {
    fn new() -> Self {
        Self {
            shared: RecordingSharedIntercept::new(),
            entered: AtomicBool::new(false),
            release: (StdMutex::new(false), Condvar::new()),
            block_once: AtomicBool::new(true),
            guard_drops: Arc::new(AtomicUsize::new(0)),
        }
    }

    async fn wait_entered(&self) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !self.entered.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("allocation registration reaches the pre-activation barrier");
    }

    fn release(&self) {
        let (lock, wake) = &self.release;
        *lock.lock().expect("release lock") = true;
        wake.notify_all();
    }
}

impl MtlsIntercept for ActivationBarrierIntercept {
    fn bind_transparent(
        &self,
        addr: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<TcpListener> {
        self.shared.bind_transparent(addr)
    }

    fn converge_shared(
        &self,
        prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.shared.converge_shared(prior, leg_f, leg_c)
    }

    fn observe_shared(
        &self,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptPostcondition>> {
        self.shared.observe_shared()
    }

    fn install_outbound(
        &self,
        _host_veth: &str,
        _leg_f_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        Ok(Box::new(DropCountGuard(Arc::clone(&self.guard_drops))))
    }

    fn install_inbound(
        &self,
        _virt: SocketAddrV4,
        _leg_c_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        if self.block_once.swap(false, Ordering::SeqCst) {
            self.entered.store(true, Ordering::SeqCst);
            let (lock, wake) = &self.release;
            let mut released = lock.lock().expect("release lock");
            while !*released {
                released = wake.wait(released).expect("release wait");
            }
            drop(released);
        }
        Ok(Box::new(DropCountGuard(Arc::clone(&self.guard_drops))))
    }
}

fn allocation_spec(name: &str) -> AllocationSpec {
    AllocationSpec {
        alloc: AllocationId::new(name).expect("allocation id"),
        identity: SpiffeId::new(&format!(
            "spiffe://overdrive.local/workload/shared-owner/alloc/{name}"
        ))
        .expect("SPIFFE ID"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: "/kernel".into(),
            rootfs: "/rootfs".into(),
        }),
        resources: Resources { cpu_milli: 1, memory_bytes: 1 },
        probe_descriptors: Vec::new(),
        network: Some(GuestNetworkAssignment {
            address: Ipv4Addr::new(100, 95, 0, 2),
            tap: "ovd-tp-0002".to_owned(),
            mac: [0x02, 0x00, 100, 95, 0, 2],
            gateway: Ipv4Addr::new(100, 95, 0, 1),
            prefix: 16,
            dns: Ipv4Addr::new(100, 95, 0, 1),
        }),
        service_ports: [8080_u16, 8443]
            .into_iter()
            .map(|port| std::num::NonZeroU16::new(port).expect("non-zero port"))
            .collect(),
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_and_owner_shutdown_during_pending_registration_return_registration_retired_and_drain_once()
 {
    for owner_shutdown in [false, true] {
        let intercept = Arc::new(ActivationBarrierIntercept::new());
        let worker = worker(intercept.clone());
        worker.start_shared_owner().await.expect("shared owner is healthy");
        let spec = allocation_spec(if owner_shutdown {
            "pending-owner-shutdown"
        } else {
            "pending-allocation-stop"
        });
        let alloc = spec.alloc.clone();
        let start = tokio::spawn({
            let worker = Arc::clone(&worker);
            async move { worker.start_alloc(&spec).await }
        });
        intercept.wait_entered().await;
        let retirement = tokio::spawn({
            let worker = Arc::clone(&worker);
            let alloc = alloc.clone();
            async move {
                if owner_shutdown {
                    worker.shutdown_owner().await.map_err(|error| error.to_string())
                } else {
                    worker.stop_alloc(&alloc).await.map_err(|error| error.to_string())
                }
            }
        });
        tokio::task::yield_now().await;
        assert!(!retirement.is_finished(), "retirement waits for the Pending owner handoff");
        intercept.release();
        let start_error =
            start.await.expect("start task joins").expect_err("retirement wins before activation");
        assert!(matches!(
            start_error,
            overdrive_worker::mtls_intercept_worker::MtlsInterceptInstallError::RegistrationRetired {
                alloc_id,
            } if alloc_id == alloc
        ));
        retirement
            .await
            .expect("retirement task joins")
            .expect("one retirement owner drains every transferred effect");
        assert_eq!(
            intercept.guard_drops.load(Ordering::SeqCst),
            3,
            "one outbound and two distinct inbound elements drop exactly once"
        );
    }
}
