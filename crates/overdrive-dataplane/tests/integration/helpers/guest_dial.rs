//! A guest-flow dial port that serves no VM, for `HostMtlsEnforcement`
//! fixtures whose inbound targets are never inside the guest prefix
//! (LEGS-DIAL: the enforcement takes the owner's `GuestStreamDial` as a
//! required constructor argument).

use std::net::SocketAddrV4;
use std::sync::Arc;

use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestDialError, GuestStream, GuestStreamDial,
};

/// Answers every dial `NotServing` — the production owner's answer for an
/// address that is not an Active VM allocation's declared port.
struct NotServingDial;

#[async_trait::async_trait]
impl GuestStreamDial for NotServingDial {
    async fn dial(
        &self,
        _target: SocketAddrV4,
        _purpose: DialPurpose,
    ) -> Result<GuestStream, GuestDialError> {
        Err(GuestDialError::NotServing)
    }
}

/// The enforcement's `guest_dial` for a fixture with no VM backend.
pub fn not_serving() -> Arc<dyn GuestStreamDial> {
    Arc::new(NotServingDial)
}
