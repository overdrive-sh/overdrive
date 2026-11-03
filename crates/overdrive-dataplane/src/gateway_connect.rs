//! `GatewayConnectDataplane` for [`EbpfDataplane`] (public-ingress-gateway,
//! ADR-0228 / ADR-0232 / ADR-0233 / ADR-0243).
//!
//! The dataplane prepares the gateway's upstream socket (marks it
//! `MTLS_TPROXY_MARK`, loopback-binds it, reads its socket cookie) and
//! registers `(cookie, ServiceKey)` in `GATEWAY_CONNECT_INTENT_MAP`; the
//! separate gateway selection `cgroup/connect4` program writes the receipt into
//! `GATEWAY_SELECTION_RECEIPT_MAP`; `take_receipt` consumes it. Applied backend
//! identities are published only after the outer `SERVICE_MAP` swap commits
//! (the commit-guarded `update_service` transaction).
//!
//! SCAFFOLD: true. Only the exact shapes the DESIGN pins are present: the
//! private POD ABI of the two maps (mirrored by `overdrive-bpf`) and the port
//! implementation. Every port body fails with SCAFFOLD text until DELIVER. The
//! commit-guard internals (`BackendIdState`, `ServiceApplyUndo`,
//! `ServiceCleanupDebt`, the `service_apply_commit` guard) are private
//! structure and are the crafter's.

#![allow(
    clippy::todo,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::unused_self,
    clippy::pub_underscore_fields,
    dead_code,
    unused_variables,
    reason = "public-ingress DISTILL scaffold: bodies are DELIVER's"
)]

use std::collections::BTreeMap;
use std::time::Instant;

use async_trait::async_trait;
use overdrive_core::id::{BackendId, SpiffeId};
use overdrive_core::public_ingress::{ServiceKey, SocketCookie};
use overdrive_core::traits::clock::Clock;
use overdrive_gateway::ports::{
    GatewayConnectCleanup, GatewayConnectCleanupSweep, GatewayConnectDataplane,
    GatewayConnectDataplaneError, GatewayConnectIntent, GatewaySelectionReceipt, PreparedUpstream,
};

use crate::EbpfDataplane;

/// Capacity of `GATEWAY_CONNECT_INTENT_MAP` and `GATEWAY_SELECTION_RECEIPT_MAP`.
pub(crate) const GATEWAY_CONNECT_MAP_CAPACITY: u32 = 128;
/// Receipt outcome: no usable backend.
pub(crate) const GATEWAY_RECEIPT_NO_BACKEND: u32 = 1;
/// Receipt outcome: a nonzero `BackendId` was selected.
pub(crate) const GATEWAY_RECEIPT_SELECTED: u32 = 2;

/// Registered intent value: exactly the Service-map key, host-order.
/// Mirrors `overdrive_bpf::maps::gateway_connect::GatewayConnectIntentPod`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct GatewayConnectIntentPod {
    pub vip_host: u32,
    pub port_host: u16,
    pub proto: u8,
    pub _pad: u8,
}

/// Selection receipt value. Mirrors
/// `overdrive_bpf::maps::gateway_connect::GatewaySelectionReceiptPod`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct GatewaySelectionReceiptPod {
    pub outcome: u32,
    pub backend_id: u32,
}

// SAFETY: `repr(C)`, eight bytes, every byte initialised (`_pad` is always
// zero), no interior pointers.
unsafe impl aya::Pod for GatewayConnectIntentPod {}
// SAFETY: `repr(C)`, eight bytes, two `u32` fields, no padding.
unsafe impl aya::Pod for GatewaySelectionReceiptPod {}

const _: () = assert!(core::mem::size_of::<GatewayConnectIntentPod>() == 8);
const _: () = assert!(core::mem::align_of::<GatewayConnectIntentPod>() == 4);
const _: () = assert!(core::mem::size_of::<GatewaySelectionReceiptPod>() == 8);
const _: () = assert!(core::mem::align_of::<GatewaySelectionReceiptPod>() == 4);

impl GatewayConnectCleanup for EbpfDataplane {
    fn cleanup(&self, intent: &GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: EbpfDataplane::cleanup")
    }

    fn cleanup_all_gateway_intents(
        &self,
    ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: EbpfDataplane::cleanup_all_gateway_intents")
    }

    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: EbpfDataplane::live_intent_count")
    }

    fn live_receipts(
        &self,
    ) -> Result<BTreeMap<SocketCookie, GatewaySelectionReceipt>, GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: EbpfDataplane::live_receipts")
    }
}

#[async_trait]
impl GatewayConnectDataplane for EbpfDataplane {
    async fn probe(
        &self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: EbpfDataplane::probe (GatewayConnectDataplane)")
    }

    fn open_upstream(
        &self,
        key: ServiceKey,
    ) -> Result<PreparedUpstream, GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: EbpfDataplane::open_upstream")
    }

    fn take_receipt(
        &self,
        intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: EbpfDataplane::take_receipt")
    }

    fn selected_backend_identity(
        &self,
        backend_id: BackendId,
    ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
        todo!("SCAFFOLD: EbpfDataplane::selected_backend_identity")
    }
}
