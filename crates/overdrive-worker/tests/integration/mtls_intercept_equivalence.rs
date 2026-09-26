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
//! ## The asserted set IS the trait contract, modulo two unobservable clauses
//!
//! | Contract clause (`mtls_intercept_port.rs`) | Asserted by |
//! |---|---|
//! | `bind_transparent` returns a bound listener whose `local_addr()` port is NON-ZERO when `addr` carried port 0 | S-MIF-09 |
//! | Each call returns a DISTINCT listener | S-MIF-10 |
//! | An address still held by a live listener of the adapter is refused with `EADDRINUSE`; it binds again once its last holder drops | S-ND295-70 (held address) |
//! | After `converge_shared` records a program, `install_*` at the recorded ports returns a guard owning exactly what the call acquired; `Drop` never panics | S-MIF-11 |
//! | `Drop` never panics even for a guard whose state was already released out-of-band | S-MIF-12 |
//! | An install before any successful `converge_shared` is refused with `SharedProgramNotConverged` and changes nothing (DISTILL gap B-8, refusal 1) | S-ND295-71 (pre-converge refusal) |
//! | An install at a port other than the recorded target of its leg is refused with `SharedListenerPortMismatch { leg, expected, actual }` (B-8, refusal 2) | S-ND295-71 (port mismatch) |
//! | A node guard dropped with no member removes the program it established: `observe_shared` reads `Ok(None)` (B-8, D15's conditional delete) | S-ND295-71 (no-member drop) |
//! | A `converge_shared` whose `prior` differs from the observed program returns `PostconditionMismatch` and changes nothing | S-ND295-71 (stale prior) |
//! | A zero listener port returns `NftRuleInstallFailed { op: "shared-ip-expected" }` and changes nothing | S-ND295-71 (zero port) |
//! | After a no-member node-guard drop, `observe_shared_state` and `converge_allocation_elements` read `Ok(None)` | S-ND295-71 (no-member drop, member-aware; from 08-02) |
//! | A node guard dropped while members exist (their element guards relinquished first) keeps the program | S-ND295-71 (drop with members; from 08-02) |
//! | A re-install of an identical capture is idempotent-by-convergence — it does not create a duplicate | **NOT asserted — substrate, owned by `HostMtlsIntercept`'s Tier-3 suite.** |
//! | *"the capture is in effect against this adapter's OWN substrate"* | **NOT asserted — deliberately.** |
//!
//! The last two clauses are **recorded here rather than silently dropped**,
//! both for the same reason: neither is observable through any trait accessor,
//! so no adapter can diverge on either *observably*.
//!
//! On the idempotence row specifically — S-MIF-12 DOES drive the re-install
//! (that is how it reaches the double-`Drop` case), but it asserts only that
//! both installs return `Ok` and that both releases are clean. **Non-
//! duplication itself is not asserted**: counting members means reading `nft`,
//! which is `HostMtlsIntercept`'s substrate and vacuous for the sim. A
//! regression adding a duplicate element would pass this suite.
//!
//! The in-effect-against-own-substrate clause is honoured and asserted
//! PER-ADAPTER — for `HostMtlsIntercept` by the existing Tier-3 suite
//! (`shared_intercept_members.rs`, `mtls_intercept_install.rs`), which observes
//! real `nft` state; for the sim, vacuously.
//!
//! ## What this suite deliberately does NOT assert
//!
//! The substrate specifics — `IP_TRANSPARENT` + `IP_FREEBIND` on the socket,
//! exactly which set elements an install adds, the element read-back, the
//! shared-routing-infra convergence — are `HostMtlsIntercept`'s **own**
//! documented obligations, NOT the trait's. Asserting them at the trait level
//! would re-introduce the § 4.1 contract defect DFS-7 fixed: a trait
//! postcondition half its sanctioned implementors cannot honour. They stay
//! asserted by the Tier-3 suite named above.
//!
//! ## Which failure arms are equivalence-tested
//!
//! Every clause the adapters reach deterministically from the same call
//! sequence is asserted on both: the `Ok` arms, and the B-8 refusals (no
//! recorded program, a mismatched port, a stale prior, a zero port), none of
//! which needs a fault injected. The **injected** fault arms are NOT
//! equivalence-testable for the classes `SimMtlsIntercept` scripts (`EPERM` on
//! `setsockopt`, an element batch the kernel rejects): the host adapter cannot
//! be made to exhibit them on demand — **and that inability is the entire
//! reason this port exists**. Those arms are pinned by the trait's rustdoc
//! contract plus the `SimMtlsIntercept` contract suite (S-MIF-06/07/08/13). The
//! host's allocation methods are not one-line delegations — they own the
//! recorded program, the port check, and the element tokens — so this suite is
//! what keeps the two adapters' observable partitions equal. **Nothing here
//! claims full host/sim equivalence.**
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
//! Every acquired guard is dropped INSIDE the test, allocation guards before
//! the node guard (the host node guard's `Drop` is a conditional delete that
//! refuses non-empty sets). A body that relinquishes an element guard on
//! purpose leaves its member for the sweep. Every body that converges a
//! program holds the cross-process kernel-state `flock` the sibling
//! kernel-touching suites hold, and runs inside a [`SharedInfraSweep`], which
//! razes the node-global intercept state before and after.

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::similar_names,
    reason = "Test body; skip messages + per-adapter execution evidence go to stderr; fixture preconditions and contract violations must panic with informative messages; leg F and leg C are the port's own names for its two listeners"
)]

