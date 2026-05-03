//! Acceptance scenarios for step 04-01 + 04-07 — Reconciler trait surface
//! and `ReconcilerName` / `TargetResource` newtype completeness.
//!
//! Per ADR-0013 amendments 2026-04-24: the trait migrated to the
//! pre-hydration + `TickContext` time-injection shape. The compile-time
//! signature pin now asserts
//! `fn(&R, &State, &State, &R::View, &TickContext) -> (Vec<Action>, R::View)`.
//! The twin-invocation unit test constructs ONE `TickContext` and passes
//! it to BOTH calls, asserting both tuples equal bit-for-bit.
//!
//! Every test enters through the driving port (`ReconcilerName::new`,
//! `TargetResource::new`, or the `Reconciler` trait surface) and asserts
//! observable outcomes (error variants, type signatures). No internal
//! state is peeked.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;
use std::time::{Duration, Instant};

use bytes::Bytes;
use proptest::prelude::*;

use overdrive_core::UnixInstant;
use overdrive_core::id::{ContentHash, CorrelationKey};
use overdrive_core::reconciler::{
    Action, HydrateError, LibsqlHandle, Reconciler, ReconcilerName, ReconcilerNameError,
    TargetResource, TargetResourceError, TickContext,
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
    let outcome = ReconcilerName::new("a");

    let name = outcome.expect("single lowercase letter must be accepted");
    assert_eq!(name.as_str(), "a");
}

#[test]
fn reconciler_name_new_accepts_maximum_length_63() {
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
// Reconciler trait — pre-hydration + TickContext contract (04-07)
//
// These are compile-time assertions. Per ADR-0013 §2 and §2c, the trait's
// pure-compute method takes `(desired, actual, view, tick) ->
// (Vec<Action>, NextView)`. `view` is the pre-hydrated async read
// (typed as the associated `R::View`); `tick` is the injected
// `TickContext` carrying wall-clock, monotonic tick counter, and
// per-tick deadline. Both parameters are passed by reference.
// ---------------------------------------------------------------------------

/// Compile-time pin of `Reconciler::reconcile`'s synchronous,
/// pre-hydration, time-injected signature.
///
/// The 5-parameter `fn(...)` type annotation IS the assertion. If the
/// trait regressed to `async fn`, took `&dyn Clock`, or reverted the
/// `NextView` tuple return, the right-hand side would fail to
/// typecheck.
// Factored into a type alias so the 5-parameter assertion stays readable
// and clippy::type_complexity does not fire. The alias IS the assertion
// — a regression that makes `reconcile` `async fn`, drops the
// `(Vec<Action>, R::View)` tuple return, or reverts the typed
// `Reconciler::State` associated type (ADR-0021) to a single placeholder
// `&State` parameter fails to typecheck at the binding site below.
type ReconcileFn<R> = fn(
    &R,
    &<R as Reconciler>::State,
    &<R as Reconciler>::State,
    &<R as Reconciler>::View,
    &TickContext,
) -> (Vec<Action>, <R as Reconciler>::View);

fn enforce_pure_sync_signature<R: Reconciler>() {
    #[allow(clippy::let_underscore_untyped, clippy::no_effect_underscore_binding)]
    let _reconcile: ReconcileFn<R> = <R as Reconciler>::reconcile;
    #[allow(clippy::let_underscore_untyped, clippy::no_effect_underscore_binding)]
    let _name: fn(&R) -> &ReconcilerName = <R as Reconciler>::name;
}

/// Minimal implementor used to prove the trait is inhabited with
/// `State = ()`, `View = ()`, and that `reconcile` returns
/// `(Vec<Action>, ())` directly. Per ADR-0021, `State` is now a typed
/// associated type rather than a single shared placeholder; a
/// reconciler with no `desired`/`actual` projection picks `State = ()`.
struct NoopReconciler {
    name: ReconcilerName,
}

impl Reconciler for NoopReconciler {
    type State = ();
    type View = ();

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    async fn hydrate(
        &self,
        _target: &overdrive_core::reconciler::TargetResource,
        _db: &LibsqlHandle,
    ) -> Result<Self::View, HydrateError> {
        Ok(())
    }

    fn reconcile(
        &self,
        _desired: &Self::State,
        _actual: &Self::State,
        _view: &Self::View,
        _tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        (vec![Action::Noop], ())
    }
}

#[test]
fn reconciler_trait_signature_is_synchronous_no_async_no_clock_param() {
    // Exercise the compile-time bound — if the trait were async, this
    // line would not compile.
    enforce_pure_sync_signature::<NoopReconciler>();

    let reconciler = NoopReconciler { name: ReconcilerName::new("noop-heartbeat").expect("valid") };

    // Per ADR-0021, `State` is per-reconciler; `NoopReconciler::State =
    // ()` so `desired`/`actual` are the unit value.
    let desired: () = ();
    let actual: () = ();
    let view: () = ();
    // Construct one `TickContext`. Test code is exempt from the
    // `Instant::now()` dst-lint ban (dst-lint only scans `src/**/*.rs`).
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    // `reconcile` returns `(Vec<Action>, Self::View)` directly — no `.await` needed.
    let (actions, _next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(actions, vec![Action::Noop]);
    assert_eq!(reconciler.name().as_str(), "noop-heartbeat");
}

#[test]
fn reconciler_twin_invocation_produces_identical_output() {
    // §18 purity: two invocations with the same `(desired, actual, view,
    // tick)` MUST produce byte-identical action vectors. Per the
    // ADR-0013 §2c amendment we construct ONE `TickContext` and pass it
    // to BOTH calls — time is an input to the pure function, shared
    // across the twin invocation.
    let reconciler = NoopReconciler { name: ReconcilerName::new("noop-heartbeat").expect("valid") };

    // Per ADR-0021, `State` is per-reconciler; `NoopReconciler::State = ()`.
    let desired: () = ();
    let actual: () = ();
    let view: () = ();
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    let (actions_a, next_view_a) = reconciler.reconcile(&desired, &actual, &view, &tick);
    let (actions_b, next_view_b) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert_eq!((actions_a, next_view_a), (actions_b, next_view_b));
}
