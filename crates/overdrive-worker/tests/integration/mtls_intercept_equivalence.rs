//! T4 — the [`MtlsIntercept`] host↔sim **`Ok`-arm equivalence** structural
//! guard (GH #250, ADR-0076 § 5.4 / OQ-8; DISTILL S-MIF-09..12; step 05-01).
//!
//! Per `.claude/rules/development.md` § "The DST equivalence test is the
//! structural guard": the trait's rustdoc is the **CONTRACT**, this suite is
//! the **ENFORCEMENT**, and each adapter's implementation is the
//! **CONSEQUENCE**. Every scenario drives BOTH sanctioned adapters —
//! `HostMtlsIntercept` (real `libc::socket` + `setsockopt` + real `nft`) and
//! `SimMtlsIntercept` with no fault armed — through the SAME call sequence and
//! asserts the SAME observables at every step. When one of these fails,
//! exactly one of the contract / host adapter / sim adapter is wrong, and the
//! failing scenario isolates which.
//!
//! ## The asserted set IS the trait contract, modulo ONE unobservable clause
//!
//! | Contract clause (`mtls_intercept_port.rs`) | Asserted by |
//! |---|---|
//! | `bind_transparent` returns a bound listener whose `local_addr()` port is NON-ZERO when `addr` carried port 0 | S-MIF-09 |
//! | Each call returns a DISTINCT listener | S-MIF-10 |
//! | An address still held by a live listener of the adapter is refused with `EADDRINUSE`; it binds again once its last holder drops | S-ND295-70 (held address) |
//! | `install_*` returns a guard owning exactly what the call acquired; `Drop` never panics | S-MIF-11 |
//! | `Drop` never panics even for a guard whose state was already released out-of-band | S-MIF-12 |
//! | A re-install of an identical capture is idempotent-by-convergence — it does not create a duplicate | **NOT asserted — substrate, owned by `HostMtlsIntercept`'s Tier-3 suite.** |
//! | *"the capture is in effect against this adapter's OWN substrate"* | **NOT asserted — deliberately.** |
//!
//! Those last two clauses are **recorded here rather than silently dropped**,
//! both for the same reason: neither is observable through any trait accessor,
//! so no adapter can diverge on either *observably*.
//!
//! On the idempotence row specifically — S-MIF-12 DOES drive the re-install
//! (that is how it reaches the double-`Drop` case), but it asserts only that
//! both installs return `Ok` and that both releases are clean. **Non-
//! duplication itself is not asserted**: counting rules means reading `nft`,
//! which is `HostMtlsIntercept`'s substrate and vacuous for the sim. A
//! regression appending a duplicate rule would pass this suite.
//!
//! The in-effect-against-own-substrate clause is honoured and asserted
//! PER-ADAPTER — for `HostMtlsIntercept` by the existing Tier-3 suite
//! (`start_alloc_installs_both_tproxy.rs`, `bidirectional_walking_skeleton.rs`,
//! which observe real `nft` state and real intercepted traffic); for the sim,
//! vacuously.
//!
//! ## What this suite deliberately does NOT assert
//!
//! The substrate specifics — `IP_TRANSPARENT` + `IP_FREEBIND` on the socket,
//! "exactly ONE `nft` rule appended", removal BY HANDLE on `Drop`, the
//! shared-routing-infra convergence — are `HostMtlsIntercept`'s **own**
//! documented obligations, NOT the trait's. Asserting them at the trait level
//! would re-introduce the § 4.1 contract defect DFS-7 fixed: a trait
//! postcondition half its sanctioned implementors cannot honour. They stay
//! asserted by the **existing** Tier-3 suite, unchanged by this feature.
//!
//! ## The fault-arm limit, recorded rather than papered over
//!
//! This suite covers the **`Ok` arms only**. The FAULT arms are NOT
//! equivalence-testable for the classes `SimMtlsIntercept` scripts (`EPERM` on
//! `setsockopt`, an absent `nft` binary): the host adapter cannot be made to
//! exhibit them on demand — **and that inability is the entire reason this port
//! exists**. Those arms are pinned by the trait's rustdoc contract plus the
//! `SimMtlsIntercept` contract suite (S-MIF-06/07/08/13, step 03-01); the host
//! adapter's fault arms are exercised, unscripted, by real operational
//! failures. **Nothing here claims full host/sim equivalence** — that would be
//! aspirational. The gap is smaller than it looks: each `HostMtlsIntercept`
//! method is a ONE-LINE delegation with no logic of its own to diverge.
//!
//! ## Lane — integration, Lima + root
//!
//! `HostMtlsIntercept` needs `CAP_NET_ADMIN` for `IP_TRANSPARENT` and real
//! `nft`. That reason holds on both sides of the DELIVER step that changes
//! `bind_transparent`'s return type (DISTILL gap B-7): every bound leg is read
//! through the `LegListener` bridge (`leg_listener.rs`), so each scenario keeps
//! its assertions whichever listener type the port returns.
//!
//! A non-root run SKIPs (it does not fail) — **and a run that skips every
//! scenario proves nothing**. Run via `cargo xtask lima run -- cargo nextest run
//! -p overdrive-worker --features integration-tests`. NEVER `--no-run`.
//!
//! ## Parametrisation
//!
//! Over the **adapter axis** `{HostMtlsIntercept, SimMtlsIntercept}` — an
//! IMPLEMENTATION axis, not a generative input space. Layer-3+ scenarios are
//! example-only per Mandate 11; there is no `proptest` here by design.
//!
//! ## Leak hygiene
//!
//! Every acquired guard is dropped INSIDE the test (the host adapter's `Drop`
//! removes its `nft` rule by handle). The two install scenarios additionally
//! hold the cross-process kernel-state `flock` the sibling kernel-touching
//! suites hold, and stand up / tear down their host-side veth around an
//! `overdrive-mtls` ruleset pre-sweep — the `NetnsGuard` discipline step 04-01
//! established.

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    reason = "Test body; skip messages + per-adapter execution evidence go to stderr; fixture preconditions and contract violations must panic with informative messages"
)]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::process::{Command, Stdio};

