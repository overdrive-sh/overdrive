//! Tier-3 regression for the single-node serve-boot dataplane wiring
//! (step 01-04 of `single-node-dataplane-wiring`, ADR-0061 § 8 / § 5).
//!
//! Proves the two-iface invariant holds at the *kernel*, against the
//! real `provision` + `EbpfDataplane::new_with_pin_dir` boot path:
//!
//! 1. HAPPY (ADR-0061 § 8): the provisioner stands up a distinct veth
//!    pair, the two DISTINCT XDP programs (`xdp_service_map_lookup` on
//!    the client side, `xdp_reverse_nat_lookup` on the backend side)
//!    BOTH attach to their two distinct veth ifaces, and construction
//!    completes with NO `DataplaneBootError` (the EBUSY no longer fires
//!    on the default path). Asserted via OBSERVABLE KERNEL STATE per
//!    `.claude/rules/testing.md` § "Tier 3 — assertion rules" — both
//!    ifaces carry an attached XDP program (`ip link show` shows an
//!    `xdp` marker + a `prog/id` on two distinct ifindexes), and the
//!    construction returns Ok.
//!
//! 2. DIAGNOSTIC (ADR-0061 § 5): when both ifaces are pointed at ONE
//!    real iface (the residual operator-error case), the second attach
//!    returns a REAL `EBUSY` (the kernel permits exactly one XDP
//!    program per netdev XDP hook), and the boot error is the typed
//!    `DataplaneError::IfaceXdpSlotBusy { iface }` wrapped in
//!    `DataplaneBootError::Construct` — NOT a masking DRV_MODE
//!    `LoadFailed`. This is the ONLY place a REAL `EBUSY` is exercised:
//!    01-01's unit tests drove the classifier against synthetic
//!    `SyscallError` values (Lima virtio-net never produces a real
//!    `EBUSY` on a fresh veth); this Tier-3 path produces a real one.
//!
//! This file drives the SAME seam the production `else` branch in
//! `run_server` uses (`crates/overdrive-control-plane/src/lib.rs`
//! ~L1031-1115): `veth_provisioner::{derive_veth_plan, provision}`
//! followed by `EbpfDataplane::new_with_pin_dir(client, backend,
//! pin_dir, cgroup)` with the construct failure wrapped into
//! `DataplaneBootError::Construct`. It deliberately does NOT call
//! `run_server` (TLS, ports, stores are out of scope for this
//! invariant). No production seam extraction was needed.
//!
//! `integration-tests`-gated (real network + BPF I/O, needs
//! `CAP_NET_ADMIN` + `CAP_BPF` + the built BPF object at
//! `target/bpf/overdrive_bpf.o`) and `#[cfg(target_os = "linux")]`.
//! The unprivileged Lima `lima` user lacks the capabilities; the
//! canonical inner-loop path is `cargo xtask lima run --` (runs as
//! root). On EPERM the test SKIPS rather than fails.
//!
//! LEFTOVER-XDP / veth HYGIENE (`.claude/rules/debugging.md`
//! § "Leftover XDP attachments across runs"): every veth this test
//! creates is deleted on scope exit via a `VethGuard` `Drop` impl
//! (deleting one end reaps both ends AND detaches any XDP program on
//! it), AND the `EbpfDataplane`'s own `XdpLinkId` `Drop` detaches the
//! programs first. A leaked XDP program on a host-netns veth breaks
//! not just this suite but other tests AND other Conductor workspaces
//! sharing the Lima VM.

#![cfg(target_os = "linux")]
// Skip-on-no-privilege / no-bpf-object messages are the legitimate way
// these Tier-3 tests communicate "capability/artifact absent, scenario
// skipped" on an unprivileged runner — `eprintln!` is exactly right.
#![allow(clippy::print_stderr)]
// `expect` / `unwrap` are the standard idiom in test code.
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use ipnet::Ipv4Net;
use overdrive_control_plane::error::DataplaneBootError;
use overdrive_control_plane::veth_provisioner::{
    VethProvisionError, VethProvisionPlan, derive_veth_plan, provision,
};
use overdrive_core::traits::dataplane::DataplaneError;
use overdrive_dataplane::EbpfDataplane;

/// Per-test iface names — suffixed with the PID (so two parallel test
/// binaries do not collide on the global host-netns iface namespace)
/// AND a per-scenario `tag` (so the two scenarios in this file, which
/// run in the same binary, do not share a veth pair or a route). Linux
/// IFNAMSIZ = 16 (15 usable); `sbcli`/`sbbk` + 1 tag + 4 hex ≤ 12 chars.
fn iface_names(tag: char) -> (String, String) {
    let suffix = std::process::id() & 0xffff;
    (format!("sbcli{tag}{suffix:04x}"), format!("sbbk{tag}{suffix:04x}"))
}

/// RAII guard: deletes the named veth pair (one `ip link del` reaps both
/// ends AND detaches any XDP program attached to either end) on scope
/// exit, even on panic. The mandatory leftover-XDP / veth hygiene per
/// `.claude/rules/debugging.md`.
struct VethGuard {
    client: String,
}

