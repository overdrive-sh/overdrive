//! `JournalStore` — runtime-owned durable journal for the `Workflow`
//! primitive's `await`-point record, per ADR-0066.
//!
//! The journal is the §18 workflow analogue of the reconciler `View`
//! store (ADR-0035): a *second redb table layout* on the SAME
//! runtime-owned redb substrate, NOT an extension of `ViewStore`. The
//! two stores share one redb file / one `Arc<Database>` / one fsync
//! ordering discipline / one Earned-Trust probe, but the access patterns
//! differ — `ViewStore` is single-blob-overwrite-per-target; the journal
//! is an append-only ordered run per `WorkflowId`. ADR-0066 §1 records
//! why this is a distinct port rather than a method on `ViewStore`.
//!
//! # Codec — CBOR (`ciborium`), NOT rkyv
//!
//! Per ADR-0066 §2 the journal is mutable, evolving, runtime-owned
//! memory — ADR-0035's codec case, not ADR-0048's content-addressed
//! case. [`LoadedEntry`] (and its two leaf enums [`JournalCommand`] /
//! [`JournalNotification`]) are CBOR (`ciborium`) `#[serde]` types with
//! additive schema evolution via `#[serde(default)]`. Each await-surface
//! slice (02 `ctx.sleep`, 03 `ctx.wait_for_signal` / `ctx.emit_action`)
//! adds one variant additively — no version-bump, no golden-fixture
//! ceremony, **no `#[serde(tag = "v")]` envelope** (greenfield
//! single-cut; no surviving on-disk journals). Step results are recorded
//! as their CBOR-encoded bytes plus a `result_digest` (the `ctx.run`
//! durable-step result), never a derived deadline/remaining cache
//! (`.claude/rules/development.md` § "Persist inputs, not derived state").
//!
//! # The typed command/notification split (ADR-0066 §2 / ADR-0064 §3)
//!
//! The journal stream is **typed by replay role**, closing the latent
//! replay-corruption trap where `Started`/`Terminal` were second-class
//! under a positional walk and a variant mismatch at the cursor silently
//! fell through to the live path:
//!
//! - [`JournalCommand`] — the replayable, **cursor-advancing** class.
//!   Every command occupies one position in the positional replay walk;
//!   identity is positional (a command's index in the command sequence
//!   IS its replay identity — no persisted `step` field).
//! - [`JournalNotification`] — the `SignalKey`-correlated class, resolved
//!   by lookup and **never advancing the cursor** (its sole variant is
//!   `SignalSeen`).
//! - [`LoadedEntry`] — the on-disk/append/load boundary sum. Commands and
//!   notifications interleave in one ordered table; the store is a dumb
//!   ordered log and never classifies. The cursor partitions once at
//!   construction (D2; step 01-03).
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

pub mod redb;

pub use redb::RedbJournalStore;

/// Result alias for `JournalStore` operations — keeps call sites short
/// without forcing the long error type on every signature.
pub type Result<T, E = JournalStoreError> = std::result::Result<T, E>;

/// Identity of a single workflow *instance*. The journal is keyed by
/// `(WorkflowId, step)`; the `WorkflowId` isolates one instance's
/// append-only run within the single shared journal table (ADR-0066 §3).
///
/// Distinct from `WorkflowName` (the workflow *kind*'s identity in
/// `overdrive-core::workflow`): a `WorkflowName` names a class of
/// workflows (`provision-record`); a `WorkflowId` names one live or
/// terminated instance of it. The grammar mirrors the kebab-label shape
/// used across the codebase: `^[a-z0-9][a-z0-9-]{0,255}$` (instance ids
/// are machine-minted, so a wider interior than `WorkflowName`'s 63-char
/// cap is allowed for embedded ULIDs / hashes, and the ceiling holds the
/// full `wf-`-prefixed mapping of a maximum-length `CorrelationKey`
/// without truncation — see [`WORKFLOW_ID_MAX`]).
///
/// Node-independent by construction (no embedded node id), leaving room
/// for the Phase-2 cross-node resume / HA adapter (#205, ADR-0066 §5).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkflowId(String);

/// The stable prefix every derived instance id carries. Guarantees a
/// grammar-valid leading char (and a non-empty body) regardless of what
/// the source correlation key began with.
const WF_PREFIX: &str = "wf-";

