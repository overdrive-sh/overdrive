//! P5 — fragment-tracking lifecycle of the egress unframe program.
//!
//! netns-density-295 vsock attachment replacement, DISTILL piece P5
//! (`docs/feature/netns-density-295/distill/test-scenarios-p5-egress-unframe-hostinternal.md`,
//! P5-S4; R5-47; G-V6). Contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Driven port — host forwarder" → "Fragment-tracking map
//! (FRAG-MAP-LIFECYCLE, ADR-0165)" (P-45, V-18).
//!
//! The invariant under test: a later fragment is moved back by one 8-byte unit
//! only if its own datagram's first fragment was stripped and its record still
//! exists; an evicted record loses at most its own datagram (whole), never
//! corrupts another; inserts never fail for capacity
//! (`GUEST_UNFRAME_FRAGMENTS_MAX` LRU entries).
//!
//! Adapter integration (Tier-3, example-based): every test drives the real
//! `GuestFlowForwarder` — `load`, `converge_unframe_interfaces` over a veth
//! (MTU 1500) into a peer namespace, `install_datagram` for the registered
//! tuple — and observes what the peer's IPv4 reassembly delivers and the
//! forwarder's `counters()` (`unframe_fragment_shifted`,
//! `unframe_fragment_orphan`). Hand-crafted fragments (the same reassembly
//! key reused; cut trains) are written with an `AF_PACKET` socket on the host
//! end of the veth, so they traverse the interface's TCX egress hook — and the
//! unframe program — without the root namespace's IPv4 defragmentation.
//!
//! The whole `overdrive-dataplane` `integration` binary is in the
//! `host-kernel-shared` nextest group.

#![allow(clippy::expect_used, clippy::doc_markdown)]

use std::collections::BTreeSet;
use std::net::{SocketAddrV4, UdpSocket};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::time::Duration;

use overdrive_core::guest_transport::wire::GUEST_UNFRAME_FRAGMENTS_MAX;
use overdrive_dataplane::guest_flow::{GuestFlowCounters, GuestFlowForwarder};

use super::helpers::guest_net::{
    DatagramAssociation, OwnerCgroup, PeerLink, frame, interface_index, patterned, recv_datagram,
    require_vsock_loopback, run_cleanup, run_ok,
};
use super::helpers::packets::ipv4_header_checksum;
use super::helpers::traffic::set_socket_rcvbuf;

/// The reassembly-key identification the S1 cell reuses for both datagrams.
const ID: u16 = 0x4242;

/// IPv4 payload carried per fragment at MTU 1500 (1480, a multiple of 8).
const FRAGMENT_DATA: usize = 1480;

fn v4(sock: &UdpSocket) -> SocketAddrV4 {
    match sock.local_addr().expect("socket has a local address") {
        std::net::SocketAddr::V4(a) => a,
        std::net::SocketAddr::V6(a) => panic!("IPv4 socket expected, got {a}"),
    }
}

/// Number of later (non-first) fragments of an IPv4 payload of `len` bytes
/// at MTU 1500.
const fn later_fragments(len: usize) -> u64 {
    (len.div_ceil(FRAGMENT_DATA) - 1) as u64
}

/// A forwarder with the unframe program converged onto `links`' host ends.
fn converged(cgroup: &OwnerCgroup, links: &[&PeerLink]) -> GuestFlowForwarder {
    let forwarder = GuestFlowForwarder::load(cgroup.fd()).expect("load");
    let ifaces: BTreeSet<_> = links.iter().map(|l| interface_index(l.host_ifindex())).collect();
    let convergence = forwarder.converge_unframe_interfaces(&ifaces).expect("converge");
    assert!(convergence.failed.is_empty(), "precondition: converge attached every interface");
    forwarder
}

fn counters(f: &GuestFlowForwarder) -> GuestFlowCounters {
    f.counters().expect("counters")
}

