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
//! `mtls/inbound.rs`, #178-deferred — NOT touched here).

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
    /// `accept_inbound_leg` / `accept_outbound_leg` could not accept the
    /// redirected connection on the intercept listener.
    #[error("leg accept failed on the {direction} intercept listener: {source}")]
    Accept {
        /// `"inbound"` or `"outbound"` — which intercept listener accept failed on.
        direction: &'static str,
        /// The originating accept error.
        #[source]
        source: std::io::Error,
    },
    /// `accept_inbound_leg` could not recover the original destination via
    /// `getsockname` on the TPROXY-redirected accepted leg.
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

/// Install the inbound nft-TPROXY prerouting intercept for ONE `virt`.
///
/// Appends exactly one per-virt TPROXY rule to a SHARED `prerouting` chain;
/// the returned guard's `Drop` removes ONLY that rule by its kernel-assigned
/// handle. The shared routing infrastructure — the `ip rule fwmark` policy
/// rule, the `ip route local … table` loopback route, the nft table + chain,
/// and the F5 `MTLS_LEG_S_DIAL_MARK` exemption at the chain head — is
/// node-global converge-on-boot state ensured idempotently here (created
/// once, NEVER torn down per-workload) so multiple concurrent inbound
/// intercepts coexist without razing one another (kernel-canonical TPROXY /
/// Cilium host-netns model — research
/// `multi-workload-tproxy-interception-resource-model-research.md` F1/F5/F6/F7;
/// converge-on-boot Bar-1 per `.claude/rules/reconcilers.md`).
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
/// orig-dst verbatim, #178).
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

    // (3) Recover the kernel-assigned handle of the rule we just appended so
    // Drop can delete EXACTLY that rule (siblings, the exemption, and the
    // shared infra all untouched) — research F7c, the nft-canonical per-rule
    // teardown.
    let handle = find_virt_rule_handle(virt, agent_port)?;
    Ok(TproxyInterceptGuard { handle })
}

/// Ensure the SHARED node-global TPROXY routing infrastructure exists,
/// idempotently (add-if-missing). Converge-on-boot Bar-1: a pre-existing
/// component is the success case, not an error — so two concurrent installs
/// (and a re-install after a prior run) both leave exactly one of each shared
/// resource, never a stacked pile.
///
/// Components (all node-global, none removed on per-workload Drop):
///   - `ip rule fwmark 0x1 lookup 100` — routes fwmark-stamped packets via the
///     local table.
///   - `ip route local 0.0.0.0/0 dev lo table 100` — delivers them to a local
///     socket (leg C) instead of forwarding.
///   - nft table `overdrive-mtls` + `prerouting` chain.
///   - the F5 `meta mark <MTLS_LEG_S_DIAL_MARK> accept` exemption, inserted at
///     the chain HEAD exactly once (must precede all per-virt tproxy rules).
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

    // F5 exemption at the chain head — insert ONCE. `nft insert` prepends, so
    // guarding against a duplicate add keeps it exactly once at the head ahead
    // of every per-virt tproxy rule.
    if !chain_has_leg_s_exemption()? {
        let leg_s_mark = overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
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
/// duplicate per install).
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
    let needle_hex = format!("fwmark {mark:#x}");
    let needle_dec = format!("fwmark {mark}");
    let lookup = format!("lookup {table}");
    text.lines()
        .any(|l| (l.contains(&needle_hex) || l.contains(&needle_dec)) && l.contains(&lookup))
}

/// True iff the shared `prerouting` chain already carries the F5
/// `meta mark <MTLS_LEG_S_DIAL_MARK> accept` exemption — used so the exemption
/// is inserted exactly once at the chain head (otherwise every install would
/// prepend another duplicate).
fn chain_has_leg_s_exemption() -> Result<bool> {
    Ok(dump_has_leg_s_exemption(&list_chain()?))
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

/// `nft -a list chain ip <table> <chain>` (with handles). Returns the dump on
/// success; maps a spawn / non-zero exit to [`InterceptError::TproxyInstall`].
fn list_chain() -> Result<String> {
    let out = Command::new("nft")
        .args(["-a", "list", "chain", "ip", NFT_TABLE, NFT_CHAIN])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| InterceptError::TproxyInstall {
            reason: format!("spawn nft list chain: {e}"),
        })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(InterceptError::TproxyInstall {
            reason: format!(
                "nft -a list chain ip {NFT_TABLE} {NFT_CHAIN} exited {:?}: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        })
    }
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

/// Extract the `<N>` from a trailing `# handle <N>` on an `nft -a` rule line.
fn parse_handle(line: &str) -> Option<u64> {
    let (_, after) = line.rsplit_once("# handle ")?;
    after.split_whitespace().next()?.parse::<u64>().ok()
}

/// RAII guard removing ONLY this virt's per-virt TPROXY rule on `Drop`.
///
/// Deletes the rule by its kernel-assigned handle. The shared routing infra —
/// `ip rule`, `ip route`, nft table/chain, and the F5 exemption — is
/// node-global and is NOT removed here; sibling intercepts' rules are
/// untouched.
pub struct TproxyInterceptGuard {
    /// The kernel-assigned handle of this virt's TPROXY rule in the shared
    /// `prerouting` chain. Drop deletes exactly this one rule.
    handle: u64,
}

impl Drop for TproxyInterceptGuard {
    fn drop(&mut self) {
        // Delete ONLY this virt's rule by handle (research F7c). `.output()`
        // (not `.status()`) drains the child reliably under the nextest
        // harness — see the D5 root-cause note on `run_best_effort`.
        let handle = self.handle.to_string();
        let _ = run_best_effort(&svec(&[
            "nft", "delete", "rule", "ip", NFT_TABLE, NFT_CHAIN, "handle", &handle,
        ]));
    }
}

/// Accept the redirected OUTBOUND workload connection on the agent's leg-F
/// listener.
///
/// Builds [`InterceptedConnection`] (`Routed::Outbound { peer }`); the owned
/// leg F is handed by value. Productionises `roles.rs::accept_leg_f`.
///
/// # Errors
///
/// Returns [`InterceptError::Accept`] if the leg-F accept fails.
pub fn accept_outbound_leg(
    leg_f_listener: &std::net::TcpListener,
    alloc: AllocationId,
    peer: SocketAddrV4,
) -> Result<InterceptedConnection> {
    let (leg_f, _peer) = leg_f_listener
        .accept()
        .map_err(|source| InterceptError::Accept { direction: "outbound", source })?;
    leg_f.set_nodelay(true).ok();
    Ok(InterceptedConnection {
        leg: OwnedFd::from(leg_f),
        routed: Routed::Outbound { peer },
        alloc,
        // v1 = authn-only (F5 / #178): the expected-peer SAN-match is
        // supplied downstream by east-west SPIFFE-ID resolution, never here.
        expected_peer: None,
    })
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

    use super::{dump_has_leg_s_exemption, parse_handle};

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
}
