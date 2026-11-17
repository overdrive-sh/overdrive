use nlpv_probe::*;
use overdrive_testing::overdrive_test;
use std::time::Duration;

macro_rules! gt { ($name:ident, $body:expr) => {
    #[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
    #[test]
    fn $name() { outer_setup(); let f: &dyn Fn(&str) = &$body; f(stringify!($name)); }
}; }
fn disk(path: &str, extra: &str) -> String { let mut a = format!("path={path},image_type=raw"); if !extra.is_empty() { a.push(','); a.push_str(extra); } a }

/// E1: ENOSPC on a sparse file whose filesystem fills while the guest writes.
fn e1(name: &str, direct: bool, no_uring: bool, no_aio: bool) {
    let m = make_xfs(name, 320, true);
    let vol = format!("{m}/vol.img");
    sparse(&vol, 512 << 20); chown(&vol, VMM_UID, VMM_UID);
    let rootdir = format!("/nlpv/run/{name}-img"); std::fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(name, &ch_binary("stock"), &rootfs);
    spec.fsize = 1 << 40; spec.memory_mib = 128;
    let mut extra = vec![]; if direct { extra.push("direct=on"); } if no_uring { extra.push("_disable_io_uring=on"); } if no_aio { extra.push("_disable_aio=on"); }
    spec.disks.push(disk(&vol, &extra.join(",")));
    spec.cmdline_params = vec!["nlpv.mode=fill".into(), "nlpv.bs=1048576".into(), "nlpv.odirect=1".into(), "nlpv.sync_every=8".into()];
    spec.ch_extra = vec!["-v".into(), "--log-file".into(), format!("/nlpv/run/{name}/ch.log")];
    let mut v = launch(&spec);
    ev(format!("E1 {name}: sparse 512MiB volume file on a 320MiB XFS; host_direct={direct}"));
    let mut reports = vec![];
    v.follow(Duration::from_secs(90), |l, _| { if l.starts_with("FIRSTERR") || l.starts_with("SYNCERR") || l.starts_with("FILLEND") || l.starts_with("READBACK") { reports.push(l.to_owned()); ev(format!("guest: {l}")); } });
    let status = v.wait_exit(Duration::from_secs(10));
    let alive_after = v.exited().is_none();
    ev(format!("E1 RESULT {name} direct={direct} no_uring={no_uring} no_aio={no_aio}: guest_reports={reports:?} vmm_exit={status:?} vmm_alive_after_guest_done={alive_after}"));
    sh("e1", &format!("busybox grep -E 'I/O error|blk_update_request|Buffer I/O' {}/console.log | busybox head -4; busybox grep -ci 'Request failed' /nlpv/run/{name}/ch.log; busybox grep -i 'Request failed' /nlpv/run/{name}/ch.log | busybox head -2 | busybox cut -c1-300; busybox df -k {m} | busybox tail -1", v.run_dir));
    v.cleanup();
}
gt!(e1_enospc_buffered_uring, |n| e1(n, false, false, false));
gt!(e1_enospc_direct_uring, |n| e1(n, true, false, false));
gt!(e1_enospc_buffered_sync, |n| e1(n, false, true, true));
gt!(e1b_prealloc_fails_up_front, |n| {
    let m = make_xfs(n, 320, true);
    let vol = format!("{m}/vol.img");
    let r = std::panic::catch_unwind(|| nlpv_probe::probes_a::fallocate(&format!("{}/vol.img", "/nlpv/xfs-e1b_prealloc_fails_up_front"), 512 << 20));
    ev(format!("E1b fallocate(512MiB) on a 320MiB XFS: {}", if r.is_err() { "FAILED up front (see panic text above: ENOSPC)" } else { "succeeded" }));
    let _ = vol;
});

