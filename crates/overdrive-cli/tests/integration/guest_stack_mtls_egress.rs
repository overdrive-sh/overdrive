//! Tier-3 metal acceptance tests for guest-stack transparent mTLS egress.
//!
//! These tests drive the real CLI command libraries: an mTLS-composed
//! `serve`, TOML `deploy`, `workload describe`, and `stop`. The client
//! is a real Cloud Hypervisor guest whose ordinary `TcpStream` resolves and
//! dials the mesh service. No test installs an intercept, route, address, or
//! resolver entry; those effects come exclusively from the production boot and
//! allocation paths.
//!
//! The workload speaks plaintext by design. Encryption is proven on the
//! inter-agent leg-B/leg-C wire: an AF_PACKET capture observes TLS application
//! records in both directions with neither byte-distinct plaintext marker, and
//! `ss -tie` observes the kernel TLS ULP on the live connection.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::missing_const_for_fn,
    clippy::panic,
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::unwrap_used,
    reason = "Tier-3 fixtures fail fast, and Contract Shape declarations use exact mandated tokens"
)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use overdrive_cli::commands::deploy::{DeployArgs, DeployOutput, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::workload::{DescribeArgs, WorkloadDescribeOutput, describe};
use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, AllocStatusRowBody, IssuedCertSummary, ResourcesBody,
    RestartBudget, TransitionRecord, TransitionSource,
};
use overdrive_control_plane::dns_responder::frontend_addr_allocator::WORKLOAD_FRONTEND_BASE;
use overdrive_control_plane::veth_provisioner::{
    DEFAULT_CLIENT_IFACE, NetSlot, WORKLOAD_SUBNET_BASE, derive_vm_tap_plan,
    derive_workload_netns_plan, responder_addr_for_slot,
};
use overdrive_core::traits::ObservationStore as _;
use overdrive_core::{SpiffeId, aggregate::WorkloadKind};
use overdrive_netlink::nft;
use overdrive_store_local::LocalObservationStore;
use overdrive_testing::vm_fixture::VmFixture;
use serial_test::serial;
use tempfile::TempDir;

use super::vm_walking_skeleton::{
    build_spin_binary, config_path, poll_until_running, poll_until_terminal, shared_staging_root,
    stage_rootfs_with_extra_binary, vm_job_toml, write_toml,
};

const SERVICE_PORT: u16 = 18_951;
const MESH_NAME: &str = "server.svc.overdrive.local";
const REQUEST: &[u8] =
    b"GTI_REQUEST_guest_plaintext_dial_by_name_must_be_encrypted_on_peer_wire_0201";
const RESPONSE: &[u8] =
    b"GTI_RESPONSE_peer_authored_distinct_reply_returns_to_guest_byte_exact_0201";
const NON_MESH_REQUEST: &[u8] =
    b"GTI_NON_MESH_REQUEST_plaintext_passthrough_outside_workload_subnet_0201";
const NON_MESH_RESPONSE: &[u8] =
    b"GTI_NON_MESH_RESPONSE_distinct_clear_reply_reaches_guest_unchanged_0201";
const LOOPBACK_IFACE: &str = "lo";

async fn spawn_mtls_server() -> (ServeHandle, TempDir) {
    let tmp = tempfile::Builder::new()
        .prefix("gti-serve-")
        .tempdir_in(shared_staging_root())
        .expect("serve tempdir on the reflink-capable metal staging root");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create serve data dir");
    std::fs::create_dir_all(&config_dir).expect("create serve config dir");

    let args = ServeArgs {
        bind: "127.0.0.1:0".parse().expect("parse loopback bind"),
        data_dir,
        config_dir,
    };
    let handle = overdrive_cli::commands::serve::run_with_kek(
        args,
        std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
    )
    .await
    .expect("start mTLS-composed serve through the CLI composition root");
    (handle, tmp)
}

fn build_static_binary(tmp: &Path, name: &str, source: &str) -> PathBuf {
    let src = tmp.join(format!("{name}.rs"));
    std::fs::write(&src, source).expect("write static test fixture source");
    let out = tmp.join(name);
    let status = Command::new("rustc")
        .arg("--edition")
        .arg("2021")
        .arg("-C")
        .arg("opt-level=0")
        .arg("-C")
        .arg("target-feature=+crt-static")
        .arg("--target")
        .arg("x86_64-unknown-linux-musl")
        .arg("-o")
        .arg(&out)
        .arg(&src)
        .status()
        .expect("spawn rustc for static test fixture");
    assert!(status.success(), "rustc must build {name}");
    out
}

fn build_mesh_peer(tmp: &Path) -> PathBuf {
    let source = format!(
        r#"
use std::io::{{Read, Write}};
use std::net::TcpListener;
use std::time::Duration;

fn main() {{
    let listener = TcpListener::bind(("0.0.0.0", {SERVICE_PORT})).unwrap();
    loop {{
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut buf = [0_u8; 4096];
        let Ok(n) = stream.read(&mut buf) else {{ continue }};
        if &buf[..n] != {REQUEST:?} {{
            continue;
        }}
        stream.write_all(&{RESPONSE:?}).unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_secs(20));
    }}
}}
"#,
    );
    build_static_binary(tmp, "gti-peer", &source)
}

fn build_mesh_guest(tmp: &Path) -> PathBuf {
    let response_len = RESPONSE.len();
    let source = format!(
        r#"
use std::io::{{Read, Write}};
use std::net::{{TcpStream, ToSocketAddrs}};
use std::time::{{Duration, Instant}};

fn main() {{
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {{
        if let Ok(addrs) = ("{MESH_NAME}", {SERVICE_PORT}).to_socket_addrs() {{
            for addr in addrs {{
                if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) {{
                    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                    if stream.write_all(&{REQUEST:?}).is_ok()
                        && stream.flush().is_ok()
                    {{
                        let mut got = vec![0_u8; {response_len}];
                        if stream.read_exact(&mut got).is_ok() && got == {RESPONSE:?} {{
                            // Keep the authenticated data socket alive long enough for the
                            // host-side exact-tuple kTLS oracle to inspect both directions.
                            std::thread::sleep(Duration::from_secs(15));
                            return;
                        }}
                    }}
                }}
            }}
        }}
        if Instant::now() >= deadline {{
            std::process::exit(41);
        }}
        std::thread::sleep(Duration::from_millis(100));
    }}
}}
"#,
    );
    build_static_binary(tmp, "gti-mesh-guest", &source)
}

