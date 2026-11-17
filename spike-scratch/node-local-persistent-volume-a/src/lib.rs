//! Helpers shared by the probes. Throwaway probe code (spike-scratch).
//!
//! Layout inside the OUTER guest (`cgroup-default`/`nested-launch`): a tmpfs at
//! /nlpv holds backing files, loop devices carry them, dm targets sit on the
//! loop devices, and a confined nested Cloud Hypervisor opens the dm device (or
//! a file) as its data disk. The INNER guest runs `nlpv-init` (guest/nlpv_init.rs).
#![allow(clippy::all, dead_code)]
use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub const BUNDLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bundle.bin"));
pub const INIT_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/nlpv-init"));
pub const CH_SPIKE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ch-spike"));
pub const KERNEL_RELEASE: &str = env!("NLPV_KERNEL_RELEASE");
pub const CH_STOCK: &str = "/usr/local/bin/cloud-hypervisor";
pub const VMM_UID: u32 = 4200;

/// Wall-clock-stamped evidence line on stderr (append-only run log).
pub fn ev(msg: impl AsRef<str>) {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
    eprintln!("NLPV-H {}.{:03} {}", t.as_secs(), t.subsec_millis(), msg.as_ref());
}

/// Run a shell command and print the whole exchange as one evidence block.
pub fn sh(label: &str, script: &str) -> (i32, String) {
    let out = Command::new("sh").arg("-c").arg(script).output();
    match out {
        Ok(o) => {
            let so = String::from_utf8_lossy(&o.stdout).into_owned();
            let se = String::from_utf8_lossy(&o.stderr).into_owned();
            let code = o.status.code().unwrap_or(-1);
            eprintln!("NLPV[{label}] $ {script}\n--stdout--\n{so}--stderr--\n{se}--exit={code}--");
            (code, so)
        }
        Err(e) => { eprintln!("NLPV[{label}] $ {script}\nspawn failed: {e}"); (-1, String::new()) }
    }
}

/// Run argv, return (exit, stdout+stderr). Printed as evidence unless quiet.
pub fn run(argv: &[&str]) -> (i32, String) {
    let out = Command::new(argv[0]).args(&argv[1..]).output();
    match out {
        Ok(o) => {
            let s = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            let code = o.status.code().unwrap_or(-1);
            ev(format!("$ {} -> exit {code}{}", argv.join(" "), if s.trim().is_empty() { String::new() } else { format!("\n{}", s.trim_end()) }));
            (code, s)
        }
        Err(e) => { ev(format!("$ {} -> spawn failed: {e}", argv.join(" "))); (-1, String::new()) }
    }
}

pub fn must(argv: &[&str]) -> String {
    let (c, s) = run(argv);
    assert!(c == 0, "{} failed ({c}): {s}", argv.join(" "));
    s
}

// ---------------------------------------------------------------- bundle
pub fn install_bundle() {
    let mut b = BUNDLE;
    let mut n = 0;
    while !b.is_empty() {
        let pl = u32::from_le_bytes(b[0..4].try_into().unwrap()) as usize;
        let path = std::str::from_utf8(&b[4..4 + pl]).unwrap().to_owned();
        let mode = u32::from_le_bytes(b[4 + pl..8 + pl].try_into().unwrap());
        let dl = u64::from_le_bytes(b[8 + pl..16 + pl].try_into().unwrap()) as usize;
        let data = &b[16 + pl..16 + pl + dl];
        b = &b[16 + pl + dl..];
        let p = Path::new(&path);
        if p.exists() && !path.starts_with("/nlpv-modules") { continue; }
        if let Some(d) = p.parent() { fs::create_dir_all(d).unwrap(); }
        fs::write(p, data).unwrap();
        fs::set_permissions(p, fs::Permissions::from_mode(mode)).unwrap();
        n += 1;
    }
    ev(format!("bundle installed: {n} files"));
}

