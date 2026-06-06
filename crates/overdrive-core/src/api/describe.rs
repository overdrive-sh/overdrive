//! Wire-shape `DescribeSpecOutput` enum and per-kind describe payloads
//! per ADR-0064.
//!
//! `DescribeSpecOutput` is the JSON body shape for the
//! `GET /v1/jobs/{id}` describe **response** — the read-only output
//! projection of the persisted [`crate::aggregate::WorkloadIntent`]
//! plus the platform-issued Service VIP. It is the describe-side member
//! of the four-layer Rust type universe — the inverse-direction sibling
//! of [`crate::api::submit::SubmitSpecInput`] (ADR-0051): where submit
//! projects `client JSON → WorkloadIntent` (validation), describe
//! projects `WorkloadIntent (+ VIP) → client JSON` (rendering).
//!
//! Tagged JSON via `#[serde(tag = "kind", rename_all = "snake_case")]`:
//! the `kind` field discriminates `job` / `service` / `schedule`.
//! `utoipa::ToSchema` renders the enum as a `oneOf`-discriminated schema
//! in the generated OpenAPI document per ADR-0064 OQ-1 / § 5.
//!
//! Unlike `SubmitSpecInput` there is NO `deny_unknown_fields`: this is a
//! server → client response shape, forward-tolerant per ADR-0064 § 1 —
//! a client deserialising a newer server's response ignores additive
//! fields rather than rejecting them.

use crate::aggregate::{DriverInput, JobSpecInput, ResourcesInput};
use crate::api::submit::ListenerInput;
use crate::id::ServiceVip;
use serde::{Deserialize, Serialize};

/// HTTP/JSON wire-shape for the `GET /v1/jobs/{id}` describe RESPONSE.
///
/// Per ADR-0064 this is the describe-side member of the type-family
/// universe — the read-only output projection distinct from the
/// submit-side [`crate::api::submit::SubmitSpecInput`] (ADR-0051)
/// because it surfaces the platform-issued Service VIP, which the
/// submit shape structurally forbids (ADR-0049 § 5).
///
/// Tagged JSON via `#[serde(tag = "kind", rename_all = "snake_case")]`
/// — the `kind` field discriminates `job` / `service` / `schedule`.
///
/// `utoipa::ToSchema` renders this as a `oneOf`-discriminated schema in
/// the generated OpenAPI document per OQ-1. NO `deny_unknown_fields`:
/// a response wire is forward-tolerant (ADR-0064 § 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DescribeSpecOutput {
    /// Run-to-completion workload — reuses [`JobSpecInput`] verbatim
    /// (the Job describe arm carries no platform-derived field, so the
    /// existing `From<&Job>` render path IS the projection per
    /// ADR-0064 § 2).
    Job(JobSpecInput),
    /// Long-running supervised workload — surfaces the platform-issued
    /// VIP via [`ServiceSpecOutput`].
    Service(ServiceSpecOutput),
    /// Cron-scheduled `Job` — see [`ScheduleSpecOutput`].
    Schedule(ScheduleSpecOutput),
}

/// HTTP/JSON wire-shape for a Service describe RESPONSE arm per
/// ADR-0064 § 2.
///
/// Mirrors the Service submit shape (`id`, `replicas`, `resources`,
/// `driver`, `listeners`) PLUS the platform-issued `vip` — the field
/// [`crate::api::submit::ServiceSpecInput`] structurally cannot carry
/// (ADR-0049 § 5). The `vip` is REQUIRED per OQ-4: a persisted-and-
/// describable Service always has an allocated VIP (submit-time
/// admission allocates before the intent is written — ADR-0049 § 4).
/// Absence is unrepresentable; a missing allocator entry is an
/// internal-invariant violation surfaced as HTTP 500, never an
/// `Option`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ServiceSpecOutput {
    pub id: String,
    pub replicas: u32,
    pub resources: ResourcesInput,
    #[serde(flatten)]
    pub driver: DriverInput,
    /// Operator-declared listeners, `(port, protocol)` only — no VIP
    /// per listener (ADR-0049 § 5a: one VIP per Service, surfaced once
    /// at the Service level, not per-listener).
    pub listeners: Vec<ListenerInput>,
    /// The platform-issued Service VIP. REQUIRED — serialised as a
    /// dotted-quad string (the [`ServiceVip`] newtype's `Display`).
    /// Read-only: the operator never sets this on submit; the platform
    /// assigns it via `ServiceVipAllocator` (ADR-0049).
    pub vip: ServiceVip,
}

