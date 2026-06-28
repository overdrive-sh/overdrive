//! S-DBN-PINGPONG — the dial-by-name-responder BIDIRECTIONAL PING-PONG demo
//! (ADR-0072 REV-2; GH #243; roadmap 03-02 / US-DBN-3 · K-DBN-3).
//!
//! This Tier-3 `#[tokio::test]` is the OPERATOR-RUNNABLE proof: two services
//! dial EACH OTHER by name. `A` resolves `b.svc.overdrive.local` and calls B,
//! `B` resolves `a.svc.overdrive.local` and calls A; each inbound call
//! increments a counter + refreshes a date on a ~10s cadence; each hop is
//! resolved through the in-agent responder, then intercepted + mTLS'd. It drives
//! ONLY the production entry points — `run_server_with_obs_and_driver` (boot) +
//! `POST /v1/jobs` (deploy, the in-process `overdrive deploy` driving port) +
//! the example specs `examples/dial-by-name-responder/{a,b}.toml` + the
//! checked-in `examples/dial-by-name-responder/ping_pong.py` client program +
//! `getaddrinfo`/`getent` (resolve, NOT `dig` — K2) from inside each deployed
//! workload's PRODUCTION-provisioned netns.
//!
//! ## This is the SECOND mesh→mesh egress hop, TWICE (criterion 7)
//!
//! The bidirectional ping-pong is TWO of the 02-02 walking-skeleton egress hops
//! (A dials b.svc, B dials a.svc) and relies DIRECTLY on the REV-5 output-hook
//! leg-B interception (`mtls_intercept.rs`) that 02-02 landed. Each resolution +
//! dial leg uses the CORRECTED PLAINTEXT-egress test model (CLAUDE.md §
//! "East-west mTLS tests" + the 02-02 RCA
//! `root-cause-analysis-dial-by-name-agent-originated-mtls-stall.md`):
//!
//! - the workload (the ping-pong bin) dials its peer PLAINTEXT over an ordinary
//!   `TcpStream` — it is identity-unaware, holds NO SVID, presents NO TLS/SNI.
//!   A TLS-presenting dialer opens a second peerless TLS session leg-F never
//!   terminates → ClientHello tunnels as plaintext, no ServerHello, the
//!   handshake STALLS → RST (the silent hang the 02-02 RCA diagnosed). The
//!   ping-pong bin therefore speaks plaintext.
//! - the per-hop mTLS proof (TLS 1.3 `application_data` records, content-type
//!   0x17, zero cleartext) is observed on the INTER-AGENT **leg-B ↔ leg-C** wire
//!   (via `tcpdump` / `ss -tie` / an AF_PACKET 0x17 wire-scan), NEVER from the
//!   dialer's handshake. Do NOT copy the INBOUND keystone's
//!   (`canonical_address_inbound_walking_skeleton.rs`) "client presents TLS"
//!   dial shape onto these egress hops — that is the exact 02-02 model error.
//!
//! ## Why this is a RED scaffold (criterion 8 + distill/red-classification.md)
//!
//! Per `docs/feature/dial-by-name-responder/distill/red-classification.md`
//! (S-DBN-PINGPONG row), this scenario lands as a `#[should_panic(expected =
//! "RED scaffold")]` Tier-3 scaffold: the production responder serve loop +
//! re-keyed `MtlsResolve` drive the bidirectional loop, but the OPERATOR-RUNNABLE
//! bidirectional proof — two `overdrive deploy`s against a `serve` that
//! converges BOTH halves to Running-AND-HEALTHY, resolves each peer, and shows
//! both counters advancing on a ~10s cadence over a 60s window — is the
//! end-to-end behaviour the full-system EDD harness (#227, the disposable
//! full-system Lima VM, on #75, the Image Factory OS image) exists to exercise
//! against the BUILT binary. The 02-02 single-direction walking skeleton's
//! S-DBN-WS / S-DBN-SINGLE-SRC are GREEN (the core dial-by-name loop is proven
//! end-to-end in-process), but its S-DBN-WS-STABLE / S-DBN-CHURN halves — which
//! need a backend-replacement / restart-after-stop verb — are `#[ignore]`'d to
//! #249 (operator-stop is sticky/overriding by design). A steady-state
//! bidirectional ping-pong does NOT cycle a backend, but its operator-observable
//! "both counters advance over a 60s window" proof is the EDD harness's job, not
//! an in-process `#[test]`'s — graduating it as the E05 EDD expectation (honest
//! `pending`, mirroring E04) is the design (criterion 3). So the body here panics
//! with the RED-scaffold marker; the example specs and the checked-in
//! `ping_pong.py` client program are real on-disk artifacts (K3-honest), so the
//! EDD runner can drive them directly when #227/#75 land.
//!
//! Do NOT contort this green by hand-installing a production effect, substituting
//! a transparent leg-F / TLS-presenting dialer, or injecting a test-only
//! production call site — that is the 02-02/03-01 anti-pattern. The RED-scaffold
//! marker IS the honest spec of the operator-runnable proof the EDD harness
//! completes.
//!
//! Requires root + `CAP_NET_ADMIN`/`CAP_SYS_ADMIN`. A non-root run SKIPs cleanly
//! (the K1 root gate). `uname -r` is recorded. Run via `cargo xtask lima run --
//! cargo nextest run -p overdrive-control-plane --features integration-tests`.
//! NEVER `--no-run`.
//!
//! ## Per-host singleton — runs SEQUENTIALLY with the sibling DNS responder tests
//!
//! This fixture (when un-scaffolded) boots a full production composition root
//! that binds the process-wide `:53` DNS responder and attaches XDP to the
//! FIXED `ovd-veth-cli` / `ovd-veth-bk` ifaces. nextest runs each `#[test]` in a
//! SEPARATE process by default, so two such fixtures collide on the `:53` bind /
//! `IfaceXdpSlotBusy`. This module joins the `.config/nextest.toml`
//! `host-kernel-shared` single-writer group alongside
//! `dns_responder_walking_skeleton` / `dns_responder_nxdomain` (the wiring is in
//! THIS step's `.config/nextest.toml` edit). A `#[should_panic]` scaffold body
//! never reaches the boot, but the membership is in place for the GREEN
//! transition.
//!
//! MERGE-BLOCKING on the pinned-6.18 appliance-kernel Tier-3 matrix (ADR-0068);
//! dev-Lima is necessary-but-not-sufficient and MUST be re-confirmed on 6.18
//! (DEVOPS/Tier-3 obligation, criterion 5).
//!
//! Helpers (`is_root`, `record_kernel`, the example spec + client-program
//! paths) are kept LOCAL to this module — sibling `tests/integration/
//! <scenario>.rs` files are distinct module roots and cannot import each other's
//! items (sharing would require promoting them into a shared module AND touching
//! the 02-02 / 03-01 files, out of this step's boundary; same note as
//! `dns_responder_nxdomain.rs`).

