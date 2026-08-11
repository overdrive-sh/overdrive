//! PROBE increment-g — guest PID 1 for SNAPSHOT / RESTORE (S-1, S-3, S-6).
//!
//! The whole probe hinges on one question: after a restore, did the guest
//! RESUME from its saved memory, or did it just BOOT AGAIN? Those look almost
//! identical from outside, and confusing them would make a broken restore read
//! as a working one.
//!
//! So the guest holds a **boot nonce that exists only in RAM**: 16 bytes read
//! from /dev/urandom once, at boot, never written anywhere. Then it ticks
//! forever, printing the nonce and a monotonically increasing counter.
//!
//!   * nonce IDENTICAL across the restore + counter CONTINUES  -> memory was
//!     genuinely restored.
//!   * nonce DIFFERENT / counter restarts at 0                 -> it rebooted,
//!     and "restore worked" would have been a false pass.
//!
//! Two things fall out of the same loop for free:
//!
//!   * **CLOCK_MONOTONIC and CLOCK_REALTIME are both printed.** A restored VM
//!     resumes with the clock it was frozen with, so wall-clock jumps backwards
//!     relative to the host. Firecracker's docs call this out as a snapshot
//!     safety hazard; here it is just visible.
//!   * **The vCPU count is re-read from /proc/cpuinfo every tick**, so a vCPU
//!     hot-plugged AFTER a restore shows up in the transcript without any
//!     further guest-side machinery. That is S-6 — whether CPU hotplug, the
//!     entire reason this feature chose Cloud Hypervisor, still works on a
//!     restored VM. Firecracker forbids it; CH's docs are silent.
//!
//! Deliberately does NOT exit. The host drives pause/snapshot/kill/restore
//! around a running guest, which is the actual lifecycle under test.

use std::ffi::CString;
use std::io::Read;

const TICK_MS: u64 = 500;

/// S-2 volume mount point. Which device/tag lands here is chosen by the host
/// via `spike.vol=` on the kernel cmdline.
const MNT_VOL: &str = "/mnt/vol";

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

/// 16 bytes of entropy, read ONCE at boot and held only in RAM. This is the
/// load-bearing value of the whole probe.
///
/// Note the second, unintended thing it demonstrates: restore the SAME snapshot
/// twice and both guests report the SAME nonce, because both inherit one frozen
/// PRNG state. That is the snapshot-uniqueness hazard Firecracker's docs warn
/// about, made visible rather than argued.
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

/// Online CPUs, per /proc/cpuinfo.
///
/// CAREFUL: /proc/cpuinfo lists only ONLINE cpus. A vCPU that was hot-plugged
/// but never brought online does not appear here, so this number ALONE cannot
/// distinguish "hotplug failed" from "hotplug worked and nobody onlined it".
/// On a normal distro udev does the onlining; this init has no udev. Reporting
/// only this count would have produced a confident, wrong "CPU hotplug does not
/// work on restored VMs".
fn cpu_online() -> usize {
    std::fs::read_to_string("/proc/cpuinfo")
        .map(|s| s.lines().filter(|l| l.starts_with("processor")).count())
        .unwrap_or(0)
}

/// CPUs PRESENT per sysfs, online or not — this is what actually answers
/// "did the hotplug land". `cpuN` directories appear as soon as the vCPU is
/// added, independent of its online state.
fn cpu_present() -> usize {
    std::fs::read_dir("/sys/devices/system/cpu")
        .map(|d| {
            d.filter_map(Result::ok)
                .filter(|e| {
                    let n = e.file_name();
                    let n = n.to_string_lossy();
                    n.starts_with("cpu")
                        && n[3..].chars().all(|c| c.is_ascii_digit())
                        && n.len() > 3
                })
                .count()
        })
        .unwrap_or(0)
}

/// Bring any present-but-offline CPU online, the job udev would normally do.
/// Returns how many were switched on, so the transcript shows the probe acting
/// rather than the kernel doing it spontaneously.
fn online_offline_cpus() -> usize {
    let mut brought = 0;
    for i in 1..64 {
        let path = format!("/sys/devices/system/cpu/cpu{i}/online");
        if let Ok(v) = std::fs::read_to_string(&path) {
            if v.trim() == "0" && std::fs::write(&path, b"1").is_ok() {
                brought += 1;
            }
        }
    }
    brought
}

/// finit_module(2). __NR_finit_module is arch-specific; both values verified
/// against the target's own headers rather than from memory.
#[cfg(target_arch = "x86_64")]
const SYS_FINIT_MODULE: libc::c_long = 313;
#[cfg(target_arch = "aarch64")]
const SYS_FINIT_MODULE: libc::c_long = 273;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("pin __NR_finit_module for this arch before building the probe");

fn insmod(path: &str) {
    let c = CString::new(path).unwrap();
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        console(&format!("init: insmod {path} -> open failed"));
        return;
    }
    let params = CString::new("").unwrap();
    let rc = unsafe { libc::syscall(SYS_FINIT_MODULE, fd, params.as_ptr(), 0) };
    unsafe { libc::close(fd) };
    console(&format!("init: insmod {path} -> rc={rc}"));
}

