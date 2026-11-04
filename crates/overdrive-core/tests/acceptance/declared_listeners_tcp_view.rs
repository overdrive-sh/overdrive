//! One declared listener set; the TCP view is a filter by construction
//! (D14 item 2, PR-4; S-55).
//!
//! `AllocationSpec::tcp_service_ports()` equals the TCP-protocol filter of
//! `declared_listeners` in declaration order, for every list (TCP only, UDP
//! only, mixed, empty, no TCP).
//! CONTRACT_SHAPE: pure-function.

#![allow(clippy::expect_used, clippy::doc_markdown, reason = "test fixtures")]

use std::num::NonZeroU16;
use std::path::PathBuf;

use overdrive_core::aggregate::Listener;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, DriverPayload, Resources, VmPayload};
use proptest::prelude::*;

fn spec_with(declared: Vec<Listener>) -> AllocationSpec {
    let alloc = AllocationId::new("tcp-view-0").expect("alloc id");
    AllocationSpec {
        alloc,
        identity: SpiffeId::new("spiffe://overdrive.local/workload/tcp-view/alloc/tcp-view-0")
            .expect("spiffe id"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/serve".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/kernel"),
            rootfs: PathBuf::from("/rootfs"),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        transport: None,
        service_ports: Vec::new(),
        declared_listeners: declared,
    }
}

fn arb_listener() -> impl Strategy<Value = Listener> {
    (prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)], 1_u16..=u16::MAX).prop_map(
        |(protocol, port)| Listener {
            protocol,
            port: NonZeroU16::new(port).expect("range excludes zero"),
        },
    )
}

proptest! {
    /// S-55: for every declared list the TCP view is the TCP filter, in order.
    #[test]
    #[ignore = "pending DELIVER: PR-4 tcp_service_ports is the TCP filter of declared_listeners"]
    fn the_tcp_view_is_the_tcp_filter_of_the_declared_listeners_in_declaration_order(
        declared in proptest::collection::vec(arb_listener(), 0..=12),
    ) {
        let expected: Vec<NonZeroU16> = declared
            .iter()
            .filter(|l| l.protocol == Proto::Tcp)
            .map(|l| l.port)
            .collect();
        prop_assert_eq!(spec_with(declared).tcp_service_ports(), expected);
    }
}

/// S-55 golden cases: a UDP-only declaration has an empty TCP view (so no
/// TPROXY rule is installed for it); a mixed one keeps TCP ports in order.
#[test]
#[ignore = "pending DELIVER: PR-4 tcp_service_ports is the TCP filter of declared_listeners"]
fn a_udp_only_declaration_has_no_tcp_ports_and_a_mixed_one_keeps_tcp_order() {
    let l = |protocol, port: u16| Listener { protocol, port: NonZeroU16::new(port).expect("port") };
    assert!(spec_with(vec![l(Proto::Udp, 5353)]).tcp_service_ports().is_empty());
    assert!(spec_with(Vec::new()).tcp_service_ports().is_empty());
    assert_eq!(
        spec_with(vec![l(Proto::Tcp, 8081), l(Proto::Udp, 5353), l(Proto::Tcp, 8080)])
            .tcp_service_ports(),
        vec![NonZeroU16::new(8081).expect("port"), NonZeroU16::new(8080).expect("port")],
    );
}
