//! Regression guard — SERVICE_MAP outer slot is keyed on the declared
//! VIP port, NOT the backend's listening port.
//!
//! Tier: Tier 3 (real eBPF, real veth, Lima). Gated behind
//! `integration-tests` via `tests/integration.rs`.
//!
//! `EbpfDataplane::update_service` derives the VIP port for the
//! SERVICE_MAP `ServiceKey` (and the reverse-NAT `VipPod` value) from
//! the `ServiceFrontend` triple `(vip, port, proto)` — the declared
//! frontend port — NOT from `backends[0].addr.port()` (the backend's
//! own listener). For a service whose VIP port differs from the backend
//! port (VIP:53 → backend:5353), the XDP program keys the outer
//! SERVICE_MAP slot on the packet's destination port (= the VIP port).
//! If the slot were stored under the backend port (5353) instead, every
//! lookup arriving on dst_port=53 would miss permanently.
//!
//! This is the structural defense for the keying bug. It is latent under
//! every other fixture in this crate because they all set
//! `VIP_PORT == BACKEND_PORT` (see `reverse_nat_e2e.rs` /
//! `multi_listener_tcp_udp_e2e.rs`), so `frontend.port()` and
//! `backends[0].addr.port()` coincide and the bug is unobservable. This
//! test deliberately breaks that coincidence.
//!
//! Capability gating: requires `CAP_NET_ADMIN` + `CAP_BPF`. Bails with a
//! skip on non-root rather than failing — run via `cargo xtask lima run
//! --` (default-root) on macOS, or the CI integration job's `sudo`
//! wrapper elsewhere.

// Fixture-wide allows mirror the sibling Tier-3 files
// (`reverse_nat_e2e.rs`, `multi_listener_tcp_udp_e2e.rs`): the netns /
// veth RAII plumbing trips `items_after_statements` on the in-fn
// `PinDirGuard` / `NetNsGuard` definitions and `used_underscore_binding`
// on the explicit `drop(_ns_guard)` teardown. Scoping each to a line
// would add pure-noise annotations.
#![allow(
    clippy::missing_panics_doc,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::used_underscore_binding
)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;
use std::path::PathBuf;

use overdrive_core::SpiffeId;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::ServiceVip;
use overdrive_core::traits::dataplane::{Backend, Dataplane};
use overdrive_dataplane::EbpfDataplane;
use overdrive_testing::netns::{NetNsError, ThreeIfaceTopology};

/// The declared service VIP. The frontend port (53) differs from the
/// backend listener port (5353) — this divergence is the whole point of
/// the test.
const VIP: Ipv4Addr = Ipv4Addr::new(10, 96, 0, 10);
/// The declared VIP / frontend port. The XDP program keys the outer
/// SERVICE_MAP slot on the packet's dst_port, which equals this.
const VIP_PORT: u16 = 53;
/// The backend's own listener port — deliberately != `VIP_PORT`.
const BACKEND_PORT: u16 = 5353;
const BACKEND_IP: Ipv4Addr = Ipv4Addr::LOCALHOST;

/// Build a UDP `ServiceFrontend` with the declared VIP port.
fn udp_frontend(vip: Ipv4Addr, port: u16) -> ServiceFrontend {
    let service_vip = ServiceVip::new(IpAddr::V4(vip)).expect("valid IPv4 ServiceVip");
    ServiceFrontend::new(
        service_vip,
        NonZeroU16::new(port).expect("non-zero listener port"),
        Proto::Udp,
    )
    .expect("IPv4 ServiceFrontend constructs")
}

fn require_root_or_skip(test_name: &str) -> bool {
    // SAFETY: `geteuid` has no preconditions; reads a kernel-managed
    // numeric.
    let euid = unsafe { libc::geteuid() };
    if euid != 0 {
        eprintln!("[skip] {test_name} needs root (CAP_NET_ADMIN + CAP_BPF); euid={euid}");
        return false;
    }
    true
}

