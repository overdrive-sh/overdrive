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
//! `jobs/<id>` / `nodes/<id>` / `allocations/<id>`.
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
use crate::id::{AllocationId, ContentHash, InvestigationId, NodeId, PolicyId, Region, WorkloadId};
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
pub use self::workload_spec::{
    CronExpr, JobSpec, Listener, ParseError, ScheduleSpec, ServiceSpec, ServiceVip, WorkloadKind,
    WorkloadSpec, WorkloadSpecInput,
};

mod workload_spec;

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
/// `WorkloadDriver::Exec(Exec { command, args })` is the single Phase-1
/// variant; future variants (`MicroVm(MicroVm)`, `Wasm(Wasm)`) append
/// additively. The driver passes the inner `Exec.command` / `Exec.args`
/// to `tokio::process::Command::new(impl AsRef<OsStr>).args(...)` — no
/// newtype is warranted (per `.claude/rules/development.md` § Newtypes),
/// and validation lives in `Job::from_spec`.
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
/// # Envelope wrapping (ADR-0048)
///
/// Per ADR-0048 § 4 (outer-envelope-only on `Job`), `Job` is the
/// inner payload type wrapped by [`JobEnvelope`]. Under the UI-02
/// amendment (alias-to-payload public API), `pub type Job = JobV1`
/// preserves every existing struct-literal `Job { id, replicas,
/// resources, driver }` construction across the workspace
/// unchanged. The persistence-boundary code (codec-internal — only
/// `LocalIntentStore` should name `JobEnvelope`) is the SOLE site
/// that wraps via [`JobEnvelope::latest`]. Embedded
/// `WorkloadDriver` and `Exec` types are NOT wrapped per ADR-0048
/// § 4 — schema changes there bump the outer `JobEnvelope` version.
pub type Job = JobV1;

/// Validated intent-side counterpart to wire-shape [`DriverInput`]. One
/// variant per driver class; new variants append in Phase 2+
/// (`MicroVm(MicroVm)`, `Wasm(Wasm)`).
///
/// Naming: `WorkloadDriver`, not `Driver`, to disambiguate from the
/// `Driver` *trait* at `crates/overdrive-core/src/traits/driver.rs`
/// (per ADR-0030 §1). The trait is the driver implementation surface
/// (`Driver::start(&AllocationSpec)`); this enum is the operator's
/// declared driver-class intent on the [`Job`] aggregate.
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
pub enum WorkloadDriver {
    /// Native binary under cgroups v2. Mirrors wire-shape
    /// [`DriverInput::Exec`].
    Exec(Exec),
    // Future Phase 2+: MicroVm(MicroVm), Wasm(Wasm).
}

/// Exec-driver invocation fields. Mirrors wire-shape [`ExecInput`] on
/// the intent side.
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
    /// `Job::from_spec`.
    pub command: String,
    /// Argv passed verbatim to the binary. No per-element validation —
    /// argv is opaque to the platform per ADR-0031 §4.
    pub args: Vec<String>,
}

// ---------------------------------------------------------------------------
// Job versioned envelope per ADR-0048 § 4
// ---------------------------------------------------------------------------
//
// Per ADR-0048 § 4 (outer-envelope-only on `Job`), the envelope wraps
// the outer `JobV1` payload; embedded `WorkloadDriver` / `Exec` types
// are NOT wrapped — their schema changes bump the outer `JobEnvelope`
// version.
//
// Per ADR-0048 § 2 Layer 1 + UI-01 amendment: both `JobEnvelope` and
// `JobV1` are `pub` (rustc E0446 forbids `pub(crate)` under a `pub`
// trait), but neither is re-exported from `overdrive_core::lib.rs`.
// The load-bearing Layer 1 target is `JobEnvelope` (the envelope
// is codec-internal; callers go through the `Job = JobV1` payload
// alias for struct-literal construction).
//
// Per ADR-0048 § 2 Layer 2: in-crate variant construction
// (`JobEnvelope::V1(...)` literal) is rejected by
// `xtask::dst_lint::scan_for_envelope_variant_construction`; the
// canonical construction path is `JobEnvelope::latest(payload)`.

