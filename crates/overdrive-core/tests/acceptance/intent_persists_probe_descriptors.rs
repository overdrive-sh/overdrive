//! GAP-6 corrective patch — `WorkloadIntent::Service` persists probe
//! descriptors end-to-end.
//!
//! Pre-corrective state: the TOML parser produced `ServiceSpec` with
//! `startup_probes` / `readiness_probes` / `liveness_probes`, but the
//! wire envelope (`ServiceSpecInput`) and persistent intent payload
//! (`ServiceV1`) both dropped the vecs on the way to the server. Probe
//! descriptors declared in TOML silently disappeared between CLI
//! submit and intent admission — surfaced when the GAP-1 corrective
//! crafter found `hydrate_desired` had no probe data to read.
//!
//! This acceptance test pins the corrective contract end-to-end so a
//! future refactor that silently drops a probe field on the way
//! through fails RED at PR time.
//!
//! ## Sub-scenarios
//!
//! * **GAP-6-AT-01** — `ServiceSpecInput` serde JSON round-trip
//!   bit-equivalent over arbitrary probe vecs.
//! * **GAP-6-AT-02** — `ServiceV1::from_submit` projects all three
//!   probe vecs through unchanged for any valid input.
//! * **GAP-6-AT-03** — `ServiceV1` rkyv archive + deserialize
//!   round-trips probe descriptors bit-equivalently. Structural
//!   guard against rkyv field-shifting silently corrupting probe
//!   bytes.
//! * **GAP-6-AT-04** — Composed parser → wire → intent end-to-end:
//!   a TOML fixture with declared probes parses to a `ServiceSpec`
//!   whose probe vecs equal the corresponding fields in the
//!   `ServiceV1` produced by `from_submit` on the wire-side input
//!   projected from the parsed spec.
//! * **GAP-6-AT-05** — Regression guard: for any non-empty input
//!   probe vec, the resulting `ServiceV1` has non-empty matching
//!   probe vec (defensive against future refactors silently
//!   dropping fields).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, ResourcesInput, ServiceV1, WorkloadDriver, WorkloadIntent,
    WorkloadIntentEnvelope, WorkloadSpecInput,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::codec::decode_envelope_bytes;
use overdrive_core::observation::ProbeRole;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

fn arb_probe_role() -> impl Strategy<Value = ProbeRole> {
    prop_oneof![Just(ProbeRole::Startup), Just(ProbeRole::Readiness), Just(ProbeRole::Liveness)]
}

fn arb_mechanic() -> impl Strategy<Value = ProbeMechanic> {
    prop_oneof![
        ("[a-zA-Z0-9._-]{1,40}", 1u16..=65535)
            .prop_map(|(host, port)| ProbeMechanic::Tcp { host, port }),
        ("/[a-zA-Z0-9_./-]{0,60}", 1u16..=65535, proptest::option::of("[a-zA-Z0-9._-]{1,40}"),)
            .prop_map(|(path, port, host)| ProbeMechanic::Http { path, port, host }),
        proptest::collection::vec("[a-zA-Z0-9_./-]{1,30}", 1..=4)
            .prop_map(|command| ProbeMechanic::Exec { command }),
    ]
}

fn arb_probe_descriptor() -> impl Strategy<Value = ProbeDescriptor> {
    (
        arb_probe_role(),
        arb_mechanic(),
        1u32..=60,
        1u32..=60,
        1u32..=300,
        proptest::option::of(1u32..=10),
        proptest::option::of(1u32..=10),
        any::<bool>(),
    )
        .prop_map(
            |(
                role,
                mechanic,
                timeout_seconds,
                interval_seconds,
                max_attempts,
                failure_threshold,
                success_threshold,
                inferred,
            )| ProbeDescriptor {
                role,
                mechanic,
                timeout_seconds,
                interval_seconds,
                max_attempts,
                failure_threshold,
                success_threshold,
                inferred,
            },
        )
}

fn arb_probe_vec() -> impl Strategy<Value = Vec<ProbeDescriptor>> {
    proptest::collection::vec(arb_probe_descriptor(), 0..=3)
}

