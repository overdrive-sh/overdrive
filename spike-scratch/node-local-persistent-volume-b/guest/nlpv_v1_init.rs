//! V-1 inner-guest PID 1 (static musl, std only). Throwaway probe code.
//!
//! Models the minimal appliance node: no udev, no LVM daemons, root filesystem mounted
//! READ-ONLY, tmpfs on /run only (plus the pseudo filesystems /proc /sys /dev). Runs one
//! phase chosen by `nlpv.phase=` on the kernel command line and reports on the serial
//! console. Every console line starts with `NLPV-G `.
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{FileExt, FileTypeExt, MetadataExt, OpenOptionsExt};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

extern "C" {
    fn mount(src: *const i8, tgt: *const i8, fstype: *const i8, flags: u64, data: *const i8) -> i32;
    fn reboot(cmd: i32) -> i32;
    fn sync();
}

const VG: &str = "overdrive";
const LV_TEST: &str = "ovd-test";
const LV_TEST_NODE: &str = "/dev/mapper/overdrive-ovd--test";
const LVM: &str = "/usr/sbin/lvm";
const LABEL: &str = "overdrive-volumes";

fn say(s: &str) {
    let mut out = OpenOptions::new().write(true).open("/dev/console").ok();
    for l in s.lines() {
        let line = format!("NLPV-G {l}\n");
        match out.as_mut() {
            Some(f) => { let _ = f.write_all(line.as_bytes()); }
            None => print!("{line}"),
        }
    }
}

fn do_mount(src: &str, tgt: &str, fs: &str, flags: u64) {
    let _ = std::fs::create_dir_all(tgt);
    let (s, t, f) = (std::ffi::CString::new(src).unwrap(), std::ffi::CString::new(tgt).unwrap(), std::ffi::CString::new(fs).unwrap());
    let r = unsafe { mount(s.as_ptr(), t.as_ptr(), f.as_ptr(), flags, std::ptr::null()) };
    if r != 0 { say(&format!("mount {fs} on {tgt} failed: {}", std::io::Error::last_os_error())); }
}

fn params() -> std::collections::BTreeMap<String, String> {
    let c = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    c.split_whitespace().filter_map(|t| t.strip_prefix("nlpv.").and_then(|t| t.split_once('='))).map(|(k, v)| (k.to_owned(), v.to_owned())).collect()
}

// ------------------------------------------------------------------ rng, hashing, payloads
struct Rng(u64);
impl Rng {
    fn seeded() -> Rng {
        let mut b = [0u8; 8];
        let _ = File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
        Rng(u64::from_le_bytes(b) | 1)
    }
    fn next(&mut self) -> u64 { let mut x = self.0; x ^= x << 13; x ^= x >> 7; x ^= x << 17; self.0 = x; x }
}
fn xorshift(mut x: u64) -> u64 { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x }
fn fill_block(buf: &mut [u8], i: u64, nonce: u64) {
    buf[0..8].copy_from_slice(b"NLPVLV01");
    buf[8..16].copy_from_slice(&i.to_le_bytes());
    buf[16..24].copy_from_slice(&nonce.to_le_bytes());
    let mut s = i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ nonce | 1;
    for chunk in buf[24..].chunks_mut(8) {
        s = xorshift(s);
        let b = s.to_le_bytes();
        let n = chunk.len();
        chunk.copy_from_slice(&b[..n]);
    }
}
fn fnv(data: &[u8]) -> u64 { let mut h = 0xcbf29ce484222325u64; for b in data { h ^= *b as u64; h = h.wrapping_mul(0x100000001b3); } h }

struct Aligned { ptr: *mut u8, len: usize }
impl Aligned {
    fn new(len: usize) -> Self {
        let layout = std::alloc::Layout::from_size_align(len, 4096).unwrap();
        Aligned { ptr: unsafe { std::alloc::alloc_zeroed(layout) }, len }
    }
    fn as_mut(&mut self) -> &mut [u8] { unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) } }
    fn as_ref(&self) -> &[u8] { unsafe { std::slice::from_raw_parts(self.ptr, self.len) } }
}

const MIB: u64 = 1 << 20;
const NONCE: u64 = 0x5eed_1234;
const WRITE_OFFSETS_MIB: [u64; 4] = [0, 4, 32, 63];

fn hash_device(path: &str) -> String {
    match File::open(path) {
        Err(e) => format!("open-failed:{e}"),
        Ok(f) => {
            let mut h = 0xcbf29ce484222325u64;
            let mut buf = vec![0u8; 1 << 20];
            let mut off = 0u64;
            let mut total = 0u64;
            loop {
                match f.read_at(&mut buf, off) {
                    Ok(0) => break,
                    Ok(n) => { for b in &buf[..n] { h ^= *b as u64; h = h.wrapping_mul(0x100000001b3); } off += n as u64; total += n as u64; }
                    Err(e) => return format!("read-failed@{off}:{e}"),
                }
            }
            format!("{h:016x}/{total}B")
        }
    }
}

// ------------------------------------------------------------------ lvm runner
struct Out { code: Option<i32>, sig: Option<i32>, out: String, err: String, ms: f64, node_ms: Option<f64>, timed_out: bool }
impl Out {
    fn ok(&self) -> bool { self.code == Some(0) }
}

/// The pinned `--config`; `filter` is the LVM device filter array body.
fn cfg(filter: &str) -> String {
    format!("activation{{udev_sync=0 udev_rules=0 monitoring=0}} devices{{filter=[{filter}] use_devicesfile=0}} backup{{backup=0 archive=0}} global{{use_lvmpolld=0 locking_dir=\"/run/lock/lvm\"}}")
}
fn filter_for(devs: &[&str]) -> String {
    let mut v: Vec<String> = devs.iter().map(|d| format!("\"a|^{d}$|\"")).collect();
    v.push("\"r|.*|\"".into());
    v.join(",")
}
const PERMISSIVE: &str = "\"a|.*|\"";

