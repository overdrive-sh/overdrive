//! Schema-evolution golden-bytes test — `JobEnvelope`.
//!
//! RED scaffold per S-EV-01.5. Lands GREEN in DELIVER step 01-04
//! (when the `Job` rename through every call site lands).

#[test]
#[should_panic(expected = "RED scaffold")]
fn job_v1_decodes_through_current_envelope() {
    panic!(
        "Not yet implemented -- RED scaffold (S-EV-01.5 / JobEnvelope V1 golden-bytes roundtrip)"
    );
}
