//! Intent-side aggregates — `Job`, `Node`, `Allocation`, `Policy`,
//! `Investigation`.
//!
//! Per ADR-0011, intent-side aggregates live here; observation-side row
//! shapes live in `crate::traits::observation_store`. The two never merge.
//!
//! Validating constructors return `Result<Self, AggregateError>`.
//! Step 01-01 (delivered) lands the `Job` / `Node` / `Allocation`
//! validating constructors and the `Resources`-deduplication invariant.
//! Step 01-03 (delivered) lands the canonical `IntentKey` derivation —
//! `workloads/<id>` / `nodes/<id>` / `allocations/<id>`.
//!
//! Still scaffolded (RED — owned by later steps): rkyv/serde derives on
//! the aggregate structs (Phase 2+), and behavioural expansion of
//! `Policy` and `Investigation` (Phase 2+).

use std::num::NonZeroU32;
use std::path::Path;

use rkyv::util::AlignedVec;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::codec::{EnvelopeError, VersionedEnvelope, decode_envelope_bytes};
use crate::id::{
    AllocationId, ContentHash, CorrelationKey, InvestigationId, NodeId, PolicyId, Region,
    WorkloadId,
};
use crate::traits::driver::Resources;
use crate::traits::intent_store::IntentStoreError;

// ---------------------------------------------------------------------------
// Re-exports for the workload-kind-discriminator parser surface.
//
// `WorkloadSpec` and friends ship as part of Slice 01 of
// `workload-kind-discriminator` per ADR-0047. The legacy `Job` aggregate
// and `JobSpecInput` remain in this module as the production path until
// downstream slices (02–06) migrate every reader.
// ---------------------------------------------------------------------------
pub use self::probe_descriptor::{
    JOB_PROBES_GUIDANCE, ProbeDescriptor, ProbeMechanic, SCHEDULE_PROBES_GUIDANCE,
};
pub use self::service_spec::{
    ServiceSpec, ServiceSpecEnvelope, ServiceSpecLatest, ServiceSpecV1, ServiceSpecV2,
};

// Re-export the parser-side `ExecInput` / `ResourcesInput` from
// `workload_spec` under disambiguating aliases. The wire-shape twins
// (`ExecInput` / `ResourcesInput` defined directly in this module)
// remain the canonical wire-side types; the parser-side variants are
// what `ServiceSpecV{1,2}` carry and what schema-evolution fixtures
// construct.
pub use self::workload_spec::{
    CronExpr, JobSpec, Listener, ParseError, ScheduleSpec, ServiceVip, WorkloadKind, WorkloadSpec,
    WorkloadSpecInput,
};
pub use self::workload_spec::{
    DriverInput as ParserDriverInput, ExecInput as ParserExecInput,
    ResourcesInput as ParserResourcesInput, VmInput as ParserVmInput,
};

mod workload_spec;

// `ProbeDescriptor` aggregate type per ADR-0057. Lands additively
// across slices 01 / 02 / 03 + 04 / 05 / 07 — TCP mechanic in 01-02,
// HTTP in 02-01, Exec in 02-02.
pub mod probe_descriptor;

// `ServiceSpec` parser-side aggregate + per-type rkyv envelope per
// ADR-0048 + ADR-0057. Step 01-02 lands the V1 → V2 envelope bump
// with the three `Vec<ProbeDescriptor>` fields.
mod service_spec;

// ---------------------------------------------------------------------------
// Aggregate error
// ---------------------------------------------------------------------------

