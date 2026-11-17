//! Host half of the kache spike: runs `scripts/host-launch.sh` with the plan
//! named in the `PLAN` file. Everything else happens inside Cloud Hypervisor
//! guests the script boots. `native_launcher` gives the script a private
//! network, mount and cgroup namespace, so its bridge, taps and mounts never
//! reach the host's.
use overdrive_testing_macros::overdrive_test;
use std::process::Command;

#[overdrive_test(native_launcher)]
#[test]
fn kache_spike() {
    let root = env!("CARGO_MANIFEST_DIR");
    let plan = std::fs::read_to_string(format!("{root}/PLAN")).expect("PLAN file");
    let status = Command::new("bash")
        .arg(format!("{root}/scripts/host-launch.sh"))
        .arg(plan.trim())
        .current_dir(root)
        .status()
        .expect("spawn host-launch.sh");
    assert!(status.success(), "host-launch.sh failed: {status}");
}
