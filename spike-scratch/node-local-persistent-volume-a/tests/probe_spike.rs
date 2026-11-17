//! Re-runs of the A3/B/E3 probes against the fixed (spike-branch) Cloud Hypervisor binary.
//! Throwaway probe code. The stock-binary originals in probe_a/b/e are the controls.
use nlpv_probe::probes_a::*;
use nlpv_probe::probes_b::*;
use nlpv_probe::*;
use overdrive_testing::overdrive_test;
use std::time::Duration;

macro_rules! gt { ($name:ident, $body:expr) => {
    #[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
    #[test]
    fn $name() { outer_setup(); let f: &dyn Fn(&str) = &$body; f(stringify!($name)); }
}; }

// ------------------------------------------------------------------ F1 (A3, A1 regression)
macro_rules! fault {
    ($test:ident, $wt:expr, $nu:expr, $na:expr, $sync:expr) => {
        #[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
        #[test]
        fn $test() {
            outer_setup();
            let r = run_fault_cell(stringify!($test), "spike", $wt, $nu, $na, false, $sync, 10, 30);
            ev(format!("detail: {:?}", r.detail));
        }
    };
}
fault!(spike_a3_fault_wb_explicit_flush_uring, false, false, false, "fdatasync");
fault!(spike_a3_fault_wb_explicit_flush_sync, false, true, true, "fdatasync");
fault!(spike_a3_fault_wt_post_write_fsync_uring, true, false, false, "none");
fault!(spike_a3_fault_wt_post_write_fsync_aio, true, true, false, "none");
fault!(spike_a3_fault_wt_post_write_fsync_sync, true, true, true, "none");

fn base(name: &'static str) -> ACell {
    ACell { name, ch: "spike", host_direct: false, guest_cache_wt: false, no_uring: false, no_aio: false,
            guest_odirect: false, sync: "fdatasync", n: 60, pace_ms: 30, kill_after_acks: None }
}
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[test]
fn spike_a1_wb_buffered_uring() {
    outer_setup();
    let c = base("spike_a1_wb_buffered_uring");
    let r = run_block_cell(&c);
    assert!(r.check.violations.is_empty(), "durability violations: {:?}", r.check.violations);
    assert!(r.check.acked >= 1);
}
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[test]
fn spike_a1_wt_buffered_uring() {
    outer_setup();
    let mut c = base("spike_a1_wt_buffered_uring");
    c.guest_cache_wt = true; c.guest_odirect = true; c.sync = "none";
    let r = run_block_cell(&c);
    assert!(r.check.violations.is_empty(), "durability violations: {:?}", r.check.violations);
    assert!(r.check.acked >= 1);
}

// ------------------------------------------------------------------ F2 (B)
/// B1 on the spike binary: first and second VMM both the fork.
fn b1(name: &str, bdev: bool, extra: &str) {
    let vol = make_volume(name, bdev, 64);
    let disk = disk_a(&vol.path, extra);
    let mut first = idle_vmm(&format!("{name}-first"), "spike", &[disk.clone()]);
    assert!(wait_for_line(&mut first, "READY", Duration::from_secs(30)), "first VMM boots: {}", tail_ch(&first));
    let (ready, detail) = second_vmm_ch(&format!("{name}-second"), "spike", &disk, 12);
    ev(format!("B1 RESULT {name} ch=spike bdev={bdev} extra={extra:?} second_booted={ready} :: {detail}"));
    ev(format!("first VMM still running: {}", first.exited().is_none()));
    first.cleanup();
    assert!(!ready, "second VMM booted on a disk the first holds");
}
gt!(spike_b1_file_default_lock, |n| b1(n, false, ""));
gt!(spike_b1_bdev_default_lock, |n| b1(n, true, ""));
gt!(spike_b1_bdev_full_lock, |n| b1(n, true, "lock_granularity=full"));

