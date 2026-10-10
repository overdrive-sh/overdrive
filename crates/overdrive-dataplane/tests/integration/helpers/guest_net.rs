//! Real-kernel fixtures for the netns-density-295 guest-flow and
//! guest-steering adapter tests (`vsock_guest_steering`,
//! `vsock_egress_unframe`, `vsock_fragment_map`).
//!
//! Test-support surface only (DISTILL owns it): nothing here is production
//! API. Every fixture drives real kernel objects — a cgroup v2 scope, veth
//! pairs into a network namespace, AF_VSOCK loopback sockets, UDP sockets —
//! and tears down what it created on `Drop`.
//!
//! Addressing follows `.claude/rules/testing.md` § "Hand-built real-netns
//! fixtures must be route-disjoint from production": every address comes from
//! a named [`TestCidrLease`], never a CIDR literal.

#![allow(
    clippy::expect_used,
    clippy::doc_markdown,
    clippy::missing_panics_doc,
    clippy::print_stderr,
    clippy::unnecessary_debug_formatting
)]

use std::fs::{self, File, OpenOptions};
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::os::fd::{AsFd as _, AsRawFd as _, BorrowedFd, FromRawFd as _, OwnedFd};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use nix::sched::{CloneFlags, setns};
use nix::sys::socket::{
    AddressFamily, Backlog, MsgFlags, SockFlag, SockType, VsockAddr, accept, bind, connect,
    getsockname, listen, send, socket,
};
use overdrive_core::guest_transport::wire::{DATAGRAM_FRAME_MAGIC, MAX_GUEST_DATAGRAM_PAYLOAD};
use overdrive_core::net::InterfaceIndex;
use overdrive_dataplane::guest_flow::{FlowPairKey, GuestFlowForwarder};
use overdrive_testing::cidr_lease::TestCidrLease;
use overdrive_testing::netns::NetNs;

/// How long a receive waits before the fixture reports "nothing arrived".
pub const RECV_WAIT: Duration = Duration::from_secs(2);

/// Run a command to completion and panic, naming the precondition, unless it
/// exits 0.
pub fn run_ok(program: &str, args: &[&str]) {
    let out = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("precondition: `{program}` must be runnable: {e}"));
    assert!(
        out.status.success(),
        "precondition: `{program} {}` must succeed (status {:?}): {}",
        args.join(" "),
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).trim(),
    );
}

/// Run a command whose stdout is the result; panics unless it exits 0.
pub fn run_stdout(program: &str, args: &[&str]) -> String {
    let out = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("precondition: `{program}` must be runnable: {e}"));
    assert!(
        out.status.success(),
        "precondition: `{program} {}` must succeed (status {:?}): {}",
        args.join(" "),
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).trim(),
    );
    String::from_utf8(out.stdout).expect("command output is UTF-8")
}

/// Best-effort cleanup command for `Drop` impls: a destructor cannot report a
/// failure, and the next run's fixture pre-clean removes any leftover.
pub fn run_cleanup(program: &str, args: &[&str]) {
    if let Err(e) = Command::new(program).args(args).output() {
        eprintln!("cleanup `{program} {}` could not run: {e}", args.join(" "));
    }
}

/// A short per-process tag for kernel object names (IFNAMSIZ-bounded).
pub fn process_tag() -> String {
    format!("{:05x}", std::process::id() & 0xf_ffff)
}

/// The kernel interface index of `name` in the root namespace.
pub fn ifindex_of(name: &str) -> u32 {
    fs::read_to_string(format!("/sys/class/net/{name}/ifindex"))
        .unwrap_or_else(|e| panic!("precondition: interface {name} exists: {e}"))
        .trim()
        .parse()
        .expect("ifindex is decimal")
}

/// The pinned [`InterfaceIndex`] for a raw kernel index (decimal `FromStr`,
/// the only constructor the design pins).
pub fn interface_index(raw: u32) -> InterfaceIndex {
    raw.to_string().parse().expect("a kernel ifindex is a valid InterfaceIndex")
}