#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::doc_markdown,
    reason = "Tier-3 ping-pong scaffold; failures must panic with informative messages; F/a/b are \
              the ADR-0072 REV-2 stable-frontend / mesh-name vocabulary"
)]

use std::process::Command;

// ============================================================================
// constants — the pinned single-example (criterion 1: no PBT at Tier 3)
// ============================================================================

/// The TWO `[service].id`s of the demo — the pinned bidirectional example. `a`
/// is reachable at `a.svc.overdrive.local`, `b` at `b.svc.overdrive.local`
/// (each id is the mesh-name stem, `format!("{id}.{}", MeshServiceName::SUFFIX)`).
/// A dials b, B dials a — a closed two-hop ping-pong loop.
const SERVICE_A_ID: &str = "a";
const SERVICE_B_ID: &str = "b";

/// The mesh names each half resolves (the on-wire `getaddrinfo` queries — K2).
const PEER_NAME_FROM_A: &str = "b.svc.overdrive.local";
const PEER_NAME_FROM_B: &str = "a.svc.overdrive.local";

/// The declared TCP listener ports — chosen to AVOID 5353 (systemd-resolved /
/// mDNS owns it in the dev Lima VM) and 53 (the in-agent DNS responder). A and
/// B differ so the two specs never collide on a shared port observation.
const SERVICE_A_PORT: u16 = 18971;
const SERVICE_B_PORT: u16 = 18972;

