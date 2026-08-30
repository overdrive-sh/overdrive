//! The worker's intercept-install + leg-acquire role (composition-root side
//! of SD-1(a), D-MTLS-14).
//!
//! Productionises the proven 01-01 test-harness
//! primitives (`mtls_roles.rs` / `mtls_netns_topology.rs::install_tproxy`)
//! into the four free functions + one RAII guard + one typed error that
//! produce the [`InterceptedConnection`] which `HostMtlsEnforcement::enforce`
//! consumes.
//!
//! This is NOT adapter API — the [`MtlsEnforcement`](overdrive_core::traits::mtls_enforcement::MtlsEnforcement)
//! trait is unchanged (4 methods: `probe`/`enforce`/`liveness`/`teardown`).
//! These are composition-root worker free functions: the worker's
//! `on_alloc_running` lifecycle (06-03) drives them to acquire a leg and
//! hand the resulting [`InterceptedConnection`] to `enforce`.
//!
//! Synchronous by design (blocking `std::net::TcpListener` accept) — leg
//! acquisition is a one-shot per intercepted connection, not an async pump.
//!
//! # Production-half vs GAP-3 (test-only) boundary
//!
//! [`install_inbound_tproxy`] productionises ONLY the TPROXY-prerouting +
//! `ip rule fwmark` + `ip route local … table` half of the harness
//! `install_tproxy`. The harness ALSO installs a GAP-3 leg-S DNAT /
//! masquerade hop (`nat OUTPUT` DNAT + `127.0.0.0/8` route off `lo` +
//! `rp_filter` relax) that fakes a distinct server-real-listener hop for the
//! netns test topology — that is TEST-ONLY and does NOT productionise. The
//! production adapter dials orig-dst verbatim (`server_dial_addr` in
//! `mtls/inbound.rs`, #241-deferred — NOT touched here).

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "raw libc syscall glue: struct-size -> socklen_t (compile-time constant) and AF_INET -> sa_family_t casts are FFI-width conversions on bounded values; cannot truncate or wrap. Mirrors the module-level allow on the sibling overdrive_dataplane::mtls adapter."
)]

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd, RawFd};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use overdrive_core::AllocationId;
use overdrive_core::traits::mtls_enforcement::{InterceptedConnection, Routed};
use overdrive_netlink::nft::{self, BaseChainSpec, ChainKind};
use overdrive_netlink::{Client, block_on_host_netlink, errno_is_idempotent};
// Re-exported: [`NetlinkError`] is already part of this module's public API (it
// is the `#[source]` field type of the decomposed [`InterceptError`] variants
// `NftRuleInstallFailed` / `IpRuleAddFailed` / `IpRouteLocalAddFailed`). The
// re-export gives that type a nameable path for the `overdrive-sim` /
// `overdrive-control-plane` doubles that construct those variants' synthetic
// sources, without those crates taking a direct dependency on the
// `adapter-host` `overdrive-netlink` crate.
pub use overdrive_netlink::NetlinkError;

/// `IP_TRANSPARENT` sockopt level value — libc 0.2 does not name it (same as
/// the proven `roles.rs::make_transparent_listener` reference).
const IP_TRANSPARENT: libc::c_int = 19;

/// `IP_FREEBIND` sockopt level value — lets the leg-C listener bind the
/// NON-LOCAL `workload_addr` (∈ 10.99.0.0/16, not assigned to the host) on the
/// OUTPUT path, so the agent's host-originated leg-B re-dial is intercepted
/// symmetric with the prerouting path. libc 0.2 does not name it. (REV-5)
const IP_FREEBIND: libc::c_int = 15;

/// The stable production nft table name for the inbound TPROXY intercept.
///
/// This table + its `prerouting` chain are SHARED node-global converge-on-boot
/// infrastructure (kernel-canonical TPROXY / Cilium host-netns model — research
/// `multi-workload-tproxy-interception-resource-model-research.md` F1/F5/F6/F7).
/// The table is ensured idempotently (created-if-missing) and is NEVER torn
/// down per-workload; each `install_inbound_tproxy` APPENDS one per-virt rule to
/// the shared `prerouting` chain, and the guard's `Drop` removes ONLY that one
/// rule by handle. Multiple concurrent inbound intercepts coexist in one chain.
const NFT_TABLE: &str = "overdrive-mtls";

/// The shared `prerouting` chain inside [`NFT_TABLE`] that holds the F5
/// leg-S-dial exemption (once, at the head) followed by every per-virt TPROXY
/// rule.
const NFT_CHAIN: &str = "prerouting";

/// The shared `output` chain inside [`NFT_TABLE`] that holds the leg-S-dial
/// exemption (once, at the head) followed by every per-virt OUTPUT divert rule.
/// Distinct from [`NFT_CHAIN`] (`prerouting`): the output chain MUST be
/// `type route hook output priority mangle` (NOT `type filter`) so the kernel
/// RE-EVALUATES the route after `meta mark set`, firing the existing
/// `ip rule fwmark` → `local` route on the OUTPUT path (spike-proven; the
/// `type filter` counter-test lands on the plaintext decoy). This diverts the
/// agent's host-originated leg-B re-dial (to a backend `workload_addr` whose
/// resolved frontend `F` ≠ that addr — the mesh→mesh hop) into the destination's
/// leg-C, symmetric with how the prerouting `tproxy` rule diverts a peer's SYN.
/// (REV-5; spike `findings-output-hook-legb.md`.)
const NFT_OUTPUT_CHAIN: &str = "output";

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

/// The agent's loopback redirect target — every TPROXY rule diverts to
/// `127.0.0.1:<leg port>` (the `IP_TRANSPARENT` leg-C / leg-F listener). The
/// hand-rolled `tproxy` expression loads this as `NFTA_TPROXY_REG_ADDR`
/// (network-order octets), per the `spike/findings-e.md` pin.
const AGENT_LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;