/// Run `f` on a thread that has entered network namespace `ns`. A socket
/// created inside `f` stays in that namespace after it is returned.
pub fn in_netns<T: Send + 'static>(ns: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let path = format!("/var/run/netns/{ns}");
    std::thread::spawn(move || {
        let handle = File::open(&path)
            .unwrap_or_else(|e| panic!("precondition: netns handle {path} opens: {e}"));
        setns(handle.as_fd(), CloneFlags::CLONE_NEWNET)
            .unwrap_or_else(|e| panic!("precondition: setns into {path}: {e}"));
        f()
    })
    .join()
    .expect("netns worker thread completes")
}

// ---------------------------------------------------------------------------
// Owner cgroup
// ---------------------------------------------------------------------------

/// A non-root cgroup v2 scope standing in for the owner's cgroup — the
/// `sock_ops` attach target `GuestFlowForwarder::load` requires (never the
/// root cgroup). Removed on `Drop`.
pub struct OwnerCgroup {
    path: PathBuf,
    dir: File,
}

impl OwnerCgroup {
    /// Create `/sys/fs/cgroup/overdrive.slice/guest-flow-<tag>.scope`.
    pub fn create(label: &str) -> Self {
        let parent = PathBuf::from("/sys/fs/cgroup/overdrive.slice");
        fs::create_dir_all(&parent)
            .unwrap_or_else(|e| panic!("precondition: cgroup v2 slice {parent:?} exists: {e}"));
        let path = parent.join(format!("guest-flow-{label}-{}.scope", std::process::id()));
        if path.exists() {
            fs::remove_dir(&path).unwrap_or_else(|e| {
                panic!("precondition: leftover owner cgroup {path:?} is removable: {e}")
            });
        }
        fs::create_dir(&path)
            .unwrap_or_else(|e| panic!("precondition: owner cgroup {path:?} is creatable: {e}"));
        let dir = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY)
            .open(&path)
            .unwrap_or_else(|e| panic!("precondition: owner cgroup {path:?} opens: {e}"));
        Self { path, dir }
    }

    /// The cgroup directory descriptor `load` takes.
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.dir.as_fd()
    }
}

impl Drop for OwnerCgroup {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_dir(&self.path) {
            eprintln!("cleanup: owner cgroup {:?} not removed: {e}", self.path);
        }
    }
}

/// The root cgroup's directory — the one `load` must refuse.
pub fn root_cgroup_dir() -> File {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY)
        .open("/sys/fs/cgroup")
        .expect("precondition: the cgroup v2 root opens")
}

// ---------------------------------------------------------------------------
// veth link into a peer namespace, addressed from a TestCidrLease
// ---------------------------------------------------------------------------

/// A veth pair whose host end stays in the root namespace (an interface the
/// boot attach converges) and whose peer end lives in its own namespace. The
/// host end holds the lease's gateway address, the peer end the lease's
/// workload address, both `/24`, MTU 1500.
pub struct PeerLink {
    /// Host-side interface name (root namespace).
    pub host_if: String,
    /// Peer-side interface name (inside [`Self::netns`]).
    pub peer_if: String,
    /// The peer namespace.
    pub netns: NetNs,
    /// Address of the host end.
    pub host_addr: Ipv4Addr,
    /// Address of the peer end.
    pub peer_addr: Ipv4Addr,
    _lease: TestCidrLease,
}

