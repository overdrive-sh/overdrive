//! Real-kernel half of R5-59 (b), the guest seam's accept-path equivalence
//! (D-UP-19, PD-5; D-UP-18): `GuestFlowOwner` over `KernelGuestOwnerIo` and
//! `KernelFlowForwarder` serves the same connection table
//! ([`ACCEPT_PATH_CASES`]) with the same per-connection outcome as the sim half
//! (`tests::tcp_accept::accept_path_outcomes_by_first_record`, over
//! `SimGuestOwnerIo`): a connection with its `TcpAccept` request buffered is
//! served — the application accepts it and the bytes written after the
//! request reach it — and one with no byte yields no application connection
//! and its V_g is closed. Order-insensitive, compared per connection.
//!
//! **Lane.** Inside an Overdrive microVM guest, as the guest's operator
//! command, through `cargo xtask metal run --` with `kvm-tests` (see
//! `transport::real_kernel`): the production `overdrive-init` (PID 1) owns
//! `GUEST_INBOUND_PORT`; this cell opens the connections over the guest's vsock
//! loopback transport (`VMADDR_CID_LOCAL`), which delivers them to that
//! listener through the same `virtio_transport_recv_pkt` path a host
//! connection takes (`net/vmw_vsock/vsock_loopback.c:142-143 @ v7.0`), and
//! plays the application on `workload_addr:<port>`. It fails closed outside a
//! guest and when the guest kernel has no vsock loopback transport.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::doc_markdown,
    reason = "real-kernel test cells: a failed precondition panics"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::mem::size_of;
use std::net::{Ipv4Addr, Shutdown, SocketAddrV4, TcpListener};
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::time::{Duration, Instant};

use aya::maps::{MapType, loaded_maps};
use aya::programs::{ProgramType, loaded_programs};
use aya::sys::{Stats, enable_stats};
use nix::sys::socket::{
    AddressFamily, MsgFlags, SockFlag, SockType, SockaddrIn, VsockAddr, connect, recv, send,
    sendto, setsockopt, shutdown, socket, sockopt,
};
use nix::sys::time::{TimeVal, TimeValLike};
use overdrive_core::guest_transport::wire::{FlowId, FlowOrigin, FlowRequest};
use overdrive_core::guest_transport::{
    GUEST_INBOUND_PORT, GUEST_TCP_INTAKE_PORT, GUEST_TCP_PENDING_MAX,
};
use overdrive_core::net::ListenPort;
use overdrive_testing::guest_kit::TC_GUEST_PATH;

use super::test_support::{ACCEPT_PATH_CASES, AcceptPathCase};
use crate::transport::{GuestTransportToken, parse_transport_token};

/// The guest's transport token; panics (fails closed) outside a guest.
fn in_guest() -> GuestTransportToken {
    let cmdline = fs::read_to_string("/proc/cmdline").expect("/proc/cmdline is readable");
    assert!(
        cmdline.contains("overdrive.transport="),
        "not running inside an Overdrive guest: this cell runs as a microVM guest's operator command"
    );
    parse_transport_token(&cmdline).expect("the guest's token parses")
}

/// A vsock STREAM connection to the guest's own inbound listener over the
/// loopback transport.
fn connect_inbound() -> File {
    let fd: OwnedFd = socket(AddressFamily::Vsock, SockType::Stream, SockFlag::SOCK_CLOEXEC, None)
        .expect("a vsock socket");
    let addr = VsockAddr::new(nix::libc::VMADDR_CID_LOCAL, GUEST_INBOUND_PORT.as_u32());
    connect(fd.as_raw_fd(), &addr).unwrap_or_else(|e| {
        panic!("connect to the guest's own GUEST_INBOUND_PORT over vsock loopback failed ({e}): the guest kernel needs CONFIG_VSOCKETS_LOOPBACK")
    });
    File::from(fd)
}

/// The application's accept on `listener` within `bound`, or `None`.
fn accept_within(listener: &TcpListener, bound: Duration) -> Option<std::net::TcpStream> {
    listener.set_nonblocking(true).expect("non-blocking listener");
    let deadline = Instant::now() + bound;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(e) if e.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => return None,
            Err(e) => panic!("accept failed: {e}"),
        }
    }
}

/// The per-connection outcome both halves compare: served with the payload,
/// or no application connection.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Served(Vec<u8>),
    NoApplicationConnection,
}

/// R5-59 (b), real half: the production guest owner's outcome per connection
/// of [`ACCEPT_PATH_CASES`] equals the sim half's — served or not, by its
/// first record.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn the_accept_path_matches_the_sim_per_connection() {
    let token = in_guest();
    let mut listeners: BTreeMap<u16, TcpListener> = BTreeMap::new();
    let mut outcomes = BTreeMap::new();
    for case in ACCEPT_PATH_CASES {
        let AcceptPathCase { seq, port, request_buffered, payload } = case;
        let listener = listeners.entry(port).or_insert_with(|| {
            TcpListener::bind(SocketAddrV4::new(token.workload_addr, port))
                .expect("the application listens")
        });
        let mut vsock = connect_inbound();
        let outcome = if request_buffered {
            let request = FlowRequest::TcpAccept {
                flow: FlowId::new(FlowOrigin::Host, seq).expect("small seq"),
                peer: token.gateway,
                listen_port: ListenPort::from_str(&port.to_string()).expect("non-zero"),
                launch: token.launch,
            };
            vsock.write_all(&request.encode()).expect("the request is written");
            let mut app =
                accept_within(listener, Duration::from_secs(5)).expect("the application accepts C");
            assert_eq!(
                app.peer_addr().expect("C's address").ip(),
                std::net::IpAddr::V4(token.gateway),
                "C is bound to the gateway"
            );
            vsock.write_all(payload).expect("the payload is written after the request");
            let mut got = vec![0u8; payload.len()];
            app.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
            app.read_exact(&mut got).expect("the payload reaches the application");
            app.shutdown(Shutdown::Both).ok();
            Outcome::Served(got)
        } else {
            shutdown(vsock.as_raw_fd(), nix::sys::socket::Shutdown::Write)
                .expect("the connection ends with no byte");
            let app = accept_within(listener, Duration::from_secs(2));
            assert!(app.is_none(), "{case:?}: no application connection");
            let mut rest = Vec::new();
            vsock.read_to_end(&mut rest).expect("V_g is closed by the guest owner");
            assert!(rest.is_empty());
            Outcome::NoApplicationConnection
        };
        outcomes.insert(seq, outcome);
    }
    for case in ACCEPT_PATH_CASES {
        let expected = if case.request_buffered {
            Outcome::Served(case.payload.to_vec())
        } else {
            Outcome::NoApplicationConnection
        };
        assert_eq!(outcomes.get(&case.seq), Some(&expected), "{case:?}");
    }
}

// ---------------------------------------------------------------------------
// TCP-CORRELATION: the real Connect4 -> final ACK -> intake-owner path
// ---------------------------------------------------------------------------