impl VethGuard {
    fn new(client: &str) -> Self {
        Self { client: client.to_owned() }
    }
}

impl Drop for VethGuard {
    fn drop(&mut self) {
        // Best-effort: deleting one end of a veth pair removes both ends
        // and tears down any XDP attachment on them.
        let _ = Command::new("ip").args(["link", "del", &self.client]).output();
    }
}

/// RAII guard for the per-test bpffs pin dir (the `SERVICE_MAP` `HoM`
/// is pinned-by-name there; a stale dir poisons the next run).
struct PinDirGuard(PathBuf);

impl Drop for PinDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn delete_pair(client: &str) {
    let _ = Command::new("ip").args(["link", "del", client]).output();
}

/// Returns `true` if the provision skipped due to missing
/// `CAP_NET_ADMIN` (so the test can bail with a skip rather than fail on
/// an unprivileged runner).
fn is_cap_skip(err: &VethProvisionError) -> bool {
    let msg = err.to_string();
    msg.contains("Operation not permitted") || msg.contains("Permission denied")
}

/// Returns `true` when a `DataplaneError` is a capability / privilege
/// failure (EPERM/EACCES) rather than a genuine wiring bug — so the
/// construct-path tests can SKIP on an unprivileged runner instead of
/// failing.
fn dataplane_err_is_cap_skip(err: &DataplaneError) -> bool {
    let msg = err.to_string();
    msg.contains("Operation not permitted")
        || msg.contains("Permission denied")
        || msg.contains("EPERM")
        || msg.contains("EACCES")
}

fn plan_for(client: &str, backend: &str, cidr: &str) -> VethProvisionPlan {
    let range: Ipv4Net = cidr.parse().expect("valid /24");
    derive_veth_plan(client, backend, range)
}

