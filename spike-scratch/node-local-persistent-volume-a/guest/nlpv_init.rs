//! Inner-guest PID 1 and workload (static musl, std only). Throwaway probe code.
//! Parameters arrive as `nlpv.<key>=<value>` tokens on the kernel command line.
use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

extern "C" {
    fn mount(src: *const i8, tgt: *const i8, fstype: *const i8, flags: u64, data: *const i8) -> i32;
    fn reboot(cmd: i32) -> i32;
    fn ioctl(fd: i32, req: u64, ...) -> i32;
    fn sync();
}

fn say(s: &str) {
    // The serial console is the evidence channel.
    let mut out = OpenOptions::new().write(true).open("/dev/console").ok();
    let line = format!("NLPV-G {s}\n");
    match out.as_mut() {
        Some(f) => { let _ = f.write_all(line.as_bytes()); }
        None => print!("{line}"),
    }
}

fn do_mount(src: &str, tgt: &str, fs: &str) {
    let _ = std::fs::create_dir_all(tgt);
    let (s, t, f) = (CString::new(src).unwrap(), CString::new(tgt).unwrap(), CString::new(fs).unwrap());
    let r = unsafe { mount(s.as_ptr(), t.as_ptr(), f.as_ptr(), 0, std::ptr::null()) };
    if r != 0 { say(&format!("mount {fs} on {tgt} failed: {}", std::io::Error::last_os_error())); }
}

fn params() -> BTreeMap<String, String> {
    let c = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    c.split_whitespace()
        .filter_map(|t| t.strip_prefix("nlpv.").and_then(|t| t.split_once('=')))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

struct Aligned { ptr: *mut u8, len: usize }
impl Aligned {
    fn new(len: usize) -> Self {
        let layout = std::alloc::Layout::from_size_align(len, 4096).unwrap();
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        Aligned { ptr, len }
    }
    fn as_mut(&mut self) -> &mut [u8] { unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) } }
    fn as_ref(&self) -> &[u8] { unsafe { std::slice::from_raw_parts(self.ptr, self.len) } }
}

fn xorshift(mut x: u64) -> u64 { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x }

