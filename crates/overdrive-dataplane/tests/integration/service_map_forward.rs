//! S-2.2-06 — Single-VIP TCP forwarding through real veth.
//!
//! Tags: `@US-02` `@K2` `@slice-02` `@real-io @adapter-integration`.
//! Tier: Tier 3.
//!
//! Drives the kernel-loaded `xdp_service_map_lookup` program against a
//! freshly-created `veth0`/`veth1` pair: SERVICE_MAP is populated from
//! userspace with VIP `10.0.0.1:8080` (TCP) → backend `10.1.0.5:9000`,
//! 10 TCP SYNs are injected from `veth1`, and the rewritten frames are
//! captured on `veth1`'s ingress (XDP_TX bounces them back out `veth0`,
//! which on a veth pair surfaces as ingress on the peer end).
//!
//! Each captured frame is asserted against:
//! * dest IPv4 = `10.1.0.5`
//! * dest TCP port = `9000`
//! * IPv4 header checksum recomputes to 0 (RFC 1071, valid)
//! * TCP checksum recomputes to 0 (RFC 793 + RFC 1071, valid)
//!
//! End-to-end this validates the architecture.md § 11 endianness
//! lockstep: userspace writes host-order bytes via aya's `HashMap`
//! interface, the BPF program reads wire-order packet bytes via
//! `read_u32_be` / `read_u16_be` and compares as host-order against the
//! map, and the rewritten packet round-trips back with valid wire-order
//! headers and checksums. A regression anywhere in the chain fails the
//! frame-content assertions.
//!
//! # veth XDP gotcha — both ends require an XDP program
//!
//! For native XDP_TX / XDP_REDIRECT on a veth pair, **both** ends must
//! have an XDP program attached. The peer-side program does not need to
//! do anything beyond return `XDP_PASS` — its presence is what enables
//! the XDP RX queue on the peer so kernel-emitted XDP frames round-trip
//! correctly. Without this, the veth driver silently drops the
//! XDP_TX'd frame on the peer's RX path. Documented in the kernel veth
//! driver and the iovisor xdp-tutorial; rediscovered at this step's
//! GREEN attempt — see commit body.
//!
//! Capability gating mirrors `veth_attach.rs`: requires `CAP_NET_ADMIN`
//! for veth setup and raw-socket bind. Bails with a skip message on
//! `EPERM` rather than failing — `cargo xtask lima run --` runs as root
//! by default; CI runs the integration job as root.

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]
// `expect_used` is workspace-wide `warn` per `.claude/rules/development.md`
// § Errors. This Tier 3 test surfaces RAII-fail-fast at the assertion
// site, matching the convention in `veth_attach.rs` and
// `crates/overdrive-worker/tests/integration/exec_driver/`.
#![allow(clippy::expect_used, clippy::print_stderr)]

use std::ffi::CString;
use std::io;
use std::mem;
use std::os::raw::c_int;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use aya::{
    Ebpf,
    maps::HashMap,
    programs::{Xdp, XdpFlags},
};

use super::helpers::packets::{
    ETH_HDR_LEN, IPV4_HDR_LEN, TCP_HDR_LEN, ipv4_header_checksum, synthesise_tcp_syn, tcp_checksum,
};
use super::helpers::veth::{VethError, VethPair};

const VIP_OCTETS: [u8; 4] = [10, 0, 0, 1];
const VIP_PORT: u16 = 8080;
const BACKEND_OCTETS: [u8; 4] = [10, 1, 0, 5];
const BACKEND_PORT: u16 = 9000;
const FRAME_COUNT: u32 = 10;

/// Outer-map key matching `ServiceKey` in
/// `crates/overdrive-dataplane/src/maps/service_map_handle.rs`. 8-byte
/// host-order POD: vip_host (u32) + port_host (u16) + _pad (u16). Same
/// shape used by the Tier 2 triptych; userspace writes host-order
/// bytes, the kernel-side compares against the wire-order packet bytes
/// converted to host-order via `read_u32_be` / `read_u16_be`
/// (architecture.md § 11).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct ServiceKey {
    vip_host: u32,
    port_host: u16,
    _pad: u16,
}
// SAFETY: repr(C), no padding-uninit issues for our writes (we always
// set _pad to 0); `aya::Pod` is the marker aya needs for raw map access.
unsafe impl aya::Pod for ServiceKey {}

