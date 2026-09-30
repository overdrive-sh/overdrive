//! S-ND295-13A seeded safety/convergence evidence for the D-295-R12 boot
//! order (FD § "[REF] Boot ordering (D-295-R12) — ACCEPTED 2026-09-24" (the boot sequence)).
//!
//! The test drives the real injected-driver server helper
//! (`run_server_with_obs_and_driver`, D-295-DISTILL-13's boundary). The same
//! Arc-backed [`SimVmHostState`](crate::adapters::SimVmHostState) is passed to
//! production VM reclamation and to the Sim shared owner, which snapshots it
//! inside the actual `sweep_stale` port call, so a sweep that ran before
//! reclamation would see the seeded VM-exclusive residue. The required `ServerConfig.mtls_intercept` port is a
//! test-local recording intercept over an inner `SimMtlsIntercept` that starts
//! with a prior-process program and seeded non-empty members. Every intercept
//! call records the Sim owner's `calls().len()` and the EXEC supervisor's
//! `is_boot_closed()` as it began, which orders it against the owner's sweep
//! and against `open_after_boot`.
//!
//! D-295-R16 composes the worker, and with it the real `HostMtlsEnforcement`
//! kTLS probe, on every `run_server*` boot (N-2, FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (`compose_mtls` is deleted)), so the module is
//! gated behind `integration-tests` and runs as root under Lima.

#![allow(clippy::doc_markdown, clippy::expect_used, clippy::print_stderr)]

#[cfg(all(test, feature = "integration-tests"))]
mod tests {
    use std::collections::BTreeSet;
    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::ops::Range;
    use std::path::PathBuf;
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;

    use overdrive_control_plane::dataplane_config::DataplaneConfig;
    use overdrive_control_plane::guest_network::{GuestNetworkOperation, SharedGuestNetworkOwner};
    use overdrive_control_plane::{ServerConfig, run_server_with_obs_and_driver};
    use overdrive_core::guest_network::{GuestNetworkExecSupervisor, GuestNetworkExecWiring};
    use overdrive_core::id::{AllocationId, NodeId};
    use overdrive_core::traits::driver::{Driver, DriverType};
    use overdrive_core::traits::observation_store::ObservationStore;
    use overdrive_core::traits::vm_host_state::VmHostState;
    use overdrive_worker::cgroup_manager::CgroupManager;
    use overdrive_worker::mtls_intercept::{InterceptPostcondition, Result as InterceptResult};
    use overdrive_worker::mtls_intercept_port::{
        InterceptGuard, InterceptMembers, InterceptState, MtlsIntercept,
    };
    use parking_lot::Mutex;
    use proptest::prelude::*;

    use crate::adapters::dataplane::SimDataplane;
    use crate::adapters::driver::SimDriver;
    use crate::adapters::guest_network::SimSharedGuestNetworkOwner;
    use crate::adapters::observation_store::SimObservationStore;
    use crate::adapters::vm_host_state::SimVmHostState;
    use crate::adapters::{SimCgroupFs, SimGuestDnsFactory, SimKek, SimMtlsIntercept};

    /// The port's listener type. The DELIVER step that carries B-7 (05-01 at
    /// the latest, FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the pinned `bind_transparent` signature)) changes this one line to
    /// `Arc<dyn InterceptListener>`; nothing here reads the listener.
    type BoundListener = std::net::TcpListener;

    /// One intercept port call, with its argument or outcome.
    #[derive(Debug, Clone)]
    enum InterceptCall {
        BindTransparent {
            requested: SocketAddrV4,
        },
        ConvergeShared {
            prior: Option<InterceptPostcondition>,
            leg_f: SocketAddrV4,
            leg_c: SocketAddrV4,
        },
        ObserveShared {
            observed: Result<Option<InterceptPostcondition>, String>,
        },
        InstallOutbound {
            source: Ipv4Addr,
        },
        InstallInbound {
            virt: SocketAddrV4,
        },
        ObserveSharedState {
            observed: Result<Option<InterceptState>, String>,
        },
        ConvergeAllocationElements {
            expected: InterceptMembers,
        },
        RemoveAllocationElements {
            source: Ipv4Addr,
            destinations: Vec<SocketAddrV4>,
        },
    }

