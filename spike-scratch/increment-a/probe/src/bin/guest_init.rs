//! PROBE increment-a — guest PID 1.
//!
//! P1 evidence: this binary is `init=/init` on an ext4 `virtio-blk` rootfs.
//! If it prints anything at all, the kernel booted, mounted /dev/vda, and
//! reached userspace.
//!
//! P2 evidence: opens AF_VSOCK to the host (CID 2) BEFORE any networking
//! exists in the guest, writes a ready beacon, then forks a child that really
//! exits 7, waits for it, and writes the REAL `WEXITSTATUS` over the same
//! channel. The 300 ms gap between the two writes makes ordering observable
//! host-side (separate `read()` returns), rather than one coalesced blob.

use std::ffi::CString;

const AF_VSOCK: libc::c_int = 40;

// __NR_finit_module is ARCH-SPECIFIC. Pinned explicitly rather than taken from
// libc so the number cannot silently differ, and any unhandled arch is a
// COMPILE error rather than a wrong syscall at runtime — a bad number here
// surfaces as an unrelated errno inside the guest with no console clue.
//
// Both values verified against the target's own asm/unistd headers:
//   aarch64 : asm-generic table                      -> 273
//   x86_64  : asm/unistd_64.h:317 on the metal box   -> 313
#[cfg(target_arch = "aarch64")]
const SYS_FINIT_MODULE: libc::c_long = 273;
#[cfg(target_arch = "x86_64")]
const SYS_FINIT_MODULE: libc::c_long = 313;
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
compile_error!("pin __NR_finit_module for this arch before building the probe");

const HOST_CID: u32 = 2;
const BEACON_PORT: u32 = 1234;
const CHILD_EXIT_CODE: i32 = 7;

/// `struct sockaddr_vm` (include/uapi/linux/vm_sockets.h), 16 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
struct SockaddrVm {
    svm_family: u16,
    svm_reserved1: u16,
    svm_port: u32,
    svm_cid: u32,
    svm_zero: [u8; 4],
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
    let rc = unsafe { libc::mount(s.as_ptr(), t.as_ptr(), f.as_ptr(), 0, std::ptr::null()) };
    console(&format!(
        "init: mount {fstype} on {target} -> rc={rc} errno={}",
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    ));
    rc
}

/// Ubuntu builds CONFIG_VSOCKETS / CONFIG_VIRTIO_VSOCKETS as MODULES, so a
/// module-free rootfs gets EAFNOSUPPORT on socket(AF_VSOCK). Load them here, in
/// dependency order, via finit_module(2). CONFIG_MODULE_SIG_FORCE is not set on
/// this kernel, so the appended Ubuntu signatures are not enforced.
fn insmod(path: &str) -> i64 {
    let c = CString::new(path).unwrap();
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        console(&format!(
            "init: insmod {path} -> open failed: {}",
            std::io::Error::last_os_error()
        ));
        return -1;
    }
    let params = CString::new("").unwrap();
    let rc = unsafe { libc::syscall(SYS_FINIT_MODULE, fd, params.as_ptr(), 0) };
    let err = std::io::Error::last_os_error();
    unsafe { libc::close(fd) };
    if rc == 0 {
        console(&format!("init: insmod {path} -> OK"));
    } else {
        console(&format!("init: insmod {path} -> rc={rc} err={err}"));
    }
    rc as i64
}

fn read_file(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| format!("<unreadable: {e}>"))
}

fn sleep_ms(ms: u64) {
    let ts = libc::timespec {
        tv_sec: (ms / 1000) as libc::time_t,
        tv_nsec: ((ms % 1000) * 1_000_000) as libc::c_long,
    };
    unsafe { libc::nanosleep(&ts, std::ptr::null_mut()) };
}