/// Typed error surface for the worker's intercept-install + leg-acquire role.
///
/// Distinct variant per failure mode (`.claude/rules/development.md`
/// § Errors): a transparent-listener setup failure, a TPROXY-install
/// failure, a leg-accept failure, and an orig-dst recovery failure each name
/// their own cause so the caller (and operator) gets cause-specific
/// diagnostics.
#[derive(Debug, thiserror::Error)]
pub enum InterceptError {
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
    /// The just-appended rule's kernel handle could not be recovered from the
    /// `GETRULE` reply — a STRUCTURAL/parse failure, not an errno (the dump
    /// carried no rule matching this virt's userdata tag), so the by-handle
    /// teardown has nothing to record (ADR-0085 D3/D10). `context` names the
    /// rule that could not be recovered.
    #[error("nft rule handle recovery failed: {context}")]
    NftHandleRecoveryFailed {
        /// The rule (virt / host-veth + redirect) whose handle was not found.
        context: String,
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
    /// `nft list chain` reported the shared table/chain absent — a benign
    /// "nothing installed yet / nothing to sweep" signal on a fresh boot,
    /// distinct from a genuine `nft` failure (binary missing, EPERM, transient
    /// lock). Callers that treat absence as empty (the §5 boot sweep) map this
    /// to a no-op; callers that require the chain propagate it.
    #[error("the shared nft table/chain does not exist (nothing to sweep)")]
    ChainAbsent,
    /// `accept_inbound_leg` / `accept_outbound_and_recover_orig_dst` could not
    /// accept the redirected connection on the intercept listener.
    #[error("leg accept failed on the {direction} intercept listener: {source}")]
    Accept {
        /// `"inbound"` or `"outbound"` — which intercept listener accept failed on.
        direction: &'static str,
        /// The originating accept error.
        #[source]
        source: std::io::Error,
    },
    /// `accept_inbound_leg` (inbound orig-dst) or
    /// `accept_outbound_and_recover_orig_dst` (outbound orig-dst recovery) could
    /// not recover the original destination via `getsockname` on the
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
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
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

/// Install the inbound nft intercept for ONE `virt` — the `prerouting`
/// `tproxy` rule PLUS the REV-5 `output` divert companion.
///
/// Appends TWO per-virt rules: one `tproxy` rule to the SHARED `prerouting`
/// chain (diverts a PEER's inbound SYN to `virt` into the agent's leg-C) and one
/// `meta mark set` divert rule to the SHARED `output` chain (REV-5 — diverts the
/// AGENT's host-originated leg-B re-dial to `virt`, the mesh→mesh hop where the
/// resolved frontend `F` ≠ this backend `workload_addr`, into the SAME leg-C).
/// The returned guard's `Drop` removes ONLY those two rules by their
/// `(chain, handle)` pairs. The shared routing infrastructure — the `ip rule fwmark` policy
/// rule, the `ip route local … table` loopback route, the nft table + chain,
/// and the F5 `MTLS_LEG_S_DIAL_MARK` exemption at the chain head — is
/// node-global converge-on-boot state ensured idempotently here (created
/// once, NEVER torn down per-workload) so multiple concurrent inbound
/// intercepts coexist without razing one another (kernel-canonical TPROXY /
/// Cilium host-netns model — research
/// `multi-workload-tproxy-interception-resource-model-research.md` F1/F5/F6/F7;
/// converge-on-boot Bar-1 per `.claude/rules/reconcilers.md`; the Bar-2
/// ref-counted host-infra reconciler promotion — only if runtime drift of the
/// shared rule enters the threat model — is tracked at
/// [#234](https://github.com/overdrive-sh/overdrive/issues/234), a sibling of
/// the #197/#198/#199 family).
///
/// Redirects a connection aimed at `virt` to the agent's leg-C listener on
/// `agent_port`. The `MTLS_LEG_S_DIAL_MARK` exemption is ordered FIRST in the
/// chain (F5 inbound) so the agent's own marked leg-S dial is accepted before
/// any per-virt TPROXY rule can match it (otherwise the dial recurses back
/// onto leg C).
///
/// Productionises the PRODUCTION HALF of
/// `mtls_netns_topology.rs::install_tproxy` ONLY — the GAP-3 netns
/// DNAT/masquerade is TEST-ONLY and does NOT productionise (the adapter dials
/// orig-dst verbatim, #241).
///
/// # Errors
///
/// Returns [`InterceptError::NftRuleInstallFailed`] if ensuring the shared infra
/// (`ip rule`, `ip route`, nft table/chain/exemption) fails, or if appending a
/// per-virt rule fails; [`InterceptError::NftHandleRecoveryFailed`] if an
/// appended rule's kernel handle cannot be recovered from the `GETRULE` reply;
/// or [`InterceptError::IpRuleAddFailed`] / [`InterceptError::IpRouteLocalAddFailed`]
/// from the shared ip-side infra.
pub fn install_inbound_tproxy(virt: SocketAddrV4, agent_port: u16) -> Result<TproxyInterceptGuard> {
    // (1) Ensure the SHARED, node-global routing infra idempotently. These are
    // add-if-missing converges (NOT a destructive preclean): a pre-existing
    // shared rule/route/table is the success case, left untouched. None of
    // these is removed on per-workload Drop.
    ensure_shared_routing_infra()?;

    let vip = *virt.ip();
    let vport = virt.port();
    let leg_s_mark = overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;

    // (2) Append exactly ONE per-virt TPROXY rule to the shared `prerouting`
    // chain (the spike-e-proven wire bytes) after the F5 exemption, tagged with
    // its structural NFTA_RULE_USERDATA identity for by-handle recovery. TPROXY
    // preserves daddr, so the agent recovers orig-dst per-flow via getsockname
    // and a single shared fwmark routes every virt.
    let inbound_tag = nft::userdata_inbound(vip, vport, agent_port);
    nft::append_rule(
        NFT_TABLE,
        NFT_CHAIN,
        &nft::inbound_tproxy_rule_exprs(vip, vport, AGENT_LOOPBACK, agent_port, TPROXY_FWMARK),
        &inbound_tag,
    )
    .map_err(|source| InterceptError::NftRuleInstallFailed { op: "append-inbound", source })?;

    // (3) REV-5 — append the companion OUTPUT divert rule for this virt to the
    // shared `output` chain (after its head leg-S exemption). This diverts the
    // agent's host-originated leg-B re-dial to `virt` (the mesh→mesh hop, where
    // the resolved frontend `F` ≠ this backend `workload_addr`) into the
    // destination's leg-C, symmetric with how the prerouting `tproxy` rule above
    // diverts a peer's SYN. The `meta mark != <MTLS_LEG_S_DIAL_MARK>` clause
    // skips the agent's marked leg-S inbound dial (which must reach the workload
    // directly) and matches only the un-marked leg-B re-dial — the recursion
    // guard the prerouting path already relies on. `type route hook output`
    // re-evaluates the route after `meta mark set <TPROXY_FWMARK>`, firing the
    // existing `ip rule fwmark` → `local table 100` route on the output path; the
    // leg-C `IP_FREEBIND` listener (set in `make_transparent_listener`) binds the
    // non-local `virt.ip()` so `getsockname` recovers orig-dst verbatim.
    let output_tag = nft::userdata_output_divert(vip, vport);
    nft::append_rule(
        NFT_TABLE,
        NFT_OUTPUT_CHAIN,
        &nft::output_divert_rule_exprs(vip, vport, leg_s_mark, TPROXY_FWMARK),
        &output_tag,
    )
    .map_err(|source| InterceptError::NftRuleInstallFailed {
        op: "append-output-divert",
        source,
    })?;

    // PARTIAL-INSTALL POSTURE (REV-5 dual-append, N1): the two appends above
    // (and the two handle recoveries below) are committed to the kernel BEFORE
    // the `TproxyInterceptGuard` is constructed, so any `?` from here back to the
    // first append (3)/(2) returns early with the rule(s) already in the chain
    // and no guard to remove them. This is the codebase's accepted
    // converge-on-boot posture, NOT an oversight: the §5 boot-recovery sweep
    // (`sweep_per_workload_tproxy_rules` → `sweep_one_chain` over BOTH the
    // prerouting and output chains) reaps any such orphan on the next
    // control-plane restart (fail-closed; #234). `nft` failing mid-sequence is
    // rare (EPERM / lock / missing binary), so within a single boot the bounded
    // leak is tolerated rather than RAII-unwound here.
    //
    // (4) Recover the kernel-assigned handle of EACH rule we just appended
    // STRUCTURALLY from the `GETRULE` reply (NFTA_RULE_USERDATA -> NFTA_RULE_HANDLE),
    // so Drop can delete EXACTLY those two rules (siblings, the exemptions, and
    // the shared infra all untouched) — the nft-canonical per-rule teardown. Each
    // rule is recovered by the exact userdata identity tag it was appended with,
    // so two installs for distinct virts capture distinct handles.
    let prerouting_handle = recover_rule_handle(NFT_CHAIN, &inbound_tag, || {
        format!("inbound tproxy virt {vip}:{vport} -> 127.0.0.1:{agent_port}")
    })?;
    let output_handle = recover_rule_handle(NFT_OUTPUT_CHAIN, &output_tag, || {
        format!("output divert virt {vip}:{vport}")
    })?;
    Ok(TproxyInterceptGuard::acquire([
        (NFT_CHAIN, prerouting_handle),
        (NFT_OUTPUT_CHAIN, output_handle),
    ]))
}

/// Install the OUTBOUND nft-TPROXY prerouting intercept for one workload's
/// host-side veth.
///
/// The active-side mirror of [`install_inbound_tproxy`] (ADR-0071 Path A
/// unifies inbound + outbound on the ONE nft-TPROXY mechanism). Where the
/// inbound rule matches a specific *destination* (`ip daddr <vip>` +
/// `tcp dport <vport>`), the egress rule matches the *ingress interface* —
/// `iifname <host_veth>` — capturing ALL of the workload's outbound TCP as it
/// ingresses the per-workload host-side veth, and TPROXY-redirecting it to the
/// agent's leg-F `IP_TRANSPARENT` listener on `agent_leg_f_port`. There is no
/// per-destination match because the workload's destination is unknown at
/// install time; TPROXY preserves the original destination, which the agent
/// recovers per-flow via `getsockname` downstream (03-02). This is the
/// production shape per the feature-delta / ADR-0071 fact 2 (*"OUTBOUND
/// interception = nft-TPROXY at the host-side veth"* — the active-side mirror
/// of inbound) — NOT the single-known-backend `ip daddr/tcp dport` shape the
/// egress spike used. The spike proved the routing MECHANISM (PREROUTING on
/// host-veth ingress + fwmark + `ip rule` + local route + `IP_TRANSPARENT`
/// leg-F + `getsockname` recovery), not the `iifname`-match clause literally;
/// the real-kernel fire of the iifname clause is the Tier-3 03-01→03-03
/// obligation (roadmap criterion 5).
///
/// Like the inbound install, this APPENDS exactly one rule to the SHARED
/// `prerouting` chain (after the F5 exemption) and returns a
/// [`TproxyInterceptGuard`]. The final guard sharing that rule's ownership
/// removes ONLY that one rule by its kernel-assigned handle; the node-global shared routing infra
/// ([`ensure_shared_routing_infra`]) is ensured idempotently and never torn
/// down per-workload.
///
/// # Idempotency
///
/// The egress rule is keyed on `(host_veth, agent_leg_f_port)` — both the
/// ingress interface AND the leg-F redirect target — because the egress rule
/// has no unique `ip daddr`/`tcp dport` of its own to distinguish it. Before
/// appending, the shared chain is presence-checked for an existing egress rule
/// matching THIS exact `(host_veth, agent_leg_f_port)`; only when such a rule
/// is already present is the append skipped and a guard for the existing
/// rule's handle returned. On the normal teardown path the final shared
/// [`TproxyInterceptGuard`] owner removes the rule by handle, so the next
/// install for that veth starts from a clean chain. Dropping an earlier owner
/// leaves the adopted rule live. (The inbound install does not need this
/// presence-check — distinct virts produce distinct rule text.)
///
/// # Caller contract — leg-F port is part of the key
///
/// Because `agent_leg_f_port` is part of the dedup key, the skip fires only for
/// the same `(host_veth, port)` pair. leg-F binds a worker-chosen *ephemeral*
/// port per alloc (`mtls_intercept_worker.rs` `leg_f_addr`), so it is NOT
/// node-stable across re-binds. A caller that re-installs a `host_veth` whose
/// PRIOR egress rule SURVIVED in the kernel — e.g. a control-plane restart that
/// left the kernel rule but dropped the in-memory guard, the surviving-veth
/// re-install at `start_alloc` (04-01) / adopt-on-restart (02-06) — with a
/// DIFFERENT leg-F port will NOT match the old `(veth, oldPort)` rule and WILL
/// append a second rule. Such a caller MUST remove the prior rule first (or pin
/// a stable-per-veth leg-F port) before re-installing.
///
/// # Errors
///
/// Returns [`InterceptError::NftRuleInstallFailed`] if ensuring the shared infra
/// fails or if appending the egress rule fails;
/// [`InterceptError::NftHandleRecoveryFailed`] if the appended rule's kernel
/// handle cannot be recovered from the `GETRULE` reply.
pub fn install_outbound_tproxy(
    host_veth: &str,
    agent_leg_f_port: u16,
) -> Result<TproxyInterceptGuard> {
    // (1) Ensure the SHARED, node-global routing infra idempotently — exactly
    // as the inbound install does. Add-if-missing converges; a pre-existing
    // shared rule/route/table/exemption is the success case, left untouched.
    ensure_shared_routing_infra()?;

    // (2) Idempotent append: the egress rule is keyed on
    // `(host_veth, agent_leg_f_port)` — both the ingress interface AND the
    // leg-F redirect target — since it has no unique daddr/dport. If a rule for
    // THIS exact `(host_veth, agent_leg_f_port)` is already in the shared chain,
    // recover and return a guard for the EXISTING rule's handle instead of
    // appending a second copy. (A surviving rule for the same veth but a
    // DIFFERENT leg-F port is NOT matched here — see the "Caller contract" in
    // the rustdoc above.)
    let egress_tag = nft::userdata_egress(host_veth, agent_leg_f_port);
    let egress_program =
        nft::egress_tproxy_rule_exprs(host_veth, AGENT_LOOPBACK, agent_leg_f_port, TPROXY_FWMARK);
    let normalized_egress_program = nft::normalized_rule_program_identity(&egress_program)
        .map_err(|source| InterceptError::NftHandleRecoveryFailed {
            context: format!("normalize egress production program: {source}"),
        })?;
    let rules = nft::list_rules(NFT_TABLE, NFT_CHAIN)
        .map_err(|source| InterceptError::NftRuleInstallFailed { op: "list-rules", source })?;
    let matching = rules.iter().filter(|rule| rule.userdata == egress_tag).collect::<Vec<_>>();
    match matching.as_slice() {
        [existing]
            if existing.counter.is_some()
                && existing.normalized_program == normalized_egress_program =>
        {
            return Ok(TproxyInterceptGuard::acquire([(NFT_CHAIN, existing.handle)]));
        }
        [] => {}
        _ => {
            return Err(InterceptError::NftHandleRecoveryFailed {
                context: format!(
                    "egress host_veth {host_veth} -> 127.0.0.1:{agent_leg_f_port} has ambiguous or non-production existing rule identity"
                ),
            });
        }
    }

    // (3) Append exactly ONE egress rule to the shared chain, after the F5
    // exemption. Match on the ingress interface (`iifname <host_veth>`) +
    // `meta l4proto tcp`; redirect ALL the workload's egress TCP to leg F.
    // TPROXY preserves orig-dst → recovered per-flow downstream (03-02), so a
    // single shared fwmark routes every flow (same as inbound).
    nft::append_rule(NFT_TABLE, NFT_CHAIN, &egress_program, &egress_tag)
        .map_err(|source| InterceptError::NftRuleInstallFailed { op: "append-egress", source })?;

    // (4) Recover the kernel-assigned handle of the rule we just appended
    // structurally (NFTA_RULE_USERDATA -> NFTA_RULE_HANDLE), so Drop can delete
    // EXACTLY that rule (siblings, the exemption, and the shared infra all
    // untouched).
    let handle = recover_rule_handle(NFT_CHAIN, &egress_tag, || {
        format!("egress host_veth {host_veth} -> 127.0.0.1:{agent_leg_f_port}")
    })?;
    // The egress install creates ONE rule in the prerouting chain and NO output
    // companion (the output-hook divert is inbound-only — it intercepts the
    // agent's host-originated leg-B re-dial TO a backend, REV-5).
    Ok(TproxyInterceptGuard::acquire([(NFT_CHAIN, handle)]))
}

/// Boot-recovery sweep (adopt-on-restart §5, D-TME-12; folds 03-01 finding D2).
///
/// Removes EVERY per-workload rule — egress (`iifname`-matched) AND inbound
/// (`ip daddr`/`tcp dport`-matched) `tproxy` rules from the `prerouting` chain,
/// AND the REV-5 `output` divert rules from the `output` chain — by handle,
/// leaving the shared infra of BOTH chains (the leg-S `meta mark
/// <MTLS_LEG_S_DIAL_MARK> accept` exemptions, the table+chains, the chain
/// policy/type/hook lines) UNTOUCHED — so a subsequent per-alloc re-install
/// appends exactly one clean rule per direction per chain. Per-workload rules
/// are recognised STRUCTURALLY by their `NFTA_RULE_USERDATA` kind discriminator
/// (`overdrive_netlink::nft::workload_rule_handles`), which covers the output
/// divert rule (no `tproxy` verb) as well as the prerouting `tproxy` rules;
/// missing the output chain would leak the divert rule across every restart (the
/// D2 class, reopened — REV-5).
///
/// # Why a sweep (not an adopt)
///
/// On a `serve` restart each per-workload rule SURVIVES in the shared chain
/// (it is appended once and NEVER torn down per-workload — [`NFT_TABLE`]
/// rustdoc), but its in-RAM RAII [`TproxyInterceptGuard`] is LOST (the CP died;
/// `Drop` never ran). The surviving rule redirects to a now-dead leg-C/leg-F
/// listener port → DEAD weight; a later re-install with a NEW ephemeral port
/// does NOT match the stale `(veth, oldPort)` rule and would APPEND A SECOND
/// rule (duplicate-stack, finding D2). Unlike the surviving netns (which the
/// boot pass ADOPTS, because the workload still lives in it), the surviving
/// rule has nothing to preserve — it points at a dead listener — so the boot
/// pass REAPS it. The clean re-install at `start_alloc` restores a correct
/// rule. (Scope: this is CLEANUP only — it does NOT re-bind legs, re-spawn
/// listeners, or re-install rules to "restore" a survivor's interception; a
/// still-Running survivor legitimately ends with no rule until reschedule,
/// the accepted #26-coupled limitation.)
///
/// # Idempotency
///
/// A no-op (returns `Ok(0)`) when the chain carries only shared infra. Safe to
/// run on every boot.
///
/// # Errors
///
/// Fail-CLOSED on every genuine failure (matching the by-handle delete path):
/// the ONLY swallowed case is the shared table/chain being absent (a GETCHAIN
/// -ENOENT structural read, per [`sweep_one_chain`]) which maps to `Ok(0)` — the
/// benign "nothing to sweep" signal on a fresh boot. A genuine netlink failure
/// (EPERM, transient lock) surfaces as [`InterceptError::NftRuleInstallFailed`]
/// and refuses the boot, as does a by-handle DELRULE failure.
// mutants: skip — thin nft-I/O shim (`nft::chain_exists` + `nft::list_rules` +
// by-handle `nft::delete_rule`); the pure decision is
// `overdrive_netlink::nft::workload_rule_handles` (unit + mutation covered in
// overdrive-netlink). Body-replacement mutants (`Ok(0)`/`Ok(1)`) are killable
// only by the real-kernel Tier-3 AT
// `serve_restart_sweeps_surviving_per_workload_tproxy_rule` (overdrive-control-plane),
// which the worker-package default-lane mutants suite cannot run.
pub fn sweep_per_workload_tproxy_rules() -> Result<usize> {
    // REV-5: sweep BOTH the `prerouting` chain (egress + inbound `tproxy` rules)
    // AND the `output` chain (the leg-B re-dial divert rules). Each chain may be
    // absent on a fresh boot — that benign case maps to 0 for that chain, while a
    // genuine `nft` failure still propagates and refuses the boot. Summing the
    // two counts gives the total swept; if `output` was never created (a node
    // that ran a pre-REV-5 binary, or a boot before any inbound install), its
    // sweep is a clean no-op.
    let prerouting = sweep_one_chain(NFT_CHAIN)?;
    let output = sweep_one_chain(NFT_OUTPUT_CHAIN)?;
    Ok(prerouting + output)
}

/// RAII owner for boot-recovery fail-closed rules.
///
/// Success drops this guard only after every replacement intercept is live; a
/// failed boot intentionally retains it in the kernel.
pub struct RecoveryQuarantine {
    guard: TproxyInterceptGuard,
}

impl RecoveryQuarantine {
    /// Leave the DROP rules in the kernel while relinquishing this process's
    /// RAII ownership. A later boot can structurally adopt and release them;
    /// leaking a process-local guard would instead pin them forever when a
    /// failed server start is retried in the same process.
    pub fn retain_in_kernel(self) {
        self.guard.disarm();
    }
}

/// One complete boot-recovery protection batch.
///
/// Dropping an unreleased batch deliberately leaves its DROP rules in the
/// kernel so every `?`/early return after the sweep is fail-closed. Only
/// [`Self::release`] removes the batch, after the composition root has crossed
/// every fallible readiness gate and can return the server owner.
pub struct RecoveryQuarantineBatch {
    quarantines: Option<Vec<RecoveryQuarantine>>,
}

impl RecoveryQuarantineBatch {
    /// Own a validated set of survivor quarantine rules.
    #[must_use]
    pub const fn new(quarantines: Vec<RecoveryQuarantine>) -> Self {
        Self { quarantines: Some(quarantines) }
    }

    /// Atomically remove the whole batch after protected readiness.
    pub fn release(mut self) -> Result<()> {
        release_recovery_quarantines(
            self.quarantines
                .take()
                .unwrap_or_else(|| unreachable!("recovery batch releases only once")),
        )
    }
}

impl Drop for RecoveryQuarantineBatch {
    fn drop(&mut self) {
        if let Some(quarantines) = self.quarantines.take() {
            retain_recovery_quarantines(quarantines);
        }
    }
}

/// Relinquish a failed recovery batch without deleting its fail-closed rules.
pub fn retain_recovery_quarantines(quarantines: Vec<RecoveryQuarantine>) {
    for quarantine in quarantines {
        quarantine.retain_in_kernel();
    }
}

/// Atomically expose a completely rebuilt survivor batch.
///
/// Every quarantine rule is deleted in one nftables transaction. If any
/// delete fails, the kernel rolls the whole batch back and the guards are
/// disarmed so the DROP rules remain structurally adoptable by the next boot.
pub fn release_recovery_quarantines(quarantines: Vec<RecoveryQuarantine>) -> Result<()> {
    let mut rule_keys =
        quarantines.iter().flat_map(|quarantine| quarantine.guard.rule_keys()).collect::<Vec<_>>();
    rule_keys.sort_unstable();
    rule_keys.dedup();
    if !rule_keys.is_empty() {
        let mutations = rule_keys
            .iter()
            .map(|(chain, handle)| nft::AtomicRuleMutation::Delete {
                table: NFT_TABLE,
                chain,
                handle: *handle,
            })
            .collect::<Vec<_>>();
        if let Err(source) = nft::apply_rule_transaction_atomically(&mutations) {
            retain_recovery_quarantines(quarantines);
            return Err(InterceptError::NftRuleInstallFailed {
                op: "release-recovery-quarantine",
                source,
            });
        }
    }
    for quarantine in &quarantines {
        quarantine.guard.disarm();
    }
    drop(quarantines);
    Ok(())
}

/// Install or adopt stable fail-closed rules for one surviving allocation.
///
/// Quarantine rules use a distinct userdata kind, so the dead-redirect sweep
/// cannot delete them. Existing old intercept rules run first until swept;
/// after the sweep the quarantine becomes the active drop boundary. New
/// intercept rules are appended behind it and become reachable only when the
/// returned guard is dropped after the full recovery batch succeeds.
pub fn install_recovery_quarantine(
    host_veth: Option<&str>,
    workload_addr: Option<Ipv4Addr>,
    service_ports: &[std::num::NonZeroU16],
) -> Result<RecoveryQuarantine> {
    ensure_shared_routing_infra()?;
    let mut rules = Vec::new();
    if let Some(host_veth) = host_veth {
        let mut key = Vec::with_capacity(1 + host_veth.len());
        key.push(b'e');
        key.extend_from_slice(host_veth.as_bytes());
        rules.push((
            NFT_CHAIN,
            install_or_adopt_quarantine(
                NFT_CHAIN,
                &nft::egress_quarantine_rule_exprs(host_veth),
                &nft::userdata_recovery_quarantine(&key),
            )?,
        ));
    }
    if let Some(addr) = workload_addr {
        for port in service_ports {
            let mut key = Vec::with_capacity(7);
            key.push(b'p');
            key.extend_from_slice(&addr.octets());
            key.extend_from_slice(&port.get().to_be_bytes());
            let tag = nft::userdata_recovery_quarantine(&key);
            let exprs = nft::inbound_quarantine_rule_exprs(addr, port.get());
            rules.push((NFT_CHAIN, install_or_adopt_quarantine(NFT_CHAIN, &exprs, &tag)?));
            key[0] = b'o';
            rules.push((
                NFT_OUTPUT_CHAIN,
                install_or_adopt_quarantine(
                    NFT_OUTPUT_CHAIN,
                    &exprs,
                    &nft::userdata_recovery_quarantine(&key),
                )?,
            ));
        }
    }
    Ok(RecoveryQuarantine { guard: TproxyInterceptGuard::acquire_vec(rules) })
}

fn install_or_adopt_quarantine(chain: &'static str, exprs: &[u8], tag: &[u8]) -> Result<u64> {
    let rules = nft::list_rules(NFT_TABLE, chain)
        .map_err(|source| InterceptError::NftRuleInstallFailed { op: "list-quarantine", source })?;
    if let Some(handle) = nft::handle_for_userdata(&rules, tag) {
        return Ok(handle);
    }
    nft::append_rule(NFT_TABLE, chain, exprs, tag).map_err(|source| {
        InterceptError::NftRuleInstallFailed { op: "append-recovery-quarantine", source }
    })?;
    recover_rule_handle(chain, tag, || "boot recovery quarantine".to_owned())
}

/// Sweep every per-workload rule out of ONE named chain by handle, returning the
/// count removed. An absent chain (a GETCHAIN -ENOENT structural read, ADR-0085
/// D10) is the benign fresh-boot "nothing to sweep" signal mapped to `Ok(0)`;
/// every genuine netlink failure propagates and refuses the boot (fail-CLOSED,
/// matching the by-handle delete path).
///
/// # Why fail-closed on a list/delete error
///
/// A still-Running survivor does NOT trigger a `start_alloc` (SPIKE-B — the
/// reconciler does not re-drive survivors), so there is no downstream install to
/// catch a stranded guard-less survivor rule (the D2 dead-weight §5 exists to
/// reap) if the list fails — fail-closed is the only posture that does not leave
/// it stranded.
// mutants: skip — thin nft-I/O shim (`nft::chain_exists` + `nft::list_rules` +
// by-handle `nft::delete_rule`); the pure decision is
// `overdrive_netlink::nft::workload_rule_handles` (unit + mutation covered in
// overdrive-netlink). Body-replacement mutants (`Ok(0)`/`Ok(1)`) are killable
// only by the real-kernel Tier-3 AT
// `serve_restart_sweeps_surviving_per_workload_tproxy_rule`
// (overdrive-control-plane), which the worker-package default-lane mutants
// suite cannot run. DOCUMENTATION ONLY — the actual suppression is the
// `replace sweep_one_chain -> Result<usize> with Ok` exclude_re entry in
// `.cargo/mutants.toml` (a bare comment suppresses nothing per testing.md).
fn sweep_one_chain(chain: &str) -> Result<usize> {
    // Absent chain (fresh boot, no mTLS workload has installed a rule) is the
    // benign "nothing to sweep" signal, detected STRUCTURALLY via a GETCHAIN
    // -ENOENT read (ADR-0085 D10), NOT an `nft` stderr substring. A genuine
    // netlink failure propagates and refuses the boot.
    if !nft::chain_exists(NFT_TABLE, chain)
        .map_err(|source| InterceptError::NftRuleInstallFailed { op: "chain-exists", source })?
    {
        return Ok(0);
    }

    // Classify (structural): collect the handle of every per-workload rule from
    // the GETRULE reply via its NFTA_RULE_USERDATA kind discriminator, leaving
    // the shared infra (chain header / type-policy line / leg-S exemption)
    // untouched. Port-blind: a restart lost the dead redirect ports, so the
    // classify keys on the rule KIND, never a port.
    let rules = nft::list_rules(NFT_TABLE, chain)
        .map_err(|source| InterceptError::NftRuleInstallFailed { op: "list-rules", source })?;
    let handles = nft::workload_rule_handles(&rules);

    // Delete each by handle — the SAME by-handle DELRULE the guard's `Drop` uses.
    // A delete failure (a real netlink error, not an absent rule) refuses the
    // boot: surface it as `NftRuleInstallFailed`.
    for handle in &handles {
        nft::delete_rule(NFT_TABLE, chain, *handle)
            .map_err(|source| InterceptError::NftRuleInstallFailed { op: "delete-rule", source })?;
    }
    Ok(handles.len())
}

/// Ensure the SHARED node-global TPROXY routing infrastructure exists,
/// idempotently (add-if-missing). Converge-on-boot Bar-1: a pre-existing
/// component is the success case, not an error — so two concurrent installs
/// (and a re-install after a prior run) both leave exactly one of each shared
/// resource, never a stacked pile.
///
/// Components (all node-global, none removed on per-workload Drop):
///   - `ip rule fwmark 0x1 lookup 100` — routes fwmark-stamped packets via the
///     local table. **UNCHANGED for the REV-5 output path** — `type route hook
///     output` re-evaluates the route after the output divert's `meta mark set`,
///     so this SAME rule fires on the output path; no `iif lo` clause and no
///     second route table are needed (spike-proven, `findings-output-hook-legb.md`).
///   - `ip route local 0.0.0.0/0 dev lo table 100` — delivers them to a local
///     socket (leg C) instead of forwarding. **UNCHANGED for REV-5** (same
///     route serves prerouting and output paths).
///   - nft table `overdrive-mtls` + `prerouting` chain (`type filter`) +
///     `output` chain (`type route` — REV-5).
///   - the leg-S `meta mark <MTLS_LEG_S_DIAL_MARK> accept` exemption, inserted
///     at EACH chain's HEAD exactly once (must precede all per-virt rules): the
///     `prerouting` head exempts the agent's inbound leg-S dial; the `output`
///     head exempts the agent's marked leg-S dial from the output divert so it
///     reaches the workload directly (REV-5).
fn ensure_shared_routing_infra() -> Result<()> {
    // ip rule (rtnetlink RTM_NEWRULE): add only if not already present, via the
    // PORTED dump-then-add guard. Spike increment-D proved a naked netlink
    // `rule add` STACKS a duplicate (netlink does NOT dedup FIB rules, identical
    // to iproute2 `ip rule add`), so the presence check before the add is
    // load-bearing (ADR-0085 D6). The dump-then-add is one logical check-and-act
    // run on a single dedicated netlink thread so it does not split across a
    // runtime gap (`.claude/rules/development.md` § "Check-and-act must be
    // atomic (no TOCTOU)").
    ensure_fwmark_rule()?;

    // ip route local (rtnetlink RTM_NEWROUTE): `-EEXIST` (already converged) is
    // idempotent-swallowed via the TYPED errno, not a locale-fragile "File
    // exists" stderr substring (ADR-0085 D6).
    ensure_local_route()?;

    // nft table + `prerouting` chain: idempotent create-if-missing NEWTABLE /
    // NEWCHAIN (`-EEXIST` swallowed via the typed errno), so re-running is a
    // no-op. The prerouting chain is `type filter hook prerouting priority
    // mangle` (where TPROXY must live).
    nft::ensure_table(NFT_TABLE)
        .map_err(|source| InterceptError::NftRuleInstallFailed { op: "ensure-table", source })?;
    // Observe-before-create is load-bearing for deterministic incompatibility
    // reporting. NEWCHAIN against an existing chain whose hook differs can
    // itself return EOPNOTSUPP, hiding whether the actual encoded TPROXY rule
    // is supported. A structurally present chain is therefore adopted here;
    // the first incompatible production expression remains the precise
    // append/insert operation surfaced to the caller.
    let prerouting_exists = nft::chain_exists(NFT_TABLE, NFT_CHAIN).map_err(|source| {
        InterceptError::NftRuleInstallFailed { op: "ensure-chain-prerouting", source }
    })?;
    if !prerouting_exists {
        nft::ensure_base_chain(
            NFT_TABLE,
            NFT_CHAIN,
            BaseChainSpec {
                hooknum: nft::NF_INET_PRE_ROUTING,
                priority: nft::PRIORITY_MANGLE,
                kind: ChainKind::Filter,
            },
        )
        .map_err(|source| InterceptError::NftRuleInstallFailed {
            op: "ensure-chain-prerouting",
            source,
        })?;
    }

    // F5 exemption at the prerouting chain head — insert ONCE. `insert_rule`
    // prepends, so guarding against a duplicate keeps it exactly once at the head
    // ahead of every per-virt tproxy rule. Presence is a STRUCTURAL read of the
    // exemption's NFTA_RULE_USERDATA tag from the GETRULE reply (ADR-0085 D10).
    ensure_exemption(NFT_CHAIN)?;

    // REV-5 OUTPUT chain: idempotent create-if-missing. It MUST be
    // `type route hook output priority mangle` (NOT `type filter`) so the kernel
    // RE-EVALUATES the route after a per-virt divert's `meta mark set`, firing
    // the `ip rule fwmark` -> `local table 100` route on the OUTPUT path
    // (spike-proven; the `type filter` counter-test lands on the plaintext
    // decoy).
    let output_exists = nft::chain_exists(NFT_TABLE, NFT_OUTPUT_CHAIN).map_err(|source| {
        InterceptError::NftRuleInstallFailed { op: "ensure-chain-output", source }
    })?;
    if !output_exists {
        nft::ensure_base_chain(
            NFT_TABLE,
            NFT_OUTPUT_CHAIN,
            BaseChainSpec {
                hooknum: nft::NF_INET_LOCAL_OUT,
                priority: nft::PRIORITY_MANGLE,
                kind: ChainKind::Route,
            },
        )
        .map_err(|source| InterceptError::NftRuleInstallFailed {
            op: "ensure-chain-output",
            source,
        })?;
    }

    // leg-S exemption at the OUTPUT chain head — insert ONCE, mirroring the
    // prerouting head. The agent's marked leg-S dial (`SO_MARK 0x2`) must reach
    // the workload directly, not be diverted back into leg-C; the exemption head
    // rule exempts it before any per-virt output divert can match.
    ensure_exemption(NFT_OUTPUT_CHAIN)?;
    Ok(())
}

/// Ensure the shared leg-S `meta mark <MTLS_LEG_S_DIAL_MARK> accept` exemption
/// is present exactly once at the head of `chain`. Presence is a STRUCTURAL read
/// of the exemption rule's `NFTA_RULE_USERDATA` tag from the `GETRULE` reply
/// (ADR-0085 D10) — the typed replacement for the deleted `nft`-text
/// `dump_has_leg_s_exemption` scrape; `insert_rule` prepends so the exemption
/// sits ahead of every per-workload rule.
///
/// # Errors
///
/// [`InterceptError::NftRuleInstallFailed`] on a `GETRULE` dump or `insert`
/// failure.
fn ensure_exemption(chain: &str) -> Result<()> {
    let leg_s_mark = overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
    let rules = nft::list_rules(NFT_TABLE, chain)
        .map_err(|source| InterceptError::NftRuleInstallFailed { op: "list-rules", source })?;
    if !nft::has_exemption(&rules) {
        nft::insert_rule(
            NFT_TABLE,
            chain,
            &nft::mark_accept_exemption_exprs(leg_s_mark),
            &nft::userdata_exemption(),
        )
        .map_err(|source| InterceptError::NftRuleInstallFailed {
            op: "insert-exemption",
            source,
        })?;
    }
    Ok(())
}

/// Recover a just-appended rule's kernel handle STRUCTURALLY from the `GETRULE`
/// reply by its exact `NFTA_RULE_USERDATA` identity tag (ADR-0085 D10) — the
/// typed replacement for the deleted `# handle N` text scrape. Distinct virts /
/// veths carry distinct tags, so the recovered handle is this rule's alone.
///
/// # Errors
///
/// [`InterceptError::NftRuleInstallFailed`] on a `GETRULE` dump failure, or
/// [`InterceptError::NftHandleRecoveryFailed`] (`context` names the rule) when
/// the reply carries no rule with the tag.
fn recover_rule_handle(chain: &str, tag: &[u8], context: impl FnOnce() -> String) -> Result<u64> {
    let rules = nft::list_rules(NFT_TABLE, chain)
        .map_err(|source| InterceptError::NftRuleInstallFailed { op: "list-rules", source })?;
    nft::handle_for_userdata(&rules, tag)
        .ok_or_else(|| InterceptError::NftHandleRecoveryFailed { context: context() })
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
fn ensure_fwmark_rule() -> Result<()> {
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
fn ensure_local_route() -> Result<()> {
    match block_on_host_netlink(|| async {
        Client::new()?.add_local_route(TPROXY_RT_TABLE, "lo").await
    }) {
        Ok(()) => Ok(()),
        Err(err) if errno_is_idempotent(err.errno()) => Ok(()),
        Err(source) => Err(InterceptError::IpRouteLocalAddFailed { source }),
    }
}

// ---- sync → async netlink bridge (ADR-0085 D5) ------------------------------
//
// The `install_*_tproxy` / `ensure_shared_routing_infra` surface is SYNC
// (blocking `std::net::TcpListener` accept), so it cannot `block_on` rtnetlink
// on the calling thread — that panics with "runtime within a runtime" when the
// caller is already on a tokio worker. Every netlink op therefore runs on a
// DEDICATED throwaway `std::thread` via `overdrive_netlink::block_on_host_netlink`
// — the single auditable home for the bridge, shared verbatim with
// `veth_provisioner` (it builds the closure's own current-thread runtime on a
// host-netns thread that is never a pooled tokio worker).

/// RAII guard removing ONLY the per-virt rules THIS install owns on final `Drop`.
///
/// Every guard for the same kernel-assigned `(chain, handle)` shares one
/// process-local ownership token. Dropping one adopted guard leaves the rule
/// live; dropping the final token deletes it exactly once. The shared
/// routing infra — `ip rule`, `ip route`, nft table/chains, and the F5
/// exemptions — is node-global and is NOT removed here; sibling intercepts'
/// rules are untouched.
///
/// An INBOUND install (`install_inbound_tproxy`) now creates TWO per-virt rules
/// — the `prerouting` `tproxy` rule AND the `output` `meta mark set` divert rule
/// (REV-5, the leg-B re-dial interception companion) — so the guard carries one
/// `(chain, handle)` pair per rule. The EGRESS install
/// (`install_outbound_tproxy`) creates ONE rule (the `prerouting` egress rule)
/// and NO output companion, so its guard carries one ownership token.
pub struct TproxyInterceptGuard {
    /// Shared ownership tokens for the `(chain, kernel-assigned handle)` pairs
    /// this install created or adopted. Final-token Drop deletes each rule,
    /// leaving shared infra and sibling intercepts untouched. One token for an
    /// egress install; two for an inbound install.
    rules: Vec<Arc<OwnedTproxyRule>>,
}

impl TproxyInterceptGuard {
    fn acquire<const N: usize>(rules: [(&'static str, u64); N]) -> Self {
        Self { rules: rules.into_iter().map(acquire_rule_ownership).collect() }
    }

    fn acquire_vec(rules: Vec<(&'static str, u64)>) -> Self {
        Self { rules: rules.into_iter().map(acquire_rule_ownership).collect() }
    }

    fn rule_keys(&self) -> impl Iterator<Item = RuleOwnershipKey> + '_ {
        self.rules.iter().map(|owner| (owner.chain, owner.handle))
    }

    fn disarm(&self) {
        for owner in &self.rules {
            owner.delete_on_drop.store(false, Ordering::Release);
        }
    }
}

type RuleOwnershipKey = (&'static str, u64);

fn rule_ownership_registry() -> &'static Mutex<HashMap<RuleOwnershipKey, Weak<OwnedTproxyRule>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<RuleOwnershipKey, Weak<OwnedTproxyRule>>>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn acquire_rule_ownership((chain, handle): RuleOwnershipKey) -> Arc<OwnedTproxyRule> {
    let mut registry =
        rule_ownership_registry().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(owner) = registry.get(&(chain, handle)).and_then(Weak::upgrade) {
        return owner;
    }
    let owner = Arc::new(OwnedTproxyRule {
        chain,
        handle,
        delete_on_drop: std::sync::atomic::AtomicBool::new(true),
    });
    registry.insert((chain, handle), Arc::downgrade(&owner));
    owner
}

struct OwnedTproxyRule {
    chain: &'static str,
    handle: u64,
    delete_on_drop: std::sync::atomic::AtomicBool,
}

impl Drop for OwnedTproxyRule {
    fn drop(&mut self) {
        if !self.delete_on_drop.load(Ordering::Acquire) {
            return;
        }
        // Delete only on the final shared owner. Best-effort: a racing §5
        // sweep or manual teardown may already have removed the kernel rule.
        let _ = nft::delete_rule(NFT_TABLE, self.chain, self.handle);
    }
}

/// Accept the redirected OUTBOUND workload connection on the agent's leg-F
/// listener and recover the workload's dialed original destination
/// (`orig_dst`).
///
/// Recovers `orig_dst` via `getsockname` on the TPROXY-intercepted leg-F socket
/// — symmetric with [`accept_inbound_leg`], which recovers inbound orig-dst the
/// same way. Under TPROXY the dialed destination IS the accepted socket's local
/// addr (D-TME-4; symmetric with the inbound `findings-inbound-intercept.md` §1
/// — NOT `SO_ORIGINAL_DST`). Returns the OWNED leg-F fd plus the recovered
/// `orig_dst` so the worker can RESOLVE `orig_dst` against the mesh
/// (`MtlsResolve`, 04-02) BEFORE deciding the connection's fate — the resolve
/// outcome (`Mesh` / `NonMesh` / `MeshUnreachable`), not a declared-peer slot,
/// now drives whether the leg is enforced over mTLS, passed through cleartext,
/// or fail-closed. The peer leg B dials on the `Mesh` arm is the RESOLVED
/// backend addr (`ResolvedBackend.addr`), which the worker stamps into
/// `Routed::Outbound { peer }` itself — NOT `orig_dst` (v1 headless: the two
/// coincide, but the worker uses the resolved addr so #167/#61 VIP→backend
/// translation wires without touching this seam).
///
/// # Errors
///
/// Returns [`InterceptError::Accept`] if the leg-F accept fails, or
/// [`InterceptError::OrigDst`] if `getsockname` orig-dst recovery fails.
pub fn accept_outbound_and_recover_orig_dst(
    leg_f_listener: &std::net::TcpListener,
) -> Result<(OwnedFd, SocketAddrV4)> {
    let (leg_f, _accept_peer) = leg_f_listener
        .accept()
        .map_err(|source| InterceptError::Accept { direction: "outbound", source })?;
    leg_f.set_nodelay(true).ok();
    // Symmetric with `accept_inbound_leg`: the dialed orig-dst IS the
    // TPROXY-intercepted accepted socket's local addr, recovered via the shared
    // `getsockname_orig` helper.
    let orig_dst = getsockname_orig(leg_f.as_raw_fd())?;
    Ok((OwnedFd::from(leg_f), orig_dst))
}

/// Accept the TPROXY-redirected INBOUND connection on leg-C.
///
/// Recovers orig-dst via `getsockname` (NOT `SO_ORIGINAL_DST`) and builds
/// [`InterceptedConnection`] (`Routed::Inbound { orig_dst }`); the owned leg C
/// is handed by value. Productionises
/// `roles.rs::{accept_leg_c_and_orig_dst, getsockname_orig}`.
///
/// # Errors
///
/// Returns [`InterceptError::Accept`] if the leg-C accept fails, or
/// [`InterceptError::OrigDst`] if `getsockname` original-destination recovery
/// fails.
pub fn accept_inbound_leg(
    leg_c_listener: &std::net::TcpListener,
    alloc: AllocationId,
) -> Result<InterceptedConnection> {
    let (leg_c, _peer) = leg_c_listener
        .accept()
        .map_err(|source| InterceptError::Accept { direction: "inbound", source })?;
    leg_c.set_nodelay(true).ok();
    // Under TPROXY the original destination IS the accepted socket's local
    // addr (`findings-inbound-intercept.md` §1 — NOT `SO_ORIGINAL_DST`).
    let orig_dst = getsockname_orig(leg_c.as_raw_fd())?;
    Ok(InterceptedConnection {
        leg: OwnedFd::from(leg_c),
        routed: Routed::Inbound { orig_dst },
        alloc,
        expected_peer: None,
    })
}

/// `getsockname` on a TPROXY-intercepted socket returns the ORIGINAL
/// destination the client aimed at. Productionises
/// `roles.rs::getsockname_orig` with typed-error propagation.
fn getsockname_orig(fd: RawFd) -> Result<SocketAddrV4> {
    // SAFETY: `sa`/`len` are correctly sized for an IPv4 sockaddr; `fd` is the
    // live accepted leg.
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockname(fd, std::ptr::from_mut(&mut sa).cast(), std::ptr::from_mut(&mut len))
    };
    if rc != 0 {
        return Err(InterceptError::OrigDst { source: std::io::Error::last_os_error() });
    }
    let ip = Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr));
    let port = u16::from_be(sa.sin_port);
    Ok(SocketAddrV4::new(ip, port))
}

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

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "unit-test bodies: a failed precondition must panic with an informative message"
)]
mod tests {
    //! Default-lane unit tests for the sync leg-acquire surface.
    //!
    //! The nft rule ENCODING + structural handle recovery is exercised (and
    //! golden-byte-pinned to `spike/findings-e.md`) in
    //! `overdrive_netlink::nft`; the real-kernel install/divert/sweep behaviour
    //! is locked by the Tier-3 ATs (`mtls_intercept_install`,
    //! `inbound_tproxy_harness`, `adopt_on_restart`). What remains here is the
    //! `getsockname` orig-dst recovery, which needs no kernel.