/// Maximum length for a workflow-instance id.
///
/// Sized off the single shared label ceiling
/// ([`overdrive_core::id::LABEL_MAX`], 253 — the DNS-name maximum) plus
/// room for the [`WF_PREFIX`]. A `WorkflowId` is a label *derived from* a
/// [`CorrelationKey`] (also bounded by `LABEL_MAX`), and
/// [`WorkflowId::for_correlation`] prepends `wf-`; the ceiling must
/// therefore be `LABEL_MAX + "wf-".len()` so that mapping a maximum-length
/// correlation key NEVER truncates. Truncation here previously collapsed
/// two distinct correlation keys (sharing a truncated prefix but differing
/// in the dropped suffix) onto ONE id, opening the wrong instance's
/// journal — see `.claude/rules/development.md` § "One shared length
/// ceiling for label-shaped ids".
const WORKFLOW_ID_MAX: usize = overdrive_core::id::LABEL_MAX + WF_PREFIX.len();

impl WorkflowId {
    /// Validating constructor.
    ///
    /// # Preconditions
    ///
    /// `raw` must be non-empty, at most [`WORKFLOW_ID_MAX`] chars, and
    /// match `^[a-z0-9][a-z0-9-]{0,255}$` (ASCII lowercase / digits /
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

    /// Derive a deterministic, valid [`WorkflowId`] for a workflow
    /// instance from its [`CorrelationKey`].
    ///
    /// The engine keys an instance's journal by `WorkflowId`; the
    /// workflow-lifecycle reconciler keys instances by `CorrelationKey`.
    /// This is the deterministic bridge between the two: the action-shim
    /// `StartWorkflow` arm derives the instance journal id from the
    /// action's correlation, so the SAME instance always resolves to the
    /// SAME journal id (crash-resume re-derives it identically — ADR-0064
    /// §5 / `development.md` Reconciler I/O rule 2: correlation links
    /// cause to response across attempts).
    ///
    /// The correlation key's canonical form (`target:purpose/<hex>`)
    /// carries `:` and `/`, which the `WorkflowId` grammar
    /// (`^[a-z0-9][a-z0-9-]{0,255}$`) rejects. The derivation maps every
    /// char outside `[a-z0-9-]` to `-`, lowercases ASCII uppercase, and
    /// prefixes a stable [`WF_PREFIX`] so the leading-char rule holds even
    /// if the correlation began with a now-mapped char. The mapping is
    /// total and deterministic — equal correlations always yield equal ids.
    ///
    /// **No truncation.** [`WORKFLOW_ID_MAX`] is sized as
    /// `LABEL_MAX + WF_PREFIX.len()`, and a [`CorrelationKey`] is itself
    /// bounded by `LABEL_MAX`; the mapped result (one output char per input
    /// char, plus the prefix) therefore always fits. The discriminating
    /// content-addressed suffix at the *end* of the key always survives.
    /// This is load-bearing: a smaller ceiling would truncate that suffix
    /// and collapse two distinct correlation keys (sharing a truncated
    /// prefix) onto one journal id — the bug this sizing closes. The
    /// length guard below is retained as a defensive invariant but is
    /// unreachable for any valid `CorrelationKey`.
    #[must_use]
    pub fn for_correlation(correlation: &overdrive_core::id::CorrelationKey) -> Self {
        let mut id = String::with_capacity(WORKFLOW_ID_MAX);
        id.push_str(WF_PREFIX);
        for c in correlation.as_str().chars() {
            let mapped = if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else {
                '-'
            };
            // Unreachable for any valid CorrelationKey (bounded by
            // LABEL_MAX, and WORKFLOW_ID_MAX = LABEL_MAX + WF_PREFIX.len()):
            // kept only as a defensive invariant against a future
            // grammar/ceiling drift, NOT as a routine truncation path.
            if id.len() >= WORKFLOW_ID_MAX {
                break;
            }
            id.push(mapped);
        }
        // The WF_PREFIX guarantees a valid leading char and a non-empty
        // body; every interior char is in `[a-z0-9-]` by the mapping
        // above; length is bounded by WORKFLOW_ID_MAX (= LABEL_MAX +
        // WF_PREFIX.len(), which holds the full mapped key). The grammar
        // therefore cannot reject the result.
        #[allow(clippy::expect_used)]
        Self::new(&id).expect("WorkflowId::for_correlation produces a grammar-valid id")
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
    /// The id did not match `^[a-z0-9][a-z0-9-]{0,255}$`.
    #[error("workflow id must match ^[a-z0-9][a-z0-9-]{{0,255}}$")]
    BadShape,
}

/// A replayable, **cursor-advancing** journal command (ADR-0066 §2 /
/// ADR-0064 §3, D1).
///
/// A command advances the cursor; identity is positional — a command's
/// index in the partitioned `Vec<JournalCommand>` IS its replay identity.
/// There is deliberately **no in-entry `step` field**: a persisted `step`
/// would be a derived cache of "my own position"
/// (`.claude/rules/development.md` § "Persist inputs, not derived state",
/// D5). The store counts entries for `next_step`; the cursor derives the
/// command-index from partition position (step 01-03).
///
/// # Variants and ordering
///
/// `Started` is command-index 0 (the engine writes it on first start —
/// step 01-05); subsequent `await`-points occupy ascending command
/// indices; `Terminal` is the last command. `Started`/`Terminal` are
/// **first-class commands** here — the trap closed by this split was that
/// they were second-class under the old positional walk that could only
/// consume `await`-point entries.
///
/// # Codec
///
/// CBOR-encoded (`ciborium`). Additive schema evolution: future
/// await-surface slices add variants under `#[serde(default)]` without a
/// version-bump. Every variant records **inputs / result digests**, never
/// a derived deadline or "remaining" cache.
///
/// # Edge cases / invariants
///
/// - A command always advances the cursor by exactly 1 on replay.
/// - `SleepArmed` records the ABSOLUTE `deadline_unix` (an input); resume
///   recomputes remaining from `deadline_unix − clock.unix_now()`, never
///   a persisted "remaining" field.
/// - A `SignalAwaited` command with no matching `SignalSeen` notification
///   (looked up off the walk) is the "crashed while still blocked" shape:
///   resume re-blocks on the same key.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum JournalCommand {
    /// The workflow instance started (slice 01) — command-index 0.
    /// Records the spec digest (the workflow *kind* + parameters
    /// identity) and the input digest (the instance's start parameters) —
    /// both inputs.
    Started {
        /// SHA-256 digest of the workflow spec's canonical identity.
        spec_digest: overdrive_core::id::ContentHash,
        /// SHA-256 digest of the instance's start input.
        input_digest: overdrive_core::id::ContentHash,
    },

