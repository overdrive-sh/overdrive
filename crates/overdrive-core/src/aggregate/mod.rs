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
/// and validation lives in `JobV1::from_submit`.
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
/// # Envelope wrapping (ADR-0050)
///
/// Per ADR-0050 single-cut migration: the `Job` payload is wrapped
/// at the persistence boundary by [`WorkloadIntentEnvelope`] via
/// the [`WorkloadIntentV1::Job`] variant — NOT by a per-type
/// `JobEnvelope`. Public callers construct `Job { ... }` (=
/// `JobV1 { ... }`) values via struct-literal syntax and wrap with
/// `WorkloadIntent::Job(job)` at the persistence boundary; the
/// codec ([`WorkloadIntentV1::archive_for_store`]) is the SOLE
/// wrapping site.
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
    /// `JobV1::from_submit`.
    pub command: String,
    /// Argv passed verbatim to the binary. No per-element validation —
    /// argv is opaque to the platform per ADR-0031 §4.
    pub args: Vec<String>,
}

// ---------------------------------------------------------------------------
// Job inner payload (envelope relocated to WorkloadIntent per ADR-0050)
// ---------------------------------------------------------------------------
//
// Per ADR-0050 single-cut migration: the persistence-boundary
// envelope on `Job` (`JobEnvelope`, `JobLatest`,
// `Job::archive_for_store`, `Job::from_store_bytes`,
// `Job::spec_digest`) was deleted in this commit. The `Job` payload
// is now persisted as the inner variant of
// [`WorkloadIntentV1::Job`]; the codec lives on
// [`WorkloadIntentV1`].
//
// `JobV1::from_submit` (the validating constructor) is preserved
// unchanged — every CLI handler and every server handler still
// routes through it. The `Job` alias (= `JobV1`) is retained so
// existing struct-literal `Job { id, replicas, resources, driver }`
// construction across the workspace stays unchanged; callers wrap
// the value via `WorkloadIntent::Job(job)` at the persistence
// boundary.

/// Inner V1 payload of the [`Job`] aggregate.
///
/// rkyv archives are **fixed positional layouts** — appending a
/// field to this struct shifts every subsequent offset and renders
/// previously-archived bytes unreadable. Layout-changing edits
/// require minting a new outer envelope variant per
/// `.claude/rules/development.md` § "Version-bump procedure". The
/// envelope today is [`WorkloadIntentEnvelope`] (per ADR-0050).
///
/// Per ADR-0031 Amendment 1, `driver` is a tagged enum
/// (`WorkloadDriver`) carrying the operator's invocation shape;
/// the projection from wire-shape `DriverInput::Exec` →
/// `WorkloadDriver::Exec` happens inside
/// [`JobV1::from_submit`](JobV1::from_submit).
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

impl JobV1 {
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

/// Public payload alias for the intent-side workload aggregate.
///
/// Per ADR-0050 the alias points at the latest payload variant —
/// today `WorkloadIntentV1`. Callers construct values via
/// `WorkloadIntent::Job(job)` / `WorkloadIntent::Service(svc)` /
/// `WorkloadIntent::Schedule(sched)` and pass the value to the
/// persistence-boundary codec ([`WorkloadIntentV1::archive_for_store`]).
pub type WorkloadIntent = WorkloadIntentV1;

/// Documentation alias for "the latest payload variant of
/// [`WorkloadIntentEnvelope`]". Mirrors the [`Job`] = [`JobV1`]
/// alias-to-payload pattern from ADR-0048 UI-02.
pub type WorkloadIntentLatest = WorkloadIntentV1;

/// Per-type rkyv versioned envelope for the intent-side workload
/// aggregate per ADR-0048 § 4 + ADR-0050 § 4.
///
/// Codec-internal — named only inside the typed
/// [`WorkloadIntentV1::archive_for_store`] / [`WorkloadIntentV1::from_store_bytes`]
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
}

/// Inner V1 payload of the intent-side workload aggregate per
/// ADR-0050 § 1. Three variants tracking the parser-side
/// [`WorkloadSpec`]: `Job` (run-to-completion), `Service`
/// (long-running supervised), `Schedule` (cron-fired Job).
///
/// rkyv archives are **fixed positional layouts** — appending a
/// variant to this enum is additive and does not shift discriminant
/// tags for existing variants per ADR-0048 § "Why a per-type rkyv
/// enum is forward-compatible". Layout-changing edits to embedded
/// per-kind payloads (e.g. adding a field to [`ServiceV1`]) require
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
pub enum WorkloadIntentV1 {
    /// Run-to-completion workload.
    Job(JobV1),
    /// Long-running supervised workload — Phase 1 minimal shape per
    /// ADR-0050 OQ-3.
    Service(ServiceV1),
    /// Cron-scheduled Job — embedded-job shape per ADR-0050 OQ-4.
    Schedule(ScheduleV1),
}

