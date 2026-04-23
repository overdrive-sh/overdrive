//! Acceptance scenarios for phase-1-control-plane-core step 01-03 —
//! `IntentKey` canonical derivation.
//!
//! Covers the §2.1 scenario 3 from
//! `docs/feature/phase-1-control-plane-core/distill/test-scenarios.md`:
//! canonical key derivation is byte-stable across calls, the canonical
//! form is `<collection>/<id>`, and no duplicate derivation exists anywhere
//! in the workspace (the shared-artifacts-registry entry from US-01).
//!
//! Per ADR-0011, `IntentKey` is the single source-of-truth — CLI and
//! server both call these functions. The grep-based test at the bottom of
//! this file enforces that no drift-prone second copy sneaks in.
//!
//! The proptests use the same `valid_label()` generator shape as
//! `tests/newtype_proptest.rs` — narrower than the full validator
//! (lowercase alnum + `-`, starts with a letter, up to 63 chars) but
//! comfortably within the underlying `validate_label` constraints.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;

use overdrive_core::aggregate::IntentKey;
use overdrive_core::id::{AllocationId, JobId, NodeId};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Happy-path derivation scenarios (AC bullets 1, 2)
// ---------------------------------------------------------------------------

#[test]
fn for_job_returns_jobs_slash_id_as_byte_sequence() {
    let id = JobId::from_str("payments").expect("canonical JobId parses");
    let key = IntentKey::for_job(&id);

    assert_eq!(
        key.as_bytes(),
        b"jobs/payments",
        "canonical JobId intent key must be `jobs/<id>` byte-for-byte"
    );
}

#[test]
fn for_job_returns_jobs_slash_id_as_str() {
    let id = JobId::from_str("payments").expect("canonical JobId parses");
    let key = IntentKey::for_job(&id);

    assert_eq!(key.as_str(), "jobs/payments");
}

#[test]
fn for_node_returns_nodes_slash_id() {
    let id = NodeId::from_str("worker-01").expect("canonical NodeId parses");
    let key = IntentKey::for_node(&id);

    assert_eq!(key.as_bytes(), b"nodes/worker-01");
    assert_eq!(key.as_str(), "nodes/worker-01");
}

#[test]
fn for_allocation_returns_allocations_slash_id() {
    let id = AllocationId::from_str("alloc-xyz").expect("canonical AllocationId parses");
    let key = IntentKey::for_allocation(&id);

    assert_eq!(key.as_bytes(), b"allocations/alloc-xyz");
    assert_eq!(key.as_str(), "allocations/alloc-xyz");
}

// ---------------------------------------------------------------------------
// Byte-stability — two calls on the same ID produce byte-identical output
// (AC bullet 1 second half).
// ---------------------------------------------------------------------------

#[test]
fn two_calls_produce_byte_identical_output_for_job() {
    let id = JobId::from_str("payments").expect("canonical JobId parses");
    let first = IntentKey::for_job(&id);
    let second = IntentKey::for_job(&id);

    assert_eq!(
        first.as_bytes(),
        second.as_bytes(),
        "IntentKey::for_job must be byte-stable across invocations"
    );
    assert_eq!(first.as_str(), second.as_str());
}

#[test]
fn two_calls_produce_byte_identical_output_for_node() {
    let id = NodeId::from_str("worker-01").expect("canonical NodeId parses");
    let first = IntentKey::for_node(&id);
    let second = IntentKey::for_node(&id);

    assert_eq!(first.as_bytes(), second.as_bytes());
}

#[test]
fn two_calls_produce_byte_identical_output_for_allocation() {
    let id = AllocationId::from_str("alloc-xyz").expect("canonical AllocationId parses");
    let first = IntentKey::for_allocation(&id);
    let second = IntentKey::for_allocation(&id);

    assert_eq!(first.as_bytes(), second.as_bytes());
}

