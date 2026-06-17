//! Tier-3 real-kernel scenario for the per-allocation netns + veth
//! provisioner (step 02-02 of `transparent-mtls-enrollment`, Path A /
//! ADR-0071; the per-allocation parallel of the host-netns
//! `veth_provision_idempotent.rs` Tier-3 suite, ADR-0061 § 3.1).
//!
//! Drives the real `ip netns` / `ip -n <ns>` / `sysctl` / `ethtool`
//! shell-out through
//! [`overdrive_control_plane::veth_provisioner::provision_workload_netns`]
//! and [`teardown_workload_netns`] against a fresh per-allocation netns.
//! `integration-tests`-gated (real network I/O, needs `CAP_NET_ADMIN`)
//! and `#[cfg(target_os = "linux")]`. The unprivileged Lima `lima` user
//! lacks `CAP_NET_ADMIN`; the canonical inner-loop path is
//! `cargo xtask lima run --` (runs as root). When `ip` returns EPERM the
//! test SKIPS rather than fails.
//!
//! Assertions are on OBSERVABLE KERNEL SIDE EFFECTS only (testing.md
//! Tier-3 assertion rules): `ip netns list`, `ip -n <ns> link/addr/route`
//! for up-state of BOTH veth ends + the netns `lo`, addresses, the default
//! route, `sysctl` for `ip_forward` / `rp_filter`, `ethtool -k` for tx
//! offload — NEVER on internal reachability.
//!
//! Cleanup: a per-test UNIQUE high [`NetSlot`] keeps the slot-derived
//! `ovd-ns-<4hex>` / `ovd-hv-<4hex>` / `ovd-wl-<4hex>` names from
//! colliding with other suites, and an RAII guard tears the netns + host
//! veth down on drop. A stale `ovd-ns-*` netns / `ovd-hv-*` veth from a
//! crashed prior run would otherwise poison the "fresh provision" / "zero
//! residue" assertions (per `.claude/rules/debugging.md` netns/veth-sweep
//! discipline).

#![cfg(target_os = "linux")]
// Skip-on-no-privilege messages are the legitimate way these Tier-3 tests
// communicate "CAP_NET_ADMIN absent, scenario skipped" on an unprivileged
// runner — `eprintln!` to the test log is exactly right.
#![allow(clippy::print_stderr)]

use overdrive_control_plane::veth_provisioner::{
    NetSlot, VethProvisionError, WorkloadNetnsPlan, derive_workload_netns_plan,
    provision_workload_netns, teardown_workload_netns,
};
use std::net::Ipv4Addr;
use std::process::Command;

/// A per-test UNIQUE high slot, derived from the PID so two parallel test
/// binaries do not collide on the slot-derived netns/veth names. The slot
/// space is `0..=4095`; we fold the PID into the TOP of that space
/// (`4095 - (pid % 256)`, i.e. `0xf00..=0xfff`) to stay clear of the low
/// slots a real allocator (step 02-04) would hand out first.
fn unique_slot() -> NetSlot {
    let pid = std::process::id();
    let value = 4095 - u16::try_from(pid % 256).unwrap_or(0);
    NetSlot::new(value).expect("4095-(pid%256) is within 0..=NET_SLOT_MAX")
}

fn plan() -> WorkloadNetnsPlan {
    // The responder addr is a plan INPUT (D-TME-9 resolv.conf injection,
    // step 02-03 — NOT exercised here); any value is fine for 02-02.
    derive_workload_netns_plan(unique_slot(), Ipv4Addr::new(10, 99, 255, 1))
}

/// RAII teardown — runs the production `teardown_workload_netns` on drop so
/// the netns + veth leave no residue even when an assertion panics
/// mid-test. Idempotent (teardown swallows "absent").
struct NetnsGuard {
    plan: WorkloadNetnsPlan,
}

impl Drop for NetnsGuard {
    fn drop(&mut self) {
        let _ = teardown_workload_netns(&self.plan);
    }
}

/// Returns `true` if a [`VethProvisionError`] is the EPERM "no privilege"
/// shape (so the test can SKIP on an unprivileged runner rather than fail
/// on a genuine provisioning bug).
fn is_cap_skip(err: &VethProvisionError) -> bool {
    let msg = err.to_string();
    msg.contains("Operation not permitted") || msg.contains("Permission denied")
}

// ---- observable-kernel-state probes (direct `ip` shell-outs) ----

