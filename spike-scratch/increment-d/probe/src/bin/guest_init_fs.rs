//! PROBE increment-d — guest PID 1 for P6 (virtiofsd + `--memory shared=on`).
//!
//! Extends increment-a's guest init. Everything increment-a proved (ext4
//! virtio-blk root, vsock beacon before networking, a real `WEXITSTATUS`) is
//! kept unchanged so P6 is measured against the SAME booting VM, not a
//! different one.
//!
//! What is new, and why each piece exists:
//!
//! * mounts the virtiofs shares and round-trips a file in BOTH directions —
//!   the `[D8]` volume mechanism.
//! * attempts a write against a **host-side** `--readonly` export. `[D8g]`
//!   frames `read_only` as a security control, which is only honest if the
//!   HOST enforces it; a guest-side `-o ro` is guest-cooperative and void
//!   against an uncooperative guest. So this init deliberately mounts the
//!   read-only share **read-write** and then tries to write.
//! * deliberately mounts a tag that does not exist, and reports the exact
//!   errno — the evidence `overdrive-init`'s refuse-to-exec path (`[D4]`
//!   amendment / `[D8g]`) is built against.
//! * touches a FIXED 128 MiB of guest memory before the beacon, so the host's
//!   `/proc` capture is taken at the same guest lifecycle point with the same
//!   page-touching in every mode. Without that the `shared=on` memory
//!   comparison would be measuring guest workload, not the backing.
//! * measures volume write throughput and per-file latency under
//!   `--cache=never` (`[D8c]` picked `never` for the volume role without
//!   measuring it, on the one path that carries the workload's output).

use std::ffi::CString;

const AF_VSOCK: libc::c_int = 40;

#[cfg(target_arch = "aarch64")]
const SYS_FINIT_MODULE: libc::c_long = 273;
#[cfg(not(target_arch = "aarch64"))]
compile_error!("probe targets aarch64 only; pin __NR_finit_module for this arch first");

const HOST_CID: u32 = 2;
const BEACON_PORT: u32 = 1234;
const CHILD_EXIT_CODE: i32 = 7;

const TAG_RW: &str = "volrw";
const TAG_RO: &str = "volro";
const TAG_MISSING: &str = "nosuchvolume";
const MNT_RW: &str = "/mnt/rw";
const MNT_RO: &str = "/mnt/ro";
const MNT_BAD: &str = "/mnt/bad";

/// Fixed guest-memory touch, identical in every mode, so the host-side
/// `shared=on` cost comparison is apples-to-apples.
const TOUCH_BYTES: usize = 128 * 1024 * 1024;
/// Throughput payload. Deliberately modest: this runs nested on Apple Silicon.
const PAYLOAD_MIB: usize = 32;
const SMALL_FILES: usize = 200;

const HOST_TO_GUEST_FILE: &str = "from-host.txt";
const GUEST_TO_HOST_FILE: &str = "from-guest.txt";
/// Byte-distinct in both directions so a round-trip assertion cannot be
/// satisfied by an echo of the other side's payload.
const GUEST_PAYLOAD: &str = "GUEST-WROTE-THIS-0123456789-abcdefghij\n";

#[repr(C)]
#[derive(Clone, Copy)]
struct SockaddrVm {
    svm_family: u16,
    svm_reserved1: u16,
    svm_port: u32,
    svm_cid: u32,
    svm_zero: [u8; 4],
}

static mut VSOCK_FD: libc::c_int = -1;

fn console(msg: &str) {
    let bytes = msg.as_bytes();
    unsafe {
        libc::write(1, bytes.as_ptr().cast(), bytes.len());
        libc::write(1, b"\n".as_ptr().cast(), 1);
    }
}

