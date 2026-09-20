//! The per-allocation transparent-mTLS **install** driven port (GH #250).
//!
//! Declares [`MtlsIntercept`] — the substitutable boundary over the three
//! privileged, un-ownable primitives
//! [`MtlsInterceptWorker::start_alloc`](crate::mtls_intercept_worker::MtlsInterceptWorker::start_alloc)
//! performs (the `IP_TRANSPARENT` socket setup and the two `nft`/`ip`
//! shell-outs) — plus [`InterceptGuard`], the marker trait its RAII install
//! handles satisfy, and [`HostMtlsIntercept`], the production binding.
//!
//! Production wires [`HostMtlsIntercept`], whose three methods are one-line
//! delegations to the same `crate::mtls_intercept` free functions
//! `start_alloc` called before this port existed; tests wire
//! `overdrive_sim::adapters::mtls_intercept::SimMtlsIntercept`.

#![allow(
    clippy::result_large_err,
    reason = "GH #295 exact InterceptError retains complete rollback identity"
)]

use std::net::SocketAddrV4;
use std::sync::Arc;

use crate::mtls_intercept::{
    InterceptPostcondition, NetlinkError, Result, TproxyInterceptGuard, install_inbound_tproxy,
    install_outbound_tproxy, make_transparent_listener,
};

/// Module-private effect seam for the shared-program observe/atomic-replace
/// algorithm. The host algorithm remains above this boundary.
#[allow(dead_code, reason = "GH #295 exact accepted scaffold; DELIVER wires the host algorithm")]
trait SharedInterceptProgramIo: Send + Sync {
    fn observe(&self) -> std::result::Result<Option<InterceptPostcondition>, NetlinkError>;

    fn replace_atomically(
        &self,
        expected_current: Option<&InterceptPostcondition>,
        desired: Option<&InterceptPostcondition>,
    ) -> std::result::Result<(), NetlinkError>;
}

/// RAII handle for ONE completed intercept install.
///
/// Marker-only by design: the guard's ENTIRE contract is its `Drop`, so the
/// trait exposes no callable method. The worker holds guards for the alloc
/// lifetime and drops them on `stop_alloc`; it never calls anything on them.
///
/// # Observable invariants
/// Dropping a guard releases EXACTLY what its originating
/// [`MtlsIntercept::install_outbound`] / [`MtlsIntercept::install_inbound`]
/// call acquired — no more, no less — and releases nothing another guard owns.
/// Dropping never panics and never errors, including for a guard whose
/// underlying state was already released out-of-band.
///
/// WHAT is acquired and released is adapter-specific and is NOT part of this
/// contract: [`HostMtlsIntercept`] acquires one `nft` rule and its `Drop`
/// removes that rule by handle; a simulation adapter acquires nothing and its
/// `Drop` is a no-op. Both honour the invariant above.
pub trait InterceptGuard: Send + Sync {}

impl InterceptGuard for TproxyInterceptGuard {}

