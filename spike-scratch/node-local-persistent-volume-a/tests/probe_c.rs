use nlpv_probe::*;
use overdrive_core::vm::config::MemoryPlan;
use overdrive_testing::overdrive_test;
use std::time::Duration;

macro_rules! gt { ($name:ident, $body:expr) => {
    #[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
    #[test]
    fn $name() { outer_setup(); let f: &dyn Fn(&str) = &$body; f(stringify!($name)); }
}; }

fn disk(path: &str, extra: &str) -> String { let mut a = format!("path={path},image_type=raw"); if !extra.is_empty() { a.push(','); a.push_str(extra); } a }

/// C1: RLIMIT_FSIZE as Overdrive derives it (max(rootfs master, guest RAM)) vs a larger volume.
fn c1(name: &str, bdev: bool, no_uring: bool, no_aio: bool) {
    let ram_mib = 128u64;
    let rootfs_bytes = 24u64 << 20;
    let plan = MemoryPlan::derive(ram_mib << 20);
    let fsize = rootfs_bytes.max(plan.guest_bytes()); // == VmConfig::rlimit_fsize()
    let vol = make_volume(name, bdev, 256);
    let rootdir = format!("/nlpv/run/{name}-img"); std::fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(name, &ch_binary("stock"), &rootfs);
    spec.memory_mib = ram_mib; spec.fsize = fsize;
    let mut extra = String::new();
    if no_uring { extra.push_str("_disable_io_uring=on"); }
    if no_aio { if !extra.is_empty() { extra.push(','); } extra.push_str("_disable_aio=on"); }
    spec.disks.push(disk(&vol.path, &extra));
    spec.cmdline_params = vec!["nlpv.mode=fill".into(), "nlpv.bs=1048576".into(), "nlpv.odirect=1".into(), "nlpv.sync_every=8".into()];
    spec.ch_extra = vec!["-v".into(), "--log-file".into(), format!("/nlpv/run/{name}/ch.log")];
    let mut v = launch(&spec);
    ev(format!("C1 {name}: volume {} MiB ({}), RLIMIT_FSIZE={fsize} ({} MiB) = max(rootfs {} MiB, RAM {ram_mib} MiB); {}", 256, if bdev { "block device" } else { "file" }, fsize >> 20, rootfs_bytes >> 20, proc_limit(v.pid(), "Max file size")));
    let mut last_progress = 0u64; let mut errs = vec![];
    v.follow(Duration::from_secs(60), |l, _| {
        if let Some(n) = l.strip_prefix("PROGRESS ") { last_progress = n.trim().parse().unwrap_or(0); }
        if l.starts_with("FIRSTERR") || l.starts_with("SYNCERR") || l.starts_with("FILLEND") || l.starts_with("READBACK") { errs.push(l.to_owned()); ev(format!("guest: {l}")); }
    });
    let status = v.wait_exit(Duration::from_secs(5));
    let sig = status.and_then(|s| std::os::unix::process::ExitStatusExt::signal(&s));
    ev(format!("C1 RESULT {name} bdev={bdev} no_uring={no_uring} no_aio={no_aio}: vmm_exit={status:?} killed_by_signal={sig:?} (SIGXFSZ=25) guest_last_progress_MiB={last_progress} guest_reports={errs:?}"));
    sh("chlog", &format!("busybox tail -5 /nlpv/run/{name}/ch.log; busybox tail -3 /nlpv/run/{name}/ch.stderr"));
    sh("console", &format!("busybox grep -E 'I/O error|blk_update|Buffer' {}/console.log | busybox head -5", v.run_dir));
    v.cleanup();
}
gt!(c1_file_uring, |n| c1(n, false, false, false));
gt!(c1_file_aio, |n| c1(n, false, true, false));
gt!(c1_file_sync, |n| c1(n, false, true, true));
gt!(c1_bdev_uring, |n| c1(n, true, false, false));

/// C1b: the same file volume with RLIMIT_FSIZE >= volume works (control), and growth by resize-disk past the limit.
gt!(c1b_control_limit_above_volume, |n| {
    let vol = make_volume(n, false, 256);
    let rootdir = format!("/nlpv/run/{n}-img"); std::fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(n, &ch_binary("stock"), &rootfs);
    spec.fsize = 512 << 20; spec.memory_mib = 128;
    spec.disks.push(disk(&vol.path, "id=datavol"));
    spec.cmdline_params = vec!["nlpv.mode=fill".into(), "nlpv.bs=1048576".into(), "nlpv.odirect=1".into(), "nlpv.sync_every=8".into()];
    let mut v = launch(&spec);
    let mut end = String::new();
    v.follow(Duration::from_secs(60), |l, _| { if l.starts_with("FILLEND") { end = l.to_owned(); } });
    ev(format!("C1b RESULT control (limit 512MiB >= volume 256MiB): {end}; exit={:?}", v.wait_exit(Duration::from_secs(5))));
    v.cleanup();
});

