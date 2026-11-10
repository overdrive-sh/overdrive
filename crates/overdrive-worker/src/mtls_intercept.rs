//! Kernel effects for the host mTLS interception adapter.
//!
//! [`crate::mtls_intercept_port::HostMtlsIntercept`] uses the operations in
//! this module to bind transparent listeners and converge the node-shared
//! intercept program and its allocation elements. Per-connection acceptance
//! and original-destination recovery are owned by the listener port.

#![allow(
    clippy::result_large_err,
    reason = "GH #295 exact source-honest rollback variants retain complete prior/requested/observed identities"
)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "raw libc syscall glue: struct-size -> socklen_t (compile-time constant) and AF_INET -> sa_family_t casts are FFI-width conversions on bounded values; cannot truncate or wrap. Mirrors the module-level allow on the sibling overdrive_dataplane::mtls adapter."
)]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::FromRawFd as _;

use overdrive_netlink::{Client, block_on_host_netlink, errno_is_idempotent};

// Re-exported: [`NetlinkError`] is already part of this module's public API (it
// is the `#[source]` field type of the decomposed [`InterceptError`] variants
// such as `NftRuleInstallFailed` / `IpRuleAddFailed` / `IpRouteLocalAddFailed`). The
// re-export gives that type a nameable path for the `overdrive-sim` /
// `overdrive-control-plane` doubles that construct those variants' synthetic
// sources, without those crates taking a direct dependency on the
// `adapter-host` `overdrive-netlink` crate.
pub use overdrive_netlink::NetlinkError;

/// One of the two node-shared transparent-interception listener legs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterceptLeg {
    F,
    C,
}