/// Errors produced by aggregate validating constructors. Per
/// `development.md` typed-error discipline — variants are pass-through
/// where appropriate and locally-defined otherwise.
#[derive(Debug, Error)]
pub enum AggregateError {
    /// Scalar-field validation failure. `field` names the offending field
    /// in the aggregate's public shape; `message` is the human-readable
    /// reason. Both fire before any store write per US-03 AC.
    #[error("{field}: {message}")]
    Validation { field: &'static str, message: String },

    /// Underlying newtype parse failure — wrapped through `#[from]` per
    /// the pass-through-embedding discipline in `development.md`.
    #[error(transparent)]
    Id(#[from] crate::id::IdParseError),

    /// A resource-shape violation that couldn't be expressed as a simple
    /// field-name / message pair (e.g. cross-field constraint).
    #[error("resources: {0}")]
    Resources(String),
}

// ---------------------------------------------------------------------------
// Job aggregate
// ---------------------------------------------------------------------------

/// The intent-side Job aggregate. Carries the authoritative declaration
/// of what the operator asked the platform to run.
///
/// Per ADR-0031 Amendment 1 the aggregate carries a tagged-enum
/// `driver: WorkloadDriver` field instead of flat `command` / `args`.
/// `WorkloadDriver::Exec(Exec { command, args })` is the Phase-1
/// variant; `WorkloadDriver::Vm(Vm { .. })` is the Phase-2 microVM
/// variant (ADR-0083 Amendment 2026-08-12, GH #42). Future variants
/// (`Wasm(Wasm)`) append additively. The driver passes the inner
/// `Exec.command` / `Exec.args` to
/// `tokio::process::Command::new(impl AsRef<OsStr>).args(...)` — no
/// newtype is warranted (per `.claude/rules/development.md` § Newtypes),
/// and validation lives in `JobV2::from_submit`.
///
/// # Canonicalisation (rkyv)
///
/// Per `.claude/rules/development.md` ("Internal data → rkyv"), the
/// archived form of `Job` is THE canonical byte sequence used for
/// content-addressed identity and Raft log payloads. Two archivals of
/// the same logical `Job` MUST produce byte-identical output — the
/// acceptance proptests in `tests/acceptance/aggregate_roundtrip.rs`
/// pin this invariant.
///
/// # Wire form (serde)
///
/// serde + JSON is the wire lane for CLI-to-server and REST ingress.
/// serde is NOT substitutable for rkyv in hashing contexts — see
/// ADR-0002.
///
/// # Envelope wrapping (ADR-0050, forked V1->V2 by ADR-0083 Amendment
/// 2026-08-12)
///
/// Per ADR-0050 single-cut migration: the `Job` payload is wrapped
/// at the persistence boundary by [`WorkloadIntentEnvelope`] via
/// the [`WorkloadIntentV2::Job`] variant — NOT by a per-type
/// `JobEnvelope`. Public callers construct `Job { ... }` (=
/// `JobV2 { ... }`) values via struct-literal syntax and wrap with
/// `WorkloadIntent::Job(job)` at the persistence boundary; the
/// codec ([`WorkloadIntentV2::archive_for_store`]) is the SOLE
/// wrapping site.
pub type Job = JobV2;

/// Validated intent-side counterpart to wire-shape [`DriverInput`].
/// Forked V1 -> V2 by ADR-0083 Amendment 2026-08-12 (GH #42) — this
/// alias always points at the live/latest fork member
/// ([`WorkloadDriverV2`]); [`WorkloadDriverV1`] is the frozen sibling
/// embedded only by the historical V1 payloads.
///
/// Naming: `WorkloadDriver`, not `Driver`, to disambiguate from the
/// `Driver` *trait* at `crates/overdrive-core/src/traits/driver.rs`
/// (per ADR-0030 §1). The trait is the driver implementation surface
/// (`Driver::start(&AllocationSpec)`); this enum is the operator's
/// declared driver-class intent on the [`Job`] aggregate.
pub type WorkloadDriver = WorkloadDriverV2;

/// **FROZEN.** Embedded only by the frozen V1 payloads ([`JobV1`],
/// [`ServiceV1`], [`ScheduleV1`]) — byte-identical to the
/// pre-ADR-0083 single-variant `WorkloadDriver`. Never touched again:
/// growing this enum would shift the archived layout of every V1
/// payload and break the pinned `FIXTURE_V1_*` golden-bytes fixtures
/// in `tests/schema_evolution/workload_intent.rs`. New driver classes
/// append to [`WorkloadDriverV2`] instead (ADR-0083 Amendment
/// 2026-08-12, GH #42).
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
pub enum WorkloadDriverV1 {
    /// Native binary under cgroups v2. Mirrors wire-shape
    /// [`DriverInput::Exec`].
    Exec(Exec),
}

/// **LIVE.** Embedded by the live V2 payloads ([`JobV2`],
/// [`ServiceV2`], [`ScheduleV2`]) — the [`WorkloadDriver`] alias
/// points here. Adds the [`Vm`] microVM variant (ADR-0083 Amendment
/// 2026-08-12, GH #42) alongside the original [`Exec`] variant.
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
pub enum WorkloadDriverV2 {
    /// Native binary under cgroups v2. Mirrors wire-shape
    /// [`DriverInput::Exec`].
    Exec(Exec),
    /// Cloud Hypervisor microVM driver per ADR-0082 / ADR-0083. The
    /// wire-side parser/admission surface (`[vm]` table dispatch,
    /// `DriverPayload::Vm`, `AllocationSpec.driver`) lands in step
    /// 01-08 — this variant carries only the intent-side invocation
    /// fields.
    Vm(Vm),
    // Future Phase 2+: Wasm(Wasm).
}

/// Exec-driver invocation fields. Mirrors wire-shape [`ExecInput`] on
/// the intent side. Shared, byte-identical, by both
/// [`WorkloadDriverV1::Exec`] and [`WorkloadDriverV2::Exec`].
///
/// Naming: bare `Exec`, not `ExecSpec` / `ExecInvocation` — the
/// `WorkloadDriver::Exec(Exec)` qualified path disambiguates from the
/// `[exec]` TOML table identifier and from the `ExecDriver` trait impl
/// in `overdrive-worker`. The bare noun reads cleanest in context.
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
pub struct Exec {
    /// Host filesystem path to the binary the driver execs. Per ADR-0031
    /// this is mandatory and validated non-empty (after trim) at
    /// `JobV2::from_submit`.
    pub command: String,
    /// Argv passed verbatim to the binary. No per-element validation —
    /// argv is opaque to the platform per ADR-0031 §4.
    pub args: Vec<String>,
}

/// microVM-driver invocation fields (ADR-0083 Amendment 2026-08-12,
/// GH #42). Mirrors the runtime `VmPayload` shape (ADR-0083 § D3) —
/// `String`, not `PathBuf`, for both `kernel` and `rootfs`
/// (rkyv/serde-clean, matches `Exec.command`'s shape). Per-VM volumes
/// are out of scope for this feature (deferred to overdrive-fs, GH #97
/// / virtiofsd, GH #43).
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
pub struct Vm {
    /// Command run INSIDE the guest.
    pub command: String,
    /// Argv passed verbatim to the in-guest command.
    pub args: Vec<String>,
    /// Operator-supplied kernel artifact path (BYO — host filesystem
    /// path to a kernel image the guest boots).
    pub kernel: String,
    /// Operator-supplied rootfs artifact path (BYO — host filesystem
    /// path to a root filesystem image).
    pub rootfs: String,
}

// ---------------------------------------------------------------------------
// Job inner payload (envelope relocated to WorkloadIntent per ADR-0050;
// forked V1 -> V2 by ADR-0083 Amendment 2026-08-12, GH #42)
// ---------------------------------------------------------------------------
//
// Per ADR-0050 single-cut migration: the persistence-boundary
// envelope on `Job` (`JobEnvelope`, `JobLatest`,
// `Job::archive_for_store`, `Job::from_store_bytes`,
// `Job::spec_digest`) was deleted in this commit. The `Job` payload
// is now persisted as the inner variant of
// [`WorkloadIntentV2::Job`]; the codec lives on
// [`WorkloadIntentV2`].
//
// Per ADR-0083 Amendment 2026-08-12: `JobV1` is now the FROZEN
// payload embedded only by `WorkloadIntentV1` (byte-identical to the
// pre-fork shape — its `driver` field re-points to the single-variant
// `WorkloadDriverV1`, so the archived layout is unchanged and
// `FIXTURE_V1_*` still decode). `JobV2` carries the validating
// constructor and is what the `Job` alias (= `JobV2`) resolves to —
// every CLI handler and every server handler routes through
// `JobV2::from_submit`. Callers wrap the value via
// `WorkloadIntent::Job(job)` at the persistence boundary.

/// **FROZEN.** Inner V1 payload of the intent-side workload
/// aggregate. Embedded only by [`WorkloadIntentV1::Job`] — exists
/// solely so [`WorkloadIntentEnvelope::V1`] can decode the pinned
/// `FIXTURE_V1_*` golden bytes in
/// `tests/schema_evolution/workload_intent.rs`. Never constructed at
/// runtime; never touched again (rkyv archives are fixed positional
/// layouts — see [`JobV2`] for the live, behaviour-carrying
/// counterpart).
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
pub struct JobV1 {
    pub id: WorkloadId,
    pub replicas: NonZeroU32,
    pub resources: Resources,
    pub driver: WorkloadDriverV1,
}

/// **LIVE.** Inner V2 payload of the [`Job`] aggregate — the [`Job`]
/// alias points here. Identical shape to [`JobV1`] except `driver` is
/// the Vm-capable [`WorkloadDriverV2`] (ADR-0083 Amendment
/// 2026-08-12, GH #42).
///
/// rkyv archives are **fixed positional layouts** — appending a
/// field to this struct shifts every subsequent offset and renders
/// previously-archived bytes unreadable. Layout-changing edits
/// require minting a new outer envelope variant per
/// `.claude/rules/development.md` § "Version-bump procedure". The
/// envelope today is [`WorkloadIntentEnvelope`] (per ADR-0050, forked
/// V1 -> V2 by ADR-0083 Amendment 2026-08-12).
///
/// Per ADR-0031 Amendment 1, `driver` is a tagged enum
/// (`WorkloadDriver`) carrying the operator's invocation shape;
/// the projection from wire-shape `DriverInput::Exec` →
/// `WorkloadDriver::Exec` happens inside
/// [`JobV2::from_submit`](JobV2::from_submit).
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
pub struct JobV2 {
    pub id: WorkloadId,
    pub replicas: NonZeroU32,
    pub resources: Resources,
    /// Driver-class declaration carrying the operator's invocation
    /// shape. Per ADR-0031 Amendment 1 this is a tagged enum
    /// mirroring the wire-shape `DriverInput`.
    pub driver: WorkloadDriverV2,
}

impl JobV2 {
    /// Validating constructor for the wire-side
    /// [`crate::api::submit::SubmitSpecInput::Job`] payload per
    /// ADR-0051 § 4 / OQ-6. Renames the legacy `from_spec` entry point
    /// (which dated from the era when the wire and parser shapes were
    /// conflated). Per US-01 AC, this is the single path into the
    /// intent-side `Job` aggregate; every CLI handler and every server
    /// handler routes through here.
    ///
    /// Rejects zero replicas, zero-byte memory capacity, and (per
    /// ADR-0031 §4) empty / whitespace-only `exec.command`. Wraps
    /// [`WorkloadId`]'s `FromStr` error through `AggregateError::Id(..)` via
    /// `#[from]`.
    pub fn from_submit(spec: JobSpecInput) -> Result<Self, AggregateError> {
        let JobSpecInput { id, replicas, resources, driver } = spec;
        let id = WorkloadId::new(&id)?;
        let replicas = NonZeroU32::new(replicas).ok_or_else(|| AggregateError::Validation {
            field: "replicas",
            message: format!("replica count must be non-zero; got {replicas}"),
        })?;
        if resources.memory_bytes == 0 {
            return Err(AggregateError::Validation {
                field: "memory_bytes",
                message: "memory capacity must be non-zero".to_string(),
            });
        }
        // Project the wire-shape `DriverInput` into the intent-shape
        // `WorkloadDriver` per ADR-0031 Amendment 1, applying the
        // ADR-0031 §4 non-empty-after-trim rule on the way. The trim
        // predicate covers `""`, `"   "`, `"\t\n\r"`, and mixed Unicode
        // whitespace via `str::trim` (Unicode whitespace class). NO
        // NUL-byte rejection (kernel `execve(2)` handles); NO length
        // cap (kernel `PATH_MAX` handles); NO per-element `args` rule
        // — argv is opaque to the platform per ADR-0031 §4. Casing is
        // preserved verbatim — the validator is a predicate, not a
        // normaliser.
        //
        // ADR-0083 (GH #42, step 01-08): the `Vm` arm applies the
        // identical non-empty-after-trim rule to the in-guest command —
        // `kernel` / `rootfs` existence is a runtime (Vmm::create-time)
        // concern, not a parse-time one, mirroring how `Exec.command`'s
        // referenced binary existence is never checked here either.
        let driver = match driver {
            DriverInput::Exec(exec_input) => {
                if exec_input.command.trim().is_empty() {
                    return Err(AggregateError::Validation {
                        field: "exec.command",
                        message: "command must be non-empty".to_string(),
                    });
                }
                WorkloadDriver::Exec(Exec { command: exec_input.command, args: exec_input.args })
            }
            DriverInput::Vm(vm_input) => {
                if vm_input.command.trim().is_empty() {
                    return Err(AggregateError::Validation {
                        field: "vm.command",
                        message: "command must be non-empty".to_string(),
                    });
                }
                WorkloadDriver::Vm(Vm {
                    command: vm_input.command,
                    args: vm_input.args,
                    kernel: vm_input.kernel,
                    rootfs: vm_input.rootfs,
                })
            }
        };
        Ok(Self {
            id,
            replicas,
            resources: Resources {
                cpu_milli: resources.cpu_milli,
                memory_bytes: resources.memory_bytes,
            },
            driver,
        })
    }
}

// ---------------------------------------------------------------------------
// `WorkloadIntent` — kind-agnostic intent-side workload aggregate (ADR-0050)
// ---------------------------------------------------------------------------
//
// Per ADR-0050 (Accepted 2026-05-14): the intent-side aggregate is a
// kind-discriminated outer enum (`WorkloadIntent::Job | Service |
// Schedule`), distinct from the parser-side `WorkloadSpec`. Pattern C
// (parsed-on-ingress, typed-on-disk) — the two type families evolve
// independently. The persistence-boundary codec lives on
// `WorkloadIntent` (per ADR-0048 § 4b — typed codec on the value);
// the `IntentStore` trait stays generic byte-level.
//
// Per OQ-5 (single-cut), every workload-scoped row sits at
// `workloads/<id>` — see `IntentKey::for_workload*`.
//
// Per ADR-0083 Amendment 2026-08-12 (GH #42): forked V1 -> V2 to add
// `WorkloadDriverV2::Vm` without breaking the frozen V1 archived
// layout. `WorkloadIntentV1` (and its embedded `JobV1` / `ServiceV1`
// / `ScheduleV1`) is FROZEN — byte-identical to the pre-fork shape,
// existing solely so `WorkloadIntentEnvelope::V1` can decode the
// pinned `FIXTURE_V1_*` golden bytes. `WorkloadIntentV2` (and its
// embedded `JobV2` / `ServiceV2` / `ScheduleV2`) is LIVE — every
// public alias (`WorkloadIntent`, `Job`, `Service`, `Schedule`,
// `WorkloadDriver`) points here.

/// Public payload alias for the intent-side workload aggregate.
///
/// Per ADR-0050 the alias points at the latest payload variant —
/// per ADR-0083 Amendment 2026-08-12, today `WorkloadIntentV2`.
/// Callers construct values via `WorkloadIntent::Job(job)` /
/// `WorkloadIntent::Service(svc)` / `WorkloadIntent::Schedule(sched)`
/// and pass the value to the persistence-boundary codec
/// ([`WorkloadIntentV2::archive_for_store`]).
pub type WorkloadIntent = WorkloadIntentV2;

/// Documentation alias for "the latest payload variant of
/// [`WorkloadIntentEnvelope`]". Mirrors the [`Job`] = [`JobV2`]
/// alias-to-payload pattern from ADR-0048 UI-02.
pub type WorkloadIntentLatest = WorkloadIntentV2;

/// Per-type rkyv versioned envelope for the intent-side workload
/// aggregate per ADR-0048 § 4 + ADR-0050 § 4, forked V1 -> V2 by
/// ADR-0083 Amendment 2026-08-12 (GH #42, `WorkloadDriverV2::Vm`).
///
/// Codec-internal — named only inside the typed
/// [`WorkloadIntentV2::archive_for_store`] / [`WorkloadIntentV2::from_store_bytes`]
/// codec methods and the persistence-boundary call sites that consume
/// them. Public callers use the [`WorkloadIntent`] alias and
/// construct payloads via the per-variant struct-literal syntax;
/// the persistence boundary wraps via
/// [`WorkloadIntentEnvelope::latest`].
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
pub enum WorkloadIntentEnvelope {
    V1(WorkloadIntentV1),
    V2(WorkloadIntentV2),
}

/// **FROZEN.** Inner V1 payload of the intent-side workload aggregate
/// per ADR-0050 § 1. Exists solely so [`WorkloadIntentEnvelope::V1`]
/// can decode the pinned `FIXTURE_V1_*` golden bytes in
/// `tests/schema_evolution/workload_intent.rs`. Never constructed at
/// runtime; never touched again. See [`WorkloadIntentV2`] for the
/// live, behaviour-carrying counterpart.
///
/// rkyv archives are **fixed positional layouts** — this enum's
/// variant SET is frozen; new workload kinds append to
/// [`WorkloadIntentV2`] instead.
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
pub enum WorkloadIntentV1 {
    /// Run-to-completion workload.
    Job(JobV1),
    /// Long-running supervised workload — Phase 1 minimal shape per
    /// ADR-0050 OQ-3.
    Service(ServiceV1),
    /// Cron-scheduled Job — embedded-job shape per ADR-0050 OQ-4.
    Schedule(ScheduleV1),
}

/// **LIVE.** Inner V2 payload of the intent-side workload aggregate.
/// The [`WorkloadIntent`] alias points here. Three variants tracking
/// the parser-side [`WorkloadSpec`]: `Job` (run-to-completion),
/// `Service` (long-running supervised), `Schedule` (cron-fired Job) —
/// identical shape to [`WorkloadIntentV1`] except each inner payload
/// carries the Vm-capable [`WorkloadDriverV2`] (ADR-0083 Amendment
/// 2026-08-12, GH #42).
///
/// rkyv archives are **fixed positional layouts** — appending a
/// variant to this enum is additive and does not shift discriminant
/// tags for existing variants per ADR-0048 § "Why a per-type rkyv
/// enum is forward-compatible". Layout-changing edits to embedded
/// per-kind payloads (e.g. adding a field to [`ServiceV2`]) require
/// minting a new envelope variant per `.claude/rules/development.md`
/// § "Version-bump procedure".
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
pub enum WorkloadIntentV2 {
    /// Run-to-completion workload.
    Job(JobV2),
    /// Long-running supervised workload — Phase 1 minimal shape per
    /// ADR-0050 OQ-3.
    Service(ServiceV2),
    /// Cron-scheduled Job — embedded-job shape per ADR-0050 OQ-4.
    Schedule(ScheduleV2),
}

/// **FROZEN.** Phase 1 minimal `Service` V1 payload per ADR-0050 § 2 +
/// OQ-3, extended with health-check probe descriptors per ADR-0057.
/// Embedded only by [`WorkloadIntentV1::Service`] — exists solely so
/// [`WorkloadIntentEnvelope::V1`] can decode the pinned
/// `FIXTURE_V1_SERVICE` golden bytes. Never constructed at runtime;
/// never touched again. See [`ServiceV2`] for the live,
/// behaviour-carrying counterpart.
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
pub struct ServiceV1 {
    pub id: WorkloadId,
    pub replicas: NonZeroU32,
    pub resources: Resources,
    pub driver: WorkloadDriverV1,
    pub listeners: Vec<Listener>,
    pub startup_probes: Vec<ProbeDescriptor>,
    pub readiness_probes: Vec<ProbeDescriptor>,
    pub liveness_probes: Vec<ProbeDescriptor>,
}

/// Public payload alias for the intent-side `Service` aggregate.
/// Minted by ADR-0083 Amendment 2026-08-12 (GH #42) — points at the
/// live/latest payload variant, [`ServiceV2`], mirroring the
/// [`Job`] = [`JobV2`] alias-to-payload pattern from ADR-0048 UI-02.
pub type Service = ServiceV2;

/// **LIVE.** Phase 1 minimal `Service` payload per ADR-0050 § 2 +
/// OQ-3, extended with health-check probe descriptors per ADR-0057
/// and the Vm-capable driver per ADR-0083 Amendment 2026-08-12
/// (GH #42). The [`Service`] alias points here.
///
/// Mirrors [`JobV2`]'s `(id, replicas, resources, driver)` shape and
/// adds `listeners` plus three `Vec<ProbeDescriptor>` slots (startup
/// / readiness / liveness). The probe vecs carry the parsed-and-
/// validated descriptors the operator declared under
/// `[[health_check.startup]]` / `[[health_check.readiness]]` /
/// `[[health_check.liveness]]` (plus any platform-synthesised
/// default-TCP probe per ADR-0058). Persisting the descriptors
/// themselves is correct per § "Persist inputs, not derived state":
/// the reconciler recomputes derived values (startup deadline,
/// inferred-flag rendering, mechanic summary) every tick from the
/// descriptors + the live policy, never persisting them as cached
/// outputs.
///
/// # rkyv schema-evolution note
///
/// Per `.claude/rules/development.md` § "rkyv schema evolution" the
/// archived layout of this struct is positional. Under the Phase-1
/// greenfield single-cut migration policy ("delete the on-disk redb
/// file" is the official upgrade path, per
/// `feedback_single_cut_greenfield_migrations.md`), the historical
/// golden-bytes fixtures under `tests/schema_evolution/workload_intent.rs`
/// are the structural defense — every persisted layout has a pinned
/// golden-bytes fixture, and this struct's own layout change (the
/// `driver` field widening to [`WorkloadDriverV2`]) is exactly what
/// forced the ADR-0083 Amendment 2026-08-12 V1 -> V2 envelope bump.
///
/// Carries no VIP — VIPs are platform-issued via
/// `ServiceVipAllocator` per ADR-0049 § 5. The aggregate carries
/// what the operator declared; the allocated VIP lives in the
/// allocator's persisted state and is projected onto listener rows
/// at dataplane-render time.
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
pub struct ServiceV2 {
    pub id: WorkloadId,
    pub replicas: NonZeroU32,
    pub resources: Resources,
    pub driver: WorkloadDriverV2,
    /// Operator-declared listeners in declaration order. Reuses the
    /// parser-layer [`Listener`] newtype — `(port, protocol)` only.
    pub listeners: Vec<Listener>,
    /// Operator-declared startup probes plus any platform-synthesised
    /// default per ADR-0058. Empty IFF the operator wrote
    /// `[[health_check.startup]] = []` (explicit opt-out, preserves
    /// Phase-1 first-Running semantics). The reconciler reads these
    /// descriptors and recomputes `max_attempts × interval` →
    /// startup-deadline on every tick (NOT a cached value).
    pub startup_probes: Vec<ProbeDescriptor>,
    /// Operator-declared readiness probes. Populated by future
    /// slices (02-01); reserved here for ServiceV2 layout stability.
    pub readiness_probes: Vec<ProbeDescriptor>,
    /// Operator-declared liveness probes. Populated by future
    /// slices (02-02); reserved here for ServiceV2 layout stability.
    pub liveness_probes: Vec<ProbeDescriptor>,
}

/// **FROZEN.** Phase 1 `Schedule` V1 payload per ADR-0050 § 2 + OQ-4
/// (embedded inner job). Embedded only by
/// [`WorkloadIntentV1::Schedule`] — exists solely so
/// [`WorkloadIntentEnvelope::V1`] can decode the pinned
/// `FIXTURE_V1_SCHEDULE` golden bytes. Never constructed at runtime;
/// never touched again. See [`ScheduleV2`] for the live,
/// behaviour-carrying counterpart.
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
pub struct ScheduleV1 {
    pub id: WorkloadId,
    pub job: JobV1,
    pub cron_expr: CronExpr,
}

/// Public payload alias for the intent-side `Schedule` aggregate.
/// Minted by ADR-0083 Amendment 2026-08-12 (GH #42) — points at the
/// live/latest payload variant, [`ScheduleV2`], mirroring the
/// [`Job`] = [`JobV2`] alias-to-payload pattern from ADR-0048 UI-02.
pub type Schedule = ScheduleV2;

/// **LIVE.** Phase 1 `Schedule` payload per ADR-0050 § 2 + OQ-4
/// (embedded inner job), extended with the Vm-capable driver per
/// ADR-0083 Amendment 2026-08-12 (GH #42, via the embedded
/// [`JobV2`]). The [`Schedule`] alias points here.
///
/// The schedule's per-fire instance IS a [`JobV2`] — embedded
/// directly rather than carried as deferred bytes (alternative
/// rejected per OQ-4 — every reader would otherwise pay a second
/// envelope decode). The cron expression is the schedule-only
/// addition.
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
pub struct ScheduleV2 {
    pub id: WorkloadId,
    pub job: JobV2,
    pub cron_expr: CronExpr,
}

// ---------------------------------------------------------------------------
// V1 -> V2 structural conversion (ADR-0083 Amendment 2026-08-12, GH #42)
// ---------------------------------------------------------------------------
//
// Every conversion is a structural field-by-field projection; the
// only semantic step is `WorkloadDriverV1::Exec ->
// WorkloadDriverV2::Exec` (the sole V1 variant maps onto its V2
// sibling of the same name).

impl From<WorkloadDriverV1> for WorkloadDriverV2 {
    fn from(v1: WorkloadDriverV1) -> Self {
        match v1 {
            WorkloadDriverV1::Exec(exec) => Self::Exec(exec),
        }
    }
}

impl From<JobV1> for JobV2 {
    fn from(v1: JobV1) -> Self {
        let JobV1 { id, replicas, resources, driver } = v1;
        Self { id, replicas, resources, driver: driver.into() }
    }
}

impl From<ServiceV1> for ServiceV2 {
    fn from(v1: ServiceV1) -> Self {
        let ServiceV1 {
            id,
            replicas,
            resources,
            driver,
            listeners,
            startup_probes,
            readiness_probes,
            liveness_probes,
        } = v1;
        Self {
            id,
            replicas,
            resources,
            driver: driver.into(),
            listeners,
            startup_probes,
            readiness_probes,
            liveness_probes,
        }
    }
}

impl From<ScheduleV1> for ScheduleV2 {
    fn from(v1: ScheduleV1) -> Self {
        let ScheduleV1 { id, job, cron_expr } = v1;
        Self { id, job: job.into(), cron_expr }
    }
}

impl From<WorkloadIntentV1> for WorkloadIntentV2 {
    fn from(v1: WorkloadIntentV1) -> Self {
        match v1 {
            WorkloadIntentV1::Job(job) => Self::Job(job.into()),
            WorkloadIntentV1::Service(svc) => Self::Service(svc.into()),
            WorkloadIntentV1::Schedule(sched) => Self::Schedule(sched.into()),
        }
    }
}

impl ServiceV2 {
    /// Validating constructor for the wire-side
    /// [`crate::api::submit::SubmitSpecInput::Service`] payload per
    /// ADR-0051 § 4. Mirrors [`JobV2::from_submit`]'s validation
    /// surface plus Service-specific listener rules:
    ///
    /// * `id` non-empty after trim → [`WorkloadId::new`].
    /// * `replicas > 0` → [`NonZeroU32`].
    /// * `resources.memory_bytes != 0`.
    /// * Driver validation (currently `exec.command` non-empty after
    ///   trim, per ADR-0031 § 4).
    /// * `listeners.len() >= 1`
    ///   ([`crate::aggregate::ParseError::ListenerMissing`] projected
    ///   onto [`AggregateError::Validation`]).
    /// * No two listeners share `(port, protocol)`.
    /// * `port != 0` per listener.
    /// * `protocol` parses to `Proto` (case-insensitive `tcp` / `udp`).
    pub fn from_submit(
        input: crate::api::submit::ServiceSpecInput,
    ) -> Result<Self, AggregateError> {
        use std::collections::BTreeSet;
        use std::num::NonZeroU16;

        use crate::dataplane::backend_key::Proto;

        let crate::api::submit::ServiceSpecInput {
            id,
            replicas,
            resources,
            driver,
            listeners,
            startup_probes,
            readiness_probes,
            liveness_probes,
        } = input;

        // Identity + scalar field validation — mirrors `JobV2::from_submit`.
        let id = WorkloadId::new(&id)?;
        let replicas = NonZeroU32::new(replicas).ok_or_else(|| AggregateError::Validation {
            field: "replicas",
            message: format!("replica count must be non-zero; got {replicas}"),
        })?;
        if resources.memory_bytes == 0 {
            return Err(AggregateError::Validation {
                field: "memory_bytes",
                message: "memory capacity must be non-zero".to_string(),
            });
        }

        // Driver projection — same shape as `JobV2::from_submit`.
        //
        // ADR-0083 §D4 (GH #42): `[vm]` + `[service]` is rejected — a
        // microVM terminates TCP inside the guest (GH #222), so it is not
        // mesh-enrolled and cannot back a Service. The `workload_spec.rs`
        // TOML parser already keeps `[service]` `[exec]`-only (never
        // constructs `DriverInput::Vm` for a Service body), but
        // `ServiceSpecInput` is also the direct wire-ingress shape
        // (ADR-0015 defence-in-depth) — an API client posting
        // `driver: {"vm": ...}` bypasses that gate, so this arm rejects it
        // explicitly rather than silently constructing an invalid state.
        // The fully-named guest-networking/probes/mTLS rejection message
        // (citing GH #257 / #222) is a later slice's AC-10 — this is the
        // safe, minimal rejection for today.
        let DriverInput::Exec(exec_input) = driver else {
            return Err(AggregateError::Validation {
                field: "driver",
                message: "[vm] is not supported for Service-kind workloads (guest networking \
                          is not mesh-enrolled, GH #222) — use [exec], or deploy Job/Schedule \
                          instead"
                    .to_string(),
            });
        };
        if exec_input.command.trim().is_empty() {
            return Err(AggregateError::Validation {
                field: "exec.command",
                message: "command must be non-empty".to_string(),
            });
        }

        // Listener validation.
        if listeners.is_empty() {
            return Err(AggregateError::Validation {
                field: "listeners",
                message: "a service requires at least one listener".to_string(),
            });
        }
        let mut seen: BTreeSet<(u16, &'static str)> = BTreeSet::new();
        let mut validated: Vec<Listener> = Vec::with_capacity(listeners.len());
        for listener in listeners {
            let port =
                NonZeroU16::new(listener.port).ok_or_else(|| AggregateError::Validation {
                    field: "listeners[].port",
                    message: "listener port must be in 1..=65535".to_string(),
                })?;
            let protocol = match listener.protocol.to_ascii_lowercase().as_str() {
                "tcp" => Proto::Tcp,
                "udp" => Proto::Udp,
                other => {
                    return Err(AggregateError::Validation {
                        field: "listeners[].protocol",
                        message: format!(
                            "unsupported listener protocol {other:?} (supported protocols: tcp, udp)"
                        ),
                    });
                }
            };
            let key = (port.get(), protocol.as_str());
            if !seen.insert(key) {
                return Err(AggregateError::Validation {
                    field: "listeners",
                    message: format!(
                        "duplicate listener (port={}, protocol={})",
                        port.get(),
                        protocol.as_str()
                    ),
                });
            }
            validated.push(Listener { port, protocol });
        }

        validate_probe_mechanics(&startup_probes, "startup_probes")?;
        validate_probe_mechanics(&readiness_probes, "readiness_probes")?;
        validate_probe_mechanics(&liveness_probes, "liveness_probes")?;

        // ADR-0080 § D1 — this is the SECOND ingress (the API/wire
        // path; the TOML parser is the first). `ProbeDescriptor.idx`
        // is parser-assigned by contract, so a caller-supplied value
        // is NOT trusted here: it is re-assigned from each vector's
        // own 0-based position. Without this, a wire client could
        // submit two descriptors of one role carrying the same `idx`
        // and collide their durable probe-result rows under the
        // `(alloc_id, role, probe_idx)` key (§ D2), silently losing
        // one probe's observations to LWW.
        let startup_probes = reindex_probes_by_position(startup_probes);
        let readiness_probes = reindex_probes_by_position(readiness_probes);
        let liveness_probes = reindex_probes_by_position(liveness_probes);

        Ok(Self {
            id,
            replicas,
            resources: Resources {
                cpu_milli: resources.cpu_milli,
                memory_bytes: resources.memory_bytes,
            },
            driver: WorkloadDriver::Exec(Exec {
                command: exec_input.command,
                args: exec_input.args,
            }),
            listeners: validated,
            startup_probes,
            readiness_probes,
            liveness_probes,
        })
    }
}

/// Re-assign every descriptor's `idx` from its 0-based position in
/// its own role vector, per ADR-0080 § D1.
///
/// The TOML parser assigns `idx` at `ServiceSpecV2` construction and
/// the projection carries it verbatim, so for the CLI path this is an
/// identity transform. For the API/wire path it is the enforcement
/// point: `ProbeDescriptor.idx` is parser-assigned by contract, and a
/// caller-supplied duplicate `(role, idx)` pair would collide two
/// probes' durable rows under the composite key.
fn reindex_probes_by_position(probes: Vec<ProbeDescriptor>) -> Vec<ProbeDescriptor> {
    probes
        .into_iter()
        .enumerate()
        .map(|(position, mut probe)| {
            probe.idx =
                crate::observation::ProbeIdx::new(u32::try_from(position).unwrap_or(u32::MAX));
            probe
        })
        .collect()
}

/// Validate probe mechanic content at the API admission boundary.
///
/// The TOML parser validates at parse time in `parse_http_mechanic` /
/// `parse_exec_mechanic`; the API path deserialises `ProbeDescriptor`
/// from JSON and must validate here. Both paths converge on
/// `ProbeMechanic::validate()`.
fn validate_probe_mechanics(
    probes: &[ProbeDescriptor],
    field: &'static str,
) -> Result<(), AggregateError> {
    for (idx, probe) in probes.iter().enumerate() {
        probe.mechanic.validate().map_err(|message| AggregateError::Validation {
            field,
            message: format!("[{idx}]: {message}"),
        })?;
    }
    Ok(())
}

impl ScheduleV2 {
    /// Validating constructor for the wire-side
    /// [`crate::api::submit::SubmitSpecInput::Schedule`] payload per
    /// ADR-0051 § 4 / OQ-5.
    ///
    /// RED scaffold per `.claude/rules/testing.md` § "Production-side
    /// scaffolds": Schedule wire-arm submission is intentionally
    /// deferred. The submit handler returns a structured rejection on
    /// `SubmitSpecInput::Schedule(_)` so this body is unreachable from
    /// any existing test. Lands GREEN in a future slice when the
    /// Schedule streaming endpoint ships.
    #[expect(
        clippy::todo,
        reason = "RED scaffold for ScheduleV2::from_submit — lands in a future slice per ADR-0051 OQ-5"
    )]
    pub fn from_submit(
        _input: crate::api::submit::ScheduleSpecInput,
    ) -> Result<Self, AggregateError> {
        todo!(
            "RED scaffold: ScheduleV2::from_submit lands in a future slice — Schedule wire-arm wiring is intentionally deferred per ADR-0051 OQ-5"
        )
    }
}

impl VersionedEnvelope for WorkloadIntentEnvelope {
    type Latest = WorkloadIntentV2;

