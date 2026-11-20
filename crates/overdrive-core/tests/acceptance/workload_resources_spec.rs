//! GH #372 — a workload's `[resources]` carries `cpu` and `memory` as human-readable strings
//! (US-1, US-2; DD-1 .. DD-8; ADR-0339 .. ADR-0343).
//!
//! Driven through the library entry points an operator's spec reaches:
//!
//! * `WorkloadSpecInput::from_toml_str` — the TOML parser (`deploy` and the server both route
//!   through it); the "refused at parse time" ACs (AC-1.2, AC-1.3, AC-2.2, AC-2.3);
//! * `Job::from_submit` / `Service::from_submit` — the aggregate admission site (the whole-MiB
//!   rule, DD-8; zero memory; D-7 zero CPU);
//! * `From<&Job> for JobSpecInput` / `Service::to_describe` — the reverse projection `describe`
//!   renders from (the canonical form is read back, AC-1.9);
//! * `WorkloadIntent::archive_for_store` / `spec_digest` — equal quantities, one digest.
//!
//! Every test is written against surface that exists today (TOML text and JSON text in, JSON text
//! and typed errors out), so it compiles before the field rename and fails on its own assertion.
//! The renamed struct fields (`Resources { cpu, memory }`, `NodeSpecInput`) have no test here; see
//! the feature delta's "AC-1.10 coverage" note.
//!
//! Evidence tier: unit / in-memory acceptance (layer 1-2). Pending bodies are complete and marked
//! `#[should_panic(expected = ...)]`; DELIVER removes the marker.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use overdrive_core::aggregate::{
    AggregateError, Job, JobSpecInput, ParseError, Service, WorkloadIntent, WorkloadSpecInput,
};
use overdrive_core::api::submit::ServiceSpecInput;
use overdrive_core::byte_size::ByteSize;
use overdrive_core::cpu_quantity::CpuQuantity;
use serde_json::{Value, json};

// ----- fixtures: spec text and wire text -------------------------------------------------------

const VM_TABLE: &str =
    "[vm]\ncommand = \"/bin/true\"\nargs = []\nkernel = \"/kernel\"\nrootfs = \"/rootfs\"\n";

fn job_toml(resources_body: &str) -> String {
    format!("[job]\nid = \"payments\"\n\n{VM_TABLE}\n[resources]\n{resources_body}\n")
}

fn service_toml(resources_body: &str) -> String {
    format!(
        "[service]\nid = \"dev-box\"\nreplicas = 1\n\n[[listener]]\nport = 8080\nprotocol = \"tcp\"\n\n{VM_TABLE}\n[resources]\n{resources_body}\n"
    )
}

fn schedule_toml(resources_body: &str) -> String {
    format!(
        "[job]\nid = \"nightly\"\n\n{VM_TABLE}\n[resources]\n{resources_body}\n\n[schedule]\ncron = \"0 2 * * *\"\n"
    )
}

fn good(cpu: &str, memory: &str) -> String {
    format!("cpu = \"{cpu}\"\nmemory = \"{memory}\"")
}

fn job_json(cpu: &str, memory: &str) -> Value {
    json!({
        "id": "payments", "replicas": 1,
        "resources": { "cpu": cpu, "memory": memory },
        "vm": { "command": "/bin/true", "args": [], "kernel": "/kernel", "rootfs": "/rootfs" }
    })
}

fn service_json(cpu: &str, memory: &str) -> Value {
    json!({
        "id": "dev-box", "replicas": 1,
        "resources": { "cpu": cpu, "memory": memory },
        "vm": { "command": "/bin/true", "args": [], "kernel": "/kernel", "rootfs": "/rootfs" },
        "listeners": [{ "port": 8080, "protocol": "tcp" }]
    })
}

/// The `[resources]` the parser carries, as the JSON it would put on the wire.
fn parsed_resources(spec: &WorkloadSpecInput) -> Value {
    match spec {
        WorkloadSpecInput::Job(j) => serde_json::to_value(&j.resources),
        WorkloadSpecInput::Service(s) => serde_json::to_value(&s.resources),
        WorkloadSpecInput::Schedule(s) => serde_json::to_value(&s.job_inner.resources),
    }
    .expect("the parsed resources serialise")
}