/// Closed shared-set identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterceptSet {
    ManagedGuestIps,
    OutboundSources,
    InboundDestinations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterceptElementOperation {
    Insert,
    Delete,
    ReadBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterceptSharedRollbackOperation {
    RestorePrior,
    ReadBackPrior,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterceptElementKey {
    Address(Ipv4Addr),
    Destination(SocketAddrV4),
}

/// Complete normalized read-back identity for one shared intercept fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterceptPostcondition {
    ListenerPort {
        leg: InterceptLeg,
        port: u16,
    },
    Element {
        set: InterceptSet,
        key: InterceptElementKey,
        present: bool,
    },
    ConstantRules {
        table_and_chains: Vec<Vec<u8>>,
        sets: Vec<Vec<u8>>,
        prerouting: Vec<Vec<u8>>,
        output: Vec<Vec<u8>>,
    },
}

/// `IP_TRANSPARENT` sockopt level value — libc 0.2 does not name it (same as
/// the proven `roles.rs::make_transparent_listener` reference).
const IP_TRANSPARENT: libc::c_int = 19;

/// `IP_FREEBIND` sockopt level value — lets the leg-C listener bind the
/// NON-LOCAL `workload_addr` (∈ 10.99.0.0/16, not assigned to the host) on the
/// OUTPUT path, so the agent's host-originated leg-B re-dial is intercepted
/// symmetric with the prerouting path. libc 0.2 does not name it. (REV-5)
const IP_FREEBIND: libc::c_int = 15;

/// The fwmark the TPROXY rule stamps and the `ip rule` companion matches so
/// the redirected connection is routed via the `local` route table. A SINGLE
/// shared fwmark suffices for N destinations: TPROXY preserves daddr, so the
/// agent recovers orig-dst per-flow via `getsockname` — there is nothing
/// per-virt to distinguish in the routing layer (research caveat
/// "single-fwmark sufficiency", F1/F5).
const TPROXY_FWMARK: u32 = 0x1;

/// The routing-policy table number the `ip rule fwmark` companion looks up
/// and the `ip route local … table` companion populates. Shared and fixed
/// across all inbound intercepts (kernel-canonical table 100).
const TPROXY_RT_TABLE: u32 = 100;

/// Typed error surface for the worker's intercept-install + leg-acquire role.
///
/// Distinct variant per failure mode (`.claude/rules/development.md`
/// § Errors): a transparent-listener setup failure, a TPROXY-install
/// failure, a leg-accept failure, and an orig-dst recovery failure each name
/// their own cause so the caller (and operator) gets cause-specific
/// diagnostics.
#[derive(Debug, thiserror::Error)]
pub enum InterceptError {
    #[error("shared mTLS listener {leg:?} port mismatch: expected {expected}, observed {actual}")]
    SharedListenerPortMismatch { leg: InterceptLeg, expected: u16, actual: u16 },
    #[error("atomic replacement of the shared mTLS rule/set program failed")]
    NftSharedReplaceFailed {
        prior: Option<InterceptPostcondition>,
        requested: InterceptPostcondition,
        #[source]
        source: NetlinkError,
    },
    #[error("rollback of the shared mTLS rule/set program failed during {operation:?}")]
    NftSharedRollbackFailed {
        operation: InterceptSharedRollbackOperation,
        prior: Option<InterceptPostcondition>,
        requested: InterceptPostcondition,
        replacement_read_source: Option<NetlinkError>,
        replacement_observed: Option<InterceptPostcondition>,
        #[source]
        source: NetlinkError,
    },
    #[error("shared mTLS rollback read-back did not restore the prior identity")]
    NftSharedRollbackPostconditionMismatch {
        prior: Option<InterceptPostcondition>,
        requested: InterceptPostcondition,
        #[source]
        replacement_read_source: Option<NetlinkError>,
        replacement_observed: Option<InterceptPostcondition>,
        rollback_observed: Option<InterceptPostcondition>,
    },
    #[error("shared mTLS replacement mismatched and the exact prior identity was restored")]
    NftSharedReplacementMismatchRolledBack {
        prior: Option<InterceptPostcondition>,
        requested: InterceptPostcondition,
        replacement_observed: Option<InterceptPostcondition>,
    },
    #[error(
        "shared mTLS replacement read failed after commit and the exact prior identity was restored"
    )]
    NftSharedReplacementReadFailedRolledBack {
        prior: Option<InterceptPostcondition>,
        requested: InterceptPostcondition,
        #[source]
        replacement_read_source: NetlinkError,
    },
    #[error("shared mTLS set element update failed: {source}")]
    NftElementUpdateFailed {
        set: InterceptSet,
        operation: InterceptElementOperation,
        key: InterceptElementKey,
        #[source]
        source: NetlinkError,
    },
    #[error("shared mTLS intercept postcondition mismatch")]
    PostconditionMismatch {
        expected: InterceptPostcondition,
        observed: Option<InterceptPostcondition>,
    },
    /// The worker's verdict over an observation whose `policy_route` is
    /// `false`: the fwmark `0x1` rule to table 100, or table 100's
    /// `local 0.0.0.0/0` route on `lo`, is missing. Source-less: it is a
    /// finding about a read, not a failed operation. No adapter returns it;
    /// only `MtlsInterceptWorker`'s boot read-back and runtime audit construct
    /// it (FD § "[REF] Driven port — intercept element release, member
    /// convergence, boot clear" (the typed causes of the observation checks)).
    // RED scaffold (DR-08 (a)): not constructed before DELIVER step 08-02.
    #[error(
        "shared mTLS policy route is absent: the fwmark 0x1 rule to table 100 or table 100's local 0.0.0.0/0 route on lo is missing"
    )]
    PolicyRouteAbsent,
    /// The worker's verdict when converging the dynamic member sets to the
    /// empty set returned `Ok` but the read-back still holds `observed`
    /// (boot step 6.2), or when boot step 6.6's read-back holds members.
    /// Source-less. No adapter returns it; only `MtlsInterceptWorker`'s boot
    /// member clear and boot read-back construct it, as the source of
    /// `MtlsSharedOwnerError::BootMemberClear`.
    // RED scaffold (DR-08 (a)): not constructed before DELIVER step 08-02.
    #[error("shared mTLS dynamic members remain after convergence to the empty set: {observed:?}")]
    MembersRemain { observed: crate::mtls_intercept_port::InterceptMembers },
    /// An allocation element method ran before this process recorded a program
    /// through a successful `converge_shared` (DISTILL gap B-8). Source-less:
    /// no lower operation ran.
    // RED scaffold (B-8): not constructed before DELIVER step 05-01.
    #[error(
        "shared mTLS intercept program has not been converged by this process; allocation intercept elements cannot be installed or removed"
    )]
    SharedProgramNotConverged,
    /// `make_transparent_listener` could not stand up the agent's
    /// `IP_TRANSPARENT` inbound leg-C listener (socket / setsockopt / bind /
    /// listen failed). Needs `CAP_NET_ADMIN` for the `IP_TRANSPARENT` setopt.
    #[error("transparent leg-C listener setup failed on {addr}: {source}")]
    TransparentListener {
        /// The address the listener was being bound to.
        addr: SocketAddrV4,
        /// The originating syscall error.
        #[source]
        source: std::io::Error,
    },
    /// A hand-rolled nftables `NETLINK_NETFILTER` op (table / chain / rule
    /// ensure, per-virt append, output-divert append, `GETRULE` recovery dump,
    /// or by-handle delete) failed (ADR-0085 D3). Carries the failing op and the
    /// embedded errno-carrying [`NetlinkError`] so the caller (and operator) get
    /// cause-specific, op-keyed diagnostics on the packet-path.
    #[error("nft rule install ({op}) failed: {source}")]
    NftRuleInstallFailed {
        /// The failing nft op (`ensure-table` / `append-inbound` / `delete-rule` / …).
        op: &'static str,
        /// The originating netlink error (op-keyed, errno-carrying).
        #[source]
        source: NetlinkError,
    },
    /// Ensuring the shared `fwmark <TPROXY_FWMARK> lookup <TPROXY_RT_TABLE>`
    /// FIB policy rule failed — either the `RTM_GETRULE` dump-then-add
    /// presence check or the `RTM_NEWRULE` add itself (the ported dump-then-add
    /// guard, ADR-0085 D3/D6). The embedded [`NetlinkError`] names the failing
    /// netlink op and carries the typed errno.
    #[error("ensuring the shared fwmark FIB rule failed: {source}")]
    IpRuleAddFailed {
        /// The originating netlink error (`op = "rule-get"` / `"rule-add"`).
        #[source]
        source: NetlinkError,
    },
    /// Ensuring the shared `local 0.0.0.0/0 dev lo table <TPROXY_RT_TABLE>`
    /// route failed for a reason other than the idempotent `-EEXIST`
    /// already-converged case (ADR-0085 D3/D6). The embedded [`NetlinkError`]
    /// names the failing netlink op and carries the typed errno.
    #[error("ensuring the shared local route failed: {source}")]
    IpRouteLocalAddFailed {
        /// The originating netlink error (`op = "local-add"`).
        #[source]
        source: NetlinkError,
    },
    /// Accepting a redirected connection on an intercept listener failed.
    #[error("leg accept failed on the {direction} intercept listener: {source}")]
    Accept {
        /// `"inbound"` or `"outbound"` — which intercept listener accept failed on.
        direction: &'static str,
        /// The originating accept error.
        #[source]
        source: std::io::Error,
    },
    /// The original destination could not be recovered via `getsockname` on a
    /// TPROXY-redirected accepted leg.
    #[error("getsockname original-destination recovery failed: {source}")]
    OrigDst {
        /// The originating `getsockname` error.
        #[source]
        source: std::io::Error,
    },
}

