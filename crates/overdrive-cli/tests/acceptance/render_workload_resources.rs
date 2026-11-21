//! GH #372 (DD-7, AC-1.5, AC-2.8, D-2) — `overdrive workload describe` prints a workload's declared size in
//! the canonical strings: `Memory:        128MiB` and `CPU:           500m`, for Job, Service and Schedule
//! reads alike, from the one live renderer `render::workload_describe`.
//!
//! The snapshot is decoded from the `GET /v1/allocs` wire JSON (strings for `cpu` / `memory`), exactly
//! as `commands::workload::describe` receives it, then rendered through the live path
//! (`WorkloadDescribeOutput` -> `workload_describe`). Per `crates/overdrive-cli/CLAUDE.md` the live
//! path is the test surface. Evidence tier: unit / in-memory acceptance (layer 2).
//!
//! Pending bodies are complete and marked `#[should_panic(expected = ...)]`; DELIVER removes the marker.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use overdrive_cli::commands::workload::WorkloadDescribeOutput;
use overdrive_cli::render::workload_describe;
use overdrive_control_plane::api::AllocStatusResponse;
use overdrive_testing::placement;
use serde_json::{Value, json};

placement!(host_pure);

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn row(cpu: &str, memory: &str) -> Value {
    json!({
        "alloc_id": "alloc-dev-box-0", "workload_id": "dev-box", "node_id": "node-1",
        "state": "running",
        "resources": { "cpu": cpu, "memory": memory }
    })
}

/// Decode a snapshot from wire JSON, as the CLI's API client does.
fn snapshot(kind: &str, rows: &[Value]) -> AllocStatusResponse {
    let running = u32::try_from(rows.len()).expect("few rows");
    let wire = json!({
        "workload_id": "dev-box", "spec_digest": DIGEST,
        "replicas_desired": running, "replicas_running": running,
        "kind": kind, "rows": rows
    });
    serde_json::from_value(wire)
        .unwrap_or_else(|e| panic!("string cpu/memory resources are not accepted yet: {e}"))
}

/// The live command path's wrapper around a snapshot, then the live renderer.
fn render(response: AllocStatusResponse) -> String {
    let allocations_total = response.rows.len();
    let out = WorkloadDescribeOutput {
        workload_id: "dev-box".to_owned(),
        spec_digest: DIGEST.to_owned(),
        allocations_total,
        empty_state_message: String::new(),
        snapshot: response,
    };
    workload_describe(&out)
}

/// A key/value line of the describe body: label padded so the value starts at offset 15.
fn kv(label: &str, value: &str) -> String {
    format!("{label:<15}{value}")
}

fn has_line(output: &str, line: &str) -> bool {
    output.lines().any(|l| l == line)
}

/// AC-1.5 / DD-7: for Job, Service and Schedule reads the describe body carries `Memory:` and `CPU:`
/// lines holding the canonical strings (value column at offset 15), not raw bytes or milli-units; a
/// size declared as `1024MiB` / `2000m` reads back as `1GiB` / `2`. CONTRACT_SHAPE: pure-function.
#[test]
fn describe_prints_the_canonical_memory_and_cpu_lines_for_every_kind() {
    for kind in ["Job", "Service", "Schedule"] {
        for (cpu, memory, shown_cpu, shown_memory) in [
            ("500m", "128MiB", "500m", "128MiB"),
            ("2000m", "1024MiB", "2", "1GiB"),
            ("0.5", "1536MiB", "500m", "1536MiB"),
        ] {
            let out = render(snapshot(kind, &[row(cpu, memory)]));
            assert!(
                has_line(&out, &kv("Memory:", shown_memory)),
                "{kind}: describe must print `Memory:` with the canonical string {shown_memory}:\n{out}"
            );
            assert!(
                has_line(&out, &kv("CPU:", shown_cpu)),
                "{kind}: describe must print `CPU:` with the canonical string {shown_cpu}:\n{out}"
            );
            for raw in ["134217728", "1073741824", "1610612736"] {
                assert!(
                    !out.contains(raw),
                    "{kind}: describe must not print a raw byte count {raw}:\n{out}"
                );
            }
        }
    }
}

/// AC-1.5: a VM-driver Service with several allocation rows prints the size once, from the first row
/// (every row of a workload carries the same declared size), under the existing presence guard.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn describe_prints_the_size_once_for_a_multi_replica_workload() {
    let rows: Vec<Value> = (0..3).map(|_| row("500m", "128MiB")).collect();
    let out = render(snapshot("Service", &rows));
    assert_eq!(out.lines().filter(|l| l.starts_with("Memory:")).count(), 1, "{out}");
    assert_eq!(out.lines().filter(|l| l.starts_with("CPU:")).count(), 1, "{out}");
}

/// AC-2.8: describe prints a `CPU:` line for every workload kind, under the same presence guard as
/// `Memory:`, and a zero CPU renders as its canonical zero form `0` (not blank, not `0m`).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn describe_prints_a_zero_cpu_as_the_canonical_zero() {
    for kind in ["Job", "Service", "Schedule"] {
        let out = render(snapshot(kind, &[row("0", "128MiB")]));
        assert!(
            has_line(&out, &kv("CPU:", "0")),
            "{kind}: describe must print `CPU:` with the canonical zero 0:\n{out}"
        );
        assert!(
            has_line(&out, &kv("Memory:", "128MiB")),
            "{kind}: a zero CPU must not hide the Memory line:\n{out}"
        );
    }
}

/// The presence guard is unchanged: with no allocation row, or a row whose declared memory is zero (the
/// `From<AllocStatusRow>` default before the handler overlays the intent), there is no resolved size to
/// show, so neither line is printed. CONTRACT_SHAPE: pure-function.
#[test]
fn describe_omits_both_size_lines_without_a_resolved_size() {
    let out = render(snapshot("Service", &[]));
    assert!(
        !out.contains("Memory:") && !out.contains("CPU:"),
        "no allocation row: no size lines:\n{out}"
    );
    let out = render(snapshot("Service", &[row("0", "0B")]));
    assert!(!out.contains("Memory:"), "no resolved memory: no Memory line:\n{out}");
    assert!(
        !out.contains("CPU:"),
        "no resolved memory: no CPU line either (one presence guard):\n{out}"
    );
}