impl PeerLink {
    /// Create link `n` of this test process under the lease name `owner`.
    pub fn create(owner: &str, n: u8) -> Self {
        let lease = TestCidrLease::acquire(owner)
            .unwrap_or_else(|e| panic!("precondition: test CIDR lease {owner}: {e}"));
        let tag = process_tag();
        let host_if = format!("gfh{tag}{n}");
        let peer_if = format!("gfp{tag}{n}");
        let ns_name = format!("gf-{}-{n}", std::process::id());
        run_cleanup("ip", &["link", "del", &host_if]);
        let netns = NetNs::create(&ns_name)
            .unwrap_or_else(|e| panic!("precondition: netns {ns_name} is creatable: {e}"));
        run_ok("ip", &["link", "add", &host_if, "type", "veth", "peer", "name", &peer_if]);
        run_ok("ip", &["link", "set", &peer_if, "netns", &ns_name]);
        let prefix = lease.prefix_len();
        let host_addr = lease.host_gateway();
        let peer_addr = lease.workload_addr();
        run_ok("ip", &["addr", "add", &format!("{host_addr}/{prefix}"), "dev", &host_if]);
        run_ok("ip", &["link", "set", &host_if, "mtu", "1500", "up"]);
        netns
            .assign_ip_and_up(&peer_if, &format!("{peer_addr}/{prefix}"))
            .unwrap_or_else(|e| panic!("precondition: peer address on {peer_if}: {e}"));
        run_ok("ip", &["netns", "exec", &ns_name, "ip", "link", "set", &peer_if, "mtu", "1500"]);
        Self { host_if, peer_if, netns, host_addr, peer_addr, _lease: lease }
    }

    /// The host end's kernel interface index.
    pub fn host_ifindex(&self) -> u32 {
        ifindex_of(&self.host_if)
    }

    /// Add a route inside the peer namespace towards `cidr` via the host end.
    pub fn route_from_peer(&self, cidr: &str) {
        self.netns
            .add_route(cidr, Some(&self.host_addr.to_string()), None)
            .unwrap_or_else(|e| panic!("precondition: peer route to {cidr}: {e}"));
    }

    /// Set a sysctl inside the peer namespace.
    pub fn peer_sysctl(&self, key: &str, value: &str) {
        self.netns
            .sysctl(key, value)
            .unwrap_or_else(|e| panic!("precondition: peer sysctl {key}={value}: {e}"));
    }

    /// The MAC address of the peer end (read inside its namespace).
    pub fn peer_mac(&self) -> [u8; 6] {
        let text = run_stdout(
            "ip",
            &[
                "netns",
                "exec",
                &self.netns.name,
                "cat",
                &format!("/sys/class/net/{}/address", self.peer_if),
            ],
        );
        parse_mac(text.trim())
    }

    /// The MAC address of the host end.
    pub fn host_mac(&self) -> [u8; 6] {
        let text = fs::read_to_string(format!("/sys/class/net/{}/address", self.host_if))
            .expect("precondition: host end MAC is readable");
        parse_mac(text.trim())
    }

    /// A UDP socket bound inside the peer namespace on the peer address
    /// (ephemeral port), with a receive timeout of [`RECV_WAIT`].
    pub fn peer_udp(&self) -> UdpSocket {
        let addr = SocketAddrV4::new(self.peer_addr, 0);
        let sock = in_netns(&self.netns.name, move || {
            UdpSocket::bind(addr).expect("precondition: peer UDP socket binds in its namespace")
        });
        sock.set_read_timeout(Some(RECV_WAIT)).expect("peer UDP read timeout");
        sock
    }
}

impl Drop for PeerLink {
    fn drop(&mut self) {
        // Deleting the host end removes the pair; the NetNs and the lease drop
        // after this body, in field order, once no route of the lease remains.
        run_cleanup("ip", &["link", "del", &self.host_if]);
    }
}

fn parse_mac(text: &str) -> [u8; 6] {
    let mut mac = [0u8; 6];
    for (slot, part) in mac.iter_mut().zip(text.split(':')) {
        *slot = u8::from_str_radix(part, 16).expect("MAC octet is hex");
    }
    mac
}

