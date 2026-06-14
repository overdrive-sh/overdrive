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

use std::io::Write as _;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd, RawFd};
use std::process::{Command, Stdio};

use overdrive_core::AllocationId;
use overdrive_core::traits::mtls_enforcement::{InterceptedConnection, Routed};

/// `IP_TRANSPARENT` sockopt level value — libc 0.2 does not name it (same as
/// the proven `roles.rs::make_transparent_listener` reference).
const IP_TRANSPARENT: libc::c_int = 19;

/// The stable production nft table name for the inbound TPROXY intercept.
/// Single table for the worker's inbound mTLS intercept; the guard's `Drop`
/// removes it whole.
const NFT_TABLE: &str = "overdrive-mtls";

/// The fwmark the TPROXY rule stamps and the `ip rule` companion matches so
/// the redirected connection is routed via the `local` route table.
const TPROXY_FWMARK: u32 = 0x1;

/// The routing-policy table number the `ip rule fwmark` companion looks up
/// and the `ip route local … table` companion populates.
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

/// Install the inbound nft-TPROXY prerouting intercept.
///
/// Adds the `ip rule fwmark` / `ip route local … table` companions and
/// redirects a connection aimed at `virt` to the agent's leg-C listener on
/// `agent_port`, with the `MTLS_LEG_S_DIAL_MARK` exemption ordered first (F5
/// inbound). Returns a guard whose `Drop` removes the rule/route/table.
///
/// Productionises the PRODUCTION HALF of
/// `mtls_netns_topology.rs::install_tproxy` ONLY — the GAP-3 netns
/// DNAT/masquerade is TEST-ONLY and does NOT productionise (the adapter dials
/// orig-dst verbatim, #178).
///
/// # Errors
///
/// Returns [`InterceptError::TproxyInstall`] if any of the `ip rule add`,
/// `ip route add`, or `nft -f` commands fails.
pub fn install_inbound_tproxy(virt: SocketAddrV4, agent_port: u16) -> Result<TproxyInterceptGuard> {
    // Idempotent pre-clean of the GLOBAL rule/route/table a prior aborted run
    // (SIGKILL between install and Drop) may have leaked — global inbound
    // state is NOT netns-scoped and survives. Mirrors the reference
    // `preclean_global_inbound_state` discipline.
    preclean_global_inbound_state();

    // The reverse-cleanup argv set the guard runs on Drop, populated as each
    // forward step lands so a mid-install failure still tears down what was
    // already added.
    let mut cleanup: Vec<Vec<String>> = Vec::new();

    // ip rule: route fwmark-stamped (redirected) packets via the local table.
    let fwmark = TPROXY_FWMARK.to_string();
    let rt_table = TPROXY_RT_TABLE.to_string();
    run_ip(&["rule", "add", "fwmark", &fwmark, "lookup", &rt_table])?;
    cleanup.push(svec(&["ip", "rule", "del", "fwmark", &fwmark, "lookup", &rt_table]));

    // ip route: the local route table sends the redirected packet to a local
    // socket (leg C) rather than forwarding it.
    run_ip(&["route", "add", "local", "0.0.0.0/0", "dev", "lo", "table", &rt_table])?;
    cleanup.push(svec(&[
        "ip",
        "route",
        "del",
        "local",
        "0.0.0.0/0",
        "dev",
        "lo",
        "table",
        &rt_table,
    ]));

    // nft prerouting: the leg-S-dial-mark exemption (F5 inbound) MUST precede
    // the TPROXY rule so the agent's own marked dial is accepted before the
    // redirect can match it (otherwise the dial recurses back onto leg C).
    let leg_s_mark = overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
    let nft_prog = format!(
        "table ip {table} {{\n\
           chain prerouting {{\n\
             type filter hook prerouting priority mangle; policy accept;\n\
             meta mark {leg_s_mark} accept;\n\
             ip daddr {vip} tcp dport {vport} tproxy to 127.0.0.1:{aport} meta mark set {mark} accept;\n\
           }}\n\
         }}\n",
        table = NFT_TABLE,
        vip = virt.ip(),
        vport = virt.port(),
        aport = agent_port,
        mark = TPROXY_FWMARK,
    );
    // Push the table cleanup BEFORE applying so a failed `nft -f` (which may
    // have partially created the table) is still torn down by the guard.
    cleanup.push(svec(&["nft", "delete", "table", "ip", NFT_TABLE]));
    let guard = TproxyInterceptGuard { cleanup };
    apply_nft(&nft_prog)?;
    Ok(guard)
}

/// RAII guard removing the nft-TPROXY table + `ip rule` / `ip route` on `Drop`.
pub struct TproxyInterceptGuard {
    /// The reverse-cleanup argv set, run best-effort on `Drop`.
    cleanup: Vec<Vec<String>>,
}

impl Drop for TproxyInterceptGuard {
    fn drop(&mut self) {
        // Run in reverse-construction order: nft table first, then route,
        // then rule.
        for argv in self.cleanup.iter().rev() {
            let _ = run_best_effort(argv);
        }
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

/// Apply an nft program via `nft -f -`; map a non-zero exit (or spawn / write
/// failure) to [`InterceptError::TproxyInstall`].
fn apply_nft(prog: &str) -> Result<()> {
    let mut child = Command::new("nft")
        .arg("-f")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| InterceptError::TproxyInstall { reason: format!("nft not available: {e}") })?;
    child
        .stdin
        .take()
        .ok_or_else(|| InterceptError::TproxyInstall { reason: "nft stdin missing".into() })?
        .write_all(prog.as_bytes())
        .map_err(|e| InterceptError::TproxyInstall { reason: format!("nft write: {e}") })?;
    let out = child
        .wait_with_output()
        .map_err(|e| InterceptError::TproxyInstall { reason: format!("nft wait: {e}") })?;
    if out.status.success() {
        Ok(())
    } else {
        Err(InterceptError::TproxyInstall {
            reason: format!("nft -f failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
        })
    }
}

/// Idempotent pre-clean of the GLOBAL inbound state (fwmark rule, local
/// route, nft table) a prior aborted run may have leaked. Best-effort: a
/// failing command is the "nothing to clean" signal.
fn preclean_global_inbound_state() {
    let fwmark = TPROXY_FWMARK.to_string();
    let rt_table = TPROXY_RT_TABLE.to_string();
    // Up to 64 stacked fwmark rules from repeated aborted runs.
    for _ in 0..64 {
        let status =
            run_best_effort(&svec(&["ip", "rule", "del", "fwmark", &fwmark, "lookup", &rt_table]));
        if !matches!(status, Ok(s) if s.success()) {
            break;
        }
    }
    let _ = run_best_effort(&svec(&[
        "ip",
        "route",
        "del",
        "local",
        "0.0.0.0/0",
        "dev",
        "lo",
        "table",
        &rt_table,
    ]));
    let _ = run_best_effort(&svec(&["nft", "delete", "table", "ip", NFT_TABLE]));
}

/// Best-effort `Command` run used by cleanup paths — a missing rule / route /
/// table is the "already gone" signal, not an error.
fn run_best_effort(argv: &[String]) -> std::io::Result<std::process::ExitStatus> {
    Command::new(&argv[0]).args(&argv[1..]).stdout(Stdio::null()).stderr(Stdio::null()).status()
}

/// `&[&str]` → `Vec<String>` for the owned cleanup argv set.
fn svec(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_string()).collect()
}