/// Per-type rkyv versioned envelope for the [`Job`] aggregate per
/// ADR-0048 § 4.
///
/// Codec-internal — named only inside `LocalIntentStore` read/write
/// paths. Public callers use the [`Job`] alias (= [`JobV1`]) and
/// construct payloads via struct-literal syntax; the persistence
/// boundary wraps via [`JobEnvelope::latest`].
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
pub enum JobEnvelope {
    V1(JobV1),
}

/// Alias for the latest payload variant of [`JobEnvelope`]. Today
/// this is [`JobV1`]; bumping to `JobV2` updates this alias and the
/// [`VersionedEnvelope::latest`] constructor in one commit per the
/// version-bump procedure in `.claude/rules/development.md`
/// § "rkyv schema evolution".
pub type JobLatest = JobV1;

/// Inner V1 payload of the [`Job`] aggregate per ADR-0048 § 4.
///
/// rkyv archives are **fixed positional layouts** — appending a
/// field to this struct shifts every subsequent offset and renders
/// previously-archived bytes unreadable. Layout-changing edits
/// require minting a new `JobV2` payload + appending a new
/// [`JobEnvelope::V2`] variant + landing a `From<JobV1> for JobV2`
/// conversion + pinning a fresh golden-bytes fixture in
/// `tests/schema_evolution/job.rs` — all in a single commit. See
/// `.claude/rules/development.md` § "Version-bump procedure".
///
/// Per ADR-0031 Amendment 1, `driver` is a tagged enum
/// (`WorkloadDriver`) carrying the operator's invocation shape;
/// the projection from wire-shape `DriverInput::Exec` →
/// `WorkloadDriver::Exec` happens inside
/// [`Job::from_spec`](JobV1::from_spec).
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
    /// Driver-class declaration carrying the operator's invocation
    /// shape. Per ADR-0031 Amendment 1 this is a tagged enum
    /// mirroring the wire-shape `DriverInput`.
    pub driver: WorkloadDriver,
}

impl VersionedEnvelope for JobEnvelope {
    type Latest = JobV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }

    /// Discriminant offset for `JobEnvelope` archives, measured from
    /// the END of the archive bytes.
    ///
    /// Empirically determined against canonical V1 payloads of
    /// varying inner-string sizes (`workload_id`, `command`, `args`):
    /// rkyv 0.8 places the outer enum's discriminant byte 64 bytes
    /// from the END of the archive, stable across all payload sizes
    /// (the trailing "root" structure has a fixed footprint; only
    /// the leading slab grows with variable-length data).
    ///
    /// Re-pin alongside the schema-evolution fixture at every
    /// version-bump per
    /// [`VersionedEnvelope::discriminant_offset_from_end`]'s
    /// docstring.
    fn discriminant_offset_from_end() -> Option<usize> {
        Some(64)
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 carries rkyv discriminant 0 (declaration order — first
        // variant). Empirically verified by archiving a canonical
        // `JobEnvelope::latest(...)` and inspecting the byte at
        // `bytes.len() - 64`.
        &[0]
    }

    fn type_name() -> &'static str {
        "JobEnvelope"
    }
}

impl JobV1 {
    /// Validating constructor. Per US-01 AC, this is the single path into
    /// the intent-side `Job` aggregate. Every CLI handler and every
    /// server handler routes through here.
    ///
    /// Rejects zero replicas, zero-byte memory capacity, and (per
    /// ADR-0031 §4) empty / whitespace-only `exec.command`. Wraps
    /// [`WorkloadId`]'s `FromStr` error through `AggregateError::Id(..)` via
    /// `#[from]`.
    pub fn from_spec(spec: JobSpecInput) -> Result<Self, AggregateError> {
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
        let DriverInput::Exec(exec_input) = driver;
        if exec_input.command.trim().is_empty() {
            return Err(AggregateError::Validation {
                field: "exec.command",
                message: "command must be non-empty".to_string(),
            });
        }
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
        })
    }
}