use std::net::{Ipv4Addr, SocketAddrV4};

use overdrive_sim::adapters::SimMtlsIntercept;
use overdrive_worker::mtls_intercept::{InterceptError, InterceptLeg};
use overdrive_worker::mtls_intercept_port::{HostMtlsIntercept, InterceptMembers, MtlsIntercept};

use super::inbound_tproxy_harness::{KernelStateLock, clean_shared_infra, is_root, record_uname};
use super::leg_listener::LegListener;

/// The PRODUCTION bind shape for BOTH intercept legs: loopback, port left to
/// the kernel. `start_alloc` binds exactly this twice (leg-F then leg-C), and
/// port `0` is the only behaviourally-distinguished value in the `u16` domain.
const LEG_ADDR: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0);

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

/// RAII sweep for every body that converges a program.
///
/// Razes the node-global `overdrive-mtls` intercept state (it PERSISTS by
/// design — converge-on-boot — so a reproducible run must raze it) when
/// created and again when dropped, so a panic leaves nothing behind. It
/// creates nothing: every program, member, rule, and route a body observes is
/// the adapter-under-test's.
struct SharedInfraSweep;

impl SharedInfraSweep {
    fn create() -> Self {
        clean_shared_infra();
        Self
    }
}

impl Drop for SharedInfraSweep {
    fn drop(&mut self) {
        clean_shared_infra();
    }
}

/// Two bound legs, their addresses, and the program converged at them — the
/// production call sequence (DISTILL gap B-8): observe the prior, bind leg F
/// and leg C at port 0, then `converge_shared(prior, F, C)`. The caller drops
/// every allocation guard before the node guard, and the node guard before the
/// listeners.
struct ConvergedLegs<L> {
    node_guard: Box<dyn overdrive_worker::mtls_intercept_port::InterceptGuard>,
    leg_f: L,
    leg_c: L,
    leg_f_addr: SocketAddrV4,
    leg_c_addr: SocketAddrV4,
}

