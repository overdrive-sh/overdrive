//! V-1 diagnostic: how does LVM get its system dir on a read-only root?
use nlpv_v1_probe::*;
use overdrive_testing::overdrive_test;
use std::time::Duration;

fn boot_s(name: &str, rootfs: &str) {
    let mut spec = VmmSpec::new(name, &ch_binary("stock"), rootfs);
    spec.disks = vec![format!("path=/nlpv/node-data.gpt,image_type=raw")];
    spec.cmdline_params = vec!["nlpv.phase=s".into()];
    spec.ro_root = true; spec.memory_mib = 256;
    let mut v = launch(&spec);
    v.follow_until(Duration::from_secs(120), |l, _| l.starts_with("PHASE s DONE"));
    let _ = v.wait_exit(Duration::from_secs(10));
    eprintln!("=== NLPV console [{name}] begin ===");
    for l in v.console_text().lines() { eprintln!("{}", l.trim_end_matches('\r')); }
    eprintln!("=== NLPV console [{name}] end ===");
    v.cleanup();
}

#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[test]
fn v1_sysdir_variants() {
    outer_setup_for(500);
    use nlpv_v1_probe::gpt::*;
    let mib = |m: u64| m * 2048;
    let parts = [
        Part { first_lba: mib(1), last_lba: mib(17) - 1, name: "efi-like-other", type_guid: guid(LINUX_FS), guid: guid("11111111-0000-4000-8000-000000000001") },
        Part { first_lba: mib(17), last_lba: mib(273) - 1, name: "overdrive-volumes", type_guid: guid(LINUX_LVM), guid: guid("11111111-0000-4000-8000-000000000002") },
    ];
    write_gpt("/nlpv/node-data.gpt", 400 << 20, &parts);
    chown("/nlpv/node-data.gpt", VMM_UID, VMM_UID);
    let d1 = "/nlpv/run/img-plain"; std::fs::create_dir_all(d1).unwrap();
    let r1 = build_inner_rootfs(d1, 96);
    ev("IMAGE VARIANT 1: no /etc/lvm in the read-only root");
    boot_s("s1", &r1);
    unsafe { std::env::set_var("NLPV_BAKE_ETC_LVM", "1"); }
    let d2 = "/nlpv/run/img-baked"; std::fs::create_dir_all(d2).unwrap();
    let r2 = build_inner_rootfs(d2, 96);
    ev("IMAGE VARIANT 2: empty /etc/lvm baked into the read-only root");
    boot_s("s2", &r2);
}
