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

use async_trait::async_trait;
use bytes::Bytes;

use crate::workflow::{
    CallRequest, Workflow, WorkflowCtx, WorkflowName, WorkflowResult, WorkflowSpec,
};

/// The canonical slice-01 reference workflow: perform one external
/// `ctx.call` (the provision-write effect), then return a terminal
/// `Success`. Authored as one ordinary `async fn run` per S-WP-01-02.
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
        let request =
            CallRequest { target: self.target, payload: Bytes::from_static(Self::PAYLOAD) };
        match ctx.call(request).await {
            Ok(_response) => WorkflowResult::Success,
            Err(_err) => WorkflowResult::Failed { reason: "provision call failed".to_string() },
        }
    }
}
