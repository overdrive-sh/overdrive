//! RAII veth-pair fixture for Tier 3 integration tests.
//!
//! Creates a `veth0` ↔ `veth1` peer pair via `ip(8)`, brings both ends
//! up, and tears the pair down on `Drop` regardless of test outcome.
//! Idempotent on `create()` — best-effort cleanup of leftover state from
//! a prior aborted run before issuing the `add` command.
//!
//! Capability gating: `ip link add … type veth` requires
//! `CAP_NET_ADMIN`. Unprivileged callers receive
//! [`VethError::CapNetAdminRequired`]; the test caller is expected to
//! return early with a skip message rather than panic, matching the
//! capability-gating convention in
//! `crates/overdrive-worker/tests/integration/exec_driver/`.

#![cfg(target_os = "linux")]
#![allow(clippy::expect_used)]

use std::process::{Command, Output};

/// Errors from veth-pair lifecycle. Distinct variants per
/// `.claude/rules/development.md` § Errors so the test caller can branch
/// on capability vs setup failure.
#[derive(Debug)]
pub enum VethError {
    /// `ip(8)` rejected the operation with EPERM/EACCES — the running
    /// process lacks `CAP_NET_ADMIN`. Tests skip rather than fail.
    CapNetAdminRequired,
    /// `ip(8)` failed for any other reason (binary missing, kernel
    /// rejection, peer-name conflict, …). Carries stderr for diagnosis.
    IpCommand { args: String, stderr: String, status: Option<i32> },
    /// Spawning `ip(8)` itself failed — typically the binary is not on
    /// `$PATH`. Distinct from `IpCommand` so the diagnostic can name the
    /// underlying I/O cause directly.
    Spawn(std::io::Error),
}

impl std::fmt::Display for VethError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CapNetAdminRequired => {
                f.write_str("ip(8) returned EPERM/EACCES — CAP_NET_ADMIN required")
            }
            Self::IpCommand { args, stderr, status } => {
                write!(f, "ip {args} failed (status={status:?}): {}", stderr.trim())
            }
            Self::Spawn(e) => write!(f, "ip(8) spawn failed: {e}"),
        }
    }
}

impl std::error::Error for VethError {}

/// RAII veth pair. `Drop` issues a best-effort `ip link del` on the
/// host-side end; the kernel automatically removes the peer when the
/// pair end is destroyed.
pub struct VethPair {
    /// Host-side endpoint — XDP attaches here.
    pub host: String,
    /// Peer endpoint — frames are injected here.
    pub peer: String,
}

impl VethPair {
    /// Create a fresh veth pair with the supplied names. If either
    /// endpoint already exists from a prior aborted run, tear it down
    /// first (best-effort) before issuing the add command. Brings both
    /// ends `up` so XDP attach and packet sendto both succeed.
    pub fn create(host: &str, peer: &str) -> Result<Self, VethError> {
        // Best-effort cleanup of leftover state. `ip link del` on a
        // missing iface returns non-zero — ignored. We only care about
        // the post-condition: neither name resolves.
        let _ = Command::new("ip").args(["link", "del", host]).output();
        let _ = Command::new("ip").args(["link", "del", peer]).output();

        run_ip(["link", "add", host, "type", "veth", "peer", "name", peer])?;
        run_ip(["link", "set", host, "up"])?;
        run_ip(["link", "set", peer, "up"])?;

        Ok(Self { host: host.to_owned(), peer: peer.to_owned() })
    }
}

impl VethPair {
    /// Set up FIB+ARP context so that `bpf_fib_lookup` against
    /// `backend_addr` from the host side returns
    /// `BPF_FIB_LKUP_RET_SUCCESS` with `ifindex == host's ifindex`
    /// (so `XDP_TX` is the optimal egress) and `dmac == peer's MAC`.
    ///
    /// Required by Phase 2.2 Slice 05-04+ tests that drive the
    /// `xdp_service_map_lookup` program: after Option α landed, the
    /// program calls `bpf_fib_lookup` between the L3+L4 rewrite and
    /// the `XDP_TX` return. Without a route + ARP entry covering the
    /// rewritten dst IP, the helper returns `RET_NOT_FWDED` /
    /// `RET_NO_NEIGH` and the program returns `XDP_PASS` (raw-socket
    /// capture on the peer side then misses the rewritten frame).
    ///
    /// Steps:
    ///  1. Assign `host_cidr` (e.g. `10.1.0.1/16`) to the host end —
    ///     creates an on-link route covering `backend_ip`.
    ///  2. Read the peer's MAC address from `/sys/class/net`.
    ///  3. Add a `permanent` neighbour entry mapping `backend_ip` →
    ///     `peer_mac` on `host` so the FIB lookup hits a populated
    ///     ARP cache on the first SYN.
    ///
    /// `host_cidr` MUST cover `backend_ip` on-link (e.g. `/16`
    /// covers `10.1.0.0/16` so a host_cidr of `10.1.0.1/16` makes
    /// `10.1.0.5` reachable directly via host).
    pub fn configure_for_xdp_tx_to_backend(
        &self,
        host_cidr: &str,
        backend_ip: std::net::Ipv4Addr,
    ) -> Result<(), VethError> {
        self.configure_for_xdp_tx_to_backends(host_cidr, &[backend_ip])
    }