/// Wire JSON -> `JobSpecInput`. The wire shape under test is the new string shape. Until DELIVER
/// accepts string `cpu` / `memory`, every test that reaches this helper stops here: the string wire
/// is the semantic RED of the feature, and the assertions after it are unreachable until it exists.
fn job_input(cpu: &str, memory: &str) -> JobSpecInput {
    serde_json::from_value(job_json(cpu, memory)).unwrap_or_else(|e| {
        panic!("string cpu/memory resources are not accepted yet ({cpu} / {memory}): {e}")
    })
}

fn service_input(cpu: &str, memory: &str) -> ServiceSpecInput {
    serde_json::from_value(service_json(cpu, memory)).unwrap_or_else(|e| {
        panic!("string cpu/memory resources are not accepted yet ({cpu} / {memory}): {e}")
    })
}

fn admit_job(cpu: &str, memory: &str) -> Result<Job, AggregateError> {
    Job::from_submit(job_input(cpu, memory))
}

fn admit_service(cpu: &str, memory: &str) -> Result<Service, AggregateError> {
    Service::from_submit(service_input(cpu, memory))
}

/// The intent-side resources of an admitted workload, as JSON (`Resources` serialises its fields).
fn intent_resources(job: &Job) -> Value {
    serde_json::to_value(job.resources).expect("the intent resources serialise")
}

fn parse_err(text: &str) -> String {
    match WorkloadSpecInput::from_toml_str(text) {
        Ok(spec) => panic!("the parser must refuse this spec at parse time; it accepted {spec:?}"),
        Err(e) => e.to_string(),
    }
}

/// A refusal surfaced through the TOML parser: it names the offending key, never points the
/// operator at a removed key, and carries the wording DESIGN pins for the rejection class (when it
/// pins one). The keyword is compared case-insensitively.
fn assert_refused_naming(label: &str, text: &str, key: &str, keyword: Option<&str>) {
    let message = parse_err(text).to_lowercase();
    assert!(
        message.contains(key),
        "the refusal must name the offending key {key} ({label}): {message}"
    );
    assert!(
        !message.contains("cpu_milli") && !message.contains("memory_bytes"),
        "the refusal must not point the operator at a removed key ({label}): {message}"
    );
    if let Some(keyword) = keyword {
        assert!(
            message.contains(&keyword.to_lowercase()),
            "the refusal must carry the class wording {keyword:?} ({label}): {message}"
        );
    }
}

// ----- US-1 / US-2: the new keys parse, in all three kinds --------------------------------------

/// AC-1.1 / AC-1.9 / AC-2.1 / AC-2.4 / AC-2.5: `[resources] cpu` and `memory` parse for Job, Service
/// and Schedule alike (one shared `ResourcesInput`) and read back as the canonical strings
/// whatever the operator wrote: `500m` / `128MiB` as written, `1024MiB` is `1GiB`, `2000m` and
/// `2.0` are `2`, `0.5` is `500m`. This is the one test whose RED is the TOML parser itself
/// (`unknown field cpu`). CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "a spec in the new resources shape must parse")]
fn a_non_canonical_spelling_is_read_back_canonically() {
    for (cpu, memory, canon_cpu, canon_memory) in [
        ("500m", "128MiB", "500m", "128MiB"),
        ("0.5", "1024MiB", "500m", "1GiB"),
        ("2000m", "1073741824B", "2", "1GiB"),
        ("2.0", "1GiB", "2", "1GiB"),
        ("0.500", "128mib", "500m", "128MiB"),
    ] {
        let body = good(cpu, memory);
        for (kind, text) in [
            ("job", job_toml(&body)),
            ("service", service_toml(&body)),
            ("schedule", schedule_toml(&body)),
        ] {
            let spec = WorkloadSpecInput::from_toml_str(&text).unwrap_or_else(|e| {
                panic!(
                    "a spec in the new resources shape must parse ({kind}, {cpu} / {memory}): {e}"
                )
            });
            assert_eq!(
                parsed_resources(&spec),
                json!({ "cpu": canon_cpu, "memory": canon_memory }),
                "the {kind} spec {cpu} / {memory} must read back canonically"
            );
        }
    }
}

// ----- AC-1.2 / AC-2.2 / D-1: the removed keys are refused, naming the unknown key -----------