/// Result alias for the intercept-install + leg-acquire surface.
pub type Result<T, E = InterceptError> = std::result::Result<T, E>;

/// Create the agent's `IP_TRANSPARENT` inbound leg-C listener bound to `addr`.
///
/// Sets `SO_REUSEADDR` + `IP_TRANSPARENT` then binds + listens, under
/// `CAP_NET_ADMIN`. Productionises `roles.rs::make_transparent_listener`.
///
/// # Errors
///
/// Returns [`InterceptError::TransparentListener`] on any failing syscall
/// (socket / setsockopt / bind / listen) — including `EPERM` when the process
/// lacks `CAP_NET_ADMIN` for the `IP_TRANSPARENT` setopt.
pub fn make_transparent_listener(addr: SocketAddrV4) -> Result<std::net::TcpListener> {
    let err = |source| InterceptError::TransparentListener { addr, source };

    // SAFETY: each raw syscall's return code is checked; on any failure the
    // partially-created fd is closed before returning, and a successful fd is
    // adopted by `TcpListener::from_raw_fd` (which owns it from then on).
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        // Defensive FFI return-code check; libc::socket() with these constant
        // args cannot be made to fail in a black-box test (only RLIMIT_NOFILE
        // exhaustion would, which is hostile/flaky), so the `< 0 → ==/<=`
        // mutants are unkillable black-box. They are accepted misses: the
        // diff-scoped gate stays ≥ 80% with them counted (the substantive
        // orig-dst recovery + preclean mutants ARE killed). The bare
        // `// mutants: skip` below documents the intent per the repo
        // convention, though cargo-mutants v27's comment-skip parser does
        // not reliably fire it for a statement-level guard (see
        // `.cargo/mutants.toml` § ProbeRunner::probe for the same limitation).
        // mutants: skip
        if fd < 0 {
            return Err(err(std::io::Error::last_os_error()));
        }
        // Any error after this point must close `fd` before returning.
        let one: libc::c_int = 1;
        let so_reuse = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            std::ptr::from_ref(&one).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        if so_reuse != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(err(e));
        }
        let ip_transparent = libc::setsockopt(
            fd,
            libc::IPPROTO_IP,
            IP_TRANSPARENT,
            std::ptr::from_ref(&one).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        if ip_transparent != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(err(e));
        }
        // IP_FREEBIND lets the leg-C listener bind a NON-LOCAL address on the
        // OUTPUT path (the agent's host-originated leg-B re-dial to a backend
        // `workload_addr` ∈ 10.99.0.0/16, not assigned to the host). Set
        // UNCONDITIONALLY: harmless on the prerouting path (which binds
        // `127.0.0.1`, already local) and required on the output path — so the
        // ONE transparent listener serves both the prerouting (peer SYN,
        // local-addr bind) and output (leg-B re-dial, non-local-addr bind) paths
        // (REV-5; spike-proven, set unconditionally). The failure maps to the
        // same `TransparentListener` variant as the `IP_TRANSPARENT` setopt above.
        let ip_freebind = libc::setsockopt(
            fd,
            libc::IPPROTO_IP,
            IP_FREEBIND,
            std::ptr::from_ref(&one).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        if ip_freebind != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(err(e));
        }
        // Mark agent-owned transparent listener replies with the existing
        // leg-S exemption mark. The shared output fallback drop otherwise
        // treats the SYN-ACK to a managed guest source as an unregistered
        // destination and prevents the node-shared listener from admitting
        // the connection. The mark is inherited by accepted sockets and is
        // consumed only by the constant head exemption.
        let mark: libc::c_uint = overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
        let mark_rc = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_MARK,
            std::ptr::from_ref(&mark).cast(),
            std::mem::size_of::<libc::c_uint>() as libc::socklen_t,
        );
        if mark_rc != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(err(e));
        }
        let sa = sockaddr_in_from(addr);
        let bind_rc = libc::bind(
            fd,
            std::ptr::from_ref(&sa).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        );
        if bind_rc != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(err(e));
        }
        if libc::listen(fd, 16) != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(err(e));
        }
        Ok(std::net::TcpListener::from_raw_fd(fd))
    }
}

