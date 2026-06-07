//! Acceptance — `input_digest` is derived from the start-input bytes, so
//! two instances of one kind with different inputs are distinguishable
//! (DISTILL RED scaffold, `workflow-result-error-model` / ADR-0065;
//! resolves #217).
//!
//! Slice 03 / D5. `WorkflowStart` carries `{ name, input: Vec<u8> }`; the
//! engine's `started_digests` derives `input_digest =
//! ContentHash::of(&spec.input)` (the start-input bytes) and `spec_digest`
//! from the kind identity. The two digests DIVERGE as intended — the #217
//! obligation is discharged: two instances of the SAME kind with
//! DIFFERENT `input` get DIFFERENT `input_digest`s, and the same input
//! yields the same digest (ADR-0065 § 5, ADR-0066 § 2).
//!
//! # Why NEW (not migrated)
//!
//! Before #217 the engine hashed `spec.name` bytes for BOTH `spec_digest`
//! AND `input_digest` (the #217 bug). The migrated
//! `workflow_engine_writes_terminal_row` /
//! `journal_records_inputs_not_derived` tests now assert `input_digest =
//! ContentHash::of(&spec.input)`. But NO existing test pins the
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
//! `input_digests` differ; for ANY repeated input, they match"): the
//! `proptest!` below quantifies over input-byte strategies. A canonical
//! readable two-distinct-inputs example accompanies it.
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

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;

use proptest::prelude::*;

use overdrive_control_plane::journal::{JournalCommand, JournalStore, LoadedEntry, WorkflowId};
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};

use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::testing::workflow::ProvisionRecord;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::WorkflowStart;

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

/// Construct a fresh engine over `Sim*` ports with `ProvisionRecord`
/// registered under its kind name. Each instance gets its OWN journal store
/// (returned alongside the engine) so two starts write to separate runs.
fn engine_with_journal() -> (WorkflowEngine, Arc<dyn JournalStore>) {
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let target: SocketAddr = "127.0.0.1:9200".parse().expect("valid addr");

    let mut registry = WorkflowRegistry::new();
    registry.register(ProvisionRecord::spec().name, move || ProvisionRecord::new(target));

    let engine =
        WorkflowEngine::new(Arc::clone(&journal), clock, transport, entropy, registry, obs);
    (engine, journal)
}

