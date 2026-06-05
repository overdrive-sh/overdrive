//! S-01-01 / S-01-02 / S-01-03 / S-02-03 — unconnected-UDP same-host
//! round-trip through the real cgroup sendmsg4 + recvmsg4 hooks.
//!
//! Feature: `unconnected-udp-sendmsg4` (GH #200, ADR-0053 rev 2026-06-05).
//! Story: US-01 (WALKING SKELETON) + US-02 Tier-3 prong. Job: J-OPS-004,
//! J-PLAT-004.
//!
//! Tags: `@walking_skeleton @US-01 @US-02 @kpi-K1 @kpi-K2 @kpi-K3`
//!       `@tier3 @real-io @adapter-integration @driving_adapter`.
//! Tier: **Tier 3 (real kernel — THE GATE).** There is NO Tier-2
//! `BPF_PROG_TEST_RUN` backstop for `cgroup_sock_addr` (ENOTSUPP ≤ 6.8);
//! the Tier-1 `reply-source-rewrite-lockstep` invariant is the structural
//! defense below this gate.
//!
//! # What these prove (the reframed app-sockaddr ACs — DDD-3a)
//!
//! With a same-host DNS-shape UDP service on a VIP and one local backend
//! registered via the production dual-write:
//!
//! - **S-01-01 (WS):** a same-host client `sendto(VIP)` WITHOUT `connect()`
//!   reaches the backend AND the source it reads via `recvfrom` is the
//!   **VIP** (the recvmsg4 reply-source rewrite). Asserted at the
//!   **application sockaddr layer** (`recvfrom` return), NOT via
//!   `tcpdump -i lo` (which shows the backend source on every round-trip
//!   regardless — recvmsg4 fires post-dequeue; research Q4).
//! - **S-01-02:** `bpftool`-equivalent dumps show BOTH the forward
//!   `LOCAL_BACKEND_MAP (vip, port, udp) -> backend` and the reverse
//!   `REVERSE_LOCAL_MAP (backend, udp) -> vip` entries after ONE
//!   `register_local_backend` (ordered reverse-first; no forward-without-
//!   reverse window).
//! - **S-01-03:** a second unconnected `sendto` reuses the same entries
//!   (stateless; no conntrack).
//! - **S-02-03:** the Tier-3 reply-source identity meets the Tier-1
//!   reply mirror at the shared backend identity. (Still RED-armed — that
//!   is step 02-01's GREEN target, not this step.)
//!
//! # Fixture discipline (S-03-03)
//!
//! The stub UDP responder binds an EPHEMERAL port (port 0) — off the
//! systemd-resolved-owned UDP 5353 (and :53) per
//! `.claude/rules/debugging.md` § 11 — and asserts a clean `bind` rather
//! than swallowing `EADDRINUSE` (§ 8 — no `let _` on fallible setup). The
//! test process runs as a descendant of the configured
//! `cgroup_attach_path` (`/sys/fs/cgroup`) so the hooks fire —
//! `cargo xtask lima run --` runs nextest as root under that ancestor
//! (the `local_backend_proto_connect.rs` harness model).

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
/// VIP port — the DNS port 53. The cgroup hooks rewrite VIP:53 → backend;
/// nothing actually binds 53, so the privileged-port bind constraint
/// never applies to the test process (the BACKEND binds an ephemeral
/// port, off systemd-resolved's :53/:5353 per debugging.md § 11).
const VIP_PORT: u16 = 53;