fn build_non_mesh_guest(tmp: &Path, destination: SocketAddr) -> PathBuf {
    let response_len = NON_MESH_RESPONSE.len();
    let source = format!(
        r#"
use std::io::{{Read, Write}};
use std::net::TcpStream;
use std::time::Duration;

fn main() {{
    // Give the operator-side test time to observe Running before the dial.
    std::thread::sleep(Duration::from_secs(2));
    let mut stream = TcpStream::connect("{destination}").unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
    stream.write_all(&{NON_MESH_REQUEST:?}).unwrap();
    stream.flush().unwrap();
    let mut got = vec![0_u8; {response_len}];
    if stream.read_exact(&mut got).is_err() || got != {NON_MESH_RESPONSE:?} {{
        std::process::exit(43);
    }}
}}
"#,
    );
    build_static_binary(tmp, "gti-non-mesh-guest", &source)
}

fn service_toml(peer: &Path) -> String {
    let command = toml::Value::String(peer.display().to_string()).to_string();
    format!(
        "[service]\nid = \"server\"\nreplicas = 1\n\n[[listener]]\nport = {SERVICE_PORT}\n\
         protocol = \"tcp\"\n\n[exec]\ncommand = {command}\nargs = []\n\n[resources]\n\
         cpu_milli = 100\nmemory_bytes = 67108864\n"
    )
}

const TLS_APPLICATION_DATA: u8 = 0x17;
const TLS_RECORD_HEADER_LEN: usize = 5;
const ETH_HEADER_LEN: usize = 14;
const IPV4_HEADER_LEN: usize = 20;
const ETH_P_ALL: std::os::raw::c_int = 0x0003;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FlowTuple {
    source: SocketAddrV4,
    destination: SocketAddrV4,
}

impl FlowTuple {
    fn reverse(self) -> Self {
        Self { source: self.destination, destination: self.source }
    }
}

#[derive(Debug, Clone, Default)]
struct WireScan {
    exact_tuple: Option<FlowTuple>,
    exact_records_to_peer: u64,
    exact_records_from_peer: u64,
    plaintext_hits_on_any_peer_stream: u64,
    peer_streams_observed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KtlsSocketEvidence {
    tuple: FlowTuple,
    record: String,
}

#[derive(Debug)]
struct CapturedFrame {
    observed_at: Instant,
    ifindex: u32,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct InterceptReadiness {
    observed_at: Instant,
    host_veth_ifindex: u32,
    kernel_snapshot: String,
}

#[derive(Debug, Clone, Copy)]
struct GuestBoundarySegment {
    observed_at: Instant,
    tuple: FlowTuple,
    flags: u8,
    payload_len: usize,
}

#[derive(Debug)]
struct GuestEgressAudit {
    /// Complete frame-derived universe on the exact allocation host-veth for
    /// guest-source TCP headed to the exact mesh peer address and port.
    segments: Vec<GuestBoundarySegment>,
    /// Diagnostic complement: every parsed TCP segment on that exact
    /// interface, retained so a tuple-correlation failure reports what the
    /// independent boundary actually observed.
    interface_tcp: Vec<GuestBoundarySegment>,
    first_syn: Option<GuestBoundarySegment>,
    plaintext_request_hits: u64,
}

struct WireCapture {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Vec<CapturedFrame>>>,
    port: u16,
}

impl WireCapture {
    fn start(iface: &str, port: u16) -> Self {
        let iface = std::ffi::CString::new(iface).expect("iface has no NUL");
        // SAFETY: libc retains neither pointer and the returned fd is owned here.
        let ifindex = unsafe { libc::if_nametoindex(iface.as_ptr()) };
        assert!(ifindex != 0, "resolve AF_PACKET interface index");

        Self::start_bound(ifindex, port)
    }

    /// Capture every interface from before the allocation veth exists. Binding
    /// AF_PACKET with ifindex zero is the kernel's all-interface shape; each
    /// recvfrom result retains its actual `sockaddr_ll.sll_ifindex`, allowing
    /// the audit to select the exact host-veth after production creates it.
    fn start_all() -> Self {
        Self::start_bound(0, 0)
    }

    fn start_bound(ifindex: u32, port: u16) -> Self {
        // SAFETY: create and bind one AF_PACKET socket. An ifindex of zero is
        // the documented all-interface binding used by `start_all`.
        let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW, ETH_P_ALL.to_be()) };
        assert!(fd >= 0, "AF_PACKET socket: {}", std::io::Error::last_os_error());
        let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        addr.sll_family = libc::AF_PACKET as u16;
        addr.sll_protocol = (ETH_P_ALL as u16).to_be();
        addr.sll_ifindex = i32::try_from(ifindex).expect("AF_PACKET interface index fits i32");
        let bound = unsafe {
            libc::bind(
                fd,
                std::ptr::from_ref(&addr).cast(),
                std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        };
        assert_eq!(bound, 0, "bind AF_PACKET socket: {}", std::io::Error::last_os_error());
        // SAFETY: set nonblocking on the fd owned above.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL, 0);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            let mut frames = Vec::new();
            let mut buf = vec![0_u8; 65_536];
            while !stop_thread.load(Ordering::SeqCst) {
                if let Some(frame) = receive_captured_frame(fd, &mut buf) {
                    frames.push(frame);
                } else {
                    std::thread::sleep(Duration::from_micros(200));
                }
            }
            while let Some(frame) = receive_captured_frame(fd, &mut buf) {
                frames.push(frame);
            }
            // SAFETY: close exactly the fd created for this capture.
            unsafe { libc::close(fd) };
            frames
        });
        Self { stop, handle: Some(handle), port }
    }

    fn stop_and_frames(mut self) -> Vec<CapturedFrame> {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.take().expect("wire capture thread").join().expect("wire capture join")
    }

    fn stop_and_scan(self, exact_tuple: Option<FlowTuple>) -> WireScan {
        let port = self.port;
        scan_frames(&self.stop_and_frames(), port, exact_tuple)
    }
}

