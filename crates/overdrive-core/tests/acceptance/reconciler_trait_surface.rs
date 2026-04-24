//! Acceptance scenarios for step 04-01 — Reconciler trait surface and
//! `ReconcilerName` / `TargetResource` newtype completeness.
//!
//! Translates the step 04-01 acceptance criteria from
//! `docs/feature/phase-1-control-plane-core/roadmap.json` into Rust tests.
//! Every test enters through the driving port (`ReconcilerName::new`,
//! `TargetResource::new`, or the `Reconciler` trait surface) and asserts
//! observable outcomes (error variants, type signatures). No internal
//! state is peeked.
//!
//! Per ADR-0013 §2 and whitepaper §18, the `Reconciler` trait is
//! synchronous by design — purity is load-bearing. The trait-shape
//! assertions use compile-time bounds: if `reconcile` were `async fn` or
//! took a `&dyn Clock` parameter, `_enforce_pure_sync_signature` below
//! would fail to compile.
//!
//! Per ADR-0013 §4 the `^[a-z][a-z0-9-]{0,62}$` regex is enforced by a
//! hand-rolled char-by-char check — NO regex crate dep on the core
//! compile graph. Proptest at `PROPTEST_CASES=1024` verifies the
//! `Display`->`FromStr` round-trip contract from `testing.md` §Newtype
//! roundtrip.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;
use std::time::Duration;

use bytes::Bytes;
use proptest::prelude::*;

use overdrive_core::id::{ContentHash, CorrelationKey};
use overdrive_core::reconciler::{
    Action, Db, Reconciler, ReconcilerName, ReconcilerNameError, State, TargetResource,
    TargetResourceError,
};

// ---------------------------------------------------------------------------
// ReconcilerName::new — acceptance criterion 1
// ---------------------------------------------------------------------------

#[test]
fn reconciler_name_new_accepts_valid_name() {
    let outcome = ReconcilerName::new("noop-heartbeat");

    let name = outcome.expect("valid kebab-case name must be accepted");
    assert_eq!(name.as_str(), "noop-heartbeat");
}

#[test]
fn reconciler_name_new_accepts_single_lowercase_letter() {
    // Minimum-length case — a single lowercase letter matches
    // `^[a-z][a-z0-9-]{0,62}$` at length 1.
    let outcome = ReconcilerName::new("a");

    let name = outcome.expect("single lowercase letter must be accepted");
    assert_eq!(name.as_str(), "a");
}

#[test]
fn reconciler_name_new_accepts_maximum_length_63() {
    // Upper-bound case — 63 chars (first + 62 interior) is exactly the
    // length ceiling from `^[a-z][a-z0-9-]{0,62}$`.
    let raw: String = "a".repeat(63);

    let outcome = ReconcilerName::new(&raw);

    let name = outcome.expect("name of length 63 must be accepted");
    assert_eq!(name.as_str(), raw.as_str());
}

#[test]
fn reconciler_name_new_accepts_digits_and_hyphens_after_lead() {
    let outcome = ReconcilerName::new("rec-1-v2");

    let name = outcome.expect("digits and hyphens after a lowercase lead must be accepted");
    assert_eq!(name.as_str(), "rec-1-v2");
}

#[test]
fn reconciler_name_new_rejects_empty() {
    let outcome = ReconcilerName::new("");

    match outcome {
        Err(ReconcilerNameError::Empty) => {}
        Err(other) => panic!("expected ReconcilerNameError::Empty, got {other:?}"),
        Ok(value) => panic!("empty input must not construct a ReconcilerName; got {value:?}"),
    }
}

#[test]
fn reconciler_name_new_rejects_uppercase_lead() {
    let outcome = ReconcilerName::new("A");

    match outcome {
        Err(ReconcilerNameError::InvalidLead) => {}
        Err(other) => panic!("expected ReconcilerNameError::InvalidLead, got {other:?}"),
        Ok(value) => panic!("uppercase lead must not construct a ReconcilerName; got {value:?}"),
    }
}

#[test]
fn reconciler_name_new_rejects_digit_lead() {
    let outcome = ReconcilerName::new("0abc");

    match outcome {
        Err(ReconcilerNameError::InvalidLead) => {}
        Err(other) => panic!("expected ReconcilerNameError::InvalidLead, got {other:?}"),
        Ok(value) => panic!("digit lead must not construct a ReconcilerName; got {value:?}"),
    }
}