/// The per-allocation transparent-mTLS **install** driven port.
///
/// Wraps the three privileged, un-ownable primitives
/// [`MtlsInterceptWorker::start_alloc`](crate::mtls_intercept_worker::MtlsInterceptWorker::start_alloc)
/// performs — the `IP_TRANSPARENT` socket setup (`libc::socket` +
/// `setsockopt`) and the two `nft`/`ip` shell-outs — so the install surface is
/// substitutable at the composition root. Production wires
/// [`HostMtlsIntercept`]; tests wire `overdrive_sim::adapters::mtls_intercept::SimMtlsIntercept`.
///
/// SYNC, not `#[async_trait]`: every underlying primitive is a blocking
/// syscall or a blocking `std::process::Command`, `start_alloc` is itself
/// sync, and the contract awaits no store I/O — the repo criterion recorded on
/// [`MtlsResolve`](overdrive_core::traits::mtls_resolve::MtlsResolve). Sync
/// also keeps the trait dyn-compatible with no `Pin<Box<dyn Future>>`
/// allocation per install.
///
/// `Send + Sync + 'static` to be held as `Arc<dyn MtlsIntercept>` and shared
/// across the worker's per-alloc calls.
///
/// # NO `probe()` — deliberate, and recorded
///
/// This port carries no Earned-Trust `probe()` and the composition root
/// installs no boot gate for it, unlike its sibling
/// [`MtlsResolve`](overdrive_core::traits::mtls_resolve::MtlsResolve) and
/// `MtlsEnforcement`. This is a deliberate scope decision recorded in
/// ADR-0076 § Decision 4, not an oversight: the capability this port depends
/// on (`CAP_NET_ADMIN`) is already proven per-deploy at the netns-provision
/// seam that runs strictly UPSTREAM of every call to this port, so a boot
/// probe would re-prove at boot what the deploy path proves anyway — buying a
/// better diagnosis, not a new safety property, at the cost of a production
/// behaviour change out of GH #250's scope. Do NOT add one back without
/// superseding that decision.
pub trait MtlsIntercept: Send + Sync + 'static {
    /// Bind ONE TCP listener at `addr`, suitable for accepting the intercept
    /// leg the caller is standing up.
    ///
    /// This is the primitive both intercept legs are built from: leg-F
    /// (outbound, workload-facing) and leg-C (inbound, peer-facing).
    ///
    /// # Preconditions
    /// - `addr` is an IPv4 address the caller intends to accept on;
    ///   `127.0.0.1:0` (agent-chosen ephemeral loopback) is the production
    ///   shape for both legs.
    ///
    /// # Postconditions on `Ok(listener)`
    /// The returned listener is bound and listening at `addr`.
    /// `listener.local_addr()` reports the concrete bound address; when `addr`
    /// carried port 0 the reported port is the kernel-assigned ephemeral port
    /// and is NON-ZERO. Ownership transfers to the caller — dropping it closes
    /// the socket.
    ///
    /// # Edge cases
    /// Every failure — a refused socket option, `EADDRINUSE`, fd exhaustion —
    /// surfaces as
    /// [`InterceptError::TransparentListener`](crate::mtls_intercept::InterceptError::TransparentListener)
    /// carrying a
    /// cause-distinct `io::Error` source. No fd leaks on any error path: a
    /// partially-created socket is closed before returning.
    ///
    /// # Observable invariants
    /// Each call returns a DISTINCT listener; two calls with port 0 bind two
    /// distinct ephemeral ports. The call installs no `nft` rule and mutates no
    /// routing state.
    ///
    /// # Substrate note (NOT part of this contract)
    /// The PRODUCTION leg semantics require the socket to carry
    /// `IP_TRANSPARENT` + `IP_FREEBIND` — both legs are TPROXY-divert targets,
    /// and a plain socket cannot receive a non-rewriting `tproxy` divert's
    /// orig-dst-addressed SYN. That is [`HostMtlsIntercept`]'s obligation and
    /// is documented on it, NOT here: a simulation adapter holds no
    /// `CAP_NET_ADMIN`, receives no diverted traffic, and honours the
    /// contract above with a plain listener. Stating the setopts as a TRAIT
    /// postcondition would make the contract unimplementable by half its
    /// sanctioned implementors.
    fn bind_transparent(&self, addr: SocketAddrV4) -> Result<std::net::TcpListener>;

    /// Converge the one node-scoped shared rule/set program.
    fn converge_shared(
        &self,
        prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> Result<Box<dyn InterceptGuard>>;

    /// Observe the complete normalized shared rule/set identity without repair.
    fn observe_shared(&self) -> Result<Option<InterceptPostcondition>>;

    /// Install the per-allocation OUTBOUND egress intercept: capture TCP
    /// egress arriving on the workload's host-side veth and divert it,
    /// without rewriting the destination, to the agent's leg-F listener at
    /// `agent_leg_f_port` on loopback (ADR-0071 Path A, D-TME-4).
    ///
    /// # Preconditions
    /// - `host_veth` names the existing host-side workload veth.
    /// - `agent_leg_f_port` is the NON-ZERO bound port of a live leg-F
    ///   listener obtained from [`bind_transparent`](Self::bind_transparent).
    ///
    /// # Postconditions on `Ok(guard)`
    /// The outbound capture for `host_veth` is in effect **against this
    /// adapter's OWN substrate**, and every prerequisite it depends on has
    /// been converged idempotently. The returned guard OWNS exactly what this
    /// call acquired: dropping it releases that and nothing else (see
    /// [`InterceptGuard`]). What "in effect" MEANS is adapter-specific and is
    /// not observable through this trait — see the substrate note below.
    ///
    /// # Edge cases
    /// - Any install failure surfaces as one of the decomposed nft/ip install
    ///   errors —
    ///   [`InterceptError::NftRuleInstallFailed`](crate::mtls_intercept::InterceptError::NftRuleInstallFailed)
    ///   (op-keyed, errno-carrying),
    ///   [`InterceptError::NftHandleRecoveryFailed`](crate::mtls_intercept::InterceptError::NftHandleRecoveryFailed),
    ///   or the shared-routing-infra
    ///   [`InterceptError::IpRuleAddFailed`](crate::mtls_intercept::InterceptError::IpRuleAddFailed)
    ///   /
    ///   [`InterceptError::IpRouteLocalAddFailed`](crate::mtls_intercept::InterceptError::IpRouteLocalAddFailed).
    /// - A re-install for a veth already carrying an identical capture is
    ///   idempotent-by-convergence; it does not create a duplicate.
    ///
    /// # Observable invariants
    /// One call acquires at most ONE capture. On `Err` NOTHING acquired by
    /// this call outlives it — every partially-applied step is reverted or was
    /// never applied, so a failed install leaks nothing.
    ///
    /// # Substrate note (NOT part of this contract)
    /// [`HostMtlsIntercept`] realises the capture as exactly one `nft` rule
    /// appended to the shared prerouting chain, after converging the
    /// node-global shared routing infra (fwmark `ip rule`, `local` route, the
    /// shared chain, the head exemption); its guard's `Drop` removes that rule
    /// by handle and leaves the shared infra intact. A simulation adapter
    /// realises it as nothing at all. Both honour the contract above.
    fn install_outbound(
        &self,
        host_veth: &str,
        agent_leg_f_port: u16,
    ) -> Result<Box<dyn InterceptGuard>>;

    /// Install ONE per-Service-port INBOUND intercept: capture connections
    /// destined for `virt` (the canonical per-workload address paired with one
    /// DECLARED Service listener port) and divert them to the agent's leg-C
    /// listener at `agent_leg_c_port` on loopback (D-A1, GH #241).
    ///
    /// # Preconditions
    /// - `virt` pairs the canonical per-workload address with a DECLARED
    ///   Service listener port — never the ephemeral leg-C port (D-BLOCKER1 /
    ///   D-TME-10 one-source/two-readers).
    /// - `agent_leg_c_port` is the NON-ZERO bound port of a live leg-C
    ///   listener obtained from [`bind_transparent`](Self::bind_transparent).
    ///
    /// # Postconditions on `Ok(guard)`
    /// The inbound capture for `virt` is in effect **against this adapter's
    /// OWN substrate**, and every prerequisite it depends on has been
    /// converged idempotently. The returned guard OWNS exactly what this call
    /// acquired: dropping it releases that and nothing else (see
    /// [`InterceptGuard`]). What "in effect" MEANS is adapter-specific and is
    /// not observable through this trait — see the substrate note below.
    ///
    /// # Edge cases
    /// Identical failure surface to
    /// [`install_outbound`](Self::install_outbound) — the decomposed nft/ip
    /// install errors
    /// ([`InterceptError::NftRuleInstallFailed`](crate::mtls_intercept::InterceptError::NftRuleInstallFailed)
    /// and siblings), each naming the failing operation.
    /// The caller installs N captures for N declared ports and ZERO for a
    /// Job-kind / host-netns workload; that N-vs-0 decision is the CALLER's,
    /// not this method's.
    ///
    /// # Observable invariants
    /// One call acquires at most ONE capture; on `Err`, nothing acquired by
    /// this call outlives it.
    ///
    /// # Substrate note (NOT part of this contract)
    /// [`HostMtlsIntercept`] realises the capture as exactly one `nft` rule
    /// keyed `ip daddr <virt.ip> tcp dport <virt.port>`, tproxy-redirected to
    /// `agent_leg_c_port`, removed by handle on guard `Drop`. A simulation
    /// adapter realises it as nothing at all.
    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        agent_leg_c_port: u16,
    ) -> Result<Box<dyn InterceptGuard>>;
}