const TCP_FASTOPEN: i32 = 23;
const TCP_SYNCOOKIES_SYSCTL: &str = "/proc/sys/net/ipv4/tcp_syncookies";
const TCP_FASTOPEN_SYSCTL: &str = "/proc/sys/net/ipv4/tcp_fastopen";
const BPF_FS: &str = "/sys/fs/bpf";
const CHILD_CORRELATION_MAX_ENTRIES: u32 = 8_194;
const TFO_QDISC_CLASS: i32 = 0x0001_0003;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum CorrelationPhase {
    Pending = 1,
    Ready = 2,
    Rejected = 3,
    CandidateReady = 4,
    CandidateRejected = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct CorrelationMeta {
    application_cookie: u64,
    destination_ip: u32,
    destination_port: u16,
    pad: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct CorrelationChildState {
    phase: u32,
    pad: u32,
    meta: CorrelationMeta,
}

impl CorrelationChildState {
    fn phase(self) -> CorrelationPhase {
        match self.phase {
            1 => CorrelationPhase::Pending,
            2 => CorrelationPhase::Ready,
            3 => CorrelationPhase::Rejected,
            4 => CorrelationPhase::CandidateReady,
            5 => CorrelationPhase::CandidateRejected,
            other => panic!("unknown child-correlation phase {other}"),
        }
    }
}

/// Source-local inspection of the two private maps. Discovery uses the
/// design-owned map shapes rather than a test-only production accessor:
/// socket storage has `(key=4, max_entries=0)`, and the child hash has the
/// exact 8,194-entry bound. The disposable guest has one production owner.
#[derive(Clone, Copy)]
struct CorrelationMaps {
    active_map: u32,
    child_map: u32,
    candidate_program: u32,
}

impl CorrelationMaps {
    fn discover() -> Self {
        let mut active = Vec::new();
        let mut child = Vec::new();
        let u64_size = u32::try_from(size_of::<u64>()).expect("u64 size fits u32");
        for info in loaded_maps() {
            let info = info.expect("the guest's loaded BPF maps are observable");
            match info.map_type().expect("the map type is known") {
                MapType::SkStorage if info.key_size() == 4 && info.max_entries() == 0 => {
                    active.push(info.id());
                }
                MapType::Hash
                    if info.key_size() == u64_size
                        && info.max_entries() == CHILD_CORRELATION_MAX_ENTRIES =>
                {
                    child.push(info.id());
                }
                _ => {}
            }
        }
        assert_eq!(active.len(), 1, "one capacity-free active-correlation socket-storage map");
        assert_eq!(child.len(), 1, "one 8,194-entry child-correlation map");
        let child_id = child[0];
        let candidates: Vec<_> = loaded_programs()
            .map(|program| program.expect("the guest's loaded BPF programs are observable"))
            .filter(|program| {
                program.program_type().ok() == Some(ProgramType::Tracing)
                    && program.name_as_str() == Some("tcp_correlation")
                    && program
                        .map_ids()
                        .expect("read the candidate program's map ids")
                        .is_some_and(|ids| ids.contains(&child_id))
            })
            .map(|program| program.id())
            .collect();
        assert_eq!(candidates.len(), 1, "one tracing candidate uses the child map");
        Self { active_map: active[0], child_map: child_id, candidate_program: candidates[0] }
    }

    #[allow(
        unsafe_code,
        reason = "Aya 0.13.1 exposes no key-only iterator; the source-local Tier-3 fixture calls BPF_MAP_GET_NEXT_KEY over exact u64 buffers without constraining the private value representation"
    )]
    fn child_cookies(&self) -> Vec<u64> {
        let info = aya::maps::MapInfo::from_id(self.child_map)
            .expect("open the child-correlation map by id");
        let map_fd = info.fd().expect("open the child-correlation map fd");
        let mut current: Option<u64> = None;
        let mut cookies = Vec::new();
        loop {
            let mut next = 0u64;
            let mut attr = [0u8; 32];
            attr[0..4].copy_from_slice(&map_fd.as_fd().as_raw_fd().cast_unsigned().to_ne_bytes());
            if let Some(key) = current.as_ref() {
                attr[8..16].copy_from_slice(&(std::ptr::from_ref(key) as u64).to_ne_bytes());
            }
            attr[16..24].copy_from_slice(&(std::ptr::from_mut(&mut next) as u64).to_ne_bytes());
            // SAFETY: BPF_MAP_GET_NEXT_KEY reads the optional live u64 key
            // and writes one u64 into `next`, matching the map's key size.
            let result =
                unsafe { nix::libc::syscall(nix::libc::SYS_bpf, 4, attr.as_ptr(), attr.len()) };
            if result == 0 {
                cookies.push(next);
                current = Some(next);
            } else {
                let error = std::io::Error::last_os_error();
                assert_eq!(
                    error.raw_os_error(),
                    Some(nix::libc::ENOENT),
                    "iterate child-correlation keys"
                );
                return cookies;
            }
        }
    }

    fn child_entries(&self) -> usize {
        self.child_cookies().len()
    }

    #[allow(
        unsafe_code,
        reason = "the source-local Tier-3 fixture reads the design-owned 24-byte child ABI through BPF_MAP_LOOKUP_ELEM"
    )]
    fn child_state(&self, cookie: u64) -> Option<CorrelationChildState> {
        let info = aya::maps::MapInfo::from_id(self.child_map)
            .expect("open the child-correlation map by id");
        assert_eq!(
            info.value_size(),
            u32::try_from(size_of::<CorrelationChildState>()).expect("child state size fits u32"),
            "the child-correlation value has the design-owned ABI"
        );
        let map_fd = info.fd().expect("open the child-correlation map fd");
        let mut value = std::mem::MaybeUninit::<CorrelationChildState>::uninit();
        let mut attr = [0u8; 32];
        attr[0..4].copy_from_slice(&map_fd.as_fd().as_raw_fd().cast_unsigned().to_ne_bytes());
        attr[8..16].copy_from_slice(&(std::ptr::from_ref(&cookie) as u64).to_ne_bytes());
        attr[16..24].copy_from_slice(&(value.as_mut_ptr() as u64).to_ne_bytes());
        // SAFETY: BPF_MAP_LOOKUP_ELEM reads the live u64 key and initializes
        // exactly the map's asserted 24-byte value on success.
        let result =
            unsafe { nix::libc::syscall(nix::libc::SYS_bpf, 1, attr.as_ptr(), attr.len()) };
        if result == 0 {
            // SAFETY: the successful lookup initialized the complete value.
            Some(unsafe { value.assume_init() })
        } else {
            let error = std::io::Error::last_os_error();
            assert_eq!(
                error.raw_os_error(),
                Some(nix::libc::ENOENT),
                "child-correlation lookup for cookie {cookie}"
            );
            None
        }
    }

    fn candidate_run_count(&self) -> u64 {
        loaded_programs()
            .map(|program| program.expect("the candidate program remains observable"))
            .find(|program| program.id() == self.candidate_program)
            .expect("the candidate program remains loaded")
            .run_count()
    }

    #[allow(
        unsafe_code,
        reason = "Aya 0.13.1 has no typed userspace SK_STORAGE lookup; the source-local Tier-3 fixture calls BPF_MAP_LOOKUP_ELEM over exact buffers"
    )]
    fn active_contains(&self, socket_fd: RawFd) -> bool {
        let info = aya::maps::MapInfo::from_id(self.active_map)
            .expect("open the active-correlation map by id");
        let map_fd = info.fd().expect("open the active-correlation map fd");
        let key = socket_fd;
        let mut value = vec![0u8; info.value_size() as usize];
        let mut attr = [0u8; 32];
        attr[0..4].copy_from_slice(&map_fd.as_fd().as_raw_fd().cast_unsigned().to_ne_bytes());
        attr[8..16].copy_from_slice(&(std::ptr::from_ref(&key) as u64).to_ne_bytes());
        attr[16..24].copy_from_slice(&(value.as_mut_ptr() as u64).to_ne_bytes());
        // SAFETY: BPF_MAP_LOOKUP_ELEM reads the initialized attr, the live
        // socket-fd key and the value-sized writable buffer for this map.
        let result =
            unsafe { nix::libc::syscall(nix::libc::SYS_bpf, 1, attr.as_ptr(), attr.len()) };
        if result == 0 {
            true
        } else {
            let error = std::io::Error::last_os_error();
            assert_eq!(
                error.raw_os_error(),
                Some(nix::libc::ENOENT),
                "active-correlation lookup for fd {socket_fd}"
            );
            false
        }
    }
}

struct ApplicationSocket(OwnedFd);

impl ApplicationSocket {
    fn new() -> Self {
        Self(
            socket(AddressFamily::Inet, SockType::Stream, SockFlag::SOCK_CLOEXEC, None)
                .expect("create the application socket"),
        )
    }

    fn raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }

    #[allow(
        unsafe_code,
        reason = "nix 0.30 has no SO_COOKIE sockopt; this source-local Tier-3 fixture supplies an aligned u64 and its exact length"
    )]
    fn cookie(&self) -> SocketCookie {
        let mut raw = 0u64;
        let mut len =
            nix::libc::socklen_t::try_from(size_of::<u64>()).expect("u64 size fits socklen_t");
        // SAFETY: SO_COOKIE writes at most `len` bytes into the aligned u64.
        let result = unsafe {
            nix::libc::getsockopt(
                self.raw_fd(),
                nix::libc::SOL_SOCKET,
                nix::libc::SO_COOKIE,
                std::ptr::from_mut(&mut raw).cast(),
                &raw mut len,
            )
        };
        assert_eq!(result, 0, "SO_COOKIE: {}", std::io::Error::last_os_error());
        SocketCookie::new(raw).expect("Linux socket cookies are non-zero")
    }

    fn connect(&self, destination: SocketAddrV4) {
        connect(self.raw_fd(), &SockaddrIn::from(destination))
            .unwrap_or_else(|e| panic!("connect to {destination}: {e}"));
    }

    fn fast_open(&self, destination: SocketAddrV4, payload: &[u8]) {
        let written = sendto(
            self.raw_fd(),
            payload,
            &SockaddrIn::from(destination),
            MsgFlags::from_bits_retain(nix::libc::MSG_FASTOPEN),
        )
        .unwrap_or_else(|e| panic!("MSG_FASTOPEN to {destination}: {e}"));
        assert_eq!(
            written,
            payload.len(),
            "MSG_FASTOPEN to {destination} writes the complete payload"
        );
    }

    fn select_tfo_gate(&self) {
        setsockopt(&self.0, sockopt::Priority, &TFO_QDISC_CLASS)
            .expect("select the private TFO qdisc class");
    }

    fn bypass_tfo_gate(&self) {
        setsockopt(&self.0, sockopt::Priority, &0).expect("restore the default qdisc class");
    }

    fn reset_on_close(&self) {
        let linger = nix::libc::linger { l_onoff: 1, l_linger: 0 };
        setsockopt(&self.0, sockopt::Linger, &linger).expect("select reset-close");
    }

    fn local_addr(&self) -> SocketAddrV4 {
        let address: SockaddrIn = nix::sys::socket::getsockname(self.raw_fd())
            .expect("read the application socket's local address");
        address.into()
    }

    fn send_probe(&self) {
        let byte = [0x5au8];
        let written = send(self.raw_fd(), &byte, MsgFlags::MSG_NOSIGNAL)
            .expect("send one post-handshake byte");
        assert_eq!(written, 1, "the post-handshake send writes one byte");
    }

    fn send_probes(&self, count: usize) {
        for _ in 0..count {
            self.send_probe();
        }
    }

    fn expect_reset(&self) {
        setsockopt(&self.0, sockopt::ReceiveTimeout, &TimeVal::seconds(2))
            .expect("set the direct connection's receive timeout");
        let mut byte = [0u8; 1];
        let error = recv(self.raw_fd(), &mut byte, MsgFlags::empty())
            .expect_err("an uncorrelated intake child is reset, not cleanly closed");
        assert_eq!(error, nix::errno::Errno::ECONNRESET, "the direct intake terminal error");
    }
}

