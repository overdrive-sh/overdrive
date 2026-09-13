//! Gateway-connect intent/receipt and applied-backend-identity adapter.
//!
//! SCAFFOLD: true. The exact port, POD ABI, and real-probe caller shape are
//! compiled now; DELIVER replaces only the marked BPF effects.

#![expect(clippy::todo, reason = "public-ingress DISTILL RED scaffolds")]

use std::time::Instant;

use async_trait::async_trait;
use overdrive_core::id::{BackendId, SpiffeId};
use overdrive_core::public_ingress::{GatewayConnectIntent, GatewaySelectionReceipt};
use overdrive_core::traits::clock::Clock;
use overdrive_gateway::ports::{
    GatewayConnectCleanupSweep, GatewayConnectDataplane, GatewayConnectDataplaneError,
};

#[expect(dead_code, reason = "activated by the DELIVER gateway map loader")]
pub(crate) const GATEWAY_CONNECT_MAP_CAPACITY: u32 = 128;
#[expect(dead_code, reason = "activated by the DELIVER receipt decoder")]
pub(crate) const GATEWAY_RECEIPT_NO_BACKEND: u32 = 1;
#[expect(dead_code, reason = "activated by the DELIVER receipt decoder")]
pub(crate) const GATEWAY_RECEIPT_SELECTED: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct GatewayConnectIntentPod {
    pub vip_host: u32,
    pub port_host: u16,
    pub proto: u8,
    pub _pad: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct GatewaySelectionReceiptPod {
    pub outcome: u32,
    pub backend_id: u32,
}

const _: [(); 8] = [(); std::mem::size_of::<GatewayConnectIntentPod>()];
const _: [(); 8] = [(); std::mem::size_of::<GatewaySelectionReceiptPod>()];
const _: [(); 4] = [(); std::mem::align_of::<GatewayConnectIntentPod>()];
const _: [(); 4] = [(); std::mem::align_of::<GatewaySelectionReceiptPod>()];

// SAFETY: Both structs are repr(C), Copy, contain no padding left
// uninitialized by constructors, and have compile-time size/alignment pins.
unsafe impl aya::Pod for GatewayConnectIntentPod {}
// SAFETY: Same representation proof as GatewayConnectIntentPod.
unsafe impl aya::Pod for GatewaySelectionReceiptPod {}

#[async_trait]
impl GatewayConnectDataplane for crate::EbpfDataplane {
    async fn probe(
        &self,
        _deadline: Instant,
        _clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: EbpfDataplane gateway-connect real cgroup probe")
    }

    fn register(&self, _intent: GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: gateway-connect intent registration")
    }

    fn take_receipt(
        &self,
        _intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: gateway-connect receipt consumption")
    }

    fn cleanup(&self, _intent: &GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: gateway-connect per-intent cleanup")
    }

    fn cleanup_all_gateway_intents(
        &self,
    ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: gateway-connect shutdown sweep")
    }

    fn selected_backend_identity(
        &self,
        _backend_id: BackendId,
    ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: committed BackendId applied-identity lookup")
    }

    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: exact gateway intent count")
    }

    fn live_receipt_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: exact gateway receipt count")
    }
}