/// Production [`MtlsIntercept`] binding.
///
/// Each method is a ONE-LINE delegation
/// to the existing `crate::mtls_intercept` free function it wraps — the
/// adapter adds no logic, so there is nothing in it for a sim adapter to
/// diverge from except the substrate itself.
///
/// # Substrate obligations (BEYOND the [`MtlsIntercept`] contract)
///
/// These are this adapter's obligations, deliberately NOT stated on the trait
/// (a trait postcondition no sanctioned implementor can honour is a broken
/// contract, per `.claude/rules/development.md` § "Trait definitions specify
/// behavior, not just signature"). They are what the Tier-3 suite asserts:
///
/// - [`bind_transparent`](MtlsIntercept::bind_transparent) returns a socket
///   carrying BOTH `IP_TRANSPARENT` (so a non-rewriting `tproxy` divert's
///   orig-dst-addressed SYN is accepted, and `getsockname` recovers the
///   orig-dst) and `IP_FREEBIND` (so leg-C can bind a non-local address on the
///   OUTPUT path).
/// - [`install_outbound`](MtlsIntercept::install_outbound) appends exactly one
///   host-veth-keyed egress TPROXY rule and returns its allocation-scoped guard.
/// - [`install_inbound`](MtlsIntercept::install_inbound) appends EXACTLY ONE
///   `nft` rule keyed `ip daddr <virt.ip> tcp dport <virt.port>` → tproxy to
///   `127.0.0.1:<agent_leg_c_port>`, removed by handle on guard `Drop`.
pub struct HostMtlsIntercept {
    shared_program_io: Arc<dyn SharedInterceptProgramIo>,
}

