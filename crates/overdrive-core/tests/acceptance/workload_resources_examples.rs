//! GH #372 (AC-1.8, D-6) — the checked-in example specs speak the new grammar: every spec under
//! `examples/` parses through `WorkloadSpecInput::from_toml_str` (the parser `deploy` routes through)
//! with `cpu` and `memory` strings. A spec an operator copies from `examples/` must deploy; parsing
//! every one is the behavioural check, not a text search. The docs and the removed-name grep gate
//! (AC-2.7) belong to FINALIZE, not to a test. Evidence tier: unit (layer 1, default lane, reads the
//! checked-in files).
//! Pending bodies are complete and marked `#[should_panic(expected = ...)]`; DELIVER removes the marker.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::path::{Path, PathBuf};

use overdrive_core::aggregate::WorkloadSpecInput;

const REPO_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn files_under(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            files_under(&path, extension, out);
        } else if path.extension().is_some_and(|e| e == extension) {
            out.push(path);
        }
    }
}

/// AC-1.8: every workload spec under `examples/` parses with the human-readable `[resources]`, so a spec
/// an operator copies deploys. Specs without a `[resources]` table (none today) are not workload
/// specs and are skipped; at least thirty are expected so a moved directory cannot pass empty.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn every_example_spec_parses_with_the_human_readable_resources() {
    let mut specs = Vec::new();
    files_under(&Path::new(REPO_ROOT).join("examples"), "toml", &mut specs);
    let mut parsed = 0_usize;
    for path in &specs {
        let text = std::fs::read_to_string(path).expect("read example spec");
        if !text.contains("[resources]") {
            continue;
        }
        let spec = WorkloadSpecInput::from_toml_str(&text).unwrap_or_else(|e| {
            panic!(
                "every example spec must parse with the human-readable resources: {}: {e}",
                path.display()
            )
        });
        let resources = match &spec {
            WorkloadSpecInput::Job(j) => serde_json::to_value(&j.resources),
            WorkloadSpecInput::Service(s) => serde_json::to_value(&s.resources),
            WorkloadSpecInput::Schedule(s) => serde_json::to_value(&s.job_inner.resources),
        }
        .expect("the parsed resources serialise");
        assert!(
            resources["cpu"].is_string() && resources["memory"].is_string(),
            "every example spec must parse with the human-readable resources: {} carries {resources}",
            path.display()
        );
        parsed += 1;
    }
    assert!(
        parsed >= 30,
        "every example spec must parse with the human-readable resources; only {parsed} were found"
    );
}
