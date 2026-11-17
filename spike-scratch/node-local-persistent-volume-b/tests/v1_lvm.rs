//! V-1: LVM on a minimal image-equivalent node without udev (GH #348). Throwaway probe.
//!
//! One outer-guest test holds the GPT backing file (tmpfs, so it survives across the inner
//! guests) and boots four nested Cloud Hypervisor guests over it, one after another:
//! a = steps 1-3 (+ data), b = step 4 (fresh guest), c = step 5 (crash storm, ended by an
//! unclean VMM SIGKILL), d = a cold guest over the storm's result. The inner guest's root
//! filesystem is mounted read-only; /run is the only tmpfs the node writes.
use nlpv_v1_probe::gpt::*;
use nlpv_v1_probe::*;
use overdrive_testing::overdrive_test;
use std::time::Duration;

const GPT: &str = "/nlpv/node-data.gpt";
const DISK_MIB: u64 = 400;

fn fnv_file(path: &str) -> u64 {
    let d = std::fs::read(path).unwrap();
    fnv(&d)
}

/// Boot one inner guest over the backing file; returns its `NLPV-G ` lines.
fn boot(name: &str, phase: &str, rootfs: &str, stop_on: &str, kill_unclean: bool) -> Vec<String> {
    let mut spec = VmmSpec::new(name, &ch_binary("stock"), rootfs);
    spec.disks = vec![format!("path={GPT},image_type=raw")];
    spec.cmdline_params = vec![format!("nlpv.phase={phase}"), "nlpv.sysdir=run".into()];
    spec.ro_root = true;
    spec.memory_mib = 256;
    spec.vcpus = 2;
    let mut v = launch(&spec);
    let t0 = std::time::Instant::now();
    let mut lines: Vec<String> = vec![];
    v.follow_until(Duration::from_secs(420), |l, _| { lines.push(l.to_owned()); l.starts_with(stop_on) });
    let reached = lines.iter().any(|l| l.starts_with(stop_on));
    let exit = if kill_unclean {
        v.kill9();
        let s = v.wait_exit(Duration::from_secs(5));
        format!("unclean SIGKILL of the VMM after {stop_on:?}: {s:?}")
    } else {
        let s = v.wait_exit(Duration::from_secs(10));
        if s.is_none() { v.kill9(); }
        format!("guest exit after {stop_on:?}: {s:?} (None = VMM did not exit in 10s; SIGKILLed)")
    };
    ev(format!("GUEST {name} phase={phase} wall={:.1}s reached_stop_line={reached}; {exit}", t0.elapsed().as_secs_f64()));
    // Evidence: the whole guest console, plus the VMM stderr.
    let all = v.console_text();
    eprintln!("=== NLPV console [{name}] begin ===");
    for l in all.lines() { let l = l.trim_end_matches('\r'); eprintln!("{l}"); }
    eprintln!("=== NLPV console [{name}] end ===");
    let se = v.stderr_text();
    if !se.trim().is_empty() { eprintln!("=== NLPV vmm stderr [{name}] ===\n{se}"); }
    v.cleanup();
    assert!(reached, "phase {phase} never printed {stop_on:?}");
    lines
}

fn find<'a>(lines: &'a [String], needle: &str) -> Option<&'a String> { lines.iter().find(|l| l.contains(needle)) }
fn field(line: &str, key: &str) -> String {
    line.split(key).nth(1).map(|r| r.split_whitespace().next().unwrap_or("").to_owned()).unwrap_or_default()
}
/// The lines of one `CMD [label]` block (until the next CMD / STEP / LATENCY line).
fn block<'a>(lines: &'a [String], label: &str) -> Vec<&'a String> {
    let start = lines.iter().position(|l| l.starts_with(&format!("CMD [{label}]")));
    let Some(s) = start else { return vec![] };
    let mut v = vec![&lines[s]];
    for l in &lines[s + 1..] { if l.starts_with("CMD [") || l.starts_with("STEP") || l.starts_with("LATENCY") || l.starts_with("NODE") { break; } v.push(l); }
    v
}
fn median_of(lines: &[String], name_part: &str) -> Option<f64> {
    let l = find(lines, &format!("LATENCY {name_part}"))?;
    field(l, "median=").trim_end_matches("ms").parse().ok()
}

