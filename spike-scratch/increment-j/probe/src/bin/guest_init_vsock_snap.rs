//! PROBE increment-j — guest PID 1 for S-3: **vsock across snapshot/restore**.
//!
//! Extends increment-g's snapshot guest (RAM-only boot nonce + monotonic tick
//! counter) with the vsock mechanics increment-a/e established. The nonce is not
//! optional decoration: a RESTORED VM and a REBOOTED VM look nearly identical
//! from outside, and every claim below ("the connection survived", "the
//! connection was reset") is meaningless unless the guest genuinely resumed from
//! saved memory. Nonce identical + counter continued = memory restore. Anything
//! else and the vsock result describes a fresh boot, not a checkpoint.
//!
//! Three things are measured, every tick, and reported on the SERIAL CONSOLE —
//! deliberately not over vsock, because vsock is the thing under test:
//!
//!   1. `held_w` — a non-blocking `send()` on the fd that was connected BEFORE
//!      the snapshot and held open across it. Reports the raw errno.
//!   2. `held_r` — a non-blocking `recv()` on that same fd. This is what
//!      separates the two failure shapes that matter:
//!        * `EAGAIN`      -> connection alive, no data pending
//!        * `0` (EOF)     -> peer closed cleanly
//!        * `ECONNRESET`  -> the device reset it (cloud-hypervisor#7958's fix)
//!        * `ENOTCONN`    -> torn down underneath us
//!      A `send()` that returns success while the host never sees the bytes is
//!      the *silently stale half-open* case, and only the host transcript can
//!      catch it — which is why the payload is host-identifiable (below).
//!   3. `new` — a FRESH `connect()` to a second port, attempted every tick from
//!      boot. This is the question that decides whether the Running gate is
//!      recoverable: an established connection dying is survivable if a new one
//!      can be made. Non-blocking with a bounded poll, so a hung connect costs
//!      one tick rather than wedging PID 1.
//!
//! Payloads carry `n=<tick> nonce=<nonce>` so the HOST transcript proves which
//! side of the checkpoint the bytes came from. "send() returned 21" is a
//! guest-side claim; "the host printed HELD n=25" is the end-to-end fact.
//!
//! SIGPIPE is ignored and every send uses MSG_NOSIGNAL. Without that, the first
//! write to a torn-down vsock kills PID 1, the kernel panics, and a *connection
//! reset* would have been recorded as *the guest died on restore* — a far
//! larger and completely wrong finding.

use std::ffi::CString;
use std::io::Read;

const TICK_MS: u64 = 500;

/// `AF_VSOCK`. Not in libc's constant set for every target; pinned per
/// increment-a, which verified it against the target's own headers.
const AF_VSOCK: libc::c_int = 40;
/// `VMADDR_CID_HOST`. The guest always dials 2 to reach the host end.
const HOST_CID: u32 = 2;

/// Connected at boot, held open across the checkpoint. The subject of Q2.
const PORT_HELD: u32 = 1234;
/// Re-dialled fresh every tick. The subject of Q4.
const PORT_NEW: u32 = 1235;

/// Bounded so a connect that never completes costs one tick, not the run.
const CONNECT_TIMEOUT_MS: libc::c_int = 300;

#[repr(C)]
struct SockaddrVm {
    svm_family: u16,
    svm_reserved1: u16,
    svm_port: u32,
    svm_cid: u32,
    svm_zero: [u8; 4],
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Symbolic errno names for the ones this probe can actually produce. Printing
/// the number alone would make the transcript unreadable at review time, and
/// printing only a name would lose the number the findings must quote.
fn errname(e: i32) -> &'static str {
    match e {
        0 => "OK",
        libc::EAGAIN => "EAGAIN",
        libc::EPIPE => "EPIPE",
        libc::ENOTCONN => "ENOTCONN",
        libc::ECONNRESET => "ECONNRESET",
        libc::ECONNREFUSED => "ECONNREFUSED",
        libc::ETIMEDOUT => "ETIMEDOUT",
        libc::EHOSTUNREACH => "EHOSTUNREACH",
        libc::EADDRNOTAVAIL => "EADDRNOTAVAIL",
        libc::EAFNOSUPPORT => "EAFNOSUPPORT",
        libc::EBADF => "EBADF",
        libc::EINVAL => "EINVAL",
        libc::ENODEV => "ENODEV",
        _ => "?",
    }
}

fn console(msg: &str) {
    let bytes = msg.as_bytes();
    unsafe {
        libc::write(1, bytes.as_ptr().cast(), bytes.len());
        libc::write(1, b"\n".as_ptr().cast(), 1);
    }
}

fn attach_console() {
    let path = CString::new("/dev/console").unwrap();
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR) };
    if fd >= 0 {
        unsafe {
            libc::dup2(fd, 0);
            libc::dup2(fd, 1);
            libc::dup2(fd, 2);
            if fd > 2 {
                libc::close(fd);
            }
        }
    }
}