static SYSDIR: std::sync::OnceLock<String> = std::sync::OnceLock::new(); // "unset" | "empty" | "run"
struct Lvm { config: String, sysdir: String }
impl Lvm {
    fn new(filter: &str) -> Lvm { Lvm { config: cfg(filter), sysdir: SYSDIR.get().cloned().unwrap_or_else(|| "unset".into()) } }
    fn with_sysdir(filter: &str, sysdir: &str) -> Lvm { Lvm { config: cfg(filter), sysdir: sysdir.into() } }
    fn command(&self, sub: &str, rest: &[&str]) -> Command {
        let mut c = Command::new(LVM);
        c.args(self.args(sub, rest));
        match self.sysdir.as_str() { "empty" => { c.env("LVM_SYSTEM_DIR", ""); } "run" => { c.env("LVM_SYSTEM_DIR", "/run/lvm"); } _ => {} }
        c
    }
    fn args(&self, sub: &str, rest: &[&str]) -> Vec<String> {
        let mut a = vec![sub.to_owned(), "--config".into(), self.config.clone()];
        a.extend(rest.iter().map(|s| s.to_string()));
        a
    }
    /// Run to completion (bounded by `bound`); optionally poll for `node` to appear.
    fn run(&self, sub: &str, rest: &[&str], node: Option<&str>, bound: Duration) -> Out {
        let t0 = Instant::now();
        let mut c = self.command(sub, rest).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn lvm");
        let mut node_ms = None;
        let mut timed_out = false;
        let status = loop {
            if let Some(n) = node { if node_ms.is_none() && std::fs::metadata(n).map(|m| m.file_type().is_block_device()).unwrap_or(false) { node_ms = Some(t0.elapsed().as_secs_f64() * 1e3); } }
            match c.try_wait().unwrap() {
                Some(s) => break s,
                None => {
                    if t0.elapsed() > bound { timed_out = true; let _ = c.kill(); break c.wait().unwrap(); }
                    std::thread::sleep(Duration::from_micros(100));
                }
            }
        };
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        let (mut o, mut e) = (String::new(), String::new());
        if let Some(mut s) = c.stdout.take() { let _ = s.read_to_string(&mut o); }
        if let Some(mut s) = c.stderr.take() { let _ = s.read_to_string(&mut e); }
        if let (Some(n), None) = (node, node_ms) { if std::fs::metadata(n).map(|m| m.file_type().is_block_device()).unwrap_or(false) { node_ms = Some(ms); } }
        Out { code: status.code(), sig: status.signal(), out: o, err: e, ms, node_ms, timed_out }
    }
    fn go(&self, sub: &str, rest: &[&str]) -> Out { self.run(sub, rest, None, Duration::from_secs(30)) }
    fn go_node(&self, sub: &str, rest: &[&str], node: &str) -> Out { self.run(sub, rest, Some(node), Duration::from_secs(30)) }
}

fn show(label: &str, sub: &str, rest: &[&str], o: &Out) {
    say(&format!("CMD [{label}] lvm {sub} {} -> code={:?} sig={:?} timed_out={} ms={:.1}{}", rest.join(" "), o.code, o.sig, o.timed_out, o.ms,
        o.node_ms.map(|n| format!(" node_ms={n:.1}")).unwrap_or_default()));
    if !o.out.trim().is_empty() { say(&format!("  stdout:\n{}", o.out.trim_end())); }
    if !o.err.trim().is_empty() { say(&format!("  stderr:\n{}", o.err.trim_end())); }
}

/// run + show
fn lv(l: &Lvm, label: &str, sub: &str, rest: &[&str]) -> Out { let o = l.go(sub, rest); show(label, sub, rest, &o); o }
fn lv_node(l: &Lvm, label: &str, sub: &str, rest: &[&str], node: &str) -> Out { let o = l.go_node(sub, rest, node); show(label, sub, rest, &o); o }

fn median(v: &mut Vec<f64>) -> f64 { v.sort_by(|a, b| a.partial_cmp(b).unwrap()); if v.is_empty() { 0.0 } else { v[v.len() / 2] } }
fn series(name: &str, v: &[f64]) {
    let mut s = v.to_vec();
    let m = median(&mut s);
    say(&format!("LATENCY {name}: n={} min={:.1}ms median={m:.1}ms max={:.1}ms samples={:?}", v.len(), s.first().copied().unwrap_or(0.0), s.last().copied().unwrap_or(0.0), v.iter().map(|x| (x * 10.0).round() / 10.0).collect::<Vec<_>>()));
}

// ------------------------------------------------------------------ node + dm state helpers
fn is_blk(path: &str) -> bool { std::fs::metadata(path).map(|m| m.file_type().is_block_device()).unwrap_or(false) }
fn rdev_mm(path: &str) -> Option<(u64, u64)> {
    let r = std::fs::metadata(path).ok()?.rdev();
    Some((((r >> 8) & 0xfff) | ((r >> 32) & !0xfff), (r & 0xff) | ((r >> 12) & !0xff)))
}
fn node_report(label: &str, path: &str) {
    match rdev_mm(path) {
        None => say(&format!("NODE [{label}] {path}: ABSENT")),
        Some((ma, mi)) => {
            let rd = |f: &str| std::fs::read_to_string(format!("/sys/dev/block/{ma}:{mi}/{f}")).unwrap_or_default().trim().to_owned();
            say(&format!("NODE [{label}] {path}: is_block_device={} major:minor={ma}:{mi} dm/name={:?} dm/uuid={:?} dm/suspended={:?} size_sectors={:?} ro={:?}", is_blk(path), rd("dm/name"), rd("dm/uuid"), rd("dm/suspended"), rd("size"), rd("ro")));
        }
    }
}
fn dm_devices() -> Vec<String> {
    let mut v = vec![];
    if let Ok(rd) = std::fs::read_dir("/sys/block") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("dm-") {
                let name = std::fs::read_to_string(format!("/sys/block/{n}/dm/name")).unwrap_or_default().trim().to_owned();
                let susp = std::fs::read_to_string(format!("/sys/block/{n}/dm/suspended")).unwrap_or_default().trim().to_owned();
                v.push(format!("{n}={name}(suspended={susp})"));
            }
        }
    }
    v.sort();
    v
}
fn ls(dir: &str) -> String {
    match std::fs::read_dir(dir) {
        Err(e) => format!("<{e}>"),
        Ok(rd) => { let mut v: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect(); v.sort(); v.join(" ") }
    }
}

