//! The guest application of the native-metal microVM lanes (GH #295, T-1):
//! the protocol the in-guest program `overdrive-guest-peer` speaks when it
//! runs as a VM workload's operator command, and the remote peer it talks to.
//!
//! A VM deployed through `overdrive serve` + `overdrive deploy` runs the
//! production `overdrive-init` as PID 1; its operator command is the
//! `overdrive-guest-peer` binary of this crate (staged at
//! [`GUEST_PEER_GUEST_PATH`] in every guest-kit image, `guest_kit`), invoked
//! with one of the [`subcommand`]s. The program is an ordinary, identity-
//! unaware guest application: it opens plain TCP and UDP sockets to
//! addresses outside the guest, and the guest kit's transport carries them.
//! It never speaks vsock or TLS itself.
//!
//! The far side is a [`RemotePeer`]: TCP and UDP servers in a network
//! namespace of their own, reached from the node's root namespace over a
//! veth pair on a leased test CIDR (`cidr_lease`, disjoint from the
//! production workload subnet and from the guest prefix). The peer is
//! therefore an off-host destination from the node's point of view: the
//! host's forwarded connection leaves the root namespace with the host-side
//! veth address as its source, which is the oracle that the bytes crossed
//! the host's forwarder and were not routed from the guest.
//!
//! Everything both ends must agree on — the subcommand names, the request
//! and response records, the deterministic byte streams, the exit codes and
//! the console lines the program prints — is defined once here; the binary
//! (`bin/guest_peer.rs`) and the peer import it.
//!
//! **Observation channel.** The program's stdout and stderr are the guest's
//! console (`overdrive-init` forwards stdio untouched), which the VMM writes
//! to the allocation's console log. Tests read that log for the lines this
//! module names ([`TERMINAL_LINE_PREFIX`], [`FLOWING_LINE`],
//! [`FLOOD_WRITTEN_PREFIX`]); a Job's exit code reaches the operator through
//! `overdrive workload describe` as for any VM workload.

#![allow(clippy::doc_markdown, reason = "kernel and protocol names in prose")]
#![allow(
    clippy::too_long_first_doc_paragraph,
    reason = "protocol docs open with the full sentence they define"
)]

use std::fmt;
use std::io;
use std::str::FromStr;

/// Where every guest-kit image carries the guest program.
pub const GUEST_PEER_GUEST_PATH: &str = "/sbin/overdrive-guest-peer";

/// The program's subcommands (its `argv[1]`).
pub mod subcommand {
    /// `round-trip <tcp ip:port> <udp ip:port> <tag>`: one TCP request and
    /// response, then one UDP datagram and its reply; exits [`super::exit::OK`]
    /// when both came back byte-exact.
    pub const ROUND_TRIP: &str = "round-trip";
    /// `hold <tcp ip:port> <tag> [crash-after=<bytes>]`: opens one TCP flow
    /// and moves the [`super::StreamGen`] streams both ways until the flow
    /// ends; prints [`super::FLOWING_LINE`] once both directions have moved
    /// [`super::HOLD_FLOWING_BYTES`], and one terminal line when its read side
    /// ends. With `crash-after`, it crashes the guest kernel once both
    /// directions moved that many bytes (`/proc/sysrq-trigger`, the guest
    /// reset). It never exits.
    pub const HOLD: &str = "hold";
    /// `flood <tcp ip:port> <tag>`: writes its stream as fast as the flow
    /// accepts it and prints [`super::FLOOD_WRITTEN_PREFIX`] lines every
    /// [`super::FLOOD_REPORT_PERIOD_MS`]. It never exits.
    pub const FLOOD: &str = "flood";
    /// `udp-churn <udp ip:port> <count> <tag>`: `count` times opens a UDP
    /// socket, sends one datagram and closes it at once; then one datagram
    /// round trip on a fresh socket. Exits [`super::exit::OK`] when every send
    /// succeeded and the reply came back.
    pub const UDP_CHURN: &str = "udp-churn";
    /// `idle`: sleeps forever (a VM that only needs to be running).
    pub const IDLE: &str = "idle";
    /// `listen <port> <tag>`: the server side of a Service VM (U-D17-DIAL).
    /// Listens on TCP `0.0.0.0:<port>` and accepts connections until killed.
    /// Each connection is served on its own thread: when its first line is a
    /// [`super::hold_hello`], the program answers with the hello's
    /// [`super::Direction::PeerToGuest`] stream and verifies the client's
    /// [`super::Direction::GuestToPeer`] stream (the `hold` protocol, so a
    /// `hold` workload can be its client); any other connection (a probe that
    /// connects and drops) is only read. Every connection ends with one
    /// [`super::LISTEN_CONN_TERMINAL_PREFIX`] line naming its read side's
    /// first terminal event. It never exits.
    pub const LISTEN: &str = "listen";
    /// `run-with-modules <module.ko>... -- <program> <args>...`: loads each
    /// module with `finit_module` (already loaded is success), then execs
    /// the program. The in-guest test lane's launcher (`guest_lane`).
    pub const RUN_WITH_MODULES: &str = "run-with-modules";
}

