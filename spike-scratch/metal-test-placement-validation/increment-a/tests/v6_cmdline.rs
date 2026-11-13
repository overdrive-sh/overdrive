//! PROBE V-6: does `ScriptedBootExtra.cmdline` put `overdrive.test_guest=1` in
//! the guest's `/proc/cmdline`, and is it absent on the host?
//!
//! Hypothesis: the guest kit appends every `cmdline` token to the guest kernel
//! command line, so a process inside the guest reads it from `/proc/cmdline`;
//! the host's own `/proc/cmdline` never carries it.
//! Prediction: guest A (token passed) prints it; guest B (control, no token)
//! does not; the host does not.
//! Falsification: guest A's `/proc/cmdline` lacks the token, or the host's has it.
use probe_macros::overdrive_test;
use std::time::Duration;

use overdrive_testing::guest_kit::cid_bands::FWD_GUEST_BOUNDARY;
use overdrive_testing::guest_kit::{GuestFile, GuestImageSpec, GuestKit, ScriptedBootExtra, WorkDir};
use overdrive_testing::vm_fixture::default_staging_root;

const TOKEN: &str = "overdrive.test_guest=1";

fn cmdline() -> String {
    std::fs::read_to_string("/proc/cmdline").unwrap_or_else(|e| format!("?{e}"))
}

/// Runs only inside a guest (ignored on the host, so nextest never schedules it).
#[test]
#[ignore = "run only inside the V-6 guests"]
fn inner_read_cmdline() {
    let line = cmdline();
    let has = line.split_whitespace().any(|t| t == TOKEN);
    println!("V6_GUEST_CMDLINE {}", line.trim());
    println!("V6_GUEST_HAS_TOKEN {has}");
    let expect = std::env::var("V6_EXPECT_TOKEN").unwrap_or_default();
    assert_eq!(has.to_string(), if expect == "1" { "true" } else { "false" }, "token presence mismatch");
}

fn boot_and_read(label: &str, cid_slot: u32, extra: ScriptedBootExtra, expect: &str) -> i32 {
    let exe = std::env::current_exe().unwrap();
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).unwrap();
    let work = WorkDir::create(&staging, &format!("v6-{label}")).unwrap();
    let mut spec = GuestImageSpec::scripted();
    let env_host = ["/usr/bin/env", "/bin/env"]
        .into_iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.is_file())
        .unwrap();
    spec.extra = vec![
        GuestFile { host: exe.canonicalize().unwrap(), guest: exe.clone(), with_shared_libraries: true },
        GuestFile { host: env_host, guest: "/usr/bin/env".into(), with_shared_libraries: true },
    ];
    let image = kit.image(work.path(), label, &spec).unwrap();
    let mut g = kit
        .boot_scripted_with(&image, FWD_GUEST_BOUNDARY.raw(cid_slot), &work.path().join("run"), &extra)
        .unwrap();
    let off = g.vm.console().len();
    let exe_s = exe.display().to_string();
    let expect_env = format!("V6_EXPECT_TOKEN={expect}");
    let code = g
        .peer()
        .exec(
            &["/usr/bin/env", &expect_env, &exe_s, "inner_read_cmdline", "--exact", "--include-ignored", "--nocapture", "--test-threads=1"],
            Duration::from_secs(120),
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let console = g.vm.console();
    for line in console.get(off..).unwrap_or("").lines().filter(|l| l.contains("V6_GUEST")) {
        eprintln!("V6 [{label}] {line}");
    }
    let _ = g.vm.kill();
    eprintln!("V6 [{label}] guest exit={code} expect_token={expect}");
    code
}

#[overdrive_test(guest = "cgroup-default")]
fn token_reaches_guest_and_not_host() {
    let host = cmdline();
    let host_has = host.split_whitespace().any(|t| t == TOKEN);
    eprintln!("V6 HOST_CMDLINE {}", host.trim());
    eprintln!("V6 HOST_HAS_TOKEN {host_has}");
    assert!(!host_has, "the host's command line must not carry the token");

    let with = boot_and_read(
        "tok",
        13,
        ScriptedBootExtra { cmdline: vec![TOKEN.to_owned()], memory_mib: None, vcpus: None },
        "1",
    );
    let without = boot_and_read("ctl", 14, ScriptedBootExtra::default(), "0");
    assert_eq!(with, 0, "guest with the token must read it from /proc/cmdline");
    assert_eq!(without, 0, "control guest without the token must not carry it");
}
