//! Acceptance — typed `Output` round-trips through the CBOR erasure
//! adapter (NEW-1, `workflow-result-error-model` / ADR-0065).
//!
//! Slice 01 / D1 (object safety via author-edge typing + engine-boundary
//! CBOR erasure). The author writes `Workflow { type Output; type Input;
//! async fn run(&self, ctx, input) -> Result<Output, TerminalError> }`; a
//! generic `ErasedWorkflowAdapter<W>` blanket-erases `Output`/`Input` to
//! CBOR into the object-safe `ErasedWorkflow { run_erased(&self, ctx,
//! input_bytes: &[u8]) -> Result<Vec<u8>, TerminalError> }` the engine
//! drives. This is the SAME typed-edge / CBOR-erased-interior split
//! `ctx.run<T>` already performs for step results (ADR-0065 § 1).
//!
//! # Why NEW (not migrated)
//!
//! Every existing slice-01/02 workflow fixture is `Output = ()` — the
//! contentless terminal the pre-ADR-0065 contentless success modelled. NO
//! existing test exercises a *non-unit* `Output` crossing the erasure
//! boundary and decoding back equal. This scenario is the genuinely-new
//! behaviour the reshape introduces: a typed value surface at the output
//! boundary. It is the validating proof that object safety is preserved
//! WITH a real output payload (the `#40` cert-rotation consumer's typed
//! cert output rides this exact path).
//!
//! # Layer / paradigm
//!
//! Layer 1 (pure, no I/O, default lane) — the adapter is unit-testable in
//! isolation over `Sim*` ctx ports. Per Mandate 9 this layer-1 surface is
//! the home for PBT-full coverage: the roundtrip is a universal property
//! ("for ANY CBOR-serialisable `Output` value, erase-then-decode is the
//! identity"). The output scenario is a `proptest!` over a domain `Output`
//! strategy; the input scenario pins the decode side of the same adapter.
//!
//! # Port-to-port
//!
//! The driving port is `ErasedWorkflow::run_erased` (the object-safe engine
//! surface). The observable outcome is its returned `Ok(Vec<u8>)` CBOR
//! bytes, which `ciborium::from_reader` decodes back to a value `==` the
//! typed `Output` the author body returned. No engine internals; the
//! adapter IS the unit under test.
//!
//! Scenario traces to: D1 (ADR-0065 § 1), Slice 01 acceptance intent
//! ("`ErasedWorkflowAdapter` round-trips a typed `Output`/`Input` through
//! CBOR"). Tags: `@in-memory` `@property` `@D1`.

#![allow(clippy::expect_used)]

use std::sync::Arc;

use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{
    AlwaysLiveCursor, ErasedWorkflow, ErasedWorkflowAdapter, JournalCursor, TerminalError,
    Workflow, WorkflowCtx,
};

use async_trait::async_trait;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::transport::SimTransport;
use proptest::prelude::*;

/// A domain `Output` shape the `#40` cert-rotation consumer is
/// representative of: a small struct of an identifier + an opaque payload
/// vector. CBOR-serialisable; `PartialEq` so the roundtrip can assert
/// byte-equal recovery.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct CertOutput {
    serial: String,
    der: Vec<u8>,
}

/// A workflow whose typed `Output` is the non-unit [`CertOutput`] and whose
/// `Input` is `()`. The body simply returns a pre-built output — the unit
/// under test is the ERASURE (encode side), not the body logic.
struct EchoOutput {
    output: CertOutput,
}

#[async_trait]
impl Workflow for EchoOutput {
    type Output = CertOutput;
    type Input = ();

    async fn run(&self, _ctx: &WorkflowCtx, _input: ()) -> Result<CertOutput, TerminalError> {
        Ok(self.output.clone())
    }
}

/// A workflow whose typed `Input` is a non-unit [`CertOutput`] and whose
/// `Output` is `()`. The body asserts it OBSERVED the decoded input — the
/// unit under test is the ERASURE (decode side). On a mismatch it returns a
/// terminal failure so the test's `Ok(())` assertion fails loudly.
struct AssertInput {
    expected: CertOutput,
}

#[async_trait]
impl Workflow for AssertInput {
    type Output = ();
    type Input = CertOutput;

    async fn run(&self, _ctx: &WorkflowCtx, input: CertOutput) -> Result<(), TerminalError> {
        if input == self.expected {
            Ok(())
        } else {
            Err(TerminalError::explicit("decoded input did not match the recorded bytes"))
        }
    }
}