/// The program's exit codes.
pub mod exit {
    /// Every check passed.
    pub const OK: i32 = 0;
    /// The TCP connect failed.
    pub const TCP_CONNECT: i32 = 10;
    /// A TCP read or write failed.
    pub const TCP_IO: i32 = 11;
    /// The TCP response was not the expected bytes.
    pub const TCP_MISMATCH: i32 = 12;
    /// A UDP socket operation failed.
    pub const UDP_IO: i32 = 20;
    /// No UDP reply within [`super::UDP_REPLY_TIMEOUT_MS`].
    pub const UDP_TIMEOUT: i32 = 21;
    /// The UDP reply was not the expected bytes.
    pub const UDP_MISMATCH: i32 = 22;
    /// A churn send failed (the guest refused a datagram: a slot not
    /// released).
    pub const CHURN_SEND: i32 = 30;
    /// The arguments did not parse.
    pub const USAGE: i32 = 64;
    /// A module failed to load.
    pub const MODULE_LOAD: i32 = 65;
    /// The launched program could not be executed.
    pub const EXEC: i32 = 66;
}

/// How long the guest waits for a UDP reply.
pub const UDP_REPLY_TIMEOUT_MS: u64 = 5_000;
/// Bytes each direction of a `hold` flow moves before [`FLOWING_LINE`].
pub const HOLD_FLOWING_BYTES: u64 = 64 * 1024;
/// How often `flood` reports its written count.
pub const FLOOD_REPORT_PERIOD_MS: u64 = 500;
/// The chunk size of every stream write.
pub const STREAM_CHUNK: usize = 4096;

/// Printed once by `hold` when both directions moved [`HOLD_FLOWING_BYTES`].
pub const FLOWING_LINE: &str = "hold: flowing";
/// Prefix of the one terminal line `hold` and `flood` print: the read side's
/// first terminal event, rendered by [`TerminalEvent`]'s `Display`.
pub const TERMINAL_LINE_PREFIX: &str = "observer guest-app: terminal=";
/// Prefix of `flood`'s periodic report, followed by the decimal byte count.
pub const FLOOD_WRITTEN_PREFIX: &str = "flood: written=";
/// Printed by `listen` once its listener is bound, followed by the port.
pub const LISTENING_PREFIX: &str = "listen: listening port=";
/// Prefix of the line `listen` prints when one connection's read side ends:
/// `listen: conn=<n> hello=<yes|no> terminal=<event>`, `n` counting accepts
/// from 1, the event rendered by [`TerminalEvent`]'s `Display`.
pub const LISTEN_CONN_TERMINAL_PREFIX: &str = "listen: conn=";

/// The TCP port and the UDP port every [`RemotePeer`] serves.
pub const PEER_TCP_PORT: u16 = 7000;
/// See [`PEER_TCP_PORT`].
pub const PEER_UDP_PORT: u16 = 7001;

/// A tag is what tells the records of one VM from another's: ASCII
/// alphanumerics and `-`, 1..=48 bytes.
#[must_use]
pub fn tag_is_valid(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 48
        && tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The TCP request `round-trip` sends.
#[must_use]
pub fn tcp_request(tag: &str) -> Vec<u8> {
    format!("REQ:{tag}\n").into_bytes()
}

/// The TCP response the peer returns to [`tcp_request`]. Byte-distinct from
/// the request, so a correct reply proves the server→client pipe, not an
/// echo.
#[must_use]
pub fn tcp_response(tag: &str) -> Vec<u8> {
    format!("RSP:{tag}:accepted-by-remote-peer\n").into_bytes()
}

/// The first line of a `hold` flow.
#[must_use]
pub fn hold_hello(tag: &str) -> Vec<u8> {
    format!("HOLD:{tag}\n").into_bytes()
}

/// The first line of a `flood` flow.
#[must_use]
pub fn flood_hello(tag: &str) -> Vec<u8> {
    format!("FLOOD:{tag}\n").into_bytes()
}

/// The datagram `round-trip` and `udp-churn` send last.
#[must_use]
pub fn datagram_request(tag: &str) -> Vec<u8> {
    format!("DGRAM-REQ:{tag}").into_bytes()
}

/// The peer's reply to [`datagram_request`] (byte-distinct).
#[must_use]
pub fn datagram_response(tag: &str) -> Vec<u8> {
    format!("DGRAM-RSP:{tag}:from-remote-peer").into_bytes()
}

/// One churn datagram.
#[must_use]
pub fn churn_datagram(tag: &str, index: u32) -> Vec<u8> {
    format!("CHURN:{tag}:{index}").into_bytes()
}

/// The two directions of a stream flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Written by the guest application, read by the peer.
    GuestToPeer,
    /// Written by the peer, read by the guest application.
    PeerToGuest,
}

