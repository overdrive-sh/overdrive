//! PROBE increment-l — guest PID 1 for the MEMORY-AS-CACHE model.
//!
//! The claim under test: **the memory snapshot is a cache, the filesystem is
//! authoritative.** Throw the memory away and the VM cold-boots onto a volume
//! that is still exactly right.
//!
//! To make that falsifiable the guest holds TWO counters, and the polarity of
//! the assertion INVERTS between them:
//!
//!   * `nonce` / `tick` — **RAM only.** 16 bytes of /dev/urandom read once at
//!     boot and never written anywhere, plus a loop counter. Inherited verbatim
//!     in spirit from increment-g, where it earned its keep: a restored VM and a
//!     rebooted VM look nearly identical from outside, and several intermediate
//!     attempts produced a live VMM with no restored guest that would have read
//!     as success. Same nonce + continued tick == memory genuinely came back.
//!
//!   * `seq` — **DURABLE.** A monotonic work counter appended to the volume as
//!     fixed-width checksummed records, and read back off the volume at boot.
//!
//! So for the resume arm the correct result is *nonce identical, tick continued*;
//! for every memory-loss arm the correct result is *nonce DIFFERENT, tick
//! restarted* — it rebooted, on purpose — **and `seq` continued anyway**. That
//! last conjunction IS the memory-as-cache claim. If the nonce were identical in
//! the discard arm, memory had accidentally been restored and nothing was
//! measured.
//!
//! ## Why records rather than a single number
//!
//! "The counter file still exists" is a weak claim. A fixed-width record with
//! its own checksum makes three distinct failure modes separable, which a single
//! number cannot:
//!
//!   * **loss** — the tail is missing (`highest_seq` below what the guest
//!     announced on the console);
//!   * **tearing** — a partial record at the tail (`file_len % 64 != 0`), i.e.
//!     the fs handed back half a write;
//!   * **holes** — a record missing from the MIDDLE. Prefix durability is what
//!     everyone assumes ext4 gives; a gap would mean out-of-order durability,
//!     which is a categorically worse story than a short tail. It is measured,
//!     not assumed.
//!
//! Each record is 64 printable ASCII bytes so the evidence can be pasted.
//!
//! ## Write mode and quiesce, both from the kernel cmdline
//!
//!   `spike.sync=1`         fsync(2) after every record  (durable-per-write)
//!   `spike.sync=0`         buffered — write(2) only, no fsync at all
//!   `spike.quiesce_at=N`   at seq N: syncfs(2) then FIFREEZE on the volume,
//!                          announce it, and then STOP writing. This is exactly
//!                          the shape of the guest-agent `fs_quiesce` RPC that
//!                          GH #100 proposes, so the C-vs-D comparison measures
//!                          whether that agent is MANDATORY or merely nice.
//!
//! ## Ordering: first durable write happens BEFORE the memory touch
//!
//! "Workload ready" is defined as the guest's first post-start work-counter
//! write, and the cold-boot-vs-restore comparison is timed to it. A guest that
//! pre-faulted a gigabyte of heap before its first write would charge cold boot
//! for warming a heap that a real workload populates by DOING work. So the order
//! is: mount -> recover -> FIRST WORK RECORD (ready) -> touch -> steady loop.
//! The touch still happens long before any snapshot is taken, so the snapshot
//! under test still contains real pages rather than zeros (the flattering case
//! for demand paging, per P13).
//!
//! Deliberately never exits. The host drives snapshot / discard / SIGKILL /
//! cold-boot around a running guest, which is the actual lifecycle under test.

use std::ffi::CString;
use std::io::Read;

/// Volume mount point. `/dev/vdb` — the second `--disk`, rootfs being vda.
const MNT_VOL: &str = "/mnt/vol";
const WORK_PATH: &str = "/mnt/vol/work.log";

/// One record. 64 printable ASCII bytes + the trailing newline INSIDE the 64,
/// so the file is a clean multiple of 64 and `len % 64 != 0` is unambiguously a
/// torn tail rather than a formatting artifact.
const REC: usize = 64;

// ---------------------------------------------------------------------------
// console
// ---------------------------------------------------------------------------

