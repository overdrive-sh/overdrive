//! Native qualification diagnostics use existing external probe fixture inputs.
//! These are negative-adapter tests; successful fixtures grant no native credit.
use super::support::*;
use std::fs;
use std::process::Command;

fn preflight(fixture: &Fixture) -> Command {
    let root = fixture.temp.path();
    for (file, value) in [
        ("cpuinfo", "flags : vmx\n"),
        ("hypervisor-type", ""),
        ("kvm", "fixture-device"),
        ("cgroup.controllers", "cpu memory pids\n"),
        ("kernel", "fixture-kernel"),
        ("rootfs", "fixture-rootfs"),
    ] {
        fs::write(root.join(file), value).expect("qualification input");
    }
    executable(&root.join("detect-none"), "#!/usr/bin/env bash\nprintf 'none\\n'\nexit 1\n");
    executable(&root.join("kvm-ok"), "#!/usr/bin/env bash\nexit 0\n");
    executable(&root.join("cloud-hypervisor"), "#!/usr/bin/env bash\nexit 0\n");
    fs::write(root.join("owner"), format!("pid={}\nstarted_at={NOW}\naction=run\nscenario=metal-isolation\nworkspace=/fixture\ncommit={COMMIT}\ntoken={RUN_A}\nrun_id={RUN_A}\nunit=overdrive-metal-run-{RUN_A}.service\nsource_digest={DIGEST}\n", std::process::id())).expect("invocation owner input");
    fs::write(
        fixture.source.join(".overdrive-metal-source"),
        format!("commit={COMMIT}\nworkspace=/fixture\nsource_digest={DIGEST}\n"),
    )
    .expect("bound source input");
    let mut command = Command::new("bash");
    command
        .arg(workspace_file("infra/metal/native-preflight.sh"))
        .env("OVERDRIVE_PREFLIGHT_ARCH", "x86_64")
        .env("OVERDRIVE_PREFLIGHT_CPUINFO", root.join("cpuinfo"))
        .env("OVERDRIVE_PREFLIGHT_HYPERVISOR_TYPE", root.join("hypervisor-type"))
        .env("OVERDRIVE_PREFLIGHT_KVM_DEVICE", root.join("kvm"))
        .env("OVERDRIVE_PREFLIGHT_KVM_CHARACTER", "yes")
        .env("OVERDRIVE_PREFLIGHT_KVM_PROBE", root.join("kvm-ok"))
        .env("OVERDRIVE_PREFLIGHT_CGROUP_CONTROLLERS", root.join("cgroup.controllers"))
        .env("OVERDRIVE_PREFLIGHT_DETECT_VIRT", root.join("detect-none"))
        .env("OVERDRIVE_PREFLIGHT_CLOUD_HYPERVISOR", root.join("cloud-hypervisor"))
        .env("OVERDRIVE_METAL_KERNEL", root.join("kernel"))
        .env("OVERDRIVE_METAL_ROOTFS", root.join("rootfs"))
        .env("OVERDRIVE_METAL_OWNER_PATH", root.join("owner"))
        .env("OVERDRIVE_EXPECTED_TOKEN", RUN_A)
        .env("OVERDRIVE_EXPECTED_COMMIT", COMMIT)
        .env("OVERDRIVE_EXPECTED_WORKSPACE", "/fixture")
        .env("OVERDRIVE_EXPECTED_SOURCE", DIGEST)
        .env("OVERDRIVE_REMOTE_DIR", &fixture.source);
    command
}

/// CONTRACT_SHAPE: bounded-change (typed negative qualification output and
/// transient probe processes only; source, invocation and physical kernel unchanged).
#[test]
fn sad_each_existing_native_prerequisite_has_the_exact_probe_and_retained_cause() {
    for probe in [
        "arch",
        "virtualization",
        "cpu",
        "kvm_device",
        "kvm_api",
        "cgroup_v2",
        "vmm",
        "guest_kernel",
        "guest_rootfs",
        "owner",
        "source",
    ] {
        let fixture = Fixture::new();
        let mut command = preflight(&fixture);
        let root = fixture.temp.path();
        match probe {
            "arch" => {
                command.env("OVERDRIVE_PREFLIGHT_ARCH", "aarch64");
            }
            "virtualization" => {
                executable(
                    &root.join("detect-none"),
                    "#!/usr/bin/env bash\nprintf 'kvm\\n'\nexit 0\n",
                );
            }
            "cpu" => {
                fs::write(root.join("cpuinfo"), "flags : sse2\n")
                    .expect("no hardware virtualization");
            }
            "kvm_device" => {
                command.env_remove("OVERDRIVE_PREFLIGHT_KVM_CHARACTER");
            }
            "kvm_api" => {
                executable(&root.join("kvm-ok"), "#!/usr/bin/env bash\nexit 1\n");
            }
            "cgroup_v2" => {
                fs::remove_file(root.join("cgroup.controllers")).expect("no cgroup v2");
            }
            "vmm" => {
                fs::remove_file(root.join("cloud-hypervisor")).expect("no VMM");
            }
            "guest_kernel" => {
                command.env_remove("OVERDRIVE_METAL_KERNEL");
            }
            "guest_rootfs" => {
                command.env("OVERDRIVE_METAL_ROOTFS", "");
            }
            "owner" => {
                command.env("OVERDRIVE_EXPECTED_TOKEN", RUN_B);
            }
            "source" => {
                command.env("OVERDRIVE_EXPECTED_SOURCE", "foreign-digest");
            }
            _ => unreachable!(),
        }
        let before = snapshot(&fixture.source);
        let output = command.output().expect("negative native qualification");
        runner_error(&output, "preflight_failed", 78);
        assert!(String::from_utf8_lossy(&output.stderr).contains(&format!("probe={probe}")));
        assert_eq!(snapshot(&fixture.source), before);
    }
}
