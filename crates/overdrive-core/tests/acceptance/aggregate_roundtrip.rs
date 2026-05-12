//! Acceptance scenarios for phase-1-control-plane-core step 01-04 —
//! `Job` / `Node` / `Allocation` round-trip through rkyv (canonical lane)
//! and serde-JSON (wire lane) without loss.
//!
//! Covers §2.1 scenario 2 from
//! `docs/feature/phase-1-control-plane-core/distill/test-scenarios.md` and
//! the hashing-determinism rule from `.claude/rules/development.md`
//! ("Internal data → rkyv"): two archivals of the same logical value must
//! produce byte-identical output — the precondition for every
//! content-addressed identity in the system (`SchematicId`, job-spec
//! hashes, Raft log digests, per ADR-0002).
//!
//! The proptests reuse the same `valid_label()` strategy pattern as
//! `tests/acceptance/intent_key_canonical.rs` — narrower than the full
//! validator (lowercase alnum + `-`, leading letter, terminal alnum, ≤ 63
//! chars) but comfortably within the underlying `validate_label`
//! constraints.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use overdrive_core::aggregate::{
    Allocation, AllocationSpecInput, DriverInput, ExecInput, Job, JobSpecInput, Node,
    NodeSpecInput, ResourcesInput,
};
use overdrive_core::id::ContentHash;
use proptest::prelude::*;
use rkyv::rancor;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn sample_job() -> Job {
    Job::from_spec(JobSpecInput {
        id: "payments".to_owned(),
        replicas: 3,
        resources: ResourcesInput { cpu_milli: 1500, memory_bytes: 512 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/opt/payments/bin/payments-server".to_string(),
            args: vec!["--port".to_string(), "8080".to_string()],
        }),
    })
    .expect("canonical JobSpecInput constructs a Job")
}

fn sample_node() -> Node {
    Node::new(NodeSpecInput {
        id: "worker-01".to_owned(),
        region: "eu-west-1".to_owned(),
        cpu_milli: 8000,
        memory_bytes: 16 * 1024 * 1024 * 1024,
    })
    .expect("canonical NodeSpecInput constructs a Node")
}

fn sample_allocation() -> Allocation {
    Allocation::new(AllocationSpecInput {
        id: "alloc-xyz".to_owned(),
        workload_id: "payments".to_owned(),
        node_id: "worker-01".to_owned(),
    })
    .expect("canonical AllocationSpecInput constructs an Allocation")
}

// ---------------------------------------------------------------------------
// rkyv round-trip — full archive → access → deserialize → equal
// ---------------------------------------------------------------------------

#[test]
fn job_rkyv_roundtrip_equals_original() {
    let original = sample_job();

    let bytes = original
        .archive_for_store()
        .expect("rkyv envelope serialization of canonical Job must succeed");

    let deserialized =
        Job::from_store_bytes(&bytes, std::path::Path::new("aggregate_roundtrip.redb"), None)
            .expect("envelope bytes must decode back to Job");

    assert_eq!(deserialized, original, "rkyv envelope round-trip must preserve Job equality");
}

/// `Job::spec_digest()` MUST equal SHA-256 over the raw payload bytes
/// (`rkyv::to_bytes(&job)`), NOT the envelope-wrapped bytes from
/// `archive_for_store`. Content-addressed identity depends only on
/// the logical payload — including the envelope discriminant byte
/// would make the digest shift on every envelope version bump.
#[test]
fn job_spec_digest_matches_raw_payload_hash() {
    let job = sample_job();

    let raw_bytes =
        rkyv::to_bytes::<rancor::Error>(&job).expect("rkyv serialization of Job must succeed");
    let hash_over_raw_bytes = ContentHash::of(raw_bytes.as_ref());

    let digest = job.spec_digest().expect("spec_digest of canonical Job must succeed");

    assert_eq!(
        digest, hash_over_raw_bytes,
        "spec_digest MUST equal SHA-256 over raw payload bytes — \
         content-addressed identity must be envelope-version-independent",
    );
}