/// Ensure the shared `fwmark <TPROXY_FWMARK> lookup <TPROXY_RT_TABLE>` FIB
/// policy rule exists, via the PORTED dump-then-add guard over rtnetlink
/// (ADR-0085 D6). The `RTM_GETRULE` presence check and the `RTM_NEWRULE` add
/// run as ONE logical check-and-act on a single dedicated netlink thread —
/// splitting them across two clients/threads would reopen a TOCTOU window in
/// which a concurrent install stacks a duplicate (the spike-D regression;
/// `.claude/rules/development.md` § "Check-and-act must be atomic (no TOCTOU)").
/// A dump OR add failure surfaces as [`InterceptError::IpRuleAddFailed`]
/// carrying the op-specific [`NetlinkError`].
pub(crate) fn ensure_fwmark_rule() -> Result<()> {
    block_on_host_netlink(|| async {
        let client = Client::new()?;
        if !client.fib_rule_fwmark_present(TPROXY_FWMARK, TPROXY_RT_TABLE).await? {
            client.add_fib_rule_fwmark(TPROXY_FWMARK, TPROXY_RT_TABLE).await?;
        }
        Ok(())
    })
    .map_err(|source| InterceptError::IpRuleAddFailed { source })
}

/// Ensure the shared `local 0.0.0.0/0 dev lo table <TPROXY_RT_TABLE>` route
/// exists, via rtnetlink (`RTM_NEWROUTE`, kind Local / scope Host). `-EEXIST`
/// (already converged — the node-global route persists) is idempotent-swallowed
/// via the TYPED errno (ADR-0085 D6), never a "File exists" stderr substring;
/// any other failure surfaces as [`InterceptError::IpRouteLocalAddFailed`].
pub(crate) fn ensure_local_route() -> Result<()> {
    match block_on_host_netlink(|| async {
        Client::new()?.add_local_route(TPROXY_RT_TABLE, "lo").await
    }) {
        Ok(()) => Ok(()),
        Err(err) if errno_is_idempotent(err.errno()) => Ok(()),
        Err(source) => Err(InterceptError::IpRouteLocalAddFailed { source }),
    }
}

