//! Family `cgroup-connect-hook` (DDD "Connect-hook family"): the per-connect cost of
//! the gateway connect program, against nothing attached on the cgroup.
//!
//! Each replicate builds a fresh cgroup, bpffs pin directory and veth pair, loads the
//! dataplane (whose boot attaches the gateway connect program), optionally detaches that
//! program's cgroup link through the kernel's link commands (arm `no-hook`), then drives a
//! loopback connect-and-close loop from inside the cgroup.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;

use aya::maps::loaded_maps;
use aya::programs::loaded_programs;
use overdrive_bench::{Replicate, ReplicateFailure};
use overdrive_dataplane::EbpfDataplane;
use overdrive_testing_macros::overdrive_bench;

const WARMUP_CONNECTS: u32 = 500;
const TIMED_CONNECTS: u32 = 5_000;
/// The kernel truncates the gateway connect program's name to this (`BPF_OBJ_NAME_LEN` - 1).
const CONNECT_PROGRAM_NAME: &str = "cgroup_connect4";
/// `GATEWAY_CONNECT_INTENT_MAP`, which only the gateway connect program references, as the
/// kernel stores its name (truncated to `BPF_OBJ_NAME_LEN` - 1).
const GATEWAY_INTENT_MAP_NAME: &str = "GATEWAY_CONNECT";
const CLIENT_IFACE: &str = "ovbch0";
const BACKEND_IFACE: &str = "ovbcp0";
const NO_HOOK: &str = "no-hook";
const NO_HOOK_REASON: &str = "no connect program is attached on this arm";

static FIXTURE_SEQUENCE: AtomicU32 = AtomicU32::new(0);

#[overdrive_bench(
    family = "cgroup-connect-hook",
    substrate = native,
    arms = ["no-hook", "gateway-hook"],
    replicates = 30,
    bpf_stats,
    metric(name = "hook_ns_per_connect", unit = ns, table = "hook-time"),
    metric(name = "wall_ns_per_connect", unit = ns, table = "wall-time"),
    reference(
        metric = "hook_ns_per_connect",
        value = 137.6249,
        label = "spike evidence, different run, gateway-hook median",
        source = "spike-scratch/pig-connect-cost/results/cgroup-connect-hook/112c3104a22101479a180f5c/report.md"
    ),
    reference(
        metric = "wall_ns_per_connect",
        value = 185.6735,
        label = "spike evidence, different run, gateway-hook minus no-hook delta",
        source = "spike-scratch/pig-connect-cost/results/cgroup-connect-hook/112c3104a22101479a180f5c/report.md"
    ),
    smoke(guest = "cgroup-default"),
)]
fn connect_hook_cost(rep: &mut Replicate) -> Result<(), ReplicateFailure> {
    let fixture = Fixture::build(rep)?;
    let gateway = fixture.gateway_program()?;
    let no_hook = rep.arm() == NO_HOOK;
    if no_hook {
        let link = find_cgroup_link(gateway)?
            .ok_or_else(|| fail("the gateway program has no cgroup link after load"))?;
        detach_link(link)?;
        if find_cgroup_link(gateway)?.is_some() {
            return Err(fail("the gateway program's cgroup link is still attached after detach"));
        }
    }

    for _ in 0..WARMUP_CONNECTS {
        fixture.connect_and_close()?;
    }
    let start = if no_hook { None } else { read_stats(rep, gateway)? };
    let began = rep.clock().now();
    for _ in 0..TIMED_CONNECTS {
        fixture.connect_and_close()?;
    }
    let wall = rep.clock().now().duration_since(began);
    let end = if no_hook { None } else { read_stats(rep, gateway)? };

    #[allow(
        clippy::cast_precision_loss,
        reason = "a nanosecond total of one loop is far below 2^52"
    )]
    rep.record("wall_ns_per_connect", wall.as_nanos() as f64 / f64::from(TIMED_CONNECTS))?;
    match (no_hook, start, end) {
        (true, ..) => rep.record_absent("hook_ns_per_connect", NO_HOOK_REASON)?,
        (false, Some(start), Some(end)) => {
            rep.record("hook_ns_per_connect", hook_ns(&start, &end)?)?;
        }
        (false, ..) => {
            rep.record_absent("hook_ns_per_connect", "BPF statistics are not held under smoke")?;
        }
    }
    Ok(())
}

fn fail(message: &str) -> ReplicateFailure {
    ReplicateFailure::fixture(message)
}

