//! RAII Linux network-namespace fixture for Tier 3 integration tests.
//!
//! Creates a named netns via `ip netns add`, brings up `lo` inside it
//! (without `lo up` the in-namespace TCP stack rejects loopback
//! traffic that some `nc` flows touch), and tears the netns down on
//! `Drop` regardless of test outcome. Idempotent on `create()` —
//! best-effort cleanup of leftover state from a prior aborted run
//! before issuing the `add`.
//!
//! Capability gating mirrors [`super::veth::VethPair`]: requires
//! `CAP_NET_ADMIN`. Unprivileged callers receive
//! [`NetNsError::CapNetAdminRequired`]; the test caller is expected
//! to bail with a skip message rather than panic.
//!
//! # Two-namespace topology used by `reverse_nat_e2e.rs`
//!
//! ```text
//!   netns "<host_ns>"                netns "<peer_ns>"
//!     ┌──────────────┐                 ┌──────────────┐
//!     │   veth host  │ <──── pair ───> │   veth peer  │
//!     │  10.0.0.100  │                 │   10.1.0.5   │
//!     │  XDP svc-map │                 │  nc -l 9000  │
//!     │  TC reverse  │                 │              │
//!     └──────────────┘                 └──────────────┘
//! ```
//!
//! The veth peer is moved into `peer_ns` via `ip link set <peer>
//! netns <ns>` after the pair exists; the host end stays in
//! `host_ns`. Both ends are brought up inside their respective
//! namespaces (`ip netns exec <ns> ip link set ...`).

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc, clippy::expect_used)]

use std::process::Command;

/// Errors from netns lifecycle. Distinct variants per
/// `.claude/rules/development.md` § Errors so the test caller can
/// branch on capability vs setup failure.
#[derive(Debug)]
pub enum NetNsError {
    /// `ip(8)` rejected the operation with EPERM/EACCES — the
    /// running process lacks `CAP_NET_ADMIN`. Tests skip rather
    /// than fail.
    CapNetAdminRequired,
    /// `ip(8)` failed for any other reason.
    IpCommand { args: String, stderr: String, status: Option<i32> },
    /// Spawning `ip(8)` itself failed.
    Spawn(std::io::Error),
}

impl std::fmt::Display for NetNsError {
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

impl std::error::Error for NetNsError {}

/// RAII network-namespace handle. `Drop` issues a best-effort
/// `ip netns del`; the kernel reaps in-namespace ifaces and
/// processes when the namespace is destroyed.
pub struct NetNs {
    /// The netns name (also the path component under
    /// `/var/run/netns/`).
    pub name: String,
}

impl NetNs {
    /// Create a fresh netns. If a namespace with the same name lingers
    /// from a prior aborted run, tear it down (best-effort) before
    /// issuing the add. Brings up `lo` inside the new namespace —
    /// otherwise loopback-touching paths inside the namespace fail.
    pub fn create(name: &str) -> Result<Self, NetNsError> {
        // Best-effort cleanup of leftover state.
        let _ = Command::new("ip").args(["netns", "del", name]).output();

        run_ip(["netns", "add", name])?;
        // `ip netns exec` rejects until the namespace exists; bring
        // `lo` up so in-namespace clients can touch loopback if needed.
        run_ip(["netns", "exec", name, "ip", "link", "set", "lo", "up"])?;

        Ok(Self { name: name.to_owned() })
    }

    /// Move an existing iface into this namespace.
    /// Configure an IPv4 address on an iface that already lives in
    /// this namespace, then bring the iface up.
    pub fn assign_ip_and_up(&self, iface: &str, cidr: &str) -> Result<(), NetNsError> {
        run_ip(["netns", "exec", &self.name, "ip", "addr", "add", cidr, "dev", iface])?;
        run_ip(["netns", "exec", &self.name, "ip", "link", "set", iface, "up"])?;
        Ok(())
    }

