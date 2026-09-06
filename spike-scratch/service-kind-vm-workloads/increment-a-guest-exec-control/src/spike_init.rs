use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::kmod::{ModuleInitFlags, finit_module};
use nix::mount::{MsFlags, mount};
use nix::sys::signal::{Signal, killpg};
use nix::sys::socket::{self, AddressFamily, SockFlag, SockType, VsockAddr};
use nix::unistd::Pid;

const HOST_CID: u32 = 2;
const BEACON_PORT: u32 = 1234;
const CONTROL_PORT: u32 = 1235;
const CAPACITY: usize = 3;

type Active = Arc<Mutex<BTreeMap<String, i32>>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("spike-init fatal: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    bootstrap()?;
    load_vsock_modules()?;

    let mut beacon = connect_vsock(BEACON_PORT, 100)?;
    writeln!(beacon, "READY pid={} port={BEACON_PORT}", std::process::id())
        .map_err(|e| format!("write READY: {e}"))?;
    beacon.flush().map_err(|e| format!("flush READY: {e}"))?;

    let mut line = String::new();
    BufReader::new(beacon.try_clone().map_err(|e| e.to_string())?)
        .read_line(&mut line).map_err(|e| format!("read primary EXEC: {e}"))?;
    let encoded = line.trim_end().strip_prefix("EXEC ")
        .ok_or_else(|| format!("expected EXEC, got {line:?}"))?;
    let argv: Vec<String> = serde_json::from_str(encoded).map_err(|e| format!("decode EXEC: {e}"))?;
    let mut primary = spawn_primary(&argv)?;

    let active: Active = Arc::new(Mutex::new(BTreeMap::new()));
    let mut session = 0_u64;
    loop {
        if let Some(status) = primary.try_wait().map_err(|e| format!("poll primary: {e}"))? {
            let code = status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
            writeln!(beacon, "EXIT {code}").map_err(|e| format!("write EXIT: {e}"))?;
            return Err(format!("primary workload exited during probe spike: {status:?}"));
        }

        let control = connect_vsock(CONTROL_PORT, 300)?;
        session += 1;
        let writer = Arc::new(Mutex::new(control.try_clone().map_err(|e| e.to_string())?));
        reply(&writer, &format!("HELLO session={session} primary_pid={} capacity={CAPACITY}", primary.id()));
        serve_session(control, Arc::clone(&writer), Arc::clone(&active))?;
        kill_all(&active);
        let deadline = Instant::now() + Duration::from_secs(3);
        while !active.lock().expect("active lock").is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        if !active.lock().expect("active lock").is_empty() {
            return Err("disconnect cleanup exceeded three seconds".to_owned());
        }
    }
}

fn bootstrap() -> Result<(), String> {
    fs::create_dir_all("/proc").map_err(|e| e.to_string())?;
    fs::create_dir_all("/sys").map_err(|e| e.to_string())?;
    let flags = MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC;
    match mount(Some("proc"), Path::new("/proc"), Some("proc"), flags, None::<&str>) {
        Ok(()) | Err(Errno::EBUSY) => {}
        Err(e) => return Err(format!("mount proc: {e}")),
    }
    match mount(Some("sysfs"), Path::new("/sys"), Some("sysfs"), flags, None::<&str>) {
        Ok(()) | Err(Errno::EBUSY) => {}
        Err(e) => return Err(format!("mount sysfs: {e}")),
    }
    Ok(())
}

fn load_vsock_modules() -> Result<(), String> {
    for name in ["vsock.ko", "vmw_vsock_virtio_transport_common.ko", "vmw_vsock_virtio_transport.ko"] {
        let path = format!("/modules/{name}");
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("open {path}: {error}")),
        };
        let empty = std::ffi::CStr::from_bytes_with_nul(b"\0").expect("empty C string");
        match finit_module(&file, empty, ModuleInitFlags::empty()) {
            Ok(()) | Err(Errno::EEXIST) => {}
            Err(error) => return Err(format!("load {path}: {error}")),
        }
    }
    Ok(())
}