const fn expected_correlation_meta(
    cookie: SocketCookie,
    destination: SocketAddrV4,
) -> CorrelationMeta {
    CorrelationMeta {
        application_cookie: cookie.get(),
        destination_ip: u32::from_ne_bytes(destination.ip().octets()),
        destination_port: destination.port(),
        pad: 0,
    }
}

fn wait_child_phase(
    maps: CorrelationMaps,
    cookie: u64,
    phase: CorrelationPhase,
) -> CorrelationChildState {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(state) = maps.child_state(cookie)
            && state.phase() == phase
        {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "child {cookie} did not reach {phase:?}: {:?}",
            maps.child_state(cookie)
        );
        std::thread::yield_now();
    }
}

fn wait_only_child_cookie(maps: CorrelationMaps, what: &str) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let cookies = maps.child_cookies();
        match cookies.as_slice() {
            [cookie] => return *cookie,
            [] => assert!(
                Instant::now() < deadline,
                "{what} created no child-correlation row within one second"
            ),
            _ => panic!("{what} created more than one child-correlation row: {cookies:?}"),
        }
        std::thread::yield_now();
    }
}

fn wait_child_absent(maps: CorrelationMaps, cookie: u64) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while maps.child_state(cookie).is_some() {
        assert!(Instant::now() < deadline, "child {cookie} was not deleted");
        std::thread::yield_now();
    }
}

/// Holds the final ACK from a real `MSG_FASTOPEN` producer long enough for
/// the production owner to observe its child in `SYN_RECV`. Only the selected
/// socket enters the delayed PRIO class; the default-priority capacity probe
/// bypasses it. Removing the root qdisc releases all fixture state.
struct TfoFinalAckGate;

impl TfoFinalAckGate {
    fn install() -> Self {
        Self::run(&["qdisc", "add", "dev", "lo", "root", "handle", "1:", "prio", "bands", "3"]);
        let gate = Self;
        let output = Command::new(TC_GUEST_PATH)
            .args([
                "qdisc", "add", "dev", "lo", "parent", "1:3", "handle", "30:", "netem", "delay",
                "1000ms", "limit", "1000",
            ])
            .output()
            .expect("run tc to install the private TFO NETEM gate");
        if !output.status.success() {
            drop(gate);
            panic!(
                "install the private TFO NETEM gate: status={:?}, stderr={}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        gate
    }

    fn remove(self) {
        Self::run(&["qdisc", "del", "dev", "lo", "root"]);
        std::mem::forget(self);
    }

    fn run(args: &[&str]) {
        let output = Command::new(TC_GUEST_PATH)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("run {} {}: {error}", TC_GUEST_PATH, args.join(" ")));
        assert!(
            output.status.success(),
            "{} {}: status={:?}, stderr={}",
            TC_GUEST_PATH,
            args.join(" "),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

impl Drop for TfoFinalAckGate {
    fn drop(&mut self) {
        let _ = Command::new(TC_GUEST_PATH).args(["qdisc", "del", "dev", "lo", "root"]).output();
    }
}

struct SysctlRestore {
    path: &'static str,
    original: String,
}

impl SysctlRestore {
    fn set(path: &'static str, value: &str) -> Self {
        let original = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        fs::write(path, value).unwrap_or_else(|e| panic!("write {path}={value}: {e}"));
        Self { path, original }
    }

    fn restore(self) {
        fs::write(self.path, self.original.trim())
            .unwrap_or_else(|e| panic!("restore {}: {e}", self.path));
    }
}

fn tcp_ext(name: &str) -> u64 {
    let text = fs::read_to_string("/proc/net/netstat").expect("read /proc/net/netstat");
    let mut lines = text.lines();
    while let (Some(names), Some(values)) = (lines.next(), lines.next()) {
        if !names.starts_with("TcpExt:") || !values.starts_with("TcpExt:") {
            continue;
        }
        for (key, value) in names.split_whitespace().skip(1).zip(values.split_whitespace().skip(1))
        {
            if key == name {
                return value.parse().expect("the TcpExt value is an integer");
            }
        }
    }
    panic!("TcpExt has no {name} counter")
}

fn bpffs_entries() -> BTreeSet<PathBuf> {
    fn visit(root: &Path, directory: &Path, entries: &mut BTreeSet<PathBuf>) {
        for entry in
            fs::read_dir(directory).unwrap_or_else(|e| panic!("read {}: {e}", directory.display()))
        {
            let entry = entry.expect("read a bpffs directory entry");
            let path = entry.path();
            entries.insert(path.strip_prefix(root).expect("entry is below bpffs").to_path_buf());
            if entry.file_type().expect("the bpffs entry has a type").is_dir() {
                visit(root, &path, entries);
            }
        }
    }
    let root = Path::new(BPF_FS);
    let mut entries = BTreeSet::new();
    visit(root, root, &mut entries);
    entries
}

fn captured_event(
    event: GuestIoEvent,
    destination: SocketAddrV4,
    cookie: SocketCookie,
) -> CapturedConn {
    let GuestIoEvent::OutboundCaptured { child, original_destination, cookie: observed_cookie } =
        event
    else {
        panic!("expected OutboundCaptured, got {event:?}");
    };
    assert_eq!(original_destination, destination, "the original destination is exact");
    assert_eq!(observed_cookie, cookie, "the application SO_COOKIE is exact");
    child
}

fn captured(
    io: &mut KernelGuestOwnerIo,
    destination: SocketAddrV4,
    cookie: SocketCookie,
) -> CapturedConn {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match io.next_event(Some(deadline)).expect("the kernel event wait succeeds") {
            Some(event @ GuestIoEvent::OutboundCaptured { .. }) => {
                return captured_event(event, destination, cookie);
            }
            Some(GuestIoEvent::SocketReleased { .. }) => {}
            Some(other) => panic!("unexpected event before capture: {other:?}"),
            None => panic!("no capture of {destination} within five seconds"),
        }
    }
}

fn no_capture_replay(io: &mut KernelGuestOwnerIo) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match io.next_event(Some(deadline)).expect("the kernel event wait succeeds") {
            Some(GuestIoEvent::OutboundCaptured { .. }) => {
                panic!("the consumed child correlation replayed")
            }
            Some(_) => {}
            None => return,
        }
    }
}

fn drive_without_capture_until_child_absent(
    io: &mut KernelGuestOwnerIo,
    maps: CorrelationMaps,
    child_cookie: u64,
    what: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while maps.child_state(child_cookie).is_some() {
        assert!(
            Instant::now() < deadline,
            "the production adapter did not consume {what} child {child_cookie}: {:?}",
            maps.child_state(child_cookie)
        );
        if let Some(GuestIoEvent::OutboundCaptured { .. }) = io
            .next_event(Some(Instant::now() + Duration::from_millis(100)))
            .expect("the rejected-child owner wait succeeds")
        {
            panic!("{what} emitted an outbound capture");
        }
    }
}

fn wait_active_absent(maps: CorrelationMaps, socket: &ApplicationSocket) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while maps.active_contains(socket.raw_fd()) {
        assert!(Instant::now() < deadline, "active socket storage survived peer input");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn assert_one_child_row(maps: CorrelationMaps, case: CorrelationCase) {
    assert_eq!(
        maps.child_entries(),
        1,
        "{case:?}: the real passive producer owns exactly one transient child row"
    );
}

fn cleanup_capture(
    io: &mut KernelGuestOwnerIo,
    maps: CorrelationMaps,
    app: &ApplicationSocket,
    child: CapturedConn,
) {
    assert_eq!(maps.child_entries(), 0, "Ready is taken once");
    app.send_probe();
    wait_active_absent(maps, app);
    io.abort(child.into());
    assert_eq!(maps.child_entries(), 0, "abort leaves no child row");
}

fn finish_capture(
    io: &mut KernelGuestOwnerIo,
    maps: CorrelationMaps,
    app: &ApplicationSocket,
    child: CapturedConn,
) {
    cleanup_capture(io, maps, app, child);
    no_capture_replay(io);
}

fn socket_fd_count() -> usize {
    fs::read_dir("/proc/self/fd")
        .expect("read this process's fd table")
        .filter_map(Result::ok)
        .filter(|entry| {
            fs::read_link(entry.path())
                .is_ok_and(|target| target.to_string_lossy().starts_with("socket:["))
        })
        .count()
}

fn tcp_state(local_port: u16, remote_port: u16) -> Option<u8> {
    fs::read_to_string("/proc/net/tcp")
        .expect("read the guest IPv4 TCP table")
        .lines()
        .skip(1)
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            let _slot = fields.next()?;
            let local = fields.next()?;
            let remote = fields.next()?;
            let state = fields.next()?;
            let port = |address: &str| {
                address.rsplit_once(':').and_then(|(_, port)| u16::from_str_radix(port, 16).ok())
            };
            (port(local) == Some(local_port) && port(remote) == Some(remote_port))
                .then(|| u8::from_str_radix(state, 16).expect("TCP state is hexadecimal"))
        })
}

fn thread_is_sleeping(tid: i32) -> bool {
    fs::read_to_string(format!("/proc/self/task/{tid}/status"))
        .expect("read the owner-wait thread status")
        .lines()
        .find_map(|line| line.strip_prefix("State:"))
        .is_some_and(|state| state.trim_start().starts_with('S'))
}

