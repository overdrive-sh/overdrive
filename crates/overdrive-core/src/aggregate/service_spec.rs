//! `ServiceSpec` aggregate + direct per-type rkyv envelope.
//!
//! The parser-side `ServiceSpec` is the validated `[service]` body
//! that flows through the IntentStore and lives in the
//! `WorkloadSpec::Service(_)` variant. The greenfield persistence
//! contract retains one direct V3 payload and one tag-zero codec arm.

use serde::{Deserialize, Serialize};

use super::probe_descriptor::ProbeDescriptor;
use super::workload_spec::{DriverInput, Listener, ResourcesInput};
use crate::codec::{EnvelopeError, VersionedEnvelope};

/// Public payload alias for the parser-side `ServiceSpec` aggregate.
/// Per ADR-0048 UI-02 alias-to-payload: this points at the latest
/// payload struct so call sites construct values via struct-literal
/// syntax (`ServiceSpec { id, replicas, driver, resources, listeners,
/// startup_probes, readiness_probes, liveness_probes }`).
pub type ServiceSpec = ServiceSpecV3;

/// Documentation alias for "the latest payload variant of
/// [`ServiceSpecEnvelope`]".
pub type ServiceSpecLatest = ServiceSpecV3;

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
    V3(ServiceSpecV3),
}

/// V3 payload — replaces the parser-only Exec field with the existing
/// driver union.
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
pub struct ServiceSpecV3 {
    pub id: String,
    pub replicas: u32,
    pub driver: DriverInput,
    pub resources: ResourcesInput,
    pub listeners: Vec<Listener>,
    pub startup_probes: Vec<ProbeDescriptor>,
    pub readiness_probes: Vec<ProbeDescriptor>,
    pub liveness_probes: Vec<ProbeDescriptor>,
}

impl VersionedEnvelope for ServiceSpecEnvelope {
    type Latest = ServiceSpecV3;

    fn latest(payload: Self::Latest) -> Self {
        Self::V3(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        let Self::V3(payload) = self;
        Ok(payload)
    }

    fn known_discriminants() -> &'static [u8] {
        &[0]
    }

    fn type_name() -> &'static str {
        "ServiceSpecEnvelope"
    }
}