impl Clone for HostMtlsIntercept {
    fn clone(&self) -> Self {
        Self { shared_program_io: Arc::clone(&self.shared_program_io) }
    }
}

impl std::fmt::Debug for HostMtlsIntercept {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("HostMtlsIntercept").finish_non_exhaustive()
    }
}

impl HostMtlsIntercept {
    /// Construct the production binding with its private real nft I/O adapter.
    #[must_use]
    #[allow(
        clippy::missing_const_for_fn,
        reason = "the accepted host binding owns an Arc to its private real shared-program I/O adapter"
    )]
    pub fn new() -> Self {
        Self { shared_program_io: Arc::new(RealSharedInterceptProgramIo) }
    }

    #[cfg(test)]
    fn with_shared_program_io(io: Arc<dyn SharedInterceptProgramIo>) -> Self {
        Self { shared_program_io: io }
    }

    #[expect(
        clippy::panic,
        reason = "RED scaffold; DELIVER implements the GH #295 replacement/read-back/rollback algorithm above this exact private I/O seam"
    )]
    #[allow(dead_code, reason = "activated by the accepted shared-rule port single cut")]
    fn replace_shared_program_for_boot(
        &self,
        _requested: InterceptPostcondition,
    ) -> Result<Box<dyn InterceptGuard>> {
        let _ = &self.shared_program_io;
        panic!("Not yet implemented -- RED scaffold (GH #295 shared intercept replacement)")
    }

    #[expect(
        clippy::panic,
        reason = "RED scaffold; DELIVER implements the GH #295 non-repairing runtime identity check"
    )]
    #[allow(dead_code, reason = "activated by the accepted shared-rule port single cut")]
    fn require_shared_program_at_runtime(&self, _expected: InterceptPostcondition) -> Result<()> {
        let _ = &self.shared_program_io;
        panic!("Not yet implemented -- RED scaffold (GH #295 shared intercept runtime audit)")
    }
}

#[derive(Debug)]
struct RealSharedInterceptProgramIo;

