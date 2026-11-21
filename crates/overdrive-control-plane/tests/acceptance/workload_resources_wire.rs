//! GH #372 (DD-6, AC-1.7) — the API carries a workload's size as canonical strings: the
//! `ResourcesBody` of `GET /v1/allocs` rows decodes and encodes `cpu` / `memory` strings and refuses the
//! removed keys, and the generated OpenAPI document describes `cpu` and `memory` as `type: string` in
//! the three resource schemas. The submit-handler refusal lives in
//! `workload_resources_submit_refusal` (integration-gated, like `guest_listeners_http_rejection`).
//!
//! The wire is exercised as JSON text (so the file compiles before the field rename) and the OpenAPI
//! document through the generator the `openapi-gen` alias runs (`openapi::generate_yaml`). Evidence tier: unit (layer 1, default lane). Pending bodies are complete and marked
//! `#[should_panic(expected = ...)]`; DELIVER removes the marker.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use overdrive_control_plane::api::ResourcesBody;
use overdrive_control_plane::openapi;
use overdrive_testing::placement;
use serde_json::json;

placement!(host_pure);

/// DD-6: a row's `resources` block reads any accepted spelling and is emitted canonically:
/// `{"cpu":"0.5","memory":"1024MiB"}` is `{"cpu":"500m","memory":"1GiB"}`. CONTRACT_SHAPE: pure-function.
#[test]
fn the_resources_block_is_emitted_as_canonical_strings() {
    let body: ResourcesBody = serde_json::from_value(json!({ "cpu": "0.5", "memory": "1024MiB" }))
        .unwrap_or_else(|e| {
            panic!("the resources block with string cpu and memory must decode: {e}")
        });
    assert_eq!(
        serde_json::to_value(body).expect("serialises"),
        json!({ "cpu": "500m", "memory": "1GiB" })
    );
}

/// DD-6: the removed keys are not an alias: neither `{"cpu_milli", "memory_bytes"}` nor a mixed body
/// decodes, and the refusal names the unknown key. CONTRACT_SHAPE: pure-function.
#[test]
fn sad_the_resources_block_refuses_the_removed_keys() {
    for (wire, unknown) in [
        (json!({ "cpu_milli": 500, "memory_bytes": 134_217_728 }), "cpu_milli"),
        (json!({ "cpu_milli": 500, "memory": "128MiB" }), "cpu_milli"),
        (json!({ "cpu": "500m", "memory_bytes": 134_217_728 }), "memory_bytes"),
    ] {
        let err = serde_json::from_value::<ResourcesBody>(wire.clone())
            .expect_err("the removed keys must not decode")
            .to_string();
        assert!(err.contains(unknown), "the removed keys must not decode, naming {unknown}: {err}");
    }
}

/// The text of one schema under `components.schemas`: the key line and every deeper-indented line after it.
fn schema_block(yaml: &str, name: &str) -> String {
    let header = format!("    {name}:");
    let mut lines = yaml.lines().skip_while(|l| *l != header);
    let first = lines.next().unwrap_or_else(|| panic!("the OpenAPI document must define {name}"));
    let mut block = String::from(first);
    for line in lines {
        if !line.starts_with("     ") && !line.is_empty() {
            break;
        }
        block.push('\n');
        block.push_str(line);
    }
    block
}

/// AC-1.7 / DD-6: in `ResourcesInput`, `ResourcesBody` and `ResourceEnvelope` the properties are `cpu` and
/// `memory`, both `type: string`; no integer-typed resource property remains. Checked against the
/// generator's output; the existing OpenAPI gate holds `api/openapi.yaml` equal to it.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn openapi_describes_cpu_and_memory_as_strings_in_the_three_schemas() {
    let generated = openapi::generate_yaml().expect("generate_yaml must succeed");
    for schema in ["ResourcesInput", "ResourcesBody", "ResourceEnvelope"] {
        let block = schema_block(&generated, schema);
        assert!(
            block.contains("        cpu:\n          type: string")
                && block.contains("        memory:\n          type: string"),
            "the OpenAPI schema must describe cpu and memory as strings ({schema}):\n{block}"
        );
        assert!(
            !block.contains("integer"),
            "the OpenAPI schema must describe cpu and memory as strings, with no integer property left ({schema}):\n{block}"
        );
    }
}