/// A deterministic byte stream (xorshift64*), seeded by a tag and a
/// direction, so both ends can produce and verify the same bytes without
/// shipping them.
#[derive(Debug, Clone)]
pub struct StreamGen {
    state: u64,
    pending: [u8; 8],
    pending_len: usize,
}

impl StreamGen {
    /// The stream of `tag` in `direction`.
    #[must_use]
    pub fn new(tag: &str, direction: Direction) -> Self {
        let mut seed: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in tag.bytes().chain(std::iter::once(match direction {
            Direction::GuestToPeer => b'>',
            Direction::PeerToGuest => b'<',
        })) {
            seed ^= u64::from(byte);
            seed = seed.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Self { state: seed | 1, pending: [0; 8], pending_len: 0 }
    }

    const fn next_word(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Fills `buf` with the next bytes of the stream.
    pub fn fill(&mut self, buf: &mut [u8]) {
        for byte in buf.iter_mut() {
            if self.pending_len == 0 {
                self.pending = self.next_word().to_le_bytes();
                self.pending_len = 8;
            }
            *byte = self.pending[8 - self.pending_len];
            self.pending_len -= 1;
        }
    }
}

/// Verifies received bytes against a [`StreamGen`].
#[derive(Debug, Clone)]
pub struct StreamCheck {
    expected: StreamGen,
    verified: u64,
}

impl StreamCheck {
    /// Checks the stream of `tag` in `direction` from its first byte.
    #[must_use]
    pub fn new(tag: &str, direction: Direction) -> Self {
        Self { expected: StreamGen::new(tag, direction), verified: 0 }
    }

    /// Checks the next `chunk`.
    ///
    /// # Errors
    ///
    /// The stream offset of the first byte that differs.
    pub fn check(&mut self, chunk: &[u8]) -> Result<(), u64> {
        let mut want = vec![0u8; chunk.len()];
        self.expected.fill(&mut want);
        if let Some(at) = chunk.iter().zip(&want).position(|(got, want)| got != want) {
            return Err(self.verified + at as u64);
        }
        self.verified += chunk.len() as u64;
        Ok(())
    }

    /// Bytes verified so far.
    #[must_use]
    pub const fn verified(&self) -> u64 {
        self.verified
    }
}

/// The first terminal event of a stream's read side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalEvent {
    /// A clean end of stream (`read` returned 0).
    Eof,
    /// `read` failed `ECONNRESET`.
    Reset,
    /// `read` failed with another errno.
    Error(i32),
}

impl TerminalEvent {
    /// The event a failed `read` reports.
    #[must_use]
    pub fn from_read_error(error: &io::Error) -> Self {
        match error.raw_os_error() {
            Some(code) if code == ECONNRESET => Self::Reset,
            Some(code) => Self::Error(code),
            None if error.kind() == io::ErrorKind::ConnectionReset => Self::Reset,
            None => Self::Error(-1),
        }
    }
}

/// `ECONNRESET` on Linux (`include/uapi/asm-generic/errno.h:84 @ v7.0`).
pub const ECONNRESET: i32 = 104;

impl fmt::Display for TerminalEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eof => f.write_str("eof"),
            Self::Reset => f.write_str("reset"),
            Self::Error(code) => write!(f, "error:{code}"),
        }
    }
}

impl FromStr for TerminalEvent {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "eof" => Ok(Self::Eof),
            "reset" => Ok(Self::Reset),
            other => other
                .strip_prefix("error:")
                .and_then(|code| code.parse().ok())
                .map(Self::Error)
                .ok_or_else(|| format!("not a terminal event: {other:?}")),
        }
    }
}

/// The terminal event a console log reports for the guest application, if
/// any: the first line starting with [`TERMINAL_LINE_PREFIX`].
#[must_use]
pub fn terminal_event_in_console(console: &str) -> Option<TerminalEvent> {
    console.lines().find_map(|line| {
        line.find(TERMINAL_LINE_PREFIX)
            .and_then(|at| line[at + TERMINAL_LINE_PREFIX.len()..].trim().parse().ok())
    })
}

/// One connection `listen` served, as its console line reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListenConnEnd {
    /// The connection's accept ordinal, from 1.
    pub conn: u64,
    /// Its first line was a [`hold_hello`].
    pub hello: bool,
    /// Its read side's first terminal event.
    pub terminal: TerminalEvent,
}