fn fast_open_capture_through_owner_pending(
    io: &mut KernelGuestOwnerIo,
    maps: CorrelationMaps,
    destination: SocketAddrV4,
    overflow_destination: SocketAddrV4,
) -> (ApplicationSocket, CapturedConn, usize) {
    let app = ApplicationSocket::new();
    let cookie = app.cookie();
    let sockets_before_owner = socket_fd_count();
    let (tid_tx, tid_rx) = std::sync::mpsc::sync_channel(0);
    let (event_tx, event_rx) = std::sync::mpsc::sync_channel(1);

    let child = std::thread::scope(|scope| {
        let waiter = scope.spawn(|| {
            tid_tx.send(nix::unistd::gettid().as_raw()).expect("publish owner-wait tid");
            let end = Instant::now() + Duration::from_secs(7);
            let event = loop {
                let event = io
                    .next_event(Some(Instant::now() + Duration::from_secs(1)))
                    .expect("the owner wait succeeds");
                if event.is_some() || Instant::now() >= end {
                    break event;
                }
            };
            event_tx.send(event).expect("publish the owner event");
        });
        let tid = tid_rx.recv().expect("the owner wait thread starts");
        let sleeping_deadline = Instant::now() + Duration::from_secs(1);
        while !thread_is_sleeping(tid) {
            assert!(Instant::now() < sleeping_deadline, "the owner did not enter its event wait");
            std::thread::yield_now();
        }

        let gate = TfoFinalAckGate::install();
        app.select_tfo_gate();

        // The ordinary socket produces every packet. The private PRIO/NETEM
        // class delays only this socket's final ACK, leaving the real TFO
        // child in SYN_RECV while default-priority overflow bypasses the gate.
        app.fast_open(destination, b"data-in-syn");
        let client = app.local_addr();
        let pending_deadline = Instant::now() + Duration::from_millis(150);
        loop {
            match event_rx.try_recv() {
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Ok(event) => {
                    panic!("Fast Open emitted before the Pending hold was observed: {event:?}")
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    panic!("the owner wait ended without an event")
                }
            }
            let child_entries = maps.child_entries();
            let child_tcp_state = tcp_state(GUEST_TCP_INTAKE_PORT, client.port());
            let owner_socket_count = socket_fd_count();
            if child_entries == 1
                && child_tcp_state == Some(3)
                && owner_socket_count == sockets_before_owner + 1
            {
                break;
            }
            assert!(
                Instant::now() < pending_deadline,
                "the production owner did not hold the real SYN_RECV Fast Open child: \
                 child_entries={child_entries}, tcp_state={child_tcp_state:?}, \
                 socket_fds={owner_socket_count}, before_owner={sockets_before_owner}"
            );
            std::thread::yield_now();
        }

        let overflow = ApplicationSocket::new();
        overflow.connect(overflow_destination);
        overflow.expect_reset();
        drop(overflow);
        assert!(
            matches!(event_rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)),
            "the over-capacity refusal completes while Fast Open is still adapter-held Pending",
        );
        assert_eq!(maps.child_entries(), 1, "the held Pending row survives the refusal");
        assert_eq!(
            socket_fd_count(),
            sockets_before_owner + 1,
            "the owner still holds exactly the Pending child after refusing overflow",
        );

        let event = event_rx
            .recv_timeout(Duration::from_secs(4))
            .unwrap_or_else(|error| {
                let child_entries = maps.child_entries();
                let child_tcp_state = tcp_state(GUEST_TCP_INTAKE_PORT, client.port());
                let owner_socket_count = socket_fd_count();
                let active = maps.active_contains(app.raw_fd());
                panic!(
                    "the naturally released final ACK did not wake the held Pending child: \
                     {error}; child_entries={child_entries}, tcp_state={child_tcp_state:?}, \
                     socket_fds={owner_socket_count}, before_owner={sockets_before_owner}, \
                     active={active}"
                )
            })
            .expect("the held Pending child becomes an event");
        gate.remove();
        waiter.join().expect("the owner wait thread does not panic");
        captured_event(event, destination, cookie)
    });
    assert_eq!(maps.child_entries(), 0, "Ready is taken once after the Pending hold");
    assert_eq!(
        socket_fd_count(),
        sockets_before_owner + 1,
        "the adapter-held Pending socket becomes the emitted CapturedConn without replacement"
    );
    (app, child, sockets_before_owner)
}

fn fill_emitted_pending_places(
    io: &mut KernelGuestOwnerIo,
    destination: SocketAddrV4,
) -> Vec<(ApplicationSocket, CapturedConn)> {
    let count = usize::try_from(GUEST_TCP_PENDING_MAX - 1).expect("the pending bound fits usize");
    let mut held = Vec::with_capacity(count);
    for _ in 0..count {
        let app = ApplicationSocket::new();
        let cookie = app.cookie();
        app.connect(destination);
        let child = captured(io, destination, cookie);
        held.push((app, child));
    }
    held
}

fn assert_capture_refused_at_combined_max(io: &mut KernelGuestOwnerIo, destination: SocketAddrV4) {
    let app = ApplicationSocket::new();
    app.connect(destination);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match io.next_event(Some(deadline)).expect("the capacity-bound owner wait succeeds") {
            Some(GuestIoEvent::OutboundCaptured { .. }) => {
                panic!("a capture past the combined pending maximum was emitted")
            }
            Some(GuestIoEvent::SocketReleased { .. }) => {}
            Some(event) => panic!("unexpected event at the combined pending maximum: {event:?}"),
            None => break,
        }
    }
    app.expect_reset();
}

fn release_emitted_pending_places(
    io: &mut KernelGuestOwnerIo,
    held: Vec<(ApplicationSocket, CapturedConn)>,
) {
    for (app, child) in held {
        io.abort(child.into());
        drop(app);
    }
}

#[derive(Debug, Clone, Copy)]
enum CorrelationCase {
    DirectUncorrelated,
    DirectFastOpenRejected,
    Ordinary,
    ImmediateApplicationClose,
    ForcedSyncookie,
    DataInSynFastOpen,
}

impl CorrelationCase {
    const ALL: [Self; 6] = [
        Self::DirectUncorrelated,
        Self::DirectFastOpenRejected,
        Self::Ordinary,
        Self::ImmediateApplicationClose,
        Self::ForcedSyncookie,
        Self::DataInSynFastOpen,
    ];

    fn destination(self) -> SocketAddrV4 {
        let suffix = match self {
            Self::DirectUncorrelated => 1,
            Self::DirectFastOpenRejected => 2,
            Self::Ordinary => 3,
            Self::ImmediateApplicationClose => 4,
            Self::ForcedSyncookie => 5,
            Self::DataInSynFastOpen => 6,
        };
        SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, suffix), 40_000 + u16::from(suffix))
    }
}

fn run_direct_fast_open_rejection(
    io: &mut KernelGuestOwnerIo,
    maps: CorrelationMaps,
    token: GuestTransportToken,
) {
    let restore = SysctlRestore::set(TCP_FASTOPEN_SYSCTL, "3");
    let gate = TfoFinalAckGate::install();
    let candidate_before = maps.candidate_run_count();
    let app = ApplicationSocket::new();
    app.select_tfo_gate();
    app.fast_open(
        SocketAddrV4::new(token.workload_addr, GUEST_TCP_INTAKE_PORT),
        b"direct-fast-open",
    );
    assert!(!maps.active_contains(app.raw_fd()), "direct Fast Open has no active metadata");
    let child_cookie = wait_only_child_cookie(maps, "direct Fast Open");
    let rejected = wait_child_phase(maps, child_cookie, CorrelationPhase::Rejected);
    assert_eq!(
        rejected.meta,
        CorrelationMeta { application_cookie: 0, destination_ip: 0, destination_port: 0, pad: 0 },
        "the direct Fast Open terminal rejection carries no fabricated identity"
    );
    assert!(
        maps.candidate_run_count() > candidate_before,
        "the live direct Fast Open traversed TcpCorrelationCandidate before Rejected"
    );
    app.bypass_tfo_gate();
    gate.remove();
    let terminal_before = maps.candidate_run_count();
    app.send_probe();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        maps.candidate_run_count(),
        terminal_before,
        "established data after direct rejection never invokes TcpCorrelationCandidate"
    );
    assert_eq!(
        maps.child_state(child_cookie),
        Some(rejected),
        "established data cannot regress direct terminal rejection"
    );
    drive_without_capture_until_child_absent(io, maps, child_cookie, "rejected direct Fast Open");
    app.expect_reset();
    wait_child_absent(maps, child_cookie);
    no_capture_replay(io);
    restore.restore();
}