// ---------------------------------------------------------------------------
// Job typed persistence-boundary codec (UI-03 — typed codec on `Job`)
// ---------------------------------------------------------------------------
//
// Per ADR-0048 § "Intent persistence boundary — typed codec on `Job`"
// (UI-03 amendment 2026-05-12): the `IntentStore` trait is a generic
// byte-level k/v store (Jobs, kind discriminators, stop markers,
// snapshot frames). The envelope-wrapping discipline lives on the
// `Job` type itself, NOT inside the adapter trait. Every Job writer in
// the workspace goes through [`Job::archive_for_store`]; every Job
// reader goes through [`Job::from_store_bytes`]. The two methods are
// the SOLE wrapping sites — public callers continue to construct
// payloads via struct-literal `Job { ... }` (= `JobV1 { ... }`)
// unchanged.

impl JobV1 {
    /// Archive a [`Job`] for persistence through the [`IntentStore`].
    ///
    /// # Preconditions
    ///
    /// `self` is a valid [`JobV1`] payload — every field has gone
    /// through [`JobV1::from_spec`] validation. There are no further
    /// preconditions.
    ///
    /// # Postconditions
    ///
    /// On `Ok(bytes)`, `bytes` is the canonical rkyv-archived byte
    /// sequence of `JobEnvelope::V1(self.clone())`. Two archivals of
    /// the same logical [`Job`] produce byte-identical output (rkyv
    /// canonicalisation, per `.claude/rules/development.md` § "Internal
    /// data → rkyv"). The returned [`AlignedVec`] is the wire shape
    /// every Job writer at the persistence boundary uses to bridge to
    /// [`IntentStore::put`] / [`IntentStore::put_if_absent`] —
    /// callers pass `bytes.as_ref()` to the trait's `&[u8]` surface.
    ///
    /// # Observable invariants
    ///
    /// `Job::from_store_bytes(&self.archive_for_store()?, p)` returns
    /// `Ok(self_owned)` bit-equivalent to `self` for any redb path
    /// `p`. The envelope wrap is internal — the caller never names
    /// [`JobEnvelope`].
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeError::Malformed`] when the rkyv serialiser
    /// itself fails. This is unreachable in practice for valid
    /// [`JobV1`] payloads — the rkyv derives accept every shape
    /// [`JobV1::from_spec`] produces — but the variant is preserved
    /// as the structured error surface so future explicit-translation
    /// migrations have a typed slot.
    pub fn archive_for_store(&self) -> Result<AlignedVec, EnvelopeError> {
        let envelope = JobEnvelope::latest(self.clone());
        rkyv::to_bytes::<rkyv::rancor::Error>(&envelope)
            .map_err(|source| EnvelopeError::Malformed { source })
    }

    /// Decode persisted bytes back into a [`Job`].
    ///
    /// # Preconditions
    ///
    /// `bytes` is either the rkyv-archived bytes produced by
    /// [`JobV1::archive_for_store`] against some [`JobV1`] payload
    /// (success path), OR an arbitrary byte slice that does NOT
    /// decode through the current envelope shape (malformed /
    /// unknown-future variant — error path). `redb_path` names the
    /// underlying redb file the bytes were read from, used in the
    /// operator-facing remediation message; it is observed but not
    /// required to exist. `key` optionally names the redb key the
    /// bytes were read from (e.g. `"jobs/svc-payments"`); when
    /// `Some`, it threads into the `health.startup.refused` tracing
    /// event so an operator with N jobs in the file can identify
    /// which specific row failed to decode. Callers without a key
    /// context (the read handlers, the reconciler runtime read path)
    /// pass `None`; the recovery walk in `LocalIntentStore::open`
    /// passes the iterated key.
    ///
    /// # Postconditions
    ///
    /// On `Ok(job)`, `job` is the canonical [`Job`] payload projected
    /// from the envelope via [`VersionedEnvelope::into_latest`]. No
    /// tracing event fires on the success path.
    ///
    /// On `Err(...)`, exactly one `tracing::error!` event with
    /// `name: "health.startup.refused"` fires BEFORE the `Err` value
    /// is returned. The event carries the `redb_path`, the `key`
    /// (`"<unknown>"` when `None`), and the underlying
    /// `envelope_error` for operator diagnosis. The returned
    /// [`IntentStoreError::Envelope`]'s [`Display`] form names the
    /// `redb_path` twice (in the decode-failure line and in the
    /// remediation hint `delete {redb_path}`) per ADR-0048 § 6.
    ///
    /// # Edge cases
    ///
    /// * Empty `bytes` → [`EnvelopeError::Malformed`] (rkyv validator
    ///   rejection).
    /// * Bytes from a writer that has bumped the envelope to
    ///   `V<N+1>` while this binary knows only up to `V<N>` →
    ///   [`EnvelopeError::UnknownVersion`] (surfaced by
    ///   [`probe_known_variant`] before rkyv decode). The structured
    ///   surface carries the observed discriminant byte and the
    ///   envelope's [`VersionedEnvelope::type_name`] for
    ///   operator-facing diagnostics.
    /// * Truncated / corrupt bytes → [`EnvelopeError::Malformed`].
    ///
    /// # Observable invariants
    ///
    /// The `health.startup.refused` event MUST NOT fire on the
    /// success path. The event MUST fire exactly once before the
    /// `Err` return. The asymmetric intent-fail-fast policy (ADR-0048
    /// § 3) is implemented by the caller — `from_store_bytes`
    /// surfaces the error; the caller (`LocalIntentStore::open`)
    /// propagates it to abort startup.
    pub fn from_store_bytes(
        bytes: &[u8],
        redb_path: &Path,
        key: Option<&str>,
    ) -> Result<Self, IntentStoreError> {
        // `decode_envelope_bytes` composes the canonical
        // "align + probe + rkyv decode + into_latest" pipeline per
        // ADR-0048 § 4b. Probing the rkyv-archived discriminant byte
        // BEFORE attempting full decode is what distinguishes a future
        // binary's `V<N+1>` (surfaces as
        // `EnvelopeError::UnknownVersion`) from corrupt bytes
        // (`Malformed`) — the operator-facing remediation diverges.
        // The intent-layer policy below (fail-fast with structured
        // `health.startup.refused` event) is per ADR-0048 § 3.
        match decode_envelope_bytes::<JobEnvelope>(bytes) {
            Ok(job) => Ok(job),
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

    /// Canonical content-addressed identity of a [`Job`].
    ///
    /// # Preconditions
    ///
    /// `self` is a valid [`JobV1`] payload.
    ///
    /// # Postconditions
    ///
    /// Returns SHA-256 over the rkyv-archived **raw payload** bytes
    /// (`rkyv::to_bytes(self)`), **not** the envelope-wrapped bytes
    /// that [`Self::archive_for_store`] produces. Two calls against
    /// the same logical [`Job`] return bit-identical hashes
    /// (canonical rkyv archive is byte-stable), and the hash is
    /// stable across envelope version bumps — a future
    /// `JobEnvelope::V2` does not change the digest for the same
    /// logical payload.
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeError::Malformed`] if the rkyv serialiser
    /// fails (unreachable for valid [`JobV1`] payloads).
    pub fn spec_digest(&self) -> Result<ContentHash, EnvelopeError> {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(self)
            .map_err(|source| EnvelopeError::Malformed { source })?;
        Ok(ContentHash::of(bytes.as_ref()))
    }
}