    /// Add an IPv4 route inside this namespace.
    /// `dest_cidr` is the destination network (e.g. `10.1.0.0/24`),
    /// `via` is either `Some("<gw>")` for a next-hop gateway or
    /// `None` for a `dev`-direct on-link route.
    pub fn add_route(
        &self,
        dest_cidr: &str,
        via: Option<&str>,
        dev: Option<&str>,
    ) -> Result<(), NetNsError> {
        let mut argv: Vec<String> = vec![
            "netns".into(),
            "exec".into(),
            self.name.clone(),
            "ip".into(),
            "route".into(),
            "add".into(),
            dest_cidr.into(),
        ];
        if let Some(gw) = via {
            argv.push("via".into());
            argv.push(gw.into());
        }
        if let Some(d) = dev {
            argv.push("dev".into());
            argv.push(d.into());
        }
        run_ip(argv)?;
        Ok(())
    }

    /// Run `sysctl -w <key>=<value>` inside this namespace.
    /// Best-effort: returns Ok if the sysctl applied, or wraps the
    /// `ip netns exec` error otherwise.
    pub fn sysctl(&self, key: &str, value: &str) -> Result<(), NetNsError> {
        run_ip(["netns", "exec", &self.name, "sysctl", "-w", &format!("{key}={value}")])?;
        Ok(())
    }

    /// Build a `Command` that, when spawned, runs `cmd` (with `args`)
    /// inside this namespace. The caller controls the rest of the
    /// builder (stdin/stdout/stderr piping, env, etc.) — this is just
    /// the `ip netns exec <ns> <cmd> <args...>` prefix.
    #[must_use]
    pub fn command<I, S>(&self, cmd: &str, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut c = Command::new("ip");
        c.arg("netns").arg("exec").arg(&self.name).arg(cmd);
        for a in args {
            c.arg(a);
        }
        c
    }
}

impl Drop for NetNs {
    fn drop(&mut self) {
        // Best-effort teardown — the kernel reaps in-namespace ifaces
        // and processes when the namespace is destroyed.
        let _ = Command::new("ip").args(["netns", "del", &self.name]).output();
    }
}

/// Three-iface transit topology for L4LB Tier 3 tests, mirroring
/// Cilium PR #16338's standalone L4LB integration test shape (per
/// `docs/research/dataplane/xdp-l4lb-test-topology-comprehensive-research.md`
/// § Recommendation 1).
///
/// ```text
///   client-ns                       lb-ns                          backend-ns
///     ┌──────────────┐                ┌──────────────────┐           ┌──────────────┐
///     │ client_veth  │ <──── pair ──> │ lb_veth_a        │           │              │
///     │ 10.0.0.10/24 │                │ 10.0.0.1/24      │           │              │
///     │              │                │                  │           │              │
///     │              │                │      lb_veth_b   │ <─pair─>  │ backend_veth │
///     │              │                │      10.1.0.1/24 │           │ 10.1.0.5/24  │
///     │              │                │  XDP+TC programs │           │  XDP_PASS    │
///     │              │                │  attach here     │           │  stub        │
///     └──────────────┘                └──────────────────┘           └──────────────┘
/// ```
///
/// Per Finding 4.2 of the research, XDP_TX into a veth peer requires
/// the receiving veth to ALSO have an XDP program attached — even a
/// no-op `XDP_PASS` stub satisfies the kernel's
/// `if (!rcu_access_pointer(rcv_priv->xdp_prog)) goto out;` gate. The
/// stub attach is the test caller's job (`crates/overdrive-bpf` ships
/// the `xdp_pass` program); this fixture only stands up the namespaces
/// and veth pairs.
///
/// IP forwarding is enabled in `lb-ns` so packets routed by the kernel
/// from `lb_veth_a` to `lb_veth_b` work for non-XDP paths (return
/// traffic from the backend, ARP resolution, ICMP, and the
/// `RET_NO_NEIGH` / `RET_NOT_FWDED` slow-path fallbacks where the
/// XDP program returns `XDP_PASS`).
///
/// **Cross-iface XDP semantics** — `XDP_TX` bounces a frame out the
/// SAME iface it arrived on, so cannot deliver from `lb_veth_a` to
/// `lb_veth_b`. Cross-iface delivery uses `bpf_redirect(fib.ifindex,
/// 0)` after `bpf_fib_lookup` resolves the egress iface and the
/// next-hop MAC; the program's MAC rewrite + redirect path is what
/// makes the 3-iface topology work. See
/// `docs/research/dataplane/cilium-bpf-fib-lookup-l2-mac-rewrite-comprehensive-research.md`
/// for the full mechanic.
///
/// `Drop` reaps the namespaces (kernel auto-reaps in-namespace
/// ifaces). The veth pairs are forgotten after `move_iface` because
/// their host-side handles no longer reference live ifaces — the ifaces
/// have moved into the per-namespace handle.
pub struct ThreeIfaceTopology {
    pub client_ns: NetNs,
    pub lb_ns: NetNs,
    pub backend_ns: NetNs,
    /// veth name in client-ns connected to lb-ns's `lb_veth_a`.
    pub client_veth: String,
    /// veth name in lb-ns connected to client-ns's `client_veth`.
    /// XDP+TC programs attach here.
    pub lb_veth_a: String,
    /// veth name in lb-ns connected to backend-ns's `backend_veth`.
    /// Stub XDP_PASS may attach here too if needed for symmetric XDP_TX
    /// delivery (research Finding 4.2 — load on BOTH peers).
    pub lb_veth_b: String,
    /// veth name in backend-ns connected to lb-ns's `lb_veth_b`.
    /// Stub XDP_PASS attaches here per research Recommendation 2.
    pub backend_veth: String,
}

/// Stable IP plan for the three-iface topology. Tests that bind to the
/// VIP (`10.0.0.1`) or the backend address (`10.1.0.5`) can reference
/// these constants instead of duplicating literals.
pub mod threeiface_ips {
    use std::net::Ipv4Addr;
    /// VIP the LB owns (also the address on `lb_veth_a` so ARP
    /// resolves; the LB's `lb_veth_a` IS the next-hop gateway from
    /// client-ns).
    pub const VIP: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    /// Client's source IP.
    pub const CLIENT_IP: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 10);
    /// Address on the LB-side iface facing the backend network.
    pub const LB_BACKEND_IP: Ipv4Addr = Ipv4Addr::new(10, 1, 0, 1);
    /// Backend's address.
    pub const BACKEND_IP: Ipv4Addr = Ipv4Addr::new(10, 1, 0, 5);
}