/// Inner-map value matching `BackendEntry` — 12-byte host-order POD.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct BackendEntry {
    ipv4_host: u32,
    port_host: u16,
    weight: u16,
    healthy: u8,
    _pad: [u8; 3],
}
// SAFETY: repr(C); `aya::Pod` permits raw map insert.
unsafe impl aya::Pod for BackendEntry {}

fn workspace_root() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut p = PathBuf::from(manifest);
    p.pop(); // remove `overdrive-dataplane`
    p.pop(); // remove `crates`
    p
}

fn bpf_artifact_path() -> PathBuf {
    workspace_root().join("target/xtask/bpf-objects/overdrive_bpf.o")
}

/// Load `Ebpf::load_file(artifact)` with bounded retry. The sibling
/// `build_rs_artifact_check` test removes-and-restores the same
/// artifact within a single test body; nextest may schedule that
/// test in parallel with this one (different processes, so
/// `serial_test` group keys do not synchronise across them).
/// Retrying for ~10 s absorbs that transient gap without weakening
/// the assertion that the artifact must exist by the time this test
/// completes its setup.
fn load_with_retry(artifact: &PathBuf, budget: Duration) -> Ebpf {
    let deadline = Instant::now() + budget;
    let mut last_err: Option<String> = None;
    while Instant::now() < deadline {
        if artifact.exists() {
            match Ebpf::load_file(artifact) {
                Ok(bpf) => return bpf,
                Err(e) => last_err = Some(format!("aya load_file({}): {e}", artifact.display())),
            }
        } else {
            last_err = Some(format!(
                "BPF artifact missing at {} — run `cargo xtask bpf-build` first",
                artifact.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("{}", last_err.unwrap_or_else(|| "load_with_retry: budget exhausted".into()));
}

/// S-2.2-06 — Ten TCP SYNs to a registered VIP all rewrite and
/// forward via veth.
///
/// `serial_test::serial(env)` — `tests/integration/build_rs_artifact_check.rs`
/// removes and restores the on-disk BPF artifact at
/// `target/xtask/bpf-objects/overdrive_bpf.o` to exercise the
/// build-script diagnostic. This test reads that same artifact via
/// `Ebpf::load_file`, so the two MUST NOT race — sharing the `env`
/// group puts both tests in the same serial sequence. Veth-state
/// isolation is provided by the unique iface names `ovd-svc{0,1}`
/// (no other test claims them) plus the `VethPair` `Drop` guard.
#[test]
#[serial_test::serial(env)]
fn ten_tcp_syns_to_vip_are_rewritten_and_forwarded_via_veth() {
    let host = "ovd-svc0";
    let peer = "ovd-svc1";

    let veth = match VethPair::create(host, peer) {
        Ok(v) => v,
        Err(VethError::CapNetAdminRequired) => {
            eprintln!(
                "skip: S-2.2-06 needs CAP_NET_ADMIN for veth setup — \
                 run via `cargo xtask lima run --` (default-root)"
            );
            return;
        }
        Err(e) => panic!("veth setup failed: {e}"),
    };

    // Resolve veth1 ifindex (needed for raw-socket bind on the peer end).
    let peer_ifindex = match if_nametoindex(&veth.peer) {
        Ok(idx) => idx,
        Err(e) => panic!("if_nametoindex({}): {e}", veth.peer),
    };

    // Load the BPF object. `Ebpf::load_file` is preferred over
    // `Ebpf::load(slice)` here — the slice path of aya 0.13 rejects
    // BTF-less ELFs in some configurations; the file path is more
    // tolerant. The artifact may transiently disappear when the
    // sibling `build_rs_artifact_check` test runs in another process
    // (nextest spawns each test in its own process; `serial_test`
    // only synchronises within one process). Retry briefly before
    // declaring the artifact missing.
    let artifact = bpf_artifact_path();
    let mut bpf = load_with_retry(&artifact, Duration::from_secs(10));

    // Attach `xdp_service_map_lookup` to the host end (veth0). Native-
    // first attach mirrors `EbpfDataplane::new`'s production wiring;
    // `EOPNOTSUPP` / `ENOTSUP` falls back to SKB_MODE.
    let _service_link = {
        let prog: &mut Xdp = bpf
            .program_mut("xdp_service_map_lookup")
            .expect("xdp_service_map_lookup program not found in BPF object")
            .try_into()
            .expect("xdp_service_map_lookup is not an Xdp program");
        prog.load().expect("xdp_service_map_lookup.load");
        prog.attach(&veth.host, XdpFlags::DRV_MODE)
            .or_else(|_| prog.attach(&veth.host, XdpFlags::SKB_MODE))
            .unwrap_or_else(|e| {
                panic!("xdp_service_map_lookup.attach({}, fallback SKB): {e}", veth.host)
            })
    };

    // Attach `xdp_pass` to the peer end (veth1) to enable its XDP RX
    // queue. See module-level docs § "veth XDP gotcha": without this,
    // XDP_TX'd frames silently drop on the peer's RX path. The peer
    // program is a pure pass-through and has no semantic effect on the
    // assertions below — only its presence matters.
    let _pass_link = {
        let prog: &mut Xdp = bpf
            .program_mut("xdp_pass")
            .expect("xdp_pass program not found in BPF object")
            .try_into()
            .expect("xdp_pass is not an Xdp program");
        prog.load().expect("xdp_pass.load");
        prog.attach(&veth.peer, XdpFlags::DRV_MODE)
            .or_else(|_| prog.attach(&veth.peer, XdpFlags::SKB_MODE))
            .unwrap_or_else(|e| panic!("xdp_pass.attach({}, fallback SKB): {e}", veth.peer))
    };

    // Populate SERVICE_MAP with VIP -> backend.
    {
        let mut sm: HashMap<_, ServiceKey, BackendEntry> =
            HashMap::try_from(bpf.map_mut("SERVICE_MAP").expect("SERVICE_MAP map not found"))
                .expect("SERVICE_MAP HashMap::try_from");
        let key = ServiceKey {
            vip_host: u32::from(std::net::Ipv4Addr::from(VIP_OCTETS)),
            port_host: VIP_PORT,
            _pad: 0,
        };
        let value = BackendEntry {
            ipv4_host: u32::from(std::net::Ipv4Addr::from(BACKEND_OCTETS)),
            port_host: BACKEND_PORT,
            weight: 1,
            healthy: 1,
            _pad: [0; 3],
        };
        sm.insert(key, value, 0).expect("SERVICE_MAP insert");
    }

    // Open a capture socket on veth1 BEFORE injecting frames. PF_PACKET
    // on veth1 sees ingress: when XDP_TX bounces the rewritten frame
    // out of veth0, the kernel hands it to veth0's peer (veth1) as an
    // ingress packet. The recv side filters on dest IP ==
    // BACKEND_OCTETS to distinguish rewritten frames from any other
    // traffic that happens to traverse veth1.
    let capture_fd =
        open_capture_socket(peer_ifindex).expect("open AF_PACKET capture socket on veth1");

    // Inject 10 TCP SYNs out of veth1 addressed to VIP. Frames travel
    // veth1 -> kernel -> veth0 ingress -> xdp_service_map_lookup ->
    // XDP_TX -> veth0 egress -> kernel -> veth1 ingress (where the
    // capture socket is reading).
    inject_tcp_syns(&veth.peer, FRAME_COUNT, VIP_OCTETS, VIP_PORT).expect("inject TCP SYNs");

    // Capture rewritten frames. Wait up to 5 s total for all 10 — the
    // SKB-mode XDP path is not free, but an idle veth pair has no
    // other traffic and 10 frames bounce back well within the budget.
    let captured =
        capture_rewritten_frames(capture_fd, FRAME_COUNT as usize, Duration::from_secs(5));

    // SAFETY: capture_fd was returned by socket(); close exactly once.
    unsafe { libc::close(capture_fd) };

    assert_eq!(
        captured.len(),
        FRAME_COUNT as usize,
        "expected {FRAME_COUNT} rewritten frames on veth1; observed {}",
        captured.len(),
    );

    for (idx, frame) in captured.iter().enumerate() {
        assert!(
            frame.len() >= ETH_HDR_LEN + IPV4_HDR_LEN + TCP_HDR_LEN,
            "frame {idx} too short: {} bytes",
            frame.len(),
        );

        // (a) Dest IP rewritten.
        let ip_dst = &frame[ETH_HDR_LEN + 16..ETH_HDR_LEN + 20];
        assert_eq!(
            ip_dst, &BACKEND_OCTETS,
            "frame {idx}: dest IP not rewritten to backend (got {ip_dst:?})"
        );

        // (b) Dest port rewritten.
        let tcp = ETH_HDR_LEN + IPV4_HDR_LEN;
        let dst_port = u16::from_be_bytes([frame[tcp + 2], frame[tcp + 3]]);
        assert_eq!(dst_port, BACKEND_PORT, "frame {idx}: dest port not rewritten (got {dst_port})");

        // (c) IPv4 header checksum is valid post-rewrite.
        let recomputed_ip = ipv4_header_checksum(&frame[ETH_HDR_LEN..ETH_HDR_LEN + IPV4_HDR_LEN]);
        assert_eq!(
            recomputed_ip, 0,
            "frame {idx}: IPv4 checksum invalid after rewrite (recomputed = {recomputed_ip:#x})"
        );

        // (d) TCP checksum is valid post-rewrite.
        let recomputed_tcp = tcp_checksum(
            &frame[ETH_HDR_LEN + 12..ETH_HDR_LEN + 16],
            &frame[ETH_HDR_LEN + 16..ETH_HDR_LEN + 20],
            &frame[tcp..tcp + TCP_HDR_LEN],
        );
        assert_eq!(
            recomputed_tcp, 0,
            "frame {idx}: TCP checksum invalid after rewrite (recomputed = {recomputed_tcp:#x})"
        );
    }
}

// ----- raw-socket helpers -----

#[allow(clippy::cast_lossless, clippy::cast_possible_truncation)]
fn if_nametoindex(iface: &str) -> Result<u32, io::Error> {
    let iface_c = CString::new(iface).expect("iface name has no NUL");
    // SAFETY: libc::if_nametoindex is a thin syscall wrapper; the
    // input pointer is not retained past the call.
    let idx = unsafe { libc::if_nametoindex(iface_c.as_ptr()) };
    if idx == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(idx)
}

const ETH_P_ALL: c_int = 0x0003;

/// Open a non-blocking PF_PACKET capture socket bound to `ifindex`.
/// The socket sees both ingress and egress on the iface; the caller
/// filters by dest IP/port at the assertion site.
#[allow(clippy::cast_lossless, clippy::cast_possible_truncation)]
fn open_capture_socket(ifindex: u32) -> Result<i32, io::Error> {
    // SAFETY: standard PF_PACKET + SOCK_RAW + ETH_P_ALL recipe.
    let fd = unsafe {
        libc::socket(libc::PF_PACKET, libc::SOCK_RAW, (ETH_P_ALL as u16).to_be() as c_int)
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    // Bind to ifindex.
    let mut addr: libc::sockaddr_ll = unsafe { mem::zeroed() };
    addr.sll_family = u16::try_from(libc::AF_PACKET).expect("AF_PACKET fits in u16");
    addr.sll_protocol = (ETH_P_ALL as u16).to_be();
    addr.sll_ifindex = i32::try_from(ifindex).expect("ifindex fits in i32");
    // SAFETY: bind(2) reads `addr` for the documented length only.
    let rc = unsafe {
        libc::bind(
            fd,
            std::ptr::addr_of!(addr).cast(),
            u32::try_from(mem::size_of::<libc::sockaddr_ll>())
                .expect("sockaddr_ll size fits in socklen_t"),
        )
    };
    if rc < 0 {
        let e = io::Error::last_os_error();
        // SAFETY: fd was returned by socket() and is open.
        unsafe { libc::close(fd) };
        return Err(e);
    }

    // Non-blocking — the capture loop polls with its own deadline.
    // SAFETY: fcntl(F_GETFL) / fcntl(F_SETFL) operate on the fd we own.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags < 0 {
        let e = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e);
    }
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if rc < 0 {
        let e = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e);
    }

    Ok(fd)
}

/// Read up to `target` rewritten frames (dest IP == BACKEND_OCTETS,
/// dest port == BACKEND_PORT) from the bound capture socket. Returns
/// each frame's bytes (Ethernet-headers-and-up). Stops at the first of:
/// `target` frames captured, or `deadline` reached.
fn capture_rewritten_frames(fd: i32, target: usize, budget: Duration) -> Vec<Vec<u8>> {
    let deadline = Instant::now() + budget;
    let mut buf = vec![0u8; 2048];
    let mut frames: Vec<Vec<u8>> = Vec::with_capacity(target);
    while frames.len() < target && Instant::now() < deadline {
        // SAFETY: recv into our owned buffer; len bound by buf.len().
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::WouldBlock {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            // Other errors are fatal for this capture; propagate via
            // empty return so the assertion fires with a useful diff.
            eprintln!("capture recv() failed: {e}");
            break;
        }
        let n = n as usize;
        if n < ETH_HDR_LEN + IPV4_HDR_LEN + TCP_HDR_LEN {
            continue;
        }
        // Filter: ethertype IPv4, dest IPv4 == BACKEND_OCTETS, proto
        // TCP, dest port == BACKEND_PORT. Anything else is collateral
        // traffic on veth1 (originals + IPv6 RA) that we ignore.
        let ethertype = u16::from_be_bytes([buf[12], buf[13]]);
        if ethertype != 0x0800 {
            continue;
        }
        let proto = buf[ETH_HDR_LEN + 9];
        if proto != 0x06 {
            continue;
        }
        let ip_dst = &buf[ETH_HDR_LEN + 16..ETH_HDR_LEN + 20];
        if ip_dst != BACKEND_OCTETS {
            continue;
        }
        let tcp = ETH_HDR_LEN + IPV4_HDR_LEN;
        let dst_port = u16::from_be_bytes([buf[tcp + 2], buf[tcp + 3]]);
        if dst_port != BACKEND_PORT {
            continue;
        }
        frames.push(buf[..n].to_vec());
    }
    frames
}

/// Inject `n` synthesised TCP SYN frames out of `iface` addressed to
/// `dst_octets:dst_port`. Mirrors `veth_attach.rs::inject_frames` but
/// uses the synthesised TCP-SYN shape from
/// `super::helpers::packets::synthesise_tcp_syn` rather than a
/// hand-coded UDP frame.
#[allow(clippy::cast_lossless, clippy::cast_possible_truncation)]
fn inject_tcp_syns(
    iface: &str,
    n: u32,
    dst_octets: [u8; 4],
    dst_port: u16,
) -> Result<(), io::Error> {
    let ifindex = if_nametoindex(iface)?;
    // SAFETY: socket(2) is a thin syscall.
    let fd = unsafe {
        libc::socket(libc::PF_PACKET, libc::SOCK_RAW, (ETH_P_ALL as u16).to_be() as c_int)
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut addr: libc::sockaddr_ll = unsafe { mem::zeroed() };
    addr.sll_family = u16::try_from(libc::AF_PACKET).expect("AF_PACKET fits in u16");
    addr.sll_protocol = (ETH_P_ALL as u16).to_be();
    addr.sll_ifindex = i32::try_from(ifindex).expect("ifindex fits in i32");

    let frame = synthesise_tcp_syn(dst_octets, dst_port);

    let mut send_err: Option<io::Error> = None;
    for _ in 0..n {
        // SAFETY: sendto writes from `frame` (length-bound) to the
        // bound socket; addr is fully initialised.
        let rc = unsafe {
            libc::sendto(
                fd,
                frame.as_ptr().cast(),
                frame.len(),
                0,
                std::ptr::addr_of!(addr).cast(),
                u32::try_from(mem::size_of::<libc::sockaddr_ll>())
                    .expect("sockaddr_ll size fits in socklen_t"),
            )
        };
        if rc < 0 {
            send_err = Some(io::Error::last_os_error());
            break;
        }
    }
    // SAFETY: fd was returned by socket(); close exactly once.
    unsafe { libc::close(fd) };
    if let Some(e) = send_err {
        return Err(e);
    }
    Ok(())
}