/// Input shape for `Job::from_spec`. The CLI deserialises TOML into this
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
/// `POST /v1/jobs` and `GET /v1/jobs/{id}` (ADR-0014 §Shared types).
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
/// `Job::from_spec` (no `From` impl: the ≥3-call-sites rule isn't met,
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
/// Today: one variant (`Exec`). Future drivers (`microvm`, `wasm`) add
/// new variants additively; no shape change to surrounding code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum DriverInput {
    /// Native binary under cgroups v2 — the `[exec]` table in TOML.
    Exec(ExecInput),
    // Future: MicroVm(MicroVmInput), Wasm(WasmInput)
}

/// Operator-facing `[exec]` table fields per ADR-0031 §2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecInput {
    /// Host filesystem path to the binary. Validated non-empty (after
    /// trim) at `Job::from_spec` per ADR-0031 §4.
    pub command: String,
    /// Argv passed verbatim. Required field — an absent `args` is a
    /// parse error, not "default to no args" (per ADR-0031 §8). Empty
    /// `Vec` is the legitimate zero-args case.
    pub args: Vec<String>,
}

/// Reverse conversion — reconstruct the wire-shape `JobSpecInput` from a
/// validated `Job` aggregate. Used by `describe_workload` (ADR-0008 §GET
/// /v1/jobs/{id}) to render the stored spec back onto the wire after
/// rkyv access + deserialize.
///
/// Non-fallible by construction: every field in `JobSpecInput` is a
/// projection of a field already validated by `Job::from_spec`. Cloning
/// the `id` is cheap — `WorkloadId::to_string()` is an owned ASCII string.
impl From<&Job> for JobSpecInput {
    fn from(job: &Job) -> Self {
        // Per ADR-0031 Amendment 1, project the intent-shape
        // `WorkloadDriver` back to the wire-shape `DriverInput`. Today
        // the destructure is irrefutable (single Phase-1 variant); when
        // future variants land it becomes a `match` and each arm
        // projects to its sibling `DriverInput::*` variant.
        let WorkloadDriver::Exec(exec) = &job.driver;
        Self {
            id: job.id.to_string(),
            replicas: job.replicas.get(),
            resources: ResourcesInput {
                cpu_milli: job.resources.cpu_milli,
                memory_bytes: job.resources.memory_bytes,
            },
            driver: DriverInput::Exec(ExecInput {
                command: exec.command.clone(),
                args: exec.args.clone(),
            }),
        }
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
/// `jobs/<WorkloadId::display>`, `nodes/<NodeId::display>`, or
/// `allocations/<AllocationId::display>` per ADR-0011.
///
/// The wrapped bytes are always valid UTF-8 by construction — the `<id>`
/// half flows through `Display` for a newtype whose `validate_label`
/// guarantees ASCII-only output (see `id::validate_label`), and the
/// prefix is a fixed ASCII literal.
pub struct IntentKey(Vec<u8>);

impl IntentKey {
    /// Derive the intent key for a Job. Stable for any valid `WorkloadId` per
    /// US-01 AC (property test).
    pub fn for_job(id: &WorkloadId) -> Self {
        Self(format!("jobs/{id}").into_bytes())
    }

    /// Derive the intent key for a Job's stop signal — `jobs/<id>/stop`.
    /// Per ADR-0027, the stop signal is a separate intent record so the
    /// original job spec stays readable for audit / rollback / debug.
    /// `IntentKey::for_job_stop(&id)` is byte-stable for any valid
    /// `WorkloadId`; the `/stop` suffix is fixed ASCII and the prefix `jobs/`
    /// reuses the canonical ASCII derivation from `for_job`.
    pub fn for_job_stop(id: &WorkloadId) -> Self {
        Self(format!("jobs/{id}/stop").into_bytes())
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
    /// [`Self::for_job`]. The string form is `schedules/<WorkloadId::Display>`.
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

    /// Derive the intent key for an Allocation.
    pub fn for_allocation(id: &AllocationId) -> Self {
        Self(format!("allocations/{id}").into_bytes())
    }

    /// Raw bytes view of the intent key. Used by `IntentStore::put` /
    /// `get`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Canonical string form — `jobs/<WorkloadId>`, `nodes/<NodeId>`, or
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