    fn latest(payload: Self::Latest) -> Self {
        Self::V2(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            // ADR-0083 Amendment 2026-08-12 (GH #42): V1 up-converts
            // through the structural `From<WorkloadIntentV1> for
            // WorkloadIntentV2` impl.
            Self::V1(v1) => Ok(v1.into()),
            Self::V2(v2) => Ok(v2),
        }
    }

    // mutants: skip — `discriminant_offset_from_end` is intentionally
    // `None`. Per ADR-0050 step 02-03a the empirical re-pin was
    // originally deferred "until V2 lands"; V2 landed at the
    // ADR-0083 Amendment 2026-08-12 fork (GH #42,
    // `WorkloadDriverV2::Vm`) and the re-pin was explicitly WAIVED
    // for that landing, NOT silently skipped — the outer envelope
    // still wraps a 3-variant inner enum per driver kind, so the
    // JobEnvelope-style 64-byte from-end pin does not trivially
    // transfer, and re-deriving the offset empirically was judged
    // not worth the investment for this fork (the golden-bytes
    // fixtures below are the load-bearing defense either way — see
    // the body comment). The pre-decode probe is structurally a
    // no-op while this returns `None`, so `Some(0)` and `None`
    // produce indistinguishable behaviour — there is no test that
    // can distinguish them.
    //
    // COUPLING: this method and `known_discriminants()` immediately
    // below MUST move together. A future re-pin MUST (1) set this to
    // `Some(N)`, (2) confirm `known_discriminants()` already lists
    // every live tag (today `&[0, 1]`), and (3) add a golden-bytes
    // test that fires on the newly-`Some` offset — do not re-pin one
    // without the other.
    fn discriminant_offset_from_end() -> Option<usize> {
        // Empirically-pinned offset remains DEFERRED for
        // `WorkloadIntentEnvelope` — waived (not merely "not yet
        // reached") at both the original ADR-0050 step 02-03a
        // landing and the ADR-0083 Amendment 2026-08-12 V1->V2 fork
        // (GH #42). The outer envelope wraps a 3-variant inner enum
        // (`WorkloadIntentV1::{Job, Service, Schedule}` /
        // `WorkloadIntentV2::{Job, Service, Schedule}`) whose
        // archived layout shifts the trailing root region in ways
        // that the JobEnvelope-style 64-byte from-end pin cannot
        // trivially adopt. Returning `None` makes the pre-decode
        // probe a no-op; unknown-future-variant bytes still surface
        // as `EnvelopeError::Malformed` via rkyv's bytecheck
        // (operator-facing remediation is the same: "delete the
        // redb file"). The structural defense against future-binary
        // surface IS preserved by the round-trip golden-bytes
        // fixtures for BOTH `V1` (`FIXTURE_V1_JOB` / `_SERVICE` /
        // `_SCHEDULE`) and `V2` (`FIXTURE_V2_JOB_VM`) in
        // `tests/schema_evolution/workload_intent.rs`; the targeted
        // `UnknownVersion` classification is the only diagnostic
        // surface that degrades. Re-pin when the empirical offset
        // becomes worth investing in.
        None
    }