fn vsock_connect(cid: u32, port: u32) -> Result<libc::c_int, String> {
    let fd = unsafe { libc::socket(AF_VSOCK, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(format!("socket(AF_VSOCK) failed: {}", std::io::Error::last_os_error()));
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
    if rc < 0 {
        let err = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(format!("connect(cid={cid}, port={port}) failed: {err}"));
    }
    Ok(fd)
}

fn vsock_write(fd: libc::c_int, msg: &str) -> isize {
    let bytes = msg.as_bytes();
    let n = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
    console(&format!("init: vsock write {n} bytes: {:?}", msg.trim_end()));
    n
}

/// Fork a child that really exits with `CHILD_EXIT_CODE`; return its real
/// `WEXITSTATUS`. This is a genuine process exit status, not a literal.
fn run_child_and_reap() -> i32 {
    console("init: about to fork()");
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // Child.
        unsafe { libc::_exit(CHILD_EXIT_CODE) };
    }
    if pid < 0 {
        console(&format!("init: fork FAILED: {}", std::io::Error::last_os_error()));
        return -1;
    }
    console(&format!("init: forked child pid={pid}; calling waitpid()"));
    let mut status: libc::c_int = 0;
    let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
    if waited < 0 {
        console(&format!("init: waitpid FAILED: {}", std::io::Error::last_os_error()));
        return -1;
    }
    console(&format!(
        "init: reaped pid={waited} raw_status=0x{status:x} exited={} code={}",
        libc::WIFEXITED(status),
        libc::WEXITSTATUS(status)
    ));
    if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { -1 }
}

fn power_off() -> ! {
    console("init: powering off (RB_POWER_OFF)");
    unsafe {
        libc::sync();
        libc::reboot(libc::RB_POWER_OFF);
        // If PSCI SYSTEM_OFF is unavailable, fall back to a reset so the VMM
        // still exits rather than hanging the probe.
        console("init: RB_POWER_OFF returned; falling back to RB_AUTOBOOT");
        libc::reboot(libc::RB_AUTOBOOT);
        loop {
            libc::pause();
        }
    }
}

fn main() {
    attach_console();
    let pid = unsafe { libc::getpid() };
    console("=========================================================");
    console(&format!("init: HELLO from overdrive spike init, pid={pid}"));
    console("=========================================================");

    mount_fs("proc", "/proc", "proc");
    mount_fs("sysfs", "/sys", "sysfs");
    mount_fs("devtmpfs", "/dev", "devtmpfs");

    console(&format!("init: /proc/version = {}", read_file("/proc/version").trim_end()));
    console(&format!("init: /proc/cmdline = {}", read_file("/proc/cmdline").trim_end()));
    console(&format!(
        "init: rootfs mounts = {}",
        read_file("/proc/mounts").lines().find(|l| l.contains(" / ")).unwrap_or("<none>")
    ));
    console(&format!(
        "init: /dev/vsock present BEFORE insmod = {}",
        std::path::Path::new("/dev/vsock").exists()
    ));

    // Dependency order matters: core, then the shared virtio transport helper,
    // then the virtio transport that binds the PCI device.
    for m in [
        "/modules/vsock.ko",
        "/modules/vmw_vsock_virtio_transport_common.ko",
        "/modules/vmw_vsock_virtio_transport.ko",
    ] {
        insmod(m);
    }
    console(&format!(
        "init: /dev/vsock present AFTER insmod = {}",
        std::path::Path::new("/dev/vsock").exists()
    ));

    // --- P2: beacon over vsock, before any guest networking exists. ---
    let ifaces = std::fs::read_dir("/sys/class/net")
        .map(|d| {
            d.filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_else(|_| "<unreadable>".into());
    console(&format!("init: guest net ifaces = [{ifaces}] (no networking configured)"));

    // The virtio-vsock PCI probe completes asynchronously after insmod, so a
    // bounded retry distinguishes "channel unavailable" from "not ready yet".
    let mut attempt = 0;
    let connected = loop {
        attempt += 1;
        match vsock_connect(HOST_CID, BEACON_PORT) {
            Ok(fd) => {
                console(&format!("init: vsock connected on attempt {attempt}"));
                break Ok(fd);
            }
            Err(e) if attempt < 25 => {
                if attempt == 1 {
                    console(&format!("init: vsock attempt {attempt} failed ({e}); retrying"));
                }
                sleep_ms(100);
            }
            Err(e) => break Err(e),
        }
    };

    match connected {
        Ok(fd) => {
            vsock_write(fd, &format!("READY pid={pid} port={BEACON_PORT}\n"));
            sleep_ms(300);
            let code = run_child_and_reap();
            vsock_write(fd, &format!("EXIT {code}\n"));
            sleep_ms(200);
            unsafe {
                libc::shutdown(fd, libc::SHUT_RDWR);
                libc::close(fd);
            }
            console("init: vsock channel closed");
        }
        Err(e) => {
            console(&format!("init: VSOCK FAILED: {e}"));
        }
    }

    sleep_ms(200);
    power_off();
}