/// Receive one datagram, or `None` when nothing arrives within the socket's
/// read timeout.
pub fn recv_datagram(sock: &UdpSocket) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; 65_536];
    match sock.recv(&mut buf) {
        Ok(n) => {
            buf.truncate(n);
            Some(buf)
        }
        Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => None,
        Err(e) => panic!("peer UDP receive failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Datagram frames and associations
// ---------------------------------------------------------------------------

/// The guest-side framing of one datagram: `DATAGRAM_FRAME_MAGIC`, the BE
/// `u32` logical payload length, the payload.
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let len = u32::try_from(payload.len()).expect("payload length fits u32");
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&DATAGRAM_FRAME_MAGIC);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// One connected AF_VSOCK loopback pair (CID 1): the connecting end plays the
/// guest's socket, the accepted end the host's.
pub struct VsockLoopbackPair {
    /// The connecting end (the guest's T_g / Q_g).
    pub guest: OwnedFd,
    /// The accepted end (the host's T_h / Q_h).
    pub host: OwnedFd,
}

impl VsockLoopbackPair {
    /// Connect a pair of type `ty` (`Stream` or `SeqPacket`).
    pub fn connect(ty: SockType) -> Self {
        let listener = socket(AddressFamily::Vsock, ty, SockFlag::SOCK_CLOEXEC, None)
            .expect("precondition: AF_VSOCK is available (vsock_loopback loaded)");
        bind(listener.as_raw_fd(), &VsockAddr::new(libc::VMADDR_CID_ANY, libc::VMADDR_PORT_ANY))
            .expect("precondition: AF_VSOCK listener binds");
        listen(&listener, Backlog::new(4).expect("backlog 4 is valid"))
            .expect("precondition: AF_VSOCK listener listens");
        let port = getsockname::<VsockAddr>(listener.as_raw_fd())
            .expect("precondition: AF_VSOCK listener has a port")
            .port();
        let guest = socket(AddressFamily::Vsock, ty, SockFlag::SOCK_CLOEXEC, None)
            .expect("precondition: AF_VSOCK client socket");
        connect(guest.as_raw_fd(), &VsockAddr::new(libc::VMADDR_CID_LOCAL, port))
            .expect("precondition: AF_VSOCK loopback connect (vsock_loopback module)");
        let raw = accept(listener.as_raw_fd()).expect("precondition: AF_VSOCK accept");
        // SAFETY: `accept` returned a fresh descriptor this process owns and
        // nothing else holds.
        let host = unsafe { OwnedFd::from_raw_fd(raw) };
        Self { guest, host }
    }
}

/// Load `vsock_loopback` so AF_VSOCK CID 1 connects within the host.
pub fn require_vsock_loopback() {
    run_ok("modprobe", &["vsock_loopback"]);
}

/// A connected IPv4 UDP socket with `SO_NO_CHECK` — the host socket H an
/// association forwards through (`install_datagram`'s `host_udp`).
pub fn host_udp_to(peer: SocketAddrV4) -> UdpSocket {
    let sock = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        .expect("precondition: host UDP socket binds");
    let one: libc::c_int = 1;
    // SAFETY: a valid socket descriptor, a valid `c_int` option value and its
    // exact length.
    let rc = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_NO_CHECK,
            std::ptr::from_ref(&one).cast(),
            libc::socklen_t::try_from(std::mem::size_of::<libc::c_int>()).expect("c_int size"),
        )
    };
    assert_eq!(rc, 0, "precondition: SO_NO_CHECK sets: {}", io::Error::last_os_error());
    sock.connect(peer).expect("precondition: host UDP socket connects to its peer");
    sock
}

/// One guest datagram association installed in the forwarder: the guest's
/// T_g (stream, toward the guest) and Q_g (seqpacket, from the guest), the
/// host's accepted ends, and the host UDP socket H.
pub struct DatagramAssociation {
    /// The forwarder's handle for the pair (held for the association's life).
    _key: FlowPairKey,
    /// Guest stream T_g / host stream T_h (held open for the association's life).
    _stream: VsockLoopbackPair,
    /// Guest seqpacket Q_g / host seqpacket Q_h.
    pub seqpacket: VsockLoopbackPair,
    /// The host UDP socket H.
    pub host_udp: UdpSocket,
}

