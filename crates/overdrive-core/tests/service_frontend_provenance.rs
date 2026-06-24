//! Tier-1 C3-guard tests — proto provenance for the desired projection
//! (udp-service-support US-01; ADR-0060 site #8; ATLAS-1 b).
//!
//! The load-bearing C3 defense: the protocol carried into the
//! `ServiceFrontend` / `Action::DataplaneUpdateService` MUST be sourced
//! from a **listener-bearing fact** (`ListenerRow`,
//! `observation_store.rs:321` — carries `(port, protocol, vip)`), and an
//! unresolvable listener protocol MUST be a structured error, NEVER a
//! silent `Proto::Tcp` default (constraint C3).
//!
//! Scenario SSOT:
//! `docs/feature/udp-service-support/distill/test-scenarios.md`
//! - S-01-C: a udp listener's protocol reaches the dataplane as Udp,
//!   never defaulted to Tcp.
//! - S-01-D: the desired projection sources protocol from the listener
//!   fact, NOT from the proto-less `service_backends` row.
//! - S-01-E: NEGATIVE — an unresolvable listener protocol produces a
//!   structured Failed, NOT a silent `Proto::Tcp`-defaulted action.
//!
//! Driving ports:
//! - `project_service_desired(row, Some(&listener))` — the obs→desired seam
//!   that sources proto from the listener-bearing fact (C3 sourcing +
//!   the Failed negative arm).
//! - `ServiceMapHydrator::reconcile` — the desired→Action emission seam;
//!   asserts the emitted action proto is Udp with no Tcp default on the
//!   path.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::missing_const_for_fn)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;
use std::time::{Duration, Instant};

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{NodeId, ServiceId, ServiceVip, SpiffeId};
use overdrive_core::reconcilers::service_map_hydrator::{
    ServiceProjectionError, project_service_desired,
};
use overdrive_core::reconcilers::{
    Action, Reconciler, ServiceMapHydrator, ServiceMapHydratorState, ServiceMapHydratorView,
    TickContext,
};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    ListenerRow, LogicalTimestamp, ServiceBackendRow, ServiceBackendRowLatest,
};
use overdrive_core::wall_clock::UnixInstant;

fn vip_v4(o: u8) -> ServiceVip {
    ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 96, 0, o))).expect("valid IPv4 ServiceVip")
}

fn backend() -> Backend {
    Backend {
        alloc: SpiffeId::new("spiffe://overdrive.local/job/dns/alloc/dns-0")
            .expect("valid SpiffeId"),
        addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 1, 0, 1)), 5353),
        weight: 1,
        healthy: true,
    }
}

fn backend_row(service_id: ServiceId, vip: Ipv4Addr) -> ServiceBackendRow {
    ServiceBackendRowLatest {
        service_id,
        vip,
        backends: vec![backend()],
        updated_at: LogicalTimestamp {
            counter: 1,
            writer: NodeId::new("node-a").expect("valid NodeId"),
        },
    }
}

fn listener(port: u16, protocol: Proto, vip: ServiceVip) -> ListenerRow {
    ListenerRow { port: NonZeroU16::new(port).expect("non-zero port"), protocol, vip: Some(vip) }
}

fn make_tick(now_secs: u64) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(now_secs)),
        tick: now_secs,
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

/// S-01-D — the desired projection sources protocol from the
/// listener-bearing fact, NOT from the proto-less `service_backends` row.
/// A `udp` listener on port 5353 yields a `ServiceDesired` carrying
/// `proto Udp` and `port 5353`.
#[test]
fn proto_sourced_from_listener_fact_not_service_backends() {
    let svc = ServiceId::new(1).expect("valid ServiceId");
    let vip = vip_v4(10);
    let row = backend_row(svc, Ipv4Addr::new(10, 96, 0, 10));
    let fact = listener(5353, Proto::Udp, vip);

    let desired = project_service_desired(&row, Some(&fact))
        .expect("S-01-D: a resolvable udp listener must project");

    assert_eq!(desired.proto, Proto::Udp, "proto must be sourced from the udp listener fact");
    assert_eq!(
        desired.port,
        NonZeroU16::new(5353).unwrap(),
        "port must be sourced from the listener fact"
    );
    assert_eq!(desired.vip, vip);
}