// ------------------------------------------------------------------ discovery (step 1)
fn uevent(name: &str) -> std::collections::BTreeMap<String, String> {
    std::fs::read_to_string(format!("/sys/class/block/{name}/uevent")).unwrap_or_default().lines()
        .filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_owned(), v.to_owned())).collect()
}
/// Returns the /dev node of the single partition whose PARTNAME is `overdrive-volumes`.
fn discover() -> String {
    let mut names: Vec<String> = std::fs::read_dir("/sys/class/block").unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    let mut found = vec![];
    for n in &names {
        let u = uevent(n);
        say(&format!("DISCOVERY /sys/class/block/{n}/uevent: DEVTYPE={:?} DEVNAME={:?} MAJOR:MINOR={}:{} PARTN={:?} PARTNAME={:?}", u.get("DEVTYPE"), u.get("DEVNAME"), u.get("MAJOR").map(String::as_str).unwrap_or("?"), u.get("MINOR").map(String::as_str).unwrap_or("?"), u.get("PARTN"), u.get("PARTNAME")));
        if u.get("PARTNAME").map(String::as_str) == Some(LABEL) { found.push((n.clone(), u)); }
    }
    assert!(found.len() == 1, "expected exactly one partition labelled {LABEL}, found {}", found.len());
    let (n, u) = &found[0];
    let dev = format!("/dev/{}", u.get("DEVNAME").unwrap_or(n));
    let mm = rdev_mm(&dev);
    say(&format!("DISCOVERY RESULT: PARTNAME={LABEL} -> {dev}; node is_block_device={} stat major:minor={mm:?} uevent MAJOR:MINOR={}:{}", is_blk(&dev), u.get("MAJOR").map(String::as_str).unwrap_or("?"), u.get("MINOR").map(String::as_str).unwrap_or("?")));
    dev
}

// ------------------------------------------------------------------ common baseline report
fn baseline(phase: &str) {
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default();
    say(&format!("PHASE {phase} START uname_r={} boot_id={}", std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim(), boot_id.trim()));
    say(&format!("cmdline: {}", std::fs::read_to_string("/proc/cmdline").unwrap_or_default().trim()));
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    say(&format!("mounts:\n{}", mounts.trim_end()));
    // read-only root: prove a write fails.
    for p in ["/probe-write-root", "/etc/probe-write", "/usr/sbin/probe-write", "/var-probe-write"] {
        let _ = std::fs::create_dir_all("/etc");
        match OpenOptions::new().write(true).create(true).open(p) {
            Ok(_) => say(&format!("ROOT-WRITE {p}: SUCCEEDED (root is NOT read-only!)")),
            Err(e) => say(&format!("ROOT-WRITE {p}: failed as required: {e} (errno {:?})", e.raw_os_error())),
        }
    }
    match std::fs::write("/run/probe-write", b"x") { Ok(_) => say("RUN-WRITE /run/probe-write: ok (tmpfs)"), Err(e) => say(&format!("RUN-WRITE failed: {e}")) }
    // no udev, no LVM daemons
    let mut procs = vec![];
    if let Ok(rd) = std::fs::read_dir("/proc") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.chars().all(|c| c.is_ascii_digit()) { procs.push(format!("{n}:{}", std::fs::read_to_string(format!("/proc/{n}/comm")).unwrap_or_default().trim())); }
        }
    }
    say(&format!("PROCESSES: {} (udev/lvm daemons present: {})", procs.join(" "), procs.iter().any(|p| p.contains("udev") || p.contains("dmeventd") || p.contains("lvmpolld") || p.contains("lvmetad"))));
    say(&format!("/run/udev exists: {}; /run/lvm exists: {}; /etc/udev exists: {}; /lib/udev exists: {}", std::path::Path::new("/run/udev").exists(), std::path::Path::new("/run/lvm").exists(), std::path::Path::new("/etc/udev").exists(), std::path::Path::new("/lib/udev").exists()));
    say(&format!("/dev before: {}", ls("/dev")));
    say(&format!("/dev/mapper before: {}", ls("/dev/mapper")));
    say(&format!("/proc/partitions:\n{}", std::fs::read_to_string("/proc/partitions").unwrap_or_default().trim_end()));
    say(&format!("lvm closure: {}", Command::new(LVM).arg("version").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().replace('\n', " | ")).unwrap_or_else(|e| format!("spawn failed: {e}"))));
}

fn others_hash(tag: &str) {
    // every device the filter must hide: root disk, the other partitions on the data disk.
    for d in ["/dev/vda", "/dev/vdb1", "/dev/vdb3", "/dev/vdb4"] { say(&format!("OTHER-HASH [{tag}] {d} {}", hash_device(d))); }
}