fn bind_and_converge(
    sut: &dyn MtlsIntercept,
    scenario: &str,
    adapter: &str,
) -> ConvergedLegs<impl LegListener> {
    let prior = sut.observe_shared().unwrap_or_else(|error| {
        panic!("[{scenario}][{adapter}] observing the prior program must succeed, got {error:?}")
    });
    let leg_f = sut.bind_transparent(LEG_ADDR).expect("leg F must bind");
    let leg_c = sut.bind_transparent(LEG_ADDR).expect("leg C must bind");
    let leg_f_addr = leg_f.bound_v4().expect("leg F reports its bound address");
    let leg_c_addr = leg_c.bound_v4().expect("leg C reports its bound address");
    let node_guard =
        sut.converge_shared(prior.as_ref(), leg_f_addr, leg_c_addr).unwrap_or_else(|error| {
            panic!(
                "[{scenario}][{adapter}] converge_shared(prior, {leg_f_addr}, {leg_c_addr}) must \
                 succeed on a swept node, got {error:?}"
            )
        });
    ConvergedLegs { node_guard, leg_f, leg_c, leg_f_addr, leg_c_addr }
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

/// S-MIF-11 — after the node program is converged, both installs hand back a
/// guard that releases without incident, whichever intercept surface is in
/// use.
///
/// The call sequence is production's (DISTILL gap B-8): observe the prior, bind
/// leg F and leg C, `converge_shared(prior, F, C)`, then install with each
/// leg's bound port. The allocation guards drop first, then the node guard,
/// then the listeners.
///
/// Universe: the `converge_shared` and two install `Result`s (all `Ok`) plus
/// the ABSENCE of a panic across every `Drop`. **This test completing IS the
/// observable** — `InterceptGuard`'s contract is ENTIRELY its `Drop`, so there
/// is no accessor to read.
///
/// Deliberately NOT asserted: which set elements the host adds and removes —
/// substrate, owned by `HostMtlsIntercept`'s Tier-3 obligations (see the module
/// doc).
///
/// Mutation target: a guard `Drop` that panics or that propagates an element
/// removal error; an install that refuses at the recorded port.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-70 — The node's protection listeners belong to the protection port: a
/// simulated node opens no socket, and a listener stops when its wait is cancelled.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn both_installs_hand_back_a_guard_that_releases_cleanly() {
    if !is_root() {
        eprintln!("SKIP both_installs_hand_back_a_guard_that_releases_cleanly: not root");
        return;
    }
    record_uname("05-01-S-MIF-11");
    let _kernel_lock = KernelStateLock::acquire();
    let _sweep = SharedInfraSweep::create();

    for adapter in ADAPTERS {
        let label = adapter.label();
        let sut = adapter.build();
        let legs = bind_and_converge(sut.as_ref(), "S-MIF-11", label);
        let outbound_port = legs.leg_f_addr.port();
        let inbound_port = legs.leg_c_addr.port();

        let outbound_guard = sut
            .install_outbound(SOURCE_ADDR, outbound_port)
            .expect("install_outbound at the recorded leg-F port must hand back a guard");
        let inbound_guard = sut
            .install_inbound(VIRT, inbound_port)
            .expect("install_inbound at the recorded leg-C port must hand back a guard");

        // Releasing each guard neither fails nor panics.
        drop(outbound_guard);
        drop(inbound_guard);
        drop(legs.node_guard);
        drop((legs.leg_f, legs.leg_c));

        eprintln!(
            "[S-MIF-11][{label}] EXECUTED — converge_shared at ({outbound_port}, {inbound_port}); \
             install_outbound({SOURCE_ADDR}, {outbound_port}) and install_inbound({VIRT}, \
             {inbound_port}) both Ok; every guard released cleanly"
        );
    }
}

