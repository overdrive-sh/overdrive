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

use std::net::SocketAddrV4;

use crate::mtls_intercept::{
    Result, TproxyInterceptGuard, install_inbound_tproxy, install_outbound_tproxy,
    make_transparent_listener,
};

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

    /// Install the per-alloc OUTBOUND egress intercept: capture all TCP egress
    /// arriving on the workload's host-side veth (`host_veth`) and divert it,
    /// without rewriting the destination, to the agent's leg-F listener at
    /// `agent_leg_f_port` on loopback (ADR-0071 Path A, D-TME-4).
    ///
    /// # Preconditions
    /// - `host_veth` names a host-side veth that EXISTS (the action-shim C3
    ///   provision seam created it and injected the name on
    ///   `AllocationSpec.host_veth`).
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
    /// - Any install failure surfaces as
    ///   [`InterceptError::TproxyInstall`](crate::mtls_intercept::InterceptError::TproxyInstall)
    ///   whose `reason` names the failing operation.
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
    /// [`install_outbound`](Self::install_outbound) —
    /// [`InterceptError::TproxyInstall`](crate::mtls_intercept::InterceptError::TproxyInstall)
    /// with an operation-naming `reason`.
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
/// - [`install_outbound`](MtlsIntercept::install_outbound) appends EXACTLY ONE
///   `nft` rule to the shared prerouting chain (`iifname <host_veth>` → tproxy
///   to `127.0.0.1:<agent_leg_f_port>`) after converging the node-global
///   shared routing infra (fwmark `ip rule`, `local` route, shared chain, head
///   exemption); the returned guard's `Drop` removes that rule BY HANDLE and
///   leaves the shared infra intact.
/// - [`install_inbound`](MtlsIntercept::install_inbound) appends EXACTLY ONE
///   `nft` rule keyed `ip daddr <virt.ip> tcp dport <virt.port>` → tproxy to
///   `127.0.0.1:<agent_leg_c_port>`, removed by handle on guard `Drop`.
#[derive(Debug, Clone, Copy)]
pub struct HostMtlsIntercept;

impl HostMtlsIntercept {
    /// Construct the production binding. Stateless.
    #[must_use]
    pub const fn new() -> Self {
        Self
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
