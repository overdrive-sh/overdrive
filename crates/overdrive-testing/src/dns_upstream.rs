//! A scripted upstream DNS resolver for real-wire tests (GH #382,
//! guest-public-dns-resolution).
//!
//! It serves `addr:53` over UDP and TCP (RFC 1035 two-byte framing, one query
//! per connection), answers by question name from a script, and records every
//! query it received. It stands in for the host's upstream resolver: the node
//! forwards guest queries to it through the production forwarder, and the test
//! reads what the upstream saw. It owns no DNS semantics beyond one A answer, an
//! NXDOMAIN, a SERVFAIL and silence, so a test that wants a malformed reply
//! writes its own socket.
//!
//! Linux-only items carry no `cfg` here: the crate gates the whole module.
//! Binding `:53` needs root; callers run in a guest group.

#![allow(clippy::expect_used, clippy::missing_panics_doc, reason = "test-support fixture")]

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, UdpSocket};
use std::os::fd::FromRawFd as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// How the scripted upstream answers one question name.
#[derive(Clone, Debug)]
pub enum Behaviour {
    /// NOERROR with one A record per address (an empty NOERROR for a non-A query).
    Answer(Vec<Ipv4Addr>),
    /// NXDOMAIN.
    NxDomain,
    /// SERVFAIL.
    ServFail,
    /// No reply at all (the connection stays open on TCP).
    Silent,
}

/// The transport a recorded query arrived over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// UDP.
    Udp,
    /// TCP.
    Tcp,
}

/// One query the upstream received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeenQuery {
    /// The question name, lower-cased, no trailing dot.
    pub name: String,
    /// The question type code (1 = A, 28 = AAAA).
    pub qtype: u16,
    /// The transport it arrived over.
    pub transport: Transport,
}

struct Shared {
    script: Mutex<BTreeMap<String, Behaviour>>,
    default: Mutex<Behaviour>,
    seen: Mutex<Vec<SeenQuery>>,
    stop: AtomicBool,
}

/// The running scripted upstream. Stops (and releases its ports) on drop.
pub struct ScriptedDnsUpstream {
    addr: Ipv4Addr,
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl ScriptedDnsUpstream {
    /// Starts serving `addr:53` over UDP and TCP. Unscripted names get NXDOMAIN.
    ///
    /// The UDP socket sets `SO_REUSEADDR` before binding, so it coexists with a
    /// wildcard `0.0.0.0:53` socket that also sets it (the node's responder).
    pub fn start(addr: Ipv4Addr) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            script: Mutex::new(BTreeMap::new()),
            default: Mutex::new(Behaviour::NxDomain),
            seen: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
        });
        let udp = bind_udp_reuse(SocketAddrV4::new(addr, 53))?;
        udp.set_read_timeout(Some(Duration::from_millis(50)))?;
        let tcp = TcpListener::bind(SocketAddrV4::new(addr, 53))?;
        tcp.set_nonblocking(true)?;
        let udp_shared = Arc::clone(&shared);
        let tcp_shared = Arc::clone(&shared);
        let threads = vec![
            thread::spawn(move || serve_udp(&udp, &udp_shared)),
            thread::spawn(move || serve_tcp(&tcp, &tcp_shared)),
        ];
        Ok(Self { addr, shared, threads })
    }

    /// The address it serves.
    #[must_use]
    pub const fn addr(&self) -> Ipv4Addr {
        self.addr
    }

    /// Scripts how `name` (case-insensitive, no trailing dot) is answered.
    pub fn script(&self, name: &str, behaviour: Behaviour) {
        self.shared.script.lock().expect("script").insert(name.to_ascii_lowercase(), behaviour);
    }

    /// Scripts how every unscripted name is answered.
    pub fn script_default(&self, behaviour: Behaviour) {
        *self.shared.default.lock().expect("default") = behaviour;
    }

    /// Every query received so far, in arrival order.
    #[must_use]
    pub fn seen(&self) -> Vec<SeenQuery> {
        self.shared.seen.lock().expect("seen").clone()
    }

    /// How many received queries asked for `name` (case-insensitive).
    #[must_use]
    pub fn seen_count(&self, name: &str) -> usize {
        let name = name.to_ascii_lowercase();
        self.seen().iter().filter(|q| q.name == name).count()
    }
}

impl Drop for ScriptedDnsUpstream {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

fn bind_udp_reuse(addr: SocketAddrV4) -> io::Result<UdpSocket> {
    // SAFETY: plain socket(2)/setsockopt(2)/bind(2) calls on a descriptor this
    // function owns; on success the descriptor is handed to `UdpSocket`, on
    // failure it is closed first.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let one: libc::c_int = 1;
        let sa = libc::sockaddr_in {
            sin_family: libc::sa_family_t::try_from(libc::AF_INET).expect("AF_INET fits"),
            sin_port: addr.port().to_be(),
            sin_addr: libc::in_addr { s_addr: u32::from(*addr.ip()).to_be() },
            sin_zero: [0; 8],
        };
        let ok = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            (&raw const one).cast(),
            u32::try_from(size_of::<libc::c_int>()).expect("size fits"),
        ) == 0
            && libc::bind(
                fd,
                (&raw const sa).cast(),
                u32::try_from(size_of::<libc::sockaddr_in>()).expect("size fits"),
            ) == 0;
        if !ok {
            let err = io::Error::last_os_error();
            libc::close(fd);
            return Err(err);
        }
        Ok(UdpSocket::from_raw_fd(fd))
    }
}

/// The host resolver file `/etc/resolv.conf`, which the node reads on every
/// forwarded query. A test that plays the host rewrites it to name its
/// scripted upstream; the original is restored on drop.
pub struct HostResolverFile {
    original: Option<Vec<u8>>,
}

