from pathlib import Path
source=Path('crates/overdrive-testing/bin/fwd_guest_suite.rs').read_text()
source=source[source.index('use std::fs;'):source.index('\nfn main()')]
start=source.index('fn workspace_root() -> PathBuf {')
end=source.index('\nfn workspace_bpf_artifact()',start)
source=source[:start]+'fn workspace_root() -> PathBuf { std::env::current_dir().expect("spike runs at the selected checkout root") }\n'+source[end:]
source+='''
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert!(!args.is_empty(), "nextest supplies its actual selected binary");
    let identity = std::env::var("NEXTEST_TEST_NAME").unwrap_or_else(|_| "direct-wrapper-call".to_owned());
    println!("METAL-DESIGN-WRAPPER binary_id={} identity={} test_group={} argv={args:?}", std::env::var("NEXTEST_BINARY_ID").unwrap_or_default(), identity, std::env::var("NEXTEST_TEST_GROUP").unwrap_or_default());
    std::env::set_current_dir(std::env::var_os("OVERDRIVE_METAL_DESIGN_REPO").expect("driver names its actual selected checkout")).expect("enter selected checkout");
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("existing native GuestKit provisions");
    let tests = InGuestTests { binary: PathBuf::from(&args[0]) };
    let boot_id_file = staging.join(format!("metal-design-host-boot-{}", std::process::id()));
    fs::copy("/proc/sys/kernel/random/boot_id", &boot_id_file).expect("capture launcher boot ID");
    let mut extra = base_tools();
    extra.push(GuestFile { host: boot_id_file.clone(), guest: PathBuf::from("/metal-design-host-boot-id"), with_shared_libraries: false });
    let mut guest = GuestRun::boot(&kit, &tests, "metal-design-probe", 7, extra, Vec::new()).expect("reuse existing probe group boot and image setup");
    let mut command = vec![GUEST_PEER_GUEST_PATH, subcommand::RUN_WITH_MODULES, LOOPBACK_MODULE_GUEST_PATH, "--", IN_GUEST_TESTS_PATH];
    command.extend(args[1..].iter().map(String::as_str));
    let status = guest.guest.peer().exec(&command, CELL_BOUND).expect("existing framed peer returns exact child result");
    let cid = guest.guest.vm.cid();
    guest.finish("metal-design-probe").expect("existing group cleanup powers off and observes exit");
    fs::remove_file(boot_id_file).expect("remove owned boot-ID fixture");
    println!("METAL-DESIGN-WRAPPER-RESULT identity={identity} child_status={status} cid={cid} cleanup=complete");
    std::process::exit(status);
}
'''
p=Path('spike-scratch/metal-run-isolation/increment-e/src/main.rs')
p.write_text('#![allow(dead_code)]\n'+source)
