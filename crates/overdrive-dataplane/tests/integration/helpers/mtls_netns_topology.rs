//! Real netns/veth + cgroup-isolated-workload fixtures for the composed
//! transparent-mTLS walking-skeleton Tier-3 gate (ADR-0069, GH #26; step 01-01,
//! GAP 3). Single-consumer (only `mtls_composed_walking_skeleton.rs`); promote to
//! `overdrive-testing` only if a second consumer appears (`.claude/rules/
//! development.md` § "Shared real-infra test fixtures").
//!
//! GAP 3 closes the spikes' loopback-+-sibling-processes shortcut: the composed
//! flow must run over a real veth topology with cgroup-isolated workloads, so the
//! `cgroup_connect4` rewrite (outbound), the `nft`-TPROXY prerouting intercept
//! (inbound), and the splice-to-workload all hold on the real topology.
//!
//! Topology (single node, single netns peer + a host-side cgroup the workload
//! runs in — the agent runs on the host):
//!
//! ```text
//!   workload cgroup  (/sys/fs/cgroup/overdrive.slice/mtls-ws-<tag>.scope)
//!     │  cgroup_connect4_mtls attaches HERE (workload subtree only — F5 exemption)
//!     ▼
//!   veth pair  wl-<h> (host) <──> wlp-<h> (workload netns mtls-wl-<tag>)
//!     (<h> = a bounded 8-hex hash of <tag> so the veth names fit IFNAMSIZ)
//!     │  the cgroup-isolated workload runs in the netns; its egress crosses veth
//!     ▼
//!   AGENT (host)  — leg-F listener (outbound) / IP_TRANSPARENT leg-C listener
//!                    (inbound) + nft-TPROXY prerouting + ip rule/route
//! ```
//!
//! Every fixture is RAII-cleaned: the cgroup scope (`cgroup.kill` then `rmdir`),
//! the netns (`ip netns del`), the veth pair (one `ip link del` reaps both sides),
//! the nft table (`nft delete table`), and the ip rule/route. Capability-gated:
//! the fixtures need root + `CAP_NET_ADMIN` (cgroup writes, netns, nft-TPROXY,
//! `IP_TRANSPARENT`) — `MtlsTopology::create` returns `Err(Unsupported)` when the
//! environment cannot provide them, and the test SKIPs rather than failing.

#![cfg(target_os = "linux")]
#![allow(dead_code)]

use std::process::Command;

/// Why a topology could not be stood up. `Unsupported` is the SKIP signal (no
/// root / no `CAP_NET_ADMIN` / missing `nft_tproxy` / missing cgroup v2 delegated
/// subtree); `Setup` is a real failure mid-build (the partially-built topology is
/// torn down before the error surfaces).
#[derive(Debug)]
pub enum TopologyError {
    /// The environment cannot provide the required privileges/kernel features —
    /// the test should SKIP, not fail.
    Unsupported(String),
    /// A setup command failed unexpectedly after privileges were confirmed.
    Setup(String),
}

impl std::fmt::Display for TopologyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(why) => write!(f, "topology unsupported (skip): {why}"),
            Self::Setup(why) => write!(f, "topology setup failed: {why}"),
        }
    }
}

impl std::error::Error for TopologyError {}

