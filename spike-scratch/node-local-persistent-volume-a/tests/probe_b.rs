use nlpv_probe::probes_b::*;
use nlpv_probe::*;
use overdrive_testing::overdrive_test;
use std::time::Duration;

macro_rules! gt { ($name:ident, $body:expr) => {
    #[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
    #[test]
    fn $name() { outer_setup(); let f: &dyn Fn(&str) = &$body; f(stringify!($name)); }
}; }

/// B1: a second VMM on the same disk while the first runs.
fn b1(name: &str, bdev: bool, extra: &str) {
    let vol = make_volume(name, bdev, 64);
    let disk = disk_a(&vol.path, extra);
    let mut first = idle_vmm(&format!("{name}-first"), "stock", &[disk.clone()]);
    assert!(wait_for_line(&mut first, "READY", Duration::from_secs(30)), "first VMM boots: {}", tail_ch(&first));
    ev(format!("first VMM pid {} holds lock: {:?}", first.pid(), ofd_getlk(&vol.path)));
    let (ready, detail) = second_vmm(&format!("{name}-second"), &disk, 12);
    ev(format!("B1 RESULT {name} bdev={bdev} extra={extra:?} second_booted={ready} :: {detail}"));
    // does the first keep running?
    ev(format!("first VMM still running: {}", first.exited().is_none()));
    first.cleanup();
    assert!(!ready, "second VMM booted on a disk the first holds: single-writer NOT enforced");
}
gt!(b1_file_default_lock, |n| b1(n, false, ""));
gt!(b1_file_full_lock, |n| b1(n, false, "lock_granularity=full"));
gt!(b1_bdev_default_lock, |n| b1(n, true, ""));
gt!(b1_bdev_full_lock, |n| b1(n, true, "lock_granularity=full"));

/// B1b: the lock is taken at boot, not at open: a second VMM that is merely *created* (not booted) and
/// a plain `open()` do not conflict. Evidence: lock state before the first boot vs after.
gt!(b1b_lock_is_taken_at_boot, |n| {
    let vol = make_volume(n, false, 64);
    ev(format!("before any VMM: lock={:?}", ofd_getlk(&vol.path)));
    let mut v = idle_vmm(&format!("{n}-v"), "stock", &[disk_a(&vol.path, "")]);
    wait_for_line(&mut v, "READY", Duration::from_secs(30));
    ev(format!("while first runs: lock={:?}", ofd_getlk(&vol.path)));
    v.cleanup();
    ev(format!("after kill: lock={:?}", ofd_getlk(&vol.path)));
});

/// B2: when is the lock released? Clean exit (guest reset ends the VMM) and SIGKILL.
fn b2_exit(name: &str, bdev: bool, kill: bool) {
    let vol = make_volume(name, bdev, 64);
    let mut v = {
        let rootdir = format!("/nlpv/run/{name}-img"); std::fs::create_dir_all(&rootdir).unwrap();
        let rootfs = build_inner_rootfs(&rootdir, 24);
        let mut spec = VmmSpec::new(name, &ch_binary("stock"), &rootfs);
        spec.disks.push(disk_a(&vol.path, ""));
        spec.cmdline_params = vec!["nlpv.mode=ackraw".into(), "nlpv.n=20".into(), "nlpv.pace_ms=100".into(), "nlpv.nonce=1".into()];
        launch(&spec)
    };
    assert!(wait_for_line(&mut v, "READY", Duration::from_secs(30)));
    ev(format!("lock held: {:?}", ofd_getlk(&vol.path)));
    if kill { std::thread::sleep(Duration::from_millis(500)); ev("SIGKILL now"); v.kill9(); } else {
        let mut seen = false;
        v.follow_until(Duration::from_secs(30), |l, _| { if l == "DONE" { seen = true; } seen });
        ev("guest printed DONE (reset follows)");
    }
    lock_timeline(name, &vol.path, &mut v, Duration::from_secs(10));
    // immediately start a replacement: does it boot?
    let (ready, detail) = second_vmm(&format!("{name}-repl"), &disk_a(&vol.path, ""), 20);
    ev(format!("B2 RESULT {name} bdev={bdev} kill={kill} replacement_booted={ready} :: {detail}"));
    v.cleanup();
}
gt!(b2_clean_exit_file, |n| b2_exit(n, false, false));
gt!(b2_clean_exit_bdev, |n| b2_exit(n, true, false));
gt!(b2_sigkill_file, |n| b2_exit(n, false, true));
gt!(b2_sigkill_bdev, |n| b2_exit(n, true, true));