    /// A `ctx.run` durable step resolved (slice 01). Records the step name
    /// (diagnostics + the replay-determinism check), the result digest,
    /// and the CBOR-encoded step result bytes the replay path decodes —
    /// the inputs replay-equivalence (K4) re-derives and compares against.
    ///
    /// Identity is POSITIONAL (the command-index); `name` is carried for
    /// diagnostics and the Layer-2 determinism check, not for identity.
    RunResult {
        /// The `ctx.run` step name — diagnostics + the replay-determinism
        /// check (a recorded name diverging from the replaying body's name
        /// at this command-index fails closed; ADR-0064 §3).
        name: String,
        /// SHA-256 digest of the step's CBOR-encoded result — sufficient
        /// for replay-equivalence (K4); the digest of `result_bytes`.
        result_digest: overdrive_core::id::ContentHash,
        /// The CBOR-encoded `ctx.run` result. Carried so a resumed run
        /// replays a byte-equal result without re-polling the step's
        /// future (ADR-0064 §3, exactly-once on the replay path).
        result_bytes: Vec<u8>,
    },

    /// A `ctx.sleep` was armed (slice 02). Records the ABSOLUTE
    /// wall-clock `deadline_unix` (an INPUT — `clock.unix_now()` at arm
    /// time + the sleep duration). Resume recomputes the remaining wait as
    /// `deadline_unix − clock.unix_now()` — there is deliberately NO
    /// persisted "remaining duration" field
    /// (`.claude/rules/development.md` § "Persist inputs, not derived
    /// state"; a remaining cache would silently desync from the live clock
    /// on resume).
    ///
    /// Additive `#[serde(default)]` per ADR-0066 §2.
    SleepArmed {
        /// Absolute wall-clock deadline (duration since the UNIX epoch)
        /// computed at arm time — an input, not a derived remaining cache.
        deadline_unix: std::time::Duration,
    },

    /// A `ctx.wait_for_signal` was armed (slice 03). Records the
    /// [`SignalKey`](overdrive_core::workflow::SignalKey) the workflow
    /// blocked on (an INPUT — the key the body named). A `SignalAwaited`
    /// command with no matching `SignalSeen` notification is the "crashed
    /// while still blocked" shape: resume re-blocks on the SAME key (the
    /// crash-safety contract proven by step 03-02).
    ///
    /// Additive `#[serde(default)]` per ADR-0066 §2.
    SignalAwaited {
        /// The signal key the workflow body blocked on — an input.
        signal_key: overdrive_core::workflow::SignalKey,
    },