fn terminal_and_delete_are_monotonic(
    io: &mut KernelGuestOwnerIo,
    maps: CorrelationMaps,
    destination: SocketAddrV4,
) {
    let gate = TfoFinalAckGate::install();
    let app = ApplicationSocket::new();
    let cookie = app.cookie();
    let expected = expected_correlation_meta(cookie, destination);
    app.select_tfo_gate();
    let candidate_before = maps.candidate_run_count();
    app.fast_open(destination, b"terminal-monotonic");
    let child_cookie = wait_only_child_cookie(maps, "terminal Fast Open");
    let pending = wait_child_phase(maps, child_cookie, CorrelationPhase::Pending);
    assert_eq!(
        pending.meta,
        CorrelationMeta { application_cookie: 0, destination_ip: 0, destination_port: 0, pad: 0 },
        "Pending carries no identity before candidate validation"
    );

    let (progress_tx, progress_rx) = std::sync::mpsc::channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(0);
    let (ready, cohort_sends) = std::thread::scope(|scope| {
        let sender_app = &app;
        let sender = scope.spawn(move || {
            let mut sent = 0usize;
            loop {
                sender_app.send_probe();
                sent += 1;
                progress_tx.send(sent).expect("publish real established-send progress");
                match ready_rx.try_recv() {
                    Ok(()) => {
                        sender_app.send_probe();
                        sent += 1;
                        progress_tx
                            .send(sent)
                            .expect("publish established-send progress after Ready");
                        return sent;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return sent,
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });

        assert_eq!(
            progress_rx
                .recv_timeout(Duration::from_millis(250))
                .expect("the parallel established-send cohort makes progress while Pending"),
            1,
            "the cohort reports its first completed socket send"
        );
        assert_eq!(
            maps.child_state(child_cookie),
            Some(pending),
            "a real established send completes while the final ACK remains delayed and Pending"
        );

        // The background socket keeps sending while this thread releases the
        // real delayed ACK and observes promotion. It is stopped only after
        // Ready is read, then performs one final send before joining, so the
        // same live cohort demonstrably spans both sides of the transition.
        gate.remove();
        let ready = wait_child_phase(maps, child_cookie, CorrelationPhase::Ready);
        let sends_when_ready = progress_rx.try_iter().last().unwrap_or(1);
        ready_tx.send(()).expect("tell the established-send cohort Ready was observed");
        let sent = sender.join().expect("the established-send cohort joins");
        assert!(
            sent > sends_when_ready,
            "the same real socket cohort progresses after Ready is observed"
        );
        (ready, sent)
    });
    app.bypass_tfo_gate();
    assert_eq!(ready.meta, expected, "the terminal row has the exact application identity");
    assert_eq!(
        maps.candidate_run_count(),
        candidate_before + 1,
        "only the gated Fast Open final ACK, not {cohort_sends} established sends, traverses \
         TcpCorrelationCandidate"
    );

    let established_before = maps.candidate_run_count();
    app.send_probes(32);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        maps.candidate_run_count(),
        established_before,
        "established application data never invokes TcpCorrelationCandidate"
    );
    assert_eq!(
        maps.child_state(child_cookie),
        Some(ready),
        "established sends cannot regress or replace the terminal row"
    );

    let child = captured(io, destination, cookie);
    assert_eq!(maps.child_state(child_cookie), None, "terminal consumption deletes the row once");
    let deleted_before = maps.candidate_run_count();
    app.send_probe();
    wait_active_absent(maps, &app);
    assert_eq!(
        maps.candidate_run_count(),
        deleted_before,
        "established data after deletion never invokes the handshake candidate"
    );
    assert_eq!(maps.child_state(child_cookie), None, "established data cannot recreate deletion");
    io.abort(child.into());
    no_capture_replay(io);
}

fn preterminal_reset_deletion_is_final(
    io: &mut KernelGuestOwnerIo,
    maps: CorrelationMaps,
    destination: SocketAddrV4,
) {
    let gate = TfoFinalAckGate::install();
    let app = ApplicationSocket::new();
    let sockets_before_owner = socket_fd_count();
    app.select_tfo_gate();
    app.fast_open(destination, b"preterminal-reset");
    let client = app.local_addr();
    let child_cookie = wait_only_child_cookie(maps, "preterminal-reset Fast Open");
    wait_child_phase(maps, child_cookie, CorrelationPhase::Pending);
    assert_eq!(
        tcp_state(GUEST_TCP_INTAKE_PORT, client.port()),
        Some(3),
        "the reset starts while the real child is preterminal SYN_RECV"
    );
    let pending_event = io
        .next_event(Some(Instant::now() + Duration::from_millis(250)))
        .expect("the preterminal owner wait succeeds");
    assert!(
        pending_event.is_none(),
        "the preterminal child emits no owner event before reset: {pending_event:?}"
    );
    assert_eq!(
        socket_fd_count(),
        sockets_before_owner + 1,
        "the production adapter holds the preterminal child"
    );

    // The final ACK was already classified into the private delayed class.
    // A reset generated after restoring the default class bypasses it and
    // closes the real child while its row is still Pending.
    app.bypass_tfo_gate();
    app.reset_on_close();
    drop(app);
    drive_without_capture_until_child_absent(io, maps, child_cookie, "reset preterminal Fast Open");
    gate.remove();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        maps.child_state(child_cookie),
        None,
        "the delayed candidate/promoter path cannot recreate a preterminal deletion"
    );
    no_capture_replay(io);
}

fn run_fast_open_correlation_case(
    io: &mut KernelGuestOwnerIo,
    maps: CorrelationMaps,
    destination: SocketAddrV4,
) {
    let restore = SysctlRestore::set(TCP_FASTOPEN_SYSCTL, "3");

    // Prime Linux's client cookie cache against the production intake. This
    // is another real Connect4/final-ACK traversal, not seeded correlation
    // state.
    let prime = ApplicationSocket::new();
    let prime_cookie = prime.cookie();
    prime.fast_open(destination, b"prime");
    assert_one_child_row(maps, CorrelationCase::DataInSynFastOpen);
    let prime_child = captured(io, destination, prime_cookie);
    finish_capture(io, maps, &prime, prime_child);

    terminal_and_delete_are_monotonic(io, maps, destination);
    preterminal_reset_deletion_is_final(io, maps, destination);

    let sockets_before_boundary = socket_fd_count();
    let emitted =
        fill_emitted_pending_places(io, SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 6), 40_006));
    let emitted_count = usize::try_from(GUEST_TCP_PENDING_MAX - 1).expect("fits usize");
    assert_eq!(
        socket_fd_count(),
        sockets_before_boundary + 2 * emitted_count,
        "each emitted pending place owns its application socket and CapturedConn",
    );

    let before = tcp_ext("TCPFastOpenPassive");
    let (app, child, sockets_before_owner) = fast_open_capture_through_owner_pending(
        io,
        maps,
        destination,
        SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 7), 40_007),
    );
    let after = tcp_ext("TCPFastOpenPassive");
    assert!(after > before, "the payload travelled in the SYN");
    assert_capture_refused_at_combined_max(
        io,
        SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 9), 40_009),
    );
    cleanup_capture(io, maps, &app, child);
    assert_eq!(
        socket_fd_count(),
        sockets_before_owner,
        "consuming abort releases the adapter-held/emitted Fast Open socket",
    );
    drop(app);

    let recovery_destination = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 8), 40_008);
    let recovery = ApplicationSocket::new();
    let recovery_cookie = recovery.cookie();
    recovery.connect(recovery_destination);
    let recovery_child = captured(io, recovery_destination, recovery_cookie);
    cleanup_capture(io, maps, &recovery, recovery_child);
    drop(recovery);

    release_emitted_pending_places(io, emitted);
    assert_eq!(
        socket_fd_count(),
        sockets_before_boundary,
        "every bounded admission place is released",
    );
    no_capture_replay(io);
    restore.restore();
}

fn run_correlation_case(
    case: CorrelationCase,
    token: GuestTransportToken,
    io: &mut KernelGuestOwnerIo,
    maps: CorrelationMaps,
) {
    assert_eq!(maps.child_entries(), 0, "{case:?}: clean baseline");
    match case {
        CorrelationCase::DirectUncorrelated => {
            let app = ApplicationSocket::new();
            app.connect(SocketAddrV4::new(token.workload_addr, GUEST_TCP_INTAKE_PORT));
            assert!(!maps.active_contains(app.raw_fd()), "a direct intake has no active origin");
            let deadline = Instant::now() + Duration::from_secs(2);
            match io.next_event(Some(deadline)).expect("the kernel event wait succeeds") {
                None => {}
                Some(event) => panic!("an uncorrelated intake emitted {event:?}"),
            }
            app.expect_reset();
        }
        CorrelationCase::DirectFastOpenRejected => {
            run_direct_fast_open_rejection(io, maps, token);
        }
        CorrelationCase::Ordinary => {
            let app = ApplicationSocket::new();
            let cookie = app.cookie();
            let destination = case.destination();
            app.connect(destination);
            assert!(maps.active_contains(app.raw_fd()), "ordinary active metadata is live");
            assert_one_child_row(maps, case);
            let child = captured(io, destination, cookie);
            finish_capture(io, maps, &app, child);
        }
        CorrelationCase::ImmediateApplicationClose => {
            let app = ApplicationSocket::new();
            let cookie = app.cookie();
            let destination = case.destination();
            app.connect(destination);
            assert!(maps.active_contains(app.raw_fd()), "metadata survives on the active socket");
            assert_one_child_row(maps, case);
            drop(app);
            let child = captured(io, destination, cookie);
            assert_eq!(maps.child_entries(), 0, "Ready is taken once");
            io.abort(child.into());
            assert_eq!(maps.child_entries(), 0, "close cleans the child");
            no_capture_replay(io);
        }
        CorrelationCase::ForcedSyncookie => {
            let restore = SysctlRestore::set(TCP_SYNCOOKIES_SYSCTL, "2");
            let before = tcp_ext("SyncookiesRecv");
            let app = ApplicationSocket::new();
            let cookie = app.cookie();
            let destination = case.destination();
            app.connect(destination);
            let after = tcp_ext("SyncookiesRecv");
            assert!(after > before, "the forced-syncookie producer ran");
            assert!(maps.active_contains(app.raw_fd()), "syncookie active metadata is live");
            assert_one_child_row(maps, case);
            let child = captured(io, destination, cookie);
            finish_capture(io, maps, &app, child);
            restore.restore();
        }
        CorrelationCase::DataInSynFastOpen => {
            run_fast_open_correlation_case(io, maps, case.destination());
        }
    }
    assert_eq!(maps.child_entries(), 0, "{case:?}: cleanup baseline");
}

