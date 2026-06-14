//! transparent-mtls-host-socket (ADR-0069, GH #26; step 06-01, D-MTLS-17 item 1)
//! — Tier-3 acceptance test for the PRODUCTION OUTBOUND BPF integration surface
//! [`MtlsDataplane`](overdrive_dataplane::mtls::MtlsDataplane).
//!
//! Drives the production `MtlsDataplane` userspace handle (NOT the test-only glue
//! in `helpers::mtls_roles`) and asserts against REAL kernel state observed via
//! `bpftool` + a real cgroup-isolated `connect()`:
//!
//! 1. After `load()` + `attach_alloc(scope)`, `bpftool cgroup show <alloc-scope>`
//!    lists `cgroup_connect4_mtls` on THAT alloc's `.scope`, and the global
//!    `workloads.slice` (here the parent `overdrive.slice`) does NOT — proving the
//!    F5-exempt per-alloc attach, distinct from the service program's
//!    global-ancestor attach.
//! 2. After `program_redirect(real_peer, leg_f)`, `bpftool map dump name
//!    MTLS_REDIRECT_DEST` shows the host-order `(real_peer)→(leg_f)` entry; after
//!    `unprogram_redirect(real_peer)`, the entry is gone.
//! 3. A cgroup-isolated workload `connect(real_peer)` under the attached scope
//!    lands on the agent's leg-F listener (the rewrite fires); a `connect` to an
//!    un-programmed dest passes through unchanged (map MISS → pass).
//! 4. Dropping the `MtlsCgroupLink` detaches the program (`bpftool cgroup show
//!    <scope>` no longer lists it).
//!
//! Tier-3 ONLY — `cgroup_sock_addr` has no `BPF_PROG_TEST_RUN` (development.md §
//! "`bpf_sock_addr.user_port`"), so the rewrite is observed via a real connect.

// `expect_used` / `unwrap_used` are workspace-wide `warn` per
// `.claude/rules/development.md` § Errors. Tier 3 tests use `.expect(...)` to
// surface fail-fast at the assertion site, matching sibling integration tests.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::missing_panics_doc)]

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

use overdrive_dataplane::mtls::MtlsDataplane;

use super::helpers::mtls_netns_topology::{MtlsTopology, TopologyError};

/// The real peer the workload aims `connect()` at. Distinct from the leg-F
/// listener — the rewrite redirects `real_peer → leg_f`.
const REAL_PEER_IP: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 7);
const REAL_PEER_PORT: u16 = 8443;

/// An un-programmed destination the second workload aims at — proving the map
/// MISS → pass-through path (no rewrite, connect proceeds to the real dest).
const UNPROGRAMMED_IP: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 99);
const UNPROGRAMMED_PORT: u16 = 9443;

/// Skip-or-panic gate: Tier-3 needs root + cgroup v2 delegation + bpffs.
macro_rules! skip_if_unsupported {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(TopologyError::Unsupported(why)) => {
                eprintln!("SKIP mtls_dataplane outbound-install Tier-3: {why}");
                return;
            }
            Err(e) => panic!("topology setup failed (not a skip): {e}"),
        }
    };
}