/// Regression: a service whose VIP port (53) differs from its backend
/// port (5353) must key the SERVICE_MAP outer slot on the VIP port.
///
/// * `service_map_contains(VIP, 53, Udp)` is `true`  — the VIP port.
/// * `service_map_contains(VIP, 5353, Udp)` is `false` — the backend
///   port; a `true` here proves the slot landed under the wrong port.
///
/// Before the fix, `update_service` sourced `vip_port` from
/// `backends[0].addr.port()` (= 5353), so the slot landed under 5353:
/// the VIP-port assertion failed and the backend-port assertion held —
/// the exact misplacement. After the fix (`vip_port = frontend.port()`)
/// the slot lands under 53 and both assertions flip to the correct
/// values.
#[test]
fn service_map_keyed_on_vip_port_not_backend_port() {
    if !require_root_or_skip("service-map-vip-port-keying") {
        return;
    }

    let topo = match ThreeIfaceTopology::create("vipport") {
        Ok(t) => t,
        Err(NetNsError::CapNetAdminRequired) => {
            eprintln!("[skip] service-map-vip-port-keying needs CAP_NET_ADMIN");
            return;
        }
        Err(e) => panic!("3-iface topology setup failed: {e}"),
    };

    let pin_dir =
        PathBuf::from(format!("/sys/fs/bpf/overdrive-test-vipport-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pin_dir);
    std::fs::create_dir_all(&pin_dir).expect("create pin dir");
    struct PinDirGuard(PathBuf);
    impl Drop for PinDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _pin_guard = PinDirGuard(pin_dir.clone());

    let _ns_guard = enter_netns(&topo.lb_ns.name).expect("setns into lb-ns");

    let dataplane = EbpfDataplane::new_with_pin_dir(
        &topo.lb_veth_a,
        &topo.lb_veth_b,
        &pin_dir,
        std::path::Path::new("/sys/fs/cgroup"),
    )
    .expect("EbpfDataplane::new_with_pin_dir");

    let runtime =
        tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio rt");

    let alloc =
        SpiffeId::new("spiffe://overdrive.local/job/e2e/alloc/vipport-B1").expect("SpiffeId");

    // Install a UDP service with VIP port 53 and backend port 5353.
    runtime
        .block_on(dataplane.update_service(
            udp_frontend(VIP, VIP_PORT),
            vec![Backend {
                alloc,
                addr: SocketAddr::new(IpAddr::V4(BACKEND_IP), BACKEND_PORT),
                weight: 1,
                healthy: true,
            }],
        ))
        .expect("update_service install UDP service VIP:53 -> backend:5353");

    // The outer slot MUST be keyed on the declared VIP port (53).
    let present_at_vip_port = dataplane
        .service_map_contains(VIP, VIP_PORT, Proto::Udp)
        .expect("service_map_contains at VIP port");
    assert!(
        present_at_vip_port,
        "SERVICE_MAP must contain the outer slot at the declared VIP port \
         ({VIP_PORT}); a miss here means update_service keyed the slot on the \
         backend port instead of the VIP port"
    );

    // The outer slot MUST NOT be keyed on the backend port (5353).
    let present_at_backend_port = dataplane
        .service_map_contains(VIP, BACKEND_PORT, Proto::Udp)
        .expect("service_map_contains at backend port");
    assert!(
        !present_at_backend_port,
        "SERVICE_MAP must NOT contain an outer slot at the backend port \
         ({BACKEND_PORT}); presence here means the slot landed under the \
         backend's listener port instead of the declared VIP port"
    );

    drop(_ns_guard);
    drop(dataplane);
    let _ = topo;
}

/// Enter `target_ns` via `setns(2)` against the netns FD opened from
/// `/var/run/netns/<name>`. Returns an RAII guard reverting the calling
/// thread's netns on Drop. Mirrors `reverse_nat_e2e.rs::enter_netns`.
fn enter_netns(target_ns: &str) -> std::io::Result<NetNsGuard> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    // SAFETY: `open(O_RDONLY)` on a kernel-managed path; owned fd, closed
    // on Drop.
    let prior_fd = {
        let path = std::ffi::CString::new("/proc/self/ns/net").unwrap();
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        unsafe { OwnedFd::from_raw_fd(fd) }
    };

    let target_path = format!("/var/run/netns/{target_ns}");
    let cstr = std::ffi::CString::new(target_path).unwrap();
    // SAFETY: open(O_RDONLY) on a netns mount; owned fd, closed on Drop.
    let target_fd = {
        let fd = unsafe { libc::open(cstr.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        unsafe { OwnedFd::from_raw_fd(fd) }
    };

    // SAFETY: setns to a network namespace; the current thread moves into
    // the target namespace.
    let rc = unsafe { libc::setns(target_fd.as_raw_fd(), libc::CLONE_NEWNET) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(NetNsGuard { prior_fd })
}

/// Reverts the calling thread to its prior netns on Drop.
struct NetNsGuard {
    prior_fd: std::os::fd::OwnedFd,
}

impl Drop for NetNsGuard {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        // SAFETY: setns back to the prior namespace FD captured in
        // `enter_netns`. Best-effort on teardown.
        let _ = unsafe { libc::setns(self.prior_fd.as_raw_fd(), libc::CLONE_NEWNET) };
    }
}