impl DatagramAssociation {
    /// Open the vsock legs and H, and install them (`install_datagram`). No
    /// request bytes are pending on either host leg: the precondition that
    /// the owner has read the requests holds.
    pub fn install(forwarder: &GuestFlowForwarder, peer: SocketAddrV4) -> Self {
        let stream = VsockLoopbackPair::connect(SockType::Stream);
        let seqpacket = VsockLoopbackPair::connect(SockType::SeqPacket);
        let host_udp = host_udp_to(peer);
        let key = forwarder
            .install_datagram(stream.host.as_fd(), seqpacket.host.as_fd(), host_udp.as_fd())
            .expect(
                "install_datagram accepts a connected stream, seqpacket and SO_NO_CHECK UDP socket",
            );
        Self { _key: key, _stream: stream, seqpacket, host_udp }
    }

    /// The guest writes one framed datagram as one seqpacket record on Q_g.
    pub fn guest_sends_record(&self, record: &[u8]) {
        let n = send(self.seqpacket.guest.as_raw_fd(), record, MsgFlags::empty())
            .expect("guest seqpacket send");
        assert_eq!(n, record.len(), "a seqpacket record is sent whole");
    }

    /// The guest sends `payload` as one framed datagram.
    pub fn guest_sends(&self, payload: &[u8]) {
        self.guest_sends_record(&frame(payload));
    }

    /// The host socket H's kernel-reported local address.
    pub fn host_local(&self) -> SocketAddrV4 {
        match self.host_udp.local_addr().expect("H has a local address") {
            std::net::SocketAddr::V4(a) => a,
            std::net::SocketAddr::V6(a) => panic!("H is IPv4, got {a}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Datagram size classes (P5-S1 / P5-S3)
// ---------------------------------------------------------------------------

/// A guest datagram shape and the application payload the peer must receive.
pub struct DatagramCase {
    /// Label for assertion messages.
    pub label: &'static str,
    /// The application payload.
    pub payload: Vec<u8>,
}

/// Every size class the egress unframe must carry byte-exact: empty, one
/// byte, payloads that themselves parse as a frame (the verdict keeps the
/// frame; the egress program strips it), a near-miss that does not, the
/// 1500-MTU boundary (a framed 1464 B payload fills 1500 B exactly; 1465 B is
/// fragmented before the program strips it), a multi-fragment payload and the
/// largest guest datagram.
pub fn datagram_cases() -> Vec<DatagramCase> {
    let mut near_miss = DATAGRAM_FRAME_MAGIC.to_vec();
    near_miss.extend_from_slice(&99u32.to_be_bytes());
    near_miss.extend_from_slice(b"short");
    let max = usize::try_from(MAX_GUEST_DATAGRAM_PAYLOAD).expect("u32 fits usize");
    vec![
        DatagramCase { label: "empty", payload: Vec::new() },
        DatagramCase { label: "one-byte", payload: vec![0xAB] },
        DatagramCase { label: "frame-shaped-empty", payload: frame(&[]) },
        DatagramCase { label: "frame-shaped-hello", payload: frame(b"hello") },
        DatagramCase { label: "near-miss", payload: near_miss },
        DatagramCase { label: "mtu-fill-1464", payload: patterned(1464) },
        DatagramCase { label: "mtu-over-1465", payload: patterned(1465) },
        DatagramCase { label: "fragmented-4000", payload: patterned(4000) },
        DatagramCase { label: "max-payload", payload: patterned(max) },
    ]
}

/// A payload whose bytes encode their offset, so a shifted or truncated
/// fragment shows up as a byte difference.
pub fn patterned(len: usize) -> Vec<u8> {
    (0..len).map(|i| u8::try_from(i % 251).expect("i % 251 < 256")).collect()
}

/// Send every case through `assoc` from the guest and require the peer to
/// receive each payload byte-exact, in order.
pub fn assert_cases_arrive_byte_exact(
    assoc: &DatagramAssociation,
    peer: &UdpSocket,
    through: &str,
) {
    for case in datagram_cases() {
        assoc.guest_sends(&case.payload);
        let got = recv_datagram(peer)
            .unwrap_or_else(|| panic!("{} through {through}: nothing arrived", case.label));
        assert_eq!(got.len(), case.payload.len(), "{} through {through}: length", case.label);
        assert!(got == case.payload, "{} through {through}: bytes differ", case.label);
    }
}