/// AC-1.2 / AC-2.2 / D-1: `cpu_milli` and `memory_bytes` are refused at parse time in every kind,
/// alone or beside the new keys, naming the unknown key and the expected `cpu` and `memory`.
/// There is no alias and no fallback. CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the removed key must be refused at parse time")]
fn sad_the_removed_keys_are_refused_naming_the_unknown_key_and_the_expected_ones() {
    let cases: &[(&str, &str)] = &[
        ("cpu_milli = 500\nmemory_bytes = 134217728", "cpu_milli"),
        ("cpu = \"500m\"\nmemory = \"128MiB\"\ncpu_milli = 500", "cpu_milli"),
        ("cpu = \"500m\"\nmemory = \"128MiB\"\nmemory_bytes = 134217728", "memory_bytes"),
        ("cpu = \"500m\"\nmemory_bytes = 134217728", "memory_bytes"),
        ("cpu_milli = 500\nmemory = \"128MiB\"", "cpu_milli"),
    ];
    for (body, unknown) in cases {
        for (kind, text) in [
            ("job", job_toml(body)),
            ("service", service_toml(body)),
            ("schedule", schedule_toml(body)),
        ] {
            let err = match WorkloadSpecInput::from_toml_str(&text) {
                Ok(spec) => panic!(
                    "the removed key must be refused at parse time ({kind}, {unknown}); it accepted {spec:?}"
                ),
                Err(e) => e,
            };
            assert!(matches!(err, ParseError::Field { .. }), "{kind}: {err:?}");
            let text = err.to_string();
            assert!(
                text.contains(unknown),
                "the refusal must name the unknown key {unknown} ({kind}): {text}"
            );
            assert!(
                text.contains("`cpu`") && text.contains("`memory`"),
                "the refusal must name the expected keys cpu and memory ({kind}): {text}"
            );
        }
    }
}

/// AC-1.2: a spec missing `cpu` or `memory` is refused (both are required), naming the missing key.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the missing key must be named")]
fn sad_a_missing_cpu_or_memory_is_refused_naming_it() {
    for (body, missing) in [("memory = \"128MiB\"", "cpu"), ("cpu = \"500m\"", "memory")] {
        let text = parse_err(&service_toml(body));
        assert!(
            text.contains(&format!("missing field `{missing}`")),
            "the missing key must be named ({missing}): {text}"
        );
        assert!(
            !text.contains("cpu_milli") && !text.contains("memory_bytes"),
            "the missing key must be named without pointing the operator at a removed key: {text}"
        );
    }
}

// ----- AC-1.3 / AC-2.3: malformed values are refused at parse time ---------------------------

/// AC-1.3 / D-4: each malformed memory value, including a bare TOML integer, is refused at parse
/// time naming the `memory` key and, where DESIGN pins wording for the class, that wording: the
/// five binary units for text outside the grammar and for a decimal unit, "smaller" for a
/// fractional count, "too large" for overflow. A bare number is never read as bytes.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the refusal must not point the operator at a removed key")]
fn sad_each_malformed_memory_is_refused_at_parse_time_naming_the_grammar() {
    let rows: &[(&str, &str, Option<&str>)] = &[
        ("empty", "\"\"", Some("MiB")),
        ("decimal unit", "\"128MB\"", Some("MiB")),
        ("negative", "\"-1MiB\"", Some("MiB")),
        ("words", "\"128 megs\"", Some("MiB")),
        ("fractional", "\"1.5MiB\"", Some("smaller")),
        ("overflow", "\"16777216TiB\"", Some("too large")),
        ("bare number string", "\"128\"", None),
        ("bare TOML integer", "134217728", None),
    ];
    for (label, value, keyword) in rows {
        let text = service_toml(&format!("cpu = \"500m\"\nmemory = {value}"));
        assert_refused_naming(label, &text, "memory", *keyword);
    }
}

/// AC-2.3: each malformed cpu value, including a bare TOML number, is refused at parse time naming
/// the `cpu` key and, where DESIGN pins wording for the class, that wording: "lowercase" for an uppercase milli suffix, "three" for four fractional digits,
/// "too large" for overflow. CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the refusal must not point the operator at a removed key")]
fn sad_each_malformed_cpu_is_refused_at_parse_time_naming_the_grammar() {
    let rows: &[(&str, &str, Option<&str>)] = &[
        ("empty", "\"\"", None),
        ("negative", "\"-1\"", None),
        ("words", "\"five hundred\"", None),
        ("uppercase milli", "\"500M\"", Some("lowercase")),
        ("four decimals", "\"1.0005\"", Some("three")),
        ("overflow", "\"4294967296m\"", Some("too large")),
        ("unknown suffix", "\"500mm\"", None),
        ("bare TOML integer", "500", None),
        ("bare TOML float", "0.5", None),
    ];
    for (label, value, keyword) in rows {
        let text = service_toml(&format!("cpu = {value}\nmemory = \"128MiB\""));
        assert_refused_naming(label, &text, "cpu", *keyword);
    }
}

