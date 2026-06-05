//! Slice 01 / US-WP-1 AC1 — author writes one ordinary `async fn run`
//! and the platform drives it to a terminal `WorkflowResult`.
//!
//! Scenario S-WP-01-01 (`docs/feature/workflow-primitive/distill/test-scenarios.md`).
//! ADR-0064 §1 (`Workflow` trait + `WorkflowCtx` in `overdrive-core`),
//! §4 (`ctx.call` is the slice-01 surface). K6 / O3.
//!
//! Port-to-port: the test exercises the *author surface* only — it
//! declares an `impl Workflow for ProvisionRecord` whose body is one
//! ordinary `async fn run`, builds a `WorkflowCtx` from the injected
//! `Sim*` ports, and drives `run` to a terminal `WorkflowResult`. It
//! never reaches into engine internals or a step cursor; the driving
//! port IS `Workflow::run` and the observable outcome IS its returned
//! `WorkflowResult`.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;

use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{CallRequest, Workflow, WorkflowCtx, WorkflowResult};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::transport::SimTransport;

/// A minimal two-step durable sequence: perform one external
/// `ctx.call` (the non-idempotent-to-repeat "provision write"), then
/// return a terminal `Success`. Written as one ordinary `async fn` —
/// no hand-written step enum, no transition match (S-WP-01-02).
struct ProvisionRecord {
    /// Where the provision-write effect is addressed.
    target: SocketAddr,
}

#[async_trait]
impl Workflow for ProvisionRecord {
    async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult {
        let request =
            CallRequest { target: self.target, payload: Bytes::from_static(b"provision-record") };
        match ctx.call(request).await {
            Ok(_response) => WorkflowResult::Success,
            Err(_err) => WorkflowResult::Failed { reason: "provision call failed".to_string() },
        }
    }
}

#[tokio::test]
async fn provision_record_drives_to_terminal_workflow_result() {
    // Driven ports — all non-determinism injected as `Sim*` adapters.
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));

    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let target: SocketAddr = "127.0.0.1:9000".parse().expect("valid addr");

    let ctx = WorkflowCtx::new(clock, transport, entropy);

    let workflow = ProvisionRecord { target };

    // Drive the author's `run` through the driving port to its terminal.
    let result = workflow.run(&ctx).await;

    assert_eq!(
        result,
        WorkflowResult::Success,
        "ProvisionRecord must drive its one async fn run to a terminal WorkflowResult::Success"
    );
}