gt!(spike_b3_alias_second_vmm, |n| {
    let vol = make_volume(&format!("{n}b"), true, 64);
    let alias = "/dev/mapper/alias-of-vol"; mknod_blk(alias, &vol.path);
    let mut first = idle_vmm(&format!("{n}-1"), "spike", &[disk_a(&vol.path, "")]);
    assert!(wait_for_line(&mut first, "READY", Duration::from_secs(30)));
    let (ready, detail) = second_vmm_ch(&format!("{n}-2"), "spike", &disk_a(alias, ""), 12);
    ev(format!("B3 ALIAS ch=spike bdev second node {alias} same major:minor: second_booted={ready} :: {detail}"));
    ev(format!("first VMM alive after the second's attempt: {}", first.exited().is_none()));
    first.cleanup();
});

/// B2 on the spike binary: replacement after release must still boot.
fn b2_exit(name: &str, bdev: bool, kill: bool) {
    let vol = make_volume(name, bdev, 64);
    let mut v = {
        let rootdir = format!("/nlpv/run/{name}-img"); std::fs::create_dir_all(&rootdir).unwrap();
        let rootfs = build_inner_rootfs(&rootdir, 24);
        let mut spec = VmmSpec::new(name, &ch_binary("spike"), &rootfs);
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
    let (ready, detail) = second_vmm_ch(&format!("{name}-repl"), "spike", &disk_a(&vol.path, ""), 20);
    ev(format!("B2 RESULT {name} ch=spike bdev={bdev} kill={kill} replacement_booted={ready} :: {detail}"));
    v.cleanup();
    assert!(ready, "replacement did not boot after release");
}
gt!(spike_b2_clean_exit_file, |n| b2_exit(n, false, false));
gt!(spike_b2_clean_exit_bdev, |n| b2_exit(n, true, false));
gt!(spike_b2_sigkill_file, |n| b2_exit(n, false, true));
gt!(spike_b2_sigkill_bdev, |n| b2_exit(n, true, true));

/// B2b on the spike binary: SIGKILL with a write in flight under dm-delay; a replacement started
/// immediately (inside the window), then again once the delayed write has drained.
fn b2_inflight(name: &str, uring: bool) {
    let f = format!("/nlpv/{name}.back"); sparse(&f, 64 << 20);
    let l = loop_attach(&f, false);
    let sectors = blk_sectors(&l);
    let delay_ms = 3000;
    let dev = dm_create("slow0", &format!("0 {sectors} delay {l} 0 {delay_ms}"));
    let rootdir = format!("/nlpv/run/{name}-img"); std::fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(name, &ch_binary("spike"), &rootfs);
    spec.disks.push(disk_a(&dev, &format!("direct=on{}", if uring { "" } else { ",_disable_io_uring=on,_disable_aio=on" })));
    spec.cmdline_params = vec!["nlpv.mode=ackraw".into(), "nlpv.n=1".into(), "nlpv.pre=1".into(), "nlpv.odirect=1".into(), "nlpv.sync=none".into(), "nlpv.nonce=77".into()];
    let mut v = launch(&spec);
    assert!(wait_for_line(&mut v, "PRE 1", Duration::from_secs(30)));
    std::thread::sleep(Duration::from_millis(600));
    let t_kill = now_us();
    v.kill9();
    ev(format!("SIGKILL; write is stuck in dm-delay ({delay_ms} ms)"));
    // Replacement attempt 1: inside the window.
    let (ready1, detail1) = second_vmm_ch(&format!("{name}-repl1"), "spike", &disk_a(&dev, ""), 6);
    let t1 = now_us() - t_kill;
    ev(format!("B2b RESULT {name} ch=spike uring={uring}: replacement #1 (inside the window, attempt ended {t1}us after kill) booted={ready1} :: {detail1}"));
    ev(format!("B2b lock after attempt 1: {:?}; process exited={:?}", ofd_getlk(&dev), v.exited()));
    // wait out the delayed write
    let t0 = now_us();
    while ofd_getlk(&dev).is_some() && now_us() - t0 < 8_000_000 { std::thread::sleep(Duration::from_millis(20)); }
    ev(format!("B2b lock released {}us after kill", now_us() - t_kill));
    let (ready2, detail2) = second_vmm_ch(&format!("{name}-repl2"), "spike", &disk_a(&dev, ""), 20);
    ev(format!("B2b RESULT {name} ch=spike uring={uring}: replacement #2 (after the drain) booted={ready2} :: {detail2}"));
    v.cleanup();
    dm_remove("slow0");
}
gt!(spike_b2_inflight_kill_uring, |n| b2_inflight(n, true));
gt!(spike_b2_inflight_kill_sync, |n| b2_inflight(n, false));

// ------------------------------------------------------------------ F3 (E3)
fn disk_typed(path: &str) -> String { format!("path={path},image_type=raw") }
fn disk_untyped(path: &str) -> String { format!("path={path}") }

/// Boot with `mode`, return (reached_guest_line, exit, stderr, the guest line).
fn boot_mode(name: &str, ch: &str, disk: &str, mode: &str, kind: &str, wait: &str) -> (bool, String) {
    let rootdir = format!("/nlpv/run/{name}-img"); std::fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(name, &ch_binary(ch), &rootfs);
    spec.disks.push(disk.to_string());
    spec.cmdline_params = vec![format!("nlpv.mode={mode}"), format!("nlpv.kind={kind}")];
    let mut v = launch(&spec);
    let mut line = String::new();
    v.follow_until(Duration::from_secs(15), |l, _| { if l.starts_with(wait) { line = l.to_owned(); } !line.is_empty() });
    let r = (!line.is_empty(), format!("guest_line={line:?}; exited={:?}; stderr=[{}]", v.exited(), tail_ch(&v)));
    v.cleanup();
    r
}

fn e3(name: &str, kind: &str, bdev: bool) {
    let vol = make_volume(name, bdev, 64);
    // boot #1 (explicit raw): the guest writes another format's marker to its own disk.
    let (w, d) = boot_mode(&format!("{name}-b1"), "stock", &disk_typed(&vol.path), "magic", kind, "WROTE");
    ev(format!("E3 boot#1 ({kind}, bdev={bdev}): wrote={w} :: {d}"));
    // boot #2, explicit image_type=raw, on the stock binary (control) and on the fixed binary.
    for ch in ["stock", "spike"] {
        let (ok, d) = boot_mode(&format!("{name}-b2-{ch}"), ch, &disk_typed(&vol.path), "magicread", kind, "MAGICREAD");
        ev(format!("E3 RESULT {name} kind={kind} bdev={bdev} ch={ch} image_type=raw: second boot reached the guest={ok} :: {d}"));
    }
    // boot #2 with NO image_type: detection must stay as stock on both.
    for ch in ["stock", "spike"] {
        let (ok, d) = boot_mode(&format!("{name}-b2u-{ch}"), ch, &disk_untyped(&vol.path), "magicread", kind, "MAGICREAD");
        ev(format!("E3 RESULT {name} kind={kind} bdev={bdev} ch={ch} NO image_type: second boot reached the guest={ok} :: {d}"));
    }
}
gt!(spike_e3_qcow2_magic_file, |n| e3(n, "qcow2", false));
gt!(spike_e3_qcow2_magic_bdev, |n| e3(n, "qcow2", true));
gt!(spike_e3_vhdx_magic_file, |n| e3(n, "vhdx", false));
gt!(spike_e3_vhd_footer_file, |n| e3(n, "vhd", false));
gt!(spike_e3_vhd_footer_bdev, |n| e3(n, "vhd", true));

/// A never-written raw volume with no image_type: stock detection says Raw on both binaries.
gt!(spike_e3_untyped_clean_raw, |n| {
    let vol = make_volume(n, false, 64);
    for ch in ["stock", "spike"] {
        let (ok, d) = boot_mode(&format!("{n}-{ch}"), ch, &disk_untyped(&vol.path), "magicread", "none", "MAGICREAD");
        ev(format!("E3 RESULT {n} ch={ch} NO image_type, clean raw file: boot reached the guest={ok} :: {d}"));
    }
});