/// The pinned ping-pong cadence — each half dials its peer on a ~10s ±5s loop;
/// the operator-observable proof is both counters advancing over a 60s window.
/// Pinned here (single golden example, Mandate 9/11) so the cadence a real
/// `overdrive alloc status` / log scrape would observe is visible at the call
/// site. (The actual 60s-window assertion lives in the EDD E05 capture once
/// #227/#75 land — see the file-level docstring.)
const PING_PONG_CADENCE_SECS: u64 = 10;
const PING_PONG_OBSERVE_WINDOW_SECS: u64 = 60;

/// The CHECKED-IN client program both `examples/dial-by-name-responder/{a,b}.toml`
/// run (K3: a real on-disk file next to the specs, no phantom path; the
/// `dns-resolver.toml` /usr/bin/socat precedent). Run by the real on-disk
/// `/usr/bin/python3` interpreter (present in the dev Lima VM). The script binds
/// a listener, replies a counted+dated PONG, and on a ~10s loop resolves its peer
/// by name (getaddrinfo, NOT dig) and dials it PLAINTEXT — the operator can run
/// it by hand with no build step.
const PING_PONG_SCRIPT: &str = "examples/dial-by-name-responder/ping_pong.py";
const PING_PONG_INTERP: &str = "/usr/bin/python3";

/// The example spec paths (criterion 2 — the per-feature `examples/<feature>/`
/// subdir convention this step introduces). `overdrive deploy <path>` consumes
/// them; the EDD E05 runner consumes them black-box.
const SPEC_A_PATH: &str = "examples/dial-by-name-responder/a.toml";
const SPEC_B_PATH: &str = "examples/dial-by-name-responder/b.toml";

// ============================================================================
// root gate + kernel record (the K1 root gate / ADR-0068 merge-gate record)
// ============================================================================