/// Regression: `spec_digest` must differ from `SHA-256(archive_for_store)`
/// because `archive_for_store` includes the envelope discriminant byte.
/// The two hashing over the same logical payload must produce different
/// values — if they match, `spec_digest` has regressed to envelope-coupled
/// hashing.
#[test]
fn job_spec_digest_differs_from_envelope_hash() {
    let job = sample_job();

    let envelope_bytes =
        job.archive_for_store().expect("archive_for_store of canonical Job must succeed");
    let hash_over_envelope = ContentHash::of(envelope_bytes.as_ref());

    let digest = job.spec_digest().expect("spec_digest of canonical Job must succeed");

    assert_ne!(
        digest, hash_over_envelope,
        "spec_digest must NOT equal SHA-256 over envelope bytes — \
         content-addressed identity must be independent of the envelope \
         discriminant byte so it stays stable across version bumps",
    );
}

#[test]
fn job_rkyv_byte_identical_on_repeated_archival() {
    let original = sample_job();

    let first = rkyv::to_bytes::<rancor::Error>(&original).expect("first archival");
    let second = rkyv::to_bytes::<rancor::Error>(&original).expect("second archival");

    assert_eq!(
        first.as_slice(),
        second.as_slice(),
        "two rkyv archivals of the same Job must produce byte-identical output \
         (precondition for content-addressed identity per ADR-0002)"
    );
}

#[test]
fn node_rkyv_roundtrip_equals_original() {
    let original = sample_node();

    let bytes = rkyv::to_bytes::<rancor::Error>(&original)
        .expect("rkyv serialization of canonical Node must succeed");

    let archived = rkyv::access::<rkyv::Archived<Node>, rancor::Error>(&bytes)
        .expect("archived bytes must validate as ArchivedNode");

    let deserialized: Node = rkyv::deserialize::<Node, rancor::Error>(archived)
        .expect("ArchivedNode must deserialize back to Node");

    assert_eq!(deserialized, original, "rkyv round-trip must preserve Node equality");
}

#[test]
fn node_rkyv_byte_identical_on_repeated_archival() {
    let original = sample_node();

    let first = rkyv::to_bytes::<rancor::Error>(&original).expect("first archival");
    let second = rkyv::to_bytes::<rancor::Error>(&original).expect("second archival");

    assert_eq!(
        first.as_slice(),
        second.as_slice(),
        "two rkyv archivals of the same Node must produce byte-identical output"
    );
}

#[test]
fn allocation_rkyv_roundtrip_equals_original() {
    let original = sample_allocation();

    let bytes = rkyv::to_bytes::<rancor::Error>(&original)
        .expect("rkyv serialization of canonical Allocation must succeed");

    let archived = rkyv::access::<rkyv::Archived<Allocation>, rancor::Error>(&bytes)
        .expect("archived bytes must validate as ArchivedAllocation");

    let deserialized: Allocation = rkyv::deserialize::<Allocation, rancor::Error>(archived)
        .expect("ArchivedAllocation must deserialize back to Allocation");

    assert_eq!(deserialized, original, "rkyv round-trip must preserve Allocation equality");
}

#[test]
fn allocation_rkyv_byte_identical_on_repeated_archival() {
    let original = sample_allocation();

    let first = rkyv::to_bytes::<rancor::Error>(&original).expect("first archival");
    let second = rkyv::to_bytes::<rancor::Error>(&original).expect("second archival");

    assert_eq!(
        first.as_slice(),
        second.as_slice(),
        "two rkyv archivals of the same Allocation must produce byte-identical output"
    );
}

// ---------------------------------------------------------------------------
// serde-JSON round-trip — wire-lane only, separate from rkyv
// ---------------------------------------------------------------------------

#[test]
fn job_serde_json_roundtrip_equals_original() {
    let original = sample_job();

    let json = serde_json::to_string(&original).expect("serde-JSON serialization of Job");
    let back: Job = serde_json::from_str(&json).expect("serde-JSON deserialization of Job");

    assert_eq!(back, original, "serde-JSON round-trip must preserve Job equality");
}

#[test]
fn node_serde_json_roundtrip_equals_original() {
    let original = sample_node();

    let json = serde_json::to_string(&original).expect("serde-JSON serialization of Node");
    let back: Node = serde_json::from_str(&json).expect("serde-JSON deserialization of Node");

    assert_eq!(back, original, "serde-JSON round-trip must preserve Node equality");
}