    // mutants: skip — `known_discriminants` is unused when
    // `discriminant_offset_from_end` returns `None` (see the
    // COUPLING note on that method above — the two are pinned
    // together and must be re-derived together). Mutations that
    // replace the `&[0, 1]` slice with `Vec::leak(vec![])` /
    // `Vec::leak(vec![0])` / `Vec::leak(vec![1])` produce no
    // observable behaviour change while the offset probe is a
    // no-op.
    fn known_discriminants() -> &'static [u8] {
        // V1 carries rkyv discriminant 0, V2 carries 1 (ADR-0083
        // Amendment 2026-08-12, GH #42). Kept accurate even though
        // `discriminant_offset_from_end` returning `None` makes the
        // probe skip this slice today — a future re-pin reads this
        // value as-is rather than also needing to backfill it.
        &[0, 1]
    }

    // mutants: skip — `type_name` feeds only the `EnvelopeError`
    // `Display` form for operator diagnostics. The string content
    // is not load-bearing for any branch (no caller pattern-matches
    // on it), so mutations to `""` / `"xyzzy"` are observationally
    // equivalent to "WorkloadIntentEnvelope". Operator-visible
    // diagnostic regression would be caught at code review of any
    // future error-message golden-string assertion, not by mutation
    // testing.
    fn type_name() -> &'static str {
        "WorkloadIntentEnvelope"
    }
}