/// IPv4 fragments of one UDP datagram (`udp_payload` after an 8-byte UDP
/// header with checksum 0, as `SO_NO_CHECK` sends), `FRAGMENT_DATA` bytes of
/// IPv4 payload each, identification `id`.
fn udp_fragments(
    src: SocketAddrV4,
    dst: SocketAddrV4,
    id: u16,
    udp_payload: &[u8],
) -> Vec<Vec<u8>> {
    let udp_len = u16::try_from(8 + udp_payload.len()).expect("UDP length fits u16");
    let mut segment = Vec::with_capacity(usize::from(udp_len));
    segment.extend_from_slice(&src.port().to_be_bytes());
    segment.extend_from_slice(&dst.port().to_be_bytes());
    segment.extend_from_slice(&udp_len.to_be_bytes());
    segment.extend_from_slice(&[0, 0]);
    segment.extend_from_slice(udp_payload);

    let chunks: Vec<&[u8]> = segment.chunks(FRAGMENT_DATA).collect();
    let last = chunks.len() - 1;
    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let total = u16::try_from(20 + chunk.len()).expect("fragment fits u16");
            let offset_units = u16::try_from(i * FRAGMENT_DATA / 8).expect("offset fits u16");
            let flags_offset = if i == last { offset_units } else { 0x2000 | offset_units };
            let mut ip = Vec::with_capacity(usize::from(total));
            ip.extend_from_slice(&[0x45, 0]);
            ip.extend_from_slice(&total.to_be_bytes());
            ip.extend_from_slice(&id.to_be_bytes());
            ip.extend_from_slice(&flags_offset.to_be_bytes());
            ip.extend_from_slice(&[64, 17, 0, 0]);
            ip.extend_from_slice(&src.ip().octets());
            ip.extend_from_slice(&dst.ip().octets());
            let csum = ipv4_header_checksum(&ip);
            ip[10..12].copy_from_slice(&csum.to_be_bytes());
            ip.extend_from_slice(chunk);
            ip
        })
        .collect()
}

/// Writes Ethernet frames onto the host end of a link through `AF_PACKET`:
/// they pass the interface's TCX egress hook (the unframe program) and reach
/// the peer.
struct PacketInjector {
    fd: OwnedFd,
    addr: libc::sockaddr_ll,
    header: [u8; 14],
}

impl PacketInjector {
    fn on(link: &PeerLink) -> Self {
        // SAFETY: plain socket(2) call; the result is checked before use.
        let raw = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW | libc::SOCK_CLOEXEC, 0) };
        assert!(raw >= 0, "precondition: AF_PACKET socket: {}", std::io::Error::last_os_error());
        // SAFETY: `raw` is a fresh descriptor this process owns.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let dst = link.peer_mac();
        let src = link.host_mac();
        // SAFETY: an all-zero `sockaddr_ll` is a valid initial value.
        let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        addr.sll_family = u16::try_from(libc::AF_PACKET).expect("AF_PACKET fits u16");
        addr.sll_protocol = u16::try_from(libc::ETH_P_IP).expect("ETH_P_IP fits u16").to_be();
        addr.sll_ifindex = i32::try_from(link.host_ifindex()).expect("ifindex fits i32");
        addr.sll_halen = 6;
        addr.sll_addr[..6].copy_from_slice(&dst);
        let mut header = [0u8; 14];
        header[..6].copy_from_slice(&dst);
        header[6..12].copy_from_slice(&src);
        header[12..].copy_from_slice(&[0x08, 0x00]);
        Self { fd, addr, header }
    }

    fn send_ip(&self, ip_packet: &[u8]) {
        let mut frame = Vec::with_capacity(14 + ip_packet.len());
        frame.extend_from_slice(&self.header);
        frame.extend_from_slice(ip_packet);
        // SAFETY: valid buffer and length; `addr` is a fully initialised
        // `sockaddr_ll` of the stated size.
        let n = unsafe {
            libc::sendto(
                self.fd.as_raw_fd(),
                frame.as_ptr().cast(),
                frame.len(),
                0,
                std::ptr::from_ref(&self.addr).cast(),
                libc::socklen_t::try_from(std::mem::size_of::<libc::sockaddr_ll>()).expect("size"),
            )
        };
        assert_eq!(
            usize::try_from(n).ok(),
            Some(frame.len()),
            "AF_PACKET send of a whole frame: {}",
            std::io::Error::last_os_error(),
        );
    }
}

/// Drain every datagram the peer has queued.
fn drain(peer: &UdpSocket) -> Vec<Vec<u8>> {
    let mut got = Vec::new();
    while let Some(d) = recv_datagram(peer) {
        got.push(d);
    }
    got
}