// ------------------------------------------------------------------ data (step 2/4)
fn write_pattern(node: &str) {
    let f = OpenOptions::new().read(true).write(true).custom_flags(0o40000).open(node).unwrap_or_else(|e| { say(&format!("DATA open {node} failed: {e}")); std::process::exit(2) });
    let mut buf = Aligned::new(MIB as usize);
    for &m in &WRITE_OFFSETS_MIB {
        fill_block(buf.as_mut(), m, NONCE);
        let h = fnv(buf.as_ref());
        match f.write_at(buf.as_ref(), m * MIB) { Ok(n) => say(&format!("DATA WROTE node={node} off={m}MiB len={n} fnv={h:016x}")), Err(e) => say(&format!("DATA WRITE FAILED off={m}MiB: {e}")) }
    }
    match f.sync_all() { Ok(()) => say("DATA fsync ok"), Err(e) => say(&format!("DATA fsync failed: {e}")) }
}
fn read_pattern(node: &str) -> bool {
    let f = match OpenOptions::new().read(true).custom_flags(0o40000).open(node) { Ok(f) => f, Err(e) => { say(&format!("DATA READBACK open {node} failed: {e}")); return false; } };
    let mut buf = Aligned::new(MIB as usize);
    let mut all = true;
    for &m in &WRITE_OFFSETS_MIB {
        let mut want = Aligned::new(MIB as usize);
        fill_block(want.as_mut(), m, NONCE);
        let wh = fnv(want.as_ref());
        match f.read_at(buf.as_mut(), m * MIB) {
            Ok(n) => { let gh = fnv(&buf.as_ref()[..n]); let ok = n == MIB as usize && gh == wh; all &= ok; say(&format!("DATA READBACK node={node} off={m}MiB len={n} fnv={gh:016x} expected={wh:016x} match={ok}")); }
            Err(e) => { all = false; say(&format!("DATA READBACK read failed off={m}MiB: {e}")); }
        }
    }
    say(&format!("DATA READBACK RESULT all_match={all}"));
    all
}

// ------------------------------------------------------------------ phases
fn phase_a(pv: &str) {
    let prod = Lvm::new(&filter_for(&[pv]));
    let open = Lvm::new(PERMISSIVE);
    baseline("a");

    // ---- step 1: filter proof
    say("STEP1 discovery + filter");
    others_hash("a-start");
    let decoy = Lvm::new(&filter_for(&["/dev/vdb3"]));
    lv(&decoy, "decoy-pvcreate", "pvcreate", &["-y", "/dev/vdb3"]);
    lv(&decoy, "decoy-vgcreate", "vgcreate", &["decoy", "/dev/vdb3"]);
    others_hash("a-after-decoy");
    lv(&open, "unfiltered-pvs-all", "pvs", &["-a", "-o", "pv_name,pv_size,vg_name"]);
    lv(&open, "unfiltered-vgs", "vgs", &["-o", "vg_name,pv_count"]);
    lv(&prod, "filtered-pvs-all", "pvs", &["-a", "-o", "pv_name,pv_size,vg_name"]);
    lv(&prod, "filtered-vgs", "vgs", &["-o", "vg_name,pv_count"]);
    let n1 = lv(&prod, "filtered-pvcreate-other-partition (must be refused)", "pvcreate", &["-y", "/dev/vdb1"]);
    let n2 = lv(&prod, "filtered-vgcreate-root-disk (must be refused)", "vgcreate", &["evil", "/dev/vda"]);
    let n3 = lv(&prod, "filtered-vgextend-decoy-pv (must be refused)", "vgcreate", &["evil2", "/dev/vdb3"]);
    say(&format!("STEP1 RESULT: refused-writes pvcreate_other={} vgcreate_root={} vgcreate_decoy_pv={}", !n1.ok(), !n2.ok(), !n3.ok()));

    // ---- step 2: vgcreate latency cycles, then the real vgcreate
    say("STEP2 vgcreate/lvcreate");
    let mut vgc = vec![]; let mut vgr = vec![];
    for i in 0..5 {
        let o = lv(&prod, &format!("vgcreate-cycle-{i}"), "vgcreate", &["-s", "4m", VG, pv]);
        assert!(o.ok(), "vgcreate failed");
        vgc.push(o.ms);
        if i < 4 { let r = lv(&prod, &format!("vgremove-cycle-{i}"), "vgremove", &["-f", VG]); assert!(r.ok(), "vgremove failed"); vgr.push(r.ms); }
    }
    series("vgcreate -s 4m (cycle 0 includes implicit PV init)", &vgc);
    series("vgremove -f (empty VG)", &vgr);
    let mut lc = vec![]; let mut ln = vec![]; let mut lrm = vec![];
    for i in 0..7 {
        let o = lv_node(&prod, &format!("lvcreate-lat-{i}"), "lvcreate", &["-L", "64m", "-n", "lat", VG], "/dev/mapper/overdrive-lat");
        assert!(o.ok() && o.node_ms.is_some(), "lat lvcreate failed or node absent");
        lc.push(o.ms); ln.push(o.node_ms.unwrap());
        if i == 0 { node_report("lat-after-create", "/dev/mapper/overdrive-lat"); }
        let r = lv(&prod, &format!("lvremove-lat-{i}"), "lvremove", &["-f", &format!("{VG}/lat")]);
        lrm.push(r.ms);
        say(&format!("after lvremove lat-{i}: node present={} dm={:?}", std::path::Path::new("/dev/mapper/overdrive-lat").exists(), dm_devices()));
    }
    series("lvcreate -L 64m total (command returned)", &lc);
    series("lvcreate -L 64m node appeared (from spawn, polled every 100us)", &ln);
    series("lvremove -f total", &lrm);
    let o = lv_node(&prod, "lvcreate-ovd-test", "lvcreate", &["-L", "64m", "-n", LV_TEST, VG], LV_TEST_NODE);
    assert!(o.ok(), "lvcreate ovd-test failed");
    node_report("ovd-test", LV_TEST_NODE);
    say(&format!("symlink /dev/{VG}/{LV_TEST}: {:?}; ls /dev/{VG}: {}", std::fs::read_link(format!("/dev/{VG}/{LV_TEST}")), ls(&format!("/dev/{VG}"))));
    say(&format!("/dev/mapper after: {}; dm devices: {:?}", ls("/dev/mapper"), dm_devices()));
    say(&format!("/run tree: /run={} /run/lock={} /run/lock/lvm={} /run/lvm={}", ls("/run"), ls("/run/lock"), ls("/run/lock/lvm"), ls("/run/lvm")));
    say(&format!("STEP2 RESULT: lvcreate exit={:?}; node is_block_device={}", o.code, is_blk(LV_TEST_NODE)));

    // ---- step 3: lvs json (the harness parses it)
    say("STEP3 lvs json");
    let j = prod.go("lvs", &["--reportformat", "json", "-o", "lv_name,lv_size,lv_attr,vg_free,vg_extent_size", "--units", "b"]);
    show("lvs-json", "lvs", &["--reportformat", "json", "-o", "lv_name,lv_size,lv_attr,vg_free,vg_extent_size", "--units", "b"], &j);
    say("LVSJSON-BEGIN");
    say(&j.out);
    say("LVSJSON-END");
    lv(&prod, "vgs", "vgs", &["-o", "vg_name,vg_extent_size,vg_extent_count,vg_free_count,vg_seqno"]);
    lv(&prod, "vgck", "vgck", &[VG]);

    // ---- data for step 4
    write_pattern(LV_TEST_NODE);
    read_pattern(LV_TEST_NODE);
    others_hash("a-end");
    unsafe { sync(); }
}

