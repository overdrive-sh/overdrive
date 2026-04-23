//! Intent-side aggregates — `Job`, `Node`, `Allocation`, `Policy`,
//! `Investigation`.
//!
//! Per ADR-0011, intent-side aggregates live here; observation-side row
//! shapes live in `crate::traits::observation_store`. The two never merge.
//!
//! Validating constructors return `Result<Self, AggregateError>`.
//! Step 01-01 (delivered) lands the `Job` / `Node` / `Allocation`
//! validating constructors and the `Resources`-deduplication invariant.
//!
//! Still scaffolded (RED — owned by later steps): `IntentKey::for_job` /
//! `for_node` / `for_allocation` / `as_str`, rkyv/serde derives on the
//! aggregate structs (step 01-03), and behavioural expansion of `Policy`
//! and `Investigation` (Phase 2+).

use std::num::NonZeroU32;

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: JobId,
    pub replicas: NonZeroU32,
    pub resources: Resources,
}

impl Job {
    /// Validating constructor. Per US-01 AC, this is the single path into
    /// the intent-side `Job` aggregate. Every CLI handler and every
    /// server handler routes through here.
    ///
    /// Rejects zero replicas and zero-byte memory capacity; wraps
    /// [`JobId`]'s `FromStr` error through `AggregateError::Id(..)` via
    /// `#[from]`.
    pub fn from_spec(spec: JobSpecInput) -> Result<Self, AggregateError> {
        let JobSpecInput { id, replicas, cpu_milli, memory_bytes } = spec;
        let id = JobId::new(&id)?;
        let replicas = NonZeroU32::new(replicas).ok_or_else(|| AggregateError::Validation {
            field: "replicas",
            message: format!("replica count must be non-zero; got {replicas}"),
        })?;
        if memory_bytes == 0 {
            return Err(AggregateError::Validation {
                field: "memory_bytes",
                message: "memory capacity must be non-zero".to_string(),
            });
        }
        let resources = Resources { cpu_milli, memory_bytes };
        Ok(Self { id, replicas, resources })
    }
}

/// Input shape for `Job::from_spec`. The CLI deserialises TOML into this
/// type; the server deserialises JSON into the same type; both route
/// through the same constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSpecInput {
    pub id: String,
    pub replicas: u32,
    pub cpu_milli: u32,
    pub memory_bytes: u64,
}

// ---------------------------------------------------------------------------
// Node aggregate
// ---------------------------------------------------------------------------

/// The intent-side Node aggregate. Carries a node's declared identity,
/// region, and capacity envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// Every caller (CLI, handler, describe) routes through these functions.
/// The string form is `jobs/<JobId::display>`, `nodes/<NodeId::display>`,
/// or `allocations/<AllocationId::display>` per ADR-0011.
///
/// SCAFFOLD: true
pub struct IntentKey(Vec<u8>);

impl IntentKey {
    /// Derive the intent key for a Job. Stable for any valid `JobId` per
    /// US-01 AC (property test).
    ///
    /// SCAFFOLD: true
    pub fn for_job(_id: &JobId) -> Self {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// Derive the intent key for a Node.
    ///
    /// SCAFFOLD: true
    pub fn for_node(_id: &NodeId) -> Self {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// Derive the intent key for an Allocation.
    ///
    /// SCAFFOLD: true
    pub fn for_allocation(_id: &AllocationId) -> Self {
        panic!("Not yet implemented -- RED scaffold")
    }

    /// Raw bytes view of the intent key. Used by `IntentStore::put` /
    /// `get`.
    ///
    /// SCAFFOLD: true
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Canonical string form — `jobs/<JobId>`, `nodes/<NodeId>`, or
    /// `allocations/<AllocationId>`.
    ///
    /// SCAFFOLD: true
    #[allow(clippy::unused_self)]
    pub fn as_str(&self) -> &str {
        panic!("Not yet implemented -- RED scaffold")
    }
}