/// C3: a second writable disk under uid 4200 + landlock + seccomp; and the negative permission cases.
gt!(c3_perms, |n| {
    // positive: covered by every probe_a cell; here the negative cases.
    let vol = make_volume(n, false, 64);
    chown(&vol.path, 0, 0); std::fs::set_permissions(&vol.path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    let mut v = idle_vmm(&format!("{n}-rootonly"), "stock", &[disk(&vol.path, "")]);
    let ok = wait_for_line(&mut v, "READY", Duration::from_secs(10));
    ev(format!("C3 file volume root:root 0600 -> VMM(uid 4200) boots={ok}; stderr=[{}]", tail_ch(&v)));
    v.cleanup();
    let bv = make_volume(&format!("{n}b"), true, 64);
    chown(&bv.path, 0, 0); std::fs::set_permissions(&bv.path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    let mut v = idle_vmm(&format!("{n}-bdev-rootonly"), "stock", &[disk(&bv.path, "")]);
    let ok = wait_for_line(&mut v, "READY", Duration::from_secs(10));
    ev(format!("C3 block device node root:root 0600 -> VMM(uid 4200) boots={ok}; stderr=[{}]", tail_ch(&v)));
    v.cleanup();
    // landlock: a volume path outside any granted rule is derived from --disk by CH itself; show the rule set is auto-derived
    let vol2 = make_volume(&format!("{n}c"), false, 64);
    let mut v = idle_vmm(&format!("{n}-ok"), "stock", &[disk(&vol2.path, "")]);
    let ok = wait_for_line(&mut v, "READY", Duration::from_secs(10));
    ev(format!("C3 control: chown 4200 file volume boots={ok}; seccomp/landlock args in use: {}", std::fs::read_to_string(format!("/proc/{}/cmdline", v.pid())).unwrap_or_default().replace('\0', " ")));
    ev(format!("C3 vmm identity: {} {}", proc_status(v.pid(), "Uid"), proc_status(v.pid(), "Seccomp")));
    sh("c3threads", &format!("for t in /proc/{}/task/*; do busybox grep -H Seccomp $t/status; done | busybox sort | busybox uniq -c | busybox head", v.pid()));
    v.cleanup();
});

/// C2: do buffered writes of the VMM to the volume count against the allocation cgroup's memory.max?
fn c2(name: &str, bdev: bool, direct: bool) {
    let plan = MemoryPlan::derive(128 << 20);
    let dir = "/srv/vm/nlpv"; std::fs::create_dir_all(dir).unwrap();
    std::fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let back = format!("{dir}/{name}.img");
    sparse(&back, 256 << 20);
    let path = if bdev {
        let l = loop_attach(&back, false);
        let sectors = blk_sectors(&l);
        dm_create("c2vol", &format!("0 {sectors} linear {l} 0"))
    } else { chown(&back, VMM_UID, VMM_UID); back.clone() };
    let rootdir = format!("/nlpv/run/{name}-img"); std::fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(name, &ch_binary("stock"), &rootfs);
    spec.memory_mib = 128; spec.fsize = 1 << 40;
    spec.cgroup_memory_max = Some(plan.cgroup_max_bytes());
    spec.disks.push(disk(&path, if direct { "direct=on" } else { "" }));
    spec.cmdline_params = vec!["nlpv.mode=fill".into(), "nlpv.bs=1048576".into(), "nlpv.odirect=1".into(), "nlpv.sync_every=8".into()];
    spec.ch_extra = vec!["-v".into(), "--log-file".into(), format!("/nlpv/run/{name}/ch.log")];
    ev(format!("C2 {name}: guest RAM 128MiB, cgroup memory.max={} MiB (MemoryPlan::derive: guest {} + reserve), volume 256MiB on /srv/vm XFS (real disk path), bdev={bdev} host_direct={direct}", plan.cgroup_max_bytes() >> 20, plan.guest_bytes() >> 20));
    let mut v = launch(&spec);
    let cg = v.cgroup.clone().unwrap();
    let t0 = std::time::Instant::now();
    let mut peak_file = 0u64;
    let mut sampler = 0u32;
    v.follow(Duration::from_secs(90), |l, _| {
        if l.starts_with("PROGRESS") || l.starts_with("FILLEND") || l.starts_with("FIRSTERR") {
            let st = read_cg(&cg, "memory.stat");
            let get = |k: &str| st.lines().find_map(|x| x.strip_prefix(&format!("{k} "))).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            peak_file = peak_file.max(get("file"));
            sampler += 1;
            if sampler % 2 == 0 || l.starts_with("FILLEND") {
                ev(format!("C2 t={}ms {l}: cg memory.current={} MiB file={} MiB file_dirty={} MiB file_writeback={} MiB anon={} MiB",
                    t0.elapsed().as_millis(), read_cg(&cg, "memory.current").trim().parse::<u64>().unwrap_or(0) >> 20, get("file") >> 20, get("file_dirty") >> 20, get("file_writeback") >> 20, get("anon") >> 20));
            }
        }
    });
    let status = v.wait_exit(Duration::from_secs(5));
    ev(format!("C2 RESULT {name} bdev={bdev} direct={direct}: elapsed_ms={} vmm_exit={status:?} peak_cgroup_file_MiB={}\n--memory.events--\n{}--memory.peak-- {}", t0.elapsed().as_millis(), peak_file >> 20, read_cg(&cg, "memory.events"), read_cg(&cg, "memory.peak").trim()));
    sh("c2log", &format!("busybox tail -3 /nlpv/run/{name}/ch.stderr; busybox grep -ci 'oom\\|killed' {}/console.log; busybox dmesg 2>/dev/null | busybox grep -i 'oom\\|Killed process' | busybox tail -3", v.run_dir));
    v.cleanup();
    if bdev { dm_remove("c2vol"); }
}
gt!(c2_file_buffered, |n| c2(n, false, false));
gt!(c2_file_direct, |n| c2(n, false, true));
gt!(c2_bdev_buffered, |n| c2(n, true, false));
gt!(c2_bdev_direct, |n| c2(n, true, true));