fn phase_b(pv: &str) {
    let prod = Lvm::new(&filter_for(&[pv]));
    baseline("b");
    say("STEP4 fresh guest over the same backing file");
    say(&format!("/dev/mapper at boot (before any lvm command): {}", ls("/dev/mapper")));
    say(&format!("dm devices at boot: {:?}; node {LV_TEST_NODE} present: {}", dm_devices(), std::path::Path::new(LV_TEST_NODE).exists()));
    let mut t = vec![];
    for i in 0..3 {
        let o = lv(&prod, &format!("pvs-{i}"), "pvs", &["-o", "pv_name,vg_name,pv_size"]); t.push(o.ms);
    }
    series("pvs (boot-probe discovery cost)", &t);
    let mut t = vec![];
    for i in 0..3 { let o = lv(&prod, &format!("vgs-{i}"), "vgs", &["-o", "vg_name,vg_free_count,vg_seqno"]); t.push(o.ms); }
    series("vgs", &t);
    let mut t = vec![];
    for i in 0..3 { let o = lv(&prod, &format!("lvs-{i}"), "lvs", &["-a", "-o", "lv_name,lv_attr,lv_active,lv_size"]); t.push(o.ms); }
    series("lvs -a", &t);
    say(&format!("node before lvchange -ay present: {}", std::path::Path::new(LV_TEST_NODE).exists()));
    let o = lv_node(&prod, "lvchange-ay", "lvchange", &["-ay", &format!("{VG}/{LV_TEST}")], LV_TEST_NODE);
    assert!(o.ok(), "lvchange -ay failed");
    node_report("ovd-test-after-lvchange-ay", LV_TEST_NODE);
    say(&format!("STEP4 node after lvchange -ay: is_block_device={}", is_blk(LV_TEST_NODE)));
    let ok = read_pattern(LV_TEST_NODE);
    say(&format!("STEP4 RESULT readback_all_match={ok}"));
    // activation latency cycles
    let (mut an, mut at, mut dt) = (vec![], vec![], vec![]);
    for i in 0..7 {
        let d = lv(&prod, &format!("lvchange-an-{i}"), "lvchange", &["-an", &format!("{VG}/{LV_TEST}")]); dt.push(d.ms);
        say(&format!("node after -an: present={} dm={:?}", std::path::Path::new(LV_TEST_NODE).exists(), dm_devices()));
        let a = lv_node(&prod, &format!("lvchange-ay-{i}"), "lvchange", &["-ay", &format!("{VG}/{LV_TEST}")], LV_TEST_NODE);
        at.push(a.ms); an.push(a.node_ms.unwrap_or(f64::NAN));
    }
    series("lvchange -an", &dt);
    series("lvchange -ay total", &at);
    series("lvchange -ay node appeared", &an);
    let v = lv(&prod, "vgchange-ay-idempotent", "vgchange", &["-ay", VG]);
    say(&format!("vgchange -ay on active VG: exit={:?}", v.code));
    let ok2 = read_pattern(LV_TEST_NODE);
    say(&format!("STEP4 RESULT after re-activation cycles: readback_all_match={ok2}"));
    lv(&prod, "lvs-active", "lvs", &["-a", "-o", "lv_name,lv_attr,lv_active,lv_size"]);
    others_hash("b-end");
}

fn lv_names(prod: &Lvm) -> Vec<String> {
    let o = prod.go("lvs", &["--noheadings", "-o", "lv_name", "-a"]);
    o.out.split_whitespace().map(|s| s.to_owned()).collect()
}
fn state_line(prod: &Lvm, tag: &str) {
    let names = lv_names(prod);
    let l = prod.go("lvs", &["-a", "--noheadings", "-o", "lv_name,lv_attr,lv_active,lv_suspended,lv_device_open,lv_size", "--units", "m"]);
    let v = prod.go("vgs", &["--noheadings", "-o", "vg_seqno,vg_free_count,lv_count"]);
    let dms = dm_devices();
    // orphans: dm devices whose name has no matching LV in metadata
    let orphans: Vec<&String> = dms.iter().filter(|d| { let n = d.split('=').nth(1).unwrap_or("").split('(').next().unwrap_or(""); n.starts_with("overdrive-") && !names.iter().any(|x| format!("overdrive-{}", x.replace('-', "--")) == n) }).collect();
    say(&format!("STATE [{tag}] lvs_code={:?} vgs={:?} lvs={:?} dm={:?} orphan_dm={:?}", l.code, v.out.trim().replace('\n', ";"), l.out.trim().replace('\n', ";"), dms, orphans));
    if !l.err.trim().is_empty() { say(&format!("STATE [{tag}] lvs stderr: {}", l.err.trim().replace('\n', " | "))); }
    if !v.err.trim().is_empty() { say(&format!("STATE [{tag}] vgs stderr: {}", v.err.trim().replace('\n', " | "))); }
}