#[test]
fn mtls_dataplane_load_attach_per_alloc_program_redirect() {
    let tag = format!("dpoi-{}", std::process::id());
    let topo = skip_if_unsupported!(MtlsTopology::create(&tag));

    // A per-test bpffs pin dir for the shared object's SERVICE_MAP HoM (the
    // production `load()` pre-pins it by name). Distinct from the production
    // `/sys/fs/bpf/overdrive` so parallel suites do not collide.
    let pin_dir =
        std::path::PathBuf::from(format!("/sys/fs/bpf/overdrive-mtls-dpoi-{}", std::process::id()));
    std::fs::create_dir_all(&pin_dir).expect("create per-test bpffs pin dir");
    let _pin_guard = PinDirGuard(pin_dir.clone());

    // --- AC drive: load the production surface (load-once). ---
    let mut dataplane = MtlsDataplane::load(&pin_dir).expect("MtlsDataplane::load");

    // The leg-F listener the agent would own — a real accepted socket on the host
    // veth IP so the cgroup-isolated workload (in the netns) can reach it.
    let host_veth_ip: Ipv4Addr = MtlsTopology::HOST_VETH_IP.parse().expect("host veth ip");
    let leg_f_listener =
        TcpListener::bind((host_veth_ip, 0)).expect("leg-F listener bind on host veth");
    let leg_f_addr = match leg_f_listener.local_addr().expect("leg-F addr") {
        std::net::SocketAddr::V4(v4) => v4,
        std::net::SocketAddr::V6(v6) => panic!("leg-F bound non-v4: {v6}"),
    };

    let real_peer = SocketAddrV4::new(REAL_PEER_IP, REAL_PEER_PORT);

    // --- AC 1: attach to THIS alloc's `.scope` (F5-exempt per-alloc attach). ---
    let scope_path = std::path::Path::new(topo.cgroup_path());
    let link = dataplane.attach_alloc(scope_path).expect("attach_alloc to alloc .scope");

    // AC 1a: bpftool cgroup show <alloc .scope> lists cgroup_connect4_mtls.
    assert!(
        cgroup_lists_program(topo.cgroup_path(), "cgroup_connect4_mtls"),
        "cgroup_connect4_mtls must be attached to the alloc's own .scope ({})",
        topo.cgroup_path(),
    );
    // AC 1b: the parent global slice (overdrive.slice) does NOT carry it — the F5
    // exemption made structural (NOT the global ancestor the service program uses).
    assert!(
        !cgroup_lists_program("/sys/fs/cgroup/overdrive.slice", "cgroup_connect4_mtls"),
        "cgroup_connect4_mtls must NOT be attached to the global overdrive.slice \
         (per-alloc scope only — F5 exemption)",
    );

    // --- AC 2: program the redirect, observe the map entry, then unprogram. ---
    dataplane.program_redirect(real_peer, leg_f_addr).expect("program_redirect");
    assert!(
        map_has_redirect_entry(real_peer, leg_f_addr),
        "MTLS_REDIRECT_DEST must show the (real_peer)→(leg_f) host-order entry after program_redirect",
    );

    // --- AC 3: a cgroup-isolated workload connect(real_peer) lands on leg-F. ---
    let rewrite_landed = workload_connect_lands_on_leg_f(&topo, &leg_f_listener, real_peer);
    assert!(
        rewrite_landed,
        "cgroup-isolated connect(real_peer) must be rewritten onto the agent's leg-F listener",
    );

    // AC 3b: a connect to an un-programmed dest passes through (map MISS → pass).
    // The un-programmed dest is unreachable, so the connect FAILS to land on leg-F
    // (no rewrite). We assert leg-F sees NO new connection within a short window.
    let unprogrammed = SocketAddrV4::new(UNPROGRAMMED_IP, UNPROGRAMMED_PORT);
    assert!(
        !workload_connect_lands_on_leg_f(&topo, &leg_f_listener, unprogrammed),
        "connect to an un-programmed dest must NOT be rewritten onto leg-F (map MISS → pass-through)",
    );

    // --- AC 2 (remove): unprogram removes the entry. ---
    dataplane.unprogram_redirect(real_peer).expect("unprogram_redirect");
    assert!(
        !map_has_redirect_entry(real_peer, leg_f_addr),
        "MTLS_REDIRECT_DEST entry must be gone after unprogram_redirect",
    );
    // Idempotent remove — absent key → Ok.
    dataplane
        .unprogram_redirect(real_peer)
        .expect("unprogram_redirect is idempotent on absent key");

    // --- AC 4: dropping the link detaches the program from the .scope. ---
    drop(link);
    assert!(
        !cgroup_lists_program(topo.cgroup_path(), "cgroup_connect4_mtls"),
        "dropping MtlsCgroupLink must detach cgroup_connect4_mtls from the .scope",
    );
}

