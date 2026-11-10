//! Tier 3 integration — `TokioTcpProber` against real loopback
//! sockets inside Lima.
//!
//! Per `.claude/rules/testing.md` § "Integration vs unit gating":
//! real-network test belongs in the `integration-tests` slow lane.
//! Per `.claude/rules/testing.md` § "Running tests — Lima VM":
//! invocation goes through `cargo xtask lima run -- cargo nextest
//! run -p overdrive-worker --features integration-tests -E
//! 'test(real_tcp_probe)'`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::Ipv4Addr;
use std::process::{Command, Stdio};
use std::time::Duration;

use overdrive_core::traits::prober::{ProbeOutcome, TcpProber};
use overdrive_testing::cidr_lease::TestCidrLease;
use overdrive_worker::probe_runner::TokioTcpProber;
use tokio::net::TcpListener;

/// S-SHCP-INT-01-01 (US-01 WS / K1) — happy path: bind a real
/// loopback listener on `127.0.0.1:0` (kernel-assigned port; no
/// race per ADR-0054 §7), probe it, assert Pass.
#[tokio::test]
async fn given_real_loopback_listener_when_tokio_tcp_prober_probes_then_returns_pass() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind loopback listener");
    let addr = listener.local_addr().expect("read local addr");

    let prober = TokioTcpProber::new();
    let outcome = prober
        .probe(&addr.ip().to_string(), addr.port(), Duration::from_secs(5))
        .await
        .expect("loopback probe call returns Ok");

    // Hold listener open until after the probe completes — the
    // handshake target must remain accepting throughout the probe.
    drop(listener);
    assert!(
        matches!(outcome, ProbeOutcome::Pass),
        "expected Pass against an accepting loopback listener; got {outcome:?}"
    );
}

/// S-SHCP-INT-01-02 (US-01 WS / K1 sad path) — connection refused
/// against an unbound port surfaces as
/// `Fail { reason: "connection refused" }`.
#[tokio::test]
async fn given_unbound_port_when_tokio_tcp_prober_probes_then_returns_fail_connection_refused() {
    // Bind a listener to discover an ephemeral port the kernel has
    // ALREADY assigned, then drop the listener immediately. The port
    // is now unbound; the kernel returns ECONNREFUSED on connect.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind loopback listener");
    let addr = listener.local_addr().expect("read local addr");
    drop(listener);

    let prober = TokioTcpProber::new();
    let outcome = prober
        .probe(&addr.ip().to_string(), addr.port(), Duration::from_secs(5))
        .await
        .expect("probe call returns Ok even when kernel refuses");

    match outcome {
        ProbeOutcome::Fail { reason } => {
            assert_eq!(
                reason, "connection refused",
                "expected named ECONNREFUSED reason; got {reason:?}"
            );
        }
        ProbeOutcome::Pass => panic!("expected Fail(connection refused); got Pass"),
    }
}

/// Uniquely-named nft table for this test's silent-SYN-drop rule.
/// Distinct from the `overdrive-mtls` table the dial-by-name Tier-3
/// tests use, so the two suites cannot collide on a shared table name
/// when run in the same VM.
const PROBE_DROP_TABLE: &str = "overdrive_probetest";

/// Stable lease name for the probe-timeout destination. The destination
/// is drawn from the shared fixture pool (`10.250.0.0/16`), which is
/// disjoint from the production workload subnet.
const PROBE_DROP_LEASE: &str = "worker-tcp-probe-timeout";

/// Run `ip` with `args`, asserting success.
fn ip(args: &[&str]) {
    let status = Command::new("ip")
        .args(args)
        .status()
        .expect("ip must be invocable (root inside the guest / CI harness)");
    assert!(status.success(), "ip {args:?} failed with status {status:?}");
}

/// RAII teardown for the route and nft silent-SYN-drop rule installed by
/// [`install_syn_drop`]. `Drop` runs on the normal return path AND on
/// unwind (a failed assertion), so a panicking probe assertion cannot
/// leak the table or route into the guest and poison a later run. Bind it
/// with `let _guard = ...;` BEFORE the probe call. The CIDR lease is
/// released after the kernel objects are removed (field drop order).
struct DropRuleGuard {
    table: &'static str,
    route: String,
    _lease: TestCidrLease,
}

impl Drop for DropRuleGuard {
    fn drop(&mut self) {
        // Best-effort teardown — both objects must be gone after the test
        // regardless of outcome. Ignore statuses: an already-absent
        // object is a harmless no-op.
        let _ = Command::new("nft").args(["delete", "table", "inet", self.table]).status();
        let _ = Command::new("ip").args(["route", "del", &self.route]).status();
    }
}