/// Every result line goes to BOTH the serial console (the primary evidence)
/// and the vsock channel (which re-proves P2 still works with the fs device
/// present).
fn report(msg: &str) {
    console(msg);
    unsafe {
        if VSOCK_FD >= 0 {
            let line = format!("{msg}\n");
            libc::write(VSOCK_FD, line.as_ptr().cast(), line.len());
        }
    }
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn errno_str() -> String {
    format!("{}", std::io::Error::last_os_error())
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

fn mkdir_p(path: &str) {
    let _ = std::fs::create_dir_all(path);
}

fn insmod(path: &str) -> i64 {
    let c = CString::new(path).unwrap();
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        console(&format!("init: insmod {path} -> open failed: {}", errno_str()));
        return -1;
    }
    let params = CString::new("").unwrap();
    let rc = unsafe { libc::syscall(SYS_FINIT_MODULE, fd, params.as_ptr(), 0) };
    let err = errno_str();
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

fn now_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64) * 1_000_000_000 + (ts.tv_nsec as u64)
}

fn vsock_connect(cid: u32, port: u32) -> Result<libc::c_int, String> {
    let fd = unsafe { libc::socket(AF_VSOCK, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(format!("socket(AF_VSOCK) failed: {}", errno_str()));
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
        let e = errno_str();
        unsafe { libc::close(fd) };
        return Err(format!("connect(cid={cid}, port={port}) failed: {e}"));
    }
    Ok(fd)
}

fn run_child_and_reap() -> i32 {
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        unsafe { libc::_exit(CHILD_EXIT_CODE) };
    }
    if pid < 0 {
        return -1;
    }
    let mut status: libc::c_int = 0;
    let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
    if waited < 0 {
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
        libc::reboot(libc::RB_AUTOBOOT);
        loop {
            libc::pause();
        }
    }
}

/// Touch a fixed amount of guest memory so the host-side RSS comparison
/// across memory backings is measuring the BACKING, not the workload.
fn touch_memory(bytes: usize) {
    let p = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            bytes,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if p == libc::MAP_FAILED {
        console(&format!("init: mmap({bytes}) FAILED: {}", errno_str()));
        return;
    }
    let page = 4096usize;
    let mut i = 0usize;
    while i < bytes {
        unsafe { *(p as *mut u8).add(i) = 0xA5 };
        i += page;
    }
    console(&format!(
        "init: touched {} MiB of guest memory (every 4K page written)",
        bytes / 1024 / 1024
    ));
    // Deliberately NOT munmap'd: the pages must stay resident until the host
    // takes its /proc snapshot.
}

fn meminfo_kb(key: &str) -> String {
    read_file("/proc/meminfo")
        .lines()
        .find(|l| l.starts_with(key))
        .map(|l| l.trim().to_string())
        .unwrap_or_else(|| format!("<{key} not found>"))
}

/// Mount one virtiofs share and report the raw outcome.
fn try_mount_share(tag: &str, target: &str, why: &str) -> bool {
    mkdir_p(target);
    let rc = mount_fs(tag, target, "virtiofs");
    if rc == 0 {
        report(&format!("FS-MOUNT tag={tag} at {target} -> OK   ({why})"));
        true
    } else {
        report(&format!(
            "FS-MOUNT tag={tag} at {target} -> rc={rc} errno={} ({})   ({why})",
            errno(),
            errno_str(),
        ));
        false
    }
}

fn write_file_bytes(path: &str, data: &[u8]) -> Result<(), (i32, String)> {
    let c = CString::new(path).unwrap();
    let fd =
        unsafe { libc::open(c.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC, 0o644) };
    if fd < 0 {
        return Err((errno(), errno_str()));
    }
    let mut off = 0usize;
    while off < data.len() {
        let n = unsafe {
            libc::write(fd, data.as_ptr().add(off).cast(), (data.len() - off) as libc::size_t)
        };
        if n <= 0 {
            let e = (errno(), errno_str());
            unsafe { libc::close(fd) };
            return Err(e);
        }
        off += n as usize;
    }
    unsafe {
        libc::fsync(fd);
        libc::close(fd);
    }
    Ok(())
}