impl WorkloadIntentV2 {
    /// Archive a [`WorkloadIntent`] for persistence through the
    /// [`IntentStore`].
    ///
    /// # Postconditions
    ///
    /// On `Ok(bytes)`, `bytes` is the canonical rkyv-archived byte
    /// sequence of `WorkloadIntentEnvelope::V2(self.clone())`. Two
    /// archivals of the same logical [`WorkloadIntent`] produce
    /// byte-identical output. Callers pass `bytes.as_ref()` to the
    /// `IntentStore` trait's `&[u8]` write surface.
    ///
    /// # Observable invariants
    ///
    /// `WorkloadIntent::from_store_bytes(&self.archive_for_store()?, p, None)`
    /// returns `Ok(self_owned)` bit-equivalent to `self` for any
    /// redb path `p`.
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeError::Malformed`] when the rkyv serialiser
    /// fails (unreachable for valid payloads).
    pub fn archive_for_store(&self) -> Result<AlignedVec, EnvelopeError> {
        let envelope = WorkloadIntentEnvelope::latest(self.clone());
        rkyv::to_bytes::<rkyv::rancor::Error>(&envelope)
            .map_err(|source| EnvelopeError::Malformed { source })
    }

    /// Decode persisted bytes back into a [`WorkloadIntent`].
    ///
    /// # Edge cases
    ///
    /// * Empty `bytes` → [`EnvelopeError::Malformed`].
    /// * Future-binary `V<N+1>` bytes → [`EnvelopeError::UnknownVersion`].
    /// * Truncated / corrupt bytes → [`EnvelopeError::Malformed`].
    ///
    /// # Observable invariants
    ///
    /// On `Err(...)`, exactly one `tracing::error!` event with
    /// `name: "health.startup.refused"` fires BEFORE the `Err`
    /// return — per ADR-0048 § 3 (intent fail-fast policy). The
    /// event carries the `redb_path`, the optional `key`
    /// (`"<unknown>"` when `None`), and the underlying
    /// `envelope_error` for operator diagnosis.
    pub fn from_store_bytes(
        bytes: &[u8],
        redb_path: &Path,
        key: Option<&str>,
    ) -> Result<Self, IntentStoreError> {
        match decode_envelope_bytes::<WorkloadIntentEnvelope>(bytes) {
            Ok(intent) => Ok(intent),
            Err(envelope_error) => {
                tracing::error!(
                    name: "health.startup.refused",
                    redb_path = %redb_path.display(),
                    key = key.unwrap_or("<unknown>"),
                    envelope_error = ?envelope_error,
                    "intent envelope decode failed; control-plane refusing to start",
                );
                Err(IntentStoreError::Envelope {
                    redb_path: redb_path.to_path_buf(),
                    source: envelope_error,
                })
            }
        }
    }

    /// Canonical content-addressed identity of a [`WorkloadIntent`].
    ///
    /// # Postconditions
    ///
    /// Returns SHA-256 over the rkyv-archived **raw inner payload
    /// bytes** of `self` (`rkyv::to_bytes(self)`) — NOT the
    /// envelope-wrapped bytes. Stable across envelope version bumps.
    ///
    /// Per ADR-0050: `WorkloadIntent::Job(j).spec_digest()` produces
    /// a value distinct from `j.spec_digest()` would have produced
    /// pre-migration — the bytes hashed are now the outer enum's
    /// archive (with discriminant + padding), not the bare `JobV1`.
    /// This is the operator-observable single-cut migration boundary
    /// for content-addressed identity. The `ServiceVipAllocator`
    /// memo (ADR-0049) keys by the value this method returns —
    /// remains stable across reconciler ticks because the input
    /// `WorkloadIntent::Service(_)` value is byte-stable.
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeError::Malformed`] if the rkyv serialiser
    /// fails (unreachable for valid payloads).
    pub fn spec_digest(&self) -> Result<ContentHash, EnvelopeError> {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(self)
            .map_err(|source| EnvelopeError::Malformed { source })?;
        Ok(ContentHash::of(bytes.as_ref()))
    }
}

