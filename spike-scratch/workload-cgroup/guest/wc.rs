//! workload-cgroup spike (V42) probe code. This file is the spike's OWN code; `gen_init.py` copies it,
//! together with a patched copy of the production `overdrive-init` sources, into a gitignored build
//! directory. Nothing here is production code.
//!
//! Mode comes from the kernel parameter `ovdwc=<mode>`:
//!   base        no change (the A2 baseline: same toolchain, same patched file set, no additions)
//!   cgonly      mkdir the workload cgroup and place the workload with CLONE_INTO_CGROUP; no listen programs
//!   child       cgroup + listen programs attached to the workload cgroup + workload placed by clone3
//!   child-nobtf as `child`, but the listen object is loaded without parsing kernel BTF
//!   outside     cgroup + listen programs attached to the workload cgroup, workload NOT placed in it
//!   rootearly   listen programs attached at the cgroup root BEFORE init's transport binds (naive control)
//!   rootlate    listen programs attached at the cgroup root where the production program loop ends
//!   nocorr      negative control for the correlation check: as base, but the root tcp_correlation sockops program is NOT attached
//!   sameobj     the guest object carries the listen programs; attached to the workload cgroup from the
//!               production Ebpf (no second Ebpf::load); workload placed by clone3
//!   sameobj-idle the same merged object, nothing attached, no cgroup (cost of the bigger object alone)
#![allow(unsafe_code, dead_code, unused_imports, clippy::all, clippy::pedantic, clippy::nursery)]

use std::ffi::CString;
use std::fs::{self, File};
use std::io;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use aya::maps::{PerCpuArray, RingBuf};
use aya::programs::{CgroupAttachMode, CgroupSock, SockOps};
use aya::{Ebpf, EbpfLoader};
use nix::libc;

#[repr(align(8))]
struct Aligned<const N: usize>([u8; N]);
static WC_OBJ: Aligned<{ include_bytes!(env!("WC_LISTEN_OBJ_PATH")).len() }> =
    Aligned(*include_bytes!(env!("WC_LISTEN_OBJ_PATH")));

const CGROUP_ROOT: &str = "/sys/fs/cgroup";
const WORKLOAD_CGROUP: &str = "/sys/fs/cgroup/workload";
const CGROUP2_MAGIC: libc::c_long = 0x6367_7270;
const SYS_CLONE3: libc::c_long = 435;
const CLONE_INTO_CGROUP: u64 = 0x2_0000_0000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Base,
    CgOnly,
    Child,
    ChildNoBtf,
    Outside,
    RootEarly,
    RootLate,
    SameObj,
    SameObjIdle,
    NoCorr,
}

static START: OnceLock<Instant> = OnceLock::new();
static MODE: OnceLock<(Mode, String)> = OnceLock::new();
static MARKS: Mutex<Vec<(&'static str, u64)>> = Mutex::new(Vec::new());
static EVENTS: Mutex<Vec<[u32; 12]>> = Mutex::new(Vec::new());
static STOP: AtomicBool = AtomicBool::new(false);
static HOLD: Mutex<Option<Ebpf>> = Mutex::new(None);
static C_TCP: Mutex<Option<PerCpuArray<aya::maps::MapData, u64>>> = Mutex::new(None);
static C_UDP: Mutex<Option<PerCpuArray<aya::maps::MapData, u64>>> = Mutex::new(None);
static READER: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static WL_CGROUP: OnceLock<File> = OnceLock::new();

fn mode() -> Mode {
    MODE.get_or_init(|| {
        let cmdline = fs::read_to_string("/proc/cmdline").unwrap_or_default();
        let name = cmdline
            .split_whitespace()
            .find_map(|t| t.strip_prefix("ovdwc="))
            .unwrap_or("base")
            .to_owned();
        let m = match name.as_str() {
            "base" => Mode::Base,
            "cgonly" => Mode::CgOnly,
            "child" => Mode::Child,
            "child-nobtf" => Mode::ChildNoBtf,
            "outside" => Mode::Outside,
            "rootearly" => Mode::RootEarly,
            "rootlate" => Mode::RootLate,
            "sameobj" => Mode::SameObj,
            "sameobj-idle" => Mode::SameObjIdle,
            "nocorr" => Mode::NoCorr,
            other => fatal(&format!("unknown ovdwc mode {other:?}")),
        };
        (m, name)
    })
    .0
}

fn fatal(msg: &str) -> ! {
    eprintln!("WC_ERR {msg}");
    std::process::exit(1)
}

pub fn mark(label: &'static str) {
    let ns = START.get().map_or(0, |s| s.elapsed().as_nanos() as u64);
    MARKS.lock().unwrap().push((label, ns));
}

pub fn start() {
    let _ = START.set(Instant::now());
    mark("start");
}

fn ensure_cgroup2() {
    fs::create_dir_all(CGROUP_ROOT).unwrap_or_else(|e| fatal(&format!("mkdir {CGROUP_ROOT}: {e}")));
    let path = CString::new(CGROUP_ROOT).unwrap();
    let mut info: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &raw mut info) } == 0 && info.f_type as libc::c_long == CGROUP2_MAGIC {
        return;
    }
    let (src, fs_) = (CString::new("none").unwrap(), CString::new("cgroup2").unwrap());
    let rc = unsafe { libc::mount(src.as_ptr(), path.as_ptr(), fs_.as_ptr(), 0, std::ptr::null()) };
    if rc < 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::EBUSY) {
            fatal(&format!("mount cgroup2: {e}"));
        }
    }
}

