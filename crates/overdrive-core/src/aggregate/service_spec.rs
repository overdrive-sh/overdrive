//! `ServiceSpec` aggregate + direct per-type rkyv envelope.
//!
//! The parser-side `ServiceSpec` is the validated `[service]` body
//! that flows through the IntentStore and lives in the
//! `WorkloadSpec::Service(_)` variant. The greenfield persistence
//! contract keeps the VM-only V1 payload frozen and appends V2 (`[vm.volume]`).

use serde::{Deserialize, Serialize};

use super::probe_descriptor::ProbeDescriptor;
use super::workload_spec::{DriverInput, DriverInputV1, Listener, ResourcesInput};
use crate::codec::{EnvelopeError, VersionedEnvelope};

/// Public payload alias for the parser-side `ServiceSpec` aggregate.
/// Per ADR-0048 UI-02 alias-to-payload: this points at the latest
/// payload struct so call sites construct values via struct-literal
/// syntax (`ServiceSpec { id, replicas, driver, resources, listeners,
/// startup_probes, readiness_probes, liveness_probes }`).
pub type ServiceSpec = ServiceSpecV2;

/// Documentation alias for "the latest payload variant of
/// [`ServiceSpecEnvelope`]".
pub type ServiceSpecLatest = ServiceSpecV2;

/// Per-type rkyv envelope for the parser-side `ServiceSpec` aggregate.
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
)]
pub enum ServiceSpecEnvelope {
    V1(ServiceSpecV1),
    V2(ServiceSpecV2),
}

/// **FROZEN.** V1 payload for the parser-side `ServiceSpec` aggregate, before
/// `[vm.volume]` existed.
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
)]
pub struct ServiceSpecV1 {
    pub id: String,
    pub replicas: u32,
    pub driver: DriverInputV1,
    pub resources: ResourcesInput,
    pub listeners: Vec<Listener>,
    pub startup_probes: Vec<ProbeDescriptor>,
    pub readiness_probes: Vec<ProbeDescriptor>,
    pub liveness_probes: Vec<ProbeDescriptor>,
}

/// Current VM-only payload for the parser-side `ServiceSpec` aggregate: V1
/// plus the optional `[vm.volume]` declaration carried on the driver table.
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
pub struct ServiceSpecV2 {
    pub id: String,
    pub replicas: u32,
    pub driver: DriverInput,
    pub resources: ResourcesInput,
    pub listeners: Vec<Listener>,
    pub startup_probes: Vec<ProbeDescriptor>,
    pub readiness_probes: Vec<ProbeDescriptor>,
    pub liveness_probes: Vec<ProbeDescriptor>,
}

impl From<ServiceSpecV1> for ServiceSpecV2 {
    fn from(v1: ServiceSpecV1) -> Self {
        Self {
            id: v1.id,
            replicas: v1.replicas,
            driver: v1.driver.into(),
            resources: v1.resources,
            listeners: v1.listeners,
            startup_probes: v1.startup_probes,
            readiness_probes: v1.readiness_probes,
            liveness_probes: v1.liveness_probes,
        }
    }
}

impl VersionedEnvelope for ServiceSpecEnvelope {
    type Latest = ServiceSpecV2;

    fn latest(payload: Self::Latest) -> Self {
        Self::V2(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(payload) => Ok(payload.into()),
            Self::V2(payload) => Ok(payload),
        }
    }

    fn known_discriminants() -> &'static [u8] {
        &[0, 1]
    }

    fn type_name() -> &'static str {
        "ServiceSpecEnvelope"
    }
}