fn receive_captured_frame(fd: std::os::fd::RawFd, buf: &mut [u8]) -> Option<CapturedFrame> {
    let mut packet_addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    let mut packet_addr_len = std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t;
    // SAFETY: recvfrom writes at most `buf.len()` bytes into owned storage and
    // fills the correctly-sized sockaddr_ll supplied for this AF_PACKET fd.
    let n = unsafe {
        libc::recvfrom(
            fd,
            buf.as_mut_ptr().cast(),
            buf.len(),
            0,
            std::ptr::from_mut(&mut packet_addr).cast(),
            &raw mut packet_addr_len,
        )
    };
    (n > 0).then(|| CapturedFrame {
        observed_at: Instant::now(),
        ifindex: u32::try_from(packet_addr.sll_ifindex).expect("packet ifindex is non-negative"),
        bytes: buf[..n as usize].to_vec(),
    })
}

fn scan_frames(
    frames: &[CapturedFrame],
    peer_port: u16,
    exact_tuple: Option<FlowTuple>,
) -> WireScan {
    let mut streams: BTreeMap<FlowTuple, Vec<u8>> = BTreeMap::new();
    for frame in frames {
        let Some((tuple, _flags, payload)) = parse_tcp_segment(&frame.bytes) else {
            continue;
        };
        if tuple.source.port() != peer_port && tuple.destination.port() != peer_port {
            continue;
        }
        if payload.is_empty() {
            continue;
        }
        streams.entry(tuple).or_default().extend_from_slice(payload);
    }

    let mut scan =
        WireScan { exact_tuple, peer_streams_observed: streams.len(), ..WireScan::default() };
    for bytes in streams.values() {
        // Confidentiality is checked across every byte stream touching the
        // peer port. It is intentionally independent of whether the stream can
        // first be parsed as TLS, so a clear or malformed escape cannot hide
        // outside the positive TLS classifier.
        scan.plaintext_hits_on_any_peer_stream += count_subslices(bytes, REQUEST);
        scan.plaintext_hits_on_any_peer_stream += count_subslices(bytes, RESPONSE);
    }
    if let Some(tuple) = exact_tuple {
        scan.exact_records_to_peer =
            streams.get(&tuple).map_or(0, |bytes| count_tls_application_records(bytes));
        scan.exact_records_from_peer =
            streams.get(&tuple.reverse()).map_or(0, |bytes| count_tls_application_records(bytes));
    }
    scan
}

fn parse_tcp_segment(frame: &[u8]) -> Option<(FlowTuple, u8, &[u8])> {
    if frame.len() < ETH_HEADER_LEN + IPV4_HEADER_LEN || frame.get(12..14)? != [0x08, 0x00] {
        return None;
    }
    let ip = ETH_HEADER_LEN;
    let ihl = usize::from(frame[ip] & 0x0f) * 4;
    if frame[ip] >> 4 != 4 || ihl < IPV4_HEADER_LEN || frame.get(ip + 9)? != &0x06 {
        return None;
    }
    let tcp = ip + ihl;
    if frame.len() < tcp + 20 {
        return None;
    }
    let source = u16::from_be_bytes([frame[tcp], frame[tcp + 1]]);
    let destination = u16::from_be_bytes([frame[tcp + 2], frame[tcp + 3]]);
    let source_addr = Ipv4Addr::new(frame[ip + 12], frame[ip + 13], frame[ip + 14], frame[ip + 15]);
    let destination_addr =
        Ipv4Addr::new(frame[ip + 16], frame[ip + 17], frame[ip + 18], frame[ip + 19]);
    let tcp_header = usize::from(frame[tcp + 12] >> 4) * 4;
    let payload = tcp + tcp_header;
    let ip_len = usize::from(u16::from_be_bytes([frame[ip + 2], frame[ip + 3]]));
    let payload_end = (ip + ip_len).min(frame.len());
    (tcp_header >= 20 && payload <= payload_end).then_some((
        FlowTuple {
            source: SocketAddrV4::new(source_addr, source),
            destination: SocketAddrV4::new(destination_addr, destination),
        },
        frame[tcp + 13],
        &frame[payload..payload_end],
    ))
}

fn outbound_rule_snapshot(host_veth: &str) -> Option<String> {
    let rules = nft::list_rules("overdrive-mtls", "prerouting").ok()?;
    let prefix = [nft::USERDATA_MAGIC, &[0x03]].concat();
    let matching = rules.iter().find(|rule| {
        rule.userdata.starts_with(&prefix) && rule.userdata.ends_with(host_veth.as_bytes())
    })?;
    Some(format!("{matching:?}"))
}

async fn poll_until_outbound_rule_ready(host_veth: String, budget: Duration) -> InterceptReadiness {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        if let Some(kernel_snapshot) = outbound_rule_snapshot(&host_veth) {
            let iface = std::ffi::CString::new(host_veth.as_str()).expect("iface has no NUL");
            // SAFETY: libc retains no pointer; `iface` is NUL-terminated.
            let host_veth_ifindex = unsafe { libc::if_nametoindex(iface.as_ptr()) };
            assert!(host_veth_ifindex != 0, "ready nft rule names a live host-veth");
            return InterceptReadiness {
                observed_at: Instant::now(),
                host_veth_ifindex,
                kernel_snapshot,
            };
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "actual nft rule for {host_veth} must become observable within {budget:?}"
        );
        tokio::task::yield_now().await;
    }
}

fn audit_guest_egress_boundary(
    frames: &[CapturedFrame],
    readiness: &InterceptReadiness,
    guest_addr: Ipv4Addr,
    mesh_destination: SocketAddrV4,
) -> GuestEgressAudit {
    let mut segments = Vec::new();
    let mut interface_tcp = Vec::new();
    let mut first_syn = None;
    let mut plaintext_request_hits = 0;
    for frame in frames.iter().filter(|frame| frame.ifindex == readiness.host_veth_ifindex) {
        let Some((tuple, flags, payload)) = parse_tcp_segment(&frame.bytes) else {
            continue;
        };
        interface_tcp.push(GuestBoundarySegment {
            observed_at: frame.observed_at,
            tuple,
            flags,
            payload_len: payload.len(),
        });
        if tuple.source.ip() != &guest_addr || tuple.destination != mesh_destination {
            continue;
        }
        let segment = GuestBoundarySegment {
            observed_at: frame.observed_at,
            tuple,
            flags,
            payload_len: payload.len(),
        };
        if first_syn.is_none() && flags & 0x02 != 0 && flags & 0x10 == 0 {
            first_syn = Some(segment);
        }
        plaintext_request_hits += count_subslices(payload, REQUEST);
        segments.push(segment);
    }
    GuestEgressAudit { segments, interface_tcp, first_syn, plaintext_request_hits }
}