/// E2: guest discard. file preallocated on XFS vs block device; sparse=on (default) vs sparse=off.
fn e2(name: &str, bdev: bool, sparse_opt: &str) {
    let (path, back) = if bdev {
        let f = format!("/nlpv/{name}.back"); sparse(&f, 64 << 20);
        let l = loop_attach(&f, false); let sectors = blk_sectors(&l);
        (dm_create("e2vol", &format!("0 {sectors} linear {l} 0")), f)
    } else {
        let m = make_xfs(name, 320, true);
        let vol = format!("{m}/vol.img"); nlpv_probe::probes_a::fallocate(&vol, 64 << 20); chown(&vol, VMM_UID, VMM_UID);
        (vol.clone(), vol)
    };
    // dirty the backing so it is allocated for the bdev case (loop file); preallocated for the file case.
    if bdev { use std::io::Write; let mut f = std::fs::OpenOptions::new().write(true).open(&back).unwrap(); f.write_all(&vec![1u8; 64 << 20]).unwrap(); f.sync_all().unwrap(); }
    let before = st_blocks_bytes(&back);
    let rootdir = format!("/nlpv/run/{name}-img"); std::fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(name, &ch_binary("stock"), &rootfs);
    spec.fsize = 1 << 40;
    spec.disks.push(disk(&path, sparse_opt));
    spec.cmdline_params = vec!["nlpv.mode=discard".into(), "nlpv.blocks=16".into()];
    let mut v = launch(&spec);
    let mut rep = vec![];
    v.follow(Duration::from_secs(60), |l, _| { if l.starts_with("QUEUE") || l.starts_with("BLKDISCARD") || l.starts_with("READBACK") { rep.push(l.to_owned()); } });
    let _ = v.wait_exit(Duration::from_secs(5));
    unsafe { libc::sync(); }
    let after = st_blocks_bytes(&back);
    ev(format!("E2 RESULT {name} bdev={bdev} sparse={sparse_opt:?}: host allocated bytes of the backing before={before} after={after} (delta {}); guest: {rep:?}", before as i64 - after as i64));
    v.cleanup();
    if bdev { dm_remove("e2vol"); }
}
gt!(e2_discard_file_sparse_default, |n| e2(n, false, ""));
gt!(e2_discard_file_sparse_off, |n| e2(n, false, "sparse=off"));
gt!(e2_discard_bdev_sparse_default, |n| e2(n, true, ""));

/// E3: guest writes another format's magic to its own data disk; does the NEXT boot fail?
fn e3(name: &str, kind: &str, bdev: bool) {
    let vol = make_volume(name, bdev, 64);
    let rootdir = format!("/nlpv/run/{name}-img"); std::fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(name, &ch_binary("stock"), &rootfs);
    spec.disks.push(disk(&vol.path, ""));
    spec.cmdline_params = vec!["nlpv.mode=magic".into(), format!("nlpv.kind={kind}")];
    let mut v = launch(&spec);
    let mut rep = vec![];
    v.follow(Duration::from_secs(60), |l, _| { if l.starts_with("READY") || l.starts_with("WROTE") { rep.push(l.to_owned()); } });
    let _ = v.wait_exit(Duration::from_secs(5));
    ev(format!("E3 boot#1 ({kind}, bdev={bdev}): guest {rep:?}; vmm exit={:?}", v.exited()));
    v.cleanup();
    // boot #2 on the same disk
    let mut v2 = idle_vmm(&format!("{name}-b2"), "stock", &[disk(&vol.path, "")]);
    let ready = wait_for_line(&mut v2, "READY", Duration::from_secs(15));
    ev(format!("E3 RESULT {name} kind={kind} bdev={bdev}: second boot reached the guest={ready}; exited={:?}; stderr=[{}]", v2.exited(), tail_ch(&v2)));
    v2.cleanup();
}
gt!(e3_qcow2_magic_file, |n| e3(n, "qcow2", false));
gt!(e3_vhdx_magic_file, |n| e3(n, "vhdx", false));
gt!(e3_vhd_footer_file, |n| e3(n, "vhd", false));
gt!(e3_qcow2_magic_bdev, |n| e3(n, "qcow2", true));

/// F: serial derivation. What the guest reads for the data disk with and without an explicit serial.
fn serial_probe(name: &str, disk_args: &str, vol_path: &str) {
    let rootdir = format!("/nlpv/run/{name}-img"); std::fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(name, &ch_binary("stock"), &rootfs);
    spec.disks.push(format!("path={vol_path},image_type=raw{disk_args}"));
    spec.cmdline_params = vec!["nlpv.mode=serial".into()];
    let mut v = launch(&spec);
    let mut s = String::new();
    v.follow(Duration::from_secs(30), |l, _| { if l.starts_with("SERIAL") { s = l.to_owned(); } });
    ev(format!("SERIAL RESULT {name} args={disk_args:?} path={vol_path}: {s}; st_dev/st_ino={:?}", std::fs::metadata(vol_path).map(|m| { use std::os::unix::fs::MetadataExt; (m.dev(), m.rdev(), m.ino()) })));
    v.cleanup();
}
gt!(f_serial_derivation, |n| {
    let v = make_volume(n, false, 64);
    serial_probe(&format!("{n}-explicit"), ",serial=nlpv-vol-0001", &v.path);
    serial_probe(&format!("{n}-default-1"), "", &v.path);
    serial_probe(&format!("{n}-default-2"), "", &v.path);
    // the same logical volume re-created (new inode): default serial changes
    let p2 = format!("{}.re", v.path); std::fs::copy(&v.path, &p2).unwrap(); chown(&p2, VMM_UID, VMM_UID);
    serial_probe(&format!("{n}-default-recreated"), "", &p2);
    let b = make_volume(&format!("{n}b"), true, 64);
    serial_probe(&format!("{n}-bdev-default"), "", &b.path);
    serial_probe(&format!("{n}-bdev-explicit"), ",serial=nlpv-vol-0002", &b.path);
    serial_probe(&format!("{n}-explicit-too-long"), ",serial=0123456789012345678901234567890", &v.path);
});
