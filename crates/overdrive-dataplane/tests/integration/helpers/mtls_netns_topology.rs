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
//!   veth pair  wl-<tag> (host) <──> wlp-<tag> (workload netns mtls-wl-<tag>)
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
    pub const VIRT_IP: &'static str = "127.0.0.2";

    /// Stand up the topology, or return `Err(Unsupported)` for the SKIP path.
    ///
    /// `tag` disambiguates parallel runs (embed the test pid). On any mid-build
    /// failure the partial topology is torn down before the error surfaces.
    pub fn create(tag: &str) -> Result<Self, TopologyError> {
        check_privileges()?;

        let cgroup_root = "/sys/fs/cgroup/overdrive.slice";
        let cgroup_path = format!("{cgroup_root}/mtls-ws-{tag}.scope");
        let netns = format!("mtls-wl-{tag}");
        let host_veth = format!("wl-{tag}");
        let wl_veth = format!("wlp-{tag}");

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

        Ok(())
    }

    /// Install the inbound nft-TPROXY intercept: a connection aimed at
    /// `VIRT_IP:virt_port` is redirected to `127.0.0.1:agent_port` (the agent's
    /// `IP_TRANSPARENT` leg-C listener) and marked, with the marked-packet route
    /// to local delivery (`findings-inbound-intercept.md` §1 / Mechanics #2).
    pub fn install_tproxy(&mut self, virt_port: u16, agent_port: u16) -> Result<(), TopologyError> {
        let fwmark = 0x1u32;
        let rt_table = 100u32;
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

        let table = format!("overdrive_mtls_ws_{}", self.tag);
        let nft_prog = format!(
            "table ip {table} {{\n\
               chain prerouting {{\n\
                 type filter hook prerouting priority mangle; policy accept;\n\
                 ip daddr {vip} tcp dport {vport} tproxy to 127.0.0.1:{aport} meta mark set {mark} accept;\n\
               }}\n\
             }}\n",
            vip = Self::VIRT_IP,
            vport = virt_port,
            aport = agent_port,
            mark = fwmark,
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