#[test]
fn reconciler_name_new_rejects_hyphen_lead() {
    // A leading hyphen would let the name start with a path-traversal-ish
    // shape; reject it at the constructor.
    let outcome = ReconcilerName::new("-abc");

    match outcome {
        Err(ReconcilerNameError::InvalidLead) => {}
        Err(other) => panic!("expected ReconcilerNameError::InvalidLead, got {other:?}"),
        Ok(value) => panic!("hyphen lead must not construct a ReconcilerName; got {value:?}"),
    }
}

#[test]
fn reconciler_name_new_rejects_dot_character() {
    let outcome = ReconcilerName::new("a.b");

    match outcome {
        Err(ReconcilerNameError::ForbiddenCharacter { found }) => {
            assert_eq!(found, '.', "ForbiddenCharacter.found must carry the rejected char");
        }
        Err(other) => panic!("expected ReconcilerNameError::ForbiddenCharacter, got {other:?}"),
        Ok(value) => panic!("dot must not construct a ReconcilerName; got {value:?}"),
    }
}

#[test]
fn reconciler_name_new_rejects_double_dot_sequence() {
    let outcome = ReconcilerName::new("a..b");

    match outcome {
        Err(ReconcilerNameError::ForbiddenCharacter { found }) => {
            assert_eq!(found, '.');
        }
        Err(other) => panic!("expected ReconcilerNameError::ForbiddenCharacter, got {other:?}"),
        Ok(value) => panic!("'..' must not construct a ReconcilerName; got {value:?}"),
    }
}

#[test]
fn reconciler_name_new_rejects_forward_slash() {
    let outcome = ReconcilerName::new("a/b");

    match outcome {
        Err(ReconcilerNameError::ForbiddenCharacter { found }) => {
            assert_eq!(found, '/');
        }
        Err(other) => panic!("expected ReconcilerNameError::ForbiddenCharacter, got {other:?}"),
        Ok(value) => panic!("'/' must not construct a ReconcilerName; got {value:?}"),
    }
}

#[test]
fn reconciler_name_new_rejects_backslash() {
    let outcome = ReconcilerName::new("a\\b");

    match outcome {
        Err(ReconcilerNameError::ForbiddenCharacter { found }) => {
            assert_eq!(found, '\\');
        }
        Err(other) => panic!("expected ReconcilerNameError::ForbiddenCharacter, got {other:?}"),
        Ok(value) => panic!("'\\' must not construct a ReconcilerName; got {value:?}"),
    }
}

#[test]
fn reconciler_name_new_rejects_colon() {
    let outcome = ReconcilerName::new("a:b");

    match outcome {
        Err(ReconcilerNameError::ForbiddenCharacter { found }) => {
            assert_eq!(found, ':');
        }
        Err(other) => panic!("expected ReconcilerNameError::ForbiddenCharacter, got {other:?}"),
        Ok(value) => panic!("':' must not construct a ReconcilerName; got {value:?}"),
    }
}

#[test]
fn reconciler_name_new_rejects_uppercase_after_lead() {
    // Uppercase anywhere in the interior must be rejected — the
    // `ForbiddenCharacter` variant carries the offending letter so the
    // operator can find it.
    let outcome = ReconcilerName::new("aBc");

    match outcome {
        Err(ReconcilerNameError::ForbiddenCharacter { found }) => {
            assert_eq!(found, 'B');
        }
        Err(other) => panic!("expected ReconcilerNameError::ForbiddenCharacter, got {other:?}"),
        Ok(value) => panic!("uppercase must not construct a ReconcilerName; got {value:?}"),
    }
}

#[test]
fn reconciler_name_new_rejects_too_long() {
    let raw: String = "a".repeat(64);

    let outcome = ReconcilerName::new(&raw);

    match outcome {
        Err(ReconcilerNameError::TooLong { got }) => {
            assert_eq!(got, 64, "TooLong.got must carry the actual length");
        }
        Err(other) => panic!("expected ReconcilerNameError::TooLong, got {other:?}"),
        Ok(value) => panic!("64-char input must not construct a ReconcilerName; got {value:?}"),
    }
}

// ---------------------------------------------------------------------------
// ReconcilerName — FromStr + Display round-trip (acceptance criterion 2)
// ---------------------------------------------------------------------------

#[test]
fn reconciler_name_from_str_forwards_to_new() {
    let via_new = ReconcilerName::new("noop-heartbeat").expect("valid");
    let via_from_str = ReconcilerName::from_str("noop-heartbeat").expect("valid");

    assert_eq!(via_new, via_from_str);
}

#[test]
fn reconciler_name_from_str_propagates_errors() {
    let outcome = ReconcilerName::from_str("");

    match outcome {
        Err(ReconcilerNameError::Empty) => {}
        other => panic!("FromStr must propagate Empty error, got {other:?}"),
    }
}

