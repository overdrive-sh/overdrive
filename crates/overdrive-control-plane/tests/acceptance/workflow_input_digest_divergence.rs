//! Acceptance — `input_digest` is derived from the start-input bytes, so
//! two instances of one kind with different inputs are distinguishable
//! (DISTILL RED scaffold, `workflow-result-error-model` / ADR-0065;
//! resolves #217).
//!
//! Slice 03 / D5. `WorkflowStart` grows `{ name, input: Vec<u8> }`; the
//! engine's `started_digests` derives `input_digest =
//! ContentHash::of(&spec.input)` (the start-input bytes) and `spec_digest`
//! from the kind identity. The two digests DIVERGE as intended — the
//! `TODO(#217)` is discharged: two instances of the SAME kind with
//! DIFFERENT `input` get DIFFERENT `input_digest`s, and the same input
//! yields the same digest (ADR-0065 § 5, ADR-0063 § 2).
//!
//! # Why NEW (not migrated)
//!
//! Today the engine hashes `spec.name` bytes for BOTH `spec_digest` AND
//! `input_digest` (the `TODO(#217)` bug). The existing
//! `workflow_engine_writes_terminal_row` /
//! `journal_records_inputs_not_derived` tests assert `input_digest =
//! ContentHash::of(ProvisionRecord::PAYLOAD)` (the transport STEP payload
//! `b"provision-record"`) — a coincidental value that is NEITHER the name
//! NOR the new contract. Those tests MIGRATE (the assertion target changes
//! to `ContentHash::of(&spec.input)`). But NO existing test pins the
//! load-bearing #217 acceptance itself: that two DISTINCT inputs of one
//! kind produce two DISTINCT digests. This scenario is that #217
//! acceptance, executable.
//!
//! # Layer / paradigm
//!
//! Layer 1-2 (the digest derivation is a pure function of `spec.input`;
//! the scenario drives it through the engine's `started_digests` /
//! `Started` journal command). Per Mandate 9 the divergence is a
//! `@property` ("for ANY two distinct input byte-vectors of one kind, the
//! `input_digests` differ; for ANY repeated input, they match") — DELIVER
//! MAY widen to a `proptest!` over input-byte strategies. The example
//! below pins the canonical two-distinct-inputs readable case.
//!
//! # Port-to-port
//!
//! The driving port is `WorkflowEngine::start` (writing `Started` at
//! command-index 0). The observable outcome is asserted at the
//! `JournalStore::load_journal` boundary: the `Started { spec_digest,
//! input_digest }` command's `input_digest` equals
//! `ContentHash::of(&spec.input)`, and two starts with distinct `input`
//! bytes yield distinct `input_digest`s on their respective journals.
//!
//! Scenario traces to: D5 (ADR-0065 § 5), #217, Slice 03 acceptance intent
//! ("two instances of the same kind with different `input` persist +
//! rehydrate with distinct `input_digest`s"). Tags: `@in-memory`
//! `@property` `@D5` `@issue-217`.
//!
//! RED-scaffold convention (`.claude/rules/testing.md` § "RED scaffolds"):
//! the bodies below are self-contained `panic!`s importing NO unbuilt
//! production type (the reshaped `WorkflowStart { name, input }` +
//! `started_digests` off `spec.input` land in DELIVER Slice 03). nextest
//! reports PASS; clippy is clean; lefthook needs no `--no-verify`.

/// `@in-memory` `@property` `@D5` `@issue-217` (NEW-2) — two instances of
/// the SAME workflow kind with DIFFERENT `spec.input` bytes record
/// DIFFERENT `input_digest`s in their `Started` commands; the digest is
/// `ContentHash::of(&spec.input)`, NOT the kind name (the #217 bug). This
/// is the executable #217 acceptance.
///
/// DELIVER (Slice 03) body, once `WorkflowStart { name, input }` +
/// `started_digests` off `spec.input` exist:
///
/// 1. One kind name; two `WorkflowStart`s `spec_a = { name, input: cbor(A) }`
///    and `spec_b = { name, input: cbor(B) }` with `A != B`.
/// 2. Drive `engine.start(&spec_a, ..)` and `engine.start(&spec_b, ..)` on
///    two distinct correlations / workflow ids (shared or separate journal).
/// 3. Read each `Started { spec_digest, input_digest }` off
///    `load_journal`.
/// 4. `assert_eq!(input_digest_a, ContentHash::of(&spec_a.input))` and the
///    same for `b` — the digest is the start-input bytes.
/// 5. `assert_ne!(input_digest_a, input_digest_b)` — distinct inputs ⇒
///    distinct digests (the #217 fix; a `spec.name`-based digest would
///    make these EQUAL — the bug).
/// 6. `assert_eq!(spec_digest_a, spec_digest_b)` — same KIND ⇒ same
///    `spec_digest` (the identity axis is unchanged).
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn two_distinct_inputs_of_one_kind_get_distinct_input_digests() {
    panic!(
        "Not yet implemented -- RED scaffold (NEW-2 / input_digest = ContentHash::of(&spec.input); \
         two distinct inputs of one kind diverge; ADR-0065 D5, resolves #217)"
    );
}

/// `@in-memory` `@D5` `@issue-217` (NEW-2b) — the same input bytes yield
/// the SAME `input_digest` (determinism), and a round-trip `StartWorkflow
/// → intent → hydrate → engine` preserves the input bytes verbatim. Pins
/// the deterministic half of the divergence property AND the
/// persist→rehydrate input fidelity (the D5 durability path; the
/// rkyv-envelope persisted spec carries `input` losslessly across a
/// restart).
///
/// DELIVER body: persist `spec.archive_for_store()?` under the instance
/// intent key, rehydrate via `WorkflowStart::from_store_bytes(value)?`, and
/// assert the rehydrated `input` is byte-equal to the original AND that the
/// engine derives the identical `input_digest` from it.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn same_input_round_trips_with_a_stable_input_digest() {
    panic!(
        "Not yet implemented -- RED scaffold (NEW-2b / same input ⇒ stable input_digest; \
         StartWorkflow→intent→hydrate preserves input bytes; ADR-0065 D5)"
    );
}