/// Read a `key=value` token out of /proc/cmdline.
fn cmdline_str(key: &str) -> Option<String> {
    let needle = format!("{key}=");
    std::fs::read_to_string("/proc/cmdline")
        .ok()?
        .split_whitespace()
        .find_map(|t| t.strip_prefix(&needle).map(str::to_owned))
}

/// S-2: mount the volume and open ONE file, keeping the fd open for the rest of
/// the run. The open fd is the actual subject — "the file is still there after a
/// restore" is a much weaker claim than "the descriptor the guest was already
/// holding still works". For virtiofs that descriptor corresponds to state
/// inside virtiofsd, which is OUTSIDE the snapshot; for virtio-blk it is just
/// guest-side page cache over a host file.
fn open_volume(kind: &str) -> Option<libc::c_int> {
    let _ = std::fs::create_dir_all(MNT_VOL);
    let (src, fstype) = match kind {
        "blk" => ("/dev/vdb", "ext4"),
        "fs" => {
            // CONFIG_VIRTIO_FS=m, so without this the mount returns ENODEV and
            // the probe reports a clean-looking `vol=none` that is really a
            // missing module.
            insmod("/modules/virtiofs.ko");
            ("volrw", "virtiofs")
        }
        _ => return None,
    };
    let rc = mount_fs(src, MNT_VOL, fstype);
    console(&format!(
        "init: S-2 mount {src} ({fstype}) at {MNT_VOL} -> rc={rc} errno={}",
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    ));
    if rc != 0 {
        return None;
    }
    let path = CString::new(format!("{MNT_VOL}/persist.bin")).unwrap();
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CREAT, 0o644) };
    if fd < 0 {
        console(&format!(
            "init: S-2 open FAILED errno={}",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
        ));
        return None;
    }
    console(&format!("init: S-2 volume fd={fd} OPEN and will be held across the snapshot"));
    Some(fd)
}

/// Write the tick through the ALREADY-OPEN fd, fsync, and read it back.
/// Returns a short status token for the tick line.
fn volume_roundtrip(fd: libc::c_int, n: u64) -> String {
    let payload = format!(
        "TICK-{n:06}
"
    );
    let b = payload.as_bytes();
    let w = unsafe { libc::pwrite(fd, b.as_ptr().cast(), b.len(), 0) };
    if w < 0 {
        return format!(
            "vol=WRITE_ERR:{}",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
        );
    }
    if unsafe { libc::fsync(fd) } < 0 {
        return format!(
            "vol=FSYNC_ERR:{}",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
        );
    }
    let mut buf = vec![0u8; b.len()];
    let r = unsafe { libc::pread(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
    if r < 0 {
        return format!(
            "vol=READ_ERR:{}",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
        );
    }
    if &buf[..] == b { "vol=ok".into() } else { "vol=MISMATCH".into() }
}

fn main() {
    attach_console();
    mount_fs("proc", "/proc", "proc");
    mount_fs("sysfs", "/sys", "sysfs");
    mount_fs("devtmpfs", "/dev", "devtmpfs");

    let nonce = boot_nonce();
    let boot_real = clock_ms(libc::CLOCK_REALTIME);

    console("=========================================================");
    console(&format!("init: SNAP PROBE up. BOOT_NONCE={nonce}"));
    console(&format!("init: boot CLOCK_REALTIME_ms={boot_real}"));
    console(&format!("init: vcpus_at_boot online={} present={}", cpu_online(), cpu_present()));
    console("=========================================================");

    let vol_kind = cmdline_str("spike.vol").unwrap_or_else(|| "none".into());
    console(&format!("init: S-2 spike.vol={vol_kind}"));
    let vol_fd = open_volume(&vol_kind);

    // Never exits. The host drives pause/snapshot/kill/restore around this.
    let mut n: u64 = 0;
    loop {
        // Online anything that appeared since the last tick, then report BOTH
        // numbers. present > online at any tick means a hotplug landed and the
        // guest simply had not enabled it yet.
        let brought = online_offline_cpus();
        // The volume round-trip runs through the fd opened BEFORE the snapshot.
        // A per-tick status makes the exact tick where it breaks visible, rather
        // than only the end state.
        let vol = match vol_fd {
            Some(fd) => volume_roundtrip(fd, n),
            None => "vol=none".to_string(),
        };
        console(&format!(
            "TICK n={n} nonce={nonce} mono_ms={} real_ms={} vcpu_online={} vcpu_present={} {vol}{}",
            clock_ms(libc::CLOCK_MONOTONIC),
            clock_ms(libc::CLOCK_REALTIME),
            cpu_online(),
            cpu_present(),
            if brought > 0 { format!(" *ONLINED {brought}*") } else { String::new() }
        ));
        n += 1;
        sleep_ms(TICK_MS);
    }
}