/// Spawn lvm, SIGKILL it after `delay`, record how it ended.
fn killed_run(prod: &Lvm, sub: &str, rest: &[&str], delay: Duration) -> String {
    let t0 = Instant::now();
    let mut c = prod.command(sub, rest).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn");
    while t0.elapsed() < delay { if c.try_wait().unwrap().is_some() { break; } std::thread::sleep(Duration::from_micros(50)); }
    if let Some(s) = c.try_wait().unwrap() { return format!("COMPLETED-BEFORE-KILL(code={:?})", s.code()); }
    let sent = t0.elapsed().as_secs_f64() * 1e3;
    c.kill().unwrap();
    let s = c.wait().unwrap();
    format!("KILLED(sig={:?}, SIGKILL sent at {sent:.1}ms, process gone at {:.1}ms)", s.signal(), t0.elapsed().as_secs_f64() * 1e3)
}

/// "absent" | "present-active" | "present-inactive" for one LV, from `lvs -a`.
fn lv_state(prod: &Lvm, name: &str) -> String {
    let o = prod.go("lvs", &["-a", "--noheadings", "-o", "lv_name,lv_attr"]);
    if !o.ok() { return format!("lvs-failed(code={:?})", o.code); }
    for l in o.out.lines() {
        let mut it = l.split_whitespace();
        if it.next() == Some(name) { let attr = it.next().unwrap_or(""); return format!("present-{}(attr {attr})", if attr.as_bytes().get(4) == Some(&b'a') { "active" } else { "inactive" }); }
    }
    "absent".into()
}
fn orphans(prod: &Lvm) -> Vec<String> {
    let names = lv_names(prod);
    dm_devices().into_iter().filter(|d| { let n = d.split('=').nth(1).unwrap_or("").split('(').next().unwrap_or(""); n.starts_with("overdrive-") && !names.iter().any(|x| format!("overdrive-{}", x.replace('-', "--")) == n) }).collect()
}