// ---------------------------------------------------------------------------
// Healthy baseline — a fragmented guest datagram is unframed exactly once
// over its own train. @contract-shape:bounded-change
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (host forwarder fragment map)"]
fn a_fragmented_guest_datagram_is_unframed_once_over_its_own_train() {
    let cgroup = OwnerCgroup::create("frag-baseline");
    let link = PeerLink::create("vsock-frag-baseline", 0);
    let forwarder = converged(&cgroup, &[&link]);
    require_vsock_loopback();
    let peer = link.peer_udp();
    let assoc = DatagramAssociation::install(&forwarder, v4(&peer));

    let before = counters(&forwarder);
    let payload = patterned(4000);
    assoc.guest_sends(&payload);
    assert_eq!(recv_datagram(&peer), Some(payload), "the fragmented datagram arrives byte-exact");

    // Pre-strip IPv4 payload = UDP header + frame header + payload.
    let after = counters(&forwarder);
    assert_eq!(
        after.unframe_fragment_shifted - before.unframe_fragment_shifted,
        later_fragments(8 + 8 + 4000),
        "every later fragment of the stripped train is moved back once",
    );
    assert_eq!(after.unframe_fragment_orphan, before.unframe_fragment_orphan, "no orphan fragment");
}

// ---------------------------------------------------------------------------
// S1 — a cut train, then an unrelated bare fragmented datagram reusing its
// reassembly key, arrives byte-exact and unshifted (a first fragment is
// authoritative for its key). @contract-shape:unbounded-preservation
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (host forwarder fragment map)"]
fn cut_train_then_unrelated_bare_fragmented_datagram_arrives_unshifted() {
    let cgroup = OwnerCgroup::create("frag-cut");
    let link = PeerLink::create("vsock-frag-cut", 0);
    let forwarder = converged(&cgroup, &[&link]);
    require_vsock_loopback();
    // The peer forgets the cut train's lone fragment after one second.
    link.peer_sysctl("net.ipv4.ipfrag_time", "1");
    let peer = link.peer_udp();
    let assoc = DatagramAssociation::install(&forwarder, v4(&peer));
    let tuple_src = assoc.host_local();
    let tuple_dst = v4(&peer);
    let injector = PacketInjector::on(&link);

    let before = counters(&forwarder);

    // GIVEN a stripped first fragment on the registered tuple whose train is
    // cut: no later fragment of it ever passes.
    let cut = udp_fragments(tuple_src, tuple_dst, ID, &frame(&patterned(3000)));
    injector.send_ip(&cut[0]);
    std::thread::sleep(Duration::from_secs(2));
    assert!(drain(&peer).is_empty(), "the cut train delivers nothing");

    // WHEN an unrelated bare (unframed) fragmented datagram reuses the key,
    let bare = patterned(2000);
    for fragment in udp_fragments(tuple_src, tuple_dst, ID, &bare) {
        injector.send_ip(&fragment);
    }

    // THEN it arrives byte-exact: its own unstripped first fragment removed the
    // stale record, so none of its later fragments was moved.
    assert_eq!(recv_datagram(&peer), Some(bare), "the bare datagram arrives unshifted");
    let after = counters(&forwarder);
    assert_eq!(
        after.unframe_fragment_shifted, before.unframe_fragment_shifted,
        "no later fragment of the bare datagram was moved back",
    );
}

// ---------------------------------------------------------------------------
// S2 — the map filled well beyond capacity with cut trains from several CPUs
// while framed fragmented datagrams flow: each arrives byte-exact or is lost
// whole (counted orphan), never corrupted.
// ---------------------------------------------------------------------------

