//! Shared workflow test fixtures — the canonical `ProvisionRecord`
//! reference workflow used across slice-01 acceptance tests.
//!
//! `ProvisionRecord` is the thinnest durable sequence with a real,
//! non-idempotent-to-repeat effect: one external `ctx.call`
//! (the "provision write", US-WP-1) followed by a terminal
//! [`WorkflowResult::Success`]. It is written as one ordinary
//! `async fn run` — no hand-written step enum, no transition match
//! (S-WP-01-02 / K6).
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
//! impl + the `WorkflowSpec` it derives from.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;

use crate::workflow::{Workflow, WorkflowCtx, WorkflowName, WorkflowResult, WorkflowSpec};

/// The canonical slice-01 reference workflow: perform one external
/// effect inside a `ctx.run` durable step (the provision-write), then
/// return a terminal `Success`. Authored as one ordinary `async fn run`
/// per S-WP-01-02.
pub struct ProvisionRecord {
    /// Where the provision-write effect is addressed.
    target: SocketAddr,
}

impl ProvisionRecord {
    /// The workflow name this fixture provisions under. Kebab-case
    /// `provision-record`, matching the `WorkflowName` grammar.
    pub const WORKFLOW_NAME: &'static str = "provision-record";

    /// The provision-write payload bytes the slice-01 `ctx.call` sends.
    pub const PAYLOAD: &'static [u8] = b"provision-record";

    /// Construct a `ProvisionRecord` addressed at `target`.
    #[must_use]
    pub const fn new(target: SocketAddr) -> Self {
        Self { target }
    }

    /// The concrete [`WorkflowSpec`] this fixture corresponds to. The
    /// journal's `Started { spec_digest }` entry (ADR-0063 §2) is
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
    pub fn spec() -> WorkflowSpec {
        WorkflowSpec {
            name: WorkflowName::new(Self::WORKFLOW_NAME)
                .unwrap_or_else(|_| unreachable!("WORKFLOW_NAME is a valid kebab constant")),
        }
    }
}

#[async_trait]
impl Workflow for ProvisionRecord {
    async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult {
        // The provision-write effect runs INSIDE a `ctx.run` durable step:
        // the transport send fires once on the live path, its result is
        // journaled, and a resumed run replays the recorded result without
        // re-firing (exactly-once on the replay path). `T` is the
        // serializable `Result<usize, String>` — the transport error folds
        // into the success type so the whole result round-trips through CBOR.
        let transport = Arc::clone(ctx.transport());
        let target = self.target;
        let payload = Bytes::from_static(Self::PAYLOAD);
        let sent: Result<usize, String> = ctx
            .run("provision-write", async move {
                transport.send_datagram(target, payload).await.map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|err| Err(err.to_string()));
        match sent {
            Ok(_bytes) => WorkflowResult::Success,
            Err(_reason) => WorkflowResult::Failed { reason: "provision call failed".to_string() },
        }
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
/// `ctx.call → terminal` shape — this is a separate fixture added
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

    /// The first (pre-sleep) `ctx.call` payload bytes.
    pub const FIRST_PAYLOAD: &'static [u8] = b"provision-record-pre-sleep";

    /// The second (post-sleep) `ctx.call` payload bytes.
    pub const SECOND_PAYLOAD: &'static [u8] = b"provision-record-post-sleep";

    /// Construct a `ProvisionRecordWithSleep` addressed at
    /// `first_target` / `second_target`, sleeping `sleep` between the
    /// two calls.
    #[must_use]
    pub const fn new(first_target: SocketAddr, second_target: SocketAddr, sleep: Duration) -> Self {
        Self { first_target, second_target, sleep }
    }

    /// The concrete [`WorkflowSpec`] this fixture corresponds to.
    ///
    /// # Panics
    ///
    /// Never in practice: [`Self::WORKFLOW_NAME`] is a compile-time
    /// kebab constant that satisfies the `WorkflowName` grammar.
    #[must_use]
    pub fn spec() -> WorkflowSpec {
        WorkflowSpec {
            name: WorkflowName::new(Self::WORKFLOW_NAME)
                .unwrap_or_else(|_| unreachable!("WORKFLOW_NAME is a valid kebab constant")),
        }
    }
}

#[async_trait]
impl Workflow for ProvisionRecordWithSleep {
    async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult {
        // Pre-sleep provision-write effect, inside a durable `ctx.run` step.
        let first_transport = Arc::clone(ctx.transport());
        let first_target = self.first_target;
        let first_payload = Bytes::from_static(Self::FIRST_PAYLOAD);
        let first: Result<usize, String> = ctx
            .run("provision-write-pre-sleep", async move {
                first_transport
                    .send_datagram(first_target, first_payload)
                    .await
                    .map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|err| Err(err.to_string()));
        if first.is_err() {
            return WorkflowResult::Failed { reason: "pre-sleep call failed".to_string() };
        }
        if ctx.sleep(self.sleep).await.is_err() {
            return WorkflowResult::Failed { reason: "sleep failed".to_string() };
        }
        // Post-sleep provision-write effect, inside a durable `ctx.run` step.
        let second_transport = Arc::clone(ctx.transport());
        let second_target = self.second_target;
        let second_payload = Bytes::from_static(Self::SECOND_PAYLOAD);
        let second: Result<usize, String> = ctx
            .run("provision-write-post-sleep", async move {
                second_transport
                    .send_datagram(second_target, second_payload)
                    .await
                    .map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|err| Err(err.to_string()));
        match second {
            Ok(_bytes) => WorkflowResult::Success,
            Err(_reason) => WorkflowResult::Failed { reason: "post-sleep call failed".to_string() },
        }
    }
}