/// S-MIF-12 — installing the SAME capture twice converges instead of
/// duplicating, and both guards release cleanly, whichever intercept surface is
/// in use.
///
/// The node program is converged once (DISTILL gap B-8); the capture is
/// installed twice at the recorded leg-F port.
///
/// Universe: the two install `Result`s (both `Ok`) plus the absence of a panic
/// across both `Drop`s — **including the second `Drop`, whose underlying state
/// the first `Drop` may already have released**.
///
/// This is a pure contract-clause assertion adding no API. It pins the two
/// clauses `mtls_intercept_port.rs` states explicitly and that no other
/// scenario reaches:
///
/// 1. `install_outbound`'s edge case — *"A re-install for an already-owned
///    source adopts the process-local element token; it does not create a
///    duplicate set element."*
/// 2. `InterceptGuard`'s invariant — *"Dropping never panics and never errors,
///    including for a guard whose underlying state was already released
///    out-of-band."*
///
/// C4a (apply twice) + C4b (inverse op without its prerequisite).
///
/// Deliberately NOT asserted: "exactly one element exists" — substrate, per
/// the module doc.
///
/// Mutation target: a non-idempotent install that adds a duplicate; a
/// double-release panic.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-70 — The node's protection listeners belong to the protection port: a
/// simulated node opens no socket, and a listener stops when its wait is cancelled.
/// CONTRACT_SHAPE: bounded-change.
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
    let _sweep = SharedInfraSweep::create();

    for adapter in ADAPTERS {
        let label = adapter.label();
        let sut = adapter.build();
        let legs = bind_and_converge(sut.as_ref(), "S-MIF-12", label);
        let leg_f_port = legs.leg_f_addr.port();

        let first = sut
            .install_outbound(SOURCE_ADDR, leg_f_port)
            .expect("the first install of the outbound capture must hand back a guard");
        let second = sut.install_outbound(SOURCE_ADDR, leg_f_port).expect(
            "a re-install of the SAME capture is idempotent-by-convergence and must still hand \
             back a guard",
        );

        // Release both IN TURN, then the node guard, then the listeners.
        drop(first);
        drop(second);
        drop(legs.node_guard);
        drop((legs.leg_f, legs.leg_c));

        eprintln!(
            "[S-MIF-12][{label}] EXECUTED — two installs of ({SOURCE_ADDR}, {leg_f_port}) after \
             one converge both Ok; both guards released in turn"
        );
    }
}

/// B-8 refusal 1 on both adapters: on a fresh adapter with bound legs, before
/// any `converge_shared`, `install_outbound` and `install_inbound` are each
/// refused with `SharedProgramNotConverged`, and the owned program stays
/// absent (`observe_shared` reads `Ok(None)` before and after).
///
/// Universe: the two install `Result`s and `observe_shared` before and after.
///
/// Mutation targets: a host `install_inbound` that falls back to a
/// per-allocation rule; an adapter that records a member with no program; a
/// refusal with a fabricated-source variant.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-71 — Protection is installed only against the node's own converged program.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 05-01 (S-ND295-71)"]
fn an_install_before_convergence_is_refused_and_changes_nothing() {
    assert!(is_root(), "S-ND295-71 host evidence requires root and CAP_NET_ADMIN");
    record_uname("S-ND295-71-pre-converge");
    let _kernel_lock = KernelStateLock::acquire();
    let _sweep = SharedInfraSweep::create();

    for adapter in ADAPTERS {
        let label = adapter.label();
        let sut = adapter.build();
        let leg_f = sut.bind_transparent(LEG_ADDR).expect("leg F must bind");
        let leg_c = sut.bind_transparent(LEG_ADDR).expect("leg C must bind");
        let leg_f_port = bound_ipv4_port(&leg_f, "S-ND295-71", label);
        let leg_c_port = bound_ipv4_port(&leg_c, "S-ND295-71", label);
        assert_eq!(
            sut.observe_shared().expect("observe before the installs"),
            None,
            "[S-ND295-71][{label}] no program exists before the installs"
        );

        let outbound = sut.install_outbound(SOURCE_ADDR, leg_f_port).map(|_guard| ());
        let inbound = sut.install_inbound(VIRT, leg_c_port).map(|_guard| ());
        eprintln!("[S-ND295-71][{label}] pre-converge outbound={outbound:?} inbound={inbound:?}");
        assert!(
            matches!(outbound, Err(InterceptError::SharedProgramNotConverged)),
            "[S-ND295-71][{label}] install_outbound before any converge is refused with \
             SharedProgramNotConverged, got {outbound:?}"
        );
        assert!(
            matches!(inbound, Err(InterceptError::SharedProgramNotConverged)),
            "[S-ND295-71][{label}] install_inbound before any converge is refused with \
             SharedProgramNotConverged, got {inbound:?}"
        );
        assert_eq!(
            sut.observe_shared().expect("observe after the refusals"),
            None,
            "[S-ND295-71][{label}] a refused install changes no owned state"
        );
        drop((leg_f, leg_c));
    }
}