impl SharedInterceptProgramIo for RealSharedInterceptProgramIo {
    #[expect(clippy::panic, reason = "RED scaffold; DELIVER wires normalized nft read-back")]
    fn observe(&self) -> std::result::Result<Option<InterceptPostcondition>, NetlinkError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 shared intercept observe)")
    }

    #[expect(clippy::panic, reason = "RED scaffold; DELIVER wires atomic nft replacement")]
    fn replace_atomically(
        &self,
        _expected_current: Option<&InterceptPostcondition>,
        _desired: Option<&InterceptPostcondition>,
    ) -> std::result::Result<(), NetlinkError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 shared intercept atomic replace)")
    }
}

impl Default for HostMtlsIntercept {
    fn default() -> Self {
        Self::new()
    }
}

impl MtlsIntercept for HostMtlsIntercept {
    fn bind_transparent(&self, addr: SocketAddrV4) -> Result<std::net::TcpListener> {
        make_transparent_listener(addr)
    }

    #[expect(clippy::panic, reason = "RED scaffold; DELIVER wires approved shared convergence")]
    fn converge_shared(
        &self,
        _prior: Option<&InterceptPostcondition>,
        _leg_f: SocketAddrV4,
        _leg_c: SocketAddrV4,
    ) -> Result<Box<dyn InterceptGuard>> {
        panic!("Not yet implemented -- RED scaffold (GH #295 shared intercept convergence)")
    }

    #[expect(clippy::panic, reason = "RED scaffold; DELIVER wires normalized observation")]
    fn observe_shared(&self) -> Result<Option<InterceptPostcondition>> {
        panic!("Not yet implemented -- RED scaffold (GH #295 shared intercept observation)")
    }

    fn install_outbound(
        &self,
        host_veth: &str,
        agent_leg_f_port: u16,
    ) -> Result<Box<dyn InterceptGuard>> {
        install_outbound_tproxy(host_veth, agent_leg_f_port)
            .map(|guard| Box::new(guard) as Box<dyn InterceptGuard>)
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        agent_leg_c_port: u16,
    ) -> Result<Box<dyn InterceptGuard>> {
        install_inbound_tproxy(virt, agent_leg_c_port)
            .map(|guard| Box::new(guard) as Box<dyn InterceptGuard>)
    }
}

#[cfg(test)]
#[allow(clippy::doc_markdown, clippy::expect_used)]
mod shared_program_rollback_acceptance {
    use std::collections::VecDeque;

    use parking_lot::Mutex;

    use super::*;
    use crate::mtls_intercept::{InterceptError, InterceptSharedRollbackOperation};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Call {
        Observe,
        ReplacePresent,
        RestorePresent,
        RestoreAbsent,
    }

    struct ScriptedIo {
        observations:
            Mutex<VecDeque<std::result::Result<Option<InterceptPostcondition>, NetlinkError>>>,
        replacements: Mutex<VecDeque<std::result::Result<(), NetlinkError>>>,
        calls: Mutex<Vec<Call>>,
    }

    impl ScriptedIo {
        fn new(
            observations: Vec<std::result::Result<Option<InterceptPostcondition>, NetlinkError>>,
            replacements: Vec<std::result::Result<(), NetlinkError>>,
        ) -> Self {
            Self {
                observations: Mutex::new(observations.into()),
                replacements: Mutex::new(replacements.into()),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.lock().clone()
        }
    }

    impl SharedInterceptProgramIo for ScriptedIo {
        fn observe(&self) -> std::result::Result<Option<InterceptPostcondition>, NetlinkError> {
            self.calls.lock().push(Call::Observe);
            self.observations.lock().pop_front().expect("scripted observe result")
        }

        fn replace_atomically(
            &self,
            expected_current: Option<&InterceptPostcondition>,
            desired: Option<&InterceptPostcondition>,
        ) -> std::result::Result<(), NetlinkError> {
            let call = match (expected_current.is_some(), desired.is_some()) {
                (_, true) => Call::ReplacePresent,
                (true, false) => Call::RestorePresent,
                (false, false) => Call::RestoreAbsent,
            };
            self.calls.lock().push(call);
            self.replacements.lock().pop_front().expect("scripted replace result")
        }
    }