#[allow(
    clippy::unused_self,
    reason = "the guard value proves the original is held and will be restored; the methods act through it"
)]
impl HostResolverFile {
    /// The resolver file's path.
    pub const PATH: &'static str = "/etc/resolv.conf";

    /// Remembers the current contents so drop can restore them.
    #[must_use]
    pub fn take() -> Self {
        Self { original: std::fs::read(Self::PATH).ok() }
    }

    /// Replaces the file with `body` (the path may be a symlink in the image).
    pub fn write(&self, body: &str) {
        let _ = std::fs::remove_file(Self::PATH);
        std::fs::write(Self::PATH, body)
            .unwrap_or_else(|e| panic!("precondition: the host resolver file is writable: {e}"));
    }

    /// Names `upstream` as the one nameserver.
    pub fn names(&self, upstream: Ipv4Addr) {
        self.write(&format!("nameserver {upstream}\n"));
    }

    /// Removes the file.
    pub fn remove(&self) {
        let _ = std::fs::remove_file(Self::PATH);
    }
}

impl Drop for HostResolverFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(Self::PATH);
        if let Some(bytes) = &self.original {
            let _ = std::fs::write(Self::PATH, bytes);
        }
    }
}

/// What the script says to do with one decoded query.
enum Action {
    Reply(Vec<u8>),
    Silence,
}

fn decide(shared: &Shared, query: &[u8], transport: Transport) -> Option<Action> {
    let parsed = parse_question(query)?;
    shared.seen.lock().expect("seen").push(SeenQuery {
        name: parsed.name.clone(),
        qtype: parsed.qtype,
        transport,
    });
    let behaviour = shared
        .script
        .lock()
        .expect("script")
        .get(&parsed.name)
        .cloned()
        .unwrap_or_else(|| shared.default.lock().expect("default").clone());
    Some(match behaviour {
        Behaviour::Silent => Action::Silence,
        Behaviour::NxDomain => Action::Reply(build_reply(query, &parsed, 3, &[])),
        Behaviour::ServFail => Action::Reply(build_reply(query, &parsed, 2, &[])),
        Behaviour::Answer(addrs) => {
            let addrs = if parsed.qtype == 1 { addrs } else { Vec::new() };
            Action::Reply(build_reply(query, &parsed, 0, &addrs))
        }
    })
}

struct Question {
    name: String,
    qtype: u16,
    /// Offset one past the question section.
    end: usize,
}

fn parse_question(query: &[u8]) -> Option<Question> {
    if query.len() < 12 || u16::from_be_bytes([query[4], query[5]]) != 1 {
        return None;
    }
    let mut at = 12;
    let mut labels = Vec::new();
    loop {
        let len = usize::from(*query.get(at)?);
        at += 1;
        if len == 0 {
            break;
        }
        labels.push(String::from_utf8_lossy(query.get(at..at + len)?).to_ascii_lowercase());
        at += len;
    }
    let qtype = u16::from_be_bytes([*query.get(at)?, *query.get(at + 1)?]);
    Some(Question { name: labels.join("."), qtype, end: at + 4 })
}

fn build_reply(query: &[u8], q: &Question, rcode: u8, addrs: &[Ipv4Addr]) -> Vec<u8> {
    let mut reply = Vec::with_capacity(q.end + addrs.len() * 16);
    reply.extend_from_slice(&query[0..2]); // id
    let rd = query[2] & 0x01;
    reply.push(0x80 | rd); // QR, opcode 0, RD copied
    reply.push(0x80 | rcode); // RA + rcode
    reply.extend_from_slice(&1u16.to_be_bytes());
    reply.extend_from_slice(&u16::try_from(addrs.len()).expect("few answers").to_be_bytes());
    reply.extend_from_slice(&[0, 0, 0, 0]);
    reply.extend_from_slice(&query[12..q.end]);
    for addr in addrs {
        reply.extend_from_slice(&[0xC0, 0x0C, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
        reply.extend_from_slice(&addr.octets());
    }
    reply
}

fn serve_udp(socket: &UdpSocket, shared: &Shared) {
    let mut buf = [0u8; 1500];
    while !shared.stop.load(Ordering::SeqCst) {
        let Ok((n, peer)) = socket.recv_from(&mut buf) else { continue };
        if let Some(Action::Reply(reply)) = decide(shared, &buf[..n], Transport::Udp) {
            let _ = socket.send_to(&reply, peer);
        }
    }
}

fn serve_tcp(listener: &TcpListener, shared: &Arc<Shared>) {
    let mut conns: Vec<JoinHandle<()>> = Vec::new();
    while !shared.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, SocketAddr::V4(_))) => {
                let shared = Arc::clone(shared);
                conns.push(thread::spawn(move || {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut len = [0u8; 2];
                    if stream.read_exact(&mut len).is_err() {
                        return;
                    }
                    let mut body = vec![0u8; usize::from(u16::from_be_bytes(len))];
                    if stream.read_exact(&mut body).is_err() {
                        return;
                    }
                    match decide(&shared, &body, Transport::Tcp) {
                        Some(Action::Reply(reply)) => {
                            let framed_len =
                                u16::try_from(reply.len()).expect("small reply").to_be_bytes();
                            let _ = stream.write_all(&framed_len);
                            let _ = stream.write_all(&reply);
                        }
                        Some(Action::Silence) => {
                            while !shared.stop.load(Ordering::SeqCst) {
                                thread::sleep(Duration::from_millis(50));
                            }
                        }
                        None => {}
                    }
                }));
            }
            Ok(_) | Err(_) => thread::sleep(Duration::from_millis(20)),
        }
    }
    for c in conns {
        let _ = c.join();
    }
}