/// Input shape for `JobV2::from_submit`. The CLI deserialises TOML into this
/// type; the server deserialises JSON into the same type; both route
/// through the same constructor.
///
/// Per ADR-0031 §2 the shape is flat top-level (`id`, `replicas`),
/// `resources: ResourcesInput`, `#[serde(flatten)] driver: DriverInput`.
/// `deny_unknown_fields` on every struct + a tagged enum enforce
/// exactly-one driver table at parse time.
///
/// Carries `Serialize` / `Deserialize` so REST handlers and the CLI can
/// reuse this type verbatim as the body / field shape for
/// `POST /v1/workloads` and `GET /v1/workloads/{id}` (ADR-0014 §Shared types).
/// Carries `utoipa::ToSchema` so the generated `OpenAPI` document
/// (ADR-0009, `cargo openapi-gen`) renders the spec shape
/// consistently across the server and CLI lanes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct JobSpecInput {
    pub id: String,
    pub replicas: u32,
    pub resources: ResourcesInput,
    #[serde(flatten)]
    pub driver: DriverInput,
}

/// Wire-shape twin of [`Resources`].
///
/// Per ADR-0031 §2 / `.claude/rules/development.md` § State-layer
/// hygiene: the rkyv-archived intent-side `Resources` is kept clean of
/// serde-only / utoipa-only concerns; this twin carries the wire-side
/// derives. The projection onto `Resources` is field-by-field inside
/// `JobV2::from_submit` (no `From` impl: the ≥3-call-sites rule isn't met,
/// and the validation rules — `memory_bytes != 0` — must fire on the
/// way through anyway).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourcesInput {
    pub cpu_milli: u32,
    pub memory_bytes: u64,
}

/// Driver dispatch on a [`JobSpecInput`].
///
/// Per ADR-0031 §2 a tagged enum with `#[serde(flatten)]` on the field
/// surfaces the table name as the discriminator in TOML / JSON: `[exec]`
/// → `DriverInput::Exec(...)`. `deny_unknown_fields` on the enum rejects
/// unknown driver tables.
///
/// Two variants as of ADR-0083 (GH #42, step 01-08): `Exec` (Phase 1) and
/// `Vm` (Cloud Hypervisor microVM). Future drivers (`wasm`) add new
/// variants additively; no shape change to surrounding code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum DriverInput {
    /// Native binary under cgroups v2 — the `[exec]` table in TOML.
    Exec(ExecInput),
    /// Cloud Hypervisor microVM — the `[vm]` table in TOML (ADR-0082 /
    /// ADR-0083, GH #42).
    Vm(VmInput),
    // Future: Wasm(WasmInput)
}

/// Operator-facing `[exec]` table fields per ADR-0031 §2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecInput {
    /// Host filesystem path to the binary. Validated non-empty (after
    /// trim) at `JobV2::from_submit` per ADR-0031 §4.
    pub command: String,
    /// Argv passed verbatim. Required field — an absent `args` is a
    /// parse error, not "default to no args" (per ADR-0031 §8). Empty
    /// `Vec` is the legitimate zero-args case.
    pub args: Vec<String>,
}

/// Operator-facing `[vm]` table fields (ADR-0082 / ADR-0083, GH #42).
/// Mirrors the runtime `traits::driver::VmPayload` shape — `String`,
/// not `PathBuf`, for both `kernel` and `rootfs` (serde-clean wire
/// shape, matches `ExecInput.command`'s shape). Per-VM volumes are out
/// of scope for this feature (deferred to overdrive-fs, GH #97 /
/// virtiofsd, GH #43).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VmInput {
    /// Command run INSIDE the guest.
    pub command: String,
    /// Argv passed verbatim to the in-guest command.
    pub args: Vec<String>,
    /// Operator-supplied kernel artifact path (BYO).
    pub kernel: String,
    /// Operator-supplied rootfs artifact path (BYO).
    pub rootfs: String,
}

