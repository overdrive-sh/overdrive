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

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::id::{AllocationId, InvestigationId, JobId, NodeId, PolicyId, Region};
use crate::traits::driver::Resources;

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
pub struct Job {
    pub id: JobId,
    pub replicas: NonZeroU32,
    pub resources: Resources,
    /// Driver-class declaration carrying the operator's invocation
    /// shape. Per ADR-0031 Amendment 1 this is a tagged enum mirroring
    /// the wire-shape `DriverInput`; the projection from
    /// `DriverInput::Exec` → `WorkloadDriver::Exec` happens inside
    /// `Job::from_spec`.
    pub driver: WorkloadDriver,
}

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

impl Job {
    /// Validating constructor. Per US-01 AC, this is the single path into
    /// the intent-side `Job` aggregate. Every CLI handler and every
    /// server handler routes through here.
    ///
    /// Rejects zero replicas, zero-byte memory capacity, and (per
    /// ADR-0031 §4) empty / whitespace-only `exec.command`. Wraps
    /// [`JobId`]'s `FromStr` error through `AggregateError::Id(..)` via
    /// `#[from]`.
    //
    // The `todo!()` below is the documented RED scaffold for step 01-02
    // per `.claude/rules/testing.md` § "RED scaffolds and intentionally-
    // failing commits". Step 01-02 replaces the panic with the real
    // `AggregateError::Validation { field: "exec.command", ... }`
    // return — until then the panic IS the specification of work not
    // yet done. The matching acceptance scenarios in
    // `tests/acceptance/exec_validation.rs` are intentionally RED.
    #[allow(clippy::todo)]
    pub fn from_spec(spec: JobSpecInput) -> Result<Self, AggregateError> {
        let JobSpecInput { id, replicas, resources, driver } = spec;
        let id = JobId::new(&id)?;
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
        let resources_struct =
            Resources { cpu_milli: resources.cpu_milli, memory_bytes: resources.memory_bytes };
        // RED scaffold — project the wire-shape `DriverInput` into the
        // intent-shape `WorkloadDriver` per ADR-0031 Amendment 1, applying
        // the ADR-0031 §4 non-empty-after-trim rule on the way. The match
        // arm that destructures `DriverInput::Exec` is in place; the
        // validation body that fires `AggregateError::Validation
        // { field: "exec.command", message: "command must be non-empty" }`
        // is the RED scaffold the DELIVER crafter replaces with the real
        // predicate.
        let DriverInput::Exec(exec_input) = driver;
        // The trim check is the load-bearing predicate. Until the
        // crafter lands the real body, the panic IS the specification
        // of work not yet done — the matching acceptance scenarios in
        // `tests/acceptance/aggregate_validation.rs` will hit this
        // arm at RED time.
        if exec_input.command.trim().is_empty() {
            todo!(
                "RED scaffold: ADR-0031 §4 — Job::from_spec must reject empty / \
                 whitespace-only `exec.command` with AggregateError::Validation \
                 {{ field: \"exec.command\", message: \"command must be non-empty\" }}"
            );
        }
        let driver =
            WorkloadDriver::Exec(Exec { command: exec_input.command, args: exec_input.args });
        Ok(Self { id, replicas, resources: resources_struct, driver })
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
/// (ADR-0009, `cargo xtask openapi-gen`) renders the spec shape
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
/// derives. `From<ResourcesInput> for Resources` is non-fallible — the
/// validation rules (`memory_bytes != 0`) live in `Job::from_spec`.
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
/// validated `Job` aggregate. Used by `describe_job` (ADR-0008 §GET
/// /v1/jobs/{id}) to render the stored spec back onto the wire after
/// rkyv access + deserialize.
///
/// Non-fallible by construction: every field in `JobSpecInput` is a
/// projection of a field already validated by `Job::from_spec`. Cloning
/// the `id` is cheap — `JobId::to_string()` is an owned ASCII string.
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
    pub job_id: JobId,
    pub node_id: NodeId,
}

impl Allocation {
    /// Validating constructor. The `AllocationId` is typically freshly
    /// minted by the caller; this constructor validates each newtype
    /// parse via their `FromStr` impls, wrapping failures through
    /// `AggregateError::Id(..)`.
    pub fn new(spec: AllocationSpecInput) -> Result<Self, AggregateError> {
        let AllocationSpecInput { id, job_id, node_id } = spec;
        let id = AllocationId::new(&id)?;
        let job_id = JobId::new(&job_id)?;
        let node_id = NodeId::new(&node_id)?;
        Ok(Self { id, job_id, node_id })
    }
}

/// Input shape for `Allocation::new`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocationSpecInput {
    pub id: String,
    pub job_id: String,
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
/// `jobs/<JobId::display>`, `nodes/<NodeId::display>`, or
/// `allocations/<AllocationId::display>` per ADR-0011.
///
/// The wrapped bytes are always valid UTF-8 by construction — the `<id>`
/// half flows through `Display` for a newtype whose `validate_label`
/// guarantees ASCII-only output (see `id::validate_label`), and the
/// prefix is a fixed ASCII literal.
pub struct IntentKey(Vec<u8>);

impl IntentKey {
    /// Derive the intent key for a Job. Stable for any valid `JobId` per
    /// US-01 AC (property test).
    pub fn for_job(id: &JobId) -> Self {
        Self(format!("jobs/{id}").into_bytes())
    }

    /// Derive the intent key for a Job's stop signal — `jobs/<id>/stop`.
    /// Per ADR-0027, the stop signal is a separate intent record so the
    /// original job spec stays readable for audit / rollback / debug.
    /// `IntentKey::for_job_stop(&id)` is byte-stable for any valid
    /// `JobId`; the `/stop` suffix is fixed ASCII and the prefix `jobs/`
    /// reuses the canonical ASCII derivation from `for_job`.
    pub fn for_job_stop(id: &JobId) -> Self {
        Self(format!("jobs/{id}/stop").into_bytes())
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

    /// Canonical string form — `jobs/<JobId>`, `nodes/<NodeId>`, or
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
