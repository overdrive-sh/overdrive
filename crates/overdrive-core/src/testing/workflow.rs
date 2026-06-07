//! Shared workflow test fixtures — the canonical `ProvisionRecord`
//! reference workflow used across slice-01 acceptance tests.
//!
//! `ProvisionRecord` is the thinnest durable sequence with a real,
//! non-idempotent-to-repeat effect: one external `ctx.run` durable step
//! (the "provision write", US-WP-1) followed by a terminal `Ok(())`
//! (`Output = ()` — the contentless terminal the pre-ADR-0065 contentless
//! success modelled). It is written as one ordinary `async fn run` — no
//! hand-written step enum, no transition match (S-WP-01-02 / K6).
//!
//! # Why this lives in `overdrive-core::testing`
//!
//! Step 01-01 defined `ProvisionRecord` inline inside
//! `tests/acceptance/workflow_trait_drives_to_terminal.rs`, which is not
//! reachable from `overdrive-sim`'s journal acceptance test (step 01-03,
//! S-WP-01-05). Promoting the fixture into the `test-utils`-gated
//! `testing` module makes it the single shared definition both
//! `overdrive-core`'s own acceptance test AND `overdrive-sim`'s journal
//! test construct — the sanctioned shared-fixture pattern, rather than a
//! duplicated copy in `overdrive-sim`.
//!
//! Constructing the `WorkflowCtx` (which needs the `Sim*` port adapters
//! living in `overdrive-sim`) stays in each test; this module exposes
//! only the runtime-agnostic `struct ProvisionRecord` + its `Workflow`
//! impl + the `WorkflowStart` it derives from.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;

use crate::reconcilers::Action;
use crate::workflow::{
    SignalKey, TerminalError, Workflow, WorkflowCtx, WorkflowName, WorkflowStart,
};

/// CBOR-encode the unit `Input` these reference workflows take — a 1-byte
/// CBOR `null` (`0xf6`). The `ErasedWorkflowAdapter` decodes this back to
/// `()` before calling `run` (ADR-0065 §1). Factored out so every fixture's
/// `spec()` derives the same opaque `input` bytes through the real CBOR
/// codec rather than a hand-pinned constant.
///
/// # Panics
///
/// Never — `ciborium::into_writer(&(), _)` over an in-memory `Vec` is total
/// for the unit type.
#[must_use]
fn cbor_unit() -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    ciborium::into_writer(&(), &mut bytes)
        .unwrap_or_else(|_| unreachable!("CBOR-encoding the unit type is total"));
    bytes
}

/// The canonical slice-01 reference workflow: perform one external
/// effect inside a `ctx.run` durable step (the provision-write), then
/// return a terminal `Ok(())`. Authored as one ordinary `async fn run`
/// per S-WP-01-02.
pub struct ProvisionRecord {
    /// Where the provision-write effect is addressed.
    target: SocketAddr,
}

impl ProvisionRecord {
    /// The workflow name this fixture provisions under. Kebab-case
    /// `provision-record`, matching the `WorkflowName` grammar.
    pub const WORKFLOW_NAME: &'static str = "provision-record";

    /// The provision-write payload bytes the slice-01 `ctx.run` step sends.
    pub const PAYLOAD: &'static [u8] = b"provision-record";

    /// Construct a `ProvisionRecord` addressed at `target`.
    #[must_use]
    pub const fn new(target: SocketAddr) -> Self {
        Self { target }
    }

    /// The concrete [`WorkflowStart`] this fixture corresponds to. The
    /// journal's `Started { spec_digest }` entry (ADR-0066 §2) is
    /// derived from this spec's CBOR/canonical encoding by the engine
    /// (or, in the slice-01 journal test, by the test itself) — the
    /// fixture exposes the spec, not a pre-computed digest, per
    /// `development.md` "Persist inputs, not derived state".
    ///
    /// # Panics
    ///
    /// Never in practice: [`Self::WORKFLOW_NAME`] is a compile-time
    /// kebab constant that satisfies the `WorkflowName` grammar.
    #[must_use]
    pub fn spec() -> WorkflowStart {
        WorkflowStart {
            name: WorkflowName::new(Self::WORKFLOW_NAME)
                .unwrap_or_else(|_| unreachable!("WORKFLOW_NAME is a valid kebab constant")),
            // These reference workflows take a unit `Input` — the opaque CBOR
            // `input` is the encoding of `()` (a 1-byte CBOR `null`), so the
            // `ErasedWorkflowAdapter` decodes it back to `()` (ADR-0065 §1).
            input: cbor_unit(),
        }
    }
}