/// Reads the cumulative statistics of the gateway program; `None` when no statistics are held
/// (smoke).
fn read_stats(
    rep: &Replicate,
    program: u32,
) -> Result<Option<overdrive_bench::RunStats>, ReplicateFailure> {
    let Some(hold) = rep.bpf_stats() else {
        return Ok(None);
    };
    hold.read(program).map(Some).map_err(|error| ReplicateFailure::measurement(&error.to_string()))
}

/// The program's `run_time_ns` delta over its `run_count` delta; a run count that did not
/// advance fails the replicate rather than recording a value.
#[allow(clippy::cast_precision_loss, reason = "per-loop nanosecond totals are far below 2^52")]
fn hook_ns(
    start: &overdrive_bench::RunStats,
    end: &overdrive_bench::RunStats,
) -> Result<f64, ReplicateFailure> {
    let time = end.run_time_ns.saturating_sub(start.run_time_ns);
    let count = end.run_count.saturating_sub(start.run_count);
    if count == 0 {
        return Err(ReplicateFailure::measurement(
            "the gateway connect program's run count did not advance over the timed loop (zero run-count delta)",
        ));
    }
    Ok(time as f64 / count as f64)
}

/// A raw `bpf(2)` command with `attr` as its argument block.
fn bpf<T>(command: libc::c_long, attr: &mut T) -> std::io::Result<libc::c_long> {
    // SAFETY: `attr` is an exclusively borrowed repr(C) block of the stated size, valid for the
    // call; the kernel reads or writes at most `size_of::<T>()` bytes of it.
    let status = unsafe {
        libc::syscall(libc::SYS_bpf, command, std::ptr::from_mut(attr), std::mem::size_of::<T>())
    };
    if status < 0 { Err(std::io::Error::last_os_error()) } else { Ok(status) }
}

const BPF_OBJ_GET_INFO_BY_FD: libc::c_long = 15;
const BPF_LINK_GET_FD_BY_ID: libc::c_long = 30;
const BPF_LINK_GET_NEXT_ID: libc::c_long = 31;
const BPF_LINK_DETACH: libc::c_long = 34;
const BPF_LINK_TYPE_CGROUP: u32 = 3;

#[repr(C)]
struct IdAttr {
    id: u32,
    next_id: u32,
    open_flags: u32,
}

#[repr(C)]
struct InfoAttr {
    fd: u32,
    info_len: u32,
    info: u64,
}

#[repr(C)]
struct DetachAttr {
    link_fd: u32,
}

/// The leading fields of `struct bpf_link_info` through the cgroup arm of its union
/// (`include/uapi/linux/bpf.h`: the union starts at offset 16).
#[repr(C)]
#[derive(Default)]
struct LinkInfo {
    link_type: u32,
    id: u32,
    prog_id: u32,
    reserved: u32,
    /// Zero once the link is detached from its cgroup (`bpf_cgroup_link_fill_link_info`).
    cgroup_id: u64,
    attach_type: u32,
}

fn link_error(step: &str, error: &std::io::Error) -> ReplicateFailure {
    fail(&format!("{step} failed: {error}"))
}

/// The id of the cgroup link that currently attaches `program` to a cgroup, if one exists.
fn find_cgroup_link(program: u32) -> Result<Option<u32>, ReplicateFailure> {
    let mut previous = 0_u32;
    loop {
        let mut next = IdAttr { id: previous, next_id: 0, open_flags: 0 };
        match bpf(BPF_LINK_GET_NEXT_ID, &mut next) {
            Ok(_) => {}
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => return Ok(None),
            Err(error) => return Err(link_error("BPF_LINK_GET_NEXT_ID", &error)),
        }
        previous = next.next_id;
        let Some(info) = link_info(previous)? else { continue };
        if info.link_type == BPF_LINK_TYPE_CGROUP && info.prog_id == program && info.cgroup_id != 0
        {
            return Ok(Some(info.id));
        }
    }
}

/// The info of link `id`; `None` if the link went away between enumeration and open.
fn link_info(id: u32) -> Result<Option<LinkInfo>, ReplicateFailure> {
    let Some(link) = open_link(id)? else { return Ok(None) };
    let mut info = LinkInfo::default();
    let mut attr = InfoAttr {
        fd: u32::try_from(link.as_raw_fd()).unwrap_or_default(),
        info_len: u32::try_from(std::mem::size_of::<LinkInfo>()).unwrap_or_default(),
        info: std::ptr::from_mut(&mut info) as u64,
    };
    bpf(BPF_OBJ_GET_INFO_BY_FD, &mut attr)
        .map_err(|error| link_error("BPF_OBJ_GET_INFO_BY_FD", &error))?;
    Ok(Some(info))
}