pub fn fill_block(buf: &mut [u8], i: u64, nonce: u64) {
    buf[0..8].copy_from_slice(b"NLPVBLK1");
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

fn errno(e: &std::io::Error) -> i32 { e.raw_os_error().unwrap_or(-1) }

fn open_dev(path: &str, direct: bool) -> File {
    let mut o = OpenOptions::new();
    o.read(true).write(true);
    if direct { o.custom_flags(0o40000); } // O_DIRECT on x86_64
    o.open(path).unwrap_or_else(|e| { say(&format!("open {path} failed: {e}")); finish(2) })
}

fn finish(_code: i32) -> ! {
    say("DONE");
    unsafe { reboot(0x0123_4567) }; // RB_AUTOBOOT: a guest reset ends the VMM in the fork
    loop { std::thread::sleep(Duration::from_secs(1)); }
}

fn set_cache(p: &BTreeMap<String, String>) {
    if let Some(mode) = p.get("cache") {
        let want = if mode == "wt" { "write through" } else { "write back" };
        let r = std::fs::write("/sys/block/vdb/cache_type", want);
        let now = std::fs::read_to_string("/sys/block/vdb/cache_type").unwrap_or_default();
        say(&format!("cache_type set {want:?} -> {r:?}; now {:?}; write_cache={:?}",
            now.trim(), std::fs::read_to_string("/sys/block/vdb/queue/write_cache").unwrap_or_default().trim()));
    }
}

fn num(p: &BTreeMap<String, String>, k: &str, d: u64) -> u64 {
    p.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
}

/// Raw block writes with per-write flush and an ACK line. The outer test
/// places a dm-log-writes mark on every ACK and later checks that every
/// acknowledged block was in the log before the last flush preceding its mark.
fn ackraw(p: &BTreeMap<String, String>) {
    let dev = p.get("dev").cloned().unwrap_or("/dev/vdb".into());
    let n = num(p, "n", 50);
    let bs = num(p, "bs", 4096) as usize;
    let pace = num(p, "pace_ms", 30);
    let nonce = num(p, "nonce", 1);
    let direct = num(p, "odirect", 0) == 1;
    let sync_kind = p.get("sync").cloned().unwrap_or("fdatasync".into());
    let startwait = num(p, "startwait_ms", 0);
    let f = open_dev(&dev, direct);
    set_cache(p);
    say(&format!("READY dev={dev} n={n} bs={bs} odirect={direct} sync={sync_kind} pace={pace}"));
    std::thread::sleep(Duration::from_millis(startwait));
    let mut buf = Aligned::new(bs);
    for i in 1..=n {
        fill_block(buf.as_mut(), i, nonce);
        if num(p, "pre", 0) == 1 { say(&format!("PRE {i}")); }
        let w = f.write_at(buf.as_ref(), i * bs as u64);
        let s = match sync_kind.as_str() {
            "fsync" => f.sync_all(),
            "fdatasync" => f.sync_data(),
            _ => Ok(()),
        };
        match (&w, &s) {
            (Ok(_), Ok(())) => say(&format!("ACK {i}")),
            _ => say(&format!("ERR {i} write={} sync={}",
                w.as_ref().err().map(errno).unwrap_or(0), s.as_ref().err().map(errno).unwrap_or(0))),
        }
        std::thread::sleep(Duration::from_millis(pace));
    }
}

/// Write until the device errors; report how it fails. For ENOSPC on a sparse file.
fn fill(p: &BTreeMap<String, String>) {
    let dev = p.get("dev").cloned().unwrap_or("/dev/vdb".into());
    let bs = num(p, "bs", 1 << 20) as usize;
    let direct = num(p, "odirect", 0) == 1;
    let f = open_dev(&dev, direct);
    let size = {
        let mut sz: u64 = 0;
        unsafe { ioctl(f.as_raw_fd(), 0x8008_1272, &mut sz as *mut u64) }; // BLKGETSIZE64
        sz
    };
    say(&format!("READY fill dev={dev} size={size} bs={bs}"));
    let mut buf = Aligned::new(bs);
    let nonce = num(p, "nonce", 7);
    let mut off = 0u64;
    let mut i = 0u64;
    let mut first_err: Option<String> = None;
    let mut errors = 0u64;
    while off + bs as u64 <= size {
        fill_block(&mut buf.as_mut()[..], i + 1, nonce);
        match f.write_at(buf.as_ref(), off) {
            Ok(_) => {}
            Err(e) => {
                errors += 1;
                if first_err.is_none() { first_err = Some(format!("write at off={off} i={i}: errno={} {e}", errno(&e))); say(&format!("FIRSTERR {}", first_err.as_ref().unwrap())); }
                if errors > 5 { break; }
            }
        }
        i += 1;
        off += bs as u64;
        if i % 32 == 0 { say(&format!("PROGRESS {i}")); }
        if i % num(p, "sync_every", 16) == 0 {
            if let Err(e) = f.sync_data() {
                errors += 1;
                say(&format!("SYNCERR after i={i}: errno={} {e}", errno(&e)));
                if errors > 5 { break; }
            }
        }
    }
    say(&format!("FILLEND blocks_attempted={i} errors={errors}"));
    // Is the guest still alive/able to read back an early block?
    let mut rb = Aligned::new(bs);
    match f.read_at(rb.as_mut(), 0) { Ok(n) => say(&format!("READBACK_OK n={n}")), Err(e) => say(&format!("READBACK_ERR {e}")) }
}

/// Guest discard: fill, discard, read back, report.
fn discard(p: &BTreeMap<String, String>) {
    let dev = p.get("dev").cloned().unwrap_or("/dev/vdb".into());
    let f = open_dev(&dev, false);
    let bs = 1 << 20;
    let nblk = num(p, "blocks", 16);
    say(&format!("QUEUE discard_max_bytes={:?} discard_granularity={:?} write_zeroes_max_bytes={:?}",
        std::fs::read_to_string("/sys/block/vdb/queue/discard_max_bytes").unwrap_or_default().trim(),
        std::fs::read_to_string("/sys/block/vdb/queue/discard_granularity").unwrap_or_default().trim(),
        std::fs::read_to_string("/sys/block/vdb/queue/write_zeroes_max_bytes").unwrap_or_default().trim()));
    let mut buf = Aligned::new(bs);
    for i in 0..nblk { fill_block(buf.as_mut(), i + 1, 9); f.write_at(buf.as_ref(), i * bs as u64).unwrap(); }
    f.sync_all().unwrap();
    say("FILLED");
    std::thread::sleep(Duration::from_millis(300));
    say("PRE_DISCARD_MARK");
    let range: [u64; 2] = [0, nblk * bs as u64];
    let r = unsafe { ioctl(f.as_raw_fd(), 0x1277, range.as_ptr()) }; // BLKDISCARD
    say(&format!("BLKDISCARD ret={r} err={}", if r != 0 { std::io::Error::last_os_error().to_string() } else { "-".into() }));
    let _ = f.sync_all();
    let mut rb = Aligned::new(bs);
    let zero = f.read_at(rb.as_mut(), 0).map(|_| rb.as_ref().iter().all(|b| *b == 0));
    say(&format!("READBACK_ZERO={zero:?}"));
    std::thread::sleep(Duration::from_millis(300));
    say("POST_DISCARD_MARK");
}

/// A guest writes other image formats' magic to its own data disk.
fn magic(p: &BTreeMap<String, String>) {
    let dev = p.get("dev").cloned().unwrap_or("/dev/vdb".into());
    let f = open_dev(&dev, false);
    let size = { let mut sz: u64 = 0; unsafe { ioctl(f.as_raw_fd(), 0x8008_1272, &mut sz as *mut u64) }; sz };
    let kind = p.get("kind").cloned().unwrap_or("qcow2".into());
    say(&format!("READY magic kind={kind} size={size}"));
    match kind.as_str() {
        "qcow2" => {
            // QCOW magic "QFI\xfb", version 3, at offset 0.
            let mut hdr = vec![0u8; 512];
            hdr[0..4].copy_from_slice(&[0x51, 0x46, 0x49, 0xfb]);
            hdr[4..8].copy_from_slice(&3u32.to_be_bytes());
            f.write_at(&hdr, 0).unwrap();
        }
        "vhdx" => {
            let mut hdr = vec![0u8; 512];
            hdr[0..8].copy_from_slice(b"vhdxfile");
            f.write_at(&hdr, 0).unwrap();
        }
        "vhd" => {
            // Fixed VHD footer: cookie "conectix" in the last 512 bytes; disk type = 2 (fixed).
            let mut ft = vec![0u8; 512];
            ft[0..8].copy_from_slice(b"conectix");
            ft[8..12].copy_from_slice(&2u32.to_be_bytes());       // features
            ft[12..16].copy_from_slice(&0x0001_0000u32.to_be_bytes()); // file format version
            ft[16..24].copy_from_slice(&u64::MAX.to_be_bytes());  // data offset: none for fixed
            ft[40..48].copy_from_slice(&(size - 512).to_be_bytes());  // original size
            ft[48..56].copy_from_slice(&(size - 512).to_be_bytes());  // current size
            ft[60..64].copy_from_slice(&2u32.to_be_bytes());      // disk type fixed
            // checksum: one's complement of the byte sum with the field zeroed
            let sum: u32 = ft.iter().map(|b| *b as u32).sum();
            ft[64..68].copy_from_slice(&(!sum).to_be_bytes());
            f.write_at(&ft, size - 512).unwrap();
        }
        _ => {}
    }
    f.sync_all().unwrap();
    say("WROTE");
}


fn pct(v: &mut Vec<u128>, q: f64) -> u128 { v.sort(); if v.is_empty() { 0 } else { v[((v.len() - 1) as f64 * q) as usize] } }

fn bench(p: &BTreeMap<String, String>) {
    let dev = "/dev/vdb";
    do_mount(dev, "/mnt", "ext4");
    let iters = num(p, "iters", 6);
    let which = p.get("w").cloned().unwrap_or("smallfiles_nofsync,smallfiles_fsync,randsync,tree".into());
    let nsf = num(p, "nsf", 1500) as usize;
    say(&format!("BENCHREADY iters={iters} w={which} mnt={:?}", std::fs::read_to_string("/proc/mounts").unwrap_or_default().lines().filter(|l| l.contains("/mnt")).collect::<Vec<_>>()));
    for it in 1..=iters {
        for w in which.split(',') {
            let dir = format!("/mnt/b{it}-{w}");
            let _ = std::fs::create_dir_all(&dir);
            unsafe { sync(); }
            let _ = std::fs::write("/proc/sys/vm/drop_caches", "3");
            let t = Instant::now();
            let (ops, extra): (u64, String) = match w {
                "smallfiles_nofsync" | "smallfiles_fsync" => {
                    let data = vec![0x5au8; 8192];
                    for i in 0..nsf {
                        let d = format!("{dir}/d{}", i % 40); if i < 40 { let _ = std::fs::create_dir_all(&d); }
                        let mut f = File::create(format!("{d}/f{i}")).unwrap();
                        f.write_all(&data).unwrap();
                        if w == "smallfiles_fsync" { f.sync_all().unwrap(); }
                    }
                    if w == "smallfiles_nofsync" { unsafe { sync(); } }
                    (nsf as u64, String::new())
                }
                "randsync" => {
                    let path = format!("{dir}/rs.bin");
                    let f = OpenOptions::new().read(true).write(true).create(true).open(&path).unwrap();
                    let blk = vec![0xa5u8; 4096];
                    for i in 0..4096u64 { f.write_at(&blk, i * 4096).unwrap(); }
                    f.sync_all().unwrap();
                    let t2 = Instant::now();
                    let mut lat: Vec<u128> = Vec::new();
                    let mut x = 0x1234_5678_9abc_def1u64 ^ it;
                    let n = num(p, "nrs", 1500);
                    for _ in 0..n {
                        x = xorshift(x);
                        let pos = (x % 4096) * 4096;
                        let t3 = Instant::now();
                        f.write_at(&blk, pos).unwrap();
                        f.sync_data().unwrap();
                        lat.push(t3.elapsed().as_micros());
                    }
                    let tot = t2.elapsed().as_micros();
                    let (p50, p99) = (pct(&mut lat.clone(), 0.5), pct(&mut lat, 0.99));
                    // timing for this workload excludes the pre-fill
                    say(&format!("BENCH w={w} iter={it} ms={} ops={n} p50_us={p50} p99_us={p99}", tot / 1000));
                    let _ = std::fs::remove_dir_all(&dir);
                    continue;
                }
                "tree" => {
                    let sizes = [2048usize, 16384, 131072, 524288];
                    let nfiles = num(p, "ntree", 600) as usize;
                    let mut hs = vec![];
                    for th in 0..4usize {
                        let dir = dir.clone();
                        hs.push(std::thread::spawn(move || {
                            let d = format!("{dir}/t{th}"); std::fs::create_dir_all(&d).unwrap();
                            for i in (th..nfiles).step_by(4) {
                                let sz = sizes[i % 4];
                                let buf = vec![(i % 251) as u8; sz];
                                let mut f = File::create(format!("{d}/o{i}.rlib")).unwrap();
                                f.write_all(&buf).unwrap();
                                if i % 10 == 0 { f.sync_all().unwrap(); }
                            }
                        }));
                    }
                    for h in hs { h.join().unwrap(); }
                    unsafe { sync(); }
                    let wr = t.elapsed().as_millis();
                    let _ = std::fs::write("/proc/sys/vm/drop_caches", "3");
                    let t4 = Instant::now();
                    let mut total = 0u64;
                    for th in 0..4usize { for i in (th..nfiles).step_by(4) { total += std::fs::read(format!("{dir}/t{th}/o{i}.rlib")).unwrap().len() as u64; } }
                    (nfiles as u64, format!("write_ms={wr} read_ms={} bytes={total}", t4.elapsed().as_millis()))
                }
                _ => (0, "unknown".into()),
            };
            say(&format!("BENCH w={w} iter={it} ms={} ops={ops} {extra}", t.elapsed().as_millis()));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    unsafe { sync(); }
    say("BENCHDONE");
}

fn main() {
    do_mount("proc", "/proc", "proc");
    do_mount("sysfs", "/sys", "sysfs");
    do_mount("devtmpfs", "/dev", "devtmpfs");
    let p = params();
    say(&format!("INIT up; params={p:?}; kernel={}", std::fs::read_to_string("/proc/version").unwrap_or_default().trim()));
    let started = Instant::now();
    match p.get("mode").map(String::as_str) {
        Some("ackraw") => ackraw(&p),
        Some("fill") => fill(&p),
        Some("discard") => discard(&p),
        Some("magic") => magic(&p),
        Some("magicread") => {
            let dev = p.get("dev").cloned().unwrap_or("/dev/vdb".into());
            let f = open_dev(&dev, false);
            let size = { let mut sz: u64 = 0; unsafe { ioctl(f.as_raw_fd(), 0x8008_1272, &mut sz as *mut u64) }; sz };
            let mut head = vec![0u8; 512]; let mut tail = vec![0u8; 512];
            let h = f.read_at(&mut head, 0); let t = f.read_at(&mut tail, size - 512);
            say(&format!("MAGICREAD size={size} head_ok={} tail_ok={} qcow2_magic={} vhdx_magic={} vhd_footer={}", h.is_ok(), t.is_ok(),
                head[0..4] == [0x51, 0x46, 0x49, 0xfb], &head[0..8] == b"vhdxfile", &tail[0..8] == b"conectix"));
        }
        Some("bench") => bench(&p),
        Some("serial") => {
            say(&format!("SERIAL vdb={:?} vda={:?}", std::fs::read_to_string("/sys/block/vdb/serial").unwrap_or_default().trim(), std::fs::read_to_string("/sys/block/vda/serial").unwrap_or_default().trim()));
        }
        Some("idle") => { say("READY idle"); loop { std::thread::sleep(Duration::from_secs(3600)); } }
        other => say(&format!("unknown mode {other:?}")),
    }
    say(&format!("elapsed_ms={}", started.elapsed().as_millis()));
    unsafe { sync(); }
    finish(0);
}