/// Build a `WorkflowCtx` over `Sim*` ports + the non-durable
/// [`AlwaysLiveCursor`] — the adapter under test performs no `ctx`
/// await-points, so the cursor is never exercised; it is required only to
/// construct the ctx.
fn sim_ctx() -> WorkflowCtx {
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let journal: Arc<dyn JournalCursor> = Arc::new(AlwaysLiveCursor);
    WorkflowCtx::new(clock, transport, entropy, journal)
}

/// CBOR-encode `value` the way the durable start intent records the opaque
/// `input` bytes — the bytes `run_erased` receives and decodes.
fn cbor_of<T: serde::Serialize>(value: &T) -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    ciborium::into_writer(value, &mut bytes).expect("CBOR-encode the input value");
    bytes
}

/// Strategy for an arbitrary [`CertOutput`] — every serial string shape +
/// every opaque DER byte vector (including empty), so the roundtrip
/// property holds across the domain `Output` space, not one picked example.
fn arb_cert_output() -> impl Strategy<Value = CertOutput> {
    ("[a-zA-Z0-9:_-]{0,48}", prop::collection::vec(any::<u8>(), 0..=128))
        .prop_map(|(serial, der)| CertOutput { serial, der })
}

proptest! {
    /// `@in-memory` `@property` `@D1` (NEW-1) — a workflow whose `Output` is
    /// a non-unit CBOR-serialisable type, driven through
    /// `ErasedWorkflowAdapter`, produces `Ok(output_bytes)` from
    /// `run_erased`, and those bytes `ciborium`-decode back to a value
    /// byte-equal to the `Output` the typed body returned. Proves the
    /// typed-edge → CBOR-erased-interior split is lossless for a real output
    /// payload (the `#40` cert-output path), for EVERY value in the domain
    /// `Output` space.
    #[test]
    fn erased_workflow_round_trips_a_non_unit_typed_output_through_cbor(
        output in arb_cert_output()
    ) {
        let ctx = sim_ctx();
        // proptest bodies are sync; drive the async erased surface on a
        // current-thread runtime built per case (the adapter performs no
        // real I/O — the runtime just polls the future to completion).
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("build current-thread runtime");
        // The erased engine surface: drive over the unit `Input`'s CBOR
        // bytes; the adapter encodes the typed `Output` to CBOR.
        let bytes = runtime
            .block_on(
                ErasedWorkflowAdapter(EchoOutput { output: output.clone() })
                    .run_erased(&ctx, &cbor_of(&())),
            )
            .expect("erased run succeeds");
        // Observable outcome: the erased bytes decode back to the typed
        // Output the body returned, byte-equal.
        let decoded: CertOutput =
            ciborium::from_reader(bytes.as_slice()).expect("output decodes");
        prop_assert_eq!(decoded, output, "erased Output round-trips through CBOR byte-equal");
    }
}

/// `@in-memory` `@D1` (NEW-1b) — the `Input` decode side of the same
/// adapter: `run_erased` receives the raw recorded CBOR `input_bytes`,
/// decodes them into the typed `W::Input`, and the body observes the
/// decoded value. Pins the INPUT half of the typed-edge surface (the OUTPUT
/// half is the property above) — together they prove the adapter is the
/// bidirectional anti-corruption layer between the typed author edge and
/// the `dyn`-dispatched engine interior.
#[tokio::test]
async fn erased_workflow_decodes_typed_input_from_recorded_cbor_bytes() {
    let ctx = sim_ctx();
    let expected = CertOutput { serial: "cert-42".to_owned(), der: vec![0xde, 0xad, 0xbe, 0xef] };
    // The recorded CBOR input bytes the engine would replay verbatim.
    let input_bytes = cbor_of(&expected);

    // Drive the erased surface: the adapter decodes input_bytes into the
    // typed Input and hands it to the body, which asserts it matches.
    let output_bytes = ErasedWorkflowAdapter(AssertInput { expected: expected.clone() })
        .run_erased(&ctx, &input_bytes)
        .await
        .expect("erased run succeeds — the body observed the decoded input");

    // Output is the unit type; its CBOR decodes back to ().
    let decoded_unit: () =
        ciborium::from_reader(output_bytes.as_slice()).expect("unit output decodes");
    assert_eq!(decoded_unit, (), "the AssertInput body returns Ok(()) on a matching decoded input");
}