#[test]
fn allocation_serde_json_roundtrip_equals_original() {
    let original = sample_allocation();

    let json = serde_json::to_string(&original).expect("serde-JSON serialization of Allocation");
    let back: Allocation =
        serde_json::from_str(&json).expect("serde-JSON deserialization of Allocation");

    assert_eq!(back, original, "serde-JSON round-trip must preserve Allocation equality");
}

// ---------------------------------------------------------------------------
// Non-substitutability — rkyv and serde-JSON are distinct canonicalisation
// lanes (per development.md "Hashing requires deterministic serialization"
// and ADR-0011 intent-vs-observation non-substitutability). Their byte
// outputs must differ for the same logical value, proving the two boundaries
// are independently addressed.
// ---------------------------------------------------------------------------

#[test]
fn rkyv_and_serde_json_are_non_substitutable() {
    let job = sample_job();

    let rkyv_bytes = rkyv::to_bytes::<rancor::Error>(&job).expect("rkyv archival of Job succeeds");
    let json_string = serde_json::to_string(&job).expect("serde-JSON of Job succeeds");

    assert_ne!(
        rkyv_bytes.as_slice(),
        json_string.as_bytes(),
        "rkyv canonical bytes and serde-JSON wire bytes MUST differ — they are \
         non-substitutable canonicalisation lanes per development.md hashing \
         guidance"
    );
}

// ---------------------------------------------------------------------------
// Proptest strategies
// ---------------------------------------------------------------------------

const ALPHA: &str = "abcdefghijklmnopqrstuvwxyz";
const ALNUM_DASH: &str = "abcdefghijklmnopqrstuvwxyz0123456789-";
const ALNUM: &str = "abcdefghijklmnopqrstuvwxyz0123456789";

/// Valid label matching the newtype's `^[a-z][a-z0-9-]{0,62}$`.
/// Same shape as the generator in `intent_key_canonical.rs`.
fn valid_label() -> impl Strategy<Value = String> {
    prop_oneof![
        proptest::sample::select(ALPHA.chars().collect::<Vec<_>>()).prop_map(|c| c.to_string()),
        (
            proptest::sample::select(ALPHA.chars().collect::<Vec<_>>()),
            prop::collection::vec(
                proptest::sample::select(ALNUM_DASH.chars().collect::<Vec<_>>()),
                0..=60,
            ),
            proptest::sample::select(ALNUM.chars().collect::<Vec<_>>()),
        )
            .prop_map(|(first, interior, last)| {
                let mut s = String::with_capacity(2 + interior.len());
                s.push(first);
                s.extend(interior);
                s.push(last);
                s
            }),
    ]
}

/// A region label — same shape as a label, re-used via `valid_label`.
fn valid_region() -> impl Strategy<Value = String> {
    valid_label()
}

/// An arbitrary valid `Job`.
fn arb_job() -> impl Strategy<Value = Job> {
    (valid_label(), 1u32..=1024, 0u32..=64_000, 1u64..=(128 * 1024 * 1024 * 1024)).prop_map(
        |(id, replicas, cpu_milli, memory_bytes)| {
            Job::from_spec(JobSpecInput {
                id,
                replicas,
                resources: ResourcesInput { cpu_milli, memory_bytes },
                driver: DriverInput::Exec(ExecInput {
                    command: "/bin/true".to_string(),
                    args: vec![],
                }),
            })
            .expect("generator yields valid Job")
        },
    )
}

/// An arbitrary valid `Node`.
fn arb_node() -> impl Strategy<Value = Node> {
    (valid_label(), valid_region(), 0u32..=128_000, 1u64..=(1024 * 1024 * 1024 * 1024)).prop_map(
        |(id, region, cpu_milli, memory_bytes)| {
            Node::new(NodeSpecInput { id, region, cpu_milli, memory_bytes })
                .expect("generator yields valid Node")
        },
    )
}

/// An arbitrary valid `Allocation`.
fn arb_allocation() -> impl Strategy<Value = Allocation> {
    (valid_label(), valid_label(), valid_label()).prop_map(|(id, workload_id, node_id)| {
        Allocation::new(AllocationSpecInput { id, workload_id, node_id })
            .expect("generator yields valid Allocation")
    })
}