impl ThreeIfaceTopology {
    /// Build the full topology. `tag` is a short (≤ 4 char) discriminator
    /// that namespaces every iface and netns name so two parallel test
    /// processes do not collide on the global iface namespace.
    pub fn create(tag: &str) -> Result<Self, NetNsError> {
        use threeiface_ips::{BACKEND_IP, CLIENT_IP, LB_BACKEND_IP, VIP};

        let suffix = std::process::id();
        let client_name = format!("3i-clt-{tag}-{suffix}");
        let lb_name = format!("3i-lb-{tag}-{suffix}");
        let backend_name = format!("3i-bck-{tag}-{suffix}");
        // Linux IFNAMSIZ = 16 (15 chars + NUL). Tag ≤ 4 chars + suffix
        // 4 hex chars + prefix 3 chars = 12 chars worst-case.
        let client_veth = format!("3i{tag}c{:04x}", suffix & 0xffff);
        let lb_veth_a = format!("3i{tag}a{:04x}", suffix & 0xffff);
        let lb_veth_b = format!("3i{tag}b{:04x}", suffix & 0xffff);
        let backend_veth = format!("3i{tag}d{:04x}", suffix & 0xffff);

        let client_ns = NetNs::create(&client_name)?;
        let lb_ns = NetNs::create(&lb_name)?;
        let backend_ns = NetNs::create(&backend_name)?;

        // Pair 1 — client_veth <-> lb_veth_a (initially in host ns).
        run_ip([
            "link",
            "add",
            client_veth.as_str(),
            "type",
            "veth",
            "peer",
            "name",
            lb_veth_a.as_str(),
        ])?;
        // Move ends into target namespaces.
        run_ip(["link", "set", client_veth.as_str(), "netns", &client_name])?;
        run_ip(["link", "set", lb_veth_a.as_str(), "netns", &lb_name])?;

        // Pair 2 — lb_veth_b <-> backend_veth.
        run_ip([
            "link",
            "add",
            lb_veth_b.as_str(),
            "type",
            "veth",
            "peer",
            "name",
            backend_veth.as_str(),
        ])?;
        run_ip(["link", "set", lb_veth_b.as_str(), "netns", &lb_name])?;
        run_ip(["link", "set", backend_veth.as_str(), "netns", &backend_name])?;

        // Configure addresses + bring up.
        client_ns.assign_ip_and_up(&client_veth, &format!("{CLIENT_IP}/24"))?;
        lb_ns.assign_ip_and_up(&lb_veth_a, &format!("{VIP}/24"))?;
        lb_ns.assign_ip_and_up(&lb_veth_b, &format!("{LB_BACKEND_IP}/24"))?;
        backend_ns.assign_ip_and_up(&backend_veth, &format!("{BACKEND_IP}/24"))?;

        // Enable IP forwarding in lb-ns so the kernel routes frames
        // arriving on lb_veth_a out of lb_veth_b for non-XDP paths
        // (return traffic, ICMP, ARP resolution against the backend,
        // FIB-fallback `XDP_PASS`-after-rewrite paths). The XDP
        // forward-path uses `bpf_fib_lookup` + `bpf_redirect` to
        // resolve the egress iface and the next-hop MAC directly.
        let _ = lb_ns.sysctl("net.ipv4.ip_forward", "1");

        // Disable rp_filter cluster-wide in lb-ns. The rewritten frame's
        // src is 10.0.0.10 (client) but arrives via lb_veth_b's
        // routing path; strict rp_filter would drop it.
        let _ = lb_ns.sysctl("net.ipv4.conf.all.rp_filter", "0");
        let _ = lb_ns.sysctl("net.ipv4.conf.default.rp_filter", "0");
        let _ = client_ns.sysctl("net.ipv4.conf.all.rp_filter", "0");
        let _ = backend_ns.sysctl("net.ipv4.conf.all.rp_filter", "0");

        // Default route in client-ns — everything flows via the LB
        // (10.0.0.1 is the VIP AND the gateway).
        let _ = client_ns.add_route("default", Some(&VIP.to_string()), None);
        // Default route in backend-ns via lb-ns's backend-side iface.
        let _ = backend_ns.add_route("default", Some(&LB_BACKEND_IP.to_string()), None);

        // Disable TX checksum offload + GSO/TSO on every veth. The
        // kernel's TCP socket layer emits SYNs with ip_summed =
        // CHECKSUM_PARTIAL and a partial-cksum value on the wire;
        // XDP_REDIRECT resets ip_summed to CHECKSUM_NONE on the
        // destination peer, forcing full validation against the wire
        // bytes. XDP's incremental update over a partial input
        // produces a partial output that fails validation and the
        // SYN is silently dropped. Disabling tx-checksumming forces
        // every emitting stack in the path to compute a full valid
        // cksum on the wire — XDP's incremental update over that
        // produces another full valid cksum the receiver accepts.
        // Standard Cilium testbed setup for veth-based L4LB tests.
        // Best-effort: older ethtool may reject some keys but
        // tx-checksum-ip-generic off is the load-bearing one.
        for (ns_name, iface) in [
            (client_ns.name.as_str(), client_veth.as_str()),
            (lb_ns.name.as_str(), lb_veth_a.as_str()),
            (lb_ns.name.as_str(), lb_veth_b.as_str()),
            (backend_ns.name.as_str(), backend_veth.as_str()),
        ] {
            for feature in ["tx-checksum-ip-generic", "tx", "rx", "tso", "gso", "gro"] {
                let _ = Command::new("ip")
                    .args(["netns", "exec", ns_name, "ethtool", "-K", iface, feature, "off"])
                    .output();
            }
        }

        Ok(Self { client_ns, lb_ns, backend_ns, client_veth, lb_veth_a, lb_veth_b, backend_veth })
    }
}

// `ThreeIfaceTopology` does not need an explicit Drop — `NetNs::drop`
// reaps each namespace (and the kernel reaps in-namespace ifaces with
// it). The veth pairs were moved into the namespaces and are no longer
// referenced from the host ns; their teardown is implicit.

/// Spawn `ip <args>`, classify the result. Mirrors the helper in
/// [`super::veth`]; kept distinct so the netns helper is usable
/// stand-alone.
fn run_ip<I, S>(args: I) -> Result<std::process::Output, NetNsError>
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
    let out = Command::new("ip").args(args).output().map_err(NetNsError::Spawn)?;
    if out.status.success() {
        return Ok(out);
    }
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if stderr.contains("Operation not permitted") || stderr.contains("Permission denied") {
        return Err(NetNsError::CapNetAdminRequired);
    }
    Err(NetNsError::IpCommand { args: arg_str, stderr, status: out.status.code() })
}