/// Observable kernel state: does `iface` carry an attached XDP program?
///
/// `ip link show <iface>` prints an `xdp`/`xdpgeneric`/`xdpdrv` token and
/// a `prog/id N` for an iface with an attached XDP program; a bare veth
/// prints neither. Asserting on this output is a kernel-side side-effect
/// assertion per `.claude/rules/testing.md` § "Tier 3 — assertion
/// rules" — NOT a program-internal-reachability assertion.
fn xdp_prog_id_on(iface: &str) -> Option<String> {
    let out = Command::new("ip").args(["-details", "link", "show", iface]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // An attached XDP program renders as e.g. "... prog/xdp id 42 ..."
    // or "xdpgeneric ... prog/xdp id 42". Require BOTH the xdp marker and
    // a `prog/xdp id <N>` so a bare veth (no XDP) returns None.
    if !text.contains("xdp") {
        return None;
    }
    let idx = text.find("prog/xdp id ")?;
    let rest = &text[idx + "prog/xdp id ".len()..];
    let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if id.is_empty() { None } else { Some(id) }
}

/// Build a per-test bpffs pin dir + its cleanup guard.
fn make_pin_dir(tag: char) -> (PathBuf, PinDirGuard) {
    let pin_dir =
        PathBuf::from(format!("/sys/fs/bpf/overdrive-test-sbveth-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pin_dir);
    std::fs::create_dir_all(&pin_dir).expect("create pin dir");
    let guard = PinDirGuard(pin_dir.clone());
    (pin_dir, guard)
}

/// HAPPY PATH (ADR-0061 § 8): default-shaped single-node veth config →
/// provisioner stands up the pair → the two DISTINCT XDP programs attach
/// to two distinct veth ifaces → construction Ok, NO `DataplaneBootError`.
///
/// Asserted via observable kernel state: each of the two distinct ifaces
/// carries an attached XDP program (`ip link show` `prog/xdp id`), and
/// the two program ids are distinct (two distinct programs, not one).
#[test]
fn serve_boot_attaches_two_distinct_xdp_programs_to_two_distinct_veths() {
    let (client, backend) = iface_names('h');
    delete_pair(&client);
    let _veth_guard = VethGuard::new(&client);

    // Step 1 — provision the veth pair (the production default path).
    let plan = plan_for(&client, &backend, "10.96.0.0/24");
    match provision(&plan) {
        Ok(()) => {}
        Err(err) if is_cap_skip(&err) => {
            eprintln!(
                "SKIP serve_boot_attaches_two_distinct_xdp_programs: CAP_NET_ADMIN required ({err})"
            );
            return;
        }
        Err(err) => panic!("provision failed for a non-privilege reason: {err}"),
    }

    let (pin_dir, _pin_guard) = make_pin_dir('h');

    // Step 2 — construct the dataplane against the TWO DISTINCT veth
    // ifaces. This drives the same seam the production `else` branch
    // uses (`new_with_pin_dir` → `DataplaneBootError::Construct` wrap).
    // On the two-distinct-iface default path the EBUSY MUST NOT fire.
    let dataplane = match EbpfDataplane::new_with_pin_dir(
        &client,
        &backend,
        &pin_dir,
        Path::new("/sys/fs/cgroup"),
    )
    .map_err(|source| DataplaneBootError::Construct {
        client_iface: client.clone(),
        backend_iface: backend.clone(),
        source,
    }) {
        Ok(dp) => dp,
        Err(DataplaneBootError::Construct { source, .. }) if dataplane_err_is_cap_skip(&source) => {
            eprintln!(
                "SKIP serve_boot_attaches_two_distinct_xdp_programs: CAP_BPF/CAP_SYS_ADMIN required ({source})"
            );
            return;
        }
        Err(other) => panic!(
            "construction must succeed on the two-distinct-iface default path, \
             got DataplaneBootError: {other}"
        ),
    };

    // Construction returned Ok — assert the observable kernel state:
    // each distinct iface carries a distinct attached XDP program.
    let client_prog = xdp_prog_id_on(&client);
    let backend_prog = xdp_prog_id_on(&backend);

    assert!(
        client_prog.is_some(),
        "client_iface {client} must carry an attached XDP program after construction \
         (ip link show prog/xdp id); got none"
    );
    assert!(
        backend_prog.is_some(),
        "backend_iface {backend} must carry an attached XDP program after construction \
         (ip link show prog/xdp id); got none"
    );
    assert_ne!(
        client_prog, backend_prog,
        "the two ifaces must carry TWO DISTINCT XDP programs \
         (xdp_service_map_lookup on client, xdp_reverse_nat_lookup on backend) — \
         distinct prog ids prove the two-iface invariant holds at the kernel"
    );

    // Explicit drop: detaches both XDP programs (XdpLinkId::Drop) before
    // the VethGuard reaps the pair. Belt-and-suspenders RAII hygiene.
    drop(dataplane);
}

/// DIAGNOSTIC (ADR-0061 § 5): pointing BOTH client + backend at ONE real
/// iface (the residual operator-error case) produces a REAL kernel
/// `EBUSY` on the second XDP attach — the kernel permits exactly one XDP
/// program per netdev hook. The boot error is the typed
/// `DataplaneError::IfaceXdpSlotBusy { iface }` wrapped in
/// `DataplaneBootError::Construct`, NOT a masking `DRV_MODE` `LoadFailed`.
///
/// This proves 01-01's `classify_iface_xdp_slot_busy` fires on a REAL
/// `EBUSY` (its unit tests used synthetic `SyscallError` values).
#[test]
fn single_iface_collision_surfaces_typed_iface_xdp_slot_busy() {
    let (single, _unused_backend) = iface_names('d');
    delete_pair(&single);
    let _veth_guard = VethGuard::new(&single);

    // Stand up ONE real veth pair; we drive BOTH the client and backend
    // attach against the SAME `single` iface to force the collision.
    // `provision` creates `single` + its peer; only `single` is used.
    let peer = format!("{single}p");
    match Command::new("ip")
        .args(["link", "add", &single, "type", "veth", "peer", "name", &peer])
        .output()
    {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("Operation not permitted") || stderr.contains("Permission denied") {
                eprintln!(
                    "SKIP single_iface_collision_surfaces_typed_iface_xdp_slot_busy: CAP_NET_ADMIN required ({stderr})"
                );
                return;
            }
            panic!("ip link add {single} failed for a non-privilege reason: {stderr}");
        }
        Err(err) => panic!("spawning `ip link add` failed: {err}"),
    }
    let _ = Command::new("ip").args(["link", "set", &single, "up"]).output();

    let (pin_dir, _pin_guard) = make_pin_dir('d');

    // Both ifaces = the SAME `single` iface. The forward XDP program
    // attaches first; the reverse XDP program then hits the occupied
    // XDP hook and the kernel returns a REAL `EBUSY`.
    let result =
        EbpfDataplane::new_with_pin_dir(&single, &single, &pin_dir, Path::new("/sys/fs/cgroup"))
            .map_err(|source| DataplaneBootError::Construct {
                client_iface: single.clone(),
                backend_iface: single.clone(),
                source,
            });

    match result {
        Ok(_dp) => panic!(
            "construction MUST fail when both ifaces point at one real iface — \
             the second XDP attach should hit EBUSY (ADR-0061 § 5)"
        ),
        Err(DataplaneBootError::Construct {
            source: DataplaneError::IfaceXdpSlotBusy { iface },
            ..
        }) => {
            // The typed variant fired on a REAL EBUSY (not synthetic) and
            // names the colliding iface — the honest slot-collision
            // diagnostic, NOT a masking DRV_MODE LoadFailed.
            assert_eq!(
                iface, single,
                "IfaceXdpSlotBusy must name the colliding iface {single}, got {iface}"
            );
        }
        Err(DataplaneBootError::Construct { source, .. }) if dataplane_err_is_cap_skip(&source) => {
            eprintln!(
                "SKIP single_iface_collision_surfaces_typed_iface_xdp_slot_busy: CAP_BPF/CAP_SYS_ADMIN required ({source})"
            );
        }
        Err(other) => panic!(
            "single-iface collision must surface DataplaneError::IfaceXdpSlotBusy \
             wrapped in DataplaneBootError::Construct (NOT a masking LoadFailed), got: {other}"
        ),
    }
}