/// B2b: SIGKILL with a host write still in flight (dm-delay under the disk, host direct=on).
/// Does the lock outlive the in-flight write, or can a replacement take it while the dead VMM's write is pending?
fn b2_inflight(name: &str, uring: bool) {
    let f = format!("/nlpv/{name}.back"); sparse(&f, 64 << 20);
    let l = loop_attach(&f, false);
    let sectors = blk_sectors(&l);
    let delay_ms = 3000;
    let dev = dm_create("slow0", &format!("0 {sectors} delay {l} 0 {delay_ms}"));
    let rootdir = format!("/nlpv/run/{name}-img"); std::fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(name, &ch_binary("stock"), &rootfs);
    spec.disks.push(disk_a(&dev, &format!("direct=on{}", if uring { "" } else { ",_disable_io_uring=on,_disable_aio=on" })));
    spec.cmdline_params = vec!["nlpv.mode=ackraw".into(), "nlpv.n=1".into(), "nlpv.pre=1".into(), "nlpv.odirect=1".into(), "nlpv.sync=none".into(), "nlpv.nonce=77".into()];
    let mut v = launch(&spec);
    assert!(wait_for_line(&mut v, "PRE 1", Duration::from_secs(30)));
    let t_pre = now_us();
    std::thread::sleep(Duration::from_millis(600));
    let t_kill = now_us();
    v.kill9();
    ev(format!("SIGKILL {}us after PRE 1; write is stuck in dm-delay ({delay_ms} ms)", t_kill - t_pre));
    let t0 = now_us();
    let mut t_unlock = None; let mut t_exit = None;
    while now_us() - t0 < 8_000_000 {
        if t_exit.is_none() && v.exited().is_some() { t_exit = Some(now_us() - t0); }
        if t_unlock.is_none() && ofd_getlk(&dev).is_none() { t_unlock = Some(now_us() - t0); }
        if t_unlock.is_some() && t_exit.is_some() { break; }
        std::thread::sleep(Duration::from_millis(2));
    }
    ev(format!("B2b RESULT {name} uring={uring} delay_ms={delay_ms} process_exit_after_kill_us={t_exit:?} lock_released_after_kill_us={t_unlock:?}  (kill came {}us after the write began)", t_kill - t_pre));
    sh("dmstat", &format!("{DMSETUP} status slow0; {DMSETUP} info slow0 | busybox grep -i open"));
    std::thread::sleep(Duration::from_millis(3500));
    let data = std::fs::read(&f).unwrap();
    let landed = data[4096..4096 + 8] == *b"NLPVBLK1";
    ev(format!("B2b the dead VMM's in-flight write landed on the device after the kill: {landed}"));
    v.cleanup();
    dm_remove("slow0");
}
gt!(b2_inflight_kill_uring, |n| b2_inflight(n, true));
gt!(b2_inflight_kill_sync, |n| b2_inflight(n, false));

/// B3: ways around the lock.
gt!(b3_plain_writer_ignores_lock, |n| {
    for bdev in [false, true] {
        let vol = make_volume(&format!("{n}{}", bdev as u8), bdev, 64);
        let mut v = idle_vmm(&format!("{n}-{}", bdev as u8), "stock", &[disk_a(&vol.path, "")]);
        assert!(wait_for_line(&mut v, "READY", Duration::from_secs(30)));
        let lock = ofd_getlk(&vol.path);
        use std::os::unix::fs::FileExt;
        let f = std::fs::OpenOptions::new().read(true).write(true).open(&vol.path);
        let w = f.as_ref().map(|f| f.write_at(b"INTRUDER", 8192));
        ev(format!("B3 plain_writer bdev={bdev}: lock_while_vmm_runs={lock:?} open_rw={:?} write={w:?}", f.as_ref().map(|_| "ok")));
        v.cleanup();
    }
});
gt!(b3_alias_second_vmm, |n| {
    // block device: a second device node for the same device; file: hardlink and symlink and bind-mount path
    let vol = make_volume(&format!("{n}b"), true, 64);
    let alias = "/dev/mapper/alias-of-vol"; mknod_blk(alias, &vol.path);
    let mut first = idle_vmm(&format!("{n}-1"), "stock", &[disk_a(&vol.path, "")]);
    assert!(wait_for_line(&mut first, "READY", Duration::from_secs(30)));
    let (ready, detail) = second_vmm(&format!("{n}-2"), &disk_a(alias, ""), 12);
    ev(format!("B3 ALIAS bdev second node {alias} same major:minor: second_booted={ready} :: {detail}"));
    ev(format!("both VMMs alive: first={} ", first.exited().is_none()));
    first.cleanup();
    let vf = make_volume(&format!("{n}f"), false, 64);
    let hard = format!("{}.hard", vf.path); let sym = format!("{}.sym", vf.path);
    let _ = std::fs::hard_link(&vf.path, &hard); let _ = std::os::unix::fs::symlink(&vf.path, &sym);
    let mut first = idle_vmm(&format!("{n}-3"), "stock", &[disk_a(&vf.path, "")]);
    assert!(wait_for_line(&mut first, "READY", Duration::from_secs(30)));
    for (label, p) in [("hardlink", hard.as_str()), ("symlink", sym.as_str())] {
        let (ready, detail) = second_vmm(&format!("{n}-{label}"), &disk_a(p, ""), 12);
        ev(format!("B3 ALIAS file {label}: second_booted={ready} :: {detail}"));
    }
    first.cleanup();
});
/// Kernel semantics of the claim CH never takes (probe-level, no CH): O_EXCL on a block device.
gt!(b3_kernel_o_excl_semantics, |n| {
    use std::os::unix::fs::OpenOptionsExt;
    let vol = make_volume(n, true, 64);
    let alias = "/dev/mapper/alias-excl"; mknod_blk(alias, &vol.path);
    let a = std::fs::OpenOptions::new().read(true).write(true).custom_flags(libc::O_EXCL).open(&vol.path);
    let b = std::fs::OpenOptions::new().read(true).write(true).custom_flags(libc::O_EXCL).open(alias);
    let c = std::fs::OpenOptions::new().read(true).write(true).open(alias);
    ev(format!("B3 O_EXCL: first O_EXCL open {:?}; second O_EXCL open via ALIAS node {:?}; plain (no O_EXCL) rw open via alias {:?}",
        a.as_ref().map(|_| "ok").map_err(|e| e.to_string()), b.as_ref().map(|_| "ok").map_err(|e| e.to_string()), c.as_ref().map(|_| "ok").map_err(|e| e.to_string())));
    drop(a);
    let d = std::fs::OpenOptions::new().read(true).write(true).custom_flags(libc::O_EXCL).open(alias);
    ev(format!("B3 O_EXCL after first closed: {:?}", d.as_ref().map(|_| "ok").map_err(|e| e.to_string())));
    // dm refuses removal while open
    let r = run(&[DMSETUP, "remove", vol.dm.as_deref().unwrap(), "--noudevsync", "--noudevrules"]);
    ev(format!("B3 dmsetup remove while an fd is open: exit {}", r.0));
});

