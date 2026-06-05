//! `JournalStore` — runtime-owned durable journal for the `Workflow`
//! primitive's `await`-point record, per ADR-0063.
//!
//! The journal is the §18 workflow analogue of the reconciler `View`
//! store (ADR-0035): a *second redb table layout* on the SAME
//! runtime-owned redb substrate, NOT an extension of `ViewStore`. The
//! two stores share one redb file / one `Arc<Database>` / one fsync
//! ordering discipline / one Earned-Trust probe, but the access patterns
//! differ — `ViewStore` is single-blob-overwrite-per-target; the journal
//! is an append-only ordered run per `WorkflowId`. ADR-0063 §1 records
//! why this is a distinct port rather than a method on `ViewStore`.
//!
//! # Codec — CBOR (`ciborium`), NOT rkyv
//!
//! Per ADR-0063 §2 the journal is mutable, evolving, runtime-owned
//! memory — ADR-0035's codec case, not ADR-0048's content-addressed
//! case. [`JournalEntry`] is a CBOR (`ciborium`) `#[serde]` enum with
//! additive schema evolution via `#[serde(default)]`. Each await-surface
//! slice (02 `ctx.sleep`, 03 `ctx.wait_for_signal` / `ctx.emit_action`)
//! adds one entry variant additively — no version-bump, no
//! golden-fixture ceremony. Effect payloads are recorded as DIGESTS
//! (`response_digest`, …), never full bodies and never a derived
//! deadline/remaining cache (`.claude/rules/development.md` § "Persist
//! inputs, not derived state").
//!
//! # Adapters
//!
//! - **`RedbJournalStore`** (step 01-04): the production adapter over the
//!   shared redb file, one append-only table `__wf_journal__` keyed
//!   `(WorkflowId, u32)`.
//! - **`SimJournalStore`** (step 01-03, `overdrive-sim::adapters::journal`):
//!   in-memory `BTreeMap<(WorkflowId, u32), Vec<u8>>` with an injectable
//!   fsync-failure handle, mirroring `SimViewStore`.

use async_trait::async_trait;
use thiserror::Error;

/// Result alias for `JournalStore` operations — keeps call sites short
/// without forcing the long error type on every signature.
pub type Result<T, E = JournalStoreError> = std::result::Result<T, E>;

/// Identity of a single workflow *instance*. The journal is keyed by
/// `(WorkflowId, step)`; the `WorkflowId` isolates one instance's
/// append-only run within the single shared journal table (ADR-0063 §3).
///
/// Distinct from `WorkflowName` (the workflow *kind*'s identity in
/// `overdrive-core::workflow`): a `WorkflowName` names a class of
/// workflows (`provision-record`); a `WorkflowId` names one live or
/// terminated instance of it. The grammar mirrors the kebab-label shape
/// used across the codebase: `^[a-z0-9][a-z0-9-]{0,126}$` (instance ids
/// are machine-minted, so a wider interior than `WorkflowName`'s 63-char
/// cap is allowed for embedded ULIDs / hashes).
///
/// Node-independent by construction (no embedded node id), leaving room
/// for the Phase-2 cross-node resume / HA adapter (#205, ADR-0063 §5).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkflowId(String);

/// Maximum length for a workflow-instance id (1 lead + up to 126 interior).
const WORKFLOW_ID_MAX: usize = 127;