/// TCP-CORRELATION, Tier 3: one production-composition table covers direct
/// ordinary and Fast Open rejection, ordinary and immediate-close captured
/// handshakes, forced syncookies, and real data-in-SYN Fast Open. Every
/// captured connection has the application's exact `SO_COOKIE` and original
/// destination, `Ready` is consumed once, and the Fast Open case witnesses
/// `Pending` through terminal promotion while the producer stays open. A
/// live direct producer traverses the candidate and reaches `Rejected` before
/// any later send or close. Parallel established sends invoke the candidate
/// zero times and cannot regress a terminal row; reset-close while Pending
/// deletes the row before the delayed ACK and neither candidate nor promoter
/// can recreate it. With 4,095 emitted places already held, overflow is
/// refused both while the valid child is adapter-held `Pending` and after it
/// becomes an emitted `CapturedConn`; consuming it then admits one
/// replacement, proving the same shared 4,096th place was retained and
/// released. Post-handshake input or socket lifetime removes active storage,
/// every child row returns to baseline, and setup leaves no staging bpffs
/// object during service.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn tcp_correlation_preserves_exact_identity_and_cleans_transients_for_every_handshake_path() {
    let token = in_guest();
    let script = LaneScript::connect_from_guest();
    let pins_before = bpffs_entries();
    let mut ops = KernelGuestTransportOps::new();
    configure_guest_transport(&token, &mut ops).expect("the production guest transport configures");
    assert_eq!(bpffs_entries(), pins_before, "the private staging pin is gone before service");
    let mut io = KernelGuestOwnerIo::new(ops)
        .expect("the configured guest transport exposes the correlation maps");
    let _stats = enable_stats(Stats::RunTime).expect("enable candidate invocation counts");
    let maps = CorrelationMaps::discover();
    assert_eq!(maps.child_entries(), 0, "clean child-map baseline");

    for case in CorrelationCase::ALL {
        run_correlation_case(case, token, &mut io, maps);
    }

    assert_eq!(bpffs_entries(), pins_before, "service creates no runtime pin or staging residue");
    script.say(owner_lane::DONE).expect("tell the host the correlation cell is complete");
}

// ---------------------------------------------------------------------------
// D-22: the Tier-3 half of R5-28 and R5-31 — the V-24 shape in the in-guest
// owner lane
// ---------------------------------------------------------------------------
//
// **Lane** (`overdrive_testing::guest_lane::run_in_guest_owner_cell`,
// feature-delta § *DISTILL handoff*, "R5-28 / R5-31 Tier-3 half: the in-guest
// lane", D-22). PID 1 is the scripted peer, so the owner built here — the
// production `GuestFlowOwner` over `KernelGuestOwnerIo`, configured by the
// production `configure_guest_transport` with the real `ovd_fwd` on the
// Linux 7.0 guest — is the guest's **only** flow owner. The host side is the
// lane's scripted host peer over the real vhost-vsock: it opens V_h to the
// guest's inbound port, writes the `TcpAccept` request and nothing else, and
// closes V_h in the order each cell names; it holds and plays the control
// session. The two ends step each other over the lane's script channel.
//
// **Oracles, all read in the guest** (D-22 (1)–(5)): the owner through its
// crate-private `guest_observation()` (DG-5, QPROJ-5; no new accessor); this
// cell's application listener on `workload_addr:APP_PORT` (no C, or C reset,
// and no byte); the host peer's record of the control session (checked by
// the lane, which fails the case on a message for the aborted flow); the
// kernel shape on a V_g the cell accepts itself (the raw witness). R5-28's
// "counted" (`guest_flow.control_discarded`) has no in-guest read surface:
// it is discharged by the seeded-sim half alone, as is the host half's
// discard of a late `Paired`.
//
// Each case also serves a positive control flow end to end on the same lane
// (`CONTROL_FLOW`): the application accepts its C and the flow pairs, so
// "no C" for the aborted flow is not a lane that cannot serve at all.

use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use nix::poll::PollFlags;
use overdrive_core::guest_transport::forwarder::ForwardOutcome;
use overdrive_core::guest_transport::wire::FLOW_REQUEST_LEN;
use overdrive_testing::guest_lane::{LaneScript, owner_lane};
use overdrive_testing::vsock::{VMADDR_CID_ANY, VsockListener};

use super::{
    CapturedConn, GuestAcceptPhase, GuestFlowOwner, GuestIoEvent, GuestOwnerIo,
    GuestSideObservation, KernelGuestOwnerIo, MonotonicGuestClock, SocketCookie,
};
use crate::transport::{KernelGuestTransportOps, configure_guest_transport};

/// Every lane wait's bound.
const LANE_BOUND: Duration = Duration::from_secs(30);
/// How long the cell observes that something does not happen (a C, a byte).
const QUIET: Duration = Duration::from_secs(1);
/// R5-31: no guest connection of an aborted flow is open this long after the
/// abort.
const ABORT_SETTLE: Duration = Duration::from_secs(1);

/// The host-origin flow id `seq`.
fn host_flow(seq: u32) -> FlowId {
    FlowId::new(FlowOrigin::Host, seq).expect("a small host-origin sequence")
}

/// The application side of the cell: accepts every C on
/// `workload_addr:APP_PORT` on its own thread, as an application would.
struct App {
    accepted: Arc<Mutex<Vec<TcpStream>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl App {
    fn start(addr: SocketAddrV4) -> Self {
        let listener = TcpListener::bind(addr).expect("the application listens");
        listener.set_nonblocking(true).expect("non-blocking listener");
        let accepted = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let accepted = Arc::clone(&accepted);
            let stop = Arc::clone(&stop);
            move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => accepted.lock().expect("app lock").push(stream),
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(e) => panic!("the application's accept failed: {e}"),
                    }
                }
            }
        });
        Self { accepted, stop, thread: Some(thread) }
    }

    fn count(&self) -> usize {
        self.accepted.lock().expect("app lock").len()
    }

    fn take_all(&self) -> Vec<TcpStream> {
        std::mem::take(&mut *self.accepted.lock().expect("app lock"))
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// How an application connection C ended, read with a bound.
#[derive(Debug, PartialEq, Eq)]
enum AppEnd {
    /// `read` failed `ECONNRESET`.
    Reset,
    /// A clean end of stream.
    Eof,
    /// Bytes arrived (the count), or another error.
    Other(String),
}

/// Reads C until its end within `bound`: the bytes it carried and how it
/// ended.
fn app_end(c: &TcpStream, bound: Duration) -> (usize, AppEnd) {
    c.set_read_timeout(Some(bound)).expect("timeout");
    let mut r = c;
    let mut got = 0usize;
    let mut buf = [0u8; 4096];
    let started = Instant::now();
    loop {
        match r.read(&mut buf) {
            Ok(0) => return (got, AppEnd::Eof),
            Ok(n) => got += n,
            Err(e) if e.kind() == ErrorKind::ConnectionReset => return (got, AppEnd::Reset),
            Err(e) => {
                // Reading SO_ERROR can consume a pending socket error, so do
                // it only after the required read oracle has already failed.
                // The tuple and status flags distinguish the accepted C from
                // another live application connection without changing the
                // successful Reset/Eof observations.
                let status_flags = app_socket_status_flags(c);
                return (
                    got,
                    AppEnd::Other(format!(
                        "{e}; fd={}; local={:?}; peer={:?}; status_flags={status_flags:?}; \
                         nonblocking={}; so_error={:?}; read_elapsed={:?}",
                        c.as_raw_fd(),
                        c.local_addr(),
                        c.peer_addr(),
                        status_flags
                            .as_ref()
                            .is_ok_and(|flags| *flags & nix::libc::O_NONBLOCK != 0),
                        c.take_error(),
                        started.elapsed(),
                    )),
                );
            }
        }
    }
}

#[allow(
    unsafe_code,
    reason = "the source-local diagnostic calls F_GETFL on the live accepted application fd"
)]
fn app_socket_status_flags(c: &TcpStream) -> std::io::Result<i32> {
    // SAFETY: F_GETFL reads the status flags of the live descriptor and takes
    // no third argument.
    let flags = unsafe { nix::libc::fcntl(c.as_raw_fd(), nix::libc::F_GETFL) };
    if flags < 0 { Err(std::io::Error::last_os_error()) } else { Ok(flags) }
}

/// The guest's only flow owner, configured by the production setup, with
/// the lane's script channel and the application.
struct OwnerCell {
    owner: GuestFlowOwner,
    script: LaneScript,
    app: App,
}

impl OwnerCell {
    /// Configures the guest transport with the production setup, constructs
    /// the production owner over the kernel seam, starts the application,
    /// and pumps the owner until the lane's host has accepted its control
    /// session.
    fn start() -> Self {
        let token = in_guest();
        let script = LaneScript::connect_from_guest();
        let mut ops = KernelGuestTransportOps::new();
        configure_guest_transport(&token, &mut ops).expect(
            "the production setup configures the guest transport (the cell is the guest's only \
             owner: PID 1 is the scripted peer)",
        );
        let owner = GuestFlowOwner::new(
            token,
            Box::new(
                KernelGuestOwnerIo::new(ops)
                    .expect("the configured guest transport exposes the correlation maps"),
            ),
            Arc::new(MonotonicGuestClock),
        );
        let app = App::start(SocketAddrV4::new(token.workload_addr, owner_lane::APP_PORT));
        let mut cell = Self { owner, script, app };
        cell.pump_until_heard(owner_lane::SESSION_ACCEPTED);
        assert!(cell.observe().session, "the owner holds its control session");
        cell
    }

