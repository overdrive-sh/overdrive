use nlpv_probe::probes_a::*;
use nlpv_probe::*;
use overdrive_testing::overdrive_test;

fn base(name: &'static str) -> ACell {
    ACell { name, ch: "stock", host_direct: false, guest_cache_wt: false, no_uring: false, no_aio: false,
            guest_odirect: false, sync: "fdatasync", n: 60, pace_ms: 30, kill_after_acks: None }
}

macro_rules! cell {
    ($test:ident, block, $mods:expr) => {
        #[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
        #[test]
        fn $test() {
            outer_setup();
            let mut c = base(stringify!($test));
            let m: &dyn Fn(&mut ACell) = &$mods; m(&mut c);
            let r = run_block_cell(&c);
            assert!(r.check.violations.is_empty(), "durability violations: {:?}", r.check.violations);
            assert!(r.check.acked >= 1, "no ACKs observed");
        }
    };
    ($test:ident, xfs $prealloc:expr, $mods:expr) => {
        #[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
        #[test]
        fn $test() {
            outer_setup();
            let mut c = base(stringify!($test));
            let m: &dyn Fn(&mut ACell) = &$mods; m(&mut c);
            let r = run_xfs_file_cell(&c, $prealloc);
            assert!(r.check.violations.is_empty(), "durability violations: {:?}", r.check.violations);
            assert!(r.check.acked >= 1, "no ACKs observed");
        }
    };
}

// ---- A1: block-device volume (dm-log-writes under the data disk)
cell!(a1_wb_buffered_uring, block, |_c| {});
cell!(a1_wb_direct_uring, block, |c| { c.host_direct = true; });
cell!(a1_wb_buffered_sync, block, |c| { c.no_uring = true; c.no_aio = true; });
cell!(a1_wb_buffered_aio, block, |c| { c.no_uring = true; });
cell!(a1_wb_direct_sync, block, |c| { c.host_direct = true; c.no_uring = true; c.no_aio = true; });
cell!(a1_wt_buffered_uring, block, |c| { c.guest_cache_wt = true; c.guest_odirect = true; c.sync = "none"; });
cell!(a1_wt_buffered_sync, block, |c| { c.guest_cache_wt = true; c.guest_odirect = true; c.sync = "none"; c.no_uring = true; c.no_aio = true; });
cell!(a1_wt_direct_uring, block, |c| { c.guest_cache_wt = true; c.guest_odirect = true; c.sync = "none"; c.host_direct = true; });
cell!(a1_guest_odirect_wb_uring, block, |c| { c.guest_odirect = true; });
cell!(a1_kill9_wb_buffered_uring, block, |c| { c.n = 400; c.pace_ms = 5; c.kill_after_acks = Some(120); });
cell!(a1_kill9_wb_direct_uring, block, |c| { c.n = 400; c.pace_ms = 5; c.kill_after_acks = Some(120); c.host_direct = true; });

// ---- negative control: the check has teeth. ACK without any flush (guest sync=none, write-back).
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[test]
fn a1_negctl_no_flush_must_violate() {
    outer_setup();
    let mut c = base("a1_negctl_no_flush_must_violate");
    c.sync = "none"; c.n = 40;
    let r = run_block_cell(&c);
    assert!(r.check.acked >= 1);
    assert!(!r.check.violations.is_empty(), "negative control FAILED: ACKs without flush produced no violation, so the check has no teeth");
}

// ---- A2: file volume on XFS
cell!(a2_xfs_prealloc_wb_buffered_uring, xfs true, |c| { c.n = 40; });
cell!(a2_xfs_sparse_wb_buffered_uring, xfs false, |c| { c.n = 40; });
cell!(a2_xfs_prealloc_wb_direct_uring, xfs true, |c| { c.n = 40; c.host_direct = true; });
cell!(a2_xfs_sparse_wt_buffered_uring, xfs false, |c| { c.n = 40; c.guest_cache_wt = true; c.guest_odirect = true; c.sync = "none"; });
cell!(a2_xfs_sparse_kill9_wb_buffered, xfs false, |c| { c.n = 300; c.pace_ms = 5; c.kill_after_acks = Some(100); });

// ---- A3: fsync-error propagation (dm-error injected after the data write)
macro_rules! fault {
    ($test:ident, $wt:expr, $nu:expr, $na:expr, $dir:expr, $sync:expr, $ch:expr) => {
        #[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
        #[test]
        fn $test() {
            outer_setup();
            let r = run_fault_cell(stringify!($test), $ch, $wt, $nu, $na, $dir, $sync, 10, 30);
            ev(format!("detail: {:?}", r.detail));
        }
    };
}
fault!(a3_fault_wb_explicit_flush_uring, false, false, false, false, "fdatasync", "stock");
fault!(a3_fault_wb_explicit_flush_sync, false, true, true, false, "fdatasync", "stock");
fault!(a3_fault_wt_post_write_fsync_uring, true, false, false, false, "none", "stock");
fault!(a3_fault_wt_post_write_fsync_aio, true, true, false, false, "none", "stock");
fault!(a3_fault_wt_post_write_fsync_sync, true, true, true, false, "none", "stock");

// ---- diagnostic: does CH fsync after each write when the guest selects write-through?
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[test]
fn a1_diag_wt_sync_strace() {
    outer_setup();
    let name = "a1_diag_wt_sync_strace";
    let data = format!("/nlpv/{name}.data"); sparse(&data, 64 << 20);
    let ld = loop_attach(&data, false);
    let sectors = blk_sectors(&ld);
    let dev = dm_create("data0", &format!("0 {sectors} linear {ld} 0"));
    let rootdir = format!("/nlpv/run/{name}-img"); std::fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(name, &ch_binary("stock"), &rootfs);
    spec.disks.push(format!("path={dev},image_type=raw,_disable_io_uring=on,_disable_aio=on"));
    spec.ch_extra = vec!["-v".into(), "--log-file".into(), format!("/nlpv/run/{name}/ch.log")];
    spec.strace = Some("fsync,fdatasync,pwritev,pwrite64".into());
    spec.cmdline_params = vec!["nlpv.mode=ackraw".into(), "nlpv.n=10".into(), "nlpv.pace_ms=50".into(), "nlpv.sync=none".into(), "nlpv.cache=wt".into(), "nlpv.nonce=5".into()];
    let mut vmm = launch(&spec);
    vmm.follow(std::time::Duration::from_secs(60), |_l, _v| {});
    let _ = vmm.wait_exit(std::time::Duration::from_secs(10));
    sh("chlog", &format!("busybox grep -i 'cache mode\\|writeback\\|writethrough' /nlpv/run/{name}/ch.log | busybox head; echo ---; busybox wc -l /nlpv/run/{name}/strace.out; busybox grep -c fsync /nlpv/run/{name}/strace.out; busybox grep -E 'pwrite|fsync' /nlpv/run/{name}/strace.out | busybox head -30"));
    vmm.cleanup();
    dm_remove("data0");
}