pub fn insmod(path: &str) { insmod_params(path, "") }
pub fn insmod_params(path: &str, p: &str) {
    let data = fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let params = CString::new(p).unwrap();
    let r = unsafe { libc::syscall(libc::SYS_init_module, data.as_ptr(), data.len(), params.as_ptr()) };
    let err = std::io::Error::last_os_error();
    ev(format!("init_module {path}: ret={r} {}", if r != 0 { err.to_string() } else { "ok".into() }));
    assert!(r == 0 || err.raw_os_error() == Some(libc::EEXIST), "insmod {path}: {err}");
}

/// Common outer-guest preparation: tools, modules, tmpfs workspace.
pub fn outer_setup() { outer_setup_for(540) }
pub fn outer_setup_for(secs: u64) {
    // Watchdog: no probe may hold the shared test machine for more than `secs`.
    std::thread::spawn(move || { std::thread::sleep(Duration::from_secs(secs)); eprintln!("NLPV-H WATCHDOG fired after {secs}s: aborting the probe"); std::process::abort(); });
    ev(format!("kernel release (build host == guest kernel): {KERNEL_RELEASE}"));
    install_bundle();
    insmod("/nlpv-modules/dm-log-writes.ko");
    insmod("/nlpv-modules/dm-flakey.ko");
    insmod("/nlpv-modules/dm-delay.ko");
    fs::create_dir_all("/nlpv").unwrap();
    let _ = run(&["mount", "-t", "tmpfs", "-o", "size=3000m,mode=0755", "tmpfs", "/nlpv"]);
    fs::create_dir_all("/nlpv/run").unwrap();
    fs::create_dir_all("/nlpv/bin").unwrap();
    fs::set_permissions("/nlpv", fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions("/nlpv/run", fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions("/nlpv/bin", fs::Permissions::from_mode(0o755)).unwrap();
    if !CH_SPIKE.is_empty() {
        fs::write("/nlpv/bin/ch-spike", CH_SPIKE).unwrap();
        fs::set_permissions("/nlpv/bin/ch-spike", fs::Permissions::from_mode(0o755)).unwrap();
    }
    sh("ids", "cat /proc/sys/kernel/osrelease; busybox stat -c '%n gid=%g mode=%a' /dev/kvm /dev/loop-control; busybox ls -l /dev/mapper/ 2>&1; busybox free -m 2>&1 | busybox head -3");
}

pub fn ch_binary(variant: &str) -> String {
    match variant {
        "stock" => CH_STOCK.to_owned(),
        "spike" => { assert!(!CH_SPIKE.is_empty(), "spike CH not built/embedded"); "/nlpv/bin/ch-spike".to_owned() }
        other => other.to_owned(),
    }
}

// ---------------------------------------------------------------- devices
pub fn sparse(path: &str, bytes: u64) {
    let f = fs::File::create(path).unwrap();
    f.set_len(bytes).unwrap();
}

pub fn loop_attach(file: &str, direct_io: bool) -> String {
    let mut args = vec!["losetup", "--find", "--show"];
    if direct_io { args.push("--direct-io=on"); }
    args.push(file);
    must(&args).trim().to_owned()
}

pub fn loop_detach(dev: &str) { let _ = run(&["losetup", "-d", dev]); }

pub fn blk_sectors(dev: &str) -> u64 {
    let n = dev.trim_start_matches("/dev/");
    let p = if dev.contains("/mapper/") {
        // resolve dm-N
        let md = fs::metadata(dev).unwrap();
        let rdev = md.rdev();
        let (maj, min) = (libc_major(rdev), libc_minor(rdev));
        format!("/sys/dev/block/{maj}:{min}/size")
    } else { format!("/sys/class/block/{n}/size") };
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}")).trim().parse().unwrap()
}
pub fn libc_major(d: u64) -> u64 { ((d >> 8) & 0xfff) | ((d >> 32) & !0xfff) }
pub fn libc_minor(d: u64) -> u64 { (d & 0xff) | ((d >> 12) & !0xff) }

pub const DMSETUP: &str = "/usr/bin/dmsetup";

pub fn dm_create(name: &str, table: &str) -> String {
    ev(format!("dmsetup create {name}: {table}"));
    let mut c = Command::new(DMSETUP)
        .args(["create", name, "--noudevsync", "--noudevrules", "--uid", "4200", "--gid", "4200", "--mode", "0660"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(format!("{table}\n").as_bytes()).unwrap();
    let o = c.wait_with_output().unwrap();
    assert!(o.status.success(), "dmsetup create: {}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let path = format!("/dev/mapper/{name}");
    // No udev here: libdm leaves only the kernel's /dev/dm-N (0600 root). Create the
    // /dev/mapper node the way udev would, owned by the confined VMM uid.
    let info = must(&[DMSETUP, "info", "-c", "--noheadings", "-o", "major,minor", name]);
    let mm: Vec<&str> = info.trim().split(':').collect();
    let _ = fs::remove_file(&path);
    must(&["mknod", &path, "b", mm[0].trim(), mm[1].trim()]);
    chown(&path, VMM_UID, VMM_UID);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o660)).unwrap();
    path
}

pub fn dm_reload(name: &str, table: &str) {
    ev(format!("dmsetup reload {name}: {table}"));
    let mut c = Command::new(DMSETUP).args(["reload", name, "--noudevsync", "--noudevrules"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(format!("{table}\n").as_bytes()).unwrap();
    let o = c.wait_with_output().unwrap();
    assert!(o.status.success(), "dmsetup reload: {}", String::from_utf8_lossy(&o.stderr));
    must(&[DMSETUP, "resume", name, "--noudevsync", "--noudevrules"]);
}

pub fn dm_remove(name: &str) { let _ = run(&[DMSETUP, "remove", "--force", name, "--noudevsync", "--noudevrules"]); }
pub fn dm_mark(name: &str, mark: &str) {
    let o = Command::new(DMSETUP).args(["message", name, "0", &format!("mark {mark}")]).output().unwrap();
    if !o.status.success() { ev(format!("mark {mark} FAILED: {}", String::from_utf8_lossy(&o.stderr))); }
}

// ---------------------------------------------------------------- inner rootfs
/// Builds the inner guest's root filesystem image with `nlpv-init` as /init.
pub fn build_inner_rootfs(dir: &str, size_mib: u64) -> String {
    let root = format!("{dir}/rootdir");
    let _ = fs::remove_dir_all(&root);
    for d in ["dev", "proc", "sys", "mnt", "tmp", "sbin"] { fs::create_dir_all(format!("{root}/{d}")).unwrap(); }
    fs::write(format!("{root}/nlpv-init"), INIT_BIN).unwrap();
    fs::set_permissions(format!("{root}/nlpv-init"), fs::Permissions::from_mode(0o755)).unwrap();
    let _ = run(&["mknod", &format!("{root}/dev/console"), "c", "5", "1"]);
    let img = format!("{dir}/rootfs.ext4");
    let _ = fs::remove_file(&img);
    must(&["mkfs.ext4", "-q", "-F", "-d", &root, &img, &format!("{size_mib}M")]);
    img
}

// ---------------------------------------------------------------- VMM launch (confined as production does)
pub struct VmmSpec {
    pub name: String,
    pub ch: String,
    pub rootfs: String,
    pub disks: Vec<String>,        // full `--disk` values for data disks
    pub cmdline_params: Vec<String>, // nlpv.* tokens
    pub memory_mib: u64,
    pub fsize: u64,                 // RLIMIT_FSIZE the wrapper sets
    pub nofile: u64,
    pub cgroup_memory_max: Option<u64>,
    pub confined: bool,
    pub ch_extra: Vec<String>,
    pub vcpus: u32,
    pub strace: Option<String>, // strace -e trace= list; output in run dir
}
impl VmmSpec {
    pub fn new(name: &str, ch: &str, rootfs: &str) -> Self {
        VmmSpec { name: name.into(), ch: ch.into(), rootfs: rootfs.into(), disks: vec![], cmdline_params: vec![],
            memory_mib: 256, fsize: 0, nofile: 1024, cgroup_memory_max: None, confined: true, ch_extra: vec![], vcpus: 1, strace: None }
    }
}
pub struct Vmm {
    pub child: Child,
    pub run_dir: String,
    pub console: String,
    pub api: String,
    pub name: String,
    pub cgroup: Option<String>,
    pub spawned: Instant,
}

pub fn kernel_path() -> String { format!("/boot/vmlinuz-{KERNEL_RELEASE}") }

pub fn chown(path: &str, uid: u32, gid: u32) {
    let c = CString::new(path).unwrap();
    let r = unsafe { libc::chown(c.as_ptr(), uid, gid) };
    assert!(r == 0, "chown {path}: {}", std::io::Error::last_os_error());
}

pub fn launch(spec: &VmmSpec) -> Vmm {
    let run_dir = format!("/nlpv/run/{}", spec.name);
    let _ = fs::remove_dir_all(&run_dir);
    fs::create_dir_all(&run_dir).unwrap();
    let kcopy = format!("{run_dir}/kernel");
    fs::copy(kernel_path(), &kcopy).unwrap();
    let console = format!("{run_dir}/console.log");
    let api = format!("{run_dir}/api.sock");
    fs::File::create(&console).unwrap();
    // As production does: the rootfs clone, the kernel copy and the run dir belong to the confined uid.
    chown(&spec.rootfs, VMM_UID, VMM_UID);
    for e in fs::read_dir(&run_dir).unwrap() { chown(e.unwrap().path().to_str().unwrap(), VMM_UID, VMM_UID); }
    chown(&run_dir, VMM_UID, VMM_UID);
    let mut cmdline = "console=ttyS0 panic=1 root=/dev/vda rw init=/nlpv-init".to_string();
    for p in &spec.cmdline_params { cmdline.push(' '); cmdline.push_str(p); }

    let kvm_gid = fs::metadata("/dev/kvm").map(|m| m.gid()).unwrap_or(0);
    let cgroup = spec.cgroup_memory_max.map(|m| {
        let cg = format!("/sys/fs/cgroup/overdrive.slice/workloads.slice/nlpv-{}.scope", spec.name);
        let _ = fs::remove_dir(&cg);
        fs::create_dir_all(&cg).unwrap();
        fs::write(format!("{cg}/memory.max"), m.to_string()).unwrap();
        cg
    });

    let mut ch_args: Vec<String> = vec![
        spec.ch.clone(), "--cpus".into(), format!("boot={}", spec.vcpus),
        "--memory".into(), format!("size={}M", spec.memory_mib),
        "--kernel".into(), kcopy, "--cmdline".into(), cmdline,
        "--disk".into(), format!("path={},image_type=raw", spec.rootfs),
    ];
    for d in &spec.disks { ch_args.push("--disk".into()); ch_args.push(d.clone()); }
    ch_args.extend(spec.ch_extra.iter().cloned());
    ch_args.extend(["--serial".into(), format!("file={console}"), "--console".into(), "off".into(),
        "--api-socket".into(), api.clone(), "--seccomp".into(), "true".into(), "--landlock".into(),
        "--landlock-rules".into(), format!("path={run_dir},access=rw")]);

    let mut full: Vec<String> = vec![];
    if spec.confined {
        let fsize = if spec.fsize == 0 { 1u64 << 40 } else { spec.fsize };
        full.extend(["prlimit".into(), format!("--fsize={fsize}"), format!("--nofile={}", spec.nofile), "--".into(),
            "setpriv".into(), format!("--reuid={VMM_UID}"), format!("--regid={VMM_UID}"),
            format!("--groups={kvm_gid}"), "--no-new-privs".into(), "--".into()]);
    }
    full.extend(ch_args);
    let quoted: Vec<String> = full.iter().map(|a| format!("'{}'", a.replace('\'', "'\\''"))).collect();
    let script = match &cgroup {
        Some(cg) => format!("echo $$ > {cg}/cgroup.procs && exec {}", quoted.join(" ")),
        None => format!("exec {}", quoted.join(" ")),
    };
    ev(format!("launch {}: {}", spec.name, quoted.join(" ")));
    let stderr = fs::File::create(format!("{run_dir}/ch.stderr")).unwrap();
    let child = Command::new("sh").arg("-c").arg(&script)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(stderr).spawn().unwrap();
    ev(format!("vmm pid {}", child.id()));
    if let Some(trace) = &spec.strace {
        let out = format!("{run_dir}/strace.out");
        let _ = Command::new("/usr/bin/strace").args(["-f", "-qq", "-ttt", "-e", &format!("trace={trace}"), "-o", &out, "-p", &child.id().to_string()])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|c| { std::mem::forget(c); });
    }
    Vmm { child, run_dir, console, api, name: spec.name.clone(), cgroup, spawned: Instant::now() }
}

impl Vmm {
    pub fn pid(&self) -> u32 { self.child.id() }
    pub fn console_text(&self) -> String { fs::read_to_string(&self.console).unwrap_or_default() }
    pub fn stderr_text(&self) -> String { fs::read_to_string(format!("{}/ch.stderr", self.run_dir)).unwrap_or_default() }
    pub fn kill9(&mut self) { unsafe { libc::kill(self.child.id() as i32, libc::SIGKILL) }; }
    pub fn exited(&mut self) -> Option<std::process::ExitStatus> { self.child.try_wait().unwrap() }
    pub fn wait_exit(&mut self, bound: Duration) -> Option<std::process::ExitStatus> {
        let t = Instant::now();
        while t.elapsed() < bound { if let Some(s) = self.exited() { return Some(s); } std::thread::sleep(Duration::from_millis(20)); }
        None
    }
    /// Stream guest console lines (those starting `NLPV-G `) to `f` until the VMM exits or `bound`.
    pub fn follow<F: FnMut(&str, &mut Vmm)>(&mut self, bound: Duration, mut f: F) {
        self.follow_until(bound, |l, v| { f(l, v); false })
    }
    /// Like `follow`, but the callback returns true to stop early.
    pub fn follow_until<F: FnMut(&str, &mut Vmm) -> bool>(&mut self, bound: Duration, mut f: F) {
        let t = Instant::now();
        let mut pos = 0usize;
        let mut carry = String::new();
        loop {
            let data = fs::read(&self.console).unwrap_or_default();
            if data.len() > pos {
                carry.push_str(&String::from_utf8_lossy(&data[pos..]));
                pos = data.len();
                while let Some(i) = carry.find('\n') {
                    let line = carry[..i].trim_end_matches('\r').to_owned();
                    carry = carry[i + 1..].to_owned();
                    if let Some(g) = line.split("NLPV-G ").nth(1) { if f(g, self) { return; } }
                }
            }
            let exited = self.exited().is_some();
            if exited || t.elapsed() > bound {
                // drain once more
                let data = fs::read(&self.console).unwrap_or_default();
                if data.len() > pos {
                    carry.push_str(&String::from_utf8_lossy(&data[pos..]));
                    for line in carry.lines() { if let Some(g) = line.trim_end_matches('\r').split("NLPV-G ").nth(1) { if f(g, self) { return; } } }
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    /// PUT to the VMM API (HTTP/1.1 over its unix socket).
    pub fn api_put(&self, endpoint: &str, body: &str) -> String {
        let mut s = UnixStream::connect(&self.api).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(8))).unwrap(); s.set_write_timeout(Some(Duration::from_secs(8))).unwrap();
        let req = format!("PUT /api/v1/{endpoint} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        s.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        let _ = s.read_to_string(&mut resp);
        resp
    }
    pub fn api_get(&self, endpoint: &str) -> String {
        let mut s = UnixStream::connect(&self.api).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(8))).unwrap(); s.set_write_timeout(Some(Duration::from_secs(8))).unwrap();
        let req = format!("GET /api/v1/{endpoint} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        let _ = s.read_to_string(&mut resp);
        resp
    }
    pub fn cleanup(&mut self) {
        self.kill9();
        let _ = self.child.wait();
        if let Some(cg) = &self.cgroup { let _ = fs::remove_dir(cg); }
    }
}

/// Open file descriptors of `pid` that point at `target`.
pub fn fds_on(pid: u32, target: &str) -> usize {
    let mut n = 0;
    if let Ok(rd) = fs::read_dir(format!("/proc/{pid}/fd")) {
        for e in rd.flatten() { if let Ok(t) = fs::read_link(e.path()) { if t.to_string_lossy() == target { n += 1; } } }
    }
    n
}

/// The pid of the cloud-hypervisor process: `sh -c exec ...` execs in place, so it is `child.id()`.
pub fn proc_limit(pid: u32, name: &str) -> String {
    fs::read_to_string(format!("/proc/{pid}/limits")).unwrap_or_default().lines().find(|l| l.starts_with(name)).unwrap_or("").to_owned()
}
pub fn proc_status(pid: u32, field: &str) -> String {
    fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default().lines()
        .find(|l| l.starts_with(field)).unwrap_or("").to_owned()
}

// ---------------------------------------------------------------- guest payloads (must match guest/nlpv_init.rs)
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
pub fn fnv(data: &[u8]) -> u64 { let mut h = 0xcbf29ce484222325u64; for b in data { h ^= *b as u64; h = h.wrapping_mul(0x100000001b3); } h }
pub fn expected_hash(i: u64, nonce: u64, bs: usize) -> u64 { let mut b = vec![0u8; bs]; fill_block(&mut b, i, nonce); fnv(&b) }

// ---------------------------------------------------------------- dm-log-writes log parser
pub const LOG_FLUSH: u64 = 1; pub const LOG_FUA: u64 = 2; pub const LOG_DISCARD: u64 = 4; pub const LOG_MARK: u64 = 8;
#[derive(Debug, Clone)]
pub struct LogEntry { pub idx: usize, pub sector: u64, pub nr_sectors: u64, pub flags: u64, pub data_off: u64, pub mark: Option<String> }
pub struct Log { pub sectorsize: u64, pub entries: Vec<LogEntry>, pub path: String, pub nr_entries_super: u64 }
impl Log {
    pub fn open(path: &str) -> Log {
        let data = fs::read(path).unwrap();
        let magic = u64::from_le_bytes(data[0..8].try_into().unwrap());
        assert_eq!(magic, 0x6a736677736872, "bad log-writes magic");
        let nr = u64::from_le_bytes(data[16..24].try_into().unwrap());
        let ss = u32::from_le_bytes(data[24..28].try_into().unwrap()) as u64;
        let mut off = ss; // super occupies the first sector
        let mut entries = vec![];
        for idx in 0..nr as usize {
            if off + 32 > data.len() as u64 { break; }
            let o = off as usize;
            let sector = u64::from_le_bytes(data[o..o + 8].try_into().unwrap());
            let nrs = u64::from_le_bytes(data[o + 8..o + 16].try_into().unwrap());
            let flags = u64::from_le_bytes(data[o + 16..o + 24].try_into().unwrap());
            let dlen = u64::from_le_bytes(data[o + 24..o + 32].try_into().unwrap());
            let mut mark = None;
            if flags & LOG_MARK != 0 { mark = Some(String::from_utf8_lossy(&data[o + 32..o + 32 + dlen as usize]).into_owned()); }
            let data_off = off + ss;
            entries.push(LogEntry { idx, sector, nr_sectors: nrs, flags, data_off, mark });
            off += ss;
            if flags & LOG_MARK == 0 && flags & LOG_DISCARD == 0 { off += nrs * ss; }
        }
        Log { sectorsize: ss, entries, path: path.to_owned(), nr_entries_super: nr }
    }
    pub fn read_data(&self, e: &LogEntry) -> Vec<u8> {
        use std::os::unix::fs::FileExt;
        let f = fs::File::open(&self.path).unwrap();
        let mut b = vec![0u8; (e.nr_sectors * self.sectorsize) as usize];
        f.read_exact_at(&mut b, e.data_off).unwrap();
        b
    }
    pub fn summary(&self) -> String {
        let (mut w, mut f, mut m, mut d) = (0, 0, 0, 0);
        for e in &self.entries { if e.flags & LOG_MARK != 0 { m += 1 } else if e.flags & LOG_FLUSH != 0 { f += 1 } else if e.flags & LOG_DISCARD != 0 { d += 1 } else { w += 1 } }
        format!("entries={} writes={w} flushes={f} marks={m} discards={d} sectorsize={}", self.entries.len(), self.sectorsize)
    }
    /// Reconstruct the device image after applying entries[..end] (writes only).
    pub fn replay_to(&self, end: usize, size_bytes: u64, out_path: &str) {
        use std::os::unix::fs::FileExt;
        let out = fs::File::create(out_path).unwrap();
        out.set_len(size_bytes).unwrap();
        for e in &self.entries[..end] {
            if e.flags & (LOG_MARK | LOG_DISCARD) != 0 || e.nr_sectors == 0 { continue; }
            out.write_all_at(&self.read_data(e), e.sector * self.sectorsize).unwrap();
        }
    }
}

/// The durability check for raw-block cells. For every `ACK<i>` mark: the last flush entry preceding it
/// must be preceded by a write of block i whose content is the expected payload.
pub struct AckCheck { pub acked: usize, pub violations: Vec<String>, pub missing_marks: usize }
pub fn check_acks(log: &Log, acked: &[u64], bs: usize, nonce: u64, dev_bs: u64) -> AckCheck {
    let mut present: BTreeMap<u64, u64> = BTreeMap::new(); // block -> content hash (latest write before cursor)
    let mut committed_at_flush: Vec<(usize, BTreeMap<u64, u64>)> = vec![];
    let mut mark_idx: BTreeMap<u64, usize> = BTreeMap::new();
    for e in &log.entries {
        if let Some(m) = &e.mark { if let Some(n) = m.strip_prefix("ACK") { if let Ok(n) = n.parse::<u64>() { mark_idx.insert(n, e.idx); } } }
        if e.flags & LOG_MARK != 0 || e.flags & LOG_DISCARD != 0 { continue; }
        if e.nr_sectors > 0 {
            let d = log.read_data(e);
            let off = e.sector * log.sectorsize;
            // split into bs-sized guest blocks
            let mut o = 0usize;
            while o + bs <= d.len() {
                let blk = (off + o as u64) / bs as u64;
                present.insert(blk, fnv(&d[o..o + bs]));
                o += bs;
            }
        }
        if e.flags & (LOG_FLUSH | LOG_FUA) != 0 { committed_at_flush.push((e.idx, present.clone())); }
    }
    let _ = dev_bs;
    let mut v = vec![]; let mut missing = 0;
    for &i in acked {
        let Some(&mi) = mark_idx.get(&i) else { missing += 1; v.push(format!("ACK{i}: no mark in log (mark lost or device log truncated)")); continue; };
        let state = committed_at_flush.iter().rev().find(|(fi, _)| *fi < mi).map(|(_, s)| s);
        let want = expected_hash(i, nonce, bs);
        match state.and_then(|s| s.get(&i)) {
            Some(h) if *h == want => {}
            Some(_) => v.push(format!("ACK{i}: block {i} present before the last flush but with WRONG content")),
            None => v.push(format!("ACK{i}: block {i} NOT in the log before the last flush preceding its mark (mark entry {mi})")),
        }
    }
    AckCheck { acked: acked.len(), violations: v, missing_marks: missing }
}

pub mod probes_a;


// ---------------------------------------------------------------- locks
/// Query the OFD lock state a *write lock over the whole file* would conflict with.
/// Returns None if no conflicting lock, else Some(description).
pub fn ofd_getlk(path: &str) -> Option<String> {
    use std::os::unix::io::AsRawFd;
    let f = fs::OpenOptions::new().read(true).open(path).ok()?;
    let mut fl: libc::flock = unsafe { std::mem::zeroed() };
    fl.l_type = libc::F_WRLCK as i16;
    fl.l_whence = libc::SEEK_SET as i16;
    fl.l_start = 0;
    fl.l_len = 0;
    fl.l_pid = 0;
    let r = unsafe { libc::fcntl(f.as_raw_fd(), libc::F_OFD_GETLK, &mut fl) };
    if r != 0 { return Some(format!("getlk error {}", std::io::Error::last_os_error())); }
    if fl.l_type == libc::F_UNLCK as i16 { None } else {
        Some(format!("type={} start={} len={} pid={}", if fl.l_type == libc::F_WRLCK as i16 { "WRITE" } else { "READ" }, fl.l_start, fl.l_len, fl.l_pid))
    }
}

pub fn now_us() -> u128 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() }

pub fn mknod_blk(path: &str, like: &str) {
    let md = fs::metadata(like).unwrap();
    let r = md.rdev();
    let _ = fs::remove_file(path);
    must(&["mknod", path, "b", &libc_major(r).to_string(), &libc_minor(r).to_string()]);
    chown(path, VMM_UID, VMM_UID);
    fs::set_permissions(path, fs::Permissions::from_mode(0o660)).unwrap();
}

pub fn tail_ch(vmm: &Vmm) -> String {
    let t = vmm.stderr_text();
    t.lines().filter(|l| !l.trim().is_empty()).take(12).collect::<Vec<_>>().join("\n")
}

/// A ready outer rig: a data volume (file on tmpfs, or dm-linear block device) the VMM can open.
pub struct Volume { pub path: String, pub is_bdev: bool, pub loopdev: Option<String>, pub dm: Option<String>, pub bytes: u64 }
pub fn make_volume(name: &str, bdev: bool, mib: u64) -> Volume {
    let f = format!("/nlpv/{name}.vol");
    sparse(&f, mib << 20);
    if !bdev {
        chown(&f, VMM_UID, VMM_UID);
        return Volume { path: f, is_bdev: false, loopdev: None, dm: None, bytes: mib << 20 };
    }
    let l = loop_attach(&f, false);
    let sectors = blk_sectors(&l);
    static DMSEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dm = format!("nlpv{}", DMSEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let dev = dm_create(&dm, &format!("0 {sectors} linear {l} 0"));
    Volume { path: dev, is_bdev: true, loopdev: Some(l), dm: Some(dm), bytes: mib << 20 }
}
pub fn idle_vmm(name: &str, ch: &str, disk_args: &[String]) -> Vmm {
    let rootdir = format!("/nlpv/run/{name}-img"); fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(name, &ch_binary(ch), &rootfs);
    spec.disks = disk_args.to_vec();
    spec.cmdline_params = vec!["nlpv.mode=idle".into()];
    launch(&spec)
}
pub fn wait_for_line(v: &mut Vmm, needle: &str, bound: Duration) -> bool {
    let mut seen = false;
    v.follow_until(bound, |l, _| { if l.starts_with(needle) { seen = true; } seen });
    seen
}
pub mod probes_b;

// ---------------------------------------------------------------- xfs helper
/// mkfs.xfs on a fresh loop device over a tmpfs file, mounted at /nlpv/xfs-<name>.
pub fn make_xfs(name: &str, mib: u64, reflink: bool) -> String {
    let f = format!("/nlpv/{name}.xfs.img");
    sparse(&f, mib << 20);
    let l = loop_attach(&f, false);
    must(&["mkfs.xfs", "-q", "-f", "-m", &format!("reflink={}", reflink as u8), &l]);
    let m = format!("/nlpv/xfs-{name}");
    fs::create_dir_all(&m).unwrap();
    must(&["mount", "-t", "xfs", "-o", "noatime", &l, &m]);
    m
}
pub fn st_blocks_bytes(path: &str) -> u64 { fs::metadata(path).map(|m| m.blocks() * 512).unwrap_or(0) }
pub fn read_cg(cg: &str, file: &str) -> String { fs::read_to_string(format!("{cg}/{file}")).unwrap_or_default() }
