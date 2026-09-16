//! ADR-0091 Service VM ingress acceptance properties.

#![allow(
    clippy::doc_markdown,
    reason = "the required per-test CONTRACT_SHAPE declaration is a literal protocol marker"
)]

use overdrive_core::aggregate::{
    DriverInput, ParserDriverInput, Service, ServiceSpecEnvelope, VmInput, WorkloadDriver,
    WorkloadSpecInput,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::codec::VersionedEnvelope;

const VM_SERVICE: &str = r#"
[service]
id = "vm-service"
replicas = 1
[vm]
command = "/bin/server"
args = []
kernel = "/kernel"
rootfs = "/rootfs"
[resources]
cpu_milli = 100
memory_bytes = 1048576
[[listener]]
port = 8080
protocol = "tcp"
"#;

fn vm_input() -> ServiceSpecInput {
    ServiceSpecInput {
        id: "vm-service".to_owned(),
        replicas: 1,
        resources: overdrive_core::aggregate::ResourcesInput {
            cpu_milli: 100,
            memory_bytes: 1_048_576,
        },
        driver: DriverInput::Vm(VmInput {
            command: "/bin/server".to_owned(),
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

/// CONTRACT_SHAPE: pure-function.
#[test]
fn service_vm_http_tcp_spec_parses_to_vm_driver_without_rewriting_probe_intent() {
    let source = format!(
        "{VM_SERVICE}\n[[health_check.startup]]\ntype = \"http\"\npath = \"/ready\"\nport = 8080\n[[health_check.readiness]]\ntype = \"tcp\"\nport = 8081\n"
    );
    let WorkloadSpecInput::Service(spec) =
        WorkloadSpecInput::from_toml_str(&source).expect("VM service parses")
    else {
        panic!("expected Service");
    };

    assert!(matches!(spec.driver, ParserDriverInput::Vm(_)));
    assert!(matches!(
        spec.startup_probes.as_slice(),
        [probe] if matches!(probe.mechanic, overdrive_core::aggregate::ProbeMechanic::Http { .. })
    ));
    assert!(matches!(
        spec.readiness_probes.as_slice(),
        [probe] if matches!(probe.mechanic, overdrive_core::aggregate::ProbeMechanic::Tcp { port: 8081, .. })
    ));
}

/// CONTRACT_SHAPE: pure-function.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn service_spec_v1_is_the_sole_direct_envelope_arm() {
    assert_eq!(ServiceSpecEnvelope::known_discriminants(), &[0]);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn service_driver_roundtrip_preserves_vm_union_arm() {
    let vm = Service::from_submit(vm_input()).expect("VM accepted");
    assert!(matches!(vm.driver, WorkloadDriver::Vm(_)));
    assert!(matches!(vm.to_describe("127.0.0.1".parse().expect("VIP")).driver, DriverInput::Vm(_)));
}