fn phase_c(pv: &str) {
    let prod = Lvm::new(&filter_for(&[pv]));
    baseline("c");
    say("STEP5 crash safety");
    lv(&prod, "activate", "lvchange", &["-ay", &format!("{VG}/{LV_TEST}")]);
    let before = prod.go("vgs", &["--noheadings", "-o", "vg_free_count,vg_extent_count,vg_seqno"]);
    say(&format!("STEP5 baseline vgs: {}", before.out.trim()));
    // baseline durations of the operations we will kill
    let (mut tc, mut tr) = (vec![], vec![]);
    for i in 0..5 {
        let c = prod.go("lvcreate", &["-L", "16m", "-n", "base", VG]);
        let r = prod.go("lvremove", &["-f", &format!("{VG}/base")]);
        say(&format!("baseline {i}: lvcreate {:.1}ms exit={:?}; lvremove {:.1}ms exit={:?}", c.ms, c.code, r.ms, r.code));
        tc.push(c.ms); tr.push(r.ms);
    }
    let (mc, mr) = (median(&mut tc), median(&mut tr));
    say(&format!("STEP5 operation baselines: lvcreate median {mc:.1}ms, lvremove median {mr:.1}ms; kill delays uniform in [0, 1.2 x median]"));
    let mut rng = Rng::seeded();
    say(&format!("STEP5 rng seed state {:#x}", rng.0));
    let mut clean_failures = 0;
    let mut cleanup_failures = 0;
    let mut leaks = 0;
    let mut killed = 0; let mut completed = 0;
    let free_baseline = before.out.split_whitespace().next().unwrap_or("").to_owned();
    for i in 1..=20 {
        // (1) SIGKILLed lvcreate
        let d1 = Duration::from_micros(((rng.next() % 1_000_000) as f64 / 1_000_000.0 * 1.2 * mc * 1000.0) as u64);
        let name = format!("kc{i}");
        let r1 = killed_run(&prod, "lvcreate", &["-L", "16m", "-n", &name, VG], d1);
        if r1.starts_with("KILLED") { killed += 1 } else { completed += 1 }
        let st1 = lv_state(&prod, &name);
        say(&format!("ITER {i} lvcreate {name} delay={:.1}ms -> {r1}; LV afterwards: {st1}", d1.as_secs_f64() * 1e3));
        state_line(&prod, &format!("iter {i} after killed lvcreate"));
        // (2) clean victim create: proves the next command needs no repair after the kill above
        let vname = format!("kr{i}");
        let v = prod.go("lvcreate", &["-L", "16m", "-n", &vname, VG]);
        if !v.ok() { clean_failures += 1; show(&format!("iter {i} CLEAN lvcreate {vname} FAILED"), "lvcreate", &["-L", "16m", "-n", &vname, VG], &v); }
        else { say(&format!("ITER {i} clean lvcreate {vname}: exit=0 {:.1}ms stderr={:?}", v.ms, v.err.trim())); }
        // (3) SIGKILLed lvremove
        let d2 = Duration::from_micros(((rng.next() % 1_000_000) as f64 / 1_000_000.0 * 1.2 * mr * 1000.0) as u64);
        let r2 = killed_run(&prod, "lvremove", &["-f", &format!("{VG}/{vname}")], d2);
        if r2.starts_with("KILLED") { killed += 1 } else { completed += 1 }
        let st2 = lv_state(&prod, &vname);
        say(&format!("ITER {i} lvremove {vname} delay={:.1}ms -> {r2}; LV afterwards: {st2}", d2.as_secs_f64() * 1e3));
        state_line(&prod, &format!("iter {i} after killed lvremove"));
        // (4) ordinary cleanup: remove whatever LV a kill left committed, with plain `lvremove -f` (no vgcfgrestore, no dmsetup).
        for n in lv_names(&prod).iter().filter(|n| *n != LV_TEST) {
            let r = prod.go("lvremove", &["-f", &format!("{VG}/{n}")]);
            if !r.ok() { cleanup_failures += 1; show(&format!("iter {i} cleanup lvremove {n} FAILED"), "lvremove", &["-f", &format!("{VG}/{n}")], &r); }
        }
        let fr = prod.go("vgs", &["--noheadings", "-o", "vg_free_count"]);
        let leaked = fr.out.trim() != free_baseline;
        if leaked { leaks += 1; }
        let orph = orphans(&prod);
        say(&format!("ITERSUM {i}: create_kill={} LV_after={st1} | remove_kill={} LV_after={st2} | after ordinary cleanup: free_extents={} (baseline {free_baseline}) leaked={leaked} orphan_dm={orph:?}",
            if r1.starts_with("KILLED") { "killed" } else { "finished-first" }, if r2.starts_with("KILLED") { "killed" } else { "finished-first" }, fr.out.trim()));
    }
    say(&format!("STEP5 STORM DONE: kills landed={killed} completed_before_kill={completed} clean_op_failures_between_kills={clean_failures} cleanup_failures={cleanup_failures} iterations_with_leaked_extents={leaks}"));

    // ---- recovery, with no manual repair
    say("STEP5 recovery (no manual repair): vgs, lvs, vgchange -ay, clean lvcreate");
    let a = lv(&prod, "vgs", "vgs", &["-o", "vg_name,vg_seqno,vg_free_count,lv_count"]);
    let b = lv(&prod, "lvs", "lvs", &["-a", "-o", "lv_name,lv_attr,lv_active,lv_suspended,lv_device_open,lv_size"]);
    let c = lv(&prod, "vgchange-ay", "vgchange", &["-ay", VG]);
    let ck = lv(&prod, "vgck", "vgck", &[VG]);
    let fin = lv_node(&prod, "clean-lvcreate", "lvcreate", &["-L", "64m", "-n", "clean-final", VG], "/dev/mapper/overdrive-clean--final");
    say(&format!("STEP5 recovery exits: vgs={:?} lvs={:?} vgchange_ay={:?} vgck={:?} clean_lvcreate={:?} node_present={}", a.code, b.code, c.code, ck.code, fin.code, is_blk("/dev/mapper/overdrive-clean--final")));
    state_line(&prod, "after recovery");
    // leftovers from kills that landed after the metadata commit are ordinary LVs: all must be removable.
    let names = lv_names(&prod);
    say(&format!("STEP5 LVs present after storm+recovery: {names:?}"));
    for n in names.iter().filter(|n| *n != LV_TEST) {
        let r = lv(&prod, &format!("cleanup-lvremove-{n}"), "lvremove", &["-f", &format!("{VG}/{n}")]);
        if !r.ok() { say(&format!("STEP5 cleanup of {n} FAILED")); }
    }
    let after = prod.go("vgs", &["--noheadings", "-o", "vg_free_count,vg_extent_count,vg_seqno"]);
    say(&format!("STEP5 vgs after cleanup: {} (baseline before storm: {}; ovd-test holds 16 of 4 MiB extents)", after.out.trim(), before.out.trim()));
    state_line(&prod, "after cleanup");
    let ok = read_pattern(LV_TEST_NODE);
    say(&format!("STEP5 RESULT: ovd-test data intact after storm: {ok}"));
    others_hash("c-end");
    say("STEP5 CHECKS-DONE");
    // No clean shutdown: the harness SIGKILLs the VMM now, so the next guest boots from a cold, crashed store.
    loop { std::thread::sleep(Duration::from_secs(1)); }
}

fn phase_d(pv: &str) {
    let prod = Lvm::new(&filter_for(&[pv]));
    baseline("d");
    say("STEP5b fresh guest after the storm and an unclean VMM kill");
    lv(&prod, "pvs", "pvs", &["-o", "pv_name,vg_name,pv_size,pv_free"]);
    lv(&prod, "vgs", "vgs", &["-o", "vg_name,vg_seqno,vg_free_count,vg_extent_count,lv_count"]);
    lv(&prod, "lvs", "lvs", &["-a", "-o", "lv_name,lv_attr,lv_active,lv_size"]);
    let c = lv(&prod, "vgchange-ay", "vgchange", &["-ay", VG]);
    let ck = lv(&prod, "vgck", "vgck", &[VG]);
    let f = lv_node(&prod, "clean-lvcreate", "lvcreate", &["-L", "64m", "-n", "final", VG], "/dev/mapper/overdrive-final");
    say(&format!("STEP5b exits: vgchange_ay={:?} vgck={:?} clean_lvcreate={:?}", c.code, ck.code, f.code));
    let ok = read_pattern(LV_TEST_NODE);
    say(&format!("STEP5b RESULT readback_all_match={ok}"));
    lv(&prod, "lvremove-final", "lvremove", &["-f", &format!("{VG}/final")]);
    lv(&prod, "vgs-end", "vgs", &["-o", "vg_name,vg_seqno,vg_free_count,lv_count"]);
    others_hash("d-end");
}

