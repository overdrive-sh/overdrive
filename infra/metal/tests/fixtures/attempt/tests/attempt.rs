//! Exact selected ELF/helper/attempt environment and real guest-kernel oracle.
//! The external tooling acceptance driver expects the intentional 255/timeout.
use std::io::Write as _;

/// CONTRACT_SHAPE: bounded-change (this fresh guest body observes only its own
/// kernel/env/helper process; physical host and other attempts are complements).
#[test]
#[ignore = "external Metal acceptance fixture: deliberate body status 255"]
fn selected_environment_and_status() {
    let host = std::env::var("RUST_LOG").expect("external driver supplied host boot ID");
    let host_boot_id = host.strip_prefix("metal_host_boot_id=").expect("host observation prefix");
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .expect("actual guest kernel boot ID");
    assert_ne!(boot_id.trim(), host_boot_id, "selected body must execute in a guest kernel");
    assert_eq!(std::env::var("PROPTEST_CASES").expect("property budget"), "7");
    assert_eq!(std::env::var("PROPTEST_RNG_SEED").expect("property seed"), "123456");
    assert_eq!(std::env::var("RUST_BACKTRACE").expect("backtrace"), "1");
    assert_eq!(
        std::env::var("NEXTEST_TEST_NAME").expect("selected identity"),
        "selected_environment_and_status"
    );
    let attempt = std::env::var("NEXTEST_ATTEMPT").expect("nextest attempt");
    assert!(attempt == "1" || attempt == "2", "existing nextest retry context");
    let helper_path = std::env::var("NEXTEST_BIN_EXE_metal-run-isolation-helper")
        .expect("nextest helper metadata staged into guest");
    let helper = std::process::Command::new(helper_path)
        .output()
        .expect("actual reported helper executable in guest");
    assert!(helper.status.success());
    assert_eq!(helper.stdout, b"METAL-ISOLATION-HELPER actual=true\n");
    println!("METAL-ISOLATION-ATTEMPT attempt={attempt} boot_id={} helper=true", boot_id.trim());
    std::io::stdout().flush().expect("publish actual body observation before exiting");
    std::process::exit(255);
}

/// CONTRACT_SHAPE: bounded-change (guest process deliberately exceeds the existing
/// nextest timeout; the owning invocation must end descendants and owned roots).
#[test]
#[ignore = "external Metal acceptance fixture: deliberate nextest timeout"]
fn selected_attempt_waits_for_its_timeout() {
    let source = std::env::var("RUST_LOG").expect("external physical host identity");
    let host = source.strip_prefix("metal_host_boot_id=").expect("host identity prefix");
    let guest = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .expect("actual selected kernel identity");
    assert_ne!(guest.trim(), host, "timeout must exercise an actual disposable guest");
    println!("METAL-ISOLATION-TIMEOUT body_started=true");
    std::io::stdout().flush().expect("body observation");
    std::thread::sleep(std::time::Duration::from_secs(180));
    panic!("nextest must terminate the attempted body before its fixture wait ends");
}

/// CONTRACT_SHAPE: bounded-change (only this positively identified disposable
/// guest resets; its real peer/VMM error must reach the existing execution adapter).
#[test]
#[ignore = "external Metal acceptance fixture: deliberate guest reset during exec"]
fn selected_attempt_resets_its_guest() {
    let host = std::env::var("RUST_LOG").expect("external host boot identity");
    let host_boot_id = host.strip_prefix("metal_host_boot_id=").expect("host identity prefix");
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .expect("actual guest boot identity");
    assert_ne!(boot_id.trim(), host_boot_id, "refuse a reset on the physical launcher");
    println!("METAL-ISOLATION-RESET guest_boot_id={} resetting=true", boot_id.trim());
    std::io::stdout().flush().expect("retain real body entry before peer loss");
    unsafe extern "C" {
        fn reboot(command: std::ffi::c_int) -> std::ffi::c_int;
    }
    // SAFETY: libc reboot has no pointer arguments. The host boot-ID guard
    // above proves this body is in its own disposable kernel before the call.
    let result = unsafe { reboot(0x0123_4567) };
    panic!("guest reboot unexpectedly returned {result}: {}", std::io::Error::last_os_error());
}
