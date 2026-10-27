//! PORTS-CONFIG (ADR-0218, user ruling 2026-10-19; GH #295) — the node's
//! vsock listener port configuration, validated at construction.
//!
//! Contract: `docs/feature/netns-density-295/feature-delta.md` § *Composition*,
//! "Node vsock ports (PORTS-CONFIG)": `NodeVsockPortConfig::new(beacon,
//! flow_stream, flow_control)` refuses, first match in this order, a fixed
//! port in 0..=1023 (`Reserved`), a fixed `u32::MAX` (`PortAny`), fixed and
//! kernel-assigned ports mixed (`Mixed`), and two equal fixed ports
//! (`Collision`, listeners in `NodeVsockListener` order); `Default` is
//! `Fixed(1234)` / `Fixed(1240)` / `Fixed(1243)`; `kernel_assigned()` is all
//! three `KernelAssigned`. Pure function, default lane (layer 1: proptest
//! over the input space, Mandate 9).

use overdrive_core::guest_transport::{
    GUEST_FLOW_CONTROL_PORT, GUEST_FLOW_STREAM_PORT, NodeVsockListener, NodeVsockPortConfig,
    NodeVsockPortConfigError, VsockListenPort,
};
use overdrive_core::vm::beacon::BEACON_VSOCK_PORT;
use overdrive_core::vm::config::VsockPort;
use proptest::prelude::*;

const LISTENERS: [NodeVsockListener; 3] =
    [NodeVsockListener::Beacon, NodeVsockListener::FlowStream, NodeVsockListener::FlowControl];

fn fixed(port: u32) -> VsockListenPort {
    VsockListenPort::Fixed(VsockPort::new(port))
}

fn config(ports: [VsockListenPort; 3]) -> Result<NodeVsockPortConfig, NodeVsockPortConfigError> {
    NodeVsockPortConfig::new(ports[0], ports[1], ports[2])
}

/// The specification of `new`, written independently of it: the first
/// refusal in the pinned order, or `None`.
fn expected_refusal(ports: [VsockListenPort; 3]) -> Option<NodeVsockPortConfigError> {
    for (listener, port) in LISTENERS.iter().zip(ports) {
        if let VsockListenPort::Fixed(p) = port {
            if p.as_u32() <= 1_023 {
                return Some(NodeVsockPortConfigError::Reserved {
                    listener: *listener,
                    port: p.as_u32(),
                });
            }
        }
    }
    for (listener, port) in LISTENERS.iter().zip(ports) {
        if port == fixed(u32::MAX) {
            return Some(NodeVsockPortConfigError::PortAny { listener: *listener });
        }
    }
    let fixed_count = ports.iter().filter(|p| matches!(p, VsockListenPort::Fixed(_))).count();
    if fixed_count != 0 && fixed_count != 3 {
        return Some(NodeVsockPortConfigError::Mixed);
    }
    for (i, a) in ports.iter().enumerate() {
        for (j, b) in ports.iter().enumerate().skip(i + 1) {
            if let (VsockListenPort::Fixed(x), VsockListenPort::Fixed(y)) = (a, b) {
                if x == y {
                    return Some(NodeVsockPortConfigError::Collision {
                        first: LISTENERS[i],
                        second: LISTENERS[j],
                        port: x.as_u32(),
                    });
                }
            }
        }
    }
    None
}

/// One listener port drawn so every refusal class is reached often: the
/// reserved range, `u32::MAX`, a small shared pool (collisions), a wide
/// range, and kernel-assigned.
fn arb_port() -> impl Strategy<Value = VsockListenPort> {
    prop_oneof![
        (0_u32..=1_023).prop_map(fixed),
        Just(fixed(u32::MAX)),
        (5_000_u32..5_003).prop_map(fixed),
        (1_024_u32..u32::MAX).prop_map(fixed),
        Just(VsockListenPort::KernelAssigned),
    ]
}

proptest! {
    /// `new` refuses exactly when the specification refuses, with the
    /// variant (and fields) of the first refusal in the pinned order; an
    /// accepted configuration reports back the ports it was given.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn new_refuses_exactly_the_specified_configurations(
        beacon in arb_port(),
        stream in arb_port(),
        control in arb_port(),
    ) {
        let ports = [beacon, stream, control];
        match (config(ports), expected_refusal(ports)) {
            (Ok(ok), None) => {
                prop_assert_eq!(ok.beacon(), beacon);
                prop_assert_eq!(ok.flow_stream(), stream);
                prop_assert_eq!(ok.flow_control(), control);
            }
            (Err(got), Some(want)) => prop_assert_eq!(got, want),
            (got, want) => prop_assert!(false, "{:?}: got {:?}, the specification says {:?}", ports, got, want),
        }
    }
}

