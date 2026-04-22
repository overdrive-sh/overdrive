//! `SimDataplane` — in-memory implementation of the [`Dataplane`] port.
//!
//! The real dataplane loads eBPF programs via aya-rs and writes BPF
//! maps. The sim dataplane stores the *same* maps in memory so that
//! control-plane logic can assert "after this write, the BPF map
//! reflects the change" without loading a kernel. Flow events are
//! pre-seeded by the test author; `drain_flow_events` returns them in
//! FIFO order and empties the queue.

use std::collections::HashMap;
use std::net::Ipv4Addr;

use async_trait::async_trait;
use parking_lot::Mutex;

use overdrive_core::traits::dataplane::{
    Backend, Dataplane, DataplaneError, FlowEvent, PolicyKey, Verdict,
};

/// Sim dataplane state. Each field is a plain `HashMap` / `Vec` behind
/// a mutex — DST workloads are synchronous-in-tick, so lock contention
/// is irrelevant.
pub struct SimDataplane {
    policy: Mutex<HashMap<PolicyKey, Verdict>>,
    services: Mutex<HashMap<Ipv4Addr, Vec<Backend>>>,
    flow_events: Mutex<Vec<FlowEvent>>,
}

impl SimDataplane {
    /// Construct an empty sim dataplane.
    #[must_use]
    pub fn new() -> Self {
        Self {
            policy: Mutex::new(HashMap::new()),
            services: Mutex::new(HashMap::new()),
            flow_events: Mutex::new(Vec::new()),
        }
    }

    /// Queue a flow event for the next `drain_flow_events` call.
    /// Tests use this to stage the telemetry the dataplane would have
    /// emitted from the kernel in a real run.
    pub fn enqueue_flow_event(&self, event: FlowEvent) {
        self.flow_events.lock().push(event);
    }

    /// Read the verdict currently stored for `key`, if any. Not part
    /// of the `Dataplane` trait — callers that use the `Dataplane`
    /// surface read verdicts by replaying flow events; this accessor
    /// is for tests that want to assert on the stored map directly.
    #[must_use]
    pub fn policy_verdict(&self, key: &PolicyKey) -> Option<Verdict> {
        self.policy.lock().get(key).copied()
    }

    /// Read the backend set currently stored for a service VIP.
    #[must_use]
    pub fn service_backends(&self, vip: Ipv4Addr) -> Option<Vec<Backend>> {
        self.services.lock().get(&vip).cloned()
    }
}

impl Default for SimDataplane {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Dataplane for SimDataplane {
    async fn update_policy(&self, key: PolicyKey, verdict: Verdict) -> Result<(), DataplaneError> {
        self.policy.lock().insert(key, verdict);
        Ok(())
    }

    async fn update_service(
        &self,
        vip: Ipv4Addr,
        backends: Vec<Backend>,
    ) -> Result<(), DataplaneError> {
        self.services.lock().insert(vip, backends);
        Ok(())
    }

    async fn drain_flow_events(&self) -> Result<Vec<FlowEvent>, DataplaneError> {
        Ok(std::mem::take(&mut *self.flow_events.lock()))
    }
}