// ----- AC-1.1 / AC-2.1: admission, and the typed intent resources -------------------------------

/// AC-1.1 / AC-2.1: an admitted Job and Service carry the declared size in the intent
/// `Resources`; the canonical strings denote the pinned quantities (memory 128MiB is 134217728
/// bytes, cpu 500m is 500 millicores). CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "string cpu/memory resources are not accepted yet")]
fn an_admitted_workload_carries_the_declared_size_in_its_intent_resources() {
    let job = admit_job("500m", "128MiB").expect("the declared size is admitted");
    assert_eq!(intent_resources(&job), json!({ "cpu": "500m", "memory": "128MiB" }));
    let service = admit_service("500m", "128MiB").expect("the declared size is admitted");
    assert_eq!(
        serde_json::to_value(service.resources).expect("serialises"),
        json!({ "cpu": "500m", "memory": "128MiB" })
    );
    assert_eq!("128MiB".parse::<ByteSize>().expect("memory parses").bytes(), 134_217_728);
    assert_eq!("500m".parse::<CpuQuantity>().expect("cpu parses").millis(), 500);
}

/// What admission does with one `(cpu, memory)` pair.
enum Admission {
    /// Admitted; the intent carries these canonical strings.
    Admitted { cpu: &'static str, memory: &'static str },
    /// Refused: zero memory, field `memory`, "non-zero".
    ZeroMemory,
    /// Refused: not a whole number of MiB, field `memory`, naming the rule and the received
    /// canonical value.
    NotWholeMib(&'static str),
}

/// DD-8 / D-7 / AC-1.1 / AC-2.1: workload memory is admitted only as a non-zero whole number of
/// MiB at both admission sites (`1024KiB` is admitted, canonical `1MiB`; `1048575B` and `1048577B`
/// straddle the MiB boundary and are refused), and zero CPU stays admitted. A refusal names the
/// field `memory`, the rule, and the received canonical value. CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "string cpu/memory resources are not accepted yet")]
fn sad_admission_follows_the_non_zero_and_whole_mib_rules_at_both_sites() {
    use Admission::{Admitted, NotWholeMib, ZeroMemory};
    let rows: &[(&str, &str, Admission)] = &[
        ("500m", "128MiB", Admitted { cpu: "500m", memory: "128MiB" }),
        ("500m", "1MiB", Admitted { cpu: "500m", memory: "1MiB" }),
        ("500m", "1024KiB", Admitted { cpu: "500m", memory: "1MiB" }),
        ("500m", "1GiB", Admitted { cpu: "500m", memory: "1GiB" }),
        ("0", "128MiB", Admitted { cpu: "0", memory: "128MiB" }),
        ("0m", "128MiB", Admitted { cpu: "0", memory: "128MiB" }),
        ("0.0", "128MiB", Admitted { cpu: "0", memory: "128MiB" }),
        ("500m", "0MiB", ZeroMemory),
        ("500m", "0B", ZeroMemory),
        ("500m", "0GiB", ZeroMemory),
        ("500m", "1000B", NotWholeMib("1000B")),
        ("500m", "1KiB", NotWholeMib("1KiB")),
        ("500m", "4097B", NotWholeMib("4097B")),
        ("500m", "1536KiB", NotWholeMib("1536KiB")),
        ("500m", "1048575B", NotWholeMib("1048575B")),
        ("500m", "1048577B", NotWholeMib("1048577B")),
    ];
    for (cpu, memory, expected) in rows {
        let outcomes = [
            (
                "Job",
                admit_job(cpu, memory)
                    .map(|j| serde_json::to_value(j.resources).expect("serialises")),
            ),
            (
                "Service",
                admit_service(cpu, memory)
                    .map(|s| serde_json::to_value(s.resources).expect("serialises")),
            ),
        ];
        for (site, outcome) in outcomes {
            match (expected, outcome) {
                (Admission::Admitted { cpu: c, memory: m }, Ok(resources)) => {
                    assert_eq!(
                        resources,
                        json!({ "cpu": c, "memory": m }),
                        "{site}: {cpu} / {memory}"
                    );
                }
                (Admission::ZeroMemory, Err(AggregateError::Validation { field, message })) => {
                    assert_eq!(field, "memory", "{site} must name the memory field for {memory}");
                    assert!(message.contains("non-zero"), "{site}: {message}");
                }
                (
                    Admission::NotWholeMib(canonical),
                    Err(AggregateError::Validation { field, message }),
                ) => {
                    assert_eq!(field, "memory", "{site} must name the memory field for {memory}");
                    assert!(
                        message.contains("whole number of MiB"),
                        "{site} must name the whole-MiB rule for {memory}: {message}"
                    );
                    assert!(
                        message.contains(canonical),
                        "{site} must name the received canonical value {canonical}: {message}"
                    );
                }
                (_, other) => {
                    panic!("{site}: {cpu} / {memory} gave an unexpected outcome: {other:?}")
                }
            }
        }
    }
}

/// DD-3 / DD-6: the wire refuses the removed keys as JSON (the server's decode of
/// `POST /v1/workloads` bodies): `cpu_milli` / `memory_bytes` are unknown fields, alone or beside
/// the new keys, and the refusal names the unknown key. CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the wire must refuse the removed key naming it")]
fn sad_the_wire_refuses_the_removed_keys() {
    for (resources, unknown) in [
        (json!({ "cpu_milli": 500, "memory_bytes": 134_217_728 }), "cpu_milli"),
        (json!({ "cpu": "500m", "memory": "128MiB", "memory_bytes": 134_217_728 }), "memory_bytes"),
    ] {
        let mut body = job_json("500m", "128MiB");
        body["resources"] = resources;
        let err = serde_json::from_value::<JobSpecInput>(body)
            .expect_err("the wire must refuse the removed key naming it")
            .to_string();
        assert!(
            err.contains(unknown),
            "the wire must refuse the removed key naming it ({unknown}): {err}"
        );
    }
}

// ----- AC-1.9 / AC-2.5: one canonical size, one digest; describe reads the canonical form --------

/// AC-1.9 / AC-2.5: identical spec content spelled two ways that canonicalise equal archives to
/// the same bytes and has the same spec digest, so an idempotent re-deploy is not a conflict.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "string cpu/memory resources are not accepted yet")]
fn equal_sizes_in_different_spellings_have_the_same_spec_digest() {
    let spellings =
        [("2", "1GiB"), ("2000m", "1024MiB"), ("2.0", "1073741824B"), ("2.000", "1048576KiB")];
    let digests: Vec<_> = spellings
        .iter()
        .map(|(cpu, memory)| {
            let job = admit_job(cpu, memory).expect("equal sizes are admitted");
            WorkloadIntent::Job(job).spec_digest().expect("digest computes")
        })
        .collect();
    assert!(
        digests.windows(2).all(|w| w[0] == w[1]),
        "equal sizes in different spellings must have the same spec digest: {digests:?}"
    );
    let other = WorkloadIntent::Job(admit_job("2", "2GiB").expect("admitted"))
        .spec_digest()
        .expect("digest");
    assert_ne!(digests[0], other, "a different size must change the digest");
}

/// AC-1.5 (library side) / DD-6: the reverse projection `describe` renders from emits the canonical
/// string for both quantities, so `1024MiB` is read back as `1GiB` and `0.5` as `500m`, for
/// Job and Service. CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "string cpu/memory resources are not accepted yet")]
fn describe_reads_back_the_canonical_form() {
    let job = admit_job("0.5", "1024MiB").expect("admitted");
    let wire = serde_json::to_value(job.to_describe()).expect("serialises");
    assert_eq!(wire["resources"], json!({ "cpu": "500m", "memory": "1GiB" }));

    let service = admit_service("2000m", "128mib").expect("admitted");
    let wire = serde_json::to_value(service.to_describe("127.0.0.1".parse().expect("VIP")))
        .expect("serialises");
    assert_eq!(wire["resources"], json!({ "cpu": "2", "memory": "128MiB" }));
}
