//! Slice 01 / US-WP-1 AC1 (K6 metric) — a durable sequence body
//! contains zero step-machine boilerplate (the O3 structural promise,
//! mechanically asserted per Eclipse H1 / L1, NOT free-hand review).
//!
//! Scenario S-WP-01-02. K6 / O3. The AST/grep check counts step-enum
//! declarations and state-transition `match` arms in the workflow impl
//! body and asserts the count is zero.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! The K6 AST/grep check over the `ProvisionRecord` impl does not exist
//! yet. `#[should_panic(expected = "RED scaffold")]` keeps this RED-not-
//! BROKEN and compiling without the unbuilt check or workflow type.

#[test]
#[should_panic(expected = "RED scaffold")]
fn provision_record_body_has_zero_step_enum_and_zero_transition_match() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-01-02 / workflow body has zero step-enum decls and zero state-transition match arms -- the K6/O3 structural metric)"
    );
}