impl WorkflowId {
    /// Validating constructor.
    ///
    /// # Preconditions
    ///
    /// `raw` must be non-empty, at most [`WORKFLOW_ID_MAX`] chars, and
    /// match `^[a-z0-9][a-z0-9-]{0,126}$` (ASCII lowercase / digits /
    /// hyphen, leading char alphanumeric).
    ///
    /// # Postconditions
    ///
    /// On `Ok`, [`WorkflowId::as_str`] returns `raw` verbatim.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowIdError`] naming the first validation failure.
    pub fn new(raw: &str) -> std::result::Result<Self, WorkflowIdError> {
        if raw.is_empty() {
            return Err(WorkflowIdError::Empty);
        }
        if raw.len() > WORKFLOW_ID_MAX {
            return Err(WorkflowIdError::TooLong { max: WORKFLOW_ID_MAX });
        }
        let mut chars = raw.chars();
        let lead = chars.next().unwrap_or_else(|| {
            unreachable!("non-empty checked above guarantees at least one char")
        });
        if !(lead.is_ascii_lowercase() || lead.is_ascii_digit()) {
            return Err(WorkflowIdError::BadShape);
        }
        if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(WorkflowIdError::BadShape);
        }
        Ok(Self(raw.to_string()))
    }

    /// The canonical string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WorkflowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Validation failures for [`WorkflowId::new`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkflowIdError {
    /// The id was empty.
    #[error("workflow id must not be empty")]
    Empty,
    /// The id exceeded the length ceiling.
    #[error("workflow id too long (max {max})")]
    TooLong {
        /// The maximum permitted length.
        max: usize,
    },
    /// The id did not match `^[a-z0-9][a-z0-9-]{0,126}$`.
    #[error("workflow id must match ^[a-z0-9][a-z0-9-]{{0,126}}$")]
    BadShape,
}

/// A single `await`-point record in a workflow instance's journal.
///
/// CBOR-encoded (`ciborium`) per ADR-0063 §2. Additive schema evolution:
/// future await-surface slices add variants under `#[serde(default)]`
/// without a version-bump. The `step` field is the monotonic await-point
/// index (the journal cursor — ADR-0064 §3).
///
/// Every variant records **inputs / result digests**, never a derived
/// deadline or "remaining" cache (`.claude/rules/development.md`
/// § "Persist inputs, not derived state"). Slice 02's `SleepArmed` will
/// record the absolute `deadline` (an input), not the remaining wait;
/// resume recomputes remaining from `deadline − clock.now()`.
///
/// `#[serde(tag = "v")]`-style versioned-envelope migration is reserved
/// for the first breaking change; slice 01 has no breaking history.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum JournalEntry {
    /// The workflow instance started (slice 01). Records the spec digest
    /// (the workflow *kind* + parameters identity) and the input digest
    /// (the instance's start parameters) — both inputs.
    Started {
        /// SHA-256 digest of the workflow spec's canonical identity.
        spec_digest: overdrive_core::id::ContentHash,
        /// SHA-256 digest of the instance's start input.
        input_digest: overdrive_core::id::ContentHash,
    },

    /// An external `ctx.call` resolved (slice 01). Records the
    /// await-point step index, the correlation key linking cause to
    /// response across replays, and the response digest — the inputs
    /// replay-equivalence (K4) re-derives and compares against.
    CallResult {
        /// The monotonic await-point index (journal cursor).
        step: u32,
        /// Correlation key derived from `(target, spec_hash, purpose)`,
        /// deterministic across attempts (ADR-0035 § Reconciler I/O
        /// rule 2). Recorded as the canonical string form.
        correlation: String,
        /// SHA-256 digest of the call's response — sufficient for
        /// replay-equivalence; the full body lives in the
        /// `external_call_results` observation row.
        response_digest: overdrive_core::id::ContentHash,
    },

    /// The workflow ran to a terminal value (slice 01). Records the
    /// terminal result string form — the engine maps this back to a
    /// `WorkflowResult` on read.
    Terminal {
        /// Operator-facing terminal result (canonical string form of
        /// the `WorkflowResult` the engine returned).
        result: String,
    },
}

/// Errors from a `JournalStore` operation. Pass-through embedding via
/// `#[from]` per `.claude/rules/development.md` § Errors — mirrors
/// [`crate::view_store::ViewStoreError`].
#[derive(Debug, Error)]
pub enum JournalStoreError {
    /// CBOR encode failure — the [`JournalEntry`] could not be
    /// serialised. Should not happen for the straightforward derive;
    /// surfaces on exotic custom impls only.
    #[error("CBOR encode failed: {0}")]
    Encode(String),