/// B4: does hot-remove release the lock?
gt!(b4_hot_remove_releases_lock, |n| {
    for bdev in [false, true] {
        let vol = make_volume(&format!("{n}{}", bdev as u8), bdev, 64);
        let mut v = idle_vmm(&format!("{n}-{}", bdev as u8), "stock", &[disk_a(&vol.path, "id=datavol")]);
        assert!(wait_for_line(&mut v, "READY", Duration::from_secs(30)));
        ev(format!("bdev={bdev}: before remove: lock={:?} fds_on_disk={}", ofd_getlk(&vol.path), fds_on(v.pid(), &vol.path)));
        let r = v.api_put("vm.remove-device", "{\"id\":\"datavol\"}");
        ev(format!("remove-device response: {}", r.replace("\r\n", " | ")));
        std::thread::sleep(Duration::from_millis(500));
        ev(format!("bdev={bdev}: after remove-device: lock={:?} fds_on_disk={} vm_info_disks={}", ofd_getlk(&vol.path), fds_on(v.pid(), &vol.path),
            v.api_get("vm.info").matches("datavol").count()));
        let (ready, detail) = second_vmm(&format!("{n}-2-{}", bdev as u8), &disk_a(&vol.path, ""), 15);
        ev(format!("B4 RESULT bdev={bdev}: replacement booted while first VMM still alive after remove-device: {ready} :: {detail}"));
        v.cleanup();
    }
});

/// B2c: a replacement started the instant the predecessor's guest ends: does it race the lock release?
gt!(b2c_replacement_races_exit, |n| {
    let mut outcomes = vec![];
    for round in 0..6 {
        let vol = make_volume(&format!("{n}{round}"), round % 2 == 1, 64);
        let rootdir = format!("/nlpv/run/{n}{round}-img"); std::fs::create_dir_all(&rootdir).unwrap();
        let rootfs = build_inner_rootfs(&rootdir, 24);
        let mut spec = VmmSpec::new(&format!("{n}{round}"), &ch_binary("stock"), &rootfs);
        spec.disks.push(disk_a(&vol.path, ""));
        spec.cmdline_params = vec!["nlpv.mode=ackraw".into(), "nlpv.n=3".into(), "nlpv.pace_ms=50".into(), "nlpv.nonce=1".into()];
        let mut first = launch(&spec);
        let mut second: Option<Vmm> = None;
        first.follow_until(Duration::from_secs(30), |l, _| {
            if l == "DONE" {
                second = Some(idle_vmm(&format!("{n}{round}-repl"), "stock", &[disk_a(&vol.path, "")]));
                return true;
            }
            false
        });
        let mut second = second.expect("DONE seen");
        let ready = wait_for_line(&mut second, "READY", Duration::from_secs(15));
        let first_exit = first.wait_exit(Duration::from_secs(5));
        let err = tail_ch(&second);
        ev(format!("B2c round {round} bdev={}: replacement launched at first's DONE -> booted={ready}; first_exit={first_exit:?}; second stderr=[{err}]", round % 2 == 1));
        outcomes.push(ready);
        first.cleanup(); second.cleanup();
    }
    ev(format!("B2c RESULT replacement booted in {}/{} rounds (the rest failed AlreadyLocked while the predecessor was still exiting)", outcomes.iter().filter(|x| **x).count(), outcomes.len()));
});