fn arb_service_spec_input() -> impl Strategy<Value = ServiceSpecInput> {
    (arb_probe_vec(), arb_probe_vec(), arb_probe_vec()).prop_map(
        |(startup_probes, readiness_probes, liveness_probes)| ServiceSpecInput {
            id: "svc-test".to_string(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: DriverInput::Exec(ExecInput {
                command: "/bin/serve".to_string(),
                args: vec![],
            }),
            listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_string() }],
            startup_probes,
            readiness_probes,
            liveness_probes,
        },
    )
}

// ---------------------------------------------------------------------------
// GAP-6-AT-01 — ServiceSpecInput wire serde roundtrip carries probe vecs.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// `serde_json::to_string(&input)` → `from_str` round-trips
    /// bit-equivalent for any probe-vec content. The wire envelope
    /// MUST carry the probe descriptors through unchanged — if it
    /// drops them, the CLI cannot send what the operator declared.
    #[test]
    fn at_01_service_spec_input_serde_roundtrip_preserves_probe_vecs(
        input in arb_service_spec_input(),
    ) {
        let json = serde_json::to_string(&input)
            .expect("ServiceSpecInput serialises");
        let parsed: ServiceSpecInput = serde_json::from_str(&json)
            .expect("ServiceSpecInput deserialises");
        prop_assert_eq!(&parsed.startup_probes, &input.startup_probes);
        prop_assert_eq!(&parsed.readiness_probes, &input.readiness_probes);
        prop_assert_eq!(&parsed.liveness_probes, &input.liveness_probes);
        prop_assert_eq!(parsed, input);
    }
}

// ---------------------------------------------------------------------------
// GAP-6-AT-02 — from_submit projects probe vecs into ServiceV1.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// `ServiceV1::from_submit(input)` produces a `ServiceV1` whose
    /// three probe vecs equal `input`'s three probe vecs — the
    /// validating constructor MUST NOT drop probe data on the way
    /// through. This is the gap that produced the bug: the legacy
    /// `from_submit` had ZERO probe-related code.
    #[test]
    fn at_02_from_submit_projects_all_three_probe_vecs(
        input in arb_service_spec_input(),
    ) {
        let expected_startup = input.startup_probes.clone();
        let expected_readiness = input.readiness_probes.clone();
        let expected_liveness = input.liveness_probes.clone();

        let svc = ServiceV1::from_submit(input)
            .expect("canonical ServiceSpecInput is valid");

        prop_assert_eq!(&svc.startup_probes, &expected_startup);
        prop_assert_eq!(&svc.readiness_probes, &expected_readiness);
        prop_assert_eq!(&svc.liveness_probes, &expected_liveness);
    }
}

// ---------------------------------------------------------------------------
// GAP-6-AT-03 — rkyv archive of ServiceV1 + envelope round-trips probes.
// ---------------------------------------------------------------------------

fn arb_service_v1_via_from_submit() -> impl Strategy<Value = ServiceV1> {
    arb_service_spec_input()
        .prop_map(|input| ServiceV1::from_submit(input).expect("valid ServiceSpecInput"))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// `WorkloadIntent::Service(svc).archive_for_store() →
    /// WorkloadIntent::from_store_bytes(...)` round-trips
    /// bit-equivalent for any `ServiceV1` with arbitrary probe
    /// vecs. Structural guard: an rkyv layout change that silently
    /// shifted probe-vec offsets would fail this assertion. Per
    /// `.claude/rules/development.md` § "rkyv schema evolution" the
    /// archived layout of `ServiceV1` is positional — appending the
    /// three probe vecs in the GAP-6 corrective patch is layout-
    /// affecting; this test pins the resulting V1 layout.
    #[test]
    fn at_03_workload_intent_envelope_roundtrip_preserves_probes(
        svc in arb_service_v1_via_from_submit(),
    ) {
        let intent = WorkloadIntent::Service(svc);

        let bytes = intent.archive_for_store()
            .expect("archive_for_store must succeed");
        let decoded = WorkloadIntent::from_store_bytes(
            bytes.as_ref(),
            std::path::Path::new("gap6.redb"),
            None,
        )
        .expect("from_store_bytes must succeed on bytes archive_for_store just produced");

        prop_assert_eq!(&decoded, &intent);

        // Also exercise the raw envelope-level decode to pin that
        // the envelope discriminant + inner payload + probe vecs
        // all survive the round-trip together.
        let raw_decoded = decode_envelope_bytes::<WorkloadIntentEnvelope>(bytes.as_ref())
            .expect("envelope decodes");
        prop_assert_eq!(raw_decoded, intent);
    }
}