use overdrive_sim::adapters::SimMtlsIntercept;
use overdrive_worker::mtls_intercept::InterceptError;
use overdrive_worker::mtls_intercept_port::{HostMtlsIntercept, MtlsIntercept};

use super::inbound_tproxy_harness::{KernelStateLock, clean_shared_infra, is_root, record_uname};
use super::leg_listener::LegListener;

/// The PRODUCTION bind shape for BOTH intercept legs: loopback, port left to
/// the kernel. `start_alloc` binds exactly this twice (leg-F then leg-C), and
/// port `0` is the only behaviourally-distinguished value in the `u16` domain.
const LEG_ADDR: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0);

/// The host-side veth `install_outbound`'s `iifname` names. Suite-distinct so
/// concurrent (flock-serialised) runs cannot collide with the sibling Tier-3
/// suites' veths.
const VETH_H: &str = "ovd-hv-eq0501";
/// The peer end of the pair — created only so the host end is a real veth
/// interface; it never carries traffic in this suite.
const VETH_PEER: &str = "ovd-wv-eq0501";

/// The canonical guest source address admitted by the shared outbound set.
const SOURCE_ADDR: Ipv4Addr = Ipv4Addr::new(10, 99, 5, 2);

/// The canonical per-workload address paired with a DECLARED Service listener
/// port — the `virt` shape `install_inbound` is called with. A suite-distinct
/// /32 for the same non-collision reason as the veth names.
const VIRT: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 99, 5, 1), 18501);

/// Which sanctioned [`MtlsIntercept`] implementation a case drives.
#[derive(Debug, Clone, Copy)]
enum Adapter {
    /// `HostMtlsIntercept` — the production binding over real `libc::socket` +
    /// `setsockopt(IP_TRANSPARENT)` and real `nft`.
    Host,
    /// `SimMtlsIntercept` with **no fault armed**, so every method takes its
    /// `Ok` arm.
    Sim,
}

/// Both sanctioned adapters. The axis is the IMPLEMENTATION, not an input
/// space — see the module doc.
const ADAPTERS: [Adapter; 2] = [Adapter::Host, Adapter::Sim];

impl Adapter {
    /// The evidence tag this adapter's execution lines carry.
    const fn label(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Sim => "sim",
        }
    }

    /// Build the adapter behind the trait object — the ONLY surface this suite
    /// touches, so neither concrete type's inherent API can leak into an
    /// assertion.
    fn build(self) -> Box<dyn MtlsIntercept> {
        match self {
            Self::Host => Box::new(HostMtlsIntercept::new()),
            Self::Sim => Box::new(SimMtlsIntercept::new()),
        }
    }
}