fn count_tls_application_records(stream: &[u8]) -> u64 {
    let mut count = 0_u64;
    let mut cursor = 0_usize;
    while cursor + TLS_RECORD_HEADER_LEN <= stream.len() {
        let version = [stream[cursor + 1], stream[cursor + 2]];
        if version != [0x03, 0x03] && version != [0x03, 0x01] {
            break;
        }
        let len = usize::from(u16::from_be_bytes([stream[cursor + 3], stream[cursor + 4]]));
        if stream[cursor] == TLS_APPLICATION_DATA {
            count += 1;
        }
        let next = cursor + TLS_RECORD_HEADER_LEN + len;
        if next <= cursor || next > stream.len() {
            break;
        }
        cursor = next;
    }
    count
}

fn count_subslices(haystack: &[u8], needle: &[u8]) -> u64 {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    haystack.windows(needle.len()).filter(|window| *window == needle).count() as u64
}

fn ktls_socket_records() -> Vec<KtlsSocketEvidence> {
    let output = Command::new("ss").args(["-H", "-n", "-t", "-i", "-e"]).output().expect("run ss");
    assert!(output.status.success(), "ss failed: {}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8_lossy(&output.stdout);
    let mut records = Vec::new();
    let mut current: Option<KtlsSocketEvidence> = None;
    for line in text.lines() {
        if line.starts_with(char::is_whitespace) {
            if let Some(record) = current.as_mut() {
                record.record.push_str(line);
                record.record.push('\n');
            }
            continue;
        }
        if let Some(record) = current.take() {
            records.push(record);
        }
        let columns = line.split_whitespace().collect::<Vec<_>>();
        let Some((source, destination)) =
            columns.get(3).zip(columns.get(4)).and_then(|(source, destination)| {
                let source = source.parse::<SocketAddr>().ok()?;
                let destination = destination.parse::<SocketAddr>().ok()?;
                match (source, destination) {
                    (SocketAddr::V4(source), SocketAddr::V4(destination)) => {
                        Some((source, destination))
                    }
                    _ => None,
                }
            })
        else {
            continue;
        };
        current = Some(KtlsSocketEvidence {
            tuple: FlowTuple { source, destination },
            record: format!("{line}\n"),
        });
    }
    if let Some(record) = current {
        records.push(record);
    }
    records
}

fn record_has_bidirectional_tls13_ktls(record: &str) -> bool {
    record.contains("tcp-ulp-tls")
        && (record.contains("version: 1.3") || record.contains("version:1.3"))
        && (record.contains("rxconf: sw") || record.contains("rxconf:sw"))
        && (record.contains("txconf: sw") || record.contains("txconf:sw"))
}

async fn poll_until_ktls(port: u16, budget: Duration) -> Option<KtlsSocketEvidence> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        if let Some(record) = ktls_socket_records().into_iter().find(|record| {
            record.tuple.destination.port() == port
                && record_has_bidirectional_tls13_ktls(&record.record)
        }) {
            return Some(record);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn without_permitted_lifecycle_delta(row: &AllocStatusRowBody) -> AllocStatusRowBody {
    // Start from the COMPLETE observable row universe and normalize only the
    // five explicitly-permitted lifecycle/ephemeral fields. Every other field
    // remains in the value compared by `assert_exact_lifecycle_delta`.
    let mut complement = row.clone();
    complement.state = AllocStateWire::Pending;
    complement.reason = None;
    complement.workload_addr = None;
    complement.started_at = None;
    complement.last_transition = None;
    complement
}

fn assert_exact_lifecycle_delta(running: &AllocStatusRowBody, terminal: &AllocStatusRowBody) {
    // Exact permitted delta over the complete row:
    // state Running -> Terminated; the live backend address is retired;
    // reason/last_transition are replaced; the logical observation timestamp
    // is restamped. No other row field may change.
    assert_eq!(running.state, AllocStateWire::Running);
    assert_eq!(terminal.state, AllocStateWire::Terminated);
    assert_ne!(
        running.reason, terminal.reason,
        "the Running transition reason is replaced by the terminal stop reason"
    );
    assert!(running.workload_addr.is_some(), "Running VM carries its live guest address");
    assert_eq!(terminal.workload_addr, None, "terminal VM is no longer a live backend");
    assert!(running.started_at.is_some(), "Running row is timestamped");
    assert!(terminal.started_at.is_some(), "terminal row is timestamped");
    assert_ne!(running.started_at, terminal.started_at, "the lifecycle observation is restamped");
    assert_ne!(
        running.last_transition, terminal.last_transition,
        "the Running transition record is replaced by the terminal transition"
    );
    assert_eq!(
        without_permitted_lifecycle_delta(terminal),
        without_permitted_lifecycle_delta(running),
        "every observable row field outside the five permitted lifecycle deltas must be equal"
    );
}

async fn genuine_pre_change_contract(
    server_dir: &Path,
    submit: &DeployOutput,
    expected_guest_addr: Ipv4Addr,
) -> WorkloadDescribeOutput {
    // Take an instantaneous CoW snapshot of the production observation DB.
    // The resulting handle is independent of the HTTP response under test and
    // cannot inherit a collateral API projection change from that response.
    let observation_db = server_dir.join("data").join("observation.redb");
    let observation_snapshot = server_dir.join("observation-pre-change-contract.redb");
    let copied = Command::new("cp")
        .arg("--reflink=always")
        .arg(&observation_db)
        .arg(&observation_snapshot)
        .status()
        .expect("snapshot the production observation database");
    assert!(copied.success(), "production observation database snapshot must succeed");

    let observations = LocalObservationStore::open(&observation_snapshot)
        .expect("open the independent observation snapshot");
    let rows = observations.alloc_status_rows().await.expect("read durable allocation rows");
    let [raw] = rows.as_slice() else {
        panic!("pre-change contract requires exactly one durable allocation row; got {rows:#?}");
    };
    assert_eq!(raw.workload_id.to_string(), submit.workload_id);
    assert_eq!(raw.state, overdrive_core::traits::observation_store::AllocState::Running);
    assert_eq!(raw.kind, WorkloadKind::Job);
    assert_eq!(
        raw.workload_addr,
        Some(expected_guest_addr),
        "the durable production fact must be the VM guest /30 address"
    );
    assert!(raw.terminal.is_none());
    assert!(raw.stderr_tail.is_none());
    assert!(raw.last_terminated.is_none());

    let logical_at = format!("(c={},w={})", raw.updated_at.counter, raw.updated_at.writer);
    let state = AllocStateWire::from(raw.state);
    let last_transition = raw.reason.clone().map(|reason| TransitionRecord {
        from: None,
        to: state,
        reason,
        source: TransitionSource::Reconciler,
        at: logical_at.clone(),
    });
    let row = AllocStatusRowBody {
        alloc_id: raw.alloc_id.to_string(),
        workload_id: raw.workload_id.to_string(),
        node_id: raw.node_id.to_string(),
        state,
        reason: raw.reason.clone(),
        resources: ResourcesBody { cpu_milli: 500, memory_bytes: 134_217_728 },
        // This is the genuine pre-step API contract: every durable input is
        // projected independently, while the one field that did not exist in
        // the pre-change response is absent.
        workload_addr: None,
        started_at: Some(logical_at),
        exit_code: None,
        last_transition,
        error: raw.detail.clone(),
        restart_count: raw.restart_count,
        last_terminated: None,
    };

    let expected_spiffe = SpiffeId::for_allocation(&raw.workload_id, &raw.alloc_id);
    let issued_rows =
        observations.issued_certificate_rows().await.expect("read durable certificate audit rows");
    let issued = issued_rows
        .iter()
        .filter(|candidate| candidate.spiffe_id == expected_spiffe)
        .max_by_key(|candidate| candidate.issuance_ordinal)
        .expect("production issuance audit contains the Running allocation identity");
    let issued_certificate = IssuedCertSummary {
        serial: issued.serial.clone(),
        spiffe_id: issued.spiffe_id.clone(),
        issuer_serial: issued.issuer_serial.clone(),
        not_after: issued.not_after,
    };

    WorkloadDescribeOutput {
        workload_id: submit.workload_id.clone(),
        spec_digest: submit.spec_digest.clone(),
        allocations_total: 1,
        empty_state_message: String::new(),
        snapshot: AllocStatusResponse {
            workload_id: Some(submit.workload_id.clone()),
            spec_digest: Some(submit.spec_digest.clone()),
            replicas_desired: 1,
            replicas_running: 1,
            rows: vec![row],
            restart_budget: Some(RestartBudget { used: 0, max: 5, exhausted: false }),
            kind: Some(WorkloadKind::Job),
            vip: None,
            listeners: Vec::new(),
            issued_certificates: vec![issued_certificate],
            probes: Vec::new(),
            probe_results: Vec::new(),
        },
    }
}

fn frozen_pre_change_render_contract(out: &WorkloadDescribeOutput) -> String {
    use std::fmt::Write as _;

    let [row] = out.snapshot.rows.as_slice() else {
        panic!("pre-change render contract requires exactly one allocation row");
    };
    let [issued] = out.snapshot.issued_certificates.as_slice() else {
        panic!("pre-change render contract requires exactly one issued certificate");
    };
    assert_eq!(out.snapshot.kind, Some(WorkloadKind::Job));
    assert_eq!(row.state, AllocStateWire::Running);
    assert_eq!(row.workload_addr, None);

    let header = format!(
        "{:<8} {:<12} {:<6} {:<20} {:<10}",
        "Attempt", "State", "Exit", "Started", "Duration"
    );
    let attempt = format!(
        "{:<8} {:<12} {:<6} {:<20} {:<10}",
        1,
        "Running",
        "\u{2014}",
        row.started_at.as_deref().expect("Running row has a logical timestamp"),
        "\u{2014}"
    );
    let mut contract = format!(
        "Job '{workload_id}' (kind: Job)\n\
         Spec digest: {spec_digest}\n\
         Verdict: In progress (no terminal yet)\n\
         \n\
         {header}\n\
         {attempt}\n\
         Memory:        {memory_bytes}\n\
         Issued certificates:\n",
        workload_id = out.workload_id,
        spec_digest = out.spec_digest,
        memory_bytes = row.resources.memory_bytes,
    );
    let _ = writeln!(contract, "  serial:        {}", issued.serial);
    let _ = writeln!(contract, "    spiffe_id:     {}", issued.spiffe_id);
    let _ = writeln!(contract, "    issuer_serial: {}", issued.issuer_serial);
    let _ = writeln!(contract, "    not_after:     {}", issued.not_after);
    contract
}

async fn poll_until_issued_identity(
    cfg: &Path,
    workload_id: &str,
    alloc_id: &str,
    budget: Duration,
) -> IssuedCertSummary {
    let expected =
        SpiffeId::new(&format!("spiffe://overdrive.local/workload/{workload_id}/alloc/{alloc_id}"))
            .expect("allocation-shaped SPIFFE ID");
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let described =
            describe(DescribeArgs { id: workload_id.to_owned(), config_path: cfg.to_path_buf() })
                .await
                .expect("describe while waiting for production SVID audit row");
        if let Some(summary) = described
            .snapshot
            .issued_certificates
            .into_iter()
            .find(|summary| summary.spiffe_id == expected)
        {
            return summary;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "production IdentityMgr must issue and audit {expected} while the allocation is Running"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

struct MeshResult {
    service_running: AllocStatusRowBody,
    vm_running: AllocStatusRowBody,
    vm_terminal: AllocStatusRowBody,
    service_identity: IssuedCertSummary,
    vm_identity: IssuedCertSummary,
    scan: WireScan,
    readiness: InterceptReadiness,
    guest_egress: GuestEgressAudit,
    ktls: Option<KtlsSocketEvidence>,
}

async fn run_mesh_guest_scenario(id: &str) -> MeshResult {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("gti-mesh-")
        .tempdir_in(shared_staging_root())
        .expect("mesh fixture tempdir on metal staging root");
    let peer = build_mesh_peer(tmp.path());
    let guest = build_mesh_guest(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &guest, "gti-mesh-guest");

    let (handle, server_tmp) = spawn_mtls_server().await;
    let cfg = config_path(server_tmp.path());

    let service_spec = write_toml(server_tmp.path(), "gti-peer.toml", &service_toml(&peer));
    let service_submit = deploy(DeployArgs { spec: service_spec, config_path: cfg.clone() })
        .await
        .expect("deploy mesh peer service through commands::deploy");
    let service_state =
        poll_until_running(&cfg, &service_submit.workload_id, Duration::from_secs(30)).await;
    // A fresh composition has exactly one declared mesh name (`server`), so
    // the production smallest-free frontend allocator assigns the first usable
    // address in its named block. This is the address DNS returns to the guest;
    // it is intentionally distinct from the Service VIP/API field.
    let mesh_destination = SocketAddrV4::new(
        Ipv4Addr::from(u32::from(WORKLOAD_FRONTEND_BASE.network()).saturating_add(1)),
        SERVICE_PORT,
    );
    let service_running =
        service_state.snapshot.rows.into_iter().next().expect("one Running service allocation");
    let service_identity = poll_until_issued_identity(
        &cfg,
        &service_submit.workload_id,
        &service_running.alloc_id,
        Duration::from_secs(10),
    )
    .await;

    // This fresh composition owns slot 0 for the already-Running service and
    // therefore slot 1 for the VM deployed next. Derive both actual guest
    // boundaries before deploy so the all-interface AF_PACKET capture and
    // kernel-rule observer are live before the host-veth itself exists.
    let vm_slot = NetSlot::new(1).expect("the second allocation owns slot one");
    let responder = responder_addr_for_slot(vm_slot);
    let vm_workload = derive_workload_netns_plan(vm_slot, responder);
    let vm_guest = derive_vm_tap_plan(vm_slot, responder);
    assert!(
        outbound_rule_snapshot(&vm_workload.host_veth).is_none(),
        "fresh VM host-veth must have no stale outbound intercept rule before deploy"
    );
    let peer_wire = WireCapture::start(LOOPBACK_IFACE, SERVICE_PORT);
    let guest_wire = WireCapture::start_all();
    let readiness_task = tokio::spawn(poll_until_outbound_rule_ready(
        vm_workload.host_veth.clone(),
        Duration::from_secs(60),
    ));
    let vm_spec = write_toml(
        server_tmp.path(),
        &format!("{id}.toml"),
        &vm_job_toml(id, "/sbin/gti-mesh-guest", &[], &fixture.kernel_path, &rootfs),
    );
    let vm_submit = deploy(DeployArgs { spec: vm_spec, config_path: cfg.clone() })
        .await
        .expect("deploy VM mesh dialer through commands::deploy");
    let readiness = tokio::time::timeout(Duration::from_secs(60), readiness_task)
        .await
        .expect("kernel rule observer remains bounded")
        .expect("kernel rule observer task does not panic");
    let vm_running = poll_until_running(&cfg, &vm_submit.workload_id, Duration::from_secs(60))
        .await
        .snapshot
        .rows
        .into_iter()
        .next()
        .expect("one Running VM allocation");
    let vm_identity = poll_until_issued_identity(
        &cfg,
        &vm_submit.workload_id,
        &vm_running.alloc_id,
        Duration::from_secs(10),
    )
    .await;
    // Preserve cleanup even when the expected kTLS state never appears. RED
    // must fail as a normal assertion, not strand a VM/service until nextest's
    // process timeout kills the whole test binary.
    let ktls = poll_until_ktls(SERVICE_PORT, Duration::from_secs(25)).await;
    let terminal = poll_until_terminal(&cfg, &vm_submit.workload_id, Duration::from_secs(60)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let guest_addr = vm_running.workload_addr.expect("Running VM carries its guest address");
    assert_eq!(guest_addr, vm_guest.guest_addr, "slot-derived guest source is exact");
    let scan = peer_wire.stop_and_scan(ktls.as_ref().map(|evidence| evidence.tuple));
    let guest_frames = guest_wire.stop_and_frames();
    let guest_egress =
        audit_guest_egress_boundary(&guest_frames, &readiness, guest_addr, mesh_destination);

    stop(StopArgs { id: service_submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop mesh peer service through commands::deploy::stop");
    let _ = poll_until_terminal(&cfg, &service_submit.workload_id, Duration::from_secs(30)).await;
    handle.shutdown().await.expect("clean mTLS serve shutdown");

    let vm_terminal = terminal
        .snapshot
        .rows
        .into_iter()
        .next()
        .expect("one VM allocation row after guest completion");
    assert_eq!(
        vm_terminal.state,
        AllocStateWire::Terminated,
        "guest mesh dialer must terminate cleanly; reason={:?} error={:?}",
        vm_terminal.reason,
        vm_terminal.error
    );
    // A clean VM guest exit is represented by Terminated; unlike a crashed
    // guest, it intentionally carries no numeric exit code on the API row.
    MeshResult {
        service_running,
        vm_running,
        vm_terminal,
        service_identity,
        vm_identity,
        scan,
        readiness,
        guest_egress,
        ktls,
    }
}

/// S-GTI-01 — a real microVM resolves and dials a mesh Service by name through
/// the production guest network and transparent-mTLS path.
///
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(cgroup)]
async fn microvm_dials_a_mesh_peer_by_name_and_receives_the_reply() {
    let result = run_mesh_guest_scenario("gti-mesh-roundtrip").await;
    // Observable universe: every field of AllocStatusRowBody. The helper
    // asserts the exact lifecycle delta and compares the complete complement.
    assert_exact_lifecycle_delta(&result.vm_running, &result.vm_terminal);
    for (row, summary) in [
        (&result.service_running, &result.service_identity),
        (&result.vm_running, &result.vm_identity),
    ] {
        let expected = SpiffeId::new(&format!(
            "spiffe://overdrive.local/workload/{}/alloc/{}",
            row.workload_id, row.alloc_id
        ))
        .expect("allocation-shaped SPIFFE ID");
        assert_eq!(
            summary.spiffe_id, expected,
            "fresh production IdentityMgr composition must issue the exact per-allocation identity"
        );
    }
}

/// S-GTI-03 — the guest's plaintext request/reply is TLS 1.3 on the peer wire,
/// with kTLS installed in both directions.
///
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[serial(cgroup)]
async fn the_guests_mesh_traffic_travels_the_peer_wire_as_mtls_never_in_the_clear() {
    let result = run_mesh_guest_scenario("gti-mesh-wire").await;
    assert_eq!(
        result.vm_terminal.state,
        AllocStateWire::Terminated,
        "wire proof is coupled to a successful guest round-trip"
    );
    let ktls = result
        .ktls
        .as_ref()
        .expect("one live outbound socket record must carry TLS 1.3 ULP plus RX and TX kTLS state");
    assert_eq!(
        result.scan.exact_tuple,
        Some(ktls.tuple),
        "wire evidence must be correlated to the exact socket tuple observed by ss"
    );
    assert!(
        result.scan.exact_records_to_peer > 0,
        "the exact kTLS socket's request direction must carry TLS application_data; got {:?}",
        result.scan
    );
    assert!(
        result.scan.exact_records_from_peer > 0,
        "the exact kTLS socket's response direction must carry TLS application_data; got {:?}",
        result.scan
    );
    assert_eq!(
        result.scan.plaintext_hits_on_any_peer_stream, 0,
        "neither plaintext litmus may occur on any stream touching the peer port; got {:?}",
        result.scan
    );
    assert!(
        result.scan.peer_streams_observed > 0,
        "the unfiltered peer-port stream universe must not be empty"
    );
    assert!(
        record_has_bidirectional_tls13_ktls(&ktls.record),
        "one ss record must itself contain tcp-ulp-tls, TLS 1.3, rxconf, and txconf; got:\n{}",
        ktls.record
    );
}

/// S-GTI-04 — a destination outside the workload mesh block is classified
/// NonMesh and reached unchanged over a clear TCP connection.
///
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(cgroup)]
async fn the_same_guest_reaches_a_non_mesh_destination_in_the_clear() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("gti-non-mesh-")
        .tempdir_in(shared_staging_root())
        .expect("non-mesh fixture tempdir on metal staging root");
    let (handle, server_tmp) = spawn_mtls_server().await;
    let cfg = config_path(server_tmp.path());

    let host_ip = overdrive_control_plane::iface::resolve_iface_ipv4(DEFAULT_CLIENT_IFACE)
        .expect("resolve production single-node client interface");
    assert!(
        !WORKLOAD_SUBNET_BASE.contains(&host_ip),
        "non-mesh fixture endpoint must be outside {WORKLOAD_SUBNET_BASE}, got {host_ip}"
    );
    let listener = TcpListener::bind((host_ip, 0)).expect("bind non-mesh plaintext endpoint");
    listener.set_nonblocking(true).expect("make non-mesh accept bounded");
    let destination = listener.local_addr().expect("non-mesh endpoint address");
    let endpoint = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
                    let mut got = vec![0_u8; NON_MESH_REQUEST.len()];
                    stream.read_exact(&mut got).unwrap();
                    stream.write_all(NON_MESH_RESPONSE).unwrap();
                    stream.flush().unwrap();
                    return got;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "guest must connect to the non-mesh endpoint within 30s"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept non-mesh endpoint: {error}"),
            }
        }
    });

    let guest = build_non_mesh_guest(tmp.path(), destination);
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &guest, "gti-non-mesh-guest");
    let vm_spec = write_toml(
        server_tmp.path(),
        "gti-non-mesh.toml",
        &vm_job_toml(
            "gti-non-mesh",
            "/sbin/gti-non-mesh-guest",
            &[],
            &fixture.kernel_path,
            &rootfs,
        ),
    );
    let submit = deploy(DeployArgs { spec: vm_spec, config_path: cfg.clone() })
        .await
        .expect("deploy non-mesh VM dialer");
    let running = poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(60))
        .await
        .snapshot
        .rows
        .into_iter()
        .next()
        .expect("one Running non-mesh VM row");
    let terminal = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let endpoint_result = endpoint.join();
    let terminal = terminal.snapshot.rows.into_iter().next().expect("one terminal non-mesh VM row");
    handle.shutdown().await.expect("clean mTLS serve shutdown");

    let received = endpoint_result.expect("join non-mesh plaintext endpoint");
    assert_eq!(
        received, NON_MESH_REQUEST,
        "plain endpoint must receive the guest-authored bytes unchanged"
    );
    assert_eq!(
        terminal.state,
        AllocStateWire::Terminated,
        "the guest terminates cleanly only after receiving the distinct clear response unchanged"
    );
    // Observable universe: every field of AllocStatusRowBody. The byte oracle
    // above plus the exact lifecycle delta and complete complement close the
    // non-mesh behavior surface.
    assert_exact_lifecycle_delta(&running, &terminal);
}

