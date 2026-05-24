//! `ServiceSpec` aggregate + per-type rkyv versioned envelope per
//! ADR-0048 § 4 + ADR-0057 § 5.
//!
//! The parser-side `ServiceSpec` is the validated `[service]` body
//! that flows through the IntentStore and lives in the
//! `WorkloadSpec::Service(_)` variant. Per ADR-0057 the type carries
//! three `Vec<ProbeDescriptor>` fields (startup / readiness /
//! liveness) — service-health-check-probes step 01-02 lands the
//! V1 → V2 envelope bump that adds them.
//!
//! # Why a per-type envelope
//!
//! Per ADR-0048 § 4 every rkyv-persisted type at a redb boundary
//! goes through a per-type versioned envelope. `ServiceSpec` is
//! persisted alongside the rest of `WorkloadIntent` via the typed
//! [`crate::aggregate::WorkloadIntentV1::archive_for_store`] codec;
//! its embedded payload (under `WorkloadIntentV1::Service(ServiceSpec)`)
//! benefits from an additional per-type envelope so future shape
//! changes (next probe field, next protocol, …) land additively
//! without bumping every other workload kind's envelope.
//!
//! # Version-bump procedure (single-commit)
//!
//! Per `.claude/rules/development.md` § "rkyv schema evolution" →
//! "Version-bump procedure", the V1 → V2 landing in step 01-02
//! lands all six steps in one commit:
//!
//! 1. New `V2` variant appended to `ServiceSpecEnvelope` (do NOT
//!    reorder existing variants — rkyv discriminant tags are positional).
//! 2. `pub type ServiceSpec = ServiceSpecV2` re-aliased (UI-02
//!    alias-to-payload). `ServiceSpecLatest = ServiceSpecV2`.
//! 3. `Envelope::latest()` constructor updated to wrap into V2.
//! 4. `From<ServiceSpecV1> for ServiceSpecV2` impl (additive — V1
//!    specs have zero probes; the projection fills the three Vecs
//!    with `vec![]`). `into_latest()` chains V1 → V2 via this impl.
//! 5. New golden-bytes fixture (`FIXTURE_V2`) pins the V2 archived
//!    bytes; `FIXTURE_V1` (pinned in this same commit) is NEVER
//!    touched on subsequent bumps.
//! 6. All five changes land together.
//!
//! # Discriminant offset
//!
//! `discriminant_offset_from_end()` returns `None` for the initial
//! landing — empirical re-pin lands when the unknown-version probe
//! becomes load-bearing for this envelope. The golden-bytes fixtures
//! remain the load-bearing schema-evolution defense regardless.

use serde::{Deserialize, Serialize};

use super::probe_descriptor::ProbeDescriptor;
use super::workload_spec::{ExecInput, Listener, ResourcesInput};
use crate::codec::{EnvelopeError, VersionedEnvelope};

/// Public payload alias for the parser-side `ServiceSpec` aggregate.
/// Per ADR-0048 UI-02 alias-to-payload: this points at the latest
/// payload struct so call sites construct values via struct-literal
/// syntax (`ServiceSpec { id, replicas, exec, resources, listeners,
/// startup_probes, readiness_probes, liveness_probes }`).
pub type ServiceSpec = ServiceSpecV2;

/// Documentation alias for "the latest payload variant of
/// [`ServiceSpecEnvelope`]".
pub type ServiceSpecLatest = ServiceSpecV2;

/// Per-type rkyv versioned envelope for the parser-side `ServiceSpec`
/// aggregate per ADR-0048 § 4 + ADR-0057 § 5.
///
/// Codec-internal — named only inside the persistence-boundary code
/// (and inside this module's `From` / `into_latest` impls). Public
/// callers use [`ServiceSpec`] and construct values via struct-literal
/// syntax.
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
    /// Initial parser shape before service-health-check-probes landed.
    V1(ServiceSpecV1),
    /// Service-health-check-probes step 01-02 — adds the three
    /// `Vec<ProbeDescriptor>` fields per ADR-0057.
    V2(ServiceSpecV2),
}

/// V1 payload — the parser-side `ServiceSpec` shape BEFORE
/// service-health-check-probes landed. Pinned for schema-evolution
/// purposes; never constructed by the parser today.
///
/// rkyv archives are **fixed positional layouts** — this struct is
/// frozen. Any change to V1 fields invalidates `FIXTURE_V1` and is
/// rejected at review time.
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
pub struct ServiceSpecV1 {
    pub id: String,
    pub replicas: u32,
    pub exec: ExecInput,
    pub resources: ResourcesInput,
    pub listeners: Vec<Listener>,
}

/// V2 payload — current parser-side `ServiceSpec`. Adds three
/// `Vec<ProbeDescriptor>` fields per ADR-0057 § 5.
///
/// Per ADR-0058 the parser populates `startup_probes` with a single
/// inferred default TCP probe when zero probes are declared AND the
/// service has at least one `[[listener]]`. An explicit empty
/// `[[health_check.startup]] = []` array opts out of inference and
/// leaves `startup_probes` empty.
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
    pub exec: ExecInput,
    pub resources: ResourcesInput,
    pub listeners: Vec<Listener>,
    /// Per ADR-0057 §5 — operator-declared startup probes plus any
    /// platform-synthesised default (per ADR-0058). Empty IFF the
    /// operator wrote `[[health_check.startup]] = []` (explicit
    /// opt-out, preserves Phase-1 first-Running semantics).
    pub startup_probes: Vec<ProbeDescriptor>,
    /// Per ADR-0057 §5 — operator-declared readiness probes.
    /// Populated by step 02-01; reserved here for V2 envelope shape
    /// stability.
    pub readiness_probes: Vec<ProbeDescriptor>,
    /// Per ADR-0057 §5 — operator-declared liveness probes.
    /// Populated by step 02-02; reserved here for V2 envelope shape
    /// stability.
    pub liveness_probes: Vec<ProbeDescriptor>,
}

/// Additive projection from the V1 payload to the V2 shape. Old V1
/// specs have zero probes by construction; the projection fills the
/// three Vecs with `vec![]`.
impl From<ServiceSpecV1> for ServiceSpecV2 {
    fn from(v1: ServiceSpecV1) -> Self {
        Self {
            id: v1.id,
            replicas: v1.replicas,
            exec: v1.exec,
            resources: v1.resources,
            listeners: v1.listeners,
            startup_probes: vec![],
            readiness_probes: vec![],
            liveness_probes: vec![],
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
            Self::V1(v1) => Ok(v1.into()),
            Self::V2(v2) => Ok(v2),
        }
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 = 0, V2 = 1 (rkyv assigns discriminants in declaration order).
        &[0, 1]
    }

    fn type_name() -> &'static str {
        "ServiceSpecEnvelope"
    }
}
