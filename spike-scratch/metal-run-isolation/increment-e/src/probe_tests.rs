/// CONTRACT_SHAPE: bounded-change.
#[test]
fn selected_guest_status_is_255() {
    std::process::exit(255);
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn selected_guest_has_an_independent_kernel_boot_id() {
    let host = std::fs::read_to_string("/metal-design-host-boot-id").unwrap();
    let guest = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    assert_ne!(host.trim(), guest.trim());
    println!("METAL-DESIGN-GUEST boot_id={}", guest.trim());
}
