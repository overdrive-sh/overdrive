//! `SimDataplane` — in-memory implementation of the [`Dataplane`] port.
//!
//! The real dataplane loads eBPF programs via aya-rs and writes BPF
//! maps. The sim dataplane stores the *same* maps in memory so that
//! control-plane logic can assert "after this write, the BPF map
//! reflects the change" without loading a kernel. Flow events are
//! pre-seeded by the test author; `drain_flow_events` returns them in
//! FIFO order and empties the queue.
//!
//! # Iteration determinism
//!
//! `services` is a [`BTreeMap`], not a [`HashMap`], per
//! `.claude/rules/development.md` § Ordered-collection choice. DST
//! harnesses observe iteration order via invariant evaluators (and
//! later, via map-iteration callsites in the slice-08 hydrator) — a
//! `HashMap`'s `RandomState`-driven order would violate the K3
//! *seed → bit-identical trajectory* property that whitepaper § 21
//! pins.
//!
//! `policy` retains its [`HashMap`] storage because `PolicyKey` (a
//! `(SpiffeId, SpiffeId)` pair) is point-accessed only — DST
//! invariants read it via `policy_verdict(&key)`, never iterate
//! over it. Promoting it to `BTreeMap` would require adding `Ord`
//! to `PolicyKey` solely for storage convenience, which is wider
//! than the dst-lint clause asks for.

use std::collections::{BTreeMap, HashMap};
use std::net::Ipv4Addr;

use async_trait::async_trait;
use parking_lot::Mutex;

use overdrive_core::traits::dataplane::{
    Backend, Dataplane, DataplaneError, FlowEvent, PolicyKey, Verdict,
};

/// Sim dataplane state. The `services` map is a `BTreeMap` keyed by
/// `Ord` on `Ipv4Addr` (load-bearing for DST seed reproducibility —
/// see module docs); `policy` stays a point-accessed `HashMap`.
pub struct SimDataplane {
    // dst-lint: hashmap-ok point-accessed only via `policy_verdict`; never iterated
    policy: Mutex<HashMap<PolicyKey, Verdict>>,
    services: Mutex<BTreeMap<Ipv4Addr, Vec<Backend>>>,
    flow_events: Mutex<Vec<FlowEvent>>,
}

impl SimDataplane {
    /// Construct an empty sim dataplane.
    #[must_use]
    pub fn new() -> Self {
        Self {
            policy: Mutex::new(HashMap::new()),
            services: Mutex::new(BTreeMap::new()),
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

    /// Enumerate every VIP currently registered, in `Ord` order on
    /// [`Ipv4Addr`]. Iteration order is a function of the keys (the
    /// `BTreeMap` invariant), never of insertion history — this is
    /// the property DST seed reproducibility relies on.
    ///
    /// Not part of the `Dataplane` trait — this accessor is for
    /// tests and DST invariant evaluators that need to assert on
    /// the stored map's iteration order directly.
    #[must_use]
    pub fn service_vip_keys(&self) -> Vec<Ipv4Addr> {
        self.services.lock().keys().copied().collect()
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