    fn observe(&self) -> GuestSideObservation {
        self.owner.guest_observation()
    }

    /// One owner step (the next event, or the 1 s audit tick at most).
    fn dispatch(&mut self) {
        self.owner.dispatch_next().unwrap_or_else(|e| panic!("the guest owner failed: {e}"));
    }

    fn say(&self, line: &str) {
        self.script.say(line).expect("the script channel is open");
    }

    /// Pumps the owner until the host's next script line, which must be
    /// `line`.
    fn pump_until_heard(&mut self, line: &str) {
        let deadline = Instant::now() + LANE_BOUND;
        loop {
            if let Some(heard) = self.script.hear_within(Duration::ZERO).expect("script channel") {
                assert_eq!(heard, line, "the host's script is out of step");
                return;
            }
            assert!(Instant::now() < deadline, "no {line} from the host within {LANE_BOUND:?}");
            self.dispatch();
        }
    }

    /// Stops dispatching the owner, tells the host (`PAUSED`), and waits for
    /// the host's `line` without dispatching: a `V_h` the host opens
    /// meanwhile stays queued at the guest (its V_g unaccepted) until the
    /// owner runs again.
    fn pause_until_heard(&mut self, line: &str) {
        self.say(owner_lane::PAUSED);
        let heard = self.script.hear(line, LANE_BOUND).expect("script channel");
        assert_eq!(heard, line, "the host's script is out of step");
    }

    /// Pumps the owner until `done` holds of its observation.
    fn pump_until(
        &mut self,
        what: &str,
        bound: Duration,
        done: impl Fn(&GuestSideObservation) -> bool,
    ) -> GuestSideObservation {
        let deadline = Instant::now() + bound;
        loop {
            let obs = self.observe();
            if done(&obs) {
                return obs;
            }
            assert!(Instant::now() < deadline, "{what} not reached within {bound:?}: {obs:?}");
            self.dispatch();
        }
    }

    /// Pumps the owner for `d` (whole dispatches).
    fn pump_for(&mut self, d: Duration) {
        let end = Instant::now() + d;
        while Instant::now() < end {
            self.dispatch();
        }
    }

    /// The positive control: the host opens `CONTROL_FLOW` and keeps V_h
    /// open; the application accepts its C and the flow pairs; the host then
    /// closes V_h and C is reset with no byte. Proves this lane serves a flow
    /// end to end, so an aborted flow's "no C" has teeth.
    fn serve_control_flow(&mut self) {
        self.serve_control_flow_checking(None);
    }

    /// [`Self::serve_control_flow`] after a flow the host aborted, whose V_g
    /// was queued BEFORE the control flow's V_h was opened (D-22 (1), (2),
    /// (5)). The listener accepts in order, so the control flow's pairing
    /// shows the owner has accepted and processed the aborted flow's V_g:
    /// only then, after `ABORT_SETTLE` more of dispatching, is the aborted
    /// flow checked: the owner holds nothing of it, it attached no pair (the
    /// control flow's is the only attach since `attached_before`, the owner's
    /// attach count before either flow), and every application connection
    /// that arrived before the control flow's is reset and carried no byte.
    /// Checking before the owner has run would pass trivially while it is
    /// paused.
    fn serve_control_flow_after_abort(&mut self, aborted: FlowId, attached_before: u64) {
        self.serve_control_flow_checking(Some((aborted, attached_before)));
    }

    fn serve_control_flow_checking(&mut self, aborted: Option<(FlowId, u64)>) {
        let flow = host_flow(owner_lane::CONTROL_FLOW);
        let before = self.app.count();
        self.pump_until_heard(owner_lane::REQUESTED);
        self.pump_until("the control flow paired", LANE_BOUND, |o| {
            matches!(o.accepted.get(&flow), Some(GuestAcceptPhase::Paired { .. }))
        });
        let mut strays = if let Some((aborted, attached_before)) = aborted {
            self.pump_for(ABORT_SETTLE);
            let obs = self.observe();
            assert!(
                !obs.accepted.contains_key(&aborted) && !obs.flows.contains_key(&aborted),
                "the aborted flow is gone once the owner has processed it: {obs:?}"
            );
            assert_eq!(
                obs.forwarder_attached,
                attached_before + 1,
                "only the control flow attached a pair: {obs:?}"
            );
            self.app.take_all()
        } else {
            Vec::new()
        };
        let deadline = Instant::now() + LANE_BOUND;
        while self.app.count() + strays.len() == before {
            assert!(Instant::now() < deadline, "the control flow's C never reached the app");
            self.dispatch();
        }
        strays.extend(self.app.take_all());
        let c = strays.pop().expect("the control flow's C");
        for stray in strays {
            let (bytes, end) = app_end(&stray, QUIET);
            assert_eq!(
                (bytes, end),
                (0, AppEnd::Reset),
                "an aborted flow's C is reset, empty (the control flow's C is the last)"
            );
        }
        self.say(owner_lane::PAIRED);
        self.pump_until_heard(owner_lane::CLOSED);
        let (bytes, end) = app_end(&c, LANE_BOUND);
        assert_eq!((bytes, end), (0, AppEnd::Reset), "the host's close resets the control C");
        self.pump_until("the control flow ended", LANE_BOUND, |o| {
            !o.accepted.contains_key(&flow) && !o.flows.contains_key(&flow)
        });
    }
}

/// `POLLRDHUP` (`include/uapi/asm-generic/poll.h`); nix 0.30's `PollFlags`
/// declares no constant for it, so the libc bit is carried as-is.
const POLLRDHUP: PollFlags = PollFlags::from_bits_retain(nix::libc::POLLRDHUP);

#[allow(
    unsafe_code,
    reason = "nix 0.30 drops the complete revents value when Linux returns its undeclared POLLRDHUP bit; this source-local kernel witness retains the raw c_short"
)]
fn poll_revents(fd: &impl std::os::fd::AsFd, events: PollFlags) -> PollFlags {
    let mut descriptor =
        nix::libc::pollfd { fd: fd.as_fd().as_raw_fd(), events: events.bits(), revents: 0 };
    // SAFETY: `descriptor` contains one live borrowed fd and points to one
    // writable `pollfd` for the duration of the call.
    let result = unsafe { nix::libc::poll(&raw mut descriptor, 1, 0) };
    assert_eq!(result, 1, "the ended V_g is immediately poll-ready");
    PollFlags::from_bits_retain(descriptor.revents)
}

/// V-24's kernel shape on a V_g the cell accepted (D-22 (4)): the request
/// read succeeds, then EOF, `POLLRDHUP`, and `EPIPE` on the first write.
fn assert_v24_shape(v_g: &mut File) {
    let mut request = [0u8; FLOW_REQUEST_LEN];
    v_g.read_exact(&mut request).expect("the request read itself succeeds");
    let mut more = [0u8; 16];
    assert_eq!(v_g.read(&mut more).expect("a read after the request"), 0, "then EOF");
    let end = poll_revents(v_g, POLLRDHUP);
    assert!(end.contains(POLLRDHUP), "V_g reports POLLRDHUP: {end:?}");
    let written = nix::sys::socket::send(v_g.as_raw_fd(), b"x", MsgFlags::MSG_NOSIGNAL);
    assert_eq!(written, Err(nix::errno::Errno::EPIPE), "the first write fails EPIPE");
}

/// D-22 (4): the V-24 shape re-witnessed in the lane, on a V_g this cell
/// accepts with its own raw listener under the host's script: in the order
/// "V_g still queued when the host closes" and in the order "V_g accepted
/// before the host writes and closes", the request read succeeds, then EOF,
/// `POLLRDHUP` and `EPIPE` on the first write. The owner-path cells
/// rest on this kernel fact, observed here on the lane's own kernel.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_v24_kernel_shape_holds_in_both_accept_orders() {
    let _token = in_guest();
    let mut script = LaneScript::connect_from_guest();
    let raw = VsockListener::bind(VMADDR_CID_ANY, owner_lane::RAW_PORT).expect("the raw listener");
    script.say(owner_lane::LISTENING).expect("script");

    // Order 1: V_g still queued when the host closes V_h.
    assert_eq!(script.hear("RAW-QUEUED", LANE_BOUND).expect("script"), owner_lane::RAW_QUEUED);
    let (v_g, _) = raw.accept_within(LANE_BOUND).expect("accept").expect("the queued V_g");
    assert_v24_shape(&mut File::from(std::os::fd::OwnedFd::from(v_g)));

    // Order 2: V_g accepted before the host writes the request and closes.
    assert_eq!(
        script.hear("RAW-CONNECTED", LANE_BOUND).expect("script"),
        owner_lane::RAW_CONNECTED
    );
    let (v_g, _) = raw.accept_within(LANE_BOUND).expect("accept").expect("the connected V_g");
    script.say(owner_lane::RAW_ACCEPTED).expect("script");
    assert_eq!(script.hear("CLOSED", LANE_BOUND).expect("script"), owner_lane::CLOSED);
    assert_v24_shape(&mut File::from(std::os::fd::OwnedFd::from(v_g)));
    script.say(owner_lane::DONE).expect("script");
}

