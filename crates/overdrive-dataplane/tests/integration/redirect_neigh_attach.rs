//! S-2.2-33 — Loader attach topology under ADR-0045
//! `bpf_redirect`-on-XDP datapath.
//!
//! Tags: `@US-09` `@slice-09` `@real-io @adapter-integration`.
//! Tier: Tier 3 (real-kernel integration).
//!
//! Verifies the post-pivot loader contract:
//!
//! 1. `xdp_service_map_lookup` is loaded and attached on the
//!    client-facing veth (`lb_veth_a`) ingress.
//! 2. `xdp_reverse_nat_lookup` is loaded and attached on the
//!    backend-facing veth (`lb_veth_b`) ingress.
//! 3. NO TC-classifier program is attached on either iface — the
//!    pre-ADR-0045 `tc_reverse_nat` egress attach is structurally
//!    retired.
//!
//! Driving port: `EbpfDataplane::new_with_pin_dir(client_iface,
//! backend_iface, pin_dir)`. Observable assertions go through
//! `bpftool net show <iface> -j` and `bpftool prog show -j` —
//! kernel-side state per `.claude/rules/testing.md` § "Assertion
//! rules": never on program-internal reachability.
//!
//! Native vs SKB fallback (Slice 01 `should_fallback_to_generic`
//! contract) is exercised on BOTH XDP attach call sites; we do not
//! retest the classifier here — the unit tests in `lib.rs` pin
//! `EOPNOTSUPP`/`ENOTSUP` → fallback and `EINVAL`/`EPERM` →
//! propagate.
//!
//! See `docs/product/architecture/adr-0045-bpf-redirect-neigh-datapath.md`
//! § Operational for the topology rationale.

#![allow(clippy::missing_panics_doc)]
// `expect_used` is workspace-wide `warn` per `.claude/rules/development.md`
// § Errors. Tier 3 tests use `.expect(...)` to surface fail-fast at the
// assertion site, matching the convention in sibling integration tests.
#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::significant_drop_tightening,
    clippy::doc_lazy_continuation,
    clippy::unwrap_used,
    clippy::doc_markdown
)]

use std::path::PathBuf;
use std::process::Command;

use super::helpers::netns::{NetNsError, ThreeIfaceTopology};

/// Kernel-observable name for the forward-path program. The kernel
/// truncates BPF program names to `BPF_OBJ_NAME_LEN` = 16 (15 chars
/// + NUL) at load time; Rust source name is `xdp_service_map_lookup`
/// (22 chars). Aya's section-name-based `program_mut(...)` lookup
/// uses the untruncated form, but `bpftool prog show id <id>`
/// displays the truncated 15-char kernel name. Both the loader's
/// `program_mut("xdp_service_map_lookup")` lookup AND the test's
/// `bpftool` observation are correct against their respective
/// surfaces; the names diverge by construction.
const KERNEL_NAME_FORWARD: &str = "xdp_service_map";
/// Kernel-observable name for the reverse-path program. Same
/// truncation rule as `KERNEL_NAME_FORWARD`: Rust source is
/// `xdp_reverse_nat_lookup` (22 chars), kernel-observable form is
/// the first 15 chars.
const KERNEL_NAME_REVERSE: &str = "xdp_reverse_nat";

/// Per-process bpffs pin dir guard. Drops on test exit (success or
/// panic), keeping the global `/sys/fs/bpf` namespace clean across
/// test runs.
struct PinDirGuard(PathBuf);