/// S-GTI-07 — workload describe surfaces the VM guest address, never its
/// transit forwarding hop.
///
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(cgroup)]
async fn the_operator_sees_the_microvm_workloads_own_mesh_address_not_its_transit_hop() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("gti-describe-")
        .tempdir_in(shared_staging_root())
        .expect("describe fixture tempdir on metal staging root");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "gti-spin");
    let (handle, server_tmp) = spawn_mtls_server().await;
    let cfg = config_path(server_tmp.path());

    let vm_spec = write_toml(
        server_tmp.path(),
        "gti-describe.toml",
        &vm_job_toml("gti-describe", "/sbin/gti-spin", &[], &fixture.kernel_path, &rootfs),
    );
    let submit = deploy(DeployArgs { spec: vm_spec, config_path: cfg.clone() })
        .await
        .expect("deploy long-lived VM for describe");
    let running = poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let running_row = running.snapshot.rows.first().expect("one Running VM row");
    let _ = poll_until_issued_identity(
        &cfg,
        &submit.workload_id,
        &running_row.alloc_id,
        Duration::from_secs(10),
    )
    .await;
    let described =
        describe(DescribeArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
            .await
            .expect("workload describe direct command call");

    let slot = NetSlot::new(0).expect("first allocation owns slot zero");
    let responder = responder_addr_for_slot(slot);
    let guest = derive_vm_tap_plan(slot, responder);
    let transit = derive_workload_netns_plan(slot, responder);
    let row = described.snapshot.rows.first().expect("one Running VM row");
    assert_eq!(
        row.workload_addr,
        Some(guest.guest_addr),
        "describe must project the persisted guest NIC address"
    );
    assert_ne!(
        row.workload_addr,
        Some(transit.workload_addr),
        "describe must never substitute the transit-veth forwarding address"
    );
    // Response observable universe: the full command wrapper plus the entire
    // serialized API snapshot. The pre-change contract is independently
    // reconstructed from a CoW snapshot of the production durable inputs; it
    // never reads or clones this post-change HTTP result. Add exactly the one
    // permitted workload_addr fact, then require whole-response equality.
    let pre_change_contract =
        genuine_pre_change_contract(server_tmp.path(), &submit, guest.guest_addr).await;
    assert_eq!(described.workload_id, pre_change_contract.workload_id);
    assert_eq!(described.spec_digest, pre_change_contract.spec_digest);
    assert_eq!(described.allocations_total, pre_change_contract.allocations_total);
    assert_eq!(described.empty_state_message, pre_change_contract.empty_state_message);
    let mut expected_post_change = pre_change_contract.clone();
    expected_post_change.snapshot.rows[0].workload_addr = Some(guest.guest_addr);
    let post_change = serde_json::to_value(&described.snapshot).expect("post-change snapshot JSON");
    let expected_post_change = serde_json::to_value(&expected_post_change.snapshot)
        .expect("independent expected post-change snapshot JSON");
    assert_eq!(
        post_change, expected_post_change,
        "adding workload_addr must leave the complete pre-change response complement unchanged"
    );

    // Render observable universe: the entire output string. The baseline is a
    // frozen pre-step render contract over those independent durable facts,
    // not a second call derived from the post-change object. Its sole permitted
    // delta is the one canonical Addresses section.
    let pre_change_render = frozen_pre_change_render_contract(&pre_change_contract);
    assert_eq!(
        overdrive_cli::render::workload_describe(&pre_change_contract),
        pre_change_render,
        "the live renderer without the additive address must match the genuine pre-change contract"
    );
    let rendered = overdrive_cli::render::workload_describe(&described);
    let address_delta = format!("Addresses:\n  {}: {}\n", row.alloc_id, guest.guest_addr);
    assert_eq!(
        rendered.match_indices(&address_delta).count(),
        1,
        "the exact canonical-address delta must occur once; got:\n{rendered}"
    );
    assert_eq!(
        rendered.replacen(&address_delta, "", 1),
        pre_change_render,
        "all rendered output outside the one permitted Addresses section must remain byte-exact"
    );
    assert!(
        !rendered.contains(&format!("{}: {}", row.alloc_id, transit.workload_addr)),
        "live operator renderer must not show the transit address; got:\n{rendered}"
    );

    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop describe VM");
    let _ = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    handle.shutdown().await.expect("clean mTLS serve shutdown");
}