/// True iff this process is uid 0 (root). The real `EbpfDataplane` XDP attach,
/// per-workload netns provision, nft, `ip rule`, `IP_TRANSPARENT`, and the `:53`
/// responder bind all need root + CAP_NET_ADMIN/CAP_SYS_ADMIN; a non-root run
/// cannot stand up the fixture, so we SKIP rather than fail.
fn is_root() -> bool {
    // SAFETY: getuid is always safe; takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

/// Record the running kernel — the Tier-3 verdict is pinned to a kernel (dev-Lima
/// and the pinned-6.18 appliance kernel differ — ADR-0068; the merge gate is the
/// 6.18 matrix, criterion 5).
fn record_kernel() -> String {
    let kr = Command::new("uname")
        .arg("-r")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    eprintln!("[03-02] uname -r = {kr} (MERGE GATE = pinned-6.18 Tier-3 matrix, ADR-0068)");
    kr
}

// ============================================================================
// S-DBN-PINGPONG — bidirectional ping-pong demo (RED scaffold; the
// operator-runnable proof graduates to the E05 EDD expectation, honest pending)
// ============================================================================

/// S-DBN-PINGPONG (US-DBN-3 · K-DBN-3) — two services dial each other by name;
/// counters advance; each hop is mTLS'd.
///
/// RED scaffold per `distill/red-classification.md` (S-DBN-PINGPONG row) +
/// criterion 8: the operator-runnable BIDIRECTIONAL proof — `overdrive deploy
/// a.toml` then `overdrive deploy b.toml` against a `serve` that converges BOTH
/// halves to Running-AND-HEALTHY, resolves each peer (`getaddrinfo`, NOT `dig` —
/// K2), shows BOTH counters advancing on a ~10s cadence over a 60s window, and
/// each hop intercepted + mTLS'd on the inter-agent leg-B ↔ leg-C wire — is the
/// end-to-end behaviour the full-system EDD harness (#227 on #75) exercises
/// against the BUILT binary. It graduates to
/// `verification/expectations/E05-dial-by-name-ping-pong-mtls` (honest `pending`
/// until #227/#75 land, mirroring E04 — criterion 3), NOT an in-process `#[test]`
/// (the demo runs against the production binary, slice-02 § "Carpaccio").
///
/// The fixtures this step ships so the EDD runner has real artifacts: the two
/// example specs (`SPEC_A_PATH` / `SPEC_B_PATH`) and the checked-in
/// `PING_PONG_SCRIPT` client program (K3 — a real on-disk `command`, no phantom
/// path; the operator runs it by hand with no build step). The PLAINTEXT-egress
/// model (criterion 7) is baked into the script: it dials its peer over an
/// ordinary plaintext socket, never presenting TLS (the 02-02 RCA model error);
/// the per-hop mTLS proof lives on the inter-agent leg-B ↔ leg-C wire.
///
/// GREEN transition: when #227/#75 land, replace the `panic!` with the
/// two-`deploy` drive + the `getent`-per-peer resolution asserts + the
/// both-counters-advance-over-60s assertion + the per-hop `tcpdump`/`ss -tie`
/// 0x17 inter-agent-wire proof (the 02-02 `WireScan` oracle, applied to BOTH
/// hops), and capture the E05 evidence black-box. Until then this is the honest
/// RED-scaffold spec.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[should_panic(expected = "RED scaffold")]
async fn two_services_dial_each_other_by_name_counters_advance_each_hop_mtls() {
    if !is_root() {
        eprintln!(
            "SKIP two_services_dial_each_other_by_name_counters_advance_each_hop_mtls: not root"
        );
        // A non-root run SKIPs cleanly (the K1 root gate) — orthogonal to RED.
        // Panic with the RED-scaffold marker so the #[should_panic] still holds
        // on the non-root path (the scaffold's RED bar is the missing
        // bidirectional drive, not the root gate).
        panic!(
            "Not yet implemented -- RED scaffold (S-DBN-PINGPONG / bidirectional ping-pong demo, \
             non-root SKIP path)"
        );
    }
    let _kr = record_kernel();

    // The K3 fixtures are checked in (the two example specs + the `ping_pong.py`
    // client program exist on disk next to each other), so the EDD E05 runner has
    // real artifacts when #227/#75 land; the bidirectional DRIVE is what the EDD
    // harness completes.
    eprintln!(
        "[03-02] S-DBN-PINGPONG fixtures: A=({SERVICE_A_ID}.svc:{SERVICE_A_PORT} dials \
         {PEER_NAME_FROM_A}); B=({SERVICE_B_ID}.svc:{SERVICE_B_PORT} dials {PEER_NAME_FROM_B}); \
         specs {SPEC_A_PATH} / {SPEC_B_PATH}; client {PING_PONG_INTERP} {PING_PONG_SCRIPT}; \
         cadence ~{PING_PONG_CADENCE_SECS}s over a {PING_PONG_OBSERVE_WINDOW_SECS}s window."
    );

    // RED scaffold (distill/red-classification.md S-DBN-PINGPONG): the
    // operator-runnable bidirectional proof — two `overdrive deploy`s against a
    // booted `overdrive serve`, both counters advancing over a 60s window, each
    // hop mTLS'd on the inter-agent leg — is the end-to-end behaviour the
    // full-system EDD harness (#227 on #75) exercises against the BUILT binary,
    // and graduates to the E05 EDD expectation (honest `pending`). It is NOT an
    // in-process `#[test]` (the demo runs against the production binary). Do NOT
    // contort this green by hand-installing a production effect or a
    // TLS-presenting dialer (the 02-02/03-01 anti-pattern).
    panic!(
        "Not yet implemented -- RED scaffold (S-DBN-PINGPONG / bidirectional ping-pong demo, \
         US-DBN-3 / K-DBN-3): the operator-runnable two-deploy bidirectional proof graduates to \
         verification/expectations/E05-dial-by-name-ping-pong-mtls (honest pending, #227/#75)"
    );
}