impl Drop for PinDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// S-2.2-33 — ADR-0045 attach topology.
///
/// Drives `EbpfDataplane::new_with_pin_dir(client_iface,
/// backend_iface, pin_dir)` against a fresh `ThreeIfaceTopology`
/// inside `lb-ns`. Asserts:
///
///   a. `xdp_service_map_lookup` is attached on `lb_veth_a` ingress.
///   b. `xdp_reverse_nat_lookup` is attached on `lb_veth_b` ingress.
///   c. No TC-classifier program is attached on either iface
///      (egress or ingress).
///
/// Capability-gated: requires `CAP_NET_ADMIN` for netns + veth
/// creation. Bails early with a skip message on EPERM/EACCES.
///
/// Lima wrapper (`cargo xtask lima run --`) runs as root by default;
/// CI runs the integration job as root.
#[test]
#[serial_test::serial(net_admin)]
fn loader_attaches_xdp_reverse_nat_on_backend_veth_and_retires_tc_egress() {
    use overdrive_dataplane::EbpfDataplane;

    if !has_cap_net_admin() {
        eprintln!(
            "skip: S-2.2-33 needs CAP_NET_ADMIN — run via \
             `cargo xtask lima run --` (default-root)"
        );
        return;
    }

    // Build the 3-iface topology — `client-ns` <-> `lb-ns` <-> `backend-ns`.
    let topo = match ThreeIfaceTopology::create("rn3") {
        Ok(t) => t,
        Err(NetNsError::CapNetAdminRequired) => {
            eprintln!("skip: S-2.2-33 needs CAP_NET_ADMIN (netns)");
            return;
        }
        Err(e) => panic!("3-iface topology setup failed: {e}"),
    };

    // Per-test bpffs pin dir for SERVICE_MAP pin-by-name. The bpffs
    // mount lives at `/sys/fs/bpf`; subdirectories are individual
    // contexts. Per-process tag avoids cross-test collision when
    // nextest runs scenarios in parallel.
    let pin_dir = PathBuf::from(format!("/sys/fs/bpf/overdrive-test-rn3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pin_dir);
    std::fs::create_dir_all(&pin_dir).expect("create per-test pin dir");
    let _pin_guard = PinDirGuard(pin_dir.clone());

    // Enter `lb-ns` for the duration of the loader call; XDP +
    // bpftool resolutions go against the calling thread's netns.
    let _ns_guard = enter_netns(&topo.lb_ns.name).expect("setns into lb-ns");

    // Driving port — the loader. Two ifaces under ADR-0045: client-
    // facing for forward path, backend-facing for reverse path.
    let _dataplane = EbpfDataplane::new_with_pin_dir(&topo.lb_veth_a, &topo.lb_veth_b, &pin_dir)
        .expect("EbpfDataplane::new_with_pin_dir(lb_veth_a, lb_veth_b, pin_dir) must succeed");

    // === Assertion (a) — xdp_service_map_lookup on lb_veth_a ingress. ===
    //
    // The kernel truncates program names to `BPF_OBJ_NAME_LEN` = 16
    // (15 chars + NUL), so `xdp_service_map_lookup` (22 chars)
    // surfaces through `bpftool prog show` as `xdp_service_map`
    // (the first 15 chars). Aya's `program_mut("...")` lookup uses
    // the section name (untruncated), but the kernel-observable
    // name IS the truncated form. Assert against that.
    let xdp_a = read_xdp_prog_name(&topo.lb_ns.name, &topo.lb_veth_a)
        .expect("read XDP prog name for lb_veth_a");
    assert_eq!(
        xdp_a.as_deref(),
        Some(KERNEL_NAME_FORWARD),
        "lb_veth_a must carry the xdp_service_map_lookup program (kernel-truncated to {KERNEL_NAME_FORWARD:?}) on ingress; \
         observed {xdp_a:?}"
    );

    // === Assertion (b) — xdp_reverse_nat_lookup on lb_veth_b ingress. ===
    let xdp_b = read_xdp_prog_name(&topo.lb_ns.name, &topo.lb_veth_b)
        .expect("read XDP prog name for lb_veth_b");
    assert_eq!(
        xdp_b.as_deref(),
        Some(KERNEL_NAME_REVERSE),
        "lb_veth_b must carry the xdp_reverse_nat_lookup program (kernel-truncated to {KERNEL_NAME_REVERSE:?}) on ingress; \
         observed {xdp_b:?}"
    );

    // === Assertion (c) — no TC-classifier on either iface. ===
    //
    // Pre-ADR-0045, the loader attached `tc_reverse_nat` to
    // `lb_veth_a` egress. Step 09-03 retires that attach (and
    // deletes the program source). `bpftool net show <iface> -j`
    // reports `tc` programs as a non-empty array when present —
    // assert empty on both ifaces (egress AND ingress).
    let tc_a =
        read_tc_progs(&topo.lb_ns.name, &topo.lb_veth_a).expect("read TC progs for lb_veth_a");
    assert!(
        tc_a.is_empty(),
        "lb_veth_a must carry no TC programs after ADR-0045 retirement; \
         observed {tc_a:?}"
    );
    let tc_b =
        read_tc_progs(&topo.lb_ns.name, &topo.lb_veth_b).expect("read TC progs for lb_veth_b");
    assert!(tc_b.is_empty(), "lb_veth_b must carry no TC programs; observed {tc_b:?}");
}

/// Pre-flight: do we have `CAP_NET_ADMIN`? Heuristic via UID — root
/// has it implicitly; non-root callers need explicit grant which the
/// test environment does not arrange.
fn has_cap_net_admin() -> bool {
    // SAFETY: `getuid()` is async-signal-safe and never errors.
    let uid = unsafe { libc::getuid() };
    uid == 0
}

/// Enter `target_ns` via `setns(2)` against
/// `/var/run/netns/<name>`. Returns a guard that reverts the calling
/// thread's netns on Drop. Mirrors the helper in
/// `reverse_nat_e2e.rs`; kept inline so this test does not depend on
/// the helper's visibility surface.
fn enter_netns(target_ns: &str) -> std::io::Result<NetNsGuard> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    let prior_fd = {
        let path = std::ffi::CString::new("/proc/self/ns/net").expect("CString /proc");
        // SAFETY: open(O_RDONLY) on a kernel-managed path; close on Drop.
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: fd produced above is owned; OwnedFd takes ownership.
        unsafe { OwnedFd::from_raw_fd(fd) }
    };

    let target_path = format!("/var/run/netns/{target_ns}");
    let cstr = std::ffi::CString::new(target_path).expect("CString netns path");
    let target_fd = {
        // SAFETY: open(O_RDONLY) on a netns mount; close on Drop.
        let fd = unsafe { libc::open(cstr.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: fd produced above is owned.
        unsafe { OwnedFd::from_raw_fd(fd) }
    };

    // SAFETY: setns to a network namespace; subsequent BPF / iface
    // ops run within it.
    let rc = unsafe { libc::setns(target_fd.as_raw_fd(), libc::CLONE_NEWNET) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(NetNsGuard { prior_fd: Some(prior_fd) })
}

struct NetNsGuard {
    prior_fd: Option<std::os::fd::OwnedFd>,
}

impl Drop for NetNsGuard {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        if let Some(fd) = self.prior_fd.take() {
            // SAFETY: best-effort revert; failure is logged-and-
            // continue territory since the test process exits soon
            // after.
            let _ = unsafe { libc::setns(fd.as_raw_fd(), libc::CLONE_NEWNET) };
        }
    }
}

/// Read the XDP program name attached to `iface` inside `ns_name`
/// via two bpftool calls:
///
///   1. `bpftool net show -j` — list all XDP/TC attachments in the
///      netns. Per bpftool 7.x, `xdp` array entries carry
///      `{ "devname", "ifindex", "id", "mode" }` — NO `name` field.
///      Filter by `devname == iface` to find ours.
///   2. `bpftool prog show id <id> -j` — resolve the program ID to
///      its symbolic `name`.
///
/// Returns `Some(name)` if exactly one XDP program is attached on
/// the iface, `None` if none. Multi-attach is unexpected and
/// surfaces as `Err`.
fn read_xdp_prog_name(ns_name: &str, iface: &str) -> Result<Option<String>, String> {
    let net_json = run_bpftool_net_show(ns_name)?;
    let ids = parse_xdp_prog_ids(&net_json, iface)?;
    match ids.as_slice() {
        [] => Ok(None),
        [id] => Ok(Some(resolve_prog_name(ns_name, *id)?)),
        many => Err(format!("unexpected multi-XDP attach on {iface}: ids={many:?}")),
    }
}

/// Read the TC program IDs attached to `iface` (any direction)
/// inside `ns_name`, resolve each to a name. Returns the Vec
/// (possibly empty) of attached classifier names.
fn read_tc_progs(ns_name: &str, iface: &str) -> Result<Vec<String>, String> {
    let net_json = run_bpftool_net_show(ns_name)?;
    let ids = parse_tc_prog_ids(&net_json, iface)?;
    let mut names = Vec::with_capacity(ids.len());
    for id in ids {
        names.push(resolve_prog_name(ns_name, id)?);
    }
    Ok(names)
}

/// `ip netns exec <ns> bpftool net show -j` — the per-netns global
/// view. The `dev <iface>` filter on bpftool 7.4.0 is silently
/// ignored, so we read all attaches and filter ourselves.
fn run_bpftool_net_show(ns_name: &str) -> Result<String, String> {
    let out = Command::new("ip")
        .args(["netns", "exec", ns_name, "bpftool", "net", "show", "-j"])
        .output()
        .map_err(|e| format!("bpftool spawn failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "bpftool net show -j failed (status={:?}): {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("bpftool stdout not UTF-8: {e}"))
}

/// `ip netns exec <ns> bpftool prog show id <id> -j` — resolve a
/// program ID to its symbolic `name`. The output shape is a single
/// JSON object (NOT an array): `{ "id": u32, "name": "...", ... }`.
fn resolve_prog_name(ns_name: &str, id: u64) -> Result<String, String> {
    let out = Command::new("ip")
        .args(["netns", "exec", ns_name, "bpftool", "prog", "show", "id", &id.to_string(), "-j"])
        .output()
        .map_err(|e| format!("bpftool prog show spawn failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "bpftool prog show id {id} -j failed (status={:?}): {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let json = String::from_utf8(out.stdout)
        .map_err(|e| format!("bpftool prog show stdout not UTF-8: {e}"))?;
    parse_prog_name(&json)
}

/// Parse `bpftool net show -j` output for `xdp` program IDs on the
/// given iface (matched by `devname`). Per bpftool 7.x, the
/// top-level shape is an array; each entry carries `xdp` (array),
/// `tc` (array), `flow_dissector` (array), `netfilter` (array).
/// Each element of `xdp` has `{ "devname", "ifindex", "id", "mode" }`.
fn parse_xdp_prog_ids(json: &str, iface: &str) -> Result<Vec<u64>, String> {
    let val: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("bpftool json parse: {e} — output: {json}"))?;
    let entries =
        val.as_array().ok_or_else(|| format!("expected JSON array at top level, got: {json}"))?;
    let mut ids: Vec<u64> = Vec::new();
    for entry in entries {
        let xdp = match entry.get("xdp") {
            Some(arr) => arr
                .as_array()
                .ok_or_else(|| format!("`xdp` field must be an array; got: {entry:?}"))?,
            None => continue,
        };
        for prog in xdp {
            let devname = prog
                .get("devname")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("xdp entry missing `devname`: {prog:?}"))?;
            if devname != iface {
                continue;
            }
            let id = prog
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format!("xdp entry missing `id`: {prog:?}"))?;
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Parse `bpftool net show -j` output for `tc` program IDs on the
/// given iface (matched by `devname`). Same shape as `xdp` — each
/// entry has `{ "devname", "ifindex", "id", "kind" }` plus
/// direction metadata.
fn parse_tc_prog_ids(json: &str, iface: &str) -> Result<Vec<u64>, String> {
    let val: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("bpftool json parse: {e} — output: {json}"))?;
    let entries =
        val.as_array().ok_or_else(|| format!("expected JSON array at top level, got: {json}"))?;
    let mut ids: Vec<u64> = Vec::new();
    for entry in entries {
        let tc = match entry.get("tc") {
            Some(arr) => arr
                .as_array()
                .ok_or_else(|| format!("`tc` field must be an array; got: {entry:?}"))?,
            None => continue,
        };
        for prog in tc {
            let devname = prog
                .get("devname")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("tc entry missing `devname`: {prog:?}"))?;
            if devname != iface {
                continue;
            }
            let id = prog
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format!("tc entry missing `id`: {prog:?}"))?;
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Parse `bpftool prog show id <id> -j` output. The output is a
/// single JSON object with at least `{ "id": u32, "name": "..." }`.
fn parse_prog_name(json: &str) -> Result<String, String> {
    let val: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("bpftool prog show json parse: {e} — output: {json}"))?;
    let name = val
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("prog show output missing `name`: {json}"))?;
    Ok(name.to_owned())
}

#[cfg(test)]
mod parser_tests {
    //! Pure unit tests for the bpftool JSON parsers — exercise the
    //! parser on synthetic inputs without spawning bpftool. Real
    //! integration coverage is the parent test driving the live
    //! kernel.

    use super::{parse_prog_name, parse_tc_prog_ids, parse_xdp_prog_ids};

    #[test]
    fn parses_xdp_attach_native_mode_for_target_iface() {
        // Representative bpftool 7.x shape — top-level array, one
        // entry covering the netns. Each `xdp` element carries
        // `{ "devname", "ifindex", "id", "mode" }`. NO `name` field —
        // resolution to a name happens via a second `bpftool prog
        // show id <id>` call.
        let json = r#"[{
            "xdp": [{"devname": "lb_veth_a", "ifindex": 7, "id": 42, "mode": "driver"}],
            "tc": [],
            "flow_dissector": [],
            "netfilter": []
        }]"#;
        assert_eq!(parse_xdp_prog_ids(json, "lb_veth_a").unwrap(), vec![42_u64]);
    }

    #[test]
    fn parses_xdp_attach_generic_mode_after_skb_fallback() {
        let json = r#"[{
            "xdp": [{"devname": "lb_veth_b", "ifindex": 9, "id": 99, "mode": "skb"}],
            "tc": []
        }]"#;
        assert_eq!(parse_xdp_prog_ids(json, "lb_veth_b").unwrap(), vec![99_u64]);
    }

    #[test]
    fn ignores_xdp_attach_on_other_iface() {
        // The netns may carry XDP attaches on unrelated ifaces (the
        // backend-ns `xdp_pass` stub, the lo iface, etc.); the
        // filter on `devname` must isolate the target.
        let json = r#"[{
            "xdp": [
                {"devname": "lo", "ifindex": 1, "id": 5, "mode": "skb"},
                {"devname": "lb_veth_a", "ifindex": 7, "id": 42, "mode": "driver"}
            ],
            "tc": []
        }]"#;
        assert_eq!(parse_xdp_prog_ids(json, "lb_veth_a").unwrap(), vec![42_u64]);
    }

    #[test]
    fn empty_xdp_array_is_empty_vec() {
        let json = r#"[{"xdp": [], "tc": []}]"#;
        assert!(parse_xdp_prog_ids(json, "lb_veth_a").unwrap().is_empty());
    }

    #[test]
    fn missing_xdp_field_is_empty_vec() {
        let json = r#"[{"tc": []}]"#;
        assert!(parse_xdp_prog_ids(json, "lb_veth_a").unwrap().is_empty());
    }

    #[test]
    fn parses_tc_empty_array() {
        let json = r#"[{"xdp": [], "tc": []}]"#;
        assert!(parse_tc_prog_ids(json, "lb_veth_a").unwrap().is_empty());
    }

    #[test]
    fn parses_tc_attached_program_id() {
        // Pre-ADR-0045 shape — would have surfaced before step 09-03.
        let json = r#"[{
            "xdp": [],
            "tc": [{"devname": "lb_veth_a", "ifindex": 7, "id": 17, "kind": "sch_clsact/egress"}]
        }]"#;
        assert_eq!(parse_tc_prog_ids(json, "lb_veth_a").unwrap(), vec![17_u64]);
    }

    #[test]
    fn rejects_non_array_top_level() {
        let json = r#"{"xdp": []}"#;
        assert!(parse_xdp_prog_ids(json, "lb_veth_a").is_err());
    }

    #[test]
    fn parses_prog_show_name() {
        let json = r#"{"id": 42, "type": "xdp", "name": "xdp_service_map_lookup", "tag": "abc"}"#;
        assert_eq!(parse_prog_name(json).unwrap(), "xdp_service_map_lookup");
    }
}
