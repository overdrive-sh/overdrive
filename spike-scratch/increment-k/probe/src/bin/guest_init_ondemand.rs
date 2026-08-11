//! PROBE increment-k — guest PID 1 for `memory_restore_mode=ondemand`.
//!
//! Extends increment-g's snapshot guest. Everything increment-g needed to avoid
//! a false pass is kept VERBATIM in spirit, because it is still exactly as easy
//! to fool here:
//!
//!   * a **boot nonce that exists only in RAM** — 16 bytes of /dev/urandom read
//!     once, never written anywhere — printed on every tick, and
//!   * a **monotonic tick counter** that must CONTINUE across the restore.
//!
//! Nonce identical + counter continued + no second boot banner = memory really
//! came back. Anything else means it REBOOTED, and a reboot is FAST — which
//! under this probe's subject would masquerade as a spectacular `ondemand` win.
//! That is the single most dangerous failure mode available here.
//!
//! Two things are NEW relative to increment-g, and both exist because the
//! subject is lazy paging rather than "does restore work at all":
//!
//! 1. **TOUCH.** At boot the guest mmaps `spike.touch` MiB and writes one byte
//!    per 4 KiB page, so the snapshot contains REAL PAGES. increment-g's guest
//!    used ~a few MiB of a 512 MiB VM; a snapshot of never-touched memory is
//!    almost entirely zeros, and zeros are exactly the case a demand-paging
//!    implementation can serve most cheaply (`UFFDIO_ZEROPAGE`, or the host
//!    never even reading the file). Measuring `ondemand` against a zero-filled
//!    snapshot would flatter it for a reason no real workload enjoys.
//!
//!    The byte written is derived from the PAGE INDEX, not a constant. A
//!    constant would be indistinguishable from a page delivered from the wrong
//!    offset; the index makes misplacement detectable.
//!
//! 2. **WALK.** After boot the guest re-reads `spike.walk` MiB of that region
//!    per tick, cycling, and VERIFIES every page against the same index-derived
//!    value. This does three jobs at once:
//!      - it drives host demand faults at a KNOWN rate (MiB/s), which is what
//!        turns "RSS climbed" into "RSS climbed *as the guest touched pages*";
//!      - `bad`/`zero` counters catch the failure a nonce cannot: pages served
//!        back as ZEROS or from the wrong offset. Lazy paging that silently
//!        hands the guest a zero page is a data-corruption bug that leaves the
//!        nonce intact if the nonce's own page happened to arrive;
//!      - `walked_mib` in the tick line lets the host correlate RSS against
//!        guest progress at the same instants.
//!
//! Tick period is 25 ms, not increment-g's 500 ms. The headline number is
//! resume-to-first-tick, and the tick period IS the quantisation floor on it —
//! at 500 ms every mode would report "half a second" and the comparison would
//! be measuring the probe.
//!
//! Deliberately never exits. The host drives pause/snapshot/kill/restore around
//! a running guest, which is the actual lifecycle under test.

use std::ffi::CString;
use std::io::Read;

/// 25 ms. See the module docs: this is the resolution floor on the headline
/// resume-latency measurement.
const TICK_MS: u64 = 25;

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

/// 16 bytes of entropy, read ONCE at boot, held only in RAM, never written to
/// any device. This is the load-bearing anti-false-pass value, inherited from
/// increment-g unchanged.
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

fn cmdline_val(key: &str) -> Option<String> {
    let needle = format!("{key}=");
    std::fs::read_to_string("/proc/cmdline")
        .ok()?
        .split_whitespace()
        .find_map(|t| t.strip_prefix(&needle).map(str::to_owned))
}

fn cmdline_u64(key: &str, dflt: u64) -> u64 {
    cmdline_val(key).and_then(|v| v.parse().ok()).unwrap_or(dflt)
}

fn meminfo_kb(key: &str) -> u64 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(key))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
        })
        .unwrap_or(0)
}

const PAGE: usize = 4096;

/// The value every page carries. Derived from the page index so that a page
/// served back from the WRONG offset is detectable, not just a zeroed one.
/// `251` is prime and < 256, so the cycle length does not divide any power of
/// two — an off-by-a-power-of-two misplacement cannot alias onto itself.
#[inline(always)]
fn page_byte(page_idx: usize) -> u8 {
    (page_idx % 251) as u8
}