    fn program(port: u16) -> InterceptPostcondition {
        InterceptPostcondition::ConstantRules {
            table_and_chains: vec![b"table-and-chains".to_vec()],
            sets: vec![b"three-set-schemas".to_vec()],
            prerouting: vec![format!("leg-f:{port}").into_bytes()],
            output: vec![format!("leg-c:{}", port + 1).into_bytes()],
        }
    }

    fn netlink_error(op: &'static str, errno: i32) -> NetlinkError {
        NetlinkError::nft(op, std::io::Error::from_raw_os_error(errno))
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER step for GH #295 shared-program replacement/rollback algorithm"]
    #[allow(
        clippy::too_many_lines,
        reason = "one finite table keeps the five disjoint source-honest rollback dispositions together"
    )]
    fn replacement_and_every_rollback_disposition_preserve_exact_identity_and_source() {
        let requested = program(20_000);
        let wrong_replacement = program(20_100);

        // Clean first boot: successful rollback must restore absence, not a
        // fabricated empty program.
        {
            let io = Arc::new(ScriptedIo::new(
                vec![Ok(None), Ok(Some(wrong_replacement.clone())), Ok(None)],
                vec![Ok(()), Ok(())],
            ));
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let error = host
                .replace_shared_program_for_boot(requested.clone())
                .err()
                .expect("replacement mismatch refuses startup after exact rollback");
            assert!(matches!(
                error,
                InterceptError::NftSharedReplacementMismatchRolledBack {
                    prior: None,
                    requested: ref actual,
                    replacement_observed: Some(ref observed),
                } if actual == &requested && observed == &wrong_replacement
            ));
            assert_eq!(
                io.calls(),
                [
                    Call::Observe,
                    Call::ReplacePresent,
                    Call::Observe,
                    Call::RestoreAbsent,
                    Call::Observe
                ]
            );
        }

        let prior = program(19_000);

        // Atomic replacement rejection preserves the exact prior and retains
        // the real netlink source without attempting rollback.
        {
            let io = Arc::new(ScriptedIo::new(
                vec![Ok(Some(prior.clone()))],
                vec![Err(netlink_error("replace", libc::EBUSY))],
            ));
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let error = host
                .replace_shared_program_for_boot(requested.clone())
                .err()
                .expect("replacement rejection refuses startup");
            assert!(matches!(
                error,
                InterceptError::NftSharedReplaceFailed {
                    prior: Some(ref actual_prior),
                    requested: ref actual_requested,
                    source: NetlinkError::Nft { .. },
                } if actual_prior == &prior && actual_requested == &requested
            ));
            assert_eq!(io.calls(), [Call::Observe, Call::ReplacePresent]);
        }

        // Rollback write failure names RestorePrior and retains the direct
        // netlink source plus prior/requested/replacement observations.
        {
            let io = Arc::new(ScriptedIo::new(
                vec![Ok(Some(prior.clone())), Ok(Some(wrong_replacement.clone()))],
                vec![Ok(()), Err(netlink_error("restore-prior", libc::EIO))],
            ));
            let host = HostMtlsIntercept::with_shared_program_io(io);
            let error = host
                .replace_shared_program_for_boot(requested.clone())
                .err()
                .expect("rollback write failure refuses startup");
            assert!(matches!(
                error,
                InterceptError::NftSharedRollbackFailed {
                    operation: InterceptSharedRollbackOperation::RestorePrior,
                    source: NetlinkError::Nft { .. },
                    ..
                }
            ));
        }

        // Rollback read failure names ReadBackPrior and retains its own real
        // source rather than fabricating a semantic mismatch.
        {
            let io = Arc::new(ScriptedIo::new(
                vec![
                    Ok(Some(prior.clone())),
                    Ok(Some(wrong_replacement.clone())),
                    Err(netlink_error("read-back-prior", libc::EIO)),
                ],
                vec![Ok(()), Ok(())],
            ));
            let host = HostMtlsIntercept::with_shared_program_io(io);
            let error = host
                .replace_shared_program_for_boot(requested.clone())
                .err()
                .expect("rollback read failure refuses startup");
            assert!(matches!(
                error,
                InterceptError::NftSharedRollbackFailed {
                    operation: InterceptSharedRollbackOperation::ReadBackPrior,
                    source: NetlinkError::Nft { .. },
                    ..
                }
            ));
        }

        // Successful rollback I/O with the wrong identity is the source-less
        // semantic mismatch variant and retains both observations.
        {
            let wrong_rollback = program(18_000);
            let io = Arc::new(ScriptedIo::new(
                vec![
                    Ok(Some(prior.clone())),
                    Ok(Some(wrong_replacement.clone())),
                    Ok(Some(wrong_rollback.clone())),
                ],
                vec![Ok(()), Ok(())],
            ));
            let host = HostMtlsIntercept::with_shared_program_io(io);
            let error = host
                .replace_shared_program_for_boot(requested.clone())
                .err()
                .expect("wrong rollback identity refuses startup without a fake source");
            assert!(matches!(
                error,
                InterceptError::NftSharedRollbackPostconditionMismatch {
                    prior: Some(ref actual_prior),
                    requested: ref actual_requested,
                    replacement_observed: Some(ref replacement),
                    rollback_observed: Some(ref rollback),
                } if actual_prior == &prior
                    && actual_requested == &requested
                    && replacement == &wrong_replacement
                    && rollback == &wrong_rollback
            ));
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER step for GH #295 successful shared-program replacement/adoption"]
    fn fresh_replace_exact_prior_rollback_and_idempotent_reapply_are_complete() {
        let requested = program(20_000);

        // Fresh absence installs and reads back the exact requested identity.
        {
            let io = Arc::new(ScriptedIo::new(
                vec![Ok(None), Ok(Some(requested.clone()))],
                vec![Ok(())],
            ));
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let guard = host
                .replace_shared_program_for_boot(requested.clone())
                .expect("fresh shared program replacement succeeds");
            assert_eq!(io.calls(), [Call::Observe, Call::ReplacePresent, Call::Observe]);
            drop(guard);
        }

        // A mismatched replacement restores an exact present prior and returns
        // the source-less rolled-back disposition.
        {
            let prior = program(19_000);
            let wrong = program(21_000);
            let io = Arc::new(ScriptedIo::new(
                vec![Ok(Some(prior.clone())), Ok(Some(wrong.clone())), Ok(Some(prior.clone()))],
                vec![Ok(()), Ok(())],
            ));
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let error = host
                .replace_shared_program_for_boot(requested.clone())
                .err()
                .expect("wrong replacement is rolled back to the exact prior");
            assert!(matches!(
                error,
                InterceptError::NftSharedReplacementMismatchRolledBack {
                    prior: Some(actual_prior),
                    requested: actual_requested,
                    replacement_observed: Some(actual_wrong),
                } if actual_prior == prior
                    && actual_requested == requested
                    && actual_wrong == wrong
            ));
            assert_eq!(
                io.calls(),
                [
                    Call::Observe,
                    Call::ReplacePresent,
                    Call::Observe,
                    Call::RestorePresent,
                    Call::Observe
                ]
            );
        }

        // Reapplying an already exact identity adopts it without rewriting.
        {
            let io = Arc::new(ScriptedIo::new(vec![Ok(Some(requested.clone()))], Vec::new()));
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let guard = host
                .replace_shared_program_for_boot(requested)
                .expect("exact prior identity is an idempotent adoption");
            assert_eq!(io.calls(), [Call::Observe]);
            drop(guard);
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER step for GH #295 non-repairing runtime shared-program audit"]
    fn runtime_present_wrong_target_is_reported_without_mutation() {
        let expected = program(20_000);
        let wrong_target = program(20_100);
        let io = Arc::new(ScriptedIo::new(vec![Ok(Some(wrong_target.clone()))], Vec::new()));
        let host = HostMtlsIntercept::with_shared_program_io(io.clone());

        let error = host
            .require_shared_program_at_runtime(expected.clone())
            .expect_err("present wrong target is a structured conflict");
        assert!(matches!(
            error,
            InterceptError::PostconditionMismatch {
                expected: ref actual_expected,
                observed: Some(ref actual_observed),
            } if actual_expected == &expected && actual_observed == &wrong_target
        ));
        assert_eq!(io.calls(), [Call::Observe], "runtime audit never rewrites a present target");
    }
}
