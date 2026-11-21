//! `ServiceSpec` — the parser-side validated `[service]` body.
//!
//! It lives in the `WorkloadSpec::Service(_)` variant. A Service is persisted
//! through `WorkloadIntentEnvelope`, not through this type.

use serde::{Deserialize, Serialize};

use super::probe_descriptor::ProbeDescriptor;
use super::workload_spec::{DriverInput, Listener, ResourcesInput};

/// The parser-side `[service]` aggregate: identity, replica count, driver
/// table, resources, listeners, and the three probe roles.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    utoipa::ToSchema,
)]
pub struct ServiceSpec {
    pub id: String,
    pub replicas: u32,
    pub driver: DriverInput,
    pub resources: ResourcesInput,
    pub listeners: Vec<Listener>,
    pub startup_probes: Vec<ProbeDescriptor>,
    pub readiness_probes: Vec<ProbeDescriptor>,
    pub liveness_probes: Vec<ProbeDescriptor>,
}