/// HTTP/JSON wire-shape for a Schedule describe RESPONSE arm per
/// ADR-0064 § 2. The per-fire instance is a Job; the schedule adds the
/// cron expression. No VIP (a Schedule is not a Service).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ScheduleSpecOutput {
    pub id: String,
    /// Inner job specification fired on each cron tick. Same wire shape
    /// standalone Jobs use.
    pub job: JobSpecInput,
    /// Cron expression. String-shaped on the wire.
    pub cron_expr: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::ExecInput;
    use proptest::prelude::*;

    /// Strategy producing an arbitrary valid `ServiceVip` (IPv4 dotted-
    /// quad). `ServiceVip` has no `Arbitrary` impl in `overdrive-core`,
    /// so we build one over four octets and the validating constructor.
    fn service_vip_strategy() -> impl Strategy<Value = ServiceVip> {
        (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(a, b, c, d)| {
            ServiceVip::new(std::net::IpAddr::V4(std::net::Ipv4Addr::new(a, b, c, d)))
                .expect("IPv4 ServiceVip construction is infallible")
        })
    }

    /// Strategy producing an arbitrary `ResourcesInput`.
    fn resources_strategy() -> impl Strategy<Value = ResourcesInput> {
        (any::<u32>(), any::<u64>())
            .prop_map(|(cpu_milli, memory_bytes)| ResourcesInput { cpu_milli, memory_bytes })
    }

    /// Strategy producing an arbitrary `DriverInput` (the single Phase-1
    /// `Exec` variant) with arbitrary command + argv strings.
    fn driver_strategy() -> impl Strategy<Value = DriverInput> {
        (".*", proptest::collection::vec(".*", 0..4))
            .prop_map(|(command, args)| DriverInput::Exec(ExecInput { command, args }))
    }

    /// Strategy producing an arbitrary `JobSpecInput`.
    fn job_spec_strategy() -> impl Strategy<Value = JobSpecInput> {
        (".*", any::<u32>(), resources_strategy(), driver_strategy()).prop_map(
            |(id, replicas, resources, driver)| JobSpecInput { id, replicas, resources, driver },
        )
    }

    /// Strategy producing an arbitrary `Vec<ListenerInput>` with
    /// arbitrary ports and protocol strings (no validation at the wire
    /// layer — that happens in `ServiceV1::from_submit`).
    fn listeners_strategy() -> impl Strategy<Value = Vec<ListenerInput>> {
        proptest::collection::vec(
            (any::<u16>(), ".*").prop_map(|(port, protocol)| ListenerInput { port, protocol }),
            0..4,
        )
    }

    /// Strategy producing an arbitrary `ServiceSpecOutput`, including a
    /// generated `ServiceVip`.
    fn service_spec_output_strategy() -> impl Strategy<Value = ServiceSpecOutput> {
        (
            ".*",
            any::<u32>(),
            resources_strategy(),
            driver_strategy(),
            listeners_strategy(),
            service_vip_strategy(),
        )
            .prop_map(|(id, replicas, resources, driver, listeners, vip)| {
                ServiceSpecOutput { id, replicas, resources, driver, listeners, vip }
            })
    }

    use crate::aggregate::{JobV1, ServiceV1};
    use crate::api::submit::ServiceSpecInput;

    /// A known-good `JobSpecInput` for building intent fixtures through
    /// the validating constructors (port-to-port: never hand-construct
    /// the intent aggregate).
    fn valid_job_spec_input() -> JobSpecInput {
        JobSpecInput {
            id: "describe-job".to_owned(),
            replicas: 2,
            resources: ResourcesInput { cpu_milli: 500, memory_bytes: 64 * 1024 * 1024 },
            driver: DriverInput::Exec(ExecInput {
                command: "/bin/true".to_owned(),
                args: vec!["--flag".to_owned()],
            }),
        }
    }

    #[test]
    fn job_to_describe_delegates_to_from_job() {
        let job: JobV1 = JobV1::from_submit(valid_job_spec_input()).expect("valid job spec");

        // `to_describe` is the describe-side render path; it must equal
        // the existing `From<&Job>` projection it delegates to.
        let rendered: JobSpecInput = job.to_describe();
        let via_from: JobSpecInput = JobSpecInput::from(&job);
        assert_eq!(rendered, via_from);
    }

    #[test]
    fn service_to_describe_carries_passed_vip_and_maps_listeners() {
        let input = ServiceSpecInput {
            id: "describe-svc".to_owned(),
            replicas: 3,
            resources: ResourcesInput { cpu_milli: 250, memory_bytes: 32 * 1024 * 1024 },
            driver: DriverInput::Exec(ExecInput {
                command: "/usr/bin/server".to_owned(),
                args: vec![],
            }),
            listeners: vec![
                ListenerInput { port: 8080, protocol: "tcp".to_owned() },
                ListenerInput { port: 53, protocol: "udp".to_owned() },
            ],
            startup_probes: vec![],
            readiness_probes: vec![],
            liveness_probes: vec![],
        };
        let svc: ServiceV1 = ServiceV1::from_submit(input).expect("valid service spec");
        let vip = ServiceVip::new(std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 96, 0, 7)))
            .expect("ipv4 vip");

        let rendered: ServiceSpecOutput = svc.to_describe(vip);

        // The platform-issued VIP is the passed parameter (not read from
        // the spec — the spec carries none).
        assert_eq!(rendered.vip, vip);
        assert_eq!(rendered.id, "describe-svc");
        assert_eq!(rendered.replicas, 3);
        // Listeners map from the intent shape (NonZeroU16 / Proto) back
        // to the wire shape (u16 / lowercase protocol string), in order.
        assert_eq!(
            rendered.listeners,
            vec![
                ListenerInput { port: 8080, protocol: "tcp".to_owned() },
                ListenerInput { port: 53, protocol: "udp".to_owned() },
            ]
        );
    }

    proptest! {
        /// Roundtrip property: a `DescribeSpecOutput::Job` survives
        /// serialise → deserialise bit-equal, AND the serialised JSON
        /// carries `"kind": "job"` per the `#[serde(tag = "kind")]`
        /// discriminator.
        #[test]
        fn describe_spec_output_job_roundtrip_and_oneof_shape(job in job_spec_strategy()) {
            let value = DescribeSpecOutput::Job(job);
            let json = serde_json::to_value(&value).expect("serialise");
            prop_assert_eq!(
                json.get("kind").and_then(serde_json::Value::as_str),
                Some("job"),
                "Job arm must carry the `kind: job` discriminator"
            );
            let back: DescribeSpecOutput = serde_json::from_value(json).expect("deserialise");
            prop_assert_eq!(back, value);
        }

        /// Roundtrip property: a `DescribeSpecOutput::Service` survives
        /// serialise → deserialise bit-equal (including the generated
        /// `ServiceVip`), AND the serialised JSON carries
        /// `"kind": "service"` AND a `"vip"` dotted-quad field.
        #[test]
        fn describe_spec_output_service_roundtrip_and_oneof_shape(
            svc in service_spec_output_strategy()
        ) {
            let expected_vip = svc.vip.to_string();
            let value = DescribeSpecOutput::Service(svc);
            let json = serde_json::to_value(&value).expect("serialise");
            prop_assert_eq!(
                json.get("kind").and_then(serde_json::Value::as_str),
                Some("service"),
                "Service arm must carry the `kind: service` discriminator"
            );
            prop_assert_eq!(
                json.get("vip").and_then(serde_json::Value::as_str),
                Some(expected_vip.as_str()),
                "Service arm must surface the platform-issued vip as a dotted-quad string"
            );
            let back: DescribeSpecOutput = serde_json::from_value(json).expect("deserialise");
            prop_assert_eq!(back, value);
        }
    }
}