/// Run `<prog> <args>` best-effort (teardown / tolerate-pre-existing).
fn run_quiet(prog: &str, args: &[&str]) {
    let _ = Command::new(prog).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// RAII real-infra fixture for the two INSTALL scenarios.
///
/// Pre-sweeps the node-global `overdrive-mtls` nft state (it PERSISTS by design
/// — converge-on-boot — so a reproducible run must raze it), then creates a
/// REAL host-side veth pair so `install_outbound`'s `iifname` names a live
/// interface. `Drop` sweeps both again.
///
/// The fixture creates only the INTERFACE; every `nft` rule this suite observes
/// is appended by the adapter under test.
struct HostVethFixture;

impl HostVethFixture {
    fn create() -> Self {
        clean_shared_infra();
        run_quiet("ip", &["link", "del", VETH_H]);
        let out = Command::new("ip")
            .args(["link", "add", VETH_H, "type", "veth", "peer", "name", VETH_PEER])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("spawn ip link add veth");
        assert!(
            out.status.success(),
            "ip link add {VETH_H} type veth peer {VETH_PEER} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        run_quiet("ip", &["link", "set", VETH_H, "up"]);
        run_quiet("ip", &["link", "set", VETH_PEER, "up"]);
        Self
    }
}

impl Drop for HostVethFixture {
    fn drop(&mut self) {
        clean_shared_infra();
        run_quiet("ip", &["link", "del", VETH_H]);
    }
}

/// The port a bound listener reports, read through the `LegListener` bridge.
///
/// The contract says `local_addr()` reports the concrete bound address. The
/// bridge reports a non-IPv4 address as an error, so a V6 report from a
/// `SocketAddrV4` bind fails loudly here rather than being silently coerced.
fn bound_ipv4_port(listener: &impl LegListener, scenario: &str, adapter: &str) -> u16 {
    let bound = listener.bound_v4().unwrap_or_else(|error| {
        panic!(
            "[{scenario}][{adapter}] bind_transparent(127.0.0.1:0) must report its IPv4 local \
             addr, got {error}"
        )
    });
    assert_eq!(
        *bound.ip(),
        Ipv4Addr::LOCALHOST,
        "[{scenario}][{adapter}] bind_transparent must report the loopback address it was asked \
         to bind",
    );
    bound.port()
}

/// S-MIF-09 — a bound intercept leg reports the concrete port the kernel
/// assigned, whichever intercept surface is in use.
///
/// Universe (port-exposed): the `Result` from `bind_transparent(127.0.0.1:0)`;
/// `listener.local_addr()` — its address family and its port.
///
/// C1a — port `0` is the minimum/zero input AND the production shape for both
/// legs.
///
/// Mutation target: an adapter that passes the requested port through verbatim
/// (so a port-0 request reports port 0), which would silently corrupt the
/// TPROXY redirect target — exactly what the `LegFLocalAddr` / `LegCLocalAddr`
/// fail-closed stages exist for.
#[test]
fn bound_leg_reports_a_non_zero_kernel_assigned_port() {
    if !is_root() {
        eprintln!("SKIP bound_leg_reports_a_non_zero_kernel_assigned_port: not root");
        return;
    }
    record_uname("05-01-S-MIF-09");

    for adapter in ADAPTERS {
        let label = adapter.label();
        let sut = adapter.build();

        let listener = sut
            .bind_transparent(LEG_ADDR)
            .expect("bind_transparent(127.0.0.1:0) must hand back a bound listener");

        let port = bound_ipv4_port(&listener, "S-MIF-09", label);
        assert_ne!(
            port, 0,
            "[S-MIF-09][{label}] a port-0 bind must report the KERNEL-assigned ephemeral port, \
             never the requested 0 — a passthrough would corrupt the TPROXY redirect target",
        );

        eprintln!("[S-MIF-09][{label}] EXECUTED — local_addr = 127.0.0.1:{port}");
    }
}

/// S-MIF-10 — two intercept legs never share a port, whichever intercept
/// surface is in use.
///
/// Universe: the two `local_addr()` ports.
///
/// Why it matters: `start_alloc` calls `bind_transparent` TWICE — leg-F then
/// leg-C — and installs one TPROXY rule per leg pointing at each leg's reported
/// port. An adapter that cached or memoised a single listener (a
/// `OnceLock`-shaped memoisation) would collapse both legs onto one socket and
/// cross-wire the intercept.
///
/// Both listeners are held ALIVE across the comparison: that is what makes this
/// "two distinct listeners" rather than a port the kernel merely re-issued
/// after the first was closed.
#[test]
fn two_bound_legs_never_share_a_port() {
    if !is_root() {
        eprintln!("SKIP two_bound_legs_never_share_a_port: not root");
        return;
    }
    record_uname("05-01-S-MIF-10");

    for adapter in ADAPTERS {
        let label = adapter.label();
        let sut = adapter.build();

        let first_leg = sut.bind_transparent(LEG_ADDR).expect("the first leg must bind");
        let second_leg = sut.bind_transparent(LEG_ADDR).expect("the second leg must bind");

        let first_port = bound_ipv4_port(&first_leg, "S-MIF-10", label);
        let second_port = bound_ipv4_port(&second_leg, "S-MIF-10", label);

        assert_ne!(
            first_port, second_port,
            "[S-MIF-10][{label}] each bind_transparent call must return a DISTINCT listener; two \
             legs sharing one port would cross-wire the intercept",
        );

        eprintln!(
            "[S-MIF-10][{label}] EXECUTED — leg-F 127.0.0.1:{first_port}, leg-C \
             127.0.0.1:{second_port} (both held live)"
        );
        drop((first_leg, second_leg));
    }
}

/// The equivalence clause S-ND295-70 adds to `bind_transparent` (feature delta
/// § *Driven port — intercept listener*, `bind_transparent` edge cases): an
/// address still held by a live listener of the adapter is refused with
/// `InterceptError::TransparentListener { addr, source }` whose source is
/// `EADDRINUSE`, and once its last holder drops, the exact address binds again
/// and reports itself, whichever sanctioned adapter is in use. The worker's
/// exact-address rebind (`converge_shared_owner`) depends on both halves.
///
/// Universe: the three `bind_transparent` results, the refusal's `addr` and
/// errno, and the bound addresses read through `LegListener::bound_v4`.
///
/// Mutation targets: an adapter that hands out a second listener at a held
/// address (a duplicate redirect target), one that refuses with a different
/// variant or errno, and one that keeps the address after its last holder
/// drops (a rebind the worker could never complete).
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-70 — The node's protection listeners belong to the protection port: a
/// simulated node opens no socket, and a listener stops when its wait is cancelled.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_held_address_is_refused_with_eaddrinuse_until_its_last_holder_drops() {
    if !is_root() {
        eprintln!(
            "SKIP a_held_address_is_refused_with_eaddrinuse_until_its_last_holder_drops: not root"
        );
        return;
    }
    record_uname("S-ND295-70-held-address");

    for adapter in ADAPTERS {
        let label = adapter.label();
        let sut = adapter.build();

        let holder = sut.bind_transparent(LEG_ADDR).expect("the first leg must bind");
        let held = holder.bound_v4().expect("the holder reports its bound address");
        assert_ne!(
            held.port(),
            0,
            "[S-ND295-70][{label}] the holder's bound port must be concrete before it is re-bound",
        );

        match sut.bind_transparent(held) {
            Err(InterceptError::TransparentListener { addr, source }) => {
                assert_eq!(
                    addr, held,
                    "[S-ND295-70][{label}] the refusal must name the held address it refused",
                );
                assert_eq!(
                    source.raw_os_error(),
                    Some(libc::EADDRINUSE),
                    "[S-ND295-70][{label}] a held address is refused with EADDRINUSE, got {source}",
                );
            }
            Err(other) => panic!(
                "[S-ND295-70][{label}] a held address must be refused with \
                 TransparentListener(EADDRINUSE), got {other:?}"
            ),
            Ok(duplicate) => panic!(
                "[S-ND295-70][{label}] a held address must never be handed out twice, got a \
                 second listener reporting {:?}",
                duplicate.bound_v4()
            ),
        }

        drop(holder);
        let rebound = sut
            .bind_transparent(held)
            .expect("the exact address binds again once its last holder has dropped");
        assert_eq!(
            rebound.bound_v4().expect("the rebound listener reports its bound address"),
            held,
            "[S-ND295-70][{label}] an exact non-zero bind is honoured exactly",
        );

        eprintln!(
            "[S-ND295-70][{label}] EXECUTED — {held} refused with EADDRINUSE while held; bound \
             again after its holder dropped"
        );
        drop(rebound);
    }
}

/// S-MIF-11 — both installs hand back a guard that releases without incident,
/// whichever intercept surface is in use.
///
/// Universe: the two `Result<Box<dyn InterceptGuard>>` values (both `Ok`) plus
/// the ABSENCE of a panic across both `Drop`s. **This test completing IS the
/// observable** — `InterceptGuard`'s contract is ENTIRELY its `Drop`, so there
/// is no accessor to read.
///
/// Deliberately NOT asserted: "exactly one nft rule exists", the
/// `IP_TRANSPARENT` / `IP_FREEBIND` setsockopts, and removal-by-handle. Those
/// are substrate — unobservable through the trait and owned by
/// `HostMtlsIntercept`'s existing Tier-3 obligations (see the module doc).
///
/// Mutation target: a guard `Drop` that panics or that propagates an `nft`
/// removal error.
#[test]
fn both_installs_hand_back_a_guard_that_releases_cleanly() {
    if !is_root() {
        eprintln!("SKIP both_installs_hand_back_a_guard_that_releases_cleanly: not root");
        return;
    }
    record_uname("05-01-S-MIF-11");
    let _kernel_lock = KernelStateLock::acquire();
    let _fixture = HostVethFixture::create();

    for adapter in ADAPTERS {
        let label = adapter.label();
        let sut = adapter.build();

        let outbound_leg = sut.bind_transparent(LEG_ADDR).expect("the outbound leg must bind");
        let inbound_leg = sut.bind_transparent(LEG_ADDR).expect("the inbound leg must bind");
        let outbound_port = bound_ipv4_port(&outbound_leg, "S-MIF-11", label);
        let inbound_port = bound_ipv4_port(&inbound_leg, "S-MIF-11", label);

        let outbound_guard = sut
            .install_outbound(SOURCE_ADDR, outbound_port)
            .expect("install_outbound against a live veth and a live leg-F must hand back a guard");
        let inbound_guard = sut
            .install_inbound(VIRT, inbound_port)
            .expect("install_inbound for a declared Service port must hand back a guard");

        // Releasing each guard neither fails nor panics.
        drop(outbound_guard);
        drop(inbound_guard);

        eprintln!(
            "[S-MIF-11][{label}] EXECUTED — install_outbound({SOURCE_ADDR}, {outbound_port}) and \
             install_inbound({VIRT}, {inbound_port}) both Ok; both guards released cleanly"
        );
    }
}

/// S-MIF-12 — installing the SAME capture twice converges instead of
/// duplicating, and both guards release cleanly, whichever intercept surface is
/// in use.
///
/// Universe: the two `Result<Box<dyn InterceptGuard>>` values (both `Ok`) plus
/// the absence of a panic across both `Drop`s — **including the second `Drop`,
/// whose underlying state the first `Drop` already released**.
///
/// This is a pure contract-clause assertion adding no API. It pins the two
/// clauses `mtls_intercept_port.rs` states explicitly and that no other
/// scenario reaches:
///
/// 1. `install_outbound`'s edge case — *"A re-install for a veth already
///    carrying an identical capture is idempotent-by-convergence; it does not
///    create a duplicate."*
/// 2. `InterceptGuard`'s invariant — *"Dropping never panics and never errors,
///    including for a guard whose underlying state was already released
///    out-of-band."*
///
/// C4a (apply twice) + C4b (inverse op without its prerequisite): the second
/// `Drop` releasing already-released state IS the
/// inverse-without-prerequisite case.
///
/// Deliberately NOT asserted: "exactly one nft rule exists" — substrate, per
/// the module doc.
///
/// Mutation target: a non-idempotent install that appends a duplicate; a
/// double-release panic.
#[test]
fn re_installing_the_same_capture_converges_and_both_guards_release_cleanly() {
    if !is_root() {
        eprintln!(
            "SKIP re_installing_the_same_capture_converges_and_both_guards_release_cleanly: not \
             root"
        );
        return;
    }
    record_uname("05-01-S-MIF-12");
    let _kernel_lock = KernelStateLock::acquire();
    let _fixture = HostVethFixture::create();

    for adapter in ADAPTERS {
        let label = adapter.label();
        let sut = adapter.build();

        let leg_f = sut.bind_transparent(LEG_ADDR).expect("the outbound leg must bind");
        let leg_f_port = bound_ipv4_port(&leg_f, "S-MIF-12", label);

        let first = sut
            .install_outbound(SOURCE_ADDR, leg_f_port)
            .expect("the first install of the outbound capture must hand back a guard");
        let second = sut.install_outbound(SOURCE_ADDR, leg_f_port).expect(
            "a re-install of the SAME capture is idempotent-by-convergence and must still hand \
             back a guard",
        );

        // Release both IN TURN. The second release acts on state the first
        // already released — and must still neither fail nor panic.
        drop(first);
        drop(second);

        eprintln!(
            "[S-MIF-12][{label}] EXECUTED — two installs of ({SOURCE_ADDR}, {leg_f_port}) both Ok; \
             both guards released in turn, the second over already-released state"
        );
    }
}
