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

use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{Workflow, WorkflowCtx, WorkflowResult};

// `ProvisionRecord` (struct + `impl Workflow`) was promoted to the
// shared `overdrive-core::testing::workflow` fixture in step 01-03 so
// `overdrive-sim`'s journal test can construct the same reference
// workflow. The canonical clean `async fn run` body the slice-01
// K6 / D-INH-4 syn-scans read now lives there (see those scans'
// `PROVISION_RECORD_SOURCE` const).
use overdrive_core::testing::workflow::ProvisionRecord;

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::transport::SimTransport;

#[tokio::test]
async fn provision_record_drives_to_terminal_workflow_result() {
    // Driven ports — all non-determinism injected as `Sim*` adapters.
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));

    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let target: SocketAddr = "127.0.0.1:9000".parse().expect("valid addr");

    let ctx = WorkflowCtx::new(clock, transport, entropy);

    let workflow = ProvisionRecord::new(target);

    // Drive the author's `run` through the driving port to its terminal.
    let result = workflow.run(&ctx).await;

    assert_eq!(
        result,
        WorkflowResult::Success,
        "ProvisionRecord must drive its one async fn run to a terminal WorkflowResult::Success"
    );
}
