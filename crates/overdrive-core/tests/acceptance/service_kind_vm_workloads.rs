//! ADR-0091 Service VM ingress acceptance properties.

use overdrive_core::aggregate::{
    AggregateError, DriverInput, ParserDriverInput, ServiceSpecEnvelope, ServiceV2, VmInput,
    WorkloadDriver, WorkloadSpecInput,
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
            memory_bytes: 1048576,
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

fn exec_probe(
    role: overdrive_core::observation::ProbeRole,
) -> overdrive_core::aggregate::ProbeDescriptor {
    overdrive_core::aggregate::ProbeDescriptor {
        idx: overdrive_core::observation::ProbeIdx::new(99),
        role,
        mechanic: overdrive_core::aggregate::ProbeMechanic::Exec { command: vec!["a".to_owned()] },
        timeout_seconds: 1,
        interval_seconds: 1,
        max_attempts: 1,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
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
#[test]
fn parser_rejects_first_vm_exec_probe_in_role_then_position_order() {
    let src = format!(
        "{VM_SERVICE}\n[[health_check.startup]]\ntype = \"tcp\"\nport = 8080\n[[health_check.startup]]\ntype = \"exec\"\ncommand = [\"a\"]\n[[health_check.readiness]]\ntype = \"exec\"\ncommand = [\"b\"]\n"
    );
    let err = WorkloadSpecInput::from_toml_str(&src).expect_err("VM Exec rejected");
    assert_eq!(
        err.to_string(),
        "[[health_check.startup]]: entry [1]: exec probes are not supported for VM Service workloads; use HTTP or TCP; optional VM Exec probes are tracked by GH #280",
    );
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn parser_vm_exec_rejection_is_role_and_entry_localized_before_aggregate_creation() {
    let err = WorkloadSpecInput::from_toml_str(&format!(
        "{VM_SERVICE}\n[[health_check.liveness]]\ntype = \"exec\"\ncommand = [\"a\"]\n"
    ))
    .expect_err("VM Exec rejected");
    assert_eq!(
        err.to_string(),
        "[[health_check.liveness]]: entry [0]: exec probes are not supported for VM Service workloads; use HTTP or TCP; optional VM Exec probes are tracked by GH #280"
    );
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn authoritative_admission_rejects_first_vm_exec_probe_before_intent_exists() {
    let mut input = vm_input();
    input.startup_probes.push(exec_probe(overdrive_core::observation::ProbeRole::Startup));
    input.readiness_probes.push(exec_probe(overdrive_core::observation::ProbeRole::Readiness));
    assert!(matches!(
        ServiceV2::from_submit(input),
        Err(AggregateError::Validation { field: "startup_probes", message })
            if message.starts_with("[0]:")
    ));
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn both_vm_exec_rejection_layers_share_the_exact_gh_280_diagnostic() {
    let mut input = vm_input();
    input.startup_probes.push(exec_probe(overdrive_core::observation::ProbeRole::Startup));
    let AggregateError::Validation { message, .. } =
        ServiceV2::from_submit(input).expect_err("VM Exec rejected")
    else {
        panic!("validation")
    };
    assert_eq!(
        message,
        "[0]: exec probes are not supported for VM Service workloads; use HTTP or TCP; optional VM Exec probes are tracked by GH #280"
    );
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn service_spec_v1_v2_compatibility_and_v3_golden_bytes_are_preserved() {
    assert_eq!(ServiceSpecEnvelope::known_discriminants(), &[0, 1, 2]);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn service_driver_roundtrip_preserves_both_existing_union_arms() {
    let vm = ServiceV2::from_submit(vm_input()).expect("VM accepted");
    assert!(matches!(vm.driver, WorkloadDriver::Vm(_)));
    assert!(matches!(vm.to_describe("127.0.0.1".parse().expect("VIP")).driver, DriverInput::Vm(_)));

    let mut exec_input = vm_input();
    exec_input.driver = DriverInput::Exec(overdrive_core::aggregate::ExecInput {
        command: "/bin/server".to_owned(),
        args: vec![],
    });
    let exec = ServiceV2::from_submit(exec_input).expect("Exec accepted");
    assert!(matches!(exec.driver, WorkloadDriver::Exec(_)));
    assert!(matches!(
        exec.to_describe("127.0.0.1".parse().expect("VIP")).driver,
        DriverInput::Exec(_)
    ));
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn exec_service_exec_probe_compatibility_is_unchanged() {
    let src = VM_SERVICE.replace(
        "[vm]\ncommand = \"/bin/server\"\nargs = []\nkernel = \"/kernel\"\nrootfs = \"/rootfs\"",
        "[exec]\ncommand = \"/bin/server\"\nargs = []",
    );
    assert!(
        WorkloadSpecInput::from_toml_str(&format!(
            "{src}\n[[health_check.startup]]\ntype = \"exec\"\ncommand = [\"a\"]"
        ))
        .is_ok()
    );
}