/// B-8 refusal 2 on both adapters: after `converge_shared` records a program,
/// an install at a port other than the recorded target of its leg is refused
/// with `SharedListenerPortMismatch { leg, expected, actual }` — `expected`
/// the recorded port, `actual` the passed one. Each install passes the other
/// leg's port (a cross-wiring).
///
/// Universe: the two install `Result`s.
///
/// Mutation targets: an adapter that ignores the passed port; one that swaps
/// `expected` and `actual`; one that names the wrong leg.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-71 — Protection is installed only against the node's own converged program.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 05-01 (S-ND295-71)"]
fn an_install_at_a_port_other_than_the_recorded_target_is_refused() {
    assert!(is_root(), "S-ND295-71 host evidence requires root and CAP_NET_ADMIN");
    record_uname("S-ND295-71-port-mismatch");
    let _kernel_lock = KernelStateLock::acquire();
    let _sweep = SharedInfraSweep::create();

    for adapter in ADAPTERS {
        let label = adapter.label();
        let sut = adapter.build();
        let legs = bind_and_converge(sut.as_ref(), "S-ND295-71", label);
        let leg_f_port = legs.leg_f_addr.port();
        let leg_c_port = legs.leg_c_addr.port();

        for (leg, result) in [
            (InterceptLeg::F, sut.install_outbound(SOURCE_ADDR, leg_c_port).map(|_guard| ())),
            (InterceptLeg::C, sut.install_inbound(VIRT, leg_f_port).map(|_guard| ())),
        ] {
            let (recorded, passed) = match leg {
                InterceptLeg::F => (leg_f_port, leg_c_port),
                InterceptLeg::C => (leg_c_port, leg_f_port),
            };
            eprintln!("[S-ND295-71][{label}] {leg:?} install at port {passed}: {result:?}");
            match result {
                Err(InterceptError::SharedListenerPortMismatch {
                    leg: got_leg,
                    expected,
                    actual,
                }) => {
                    assert_eq!(got_leg, leg, "[S-ND295-71][{label}] the refusal names the leg");
                    assert_eq!(
                        expected, recorded,
                        "[S-ND295-71][{label}] expected is the recorded port"
                    );
                    assert_eq!(actual, passed, "[S-ND295-71][{label}] actual is the passed port");
                }
                other => panic!(
                    "[S-ND295-71][{label}] a {leg:?} install at port {passed} must be refused \
                     with SharedListenerPortMismatch, got {other:?}"
                ),
            }
        }
        drop(legs.node_guard);
        drop((legs.leg_f, legs.leg_c));
    }
}

/// B-8 program clause on both adapters: a node guard dropped with no member
/// removes the program its convergence established — D15's conditional
/// delete, which the sim models — so `observe_shared` reads `Ok(None)`.
///
/// Universe: `observe_shared` before and after the drop.
///
/// Mutation targets: an inert node guard; a guard that deletes a program it
/// did not establish.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-71 — Protection is installed only against the node's own converged program.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 05-01 (S-ND295-71)"]
fn a_node_guard_dropped_with_no_members_leaves_no_program() {
    assert!(is_root(), "S-ND295-71 host evidence requires root and CAP_NET_ADMIN");
    record_uname("S-ND295-71-no-member-drop");
    let _kernel_lock = KernelStateLock::acquire();
    let _sweep = SharedInfraSweep::create();

    for adapter in ADAPTERS {
        let label = adapter.label();
        let sut = adapter.build();
        let legs = bind_and_converge(sut.as_ref(), "S-ND295-71", label);
        assert!(
            sut.observe_shared().expect("observe the converged program").is_some(),
            "[S-ND295-71][{label}] the converged program is present before the drop"
        );

        drop(legs.node_guard);
        assert_eq!(
            sut.observe_shared().expect("observe after the drop"),
            None,
            "[S-ND295-71][{label}] a node guard dropped with no member removes its program"
        );
        drop((legs.leg_f, legs.leg_c));
    }
}

