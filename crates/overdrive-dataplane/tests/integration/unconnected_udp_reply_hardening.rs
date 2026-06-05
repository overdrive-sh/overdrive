//! S-03-01 / S-03-02 / S-03-03 — unconnected-UDP reply-path error
//! hardening: no-op-on-miss (non-service UDP unaffected; no backend-IP
//! leak via always-hit), below-floor attach refusal, and
//! fixture-collision discipline.
//!
//! Feature: `unconnected-udp-sendmsg4` (GH #200, ADR-0053 rev 2026-06-05).
//! Story: US-03. Job: J-OPS-004 (primary), J-PLAT-004.
//!
//! Tags: `@US-03 @kpi-K5 @tier3 @real-io @adapter-integration @error`.
//! Tier: **Tier 3 (real kernel — THE GATE).** No Tier-2 backstop.
//!
//! # What these prove (the corrected app-sockaddr ACs — DDD-3 / DDD-3a / CA-3 / UI-1)
//!
//! - **S-03-01 (no-op-on-miss; non-service UDP unaffected):** recvmsg4
//!   attaches at a cgroup ANCESTOR and fires on EVERY unconnected-UDP
//!   `recvmsg`/`recvfrom` from any descendant — service replies AND all
//!   unrelated same-host UDP (DNS clients, a backend's own `recvfrom` of
//!   an inbound query). The REVERSE_LOCAL_MAP lookup is the discriminator.
//!   Three corrected assertions:
//!   (a) **non-service unconnected UDP is unaffected** — a same-host
//!       exchange whose source is NOT a registered backend reads its REAL
//!       sender address via `recvfrom`/`msg_name`; recvmsg4 leaves it
//!       byte-for-byte intact (pure no-op on a miss — the load-bearing
//!       new assertion, the regression the correction fixes);
//!   (b) **a service reply always HITS → VIP-sourced** — under the D1
//!       reverse-first dual-write a genuine service reply's source is
//!       always a registered backend identity, so it always hits and the
//!       app reads the VIP as the source — no backend-IP-leak path;
//!   (c) **the miss counter is observable but inert** —
//!       REVERSE_LOCAL_MISS_COUNTER increments on a non-service recv AND
//!       the source the app read on that same recv is untouched (counted
//!       but no source rewrite). recvmsg4 CANNOT deny (verifier `[1,1]`,
//!       research Q1) — every path returns 1; the no-leak guarantee (K5)
//!       holds via the always-hit dual-write, NOT a miss-path sentinel.
//!   App-sockaddr assertions, NOT `tcpdump`/wire (recvmsg4 never touches
//!   the wire). There is NO sentinel `192.0.2.1` rewrite on the miss
//!   path — it would corrupt every non-service datagram's sender address
//!   (Tier-3-observed, fixed in DELIVER step 01-03, commit `e71ad780`).
//!   No-op-on-miss is Cilium-aligned. Per DDD-3 / feature-delta CA-3 /
//!   research addendum "UI-1 adjudication (2026-06-05)".
//! - **S-03-02 (below-floor refusal):** a host below the recvmsg4 floor
//!   (< 4.20) fails `attach()` and the composition root refuses to start
//!   with a structured `health.startup.refused` (the `attach()` syscall
//!   IS the preflight — NO `/proc`/`uname` parse, DDD-5b/c). The failure
//!   routes through a `#[from]`-typed DataplaneBootError variant, never a
//!   flattened `Internal(String)`. On the 5.10+ Lima matrix this asserts
//!   the refusal SHAPE via the typed-error path (a real <4.20 kernel is
//!   not on the matrix).
//! - **S-03-03 (fixture collision):** the stub resolver binds OFF UDP
//!   5353 and asserts a clean `bind` — an `EADDRINUSE` fails the test
//!   loudly, never swallowed with `.ok()` / `let _`
//!   (`.claude/rules/debugging.md` § 11 + § 8).
//!
//! # RED scaffold
//!
//! `#[should_panic(expected = "RED scaffold")]` per the project RED
//! convention. DELIVER replaces each `panic!` with the real
//! `EbpfDataplane`-driven assertion (Slice 03 GREEN; depends on Slice 01
//! + Slice 02).