fn mount_fs(source: &str, target: &str, fstype: &str) -> i32 {
    let s = CString::new(source).unwrap();
    let t = CString::new(target).unwrap();
    let f = CString::new(fstype).unwrap();
    unsafe { libc::mount(s.as_ptr(), t.as_ptr(), f.as_ptr(), 0, std::ptr::null()) }
}

fn sleep_ms(ms: u64) {
    let ts = libc::timespec {
        tv_sec: (ms / 1000) as libc::time_t,
        tv_nsec: ((ms % 1000) * 1_000_000) as libc::c_long,
    };
    unsafe { libc::nanosleep(&ts, std::ptr::null_mut()) };
}

fn clock_ms(which: libc::clockid_t) -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(which, &mut ts) };
    (ts.tv_sec as u64) * 1000 + (ts.tv_nsec as u64) / 1_000_000
}

/// 16 bytes of entropy read ONCE at boot and held only in RAM. Nothing writes
/// it anywhere. Identical across the checkpoint == the memory really came back.
fn boot_nonce() -> String {
    match std::fs::File::open("/dev/urandom") {
        Ok(mut f) => {
            let mut buf = [0u8; 16];
            match f.read_exact(&mut buf) {
                Ok(()) => buf.iter().map(|b| format!("{b:02x}")).collect(),
                Err(e) => format!("<urandom read failed: {e}>"),
            }
        }
        Err(e) => format!("<urandom open failed: {e}>"),
    }
}

fn cpu_online() -> usize {
    std::fs::read_to_string("/proc/cpuinfo")
        .map(|s| s.lines().filter(|l| l.starts_with("processor")).count())
        .unwrap_or(0)
}

/// finit_module(2). Arch-specific; both values verified against the target's own
/// headers rather than from memory (per increment-a).
#[cfg(target_arch = "x86_64")]
const SYS_FINIT_MODULE: libc::c_long = 313;
#[cfg(target_arch = "aarch64")]
const SYS_FINIT_MODULE: libc::c_long = 273;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("pin __NR_finit_module for this arch before building the probe");

fn insmod(path: &str) -> bool {
    let c = CString::new(path).unwrap();
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        console(&format!("init: insmod {path} -> open FAILED errno={}", errno()));
        return false;
    }
    let params = CString::new("").unwrap();
    let rc = unsafe { libc::syscall(SYS_FINIT_MODULE, fd, params.as_ptr(), 0) };
    let e = errno();
    unsafe { libc::close(fd) };
    console(&format!(
        "init: insmod {path} -> rc={rc}{}",
        if rc == 0 { String::new() } else { format!(" errno={e} ({})", errname(e)) }
    ));
    rc == 0
}

fn path_exists(p: &str) -> bool {
    std::fs::metadata(p).is_ok()
}

/// Non-blocking connect with a bounded poll. Returns the fd or the errno that
/// explains the refusal — which is the whole point for Q4: "nothing is listening
/// on the host" and "the device is gone" must be distinguishable.
fn vsock_connect(cid: u32, port: u32) -> Result<libc::c_int, i32> {
    let fd = unsafe { libc::socket(AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_NONBLOCK, 0) };
    if fd < 0 {
        return Err(errno());
    }
    let addr = SockaddrVm {
        svm_family: AF_VSOCK as u16,
        svm_reserved1: 0,
        svm_port: port,
        svm_cid: cid,
        svm_zero: [0; 4],
    };
    let rc = unsafe {
        libc::connect(
            fd,
            std::ptr::addr_of!(addr).cast(),
            std::mem::size_of::<SockaddrVm>() as libc::socklen_t,
        )
    };
    if rc == 0 {
        return Ok(fd);
    }
    let e = errno();
    if e != libc::EINPROGRESS {
        unsafe { libc::close(fd) };
        return Err(e);
    }
    let mut pfd = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
    let pr = unsafe { libc::poll(&mut pfd, 1, CONNECT_TIMEOUT_MS) };
    if pr == 0 {
        unsafe { libc::close(fd) };
        return Err(libc::ETIMEDOUT);
    }
    if pr < 0 {
        let e = errno();
        unsafe { libc::close(fd) };
        return Err(e);
    }
    let mut so_err: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            std::ptr::addr_of_mut!(so_err).cast(),
            &mut len,
        )
    };
    if so_err != 0 {
        unsafe { libc::close(fd) };
        return Err(so_err);
    }
    Ok(fd)
}

/// MSG_NOSIGNAL is load-bearing: without it the first write to a torn-down
/// vsock raises SIGPIPE, kills PID 1, and panics the guest — which would be
/// recorded as "the guest died on restore" instead of "the connection reset".
fn probe_send(fd: libc::c_int, payload: &str) -> String {
    let b = payload.as_bytes();
    let w = unsafe {
        libc::send(fd, b.as_ptr().cast(), b.len(), libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL)
    };
    if w >= 0 {
        format!("w={w}")
    } else {
        let e = errno();
        format!("E{e}/{}", errname(e))
    }
}

