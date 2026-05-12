//! Schema-evolution golden-bytes test — `AllocStatusRowEnvelope`.
//!
//! RED scaffold per S-EV-01.1. Lands GREEN in DELIVER step 01-02
//! (Walking Skeleton — `AllocStatusRowEnvelope` V1 roundtrip).

#[test]
#[should_panic(expected = "RED scaffold")]
fn alloc_status_row_v1_decodes_through_current_envelope() {
    panic!(
        "Not yet implemented -- RED scaffold (S-EV-01.1 / AllocStatusRowEnvelope V1 golden-bytes roundtrip)"
    );
}