#![allow(clippy::missing_panics_doc)]
#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::items_after_statements,
    clippy::doc_markdown
)]

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_dataplane::EbpfDataplane;

use super::helpers::veth::{VethError, VethPair};

/// DNS-shape service VIP. Distinct from any host-assigned address.
const VIP: Ipv4Addr = Ipv4Addr::new(10, 96, 0, 10);
/// VIP port — the DNS port 53. Nothing binds 53; the cgroup hooks rewrite
/// VIP:53 → backend (the backend binds an ephemeral port off :53/:5353 per
/// `.claude/rules/debugging.md` § 11).
const VIP_PORT: u16 = 53;

/// Per-test bpffs pin dir, cleaned on construction + on drop.
struct PinDirGuard(PathBuf);
impl Drop for PinDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Bring up a veth pair + per-test bpffs pin dir + the production
/// `EbpfDataplane` with all three cgroup hooks attached at
/// `/sys/fs/cgroup`. Returns `None` (skip) when `CAP_NET_ADMIN` is absent.
/// Mirrors `unconnected_udp_roundtrip::bring_up`.
fn bring_up(host: &str, peer: &str) -> Option<(EbpfDataplane, VethPair, PinDirGuard)> {
    let veth = match VethPair::create(host, peer) {
        Ok(v) => v,
        Err(VethError::CapNetAdminRequired) => {
            eprintln!(
                "skip: recvmsg4 no-op-on-miss hardening needs CAP_NET_ADMIN for veth setup — \
                 run via `cargo xtask lima run --` (default-root)"
            );
            return None;
        }
        Err(e) => panic!("veth setup failed: {e}"),
    };

    let pin_dir =
        PathBuf::from(format!("/sys/fs/bpf/overdrive-test-uudp-hard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pin_dir);
    std::fs::create_dir_all(&pin_dir).expect("create pin dir");
    let pin_guard = PinDirGuard(pin_dir.clone());

    let dataplane = EbpfDataplane::new_with_pin_dir(
        &veth.host,
        &veth.peer,
        &pin_dir,
        std::path::Path::new("/sys/fs/cgroup"),
    )
    .expect("EbpfDataplane::new_with_pin_dir with cgroup sendmsg4+recvmsg4 attach");

    Some((dataplane, veth, pin_guard))
}

/// Spawn a UDP echo responder on an EPHEMERAL loopback port (off
/// systemd-resolved's :53/:5353). Echoes each datagram back to its sender.
fn spawn_udp_echo(rounds: usize) -> SocketAddrV4 {
    let sock = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("bind UDP echo backend (ephemeral port, off :53/:5353)");
    let bound = match sock.local_addr().expect("udp local_addr") {
        std::net::SocketAddr::V4(v4) => v4,
        std::net::SocketAddr::V6(_) => unreachable!("bound IPv4 backend"),
    };
    thread::spawn(move || {
        let mut buf = [0u8; 64];
        for _ in 0..rounds {
            match sock.recv_from(&mut buf) {
                Ok((n, src)) => {
                    let _ = sock.send_to(&buf[..n], src);
                }
                Err(_) => return,
            }
        }
    });
    bound
}

/// One unconnected `sendto(VIP:VIP_PORT)` + `recvfrom`. The socket is NEVER
/// `connect()`ed — sendmsg4 fires here and rewrites VIP:53 → backend.
fn unconnected_query(payload: &[u8]) -> Option<([u8; 64], usize, SocketAddrV4)> {
    let client = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).ok()?;
    client.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    client.send_to(payload, (VIP, VIP_PORT)).ok()?;
    let mut buf = [0u8; 64];
    let (n, src) = client.recv_from(&mut buf).ok()?;
    let src_v4 = match src {
        std::net::SocketAddr::V4(v4) => v4,
        std::net::SocketAddr::V6(_) => return None,
    };
    Some((buf, n, src_v4))
}

/// Poll `f` until it returns `Some` or the deadline elapses.
fn poll_until<T>(budget: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// A same-host UDP exchange whose source is NOT a registered backend.
/// `peer` echoes back to the client on a FREE ephemeral port; the client
/// `sendto(peer)` directly (no VIP, no `connect()`), and `recvfrom`s the
/// reply. recvmsg4 fires on this recv (cgroup-ancestor attach) and MUST be
/// a pure no-op on the `REVERSE_LOCAL_MAP` miss — the source the app reads
/// MUST be the real `peer` address, byte-for-byte.
fn non_service_exchange(payload: &[u8]) -> Option<([u8; 64], usize, SocketAddrV4)> {
    // The peer is a plain echo server on a free ephemeral port — its
    // address is NOT in REVERSE_LOCAL_MAP, so its reply is a miss.
    let peer = spawn_udp_echo(1);
    let client = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).ok()?;
    client.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    client.send_to(payload, peer).ok()?;
    let mut buf = [0u8; 64];
    let (n, src) = client.recv_from(&mut buf).ok()?;
    let src_v4 = match src {
        std::net::SocketAddr::V4(v4) => v4,
        std::net::SocketAddr::V6(_) => return None,
    };
    // Real sender source MUST be the peer we sent to — recvmsg4 left it
    // untouched on the miss.
    Some((buf, n, src_v4))
}

/// S-03-01 — recvmsg4 no-op-on-miss: non-service unconnected UDP reads its
/// real source; a service reply always hits and is VIP-sourced; the miss
/// counter increments on non-service recv but is behaviorally inert.
///
/// THE corrected app-sockaddr ACs (DDD-3 / DDD-3a / CA-3 / UI-1). All three
/// assertions are made against the same live `EbpfDataplane` so the
/// counted-but-inert pinning (assertion c) is observed on a real
/// non-service recv. App-sockaddr (`recvfrom` source), NOT `tcpdump`.
///
/// Equivalence pin against shipped behavior: the corrected no-op-on-miss
/// HIT-rewrite + counter-bump landed in step 01-03 (commit `e71ad780`,
/// Tier-3 green). This hardening test lands GREEN immediately — it pins the
/// three corrected assertions against already-shipped production rather
/// than driving a RED→GREEN transition.
#[test]
#[serial_test::serial(env)]
fn non_service_unconnected_udp_reads_real_source_recvmsg4_noop_on_miss() {
    let Some((dataplane, _veth, _pin_guard)) = bring_up("ovd-uudph0a", "ovd-uudph0b") else {
        return;
    };

    // Register a genuine service backend so the reverse-first dual-write
    // populates REVERSE_LOCAL_MAP — assertion (b) needs a real registered
    // backend whose reply ALWAYS hits.
    let backend = spawn_udp_echo(1);
    assert_ne!(backend.port(), 53, "fixture: backend must bind off :53");
    assert_ne!(backend.port(), 5353, "fixture: backend must bind off :5353");

    let rt = tokio::runtime::Runtime::new().expect("tokio rt");
    rt.block_on(async {
        dataplane
            .register_local_backend(VIP, VIP_PORT, backend, Proto::Udp)
            .await
            .expect("register UDP local backend (reverse-first dual-write)");
    });

    // ---- Assertion (a) — non-service unconnected UDP is unaffected.
    //      A same-host exchange whose source is NOT a registered backend
    //      reads its REAL sender address. The miss counter before/after
    //      brackets this recv so assertion (c) can pin the increment.
    let miss_before =
        dataplane.reverse_local_miss_count().expect("dump REVERSE_LOCAL_MISS_COUNTER (before)");

    let probe_ns = b"non-service-unconnected-udp";
    let (buf_ns, n_ns, ns_src) = poll_until(Duration::from_secs(2), || {
        let (buf, n, src) = non_service_exchange(probe_ns)?;
        (&buf[..n] == probe_ns).then_some((buf, n, src))
    })
    .expect("non-service unconnected UDP exchange did not round-trip within 2s");
    assert_eq!(&buf_ns[..n_ns], probe_ns, "non-service echo payload integrity");

    // The load-bearing new assertion (the regression the correction fixes):
    // recvmsg4 left the real sender source byte-for-byte intact on the miss.
    // It is the loopback peer's real address — NOT the VIP, NOT the sentinel
    // 192.0.2.1, NOT mangled. A sentinel/source rewrite on the miss path
    // would corrupt this.
    assert_eq!(
        *ns_src.ip(),
        Ipv4Addr::LOCALHOST,
        "non-service recvfrom source MUST be the REAL sender ({}), left untouched by the \
         recvmsg4 no-op-on-miss — got {}; a miss-path source rewrite (sentinel/VIP) would \
         corrupt every non-service datagram's sender address",
        Ipv4Addr::LOCALHOST,
        ns_src.ip()
    );
    assert_ne!(
        *ns_src.ip(),
        VIP,
        "non-service recvfrom source must NOT be rewritten to the VIP on a miss"
    );
    assert_ne!(
        *ns_src.ip(),
        Ipv4Addr::new(192, 0, 2, 1),
        "non-service recvfrom source must NOT be the rejected sentinel 192.0.2.1 — \
         no sentinel rewrite exists on the miss path (UI-1)"
    );

    // ---- Assertion (c), part 1 — the miss counter incremented on the
    //      non-service recv (observable via the percpu-array dump). It is
    //      behaviorally inert: assertion (a) above proved the source on the
    //      SAME class of recv was untouched. Counted but inert, together.
    let miss_after =
        dataplane.reverse_local_miss_count().expect("dump REVERSE_LOCAL_MISS_COUNTER (after)");
    assert!(
        miss_after > miss_before,
        "REVERSE_LOCAL_MISS_COUNTER MUST increment on the non-service recv (the source was \
         not a registered backend → REVERSE_LOCAL_MAP miss); before={miss_before} after={miss_after}"
    );

    // ---- Assertion (b) — a genuine service reply ALWAYS hits → VIP-sourced.
    //      Under the reverse-first dual-write the backend's reply source is
    //      a registered backend identity, so recvmsg4 HITS and rewrites the
    //      source the app reads to the VIP — no backend-IP-leak path.
    let probe_svc = b"service-reply-query";
    let svc_src = poll_until(Duration::from_secs(2), || {
        let (buf, n, src) = unconnected_query(probe_svc)?;
        (&buf[..n] == probe_svc).then_some(src)
    })
    .expect(
        "service unconnected sendto(VIP:53) did not round-trip + echo within 2s — \
         cgroup sendmsg4 forward rewrite (VIP→backend) regression",
    );
    assert_eq!(
        *svc_src.ip(),
        VIP,
        "service reply recvfrom source MUST be the VIP {VIP} (recvmsg4 HIT reverse rewrite) — \
         got {}; the registered backend always hits, so no backend IP ever reaches the app",
        svc_src.ip()
    );

    drop(dataplane);
}

/// S-03-02 — a below-floor kernel refuses observably at attach/preflight
/// via a typed DataplaneBootError, never a forward-only half-working service.
#[test]
#[should_panic(expected = "RED scaffold")]
fn below_floor_kernel_refuses_at_attach_preflight_observably() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03-02: below recvmsg4 floor (<4.20) the \
         attach() syscall fails and the composition root refuses with health.startup.refused \
         via a #[from]-typed DataplaneBootError -- never Internal(String), never a forward-only \
         half-working service; NO /proc/uname parse). Blocked on: probe attaches both hooks + \
         CgroupSendRecvAttach/ReverseLocalProbe error variants (Slice 03 GREEN)."
    );
}

/// S-03-03 — the Tier-3 stub resolver binds off UDP 5353 and asserts a
/// clean bind; an EADDRINUSE fails the test loudly.
#[test]
#[should_panic(expected = "RED scaffold")]
fn stub_resolver_binds_off_5353_and_asserts_clean_bind() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03-03: the stub UDP responder binds off the \
         systemd-resolved-owned UDP 5353/:53 and asserts a clean bind; an EADDRINUSE fails \
         the test loudly, never swallowed with .ok()/let _). Blocked on: the Tier-3 stub-resolver \
         fixture (Slice 03 GREEN)."
    );
}