/// EAGAIN here means "alive, nothing to read" — the healthy steady state.
/// 0 means the peer closed. ECONNRESET means the device reset it.
fn probe_recv(fd: libc::c_int) -> String {
    let mut buf = [0u8; 64];
    let r = unsafe {
        libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL)
    };
    if r > 0 {
        format!("r={r}")
    } else if r == 0 {
        "EOF".to_string()
    } else {
        let e = errno();
        format!("E{e}/{}", errname(e))
    }
}

fn main() {
    attach_console();
    mount_fs("proc", "/proc", "proc");
    mount_fs("sysfs", "/sys", "sysfs");
    mount_fs("devtmpfs", "/dev", "devtmpfs");
    // See the module docstring: without this a torn-down vsock write kills PID 1.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };

    let nonce = boot_nonce();

    console("=========================================================");
    console(&format!("init: VSOCK-SNAP PROBE up. BOOT_NONCE={nonce}"));
    console(&format!("init: vcpus_online={}", cpu_online()));

    // ---- PREREQUISITE EVIDENCE ------------------------------------------
    // Two prior probes in this spike recorded confident, WRONG negatives because
    // a guest-side prerequisite was missing (a module that was not staged made
    // the mechanism look unsupported). So before any claim about vsock surviving
    // anything, the transcript must show the guest could exercise vsock at all:
    // modules loaded, /dev/vsock present, socket(AF_VSOCK) accepted.
    console(&format!("init: /dev/vsock present BEFORE insmod = {}", path_exists("/dev/vsock")));
    let mods_ok = [
        "/modules/vsock.ko",
        "/modules/vmw_vsock_virtio_transport_common.ko",
        "/modules/vmw_vsock_virtio_transport.ko",
    ]
    .iter()
    .all(|m| insmod(m));
    console(&format!("init: all three vsock modules loaded = {mods_ok}"));
    console(&format!("init: /dev/vsock present AFTER insmod = {}", path_exists("/dev/vsock")));

    let probe_sock = unsafe { libc::socket(AF_VSOCK, libc::SOCK_STREAM, 0) };
    if probe_sock < 0 {
        let e = errno();
        console(&format!(
            "init: PREREQ FAIL socket(AF_VSOCK) errno={e} ({}) -- every vsock result below is meaningless",
            errname(e)
        ));
    } else {
        console(&format!("init: PREREQ OK socket(AF_VSOCK) = fd {probe_sock}"));
        unsafe { libc::close(probe_sock) };
    }
    console("=========================================================");

    // ---- the connection held ACROSS the checkpoint ------------------------
    let mut held_fd: Option<libc::c_int> = None;
    for attempt in 1..=25 {
        match vsock_connect(HOST_CID, PORT_HELD) {
            Ok(fd) => {
                console(&format!(
                    "init: HELD connect(cid=2,port={PORT_HELD}) OK fd={fd} attempt={attempt}"
                ));
                let s = probe_send(fd, &format!("HELD-OPEN n=0 nonce={nonce}\n"));
                console(&format!("init: HELD first send -> {s}"));
                held_fd = Some(fd);
                break;
            }
            Err(e) => {
                if attempt == 1 || attempt == 25 {
                    console(&format!(
                        "init: HELD connect attempt {attempt} failed errno={e} ({})",
                        errname(e)
                    ));
                }
                sleep_ms(100);
            }
        }
    }
    if held_fd.is_none() {
        console("init: HELD connection NEVER established -- Q2 is unanswerable this run");
    }

    // Never exits. The host drives pause/snapshot/kill/restore around this.
    let mut n: u64 = 0;
    let mut new_ok_total: u64 = 0;
    loop {
        let held = match held_fd {
            Some(fd) => {
                let w = probe_send(fd, &format!("HELD n={n} nonce={nonce}\n"));
                let r = probe_recv(fd);
                format!("held_w={w} held_r={r}")
            }
            None => "held_w=NOFD held_r=NOFD".to_string(),
        };
        // A fresh dial every tick, from boot. Before the checkpoint this shows
        // the healthy baseline; during the gap it shows what the guest sees when
        // no host listener exists; after a fresh listener is bound it shows
        // whether the Running gate is recoverable.
        let new = match vsock_connect(HOST_CID, PORT_NEW) {
            Ok(fd) => {
                let s = probe_send(fd, &format!("NEW n={n} nonce={nonce}\n"));
                unsafe { libc::close(fd) };
                new_ok_total += 1;
                format!("new=ok({s})")
            }
            Err(e) => format!("new=E{e}/{}", errname(e)),
        };
        console(&format!(
            "TICK n={n} nonce={nonce} mono_ms={} real_ms={} vcpu_online={} {held} {new} new_ok_total={new_ok_total}",
            clock_ms(libc::CLOCK_MONOTONIC),
            clock_ms(libc::CLOCK_REALTIME),
            cpu_online(),
        ));
        n += 1;
        sleep_ms(TICK_MS);
    }
}