/// S-01-C — a udp listener's protocol reaches the dataplane as Udp,
/// never defaulted to Tcp. Driving port: `reconcile` emitting the action.
#[test]
fn udp_listener_protocol_reaches_dataplane_as_udp() {
    let r = ServiceMapHydrator::canonical(
        Ipv4Addr::UNSPECIFIED,
        ipnet::Ipv4Net::new(Ipv4Addr::new(10, 99, 0, 0), 16).expect("valid /16"),
    );
    let svc = ServiceId::new(1).expect("valid ServiceId");
    let vip = vip_v4(10);
    let row = backend_row(svc, Ipv4Addr::new(10, 96, 0, 10));
    let fact = listener(5353, Proto::Udp, vip);
    let desired_svc =
        project_service_desired(&row, Some(&fact)).expect("udp listener must project");

    let mut desired = BTreeMap::new();
    desired.insert(svc, desired_svc);
    let state = ServiceMapHydratorState { desired, actual: BTreeMap::new() };
    let view = ServiceMapHydratorView::default();
    let tick = make_tick(0);

    let (actions, _next_view) = r.reconcile(&state, &state, &view, &tick);

    let proto_reaching_dataplane = actions.iter().find_map(|a| match a {
        Action::DataplaneUpdateService { proto, .. } => Some(*proto),
        _ => None,
    });
    assert_eq!(
        proto_reaching_dataplane,
        Some(Proto::Udp),
        "S-01-C: the udp listener proto must reach update_service as Udp"
    );
    // No DataplaneUpdateService action on this path may carry a Tcp proto.
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, Action::DataplaneUpdateService { proto: Proto::Tcp, .. })),
        "S-01-C: no Tcp-defaulted update_service action may appear on the udp path"
    );
}

/// S-01-E — NEGATIVE (the C3 load-bearing defense): a desired projection
/// with NO resolvable listener protocol produces a structured error, and
/// does NOT silently default to `Proto::Tcp`.
#[test]
fn unresolvable_listener_proto_is_structured_error_not_tcp_default() {
    let svc = ServiceId::new(1).expect("valid ServiceId");
    let row = backend_row(svc, Ipv4Addr::new(10, 96, 0, 10));
    // No listener fact at all — proto is unresolvable.
    let result = project_service_desired(&row, None);

    assert!(
        matches!(result, Err(ServiceProjectionError::NoListenerProto { .. })),
        "S-01-E: an unresolvable listener proto must be a structured error, \
         got {result:?}"
    );
    // The structured error means NO ServiceDesired (and therefore no
    // Tcp-defaulted action) is produced — assert the absence directly.
    assert!(
        result.is_err(),
        "S-01-E: no ServiceDesired may be silently produced with a Tcp default"
    );
}

/// Regression (review of `service_map_hydrator.rs:111`): the projection
/// takes a SINGLE `Option<&ListenerRow>`, not a `&[ListenerRow]`.
/// The caller pre-resolves the one fact via
/// `ListenerFactStore::fact_for(row.service_id)` — an O(1) keyed read on the
/// full `(vip, port, proto)` identity — so a slice of multiple same-VIP
/// listeners co-locating protocols (e.g. tcp/53 + udp/53) is structurally
/// unrepresentable and can no longer be silently mis-resolved to the first
/// entry. The retained VIP guard rejects a fact handed in for a different
/// VIP rather than mis-stamping `(port, proto)` onto the projection.
#[test]
fn listener_fact_for_a_different_vip_does_not_project() {
    let svc = ServiceId::new(1).expect("valid ServiceId");
    let row = backend_row(svc, Ipv4Addr::new(10, 96, 0, 10));
    // A listener fact whose VIP differs from the row's VIP must NOT be
    // stamped onto the projection — the guard rejects it as unresolvable
    // rather than silently sourcing the wrong port/proto.
    let wrong_vip_fact = listener(53, Proto::Tcp, vip_v4(99));

    let result = project_service_desired(&row, Some(&wrong_vip_fact));

    assert!(
        matches!(result, Err(ServiceProjectionError::NoListenerProto { .. })),
        "a listener fact for a different VIP must not mis-stamp (port, proto); got {result:?}"
    );
}
