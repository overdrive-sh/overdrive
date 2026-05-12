//! Schema-evolution golden-bytes test — `NodeHealthRowEnvelope`.
//!
//! RED scaffold per S-EV-01.2. Lands GREEN in DELIVER step 02-01.

#[test]
#[should_panic(expected = "RED scaffold")]
fn node_health_row_v1_decodes_through_current_envelope() {
    panic!(
        "Not yet implemented -- RED scaffold (S-EV-01.2 / NodeHealthRowEnvelope V1 golden-bytes roundtrip)"
    );
}
