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

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::workload::{DescribeArgs, describe};
use overdrive_control_plane::api::{AllocStateWire, AllocStatusRowBody, IssuedCertSummary};
use overdrive_control_plane::veth_provisioner::{
    DEFAULT_CLIENT_IFACE, NetSlot, WORKLOAD_SUBNET_BASE, derive_vm_tap_plan,
    derive_workload_netns_plan, responder_addr_for_slot,
};
use overdrive_core::SpiffeId;
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

struct WireCapture {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Vec<Vec<u8>>>>,
    port: u16,
}

impl WireCapture {
    fn start(iface: &str, port: u16) -> Self {
        let iface = std::ffi::CString::new(iface).expect("iface has no NUL");
        // SAFETY: libc retains neither pointer and the returned fd is owned here.
        let ifindex = unsafe { libc::if_nametoindex(iface.as_ptr()) };
        assert!(ifindex != 0, "resolve AF_PACKET interface index");

        // SAFETY: create and bind one AF_PACKET socket to the resolved iface.
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
                // SAFETY: recv writes at most buf.len() bytes into owned storage.
                let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
                if n > 0 {
                    frames.push(buf[..n as usize].to_vec());
                } else {
                    std::thread::sleep(Duration::from_micros(200));
                }
            }
            loop {
                // SAFETY: final bounded drain of the same owned fd.
                let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
                if n <= 0 {
                    break;
                }
                frames.push(buf[..n as usize].to_vec());
            }
            // SAFETY: close exactly the fd created for this capture.
            unsafe { libc::close(fd) };
            frames
        });
        Self { stop, handle: Some(handle), port }
    }

    fn stop_and_scan(mut self, exact_tuple: Option<FlowTuple>) -> WireScan {
        self.stop.store(true, Ordering::SeqCst);
        let frames =
            self.handle.take().expect("wire capture thread").join().expect("wire capture join");
        scan_frames(&frames, self.port, exact_tuple)
    }
}

fn scan_frames(frames: &[Vec<u8>], peer_port: u16, exact_tuple: Option<FlowTuple>) -> WireScan {
    let mut streams: BTreeMap<FlowTuple, Vec<u8>> = BTreeMap::new();
    for frame in frames {
        let Some((tuple, payload)) = parse_tcp_payload(frame) else {
            continue;
        };
        if payload.is_empty()
            || (tuple.source.port() != peer_port && tuple.destination.port() != peer_port)
        {
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

fn parse_tcp_payload(frame: &[u8]) -> Option<(FlowTuple, &[u8])> {
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
        &frame[payload..payload_end],
    ))
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
    let service_running =
        poll_until_running(&cfg, &service_submit.workload_id, Duration::from_secs(30))
            .await
            .snapshot
            .rows
            .into_iter()
            .next()
            .expect("one Running service allocation");
    let service_identity = poll_until_issued_identity(
        &cfg,
        &service_submit.workload_id,
        &service_running.alloc_id,
        Duration::from_secs(10),
    )
    .await;

    let wire = WireCapture::start(LOOPBACK_IFACE, SERVICE_PORT);
    let vm_spec = write_toml(
        server_tmp.path(),
        &format!("{id}.toml"),
        &vm_job_toml(id, "/sbin/gti-mesh-guest", &[], &fixture.kernel_path, &rootfs),
    );
    let vm_submit = deploy(DeployArgs { spec: vm_spec, config_path: cfg.clone() })
        .await
        .expect("deploy VM mesh dialer through commands::deploy");
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
    let scan = wire.stop_and_scan(ktls.as_ref().map(|evidence| evidence.tuple));

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
    let _ = poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
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
    // Response observable universe: the entire serialized snapshot. Construct
    // the pre-change wire baseline by removing exactly the one additive
    // workload_addr slot, then reconstruct the post-change response by adding
    // only the expected guest address. Whole-value equality proves the full
    // adjacent response complement.
    let post_change = serde_json::to_value(&described.snapshot).expect("post-change snapshot JSON");
    let mut pre_change = post_change.clone();
    let pre_rows = pre_change
        .get_mut("rows")
        .and_then(serde_json::Value::as_array_mut)
        .expect("pre-change response rows");
    let removed = pre_rows[0]
        .as_object_mut()
        .expect("pre-change allocation row")
        .remove("workload_addr")
        .expect("post-change response carries workload_addr");
    assert_eq!(removed, serde_json::json!(guest.guest_addr.to_string()));
    let mut expected_post_change = pre_change.clone();
    expected_post_change["rows"][0]["workload_addr"] =
        serde_json::json!(guest.guest_addr.to_string());
    assert_eq!(
        post_change, expected_post_change,
        "adding workload_addr must leave the complete pre-change response complement unchanged"
    );

    // Render observable universe: the entire output string. Its sole permitted
    // delta is the one canonical Addresses section.
    let rendered = overdrive_cli::render::workload_describe(&described);
    let address_delta = format!("Addresses:\n  {}: {}\n", row.alloc_id, guest.guest_addr);
    assert_eq!(
        rendered.match_indices(&address_delta).count(),
        1,
        "the exact canonical-address delta must occur once; got:\n{rendered}"
    );
    let mut address_free = described.clone();
    for row in &mut address_free.snapshot.rows {
        row.workload_addr = None;
    }
    let baseline = overdrive_cli::render::workload_describe(&address_free);
    assert_eq!(
        rendered.replacen(&address_delta, "", 1),
        baseline,
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
#[test]
#[should_panic(expected = "RED scaffold")]
fn the_guests_first_mesh_dial_is_born_intercepted_no_cleartext_escapes() {
    panic!("Not yet implemented -- RED scaffold (S-GTI-02 / step 02-02)");
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
