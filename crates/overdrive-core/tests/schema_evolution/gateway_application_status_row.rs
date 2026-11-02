//! Schema-evolution golden-bytes test — `GatewayApplicationStatusRowEnvelope`
//! (public ingress). Pins the V1 archived layout of the redacted Gateway
//! Application status row.
//!
//! **`FIXTURE_V1` is never touched.** Bumping the envelope to `V2` adds a new
//! `FIXTURE_V2` constant and a new assertion in the same commit
//! (`development.md` § "Version-bump procedure"). The payload is a frozen
//! hand-pinned `GatewayApplicationStatusRowEnvelope::V1`, never built through
//! an alias.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::dataplane::Proto;
use overdrive_core::gateway_identity::GatewayIdentityEpoch;
use overdrive_core::id::{CertSerial, ServiceVip};
use overdrive_core::public_ingress::{
    GatewayApplicationStatusRowEnvelope, GatewayApplicationUnavailableCause,
    GatewayConnectPathStatus, GatewayIdentityStatus, GatewayListenerStatus,
};

use super::harness::decode_envelope_v_latest;

/// Hex-encoded rkyv-archived bytes of
/// `GatewayApplicationStatusRowEnvelope::V1`: node `gateway-node`, listener
/// bound at `127.0.0.2:443`, no staged generation, a current generation
/// (application `[4; 32]`, Route `public-api` generation `[9; 32]`, Service key
/// `10.96.0.7:8080/tcp`), one draining generation (application `[5; 32]`, Route
/// generation `[10; 32]`), unavailable cause `WatchGap`, a current gateway
/// identity (epoch 1, `spiffe://overdrive.local/gateway/gateway-node`, serial
/// `0a1b2c`, expiry 3 000 s), connect path `Ready` and the logical timestamp
/// `(2, gateway-node)`.
const FIXTURE_V1: &str = "676174657761792d6e6f64657075626c69632d6170697075626c69632d61706905050505050505050505050505050505050505050505050505050505050505058a000000d6ffffff0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a000a60000700000000000000000000000000901f000000007370696666653a2f2f6f76657264726976652e6c6f63616c2f676174657761792f676174657761792d6e6f6465676174657761792d6e6f64650000000000000000000000000000008c00000038ffffff01007f000002bb01000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100000004040404040404040404040404040404040404040404040404040404040404048a000000acfeffff0909090909090909090909090909090909090909090909090909090909090909000a60000700000000000000000000000000901f0000000080feffff01000000010800000000000001000000000000000100000000000000ad000000c0feffff18000000306131623263ffff00000000b80b0000000000000000000000000000010000000000000002000000000000008c000000b5feffff";

/// CONTRACT_SHAPE: pure-function.
#[test]
fn gateway_application_status_v1_decodes_through_current_envelope() {
    let latest = decode_envelope_v_latest::<GatewayApplicationStatusRowEnvelope>(FIXTURE_V1);
    let vip = ServiceVip::new("10.96.0.7".parse().expect("IP")).expect("VIP");

    assert_eq!(latest.node_id.as_str(), "gateway-node");
    assert_eq!(
        latest.listener,
        GatewayListenerStatus::Bound {
            address: SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 2), 443)
        },
    );
    assert!(latest.staged.is_none());

    let current = latest.current.as_ref().expect("a current generation");
    assert_eq!(current.application_generation.as_bytes(), &[4u8; 32]);
    assert_eq!(current.route_id.as_str(), "public-api");
    assert_eq!(current.route_generation.as_bytes(), &[9u8; 32]);
    assert_eq!(current.service_key.vip(), vip);
    assert_eq!(current.service_key.port().get(), 8080);
    assert_eq!(current.service_key.protocol(), Proto::Tcp);

    assert_eq!(latest.draining.len(), 1);
    let draining = &latest.draining[0];
    assert_eq!(draining.application_generation.as_bytes(), &[5u8; 32]);
    assert_eq!(draining.route_id.as_str(), "public-api");
    assert_eq!(draining.route_generation.as_bytes(), &[10u8; 32]);
    assert_eq!(draining.service_key.vip(), vip);

    assert_eq!(latest.unavailable, Some(GatewayApplicationUnavailableCause::WatchGap));
    match &latest.gateway_identity {
        GatewayIdentityStatus::Current { epoch, spiffe_id, serial, not_after } => {
            assert_eq!(*epoch, GatewayIdentityEpoch::first());
            assert_eq!(spiffe_id.as_str(), "spiffe://overdrive.local/gateway/gateway-node");
            assert_eq!(serial, &CertSerial::new("0a1b2c").expect("serial"));
            assert_eq!(*not_after, UnixInstant::from_unix_duration(Duration::from_secs(3_000)));
        }
        other => panic!("expected the Current identity, got {other:?}"),
    }
    assert_eq!(latest.connect_path, GatewayConnectPathStatus::Ready);
    assert_eq!(latest.updated_at.counter, 2);
    assert_eq!(latest.updated_at.writer.as_str(), "gateway-node");
}