fn measure_throughput(dir: &str) {
    let path = format!("{dir}/payload.bin");
    let chunk = vec![0x5Au8; 1024 * 1024];
    let c = CString::new(path.clone()).unwrap();
    let fd =
        unsafe { libc::open(c.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC, 0o644) };
    if fd < 0 {
        report(&format!("FS-THROUGHPUT open({path}) FAILED errno={} ({})", errno(), errno_str()));
        return;
    }
    let t0 = now_ns();
    for _ in 0..PAYLOAD_MIB {
        let mut off = 0usize;
        while off < chunk.len() {
            let n = unsafe {
                libc::write(fd, chunk.as_ptr().add(off).cast(), (chunk.len() - off) as libc::size_t)
            };
            if n <= 0 {
                report(&format!("FS-THROUGHPUT write FAILED errno={} ({})", errno(), errno_str()));
                unsafe { libc::close(fd) };
                return;
            }
            off += n as usize;
        }
    }
    let t_write = now_ns();
    unsafe { libc::fsync(fd) };
    let t_sync = now_ns();
    unsafe { libc::close(fd) };

    let write_s = (t_write - t0) as f64 / 1e9;
    let sync_s = (t_sync - t_write) as f64 / 1e9;
    let total_s = (t_sync - t0) as f64 / 1e9;
    report(&format!(
        "FS-THROUGHPUT {PAYLOAD_MIB} MiB write={write_s:.3}s fsync={sync_s:.3}s total={total_s:.3}s \
         -> {:.1} MiB/s (write only) / {:.1} MiB/s (incl. fsync)",
        PAYLOAD_MIB as f64 / write_s,
        PAYLOAD_MIB as f64 / total_s
    ));
}

fn measure_per_file_latency(dir: &str) {
    let sub = format!("{dir}/manyfiles");
    mkdir_p(&sub);
    let body = b"per-file-latency-probe\n";
    let t0 = now_ns();
    let mut failed = 0usize;
    for i in 0..SMALL_FILES {
        if write_file_bytes(&format!("{sub}/f{i:04}.txt"), body).is_err() {
            failed += 1;
        }
    }
    let dt = now_ns() - t0;
    report(&format!(
        "FS-LATENCY {SMALL_FILES} files (open+write+fsync+close) total={:.3}s \
         -> mean {:.2} ms/file, failures={failed}",
        dt as f64 / 1e9,
        dt as f64 / 1e6 / SMALL_FILES as f64
    ));
}