/// mmap `bytes` of anonymous memory and write one index-derived byte per 4 KiB
/// page. Deliberately NEVER unmapped: the pages must stay resident so the
/// snapshot captures them.
///
/// Returns the base pointer, or None if the mapping failed.
fn touch_memory(bytes: usize) -> Option<*mut u8> {
    if bytes == 0 {
        return None;
    }
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
        console(&format!(
            "init: mmap({bytes}) FAILED errno={}",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
        ));
        return None;
    }
    let base = p as *mut u8;
    let t0 = clock_ms(libc::CLOCK_MONOTONIC);
    let pages = bytes / PAGE;
    for i in 0..pages {
        unsafe { std::ptr::write_volatile(base.add(i * PAGE), page_byte(i)) };
    }
    let t1 = clock_ms(libc::CLOCK_MONOTONIC);
    console(&format!(
        "init: TOUCHED {} MiB ({pages} pages, one index-derived byte each) in {} ms",
        bytes / 1024 / 1024,
        t1 - t0
    ));
    Some(base)
}

/// Read+verify `bytes` starting at page `from_page`, wrapping at `total_pages`.
/// Returns `(pages_checked, bad, zero, next_page)`.
///
/// `read_volatile` so the verification cannot be optimised out — a walk the
/// compiler elided would produce no demand faults at all and the whole RSS
/// trajectory would be an artifact of the probe.
fn walk_verify(
    base: *mut u8,
    total_pages: usize,
    from_page: usize,
    bytes: usize,
) -> (usize, usize, usize, usize) {
    let want = bytes / PAGE;
    let mut bad = 0usize;
    let mut zero = 0usize;
    let mut p = from_page;
    for _ in 0..want {
        if p >= total_pages {
            p = 0;
        }
        let got = unsafe { std::ptr::read_volatile(base.add(p * PAGE)) };
        let expect = page_byte(p);
        if got != expect {
            bad += 1;
            // A page served back as ZERO is the specific signature of lazy
            // paging that never fetched the snapshot's content. Counted
            // separately from "wrong content" because they are different bugs.
            if got == 0 && expect != 0 {
                zero += 1;
            }
        }
        p += 1;
    }
    (want, bad, zero, p)
}

fn main() {
    attach_console();
    mount_fs("proc", "/proc", "proc");
    mount_fs("sysfs", "/sys", "sysfs");
    mount_fs("devtmpfs", "/dev", "devtmpfs");

    let nonce = boot_nonce();

    // The banner is counted post-restore and MUST be 0. The restored VM
    // truncates the console file, so any banner in the post-restore transcript
    // means main() ran again, i.e. it rebooted.
    console("=========================================================");
    console(&format!("init: OND PROBE up. BOOT_NONCE={nonce}"));
    console(&format!(
        "init: MemTotal_kB={} MemFree_kB={}",
        meminfo_kb("MemTotal"),
        meminfo_kb("MemFree")
    ));

    let touch_mib = cmdline_u64("spike.touch", 0) as usize;
    let walk_mib = cmdline_u64("spike.walk", 2) as usize;
    console(&format!("init: spike.touch={touch_mib} MiB  spike.walk={walk_mib} MiB/tick"));

    let touch_bytes = touch_mib * 1024 * 1024;
    let base = touch_memory(touch_bytes);
    let total_pages = touch_bytes / PAGE;
    console(&format!("init: post-touch MemFree_kB={}", meminfo_kb("MemFree")));
    console("=========================================================");

    let walk_bytes = walk_mib * 1024 * 1024;
    let mut cursor = 0usize;
    let mut walked_pages: u64 = 0;
    let mut bad_total: u64 = 0;
    let mut zero_total: u64 = 0;

    let mut n: u64 = 0;
    loop {
        if let Some(b) = base {
            let (did, bad, zero, next) = walk_verify(b, total_pages, cursor, walk_bytes);
            cursor = next;
            walked_pages += did as u64;
            bad_total += bad as u64;
            zero_total += zero as u64;
        }
        console(&format!(
            "TICK n={n} nonce={nonce} mono_ms={} real_ms={} walked_mib={} bad={bad_total} zero={zero_total}",
            clock_ms(libc::CLOCK_MONOTONIC),
            clock_ms(libc::CLOCK_REALTIME),
            walked_pages * PAGE as u64 / 1024 / 1024,
        ));
        n += 1;
        sleep_ms(TICK_MS);
    }
}
