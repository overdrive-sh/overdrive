//! Node-local persistent volume (GH #348, ADR-0315 operator surface):
//! `overdrive workload describe` shows the declared volume size and the guest
//! disk serial. Driven through the render function the CLI calls
//! (`render::workload_describe`) over a wire fixture, as the
//! `crates/overdrive-cli/CLAUDE.md` pure-render discipline requires. The fixture
//! is the JSON an operator's server returns, so it needs no scaffold: today the
//! unknown `volume` key is ignored and the section is missing.
//!
//! Evidence tier: unit (layer 1). CONTRACT_SHAPE: pure-function.

#![allow(clippy::expect_used, clippy::doc_markdown)]

use overdrive_cli::commands::workload::WorkloadDescribeOutput;
use overdrive_cli::render::workload_describe;
use overdrive_control_plane::api::AllocStatusResponse;
use overdrive_testing::placement;
use serde_json::{Value, json};

placement!(host_pure);

fn describe(volume: Option<Value>) -> String {
    let mut wire =
        json!({"workload_id": "dev-box", "replicas_desired": 1, "replicas_running": 0, "rows": []});
    if let Some(volume) = volume {
        wire["volume"] = volume;
    }
    let snapshot: AllocStatusResponse = serde_json::from_value(wire).expect("a describe response");
    workload_describe(&WorkloadDescribeOutput {
        workload_id: "dev-box".to_owned(),
        spec_digest: "0".repeat(64),
        allocations_total: 0,
        empty_state_message: String::new(),
        snapshot,
    })
}

/// A workload with a volume shows its size and the static guest-disk serial.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn describe_shows_the_volume_size_and_guest_serial() {
    let text = describe(Some(json!({"size": "40GiB"})));
    assert!(
        text.contains("Volume") && text.contains("40GiB") && text.contains("ovd-volume"),
        "the describe output must show the declared volume, got:\n{text}"
    );
}

/// A larger size is shown in its canonical spelling, as the server sent it.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn describe_shows_a_terabyte_volume_verbatim() {
    let text = describe(Some(json!({"size": "2TiB"})));
    assert!(
        text.contains("2TiB"),
        "the describe output must show the declared volume, got:\n{text}"
    );
}

/// GUARD (boundary 6, feature disabled): a workload without a volume prints no
/// volume line, so the pre-feature output is unchanged.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn guard_a_workload_without_a_volume_prints_no_volume_section() {
    let text = describe(None);
    assert!(!text.contains("Volume"), "no volume declared, none shown:\n{text}");
}