/// `ip netns list` contains `<netns>`.
fn netns_present(netns: &str) -> bool {
    let out = Command::new("ip").args(["netns", "list"]).output().expect("spawn ip netns list");
    String::from_utf8_lossy(&out.stdout).lines().any(|l| {
        // `ip netns list` prints e.g. `ovd-ns-0fff (id: 0)` — match the
        // first whitespace-delimited token.
        l.split_whitespace().next() == Some(netns)
    })
}

/// `ip link show <iface>` (host netns) succeeds.
fn host_link_present(iface: &str) -> bool {
    Command::new("ip").args(["link", "show", iface]).output().is_ok_and(|o| o.status.success())
}

/// `ip -n <netns> link show <iface>` succeeds (iface exists in `netns`).
fn netns_link_present(netns: &str, iface: &str) -> bool {
    Command::new("ip")
        .args(["-n", netns, "link", "show", iface])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Whether `ip [-n <netns>] link show <iface>` reports the iface UP. When
/// `netns` is `None` the iface is read in the host netns.
fn link_up(netns: Option<&str>, iface: &str) -> bool {
    let mut args: Vec<&str> = Vec::new();
    if let Some(ns) = netns {
        args.extend(["-n", ns]);
    }
    args.extend(["link", "show", iface]);
    let out = Command::new("ip").args(&args).output().expect("spawn ip link show");
    if !out.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // `ip link show` prints admin flags between angle brackets
    // (e.g. `<BROADCAST,MULTICAST,UP,LOWER_UP>`) and `state UP`/`state
    // UNKNOWN` (loopback reports UNKNOWN even when up — match the flag).
    stdout.contains(",UP,")
        || stdout.contains("<UP,")
        || stdout.contains(",UP>")
        || stdout.contains("state UP")
}

/// Whether `ip -n <netns> addr show dev <iface>` reports `<addr>` bound.
fn netns_iface_has_addr(netns: &str, iface: &str, addr: Ipv4Addr) -> bool {
    let out = Command::new("ip")
        .args(["-n", netns, "addr", "show", "dev", iface])
        .output()
        .expect("spawn ip addr show");
    if !out.status.success() {
        return false;
    }
    let needle = format!("inet {addr}/");
    String::from_utf8_lossy(&out.stdout).contains(&needle)
}

/// Whether `ip addr show dev <iface>` (host netns) reports `<addr>` bound.
fn host_iface_has_addr(iface: &str, addr: Ipv4Addr) -> bool {
    let out = Command::new("ip")
        .args(["addr", "show", "dev", iface])
        .output()
        .expect("spawn ip addr show");
    if !out.status.success() {
        return false;
    }
    let needle = format!("inet {addr}/");
    String::from_utf8_lossy(&out.stdout).contains(&needle)
}

/// Whether `ip -n <netns> route show` carries a default route via `<gw>`.
fn netns_default_route_via(netns: &str, gw: Ipv4Addr) -> bool {
    let out = Command::new("ip")
        .args(["-n", netns, "route", "show", "default"])
        .output()
        .expect("spawn ip route show default");
    if !out.status.success() {
        return false;
    }
    let needle = format!("default via {gw}");
    String::from_utf8_lossy(&out.stdout).contains(&needle)
}

/// Read a `sysctl` integer knob, returning its value (`None` when the knob
/// cannot be read — a missing per-iface knob reads `None`, not `0`).
fn sysctl_int(key: &str) -> Option<i64> {
    let out = Command::new("sysctl").args(["-n", key]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// `ethtool -k <iface>` (host netns) reports `tx-checksumming: on`?
/// `None` when ethtool/feature is unavailable on the runner.
fn host_tx_checksumming_on(iface: &str) -> Option<bool> {
    tx_checksumming_on(&["-k", iface])
}

/// `ip netns exec <netns> ethtool -k <iface>` reports `tx-checksumming:
/// on`? `None` when ethtool/feature is unavailable.
fn netns_tx_checksumming_on(netns: &str, iface: &str) -> Option<bool> {
    tx_checksumming_on(&["netns", "exec", netns, "ethtool", "-k", iface])
}

/// Run `ip <args>` OR `ethtool <args>` and parse the `tx-checksumming:`
/// line. The first arg distinguishes the two shapes: `["-k", iface]` runs
/// `ethtool` directly; `["netns", "exec", ...]` runs `ip` (the netns shim).
fn tx_checksumming_on(args: &[&str]) -> Option<bool> {
    let (prog, real_args): (&str, &[&str]) =
        if args.first() == Some(&"netns") { ("ip", args) } else { ("ethtool", args) };
    let out = Command::new(prog).args(real_args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).lines().find_map(|line| {
        let rest = line.trim().strip_prefix("tx-checksumming:")?;
        Some(rest.split_whitespace().next() == Some("on"))
    })
}

/// Sweep any pre-existing residue for this test's plan so a crashed prior
/// run cannot poison the fresh-provision / zero-residue assertions.
fn sweep(plan: &WorkloadNetnsPlan) {
    let _ = Command::new("ip").args(["netns", "del", &plan.netns]).output();
    let _ = Command::new("ip").args(["link", "del", &plan.host_veth]).output();
}

/// THE Tier-3 acceptance scenario (criteria 2–4): one provision/idempotency/
/// half-provisioned-heal/teardown walkthrough against a real kernel.
///
/// `provision_creates_and_idempotently_converges_per_workload_netns`:
///
/// 1. FRESH provision creates the netns + veth pair with the in-netns end
///    INSIDE the netns; host-side end UP, in-netns end UP, netns `lo` UP
///    (B2 — all THREE up-states); host + in-netns addresses present; in-netns
///    default route present; `ip_forward=1`; `rp_filter` relaxed GLOBALLY
///    (`all` + `lo`) AND on the per-host-veth knob (S3); `tx off` on both
///    ends.
/// 2. RE-running provision is an all-noop idempotent converge (state
///    unchanged, no error).
/// 3. A HALF-provisioned netns (veth absent) is COMPLETED.
/// 4. Teardown removes the netns + veth leaving ZERO residue.
#[test]
fn provision_creates_and_idempotently_converges_per_workload_netns() {
    let plan = plan();
    sweep(&plan);
    assert!(!netns_present(&plan.netns), "precondition: netns must be absent");

    let guard = NetnsGuard { plan };

    // --- 1. FRESH provision ---
    match provision_workload_netns(&guard.plan) {
        Ok(()) => {}
        Err(err) if is_cap_skip(&err) => {
            eprintln!(
                "SKIP provision_creates_and_idempotently_converges_per_workload_netns: \
                 CAP_NET_ADMIN required ({err})"
            );
            return;
        }
        Err(err) => panic!("fresh provision failed for a non-privilege reason: {err}"),
    }

    let p = &guard.plan;
    // Netns + pair shape (observable: ip netns list, ip [-n] link show).
    assert!(netns_present(&p.netns), "netns must exist after provision");
    assert!(host_link_present(&p.host_veth), "host-side veth must exist in host netns");
    assert!(
        netns_link_present(&p.netns, &p.workload_veth),
        "in-netns veth end must be INSIDE the netns after the move",
    );
    assert!(
        !host_link_present(&p.workload_veth),
        "in-netns veth end must NOT remain in the host netns (it was moved)",
    );

    // B2 — all THREE up-states (a veth forwards only when both ends are up
    // and a fresh netns has `lo` down).
    assert!(link_up(None, &p.host_veth), "host-side veth end must be UP");
    assert!(link_up(Some(&p.netns), &p.workload_veth), "in-netns veth end must be UP");
    assert!(link_up(Some(&p.netns), "lo"), "netns loopback (lo) must be UP");

    // Addresses + default route (observable: ip addr / ip route show).
    assert!(host_iface_has_addr(&p.host_veth, p.host_addr), "host-side address must be present");
    assert!(
        netns_iface_has_addr(&p.netns, &p.workload_veth, p.workload_addr),
        "in-netns address must be present",
    );
    assert!(
        netns_default_route_via(&p.netns, p.gateway),
        "in-netns default route via the host-side gateway must be present",
    );

    // Criterion 4 — spike-proven host prereqs.
    //
    // The three GLOBAL knobs (ip_forward, all/lo rp_filter) are WEAK
    // regression guards by necessity. Production's converge contract is
    // "rp_filter relaxed == NOT STRICT" (`sysctl_rp_filter_relaxed` reads any
    // value `!= 1`), so a `RelaxGlobalRpFilter` step is emitted ONLY when a
    // global knob is strict (`1`). The Lima VM ships `all`/`lo` rp_filter == 2
    // (loose) and ip_forward == 1 host-globally — already non-strict — so
    // production CORRECTLY leaves the globals untouched (ADR-0061: do not
    // re-write a knob that already satisfies desired). Asserting `== 0` here
    // would be a FALSE expectation (production writes 0 only when it has to
    // un-strict a `1`, which never happens on this VM); we therefore assert
    // the contract production actually enforces — NOT STRICT — and accept that
    // the VM default masks a regressed global step. These knobs are also
    // host-sticky and shared with the concurrent host-netns veth suite, so
    // they cannot be cleanly isolated; the PER-HOST-VETH knob below is the
    // load-bearing rp_filter regression guard.
    assert_eq!(
        sysctl_int("net.ipv4.ip_forward"),
        Some(1),
        "ip_forward must be 1 (weak guard: VM default is already 1)",
    );
    assert_ne!(
        sysctl_int("net.ipv4.conf.all.rp_filter"),
        Some(1),
        "global `all` rp_filter must be relaxed/not-strict (weak guard: VM default 2 already satisfies, host-sticky/shared)",
    );
    assert_ne!(
        sysctl_int("net.ipv4.conf.lo.rp_filter"),
        Some(1),
        "global `lo` rp_filter must be relaxed/not-strict (weak guard: VM default 2 already satisfies, host-sticky/shared)",
    );
    // LOAD-BEARING per-host-veth rp_filter guard. dot separator (NOT `/` —
    // procps swaps `.`/`/`) so this reads the knob production actually writes.
    // A freshly created veth inherits `default.rp_filter == 2`, and the
    // converge plan ALWAYS emits `RelaxHostVethRpFilter` on a (re)built pair
    // (it writes `0`); so exact `== 0` is falsifiable — if that step did not
    // run, the knob would read `2` and this assert FAILS.
    let host_veth_rp = format!("net.ipv4.conf.{}.rp_filter", p.host_veth);
    assert_eq!(
        sysctl_int(&host_veth_rp),
        Some(0),
        "per-host-veth rp_filter must be relaxed to 0 by RelaxHostVethRpFilter",
    );

    // tx offload OFF on both ends (criterion 4). `None` → ethtool/feature
    // unavailable on this runner; skip that end's assertion.
    if let Some(on) = host_tx_checksumming_on(&p.host_veth) {
        assert!(!on, "host-side veth must have tx-checksumming OFF");
    }
    if let Some(on) = netns_tx_checksumming_on(&p.netns, &p.workload_veth) {
        assert!(!on, "in-netns veth end must have tx-checksumming OFF");
    }

    // --- 2. RE-run provision: all-noop idempotent converge ---
    provision_workload_netns(p)
        .expect("second provision over a complete netns must converge silently");
    assert!(netns_present(&p.netns), "netns must still exist after re-converge");
    assert!(
        netns_iface_has_addr(&p.netns, &p.workload_veth, p.workload_addr),
        "in-netns address must be undisturbed after re-converge",
    );
    assert!(
        netns_default_route_via(&p.netns, p.gateway),
        "in-netns default route must be undisturbed after re-converge",
    );

    // --- 3. HALF-provisioned netns (veth absent) is COMPLETED ---
    // Delete only the host-side veth end (reaps both ends of the pair),
    // leaving the netns present but the pair absent — the crash-after-netns-
    // create-but-before-veth shape.
    let del = Command::new("ip").args(["link", "del", &p.host_veth]).output().expect("spawn ip");
    assert!(del.status.success(), "could not delete host veth to construct half-provisioned state");
    assert!(netns_present(&p.netns), "precondition: netns survives the veth delete");
    assert!(!host_link_present(&p.host_veth), "precondition: pair is now absent");

    provision_workload_netns(p).expect("provision must complete a half-provisioned netns");
    assert!(host_link_present(&p.host_veth), "host-side veth must be recreated");
    assert!(
        netns_link_present(&p.netns, &p.workload_veth),
        "in-netns veth end must be moved back into the netns",
    );
    assert!(link_up(Some(&p.netns), &p.workload_veth), "recreated in-netns end must be UP");
    assert!(
        netns_iface_has_addr(&p.netns, &p.workload_veth, p.workload_addr),
        "in-netns address must be restored after completing the half-provisioned netns",
    );

    // --- 4. Teardown leaves ZERO residue ---
    teardown_workload_netns(p).expect("teardown must succeed");
    assert!(!netns_present(&p.netns), "netns must be gone after teardown (zero residue)");
    assert!(!host_link_present(&p.host_veth), "host-side veth must be gone after teardown");
    assert!(
        !host_link_present(&p.workload_veth),
        "in-netns veth end must be gone after teardown (reaped with the netns)",
    );

    // Teardown is idempotent — a second teardown over the now-absent netns
    // is a silent success.
    teardown_workload_netns(p).expect("teardown must be idempotent (absent is benign)");

    // Drop runs the guard's teardown again — also a benign no-op.
    drop(guard);
}