fn open_link(id: u32) -> Result<Option<OwnedFd>, ReplicateFailure> {
    let mut attr = IdAttr { id, next_id: 0, open_flags: 0 };
    match bpf(BPF_LINK_GET_FD_BY_ID, &mut attr) {
        // SAFETY: the kernel returned a new descriptor that nothing else owns.
        Ok(fd) => Ok(Some(unsafe { OwnedFd::from_raw_fd(i32::try_from(fd).unwrap_or(-1)) })),
        Err(error) if error.raw_os_error() == Some(libc::ENOENT) => Ok(None),
        Err(error) => Err(link_error("BPF_LINK_GET_FD_BY_ID", &error)),
    }
}

fn detach_link(id: u32) -> Result<(), ReplicateFailure> {
    let link = open_link(id)?.ok_or_else(|| fail("the gateway link vanished before detach"))?;
    let mut attr = DetachAttr { link_fd: u32::try_from(link.as_raw_fd()).unwrap_or_default() };
    bpf(BPF_LINK_DETACH, &mut attr).map(drop).map_err(|error| link_error("BPF_LINK_DETACH", &error))
}

/// One replicate's cgroup, pin directory, veth pair, listener and loaded dataplane. Teardown
/// is `Drop`, in reverse build order.
struct Fixture {
    addr: SocketAddr,
    programs_before: std::collections::BTreeSet<u32>,
    // Fields drop in this order after `Drop::drop`: the listener, the veth, the cgroup last.
    dataplane: Option<EbpfDataplane>,
    _listener: Listener,
    _veth: Veth,
    _pin: PinDir,
    _cgroup: Cgroup,
}

impl Fixture {
    fn build(rep: &mut Replicate) -> Result<Self, ReplicateFailure> {
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let tag = format!("{}-{sequence}", std::process::id());
        let cgroup = Cgroup::enter(&tag)?;
        let pin = PinDir::create(PathBuf::from(format!("/sys/fs/bpf/ovbench-connect-{tag}")))?;
        let veth = Veth::create(rep)?;
        let listener = Listener::start()?;
        let programs_before = program_ids();
        let dataplane =
            EbpfDataplane::new_with_pin_dir(CLIENT_IFACE, BACKEND_IFACE, pin.path(), cgroup.path())
                .map_err(|error| fail(&format!("loading the dataplane: {error}")))?;
        Ok(Self {
            addr: listener.addr,
            programs_before,
            dataplane: Some(dataplane),
            _listener: listener,
            _veth: veth,
            _pin: pin,
            _cgroup: cgroup,
        })
    }

    /// The gateway connect program: the one program loaded by this fixture's dataplane under
    /// the connect program name, confirmed by its reference to the gateway intent map. Zero
    /// candidates, several, or one that does not reference the map fails the replicate naming
    /// what was found, so a detach never lands on a program picked by name alone.
    fn gateway_program(&self) -> Result<u32, ReplicateFailure> {
        let gateway_maps: std::collections::BTreeSet<u32> = loaded_maps()
            .filter_map(Result::ok)
            .filter(|map| map.name_as_str() == Some(GATEWAY_INTENT_MAP_NAME))
            .map(|map| map.id())
            .collect();
        let mut candidates = Vec::with_capacity(1);
        for program in loaded_programs().filter_map(Result::ok) {
            if self.programs_before.contains(&program.id())
                || program.name_as_str() != Some(CONNECT_PROGRAM_NAME)
            {
                continue;
            }
            let map_ids = program
                .map_ids()
                .map_err(|error| {
                    fail(&format!("reading the maps of program {}: {error}", program.id()))
                })?
                .unwrap_or_default();
            let references_gateway_map = map_ids.iter().any(|id| gateway_maps.contains(id));
            candidates.push((program.id(), references_gateway_map));
        }
        match candidates.as_slice() {
            [(gateway, true)] => Ok(*gateway),
            other => Err(fail(&format!(
                "expected exactly one new {CONNECT_PROGRAM_NAME} program referencing {GATEWAY_INTENT_MAP_NAME}* (program id, references the map), found {other:?}"
            ))),
        }
    }

