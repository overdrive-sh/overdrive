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
//! to_failed_terminal`, which MIGRATES to `TerminalError::explicit`) and an
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
//!
//! RED-scaffold convention (`.claude/rules/testing.md` § "RED scaffolds"):
//! the body below is a self-contained `panic!` importing NO unbuilt
//! production type (`WorkflowStatus` / `TerminalError` / `TerminalErrorKind`
//! plus the reshaped `WorkflowStart { name, input }` and the decode-in-adapter
//! path land in DELIVER Slices 01-03). nextest reports PASS; clippy is
//! clean; lefthook needs no `--no-verify`.

/// `@in-memory` `@error` `@D2` `@D3` (NEW-4) — a workflow whose `Input` is
/// a non-unit type, started with `spec.input` bytes that are NOT valid CBOR
/// for that `Input`, terminates `WorkflowStatus::Failed { terminal: kind ==
/// MalformedInput }` (engine-minted) and tears down its live-instance
/// entry. The body never runs to author a failure; the adapter's decode
/// failure IS the terminal.
///
/// DELIVER (Slices 01-03) body, once the types + adapter decode path exist:
///
/// 1. A fixture `impl Workflow { type Input = <non-unit type>; type Output
///    = (); run(&self, _ctx, _input) -> Result<(), TerminalError> { Ok(()) }
///    }` whose body would succeed IF the input decoded.
/// 2. `let spec = WorkflowStart { name, input: b"not-cbor-for-this-input"
///    .to_vec() };` — bytes that fail `ciborium::from_reader::<Input>`.
/// 3. `engine.start(&spec, &correlation, &workflow_id).await` then
///    `join_all`.
/// 4. Read the `WorkflowTerminal { status }` row; assert
///    `matches!(status, WorkflowStatus::Failed { terminal } if
///    terminal.kind() == TerminalErrorKind::MalformedInput)`.
/// 5. `assert!(!engine.live_instances().contains(&correlation))` — clean
///    termination, no strand.
/// 6. The body's run-counter (a shared `AtomicUsize`) is 0 — the body was
///    NOT entered (the decode gate fired first).
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn undecodable_input_terminates_failed_malformed_input_without_running_the_body() {
    panic!(
        "Not yet implemented -- RED scaffold (NEW-4 / undecodable start input ⇒ engine-minted \
         WorkflowStatus::Failed{{MalformedInput}}, body never entered; ADR-0065 D1/D2/D3)"
    );
}