#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[test]
fn v1_lvm_five_steps() {
    outer_setup_for(1000);
    ev(format!("V-1 probe; host/outer kernel (== inner kernel) {KERNEL_RELEASE}"));
    eprintln!("LVM MANIFEST (what the node carries):\n{LVM_MANIFEST}");
    sh("outer", "uname -r; cat /proc/sys/kernel/random/boot_id; cloud-hypervisor --version");

    // GPT backing file with four partitions; exactly one is named overdrive-volumes.
    let mib = |m: u64| m * 2048;
    let parts = [
        Part { first_lba: mib(1), last_lba: mib(17) - 1, name: "efi-like-other", type_guid: guid(LINUX_FS), guid: guid("11111111-0000-4000-8000-000000000001") },
        Part { first_lba: mib(17), last_lba: mib(273) - 1, name: "overdrive-volumes", type_guid: guid(LINUX_LVM), guid: guid("11111111-0000-4000-8000-000000000002") },
        Part { first_lba: mib(273), last_lba: mib(337) - 1, name: "other-lvm-decoy", type_guid: guid(LINUX_LVM), guid: guid("11111111-0000-4000-8000-000000000003") },
        Part { first_lba: mib(337), last_lba: mib(369) - 1, name: "overdrive-volumes-old", type_guid: guid(LINUX_FS), guid: guid("11111111-0000-4000-8000-000000000004") },
    ];
    write_gpt(GPT, DISK_MIB << 20, &parts);
    chown(GPT, VMM_UID, VMM_UID);
    ev(format!("GPT backing file {GPT}: {DISK_MIB} MiB; partitions: 1=efi-like-other(16M) 2=overdrive-volumes(256M) 3=other-lvm-decoy(64M) 4=overdrive-volumes-old(32M)"));

    let rootdir = "/nlpv/run/node-img";
    std::fs::create_dir_all(rootdir).unwrap();
    let rootfs = build_inner_rootfs(rootdir, 96);
    let rootfs_hash_before = fnv_file(&rootfs);
    ev(format!("inner rootfs {rootfs}: fnv-64 before = {rootfs_hash_before:016x}"));
    sh("rootfs-listing", &format!("busybox ls -la {rootdir}/rootdir {rootdir}/rootdir/usr/sbin; busybox du -sh {rootdir}/rootdir; busybox ls -l {rootfs}"));

    let a = boot("a", "a", &rootfs, "PHASE a DONE", false);
    let b = boot("b", "b", &rootfs, "PHASE b DONE", false);
    let c = boot("c", "c", &rootfs, "STEP5 CHECKS-DONE", true);
    let d = boot("d", "d", &rootfs, "PHASE d DONE", false);
    let rootfs_hash_after = fnv_file(&rootfs);

    // ------------------------------------------------------------ verdicts
    let mut verdicts: Vec<(String, bool, String)> = vec![];
    let mut v = |name: &str, ok: bool, detail: String| { eprintln!("V1 VERDICT {name}: {} {detail}", if ok { "PASS" } else { "FAIL" }); verdicts.push((name.into(), ok, detail)); };

    // node baseline
    let ro_ok = a.iter().filter(|l| l.starts_with("ROOT-WRITE")).all(|l| l.contains("failed as required") && l.contains("errno Some(30)")) && a.iter().any(|l| l.starts_with("ROOT-WRITE"));
    let mount_ro = a.iter().any(|l| l.contains("/dev/root / ext4 ro,") || l.contains("/dev/vda / ext4 ro,"));
    let no_udev = a.iter().any(|l| l.contains("/run/udev exists: false")) && a.iter().any(|l| l.contains("udev/lvm daemons present: false"));
    v("node-baseline: root read-only, no udev, no LVM daemons", ro_ok && mount_ro && no_udev && rootfs_hash_before == rootfs_hash_after,
        format!("root_write_refused_EROFS={ro_ok} root_mounted_ro={mount_ro} no_udev_no_daemons={no_udev} rootfs_image_hash_unchanged={}", rootfs_hash_before == rootfs_hash_after));

    // step 1
    let disc = find(&a, "DISCOVERY RESULT").cloned().unwrap_or_default();
    let filt = block(&a, "filtered-pvs-all").into_iter().cloned().collect::<Vec<_>>().join("\n");
    let unf = block(&a, "unfiltered-pvs-all").into_iter().cloned().collect::<Vec<_>>().join("\n");
    let unf_vgs = block(&a, "unfiltered-vgs").into_iter().cloned().collect::<Vec<_>>().join("\n");
    let hidden = filt.contains("/dev/vdb2") && !["/dev/vda", "/dev/root", "/dev/vdb1", "/dev/vdb3", "/dev/vdb4", "/dev/vdb ", "decoy"].iter().any(|x| filt.contains(x));
    let shows_all = ["/dev/root", "/dev/vdb1", "/dev/vdb3", "/dev/vdb4"].iter().all(|x| unf.contains(x)) && unf_vgs.contains("decoy");
    let refused = find(&a, "STEP1 RESULT").map(|l| l.contains("pvcreate_other=true") && l.contains("vgcreate_root=true") && l.contains("vgcreate_decoy_pv=true")).unwrap_or(false);
    let hashes = |tag: &str, lines: &[String]| -> Vec<String> { lines.iter().filter(|l| l.starts_with(&format!("OTHER-HASH [{tag}]"))).map(|l| l.split(']').nth(1).unwrap_or("").trim().to_owned()).collect() };
    let h_decoy = hashes("a-after-decoy", &a);
    let untouched = !h_decoy.is_empty() && [hashes("a-end", &a), hashes("b-end", &b), hashes("c-end", &c), hashes("d-end", &d)].iter().all(|h| *h == h_decoy);
    v("step1: discovery by PARTNAME, LVM filter hides every other device", disc.contains("-> /dev/vdb2") && hidden && shows_all && refused && untouched,
        format!("discovered={:?}; filtered_pvs_lists_only_vdb2={hidden}; unfiltered_sees_all_and_decoy_vg={shows_all}; writes_to_hidden_devices_refused={refused}; hidden_devices_byte_identical_through_all_four_guests={untouched}", disc.trim()));

    // step 2
    let s2 = find(&a, "STEP2 RESULT").cloned().unwrap_or_default();
    let node = find(&a, "NODE [ovd-test]").cloned().unwrap_or_default();
    v("step2: vgcreate -s 4m + lvcreate -L 64m, node without udev", s2.contains("exit=Some(0)") && s2.contains("is_block_device=true") && node.contains("dm/name=\"overdrive-ovd--test\"") && node.contains("size_sectors=\"131072\""),
        format!("{} | {}", s2.trim(), node.trim()));

    // step 3
    let jstart = a.iter().position(|l| l == "LVSJSON-BEGIN");
    let jend = a.iter().position(|l| l == "LVSJSON-END");
    let (mut parsed, mut pdetail) = (false, String::from("no json"));
    if let (Some(s), Some(e)) = (jstart, jend) {
        let text = a[s + 1..e].join("\n");
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(j) => {
                let row = &j["report"][0]["lv"];
                let t = row.as_array().and_then(|r| r.iter().find(|x| x["lv_name"] == "ovd-test"));
                pdetail = format!("parsed with serde_json: report[0].lv = {row}");
                parsed = t.map(|x| x["lv_size"] == "67108864B" && x["vg_extent_size"] == "4194304B" && x["vg_free"].is_string() && x["lv_attr"].is_string()).unwrap_or(false);
            }
            Err(e) => pdetail = format!("json parse error: {e}: {text}"),
        }
    }
    v("step3: lvs --reportformat json parses", parsed, pdetail);

    // step 4
    let boot_a = find(&a, "PHASE a START").map(|l| field(l, "boot_id=")).unwrap_or_default();
    let boot_b = find(&b, "PHASE b START").map(|l| field(l, "boot_id=")).unwrap_or_default();
    let fresh = !boot_a.is_empty() && boot_a != boot_b;
    let nonode_before = find(&b, "dm devices at boot").map(|l| l.contains("[]") && l.contains("present: false")).unwrap_or(false);
    let rb = b.iter().filter(|l| l.contains("STEP4 RESULT")).all(|l| l.contains("true")) && b.iter().any(|l| l.contains("STEP4 RESULT"));
    let ay_ok = b.iter().any(|l| l.contains("STEP4 node after lvchange -ay: is_block_device=true"));
    v("step4: fresh guest, lvchange -ay re-creates the node, data reads back", fresh && nonode_before && rb && ay_ok,
        format!("fresh_guest(boot_id differs: {boot_a} vs {boot_b})={fresh} no_dm_device_or_node_at_boot={nonode_before} node_after_lvchange_ay={ay_ok} readback_all_match(first activation and after 7 -an/-ay cycles)={rb}"));

    // step 5
    let storm = find(&c, "STEP5 STORM DONE").cloned().unwrap_or_default();
    let rec = find(&c, "STEP5 recovery exits").cloned().unwrap_or_default();
    let rec_ok = rec.contains("vgs=Some(0)") && rec.contains("lvs=Some(0)") && rec.contains("vgchange_ay=Some(0)") && rec.contains("vgck=Some(0)") && rec.contains("clean_lvcreate=Some(0)") && rec.contains("node_present=true");
    let c_data = find(&c, "STEP5 RESULT").map(|l| l.contains("true")).unwrap_or(false);
    let no_clean_fail = storm.contains("clean_op_failures_between_kills=0") && storm.contains("cleanup_failures=0") && storm.contains("iterations_with_leaked_extents=0");
    let d_ex = find(&d, "STEP5b exits").cloned().unwrap_or_default();
    let d_ok = d_ex.contains("vgchange_ay=Some(0)") && d_ex.contains("vgck=Some(0)") && d_ex.contains("clean_lvcreate=Some(0)") && find(&d, "STEP5b RESULT").map(|l| l.contains("true")).unwrap_or(false);
    let free_equal = {
        let before = find(&c, "STEP5 baseline vgs:").map(|l| l.split(':').nth(1).unwrap_or("").trim().split_whitespace().take(2).collect::<Vec<_>>().join(" ")).unwrap_or_default();
        let after = find(&c, "STEP5 vgs after cleanup").map(|l| l.split(':').nth(1).unwrap_or("").trim().split(" (").next().unwrap_or("").split_whitespace().take(2).collect::<Vec<_>>().join(" ")).unwrap_or_default();
        eprintln!("V1 extents: free/total before storm = {before:?}, after storm + cleanup = {after:?}");
        !before.is_empty() && before == after
    };
    v("step5: 20 SIGKILLed lvcreate/lvremove iterations, recovery with no manual repair", rec_ok && c_data && no_clean_fail && d_ok && free_equal,
        format!("{} | recovery: {} | clean_lvcreate_between_kills_ok={no_clean_fail} data_intact={c_data} no_extents_leaked={free_equal} | cold fresh guest after unclean VMM kill: {}", storm.trim(), rec.trim(), d_ex.trim()));

    // latencies (the boot probe's cost)
    for (nm, p) in [("vgcreate", "vgcreate -s 4m"), ("lvcreate total", "lvcreate -L 64m total"), ("lvcreate node-appearance", "lvcreate -L 64m node appeared"), ("lvremove", "lvremove -f total"), ("pvs (discovery)", "pvs"), ("vgs", "vgs"), ("lvs -a", "lvs -a")] {
        let src = if p.starts_with("pvs") || p == "vgs" || p == "lvs -a" { &b } else { &a };
        eprintln!("V1 LATENCY {nm}: median {:?} ms", median_of(src, p));
    }
    eprintln!("V1 LATENCY lvchange -ay total: median {:?} ms; node appeared: median {:?} ms; lvchange -an: median {:?} ms", median_of(&b, "lvchange -ay total"), median_of(&b, "lvchange -ay node appeared"), median_of(&b, "lvchange -an"));
    for (n, ok, d) in &verdicts { eprintln!("V1 SUMMARY {n}: {}{}", if *ok { "PASS" } else { "FAIL" }, if *ok { String::new() } else { format!(" -- {d}") }); }
    assert!(verdicts.iter().all(|(_, ok, _)| *ok), "V-1 has failing steps; see V1 VERDICT lines");
}