fn make_workload_cgroup() {
    ensure_cgroup2();
    if let Err(e) = fs::create_dir(WORKLOAD_CGROUP) {
        if e.kind() != io::ErrorKind::AlreadyExists {
            fatal(&format!("mkdir {WORKLOAD_CGROUP}: {e}"));
        }
    }
    let f = File::open(WORKLOAD_CGROUP).unwrap_or_else(|e| fatal(&format!("open {WORKLOAD_CGROUP}: {e}")));
    let _ = WL_CGROUP.set(f);
}

fn install_listen(cgroup: &File, nobtf: bool) {
    let mut loader = EbpfLoader::new();
    if nobtf {
        loader.btf(None);
    }
    let mut bpf = loader.load(&WC_OBJ.0).unwrap_or_else(|e| fatal(&format!("load listen object: {e}")));
    mark("wc_loaded");
    attach_from(&mut bpf, cgroup);
    *HOLD.lock().unwrap() = Some(bpf);
}

/// Loads the three listen programs out of `bpf` and attaches them to `cgroup`; takes the ring and the
/// counters out of `bpf` and starts the ring reader. The caller keeps `bpf` alive (it owns the links).
fn attach_from(bpf: &mut Ebpf, cgroup: &File) {
    {
        let prog: &mut SockOps = bpf
            .program_mut("wc_listen")
            .unwrap_or_else(|| fatal("no wc_listen"))
            .try_into()
            .unwrap_or_else(|e| fatal(&format!("wc_listen type: {e}")));
        prog.load().unwrap_or_else(|e| fatal(&format!("load wc_listen: {e}")));
        prog.attach(cgroup.try_clone().unwrap(), CgroupAttachMode::Single)
            .unwrap_or_else(|e| fatal(&format!("attach wc_listen: {e}")));
    }
    for name in ["wc_post_bind4", "wc_post_bind6"] {
        let prog: &mut CgroupSock = bpf
            .program_mut(name)
            .unwrap_or_else(|| fatal(name))
            .try_into()
            .unwrap_or_else(|e| fatal(&format!("{name} type: {e}")));
        prog.load().unwrap_or_else(|e| fatal(&format!("load {name}: {e}")));
        prog.attach(cgroup.try_clone().unwrap(), CgroupAttachMode::Single)
            .unwrap_or_else(|e| fatal(&format!("attach {name}: {e}")));
    }
    mark("wc_attached");
    let map = bpf.take_map("LISTEN_EVENTS").unwrap_or_else(|| fatal("no LISTEN_EVENTS"));
    let ring = RingBuf::try_from(map).unwrap_or_else(|e| fatal(&format!("ringbuf: {e}")));
    let ct = bpf.take_map("LISTEN_COUNTERS").unwrap_or_else(|| fatal("no LISTEN_COUNTERS"));
    *C_TCP.lock().unwrap() = Some(PerCpuArray::try_from(ct).unwrap_or_else(|e| fatal(&format!("counters: {e}"))));
    let cu = bpf.take_map("WC_UDP").unwrap_or_else(|| fatal("no WC_UDP"));
    *C_UDP.lock().unwrap() = Some(PerCpuArray::try_from(cu).unwrap_or_else(|e| fatal(&format!("udp counters: {e}"))));
    let h = std::thread::Builder::new()
        .name("wc-reader".into())
        .spawn(move || reader(ring))
        .unwrap_or_else(|e| fatal(&format!("reader thread: {e}")));
    *READER.lock().unwrap() = Some(h);
}

