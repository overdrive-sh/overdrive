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

pub const BUNDLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/lvm-bundle.bin"));
pub const INIT_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/nlpv-v1-init"));
pub const LVM_MANIFEST: &str = include_str!(concat!(env!("OUT_DIR"), "/lvm-manifest.txt"));
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
/// Unpack the LVM bundle (binary + libraries) under `root` (the inner guest's rootdir).
pub fn unpack_bundle(root: &str) {
    let mut b = BUNDLE;
    let mut n = 0;
    while !b.is_empty() {
        let pl = u32::from_le_bytes(b[0..4].try_into().unwrap()) as usize;
        let path = std::str::from_utf8(&b[4..4 + pl]).unwrap().to_owned();
        let mode = u32::from_le_bytes(b[4 + pl..8 + pl].try_into().unwrap());
        let dl = u64::from_le_bytes(b[8 + pl..16 + pl].try_into().unwrap()) as usize;
        let data = &b[16 + pl..16 + pl + dl];
        b = &b[16 + pl + dl..];
        let p = PathBuf::from(format!("{root}{path}"));
        if let Some(d) = p.parent() { fs::create_dir_all(d).unwrap(); }
        fs::write(&p, data).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(mode)).unwrap();
        n += 1;
    }
    ev(format!("lvm bundle unpacked under {root}: {n} files"));
}

/// Common outer-guest preparation: tools, modules, tmpfs workspace.
pub fn outer_setup() { outer_setup_for(540) }
pub fn outer_setup_for(secs: u64) {
    // Watchdog: no probe may hold the shared test machine for more than `secs`.
    std::thread::spawn(move || { std::thread::sleep(Duration::from_secs(secs)); eprintln!("NLPV-H WATCHDOG fired after {secs}s: aborting the probe"); std::process::abort(); });
    ev(format!("kernel release (build host == guest kernel): {KERNEL_RELEASE}"));
    fs::create_dir_all("/nlpv").unwrap();
    let _ = run(&["mount", "-t", "tmpfs", "-o", "size=3000m,mode=0755", "tmpfs", "/nlpv"]);
    fs::create_dir_all("/nlpv/run").unwrap();
    fs::create_dir_all("/nlpv/bin").unwrap();
    fs::set_permissions("/nlpv", fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions("/nlpv/run", fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions("/nlpv/bin", fs::Permissions::from_mode(0o755)).unwrap();
    sh("ids", "cat /proc/sys/kernel/osrelease; busybox stat -c '%n gid=%g mode=%a' /dev/kvm /dev/loop-control; busybox ls -l /dev/mapper/ 2>&1; busybox free -m 2>&1 | busybox head -3");
}

pub fn ch_binary(variant: &str) -> String {
    match variant {
        "stock" => CH_STOCK.to_owned(),
        other => other.to_owned(),
    }
}

// ---------------------------------------------------------------- inner rootfs
/// Builds the inner guest's root filesystem image with `nlpv-init` as /init.
pub fn build_inner_rootfs(dir: &str, size_mib: u64) -> String {
    let root = format!("{dir}/rootdir");
    let _ = fs::remove_dir_all(&root);
    for d in ["dev", "proc", "sys", "mnt", "tmp", "sbin", "run", "etc", "var", "usr/sbin"]  { fs::create_dir_all(format!("{root}/{d}")).unwrap(); }
    unpack_bundle(&root);
    if std::env::var("NLPV_BAKE_ETC_LVM").is_ok() { fs::create_dir_all(format!("{root}/etc/lvm")).unwrap(); ev("image variant: empty /etc/lvm baked into the read-only root"); }
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
    pub strace: Option<String>,
    pub ro_root: bool,
}
impl VmmSpec {
    pub fn new(name: &str, ch: &str, rootfs: &str) -> Self {
        VmmSpec { name: name.into(), ch: ch.into(), rootfs: rootfs.into(), disks: vec![], cmdline_params: vec![],
            memory_mib: 256, fsize: 0, nofile: 1024, cgroup_memory_max: None, confined: true, ch_extra: vec![], vcpus: 1, strace: None, ro_root: false }
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
    let mut cmdline = "console=ttyS0 panic=1 root=/dev/vda {} init=/nlpv-init".replace("{}", if spec.ro_root { "ro" } else { "rw" });
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

pub mod gpt;
pub fn fnv(data: &[u8]) -> u64 { let mut h = 0xcbf29ce484222325u64; for b in data { h ^= *b as u64; h = h.wrapping_mul(0x100000001b3); } h }