/// B-8 program clause on both adapters: a `converge_shared` whose `prior`
/// differs from the observed program — here an absent prior over a converged
/// program at the same targets — returns `PostconditionMismatch { expected,
/// observed }` with `expected` the requested identity (equal to the program at
/// the same targets) and `observed` the program, and changes nothing.
///
/// Universe: the refused `Result` and `observe_shared` after it.
///
/// Mutation targets: an adapter that ignores `prior`; one that rewrites the
/// program before comparing.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-71 — Protection is installed only against the node's own converged program.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 05-01 (S-ND295-71)"]
fn a_convergence_from_a_stale_prior_is_refused_and_changes_nothing() {
    assert!(is_root(), "S-ND295-71 host evidence requires root and CAP_NET_ADMIN");
    record_uname("S-ND295-71-stale-prior");
    let _kernel_lock = KernelStateLock::acquire();
    let _sweep = SharedInfraSweep::create();

    for adapter in ADAPTERS {
        let label = adapter.label();
        let sut = adapter.build();
        let legs = bind_and_converge(sut.as_ref(), "S-ND295-71", label);
        let program = sut
            .observe_shared()
            .expect("observe the converged program")
            .expect("the converged program is present");

        let Err(refused) = sut.converge_shared(None, legs.leg_f_addr, legs.leg_c_addr) else {
            panic!("[S-ND295-71][{label}] an absent prior over a present program is refused")
        };
        match refused {
            InterceptError::PostconditionMismatch { expected, observed } => {
                assert_eq!(
                    expected, program,
                    "[S-ND295-71][{label}] expected is the requested identity"
                );
                assert_eq!(
                    observed,
                    Some(program.clone()),
                    "[S-ND295-71][{label}] observed is the program"
                );
            }
            other => panic!(
                "[S-ND295-71][{label}] a stale prior is refused with PostconditionMismatch, got \
                 {other:?}"
            ),
        }
        assert_eq!(
            sut.observe_shared().expect("observe after the refusal"),
            Some(program),
            "[S-ND295-71][{label}] a stale-prior refusal changes nothing"
        );
        drop(legs.node_guard);
        drop((legs.leg_f, legs.leg_c));
    }
}

/// B-8 program clause on both adapters: a zero listener port on either leg
/// returns `NftRuleInstallFailed { op: "shared-ip-expected" }` and changes
/// nothing (the program stays absent).
///
/// Universe: the two refused `Result`s and `observe_shared` after them.
///
/// Mutation target: an adapter that records a zero target.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-71 — Protection is installed only against the node's own converged program.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 05-01 (S-ND295-71)"]
fn a_zero_listener_port_is_refused_before_any_program_change() {
    assert!(is_root(), "S-ND295-71 host evidence requires root and CAP_NET_ADMIN");
    record_uname("S-ND295-71-zero-port");
    let _kernel_lock = KernelStateLock::acquire();
    let _sweep = SharedInfraSweep::create();

    for adapter in ADAPTERS {
        let label = adapter.label();
        let sut = adapter.build();
        let leg = sut.bind_transparent(LEG_ADDR).expect("one leg must bind");
        let bound = leg.bound_v4().expect("the leg reports its bound address");

        for (row, leg_f, leg_c) in [("leg F", LEG_ADDR, bound), ("leg C", bound, LEG_ADDR)] {
            let Err(refused) = sut.converge_shared(None, leg_f, leg_c) else {
                panic!("[S-ND295-71][{label}] a zero {row} port is refused")
            };
            assert!(
                matches!(
                    refused,
                    InterceptError::NftRuleInstallFailed { op: "shared-ip-expected", .. }
                ),
                "[S-ND295-71][{label}] a zero {row} port is refused as shared-ip-expected, got \
                 {refused:?}"
            );
        }
        assert_eq!(
            sut.observe_shared().expect("observe after the refusals"),
            None,
            "[S-ND295-71][{label}] a zero-port refusal changes nothing"
        );
        drop(leg);
    }
}

