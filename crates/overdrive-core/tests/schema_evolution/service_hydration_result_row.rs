//! Schema-evolution golden-bytes test — `ServiceHydrationResultRowEnvelope`.
//!
//! RED scaffold per S-EV-01.3. Lands GREEN in DELIVER step 02-02.

#[test]
#[should_panic(expected = "RED scaffold")]
fn service_hydration_result_row_v1_decodes_through_current_envelope() {
    panic!(
        "Not yet implemented -- RED scaffold (S-EV-01.3 / ServiceHydrationResultRowEnvelope V1 golden-bytes roundtrip)"
    );
}