// Proptest generator for `^[a-z][a-z0-9-]{0,62}$` — first char lowercase
// letter, interior chars lowercase letter/digit/hyphen, total length
// 1..=63.
fn valid_reconciler_name() -> impl Strategy<Value = String> {
    (
        proptest::sample::select("abcdefghijklmnopqrstuvwxyz".chars().collect::<Vec<_>>()),
        proptest::collection::vec(
            proptest::sample::select(
                "abcdefghijklmnopqrstuvwxyz0123456789-".chars().collect::<Vec<_>>(),
            ),
            0..=62,
        ),
    )
        .prop_map(|(first, interior)| {
            let mut s = String::with_capacity(1 + interior.len());
            s.push(first);
            s.extend(interior);
            s
        })
}

proptest! {
    /// For any valid name matching `^[a-z][a-z0-9-]{0,62}$`,
    /// `ReconcilerName::new(raw).map(|n| n.to_string())` round-trips to
    /// the original string, and `FromStr` agrees with `new`.
    #[test]
    fn reconciler_name_display_from_str_round_trips(raw in valid_reconciler_name()) {
        let via_new = ReconcilerName::new(&raw)
            .expect("generator yields only valid names");

        let rendered = via_new.to_string();
        prop_assert_eq!(&rendered, &raw);

        let reparsed = ReconcilerName::from_str(&rendered)
            .expect("Display output must be parseable by FromStr");
        prop_assert_eq!(reparsed, via_new);
    }
}

// ---------------------------------------------------------------------------
// TargetResource::new — acceptance criterion 3
// ---------------------------------------------------------------------------

#[test]
fn target_resource_new_accepts_job_shape() {
    let outcome = TargetResource::new("job/payments");

    let target = outcome.expect("canonical job/<id> must be accepted");
    assert_eq!(target.as_str(), "job/payments");
}

#[test]
fn target_resource_new_accepts_node_shape() {
    let outcome = TargetResource::new("node/n1");

    let target = outcome.expect("canonical node/<id> must be accepted");
    assert_eq!(target.as_str(), "node/n1");
}

#[test]
fn target_resource_new_accepts_alloc_shape() {
    let outcome = TargetResource::new("alloc/a1");

    let target = outcome.expect("canonical alloc/<id> must be accepted");
    assert_eq!(target.as_str(), "alloc/a1");
}

#[test]
fn target_resource_new_rejects_empty() {
    let outcome = TargetResource::new("");

    match outcome {
        Err(TargetResourceError::Empty) => {}
        Err(other) => panic!("expected TargetResourceError::Empty, got {other:?}"),
        Ok(value) => panic!("empty input must not construct a TargetResource; got {value:?}"),
    }
}

#[test]
fn target_resource_new_rejects_unknown_shape() {
    let outcome = TargetResource::new("garbage");

    match outcome {
        Err(TargetResourceError::UnknownShape { raw }) => {
            assert_eq!(raw, "garbage", "UnknownShape.raw must carry the rejected input");
        }
        Err(other) => panic!("expected TargetResourceError::UnknownShape, got {other:?}"),
        Ok(value) => panic!("unknown prefix must not construct a TargetResource; got {value:?}"),
    }
}

#[test]
fn target_resource_new_rejects_shape_missing_id() {
    // `job/` with no identifier is not a valid canonical form.
    let outcome = TargetResource::new("job/");

    match outcome {
        Err(TargetResourceError::UnknownShape { .. }) => {}
        Err(other) => panic!("expected TargetResourceError::UnknownShape, got {other:?}"),
        Ok(value) => panic!("'job/' with empty id must not construct; got {value:?}"),
    }
}

#[test]
fn target_resource_from_str_forwards_to_new() {
    let via_new = TargetResource::new("job/payments").expect("valid");
    let via_from_str = TargetResource::from_str("job/payments").expect("valid");

    assert_eq!(via_new, via_from_str);
}

#[test]
fn target_resource_display_renders_canonical_string() {
    // Display is part of the newtype completeness contract from
    // development.md §Newtypes — every newtype has `FromStr` + `Display`
    // matching exactly. A regression that replaces the Display body with
    // `Default::default()` must fail this assertion.
    let target = TargetResource::new("job/payments").expect("valid");

    assert_eq!(target.to_string(), "job/payments");
    assert_eq!(format!("{target}"), "job/payments");
}

// ---------------------------------------------------------------------------
// Action::HttpCall — acceptance criterion 4
// ---------------------------------------------------------------------------