/// Which way does LVM get a system dir on a read-only root? Try each, one cheap read-only command.
fn phase_s(pv: &str) {
    baseline("s");
    say(&format!("/etc/lvm exists in this image: {}", std::path::Path::new("/etc/lvm").exists()));
    for sd in ["unset", "empty", "run"] {
        let l = Lvm::with_sysdir(&filter_for(&[pv]), sd);
        say(&format!("SYSDIR variant {sd:?} (LVM_SYSTEM_DIR: unset / set to empty string / /run/lvm)"));
        lv(&l, &format!("sysdir-{sd}-pvs"), "pvs", &["-o", "pv_name,vg_name"]);
        say(&format!("after: /run/lvm exists={} ls /run={}", std::path::Path::new("/run/lvm").exists(), ls("/run")));
    }
}

/// Phase e: the boot after a VMM crash. Report the store's state with ordinary read commands, then
/// loop lvcreate/lvremove until the harness SIGKILLs the whole VMM (a node crash mid-operation).
fn phase_e(pv: &str, seq: &str) {
    let prod = Lvm::new(&filter_for(&[pv]));
    baseline("e");
    say(&format!("CRASHBOOT {seq}: state after the previous VMM crash, before any write"));
    lv(&prod, "pvs", "pvs", &["-o", "pv_name,vg_name,pv_free"]);
    let v = lv(&prod, "vgs", "vgs", &["-o", "vg_name,vg_seqno,vg_free_count,lv_count"]);
    let l = lv(&prod, "lvs", "lvs", &["-a", "-o", "lv_name,lv_attr,lv_active,lv_size"]);
    let ck = lv(&prod, "vgck", "vgck", &[VG]);
    let ay = lv(&prod, "vgchange-ay", "vgchange", &["-ay", VG]);
    say(&format!("CRASHBOOT {seq} RESULT exits: vgs={:?} lvs={:?} vgck={:?} vgchange_ay={:?}", v.code, l.code, ck.code, ay.code));
    say("LOOP-START");
    let mut i = 0u64;
    loop {
        let name = format!("e{seq}x{i}");
        say(&format!("LOOP begin lvcreate {name}"));
        let c = prod.go("lvcreate", &["-L", "16m", "-n", &name, VG]);
        if !c.ok() { say(&format!("LOOP lvcreate {name} exit={:?} err={:?}", c.code, c.err.trim())); }
        say(&format!("LOOP begin lvremove {name}"));
        let r = prod.go("lvremove", &["-f", &format!("{VG}/{name}")]);
        if !r.ok() { say(&format!("LOOP lvremove {name} exit={:?} err={:?}", r.code, r.err.trim())); }
        i += 1;
        say(&format!("LOOP cycle {i} done"));
    }
}

/// Phase f: cold check after the VMM-crash series; ordinary commands only.
fn phase_f(pv: &str) {
    let prod = Lvm::new(&filter_for(&[pv]));
    baseline("f");
    let v = lv(&prod, "vgs", "vgs", &["-o", "vg_name,vg_seqno,vg_free_count,vg_extent_count,lv_count"]);
    let l = lv(&prod, "lvs", "lvs", &["-a", "-o", "lv_name,lv_attr,lv_active,lv_size"]);
    let ck = lv(&prod, "vgck", "vgck", &[VG]);
    let ay = lv(&prod, "vgchange-ay", "vgchange", &["-ay", VG]);
    let names = lv_names(&prod);
    say(&format!("CRASHFINAL LVs present: {names:?}; dm devices: {:?}", dm_devices()));
    let mut cleanup_fail = 0;
    for n in names.iter().filter(|n| *n != LV_TEST) { let r = lv(&prod, &format!("cleanup-lvremove-{n}"), "lvremove", &["-f", &format!("{VG}/{n}")]); if !r.ok() { cleanup_fail += 1; } }
    let fin = lv_node(&prod, "clean-lvcreate", "lvcreate", &["-L", "64m", "-n", "final", VG], "/dev/mapper/overdrive-final");
    lv(&prod, "lvremove-final", "lvremove", &["-f", &format!("{VG}/final")]);
    let after = prod.go("vgs", &["--noheadings", "-o", "vg_free_count,vg_extent_count"]);
    say(&format!("CRASHFINAL RESULT exits: vgs={:?} lvs={:?} vgck={:?} vgchange_ay={:?} cleanup_failures={cleanup_fail} clean_lvcreate={:?}; free/total extents after cleanup: {} (baseline 47 63); orphan_dm={:?}", v.code, l.code, ck.code, ay.code, fin.code, after.out.trim(), orphans(&prod)));
    lv_node(&prod, "ovd-test-activate", "lvchange", &["-ay", &format!("{VG}/{LV_TEST}")], LV_TEST_NODE);
    let ok = read_pattern(LV_TEST_NODE);
    say(&format!("CRASHFINAL DATA readback_all_match={ok}"));
    others_hash("f-end");
}

fn main() {
    do_mount("proc", "/proc", "proc", 0);
    do_mount("sysfs", "/sys", "sysfs", 0);
    do_mount("devtmpfs", "/dev", "devtmpfs", 0);
    do_mount("tmpfs", "/run", "tmpfs", 0);
    let p = params();
    let _ = SYSDIR.set(p.get("sysdir").cloned().unwrap_or_else(|| "unset".into()));
    let phase = p.get("phase").cloned().unwrap_or_default();
    let pv = discover();
    match phase.as_str() {
        "s" => phase_s(&pv),
        "e" => phase_e(&pv, p.get("seq").map(String::as_str).unwrap_or("0")),
        "f" => phase_f(&pv),
        "a" => phase_a(&pv),
        "b" => phase_b(&pv),
        "c" => phase_c(&pv),
        "d" => phase_d(&pv),
        other => say(&format!("unknown phase {other:?}")),
    }
    say(&format!("PHASE {phase} DONE"));
    unsafe { sync(); reboot(0x4321_fedc) }; // RB_POWER_OFF
    loop { std::thread::sleep(Duration::from_secs(1)); }
}
