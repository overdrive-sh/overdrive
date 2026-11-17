//! The two documents the program keeps beside its code: `benchmarks/methodology.md`
//! (the versioned statistical protocol, #287 AC 1) and `benchmarks/catalogue.md`
//! (one row per benchmark). Evidence lane: integration (reads the repository).
//! Traces: US-BENCH-02 AC (g), US-BENCH-01 AC (f), DoD (methodology and catalogue).
use std::path::PathBuf;

fn benchmarks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks")
}

/// The methodology document states its version (1) and each topic #287 requires:
/// runner isolation, host eligibility, clock boundaries, cold and warm handling,
/// the sample and percentile method, the raw-result format and failure accounting.
/// CONTRACT_SHAPE: pure-function (document text in, verdict out).
#[test]
fn the_methodology_document_states_its_version_and_every_required_topic() {
    let path = benchmarks_dir().join("methodology.md");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("benchmarks/methodology.md must exist at {}", path.display()))
        .to_lowercase();
    assert!(
        text.contains("methodology version 1") || text.contains("methodology version: 1"),
        "the document must state methodology version 1"
    );
    for topic in [
        "runner isolation",
        "host eligibility",
        "clock",
        "cold",
        "warm",
        "percentile",
        "record format",
        "failure accounting",
    ] {
        assert!(text.contains(topic), "the methodology must cover `{topic}`");
    }
}

/// The catalogue has one row per declared benchmark naming its family, its workload
/// matrix, its clock boundary and its result owner. Edge: every family the program
/// ships has a row (the connect-hook benchmark is the first).
/// CONTRACT_SHAPE: pure-function (document text in, verdict out).
#[test]
fn the_catalogue_has_a_row_per_benchmark_with_its_workload_clock_and_owner() {
    let path = benchmarks_dir().join("catalogue.md");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("benchmarks/catalogue.md must exist at {}", path.display()));
    let row = text
        .lines()
        .find(|l| l.contains("cgroup-connect-hook") && l.contains("connect_hook_cost"))
        .expect("the catalogue must have a row for the connect-hook benchmark");
    let lower = row.to_lowercase();
    assert!(
        lower.contains("service-only") && lower.contains("service+gateway"),
        "the row names the boundary and its arms: {row}"
    );
    assert!(
        row.matches('|').count() >= 5,
        "the row carries family, boundary and arms, workload matrix, clock boundary and result owner: {row}"
    );
}