/// S-GTI-02 — the guest's first mesh connection is born intercepted.
///
/// Observable universe: every AF_PACKET frame captured from before VM deploy
/// through guest termination on the exact allocation host-veth whose source is
/// the exact guest address and whose destination is the exact mesh peer tuple,
/// plus every peer-port loopback stream and the typed kernel nft-rule snapshot.
/// Assertions quantify over those complete captured collections; no sampled
/// event field or selected unrelated socket stands in for their complement.
///
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: unbounded-preservation.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn the_guests_first_mesh_dial_is_born_intercepted_no_cleartext_escapes() {
    let result = run_mesh_guest_scenario("gti-born-captured").await;
    let first_syn = result.guest_egress.first_syn.unwrap_or_else(|| {
        panic!(
            "the exact guest-to-mesh host-veth universe contains the command's first SYN; \
             readiness={:?}; all interface TCP={:#?}",
            result.readiness, result.guest_egress.interface_tcp
        )
    });
    assert!(
        !result.readiness.kernel_snapshot.is_empty(),
        "the independently decoded kernel rule snapshot must identify the exact VM host-veth"
    );
    assert!(
        !result.guest_egress.segments.is_empty(),
        "the exact guest escape-boundary universe must be non-empty"
    );
    assert!(
        first_syn.flags & 0x02 != 0 && first_syn.flags & 0x10 == 0,
        "the independently captured first exact-tuple packet is an initial SYN: {first_syn:?}"
    );
    assert_eq!(first_syn.payload_len, 0, "an initial SYN has no application payload");
    assert!(
        first_syn.tuple.source.port() != 0,
        "the first guest SYN carries one exact ephemeral source port"
    );
    let pre_ready = result
        .guest_egress
        .segments
        .iter()
        .filter(|segment| segment.observed_at < result.readiness.observed_at)
        .collect::<Vec<_>>();
    assert!(
        pre_ready.is_empty(),
        "actual kernel-rule readiness must precede every captured TCP segment on the exact guest \
         -> mesh tuple, including the causally-first SYN after EXEC; pre-ready={pre_ready:#?}, \
         readiness={:?}",
        result.readiness
    );
    assert!(
        first_syn.observed_at >= result.readiness.observed_at,
        "actual nft readiness must precede the first exact guest SYN"
    );
    assert!(
        result.guest_egress.plaintext_request_hits > 0,
        "the exact host-veth capture must independently observe the guest-authored plaintext \
         request before TPROXY consumes it"
    );
    assert_eq!(
        result.scan.plaintext_hits_on_any_peer_stream, 0,
        "the first captured mesh connection must not expose either plaintext litmus; got {:?}",
        result.scan
    );
    assert!(
        result.scan.exact_records_to_peer > 0 && result.scan.exact_records_from_peer > 0,
        "the born-captured connection must carry TLS application_data in both directions; got {:?}",
        result.scan
    );
    assert!(
        result
            .ktls
            .as_ref()
            .is_some_and(|evidence| { record_has_bidirectional_tls13_ktls(&evidence.record) }),
        "the born-captured connection must install bidirectional TLS 1.3 kTLS"
    );
}