    impl std::fmt::Display for InterceptCall {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::BindTransparent { requested } => write!(f, "bind_transparent({requested})"),
                Self::ConvergeShared { prior, leg_f, leg_c } => {
                    write!(f, "converge_shared(prior={prior:?}, F={leg_f}, C={leg_c})")
                }
                Self::ObserveShared { observed } => write!(f, "observe_shared -> {observed:?}"),
                Self::InstallOutbound { source } => write!(f, "install_outbound({source})"),
                Self::InstallInbound { virt } => write!(f, "install_inbound({virt})"),
                Self::ObserveSharedState { observed } => {
                    write!(f, "observe_shared_state -> {observed:?}")
                }
                Self::ConvergeAllocationElements { expected } => {
                    write!(f, "converge_allocation_elements({expected:?})")
                }
                Self::RemoveAllocationElements { source, destinations } => {
                    write!(f, "remove_allocation_elements({source}, {destinations:?})")
                }
            }
        }
    }

    /// One recorded call and the two ordering observations taken as it began.
    #[derive(Debug, Clone)]
    struct RecordedCall {
        call: InterceptCall,
        /// The Sim owner's `calls().len()`: every owner operation with a
        /// smaller index had already been recorded.
        owner_calls: usize,
        /// `supervisor.is_boot_closed()`; `None` if the supervisor was not yet
        /// bound (a harness defect).
        boot_closed: Option<bool>,
    }

    /// Test-local recording `MtlsIntercept` over an inner `SimMtlsIntercept`.
    struct BootOrderIntercept {
        inner: SimMtlsIntercept,
        owner: Arc<SimSharedGuestNetworkOwner>,
        supervisor: OnceLock<Arc<GuestNetworkExecSupervisor>>,
        calls: Mutex<Vec<RecordedCall>>,
        /// The seeded members, read back before boot.
        seeded: InterceptMembers,
        /// Held only so the seeded prior-process elements persist through boot
        /// the way a killed process's kernel elements do; the sim removes an
        /// element when its guard drops.
        _residue: Vec<Box<dyn InterceptGuard>>,
    }

    impl BootOrderIntercept {
        /// Seed the inner sim with a prior process's program and non-empty
        /// members, before any production call is recorded.
        fn seeded(seed: u64, owner: Arc<SimSharedGuestNetworkOwner>) -> Self {
            let inner = SimMtlsIntercept::new();
            let bytes = seed.to_be_bytes();
            let prior_f = SocketAddrV4::new(
                Ipv4Addr::LOCALHOST,
                40_000 + u16::from_be_bytes([bytes[0], bytes[1]]) % 5_000,
            );
            let prior_c = SocketAddrV4::new(Ipv4Addr::LOCALHOST, prior_f.port() + 1);
            let guest = Ipv4Addr::new(100, 95, bytes[2], bytes[3].max(2));
            let virt = SocketAddrV4::new(
                Ipv4Addr::new(10, 98, bytes[4], bytes[5]),
                1_024 + u16::from(bytes[6]),
            );
            let node_guard = inner
                .converge_shared(None, prior_f, prior_c)
                .expect("seed the prior process's shared program");
            let outbound = inner
                .install_outbound(guest, prior_f.port())
                .expect("seed a stale outbound member");
            let inbound =
                inner.install_inbound(virt, prior_c.port()).expect("seed a stale inbound member");
            // Dropped in vector order: the element guards before the node guard
            // (DISTILL gap B-8's guard-ordering rule).
            let residue = vec![outbound, inbound, node_guard];
            let seeded = inner
                .observe_shared_state()
                .expect("read back the seeded program")
                .expect("the seeded program is present")
                .members;
            assert!(
                !seeded.managed_guest_ips.is_empty()
                    && !seeded.outbound_sources.is_empty()
                    && !seeded.inbound_destinations.is_empty(),
                "seed={seed}: harness precondition failed: seeded members are empty: {seeded:?}"
            );
            Self {
                inner,
                owner,
                supervisor: OnceLock::new(),
                calls: Mutex::new(Vec::new()),
                seeded,
                _residue: residue,
            }
        }

        fn bind_supervisor(&self, supervisor: Arc<GuestNetworkExecSupervisor>) {
            assert!(
                self.supervisor.set(supervisor).is_ok(),
                "harness precondition failed: the recording intercept's supervisor was bound twice"
            );
        }

        fn record(&self, call: InterceptCall, owner_calls: usize, boot_closed: Option<bool>) {
            self.calls.lock().push(RecordedCall { call, owner_calls, boot_closed });
        }

        /// The two ordering observations, taken as a call begins.
        fn snapshot(&self) -> (usize, Option<bool>) {
            let owner_calls = self.owner.calls().len();
            let boot_closed = self.supervisor.get().map(|supervisor| supervisor.is_boot_closed());
            (owner_calls, boot_closed)
        }

        fn calls(&self) -> Vec<RecordedCall> {
            self.calls.lock().clone()
        }
    }

    impl MtlsIntercept for BootOrderIntercept {
        fn bind_transparent(&self, addr: SocketAddrV4) -> InterceptResult<BoundListener> {
            let (owner_calls, boot_closed) = self.snapshot();
            self.record(
                InterceptCall::BindTransparent { requested: addr },
                owner_calls,
                boot_closed,
            );
            self.inner.bind_transparent(addr)
        }

        fn converge_shared(
            &self,
            prior: Option<&InterceptPostcondition>,
            leg_f: SocketAddrV4,
            leg_c: SocketAddrV4,
        ) -> InterceptResult<Box<dyn InterceptGuard>> {
            let (owner_calls, boot_closed) = self.snapshot();
            self.record(
                InterceptCall::ConvergeShared { prior: prior.cloned(), leg_f, leg_c },
                owner_calls,
                boot_closed,
            );
            self.inner.converge_shared(prior, leg_f, leg_c)
        }

        fn observe_shared(&self) -> InterceptResult<Option<InterceptPostcondition>> {
            let (owner_calls, boot_closed) = self.snapshot();
            let outcome = self.inner.observe_shared();
            let observed = outcome.as_ref().cloned().map_err(ToString::to_string);
            self.record(InterceptCall::ObserveShared { observed }, owner_calls, boot_closed);
            outcome
        }

        fn install_outbound(
            &self,
            source_addr: Ipv4Addr,
            agent_leg_f_port: u16,
        ) -> InterceptResult<Box<dyn InterceptGuard>> {
            let (owner_calls, boot_closed) = self.snapshot();
            self.record(
                InterceptCall::InstallOutbound { source: source_addr },
                owner_calls,
                boot_closed,
            );
            self.inner.install_outbound(source_addr, agent_leg_f_port)
        }

        fn install_inbound(
            &self,
            virt: SocketAddrV4,
            agent_leg_c_port: u16,
        ) -> InterceptResult<Box<dyn InterceptGuard>> {
            let (owner_calls, boot_closed) = self.snapshot();
            self.record(InterceptCall::InstallInbound { virt }, owner_calls, boot_closed);
            self.inner.install_inbound(virt, agent_leg_c_port)
        }

        fn observe_shared_state(&self) -> InterceptResult<Option<InterceptState>> {
            let (owner_calls, boot_closed) = self.snapshot();
            let outcome = self.inner.observe_shared_state();
            let observed = outcome.as_ref().cloned().map_err(ToString::to_string);
            self.record(InterceptCall::ObserveSharedState { observed }, owner_calls, boot_closed);
            outcome
        }

        fn converge_allocation_elements(
            &self,
            expected: &InterceptMembers,
        ) -> InterceptResult<Option<InterceptState>> {
            let (owner_calls, boot_closed) = self.snapshot();
            self.record(
                InterceptCall::ConvergeAllocationElements { expected: expected.clone() },
                owner_calls,
                boot_closed,
            );
            self.inner.converge_allocation_elements(expected)
        }

        fn remove_allocation_elements(
            &self,
            source_addr: Ipv4Addr,
            destinations: &[SocketAddrV4],
        ) -> InterceptResult<InterceptState> {
            let (owner_calls, boot_closed) = self.snapshot();
            self.record(
                InterceptCall::RemoveAllocationElements {
                    source: source_addr,
                    destinations: destinations.to_vec(),
                },
                owner_calls,
                boot_closed,
            );
            self.inner.remove_allocation_elements(source_addr, destinations)
        }
    }

    fn render(calls: &[RecordedCall]) -> String {
        calls
            .iter()
            .enumerate()
            .map(|(index, recorded)| {
                format!(
                    "  #{index} owner_calls={} boot_closed={:?} {}",
                    recorded.owner_calls, recorded.boot_closed, recorded.call
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn chain_failure(seed: u64, calls: &[RecordedCall], what: &str) -> ! {
        panic!("seed={seed}: {what}\nrecorded intercept calls:\n{}", render(calls))
    }

    /// The first recorded call at or after `from` that `wanted` accepts.
    fn next_call(
        calls: &[RecordedCall],
        from: usize,
        wanted: impl Fn(&InterceptCall) -> bool,
    ) -> Option<(usize, &RecordedCall)> {
        calls.iter().enumerate().skip(from).find(|(_, recorded)| wanted(&recorded.call))
    }

    /// Fails the chain when a call `forbidden` accepts was recorded at an index
    /// in `before`, whose end is the index of the chain step `step` that must
    /// precede every such call.
    fn forbid_before(
        seed: u64,
        calls: &[RecordedCall],
        before: Range<usize>,
        step: &str,
        forbidden: impl Fn(&InterceptCall) -> bool,
    ) {
        let end = before.end;
        if let Some((early, recorded)) = calls
            .iter()
            .enumerate()
            .take(before.end)
            .skip(before.start)
            .find(|(_, recorded)| forbidden(&recorded.call))
        {
            chain_failure(
                seed,
                calls,
                &format!("{} (#{early}) ran before {step} (#{end})", recorded.call),
            );
        }
    }

    /// The R12 intercept chain: sweep ≺ the shared owner's `converge_shared`
    /// (step 5, the bridge) ≺ `converge_allocation_elements(∅)` ≺
    /// `observe_shared` ≺ `converge_shared(prior, F, C)` ≺
    /// `observe_shared_state` (zero members), every call BootClosed. The order
    /// is relative, not merely "some later call exists": no `observe_shared`
    /// or `converge_shared` call precedes the stale-member clear, and no
    /// `converge_shared` call precedes the `observe_shared` that captured its
    /// prior identity (FD § "[REF] Boot ordering (D-295-R12)" steps 6.2 to
    /// 6.5). Returns the index of the read-back that ends the chain.
    fn assert_boot_chain(
        seed: u64,
        calls: &[RecordedCall],
        owner_ops: &[GuestNetworkOperation],
        sweep_index: usize,
        seeded: &InterceptMembers,
    ) -> usize {
        let clear = assert_stale_member_clear(seed, calls, owner_ops, sweep_index, seeded);

        let Some((observe, observe_call)) =
            next_call(calls, clear + 1, |call| matches!(call, InterceptCall::ObserveShared { .. }))
        else {
            chain_failure(
                seed,
                calls,
                &format!("no observe_shared call after the stale-member clear (#{clear})"),
            );
        };
        forbid_before(
            seed,
            calls,
            clear + 1..observe,
            "observe_shared, which captures the prior identity converge_shared must be given",
            |call| matches!(call, InterceptCall::ConvergeShared { .. }),
        );
        let InterceptCall::ObserveShared { observed: Ok(prior) } = &observe_call.call else {
            chain_failure(seed, calls, &format!("observe_shared (#{observe}) failed"));
        };
        assert_converge_and_read_back(seed, calls, observe, prior.as_ref())
    }

    /// Chain step 6.2: the first `converge_allocation_elements(∅)` began after
    /// the owner's sweep and after the owner's `converge_shared` (step 5,
    /// `BridgeConverge`) that follows the sweep, and no `observe_shared` or
    /// `converge_shared` call precedes it (FD § "[REF] Boot ordering
    /// (D-295-R12)" steps 4, 5, 6.2). Returns its index.
    fn assert_stale_member_clear(
        seed: u64,
        calls: &[RecordedCall],
        owner_ops: &[GuestNetworkOperation],
        sweep_index: usize,
        seeded: &InterceptMembers,
    ) -> usize {
        let Some((clear, clear_call)) = next_call(calls, 0, |call| {
            matches!(
                call,
                InterceptCall::ConvergeAllocationElements { expected }
                    if *expected == InterceptMembers::default()
            )
        }) else {
            chain_failure(
                seed,
                calls,
                &format!(
                    "boot never cleared the stale intercept members {seeded:?}: no \
                     converge_allocation_elements(∅) call"
                ),
            );
        };
        if clear_call.owner_calls <= sweep_index {
            chain_failure(
                seed,
                calls,
                &format!(
                    "the stale-member clear (#{clear}) began with {} owner calls recorded, \
                     before the sweep (owner call {sweep_index})",
                    clear_call.owner_calls
                ),
            );
        }
        let bridge_converge = owner_ops
            .iter()
            .enumerate()
            .skip(sweep_index + 1)
            .find(|(_, operation)| **operation == GuestNetworkOperation::BridgeConverge)
            .map(|(index, _)| index);
        if bridge_converge.is_none_or(|index| index >= clear_call.owner_calls) {
            chain_failure(
                seed,
                calls,
                &format!(
                    "the shared owner's converge_shared (R12 step 5) must run after the sweep \
                     (owner call {sweep_index}) and before the stale-member clear (#{clear}, \
                     which began with {} owner calls recorded); first BridgeConverge after the \
                     sweep: owner call {bridge_converge:?}; owner calls: {owner_ops:?}",
                    clear_call.owner_calls
                ),
            );
        }
        forbid_before(
            seed,
            calls,
            0..clear,
            "the stale-member clear, which must precede every read or convergence of the program",
            |call| {
                matches!(
                    call,
                    InterceptCall::ObserveShared { .. } | InterceptCall::ConvergeShared { .. }
                )
            },
        );
        clear
    }

    /// Chain steps 6.5 and 6.6, after the `observe_shared` at `observe` that
    /// captured `prior`: `converge_shared` is given that identity, the
    /// read-back observes the program with zero members, and every call up to
    /// the read-back ran BootClosed. Returns the read-back's index.
    fn assert_converge_and_read_back(
        seed: u64,
        calls: &[RecordedCall],
        observe: usize,
        prior: Option<&InterceptPostcondition>,
    ) -> usize {
        let Some((converge, converge_call)) = next_call(calls, observe + 1, |call| {
            matches!(call, InterceptCall::ConvergeShared { .. })
        }) else {
            chain_failure(
                seed,
                calls,
                &format!("no converge_shared call after observe_shared (#{observe})"),
            );
        };
        let InterceptCall::ConvergeShared { prior: passed, .. } = &converge_call.call else {
            unreachable!("the search matched a ConvergeShared call");
        };
        if passed.as_ref() != prior {
            chain_failure(
                seed,
                calls,
                &format!(
                    "converge_shared (#{converge}) was given prior {passed:?}, not the identity \
                     observe_shared (#{observe}) captured: {prior:?}"
                ),
            );
        }

        let Some((read_back, read_back_call)) = next_call(calls, converge + 1, |call| {
            matches!(call, InterceptCall::ObserveSharedState { .. })
        }) else {
            chain_failure(
                seed,
                calls,
                &format!("no observe_shared_state read-back after converge_shared (#{converge})"),
            );
        };
        match &read_back_call.call {
            InterceptCall::ObserveSharedState { observed: Ok(Some(state)) }
                if state.members == InterceptMembers::default() => {}
            other => chain_failure(
                seed,
                calls,
                &format!(
                    "the read-back (#{read_back}) did not observe the program with zero \
                     members: {other}"
                ),
            ),
        }

        if let Some((index, recorded)) = calls
            .iter()
            .enumerate()
            .take(read_back + 1)
            .find(|(_, recorded)| recorded.boot_closed != Some(true))
        {
            chain_failure(
                seed,
                calls,
                &format!(
                    "intercept call #{index} of the boot chain ran with boot_closed={:?}; \
                     admission must stay BootClosed through the read-back (#{read_back})",
                    recorded.boot_closed
                ),
            );
        }
        read_back
    }

    async fn run_seed(seed: u64) {
        eprintln!(
            "seed={seed} invariant=reclamation_completes_before_stale_shared_network_sweep_for_every_seeded_prior_vm"
        );
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let data_dir = tmp.path().join("data");
        let operator_config_dir = tmp.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("data dir");
        std::fs::create_dir_all(&operator_config_dir).expect("config dir");

        let host = SimVmHostState::new();
        let allocation_count = usize::try_from(seed % 4 + 1).expect("1..=4 fits usize");
        let mut allocations = Vec::with_capacity(allocation_count);
        for index in 0..allocation_count {
            let alloc = AllocationId::new(&format!("nd295-s13-{seed:016x}-{index}"))
                .expect("seeded allocation id");
            if (seed.rotate_right(u32::try_from(index).unwrap_or(0)) & 1) == 1 {
                host.set_scope(alloc.clone(), BTreeSet::new());
            }
            host.set_run_dir(alloc.clone());
            host.set_clone(
                alloc.clone(),
                PathBuf::from(format!("/var/lib/overdrive/vm-clones/{alloc}.img")),
            );
            allocations.push(alloc);
        }

        let owner = Arc::new(SimSharedGuestNetworkOwner::with_sweep_host_state(host.clone()));
        let owner_port: Arc<dyn SharedGuestNetworkOwner> = owner.clone();
        let intercept = Arc::new(BootOrderIntercept::seeded(seed, Arc::clone(&owner)));
        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().expect("loopback bind"),
            data_dir,
            operator_config_dir,
            dataplane: Some(DataplaneConfig {
                client_iface: "lo".to_owned(),
                backend_iface: "lo".to_owned(),
            }),
            dataplane_override: Some(Arc::new(SimDataplane::new())),
            ..ServerConfig::new(
                Arc::new(SimKek::for_boot()),
                Arc::clone(&intercept) as Arc<dyn MtlsIntercept>,
                Arc::new(SimGuestDnsFactory::default()),
            )
        };
        let host_port: Arc<dyn VmHostState> = Arc::new(host);
        let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
            NodeId::new("nd295-s13-node").expect("node id"),
            0,
        ));
        let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
        let wiring = GuestNetworkExecWiring::new(Arc::clone(&config.clock));
        // Kept before the wiring moves into the boot call (TS § In-process
        // observation).
        let supervisor = wiring.supervisor();
        intercept.bind_supervisor(Arc::clone(&supervisor));

        let handle = run_server_with_obs_and_driver(
            config,
            obs,
            driver,
            host_port,
            owner_port,
            wiring,
            CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), Arc::new(SimCgroupFs::new())),
        )
        .await
        .unwrap_or_else(|error| {
            panic!("seed={seed}: production boot failed before oracle: {error}")
        });
        let recorded = intercept.calls();
        let opened = !supervisor.is_boot_closed();

        let calls = owner.calls();
        let sweep_calls = owner.sweep_calls();
        assert_eq!(sweep_calls.len(), 1, "seed={seed}: exactly one actual sweep port call");
        let sweep = &sweep_calls[0];
        assert_eq!(
            calls.get(sweep.call_index),
            Some(&GuestNetworkOperation::CleanupComplement),
            "seed={seed}: sweep snapshot must name its CleanupComplement call"
        );
        for alloc in allocations {
            assert!(
                !sweep.host.scopes.contains_key(&alloc),
                "seed={seed}: {alloc} scope remained at stale shared-network sweep"
            );
            assert!(
                !sweep.host.run_dirs.contains(&alloc),
                "seed={seed}: {alloc} run directory remained at stale shared-network sweep"
            );
            assert!(
                !sweep.host.clones.contains_key(&alloc),
                "seed={seed}: {alloc} clone remained at stale shared-network sweep"
            );
        }

        let read_back =
            assert_boot_chain(seed, &recorded, &calls, sweep.call_index, &intercept.seeded);
        assert!(
            opened,
            "seed={seed}: the EXEC gate is still BootClosed after boot returned; it must open \
             after the read-back (#{read_back})\nrecorded intercept calls:\n{}",
            render(&recorded)
        );
        handle.shutdown(Duration::from_secs(2)).await.expect("clean seeded server shutdown");
    }

    proptest! {
        /// Outcome anchor: OUT-ND295-SHARED-SWITCH.
        /// S-ND295-13A — Boot reclaims old VMs, sweeps, clears stale members, then
        /// converges the program before admitting work.
        /// CONTRACT_SHAPE: bounded-change.
        #[test]
        #[ignore = "pending DELIVER step 08-02 (S-ND295-13A)"]
        fn reclamation_completes_before_stale_shared_network_sweep_for_every_seeded_prior_vm(
            seed in any::<u64>(),
        ) {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("seeded invariant runtime")
                .block_on(run_seed(seed));
        }
    }
}