    /// A `ctx.emit_action` sent a typed Action on the Action channel
    /// (slice 03). Records the `action_digest` — the content digest of the
    /// emitted Action's inputs (per `.claude/rules/development.md`
    /// § "Persist inputs, not derived state"). The presence of this
    /// command at the cursor makes the emit idempotent on resume: a
    /// resumed run sees `ActionEmitted` and does NOT re-send the Action
    /// (exactly-once *on the replay path*). This is NOT an unconditional
    /// exactly-once guarantee — the live emit is send-before-record, so a
    /// crash AFTER the send but BEFORE this command is journaled leaves no
    /// `ActionEmitted` at the cursor and the resume re-sends
    /// (at-least-once; safety rests on downstream idempotency). See
    /// `WorkflowCtx::emit_action` "Honest semantics". ADR-0064 §4.
    ///
    /// Additive `#[serde(default)]` per ADR-0066 §2.
    ActionEmitted {
        /// SHA-256 digest of the emitted Action's canonical inputs.
        action_digest: overdrive_core::id::ContentHash,
    },

    /// The engine recorded a retry ATTEMPT after absorbing a transient
    /// (retryable) failure and before re-driving the body (slice 04,
    /// ADR-0065 §4). Records the `attempt_digest` — the content digest of
    /// the attempt's INPUTS (per `.claude/rules/development.md` § "Persist
    /// inputs, not derived state"). The journal is the single durable SSOT
    /// for the instance's retry state: the engine recomputes `attempts` (and
    /// the next backoff window) from the COUNT of these commands against the
    /// live `WORKFLOW_RETRY_BUDGET` policy on each re-drive — never a
    /// persisted attempt-count or deadline cache. Once the count reaches the
    /// budget the engine mints
    /// [`TerminalError::budget_exhausted`](overdrive_core::workflow::TerminalError::budget_exhausted)
    /// → [`WorkflowStatus::Failed`](overdrive_core::workflow::WorkflowStatus).
    ///
    /// Additive `#[serde(default)]` per ADR-0066 §2 — an older journal
    /// without this variant decodes cleanly (the variant is simply absent;
    /// the additive-variant tolerance the codec provides). NOT cursor
    /// identity — like every other command it advances the positional walk
    /// by exactly 1 on replay; no in-entry `step` (D5).
    ///
    /// # Gap 2 — the retry window's start instant (`started_at_unix`)
    ///
    /// The per-step [`RunRetryPolicy`](overdrive_core::workflow::RunRetryPolicy)
    /// `max_duration` gate (ADR-0065 Gap 2) needs the wall-clock instant the
    /// retry window OPENED, so elapsed can be recomputed against the live clock
    /// on every drive (and across crash-resume). The FIRST `RetryAttempted` for
    /// an instance carries `started_at_unix: Some(clock.unix_now())`; every
    /// subsequent one carries `None`. The engine recovers the window start by
    /// scanning the loaded run for the first `RetryAttempted` carrying `Some`,
    /// then gates a re-drive on BOTH `attempts < max_attempts` AND
    /// `elapsed < max_duration`. Per `.claude/rules/development.md` § "Persist
    /// inputs, not derived state" the START instant is journaled (an input);
    /// the elapsed window and the deadline are RECOMPUTED each drive, never
    /// persisted. The field is `Option` + additive `#[serde(default)]` so an
    /// older journal (or a non-first attempt) decodes to `None` cleanly,
    /// mirroring `SleepArmed.deadline_unix`'s absolute-wall-clock shape.
    RetryAttempted {
        /// SHA-256 digest of the retry attempt's canonical inputs — an
        /// input, not a derived attempt-count cache.
        attempt_digest: overdrive_core::id::ContentHash,
        /// The absolute wall-clock instant (duration since the UNIX epoch) the
        /// retry window OPENED — `Some` on the FIRST `RetryAttempted` for the
        /// instance, `None` thereafter (ADR-0065 Gap 2). An input the engine
        /// recovers to recompute `elapsed` for the `max_duration` gate; never a
        /// derived deadline cache. Additive `#[serde(default)]` → an older
        /// journal lacking the field decodes to `None`.
        #[serde(default)]
        started_at_unix: Option<std::time::Duration>,
    },

    /// The workflow ran to a terminal value (slice 01) — the last command.
    /// Records the FULL [`WorkflowStatus`](overdrive_core::workflow::WorkflowStatus)
    /// the engine projected (not a lossy string label), so a resumed run
    /// reads back the exact terminal status — including a `Failed`'s
    /// structured [`TerminalError`](overdrive_core::workflow::TerminalError)
    /// (kind + detail) — and can re-publish the terminal observation row
    /// losslessly without re-running the author body
    /// (`docs/feature/fix-workflow-terminal-redrive/deliver/rca.md`,
    /// Option 1; `.claude/rules/development.md` § "Persist inputs, not
    /// derived state"). The structured `TerminalError` (vs the old free-text
    /// reason) closes the ADR-0064 §3 replay-determinism hazard.
    Terminal {
        /// The workflow instance's full terminal status.
        status: overdrive_core::workflow::WorkflowStatus,
    },
}