/// RAII cleanup of the per-test bpffs pin dir (unlink the `SERVICE_MAP` pin + rmdir).
struct PinDirGuard(std::path::PathBuf);
impl Drop for PinDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join("SERVICE_MAP"));
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// The kernel truncates BPF program names to 15 chars (`BPF_OBJ_NAME_LEN - 1`), so
/// `cgroup_connect4_mtls` (20 chars) surfaces in `bpftool` as `cgroup_connect4`.
/// We match the truncated name AND the `connect4` attach type — the pair is what
/// distinguishes the mTLS intercept from any other program on the scope.
const MTLS_PROG_TRUNCATED: &str = "cgroup_connect4";
/// `bpftool` reports the `BPF_CGROUP_INET4_CONNECT` attach type verbatim (matches
/// AC 1 — "a `cgroup_inet4_connect` program").
const MTLS_ATTACH_TYPE: &str = "cgroup_inet4_connect";

/// `MTLS_REDIRECT_DEST` truncated to the kernel's 15-char `BPF_OBJ_NAME_LEN - 1`
/// ceiling — the name `bpftool map dump name` resolves against.
const MTLS_MAP_TRUNCATED: &str = "MTLS_REDIRECT_D";

/// `bpftool cgroup show <cgroup-dir> -j` — true iff a `connect4`-type program with
/// the (truncated) mTLS name is attached DIRECTLY on THAT cgroup directory. An
/// empty cgroup prints nothing (empty stdout) → false. The JSON form is a flat
/// array of attachment objects carrying `attach_type` + `name`.
fn cgroup_lists_program(cgroup_dir: &str, prog_name: &str) -> bool {
    // `prog_name` is the full source name; map it to the kernel-truncated form for
    // matching (the AC names the full program, the kernel stores 15 chars).
    let truncated: String = prog_name.chars().take(15).collect();
    let out = Command::new("bpftool")
        .args(["cgroup", "show", cgroup_dir, "-j"])
        .output()
        .expect("bpftool cgroup show spawn");
    if !out.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.trim().is_empty() {
        return false; // empty cgroup — no attachments
    }
    let json: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let Some(entries) = json.as_array() else { return false };
    entries.iter().any(|e| {
        let name = e.get("name").and_then(serde_json::Value::as_str).unwrap_or_default();
        let attach = e.get("attach_type").and_then(serde_json::Value::as_str).unwrap_or_default();
        name == truncated && name == MTLS_PROG_TRUNCATED && attach == MTLS_ATTACH_TYPE
    })
}

/// `bpftool map dump name MTLS_REDIRECT_DEST` — true iff the host-order
/// `(real_peer)→(leg_f)` entry is present. The dump renders key/value as hex byte
/// arrays; we match the host-order key bytes the production handle writes
/// (`u32::from(Ipv4Addr)` little-endian on the test arch + the u16 port).
fn map_has_redirect_entry(real_peer: SocketAddrV4, leg_f: SocketAddrV4) -> bool {
    // The kernel truncates map names to 15 chars (`BPF_OBJ_NAME_LEN - 1`), so
    // `MTLS_REDIRECT_DEST` (18 chars) is stored — and dumpable — as
    // `MTLS_REDIRECT_D`. `bpftool map dump name <full>` returns "can't parse name".
    let out = Command::new("bpftool")
        .args(["map", "dump", "name", MTLS_MAP_TRUNCATED, "-j"])
        .output()
        .expect("bpftool map dump spawn");
    if !out.status.success() {
        return false;
    }
    let json: serde_json::Value = match serde_json::from_slice(&out.stdout) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let key_bytes = dest_key_bytes(real_peer);
    let val_bytes = addr_port_bytes(leg_f);
    let Some(entries) = json.as_array() else { return false };
    entries.iter().any(|entry| {
        let key = hex_byte_array(entry.get("key"));
        let value = hex_byte_array(entry.get("value"));
        key == key_bytes && value == val_bytes
    })
}