/// Reverse conversion — reconstruct the wire-shape `JobSpecInput` from a
/// validated `Job` aggregate. Used by `describe_workload` (ADR-0008 §GET
/// /v1/workloads/{id}) to render the stored spec back onto the wire after
/// rkyv access + deserialize.
///
/// Non-fallible by construction: every field in `JobSpecInput` is a
/// projection of a field already validated by `JobV2::from_submit`. Cloning
/// the `id` is cheap — `WorkloadId::to_string()` is an owned ASCII string.
impl From<&Job> for JobSpecInput {
    fn from(job: &Job) -> Self {
        // Per ADR-0031 Amendment 1, project the intent-shape
        // `WorkloadDriver` back to the wire-shape `DriverInput`,
        // preserving the driver KIND rather than collapsing to a flat
        // (command, args) tuple and always rewrapping as `Exec` — that
        // would silently mis-render a Vm-driven Job as Exec-driven on
        // `GET /v1/workloads/{id}`. `DriverInput::Vm` lands here per
        // ADR-0083 Amendment 2026-08-12 (GH #42, step 01-08).
        let driver = match &job.driver {
            WorkloadDriver::Exec(exec) => DriverInput::Exec(ExecInput {
                command: exec.command.clone(),
                args: exec.args.clone(),
            }),
            WorkloadDriver::Vm(vm) => DriverInput::Vm(VmInput {
                command: vm.command.clone(),
                args: vm.args.clone(),
                kernel: vm.kernel.clone(),
                rootfs: vm.rootfs.clone(),
            }),
        };
        Self {
            id: job.id.to_string(),
            replicas: job.replicas.get(),
            resources: ResourcesInput {
                cpu_milli: job.resources.cpu_milli,
                memory_bytes: job.resources.memory_bytes,
            },
            driver,
        }
    }
}

// ---------------------------------------------------------------------------
// Describe-wire render constructors — the inverse of the `from_submit`
// family per ADR-0064 § 3. Validation lives on the submit side
// (`from_submit`); rendering lives here (`to_describe`). Each projects a
// persisted intent payload onto its describe-wire shape in
// `crate::api::describe`.
// ---------------------------------------------------------------------------

impl JobV2 {
    /// Project a persisted `JobV2` onto its describe-wire shape per
    /// ADR-0064 § 3. The Job arm carries no platform-derived field, so
    /// this delegates to the existing [`From<&Job>`] impl. (`Job =
    /// JobV2`.)
    #[must_use]
    pub fn to_describe(&self) -> JobSpecInput {
        JobSpecInput::from(self)
    }
}

impl ServiceV2 {
    /// The **single source** for this Service's operator-declared
    /// listener-port set, in declaration order (D-BLOCKER1
    /// one-source/two-readers, GH #241).
    ///
    /// `self.listeners[].port` is the canonical declaration the
    /// inbound-TPROXY path keys on. Both the reconciler producer
    /// (`overdrive_reconcilers::project_service_listen_ports`, the
    /// Service arm) and the Slice 05 liveness-restart spec path read
    /// through this one method, so the projected set stays structurally
    /// identical across the two readers — a future filter / dedup / sort
    /// lives here and cannot diverge between them.
    #[must_use]
    pub fn listen_ports(&self) -> Vec<std::num::NonZeroU16> {
        self.listeners.iter().map(|l| l.port).collect()
    }

    /// Project a persisted `ServiceV2` plus its platform-issued VIP onto
    /// the describe-wire [`crate::api::describe::ServiceSpecOutput`] per
    /// ADR-0064 § 3.
    ///
    /// The `vip` is passed in by the handler after a read-only
    /// `allocator.get(&spec_digest)` (ADR-0064 OQ-7) — the allocator
    /// memo is the source of truth (ADR-0049 § 5a), so the VIP is NOT
    /// read from the spec (the spec carries no VIP). Keeping the VIP a
    /// parameter keeps this render constructor pure and the dependency
    /// direction correct: `overdrive-core` does not reach into the
    /// control plane (ADR-0064 § 3).
    ///
    /// Listeners project from the intent shape (`NonZeroU16` / `Proto`)
    /// back to the wire shape (`u16` / lowercase protocol string), in
    /// declaration order — the inverse of the projection
    /// [`ServiceV2::from_submit`] applies.
    ///
    /// The three probe vectors (`startup_probes`, `readiness_probes`,
    /// `liveness_probes`) project read-only from the persisted intent so
    /// an operator who declared a `[[health_check.startup]]` probe sees
    /// it reflected on describe (the round-trip gap this closes per the
    /// ADR-0064 amendment). Readiness / liveness surface as `[]` until
    /// slices 02-01 / 02-02 populate them.
    #[must_use]
    pub fn to_describe(
        &self,
        vip: crate::id::ServiceVip,
    ) -> crate::api::describe::ServiceSpecOutput {
        // Per ADR-0083 §D4 (GH #42): a `ServiceV2` can never legitimately
        // hold `WorkloadDriver::Vm` — `ServiceV2::from_submit` rejects
        // `driver: DriverInput::Vm(_)` before a `ServiceV2` is ever
        // constructed (guest networking is not mesh-enrolled, GH #222).
        // Reaching this arm would mean that invariant was bypassed
        // elsewhere — a logic bug, not a runtime condition to render.
        let (command, args) = match &self.driver {
            WorkloadDriver::Exec(exec) => (exec.command.clone(), exec.args.clone()),
            WorkloadDriver::Vm(_) => unreachable!(
                "ServiceV2::from_submit rejects DriverInput::Vm; a ServiceV2 with \
                 WorkloadDriver::Vm should never exist"
            ),
        };
        let listeners = self
            .listeners
            .iter()
            .map(|listener| crate::api::submit::ListenerInput {
                port: listener.port.get(),
                protocol: listener.protocol.as_str().to_owned(),
            })
            .collect();
        crate::api::describe::ServiceSpecOutput {
            id: self.id.to_string(),
            replicas: self.replicas.get(),
            resources: ResourcesInput {
                cpu_milli: self.resources.cpu_milli,
                memory_bytes: self.resources.memory_bytes,
            },
            driver: DriverInput::Exec(ExecInput { command, args }),
            listeners,
            startup_probes: self.startup_probes.clone(),
            readiness_probes: self.readiness_probes.clone(),
            liveness_probes: self.liveness_probes.clone(),
            vip,
        }
    }
}

impl ScheduleV2 {
    /// RED scaffold per `.claude/rules/testing.md` § "Production-side
    /// scaffolds": Schedule describe is unreachable in Phase 1 (no
    /// Schedule can be persisted — [`ScheduleV2::from_submit`] is itself
    /// a scaffold). The describe handler returns a structured rejection
    /// on `WorkloadIntent::Schedule`, so this body is unreachable from
    /// any existing test. Lands GREEN when the Schedule submit path
    /// ships per ADR-0064 OQ-5.
    #[expect(clippy::todo, reason = "RED scaffold — lands with Schedule submit per OQ-5")]
    #[must_use]
    pub fn to_describe(&self) -> crate::api::describe::ScheduleSpecOutput {
        todo!(
            "RED scaffold: ScheduleV2::to_describe lands with the Schedule submit path per ADR-0064 OQ-5"
        )
    }
}

// ---------------------------------------------------------------------------
// Node aggregate
// ---------------------------------------------------------------------------

/// The intent-side Node aggregate. Carries a node's declared identity,
/// region, and capacity envelope.
///
/// rkyv-archived bytes are canonical; serde-JSON is the wire form. See
/// [`Job`] for the full canonicalisation story.
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
pub struct Node {
    pub id: NodeId,
    pub region: Region,
    pub capacity: Resources,
}

impl Node {
    /// Validating constructor. Rejects zero-memory capacity per US-01 AC.
    ///
    /// Wraps [`NodeId`] and [`Region`] `FromStr` errors through
    /// `AggregateError::Id(..)` via `#[from]`.
    pub fn new(spec: NodeSpecInput) -> Result<Self, AggregateError> {
        let NodeSpecInput { id, region, cpu_milli, memory_bytes } = spec;
        let id = NodeId::new(&id)?;
        let region = Region::new(&region)?;
        if memory_bytes == 0 {
            return Err(AggregateError::Validation {
                field: "memory_bytes",
                message: "node capacity must not declare zero memory".to_string(),
            });
        }
        let capacity = Resources { cpu_milli, memory_bytes };
        Ok(Self { id, region, capacity })
    }
}

/// Input shape for `Node::new`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSpecInput {
    pub id: String,
    pub region: String,
    pub cpu_milli: u32,
    pub memory_bytes: u64,
}

// ---------------------------------------------------------------------------
// Allocation aggregate
// ---------------------------------------------------------------------------

