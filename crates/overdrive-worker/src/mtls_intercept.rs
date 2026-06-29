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

use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd, RawFd};
use std::process::{Command, Stdio};

use overdrive_core::AllocationId;
use overdrive_core::traits::mtls_enforcement::{InterceptedConnection, Routed};

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
    /// `install_inbound_tproxy` could not install the nft-TPROXY rule or its
    /// `ip rule` / `ip route` companions.
    #[error("nft-TPROXY intercept install failed: {reason}")]
    TproxyInstall {
        /// Human-readable cause (the failing `nft` / `ip` command + stderr).
        reason: String,
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
/// Returns [`InterceptError::TproxyInstall`] if ensuring the shared infra
/// (`ip rule`, `ip route`, nft table/chain/exemption) fails for a reason other
/// than "already present", if appending the per-virt TPROXY rule fails, or if
/// the rule's handle cannot be recovered from the post-append chain dump.
pub fn install_inbound_tproxy(virt: SocketAddrV4, agent_port: u16) -> Result<TproxyInterceptGuard> {
    // (1) Ensure the SHARED, node-global routing infra idempotently. These are
    // add-if-missing converges (NOT a destructive preclean): a pre-existing
    // shared rule/route/table is the success case, left untouched. None of
    // these is removed on per-workload Drop.
    ensure_shared_routing_infra()?;

    // (2) Append exactly ONE per-virt TPROXY rule to the shared chain, after
    // the F5 exemption. TPROXY preserves daddr → the agent recovers orig-dst
    // per-flow via getsockname, so a single shared fwmark routes every virt.
    run_nft(&[
        "add",
        "rule",
        "ip",
        NFT_TABLE,
        NFT_CHAIN,
        "ip",
        "daddr",
        &virt.ip().to_string(),
        "tcp",
        "dport",
        &virt.port().to_string(),
        "tproxy",
        "to",
        &format!("127.0.0.1:{agent_port}"),
        "meta",
        "mark",
        "set",
        &format!("{TPROXY_FWMARK:#x}"),
        "accept",
    ])?;

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
    let leg_s_mark = overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
    run_nft(&[
        "add",
        "rule",
        "ip",
        NFT_TABLE,
        NFT_OUTPUT_CHAIN,
        "ip",
        "daddr",
        &virt.ip().to_string(),
        "tcp",
        "dport",
        &virt.port().to_string(),
        "meta",
        "mark",
        "!=",
        &format!("{leg_s_mark:#x}"),
        "meta",
        "mark",
        "set",
        &format!("{TPROXY_FWMARK:#x}"),
        "accept",
    ])?;

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
    // (4) Recover the kernel-assigned handle of EACH rule we just appended so
    // Drop can delete EXACTLY those two rules (siblings, the exemptions, and the
    // shared infra all untouched) — research F7c, the nft-canonical per-rule
    // teardown. The prerouting rule is recovered by its `tproxy to` redirect; the
    // output rule (which carries NO `tproxy to`) is recovered by its
    // `ip daddr`/`tcp dport`/`meta mark set` shape in the OUTPUT chain.
    let prerouting_handle = find_virt_rule_handle(virt, agent_port)?;
    let output_handle = find_output_divert_rule_handle(virt)?;
    Ok(TproxyInterceptGuard {
        rules: vec![(NFT_CHAIN, prerouting_handle), (NFT_OUTPUT_CHAIN, output_handle)],
    })
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
/// [`TproxyInterceptGuard`] whose `Drop` removes ONLY that one rule by its
/// kernel-assigned handle; the node-global shared routing infra
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
/// rule's handle returned. On the normal teardown path the returned guard's
/// [`TproxyInterceptGuard`] `Drop` removes the rule by handle, so the next
/// install for that veth starts from a clean chain. (The inbound install does
/// not need this presence-check — distinct virts produce distinct rule text.)
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
/// Returns [`InterceptError::TproxyInstall`] if ensuring the shared infra
/// fails for a reason other than "already present", if appending the egress
/// rule fails, or if the rule's handle cannot be recovered from the chain dump.
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
    let dump = list_chain()?;
    if dump_has_egress_rule(&dump, host_veth, agent_leg_f_port)
        && let Some(existing) = find_egress_rule_handle_in_dump(&dump, host_veth, agent_leg_f_port)
    {
        return Ok(TproxyInterceptGuard { rules: vec![(NFT_CHAIN, existing)] });
    }

    // (3) Append exactly ONE egress rule to the shared chain, after the F5
    // exemption. Match on the ingress interface (`iifname <host_veth>`) +
    // `meta l4proto tcp`; redirect ALL the workload's egress TCP to leg F.
    // TPROXY preserves orig-dst → recovered per-flow downstream (03-02), so a
    // single shared fwmark routes every flow (same as inbound).
    run_nft(&[
        "add",
        "rule",
        "ip",
        NFT_TABLE,
        NFT_CHAIN,
        "iifname",
        host_veth,
        "meta",
        "l4proto",
        "tcp",
        "tproxy",
        "to",
        &format!("127.0.0.1:{agent_leg_f_port}"),
        "meta",
        "mark",
        "set",
        &format!("{TPROXY_FWMARK:#x}"),
        "accept",
    ])?;

    // (4) Recover the kernel-assigned handle of the rule we just appended so
    // Drop can delete EXACTLY that rule (siblings, the exemption, and the
    // shared infra all untouched).
    let handle = find_egress_rule_handle(host_veth, agent_leg_f_port)?.ok_or_else(|| {
        InterceptError::TproxyInstall {
            reason: format!(
                "could not recover nft rule handle for egress host_veth {host_veth} → 127.0.0.1:{agent_leg_f_port} after append"
            ),
        }
    })?;
    // The egress install creates ONE rule in the prerouting chain and NO output
    // companion (the output-hook divert is inbound-only — it intercepts the
    // agent's host-originated leg-B re-dial TO a backend, REV-5).
    Ok(TproxyInterceptGuard { rules: vec![(NFT_CHAIN, handle)] })
}

/// Boot-recovery sweep (adopt-on-restart §5, D-TME-12; folds 03-01 finding D2).
///
/// Removes EVERY per-workload rule — egress (`iifname`-matched) AND inbound
/// (`ip daddr`/`tcp dport`-matched) `tproxy` rules from the `prerouting` chain,
/// AND the REV-5 `output` divert rules from the `output` chain — by handle,
/// leaving the shared infra of BOTH chains (the leg-S `meta mark
/// <MTLS_LEG_S_DIAL_MARK> accept` exemptions, the table+chains, the chain
/// policy/type/hook lines) UNTOUCHED — so a subsequent per-alloc re-install
/// appends exactly one clean rule per direction per chain. The output rule has
/// no `tproxy` verb, so the classifier
/// ([`per_workload_rule_handles_in_dump`]) recognises it by its `ip daddr` +
/// `meta mark set` + `tcp dport` shape; missing the output chain would leak the
/// divert rule across every restart (the D2 class, reopened — REV-5).
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
/// the ONLY swallowed case is the shared table/chain being absent
/// ([`InterceptError::ChainAbsent`] from [`list_chain`]) → `Ok(0)`, the benign
/// "nothing to sweep" signal on a fresh boot. A spawn error or a genuine `nft`
/// failure on the `list chain` (binary missing, EPERM, transient lock —
/// surfaced by [`list_chain`] as [`InterceptError::TproxyInstall`]) propagates
/// and refuses the boot, as does a by-handle `nft delete rule` failure.
// mutants: skip — thin nft-I/O shim (`list_chain` + by-handle `run_nft delete`);
// the pure decision is `per_workload_rule_handles_in_dump` (unit + mutation
// covered). Body-replacement mutants (`Ok(0)`/`Ok(1)`) are killable only by the
// real-kernel Tier-3 AT `serve_restart_sweeps_surviving_per_workload_tproxy_rule`
// (overdrive-control-plane), which the worker-package default-lane mutants suite
// cannot run.
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

/// Sweep every per-workload rule out of ONE named chain by handle, returning the
/// count removed. An absent chain ([`InterceptError::ChainAbsent`]) is the
/// benign fresh-boot "nothing to sweep" signal → `Ok(0)`; every genuine `nft`
/// failure propagates and refuses the boot (fail-CLOSED, matching the by-handle
/// delete path).
///
/// # Why fail-closed on a list/delete error
///
/// A still-Running survivor does NOT trigger a `start_alloc` (SPIKE-B — the
/// reconciler does not re-drive survivors), so there is no downstream install to
/// catch a stranded guard-less survivor rule (the D2 dead-weight §5 exists to
/// reap) if the list fails — fail-closed is the only posture that does not leave
/// it stranded.
// mutants: skip — thin nft-I/O shim (`list_named_chain` + by-handle `run_nft
// delete`); the pure decision is `per_workload_rule_handles_in_dump` (unit +
// mutation covered). Body-replacement mutants (`Ok(0)`/`Ok(1)`) are killable
// only by the real-kernel Tier-3 AT
// `serve_restart_sweeps_surviving_per_workload_tproxy_rule`
// (overdrive-control-plane), which the worker-package default-lane mutants
// suite cannot run. DOCUMENTATION ONLY — the actual suppression is the
// `replace sweep_one_chain -> Result<usize> with Ok` exclude_re entry in
// `.cargo/mutants.toml` (a bare comment suppresses nothing per testing.md).
fn sweep_one_chain(chain: &str) -> Result<usize> {
    let dump = match list_named_chain(chain) {
        Ok(dump) => dump,
        Err(InterceptError::ChainAbsent) => return Ok(0),
        Err(e) => return Err(e),
    };

    // Classify (pure): collect the handle of every per-workload rule, leaving the
    // shared infra (chain header / type-policy line / leg-S exemption) untouched.
    let handles = per_workload_rule_handles_in_dump(&dump);

    // Delete each by handle — the SAME by-handle `nft delete rule … handle <N>`
    // the guard's `Drop` uses. A delete failure (a real `nft` error, not an
    // absent rule) refuses the boot: surface it as `TproxyInstall`.
    for handle in &handles {
        let h = handle.to_string();
        run_nft(&["delete", "rule", "ip", NFT_TABLE, chain, "handle", &h])?;
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
    let fwmark = format!("{TPROXY_FWMARK:#x}");
    let rt_table = TPROXY_RT_TABLE.to_string();

    // ip rule: add only if not already present (add-if-missing — `ip rule add`
    // would stack a duplicate on every install otherwise).
    if !ip_rule_fwmark_present(TPROXY_FWMARK, TPROXY_RT_TABLE) {
        run_ip(&["rule", "add", "fwmark", &fwmark, "lookup", &rt_table])?;
    }

    // ip route: `ip route add` returns EEXIST (exit 2) when already present —
    // tolerate that one case, propagate any other failure.
    ensure_ip_route_local()?;

    // nft table + chain: `nft add table` / `nft add chain` are idempotent
    // create-if-missing for table/chain, so re-running is a no-op.
    run_nft(&["add", "table", "ip", NFT_TABLE])?;
    run_nft(&[
        "add",
        "chain",
        "ip",
        NFT_TABLE,
        NFT_CHAIN,
        "{",
        "type",
        "filter",
        "hook",
        "prerouting",
        "priority",
        "mangle;",
        "policy",
        "accept;",
        "}",
    ])?;

    // F5 exemption at the prerouting chain head — insert ONCE. `nft insert`
    // prepends, so guarding against a duplicate add keeps it exactly once at the
    // head ahead of every per-virt tproxy rule.
    let leg_s_mark = overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
    if !chain_has_leg_s_exemption(NFT_CHAIN)? {
        run_nft(&[
            "insert",
            "rule",
            "ip",
            NFT_TABLE,
            NFT_CHAIN,
            "meta",
            "mark",
            &format!("{leg_s_mark:#x}"),
            "accept",
        ])?;
    }

    // REV-5 OUTPUT chain: idempotent create-if-missing. It MUST be
    // `type route hook output priority mangle` (NOT `type filter`) so the kernel
    // RE-EVALUATES the route after a per-virt divert's `meta mark set`, firing
    // the `ip rule fwmark` → `local table 100` route on the OUTPUT path
    // (spike-proven; the `type filter` counter-test lands on the plaintext
    // decoy). `nft add chain` is create-if-missing, so re-running is a no-op.
    run_nft(&[
        "add",
        "chain",
        "ip",
        NFT_TABLE,
        NFT_OUTPUT_CHAIN,
        "{",
        "type",
        "route",
        "hook",
        "output",
        "priority",
        "mangle;",
        "policy",
        "accept;",
        "}",
    ])?;

    // leg-S exemption at the OUTPUT chain head — insert ONCE, mirroring the
    // prerouting head. The agent's marked leg-S dial (`SO_MARK 0x2`) must reach
    // the workload directly, not be diverted back into leg-C; the `meta mark 0x2
    // accept` head rule exempts it before any per-virt output divert can match.
    if !chain_has_leg_s_exemption(NFT_OUTPUT_CHAIN)? {
        run_nft(&[
            "insert",
            "rule",
            "ip",
            NFT_TABLE,
            NFT_OUTPUT_CHAIN,
            "meta",
            "mark",
            &format!("{leg_s_mark:#x}"),
            "accept",
        ])?;
    }
    Ok(())
}

/// `ip route add local 0.0.0.0/0 dev lo table 100`, tolerating an EEXIST
/// (`ip` exits 2, stderr "File exists") as the already-converged success case.
fn ensure_ip_route_local() -> Result<()> {
    let rt_table = TPROXY_RT_TABLE.to_string();
    let out = Command::new("ip")
        .args(["route", "add", "local", "0.0.0.0/0", "dev", "lo", "table", &rt_table])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| InterceptError::TproxyInstall {
            reason: format!("spawn ip route add: {e}"),
        })?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("File exists") {
        // Already converged — the shared route is node-global and persists.
        return Ok(());
    }
    Err(InterceptError::TproxyInstall {
        reason: format!(
            "ip route add local … table {rt_table} exited {:?}: {}",
            out.status.code(),
            stderr.trim()
        ),
    })
}

/// True iff an `ip rule` line for `fwmark <mark>` lookup `<table>` already
/// exists — used so [`ensure_shared_routing_infra`] adds the rule only when
/// missing (idempotent ensure; `ip rule add` would otherwise stack a
/// duplicate per install). Thin shell-out shim over [`ip_rule_dump_has_fwmark`];
/// the predicate logic is unit-tested there.
// mutants: skip
fn ip_rule_fwmark_present(mark: u32, table: u32) -> bool {
    let Ok(out) = Command::new("ip")
        .args(["rule", "show"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    ip_rule_dump_has_fwmark(&text, mark, table)
}

/// True iff an `ip rule show` dump carries a line that BOTH marks on
/// `fwmark <mark>` (rendered either hex `fwmark 0x1` or decimal `fwmark 1`)
/// AND routes via `lookup <table>`. Both conjuncts must hold on the SAME
/// line: a rule that fwmark-matches but routes elsewhere, or one that looks
/// up `<table>` for a different mark, is NOT the rule we ensure — treating
/// either as present would skip the `ip rule add` and leave the fwmark
/// unrouted. Pure so a unit test can pin the conjunction against captured
/// `ip rule show` output.
fn ip_rule_dump_has_fwmark(dump: &str, mark: u32, table: u32) -> bool {
    let needle_hex = format!("fwmark {mark:#x}");
    let needle_dec = format!("fwmark {mark}");
    let lookup = format!("lookup {table}");
    dump.lines()
        .any(|l| (l.contains(&needle_hex) || l.contains(&needle_dec)) && l.contains(&lookup))
}

/// True iff the named shared chain already carries the leg-S-dial
/// `meta mark <MTLS_LEG_S_DIAL_MARK> accept` exemption — used so the exemption
/// is inserted exactly once at each chain's head (otherwise every install would
/// prepend another duplicate). Thin shell-out shim over
/// [`dump_has_leg_s_exemption`] (`nft` via [`list_named_chain`]); the predicate
/// logic is unit-tested there. Parameterised by `chain` so the SAME guard
/// covers both the `prerouting` chain and the REV-5 `output` chain.
// mutants: skip — thin nft-I/O shim; the pure decision is
// `dump_has_leg_s_exemption` (unit-tested, in mutation scope). The whole-fn
// body-replacement mutant (`Ok(true)`/`Ok(false)`) is killable only by a
// real-kernel Tier-3 run. DOCUMENTATION ONLY — the actual suppression is the
// `replace chain_has_leg_s_exemption -> Result<bool> with Ok` exclude_re entry
// in `.cargo/mutants.toml` (a bare comment suppresses nothing per testing.md).
fn chain_has_leg_s_exemption(chain: &str) -> Result<bool> {
    Ok(dump_has_leg_s_exemption(&list_named_chain(chain)?))
}

/// True iff a `nft -a list chain` dump carries a `meta mark
/// <MTLS_LEG_S_DIAL_MARK> accept` line. nft renders the mark as a zero-padded
/// 8-hex-digit value (e.g. `0x00000002`), NOT `0x2` or decimal `2`, so the
/// match must canonicalise to nft's rendering — matching `0x2` would never
/// fire and the exemption would be re-inserted on every install. Pure so a
/// unit test can pin the parse against captured nft output.
fn dump_has_leg_s_exemption(dump: &str) -> bool {
    let leg_s_mark = overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
    // nft's canonical rendering: `meta mark 0x00000002 accept`.
    let nft_rendered = format!("meta mark {leg_s_mark:#010x} accept");
    dump.lines().any(|l| l.trim().contains(&nft_rendered))
}

/// True iff `nft`'s stderr for a `list chain` of an ABSENT table/chain
/// (the benign "nothing to sweep" case), distinct from a genuine failure.
/// nft emits "No such file or directory" / "does not exist" for the
/// absent case. Pure so a unit test pins the classification without nft.
fn stderr_reports_absent_chain(stderr: &str) -> bool {
    stderr.contains("No such file or directory") || stderr.contains("does not exist")
}

/// `nft -a list chain` over the [`NFT_CHAIN`] (`prerouting`) chain, with
/// handles. Thin convenience over [`list_named_chain`] for the dominant
/// prerouting case.
fn list_chain() -> Result<String> {
    list_named_chain(NFT_CHAIN)
}

/// `nft -a list chain ip <table> <chain>` (with handles). Returns the dump on
/// success; maps a non-zero exit whose stderr reports the table/chain absent to
/// [`InterceptError::ChainAbsent`] (the benign "nothing to sweep" signal), and
/// every other failure (spawn error, or a non-success whose stderr is a genuine
/// `nft` error — EPERM, missing binary, transient lock) to
/// [`InterceptError::TproxyInstall`].
///
/// Parameterised by `chain` so both the `prerouting` chain and the REV-5
/// `output` chain can be dumped (handle recovery + the §5 boot sweep run over
/// BOTH chains).
// mutants: skip — thin nft-I/O shim (`nft -a list chain` + the
// `stderr_reports_absent_chain` classification, which IS unit + mutation
// covered). The whole-fn body-replacement mutant (`Ok(...)`) is killable only by
// the real-kernel Tier-3 ATs that drive `nft` for real (the §5 sweep + the
// dial-by-name walking skeleton), which the worker-package default-lane mutants
// suite cannot run. DOCUMENTATION ONLY — the actual suppression is the
// `replace list_named_chain -> Result<String> with Ok` exclude_re entry in
// `.cargo/mutants.toml` (a bare comment suppresses nothing per testing.md).
fn list_named_chain(chain: &str) -> Result<String> {
    let out = Command::new("nft")
        .args(["-a", "list", "chain", "ip", NFT_TABLE, chain])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| InterceptError::TproxyInstall {
            reason: format!("spawn nft list chain: {e}"),
        })?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr_reports_absent_chain(&stderr) {
        // Benign: the shared table/chain has never been created (fresh boot, no
        // mTLS workload has installed a rule). Callers that tolerate absence (the
        // §5 sweep) map this to a no-op; callers that require the chain propagate.
        return Err(InterceptError::ChainAbsent);
    }
    Err(InterceptError::TproxyInstall {
        reason: format!(
            "nft -a list chain ip {NFT_TABLE} {chain} exited {:?}: {}",
            out.status.code(),
            stderr.trim()
        ),
    })
}

/// Recover this virt's TPROXY rule handle from the `nft -a list chain` dump.
///
/// Parses the kernel-assigned handle of the per-virt rule matching `virt`'s
/// daddr/dport and the agent redirect target. nft renders an appended rule
/// with a trailing `# handle <N>`; we match the line carrying this virt's
/// `ip daddr <vip>` + `tcp dport <vport>` + the
/// `tproxy to 127.0.0.1:<agent_port>` redirect so two installs for distinct
/// virts capture distinct handles.
fn find_virt_rule_handle(virt: SocketAddrV4, agent_port: u16) -> Result<u64> {
    let dump = list_chain()?;
    let vip = virt.ip().to_string();
    let vport = virt.port().to_string();
    let daddr = format!("ip daddr {vip}");
    let dport = format!("tcp dport {vport}");
    let redirect = format!("tproxy to 127.0.0.1:{agent_port}");
    for line in dump.lines() {
        if line.contains(&daddr)
            && line.contains(&dport)
            && line.contains(&redirect)
            && line.contains("# handle ")
            && let Some(handle) = parse_handle(line)
        {
            return Ok(handle);
        }
    }
    Err(InterceptError::TproxyInstall {
        reason: format!(
            "could not recover nft rule handle for virt {vip}:{vport} → 127.0.0.1:{agent_port} in chain dump:\n{dump}"
        ),
    })
}

/// Recover this virt's REV-5 OUTPUT divert rule handle from the live `output`
/// chain.
///
/// The output divert rule carries NO `tproxy to` redirect (it is `meta mark
/// set`, not a tproxy), so [`find_virt_rule_handle`]'s redirect-keyed parse
/// cannot recover it. This sibling parser matches on the output rule's ACTUAL
/// text — `ip daddr <vip>` + `tcp dport <vport>` + `meta mark set` — in the
/// OUTPUT chain dump, via the pure [`output_divert_handle_in_dump`] (unit-tested
/// there). Distinct virts produce distinct daddr/dport, so the recovered handle
/// is this virt's alone.
///
/// # Errors
///
/// Returns [`InterceptError::TproxyInstall`] if the output chain dump cannot be
/// obtained, or if the just-appended rule's handle is not found in it.
// mutants: skip — thin nft-I/O shim (`list_named_chain` + the pure
// `output_divert_handle_in_dump`, which IS unit + mutation covered, Praise P5).
// The whole-fn body-replacement mutant is killable only by a real-kernel Tier-3
// run. DOCUMENTATION ONLY — the actual suppression is the
// `replace find_output_divert_rule_handle -> Result<u64> with Ok` exclude_re
// entry in `.cargo/mutants.toml` (a bare comment suppresses nothing per
// testing.md).
fn find_output_divert_rule_handle(virt: SocketAddrV4) -> Result<u64> {
    let dump = list_named_chain(NFT_OUTPUT_CHAIN)?;
    output_divert_handle_in_dump(&dump, virt).ok_or_else(|| InterceptError::TproxyInstall {
        reason: format!(
            "could not recover nft OUTPUT divert rule handle for virt {}:{} in output chain dump:\n{dump}",
            virt.ip(),
            virt.port()
        ),
    })
}

/// Pure: parse the kernel-assigned handle of the REV-5 OUTPUT divert rule for
/// `virt` from an `nft -a list chain … output` dump, or `None` if absent.
///
/// The output divert rule matches `ip daddr <vip>` AND `tcp dport <vport>` AND
/// `meta mark set ` (the divert's `meta mark set 0x1` token) on the SAME line —
/// all three conjuncts are required so the head leg-S exemption (`meta mark
/// 0x00000002 accept` — a MATCH, no `set`, no `ip daddr`/`tcp dport`) is NOT
/// mistaken for a divert rule, and so two distinct virts' divert rules are not
/// confused. The handle is read off the trailing `# handle <N>`. Pure so a unit
/// test can pin the conjunction + parse against captured nft output.
fn output_divert_handle_in_dump(dump: &str, virt: SocketAddrV4) -> Option<u64> {
    let daddr = format!("ip daddr {}", virt.ip());
    let dport = format!("tcp dport {}", virt.port());
    dump.lines()
        .filter(|l| {
            l.contains(&daddr)
                && l.contains(&dport)
                && l.contains("meta mark set ")
                && l.contains("# handle ")
        })
        .find_map(parse_handle)
}

/// Recover this host-veth's EGRESS TPROXY rule handle from the live shared
/// chain, or `None` if no such rule is present.
///
/// Thin shell-out shim over [`find_egress_rule_handle_in_dump`] (the pure
/// parse, unit-tested there) — `Ok(None)` means "no egress rule for this veth
/// yet" (the first-install / append case), `Ok(Some(handle))` means "already
/// present" (the idempotent re-install case), and an `Err` means the chain
/// dump itself could not be obtained.
// mutants: skip
fn find_egress_rule_handle(host_veth: &str, agent_leg_f_port: u16) -> Result<Option<u64>> {
    Ok(find_egress_rule_handle_in_dump(&list_chain()?, host_veth, agent_leg_f_port))
}

/// Pure: parse the kernel-assigned handle of the egress rule for `host_veth` +
/// `agent_leg_f_port` from an `nft -a list chain` dump, or `None` if absent.
///
/// The egress rule matches BOTH `iifname "<host_veth>"` AND the
/// `tproxy to 127.0.0.1:<agent_leg_f_port>` redirect on the SAME line — both
/// conjuncts are required so an inbound `ip daddr`/`tcp dport` rule sharing the
/// redirect target, or a different veth's egress rule sharing the redirect, is
/// NOT mistaken for this veth's rule. The handle is read off the trailing
/// `# handle <N>`. Pure so a unit test can pin the conjunction + parse against
/// captured nft output without a kernel.
fn find_egress_rule_handle_in_dump(
    dump: &str,
    host_veth: &str,
    agent_leg_f_port: u16,
) -> Option<u64> {
    let iifname = format!("iifname \"{host_veth}\"");
    let redirect = format!("tproxy to 127.0.0.1:{agent_leg_f_port}");
    dump.lines()
        .filter(|l| l.contains(&iifname) && l.contains(&redirect) && l.contains("# handle "))
        .find_map(parse_handle)
}

/// Pure: true iff the `nft -a list chain` dump already carries the egress rule
/// for `host_veth` + `agent_leg_f_port` — used so the idempotent
/// [`install_outbound_tproxy`] append fires only when the rule is missing
/// (otherwise a repeat install for the same veth stacks a duplicate, since the
/// egress rule has no unique daddr/dport to distinguish it).
///
/// Requires BOTH the `iifname "<host_veth>"` match AND the
/// `tproxy to 127.0.0.1:<agent_leg_f_port>` redirect on the SAME line: an
/// inbound daddr/dport rule, or a different veth's egress rule, must not be
/// read as this veth's egress rule. Pure so a unit test pins the conjunction
/// against captured nft output.
fn dump_has_egress_rule(dump: &str, host_veth: &str, agent_leg_f_port: u16) -> bool {
    let iifname = format!("iifname \"{host_veth}\"");
    let redirect = format!("tproxy to 127.0.0.1:{agent_leg_f_port}");
    dump.lines().any(|l| l.contains(&iifname) && l.contains(&redirect))
}

/// Extract the `<N>` from a trailing `# handle <N>` on an `nft -a` rule line.
fn parse_handle(line: &str) -> Option<u64> {
    let (_, after) = line.rsplit_once("# handle ")?;
    after.split_whitespace().next()?.parse::<u64>().ok()
}

/// Pure: collect the kernel-assigned handle of EVERY per-workload rule in an
/// `nft -a list chain` dump, port-blind (§5 boot-recovery sweep). Covers BOTH
/// the prerouting `tproxy` rules AND the REV-5 `output` divert rules.
///
/// A per-workload rule is recognised by EITHER discriminator, paired with a
/// trailing `# handle <N>`:
///   - **`tproxy to `** — the prerouting rules: egress (`iifname "<veth>" …
///     tproxy to …`) and inbound (`ip daddr <vip> tcp dport <vport> … tproxy to
///     …`) both carry it.
///   - **(`ip daddr ` AND `meta mark set ` AND `tcp dport `)** — the REV-5
///     OUTPUT divert rule (`ip daddr <vip> tcp dport <vport> meta mark != 0x2
///     meta mark set 0x1 accept`), which carries NO `tproxy to`. Without this
///     arm the output divert rule leaks across every control-plane restart (the
///     exact D2 dead-weight class the sweep closes, reopened for the output rule).
///
/// The SHARED infra carries NEITHER discriminator and is KEPT: the chain header
/// (`chain prerouting { # handle 1`), the type/policy line, and the leg-S
/// `meta mark <MTLS_LEG_S_DIAL_MARK> accept` exemption all carry no `tproxy to `,
/// and the exemption is a `meta mark` MATCH (`accept`) with NO `set` and NO
/// `ip daddr`/`tcp dport` — so the `meta mark set ` token (a `set`, not a match)
/// plus `ip daddr ` excludes it cleanly. Port-blind by design: a restart loses
/// the dead leg-C/leg-F ports, so the port-keyed predicates
/// ([`find_egress_rule_handle_in_dump`], [`find_virt_rule_handle`],
/// [`output_divert_handle_in_dump`]) cannot drive the sweep — it removes ALL
/// per-workload rules regardless of redirect port.
///
/// Pure so a unit test can pin the keep/collect partition against the verbatim
/// captured nft fixtures without a kernel.
fn per_workload_rule_handles_in_dump(dump: &str) -> Vec<u64> {
    dump.lines()
        .filter(|line| {
            // Prerouting `tproxy` rules (egress + inbound) carry `tproxy to `.
            line.contains("tproxy to ")
                // The REV-5 OUTPUT divert rule carries no `tproxy to`; it is the
                // only line carrying BOTH a `meta mark set ` (a SET — excluding
                // the `meta mark … accept` exemption MATCH) AND an `ip daddr `
                // + `tcp dport ` (excluding the chain header / type-policy line).
                || (line.contains("ip daddr ")
                    && line.contains("meta mark set ")
                    && line.contains("tcp dport "))
        })
        // The trailing `# handle <N>` is parsed by `parse_handle`; a matching
        // line without a handle marker (a non-`-a` / truncated dump) yields
        // nothing to delete and is skipped by `filter_map`.
        .filter_map(parse_handle)
        .collect()
}

/// RAII guard removing ONLY the per-virt rules THIS install created on `Drop`.
///
/// Deletes each rule by its kernel-assigned `(chain, handle)` pair. The shared
/// routing infra — `ip rule`, `ip route`, nft table/chains, and the F5
/// exemptions — is node-global and is NOT removed here; sibling intercepts'
/// rules are untouched.
///
/// An INBOUND install (`install_inbound_tproxy`) now creates TWO per-virt rules
/// — the `prerouting` `tproxy` rule AND the `output` `meta mark set` divert rule
/// (REV-5, the leg-B re-dial interception companion) — so the guard carries one
/// `(chain, handle)` pair per rule. The EGRESS install
/// (`install_outbound_tproxy`) creates ONE rule (the `prerouting` egress rule)
/// and NO output companion, so its guard carries a single pair. `Drop` iterates
/// and deletes each.
pub struct TproxyInterceptGuard {
    /// The `(chain, kernel-assigned handle)` pairs this install created. Drop
    /// deletes each rule by `(chain, handle)`, leaving the shared infra and
    /// sibling intercepts untouched. One pair for an egress install; two (the
    /// `prerouting` `tproxy` rule + the `output` divert rule) for an inbound
    /// install.
    rules: Vec<(&'static str, u64)>,
}

impl Drop for TproxyInterceptGuard {
    fn drop(&mut self) {
        // Delete ONLY the rules this install created, each by its
        // `(chain, handle)` pair (research F7c). `.output()` (not `.status()`)
        // drains the child reliably under the nextest harness — see the D5
        // root-cause note on `run_best_effort`.
        for (chain, handle) in &self.rules {
            let h = handle.to_string();
            let _ = run_best_effort(&svec(&[
                "nft", "delete", "rule", "ip", NFT_TABLE, chain, "handle", &h,
            ]));
        }
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

/// Run `ip <args>`; map a non-zero exit (or spawn failure) to
/// [`InterceptError::TproxyInstall`] with the command + stderr as the cause.
fn run_ip(args: &[&str]) -> Result<()> {
    let out = Command::new("ip")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| InterceptError::TproxyInstall { reason: format!("spawn ip {args:?}: {e}") })?;
    if out.status.success() {
        Ok(())
    } else {
        Err(InterceptError::TproxyInstall {
            reason: format!(
                "ip {args:?} exited {:?}: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        })
    }
}

/// Run `nft <args>`; map a non-zero exit (or spawn failure) to
/// [`InterceptError::TproxyInstall`] with the command + stderr as the cause.
///
/// Used for the idempotent `add table` / `add chain` / `insert rule` /
/// `add rule` operations. `add table`/`add chain` are create-if-missing
/// (re-running is a no-op); the callers guard `add rule`/`insert rule` against
/// duplicates via the chain dump.
fn run_nft(args: &[&str]) -> Result<()> {
    let out = Command::new("nft")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| InterceptError::TproxyInstall {
            reason: format!("spawn nft {args:?}: {e}"),
        })?;
    if out.status.success() {
        Ok(())
    } else {
        Err(InterceptError::TproxyInstall {
            reason: format!(
                "nft {args:?} exited {:?}: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        })
    }
}

/// Best-effort `Command` run used by the guard's per-rule teardown.
///
/// A missing rule is the "already gone" signal, not an error.
/// Uses `.output()` (not `.status()`): D5 root-cause — under the nextest test
/// harness a bare `Command::status()` (which calls `wait()` directly on the
/// child) can race the harness's own child handling and report a spurious
/// non-success / `ECHILD`, whereas `.output()` (→ `wait_with_output()`) reads
/// the child's piped stdout/stderr to EOF before reaping, which drains
/// reliably. The old drain loop that broke on the first non-success is gone
/// (the shared infra is no longer drained per-install under the (b) model), so
/// this is the only remaining shell-out on the teardown path; `.output()` is
/// what makes the by-handle delete actually fire under the gate.
fn run_best_effort(argv: &[String]) -> std::io::Result<std::process::Output> {
    debug_assert!(!argv.is_empty(), "run_best_effort requires a non-empty argv (program name)");
    Command::new(&argv[0]).args(&argv[1..]).stdout(Stdio::piped()).stderr(Stdio::piped()).output()
}

/// `&[&str]` → `Vec<String>` for the owned cleanup argv set.
fn svec(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_string()).collect()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "unit-test bodies: a failed precondition must panic with an informative message"
)]
mod tests {
    //! Pure-logic unit tests for the nft-dump parse helpers. These pin the
    //! exact `nft -a list chain` rendering against which the production
    //! idempotent-ensure (exemption dedup) and by-handle teardown operate — the
    //! rendering the integration AT exercises end-to-end but cannot isolate.
    //! The fixture is a verbatim capture of real `nft -a` output (zero-padded
    //! 8-hex marks, trailing `# handle <N>`), so a drift in nft's format OR a
    //! regression in the parse is caught here without a kernel.

    use super::{
        dump_has_egress_rule, dump_has_leg_s_exemption, find_egress_rule_handle_in_dump,
        ip_rule_dump_has_fwmark, output_divert_handle_in_dump, parse_handle,
        per_workload_rule_handles_in_dump,
    };

    // --- `ip rule show` fwmark-routing predicate (extracted from the
    // `ip`-shelling shim so the conjunction is unit-killable; mirrors the
    // `dump_has_leg_s_exemption` split out of the `nft` path) ---

    #[test]
    fn ip_rule_fwmark_detected_against_hex_rendered_dump() {
        // `ip rule show` renders the fwmark in hex on a modern iproute2. The
        // rule that BOTH marks on the fwmark AND looks up our table is the one
        // we ensure — it must be detected so the idempotent `add` is skipped.
        let dump = "\
0:\tfrom all lookup local
32765:\tfrom all fwmark 0x1 lookup 100
32766:\tfrom all lookup main
32767:\tfrom all lookup default";
        assert!(
            ip_rule_dump_has_fwmark(dump, 1, 100),
            "a `fwmark 0x1 ... lookup 100` rule must be detected (hex rendering)"
        );
    }

    #[test]
    fn ip_rule_fwmark_detected_against_decimal_rendered_dump() {
        // Older iproute2 renders the mark in decimal (`fwmark 1`); the
        // predicate must canonicalise across both renderings.
        let dump = "32765:\tfrom all fwmark 1 lookup 100";
        assert!(
            ip_rule_dump_has_fwmark(dump, 1, 100),
            "a `fwmark 1 ... lookup 100` rule must be detected (decimal rendering)"
        );
    }

    #[test]
    fn ip_rule_fwmark_requires_both_conjuncts_on_the_same_line() {
        // Discriminating case that KILLS the `&& -> ||` mutant on the
        // extracted predicate. NO single line both fwmark-matches AND looks up
        // our table: line A marks on our fwmark but routes to a DIFFERENT
        // table (200, not 100); line B looks up table 100 but for a DIFFERENT
        // fwmark (0x2, not 0x1). Under `&&` neither line qualifies -> false
        // (correct: our rule is absent, so `ip rule add` must still fire).
        // Under the `||` mutant, line A satisfies the fwmark conjunct and line
        // B satisfies the lookup conjunct -> the mutant wrongly returns true,
        // skipping the add and leaving the fwmark unrouted.
        let dump = "\
32764:\tfrom all fwmark 0x1 lookup 200
32765:\tfrom all fwmark 0x2 lookup 100";
        assert!(
            !ip_rule_dump_has_fwmark(dump, 1, 100),
            "neither line both marks on fwmark 0x1 AND looks up table 100; the \
             rule is absent and the predicate must return false (the `||` \
             mutant would wrongly report it present)"
        );
    }

    #[test]
    fn ip_rule_fwmark_absent_from_a_dump_with_no_matching_rule() {
        // True-negative: a vanilla policy table with none of our fwmark rules.
        let dump = "\
0:\tfrom all lookup local
32766:\tfrom all lookup main
32767:\tfrom all lookup default";
        assert!(
            !ip_rule_dump_has_fwmark(dump, 1, 100),
            "a dump carrying no `fwmark 0x1 ... lookup 100` rule must read as absent"
        );
    }

    /// A verbatim-shaped `nft -a list chain ip overdrive-mtls prerouting` dump
    /// with the F5 exemption (rendered `0x00000002`) at the head followed by
    /// two per-virt tproxy rules, each carrying a trailing `# handle <N>`.
    const CHAIN_DUMP: &str = "\
table ip overdrive-mtls {
\tchain prerouting { # handle 1
\t\ttype filter hook prerouting priority mangle; policy accept;
\t\tmeta mark 0x00000002 accept # handle 2
\t\tip daddr 127.0.0.5 tcp dport 18555 tproxy to 127.0.0.1:36533 meta mark set 0x00000001 accept # handle 3
\t\tip daddr 127.0.0.6 tcp dport 18666 tproxy to 127.0.0.1:36533 meta mark set 0x00000001 accept # handle 9
\t}
}";

    #[test]
    fn exemption_detected_against_nft_zero_padded_rendering() {
        // The exact bug the (b)-refined model first hit: nft renders the mark
        // `0x00000002`, NOT `0x2`/`2`. The dedup check MUST recognise nft's
        // canonical form, else the exemption stacks on every install.
        assert!(
            dump_has_leg_s_exemption(CHAIN_DUMP),
            "the F5 `meta mark 0x00000002 accept` exemption must be detected in nft's canonical rendering"
        );
    }

    #[test]
    fn exemption_absent_when_chain_has_only_tproxy_rules() {
        let no_exemption = "\
table ip overdrive-mtls {
\tchain prerouting { # handle 1
\t\ttype filter hook prerouting priority mangle; policy accept;
\t\tip daddr 127.0.0.5 tcp dport 18555 tproxy to 127.0.0.1:36533 meta mark set 0x00000001 accept # handle 3
\t}
}";
        assert!(
            !dump_has_leg_s_exemption(no_exemption),
            "a chain with only tproxy rules (set-mark, not match-mark) must NOT be read as carrying the exemption"
        );
    }

    #[test]
    fn handle_parsed_from_trailing_handle_marker() {
        // Each per-virt rule line is matched by its daddr/dport in production;
        // here we pin that the trailing `# handle <N>` yields the right N for
        // distinct rules so two installs capture distinct handles.
        let line_a = CHAIN_DUMP
            .lines()
            .find(|l| l.contains("127.0.0.5") && l.contains("18555"))
            .expect("virt_a rule line present");
        let line_b = CHAIN_DUMP
            .lines()
            .find(|l| l.contains("127.0.0.6") && l.contains("18666"))
            .expect("virt_b rule line present");
        assert_eq!(parse_handle(line_a), Some(3), "virt_a rule handle must parse to 3");
        assert_eq!(parse_handle(line_b), Some(9), "virt_b rule handle must parse to 9");
    }

    #[test]
    fn handle_parse_rejects_a_line_with_no_handle_marker() {
        let header = "\t\ttype filter hook prerouting priority mangle; policy accept;";
        assert_eq!(parse_handle(header), None, "a line with no `# handle` marker yields None");
    }

    // --- egress (`install_outbound_tproxy`) dump-parse helpers ---
    //
    // The egress rule differs from the inbound one ONLY in its match: it has
    // NO `ip daddr` / `tcp dport` (the workload's destination is unknown at
    // install — per-flow orig-dst recovery is 03-02), so it matches on the
    // ingress interface `iifname "<host_veth>"` and TPROXY-redirects ALL of
    // the workload's egress TCP to the agent's leg-F listener. The dedup
    // predicate keys on `(host_veth, agent_leg_f_port)` — both the ingress
    // interface AND the leg-F redirect target on the same line — because the
    // egress rule has no `ip daddr` / `tcp dport` of its own to distinguish a
    // repeat install for the same veth from a fresh one; a presence-check on
    // both conjuncts is what skips a literal-duplicate append, which the
    // inbound (distinct daddr/dport per virt) does not need.

    /// A verbatim-shaped `nft -a list chain ip overdrive-mtls prerouting` dump
    /// carrying the F5 exemption at the head, ONE inbound per-virt tproxy rule,
    /// and TWO egress (iifname-matched) tproxy rules for distinct host veths —
    /// each rendered as nft renders it (quoted iifname, zero-padded
    /// `0x00000001` set-mark, trailing `# handle <N>`).
    const EGRESS_CHAIN_DUMP: &str = "\
table ip overdrive-mtls {
\tchain prerouting { # handle 1
\t\ttype filter hook prerouting priority mangle; policy accept;
\t\tmeta mark 0x00000002 accept # handle 2
\t\tip daddr 127.0.0.5 tcp dport 18555 tproxy to 127.0.0.1:36533 meta mark set 0x00000001 accept # handle 3
\t\tiifname \"ovh-aaaa0\" meta l4proto tcp tproxy to 127.0.0.1:41000 meta mark set 0x00000001 accept # handle 7
\t\tiifname \"ovh-bbbb1\" meta l4proto tcp tproxy to 127.0.0.1:41000 meta mark set 0x00000001 accept # handle 12
\t}
}";

    #[test]
    fn egress_rule_shape_is_recognised_and_handle_parsed_in_shared_chain_dump() {
        // Headline (RED_ACCEPTANCE-level) scenario for this default-lane step.
        // This test exercises ONLY the pure predicates against a static fixture
        // — it does NOT call `install_outbound_tproxy` and proves no append (the
        // orchestration that wires ensure → presence-check → append → handle-
        // recover shells out and is the Tier-3 03-03 obligation, the symmetric
        // companion to inbound AC2). What it pins default-lane: the egress rule
        // that `install_outbound_tproxy(host_veth, port)` appends to the SHARED
        // `prerouting` chain has the design-pinned shape — `iifname
        // "<host_veth>" ... tproxy to 127.0.0.1:<port> ...` — and is recognised
        // in the chain dump, with its kernel-assigned handle parsed off the
        // trailing `# handle <N>`. The dedup/teardown MECHANICS (the predicates
        // that DRIVE the skip-append and by-handle-delete decisions) are proven
        // here; the real kernel CAPTURE is Tier-3 03-03.
        assert!(
            dump_has_egress_rule(EGRESS_CHAIN_DUMP, "ovh-aaaa0", 41000),
            "the egress rule appended for host_veth `ovh-aaaa0` → 127.0.0.1:41000 must be \
             recognised in the shared-chain dump (iifname match + redirect)"
        );
        assert_eq!(
            find_egress_rule_handle_in_dump(EGRESS_CHAIN_DUMP, "ovh-aaaa0", 41000),
            Some(7),
            "the egress rule's kernel-assigned handle must parse off the trailing `# handle 7`"
        );
    }

    #[test]
    fn egress_rule_present_only_for_its_own_host_veth() {
        // Idempotency presence-check: a chain that ALREADY carries this veth's
        // egress rule reads as present (so re-install skips the append); a
        // chain WITHOUT it reads as absent (so the first install appends).
        assert!(
            dump_has_egress_rule(EGRESS_CHAIN_DUMP, "ovh-bbbb1", 41000),
            "ovh-bbbb1's egress rule IS in the dump → present → re-install must skip"
        );
        let no_egress = "\
table ip overdrive-mtls {
\tchain prerouting { # handle 1
\t\ttype filter hook prerouting priority mangle; policy accept;
\t\tmeta mark 0x00000002 accept # handle 2
\t\tip daddr 127.0.0.5 tcp dport 18555 tproxy to 127.0.0.1:36533 meta mark set 0x00000001 accept # handle 3
\t}
}";
        assert!(
            !dump_has_egress_rule(no_egress, "ovh-aaaa0", 41000),
            "a chain with no egress rule for ovh-aaaa0 must read as absent → first install appends"
        );
    }

    #[test]
    fn egress_rule_requires_iifname_and_redirect_to_match_the_same_rule() {
        // Discriminating case that KILLS the `&&`→`||` and wrong-needle mutants
        // on the egress predicate. Line A carries OUR iifname but a DIFFERENT
        // redirect target (41999, not 41000); line B carries OUR redirect but a
        // DIFFERENT iifname (ovh-other2). Under correct `&&` neither qualifies
        // for (ovh-aaaa0, 41000): false. Under the `||` mutant, line A
        // satisfies the iifname conjunct and line B satisfies the redirect
        // conjunct → the mutant wrongly returns true and a duplicate is left
        // unappended (or, on the handle path, the wrong handle recovered).
        let cross = "\
table ip overdrive-mtls {
\tchain prerouting { # handle 1
\t\tiifname \"ovh-aaaa0\" meta l4proto tcp tproxy to 127.0.0.1:41999 meta mark set 0x00000001 accept # handle 5
\t\tiifname \"ovh-other2\" meta l4proto tcp tproxy to 127.0.0.1:41000 meta mark set 0x00000001 accept # handle 6
\t}
}";
        assert!(
            !dump_has_egress_rule(cross, "ovh-aaaa0", 41000),
            "no single line both matches iifname `ovh-aaaa0` AND redirects to 127.0.0.1:41000; \
             the rule is absent and the predicate must return false (the `||` mutant would \
             wrongly report it present and skip the needed append)"
        );
        assert_eq!(
            find_egress_rule_handle_in_dump(cross, "ovh-aaaa0", 41000),
            None,
            "with no line matching BOTH conjuncts, no handle is recoverable for (ovh-aaaa0, 41000)"
        );
    }

    #[test]
    fn egress_handle_parsed_per_host_veth_in_a_multi_rule_chain() {
        // Handle-recovery: distinct host veths in a multi-rule fixture yield
        // distinct handles, so two egress installs capture distinct handles
        // and each guard's Drop deletes EXACTLY its own rule.
        assert_eq!(
            find_egress_rule_handle_in_dump(EGRESS_CHAIN_DUMP, "ovh-aaaa0", 41000),
            Some(7),
            "ovh-aaaa0's egress rule handle must parse to 7"
        );
        assert_eq!(
            find_egress_rule_handle_in_dump(EGRESS_CHAIN_DUMP, "ovh-bbbb1", 41000),
            Some(12),
            "ovh-bbbb1's egress rule handle must parse to 12"
        );
    }

    #[test]
    fn egress_handle_path_yields_none_for_a_matching_line_without_a_handle_marker() {
        // T1: pins the handle-recovery contract for a line that matches BOTH
        // `iifname "ovh-aaaa0"` AND the `tproxy to 127.0.0.1:41000` redirect but
        // carries NO trailing `# handle <N>` marker (e.g. an `nft list chain`
        // dump taken WITHOUT `-a`, or a truncated capture). The handle path must
        // read `None` — there is no kernel-assigned handle to recover — while
        // the presence-check `dump_has_egress_rule` (which does NOT require the
        // marker) still reads `true` for the SAME line. This distinguishes the
        // two predicates: presence = `iifname` + `redirect`; handle-recovery =
        // presence + a recoverable `# handle <N>`. (Note: the `# handle `
        // conjunct in the `find_egress_rule_handle_in_dump` filter is
        // belt-and-suspenders with the downstream `parse_handle`, which is
        // itself a `# handle ` guard — so this test pins the observable
        // None-on-marker-less CONTRACT, not an independent mutant kill of the
        // conjunct; the conjunct cannot diverge from `parse_handle` while
        // `parse_handle` stays the handle extractor.)
        let no_handle = "\
table ip overdrive-mtls {
\tchain prerouting {
\t\tiifname \"ovh-aaaa0\" meta l4proto tcp tproxy to 127.0.0.1:41000 meta mark set 0x00000001 accept
\t}
}";
        assert_eq!(
            find_egress_rule_handle_in_dump(no_handle, "ovh-aaaa0", 41000),
            None,
            "a matching egress line with no `# handle <N>` marker yields no recoverable handle"
        );
        assert!(
            dump_has_egress_rule(no_handle, "ovh-aaaa0", 41000),
            "the SAME marker-less line IS recognised as present by `dump_has_egress_rule` \
             (iifname + redirect, no marker required) — presence and handle-recovery are \
             distinct contracts"
        );
    }

    #[test]
    fn egress_predicate_does_not_mistake_an_inbound_daddr_rule_for_an_egress_rule() {
        // The inbound rule (ip daddr/tcp dport, NO iifname) must NOT be read as
        // any veth's egress rule — guards against an over-broad needle that
        // matches on the shared `tproxy to 127.0.0.1:<port>` tail alone.
        let inbound_only = "\
table ip overdrive-mtls {
\tchain prerouting { # handle 1
\t\tip daddr 127.0.0.5 tcp dport 18555 tproxy to 127.0.0.1:41000 meta mark set 0x00000001 accept # handle 3
\t}
}";
        assert!(
            !dump_has_egress_rule(inbound_only, "ovh-aaaa0", 41000),
            "an inbound daddr/dport rule (no iifname) must NOT be read as ovh-aaaa0's egress rule"
        );
    }

    // --- §5 boot-recovery sweep classifier (`per_workload_rule_handles_in_dump`) ---
    //
    // The sweep is port-BLIND (a restart loses the dead leg-C/leg-F ports, so the
    // port-keyed predicates above cannot drive it). The classifier walks the
    // shared-chain dump and collects the `# handle <N>` of every per-workload
    // TPROXY rule (egress `iifname`-matched AND inbound `daddr`/`dport`-matched),
    // recognising both by the `tproxy to ` redirect they share, while KEEPING the
    // shared infra (chain header, type/policy line, and the F5 `meta mark … accept`
    // exemption — none of which carry `tproxy to `). This is the §5 mutation
    // target: pinned against the verbatim fixtures the egress/inbound tests reuse.

    #[test]
    fn classifier_collects_every_per_workload_handle_and_no_shared_infra_handle() {
        // `EGRESS_CHAIN_DUMP` = F5 exemption (# handle 2) + chain header
        // (# handle 1) + ONE inbound rule (# handle 3) + TWO egress rules
        // (# handle 7, # handle 12). The classifier must yield EXACTLY the three
        // per-workload handles {3, 7, 12} and NEVER the chain-header (1) or
        // exemption (2) handle.
        let mut handles = per_workload_rule_handles_in_dump(EGRESS_CHAIN_DUMP);
        handles.sort_unstable();
        assert_eq!(
            handles,
            vec![3, 7, 12],
            "the classifier must collect every per-workload (egress + inbound) handle and \
             NEVER the chain-header (1) or F5-exemption (2) handle"
        );
        assert!(
            !handles.contains(&1),
            "the chain-header `# handle 1` must NEVER be swept (it is the chain itself, not a rule)"
        );
        assert!(
            !handles.contains(&2),
            "the F5 exemption `# handle 2` must NEVER be swept (it is shared infra)"
        );
    }

    #[test]
    fn classifier_collects_both_inbound_per_workload_handles() {
        // `CHAIN_DUMP` = F5 exemption (# handle 2) + chain header (# handle 1) +
        // TWO inbound rules (# handle 3, # handle 9). The classifier recognises
        // inbound rules by the SAME `tproxy to ` redirect, so it must yield
        // {3, 9} — proving it is not egress-only (which would miss the
        // #241-forward inbound survivor the sweep must also cover).
        let mut handles = per_workload_rule_handles_in_dump(CHAIN_DUMP);
        handles.sort_unstable();
        assert_eq!(
            handles,
            vec![3, 9],
            "the classifier must collect inbound (`ip daddr`/`tcp dport`) per-workload handles too, \
             not only egress — both share the `tproxy to` redirect that distinguishes a rule \
             from the F5 exemption"
        );
    }

    #[test]
    fn classifier_is_a_noop_on_a_chain_with_only_shared_infra() {
        // A chain carrying ONLY the shared infra (chain header, type/policy line,
        // F5 exemption) — no per-workload TPROXY rule — must yield ZERO handles,
        // so the sweep is an idempotent no-op (the re-run / clean-boot case).
        let infra_only = "\
table ip overdrive-mtls {
\tchain prerouting { # handle 1
\t\ttype filter hook prerouting priority mangle; policy accept;
\t\tmeta mark 0x00000002 accept # handle 2
\t}
}";
        assert!(
            per_workload_rule_handles_in_dump(infra_only).is_empty(),
            "a chain carrying only shared infra (header + policy + F5 exemption) must yield NO \
             sweepable handles → the sweep is an idempotent no-op"
        );
    }

    #[test]
    fn classifier_does_not_collect_a_per_workload_line_lacking_a_handle_marker() {
        // A `tproxy to` rule line WITHOUT a trailing `# handle <N>` (e.g. a dump
        // taken without `-a`, or truncated) yields no handle — there is nothing
        // to delete by handle. KILLS a mutant that would collect a sentinel /
        // panic on a marker-less rule line.
        let no_handle = "\
table ip overdrive-mtls {
\tchain prerouting { # handle 1
\t\tmeta mark 0x00000002 accept # handle 2
\t\tiifname \"ovh-aaaa0\" meta l4proto tcp tproxy to 127.0.0.1:41000 meta mark set 0x00000001 accept
\t}
}";
        assert!(
            per_workload_rule_handles_in_dump(no_handle).is_empty(),
            "a `tproxy to` rule with NO trailing `# handle <N>` marker yields no sweepable handle \
             (nothing to delete by handle); the chain-header/exemption handles are still excluded"
        );
    }

    // --- REV-5 OUTPUT divert rule parse + teardown classifier widening ---
    //
    // The output divert rule carries NO `tproxy to` redirect — it is
    // `ip daddr <vip> tcp dport <vport> meta mark != 0x2 meta mark set 0x1
    // accept`. Its handle is recovered by `output_divert_handle_in_dump`
    // (the install's by-handle teardown source), and the boot-sweep classifier
    // `per_workload_rule_handles_in_dump` is widened to ALSO collect it (else it
    // leaks across every restart — the D2 class reopened). The head leg-S
    // exemption (`meta mark 0x00000002 accept` — a MATCH, no `set`) must stay
    // OUT of both. Pinned here against a verbatim-shaped `output` chain dump.

    /// A verbatim-shaped `nft -a list chain ip overdrive-mtls output` dump: the
    /// leg-S exemption (`meta mark 0x00000002 accept`, a MATCH) at the head,
    /// then TWO per-virt OUTPUT divert rules for distinct virts — each rendered
    /// as nft renders it (`meta mark != 0x00000002 meta mark set 0x00000001`,
    /// NO `tproxy to`, trailing `# handle <N>`).
    const OUTPUT_CHAIN_DUMP: &str = "\
table ip overdrive-mtls {
\tchain output { # handle 1
\t\ttype route hook output priority mangle; policy accept;
\t\tmeta mark 0x00000002 accept # handle 2
\t\tip daddr 10.99.0.2 tcp dport 8080 meta mark != 0x00000002 meta mark set 0x00000001 accept # handle 4
\t\tip daddr 10.99.0.3 tcp dport 9090 meta mark != 0x00000002 meta mark set 0x00000001 accept # handle 8
\t}
}";

    #[test]
    fn output_divert_handle_recovered_per_virt_and_exemption_excluded() {
        use std::net::{Ipv4Addr, SocketAddrV4};
        // Distinct virts recover distinct handles — so each inbound install's
        // guard deletes EXACTLY its own output divert rule. The head exemption
        // (a `meta mark … accept` MATCH with no `ip daddr`/`set`) is NEVER
        // recovered as a divert rule.
        let virt_a = SocketAddrV4::new(Ipv4Addr::new(10, 99, 0, 2), 8080);
        let virt_b = SocketAddrV4::new(Ipv4Addr::new(10, 99, 0, 3), 9090);
        assert_eq!(
            output_divert_handle_in_dump(OUTPUT_CHAIN_DUMP, virt_a),
            Some(4),
            "virt_a (10.99.0.2:8080)'s output divert rule handle must parse to 4"
        );
        assert_eq!(
            output_divert_handle_in_dump(OUTPUT_CHAIN_DUMP, virt_b),
            Some(8),
            "virt_b (10.99.0.3:9090)'s output divert rule handle must parse to 8"
        );
        // A virt with the right daddr but a DIFFERENT dport must not match
        // virt_a's rule (kills a daddr-only / dport-dropped needle mutant).
        let wrong_port = SocketAddrV4::new(Ipv4Addr::new(10, 99, 0, 2), 9999);
        assert_eq!(
            output_divert_handle_in_dump(OUTPUT_CHAIN_DUMP, wrong_port),
            None,
            "a virt sharing daddr 10.99.0.2 but a different dport (9999) must NOT match \
             the 8080 divert rule — both daddr AND dport conjuncts are required"
        );
    }

    #[test]
    fn output_divert_parse_requires_meta_mark_set_not_the_exemption_match() {
        use std::net::{Ipv4Addr, SocketAddrV4};
        // A chain whose ONLY `ip daddr`/`tcp dport` line is a `meta mark …
        // accept` MATCH (no `set`) — i.e. a hypothetical exemption-shaped line —
        // must yield NO divert handle: the `meta mark set ` conjunct (a SET) is
        // what distinguishes a divert rule from a match. KILLS a mutant that
        // drops the `meta mark set ` conjunct (which would mis-recover the
        // exemption's handle and tear down the shared infra).
        let match_only = "\
table ip overdrive-mtls {
\tchain output { # handle 1
\t\tip daddr 10.99.0.2 tcp dport 8080 meta mark 0x00000002 accept # handle 2
\t}
}";
        let virt = SocketAddrV4::new(Ipv4Addr::new(10, 99, 0, 2), 8080);
        assert_eq!(
            output_divert_handle_in_dump(match_only, virt),
            None,
            "a `meta mark … accept` MATCH line (no `set`) must NOT be recovered as a divert rule \
             — the `meta mark set ` conjunct is the divert discriminator"
        );
    }

    #[test]
    fn classifier_collects_output_divert_rules_and_never_the_output_exemption() {
        // The widened §5 sweep classifier must collect every OUTPUT divert rule
        // (which carries NO `tproxy to`) by its `ip daddr` + `meta mark set` +
        // `tcp dport` shape, while KEEPING the chain header (# handle 1) and the
        // leg-S exemption (# handle 2, a `meta mark … accept` MATCH). Without the
        // widening the divert rules leak across every restart (D2 reopened);
        // over-broadening to collect the exemption would tear down shared infra.
        let mut handles = per_workload_rule_handles_in_dump(OUTPUT_CHAIN_DUMP);
        handles.sort_unstable();
        assert_eq!(
            handles,
            vec![4, 8],
            "the classifier must collect both OUTPUT divert rules (# handle 4, 8) and NEVER the \
             chain-header (1) or leg-S-exemption (2) handle"
        );
        assert!(
            !handles.contains(&2),
            "the output-chain leg-S exemption (`meta mark 0x00000002 accept`, a MATCH not a SET) \
             must NEVER be swept — the `meta mark set ` token excludes it"
        );
    }

    #[test]
    fn classifier_output_divert_branch_requires_all_three_conjuncts() {
        // KILLS the two `&& -> ||` mutants on the REV-5 OUTPUT-divert recognition
        // branch (`ip daddr ` && `meta mark set ` && `tcp dport `): a line
        // satisfying SOME but not ALL three conjuncts must be EXCLUDED. Under the
        // correct `&&` every line below is rejected (none has `tproxy to ` AND
        // none satisfies the full conjunction); flipping either `&&` to `||`
        // would wrongly collect a partial-match line and tear down shared infra
        // / a non-divert rule.
        //
        //   - handle 2: leg-S exemption — `meta mark … accept` MATCH (no `set`,
        //     no `ip daddr`/`tcp dport`). Already covered, kept as the baseline.
        //   - handle 3: `ip daddr ` + `tcp dport ` but NO `meta mark set ` (a
        //     hypothetical non-divert filter rule). Correct `&&`: excluded
        //     (missing the `set` conjunct). `&& -> ||` at the `meta mark set `
        //     position: WRONGLY collected.
        //   - handle 4: `meta mark set ` + `tcp dport ` but NO `ip daddr `.
        //     Correct `&&`: excluded. `&& -> ||` at the `ip daddr ` position:
        //     WRONGLY collected.
        //   - handle 5: `ip daddr ` + `meta mark set ` but NO `tcp dport `.
        //     Correct `&&`: excluded. `&& -> ||` at the `tcp dport ` position:
        //     WRONGLY collected.
        let partial_conjuncts = "\
table ip overdrive-mtls {
\tchain output { # handle 1
\t\tmeta mark 0x00000002 accept # handle 2
\t\tip daddr 10.99.0.2 tcp dport 8080 meta mark != 0x00000002 accept # handle 3
\t\tmeta mark set 0x00000001 tcp dport 8080 accept # handle 4
\t\tip daddr 10.99.0.3 meta mark set 0x00000001 accept # handle 5
\t}
}";
        assert!(
            per_workload_rule_handles_in_dump(partial_conjuncts).is_empty(),
            "no line satisfies all THREE conjuncts (ip daddr AND meta mark set AND tcp dport) and \
             none carries `tproxy to ` — every line must be EXCLUDED; collecting any partial-match \
             line is the `&& -> ||` mutant (would sweep shared infra / a non-divert rule)"
        );
    }

    // --- §5 sweep absent-chain classifier (`stderr_reports_absent_chain`) ---
    //
    // The §5 boot sweep treats the shared table/chain being ABSENT as the benign
    // "nothing to sweep" signal (`Ok(0)`), distinct from a genuine `nft` failure
    // (binary missing, EPERM, transient lock) which must propagate and refuse the
    // boot (fail-CLOSED, matching the by-handle delete path). The discriminator is
    // this pure classifier over `nft`'s stderr — pinned here without a kernel.

    #[test]
    fn stderr_reports_absent_chain_classifies_absent_vs_genuine_failures() {
        use super::stderr_reports_absent_chain;
        // ABSENT-table / absent-chain shapes nft emits when the table or chain
        // does not exist — the benign fresh-boot "nothing to sweep" case → true.
        for absent in [
            "Error: No such file or directory",
            "Error: No such file or directory\nlist chain ip overdrive-mtls prerouting\n      ^^^^^^^^^^^^",
            "Error: chain `prerouting` does not exist in table `overdrive-mtls`",
            "table `overdrive-mtls` does not exist",
        ] {
            assert!(
                stderr_reports_absent_chain(absent),
                "absent-table/chain stderr must classify as absent (→ Ok(0), nothing to sweep): {absent:?}"
            );
        }
        // GENUINE failures — these are NOT "absent"; the sweep must propagate them
        // and refuse the boot. KILLS a mutant that flips the predicate to a
        // constant `true` (which would re-open the swallow the fix closes).
        for genuine in [
            "Error: Operation not permitted",
            "nft: command not found",
            "Error: Could not process rule: Resource temporarily unavailable",
            "",
        ] {
            assert!(
                !stderr_reports_absent_chain(genuine),
                "a genuine nft failure (EPERM / missing binary / transient lock / empty) must NOT \
                 classify as absent — it propagates and refuses the boot: {genuine:?}"
            );
        }
    }

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
