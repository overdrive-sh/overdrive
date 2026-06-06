//! Acceptance — start-input bytes that do not CBOR-decode into the
//! workflow's `Input` produce an engine-minted `WorkflowStatus::Failed {
//! terminal: MalformedInput }` — NOT a panic, NOT a retry (DISTILL RED
//! scaffold, `workflow-result-error-model` / ADR-0065).
//!
//! Slice 02-03 / D1 + D2 + D3. The `ErasedWorkflowAdapter::run_erased`
//! decodes `input_bytes` into `W::Input` BEFORE calling the body; a decode
//! failure maps to `TerminalError::malformed_input(...)` (the adapter's
//! `ciborium::from_reader` error). The engine projects that to
//! `WorkflowStatus::Failed { terminal: TerminalError { kind:
//! MalformedInput, .. } }` and writes it to the durable terminal — the
//! bytes will not change on re-drive, so it is terminal-not-retryable
//! (ADR-0065 § 1 adapter, § 2 `TerminalErrorKind::MalformedInput`, § 3
//! engine mapping).
//!
//! # Why NEW (not migrated)
//!
//! `MalformedInput` is a genuinely-new terminal cause introduced by the
//! typed-input surface (D5): there was no input to malform before #217. It
//! is engine-MINTED (the body never authored it), which distinguishes it
//! from both the existing panic→`Failed` path (`workflow_panic_converges_
//! to_failed_terminal`, which uses `TerminalError::explicit`) and an
//! author's explicit `Err(TerminalError::explicit(..))`. NO existing test
//! covers a non-author-authored terminal arising from undecodable input.
//!
//! # Layer / paradigm
//!
//! Layer 1-2 (engine drives the adapter over `Sim*` ports; the decode
//! failure is deterministic). Per Mandate 11 this engine-boundary sad path
//! is an EXAMPLE-based test (one explicit malformed-byte input), NOT
//! PBT-generated — sad paths at the engine boundary are enumerated
//! explicitly.
//!
//! # Port-to-port
//!
//! The driving port is `WorkflowEngine::start` driven with a spec whose
//! `input` bytes are not valid CBOR for the workflow's `Input`. The
//! observable outcome is asserted at TWO driven-port boundaries:
//! (a) the `ObservationStore` `WorkflowTerminal { status }` row carries
//! `WorkflowStatus::Failed { terminal: kind == MalformedInput }`;
//! (b) the engine's `live_instances` no longer contains the correlation
//! (the instance terminated cleanly, not stranded). The process stays
//! healthy — no panic, no unbounded re-drive.
//!
//! Scenario traces to: D1/D2/D3 (ADR-0065 §§ 1-3), Slice 03 acceptance
//! intent ("a malformed/undecodable intent refuses"). Tags: `@in-memory`
//! `@error` `@D2` `@D3`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;

use overdrive_control_plane::journal::{JournalStore, WorkflowId};
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};

use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::traits::observation_store::{
    ObservationRow, ObservationStore, ObservationSubscription,
};
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{
    TerminalError, TerminalErrorKind, Workflow, WorkflowCtx, WorkflowName, WorkflowStart,
    WorkflowStatus,
};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

/// A fixture workflow whose `Input` is a NON-unit type (`u64`), so a
/// `spec.input` byte-string that is not valid CBOR for a `u64` fails the
/// adapter's decode gate BEFORE the body runs. The body would succeed
/// (`Ok(())`) if the input decoded — it bumps a shared run-counter so the
/// test can prove the body was NEVER entered.
struct DecodesU64Input {
    /// Bumped once if (and only if) `run` is entered. Proving it stays 0
    /// is the "body never ran" half of the acceptance.
    run_counter: Arc<AtomicUsize>,
}

impl DecodesU64Input {
    const WORKFLOW_NAME: &'static str = "decodes-u64-input";
}

#[async_trait]
impl Workflow for DecodesU64Input {
    type Input = u64;
    type Output = ();