    /// CBOR decode failure — a persisted entry could not be decoded.
    /// Indicates schema skew between the in-memory [`JournalEntry`]
    /// shape and the on-disk bytes; the runtime surfaces this as a hard
    /// boot failure (Earned-Trust gate).
    #[error("CBOR decode failed: {0}")]
    Decode(String),

    /// The underlying durable append completed the write but the fsync
    /// syscall failed. Per ADR-0063 §4 (reusing ADR-0035 §6
    /// `WriteThroughOrdering`): when this fires the entry MUST NOT be
    /// observable — neither persisted on disk nor visible to a
    /// subsequent `load_journal`.
    #[error("fsync failed: {message}")]
    FsyncFailed {
        /// Cause string from the underlying engine (or sim injection).
        message: String,
    },

    /// Underlying I/O error from the storage engine.
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors from the Earned-Trust startup probe per ADR-0063 §4 (reused
/// verbatim from ADR-0035). Mirrors [`crate::view_store::ProbeError`].
///
/// The probe writes a sentinel entry, fsyncs, reads it back byte-equal,
/// and deletes it. Any variant short-circuits boot with a
/// `health.startup.refused` event.
#[derive(Debug, Error)]
pub enum ProbeError {
    /// The probe could not write its sentinel entry. Typical causes: a
    /// read-only filesystem, missing parent directory, or a sim adapter
    /// with `inject_fsync_failure()` set.
    #[error("probe write failed: {source}")]
    WriteFailed {
        /// Underlying `JournalStoreError` cause.
        #[source]
        source: JournalStoreError,
    },

    /// The probe wrote and fsynced its sentinel entry but the durable
    /// commit reported failure (disk full, checksum mismatch on
    /// readback, atomic-rename failure).
    #[error("probe commit failed: {source}")]
    CommitFailed {
        /// Underlying `JournalStoreError` cause.
        #[source]
        source: JournalStoreError,
    },

    /// The probe could not read back its sentinel entry because the
    /// underlying read transaction failed. Distinct from
    /// `RoundTripMismatch`, which fires only after a successful read
    /// returns non-byte-equal bytes.
    #[error("probe read-back failed: {source}")]
    ReadFailed {
        /// Underlying `JournalStoreError` cause.
        #[source]
        source: JournalStoreError,
    },

    /// The probe wrote a sentinel entry, fsynced, and read back a
    /// non-byte-equal value. Indicates engine corruption between write
    /// and read — reject startup rather than operate against a
    /// corrupted store.
    #[error("probe round-trip mismatch: wrote {wrote:?}, read {got:?}")]
    RoundTripMismatch {
        /// Bytes written (probe sentinel).
        wrote: Vec<u8>,
        /// Bytes read back from the store.
        got: Vec<u8>,
    },

    /// The probe wrote and read back successfully but could not delete
    /// its sentinel entry. Surfacing this prevents a store that "works
    /// for writes but rejects deletes" from being brought online
    /// silently.
    #[error("probe cleanup failed: {source}")]
    CleanupFailed {
        /// Underlying `JournalStoreError` cause.
        #[source]
        source: JournalStoreError,
    },
}

/// Runtime-owned durable journal for workflow `await`-point records.
///
/// The driving port for the workflow engine's durable memory. The engine
/// (step 01-05) calls [`append`](JournalStore::append) before suspending
/// each await-point and [`load_journal`](JournalStore::load_journal) on
/// resume to replay the recorded run. [`probe`](JournalStore::probe)
/// runs once at boot (Earned Trust).
///
/// **Append-only ordered run per instance** — entries are never
/// overwritten; `(workflow_id, step)` is unique per append. This is the
/// structural difference from `ViewStore`'s single-blob-overwrite
/// contract that makes the journal a distinct port (ADR-0063 §1).
///
/// Encodes/decodes [`JournalEntry`] via `ciborium` internally — the
/// trait surface takes/returns typed `JournalEntry`, not raw bytes,
/// because (unlike `ViewStore`'s heterogeneous-`View` problem) the
/// journal stores one homogeneous entry type, so no dyn-compat byte
/// indirection is needed.
#[async_trait]
pub trait JournalStore: Send + Sync {
    /// Append one [`JournalEntry`] to `workflow_id`'s run at the entry's
    /// implied next step, durably (one fsync'd write) BEFORE return.
    ///
    /// # Preconditions
    ///
    /// `workflow_id` is a valid instance id; `entry` is a well-formed
    /// [`JournalEntry`].
    ///
    /// # Postconditions
    ///
    /// On `Ok(())` the entry is durable and a subsequent
    /// [`load_journal`](Self::load_journal) for the same `workflow_id`
    /// returns it appended at the END of the ordered run (append
    /// order == load order). Per ADR-0063 §4 (fsync-then-memory): the
    /// fsync completes before this returns, so a crash after `Ok(())`
    /// preserves the entry across the next boot's `load_journal`.
    ///
    /// # Edge cases
    ///
    /// - First append for a fresh `workflow_id` creates that instance's
    ///   run (no prior `Started` required by the store; ordering is the
    ///   engine's concern).
    /// - The store assigns the step index by append position; appending
    ///   N entries yields steps `0..N` in append order.
    ///
    /// # Errors
    ///
    /// - [`JournalStoreError::FsyncFailed`] — sim injection or real
    ///   fsync error. Per ADR-0063 §4 the entry MUST NOT be observable
    ///   when this fires (not persisted, not returned by a later
    ///   `load_journal`).
    /// - [`JournalStoreError::Encode`] — the entry could not be
    ///   CBOR-encoded.
    /// - [`JournalStoreError::Io`] — underlying engine I/O failure.
    async fn append(&self, workflow_id: &WorkflowId, entry: &JournalEntry) -> Result<()>;

    /// Load the full ordered run for `workflow_id` — a range scan
    /// `(id, 0)..=(id, u32::MAX)` decoded into a `Vec<JournalEntry>` in
    /// step order (ADR-0063 §3).
    ///
    /// # Postconditions
    ///
    /// Returns the entries previously [`append`](Self::append)ed for
    /// `workflow_id`, byte-equal after the CBOR round-trip, in append
    /// (== ascending step) order.
    ///
    /// # Edge cases
    ///
    /// Returns an empty `Vec` for a `workflow_id` with no appended
    /// entries (unknown / fresh instance) — never an error. This is the
    /// common case for an instance that has not started.
    ///
    /// # Errors
    ///
    /// - [`JournalStoreError::Decode`] — a persisted entry could not be
    ///   CBOR-decoded (schema skew); the runtime treats this as a hard
    ///   boot failure.
    /// - [`JournalStoreError::Io`] — underlying engine I/O failure.
    async fn load_journal(&self, workflow_id: &WorkflowId) -> Result<Vec<JournalEntry>>;

    /// Earned-Trust startup probe per ADR-0063 §4 (reused from
    /// ADR-0035 § Earned Trust).
    ///
    /// Composition root invariant: write a sentinel entry → fsync →
    /// read it back byte-equal → delete the sentinel. Called once at
    /// boot before the first workflow starts; on any failure the runtime
    /// emits `health.startup.refused` and exits non-zero.
    ///
    /// # Postconditions
    ///
    /// On `Ok(())` the store contains no probe-entry residue and is
    /// proven writable/readable/deletable.
    ///
    /// # Errors
    ///
    /// Returns a [`ProbeError`] variant naming which stage of the
    /// write → fsync → readback → delete handshake failed.
    async fn probe(&self) -> std::result::Result<(), ProbeError>;
}
