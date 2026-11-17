//! V-1 extra evidence (beyond the five steps): the whole node dies mid-operation. A guest loops
//! lvcreate/lvremove; the harness SIGKILLs the VMM at a random point; the next fresh guest reports
//! the store's state with ordinary commands and loops again. After 10 crashes a cold guest checks
//! recovery with no manual repair and that the earlier data is intact.
use nlpv_v1_probe::gpt::*;
use nlpv_v1_probe::*;
use overdrive_testing::overdrive_test;
use std::time::Duration;

const GPT: &str = "/nlpv/node-data.gpt";

fn boot(name: &str, params: Vec<String>, rootfs: &str, stop_on: &str, kill_after_ms: Option<u64>) -> Vec<String> {
    let mut spec = VmmSpec::new(name, &ch_binary("stock"), rootfs);
    spec.disks = vec![format!("path={GPT},image_type=raw")];
    spec.cmdline_params = params;
    spec.cmdline_params.push("nlpv.sysdir=run".into());
    spec.ro_root = true; spec.memory_mib = 256; spec.vcpus = 2;
    let mut v = launch(&spec);
    let mut seen: Vec<String> = vec![];
    let mut kill_at: Option<std::time::Instant> = None;
    v.follow_until(Duration::from_secs(300), |l, vv| {
        seen.push(l.to_owned());
        if l.starts_with("LOOP-START") { if let Some(ms) = kill_after_ms { kill_at = Some(std::time::Instant::now() + Duration::from_millis(ms)); } }
        if let Some(t) = kill_at { while std::time::Instant::now() < t { std::thread::sleep(Duration::from_micros(100)); } vv.kill9(); return true; }
        l.starts_with(stop_on)
    });
    let how = if kill_after_ms.is_some() { let s = v.wait_exit(Duration::from_secs(5)); format!("VMM SIGKILLed {}ms after LOOP-START: {s:?}", kill_after_ms.unwrap()) } else { format!("{:?}", v.wait_exit(Duration::from_secs(10))) };
    ev(format!("GUEST {name}: {how}"));
    let full = v.console_text();
    eprintln!("=== NLPV console [{name}] begin ===");
    for l in full.lines() { eprintln!("{}", l.trim_end_matches('\r')); }
    eprintln!("=== NLPV console [{name}] end ===");
    v.cleanup();
    // all guest lines, including those written between the kill decision and the kill itself
    full.lines().filter_map(|l| l.trim_end_matches('\r').split("NLPV-G ").nth(1).map(str::to_owned)).collect()
}

#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[test]
fn v1_vmm_crash_series() {
    outer_setup_for(1000);
    let mib = |m: u64| m * 2048;
    let parts = [
        Part { first_lba: mib(1), last_lba: mib(17) - 1, name: "efi-like-other", type_guid: guid(LINUX_FS), guid: guid("11111111-0000-4000-8000-000000000001") },
        Part { first_lba: mib(17), last_lba: mib(273) - 1, name: "overdrive-volumes", type_guid: guid(LINUX_LVM), guid: guid("11111111-0000-4000-8000-000000000002") },
        Part { first_lba: mib(273), last_lba: mib(337) - 1, name: "other-lvm-decoy", type_guid: guid(LINUX_LVM), guid: guid("11111111-0000-4000-8000-000000000003") },
        Part { first_lba: mib(337), last_lba: mib(369) - 1, name: "overdrive-volumes-old", type_guid: guid(LINUX_FS), guid: guid("11111111-0000-4000-8000-000000000004") },
    ];
    write_gpt(GPT, 400 << 20, &parts);
    chown(GPT, VMM_UID, VMM_UID);
    let d = "/nlpv/run/node-img"; std::fs::create_dir_all(d).unwrap();
    let rootfs = build_inner_rootfs(d, 96);
    let a = boot("a", vec!["nlpv.phase=a".into()], &rootfs, "PHASE a DONE", None);
    assert!(a.iter().any(|l| l.contains("STEP2 RESULT") && l.contains("is_block_device=true")), "setup phase a failed");
    // seeded from the clock; printed so a run can be reproduced in shape (not bit-for-bit: wall-clock timing)
    let mut x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64 | 1;
    let mut rnd = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
    let mut summary = vec![];
    for k in 1..=15u64 {
        let delay = rnd() % 600; // ms after the first lvcreate starts; one lvcreate+lvremove cycle is ~60-80 ms
        let lines = boot(&format!("e{k}"), vec!["nlpv.phase=e".into(), format!("nlpv.seq={k}")], &rootfs, "NEVER", Some(delay));
        let res = lines.iter().find(|l| l.contains("RESULT exits")).cloned().unwrap_or_default();
        let warn: Vec<String> = lines.iter().filter(|l| { let x = l.to_lowercase(); !l.starts_with("ROOT-WRITE") && !l.starts_with("lvm closure") && !l.starts_with("cmdline") && !l.contains("devtmpfs") && (x.contains("warning") || x.contains("inconsistent") || x.contains("missing") || x.contains("corrupt") || x.contains("repair") || x.contains("failed") || x.contains("error")) }).cloned().collect();
        let last_op = lines.iter().rev().find(|l| l.starts_with("LOOP begin")).cloned().unwrap_or_else(|| "none (killed before the first op began)".into());
        let cycles = lines.iter().filter(|l| l.starts_with("LOOP cycle")).count();
        let boot_lvs = lines.iter().position(|l| l.starts_with("CMD [lvs]")).map(|p| lines[p + 1..].iter().skip_while(|l| !l.contains("LV ")).take_while(|l| !l.starts_with("CMD [")).map(|l| l.trim().to_owned()).collect::<Vec<_>>().join(" ; ")).unwrap_or_default();
        summary.push(format!("crash {k}: this boot saw (state left by crash {}): {res} | LVs at boot: [{boot_lvs}] | warnings: {warn:?}\n    ... then ran {cycles} full create+remove cycles; VMM SIGKILLed {delay}ms after LOOP-START with {last_op:?} the last op begun", k - 1));
    }
    let f = boot("f", vec!["nlpv.phase=f".into()], &rootfs, "PHASE f DONE", None);
    let fin = f.iter().find(|l| l.contains("CRASHFINAL RESULT")).cloned().unwrap_or_default();
    let data = f.iter().any(|l| l.contains("CRASHFINAL DATA readback_all_match=true"));
    for s in &summary { eprintln!("V1 CRASH {s}"); }
    eprintln!("V1 CRASH FINAL: {fin} data_intact={data}");
    let all_ok = summary.iter().all(|s| s.contains("vgs=Some(0)") && s.contains("lvs=Some(0)") && s.contains("vgck=Some(0)") && s.contains("vgchange_ay=Some(0)"));
    let fin_ok = fin.contains("vgs=Some(0)") && fin.contains("vgck=Some(0)") && fin.contains("vgchange_ay=Some(0)") && fin.contains("cleanup_failures=0") && fin.contains("clean_lvcreate=Some(0)") && fin.contains("(baseline 47 63)") && fin.contains("orphan_dm=[]");
    eprintln!("V1 VERDICT vmm-crash-series: {}", if all_ok && fin_ok && data { "PASS" } else { "FAIL" });
    assert!(all_ok && fin_ok && data, "VMM crash series failed");
}