fn connect_vsock(port: u32, attempts: u32) -> Result<File, String> {
    let mut last = String::new();
    for _ in 0..attempts {
        match socket::socket(AddressFamily::Vsock, SockType::Stream, SockFlag::empty(), None) {
            Ok(fd) => match socket::connect(fd.as_raw_fd(), &VsockAddr::new(HOST_CID, port)) {
                Ok(()) => return Ok(File::from(fd)),
                Err(error) => last = error.to_string(),
            },
            Err(error) => last = error.to_string(),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!("connect vsock port {port}: {last}"))
}

fn spawn_primary(argv: &[String]) -> Result<Child, String> {
    let (program, args) = argv.split_first().ok_or("primary argv empty")?;
    Command::new(program).args(args).spawn().map_err(|e| format!("spawn primary {program}: {e}"))
}

fn serve_session(control: File, writer: Arc<Mutex<File>>, active: Active) -> Result<(), String> {
    let reader = BufReader::new(control);
    for incoming in reader.lines() {
        let line = incoming.map_err(|e| format!("control read: {e}"))?;
        let fields = line.split_whitespace().collect::<Vec<_>>();
        match fields.as_slice() {
            ["STATUS", id] => {
                let count = active.lock().expect("active lock").len();
                reply(&writer, &format!("STATUS {id} primary=alive active={count}"));
            }
            ["RUN", id, mode, timeout_ms] => {
                let timeout_ms = timeout_ms.parse::<u64>().map_err(|e| format!("timeout: {e}"))?;
                start_probe(id, mode, timeout_ms, Arc::clone(&writer), Arc::clone(&active));
            }
            _ => reply(&writer, "ERROR malformed"),
        }
    }
    Ok(())
}

fn start_probe(id: &str, mode: &str, timeout_ms: u64, writer: Arc<Mutex<File>>, active: Active) {
    let id = id.to_owned();
    let mode = mode.to_owned();
    let mut registry = active.lock().expect("active lock");
    if registry.len() >= CAPACITY {
        drop(registry);
        reply(&writer, &format!("OVERLOAD {id} capacity={CAPACITY}"));
        return;
    }
    let mut command = Command::new("/sbin/spike-probe");
    command.arg(&mode).process_group(0);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            drop(registry);
            reply(&writer, &format!("RESULT {id} outcome=spawn:{error} residual=0"));
            return;
        }
    };
    let pgid = i32::try_from(child.id()).expect("guest pid fits i32");
    registry.insert(id.clone(), pgid);
    drop(registry);

    std::thread::spawn(move || finish_probe(id, child, pgid, timeout_ms, writer, active));
}

fn finish_probe(id: String, mut child: Child, pgid: i32, timeout_ms: u64, writer: Arc<Mutex<File>>, active: Active) {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if let Some(code) = status.code() { break format!("exit:{code}"); }
                break format!("signal:{}", status.signal().unwrap_or(0));
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = killpg(Pid::from_raw(pgid), Signal::SIGKILL);
                let _ = child.wait();
                break "timeout".to_owned();
            }
            Err(error) => break format!("wait:{error}"),
        }
    };
    let _ = killpg(Pid::from_raw(pgid), Signal::SIGKILL);
    reap_group(pgid);
    let residual = group_exists(pgid);
    active.lock().expect("active lock").remove(&id);
    reply(&writer, &format!("RESULT {id} outcome={outcome} residual={}", u8::from(residual)));
}

fn kill_all(active: &Active) {
    let pgids = active.lock().expect("active lock").values().copied().collect::<Vec<_>>();
    for pgid in pgids { let _ = killpg(Pid::from_raw(pgid), Signal::SIGKILL); }
}

fn reap_group(pgid: i32) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        match nix::sys::wait::waitpid(
            Pid::from_raw(-pgid),
            Some(nix::sys::wait::WaitPidFlag::WNOHANG),
        ) {
            Ok(nix::sys::wait::WaitStatus::StillAlive) | Err(Errno::ECHILD) => {}
            Ok(_) => continue,
            Err(_) => {}
        }
        if !group_exists(pgid) { return; }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn group_exists(pgid: i32) -> bool {
    match killpg(Pid::from_raw(pgid), None) {
        Ok(()) | Err(Errno::EPERM) => true,
        Err(Errno::ESRCH) => false,
        Err(_) => true,
    }
}

fn reply(writer: &Arc<Mutex<File>>, line: &str) {
    if let Ok(mut writer) = writer.lock() {
        let _ = writeln!(writer, "{line}");
        let _ = writer.flush();
    }
}