/// Pin the calling thread to `cpu`.
fn pin_to_cpu(cpu: usize) {
    // SAFETY: a zeroed `cpu_set_t` is the empty set; `CPU_SET` writes within it.
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    // SAFETY: `cpu` is below the set's capacity (checked by the caller via
    // `available_parallelism`, far below CPU_SETSIZE).
    unsafe { libc::CPU_SET(cpu, &mut set) };
    // SAFETY: pid 0 = the calling thread; a valid set of its exact size.
    let rc = unsafe {
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &raw const set)
    };
    assert_eq!(rc, 0, "precondition: pin to CPU {cpu}: {}", std::io::Error::last_os_error());
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (host forwarder fragment map)"]
fn map_filled_beyond_capacity_loses_whole_datagrams_never_corrupts() {
    const MEASURED: u8 = 64;
    let cgroup = OwnerCgroup::create("frag-fill");
    let measured_link = PeerLink::create("vsock-frag-fill-a", 0);
    let flood_link = PeerLink::create("vsock-frag-fill-b", 1);
    let forwarder = converged(&cgroup, &[&measured_link, &flood_link]);
    require_vsock_loopback();

    let peer = measured_link.peer_udp();
    set_socket_rcvbuf(peer.as_raw_fd(), 8 << 20).expect("precondition: peer receive buffer");
    let assoc = DatagramAssociation::install(&forwarder, v4(&peer));
    // The flood runs on its own registered tuple (another link and peer), so
    // its reassembly keys never meet the measured datagrams at a peer.
    let flood_peer = flood_link.peer_udp();
    let flood_assoc = DatagramAssociation::install(&forwarder, v4(&flood_peer));
    let flood_src = flood_assoc.host_local();
    let flood_dst = v4(&flood_peer);

    let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get).min(4);
    let total = u64::from(GUEST_UNFRAME_FRAGMENTS_MAX) * 4;
    let per_cpu = total / cpus as u64;
    let cut_first = udp_fragments(flood_src, flood_dst, 0, &frame(&patterned(3000)))
        .into_iter()
        .next()
        .expect("a first fragment");

    let before = counters(&forwarder);
    let sent: Vec<Vec<u8>> = (0..MEASURED)
        .map(|seq| {
            let mut p = patterned(4000);
            p[0] = seq;
            p
        })
        .collect();

    std::thread::scope(|scope| {
        for cpu in 0..cpus {
            let flood_link = &flood_link;
            let cut_first = cut_first.clone();
            scope.spawn(move || {
                pin_to_cpu(cpu);
                let injector = PacketInjector::on(flood_link);
                let mut packet = cut_first;
                for k in 0..per_cpu {
                    // A distinct reassembly key per cut train.
                    let id = u16::try_from((cpu as u64 * per_cpu + k) % 65_536).expect("u16");
                    packet[4..6].copy_from_slice(&id.to_be_bytes());
                    packet[10..12].copy_from_slice(&[0, 0]);
                    let csum = ipv4_header_checksum(&packet[..20]);
                    packet[10..12].copy_from_slice(&csum.to_be_bytes());
                    injector.send_ip(&packet);
                }
            });
        }
        for payload in &sent {
            assoc.guest_sends(payload);
            std::thread::sleep(Duration::from_millis(2));
        }
    });

    let received = drain(&peer);
    let corrupted = received.iter().filter(|d| !sent.contains(d)).count();
    let lost = sent.iter().filter(|p| !received.contains(p)).count() as u64;
    let after = counters(&forwarder);

    assert_eq!(corrupted, 0, "no datagram is delivered corrupted");
    assert!(
        after.unframe_fragment_orphan - before.unframe_fragment_orphan >= lost,
        "each lost datagram lost its record (orphan count >= {lost})",
    );
    drop(flood_assoc);
}

// ---------------------------------------------------------------------------
// S3 — an ordinary congested qdisc dropping fragments after the program for
// three runs leaves every later datagram byte-exact.
// @contract-shape:unbounded-preservation
// ---------------------------------------------------------------------------

/// A root qdisc on an interface for the test's duration.
struct EgressQdisc {
    iface: String,
}

impl EgressQdisc {
    fn netem(iface: &str, spec: &[&str]) -> Self {
        let mut args = vec!["qdisc", "replace", "dev", iface, "root", "netem"];
        args.extend_from_slice(spec);
        run_ok("tc", &args);
        Self { iface: iface.to_owned() }
    }

    fn clear(self) {
        run_ok("tc", &["qdisc", "del", "dev", &self.iface, "root"]);
        std::mem::forget(self);
    }
}

impl Drop for EgressQdisc {
    fn drop(&mut self) {
        run_cleanup("tc", &["qdisc", "del", "dev", &self.iface, "root"]);
    }
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock new-step 4 (host forwarder fragment map)"]
fn congested_qdisc_dropping_fragments_leaves_later_datagrams_byte_exact() {
    let cgroup = OwnerCgroup::create("frag-qdisc");
    let link = PeerLink::create("vsock-frag-qdisc", 0);
    let forwarder = converged(&cgroup, &[&link]);
    require_vsock_loopback();
    let peer = link.peer_udp();
    let assoc = DatagramAssociation::install(&forwarder, v4(&peer));

    // GIVEN a qdisc that drops fragments after the program for three runs.
    let qdisc = EgressQdisc::netem(&link.host_if, &["delay", "20ms", "limit", "1"]);
    for run in 0..3u8 {
        for k in 0..5u8 {
            let mut p = patterned(4097);
            p[0] = run;
            p[1] = k;
            assoc.guest_sends(&p);
        }
        // Whatever survives the congestion is not the subject here.
        let _survivors = drain(&peer);
    }
    qdisc.clear();

    // THEN every later datagram arrives byte-exact.
    for k in 0..3u8 {
        let mut p = patterned(4096);
        p[0] = 0xF0 | k;
        assoc.guest_sends(&p);
        assert_eq!(
            recv_datagram(&peer),
            Some(p),
            "datagram {k} after the congestion is byte-exact"
        );
    }
}