/// A `SignalKey`-correlated journal notification (ADR-0066 §2 /
/// ADR-0064 §4, D1).
///
/// A notification is resolved by `SignalKey` lookup and **never advances
/// the cursor** — it lives off the positional command walk. Its sole
/// variant is `SignalSeen`: the satisfied half of a `ctx.wait_for_signal`,
/// paired by `SignalKey` with the `SignalAwaited` command. The cursor
/// partitions the loaded run into the command sequence plus a
/// `BTreeMap<SignalKey, JournalNotification>` once at construction (D2;
/// step 01-03); a `SignalAwaited` command with no matching notification
/// re-blocks on resume.
///
/// Deliberately minimal (D6): a single notification shape, no general
/// Restate-style `NotificationId` correlation model — single-node Phase 1
/// has exactly one notification kind, and the general model is rejected,
/// not deferred.
///
/// CBOR-encoded (`ciborium`); additive `#[serde(default)]` evolution per
/// ADR-0066 §2. No in-entry `step` field — identity is the `SignalKey`,
/// never a position (D5).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum JournalNotification {
    /// A `ctx.wait_for_signal` was satisfied (slice 03). Records the
    /// [`SignalKey`](overdrive_core::workflow::SignalKey), the
    /// `value_digest` (the content digest of the observed
    /// [`SignalValue`](overdrive_core::workflow::SignalValue)'s bytes — an
    /// INPUT, per `.claude/rules/development.md` § "Persist inputs, not
    /// derived state"), and the observed `value` itself, carried so a
    /// resumed run replays the exact value the live run received without
    /// re-reading the signal surface (ADR-0064 §4, exactly-once on the
    /// replay path).
    SignalSeen {
        /// The signal key that was satisfied — an input. The correlation
        /// key the cursor looks this notification up by.
        signal_key: overdrive_core::workflow::SignalKey,
        /// SHA-256 digest of the observed signal value's bytes — the
        /// input replay-equivalence (K4) re-derives and compares.
        value_digest: overdrive_core::id::ContentHash,
        /// The observed signal value, carried so a resumed run replays the
        /// exact value the live run received without re-reading the signal
        /// surface (ADR-0064 §4).
        value: overdrive_core::workflow::SignalValue,
    },
}

/// The on-disk/append/load boundary representation — the dumb-store
/// ordered-table shape (ADR-0066 §2 / ADR-0064 §3, D1).
///
/// Commands and notifications **interleave in one ordered table**; the
/// store ([`JournalStore`]) is a dumb ordered log and never classifies.
/// [`JournalStore::append`] takes a `LoadedEntry`;
/// [`JournalStore::load_journal`] returns the flat ordered
/// `Vec<LoadedEntry>` in append order. The cursor partitions this sum
/// ONCE at construction into the positional command walk plus the
/// `SignalKey`-keyed notification lookup (D2; step 01-03) — classification
/// is the cursor's job, not the store's.
///
/// `LoadedEntry` is the one genuinely-new type in the split: a thin
/// boundary sum over the two existing-derived leaf enums, not a new
/// component. CBOR-encoded (`ciborium`), no `#[serde(tag = "v")]` envelope
/// bump (greenfield single-cut).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LoadedEntry {
    /// A replayable, cursor-advancing [`JournalCommand`].
    Command(JournalCommand),
    /// A `SignalKey`-correlated, off-the-walk [`JournalNotification`].
    Notification(JournalNotification),
}

/// Errors from a `JournalStore` operation. Pass-through embedding via
/// `#[from]` per `.claude/rules/development.md` § Errors — mirrors
/// [`crate::view_store::ViewStoreError`].
#[derive(Debug, Error)]
pub enum JournalStoreError {
    /// CBOR encode failure — the [`LoadedEntry`] could not be
    /// serialised. Should not happen for the straightforward derive;
    /// surfaces on exotic custom impls only.
    #[error("CBOR encode failed: {0}")]
    Encode(String),

    /// CBOR decode failure — a persisted entry could not be decoded.
    /// Indicates schema skew between the in-memory [`LoadedEntry`]
    /// shape and the on-disk bytes; the runtime surfaces this as a hard
    /// boot failure (Earned-Trust gate).
    #[error("CBOR decode failed: {0}")]
    Decode(String),

