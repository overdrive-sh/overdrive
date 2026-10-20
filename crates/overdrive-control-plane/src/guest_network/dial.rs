//! INB-1 (ADR-0173, ADR-0174, ADR-0187): the host guest-flow owner is the
//! production `GuestStreamDial` (§ *Driving port — guest stream dial
//! `GuestStreamDial`*). A host platform client (leg-S, the VM probe runner)
//! reaches a declared TCP port of an Active VM allocation only through a
//! flow this owner opens over vsock.
//!
//! Order of decision (the owner, in the target CID's serialized order, M-7):
//! not an Active lease's `workload_addr` with a declared port →
//! `NotServing`; node forwarding quiesced → `Quiesced`; the CID's control
//! session not live as the owner observes it → `NoSession`; node flow slots
//! full, or (for `DialPurpose::Relay` only) the allocation's quota full →
//! `Capacity`. Steps 1–4 return immediately and change nothing. Then:
//! `connect_guest(cid)` → `watch(V_h)` (EVMOD-END; a watch refused
//! `HalfEnded` ends the dial `Unreachable { errno }`, EVMOD-ATTACH-END) → the
//! TcpAccept request (peer = gateway:0, the declared port, the live launch
//! token) → the guest's `Paired` / `Refused { errno }` / `TimedOut { errno }`,
//! or `GUEST_DIAL_PAIRING_DEADLINE` → on `Paired`, `into_guest_stream(V_h,
//! watch, flow, target)`. The stream resolves every end it observes from the
//! watch (EVMOD-STREAM-END); an owner abort ends it at once without touching
//! `V_h` (EVMOD-STREAM-ABORT, WATCH-ABORT); its drop aborts the watch, then
//! closes `V_h`; the node flow slot and, for `Relay`, the quota place are
//! released at the drop or the owner's abort (EVMOD-AB).
//!
//! RED scaffold: the dial lands in DELIVER step 9 (attachment cut, DG-2
//! row 19).

// SCAFFOLD: true — netns-density-295 DISTILL (vsock DELIVER attachment-cut step 9, INB-1).

use std::net::SocketAddrV4;

use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestDialError, GuestStream, GuestStreamDial,
};

use super::HostSharedGuestNetworkOwner;

#[async_trait::async_trait]
impl GuestStreamDial for HostSharedGuestNetworkOwner {
    async fn dial(
        &self,
        _target: SocketAddrV4,
        _purpose: DialPurpose,
    ) -> Result<GuestStream, GuestDialError> {
        panic!("Not yet implemented -- RED scaffold")
    }
}
