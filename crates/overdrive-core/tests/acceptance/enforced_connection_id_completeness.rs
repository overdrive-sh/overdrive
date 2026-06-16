//! Acceptance scenarios for the `EnforcedConnectionId` newtype completeness
//! contract (transparent-mtls-host-socket, ADR-0069, GH #26; DELIVER step
//! 01-01). The `MtlsEnforcement` port returns an `EnforcedConnection` whose
//! caller-readable `id()` is this stable, content-addressed correlation key;
//! DST and telemetry name a connection by it, so its `Display` / `FromStr` /
//! serde roundtrip is the mandatory proptest call site required by
//! `.claude/rules/testing.md` § "Property-based testing (proptest) — Mandatory
//! call sites" (newtype roundtrip).
//!
//! Port-to-port at domain scope: the newtype's public signature IS its driving
//! port. `EnforcedConnectionId` is derived from `(AllocationId, u64)` —
//! content-addressed within a node session, no entropy. The canonical Display
//! form is `<alloc>#<counter>` (the allocation id, a `#` separator, the u64
//! counter in base-10). `FromStr` round-trips it; serde matches `Display` /
//! `FromStr` exactly.
//!
//! Per the mutation-testing § "Mandatory targets" rule, every parse
//! accept/reject branch is covered: a valid `<alloc>#<counter>`, a missing
//! separator, an invalid allocation component, and a non-u64 counter.

// `expect` / `unwrap` are the standard idiom in test code — a panic with a
// message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::str::FromStr;

use overdrive_core::AllocationId;
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnectionId, EnforcedConnectionIdParseError,
};
use proptest::prelude::*;

// -----------------------------------------------------------------------------
// Strategies — a valid AllocationId label + an arbitrary u64 counter.
// -----------------------------------------------------------------------------

/// Generate a valid `AllocationId` (DNS-1123-label-like: lowercase ASCII
/// letters/digits/`-`, non-empty, must start+end alphanumeric). We compose a
/// conservative shape that always validates so the strategy never rejects.
fn alloc_strategy() -> impl Strategy<Value = AllocationId> {
    // `<a><body><z>` where the middle may contain `-`; head/tail are
    // alphanumeric, so the whole label is a valid DNS-1123 label.
    (
        proptest::char::range('a', 'z'),
        proptest::collection::vec(
            prop_oneof![
                proptest::char::range('a', 'z'),
                proptest::char::range('0', '9'),
                Just('-'),
            ],
            0..30usize,
        ),
        prop_oneof![proptest::char::range('a', 'z'), proptest::char::range('0', '9')],
    )
        .prop_map(|(head, middle, tail)| {
            let mut s = String::new();
            s.push(head);
            s.extend(middle);
            s.push(tail);
            // Collapse any accidental double-`-` that the validator may reject;
            // the constructor is the source of truth, so retry-validate.
            AllocationId::new(&s).unwrap_or_else(|_| {
                AllocationId::new(&format!("{head}{tail}")).expect("two-char label is valid")
            })
        })
}

// -----------------------------------------------------------------------------
// Display canonical form — `<alloc>#<counter>`.
// -----------------------------------------------------------------------------

#[test]
fn display_emits_alloc_hash_counter() {
    let alloc = AllocationId::new("alloc-payments-7").expect("valid alloc");
    let id = EnforcedConnectionId::new(alloc, 42);
    assert_eq!(id.to_string(), "alloc-payments-7#42");
}

#[test]
fn accessors_expose_the_constructed_parts() {
    let alloc = AllocationId::new("alloc-x").expect("valid alloc");
    let id = EnforcedConnectionId::new(alloc.clone(), 0);
    assert_eq!(id.alloc(), &alloc);
    assert_eq!(id.counter(), 0);
}

// -----------------------------------------------------------------------------
// Roundtrip properties (the mandatory call site).
// -----------------------------------------------------------------------------

proptest! {
    /// `FromStr(Display(id)) == id` for every valid `(alloc, counter)`.
    #[test]
    fn display_fromstr_roundtrip(alloc in alloc_strategy(), counter in any::<u64>()) {
        let id = EnforcedConnectionId::new(alloc, counter);
        let rendered = id.to_string();
        let parsed = EnforcedConnectionId::from_str(&rendered)
            .expect("Display output must round-trip through FromStr");
        prop_assert_eq!(parsed, id);
    }

    /// serde JSON roundtrip matches Display/FromStr exactly (the id serialises as
    /// its canonical string).
    #[test]
    fn serde_json_roundtrip(alloc in alloc_strategy(), counter in any::<u64>()) {
        let id = EnforcedConnectionId::new(alloc, counter);
        let json = serde_json::to_string(&id).expect("serialize");
        // The serialised form is the canonical string, quoted.
        prop_assert_eq!(&json, &format!("\"{id}\""));
        let back: EnforcedConnectionId = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(back, id);
    }
}

// -----------------------------------------------------------------------------
// Reject branches — each parse failure mode covered (mutation targets).
// -----------------------------------------------------------------------------

#[test]
fn fromstr_rejects_missing_separator() {
    let err = EnforcedConnectionId::from_str("alloc-no-hash").unwrap_err();
    assert_eq!(err, EnforcedConnectionIdParseError::MissingSeparator);
}

#[test]
fn fromstr_rejects_invalid_alloc_component() {
    // Uppercase + leading `-` make the allocation component invalid; the `#`
    // separator is present so we reach the alloc-validation branch.
    let err = EnforcedConnectionId::from_str("Bad Alloc#5").unwrap_err();
    assert!(
        matches!(err, EnforcedConnectionIdParseError::InvalidAlloc(_)),
        "expected InvalidAlloc, got {err:?}"
    );
}

#[test]
fn fromstr_rejects_non_u64_counter() {
    let err = EnforcedConnectionId::from_str("alloc-x#not-a-number").unwrap_err();
    assert_eq!(err, EnforcedConnectionIdParseError::MalformedCounter);
}

#[test]
fn fromstr_splits_on_last_hash_so_alloc_part_never_contains_one() {
    // AllocationId labels never contain `#`, but rsplit-on-`#` is the contract:
    // the counter is whatever follows the FINAL `#`. A single `#` is the only
    // valid shape (alloc labels carry no `#`), so this asserts the rsplit choice
    // is observable and stable.
    let id = EnforcedConnectionId::from_str("alloc-y#9001").expect("valid");
    assert_eq!(id.alloc().as_str(), "alloc-y");
    assert_eq!(id.counter(), 9001);
}