    /// The underlying durable append completed the write but the fsync
    /// syscall failed. Per ADR-0066 §4 (reusing ADR-0035 §6
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

/// Errors from the Earned-Trust startup probe per ADR-0066 §4 (reused
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
/// contract that makes the journal a distinct port (ADR-0066 §1).
///
/// Encodes/decodes [`LoadedEntry`] via `ciborium` internally — the trait
/// surface takes/returns typed `LoadedEntry`, not raw bytes, because
/// (unlike `ViewStore`'s heterogeneous-`View` problem) the journal stores
/// one homogeneous boundary sum, so no dyn-compat byte indirection is
/// needed. **The store is a dumb ordered log** — commands and
/// notifications interleave; the store never classifies (D2; the cursor
/// partitions at construction, step 01-03).
#[async_trait]
pub trait JournalStore: Send + Sync {
    /// Append one [`LoadedEntry`] to `workflow_id`'s run at the next
    /// storage append-position, durably (one fsync'd write) BEFORE return.
    ///
    /// # Preconditions
    ///
    /// `workflow_id` is a valid instance id; `entry` is a well-formed
    /// [`LoadedEntry`] (a command or a notification — the store does not
    /// distinguish them).
    ///
    /// # Postconditions
    ///
    /// On `Ok(())` the entry is durable and a subsequent
    /// [`load_journal`](Self::load_journal) for the same `workflow_id`
    /// returns it appended at the END of the ordered run (append
    /// order == load order). A notification and a command append
    /// identically — the store assigns the `u32` by append position over
    /// ALL entries, never by class. Per ADR-0066 §4 (fsync-then-memory):
    /// the fsync completes before this returns, so a crash after `Ok(())`
    /// preserves the entry across the next boot's `load_journal`.
    ///
    /// # Edge cases
    ///
    /// - First append for a fresh `workflow_id` creates that instance's
    ///   run (no prior `Started` required by the store; ordering is the
    ///   engine's concern).
    /// - The store assigns the step index by append position over ALL
    ///   entries (commands + notifications); appending N entries yields
    ///   steps `0..N` in append order. The adapter's `next_step` count is
    ///   count-ALL: it counts every entry regardless of class, so a
    ///   notification advances the storage append-position exactly as a
    ///   command does.
    ///
    /// # Invariants
    ///
    /// - **Append order == load order.** Entries are returned by
    ///   [`load_journal`](Self::load_journal) in the order they were
    ///   appended; the store never reorders.
    /// - **Storage append-position is NOT the command-index.** The `u32`
    ///   the store assigns is the position over ALL entries (commands AND
    ///   notifications interleaved). The replay command-index — a command's
    ///   identity in the positional replay walk — is derived by the cursor
    ///   AFTER it partitions the run (D3; step 01-03), by counting ONLY the
    ///   `Command(_)` entries that precede it. The store does NOT compute,
    ///   persist, or expose the command-index; classification is the
    ///   cursor's job, not the store's (D2). A future HA `JournalStore`
    ///   adapter (#205) re-implements this dumb ordered log over a
    ///   different substrate WITHOUT re-deriving replay semantics: it owns
    ///   only "append at the next position, load in order"; the
    ///   append-position-vs-command-index distinction stays at the cursor.
    ///
    /// # Errors
    ///
    /// - [`JournalStoreError::FsyncFailed`] — sim injection or real
    ///   fsync error. Per ADR-0066 §4 the entry MUST NOT be observable
    ///   when this fires (not persisted, not returned by a later
    ///   `load_journal`).
    /// - [`JournalStoreError::Encode`] — the entry could not be
    ///   CBOR-encoded.
    /// - [`JournalStoreError::Io`] — underlying engine I/O failure.
    async fn append(&self, workflow_id: &WorkflowId, entry: &LoadedEntry) -> Result<()>;

    /// Load the full ordered run for `workflow_id` — a range scan
    /// `(id, 0)..=(id, u32::MAX)` decoded into a flat `Vec<LoadedEntry>`
    /// in append order (ADR-0066 §3).
    ///
    /// # Postconditions
    ///
    /// Returns the entries previously [`append`](Self::append)ed for
    /// `workflow_id`, byte-equal after the CBOR round-trip, in append
    /// (== ascending storage-step) order. The store does NOT partition
    /// the run into commands and notifications — that is the cursor's job
    /// (D2). Commands and notifications are returned interleaved exactly
    /// as they were appended.
    ///
    /// # Edge cases
    ///
    /// Returns an empty `Vec` for a `workflow_id` with no appended
    /// entries (unknown / fresh instance) — never an error. This is the
    /// common case for an instance that has not started.
    ///
    /// # Invariants
    ///
    /// - **Dumb ordered log (D2).** The returned `Vec` is the flat run as
    ///   appended — commands and notifications interleaved, in
    ///   append-position order. The store never partitions the run into a
    ///   command sequence + a `SignalKey`-keyed notification lookup; the
    ///   cursor does that ONCE at construction (step 01-03).
    /// - **Append-position, not command-index.** The Vec index of an entry
    ///   is its storage append-position over ALL entries, NOT its replay
    ///   command-index. The cursor derives the command-index by counting
    ///   only the preceding `Command(_)` entries after it partitions (D3) —
    ///   a future HA adapter (#205) re-implements the load over a different
    ///   substrate without re-deriving that replay semantics; the store
    ///   owes only "hand back the verbatim ordered run."
    ///
    /// # Errors
    ///
    /// - [`JournalStoreError::Decode`] — a persisted entry could not be
    ///   CBOR-decoded (schema skew); the runtime treats this as a hard
    ///   boot failure.
    /// - [`JournalStoreError::Io`] — underlying engine I/O failure.
    async fn load_journal(&self, workflow_id: &WorkflowId) -> Result<Vec<LoadedEntry>>;

