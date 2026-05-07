//! `NoopDataplane` — production-side `Dataplane` stub used by single-mode
//! Phase 2.2 boot before the kernel-side `EbpfDataplane` is wired through
//! `AppState`.
//!
//! This is a temporary placeholder. The Slice 08-02 reconciler that emits
//! `Action::DataplaneUpdateService` ships in a follow-up step; until that
//! reconciler is registered with the runtime, the production action shim
//! never reaches the dispatch arm that consumes the dataplane. The noop
//! impl returns `Ok(())` for every operation so the wiring compiles
//! without forcing the (Linux-only) `EbpfDataplane` into the
//! single-mode default boot path.
//!
//! Tests that exercise `Action::DataplaneUpdateService` install
//! `Arc<SimDataplane>` (or a fault-injecting wrapper) directly — they
//! never see this noop.

use std::net::Ipv4Addr;

use async_trait::async_trait;
use overdrive_core::traits::dataplane::{
    Backend, Dataplane, DataplaneError, FlowEvent, PolicyKey, Verdict,
};

/// A no-op [`Dataplane`] used as the production single-mode default.
///
/// Returns `Ok(())` from every write and an empty `Vec` from
/// `drain_flow_events`. Phase 2.2's hydrator reconciler is the only
/// emitter of `Action::DataplaneUpdateService`; until that reconciler
/// is registered, the action shim's dataplane parameter is unreachable
/// at runtime.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopDataplane;

#[async_trait]
impl Dataplane for NoopDataplane {
    async fn update_policy(
        &self,
        _key: PolicyKey,
        _verdict: Verdict,
    ) -> Result<(), DataplaneError> {
        Ok(())
    }

    async fn update_service(
        &self,
        _vip: Ipv4Addr,
        _backends: Vec<Backend>,
    ) -> Result<(), DataplaneError> {
        Ok(())
    }

    async fn drain_flow_events(&self) -> Result<Vec<FlowEvent>, DataplaneError> {
        Ok(Vec::new())
    }
}