/// R5-31, quiescence shape: the host aborted a host-opened flow before the
/// guest accepted V_g (request written, V_h closed, V_g still queued). The
/// production guest owner ends the flow: it never pairs, no C reaches the
/// application and no byte; within 1 s the owner holds nothing of it. The
/// lane checks the control session carries no message for the flow. The
/// positive control flow is then served end to end.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_flow_aborted_before_its_v_g_is_accepted_ends_with_no_application_connection() {
    let mut cell = OwnerCell::start();
    let attached = cell.observe().forwarder_attached;
    cell.pause_until_heard(owner_lane::QUEUED);
    cell.serve_control_flow_after_abort(host_flow(owner_lane::ABORTED_FLOW), attached);
    cell.say(owner_lane::DONE);
}

/// R5-31, session-loss shape: the host aborted the flow by control-session
/// loss — request written, V_h closed, then the session closed, V_g still
/// queued — and the guest owner reconnects. The flow ends with no C and no
/// byte; the owner holds its session again after the reconnect; the new
/// session carries no message for the flow (checked by the lane). The
/// positive control flow is then served on the new session.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_flow_aborted_by_session_loss_ends_and_the_session_is_held_again() {
    let mut cell = OwnerCell::start();
    let attached = cell.observe().forwarder_attached;
    cell.pause_until_heard(owner_lane::QUEUED_THEN_SESSION_CLOSED);
    cell.pump_until_heard(owner_lane::SESSION_ACCEPTED);
    let obs = cell.observe();
    assert!(obs.session, "the owner holds its session again after the reconnect: {obs:?}");
    cell.serve_control_flow_after_abort(host_flow(owner_lane::ABORTED_FLOW), attached);
    cell.say(owner_lane::DONE);
}

/// R5-28, the guest-accepted order: the guest owner has accepted V_g, read
/// the request and issued its connect to the application (`Accepted`) when
/// the host closes V_h having written nothing after the request. The
/// guest half ends through that close: C, if it was connected, is reset with
/// no byte; within 1 s the owner holds nothing of the flow. The lane checks
/// the guest wrote at most the flow's (late) `Paired` and answers it with
/// nothing.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_flow_the_host_closes_after_the_guest_accepted_it_ends_on_the_guest() {
    let mut cell = OwnerCell::start();
    let flow = host_flow(owner_lane::ABORTED_FLOW);
    let attached_before = cell.observe().forwarder_attached;
    cell.pump_until_heard(owner_lane::REQUESTED);
    cell.pump_until("the flow accepted", LANE_BOUND, |o| o.accepted.contains_key(&flow));
    cell.say(owner_lane::ACCEPTED);
    cell.script
        .hear("CLOSED", LANE_BOUND)
        .map(|l| assert_eq!(l, owner_lane::CLOSED))
        .expect("script");
    let settle = Instant::now() + ABORT_SETTLE;
    cell.pump_until("the flow ended", ABORT_SETTLE, |o| {
        !o.accepted.contains_key(&flow) && !o.flows.contains_key(&flow)
    });
    while Instant::now() < settle {
        cell.dispatch();
    }
    for c in cell.app.take_all() {
        assert_eq!(app_end(&c, QUIET), (0, AppEnd::Reset), "C is reset and carried no byte");
    }
    let obs = cell.observe();
    assert_eq!(
        (obs.accepted.contains_key(&flow), obs.flows.contains_key(&flow)),
        (false, false),
        "nothing of the flow is left after the owner has run for 1 s: {obs:?}"
    );
    // Whether the owner paired the flow before it saw the close depends on
    // its dispatch order, so the cell pins the invariant instead: a pair the
    // owner attached for this flow ended through the close and yielded
    // exactly one consumed end record, an abort; a flow it never attached
    // yielded none.
    let attached = obs.forwarder_attached - attached_before;
    assert_eq!(
        obs.consumed_ends, attached,
        "one consumed end record per pair the close ended, none for an unattached flow: {obs:?}"
    );
    if attached > 0 {
        let end = obs.last_consumed_end.expect("the consumed end record is kept");
        assert!(
            matches!(end.outcome, ForwardOutcome::Aborted { .. }),
            "a pair the close ended is an abort: {end:?}"
        );
    }
    // Keep the positive control out of the application's accepted-C set
    // until the aborted C has been observed. The owner is stopped while the
    // host receives this private fixture line.
    cell.say(owner_lane::PAUSED);
    cell.serve_control_flow();
    cell.say(owner_lane::DONE);
}

/// R5-28, the not-yet-held order: the host sends `Abort` for a flow the
/// guest does not hold; the owner runs for `QUIET` with it (and discards
/// it — its count is discharged by the seeded-sim half) and keeps its
/// session; then the host opens, requests and closes that flow's V_h while
/// the owner is paused (V_g still queued). The flow ends through the host's
/// close with no C and no byte. The lane checks the guest sent no reply to
/// the `Abort` and no message for the flow.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn an_abort_for_a_flow_the_guest_does_not_hold_changes_nothing_and_the_flow_ends_on_the_close() {
    let mut cell = OwnerCell::start();
    let attached = cell.observe().forwarder_attached;
    cell.pump_until_heard(owner_lane::ABORT_SENT);
    cell.pump_for(QUIET);
    let obs = cell.observe();
    assert!(obs.session, "the discarded Abort did not end the session: {obs:?}");
    assert!(
        !obs.accepted.contains_key(&host_flow(owner_lane::ABORTED_FLOW)),
        "an Abort for an unheld flow creates nothing: {obs:?}"
    );
    cell.pause_until_heard(owner_lane::QUEUED);
    cell.serve_control_flow_after_abort(host_flow(owner_lane::ABORTED_FLOW), attached);
    assert!(cell.observe().session, "the session is still held after the flow's end");
    cell.say(owner_lane::DONE);
}

/// R5-31's burst: `BURST` host-opened flows paired, then every V_h closed
/// at once by the host. Every C is reset with no byte and, 1 s after the
/// host's close, the owner holds none of them. The abort-to-reset latency
/// (from the cell hearing `CLOSED-ALL` to each C's reset) is reported as p50
/// / p99 and not asserted (P-38: p99 112 ms on 2 vCPUs, from guest
/// scheduling).
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[expect(clippy::print_stdout, reason = "R5-31's abort-to-reset latency is reported, not asserted")]
fn a_burst_of_host_aborts_resets_every_application_connection() {
    let mut cell = OwnerCell::start();
    let burst = owner_lane::BURST_FIRST..owner_lane::BURST_FIRST + owner_lane::BURST;
    cell.pump_until_heard(owner_lane::REQUESTED);
    let flows: Vec<FlowId> = burst.map(host_flow).collect();
    cell.pump_until("every burst flow paired", LANE_BOUND * 4, |o| {
        flows.iter().all(|f| matches!(o.accepted.get(f), Some(GuestAcceptPhase::Paired { .. })))
    });
    let deadline = Instant::now() + LANE_BOUND;
    while cell.app.count() < flows.len() {
        assert!(Instant::now() < deadline, "only {} Cs reached the app", cell.app.count());
        cell.dispatch();
    }
    let cs = cell.app.take_all();
    for c in &cs {
        c.set_nonblocking(true).expect("non-blocking C");
    }
    cell.say(owner_lane::ALL_PAIRED);
    cell.script
        .hear("CLOSED-ALL", LANE_BOUND)
        .map(|l| assert_eq!(l, owner_lane::CLOSED_ALL))
        .expect("script");
    let t0 = Instant::now();
    let reader = thread::spawn(move || {
        let mut ended: Vec<Option<(Duration, AppEnd, usize)>> = cs.iter().map(|_| None).collect();
        let mut buf = [0u8; 4096];
        while ended.iter().any(Option::is_none) && t0.elapsed() < LANE_BOUND {
            for (c, slot) in cs.iter().zip(ended.iter_mut()) {
                if slot.is_some() {
                    continue;
                }
                let mut r = c;
                match r.read(&mut buf) {
                    Ok(0) => *slot = Some((t0.elapsed(), AppEnd::Eof, 0)),
                    Ok(n) => *slot = Some((t0.elapsed(), AppEnd::Other(format!("{n} bytes")), n)),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                    Err(e) if e.kind() == ErrorKind::ConnectionReset => {
                        *slot = Some((t0.elapsed(), AppEnd::Reset, 0));
                    }
                    Err(e) => *slot = Some((t0.elapsed(), AppEnd::Other(e.to_string()), 0)),
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
        ended
    });
    cell.pump_until("every burst flow ended", ABORT_SETTLE, |o| {
        flows.iter().all(|f| !o.accepted.contains_key(f) && !o.flows.contains_key(f))
    });
    let ended = reader.join().expect("the reader thread");
    let mut latencies: Vec<Duration> = Vec::with_capacity(ended.len());
    for (i, slot) in ended.into_iter().enumerate() {
        let (at, end, bytes) = slot.unwrap_or_else(|| panic!("burst C {i} never ended"));
        assert_eq!((end, bytes), (AppEnd::Reset, 0), "burst C {i} is reset with no byte");
        assert!(at <= ABORT_SETTLE, "burst C {i} still open {at:?} after the abort");
        latencies.push(at);
    }
    latencies.sort();
    let p = |q: usize| latencies[(latencies.len() * q / 100).min(latencies.len() - 1)];
    println!(
        "R5-31 burst of {}: abort-to-reset p50 {:?}, p99 {:?} (reported, not asserted)",
        latencies.len(),
        p(50),
        p(99)
    );
    cell.say(owner_lane::DONE);
}
