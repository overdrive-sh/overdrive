//! The two documents the program keeps beside its code: `benchmarks/methodology.md`
//! (the versioned statistical protocol, #287 AC 1) and `benchmarks/catalogue.md`
//! (one row per benchmark). Evidence lane: integration (reads the repository).
//! Traces: US-BENCH-02 AC (g), US-BENCH-01 AC (f), DoD (methodology and catalogue).
use std::path::PathBuf;

fn benchmarks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks")
}

/// The methodology document covers each topic #287 requires (its stated version is pinned to
/// the runner's constant by a unit test in `record.rs`):
/// runner isolation, host eligibility, clock boundaries, cold and warm handling,
/// the sample and percentile method, the raw-result format and failure accounting.
/// CONTRACT_SHAPE: pure-function (document text in, verdict out).
#[test]
fn the_methodology_document_states_its_version_and_every_required_topic() {
    let path = benchmarks_dir().join("methodology.md");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("benchmarks/methodology.md must exist at {}", path.display()))
        .to_lowercase();
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
        lower.contains("no-hook") && lower.contains("gateway-hook"),
        "the row names the boundary and its arms: {row}"
    );
    assert!(
        row.matches('|').count() >= 5,
        "the row carries family, boundary and arms, workload matrix, clock boundary and result owner: {row}"
    );
}

// ---- The guest substrate in the two documents (guest-benchmark-substrate, DDD-3, DDD-8) ----

fn document(name: &str) -> String {
    let path = benchmarks_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("benchmarks/{name} must exist at {}", path.display()))
}

/// The text of the `## <heading>` section, to the next `## ` heading.
fn section_of(text: &str, heading: &str) -> Option<String> {
    let marker = format!("## {heading}");
    let start = text.find(&marker)?;
    let rest = &text[start + marker.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

/// The methodology has a `Guest substrate` section stating isolation (launched directly on the
/// exclusive host, never nested, fresh per replicate), eligibility (the full host list on the real
/// host, the guest's settings recorded as not visible from a guest), vCPU marking (unpinned),
/// staged artifacts (named by digest, `cargo xtask guest-kit-prepare` at the measured commit),
/// comparability (never compared across substrates), the `bpf_stats` rule and failure accounting
/// (`replicates_ok`). US-GBS-01 DoD; DDD-3.
/// CONTRACT_SHAPE: pure-function (document text in, verdict out).
#[test]
fn the_methodology_has_a_guest_substrate_section_stating_its_rules() {
    let text = document("methodology.md");
    let section = section_of(&text, "Guest substrate")
        .expect("the methodology must have a `## Guest substrate` section")
        .to_lowercase();
    for topic in [
        "never nested",
        "fresh",
        "unpinned",
        "not visible from a guest",
        "digest",
        "cargo xtask guest-kit-prepare",
        "never compared",
        "bpf_stats",
        "replicates_ok",
    ] {
        assert!(section.contains(topic), "the guest substrate section must state `{topic}`");
    }
}

/// Adding a substrate does not bump the methodology version (ADR-0327): the document still
/// states version 1 (the runner's constant is pinned to it by a unit test) and its version policy
/// says an addition for a new substrate does not bump it, while a change to or removal of a rule
/// does. The native run's requirement is word for word what it was, now substrate-conditional.
/// CONTRACT_SHAPE: pure-function (document text in, verdict out).
#[test]
fn the_methodology_keeps_version_1_and_says_a_new_substrate_does_not_bump_it() {
    let text = document("methodology.md").to_lowercase();
    assert!(text.contains("methodology version 1"), "the methodology still states version 1");
    assert!(
        text.contains("an addition for a new substrate does not"),
        "the version policy must say an addition for a new substrate does not bump the version"
    );
    assert!(
        text.contains("a native benchmark runs on the native substrate"),
        "the isolation rule is conditional on the substrate"
    );
}

/// The catalogue has a row for the guest family naming its boundary and arm, workload matrix,
/// clock boundary and result owner, and carrying the caveats the design pins: the two vCPUs are
/// unpinned, the unmeasured paths, no BPF statistics inside a guest. US-GBS-01 AC (g); DDD-8.
/// CONTRACT_SHAPE: pure-function (document text in, verdict out).
#[test]
fn the_catalogue_has_a_row_for_the_guest_family_with_its_workload_clock_owner_and_caveats() {
    let text = document("catalogue.md");
    let row = text
        .lines()
        .find(|line| {
            line.contains("guest-flow-forwarding") && line.contains("guest_flow_forwarding")
        })
        .expect("the catalogue must have a row for the guest family")
        .to_lowercase();
    assert!(
        row.matches('|').count() >= 5,
        "the row carries family, boundary and arms, workload matrix, clock boundary and result owner: {row}"
    );
    for part in [
        "forwarding",
        "forwarded_round_trip_ns",
        "2 vcpus",
        "512 mib",
        "fresh guest per replicate",
        "clock_monotonic",
        "overdrive-init",
        "overdrive-fwd",
        "#368",
        "#370",
        "#371",
    ] {
        assert!(row.contains(part), "the guest family's row must state `{part}`: {row}");
    }
}

/// The catalogue states the rule that a family that launches VMs is not a guest benchmark and
/// stays native until nested virtualization for VM workloads exists, so `vm-lifecycle` is a
/// native benchmark. US-GBS-01 AC (e); D9.
/// CONTRACT_SHAPE: pure-function (document text in, verdict out).
#[test]
fn the_catalogue_states_that_a_family_that_launches_vms_is_not_a_guest_benchmark() {
    let text = document("catalogue.md").to_lowercase();
    assert!(
        text.contains("not a guest benchmark"),
        "the catalogue must state that a family that launches VMs is not a guest benchmark"
    );
    assert!(text.contains("#347"), "the rule cites the issue that lifts it");
}