/// The intent-side Allocation aggregate. Links a Job and a Node through
/// typed newtypes only — no raw String / u64 identifiers per US-01 AC.
///
/// rkyv-archived bytes are canonical; serde-JSON is the wire form. See
/// [`Job`] for the full canonicalisation story.
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
pub struct Allocation {
    pub id: AllocationId,
    pub workload_id: WorkloadId,
    pub node_id: NodeId,
}

impl Allocation {
    /// Validating constructor. The `AllocationId` is typically freshly
    /// minted by the caller; this constructor validates each newtype
    /// parse via their `FromStr` impls, wrapping failures through
    /// `AggregateError::Id(..)`.
    pub fn new(spec: AllocationSpecInput) -> Result<Self, AggregateError> {
        let AllocationSpecInput { id, workload_id, node_id } = spec;
        let id = AllocationId::new(&id)?;
        let workload_id = WorkloadId::new(&workload_id)?;
        let node_id = NodeId::new(&node_id)?;
        Ok(Self { id, workload_id, node_id })
    }
}

/// Input shape for `Allocation::new`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocationSpecInput {
    pub id: String,
    pub workload_id: String,
    pub node_id: String,
}

// ---------------------------------------------------------------------------
// Policy / Investigation stubs
// ---------------------------------------------------------------------------

/// Policy aggregate stub. Per ADR-0011, this carries only the ID newtype
/// as primary field in Phase 1; behavioural fields land Phase 2+.
///
/// SCAFFOLD: true
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub id: PolicyId,
}

/// Investigation aggregate stub. Per ADR-0011 and whitepaper §12, this
/// carries only the ID newtype in Phase 1.
///
/// SCAFFOLD: true
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Investigation {
    pub id: InvestigationId,
}

// ---------------------------------------------------------------------------
// Intent-key derivation
// ---------------------------------------------------------------------------

/// Canonical intent-key derivation surface.
///
/// Every caller (CLI, handler, describe) routes through these functions —
/// any drift-prone second copy in production code violates US-01's
/// shared-artifacts-registry entry for `intent_key`. The string form is
/// `workloads/<WorkloadId::display>`, `nodes/<NodeId::display>`, or
/// `allocations/<AllocationId::display>` per ADR-0011.
///
/// The wrapped bytes are always valid UTF-8 by construction — the `<id>`
/// half flows through `Display` for a newtype whose `validate_label`
/// guarantees ASCII-only output (see `id::validate_label`), and the
/// prefix is a fixed ASCII literal.
pub struct IntentKey(Vec<u8>);

impl IntentKey {
    /// Derive the intent key for a workload aggregate body —
    /// `workloads/<id>`. Per ADR-0050 OQ-5 single-cut migration: this
    /// replaces the legacy `for_job` derivation. The aggregate body
    /// at this key carries `WorkloadIntentEnvelope` rkyv-archived
    /// bytes (Job / Service / Schedule).
    pub fn for_workload(id: &WorkloadId) -> Self {
        Self(format!("workloads/{id}").into_bytes())
    }

    /// Derive the intent key for a workload's stop signal —
    /// `workloads/<id>/stop`. Per ADR-0050 OQ-5 single-cut migration:
    /// this replaces the legacy `for_job_stop` derivation. The stop
    /// sentinel is a separate intent record so the original aggregate
    /// stays readable for audit / rollback / debug; the value is the
    /// empty byte slice — the existence is the signal.
    pub fn for_workload_stop(id: &WorkloadId) -> Self {
        Self(format!("workloads/{id}/stop").into_bytes())
    }

    /// Derive the intent key for a workload's desired-run generation —
    /// `workloads/<id>/generation`.
    ///
    /// A monotonic `u64` bumped by `overdrive workload restart` (ADR-0073
    /// item 4); the `WorkloadLifecycle` reconciler places a fresh instance
    /// when its View's `observed_generation < generation`. The value is
    /// 8-byte big-endian; an absent key decodes as generation `0`. Stored
    /// as a separate intent record (sibling-key precedent:
    /// `workloads/<id>/stop` is an empty-byte sentinel, `workloads/<id>/kind`
    /// is a single ASCII discriminator — neither is rkyv-envelope-wrapped,
    /// and neither is this).
    ///
    /// Aligns with #180's `workloads/<id>/current` pointer vocabulary;
    /// folds into that pointer row when #180's revision lineage lands (a
    /// single-cut migration already anticipated in ADR-0050 § Consequences).
    pub fn for_workload_generation(id: &WorkloadId) -> Self {
        Self(format!("workloads/{id}/generation").into_bytes())
    }

    /// Derive the intent key for a workload's kind discriminator —
    /// `workloads/<id>/kind`.
    ///
    /// Per ADR-0047 §1 / slice 02 of `workload-kind-discriminator`: the
    /// workload-kind discriminator (`service` / `job` / `schedule`) is
    /// persisted as a separate intent record alongside the `Job`
    /// aggregate. The streaming endpoint reads this key at submit-stream
    /// open time to dispatch on per-kind streaming-event sibling enums
    /// (ADR-0047 §3 [D7]); the reconciler runtime reads it at
    /// `hydrate_desired` time to populate `WorkloadLifecycleState.workload_kind`
    /// so the natural-exit emission path (ADR-0037 Amendment 2026-05-10)
    /// fires for Job-kind workloads.
    ///
    /// The value at this key is a single ASCII byte: `s` for Service,
    /// `j` for Job, `c` for sChedule. A single-byte discriminator (vs
    /// rkyv-archived enum) keeps the read path branch-free at every
    /// consumer and makes the file shape trivially debuggable with
    /// `bpftool` / `redb-cli` / hex dumps.
    pub fn for_workload_kind(id: &WorkloadId) -> Self {
        Self(format!("workloads/{id}/kind").into_bytes())
    }

    /// Derive the intent key for a Schedule. Stable for any valid
    /// `WorkloadId` per the same ASCII-only invariants that govern
    /// [`Self::for_workload`]. The string form is `schedules/<WorkloadId::Display>`.
    ///
    /// Per ADR-0047 §1 / slice 05 of `workload-kind-discriminator`,
    /// Schedule is a third workload kind alongside Service and Job;
    /// it persists alongside `[job]` in TOML but lives at its own
    /// canonical key prefix so a job-named-the-same and a
    /// schedule-named-the-same remain distinct intents at the
    /// IntentStore level (no key collision, no "stop the schedule"
    /// shape stops the standalone job, ...).
    pub fn for_schedule(id: &WorkloadId) -> Self {
        Self(format!("schedules/{id}").into_bytes())
    }

    /// Derive the intent key for a Node.
    pub fn for_node(id: &NodeId) -> Self {
        Self(format!("nodes/{id}").into_bytes())
    }

    /// Derive the intent key for a workflow *instance* —
    /// `workflows/<correlation>`. Per ADR-0064 §5: a committed
    /// `Action::StartWorkflow` persists a workflow-instance desired-intent
    /// keyed by the instance [`CorrelationKey`], mirroring how
    /// `StartAllocation` persists workload intent. The
    /// `WorkflowLifecycle` reconciler's `hydrate_desired` scans the
    /// `workflows/` prefix to read every desired instance back; the value
    /// at this key is the workflow spec's inputs (the kind name), NOT a
    /// derived status (`development.md` § "Persist inputs, not derived
    /// state").
    ///
    /// The `<correlation>` half flows through the `CorrelationKey`'s
    /// canonical string form, which is valid UTF-8 by construction, so the
    /// wrapped bytes stay UTF-8 (the struct-level invariant).
    pub fn for_workflow_instance(correlation: &CorrelationKey) -> Self {
        Self(format!("workflows/{correlation}").into_bytes())
    }

    /// The `workflows/` key prefix — the scan root the
    /// `WorkflowLifecycle` reconciler's `hydrate_desired` uses to read
    /// every persisted workflow-instance intent (paired with
    /// [`Self::for_workflow_instance`]).
    #[must_use]
    pub const fn workflow_instance_prefix() -> &'static [u8] {
        b"workflows/"
    }

    /// Derive the intent key for an Allocation.
    pub fn for_allocation(id: &AllocationId) -> Self {
        Self(format!("allocations/{id}").into_bytes())
    }

    /// Raw bytes view of the intent key. Used by `IntentStore::put` /
    /// `get`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Canonical string form — `workloads/<WorkloadId>`, `nodes/<NodeId>`, or
    /// `allocations/<AllocationId>`. Always succeeds: the byte buffer is
    /// UTF-8 by construction (see the struct-level docs).
    ///
    /// `expect` is the right idiom here: the buffer is built entirely
    /// from a fixed ASCII prefix and the lowercased-ASCII output of
    /// `validate_label`, so `from_utf8` cannot fail without violating a
    /// type-system invariant the `id.rs` proptests pin.
    #[allow(clippy::expect_used)]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0)
            .expect("IntentKey bytes are always valid UTF-8 by construction")
    }
}
