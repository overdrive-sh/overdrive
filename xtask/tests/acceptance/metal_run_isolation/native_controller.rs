// Actual bootstrap -> transient systemd owner -> controller ABORT -> finalizer
// -> collector. Only SSH transport/native substrate probes/compiler are external
// fixtures. The compiler emits no guest or native qualification evidence.
use super::support::*;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

pub struct NativeController {
    pub fixture: Fixture,
    pub body: PathBuf,
}
impl NativeController {
    pub fn new() -> Self {
        let fixture = Fixture::new();
        // Keep systemd itself real; collection must observe the actual unit.
        fs::remove_file(fixture.bin.join("systemctl")).expect("remove observation-only substitute");
        let root = fixture.temp.path();
        let body = root.join("selected-compiler.calls");
        let bash_env = root.join("unit-bash-env");
        fs::write(&bash_env, format!("export PATH='{}'\n", fixture.path().to_string_lossy()))
            .expect("external compiler PATH fixture");
        executable(
            &fixture.bin.join("cargo"),
            &format!(
                "#!/usr/bin/env bash\nset -eu\nprintf 'selected compiler argv:' >> '{}'\nprintf ' %s' \"$@\" >> '{}'\nprintf '\\n' >> '{}'\nexit 0\n",
                body.display(),
                body.display(),
                body.display()
            ),
        );
        let compiler_path = fixture.bin.join("cargo");
        let original = text(&compiler_path);
        let report = format!(
            "printf 'kernel=%s\\nrootfs=%s\\n' \"${{OVERDRIVE_METAL_KERNEL:-<unset>}}\" \"${{OVERDRIVE_METAL_ROOTFS:-<unset>}}\" > '{}'\n",
            body.with_extension("environment").display()
        );
        executable(&compiler_path, &original.replace("exit 0\n", &format!("{report}exit 0\n")));
        executable(
            &fixture.bin.join("systemd-run"),
            &format!(
                "#!/usr/bin/env bash\nset -eu\nexec /usr/bin/systemd-run --setenv=BASH_ENV='{}' \"$@\"\n",
                bash_env.display()
            ),
        );
        let original = text(&workspace_file("infra/metal/tests/fixtures/ssh"));
        let transport = original.replace("command=\"${!#}\"", &format!("command=\"${{!#}}\"\ncommand=\"${{command//\\/var\\/lib\\/overdrive-metal\\/runs/{}}}\"", fixture.runs.display()));
        executable(&fixture.bin.join("ssh"), &transport);
        for (name, body) in [
            ("cpuinfo", "flags : vmx\n"),
            ("hypervisor-type", ""),
            ("kvm", "fixture-kvm"),
            ("cgroup.controllers", "cpu memory pids\n"),
            ("kernel", "fixture-kernel"),
            ("rootfs", "fixture-rootfs"),
        ] {
            fs::write(root.join(name), body).expect("external prerequisite input");
        }
        executable(&root.join("detect-none"), "#!/usr/bin/env bash\nprintf 'none\\n'\nexit 1\n");
        executable(&root.join("kvm-ok"), "#!/usr/bin/env bash\nexit 0\n");
        executable(&root.join("cloud-hypervisor"), "#!/usr/bin/env bash\nexit 0\n");
        Self { fixture, body }
    }
    pub fn command(&self, no_sync: bool) -> Command {
        self.command_with_class(no_sync, false, true)
    }
    pub fn command_with_class(&self, no_sync: bool, exclusive: bool, no_sudo: bool) -> Command {
        let mut args = vec!["--run"];
        if no_sync {
            args.push("--no-sync");
        }
        if exclusive {
            args.push("--exclusive");
        }
        if no_sudo {
            args.push("--no-sudo");
        }
        args.extend(["--", "cargo", "check", "-p", "xtask"]);
        let mut command = self.fixture.bootstrap(&self.fixture.source, &args);
        let root = self.fixture.temp.path();
        command
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
            .env("OVERDRIVE_METAL_ROOTFS", root.join("rootfs"));
        command
    }
    pub fn run_id(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .find_map(|line| line.strip_prefix("OVERDRIVE_METAL_RUN_ID="))
            .expect("actual bootstrap-selected run ID")
            .to_owned()
    }
    pub fn assert_terminal(&self, id: &str, error: &str, status: i32) {
        let directory = self.fixture.runs.join(id);
        assert_eq!(fields(&directory.join("phase.v1"))["phase"], "preparing");
        let terminal = fields(&directory.join("terminal.v1"));
        assert_eq!(terminal["kind"], "runner");
        assert_eq!(terminal["error"], error);
        assert_eq!(fields(&directory.join("completion.v1"))["kind"], "complete");
        let before = snapshot(&self.fixture.runs);
        runner_error(
            &self.fixture.collect(id).output().expect("actual retained runner result"),
            error,
            status,
        );
        assert_eq!(
            snapshot(&self.fixture.runs),
            before,
            "collection remains read-only after real producer failure"
        );
    }
}