/// Read the two node-wide policy-route objects without mutating routing state.
/// The error variants match the existing ownership of the fwmark rule and
/// local route operations.
pub(crate) fn observe_shared_policy_route() -> Result<bool> {
    let rule_present = block_on_host_netlink(|| async {
        Client::new()?.fib_rule_fwmark_present(TPROXY_FWMARK, TPROXY_RT_TABLE).await
    })
    .map_err(|source| InterceptError::IpRuleAddFailed { source })?;
    let route_present = block_on_host_netlink(|| async {
        Client::new()?.local_route_present(TPROXY_RT_TABLE, "lo").await
    })
    .map_err(|source| InterceptError::IpRouteLocalAddFailed { source })?;
    Ok(rule_present && route_present)
}

// ---- sync → async netlink bridge (ADR-0085 D5) ------------------------------
//
// The shared policy-route surface is SYNC, so it cannot `block_on` rtnetlink
// on the calling thread — that panics with "runtime within a runtime" when the
// caller is already on a tokio worker. Every netlink op therefore runs on a
// DEDICATED throwaway `std::thread` via `overdrive_netlink::block_on_host_netlink`
// — the single auditable home for the bridge, shared verbatim with
// `veth_provisioner` (it builds the closure's own current-thread runtime on a
// host-netns thread that is never a pooled tokio worker).

/// Build a `libc::sockaddr_in` from a [`SocketAddrV4`] (host→network byte
/// order for the port; native bytes for the address). Mirrors
/// `roles.rs::sockaddr_in_from`.
const fn sockaddr_in_from(addr: SocketAddrV4) -> libc::sockaddr_in {
    // SAFETY: zeroed sockaddr_in is a valid all-fields-zero value we then
    // populate.
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    sa.sin_family = libc::AF_INET as libc::sa_family_t;
    sa.sin_port = addr.port().to_be();
    sa.sin_addr.s_addr = u32::from_ne_bytes(addr.ip().octets());
    sa
}