// ---------------------------------------------------------------------------
// GAP-6-AT-04 — end-to-end TOML → wire → intent carries probe vecs.
// ---------------------------------------------------------------------------

/// TOML fixture declaring a single TCP startup probe — exercises
/// the parser-side default-rejection + explicit-declaration path.
const TOML_WITH_EXPLICIT_PROBE: &str = r#"
[service]
id = "svc-payments"
replicas = 1

[resources]
cpu_milli = 250
memory_bytes = 134217728

[exec]
command = "/usr/local/bin/payments"
args = []

[[listener]]
port = 8080
protocol = "tcp"

[[health_check.startup]]
type = "tcp"
host = "127.0.0.1"
port = 8080
timeout_seconds = 3
interval_seconds = 1
max_attempts = 15
"#;

#[test]
fn at_04_toml_to_intent_end_to_end_carries_startup_probes() {
    // Parser side — produce a `ServiceSpec` from TOML.
    let parsed = WorkloadSpecInput::from_toml_str(TOML_WITH_EXPLICIT_PROBE)
        .expect("TOML parses to WorkloadSpec");
    let service_spec = match parsed {
        WorkloadSpecInput::Service(spec) => spec,
        other => panic!("expected WorkloadSpecInput::Service, got {other:?}"),
    };

    // The parser MUST have populated startup_probes — either with
    // the explicitly declared probe or the default-inferred one.
    // We declared one explicitly above, so expect exactly one.
    assert_eq!(
        service_spec.startup_probes.len(),
        1,
        "parser must populate startup_probes from the [[health_check.startup]] block; \
         got {:?}",
        service_spec.startup_probes,
    );

    // The parser-declared probe MUST NOT be flagged as inferred —
    // operator declared it explicitly. (Inferred probes per ADR-0058
    // are the platform-synthesised default; operator-declared ones
    // carry inferred=false.)
    assert!(
        !service_spec.startup_probes[0].inferred,
        "operator-declared probe must carry inferred=false; descriptor={:?}",
        service_spec.startup_probes[0],
    );

    // Wire side — project parser-side `ServiceSpec` to wire-side
    // `ServiceSpecInput`, mirroring the CLI's
    // `submit_streaming_service` projection.
    let wire_input = ServiceSpecInput {
        id: service_spec.id.clone(),
        replicas: service_spec.replicas,
        resources: ResourcesInput {
            cpu_milli: service_spec.resources.cpu_milli,
            memory_bytes: service_spec.resources.memory_bytes,
        },
        driver: DriverInput::Exec(ExecInput {
            command: service_spec.exec.command.clone(),
            args: service_spec.exec.args.clone(),
        }),
        listeners: service_spec
            .listeners
            .iter()
            .map(|l| ListenerInput { port: l.port.get(), protocol: l.protocol.as_str().to_owned() })
            .collect(),
        startup_probes: service_spec.startup_probes.clone(),
        readiness_probes: service_spec.readiness_probes.clone(),
        liveness_probes: service_spec.liveness_probes.clone(),
    };

    // Intent side — validating constructor MUST carry probe vecs
    // through unchanged.
    let svc = ServiceV1::from_submit(wire_input).expect("valid service spec");

    assert_eq!(
        svc.startup_probes, service_spec.startup_probes,
        "ServiceV1.startup_probes MUST equal parser-side ServiceSpec.startup_probes — \
         from_submit MUST NOT drop probes on the way through",
    );
    assert_eq!(svc.readiness_probes, service_spec.readiness_probes);
    assert_eq!(svc.liveness_probes, service_spec.liveness_probes);

    // And the rkyv envelope round-trip preserves them — the
    // complete CLI submit → server admission → IntentStore read
    // chain.
    let expected_startup_probes = svc.startup_probes.clone();
    let intent = WorkloadIntent::Service(svc);
    let bytes = intent.archive_for_store().expect("archive_for_store");
    let decoded =
        WorkloadIntent::from_store_bytes(bytes.as_ref(), std::path::Path::new("at-04.redb"), None)
            .expect("from_store_bytes");
    let decoded_svc = match decoded {
        WorkloadIntent::Service(s) => s,
        other => panic!("expected WorkloadIntent::Service, got {other:?}"),
    };
    assert_eq!(
        decoded_svc.startup_probes, expected_startup_probes,
        "rkyv envelope round-trip MUST preserve startup_probes",
    );
}