    // --- `accept_outbound_and_recover_orig_dst` getsockname recovery (D-TME-4) ---

    #[test]
    fn accept_outbound_and_recover_orig_dst_returns_the_getsockname_dialed_addr() {
        // `accept_outbound_and_recover_orig_dst` recovers the dialed orig-dst via
        // `getsockname` on the accepted leg-F socket (symmetric with
        // `accept_inbound_leg`). `accept` + `getsockname` + `set_nodelay` do no
        // privileged syscall, so this is default-lane (no root / no TPROXY): on a
        // plain loopback listener `getsockname` of the accepted socket returns the
        // dialed local addr. The real TPROXY orig-dst==dialed-dst on a live
        // intercepted connect is the Tier-3 03-03 / 05-01 obligation; here we pin
        // that the recovered orig_dst is the getsockname addr and the owned leg is
        // the genuine accepted socket.
        use std::io::{Read as _, Write as _};
        use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
        use std::os::fd::{AsRawFd as _, FromRawFd as _};
        use std::time::Duration;

        use super::accept_outbound_and_recover_orig_dst;

        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("bind plain loopback leg-F listener");
        let dialed_addr = match listener.local_addr().expect("local_addr") {
            std::net::SocketAddr::V4(a) => a,
            v6 @ std::net::SocketAddr::V6(_) => panic!("expected V4 addr, got {v6}"),
        };

        // Client dials so the production `accept()` has a pending connection, then
        // reads one byte written back through the recovered owned leg — proving
        // the returned fd IS the genuine accepted socket.
        let client = std::thread::spawn(move || {
            let mut s = TcpStream::connect_timeout(&dialed_addr.into(), Duration::from_secs(5))
                .expect("dial loopback leg-F");
            let mut buf = [0u8; 1];
            s.read_exact(&mut buf).expect("read echoed byte");
            buf
        });

        let (leg, orig_dst) = accept_outbound_and_recover_orig_dst(&listener)
            .expect("accept_outbound_and_recover_orig_dst must recover orig-dst");

        assert_eq!(
            orig_dst, dialed_addr,
            "recovered orig_dst must be the getsockname-recovered dialed addr"
        );

        // Write a byte through the owned leg; the client reads it back byte-exact.
        // SAFETY: a fresh owned fd over the accepted TCP leg; dropped at scope end.
        let mut stream = unsafe { TcpStream::from_raw_fd(libc_dup(leg.as_raw_fd())) };
        stream.write_all(b"X").expect("write through the owned leg");
        stream.flush().ok();
        drop(stream);

        assert_eq!(&client.join().expect("client thread"), b"X");
        drop(leg);
    }

    /// `dup(2)` a raw fd so the test can write through a copy while production
    /// keeps owning the original `OwnedFd`.
    fn libc_dup(fd: i32) -> i32 {
        // SAFETY: dup of a live fd; the returned fd is owned by the caller.
        let new = unsafe { libc::dup(fd) };
        assert!(new >= 0, "dup: {}", std::io::Error::last_os_error());
        new
    }
}