    /// Multi-backend variant of [`Self::configure_for_xdp_tx_to_backend`].
    /// Assigns `host_cidr` once, then adds a permanent ARP entry for
    /// each `backend_ip` mapped to the peer's MAC. All `backend_ip`s
    /// must lie within the on-link prefix of `host_cidr`. The XDP
    /// program calls `bpf_fib_lookup` with `flags = 0`, which
    /// requires `net.ipv4.ip_forward=1` on the ingress iface to
    /// return success for non-local destinations.
    pub fn configure_for_xdp_tx_to_backends(
        &self,
        host_cidr: &str,
        backend_ips: &[std::net::Ipv4Addr],
    ) -> Result<(), VethError> {
        // `bpf_fib_lookup` (flags = 0) requires forwarding enabled on
        // the ingress iface to return success for non-local
        // destinations. Enable it globally — but DISABLE forwarding
        // on the peer iface so the kernel does not re-forward the
        // post-XDP_TX rewritten frame back through host (creating
        // an infinite peer↔host loop in the single-netns topology).
        // The XDP fast path on host is the only forward step we
        // want; once the rewritten frame arrives on peer's RX path,
        // it must reach the PF_PACKET capture and stop.
        run_sysctl("net.ipv4.ip_forward=1")?;
        run_sysctl(&format!("net.ipv4.conf.{}.forwarding=1", self.host))?;
        run_sysctl(&format!("net.ipv4.conf.{}.forwarding=0", self.peer))?;
        // rp_filter strict-mode would drop the rewritten frame.
        // Effective rp_filter = MAX(conf.all, conf.<iface>), so both
        // the global and per-interface values must be zeroed. The
        // veths already exist at this point — their per-iface values
        // inherited from `default` at creation time (typically 2 on
        // Ubuntu). Setting `all` and `default` alone is insufficient.
        run_sysctl("net.ipv4.conf.all.rp_filter=0")?;
        run_sysctl("net.ipv4.conf.default.rp_filter=0")?;
        run_sysctl(&format!("net.ipv4.conf.{}.rp_filter=0", self.host))?;
        run_sysctl(&format!("net.ipv4.conf.{}.rp_filter=0", self.peer))?;

        run_ip(["addr", "add", host_cidr, "dev", &self.host])?;
        let peer_mac = read_iface_mac(&self.peer)?;
        for backend_ip in backend_ips {
            run_ip([
                "neigh",
                "replace",
                &backend_ip.to_string(),
                "lladdr",
                &peer_mac,
                "dev",
                &self.host,
                "nud",
                "permanent",
            ])?;
        }
        Ok(())
    }
}

/// Read `/sys/class/net/<iface>/address` from the host netns.
fn read_iface_mac(iface: &str) -> Result<String, VethError> {
    let out = Command::new("cat")
        .arg(format!("/sys/class/net/{iface}/address"))
        .output()
        .map_err(VethError::Spawn)?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(VethError::IpCommand {
            args: format!("cat /sys/class/net/{iface}/address"),
            stderr,
            status: out.status.code(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl Drop for VethPair {
    fn drop(&mut self) {
        // Best-effort teardown — ignore exit status. The kernel removes
        // the peer when the host side is destroyed.
        let _ = Command::new("ip").args(["link", "del", &self.host]).output();
    }
}

/// Spawn `ip <args>`, classify the result. EPERM/EACCES on stderr (or
/// the `Operation not permitted` text the iproute2 wrapper prints) maps
/// to [`VethError::CapNetAdminRequired`]; other non-zero exits map to
/// [`VethError::IpCommand`].
fn run_ip<I, S>(args: I) -> Result<Output, VethError>
where
    I: IntoIterator<Item = S> + Clone,
    S: AsRef<std::ffi::OsStr>,
{
    let arg_str = args
        .clone()
        .into_iter()
        .map(|s| s.as_ref().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    let out = Command::new("ip").args(args).output().map_err(VethError::Spawn)?;
    if out.status.success() {
        return Ok(out);
    }
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    // iproute2 surfaces EPERM as "Operation not permitted" and EACCES
    // as "Permission denied". Either signal a missing CAP_NET_ADMIN.
    if stderr.contains("Operation not permitted") || stderr.contains("Permission denied") {
        return Err(VethError::CapNetAdminRequired);
    }
    Err(VethError::IpCommand { args: arg_str, stderr, status: out.status.code() })
}

fn run_sysctl(kv: &str) -> Result<(), VethError> {
    let out = Command::new("sysctl").args(["-w", kv]).output().map_err(VethError::Spawn)?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if stderr.contains("Operation not permitted") || stderr.contains("Permission denied") {
        return Err(VethError::CapNetAdminRequired);
    }
    Err(VethError::IpCommand { args: format!("sysctl -w {kv}"), stderr, status: out.status.code() })
}
