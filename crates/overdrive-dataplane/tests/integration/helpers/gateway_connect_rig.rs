//! Fixtures for the gateway connect-dataplane Tier-3 tests (S-PIG-41, 56, 57,
//! 63, 67, 71, 72): a real `EbpfDataplane` on a veth pair with the cgroup
//! `connect4` attach at the root cgroup, model Service frontends, model backend
//! identities and a bounded dial.
//!
//! The fixtures install no map entry, nft rule or route that production does not
//! install; the rig only constructs the production dataplane exactly as the
//! other Tier-3 tests in this binary do.

#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::doc_markdown,
    clippy::missing_const_for_fn,
    reason = "fixture preconditions panic with their cause"
)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use overdrive_core::dataplane::{Proto, ServiceFrontend};
use overdrive_core::id::{ServiceVip, SpiffeId};
use overdrive_core::public_ingress::ServiceKey;
use overdrive_core::traits::dataplane::Backend;
use overdrive_dataplane::EbpfDataplane;
use tokio::net::{TcpSocket, TcpStream};

use super::veth::{VethError, VethPair};

/// Port of every model Service frontend.
pub const FRONTEND_PORT: u16 = 8080;
/// Port of every model backend.
pub const BACKEND_PORT: u16 = 9000;

/// The bpffs pin directory of one rig; removed on drop.
pub struct PinGuard(pub PathBuf);

impl Drop for PinGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A real `EbpfDataplane` with its veth pair and pin directory.
///
/// Field order is drop order: the dataplane detaches its XDP programs before the
/// veth pair is deleted and the pins are removed.
pub struct HostRig {
    pub dataplane: EbpfDataplane,
    pub veth: VethPair,
    pub pins: PinGuard,
    pub tag: String,
}

impl HostRig {
    /// Build the rig for `tag` (short, unique per test: it names the veth pair
    /// and the pin directory). `None` when the process lacks `CAP_NET_ADMIN`
    /// (run through `cargo xtask metal run --`).
    pub fn new(tag: &str) -> Option<Self> {
        let veth = match VethPair::create(&format!("pg{tag}h"), &format!("pg{tag}p")) {
            Ok(veth) => veth,
            Err(VethError::CapNetAdminRequired) => {
                eprintln!(
                    "skip: needs CAP_NET_ADMIN for veth setup: run via `cargo xtask metal run --`"
                );
                return None;
            }
            Err(error) => panic!("veth setup failed: {error}"),
        };
        let pins = PinGuard(PathBuf::from(format!(
            "/sys/fs/bpf/overdrive-test-pg{tag}-{}",
            std::process::id()
        )));
        let _ = std::fs::remove_dir_all(&pins.0);
        std::fs::create_dir_all(&pins.0).expect("create the bpffs pin directory");
        let dataplane = Self::build(&veth, &pins);
        Some(Self { dataplane, veth, pins, tag: tag.to_owned() })
    }

    fn build(veth: &VethPair, pins: &PinGuard) -> EbpfDataplane {
        EbpfDataplane::new_with_pin_dir(
            &veth.host,
            &veth.peer,
            &pins.0,
            std::path::Path::new("/sys/fs/cgroup"),
        )
        .expect("EbpfDataplane::new_with_pin_dir with the cgroup attach")
    }

    /// Drop the dataplane and build a new instance on the same pin directory (a
    /// serve restart: SR-2). The new instance is built on a second veth pair
    /// beside the old one, because the loader owns an interface's XDP slot;
    /// the old instance and its pair are dropped afterwards.
    pub fn recreate(self) -> Self {
        let tag = format!("{}r", self.tag);
        let veth = VethPair::create(&format!("pg{tag}h"), &format!("pg{tag}p"))
            .expect("second veth pair for the re-created instance");
        let dataplane = Self::build(&veth, &self.pins);
        let Self { dataplane: old, veth: old_veth, pins, .. } = self;
        drop(old);
        drop(old_veth);
        Self { dataplane, veth, pins, tag }
    }
}

/// Model Service frontend `n`: `198.18.250.<n>:8080/tcp` (the benchmarking
/// range: not a workload address, not routed, not a fixture lease).
pub fn frontend(n: u8) -> ServiceFrontend {
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(198, 18, 250, n))).expect("IPv4 VIP");
    ServiceFrontend::new(vip, NonZeroU16::new(FRONTEND_PORT).expect("nonzero"), Proto::Tcp)
        .expect("IPv4 frontend")
}

/// The Service-map key of model frontend `n`.
pub fn service_key(n: u8) -> ServiceKey {
    ServiceKey::from_frontend(frontend(n))
}

/// The dial target of model frontend `n`.
pub fn frontend_target(n: u8) -> SocketAddrV4 {
    SocketAddrV4::new(frontend(n).vip_v4(), FRONTEND_PORT)
}

/// Model allocation `n` as a workload identity.
pub fn alloc(n: u8) -> SpiffeId {
    SpiffeId::new(&format!("spiffe://overdrive.local/workload/gw/alloc/a{n}"))
        .expect("allocation SpiffeId")
}

/// A model backend: loopback address `127.1.<hi>.<lo>` (so every address is
/// distinct and locally routable), port 9000, weight 1.
pub fn backend(address: u16, allocation: u8, healthy: bool) -> Backend {
    let [hi, lo] = address.to_be_bytes();
    Backend {
        alloc: alloc(allocation),
        addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 1, hi, lo)), BACKEND_PORT),
        weight: 1,
        healthy,
    }
}

/// The dial target of [`backend`] `address`.
pub fn backend_target(address: u16) -> SocketAddrV4 {
    let [hi, lo] = address.to_be_bytes();
    SocketAddrV4::new(Ipv4Addr::new(127, 1, hi, lo), BACKEND_PORT)
}

/// Dial `target` from the prepared `socket`, bounded by `bound`. `None` when the
/// dial failed or timed out (the hook has by then written its receipt).
pub async fn dial(socket: TcpSocket, target: SocketAddrV4, bound: Duration) -> Option<TcpStream> {
    tokio::time::timeout(bound, socket.connect(SocketAddr::V4(target))).await.ok()?.ok()
}

/// The text of one kernel configuration command, for before/after equality.
/// A command that cannot run yields its failure text, so both snapshots of an
/// unchanged host still compare equal.
pub fn snapshot(program: &str, args: &[&str]) -> String {
    match Command::new(program).args(args).output() {
        Ok(output) => format!(
            "{}\n--stderr--\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) => format!("unavailable: {error}"),
    }
}

/// `nft list ruleset`, `ip rule` and `ip route show table 100`.
pub fn routing_snapshot() -> (String, String, String) {
    (
        snapshot("nft", &["list", "ruleset"]),
        snapshot("ip", &["rule", "show"]),
        snapshot("ip", &["route", "show", "table", "100"]),
    )
}