impl fmt::Display for ListenConnEnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{LISTEN_CONN_TERMINAL_PREFIX}{} hello={} terminal={}",
            self.conn,
            if self.hello { "yes" } else { "no" },
            self.terminal
        )
    }
}

impl FromStr for ListenConnEnd {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s
            .strip_prefix(LISTEN_CONN_TERMINAL_PREFIX)
            .ok_or_else(|| format!("not a listen connection line: {s:?}"))?;
        let mut words = rest.split(' ');
        let conn = words
            .next()
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| format!("no connection ordinal in {s:?}"))?;
        let hello = match words.next() {
            Some("hello=yes") => true,
            Some("hello=no") => false,
            _ => return Err(format!("no hello field in {s:?}")),
        };
        let terminal = words
            .next()
            .and_then(|w| w.strip_prefix("terminal="))
            .ok_or_else(|| format!("no terminal field in {s:?}"))?
            .parse()?;
        if words.next().is_some() {
            return Err(format!("trailing fields in {s:?}"));
        }
        Ok(Self { conn, hello, terminal })
    }
}

/// Every connection end a `listen` console log reports, in order.
#[must_use]
pub fn listen_conn_ends_in_console(console: &str) -> Vec<ListenConnEnd> {
    console
        .lines()
        .filter_map(|line| {
            line.find(LISTEN_CONN_TERMINAL_PREFIX).and_then(|at| line[at..].trim().parse().ok())
        })
        .collect()
}

/// Every `flood` written count a console log reports, in order.
#[must_use]
pub fn flood_written_in_console(console: &str) -> Vec<u64> {
    console
        .lines()
        .filter_map(|line| {
            line.find(FLOOD_WRITTEN_PREFIX)
                .and_then(|at| line[at + FLOOD_WRITTEN_PREFIX.len()..].trim().parse().ok())
        })
        .collect()
}

#[cfg(target_os = "linux")]
pub use remote_peer::{
    FloodObservation, HoldObservation, PeerLog, RemotePeer, RemotePeerError, ServedDatagram,
    ServedRequest,
};

#[cfg(target_os = "linux")]
mod remote_peer {
    use std::collections::BTreeMap;
    use std::io::{self, Read, Write};
    use std::net::{
        Ipv4Addr, Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket,
    };
    use std::os::fd::AsRawFd;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use super::{
        Direction, PEER_TCP_PORT, PEER_UDP_PORT, STREAM_CHUNK, StreamCheck, StreamGen,
        TerminalEvent, datagram_response, tcp_response,
    };
    use crate::cidr_lease::{CidrLeaseError, TestCidrLease};
    use crate::netns::{NetNs, NetNsError};

    /// Why a [`RemotePeer`] could not start.
    #[derive(Debug, thiserror::Error)]
    pub enum RemotePeerError {
        /// No test CIDR could be leased.
        #[error("leasing a test CIDR for the remote peer: {0}")]
        Lease(#[from] CidrLeaseError),
        /// The namespace or its veth pair could not be set up.
        #[error("setting up the remote peer's namespace: {0}")]
        Namespace(#[from] NetNsError),
        /// A socket of the peer could not be created inside its namespace.
        #[error("creating the remote peer's {what}: {source}")]
        Socket {
            /// Which socket.
            what: &'static str,
            /// The error.
            #[source]
            source: io::Error,
        },
    }

    /// One served TCP request.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ServedRequest {
        /// The request's tag.
        pub tag: String,
        /// The connection's source address as the peer saw it.
        pub from: SocketAddrV4,
    }

    /// One served datagram request.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ServedDatagram {
        /// The datagram's tag.
        pub tag: String,
        /// Its source address.
        pub from: SocketAddrV4,
    }

