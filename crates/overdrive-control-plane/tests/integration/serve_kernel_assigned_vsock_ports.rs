//! PORTS-CONFIG (ADR-0218, user ruling 2026-10-19; GH #295) — a `serve`
//! configured with kernel-assigned vsock listener ports binds the real
//! production flow and control listeners on ports the kernel chose, reports
//! them through `ServerHandle::vsock_ports`, and holds them: two `serve`s on
//! one host never collide, whichever workspace runs them.
//!
//! Contract: `docs/feature/netns-density-295/feature-delta.md` § *Composition*,
//! "Node vsock ports (PORTS-CONFIG)": `VsockListenPort::KernelAssigned`
//! binds `VMADDR_PORT_ANY` and the kernel picks a port above
//! `LAST_RESERVED_PORT` (`net/vmw_vsock/af_vsock.c:787-810 @ v7.0`); the
//! owner's `flow_ports()` is `Some` from its first `Ok` `converge_shared`
//! and constant for the process; `run_server` reads it once and
//! `ServerHandle::vsock_ports() -> BoundVsockPorts` reports it, with
//! `beacon` `None` when no `Vm` driver was composed.
//!
//! Driving port: `run_server(config, adapters)` over the host bundle
//! (`node_adapters`: the real `GuestFlowAdapter::host()`, the real
//! forwarder probe — the `ovd_fwd` lane prerequisite is checked first).
//! Oracles: the reported ports, a second boot's ports, and the kernel's own
//! verdict on a bind of a reported port (`EADDRINUSE` while the server
//! holds it). Real vsock listeners: `integration-tests`, Lima as root,
//! whole binary in `host-kernel-shared`.

use std::time::Duration;

use overdrive_core::guest_transport::{GUEST_FLOW_CONTROL_PORT, GUEST_FLOW_STREAM_PORT};
use overdrive_core::vm::beacon::BEACON_VSOCK_PORT;
use overdrive_core::vm::config::VsockPort;
use overdrive_testing::vsock::{VMADDR_CID_ANY, VsockListener};
use tempfile::TempDir;

use super::node_adapters::{boot, test_config};

fn assigned(port: VsockPort, default: VsockPort, what: &str) {
    let raw = port.as_u32();
    assert!(raw > 1_023 && raw != u32::MAX, "{what}: a kernel-assigned port, got {raw}");
    assert_ne!(port, default, "{what}: not the production default {default}");
}

/// The kernel still holds `port` for a listener: a bind of it fails
/// `EADDRINUSE`.
fn held(port: VsockPort, what: &str) {
    match VsockListener::bind(VMADDR_CID_ANY, port.as_u32()) {
        Err(e) if e.raw_os_error() == Some(libc::EADDRINUSE) => {}
        other => panic!("{what}: port {port} is held by the server's listener, got {other:?}"),
    }
}

/// PORTS-CONFIG — two `serve`s configured `kernel_assigned()` boot side by
/// side; each reports distinct, non-default, kernel-assigned flow stream and
/// control ports, each port is held by a real listener, the two servers'
/// ports differ, and the report is constant while the server runs. With no
/// `Vm` driver composed (the host VMM probe is not required) `beacon` is
/// `None`; when one is, it is a kernel-assigned port too.
/// `CONTRACT_SHAPE`: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER attachment-cut step (6): PORTS-CONFIG"]
async fn kernel_assigned_listener_ports_are_reported_held_and_distinct_across_two_serves() {
    let (first_tmp, second_tmp) = (TempDir::new().expect("tmp"), TempDir::new().expect("tmp"));
    let first = boot(test_config(first_tmp.path().join("data"), first_tmp.path().join("conf")))
        .await
        .expect("the first serve boots on kernel-assigned ports");
    let second = boot(test_config(second_tmp.path().join("data"), second_tmp.path().join("conf")))
        .await
        .expect("a second serve on the same host boots: nothing collides");

    let a = first.vsock_ports();
    let b = second.vsock_ports();
    for (ports, who) in [(a, "first"), (b, "second")] {
        assigned(ports.flow.stream(), GUEST_FLOW_STREAM_PORT, &format!("{who} stream"));
        assigned(ports.flow.control(), GUEST_FLOW_CONTROL_PORT, &format!("{who} control"));
        assert_ne!(ports.flow.stream(), ports.flow.control(), "{who}: two listeners, two ports");
        held(ports.flow.stream(), &format!("{who} stream"));
        held(ports.flow.control(), &format!("{who} control"));
        if let Some(beacon) = ports.beacon {
            assigned(beacon, BEACON_VSOCK_PORT, &format!("{who} beacon"));
            held(beacon, &format!("{who} beacon"));
        }
    }
    let first_set = [a.flow.stream(), a.flow.control()];
    assert!(
        !first_set.contains(&b.flow.stream()) && !first_set.contains(&b.flow.control()),
        "the two servers' listeners hold different ports: {a:?} vs {b:?}"
    );

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(first.vsock_ports(), a, "the report is constant for the process");
    second.shutdown(Duration::from_secs(5)).await.expect("clean shutdown");
    first.shutdown(Duration::from_secs(5)).await.expect("clean shutdown");
}