/// The pinned boundaries and the order between refusal classes.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_refusal_boundaries_and_their_order() {
    // 1023 is reserved, 1024 is not; u32::MAX - 1 is a valid fixed port.
    assert_eq!(
        config([fixed(1_023), fixed(2_000), fixed(2_001)]),
        Err(NodeVsockPortConfigError::Reserved {
            listener: NodeVsockListener::Beacon,
            port: 1_023
        })
    );
    assert!(config([fixed(1_024), fixed(2_000), fixed(u32::MAX - 1)]).is_ok());
    // Reserved before PortAny, PortAny before Mixed, Mixed before Collision.
    assert_eq!(
        config([fixed(u32::MAX), fixed(0), VsockListenPort::KernelAssigned]),
        Err(NodeVsockPortConfigError::Reserved {
            listener: NodeVsockListener::FlowStream,
            port: 0
        })
    );
    assert_eq!(
        config([fixed(2_000), fixed(u32::MAX), VsockListenPort::KernelAssigned]),
        Err(NodeVsockPortConfigError::PortAny { listener: NodeVsockListener::FlowStream })
    );
    assert_eq!(
        config([fixed(2_000), fixed(2_000), VsockListenPort::KernelAssigned]),
        Err(NodeVsockPortConfigError::Mixed)
    );
    // A collision names the first pair in listener order.
    assert_eq!(
        config([fixed(3_000), fixed(2_000), fixed(3_000)]),
        Err(NodeVsockPortConfigError::Collision {
            first: NodeVsockListener::Beacon,
            second: NodeVsockListener::FlowControl,
            port: 3_000,
        })
    );
}

/// `Default` is the production ports; `kernel_assigned()` is three
/// kernel-assigned ports and is itself valid.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_default_is_the_production_ports_and_kernel_assigned_is_valid() {
    let default = NodeVsockPortConfig::default();
    assert_eq!(default.beacon(), VsockListenPort::Fixed(BEACON_VSOCK_PORT));
    assert_eq!(default.flow_stream(), VsockListenPort::Fixed(GUEST_FLOW_STREAM_PORT));
    assert_eq!(default.flow_control(), VsockListenPort::Fixed(GUEST_FLOW_CONTROL_PORT));
    assert_eq!(
        config([fixed(1_234), fixed(1_240), fixed(1_243)]),
        Ok(default),
        "the default passes its own validation"
    );
    let assigned = NodeVsockPortConfig::kernel_assigned();
    for port in [assigned.beacon(), assigned.flow_stream(), assigned.flow_control()] {
        assert_eq!(port, VsockListenPort::KernelAssigned);
    }
    assert_eq!(
        config([VsockListenPort::KernelAssigned; 3]),
        Ok(assigned),
        "kernel_assigned() is the validated all-kernel-assigned configuration"
    );
}

/// The listener labels and the refusal messages an operator reads.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_labels_and_refusal_messages_are_the_pinned_text() {
    let labels: Vec<&str> = LISTENERS.iter().map(|l| l.as_str()).collect();
    assert_eq!(labels, ["beacon", "flow_stream", "flow_control"]);
    assert_eq!(
        NodeVsockPortConfigError::Reserved { listener: NodeVsockListener::FlowControl, port: 80 }
            .to_string(),
        "vsock port 80 for the flow_control listener is in the reserved range 0-1023"
    );
    assert_eq!(
        NodeVsockPortConfigError::PortAny { listener: NodeVsockListener::Beacon }.to_string(),
        "vsock port 4294967295 (VMADDR_PORT_ANY) is not a fixed port for the beacon listener; \
         use 0 for a kernel-assigned port"
    );
    assert_eq!(
        NodeVsockPortConfigError::Mixed.to_string(),
        "vsock listener ports must be all fixed or all kernel-assigned"
    );
    assert_eq!(
        NodeVsockPortConfigError::Collision {
            first: NodeVsockListener::Beacon,
            second: NodeVsockListener::FlowStream,
            port: 4_000,
        }
        .to_string(),
        "the beacon and flow_stream listeners are both configured on vsock port 4000"
    );
}
