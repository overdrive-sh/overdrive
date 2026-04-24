//! Acceptance tests for `Display` on observation-store row-adjacent
//! types exposed from `overdrive_core::traits::observation_store`.
//!
//! The `Display` impl on `AllocState` is load-bearing: handlers at
//! `overdrive-control-plane::handlers::submit_job` +
//! `alloc_status` render `AllocStatusRow.state` onto the REST wire via
//! `AllocState::to_string()` (see `AllocStatusRowBody::from(row)`).
//! An `Ok(Default::default())` mutation of the `fmt` body would emit an
//! empty string — callers would ship `state: ""` on the wire without
//! the test suite noticing.
//!
//! This test pins the canonical lowercase rendering defined in the
//! `AllocState::fmt` impl against every enum variant, so a mutation
//! that truncates `fmt` or swaps two variants fails.

use overdrive_core::traits::observation_store::AllocState;

#[test]
fn alloc_state_display_is_canonical_lowercase_per_whitepaper_section_4_and_14() {
    // (state, canonical rendering) — exhaustively covering every
    // variant. New variants added later MUST extend this table; a
    // compile-time `match` in production plus an explicit `Display`
    // assertion here is the belt-and-braces guard.
    let cases: &[(AllocState, &str)] = &[
        (AllocState::Pending, "pending"),
        (AllocState::Running, "running"),
        (AllocState::Draining, "draining"),
        (AllocState::Suspended, "suspended"),
        (AllocState::Terminated, "terminated"),
    ];

    for (state, expected) in cases {
        let rendered = state.to_string();
        assert_eq!(
            rendered, *expected,
            "AllocState::{state:?} must render as canonical lowercase `{expected}`; \
             got `{rendered}` — check the Display impl did not degrade to empty output",
        );
        // Belt-and-braces: a mutation that replaces the body with
        // `Ok(Default::default())` produces an empty string. The
        // emptiness-check here is an explicit trap for that shape.
        assert!(
            !rendered.is_empty(),
            "AllocState::{state:?} Display must not render as empty string",
        );
    }

    // Symmetry check — no two variants share a rendering.
    let renderings: Vec<String> = cases.iter().map(|(s, _)| s.to_string()).collect();
    let mut sorted = renderings.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        renderings.len(),
        sorted.len(),
        "each AllocState variant must render as a unique string; got {renderings:?}",
    );
}
