//! Acceptance — typed `Output` round-trips through the CBOR erasure
//! adapter (DISTILL RED scaffold, `workflow-result-error-model` / ADR-0065).
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
//! contentless terminal the old `WorkflowResult::Success` modelled. NO
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
//! the home for PBT-full coverage in DELIVER: the roundtrip is a universal
//! property ("for ANY CBOR-serialisable `Output` value, erase-then-decode
//! is the identity"). DELIVER replaces the `#[should_panic]` body with a
//! `proptest!` over a domain `Output` strategy (workspace pins
//! `proptest = "1"`); the example below pins the canonical readable case.
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
//!
//! RED-scaffold convention (`.claude/rules/testing.md` § "RED scaffolds"):
//! the body below is a self-contained `panic!` that imports NO unbuilt
//! production type (`ErasedWorkflowAdapter` / `ErasedWorkflow` /
//! `TerminalError` land in DELIVER Slice 01). nextest reports PASS
//! (expected panic), clippy is clean, lefthook needs no `--no-verify`.
//! DELIVER Slice 01 replaces the panic with the real adapter roundtrip
//! (and promotes it to `proptest!`).

/// `@in-memory` `@property` `@D1` (NEW-1) — a workflow whose `Output` is a
/// non-unit CBOR-serialisable type, driven through `ErasedWorkflowAdapter`,
/// produces `Ok(output_bytes)` from `run_erased`, and those bytes
/// `ciborium`-decode back to a value byte-equal to the `Output` the typed
/// body returned. Proves the typed-edge → CBOR-erased-interior split is
/// lossless for a real output payload (the `#40` cert-output path).
///
/// DELIVER (Slice 01) body, once `ErasedWorkflowAdapter<W>` /
/// `ErasedWorkflow` / `TerminalError` exist:
///
/// 1. Define a local fixture `struct EchoOutput;` with
///    `impl Workflow for EchoOutput { type Output = <domain type>; type
///    Input = (); async fn run(&self, _ctx, _input: ()) ->
///    Result<Self::Output, TerminalError> { Ok(<value>) } }`.
/// 2. Build a `WorkflowCtx` from `Sim*` ports + `AlwaysLiveCursor`.
/// 3. `let bytes = ErasedWorkflowAdapter(EchoOutput).run_erased(&ctx,
///    &cbor_of_unit).await.expect("erased run succeeds");`
/// 4. `let decoded: <domain type> = ciborium::from_reader(&bytes[..])
///    .expect("output decodes");`
/// 5. `assert_eq!(decoded, <value>)` — the erased output round-trips.
///
/// Then widen to `proptest!` over a strategy for the domain `Output`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn erased_workflow_round_trips_a_non_unit_typed_output_through_cbor() {
    panic!(
        "Not yet implemented -- RED scaffold (NEW-1 / typed Output round-trips through \
         ErasedWorkflowAdapter CBOR erasure; ADR-0065 D1)"
    );
}

/// `@in-memory` `@D1` (NEW-1b) — the `Input` decode side of the same
/// adapter: `run_erased` receives the raw recorded CBOR `input_bytes`,
/// decodes them into the typed `W::Input`, and the body observes the
/// decoded value. Pins the INPUT half of the typed-edge surface (the
/// OUTPUT half is the property above) — together they prove the adapter is
/// the bidirectional anti-corruption layer between the typed author edge
/// and the `dyn`-dispatched engine interior.
///
/// DELIVER body: a fixture `impl Workflow { type Input = <domain type>;
/// type Output = (); run(&self, _ctx, input) -> Result<(), TerminalError>
/// { assert input == <expected>; Ok(()) } }`, driven via
/// `ErasedWorkflowAdapter(..).run_erased(&ctx, &cbor_of_input)`, asserting
/// `Ok(())` and that the body saw the decoded input.
#[test]
#[should_panic(expected = "RED scaffold")]
fn erased_workflow_decodes_typed_input_from_recorded_cbor_bytes() {
    panic!(
        "Not yet implemented -- RED scaffold (NEW-1b / ErasedWorkflowAdapter decodes typed \
         Input from recorded CBOR bytes; ADR-0065 D1)"
    );
}
