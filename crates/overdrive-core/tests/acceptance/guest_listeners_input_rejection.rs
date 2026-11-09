//! D14-E2: the raw Service constructor and operator TOML parser reject invalid
//! placement of the platform-only GuestListeners mechanic with typed causes.

#![allow(clippy::expect_used, clippy::doc_markdown)]

use overdrive_core::aggregate::{
    AggregateError, DriverInput, ParseError, ProbeDescriptor, ProbeMechanic, ResourcesInput,
    Service, VmInput, WorkloadSpecInput,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use proptest::prelude::*;

fn service_input() -> ServiceSpecInput {
    ServiceSpecInput {
        id: "guest-listener-input".to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 134_217_728 },
        driver: DriverInput::Vm(VmInput {
            command: "/bin/serve".to_owned(),
            args: vec![],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

const fn descriptor(role: ProbeRole, mechanic: ProbeMechanic) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role,
        mechanic,
        timeout_seconds: 5,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    }
}

fn tcp(role: ProbeRole) -> ProbeDescriptor {
    descriptor(role, ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port: 8080 })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// D14-E2: vector position and the submitted descriptor role both survive
    /// the first invalid GuestListeners rejection; no aggregate is constructed.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER: D14-E2 typed GuestListeners role and placement rejection"]
    fn raw_service_guest_listeners_wrong_role_is_typed(
        vector in 0usize..3,
        descriptor_role in prop_oneof![Just(ProbeRole::Startup), Just(ProbeRole::Readiness), Just(ProbeRole::Liveness)],
        index in 0usize..3,
    ) {
        let vector_role = [ProbeRole::Startup, ProbeRole::Readiness, ProbeRole::Liveness][vector];
        prop_assume!(vector_role != ProbeRole::Startup || descriptor_role != ProbeRole::Startup);
        let mut input = service_input();
        let probes = match vector {
            0 => &mut input.startup_probes,
            1 => &mut input.readiness_probes,
            _ => &mut input.liveness_probes,
        };
        for _ in 0..index {
            probes.push(tcp(vector_role));
        }
        probes.push(descriptor(descriptor_role, ProbeMechanic::GuestListeners));
        if vector < 2 {
            let later = if vector == 0 { &mut input.readiness_probes } else { &mut input.liveness_probes };
            later.push(descriptor(ProbeRole::Liveness, ProbeMechanic::GuestListeners));
        }
        let before = input.clone();
        let outcome = Service::from_submit(input.clone());
        prop_assert_eq!(input, before, "validation does not mutate the submitted value");
        match outcome {
            Err(AggregateError::GuestListenersInvalidRole {
                vector_role: got_vector, descriptor_role: got_descriptor, index: got_index,
            }) => {
                prop_assert_eq!(got_vector, vector_role);
                prop_assert_eq!(got_descriptor, descriptor_role);
                prop_assert_eq!(got_index, index);
            }
            other => prop_assert!(false, "expected exact typed role error, got {other:?}"),
        }
    }
}

/// D14-E2: a correctly tagged GuestListeners descriptor still cannot share
/// startup with another descriptor, even when the general probe cap is exceeded.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: D14-E2 sole startup check precedes general probe count"]
fn raw_service_guest_listeners_not_sole_startup_precedes_count_check() {
    for count in [2usize, 7, 8, 9] {
        let mut input = service_input();
        input.startup_probes.push(descriptor(ProbeRole::Startup, ProbeMechanic::GuestListeners));
        input.startup_probes.extend((1..count).map(|_| tcp(ProbeRole::Startup)));
        assert_eq!(input.startup_probes.len(), count);
        let before = input.clone();
        let result = Service::from_submit(input.clone());
        assert_eq!(input, before, "validation does not mutate the submitted value");
        match result {
            Err(AggregateError::GuestListenersNotSoleStartup { count: got }) => {
                assert_eq!(got, count);
            }
            other => panic!("expected exact typed sole-startup error, got {other:?}"),
        }
    }
}

/// D14-E2 control: one correctly tagged raw Service startup descriptor is
/// admitted, independent of whether it came from the operator JSON client.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn raw_service_guest_listeners_as_sole_startup_is_admitted() {
    let without_guest = Service::from_submit(service_input()).expect("zero guest descriptors");
    assert!(without_guest.startup_probes.is_empty());
    let mut input = service_input();
    input.startup_probes.push(descriptor(ProbeRole::Startup, ProbeMechanic::GuestListeners));
    let service = Service::from_submit(input).expect("the raw sole startup value is valid");
    assert_eq!(service.startup_probes.len(), 1);
    assert_eq!(service.startup_probes[0].role, ProbeRole::Startup);
    assert_eq!(service.startup_probes[0].mechanic, ProbeMechanic::GuestListeners);
}

const SERVICE_TOML: &str = "[service]\nid = \"guest-listener-input\"\nreplicas = 1\n\n\
[[listener]]\nport = 8080\nprotocol = \"tcp\"\n\n\
[vm]\ncommand = \"/bin/serve\"\nargs = []\nkernel = \"/kernel\"\nrootfs = \"/rootfs\"\n\n\
[resources]\ncpu_milli = 100\nmemory_bytes = 134217728\n";

/// D14-E2: the canonical token, in either case, is recognized as a
/// platform-only mechanic in each Service role at its exact position.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: D14-E2 canonical GuestListeners TOML token is typed"]
fn operator_toml_guest_listeners_is_typed_for_every_role_and_case() {
    for role in [ProbeRole::Startup, ProbeRole::Readiness, ProbeRole::Liveness] {
        let name = match role {
            ProbeRole::Startup => "startup",
            ProbeRole::Readiness => "readiness",
            ProbeRole::Liveness => "liveness",
        };
        for token in ["guest_listeners", "GUEST_LISTENERS"] {
            let toml = format!(
                "{SERVICE_TOML}\n[[health_check.{name}]]\ntype = \"tcp\"\nport = 8080\n\
                 [[health_check.{name}]]\ntype = \"{token}\"\n"
            );
            match WorkloadSpecInput::from_toml_str(&toml) {
                Err(ParseError::GuestListenersNotOperatorConfigurable { role: got, probe_idx }) => {
                    assert_eq!(got, role);
                    assert_eq!(probe_idx, 1);
                }
                other => {
                    panic!("{name}/{token}: expected typed platform-only error, got {other:?}")
                }
            }
        }
    }
    let both = format!(
        "{SERVICE_TOML}\n[[health_check.startup]]\ntype = \"GUEST_LISTENERS\"\n\
         [[health_check.readiness]]\ntype = \"guest_listeners\"\n"
    );
    match WorkloadSpecInput::from_toml_str(&both) {
        Err(ParseError::GuestListenersNotOperatorConfigurable { role, probe_idx }) => {
            assert_eq!(role, ProbeRole::Startup, "parser role order starts at Startup");
            assert_eq!(probe_idx, 0);
        }
        other => panic!("first platform-only token must be reported, got {other:?}"),
    }
    let noncanonical =
        format!("{SERVICE_TOML}\n[[health_check.startup]]\ntype = \"GuestListeners\"\n");
    match WorkloadSpecInput::from_toml_str(&noncanonical) {
        Err(ParseError::UnknownProbeType { probe_idx, found }) => {
            assert_eq!(probe_idx, 0);
            assert_eq!(found, "guestlisteners", "no new alias without the underscore");
        }
        other => panic!("noncanonical spelling remains unknown, got {other:?}"),
    }
}