/// Drive `engine.start` for a spec carrying `input`, then read the first
/// `Started` command's `(spec_digest, input_digest)` off the loaded run.
async fn started_digests_via_engine(input: Vec<u8>, id_slug: &str) -> (ContentHash, ContentHash) {
    let (engine, journal) = engine_with_journal();
    let spec = WorkflowStart { name: ProvisionRecord::spec().name, input };
    let correlation = CorrelationKey::derive(
        id_slug,
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new(id_slug).expect("valid workflow id");

    engine.start(&spec, &correlation, &workflow_id).await.expect("engine start succeeds");
    engine.join_all().await;

    let loaded = journal.load_journal(&workflow_id).await.expect("load journal after start");
    loaded
        .iter()
        .find_map(|entry| match entry {
            LoadedEntry::Command(JournalCommand::Started { spec_digest, input_digest }) => {
                Some((*spec_digest, *input_digest))
            }
            _ => None,
        })
        .expect("the loaded run begins with a Started command")
}

/// `@in-memory` `@property` `@D5` `@issue-217` (NEW-2) — two instances of
/// the SAME workflow kind with DIFFERENT `spec.input` bytes record
/// DIFFERENT `input_digest`s in their `Started` commands; the digest is
/// `ContentHash::of(&spec.input)`, NOT the kind name (the #217 bug). This
/// is the executable #217 acceptance — the canonical two-distinct-inputs
/// readable case (the proptest below quantifies the universal property).
#[tokio::test]
async fn two_distinct_inputs_of_one_kind_get_distinct_input_digests() {
    // Two distinct, non-equal input byte-vectors for ONE kind.
    let input_a = b"input-alpha".to_vec();
    let input_b = b"input-bravo".to_vec();
    assert_ne!(input_a, input_b, "the two inputs must differ for the divergence to be meaningful");

    let (spec_digest_a, input_digest_a) =
        started_digests_via_engine(input_a.clone(), "wf-divergence-a").await;
    let (spec_digest_b, input_digest_b) =
        started_digests_via_engine(input_b.clone(), "wf-divergence-b").await;

    // The digest is the start-input bytes (the #217 fix; ADR-0065 §5).
    assert_eq!(
        input_digest_a,
        ContentHash::of(&input_a),
        "input_digest must be ContentHash::of(&spec.input) (ADR-0065 §5, resolves #217)"
    );
    assert_eq!(
        input_digest_b,
        ContentHash::of(&input_b),
        "input_digest must be ContentHash::of(&spec.input) (ADR-0065 §5, resolves #217)"
    );

    // Distinct inputs ⇒ distinct digests. A `spec.name`-based digest (the
    // #217 bug) would make these EQUAL — the regression this pins.
    assert_ne!(
        input_digest_a, input_digest_b,
        "two distinct inputs of one kind must produce distinct input_digests (#217)"
    );

    // Same KIND ⇒ same `spec_digest` (the identity axis is unchanged).
    assert_eq!(
        spec_digest_a, spec_digest_b,
        "two instances of the SAME kind must share one spec_digest (kind identity unchanged)"
    );
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// `@in-memory` `@property` `@D5` `@issue-217` (NEW-2, universal) — for
    /// ANY two distinct input byte-vectors of one kind, the engine's
    /// `input_digest`s differ AND each equals `ContentHash::of(&spec.input)`;
    /// the `spec_digest` is invariant across inputs (kind identity). This is
    /// the Hebert "generalizing example tests" form of the readable case
    /// above (Mandate 9): the divergence is a property of `spec.input`, not
    /// of the two hand-picked vectors.
    #[test]
    fn input_digest_diverges_for_any_two_distinct_inputs_of_one_kind(
        input_a in proptest::collection::vec(any::<u8>(), 0..64),
        input_b in proptest::collection::vec(any::<u8>(), 0..64),
    ) {
        prop_assume!(input_a != input_b);

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("build current-thread runtime");
        rt.block_on(async {
            let (spec_digest_a, input_digest_a) =
                started_digests_via_engine(input_a.clone(), "wf-pbt-a").await;
            let (spec_digest_b, input_digest_b) =
                started_digests_via_engine(input_b.clone(), "wf-pbt-b").await;

            // Each input_digest is the content hash of THAT instance's input.
            prop_assert_eq!(input_digest_a, ContentHash::of(&input_a));
            prop_assert_eq!(input_digest_b, ContentHash::of(&input_b));
            // Distinct inputs ⇒ distinct digests (the divergence property).
            prop_assert_ne!(input_digest_a, input_digest_b);
            // Same kind ⇒ invariant spec_digest (the identity axis).
            prop_assert_eq!(spec_digest_a, spec_digest_b);
            Ok(())
        })?;
    }
}

/// `@in-memory` `@D5` `@issue-217` (NEW-2b) — the same input bytes yield
/// the SAME `input_digest` (determinism), and a round-trip
/// `archive_for_store → from_store_bytes` preserves the input bytes
/// verbatim. Pins the deterministic half of the divergence property AND the
/// persist→rehydrate input fidelity (the D5 durability path; the
/// rkyv-envelope persisted spec carries `input` losslessly across a
/// restart).
#[tokio::test]
async fn same_input_round_trips_with_a_stable_input_digest() {
    let input = b"input-stable".to_vec();

    // Two independent starts with the SAME input bytes record the SAME
    // input_digest (determinism — the input is the sole determinant).
    let (_, input_digest_first) =
        started_digests_via_engine(input.clone(), "wf-stable-first").await;
    let (_, input_digest_second) =
        started_digests_via_engine(input.clone(), "wf-stable-second").await;
    assert_eq!(
        input_digest_first, input_digest_second,
        "the same input bytes must yield the same input_digest (determinism)"
    );

    // Persist→rehydrate fidelity: the rkyv-envelope codec carries `input`
    // losslessly across a (modelled) restart. Persist via `archive_for_store`,
    // rehydrate via `from_store_bytes`, and assert the rehydrated `input` is
    // byte-equal to the original AND the engine derives the identical
    // input_digest from it.
    let spec = WorkflowStart { name: ProvisionRecord::spec().name, input: input.clone() };
    let archived = spec.archive_for_store().expect("archive_for_store succeeds");
    let rehydrated = WorkflowStart::from_store_bytes(&archived).expect("from_store_bytes decodes");

    assert_eq!(
        rehydrated.input, input,
        "the rehydrated start intent's input is byte-equal to the original (D5 durability)"
    );
    let (_, input_digest_rehydrated) =
        started_digests_via_engine(rehydrated.input.clone(), "wf-stable-rehydrated").await;
    assert_eq!(
        input_digest_rehydrated, input_digest_first,
        "the engine derives the identical input_digest from the rehydrated input bytes"
    );
}