/// S-GTI-05 — an intercept-install failure refuses execution fail-closed.
#[test]
#[should_panic(expected = "RED scaffold")]
fn when_the_mesh_guard_cannot_be_installed_the_workload_is_refused() {
    panic!("Not yet implemented -- RED scaffold (S-GTI-05 / step 02-02)");
}

/// S-GTI-06 — restart re-enrols a VM before releasing guest execution.
#[test]
#[should_panic(expected = "RED scaffold")]
fn a_restarted_microvm_workload_is_re_enrolled_in_the_mesh_before_it_runs_again() {
    panic!("Not yet implemented -- RED scaffold (S-GTI-06 / step 02-03)");
}

/// S-GTI-08 — guest net-apply failure is a classified boot refusal.
#[test]
#[should_panic(expected = "RED scaffold")]
fn a_microvm_that_cannot_address_its_network_is_refused_as_a_boot_failure() {
    panic!("Not yet implemented -- RED scaffold (S-GTI-08 / step 02-04)");
}

/// S-GTI-12 — stop tears down the VM intercept without disturbing peers.
#[test]
#[should_panic(expected = "RED scaffold")]
fn a_stopped_microvm_workloads_egress_mesh_guard_is_torn_down_never_left_behind() {
    panic!("Not yet implemented -- RED scaffold (S-GTI-12 / step 02-03)");
}