// ---------------------------------------------------------------------------
// Proptest generator — valid label inputs.
//
// The AC references `^[a-z][a-z0-9-]{0,62}$` (narrower than the full
// `validate_label` surface in `id.rs` which also accepts `_`, `.`, and up
// to 253 chars). We stay within that narrow class deliberately — it keeps
// shrinking fast and covers every byte the `IntentKey` constructor needs
// to concatenate.
// ---------------------------------------------------------------------------

const ALPHA: &str = "abcdefghijklmnopqrstuvwxyz";
const ALNUM_DASH: &str = "abcdefghijklmnopqrstuvwxyz0123456789-";

/// A valid label matching the AC's `^[a-z][a-z0-9-]{0,62}$`.
///
/// Always leads with a letter (so the validator's "starts alphanumeric"
/// rule is satisfied) and forbids a trailing dash (so the validator's
/// "ends alphanumeric" rule is also satisfied). The `-` interior is
/// preserved, which is where drift-prone manual concatenation bugs tend
/// to hide.
fn valid_label() -> impl Strategy<Value = String> {
    prop_oneof![
        // Single-character case — just a leading letter.
        proptest::sample::select(ALPHA.chars().collect::<Vec<_>>()).prop_map(|c| c.to_string()),
        // Multi-character case: leading letter, interior body, terminal alnum.
        (
            proptest::sample::select(ALPHA.chars().collect::<Vec<_>>()),
            prop::collection::vec(
                proptest::sample::select(ALNUM_DASH.chars().collect::<Vec<_>>()),
                0..=60,
            ),
            proptest::sample::select("abcdefghijklmnopqrstuvwxyz0123456789".chars().collect::<Vec<_>>()),
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

// ---------------------------------------------------------------------------
// Proptest bodies (AC bullet 3).
//
// The default CI budget is PROPTEST_CASES=1024 per `.claude/rules/testing.md`;
// each proptest below inherits that via the env.
// ---------------------------------------------------------------------------

proptest! {
    /// For any valid JobId, `IntentKey::for_job(&id).as_bytes()` equals
    /// `format!("jobs/{}", id).as_bytes()` AND is stable across two
    /// invocations.
    #[test]
    fn for_job_is_stable_and_matches_format(raw in valid_label()) {
        let id = JobId::new(&raw).expect("generator yields valid JobId");
        let expected = format!("jobs/{id}");

        let first = IntentKey::for_job(&id);
        let second = IntentKey::for_job(&id);

        prop_assert_eq!(first.as_bytes(), expected.as_bytes());
        prop_assert_eq!(first.as_bytes(), second.as_bytes());
        prop_assert_eq!(first.as_str(), expected.as_str());
    }

    /// Same property for NodeId -> `nodes/<id>`.
    #[test]
    fn for_node_is_stable_and_matches_format(raw in valid_label()) {
        let id = NodeId::new(&raw).expect("generator yields valid NodeId");
        let expected = format!("nodes/{id}");

        let first = IntentKey::for_node(&id);
        let second = IntentKey::for_node(&id);

        prop_assert_eq!(first.as_bytes(), expected.as_bytes());
        prop_assert_eq!(first.as_bytes(), second.as_bytes());
        prop_assert_eq!(first.as_str(), expected.as_str());
    }

    /// Same property for AllocationId -> `allocations/<id>`.
    #[test]
    fn for_allocation_is_stable_and_matches_format(raw in valid_label()) {
        let id = AllocationId::new(&raw).expect("generator yields valid AllocationId");
        let expected = format!("allocations/{id}");

        let first = IntentKey::for_allocation(&id);
        let second = IntentKey::for_allocation(&id);

        prop_assert_eq!(first.as_bytes(), expected.as_bytes());
        prop_assert_eq!(first.as_bytes(), second.as_bytes());
        prop_assert_eq!(first.as_str(), expected.as_str());
    }
}

// ---------------------------------------------------------------------------
// Grep gate (AC bullet 4).
//
// The three collection prefixes (`"jobs/`, `"nodes/`, `"allocations/`)
// MUST appear as string-literal openers in exactly ONE production source
// file each — the `IntentKey::for_*` method body in `aggregate/mod.rs`.
// Any drift-prone second copy in production code violates US-01's
// shared-artifacts-registry entry for `intent_key`.
//
// We search for the open-quote + prefix (e.g. `"jobs/`) rather than the
// fully-closed `"jobs/"` because the production derivation uses
// `format!("jobs/{id}")` — the prefix opens a string literal but does
// not immediately close it. The open-quote pattern is also what keeps
// doc comments out of the match set: a `// GET /v1/jobs/{id}` comment
// has no leading `"`, and a rustdoc code-block `` ` `` fence also
// doesn't match.
//
// Scope:
//   * Scanned: `crates/*/src/**/*.rs` (production sources only).
//   * NOT scanned: `tests/`, `benches/`, `examples/`, `docs/` — tests and
//     fixtures legitimately use the prefix strings as consumer inputs
//     (e.g. `store.watch(b"jobs/")`); that is not drift.
//
// The test shells out to `rg` and degrades gracefully when it isn't on
// PATH — CI runners have it; bare dev shells occasionally don't.
// ---------------------------------------------------------------------------

/// Walk upward from `CARGO_MANIFEST_DIR` to find the workspace root. The
/// marker is the top-level `Cargo.toml` with `[workspace]`.
fn workspace_root() -> std::path::PathBuf {
    let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            let contents = std::fs::read_to_string(&cargo_toml).unwrap_or_default();
            if contents.contains("[workspace]") {
                return dir;
            }
        }
        if !dir.pop() {
            panic!("could not locate workspace root from {}", env!("CARGO_MANIFEST_DIR"));
        }
    }
}

/// Run `rg -c "<literal>" --type rust --glob 'crates/*/src/**/*.rs'` under
/// the workspace root and return the number of files that matched.
/// Returns `None` if `rg` isn't available — the test then short-circuits
/// with an informational message instead of failing.
fn count_files_matching(literal: &str) -> Option<Vec<std::path::PathBuf>> {
    use std::process::Command;

    let root = workspace_root();
    let output = Command::new("rg")
        .arg("--type")
        .arg("rust")
        .arg("--glob")
        .arg("crates/*/src/**/*.rs")
        .arg("--files-with-matches")
        .arg(literal)
        .current_dir(&root)
        .output()
        .ok()?;

    // rg exit 0 => matches found, 1 => no matches, 2 => error.
    if output.status.code() == Some(2) {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let paths = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| root.join(l))
        .collect::<Vec<_>>();
    Some(paths)
}

fn assert_literal_in_exactly_one_production_file(literal: &str, expected_suffix: &str) {
    let Some(files) = count_files_matching(literal) else {
        // `rg` not on PATH. Log and skip — CI has rg, local dev may not.
        eprintln!(
            "skipping grep-gate for {literal:?}: `rg` not available on PATH"
        );
        return;
    };

    assert_eq!(
        files.len(),
        1,
        "literal {literal:?} must appear in exactly one production `.rs` \
         file (the `IntentKey::for_*` body in `aggregate/mod.rs`); \
         found in {files:?}"
    );

    let only = &files[0];
    assert!(
        only.to_string_lossy().ends_with(expected_suffix),
        "literal {literal:?} must live in `{expected_suffix}`; \
         found in {only:?}"
    );
}

#[test]
fn jobs_prefix_appears_in_exactly_one_production_file() {
    assert_literal_in_exactly_one_production_file(
        "\"jobs/",
        "crates/overdrive-core/src/aggregate/mod.rs",
    );
}

#[test]
fn nodes_prefix_appears_in_exactly_one_production_file() {
    assert_literal_in_exactly_one_production_file(
        "\"nodes/",
        "crates/overdrive-core/src/aggregate/mod.rs",
    );
}

#[test]
fn allocations_prefix_appears_in_exactly_one_production_file() {
    assert_literal_in_exactly_one_production_file(
        "\"allocations/",
        "crates/overdrive-core/src/aggregate/mod.rs",
    );
}