    async fn run(&self, _ctx: &WorkflowCtx, _input: u64) -> Result<(), TerminalError> {
        // Reaching here means the input decoded — which the malformed-input
        // gate must prevent. The counter bump is the falsifiable signal.
        self.run_counter.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// `@in-memory` `@error` `@D2` `@D3` (NEW-4) — a workflow whose `Input` is
/// a non-unit type, started with `spec.input` bytes that are NOT valid CBOR
/// for that `Input`, terminates `WorkflowStatus::Failed { terminal: kind ==
/// MalformedInput }` (engine-minted) and tears down its live-instance
/// entry. The body never runs to author a failure; the adapter's decode
/// failure IS the terminal.
#[tokio::test]
async fn undecodable_input_terminates_failed_malformed_input_without_running_the_body() {
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    // Bind a target into the fixture so its constructor matches the
    // registry's `Fn() -> W` shape; the body never uses it (it fails to
    // decode first), but the closure must be self-contained.
    let _target: SocketAddr = "127.0.0.1:9300".parse().expect("valid addr");

    let run_counter = Arc::new(AtomicUsize::new(0));
    let name = WorkflowName::new(DecodesU64Input::WORKFLOW_NAME).expect("valid kebab name");

    let mut registry = WorkflowRegistry::new();
    let counter_for_factory = Arc::clone(&run_counter);
    registry.register(name.clone(), move || DecodesU64Input {
        run_counter: Arc::clone(&counter_for_factory),
    });

    let engine = WorkflowEngine::new(
        Arc::clone(&journal),
        Arc::clone(&clock),
        Arc::clone(&transport),
        Arc::clone(&entropy),
        registry,
        Arc::clone(&obs),
    );

    // `input` bytes that are NOT valid CBOR for a `u64`. A CBOR text-string
    // header (`0x6e` = major type 3, length 14) followed by ASCII decodes to
    // a `String`, never a `u64` — `ciborium::from_reader::<u64>` rejects it.
    let spec = WorkflowStart { name: name.clone(), input: b"not-cbor-for-u64".to_vec() };
    let correlation = CorrelationKey::derive(
        "wf-malformed-0001",
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-malformed-0001").expect("valid workflow id");

    // Subscribe BEFORE driving so the terminal row is observed on the stream.
    let mut subscription: ObservationSubscription =
        obs.subscribe_all().await.expect("subscribe succeeds");

    engine.start(&spec, &correlation, &workflow_id).await.expect("engine start succeeds");
    engine.join_all().await;

    // Observable outcome (a) — the WorkflowTerminal row carries a Failed
    // status whose terminal kind is MalformedInput (engine-minted).
    let mut found: Option<WorkflowStatus> = None;
    for _ in 0..8 {
        let next = tokio::time::timeout(Duration::from_secs(1), subscription.next()).await;
        match next {
            Ok(Some(ObservationRow::WorkflowTerminal { correlation: got, status }))
                if got == correlation =>
            {
                found = Some(status);
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
    let status = found.expect("engine must write a WorkflowTerminal row for the malformed input");
    let WorkflowStatus::Failed { terminal } = status else {
        panic!("undecodable input must terminate Failed, got {status:?}");
    };
    assert_eq!(
        terminal.kind(),
        TerminalErrorKind::MalformedInput,
        "the engine-minted terminal must carry kind == MalformedInput (ADR-0065 §2/§3)"
    );

    // Observable outcome (b) — clean termination, no strand: the engine's
    // live-instance set no longer contains the correlation.
    assert!(
        !engine.live_instances().contains(&correlation),
        "a malformed-input instance must terminate cleanly, leaving no live-task strand"
    );

    // Observable outcome (c) — the author body was NEVER entered: the decode
    // gate fired first, so the run-counter is still 0.
    assert_eq!(
        run_counter.load(Ordering::SeqCst),
        0,
        "the typed body must NOT run on undecodable input — the decode gate fires first"
    );
}