#[async_trait]
impl Workflow for ProvisionRecord {
    type Output = ();
    type Input = ();

    async fn run(&self, ctx: &WorkflowCtx, _input: ()) -> Result<(), TerminalError> {
        let transport = Arc::clone(ctx.transport());
        let target = self.target;
        let payload = Bytes::from_static(Self::PAYLOAD);
        ctx.run(
            "provision-write",
            async move { Ok(transport.send_datagram(target, payload).await?) },
        )
        .await?;
        Ok(())
    }
}

/// The canonical slice-02 reference workflow: the thinnest durable
/// sequence that exercises `ctx.sleep` BETWEEN two external effects — a
/// `ctx.run → ctx.sleep → ctx.run` 3-await shape (slice-02 consumer per
/// step 02-01 AC5). Authored as one ordinary `async fn run`.
///
/// **Distinct from [`ProvisionRecord`], not a mutation of it.** The
/// slice-01 e2e (01-08) and the `replay_equivalence_provision_record`
/// invariant (01-07) depend on `ProvisionRecord` staying a
/// `ctx.run → terminal` shape — this is a separate fixture added
/// alongside it so the slice-02 `ctx.sleep` await-surface has a real
/// 3-await consumer without disturbing slice-01's invariants.
pub struct ProvisionRecordWithSleep {
    /// Where the first (pre-sleep) provision-write effect is addressed.
    first_target: SocketAddr,
    /// Where the second (post-sleep) provision-write effect is addressed.
    second_target: SocketAddr,
    /// The logical wait armed via `ctx.sleep` between the two calls.
    sleep: Duration,
}

impl ProvisionRecordWithSleep {
    /// The workflow name this fixture provisions under.
    pub const WORKFLOW_NAME: &'static str = "provision-record-with-sleep";

    /// The first (pre-sleep) `ctx.run` payload bytes.
    pub const FIRST_PAYLOAD: &'static [u8] = b"provision-record-pre-sleep";

    /// The second (post-sleep) `ctx.run` payload bytes.
    pub const SECOND_PAYLOAD: &'static [u8] = b"provision-record-post-sleep";

    /// Construct a `ProvisionRecordWithSleep` addressed at
    /// `first_target` / `second_target`, sleeping `sleep` between the
    /// two calls.
    #[must_use]
    pub const fn new(first_target: SocketAddr, second_target: SocketAddr, sleep: Duration) -> Self {
        Self { first_target, second_target, sleep }
    }

    /// The concrete [`WorkflowStart`] this fixture corresponds to.
    ///
    /// # Panics
    ///
    /// Never in practice: [`Self::WORKFLOW_NAME`] is a compile-time
    /// kebab constant that satisfies the `WorkflowName` grammar.
    #[must_use]
    pub fn spec() -> WorkflowStart {
        WorkflowStart {
            name: WorkflowName::new(Self::WORKFLOW_NAME)
                .unwrap_or_else(|_| unreachable!("WORKFLOW_NAME is a valid kebab constant")),
            // These reference workflows take a unit `Input` — the opaque CBOR
            // `input` is the encoding of `()` (a 1-byte CBOR `null`), so the
            // `ErasedWorkflowAdapter` decodes it back to `()` (ADR-0065 §1).
            input: cbor_unit(),
        }
    }
}

