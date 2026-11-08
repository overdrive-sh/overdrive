/// CONTRACT_SHAPE: bounded-change.
#[test]
fn selected_guest_status_is_255() {
    use std::io::Write as _;
    println!("METAL-DESIGN-255 boot_id={}", std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap().trim());
    std::io::stdout().flush().unwrap();
    std::process::exit(255);
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn selected_guest_has_an_independent_kernel_boot_id() {
    let host = std::fs::read_to_string("/metal-design-host-boot-id").unwrap();
    let guest = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    assert_ne!(host.trim(), guest.trim());
    println!("METAL-DESIGN-GUEST boot_id={}", guest.trim());
    std::thread::sleep(std::time::Duration::from_secs(5));
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn selected_guest_timeout_owns_resources() {
    println!("METAL-DESIGN-TIMEOUT-BODY running=true");
    std::thread::sleep(std::time::Duration::from_secs(45));
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn selected_guest_environment_is_preserved() {
    assert_eq!(std::env::var("PROPTEST_CASES").unwrap(), "7");
    assert_eq!(std::env::var("PROPTEST_RNG_SEED").unwrap(), "123456");
    assert_eq!(std::env::var("RUST_BACKTRACE").unwrap(), "1");
    assert_eq!(std::env::var("NEXTEST_TEST_NAME").unwrap(), "selected_guest_environment_is_preserved");
    assert_eq!(std::env::var("NEXTEST_ATTEMPT").unwrap(), "1");
    println!("METAL-DESIGN-ENV preserved=true");
}