// ---------------------------------------------------------------------------
// GAP-6-AT-05 — regression guard: non-empty input → non-empty intent.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// For any non-empty input probe vec, the projected `ServiceV1`
    /// has a non-empty matching probe vec. Defensive guard against
    /// a future refactor that silently drops a field — for example,
    /// a rewrite of `from_submit` that omits one of the three
    /// probe fields would pass `at_02` for the slot it kept but
    /// fail this regression on the other two.
    #[test]
    fn at_05_non_empty_probes_remain_non_empty_through_from_submit(
        startup in proptest::collection::vec(arb_probe_descriptor(), 1..=3),
        readiness in proptest::collection::vec(arb_probe_descriptor(), 1..=3),
        liveness in proptest::collection::vec(arb_probe_descriptor(), 1..=3),
    ) {
        let input = ServiceSpecInput {
            id: "svc-guard".to_string(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: DriverInput::Exec(ExecInput {
                command: "/bin/serve".to_string(),
                args: vec![],
            }),
            listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_string() }],
            startup_probes: startup,
            readiness_probes: readiness,
            liveness_probes: liveness,
        };

        let svc = ServiceV1::from_submit(input)
            .expect("canonical ServiceSpecInput is valid");

        prop_assert!(
            !svc.startup_probes.is_empty(),
            "startup_probes MUST remain non-empty through from_submit",
        );
        prop_assert!(
            !svc.readiness_probes.is_empty(),
            "readiness_probes MUST remain non-empty through from_submit",
        );
        prop_assert!(
            !svc.liveness_probes.is_empty(),
            "liveness_probes MUST remain non-empty through from_submit",
        );
    }
}

// ---------------------------------------------------------------------------
// Default-omitted serde behaviour on the wire — backwards-compat
// guarantee that legacy JSON without the probe fields still deserialises
// (since both server-side handlers and clients rev independently).
// ---------------------------------------------------------------------------

#[test]
fn service_spec_input_legacy_json_without_probe_fields_deserialises_with_empty_vecs() {
    // Pre-GAP-6 JSON shape — no probe fields. `#[serde(default)]`
    // on each probe field MUST default to empty vec so a legacy
    // client can still hit a corrective-patched server (and a
    // patched client can hit a legacy server during rollout — the
    // single-cut greenfield policy notwithstanding, the codec
    // surface defaults are load-bearing for client/server skew).
    let legacy_json = r#"{
        "id": "svc-legacy",
        "replicas": 1,
        "resources": {"cpu_milli": 100, "memory_bytes": 67108864},
        "exec": {"command": "/bin/legacy", "args": []},
        "listeners": [{"port": 80, "protocol": "tcp"}]
    }"#;

    let parsed: ServiceSpecInput =
        serde_json::from_str(legacy_json).expect("legacy JSON must still deserialise");

    assert!(parsed.startup_probes.is_empty());
    assert!(parsed.readiness_probes.is_empty());
    assert!(parsed.liveness_probes.is_empty());

    // And it still flows through `from_submit` unchanged.
    let svc = ServiceV1::from_submit(parsed).expect("legacy spec validates");
    assert!(svc.startup_probes.is_empty());
    assert!(svc.readiness_probes.is_empty());
    assert!(svc.liveness_probes.is_empty());

    // ServiceV1's driver projection MUST be Exec("/bin/legacy") —
    // smoke check that the rest of the projection still works.
    let WorkloadDriver::Exec(exec) = svc.driver;
    assert_eq!(exec.command, "/bin/legacy");
}