// ---------------------------------------------------------------------------
// Proptest bodies — PROPTEST_CASES=1024 per .claude/rules/testing.md.
// These bodies close the "rkyv roundtrip" and "newtype roundtrip" mandatory
// call sites from testing.md.
// ---------------------------------------------------------------------------

proptest! {
    /// For any valid Job, rkyv round-trip preserves equality AND two
    /// archivals are byte-identical. This is the rkyv mandatory call site
    /// from testing.md.
    #[test]
    fn job_rkyv_roundtrip_and_byte_identity_property(job in arb_job()) {
        let first = rkyv::to_bytes::<rancor::Error>(&job)
            .expect("rkyv archival must succeed for any valid Job");
        let second = rkyv::to_bytes::<rancor::Error>(&job)
            .expect("second rkyv archival must succeed");

        prop_assert_eq!(first.as_slice(), second.as_slice(),
            "byte-identical re-archival — the canonical-hash precondition");

        let archived = rkyv::access::<rkyv::Archived<Job>, rancor::Error>(&first)
            .expect("archived bytes must validate");
        let back: Job = rkyv::deserialize::<Job, rancor::Error>(archived)
            .expect("archived Job must deserialize");

        prop_assert_eq!(back, job);
    }

    /// Same property for Node.
    #[test]
    fn node_rkyv_roundtrip_and_byte_identity_property(node in arb_node()) {
        let first = rkyv::to_bytes::<rancor::Error>(&node)
            .expect("rkyv archival must succeed for any valid Node");
        let second = rkyv::to_bytes::<rancor::Error>(&node)
            .expect("second rkyv archival must succeed");

        prop_assert_eq!(first.as_slice(), second.as_slice());

        let archived = rkyv::access::<rkyv::Archived<Node>, rancor::Error>(&first)
            .expect("archived bytes must validate");
        let back: Node = rkyv::deserialize::<Node, rancor::Error>(archived)
            .expect("archived Node must deserialize");

        prop_assert_eq!(back, node);
    }

    /// Same property for Allocation.
    #[test]
    fn allocation_rkyv_roundtrip_and_byte_identity_property(
        allocation in arb_allocation(),
    ) {
        let first = rkyv::to_bytes::<rancor::Error>(&allocation)
            .expect("rkyv archival must succeed for any valid Allocation");
        let second = rkyv::to_bytes::<rancor::Error>(&allocation)
            .expect("second rkyv archival must succeed");

        prop_assert_eq!(first.as_slice(), second.as_slice());

        let archived =
            rkyv::access::<rkyv::Archived<Allocation>, rancor::Error>(&first)
                .expect("archived bytes must validate");
        let back: Allocation =
            rkyv::deserialize::<Allocation, rancor::Error>(archived)
                .expect("archived Allocation must deserialize");

        prop_assert_eq!(back, allocation);
    }

    /// For any valid Job, serde-JSON round-trip preserves equality.
    #[test]
    fn job_serde_json_roundtrip_property(job in arb_job()) {
        let json = serde_json::to_string(&job)
            .expect("serde-JSON serialization must succeed");
        let back: Job = serde_json::from_str(&json)
            .expect("serde-JSON deserialization must succeed");
        prop_assert_eq!(back, job);
    }

    /// Same property for Node.
    #[test]
    fn node_serde_json_roundtrip_property(node in arb_node()) {
        let json = serde_json::to_string(&node)
            .expect("serde-JSON serialization must succeed");
        let back: Node = serde_json::from_str(&json)
            .expect("serde-JSON deserialization must succeed");
        prop_assert_eq!(back, node);
    }

    /// Same property for Allocation.
    #[test]
    fn allocation_serde_json_roundtrip_property(allocation in arb_allocation()) {
        let json = serde_json::to_string(&allocation)
            .expect("serde-JSON serialization must succeed");
        let back: Allocation = serde_json::from_str(&json)
            .expect("serde-JSON deserialization must succeed");
        prop_assert_eq!(back, allocation);
    }
}