/// B-8 program clause on both adapters, member-aware: after a no-member
/// node-guard drop, `observe_shared_state` and `converge_allocation_elements`
/// both read `Ok(None)` (the host's two member methods stop being RED
/// scaffolds at 08-02).
///
/// Universe: `observe_shared_state` and `converge_allocation_elements(∅)` after
/// the drop.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-71 — Protection is installed only against the node's own converged program.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 08-02 (S-ND295-71)"]
fn a_node_guard_dropped_with_no_members_leaves_no_member_state() {
    assert!(is_root(), "S-ND295-71 host evidence requires root and CAP_NET_ADMIN");
    record_uname("S-ND295-71-no-member-drop-state");
    let _kernel_lock = KernelStateLock::acquire();
    let _sweep = SharedInfraSweep::create();

    for adapter in ADAPTERS {
        let label = adapter.label();
        let sut = adapter.build();
        let legs = bind_and_converge(sut.as_ref(), "S-ND295-71", label);

        drop(legs.node_guard);
        assert_eq!(
            sut.observe_shared_state().expect("observe the state after the drop"),
            None,
            "[S-ND295-71][{label}] the member-aware observation reads the program as absent"
        );
        assert_eq!(
            sut.converge_allocation_elements(&InterceptMembers::default())
                .expect("member convergence without a program succeeds"),
            None,
            "[S-ND295-71][{label}] member convergence reads the program as absent"
        );
        drop((legs.leg_f, legs.leg_c));
    }
}

/// B-8 program clause on both adapters: a node guard dropped while a member
/// exists keeps the program. The member's element guard is relinquished, not
/// dropped, first — the guard-ordering rule's one sanctioned way to hold a
/// member past a node-guard drop — and the member-aware observation reads the
/// program with that member (08-02; through `observe_shared` the same holds
/// once the strict observation stops refusing members, 08-03).
///
/// Universe: `observe_shared_state` after the drop.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-71 — Protection is installed only against the node's own converged program.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 08-02 (S-ND295-71)"]
fn a_node_guard_dropped_while_members_exist_keeps_the_program() {
    assert!(is_root(), "S-ND295-71 host evidence requires root and CAP_NET_ADMIN");
    record_uname("S-ND295-71-drop-with-members");
    let _kernel_lock = KernelStateLock::acquire();
    let _sweep = SharedInfraSweep::create();

    for adapter in ADAPTERS {
        let label = adapter.label();
        let sut = adapter.build();
        let legs = bind_and_converge(sut.as_ref(), "S-ND295-71", label);
        let program = sut
            .observe_shared()
            .expect("observe the converged program")
            .expect("the converged program is present");
        let member = sut
            .install_outbound(SOURCE_ADDR, legs.leg_f_addr.port())
            .expect("install one member at the recorded port");
        // Relinquish, not drop: the member stays for the sweep.
        std::mem::forget(member);

        drop(legs.node_guard);
        let state = sut
            .observe_shared_state()
            .expect("observe the state after the drop")
            .expect("a node guard dropped while members exist keeps the program");
        assert_eq!(state.program, program, "[S-ND295-71][{label}] the program is unchanged");
        assert!(
            state.members.outbound_sources.contains(&SOURCE_ADDR),
            "[S-ND295-71][{label}] the relinquished member is still present"
        );
        drop((legs.leg_f, legs.leg_c));
    }
}