/// Make a destination that has a ROUTE but silently loses every TCP SYN,
/// and return it with its RAII teardown.
///
/// The destination is the leased fixture network's workload address,
/// reached through a `/32` route on `lo`: `connect()` finds a route (no
/// `ENETUNREACH`, which a guest with no uplink returns for any
/// unrouted address), the SYN reaches the nft OUTPUT hook, and an nft
/// `drop` discards it *before* it leaves the host. No SYN-ACK ever
/// returns, so `connect()` blocks until the app-level timeout fires — a
/// deterministic timeout that does NOT depend on the host's networking
/// backend. A kernel `blackhole`/`unreachable` route is wrong here: it
/// fails *immediately* and routes through the prober's connect-error
/// branch rather than the `tokio::time::timeout` elapsed branch this test
/// exercises.
fn install_syn_drop() -> (DropRuleGuard, Ipv4Addr) {
    let lease = TestCidrLease::acquire(PROBE_DROP_LEASE).expect("lease a route-disjoint /24");
    let dest = lease.workload_addr();

    // Clear any stale table from a SIGKILL'd prior run (the leftover-state
    // hazard this fixture defends against). Tolerate "not found".
    let _ = Command::new("nft")
        .args(["delete", "table", "inet", PROBE_DROP_TABLE])
        .stderr(Stdio::null())
        .status();

    let route = format!("{dest}/32");
    let _ = Command::new("ip").args(["route", "del", &route]).stderr(Stdio::null()).status();
    ip(&["route", "add", &route, "dev", "lo"]);
    // From here the guard owns teardown, including on a later setup panic.
    let guard = DropRuleGuard { table: PROBE_DROP_TABLE, route, _lease: lease };

    let table = format!("inet {PROBE_DROP_TABLE}");
    let chain = format!(
        "inet {PROBE_DROP_TABLE} out {{ type filter hook output priority 0; policy accept; }}"
    );
    let rule = format!("inet {PROBE_DROP_TABLE} out ip daddr {dest} tcp dport 80 drop");

    for args in [vec!["add", "table"], vec!["add", "chain"], vec!["add", "rule"]] {
        let target = match args[1] {
            "table" => &table,
            "chain" => &chain,
            _ => &rule,
        };
        let status = Command::new("nft")
            .args(&args)
            .arg(target)
            .status()
            .expect("nft must be invocable (root inside the guest / CI harness)");
        assert!(status.success(), "nft {args:?} {target:?} failed with status {status:?}");
    }

    (guard, dest)
}

/// S-SHCP-INT-01-03 (US-01 WS / K1 sad path) — a non-responding target
/// with a short timeout surfaces as
/// `Fail { reason: "timeout after <duration>" }`.
///
/// Exercises the prober's `tokio::time::timeout` elapsed branch
/// (`tcp_prober.rs` lines 69-73). The deterministic, backend-independent
/// non-response is produced by an nft OUTPUT `drop` rule that silently
/// discards the TCP SYN before it leaves the host: no SYN-ACK ever
/// returns, so `connect()` blocks until the 250ms app-level timeout
/// elapses. The destination is deliberately NOT an unrouted address: a
/// guest with no uplink fails such a connect with `ENETUNREACH` at route
/// lookup, before the SYN exists, and Lima's user-v2/gvisor stack
/// SYN-ACKs every destination. A leased, route-disjoint address with a
/// `/32` route on `lo` plus the silent SYN drop makes the timeout
/// deterministic across networking backends.
#[tokio::test]
async fn given_blackhole_address_when_tokio_tcp_prober_probes_then_returns_fail_timeout() {
    // Install the silent-SYN-drop rule and bind its RAII teardown BEFORE
    // the probe, so a panicking assertion below still tears the table
    // down on unwind.
    let (_guard, dest) = install_syn_drop();

    let prober = TokioTcpProber::new();
    let outcome = prober
        .probe(&dest.to_string(), 80, Duration::from_millis(250))
        .await
        .expect("probe call returns Ok even on timeout");

    match outcome {
        ProbeOutcome::Fail { reason } => {
            assert!(
                reason.starts_with("timeout after "),
                "expected timeout-shaped reason; got {reason:?}"
            );
        }
        ProbeOutcome::Pass => panic!("expected Fail(timeout); got Pass"),
    }
}
