//! R5-59 (a), beacon seam: accept-path adapter equivalence of the production
//! node beacon listener `VsockBeaconIo` and `SimBeaconIo` (D-UP-19; PD-5,
//! approved 2026-10-14) — the named Tier-3 pair of the sim's beacon accept
//! arbiter (`queue_connection`, `beacon_socket_observation`) that the
//! `flows_beacon` conformance driver reads (QPROJ-7).
//!
//! **Oracle**, per connection and order-insensitive: every connection the
//! guest opened is returned by `accept` exactly once and no further accept
//! completes; each carries its kernel identity (`BeaconConnKey`: the guest's
//! real CID and its source port) and its buffered first line byte-exact
//! whatever its queue position; one with no buffered line reads end of
//! stream once the guest ends it.
//!
//! **Stimulus.** A scripted guest vsock peer in a Cloud Hypervisor fork
//! microVM at a CID of the [`HOST_BEACON_ACCEPT_PATH`] band (the guest-kit
//! fixture, T-1; D-1) connects to the listener's bound port from distinct source
//! ports, a READY line buffered or none. Its packets enter through vhost
//! (`drivers/vhost/vsock.c:555 @ v7.0`), so the accept-queue insertion is the
//! production path (`net/vmw_vsock/virtio_transport_common.c:1557-1590 @
//! v7.0`). The loopback transport cannot serve: its peer CID 1 is not a
//! `GuestCid`, and the adapter closes such a connection unreturned.
//!
//! Lane: `integration-tests,kvm-tests`, native x86_64 metal (`cargo xtask
//! metal run -- cargo nextest run -p overdrive-host --features
//! integration-tests,kvm-tests`). The beacon port is node-global: this
//! binary runs in `host-kernel-shared`. Pending the attachment cut's step
//! (6), which lands `VsockBeaconIo`.

#![allow(clippy::doc_markdown, reason = "kernel symbol names in prose")]

use std::collections::BTreeMap;
use std::net::Shutdown;
use std::time::Duration;

use overdrive_core::guest_transport::{GuestCid, VsockListenPort};
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo, BeaconSocket};
use overdrive_core::vm::config::VsockPort;
use overdrive_host::beacon_io::VsockBeaconIo;
use overdrive_sim::SimVmm;
use overdrive_sim::adapters::beacon_io::SimBeaconIo;
use overdrive_testing::guest_kit::cid_bands::HOST_BEACON_ACCEPT_PATH;
use overdrive_testing::guest_kit::{GuestImageSpec, GuestKit, WorkDir};
use overdrive_testing::vm_fixture::default_staging_root;
use overdrive_testing::vsock::VMADDR_CID_HOST;

/// One connection: the guest's source port and whether a READY line is
/// buffered before the host accepts.
const CASES: [(u32, bool); 5] =
    [(53_001, true), (53_002, false), (53_003, true), (53_004, false), (53_005, true)];

/// The READY line of a case (byte-distinct per source port).
fn ready_line(source_port: u32) -> String {
    format!("READY pid=1 port=1234 launch={:032x}", u128::from(source_port) + 1)
}

/// Accepts every connection, lets `end_unbuffered` end the ones with no
/// line, then reads one line from each; asserts no further accept completes.
async fn drive(
    io: &dyn BeaconIo,
    end_unbuffered: impl Fn(BeaconConnKey),
) -> BTreeMap<u32, (GuestCid, Option<String>)> {
    let mut accepted: Vec<(BeaconSocket, BeaconConnKey)> = Vec::new();
    for _ in CASES {
        let conn = tokio::time::timeout(Duration::from_secs(10), io.accept())
            .await
            .expect("every opened connection is returned by accept")
            .expect("accept");
        accepted.push(conn);
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(500), io.accept()).await.is_err(),
        "no connection beyond the opened ones is observable"
    );
    let mut out = BTreeMap::new();
    for (socket, key) in &accepted {
        let port = key.peer_port.as_u32();
        let (_, buffered) = CASES
            .iter()
            .find(|(p, _)| *p == port)
            .copied()
            .unwrap_or_else(|| panic!("an unexpected source port {key:?}"));
        if !buffered {
            end_unbuffered(*key);
        }
        let line = tokio::time::timeout(Duration::from_secs(10), io.read_line(socket))
            .await
            .expect("a line read completes")
            .expect("read_line");
        out.insert(port, (key.peer_cid, line));
    }
    for (socket, _) in accepted {
        io.close(socket).await;
    }
    out
}

/// R5-59 (a), beacon seam: the same connections, opened by a guest at a
/// leased CID to the real listener and queued into the sim, are accepted
/// with the same identities and first lines.
/// CONTRACT_SHAPE: pure-function.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_beacon_listeners_accept_path_matches_the_sim_per_connection() {
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("precondition: the guest kit provisions");
    let work = WorkDir::create(&staging, "accept-path-beacon").expect("work dir");
    let image =
        kit.image(work.path(), "peer", &GuestImageSpec::scripted()).expect("the scripted image");
    let cid = HOST_BEACON_ACCEPT_PATH.raw(0);
    let mut guest =
        kit.boot_scripted(&image, cid, &work.path().join("run")).expect("the scripted guest");

    let real = VsockBeaconIo::new();
    // PORTS-CONFIG (ADR-0218): the listener binds a kernel-assigned port and
    // the guest connects to the port read back, never a fixed number.
    let beacon_port =
        real.bind(VsockListenPort::KernelAssigned).await.expect("bind the node beacon listener");
    for (n, (source_port, buffered)) in CASES.iter().enumerate() {
        let handle = format!("b{n}");
        let peer = guest.peer();
        peer.vsock_connect(&handle, VMADDR_CID_HOST, beacon_port.as_u32(), Some(*source_port))
            .expect("the guest connects from its chosen source port");
        if *buffered {
            peer.write(&handle, format!("{}\n", ready_line(*source_port)).as_bytes())
                .expect("the guest buffers its READY");
        }
    }
    let guest = std::sync::Mutex::new(guest);
    let real_seen = drive(&real, |key| {
        let n = CASES
            .iter()
            .position(|(p, _)| *p == key.peer_port.as_u32())
            .expect("a case of the table");
        guest
            .lock()
            .expect("the guest")
            .peer()
            .shutdown(&format!("b{n}"), Shutdown::Both)
            .expect("the guest ends its connection");
    })
    .await;

    let vmm = SimVmm::new();
    let control = vmm.spawn_bare_process();
    let sim = SimBeaconIo::new(vmm.clone());
    sim.bind(VsockListenPort::KernelAssigned).await.expect("bind");
    let peer_cid = GuestCid::new(cid).expect("a band CID is a GuestCid");
    for (source_port, buffered) in CASES {
        let key = BeaconConnKey { peer_cid, peer_port: VsockPort::new(source_port) };
        let line = ready_line(source_port);
        let lines: Vec<&str> = if buffered { vec![line.as_str()] } else { Vec::new() };
        sim.queue_connection(key, control.pid, &lines).expect("queued");
    }
    let sim_seen = drive(&sim, |key| assert!(sim.guest_close(key), "the guest ends it")).await;

    assert_eq!(real_seen, sim_seen, "per connection: identity and first line");
    for (port, (seen_cid, line)) in &real_seen {
        assert_eq!(*seen_cid, peer_cid, "connection {port} carries the guest's real CID");
        let buffered = CASES.iter().any(|(p, b)| p == port && *b);
        assert_eq!(line.is_some(), buffered, "connection {port}: a line iff one was buffered");
    }
    drop(guest);
}