    /// Earned-Trust startup probe per ADR-0066 §4 (reused from
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

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::{JournalCommand, LoadedEntry, WorkflowId};
    use overdrive_core::id::{ContentHash, CorrelationKey};
    use overdrive_core::workflow::{TerminalError, TerminalErrorKind, WorkflowStatus};

    /// The journal `Terminal` command now carries the FULL `WorkflowStatus`
    /// (fix-workflow-terminal-redrive, RCA Option 1; ADR-0065 §3) — not a
    /// lossy string label. A CBOR (`ciborium`) round-trip of `Terminal {
    /// status: Failed { terminal } }` must preserve the structured
    /// `TerminalError` (kind + detail) exactly: this is the lossless-terminal
    /// guarantee the start-time short-circuit relies on to re-publish the
    /// terminal observation row without re-running the body.
    ///
    /// The OLD `result: String` label folded the failure to the constant
    /// `"Failed"`, discarding the cause with no inverse; this test pins that
    /// the durable terminal preserves the structured `TerminalError` instead.
    #[test]
    fn terminal_command_cbor_roundtrip_preserves_failed_terminal() {
        let entry = LoadedEntry::Command(JournalCommand::Terminal {
            status: WorkflowStatus::Failed {
                terminal: TerminalError::explicit("disk full at step 3"),
            },
        });

        let mut buf = Vec::new();
        ciborium::into_writer(&entry, &mut buf).expect("CBOR encode succeeds");
        let decoded: LoadedEntry =
            ciborium::from_reader(buf.as_slice()).expect("CBOR decode succeeds");

        assert_eq!(
            decoded, entry,
            "the journal Terminal command must round-trip the full WorkflowStatus \
             (including a Failed's structured TerminalError) byte-equal through CBOR"
        );
        // The structured cause survives — the property the old free-text
        // reason label could not provide.
        let LoadedEntry::Command(JournalCommand::Terminal {
            status: WorkflowStatus::Failed { terminal },
        }) = decoded
        else {
            panic!("decoded entry must be a Terminal carrying a Failed status");
        };
        assert_eq!(terminal.kind(), TerminalErrorKind::Explicit);
        assert_eq!(terminal.detail(), "disk full at step 3");
    }

    /// `WorkflowId::for_correlation` is total and deterministic over
    /// every valid correlation key: the derived id is grammar-valid and
    /// equal correlations yield equal ids (the crash-resume requirement —
    /// ADR-0064 §5).
    #[test]
    fn for_correlation_is_deterministic_and_grammar_valid() {
        // A derived correlation key carries `:` and `/` — chars the
        // WorkflowId grammar rejects. The derivation must sanitise them.
        let corr = CorrelationKey::derive(
            "127.0.0.1:9000",
            &ContentHash::of(b"provision-record"),
            "start-workflow",
        );
        let a = WorkflowId::for_correlation(&corr);
        let b = WorkflowId::for_correlation(&corr);
        assert_eq!(a, b, "equal correlations must yield equal ids (crash-resume)");
        // Grammar: leading char ascii-lower/digit, interior in [a-z0-9-].
        let s = a.as_str();
        let mut chars = s.chars();
        let lead = chars.next().expect("non-empty");
        assert!(lead.is_ascii_lowercase() || lead.is_ascii_digit(), "valid leading char");
        assert!(
            chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "every interior char is in the WorkflowId grammar"
        );
        assert!(s.starts_with("wf-"), "stable wf- prefix");
    }