/// Called from the production `attach_program` once the last production program (SockRelease) is attached:
/// the same-object variants use the production `Ebpf` that already holds the listen programs.
pub fn attach_in_object(bpf: &mut Ebpf) {
    if mode() == Mode::SameObj {
        make_workload_cgroup();
        mark("wc_cgroup_made");
        let cg = WL_CGROUP.get().unwrap();
        attach_from(bpf, cg);
    }
}

fn drain(ring: &mut RingBuf<aya::maps::MapData>) {
    while let Some(item) = ring.next() {
        let b: &[u8] = &item;
        if b.len() < 48 {
            continue;
        }
        let mut ev = [0u32; 12];
        for (i, w) in ev.iter_mut().enumerate() {
            *w = u32::from_ne_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
        }
        EVENTS.lock().unwrap().push(ev);
    }
}

fn reader(mut ring: RingBuf<aya::maps::MapData>) {
    loop {
        let stop = STOP.load(Ordering::SeqCst);
        drain(&mut ring);
        if stop {
            break;
        }
        let mut pfd = libc::pollfd { fd: ring.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        unsafe { libc::poll(&raw mut pfd, 1, 3) };
    }
}

/// Called at the top of `configure_guest_transport`, before init's own TCP/UDP binds.
pub fn pre_binds() {
    mark("cfg_begin");
    if mode() == Mode::RootEarly {
        ensure_cgroup2();
        let root = File::open(CGROUP_ROOT).unwrap_or_else(|e| fatal(&format!("open root cgroup: {e}")));
        install_listen(&root, false);
    }
}

/// Called after the production program loop, before the forwarder module, still pre-READY.
pub fn post_programs() {
    mark("programs_done");
    match mode() {
        Mode::Base | Mode::RootEarly | Mode::SameObj | Mode::SameObjIdle | Mode::NoCorr => {}
        Mode::RootLate => {
            let root = File::open(CGROUP_ROOT).unwrap_or_else(|e| fatal(&format!("open root cgroup: {e}")));
            install_listen(&root, false);
        }
        Mode::CgOnly => {
            make_workload_cgroup();
            mark("wc_cgroup_made");
        }
        Mode::Child | Mode::Outside | Mode::ChildNoBtf => {
            make_workload_cgroup();
            mark("wc_cgroup_made");
            let cg = WL_CGROUP.get().unwrap();
            install_listen(cg, mode() == Mode::ChildNoBtf);
        }
    }
    mark("wc_done");
}

pub fn skip_correlation() -> bool {
    mode() == Mode::NoCorr
}

pub fn places_in_cgroup() -> bool {
    matches!(mode(), Mode::Child | Mode::ChildNoBtf | Mode::CgOnly | Mode::SameObj)
}

#[repr(C)]
struct CloneArgs {
    flags: u64,
    pidfd: u64,
    child_tid: u64,
    parent_tid: u64,
    exit_signal: u64,
    stack: u64,
    stack_size: u64,
    tls: u64,
    set_tid: u64,
    set_tid_size: u64,
    cgroup: u64,
}

/// clone3(CLONE_INTO_CGROUP): the child is created already inside the workload cgroup, so no
/// instruction of the workload (nor of the child between clone and execve) runs in any other cgroup.
pub fn spawn_in_workload_cgroup(argv: &[String]) -> io::Result<i32> {
    let cg = WL_CGROUP.get().ok_or_else(|| io::Error::other("no workload cgroup"))?;
    let c_argv: Vec<CString> = argv.iter().map(|a| CString::new(a.as_bytes()).unwrap()).collect();
    let mut p_argv: Vec<*const libc::c_char> = c_argv.iter().map(|c| c.as_ptr()).collect();
    p_argv.push(std::ptr::null());
    let c_env: Vec<CString> = std::env::vars_os()
        .map(|(k, v)| {
            let mut s = k.into_encoded_bytes();
            s.push(b'=');
            s.extend(v.into_encoded_bytes());
            CString::new(s).unwrap()
        })
        .collect();
    let mut p_env: Vec<*const libc::c_char> = c_env.iter().map(|c| c.as_ptr()).collect();
    p_env.push(std::ptr::null());
    let args = CloneArgs {
        flags: CLONE_INTO_CGROUP,
        pidfd: 0,
        child_tid: 0,
        parent_tid: 0,
        exit_signal: libc::SIGCHLD as u64,
        stack: 0,
        stack_size: 0,
        tls: 0,
        set_tid: 0,
        set_tid_size: 0,
        cgroup: cg.as_raw_fd() as u64,
    };
    let pid = unsafe { libc::syscall(SYS_CLONE3, &raw const args, std::mem::size_of::<CloneArgs>()) };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        unsafe {
            libc::setpgid(0, 0);
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
            libc::execve(c_argv[0].as_ptr(), p_argv.as_ptr(), p_env.as_ptr());
            libc::_exit(127);
        }
    }
    Ok(pid as i32)
}