/// The canonical slice-03 reference workflow: the thinnest durable
/// sequence that exercises `ctx.wait_for_signal` followed by
/// `ctx.emit_action` — a `ctx.wait_for_signal → ctx.emit_action →
/// terminal` shape (slice-03 consumer per US-WP-5). Authored as one
/// ordinary `async fn run`.
///
/// **Distinct from [`ProvisionRecord`] / [`ProvisionRecordWithSleep`],
/// not a mutation of either.** The slice-01/02 invariants depend on
/// those fixtures keeping their exact await shapes — this is a separate
/// fixture added alongside them so the slice-03 signal + emit
/// await-surfaces have a real consumer without disturbing the earlier
/// slices' invariants.
///
/// The workflow BLOCKS on `ctx.wait_for_signal(signal)` until the
/// signal row is present in the `ObservationStore` (in-process
/// single-node delivery), then emits one `Action` and returns a terminal
/// `Ok(())`. This is the honest blocking shape step 03-02 proves
/// crash-safe (S-WP-03-01): a crash WHILE blocked on the absent signal
/// re-blocks on the SAME signal on resume.
pub struct ProvisionRecordWithSignalEmit {
    /// The signal key the workflow blocks on via `ctx.wait_for_signal`.
    signal: SignalKey,
    /// The Action emitted via `ctx.emit_action` once the signal fires.
    action: Action,
}

impl ProvisionRecordWithSignalEmit {
    /// The workflow name this fixture provisions under.
    pub const WORKFLOW_NAME: &'static str = "provision-record-with-signal-emit";

    /// The canonical signal key this fixture blocks on. A producer
    /// satisfies the wait by writing an `ObservationRow::Signal` keyed by
    /// this same key.
    pub const SIGNAL_KEY: &'static str = "cert-ready";

    /// Construct a `ProvisionRecordWithSignalEmit` blocking on `signal`
    /// and emitting `action` once the signal fires.
    #[must_use]
    pub const fn new(signal: SignalKey, action: Action) -> Self {
        Self { signal, action }
    }

    /// The canonical [`SignalKey`] this fixture blocks on.
    ///
    /// # Panics
    ///
    /// Never in practice: [`Self::SIGNAL_KEY`] is a compile-time kebab
    /// constant satisfying the `SignalKey` grammar.
    #[must_use]
    pub fn signal_key() -> SignalKey {
        SignalKey::new(Self::SIGNAL_KEY)
            .unwrap_or_else(|_| unreachable!("SIGNAL_KEY is a valid kebab constant"))
    }

    /// The concrete [`WorkflowStart`] this fixture corresponds to.
    ///
    /// # Panics
    ///
    /// Never in practice: [`Self::WORKFLOW_NAME`] is a compile-time
    /// kebab constant that satisfies the `WorkflowName` grammar.
    #[must_use]
    pub fn spec() -> WorkflowStart {
        WorkflowStart {
            name: WorkflowName::new(Self::WORKFLOW_NAME)
                .unwrap_or_else(|_| unreachable!("WORKFLOW_NAME is a valid kebab constant")),
            // These reference workflows take a unit `Input` — the opaque CBOR
            // `input` is the encoding of `()` (a 1-byte CBOR `null`), so the
            // `ErasedWorkflowAdapter` decodes it back to `()` (ADR-0065 §1).
            input: cbor_unit(),
        }
    }
}

#[async_trait]
impl Workflow for ProvisionRecordWithSignalEmit {
    type Output = ();
    type Input = ();

    async fn run(&self, ctx: &WorkflowCtx, _input: ()) -> Result<(), TerminalError> {
        ctx.wait_for_signal(self.signal.clone()).await?;
        ctx.emit_action(self.action.clone()).await?;
        Ok(())
    }
}

#[async_trait]
impl Workflow for ProvisionRecordWithSleep {
    type Output = ();
    type Input = ();

    async fn run(&self, ctx: &WorkflowCtx, _input: ()) -> Result<(), TerminalError> {
        let first_transport = Arc::clone(ctx.transport());
        let first_target = self.first_target;
        let first_payload = Bytes::from_static(Self::FIRST_PAYLOAD);
        ctx.run("provision-write-pre-sleep", async move {
            Ok(first_transport.send_datagram(first_target, first_payload).await?)
        })
        .await?;

        ctx.sleep(self.sleep).await?;

        let second_transport = Arc::clone(ctx.transport());
        let second_target = self.second_target;
        let second_payload = Bytes::from_static(Self::SECOND_PAYLOAD);
        ctx.run("provision-write-post-sleep", async move {
            Ok(second_transport.send_datagram(second_target, second_payload).await?)
        })
        .await?;
        Ok(())
    }
}