    fn connect_and_close(&self) -> Result<(), ReplicateFailure> {
        let stream = TcpStream::connect(self.addr)
            .map_err(|error| fail(&format!("loopback connect failed: {error}")))?;
        let linger = libc::linger { l_onoff: 1, l_linger: 0 };
        // SAFETY: a live socket descriptor and a valid pointer and length for the call.
        let status = unsafe {
            libc::setsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_LINGER,
                std::ptr::from_ref(&linger).cast(),
                libc::socklen_t::try_from(std::mem::size_of::<libc::linger>()).unwrap_or_default(),
            )
        };
        if status != 0 {
            return Err(fail("SO_LINGER could not be set"));
        }
        drop(stream);
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // The dataplane's links go first; the listener, the veth, the pin directory and the
        // cgroup follow as fields drop.
        drop(self.dataplane.take());
    }
}

/// The bpffs pin directory of one replicate; removed on drop, so a later fallible build step
/// does not leak it.
struct PinDir(PathBuf);

impl PinDir {
    fn create(path: PathBuf) -> Result<Self, ReplicateFailure> {
        std::fs::create_dir_all(&path).map_err(|error| {
            fail(&format!("creating pin directory {}: {error}", path.display()))
        })?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PinDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fresh cgroup the process runs in for the replicate.
struct Cgroup {
    path: PathBuf,
    previous: PathBuf,
}

impl Cgroup {
    fn enter(tag: &str) -> Result<Self, ReplicateFailure> {
        let membership = std::fs::read_to_string("/proc/self/cgroup")
            .map_err(|error| fail(&format!("reading /proc/self/cgroup: {error}")))?;
        let relative = membership
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .ok_or_else(|| fail("the process is not in a cgroup v2 hierarchy"))?;
        let previous = Path::new("/sys/fs/cgroup").join(relative.trim_start_matches('/'));
        let path = PathBuf::from(format!("/sys/fs/cgroup/ovbench-connect-{tag}"));
        std::fs::create_dir(&path)
            .map_err(|error| fail(&format!("creating cgroup {}: {error}", path.display())))?;
        let cgroup = Self { path, previous };
        std::fs::write(cgroup.path.join("cgroup.procs"), std::process::id().to_string()).map_err(
            |error| fail(&format!("entering cgroup {}: {error}", cgroup.path.display())),
        )?;
        Ok(cgroup)
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Cgroup {
    fn drop(&mut self) {
        let _ = std::fs::write(self.previous.join("cgroup.procs"), std::process::id().to_string());
        let _ = std::fs::remove_dir(&self.path);
    }
}

/// The veth pair the dataplane's forward program attaches to.
struct Veth;

impl Veth {
    fn create(rep: &mut Replicate) -> Result<Self, ReplicateFailure> {
        // A pair a crashed earlier run left behind is removed; a clean host has none, and a
        // failed removal fails the replicate.
        if Path::new("/sys/class/net").join(CLIENT_IFACE).exists() {
            rep.exec("ip", &["link", "del", CLIENT_IFACE])?;
        }
        rep.exec(
            "ip",
            &["link", "add", CLIENT_IFACE, "type", "veth", "peer", "name", BACKEND_IFACE],
        )?;
        let veth = Self;
        rep.exec("ip", &["link", "set", CLIENT_IFACE, "up"])?;
        rep.exec("ip", &["link", "set", BACKEND_IFACE, "up"])?;
        Ok(veth)
    }
}

impl Drop for Veth {
    fn drop(&mut self) {
        // subprocess-ok: teardown of this replicate's own veth pair; `Drop` has no replicate handle for `exec`, and a failed removal is caught by the next replicate's stale-pair check
        let _ = std::process::Command::new("ip").args(["link", "del", CLIENT_IFACE]).output();
    }
}

/// A loopback listener that accepts and closes until stopped.
struct Listener {
    addr: SocketAddr,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Listener {
    fn start() -> Result<Self, ReplicateFailure> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| fail(&format!("binding the loopback listener: {error}")))?;
        let addr = listener
            .local_addr()
            .map_err(|error| fail(&format!("reading the listener address: {error}")))?;
        let stopping = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&stopping);
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if observed.load(Ordering::Acquire) {
                    return;
                }
                drop(stream);
            }
        });
        Ok(Self { addr, stopping, thread: Some(thread) })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        // Wake the accept loop so it observes the flag.
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn program_ids() -> std::collections::BTreeSet<u32> {
    loaded_programs().filter_map(Result::ok).map(|program| program.id()).collect()
}