#[test]
fn action_http_call_constructable_with_get_method_and_idempotency_key() {
    let correlation =
        CorrelationKey::derive("job/payments", &ContentHash::from_bytes([0u8; 32]), "register");

    let action = Action::HttpCall {
        correlation: correlation.clone(),
        target: "https://example.com/api".to_string(),
        method: "GET".to_string(),
        body: Bytes::new(),
        timeout: Duration::from_secs(30),
        idempotency_key: Some("k1".to_string()),
    };

    match action {
        Action::HttpCall {
            correlation: got_correlation,
            target,
            method,
            body,
            timeout,
            idempotency_key,
        } => {
            assert_eq!(got_correlation, correlation);
            assert_eq!(target, "https://example.com/api");
            assert_eq!(method, "GET");
            assert!(body.is_empty());
            assert_eq!(timeout, Duration::from_secs(30));
            assert_eq!(idempotency_key, Some("k1".to_string()));
        }
        other => panic!("expected Action::HttpCall, got {other:?}"),
    }
}

#[test]
fn action_http_call_constructable_with_post_method_and_no_idempotency_key() {
    let correlation =
        CorrelationKey::derive("node/n1", &ContentHash::from_bytes([1u8; 32]), "drain");

    let action = Action::HttpCall {
        correlation,
        target: "https://example.com/drain".to_string(),
        method: "POST".to_string(),
        body: Bytes::from_static(b"{}"),
        timeout: Duration::from_secs(5),
        idempotency_key: None,
    };

    match action {
        Action::HttpCall { method, idempotency_key, body, .. } => {
            assert_eq!(method, "POST");
            assert_eq!(idempotency_key, None);
            assert_eq!(body.as_ref(), b"{}");
        }
        other => panic!("expected Action::HttpCall, got {other:?}"),
    }
}

#[test]
fn action_noop_is_constructable() {
    let action = Action::Noop;

    assert!(matches!(action, Action::Noop));
}

// ---------------------------------------------------------------------------
// Reconciler trait — synchronous pure contract (acceptance criterion 5)
//
// These are compile-time assertions. If `reconcile` were `async fn` or
// its signature took a `&dyn Clock` / `Transport` / `Entropy` parameter,
// these bounds would fail to compile. The tests exist so a regression
// that weakens the trait surface is caught before merge.
// ---------------------------------------------------------------------------

/// Compile-time pin of `Reconciler::reconcile`'s synchronous signature.
/// Taking the function as a value with an explicit `fn(...)` type fails
/// to typecheck if the trait method is `async fn` (the pointer would be
/// `fn(...) -> impl Future<...>` or `fn(...) -> Pin<Box<dyn Future<...>>>`
/// depending on the `async fn in trait` lowering).
fn _enforce_pure_sync_signature<R: Reconciler>() {
    let _reconcile: fn(&R, &State, &State, &Db) -> Vec<Action> = <R as Reconciler>::reconcile;
    let _name: fn(&R) -> &ReconcilerName = <R as Reconciler>::name;
}

/// Minimal implementor used to prove the trait is inhabited and that
/// `reconcile` returns `Vec<Action>` directly (not a `Future`).
struct NoopReconciler {
    name: ReconcilerName,
}

impl Reconciler for NoopReconciler {
    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    fn reconcile(&self, _desired: &State, _actual: &State, _db: &Db) -> Vec<Action> {
        vec![Action::Noop]
    }
}

#[test]
fn reconciler_trait_signature_is_synchronous_no_async_no_clock_param() {
    // Exercise the compile-time bound — if the trait were async, this
    // line would not compile.
    _enforce_pure_sync_signature::<NoopReconciler>();

    let reconciler = NoopReconciler { name: ReconcilerName::new("noop-heartbeat").expect("valid") };

    let desired = State;
    let actual = State;
    let db = Db;

    // `reconcile` returns `Vec<Action>` directly — no `.await` needed.
    let actions = reconciler.reconcile(&desired, &actual, &db);

    assert_eq!(actions, vec![Action::Noop]);
    assert_eq!(reconciler.name().as_str(), "noop-heartbeat");
}

#[test]
fn reconciler_twin_invocation_produces_identical_output() {
    // §18 purity: `reconcile` is a pure function of its inputs. Two
    // invocations on the same `(desired, actual, db)` MUST produce
    // byte-identical action vectors. This is the shape the ADR-0017
    // `reconciler_is_pure` invariant will evaluate against the full
    // registry.
    let reconciler = NoopReconciler { name: ReconcilerName::new("noop-heartbeat").expect("valid") };

    let desired = State;
    let actual = State;
    let db = Db;

    let first = reconciler.reconcile(&desired, &actual, &db);
    let second = reconciler.reconcile(&desired, &actual, &db);

    assert_eq!(first, second);
}
