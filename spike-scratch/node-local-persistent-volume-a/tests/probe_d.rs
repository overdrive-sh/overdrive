use nlpv_probe::probes_a::fallocate;
use nlpv_probe::*;
use overdrive_testing::overdrive_test;
use std::collections::BTreeMap;
use std::time::Duration;

fn disk(path: &str, extra: &str) -> String { let mut a = format!("path={path},image_type=raw"); if !extra.is_empty() { a.push(','); a.push_str(extra); } a }

fn median(v: &mut Vec<f64>) -> f64 { v.sort_by(|a, b| a.partial_cmp(b).unwrap()); if v.is_empty() { return 0.0 } let n = v.len(); if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 } }

/// D: one cell = (substrate, backing, host direct). Runs the guest bench 6 times, drops iteration 1 (warm-up).
fn d(name: &str, substrate: &str, backing: &str, direct: bool) {
    outer_setup_for(900);
    insmod_params("/nlpv-modules/brd.ko", "rd_nr=2 rd_size=524288");
    sh("env", "busybox cat /proc/loadavg; busybox nproc; busybox free -m | busybox head -2");
    // substrate devices: s_xfs (hosts the XFS for file backings), s_dm (hosts dm-linear for bdev backing)
    let (s_a, s_b) = match substrate {
        "brd" => ("/dev/ram0".to_string(), "/dev/ram1".to_string()),
        _ => {
            std::fs::create_dir_all("/var/tmp/nlpv").unwrap();
            let (fa, fb) = ("/var/tmp/nlpv/sa.img".to_string(), "/var/tmp/nlpv/sb.img".to_string());
            sparse(&fa, 384 << 20); sparse(&fb, 384 << 20);
            (loop_attach(&fa, false), loop_attach(&fb, false))
        }
    };
    let vol_bytes = 256u64 << 20;
    let path = match backing {
        "xfs_prealloc" | "xfs_sparse" => {
            must(&["mkfs.xfs", "-q", "-f", "-m", "reflink=1", &s_a]);
            std::fs::create_dir_all("/nlpv/dmnt").unwrap(); must(&["mount", "-t", "xfs", "-o", "noatime", &s_a, "/nlpv/dmnt"]);
            let v = "/nlpv/dmnt/vol.img".to_string();
            if backing == "xfs_prealloc" { fallocate(&v, vol_bytes); } else { sparse(&v, vol_bytes); }
            must(&["mkfs.ext4", "-q", "-F", "-E", "lazy_itable_init=0,lazy_journal_init=0", &v]);
            chown(&v, VMM_UID, VMM_UID);
            sh("volstate", &format!("busybox ls -ls {v}; /usr/bin/xfs_io -c 'fiemap -v' {v} | busybox head -4"));
            v
        }
        _ => {
            let sectors = vol_bytes / 512;
            let dev = dm_create("lv0", &format!("0 {sectors} linear {s_b} 0"));
            must(&["mkfs.ext4", "-q", "-F", "-E", "lazy_itable_init=0,lazy_journal_init=0", &dev]);
            dev
        }
    };
    unsafe { libc::sync(); }
    let rootdir = format!("/nlpv/run/{name}-img"); std::fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(name, &ch_binary("stock"), &rootfs);
    spec.memory_mib = 512; spec.vcpus = 2; spec.fsize = 512 << 20;
    spec.disks.push(disk(&path, if direct { "direct=on" } else { "" }));
    spec.cmdline_params = vec!["nlpv.mode=bench".into(), "nlpv.iters=6".into()];
    let mut v = launch(&spec);
    let mut samples: BTreeMap<String, Vec<(u64, BTreeMap<String, f64>)>> = BTreeMap::new();
    v.follow(Duration::from_secs(840), |l, _| {
        if l.starts_with("BENCH ") && !l.starts_with("BENCHREADY") && !l.starts_with("BENCHDONE") {
            let mut kv = BTreeMap::new();
            for t in l[6..].split_whitespace() { if let Some((k, val)) = t.split_once('=') { if let Ok(x) = val.parse::<f64>() { kv.insert(k.to_owned(), x); } else { kv.insert(format!("s:{k}"), 0.0); } } }
            let w = l[6..].split_whitespace().find_map(|t| t.strip_prefix("w=")).unwrap_or("?").to_owned();
            let iter = kv.get("iter").copied().unwrap_or(0.0) as u64;
            ev(format!("raw {l}"));
            samples.entry(w).or_default().push((iter, kv));
        } else if l.starts_with("BENCHREADY") || l.starts_with("mount") { ev(format!("guest: {l}")); }
    });
    let _ = v.wait_exit(Duration::from_secs(10));
    for (w, runs) in &samples {
        let keep: Vec<&BTreeMap<String, f64>> = runs.iter().filter(|(i, _)| *i >= 2).map(|(_, k)| k).collect();
        let mut ms: Vec<f64> = keep.iter().filter_map(|k| k.get("ms").copied()).collect();
        let mut p50: Vec<f64> = keep.iter().filter_map(|k| k.get("p50_us").copied()).collect();
        let mut p99: Vec<f64> = keep.iter().filter_map(|k| k.get("p99_us").copied()).collect();
        let ops = keep.first().and_then(|k| k.get("ops")).copied().unwrap_or(0.0);
        let (mn, mx) = (ms.iter().cloned().fold(f64::MAX, f64::min), ms.iter().cloned().fold(0.0, f64::max));
        ev(format!("BENCHRESULT cell={name} substrate={substrate} backing={backing} direct={direct} w={w} n={} median_ms={:.0} min_ms={mn:.0} max_ms={mx:.0} ops={ops} median_p50_us={:.0} median_p99_us={:.0}",
            ms.len(), median(&mut ms), median(&mut p50), median(&mut p99)));
    }
    v.cleanup();
    if backing == "bdev" { dm_remove("lv0"); }
}
macro_rules! cell { ($t:ident, $s:expr, $b:expr, $d:expr) => {
    #[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
    #[test]
    fn $t() { d(stringify!($t), $s, $b, $d); }
}; }
cell!(d_brd_xfs_prealloc_buf, "brd", "xfs_prealloc", false);
cell!(d_brd_xfs_prealloc_dir, "brd", "xfs_prealloc", true);
cell!(d_brd_xfs_sparse_buf, "brd", "xfs_sparse", false);
cell!(d_brd_xfs_sparse_dir, "brd", "xfs_sparse", true);
cell!(d_brd_bdev_buf, "brd", "bdev", false);
cell!(d_brd_bdev_dir, "brd", "bdev", true);
cell!(d_disk_xfs_prealloc_buf, "disk", "xfs_prealloc", false);
cell!(d_disk_xfs_prealloc_dir, "disk", "xfs_prealloc", true);
cell!(d_disk_xfs_sparse_buf, "disk", "xfs_sparse", false);
cell!(d_disk_xfs_sparse_dir, "disk", "xfs_sparse", true);
cell!(d_disk_bdev_buf, "disk", "bdev", false);
cell!(d_disk_bdev_dir, "disk", "bdev", true);