pub fn ready_sent() {
    mark("ready_sent");
    let m = MARKS.lock().unwrap();
    let mut line = format!("WC_T mode={}", MODE.get().map_or("?", |x| x.1.as_str()));
    for (l, ns) in m.iter() {
        line.push_str(&format!(" {l}={ns}"));
    }
    drop(m);
    let cg = fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    eprintln!("{line}");
    eprintln!("WC_CG init={}", cg.trim());
}

fn dotted(a: u32) -> String {
    let b = a.to_ne_bytes();
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

/// After the workload has exited: stop the reader, drain, print the tally.
pub fn finish() {
    let m = mode();
    STOP.store(true, Ordering::SeqCst);
    if let Some(h) = READER.lock().unwrap().take() {
        let _ = h.join();
    }
    let ev = EVENTS.lock().unwrap();
    let (mut it, mut iu, mut io_, mut wt, mut wu, mut wo) = (0, 0, 0, 0, 0, 0);
    let mut init_udp_ports: Vec<u32> = Vec::new();
    let mut init_tcp_ports: Vec<u32> = Vec::new();
    for e in ev.iter() {
        let (kind, pid, port) = (e[0], e[3], e[2]);
        if pid == 1 {
            match kind {
                1 => {
                    it += 1;
                    init_tcp_ports.push(port);
                }
                2 => {
                    iu += 1;
                    init_udp_ports.push(port);
                }
                _ => io_ += 1,
            }
        } else {
            match kind {
                1 => wt += 1,
                2 => wu += 1,
                _ => wo += 1,
            }
            let k = match kind {
                1 => "tcp_listen",
                2 => "udp_bind",
                _ => "other",
            };
            eprintln!(
                "WC_EV kind={k} pid={pid} port={port} family={} addr4={} proto={}",
                e[1],
                dotted(e[4]),
                e[9]
            );
        }
    }
    let (mut seen, mut drops, mut useen, mut udrops) = (0u64, 0u64, 0u64, 0u64);
    if let Some(c) = C_TCP.lock().unwrap().as_ref() {
        seen = c.get(&0, 0).map(|v| v.iter().sum()).unwrap_or(0);
        drops = c.get(&1, 0).map(|v| v.iter().sum()).unwrap_or(0);
    }
    if let Some(c) = C_UDP.lock().unwrap().as_ref() {
        useen = c.get(&0, 0).map(|v| v.iter().sum()).unwrap_or(0);
        udrops = c.get(&1, 0).map(|v| v.iter().sum()).unwrap_or(0);
    }
    init_udp_ports.sort_unstable();
    init_tcp_ports.sort_unstable();
    eprintln!(
        "WC_SUM mode={} init_tcp={it} init_udp={iu} init_other={io_} wl_tcp={wt} wl_udp={wu} wl_other={wo} total={} tcp_seen={seen} tcp_drops={drops} udp_seen={useen} udp_drops={udrops} init_tcp_ports={:?} init_udp_range={}-{}",
        MODE.get().map_or("?", |x| x.1.as_str()),
        ev.len(),
        init_tcp_ports.iter().take(4).collect::<Vec<_>>(),
        init_udp_ports.first().copied().unwrap_or(0),
        init_udp_ports.last().copied().unwrap_or(0),
    );
    eprintln!("WC_END");
}