    /// Distinct correlation keys map to distinct ids (no collision for
    /// the canonical derived shape), and the mapping rejects nothing
    /// (totality) — a plain non-empty key also derives cleanly.
    #[test]
    fn for_correlation_distinguishes_distinct_keys_and_accepts_plain_keys() {
        let a =
            WorkflowId::for_correlation(&CorrelationKey::new("alpha:start/aa").expect("valid key"));
        let b =
            WorkflowId::for_correlation(&CorrelationKey::new("beta:start/bb").expect("valid key"));
        assert_ne!(a, b, "distinct correlations derive distinct ids");
        // Totality: a key with no special chars still derives.
        let plain =
            WorkflowId::for_correlation(&CorrelationKey::new("plainkey").expect("valid key"));
        assert_eq!(plain.as_str(), "wf-plainkey");
    }

    /// `WorkflowId::for_correlation` preserves ASCII digits verbatim. The
    /// char-map keeps `[a-z0-9-]` as-is and folds everything else to `-`;
    /// a digit is in the keep-set, NOT the fold-set. Pins the
    /// `is_ascii_digit() || c == '-'` disjunction against an `&&` collapse
    /// (`is_ascii_digit() && c == '-'` is unsatisfiable, so every digit
    /// would silently fold to `-`).
    #[test]
    fn for_correlation_preserves_digits_verbatim() {
        let id = WorkflowId::for_correlation(&CorrelationKey::new("abc123").expect("valid key"));
        assert_eq!(
            id.as_str(),
            "wf-abc123",
            "digits survive the char map verbatim — they are not folded to '-'"
        );
    }

    /// Regression — journal-key collision for long correlation keys.
    ///
    /// `WorkflowId::for_correlation` used to truncate the mapped id to a
    /// bespoke 127-char ceiling (`wf-` prefix + at most 124 correlation
    /// chars), while a `CorrelationKey` may be up to
    /// [`overdrive_core::id::LABEL_MAX`] (253) chars. Two distinct keys that
    /// shared their first 124 mapped chars but differed only past that
    /// boundary truncated to the SAME `WorkflowId` — the second instance's
    /// `start()` then opened the first instance's journal (silent no-op on a
    /// `Terminal` row, or a wrong-sequence replay). The canonical
    /// `target:purpose/<hex>` form places the discriminating
    /// content-addressed suffix at the END of the string, i.e. in exactly
    /// the region truncation dropped.
    ///
    /// The fix unifies the ceiling on the shared `LABEL_MAX` (plus room for
    /// the `wf-` prefix), so a maximum-length correlation key never
    /// truncates and the end-of-string discriminant always survives.
    #[test]
    fn for_correlation_long_keys_sharing_truncated_prefix_do_not_collide() {
        // 124 shared mapped chars — exactly the slice the old 127-char
        // ceiling (3 prefix + 124) preserved. Both keys are identical here
        // and diverge only AFTER it, in the region the old code dropped.
        let shared_prefix = "a".repeat(124);
        let key_a =
            CorrelationKey::new(&format!("{shared_prefix}:start/1111111111")).expect("valid key");
        let key_b =
            CorrelationKey::new(&format!("{shared_prefix}:start/2222222222")).expect("valid key");
        assert_ne!(key_a, key_b, "the two correlation keys are genuinely distinct");

        let id_a = WorkflowId::for_correlation(&key_a);
        let id_b = WorkflowId::for_correlation(&key_b);

        // The defect: both ids were `wf-` + the shared 124-char prefix.
        assert_ne!(
            id_a, id_b,
            "distinct correlation keys differing only past the old truncation \
             boundary must derive distinct WorkflowIds — else two instances \
             share one journal"
        );
        // The discriminating suffix survives end-to-end (no truncation).
        assert!(id_a.as_str().ends_with("1111111111"), "key_a suffix preserved");
        assert!(id_b.as_str().ends_with("2222222222"), "key_b suffix preserved");
    }

    /// A maximum-length `CorrelationKey` (253 chars) maps without
    /// truncation: the derived id is the full `wf-`-prefixed mapping and
    /// stays within the grammar. Pins the `WORKFLOW_ID_MAX = LABEL_MAX +
    /// WF_PREFIX.len()` sizing against a regression to a smaller ceiling.
    #[test]
    fn for_correlation_does_not_truncate_a_maximum_length_key() {
        let raw = "z".repeat(overdrive_core::id::LABEL_MAX);
        let key = CorrelationKey::new(&raw).expect("LABEL_MAX-length key is valid");
        let id = WorkflowId::for_correlation(&key);
        assert_eq!(
            id.as_str().len(),
            "wf-".len() + overdrive_core::id::LABEL_MAX,
            "the full key is mapped 1:1 with no truncation"
        );
        assert_eq!(id.as_str(), format!("wf-{raw}"), "every char survives the map");
    }
}
