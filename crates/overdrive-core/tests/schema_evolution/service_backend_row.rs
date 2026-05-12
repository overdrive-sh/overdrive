//! Schema-evolution golden-bytes test — `ServiceBackendRowEnvelope`.
//!
//! RED scaffold per S-EV-01.4. Lands GREEN in DELIVER step 02-03.

#[test]
#[should_panic(expected = "RED scaffold")]
fn service_backend_row_v1_decodes_through_current_envelope() {
    panic!(
        "Not yet implemented -- RED scaffold (S-EV-01.4 / ServiceBackendRowEnvelope V1 golden-bytes roundtrip)"
    );
}