/// Host-order 8-byte `MtlsDestKey` bytes for a peer (matches the production
/// `MtlsDestKey { ip_host, port_host, _pad }` `#[repr(C)]` layout).
fn dest_key_bytes(peer: SocketAddrV4) -> Vec<u8> {
    let mut v = Vec::with_capacity(8);
    v.extend_from_slice(&u32::from(*peer.ip()).to_ne_bytes());
    v.extend_from_slice(&peer.port().to_ne_bytes());
    v.extend_from_slice(&0u16.to_ne_bytes());
    v
}

/// Host-order 8-byte `MtlsAddrPort` bytes for the leg-F listener.
fn addr_port_bytes(leg_f: SocketAddrV4) -> Vec<u8> {
    dest_key_bytes(leg_f)
}

/// Parse a bpftool `-j` hex byte array (e.g. `["0x07","0x00",...]` or a flat
/// string) into raw bytes.
fn hex_byte_array(v: Option<&serde_json::Value>) -> Vec<u8> {
    let Some(v) = v else { return Vec::new() };
    if let Some(arr) = v.as_array() {
        return arr
            .iter()
            .filter_map(|b| b.as_str())
            .filter_map(|s| u8::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .collect();
    }
    Vec::new()
}

/// Spawn a cgroup-isolated workload in the netns that `connect()`s to `dest`,
/// writes a probe byte, and reads a reply. Returns true iff the leg-F listener
/// accepted a connection (the rewrite fired) and the round-trip completed.
fn workload_connect_lands_on_leg_f(
    topo: &MtlsTopology,
    leg_f_listener: &TcpListener,
    dest: SocketAddrV4,
) -> bool {
    leg_f_listener.set_nonblocking(false).expect("blocking leg-F listener");

    let probe = b"06-01-probe";
    let reply = b"06-01-reply";
    let script = format!(
        r#"
import socket, sys
try:
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(4)
    s.connect(("{ip}", {port}))
    s.sendall({probe})
    got = s.recv(64)
    sys.exit(0 if got == {reply} else 11)
except Exception as e:
    sys.stderr.write("workload err: %s\n" % e)
    sys.exit(20)
"#,
        ip = dest.ip(),
        port = dest.port(),
        probe = py_bytes(probe),
        reply = py_bytes(reply),
    );

    let procs = format!("{}/cgroup.procs", topo.cgroup_path());
    let mut cmd = Command::new("ip");
    cmd.args(["netns", "exec", topo.netns(), "python3", "-c", &script])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // SAFETY: pre_exec runs in the forked child before exec; writing our own pid
    // to cgroup.procs is a single async-signal-safe write. Placing the process in
    // the alloc's .scope is what makes its connect() fire cgroup_connect4_mtls.
    unsafe {
        cmd.pre_exec(move || {
            let pid = std::process::id();
            std::fs::write(&procs, pid.to_string())
                .map_err(|e| std::io::Error::other(format!("join cgroup: {e}")))?;
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("spawn workload");

    // Accept the (possibly rewritten) connection on leg-F within a window. If the
    // rewrite fired, the connect lands here; if it was a MISS pass-through to an
    // unreachable real dest, nothing arrives and accept times out.
    leg_f_listener.set_nonblocking(true).expect("nonblocking leg-F accept");
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let landed = loop {
        match leg_f_listener.accept() {
            Ok((mut conn, _)) => {
                let mut buf = [0u8; 64];
                let n = conn.read(&mut buf).unwrap_or(0);
                if &buf[..n] == probe {
                    let _ = conn.write_all(reply);
                    break true;
                }
                break false;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    break false;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => break false,
        }
    };
    let _ = child.wait();
    landed
}

/// Render bytes as a Python `bytes` literal for the inline workload script.
fn py_bytes(b: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::from("b\"");
    for &byte in b {
        let _ = write!(s, "\\x{byte:02x}");
    }
    s.push('"');
    s
}