/// ONE write(2) per line, newline included.
///
/// increment-g/k emitted the message and the newline as two separate writes.
/// That was safe there because those probes ran at `loglevel=4`. This probe runs
/// at `loglevel=7` on purpose — the kernel's own "EXT4-fs (vdb): recovery
/// complete" is direct evidence for the journal question — and printk can land
/// between two writes, splicing a kernel line into the middle of a guest line
/// and corrupting whichever parser reads it. Single write, single line.
fn console(msg: &str) {
    let mut b = String::with_capacity(msg.len() + 1);
    b.push_str(msg);
    b.push('\n');
    let bytes = b.as_bytes();
    unsafe {
        libc::write(1, bytes.as_ptr().cast(), bytes.len());
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

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
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
/// any device. The load-bearing anti-false-pass value.
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

// ---------------------------------------------------------------------------
// the record format — MUST stay byte-identical to host_cache.rs's scanner
// ---------------------------------------------------------------------------
//
//   0..4    "REC "
//   4..16   seq,   12 ASCII digits, zero padded
//   16      ' '
//   17..23  epoch,  6 ASCII digits, zero padded
//   23      ' '
//   24..55  31 bytes of index-derived filler (lowercase letters)
//   55..63  fnv1a32(bytes[0..55]) as 8 lowercase hex
//   63      '\n'

fn fnv1a32(b: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &x in b {
        h ^= x as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn write_dec(dst: &mut [u8], mut v: u64) {
    for slot in dst.iter_mut().rev() {
        *slot = b'0' + (v % 10) as u8;
        v /= 10;
    }
}

fn write_hex8(dst: &mut [u8], v: u32) {
    const H: &[u8; 16] = b"0123456789abcdef";
    for i in 0..8 {
        dst[7 - i] = H[((v >> (i * 4)) & 0xf) as usize];
    }
}

/// Filler byte for record `seq`/`epoch` at offset `i`.
///
/// Derived from the record identity rather than constant, so a record served
/// back from the WRONG offset is detectable and not merely a zeroed one. The
/// checksum would catch it too; carrying both means a checksum collision is not
/// a single point of failure.
#[inline]
fn fill_byte(seq: u64, epoch: u64, i: usize) -> u8 {
    b'a' + ((seq.wrapping_mul(31).wrapping_add(epoch.wrapping_mul(7)).wrapping_add(i as u64)) % 26)
        as u8
}

fn build_record(seq: u64, epoch: u64) -> [u8; REC] {
    let mut r = [0u8; REC];
    r[0..4].copy_from_slice(b"REC ");
    write_dec(&mut r[4..16], seq);
    r[16] = b' ';
    write_dec(&mut r[17..23], epoch);
    r[23] = b' ';
    for i in 0..31 {
        r[24 + i] = fill_byte(seq, epoch, i);
    }
    let sum = fnv1a32(&r[0..55]);
    write_hex8(&mut r[55..63], sum);
    r[63] = b'\n';
    r
}

/// Verify one 64-byte chunk. Returns `Some((seq, epoch))` when it is a whole,
/// self-consistent record.
fn parse_record(c: &[u8]) -> Option<(u64, u64)> {
    if c.len() != REC || &c[0..4] != b"REC " || c[63] != b'\n' {
        return None;
    }
    if fnv1a32(&c[0..55]) != u32::from_str_radix(std::str::from_utf8(&c[55..63]).ok()?, 16).ok()? {
        return None;
    }
    let seq = std::str::from_utf8(&c[4..16]).ok()?.parse().ok()?;
    let epoch = std::str::from_utf8(&c[17..23]).ok()?.parse().ok()?;
    Some((seq, epoch))
}

#[derive(Default)]
struct Recovered {
    file_bytes: u64,
    records_ok: u64,
    corrupt: u64,
    torn_tail_bytes: u64,
    highest_seq: i64,
    max_epoch: u64,
    gaps: u64,
    first_gap: i64,
}

/// Scan the journal. Everything a caller could want to conclude is reported as a
/// separate number: a scan that returns only "highest seq" cannot distinguish a
/// clean short tail from a hole in the middle, and those are different bugs.
fn recover(path: &str) -> Recovered {
    let mut r = Recovered { highest_seq: -1, first_gap: -1, ..Default::default() };
    let buf = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return r,
    };
    r.file_bytes = buf.len() as u64;
    r.torn_tail_bytes = (buf.len() % REC) as u64;
    let n = buf.len() / REC;
    let mut seen = vec![false; n + 1];
    for i in 0..n {
        match parse_record(&buf[i * REC..(i + 1) * REC]) {
            Some((seq, epoch)) => {
                r.records_ok += 1;
                if (seq as i64) > r.highest_seq {
                    r.highest_seq = seq as i64;
                }
                if epoch > r.max_epoch {
                    r.max_epoch = epoch;
                }
                if (seq as usize) < seen.len() {
                    seen[seq as usize] = true;
                }
            }
            None => r.corrupt += 1,
        }
    }
    if r.highest_seq >= 0 {
        for (s, hit) in seen.iter().enumerate().take(r.highest_seq as usize + 1) {
            if !hit {
                r.gaps += 1;
                if r.first_gap < 0 {
                    r.first_gap = s as i64;
                }
            }
        }
    }
    r
}

// ---------------------------------------------------------------------------
// quiesce — syncfs(2) + FIFREEZE, the guest half of an `fs_quiesce` RPC
// ---------------------------------------------------------------------------

/// `_IOWR('X', 119, int)` == 0xC0045877. Value pinned by hand from the macro
/// rather than recalled: 'X' is 0x58, dir=3<<30, size=4<<16.
const FIFREEZE: u32 = 0xC004_5877;

/// Freeze ONLY the volume, never the root filesystem — freezing the fs the
/// running binary lives on deadlocks the guest against its own next page-in.
/// That is the same scoping a real `fs_quiesce` would use: quiesce the data
/// volume, leave the read-mostly rootfs alone.
fn quiesce(vol_fd: libc::c_int) -> (i32, i32, u64, i32, i32) {
    let t0 = clock_ms(libc::CLOCK_MONOTONIC);
    let s_rc = unsafe { libc::syscall(libc::SYS_syncfs, vol_fd) } as i32;
    let s_err = if s_rc < 0 { errno() } else { 0 };
    let s_ms = clock_ms(libc::CLOCK_MONOTONIC) - t0;
    let f_rc = unsafe { libc::ioctl(vol_fd, FIFREEZE as _, 0 as libc::c_int) };
    let f_err = if f_rc < 0 { errno() } else { 0 };
    (s_rc, s_err, s_ms, f_rc, f_err)
}

// ---------------------------------------------------------------------------

fn main() {
    attach_console();
    mount_fs("proc", "/proc", "proc");
    mount_fs("sysfs", "/sys", "sysfs");
    mount_fs("devtmpfs", "/dev", "devtmpfs");

    let t_init = clock_ms(libc::CLOCK_MONOTONIC);
    let nonce = boot_nonce();

    // Counted post-cut and MUST be 0 in the resume arm / MUST be 1 in every
    // memory-loss arm. main() running again IS the reboot.
    console("=========================================================");
    console(&format!("init: L PROBE up. BOOT_NONCE={nonce}"));
    console(&format!(
        "init: t_init_mono_ms={t_init} MemTotal_kB={} MemFree_kB={}",
        meminfo_kb("MemTotal"),
        meminfo_kb("MemFree")
    ));

    let do_sync = cmdline_u64("spike.sync", 1) == 1;
    let touch_mib = cmdline_u64("spike.touch", 0) as usize;
    let work_ms = cmdline_u64("spike.work_ms", 20);
    let quiesce_at = cmdline_u64("spike.quiesce_at", 0);
    console(&format!(
        "init: spike.sync={} spike.touch={touch_mib} spike.work_ms={work_ms} spike.quiesce_at={quiesce_at}",
        if do_sync { 1 } else { 0 }
    ));

    // ---- mount the volume -------------------------------------------------
    let _ = std::fs::create_dir_all(MNT_VOL);
    let t_m0 = clock_ms(libc::CLOCK_MONOTONIC);
    let mrc = mount_fs("/dev/vdb", MNT_VOL, "ext4");
    let merr = if mrc != 0 { errno() } else { 0 };
    let t_m1 = clock_ms(libc::CLOCK_MONOTONIC);
    console(&format!(
        "init: MOUNT /dev/vdb ext4 -> rc={mrc} errno={merr} mount_ms={} mono_ms={t_m1}",
        t_m1 - t_m0
    ));
    if mrc != 0 {
        // A failed mount would make every downstream number vacuous — "0 records
        // durable" would read as total data loss when it is really a harness gap.
        // P9 lost a probe to exactly this shape (a missing virtiofs.ko turned a
        // working arm into a confident wrong negative), so it is fatal and loud.
        console("init: FATAL volume mount failed — every record count below would be VACUOUS");
        loop {
            console(&format!(
                "WORK seq=-1 epoch=0 synced=0 nonce={nonce} tick=0 mono_ms={} MOUNT_FAILED",
                clock_ms(libc::CLOCK_MONOTONIC)
            ));
            sleep_ms(500);
        }
    }

    // ---- recover ----------------------------------------------------------
    let t_r0 = clock_ms(libc::CLOCK_MONOTONIC);
    let rec = recover(WORK_PATH);
    let t_r1 = clock_ms(libc::CLOCK_MONOTONIC);
    console(&format!(
        "init: RECOVER file_bytes={} records_ok={} corrupt={} torn_tail_bytes={} highest_seq={} \
max_epoch={} gaps={} first_gap={} scan_ms={}",
        rec.file_bytes,
        rec.records_ok,
        rec.corrupt,
        rec.torn_tail_bytes,
        rec.highest_seq,
        rec.max_epoch,
        rec.gaps,
        rec.first_gap,
        t_r1 - t_r0
    ));

    let epoch = rec.max_epoch + 1;
    let mut seq: u64 = (rec.highest_seq + 1) as u64;
    // Append at a 64-aligned offset. A torn tail from a crash would otherwise
    // shift every subsequent record and make the whole file unparseable — the
    // recovery must be able to write PAST the damage, which is what a real
    // append-only workload does.
    let mut off: i64 = ((rec.file_bytes / REC as u64) * REC as u64) as i64;
    console(&format!("init: CONTINUE epoch={epoch} next_seq={seq} append_off={off}"));

    let path = CString::new(WORK_PATH).unwrap();
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CREAT, 0o644) };
    if fd < 0 {
        console(&format!("init: FATAL open {WORK_PATH} errno={}", errno()));
        loop {
            sleep_ms(1000);
        }
    }
    let dir = CString::new(MNT_VOL).unwrap();
    let dir_fd = unsafe { libc::open(dir.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY) };

    let mut tick: u64 = 0;
    let mut frozen = false;

    // One closure-shaped step so the FIRST record (the readiness event) and the
    // steady-state records go through identical code. If they differed, the
    // timing bench would be measuring a different operation from the one the
    // durability arms measure.
    let mut write_one = |seq: u64, tick: u64, off: &mut i64| -> (i32, i32) {
        let r = build_record(seq, epoch);
        let w = unsafe { libc::pwrite(fd, r.as_ptr().cast(), REC, *off as libc::off_t) };
        let werr = if w < 0 { errno() } else { 0 };
        if w == REC as isize {
            *off += REC as i64;
        }
        let mut ferr = 0;
        if do_sync {
            if unsafe { libc::fsync(fd) } < 0 {
                ferr = errno();
            }
        }
        // Announced AFTER the write (and after the fsync when there is one) so
        // the console is a LOWER bound on what reached the device, never an
        // upper one. Announcing first would let a lagging console understate the
        // loss — an error in the flattering direction, which is the one that
        // must not be available here.
        console(&format!(
            "WORK seq={seq} epoch={epoch} synced={} nonce={nonce} tick={tick} mono_ms={} real_ms={} w={w} werr={werr} ferr={ferr}",
            if do_sync { 1 } else { 0 },
            clock_ms(libc::CLOCK_MONOTONIC),
            clock_ms(libc::CLOCK_REALTIME),
        ));
        (werr, ferr)
    };

    // ---- FIRST record == "workload ready". Timed by the bench. ------------
    write_one(seq, tick, &mut off);
    let ready = clock_ms(libc::CLOCK_MONOTONIC);
    console(&format!(
        "init: READY_AT_MONO_MS={ready} first_seq={seq} boot_to_ready_ms={}",
        ready - t_init
    ));
    seq += 1;
    tick += 1;

    // ---- touch, AFTER ready (see module docs) -----------------------------
    if touch_mib > 0 {
        let bytes = touch_mib * 1024 * 1024;
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
            console(&format!("init: mmap({bytes}) FAILED errno={}", errno()));
        } else {
            let base = p as *mut u8;
            let t0 = clock_ms(libc::CLOCK_MONOTONIC);
            let pages = bytes / 4096;
            for i in 0..pages {
                unsafe { std::ptr::write_volatile(base.add(i * 4096), (i % 251) as u8) };
            }
            console(&format!(
                "init: TOUCHED {touch_mib} MiB ({pages} pages) in {} ms; MemFree_kB={}",
                clock_ms(libc::CLOCK_MONOTONIC) - t0,
                meminfo_kb("MemFree")
            ));
        }
    }
    console("=========================================================");

    // ---- steady work loop -------------------------------------------------
    loop {
        sleep_ms(work_ms);
        if frozen {
            // Post-quiesce the fs is frozen; a write here would block forever.
            // Keep ticking so the host can still see the guest is alive and can
            // still tell a frozen guest from a dead one.
            console(&format!(
                "TICK n={tick} nonce={nonce} mono_ms={} FROZEN last_seq={}",
                clock_ms(libc::CLOCK_MONOTONIC),
                seq - 1
            ));
            tick += 1;
            continue;
        }
        write_one(seq, tick, &mut off);
        if quiesce_at > 0 && seq >= quiesce_at {
            let (s_rc, s_err, s_ms, f_rc, f_err) = quiesce(if dir_fd >= 0 { dir_fd } else { fd });
            console(&format!(
                "QUIESCE seq={seq} syncfs_rc={s_rc} syncfs_errno={s_err} syncfs_ms={s_ms} \
freeze_rc={f_rc} freeze_errno={f_err} mono_ms={}",
                clock_ms(libc::CLOCK_MONOTONIC)
            ));
            frozen = true;
        }
        seq += 1;
        tick += 1;
    }
}