    /// What the peer saw of one `hold` flow.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct HoldObservation {
        /// The connection's source address.
        pub from: SocketAddrV4,
        /// Bytes of the guest's stream verified.
        pub received: u64,
        /// Bytes of the peer's stream written.
        pub sent: u64,
        /// The read side's first terminal event, once it happened.
        pub terminal: Option<TerminalEvent>,
        /// The stream offset of a corrupted byte, if one arrived.
        pub corrupted_at: Option<u64>,
    }

    /// What the peer saw of one `flood` flow.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct FloodObservation {
        /// The connection's source address.
        pub from: SocketAddrV4,
        /// Bytes of the guest's stream verified.
        pub verified: u64,
        /// The stream offset of a corrupted byte, if one arrived.
        pub corrupted_at: Option<u64>,
        /// The read side's terminal event, once it happened.
        pub terminal: Option<TerminalEvent>,
    }

    /// Everything the peer served.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct PeerLog {
        /// Served TCP requests, in order.
        pub requests: Vec<ServedRequest>,
        /// Served datagram requests, in order.
        pub datagrams: Vec<ServedDatagram>,
        /// Churn datagrams received, per tag.
        pub churn: BTreeMap<String, u32>,
        /// `hold` flows, keyed by tag; a later connection with a tag already
        /// seen (a restarted VM's) is keyed `<tag>#<n>`, n = 2, 3, ….
        pub holds: BTreeMap<String, HoldObservation>,
        /// `flood` flows, keyed as `holds`.
        pub floods: BTreeMap<String, FloodObservation>,
    }

    struct Shared {
        log: Mutex<PeerLog>,
        stop: AtomicBool,
        floods_stalled: AtomicBool,
    }

    /// TCP and UDP servers in a namespace of their own, reached from the
    /// node's root namespace over a veth pair on a leased test `/24`.
    ///
    /// Root side: the lease's host gateway on `<if>h`. Peer side: the lease's
    /// workload address on `<if>p`, with a default route back through the
    /// host gateway. Servers: [`PEER_TCP_PORT`] and [`PEER_UDP_PORT`] on the
    /// peer address. `Drop` stops the servers, deletes the veth pair and the
    /// namespace, then releases the lease.
    pub struct RemotePeer {
        shared: Arc<Shared>,
        threads: Vec<JoinHandle<()>>,
        host_if: String,
        // Dropped after the veth (field order): the namespace, then the lease.
        _ns: NetNs,
        lease: TestCidrLease,
    }

    impl RemotePeer {
        /// Starts a peer for `owner` (a stable, module-unique name; it names
        /// the lease). `index` (0..=99) makes the namespace and interface
        /// names unique among the peers one test file runs at once.
        ///
        /// # Errors
        ///
        /// [`RemotePeerError`] naming the step that failed; a partially built
        /// peer is torn down before the error returns.
        pub fn start(owner: &str, index: u8) -> Result<Self, RemotePeerError> {
            let lease = TestCidrLease::acquire(&format!("remote-peer-{owner}-{index}"))?;
            let ns_name = format!("ovd-rp-{index:02}");
            let host_if = format!("ovdrp{index:02}h");
            let peer_if = format!("ovdrp{index:02}p");
            let ns = NetNs::create(&ns_name)?;
            let peer = lease.workload_addr();
            let gateway = lease.host_gateway();
            let prefix = lease.prefix_len();
            // A veth left by an interrupted run is removed first.
            let _ = Command::new("ip").args(["link", "del", &host_if]).output();
            ip(&["link", "add", &host_if, "type", "veth", "peer", "name", &peer_if])?;
            let built = (|| -> Result<(), RemotePeerError> {
                ip(&["link", "set", &peer_if, "netns", &ns_name])?;
                ip(&["addr", "add", &format!("{gateway}/{prefix}"), "dev", &host_if])?;
                ip(&["link", "set", &host_if, "up"])?;
                ns.assign_ip_and_up(&peer_if, &format!("{peer}/{prefix}"))?;
                ns.add_route("default", Some(&gateway.to_string()), None)?;
                Ok(())
            })();
            if let Err(error) = built {
                let _ = Command::new("ip").args(["link", "del", &host_if]).output();
                return Err(error);
            }
            let (tcp, udp) = match in_netns(&ns_name, move || {
                let tcp = TcpListener::bind(SocketAddrV4::new(peer, PEER_TCP_PORT))
                    .map_err(|source| RemotePeerError::Socket { what: "TCP listener", source })?;
                let udp = UdpSocket::bind(SocketAddrV4::new(peer, PEER_UDP_PORT))
                    .map_err(|source| RemotePeerError::Socket { what: "UDP socket", source })?;
                Ok((tcp, udp))
            }) {
                Ok(sockets) => sockets,
                Err(error) => {
                    let _ = Command::new("ip").args(["link", "del", &host_if]).output();
                    return Err(error);
                }
            };
            let shared = Arc::new(Shared {
                log: Mutex::new(PeerLog::default()),
                stop: AtomicBool::new(false),
                floods_stalled: AtomicBool::new(false),
            });
            let threads = vec![
                spawn_named("remote-peer-tcp", {
                    let shared = Arc::clone(&shared);
                    move || serve_tcp(&tcp, &shared)
                }),
                spawn_named("remote-peer-udp", {
                    let shared = Arc::clone(&shared);
                    move || serve_udp(&udp, &shared)
                }),
            ];
            Ok(Self { shared, threads, host_if, _ns: ns, lease })
        }

        /// The peer's TCP address.
        #[must_use]
        pub const fn tcp_addr(&self) -> SocketAddrV4 {
            SocketAddrV4::new(self.lease.workload_addr(), PEER_TCP_PORT)
        }

        /// The peer's UDP address.
        #[must_use]
        pub const fn udp_addr(&self) -> SocketAddrV4 {
            SocketAddrV4::new(self.lease.workload_addr(), PEER_UDP_PORT)
        }

        /// The root namespace's address on the veth: the source address of
        /// every connection the node opens to the peer.
        #[must_use]
        pub const fn node_side_addr(&self) -> Ipv4Addr {
            self.lease.host_gateway()
        }

        /// A copy of everything served so far.
        #[must_use]
        pub fn snapshot(&self) -> PeerLog {
            lock(&self.shared.log).clone()
        }

        /// Waits up to `bound` for `done` to hold over the log; returns the
        /// log either way (`Err` when the bound passed first).
        ///
        /// # Errors
        ///
        /// The last log, when `done` never held within `bound`.
        pub fn wait_for(
            &self,
            bound: Duration,
            done: impl Fn(&PeerLog) -> bool,
        ) -> Result<PeerLog, PeerLog> {
            let deadline = Instant::now() + bound;
            loop {
                let log = self.snapshot();
                if done(&log) {
                    return Ok(log);
                }
                if Instant::now() >= deadline {
                    return Err(log);
                }
                thread::sleep(Duration::from_millis(50));
            }
        }

        /// While `true`, every `flood` flow's reader stops reading (the
        /// stalled far reader of R5-62); `false` resumes them.
        pub fn stall_floods(&self, stalled: bool) {
            self.shared.floods_stalled.store(stalled, Ordering::SeqCst);
        }
    }

    impl Drop for RemotePeer {
        fn drop(&mut self) {
            self.shared.stop.store(true, Ordering::SeqCst);
            for thread in self.threads.drain(..) {
                let _ = thread.join();
            }
            let _ = Command::new("ip").args(["link", "del", &self.host_if]).output();
        }
    }

    fn lock(log: &Mutex<PeerLog>) -> std::sync::MutexGuard<'_, PeerLog> {
        log.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn spawn_named(name: &str, body: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
        thread::Builder::new()
            .name(name.to_owned())
            .spawn(body)
            .unwrap_or_else(|error| panic!("spawn {name}: {error}"))
    }

    fn ip(args: &[&str]) -> Result<(), RemotePeerError> {
        let out = Command::new("ip")
            .args(args)
            .output()
            .map_err(|source| RemotePeerError::Namespace(NetNsError::Spawn(source)))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(RemotePeerError::Namespace(NetNsError::IpCommand {
                args: args.join(" "),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                status: out.status.code(),
            }))
        }
    }

    /// Runs `body` on a thread that has entered the network namespace
    /// `ns_name`; sockets it creates stay in that namespace.
    fn in_netns<T: Send + 'static>(
        ns_name: &str,
        body: impl FnOnce() -> Result<T, RemotePeerError> + Send + 'static,
    ) -> Result<T, RemotePeerError> {
        let path = format!("/var/run/netns/{ns_name}");
        thread::spawn(move || {
            let file = std::fs::File::open(&path)
                .map_err(|source| RemotePeerError::Socket { what: "namespace handle", source })?;
            // SAFETY: `file` is an open namespace descriptor; `setns` changes
            // only the calling thread's network namespace.
            let ret = unsafe { libc::setns(file.as_raw_fd(), libc::CLONE_NEWNET) };
            if ret != 0 {
                return Err(RemotePeerError::Socket {
                    what: "setns into the peer namespace",
                    source: io::Error::last_os_error(),
                });
            }
            body()
        })
        .join()
        .unwrap_or_else(|_| panic!("the namespace thread panicked"))
    }

    fn v4(addr: SocketAddr) -> SocketAddrV4 {
        match addr {
            SocketAddr::V4(v4) => v4,
            SocketAddr::V6(v6) => panic!("the peer serves IPv4 only, got {v6}"),
        }
    }

    fn serve_tcp(listener: &TcpListener, shared: &Arc<Shared>) {
        listener.set_nonblocking(true).unwrap_or_else(|e| panic!("non-blocking listener: {e}"));
        while !shared.stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, from)) => {
                    // An accepted socket inherits nothing from the listener's
                    // non-blocking flag on Linux; the handler sets its own
                    // read timeout. Handlers end when the peer stops.
                    let shared = Arc::clone(shared);
                    drop(spawn_named("remote-peer-conn", move || {
                        handle_connection(stream, v4(from), &shared);
                    }));
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("remote peer accept failed: {e}"),
            }
        }
    }

    fn read_line(stream: &mut TcpStream) -> io::Result<String> {
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        while line.len() < 128 {
            match stream.read(&mut byte)? {
                0 => break,
                _ if byte[0] == b'\n' => break,
                _ => line.push(byte[0]),
            }
        }
        Ok(String::from_utf8_lossy(&line).into_owned())
    }

    fn handle_connection(mut stream: TcpStream, from: SocketAddrV4, shared: &Arc<Shared>) {
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap_or_else(|e| panic!("read timeout: {e}"));
        let deadline = Instant::now() + Duration::from_secs(10);
        let first = loop {
            match read_line(&mut stream) {
                Ok(line) => break line,
                Err(e)
                    if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
                        && Instant::now() < deadline => {}
                Err(_) => return,
            }
        };
        if let Some(tag) = first.strip_prefix("REQ:") {
            let tag = tag.to_owned();
            if stream.write_all(&tcp_response(&tag)).is_ok() {
                let _ = stream.shutdown(Shutdown::Write);
            }
            lock(&shared.log).requests.push(ServedRequest { tag, from });
        } else if let Some(tag) = first.strip_prefix("HOLD:") {
            hold(stream, from, tag, shared);
        } else if let Some(tag) = first.strip_prefix("FLOOD:") {
            flood(stream, from, tag, shared);
        }
    }

    /// The log key of a new connection of `tag` among `taken`.
    fn unique_key<V>(taken: &BTreeMap<String, V>, tag: &str) -> String {
        if !taken.contains_key(tag) {
            return tag.to_owned();
        }
        (2u32..=u32::MAX)
            .map(|n| format!("{tag}#{n}"))
            .find(|key| !taken.contains_key(key))
            .unwrap_or_else(|| unreachable!("an unbounded range yields a free key"))
    }

    fn hold(mut stream: TcpStream, from: SocketAddrV4, tag: &str, shared: &Arc<Shared>) {
        let key = {
            let mut log = lock(&shared.log);
            let key = unique_key(&log.holds, tag);
            log.holds.insert(
                key.clone(),
                HoldObservation { from, received: 0, sent: 0, terminal: None, corrupted_at: None },
            );
            key
        };
        let writer = stream.try_clone().unwrap_or_else(|e| panic!("clone the hold stream: {e}"));
        let writer_shared = Arc::clone(shared);
        let writer_tag = tag.to_owned();
        let writer_key = key.clone();
        let writer_done = Arc::new(AtomicBool::new(false));
        let writer_stop = Arc::clone(&writer_done);
        let writer_thread = spawn_named("remote-peer-hold-writer", move || {
            let mut writer = writer;
            let mut stream_gen = StreamGen::new(&writer_tag, Direction::PeerToGuest);
            let mut chunk = vec![0u8; STREAM_CHUNK];
            while !writer_shared.stop.load(Ordering::SeqCst) && !writer_stop.load(Ordering::SeqCst)
            {
                stream_gen.fill(&mut chunk);
                if writer.write_all(&chunk).is_err() {
                    return;
                }
                if let Some(obs) = lock(&writer_shared.log).holds.get_mut(&writer_key) {
                    obs.sent += chunk.len() as u64;
                }
                thread::sleep(Duration::from_millis(2));
            }
        });
        let mut check = StreamCheck::new(tag, Direction::GuestToPeer);
        let mut buf = vec![0u8; STREAM_CHUNK];
        let terminal = loop {
            if shared.stop.load(Ordering::SeqCst) {
                break None;
            }
            match stream.read(&mut buf) {
                Ok(0) => break Some(TerminalEvent::Eof),
                Ok(n) => {
                    let corrupted = check.check(&buf[..n]).err();
                    let mut log = lock(&shared.log);
                    if let Some(obs) = log.holds.get_mut(&key) {
                        obs.received = check.verified();
                        if obs.corrupted_at.is_none() {
                            obs.corrupted_at = corrupted;
                        }
                    }
                }
                Err(e)
                    if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                Err(e) => break Some(TerminalEvent::from_read_error(&e)),
            }
        };
        if let Some(terminal) = terminal
            && let Some(obs) = lock(&shared.log).holds.get_mut(&key)
        {
            obs.terminal.get_or_insert(terminal);
        }
        writer_done.store(true, Ordering::SeqCst);
        let _ = writer_thread.join();
    }

    fn flood(mut stream: TcpStream, from: SocketAddrV4, tag: &str, shared: &Arc<Shared>) {
        let key = {
            let mut log = lock(&shared.log);
            let key = unique_key(&log.floods, tag);
            log.floods.insert(
                key.clone(),
                FloodObservation { from, verified: 0, corrupted_at: None, terminal: None },
            );
            key
        };
        let mut check = StreamCheck::new(tag, Direction::GuestToPeer);
        let mut buf = vec![0u8; STREAM_CHUNK];
        loop {
            if shared.stop.load(Ordering::SeqCst) {
                return;
            }
            if shared.floods_stalled.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            match stream.read(&mut buf) {
                Ok(0) => {
                    if let Some(obs) = lock(&shared.log).floods.get_mut(&key) {
                        obs.terminal.get_or_insert(TerminalEvent::Eof);
                    }
                    return;
                }
                Ok(n) => {
                    let corrupted = check.check(&buf[..n]).err();
                    if let Some(obs) = lock(&shared.log).floods.get_mut(&key) {
                        obs.verified = check.verified();
                        if obs.corrupted_at.is_none() {
                            obs.corrupted_at = corrupted;
                        }
                    }
                }
                Err(e)
                    if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                Err(e) => {
                    if let Some(obs) = lock(&shared.log).floods.get_mut(&key) {
                        obs.terminal.get_or_insert_with(|| TerminalEvent::from_read_error(&e));
                    }
                    return;
                }
            }
        }
    }

    fn serve_udp(socket: &UdpSocket, shared: &Arc<Shared>) {
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap_or_else(|e| panic!("UDP read timeout: {e}"));
        let mut buf = vec![0u8; 65_536];
        while !shared.stop.load(Ordering::SeqCst) {
            match socket.recv_from(&mut buf) {
                Ok((n, from)) => {
                    let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                    if let Some(tag) = text.strip_prefix("DGRAM-REQ:") {
                        let tag = tag.to_owned();
                        let _ = socket.send_to(&datagram_response(&tag), from);
                        lock(&shared.log).datagrams.push(ServedDatagram { tag, from: v4(from) });
                    } else if let Some(rest) = text.strip_prefix("CHURN:") {
                        let tag = rest.rsplit_once(':').map_or(rest, |(tag, _)| tag).to_owned();
                        *lock(&shared.log).churn.entry(tag).or_insert(0) += 1;
                    }
                }
                Err(e)
                    if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                Err(e) => panic!("remote peer UDP receive failed: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_verifies_against_its_own_generator_and_rejects_the_other_direction() {
        let mut gen_ = StreamGen::new("vm-1", Direction::GuestToPeer);
        let mut chunk = vec![0u8; 10_000];
        gen_.fill(&mut chunk);
        let mut check = StreamCheck::new("vm-1", Direction::GuestToPeer);
        assert_eq!(check.check(&chunk[..3]), Ok(()));
        assert_eq!(check.check(&chunk[3..]), Ok(()));
        assert_eq!(check.verified(), 10_000);
        let mut other = StreamCheck::new("vm-1", Direction::PeerToGuest);
        assert!(other.check(&chunk).is_err(), "the two directions differ");
    }

    #[test]
    fn terminal_events_round_trip_through_the_console_line() {
        for event in [TerminalEvent::Eof, TerminalEvent::Reset, TerminalEvent::Error(32)] {
            let console = format!("[ 1.0] noise\n{TERMINAL_LINE_PREFIX}{event}\nmore\n");
            assert_eq!(terminal_event_in_console(&console), Some(event));
        }
        assert_eq!(terminal_event_in_console("nothing here"), None);
        let reports = format!("{FLOOD_WRITTEN_PREFIX}10\nx\n{FLOOD_WRITTEN_PREFIX}20\n");
        assert_eq!(flood_written_in_console(&reports), vec![10, 20]);
    }

    #[test]
    fn listen_connection_ends_round_trip_through_the_console_line() {
        let ends = [
            ListenConnEnd { conn: 1, hello: false, terminal: TerminalEvent::Reset },
            ListenConnEnd { conn: 2, hello: true, terminal: TerminalEvent::Eof },
            ListenConnEnd { conn: 30, hello: false, terminal: TerminalEvent::Error(32) },
        ];
        let mut console = String::new();
        for e in &ends {
            console.push_str("[ 2.0] ");
            console.push_str(&e.to_string());
            console.push_str("\nunrelated line\n");
        }
        assert_eq!(listen_conn_ends_in_console(&console), ends.to_vec());
        for bad in [
            "listen: conn=x hello=no terminal=eof",
            "listen: conn=1 hello=maybe terminal=eof",
            "listen: conn=1 hello=no",
            "listen: conn=1 hello=no terminal=eof extra",
        ] {
            assert!(bad.parse::<ListenConnEnd>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn requests_and_responses_are_byte_distinct() {
        assert_ne!(tcp_request("a"), tcp_response("a"));
        assert_ne!(datagram_request("a"), datagram_response("a"));
        assert!(tag_is_valid("vm-07"));
        assert!(!tag_is_valid("bad tag"));
        assert!(!tag_is_valid(""));
    }
}