/// Run a command, returning `Err(Setup)` with stderr on non-zero exit.
fn run(argv: &[&str]) -> Result<(), TopologyError> {
    let out = Command::new(argv[0])
        .args(&argv[1..])
        .output()
        .map_err(|e| TopologyError::Setup(format!("spawn {argv:?}: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(TopologyError::Setup(format!(
            "{argv:?} exited {:?}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// Best-effort run — never errors (used in cleanup and idempotent pre-clean).
fn run_ok(argv: &[&str]) {
    let _ = Command::new(argv[0])
        .args(&argv[1..])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Apply an nft program via `nft -f -`.
fn apply_nft(prog: &str) -> Result<(), TopologyError> {
    use std::io::Write as _;
    let mut child = Command::new("nft")
        .arg("-f")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| TopologyError::Unsupported(format!("nft not available: {e}")))?;
    child
        .stdin
        .take()
        .ok_or_else(|| TopologyError::Setup("nft stdin missing".into()))?
        .write_all(prog.as_bytes())
        .map_err(|e| TopologyError::Setup(format!("nft write: {e}")))?;
    let out =
        child.wait_with_output().map_err(|e| TopologyError::Setup(format!("nft wait: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(TopologyError::Setup(format!(
            "nft -f failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// Detect whether the current process can stand up the topology (root +
/// `CAP_NET_ADMIN`-shaped privileges). Returns the SKIP reason if not.
/// Bounded, deterministic 8-hex iface suffix derived from a topology `tag`, so
/// the `wlp-<suffix>` peer veth name fits IFNAMSIZ (15). The low 32 bits of a
/// stable (non-`RandomState`) hash are masked and zero-padded to 8 hex chars;
/// `wlp-<8hex>` = 12 ≤ 15 for any tag / pid width. See `.claude/rules/
/// development.md` § "size a derived id to its grammar's ceiling".
fn iface_suffix(tag: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tag.hash(&mut h);
    format!("{:08x}", h.finish() & 0xFFFF_FFFF)
}

fn check_privileges() -> Result<(), TopologyError> {
    // `id -u` == 0 is the cheap root probe; the canonical inner loop runs the
    // Tier-3 suite as root via `cargo xtask lima run --`. A non-root run cannot
    // create netns / attach cgroup_connect4 / set IP_TRANSPARENT.
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .map_err(|e| TopologyError::Unsupported(format!("cannot probe uid: {e}")))?;
    let uid_str = String::from_utf8_lossy(&uid.stdout);
    if uid_str.trim() != "0" {
        return Err(TopologyError::Unsupported(format!(
            "not root (uid={}); netns/cgroup/IP_TRANSPARENT need root + CAP_NET_ADMIN",
            uid_str.trim()
        )));
    }
    Ok(())
}

/// The composed-walking-skeleton topology: a cgroup-isolated workload reachable
/// over a real veth pair, plus the host-side intercept scaffolding (nft-TPROXY +
/// ip rule/route) the agent drives. RAII-cleaned on `Drop`.
pub struct MtlsTopology {
    tag: String,
    /// Absolute path of the workload cgroup scope directory (cgroup v2). The
    /// `cgroup_connect4_mtls` program attaches to this subtree (F5 — workload
    /// subtree only, so the agent's own dial is not re-intercepted).
    cgroup_path: String,
    /// The workload netns name (`ip netns` form).
    netns: String,
    /// Host-side veth name (the workload's peer is inside the netns).
    host_veth: String,
    /// Cleanup commands, run in reverse-construction order on `Drop`.
    cleanup: Vec<Vec<String>>,
}

impl MtlsTopology {
    /// The workload's logical/virtual IPv4 the inbound client aims at (the
    /// TPROXY-intercepted destination — selects the server SVID via orig-dst).
    /// Loopback so the host-side client's SYN traverses the host `prerouting` hook
    /// where the `nft`-TPROXY rule fires.
    pub const VIRT_IP: &'static str = "127.0.0.2";

    /// The inbound virtual port: the port the client aims at on [`Self::VIRT_IP`],
    /// the TPROXY rule's `dport`, and the orig-dst port the agent recovers via
    /// `getsockname` (which the production adapter dials VERBATIM via
    /// `server_dial_addr(orig_dst)`). GAP 3: the netns-isolated server S binds this
    /// same port on [`Self::server_netns_ip`], and the harness DNATs the agent's
    /// marked leg-S dial (`VIRT_IP:VIRT_PORT → server_netns_ip:VIRT_PORT`) so the
    /// verbatim orig-dst dial reaches S over the veth — closing GAP-3-inbound purely
    /// in the harness, with NO change to the reserved `server_dial_addr` (#178).
    pub const VIRT_PORT: u16 = 18443;

    /// The netns-side veth IPv4 — where the cgroup-isolated server workload S binds
    /// (reachable from the host over the veth). The agent's leg-S dial reaches it
    /// after the harness DNAT rewrites the verbatim orig-dst (`VIRT_IP`) to this.
    /// Matches the netns veth address assigned in `try_setup` (`10.66.0.2/24`).
    pub const SERVER_NETNS_IP: &'static str = "10.66.0.2";

    /// The host-side veth IPv4 — the masquerade source for the agent's DNAT'd leg-S
    /// dial (a packet DNAT'd from a `127.0.0.2` loopback dst keeps its `127.0.0.1`
    /// loopback src, which cannot egress the veth; masquerade rewrites it to this so
    /// S's reply routes back and conntrack un-NATs the connection the agent dialed).
    /// Matches the host veth address assigned in `try_setup` (`10.66.0.1/24`).
    pub const HOST_VETH_IP: &'static str = "10.66.0.1";

    /// Stand up the topology, or return `Err(Unsupported)` for the SKIP path.
    ///
    /// `tag` disambiguates parallel runs (embed the test pid). On any mid-build
    /// failure the partial topology is torn down before the error surfaces.
    pub fn create(tag: &str) -> Result<Self, TopologyError> {
        // The test is the COMPOSITION ROOT for the test: it drives `probe`/`enforce`
        // directly (no `overdrive serve` boot), so it must install the process-default
        // rustls `CryptoProvider` itself — the adapter no longer installs it (a library
        // mutating process-global crypto state is the wrong layer; production's
        // composition root in `overdrive-control-plane` owns the install). Guarded by
        // `Once` so the single install happens exactly once across every test in the
        // binary, regardless of how many topologies are stood up.
        install_crypto_provider_once();
        check_privileges()?;

        let cgroup_root = "/sys/fs/cgroup/overdrive.slice";
        let cgroup_path = format!("{cgroup_root}/mtls-ws-{tag}.scope");
        let netns = format!("mtls-wl-{tag}");
        // Veth iface names are IFNAMSIZ-bound (15 usable chars). The `wlp-` peer
        // prefix is the binding constraint, so the suffix must be ≤ 11 chars:
        // `wlp-dpoi-<7-digit-pid>` = 16 overflows ("not a valid ifname"). Derive
        // a bounded, deterministic 8-hex iface suffix from the full tag (which
        // still names the IFNAMSIZ-free cgroup/netns), per `.claude/rules/
        // development.md` § "size a derived id to its grammar's ceiling" —
        // `wlp-<8hex>` = 12 ≤ 15 for any tag / pid width.
        let iface = iface_suffix(tag);
        let host_veth = format!("wl-{iface}");
        let wl_veth = format!("wlp-{iface}");

        // Idempotent pre-clean of any leftover state from an aborted run.
        run_ok(&["ip", "netns", "del", &netns]);
        run_ok(&["ip", "link", "del", &host_veth]);

        let mut topo = Self {
            tag: tag.to_owned(),
            cgroup_path: cgroup_path.clone(),
            netns: netns.clone(),
            host_veth: host_veth.clone(),
            cleanup: Vec::new(),
        };

        // 1. Workload cgroup scope (cgroup v2). `mkdir -p` the parent slice first.
        std::fs::create_dir_all(cgroup_root).map_err(|e| {
            TopologyError::Unsupported(format!("cgroup v2 root not delegated: {e}"))
        })?;
        std::fs::create_dir(&cgroup_path).map_err(|e| {
            TopologyError::Unsupported(format!("cannot create workload cgroup scope: {e}"))
        })?;
        topo.cleanup.push(svec(&["__cgroup_kill_rmdir__", &cgroup_path]));

        // 2. netns for the workload.
        topo.try_setup(&netns, &host_veth, &wl_veth)?;

        Ok(topo)
    }

    fn try_setup(
        &mut self,
        netns: &str,
        host_veth: &str,
        wl_veth: &str,
    ) -> Result<(), TopologyError> {
        run(&["ip", "netns", "add", netns])
            .map_err(|e| TopologyError::Unsupported(format!("ip netns add: {e}")))?;
        self.cleanup.push(svec(&["ip", "netns", "del", netns]));
        run(&["ip", "netns", "exec", netns, "ip", "link", "set", "lo", "up"])?;

        // 3. veth pair: host side stays on host, peer moves into the netns.
        run(&["ip", "link", "add", host_veth, "type", "veth", "peer", "name", wl_veth])?;
        self.cleanup.push(svec(&["ip", "link", "del", host_veth]));
        run(&["ip", "link", "set", wl_veth, "netns", netns])?;
        run(&["ip", "addr", "add", "10.66.0.1/24", "dev", host_veth])?;
        run(&["ip", "link", "set", host_veth, "up"])?;
        run(&["ip", "netns", "exec", netns, "ip", "addr", "add", "10.66.0.2/24", "dev", wl_veth])?;
        run(&["ip", "netns", "exec", netns, "ip", "link", "set", wl_veth, "up"])?;

        // Relax the netns-side reverse-path filter so S's reply to the masqueraded
        // host-veth source (the GAP-3 leg-S DNAT path) is not reverse-path-dropped.
        // Best-effort: a kernel without these knobs still works for the common case
        // (the standalone repro confirmed the round-trip), so do not fail setup on a
        // missing knob.
        run_ok(&["ip", "netns", "exec", netns, "sysctl", "-w", "net.ipv4.conf.all.rp_filter=0"]);
        run_ok(&[
            "ip",
            "netns",
            "exec",
            netns,
            "sysctl",
            "-w",
            &format!("net.ipv4.conf.{wl_veth}.rp_filter=0"),
        ]);

        Ok(())
    }

    /// Idempotent pre-clean of the GLOBAL inbound state (the `fwmark` rule, the
    /// `local` route in `rt_table`, and the `nft` table). These are not netns-scoped,
    /// so the topology RAII `Drop` does not reclaim them on a SIGKILL (nextest
    /// slow-timeout, Bash wall-clock cap, user cancel; see `.claude/rules/testing.md`
    /// "Leaked workload cgroups"). A prior aborted run can leave a stale `fwmark` rule
    /// (even many duplicates) and a stale route, which would make a fresh `ip route
    /// add ... table 100` fail with `File exists`. Draining them up front makes the
    /// install idempotent across aborts; clean exits still tear down via `self.cleanup`.
    fn preclean_global_inbound_state(fwmark: u32, rt_table: u32, table: &str) {
        for _ in 0..64 {
            let out = Command::new("ip")
                .args([
                    "rule",
                    "del",
                    "fwmark",
                    &fwmark.to_string(),
                    "lookup",
                    &rt_table.to_string(),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if !matches!(out, Ok(s) if s.success()) {
                break; // no more matching rules
            }
        }
        run_ok(&[
            "ip",
            "route",
            "del",
            "local",
            "0.0.0.0/0",
            "dev",
            "lo",
            "table",
            &rt_table.to_string(),
        ]);
        run_ok(&["nft", "delete", "table", "ip", table]);
    }

    /// The GAP-3 leg-S routing prerequisites: the agent's marked leg-S dial targets a
    /// `127.0.0.2` loopback VIP (the verbatim orig-dst), which the kernel hard-routes
    /// to `lo` and treats as a martian destination. `route_localnet=1` lets
    /// `127.0.0.0/8` route off `lo` so the `nat OUTPUT` DNAT can redirect the dial out
    /// the veth; relaxed `rp_filter` lets the masqueraded reverse path (S → host-veth)
    /// pass the reverse-path check. Without these the DNAT'd SYN never leaves `lo` and
    /// the agent's blocking `connect` hangs (proven by the standalone leg-S DNAT repro:
    /// with these sysctls the marked dial reaches the netns S over the veth and
    /// round-trips byte-exact). Idempotent; host-wide tunables on the ephemeral test VM
    /// (no RAII reset needed).
    fn enable_leg_s_routing_sysctls(&self) -> Result<(), TopologyError> {
        let host_veth = self.host_veth.clone();
        for (knob, val) in [
            ("net.ipv4.conf.all.route_localnet", "1"),
            (&format!("net.ipv4.conf.{host_veth}.route_localnet"), "1"),
            ("net.ipv4.conf.all.rp_filter", "0"),
            (&format!("net.ipv4.conf.{host_veth}.rp_filter"), "0"),
            ("net.ipv4.conf.lo.rp_filter", "0"),
        ] {
            run(&["sysctl", "-w", &format!("{knob}={val}")]).map_err(|e| {
                TopologyError::Unsupported(format!("leg-S routing sysctl {knob}: {e}"))
            })?;
        }
        Ok(())
    }

    /// Install the inbound `nft`-TPROXY intercept + the GAP-3 leg-S routing, all
    /// RAII-cleaned via the topology's `Drop` and FAILURE-PROPAGATING (a setup failure
    /// surfaces as `Err`, which the gate treats as a hard failure — no silent
    /// best-effort). Single source of truth for the inbound fixture (the duplicate
    /// standalone `install_inbound_tproxy` in `roles.rs` is removed).
    ///
    /// Three things land here (`findings-inbound-intercept.md` §1 / Mechanics #2). The
    /// TPROXY redirect sends a connection aimed at `VIRT_IP:VIRT_PORT` to
    /// `127.0.0.1:agent_port` (the agent's `IP_TRANSPARENT` leg-C listener), marked for
    /// the `local` route table. The leg-S exemption (F5) `accept`s the
    /// [`MTLS_LEG_S_DIAL_MARK`](overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK)-stamped agent dial FIRST in the prerouting chain so the
    /// agent's own dial (which targets the same virtual address the client aimed at) is
    /// NOT re-TPROXY'd back to leg C — without it the dial recurses. The GAP-3 leg-S
    /// DNAT + masquerade route the agent's verbatim orig-dst dial to the netns server:
    /// S binds `SERVER_NETNS_IP:VIRT_PORT`, but the production adapter dials the orig-dst
    /// VERBATIM (`server_dial_addr(orig_dst) == orig_dst == VIRT_IP:VIRT_PORT`, reserved
    /// for #178, NOT touched), so the harness DNATs the marked leg-S dial
    /// `VIRT_IP:VIRT_PORT → SERVER_NETNS_IP:VIRT_PORT` in `nat OUTPUT` and masquerades
    /// the loopback source to the host veth on egress, so the dial reaches S over the
    /// veth and conntrack un-NATs S's reply — the "non-trivial netns routing in the
    /// HARNESS" that closes GAP-3-inbound with no production-surface change.
    pub fn install_tproxy(&mut self, agent_port: u16) -> Result<(), TopologyError> {
        let fwmark = 0x1u32;
        let rt_table = 100u32;
        let leg_s_mark = overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
        let table = format!("overdrive_mtls_ws_{}", self.tag);

        // Idempotent pre-clean of the GLOBAL rule/route/table a prior SIGKILL'd run may
        // have leaked, then the GAP-3 leg-S routing sysctls. Factored out to keep this
        // method readable.
        Self::preclean_global_inbound_state(fwmark, rt_table, &table);
        self.enable_leg_s_routing_sysctls()?;

        run(&[
            "ip",
            "rule",
            "add",
            "fwmark",
            &fwmark.to_string(),
            "lookup",
            &rt_table.to_string(),
        ])?;
        self.cleanup.push(svec(&[
            "ip",
            "rule",
            "del",
            "fwmark",
            &fwmark.to_string(),
            "lookup",
            &rt_table.to_string(),
        ]));
        run(&[
            "ip",
            "route",
            "add",
            "local",
            "0.0.0.0/0",
            "dev",
            "lo",
            "table",
            &rt_table.to_string(),
        ])?;
        self.cleanup.push(svec(&[
            "ip",
            "route",
            "del",
            "local",
            "0.0.0.0/0",
            "dev",
            "lo",
            "table",
            &rt_table.to_string(),
        ]));

        // prerouting: the leg-S-dial-mark exemption (F5) MUST precede the TPROXY rule
        // so the agent's own dial is accepted before the redirect can match it. The
        // GAP-3 leg-S DNAT + masquerade route the agent's (marked, loopback-sourced)
        // dial to the netns server S over the veth.
        let nft_prog = format!(
            "table ip {table} {{\n\
               chain prerouting {{\n\
                 type filter hook prerouting priority mangle; policy accept;\n\
                 meta mark {leg_s_mark} accept;\n\
                 ip daddr {vip} tcp dport {vport} tproxy to 127.0.0.1:{aport} meta mark set {mark} accept;\n\
               }}\n\
               chain output {{\n\
                 type nat hook output priority dstnat; policy accept;\n\
                 meta mark {leg_s_mark} ip daddr {vip} tcp dport {vport} dnat to {snip}:{vport};\n\
               }}\n\
               chain postrouting {{\n\
                 type nat hook postrouting priority srcnat; policy accept;\n\
                 meta mark {leg_s_mark} oifname \"{hveth}\" masquerade;\n\
               }}\n\
             }}\n",
            vip = Self::VIRT_IP,
            vport = Self::VIRT_PORT,
            aport = agent_port,
            mark = fwmark,
            snip = Self::SERVER_NETNS_IP,
            hveth = self.host_veth,
        );
        apply_nft(&nft_prog)
            .map_err(|e| TopologyError::Unsupported(format!("nft_tproxy unavailable: {e}")))?;
        self.cleanup.push(svec(&["nft", "delete", "table", "ip", &table]));
        Ok(())
    }

    /// The workload cgroup scope path (the `cgroup_connect4_mtls` attach target +
    /// where workload processes are placed via `cgroup.procs`).
    #[must_use]
    pub fn cgroup_path(&self) -> &str {
        &self.cgroup_path
    }

    /// The workload netns name (run the cgroup-isolated workload inside it).
    #[must_use]
    pub fn netns(&self) -> &str {
        &self.netns
    }

    /// Place a process pid into the workload cgroup scope (write to
    /// `cgroup.procs`). Used by the workload-spawn `pre_exec` hook so the
    /// workload's `connect()` fires the `cgroup_connect4_mtls` hook.
    pub fn join_cgroup(&self, pid: u32) -> std::io::Result<()> {
        std::fs::write(format!("{}/cgroup.procs", self.cgroup_path), pid.to_string())
    }
}

impl Drop for MtlsTopology {
    fn drop(&mut self) {
        // Reverse-construction-order teardown; every step best-effort so one
        // failure never strands the rest.
        for cmd in self.cleanup.iter().rev() {
            if cmd[0] == "__cgroup_kill_rmdir__" {
                let scope = &cmd[1];
                // cgroup v2 mass-kill, then reclaim the directory.
                let _ = std::fs::write(format!("{scope}/cgroup.kill"), "1");
                let _ = std::fs::remove_dir(scope);
            } else {
                let args: Vec<&str> = cmd.iter().map(String::as_str).collect();
                run_ok(&args);
            }
        }
    }
}

fn svec(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_string()).collect()
}

/// Install the process-default rustls `CryptoProvider` exactly once for this test
/// binary. The composed gate's `probe`/`enforce` run sentinel + real rustls
/// handshakes that consume the process-default provider via
/// `ServerConfig::builder()` / `ClientConfig::builder()`; with the adapter no
/// longer installing it (the install is the composition root's job), the test
/// harness — the composition root for the test — installs it. `Once` makes the
/// single install idempotent across every topology stood up in the binary.
fn install_crypto_provider_once() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        // A second install would return `Err`; `Once` guarantees this runs once, so
        // the result is the authoritative install. Ignore it — if a provider was
        // already installed by some earlier entrypoint, the handshakes still work.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
