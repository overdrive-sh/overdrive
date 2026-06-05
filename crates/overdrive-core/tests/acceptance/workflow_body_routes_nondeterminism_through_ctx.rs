//! Slice 01 / US-WP-1 AC2 — every non-deterministic input flows through
//! `ctx`, never the ambient runtime (D-INH-4; the replay-equivalence
//! precondition).
//!
//! Scenario S-WP-01-03. O5 precondition. A `dst-lint`-style scan over the
//! workflow impl source finds no `Instant::now()` / `reqwest` /
//! `tokio::time::sleep` / `rand::*`; the side effect is performed through
//! `ctx.call(...).await` only. Negative testing: a body that smuggles a
//! non-`ctx` source is rejected (the failure case is asserted, not just
//! the happy case).
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! The `dst-lint`-style workflow-body scan does not exist yet. Two
//! `#[should_panic(expected = "RED scaffold")]` bodies pin both the
//! positive (clean body passes) and negative (smuggled source rejected)
//! halves of the contract; both compile and PASS at the bar without the
//! unbuilt scanner.

#[test]
#[should_panic(expected = "RED scaffold")]
fn clean_workflow_body_routes_all_nondeterminism_through_ctx() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-01-03 / a workflow body whose only side effect is ctx.call passes the dst-lint-style ctx-only scan)"
    );
}

#[test]
#[should_panic(expected = "RED scaffold")]
fn workflow_body_smuggling_non_ctx_nondeterminism_is_rejected() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-01-03 / a workflow body using Instant::now/reqwest/tokio::time::sleep/rand outside ctx is rejected by the scan -- negative testing)"
    );
}