/// Per-test bpffs pin dir, cleaned on construction + on drop.
struct PinDirGuard(PathBuf);
impl Drop for PinDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Bring up a veth pair (real ifaces for the XDP attach) + a per-test
/// bpffs pin dir + the production `EbpfDataplane` with all three cgroup
/// hooks attached at `/sys/fs/cgroup`. Returns `None` (with a skip
/// message) when `CAP_NET_ADMIN` is absent — the test caller returns
/// early rather than failing.
///
/// The veth pair only exists so `EbpfDataplane::new_with_pin_dir` has
/// real ifaces to attach its XDP programs to; the cgroup path under test
/// rewrites to the backend's own address and never traverses the veth.
fn bring_up(host: &str, peer: &str) -> Option<(EbpfDataplane, VethPair, PinDirGuard)> {
    let veth = match VethPair::create(host, peer) {
        Ok(v) => v,
        Err(VethError::CapNetAdminRequired) => {
            eprintln!(
                "skip: unconnected-UDP round-trip needs CAP_NET_ADMIN for veth setup — \
                 run via `cargo xtask lima run --` (default-root)"
            );
            return None;
        }
        Err(e) => panic!("veth setup failed: {e}"),
    };

    let pin_dir = PathBuf::from(format!("/sys/fs/bpf/overdrive-test-uudp-{}", std::process::id()));
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

/// Spawn a UDP echo responder bound to an EPHEMERAL loopback port (off
/// systemd-resolved's :53/:5353). Echoes each datagram back to its
/// sender for `rounds` rounds. Returns the bound `SocketAddrV4`.
fn spawn_udp_stub_resolver(rounds: usize) -> SocketAddrV4 {
    let sock = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("bind UDP stub-resolver backend (ephemeral port, off :53/:5353)");
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

/// Perform ONE unconnected `sendto(VIP:VIP_PORT)` + `recvfrom`, returning
/// the `(payload_echoed, recvfrom_source)` observed by the app. The
/// socket is NEVER `connect()`ed — `sendto` per datagram, the canonical
/// resolver idiom that `connect4` never sees and `sendmsg4` does.
///
/// Returns `None` on timeout / I/O error so the caller can poll.
fn unconnected_query(payload: &[u8]) -> Option<([u8; 64], usize, SocketAddrV4)> {
    let client = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).ok()?;
    client.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    // Unconnected send — no prior connect(). sendmsg4 fires here and
    // rewrites the destination VIP:53 → backend.
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

/// S-01-01 (WALKING SKELETON) — unconnected `sendto`/`recvfrom` round-trip;
/// the `recvfrom` source the app reads is the VIP, not the backend IP.
///
/// ASSERTION LAYER (DDD-3a): this asserts the APPLICATION sockaddr the
/// client reads back — the source in `recvfrom` MUST equal the VIP
/// (10.96.0.10), NOT the backend IP. It does NOT assert the wire. The
/// round-trip must really deliver (forward sendmsg4 rewrite fires) AND
/// the app-source rewrite must really fire (reverse recvmsg4 rewrite).
#[test]
#[serial_test::serial(env)]
fn unconnected_sendto_recvfrom_reads_vip_sourced_reply() {
    let Some((dataplane, _veth, _pin_guard)) = bring_up("ovd-uudp0a", "ovd-uudp0b") else {
        return;
    };

    let backend = spawn_udp_stub_resolver(1);
    assert_ne!(backend.port(), 53, "fixture: backend must bind off :53");
    assert_ne!(backend.port(), 5353, "fixture: backend must bind off :5353");

    let rt = tokio::runtime::Runtime::new().expect("tokio rt");
    rt.block_on(async {
        dataplane
            .register_local_backend(VIP, VIP_PORT, backend, Proto::Udp)
            .await
            .expect("register UDP local backend (reverse-first dual-write)");
    });

    let probe = b"unconnected-dns-query-1";
    let observed = poll_until(Duration::from_secs(2), || {
        let (buf, n, src) = unconnected_query(probe)?;
        (&buf[..n] == probe).then_some(src)
    });

    let src = observed.expect(
        "unconnected sendto(VIP:53) did not round-trip + echo within 2s — \
         cgroup sendmsg4 forward rewrite (VIP→backend) regression",
    );

    // THE walking-skeleton assertion: the source the app reads from
    // recvfrom is the VIP, NOT the backend IP — the recvmsg4 reverse
    // source rewrite fired.
    assert_eq!(
        *src.ip(),
        VIP,
        "recvfrom source MUST be the VIP {VIP} (recvmsg4 reverse source rewrite), \
         got {} — without recvmsg4 a source-validating resolver discards this reply",
        src.ip()
    );

    drop(dataplane);
}

/// S-01-02 — both forward LOCAL_BACKEND_MAP and reverse REVERSE_LOCAL_MAP
/// entries present after ONE register_local_backend (ordered reverse-first).
#[test]
#[serial_test::serial(env)]
fn forward_and_reverse_map_entries_present_after_one_register() {
    let Some((dataplane, _veth, _pin_guard)) = bring_up("ovd-uudp1a", "ovd-uudp1b") else {
        return;
    };

    let backend = spawn_udp_stub_resolver(0);

    let rt = tokio::runtime::Runtime::new().expect("tokio rt");
    rt.block_on(async {
        dataplane
            .register_local_backend(VIP, VIP_PORT, backend, Proto::Udp)
            .await
            .expect("register UDP local backend (single dual-write)");
    });

    // Forward: LOCAL_BACKEND_MAP (vip, 53, udp) → backend.
    let fwd = dataplane.local_backend_map_entries().expect("dump LOCAL_BACKEND_MAP");
    let fwd_present = fwd.iter().any(|(k, v)| {
        k.vip_host == u32::from(VIP)
            && k.port_host == VIP_PORT
            && k.proto == Proto::Udp.as_u8()
            && v.backend_ip_host == u32::from(*backend.ip())
            && v.backend_port_host == backend.port()
    });
    assert!(
        fwd_present,
        "LOCAL_BACKEND_MAP must carry (vip, 53, udp) → backend after one register; entries={fwd:?}"
    );

    // Reverse: REVERSE_LOCAL_MAP (backend_ip, backend_port, udp) → vip.
    let rev = dataplane.reverse_local_map_entries().expect("dump REVERSE_LOCAL_MAP");
    let rev_present = rev.iter().any(|(k, vip_host)| {
        k.backend_ip_host == u32::from(*backend.ip())
            && k.backend_port_host == backend.port()
            && k.proto == Proto::Udp.as_u8()
            && *vip_host == u32::from(VIP)
    });
    assert!(
        rev_present,
        "REVERSE_LOCAL_MAP must carry (backend, udp) → vip after the SAME single register \
         (ordered reverse-first — no forward-without-reverse window); entries={rev:?}"
    );

    drop(dataplane);
}

/// S-01-03 — a second unconnected query reuses the same mapping (stateless).
#[test]
#[serial_test::serial(env)]
fn second_unconnected_query_reuses_same_mapping_statelessly() {
    let Some((dataplane, _veth, _pin_guard)) = bring_up("ovd-uudp2a", "ovd-uudp2b") else {
        return;
    };

    // Two echo rounds — the second query reuses the SAME map entries, no
    // per-flow state is created between them (UDP stateless, no conntrack).
    let backend = spawn_udp_stub_resolver(2);

    let rt = tokio::runtime::Runtime::new().expect("tokio rt");
    rt.block_on(async {
        dataplane
            .register_local_backend(VIP, VIP_PORT, backend, Proto::Udp)
            .await
            .expect("register UDP local backend");
    });

    // First query — establishes the round-trip works.
    let first = b"unconnected-dns-query-A";
    let first_src = poll_until(Duration::from_secs(2), || {
        let (buf, n, src) = unconnected_query(first)?;
        (&buf[..n] == first).then_some(src)
    })
    .expect("first unconnected query round-trip");
    assert_eq!(*first_src.ip(), VIP, "first reply source == VIP");

    // Second query for a DIFFERENT name, immediately after, from a fresh
    // unconnected socket — served by the SAME entries (point-lookup), no
    // new state. Source is again the VIP.
    let second = b"unconnected-dns-query-B-diff";
    let second_src = poll_until(Duration::from_secs(2), || {
        let (buf, n, src) = unconnected_query(second)?;
        (&buf[..n] == second).then_some(src)
    })
    .expect(
        "second unconnected query did NOT reuse the same mapping within 2s — \
         a per-flow-state assumption (conntrack) would break this; the cgroup \
         path is stateless point-lookup",
    );
    assert_eq!(
        *second_src.ip(),
        VIP,
        "second reply source MUST again be the VIP {VIP} — same REVERSE_LOCAL_MAP entry, \
         no per-flow state; got {}",
        second_src.ip()
    );

    drop(dataplane);
}

/// S-02-03 — the Tier-3 reply-source identity meets the Tier-1 reply
/// mirror at the shared backend identity.
///
/// STILL RED-armed: this is step 02-01's GREEN target (the SimDataplane
/// reply-mirror write + the `reply_source_rewrite_lockstep` invariant
/// flip), NOT this step. Step 01-03 lands the EbpfDataplane dual-write +
/// the kernel programs only; this scaffold stays `#[should_panic]`-armed
/// and green-by-construction until 02-01.
#[test]
#[should_panic(expected = "RED scaffold")]
fn kernel_reply_source_meets_tier1_reply_mirror_at_backend_identity() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02-03: the real recvmsg4 reply source (VIP) \
         and REVERSE_LOCAL_MAP (backend,udp)->vip match the Tier-1 reply_source_for(...) for \
         the same BackendKey; removing the kernel reply rewrite fails this Tier-3 acceptance). \
         Blocked on: Slice 01 + Slice 02 GREEN (step 02-01 SimDataplane reply-mirror)."
    );
}