fn main() {
    attach_console();
    let pid = unsafe { libc::getpid() };
    console("=========================================================");
    console(&format!("init: HELLO from overdrive spike init (P6), pid={pid}"));
    console("=========================================================");

    mount_fs("proc", "/proc", "proc");
    mount_fs("sysfs", "/sys", "sysfs");
    mount_fs("devtmpfs", "/dev", "devtmpfs");

    console(&format!("init: /proc/cmdline = {}", read_file("/proc/cmdline").trim_end()));

    for m in [
        "/modules/vsock.ko",
        "/modules/vmw_vsock_virtio_transport_common.ko",
        "/modules/vmw_vsock_virtio_transport.ko",
        "/modules/virtiofs.ko",
    ] {
        insmod(m);
    }
    console(&format!(
        "init: /proc/filesystems virtiofs line = {:?}",
        read_file("/proc/filesystems")
            .lines()
            .find(|l| l.contains("virtiofs"))
            .unwrap_or("<virtiofs NOT registered>")
    ));

    // --- fixed memory touch, BEFORE the beacon, identical in every mode ----
    touch_memory(TOUCH_BYTES);
    console(&format!("init: guest {}", meminfo_kb("MemTotal")));
    console(&format!("init: guest {}", meminfo_kb("MemAvailable")));

    // --- vsock beacon (unchanged from increment-a) -------------------------
    let mut attempt = 0;
    let connected = loop {
        attempt += 1;
        match vsock_connect(HOST_CID, BEACON_PORT) {
            Ok(fd) => break Ok(fd),
            Err(e) if attempt < 25 => {
                if attempt == 1 {
                    console(&format!("init: vsock attempt 1 failed ({e}); retrying"));
                }
                sleep_ms(100);
            }
            Err(e) => break Err(e),
        }
    };
    match connected {
        Ok(fd) => {
            unsafe { VSOCK_FD = fd };
            report(&format!("READY pid={pid} port={BEACON_PORT}"));
        }
        Err(e) => console(&format!("init: VSOCK FAILED: {e}")),
    }
    // Give the host a defined window to snapshot /proc at the beacon, with the
    // 128 MiB already resident, BEFORE any filesystem I/O perturbs it.
    report(&format!("MEMTOTAL {}", meminfo_kb("MemTotal")));
    sleep_ms(4000);
    report("MEMSNAP-WINDOW-CLOSED");

    // --- P6 proper ---------------------------------------------------------
    console("---------------- P6: virtiofs shares ----------------");
    let rw_ok = try_mount_share(TAG_RW, MNT_RW, "read-write volume");
    // NOTE: mounted READ-WRITE on purpose. A guest-side `-o ro` would be
    // guest-cooperative and prove nothing about host-side enforcement.
    let ro_ok = try_mount_share(
        TAG_RO,
        MNT_RO,
        "host-side --readonly export, mounted RW by the guest ON PURPOSE",
    );
    // The refuse-to-exec evidence: what a FAILED mount looks like from inside.
    try_mount_share(TAG_MISSING, MNT_BAD, "DELIBERATELY BROKEN — tag does not exist");

    if rw_ok {
        let listing = std::fs::read_dir(MNT_RW)
            .map(|d| {
                let mut v: Vec<String> = d
                    .filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                v.sort();
                v.join(",")
            })
            .unwrap_or_else(|e| format!("<readdir failed: {e}>"));
        report(&format!("FS-RW-LISTING [{listing}]"));

        // host -> guest
        let from_host = read_file(&format!("{MNT_RW}/{HOST_TO_GUEST_FILE}"));
        report(&format!("FS-HOST-TO-GUEST {:?}", from_host.trim_end()));

        // guest -> host
        match write_file_bytes(&format!("{MNT_RW}/{GUEST_TO_HOST_FILE}"), GUEST_PAYLOAD.as_bytes())
        {
            Ok(()) => report(&format!("FS-GUEST-TO-HOST wrote {:?}", GUEST_PAYLOAD.trim_end())),
            Err((e, s)) => report(&format!("FS-GUEST-TO-HOST FAILED errno={e} ({s})")),
        }
    }

    if ro_ok {
        // (a) create a NEW file in the read-only export
        match write_file_bytes(
            &format!("{MNT_RO}/guest-should-not-create.txt"),
            b"if you can read this, host-side read_only is NOT enforced\n",
        ) {
            Ok(()) => report(
                "FS-RO-CREATE **SUCCEEDED** -> host-side read_only is NOT enforced (D8g security framing is false)",
            ),
            Err((e, s)) => report(&format!("FS-RO-CREATE refused errno={e} ({s})")),
        }
        // (b) overwrite an EXISTING file in the read-only export — the
        //     stronger check; a create can fail for reasons other than RO.
        match write_file_bytes(
            &format!("{MNT_RO}/preexisting-host-file.txt"),
            b"OVERWRITTEN-BY-GUEST\n",
        ) {
            Ok(()) => {
                report("FS-RO-OVERWRITE **SUCCEEDED** -> host-side read_only is NOT enforced")
            }
            Err((e, s)) => report(&format!("FS-RO-OVERWRITE refused errno={e} ({s})")),
        }
        // (c) can it at least READ?
        let ro_read = read_file(&format!("{MNT_RO}/preexisting-host-file.txt"));
        report(&format!("FS-RO-READ {:?}", ro_read.trim_end()));
    }

    if rw_ok {
        console("---------------- P6: volume I/O cost (--cache=never) ----------------");
        measure_throughput(MNT_RW);
        measure_per_file_latency(MNT_RW);
    }

    // --- exit status, unchanged from increment-a ---------------------------
    let code = run_child_and_reap();
    report(&format!("EXIT {code}"));
    report("DONE");
    sleep_ms(300);
    unsafe {
        if VSOCK_FD >= 0 {
            libc::shutdown(VSOCK_FD, libc::SHUT_RDWR);
            libc::close(VSOCK_FD);
        }
    }
    sleep_ms(200);
    power_off();
}