/// Phase 1 minimal `Service` payload per ADR-0050 § 2 + OQ-3.
///
/// Mirrors [`JobV1`]'s `(id, replicas, resources, driver)` shape and
/// adds `listeners`. Listener health-check policy, TLS-termination
/// config, and backend weights are deferred per OQ-3 (additive
/// envelope evolution).
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
pub struct ServiceV1 {
    pub id: WorkloadId,
    pub replicas: NonZeroU32,
    pub resources: Resources,
    pub driver: WorkloadDriver,
    /// Operator-declared listeners in declaration order. Reuses the
    /// parser-layer [`Listener`] newtype — `(port, protocol)` only.
    pub listeners: Vec<Listener>,
}

/// Phase 1 `Schedule` payload per ADR-0050 § 2 + OQ-4 (embedded
/// inner job).
///
/// The schedule's per-fire instance IS a [`JobV1`] — embedded
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
pub struct ScheduleV1 {
    pub id: WorkloadId,
    pub job: JobV1,
    pub cron_expr: CronExpr,
}

impl ServiceV1 {
    /// Validating constructor for the wire-side
    /// [`crate::api::submit::SubmitSpecInput::Service`] payload per
    /// ADR-0051 § 4. Mirrors [`JobV1::from_submit`]'s validation
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

        let crate::api::submit::ServiceSpecInput { id, replicas, resources, driver, listeners } =
            input;

        // Identity + scalar field validation — mirrors `JobV1::from_submit`.
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

        // Driver projection — same shape as `JobV1::from_submit`.
        let DriverInput::Exec(exec_input) = driver;
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
        })
    }
}

impl ScheduleV1 {
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
        reason = "RED scaffold for ScheduleV1::from_submit — lands in a future slice per ADR-0051 OQ-5"
    )]
    pub fn from_submit(
        _input: crate::api::submit::ScheduleSpecInput,
    ) -> Result<Self, AggregateError> {
        todo!(
            "RED scaffold: ScheduleV1::from_submit lands in a future slice — Schedule wire-arm wiring is intentionally deferred per ADR-0051 OQ-5"
        )
    }
}

impl VersionedEnvelope for WorkloadIntentEnvelope {
    type Latest = WorkloadIntentV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }

    fn discriminant_offset_from_end() -> Option<usize> {
        // Empirically-pinned offset is DEFERRED for `WorkloadIntentEnvelope`
        // per ADR-0050 step 02-03a — the outer envelope wraps a
        // 3-variant inner enum (`WorkloadIntentV1::{Job, Service,
        // Schedule}`) whose archived layout shifts the trailing root
        // region in ways that the JobEnvelope-style 64-byte from-end
        // pin cannot trivially adopt. Returning `None` makes the
        // pre-decode probe a no-op; unknown-future-variant bytes
        // still surface as `EnvelopeError::Malformed` via rkyv's
        // bytecheck (operator-facing remediation is the same:
        // "delete the redb file"). The structural defense against
        // future-binary surface IS preserved by the round-trip
        // golden-bytes fixture for V1 (Job / Service / Schedule);
        // the targeted `UnknownVersion` classification is the only
        // diagnostic surface that degrades. Re-pin in a follow-up
        // commit when V2 lands and the empirical offset becomes
        // worth investing in.
        None
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 carries rkyv discriminant 0; when `discriminant_offset_from_end`
        // is `None`, the probe is skipped and this slice is unused.
        &[0]
    }

    fn type_name() -> &'static str {
        "WorkloadIntentEnvelope"
    }
}

impl WorkloadIntentV1 {
    /// Archive a [`WorkloadIntent`] for persistence through the
    /// [`IntentStore`].
    ///
    /// # Postconditions
    ///
    /// On `Ok(bytes)`, `bytes` is the canonical rkyv-archived byte
    /// sequence of `WorkloadIntentEnvelope::V1(self.clone())`. Two
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

/// Input shape for `JobV1::from_submit`. The CLI deserialises TOML into this
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
/// `JobV1::from_submit` (no `From` impl: the ≥3-call-sites rule isn't met,
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
    /// trim) at `JobV1::from_submit` per ADR-0031 §4.
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
/// projection of a field already validated by `JobV1::from_submit`. Cloning
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
