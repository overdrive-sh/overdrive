//! PROBE increment-l — host harness for the MEMORY-AS-CACHE model.
//!
//! Four arms, each a different way for the guest's memory to stop existing, and
//! one question asked of all four: **is the VOLUME still exactly right?**
//!
//!   A  resume        pause -> snapshot -> kill -> restore -> resume.
//!                    The control. Memory comes back. Nonce identical.
//!   B  discard       pause -> snapshot -> kill -> DELETE memory-ranges ->
//!                    cold boot from the same rootfs + volume. This is the
//!                    memory-as-cache model executed literally: the bytes that
//!                    would have made a warm pool expensive are erased, and the
//!                    VM has to come back from the filesystem alone.
//!   C  crash         no pause, no snapshot: SIGKILL the VMM mid-write, then
//!                    cold boot. Models losing the VM with no warning.
//!   D  quiesce+crash guest issues syncfs(2)+FIFREEZE first, THEN SIGKILL, then
//!                    cold boot.
//!
//! **C vs D is the load-bearing comparison.** It is the only thing in this probe
//! that decides whether the guest-side `fs_quiesce` of GH #100 is MANDATORY or
//! merely nice, so the two are never blurred: they differ in exactly one step.
//!
//! Every arm runs in BOTH write modes (`--sync 1` fsync-per-write, `--sync 0`
//! buffered), because the interesting question is not "does a filesystem
//! survive" but "what does the guest have to DO for it to survive".
//!
//! ## The control that could falsify the headline
//!
//! P11 nearly published 2958 MiB/s on a 1163 MiB/s device; it survived five
//! interleaved trials and a disjoint-ranges check, and only an explicit control
//! caught it. The equivalent control here is **A-buffered vs B-buffered**. Those
//! two runs are byte-identical up to one step — whether `memory-ranges` is
//! restored or deleted — so any difference in surviving records is attributable
//! to the memory discard and to nothing else in this harness. If B-buffered
//! loses records and A-buffered does not, the loss is real. If BOTH lose, the
//! harness is broken, not the model.
//!
//! ## Forensics are done on a COPY, in an order that cannot destroy its own
//! ## evidence
//!
//! Mounting an ext4 image with a dirty journal REPLAYS the journal and rewrites
//! the image. Doing that before measuring would silently convert "needed
//! recovery" into "was clean". So the volume image is reflink-copied the instant
//! the cut lands, and the copy is walked in this order:
//!
//!   1. `dumpe2fs -h`      -> Filesystem state, and whether `needs_recovery` is
//!                            set in the feature list.
//!   2. `e2fsck -fn`       -> read-only check; the exit code IS the verdict.
//!   3. mount `ro,noload`  -> scan. `noload` SKIPS journal replay, so this is
//!                            the strictly-durable set: what survived without
//!                            any recovery at all.
//!   4. mount rw           -> replays the journal -> scan again. The delta
//!                            between 3 and 4 is exactly what the journal
//!                            recovered, which is the number that distinguishes
//!                            "consistent" from "consistent after recovery".
//!
//! ## Trap 6 — the box is SHARED
//!
//! A previous probe was corrupted when a concurrent agent's
//! `pkill -9 -x cloud-hyperviso` SIGKILLed its VMM mid-run, producing an empty
//! log that read exactly like a genuine failure. So this harness kills ONLY pids
//! it spawned, by pid, never by name; `kill(pid,0)`-checks its VMM at every
//! wait; and labels an externally-killed run `HARNESS_DEFECT=EXTERNAL_KILL` so
//! it is discarded rather than averaged in.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

// ===========================================================================
// HTTP/1.1 over a unix socket (CH's API; `ch-remote` is not on the box)
// ===========================================================================

/// Returns `(status, body, elapsed)`. Parsed by headers, not read-to-EOF: if CH
/// did not honour `Connection: close` a read-to-EOF would block until the read
/// timeout and that timeout would silently become the reported latency.
fn api(sock: &Path, method: &str, path: &str, body: Option<&str>) -> (u16, String, Duration) {
    let t0 = Instant::now();
    let mut s = match UnixStream::connect(sock) {
        Ok(s) => s,
        Err(e) => return (0, format!("<connect failed: {e}>"), t0.elapsed()),
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(600)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(60)));
    let b = body.unwrap_or("");
    let req = format!(
        "{method} /api/v1/{path} HTTP/1.1\r\nHost: localhost\r\nAccept: */*\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}",
        b.len()
    );
    if let Err(e) = s.write_all(req.as_bytes()) {
        return (0, format!("<write failed: {e}>"), t0.elapsed());
    }
    let _ = s.flush();
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(p) = find(&buf, b"\r\n\r\n") {
            break p + 4;
        }
        match s.read(&mut chunk) {
            Ok(0) => break buf.len(),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) => return (0, format!("<read failed: {e}>"), t0.elapsed()),
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let status: u16 = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let clen: Option<usize> = head.lines().find_map(|l| {
        let (k, v) = l.trim().split_once(':')?;
        if k.eq_ignore_ascii_case("content-length") { v.trim().parse().ok() } else { None }
    });
    let want = match clen {
        Some(n) => n,
        None if status == 204 || status == 304 => 0,
        None => usize::MAX,
    };
    while buf.len() - head_end < want {
        match s.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    (status, String::from_utf8_lossy(&buf[head_end..]).trim().to_string(), t0.elapsed())
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

// ===========================================================================
// process / fs helpers
// ===========================================================================

fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

fn kill_pid(pid: i32) {
    unsafe { libc::kill(pid, libc::SIGKILL) };
}

/// `comm` is truncated to 15 chars by the kernel, so the name to match is
/// `cloud-hyperviso`. Matching `cloud-hypervisor` NEVER matches anything (P8).
fn foreign_vmms(mine: &[i32]) -> Vec<i32> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/proc") {
        for e in rd.flatten() {
            if let Ok(pid) = e.file_name().to_string_lossy().parse::<i32>() {
                if mine.contains(&pid) {
                    continue;
                }
                if let Ok(c) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
                    if c.trim() == "cloud-hyperviso" {
                        out.push(pid);
                    }
                }
            }
        }
    }
    out
}

fn wait_for_socket(p: &Path, secs: u64) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(secs) {
        if p.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

fn sh(cmd: &str, args: &[&str]) -> (i32, String) {
    let o = Command::new(cmd).args(args).output();
    match o {
        Ok(o) => (
            o.status.code().unwrap_or(-1),
            format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)),
        ),
        Err(e) => (-1, format!("<spawn failed: {e}>")),
    }
}

/// Free bytes on the filesystem holding `p`. `du` cannot see reflinked extents
/// (trap 10), so real space is only ever measured as a `df` delta.
fn fs_avail_bytes(p: &Path) -> u64 {
    let c = std::ffi::CString::new(p.to_string_lossy().as_bytes()).unwrap();
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return 0;
    }
    st.f_bavail as u64 * st.f_frsize as u64
}

fn dir_bytes(p: &Path) -> (u64, u64) {
    let (mut app, mut disk) = (0u64, 0u64);
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            if let Ok(m) = e.metadata() {
                use std::os::unix::fs::MetadataExt;
                app += m.len();
                disk += m.blocks() * 512;
            }
        }
    }
    (app, disk)
}

fn kb_field(s: &str, key: &str) -> u64 {
    s.lines()
        .find(|l| l.starts_with(key))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn vmm_rss_kb(pid: i32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .map(|s| kb_field(&s, "VmRSS:"))
        .unwrap_or(0)
}

/// Open flags of every fd the VMM holds on the volume image.
///
/// The guest fsync lands in the HOST page cache unless CH opened the backing
/// file `O_DIRECT`. That is the difference between "survives losing the VM" and
/// "survives losing the host", and it is not something to assume from the
/// `direct=` option string — read the flags the kernel actually recorded.
fn disk_fd_flags(pid: i32, want: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(format!("/proc/{pid}/fd")) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if let Ok(t) = std::fs::read_link(e.path()) {
                if t == want {
                    let fi = std::fs::read_to_string(format!("/proc/{pid}/fdinfo/{n}"))
                        .unwrap_or_default();
                    let flags = fi
                        .lines()
                        .find(|l| l.starts_with("flags:"))
                        .unwrap_or("flags:?")
                        .to_string();
                    // O_DIRECT is 0o40000 on x86_64.
                    let raw = flags
                        .split_whitespace()
                        .nth(1)
                        .and_then(|v| u32::from_str_radix(v, 8).ok())
                        .unwrap_or(0);
                    out.push(format!(
                        "fd={n} {flags} O_DIRECT={}",
                        if raw & 0o40000 != 0 { "YES" } else { "no" }
                    ));
                }
            }
        }
    }
    out
}

// ===========================================================================
// console parsing
// ===========================================================================

/// Read the transcript, DISCARDING any trailing partial line.
///
/// The guest writes this file continuously; a read landing mid-write returns a
/// truncated final line. increment-k documented both directions this corrupts:
/// a truncated nonce can never match a complete one (false negative), and a
/// truncated `seq=1234` parses as `seq=12` — which here would understate the
/// durable set and INVENT data loss. Truncating at the last newline makes every
/// parser below see only whole lines, which is what they all assume.
fn read_console(p: &Path) -> String {
    let raw = std::fs::read(p).map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default();
    match raw.rfind('\n') {
        Some(i) => raw[..=i].to_string(),
        None => String::new(),
    }
}

fn work_seqs(txt: &str) -> Vec<i64> {
    txt.lines()
        .filter(|l| l.starts_with("WORK seq="))
        .filter_map(|l| l.strip_prefix("WORK seq="))
        .filter_map(|r| r.split_whitespace().next())
        .filter_map(|v| v.parse::<i64>().ok())
        .collect()
}

fn max_work_seq(txt: &str) -> i64 {
    work_seqs(txt).into_iter().max().unwrap_or(-1)
}

fn last_field(txt: &str, line_prefix: &str, key: &str) -> String {
    txt.lines()
        .filter(|l| l.starts_with(line_prefix))
        .next_back()
        .and_then(|l| {
            l.split_whitespace().find_map(|t| t.strip_prefix(&format!("{key}=")).map(str::to_owned))
        })
        .unwrap_or_else(|| "<none>".into())
}

/// Field of the FIRST matching line, not the last.
///
/// HARNESS DEFECT, caught in the first full run and fixed here. The RAM-tick
/// polarity check compared the LAST tick before the cut against the LAST tick
/// after it. In the cold-boot arms the recovered guest restarts its RAM tick at
/// 0 — correctly — but then runs long enough to climb PAST the pre-cut value, so
/// `tick_after > tick_before` reported "CONTINUED" for a guest that had plainly
/// rebooted, and three genuinely-correct trials were flagged `polarity_ok=false`.
/// The tick that carries the signal is the one on the FIRST post-cut record: 0
/// means restarted, anything else means resumed mid-stream.
fn first_field(txt: &str, line_prefix: &str, key: &str) -> String {
    txt.lines()
        .find(|l| l.starts_with(line_prefix))
        .and_then(|l| {
            l.split_whitespace().find_map(|t| t.strip_prefix(&format!("{key}=")).map(str::to_owned))
        })
        .unwrap_or_else(|| "<none>".into())
}

fn line_containing(txt: &str, needle: &str) -> String {
    txt.lines().find(|l| l.contains(needle)).unwrap_or("<absent>").trim().to_string()
}

fn banners(txt: &str) -> usize {
    txt.matches("L PROBE up").count()
}

// ===========================================================================
// the record scanner — MUST stay byte-identical to guest_init_l.rs
// ===========================================================================

const REC: usize = 64;

fn fnv1a32(b: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &x in b {
        h ^= x as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

#[derive(Clone)]
struct Scan {
    /// The mount itself succeeded. Without this, a FAILED MOUNT and a genuinely
    /// empty filesystem are the same row of numbers — and the first run of this
    /// probe produced exactly that confusion twice (a loop-device reuse made an
    /// rw mount fail, and the trial read as total data loss).
    mount_ok: bool,
    present: bool,
    file_bytes: u64,
    records_ok: u64,
    corrupt: u64,
    torn_tail_bytes: u64,
    highest_seq: i64,
    max_epoch: u64,
    gaps: u64,
    first_gap: i64,
    first_corrupt_at: i64,
    first_corrupt_all_zero: bool,
    first_corrupt_hex: String,
}

impl Default for Scan {
    /// `highest_seq` defaults to **-1**, not 0. `0` is a legitimate seq, so a
    /// derived `Default` makes "no records at all" indistinguishable from "one
    /// record, number zero" in every downstream subtraction.
    fn default() -> Self {
        Self {
            mount_ok: false,
            present: false,
            file_bytes: 0,
            records_ok: 0,
            corrupt: 0,
            torn_tail_bytes: 0,
            highest_seq: -1,
            max_epoch: 0,
            gaps: 0,
            first_gap: -1,
            first_corrupt_at: -1,
            first_corrupt_all_zero: false,
            first_corrupt_hex: String::new(),
        }
    }
}

impl Scan {
    fn line(&self) -> String {
        format!(
            "mount_ok={} present={} bytes={} ok={} corrupt={} torn_tail={} highest_seq={} max_epoch={} gaps={} first_gap={} first_corrupt_at={} first_corrupt_all_zero={} first_corrupt_hex={}",
            self.mount_ok,
            self.present,
            self.file_bytes,
            self.records_ok,
            self.corrupt,
            self.torn_tail_bytes,
            self.highest_seq,
            self.max_epoch,
            self.gaps,
            self.first_gap,
            self.first_corrupt_at,
            self.first_corrupt_all_zero,
            if self.first_corrupt_hex.is_empty() { "-" } else { &self.first_corrupt_hex }
        )
    }
}

fn scan_bytes(buf: &[u8]) -> Scan {
    let mut s = Scan {
        mount_ok: true,
        present: true,
        file_bytes: buf.len() as u64,
        torn_tail_bytes: (buf.len() % REC) as u64,
        ..Default::default()
    };
    let n = buf.len() / REC;
    let mut seen = vec![false; n + 1];
    for i in 0..n {
        let c = &buf[i * REC..(i + 1) * REC];
        let ok = c[0..4] == *b"REC "
            && c[63] == b'\n'
            && std::str::from_utf8(&c[55..63]).ok().and_then(|h| u32::from_str_radix(h, 16).ok())
                == Some(fnv1a32(&c[0..55]));
        if !ok {
            s.corrupt += 1;
            // A corrupt chunk has TWO very different explanations and the bytes
            // tell them apart: all-zero means ext4 allocated the block and the
            // data never arrived (the delalloc crash artifact — the tail is
            // PRESENT but zeroed, not missing), whereas non-zero garbage would
            // mean a torn or misdirected write. Recording the first one turns
            // that from an inference into an observation.
            if s.first_corrupt_at < 0 {
                s.first_corrupt_at = i as i64;
                s.first_corrupt_all_zero = c.iter().all(|b| *b == 0);
                let mut h = String::new();
                for b in c.iter().take(16) {
                    h.push_str(&format!("{b:02x}"));
                }
                s.first_corrupt_hex = h;
            }
            continue;
        }
        let seq: i64 =
            std::str::from_utf8(&c[4..16]).ok().and_then(|v| v.parse().ok()).unwrap_or(-1);
        let ep: u64 =
            std::str::from_utf8(&c[17..23]).ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        s.records_ok += 1;
        if seq > s.highest_seq {
            s.highest_seq = seq;
        }
        if ep > s.max_epoch {
            s.max_epoch = ep;
        }
        if seq >= 0 && (seq as usize) < seen.len() {
            seen[seq as usize] = true;
        }
    }
    if s.highest_seq >= 0 {
        for (i, hit) in seen.iter().enumerate().take(s.highest_seq as usize + 1) {
            if !hit {
                s.gaps += 1;
                if s.first_gap < 0 {
                    s.first_gap = i as i64;
                }
            }
        }
    }
    s
}

/// Mount `img` at `mnt` with `opts`, scan `work.log`, unmount.
///
/// `noload` in `opts` is what makes the strictly-durable read possible: it
/// tells ext4 to skip journal replay, so the scan sees the device as it was
/// left, not as recovery would repair it.
fn scan_image(img: &Path, mnt: &Path, opts: &str) -> (Scan, String) {
    let _ = std::fs::create_dir_all(mnt);
    let (rc, out) = sh("mount", &["-o", opts, img.to_str().unwrap(), mnt.to_str().unwrap()]);
    if rc != 0 {
        return (
            Scan::default(),
            format!("!!! HARNESS: mount -o {opts} FAILED rc={rc}: {}", out.trim()),
        );
    }
    let f = mnt.join("work.log");
    let raw = std::fs::read(&f);
    let (s, absent_note) = match &raw {
        Ok(b) => (scan_bytes(b), String::new()),
        Err(e) => (
            // Mounted fine, file simply is not there. Distinct from a mount
            // failure: `mount_ok` stays true so the row cannot be misread.
            Scan { mount_ok: true, ..Default::default() },
            format!("work.log ABSENT (mount succeeded): {e}"),
        ),
    };
    let head = raw
        .ok()
        .filter(|b| b.len() >= REC)
        .map(|b| String::from_utf8_lossy(&b[..REC]).trim_end().to_string())
        .unwrap_or_default();
    let (urc, uout) = sh("umount", &[mnt.to_str().unwrap()]);
    // Release OUR loop device by backing file. NEVER `losetup -D` — the box is
    // shared and that would detach a concurrent tenant's loops.
    let (_, loops) = sh("losetup", &["-j", img.to_str().unwrap()]);
    for l in loops.lines() {
        if let Some(dev) = l.split(':').next() {
            let _ = sh("losetup", &["-d", dev.trim()]);
        }
    }
    let mut note = if head.is_empty() { absent_note } else { format!("first_record={head}") };
    if urc != 0 {
        note.push_str(&format!("  (!!! umount rc={urc}: {})", uout.trim()));
    }
    (s, note)
}

// ===========================================================================
// forensics
// ===========================================================================

struct Forensics {
    state: String,
    features: String,
    needs_recovery: bool,
    fsck_rc: i32,
    fsck_out: String,
    noload: Scan,
    noload_note: String,
    replayed: Scan,
    replayed_note: String,
    state_after: String,
    needs_recovery_after: bool,
    fsck_after_rc: i32,
    fsck_after_out: String,
}

/// Walk the post-cut volume copy in the one order that cannot destroy its own
/// evidence. See the module docs.
/// `copy_a` is walked with journal replay SUPPRESSED, `copy_b` with an ordinary
/// mount that replays it.
///
/// TWO copies, not one, and that is not tidiness. util-linux reuses an existing
/// loop device for the same backing file, so a `ro,noload` mount leaves
/// `/dev/loopN` **read-only** and the subsequent rw mount of the same file fails
/// with `cannot mount /dev/loop0 read-only`. In the first run of this probe that
/// produced two trials whose forensics read as TOTAL DATA LOSS when the
/// filesystem was fine. Separate files, separate loop devices, no reuse.
fn forensics(copy_a: &Path, copy_b: &Path, mnt: &Path) -> Forensics {
    forensics_inner(copy_a, copy_b, mnt)
}

fn forensics_inner(copy: &Path, copy_b: &Path, mnt: &Path) -> Forensics {
    let feat = |d: &str| -> (String, bool) {
        let l = d
            .lines()
            .find(|l| l.starts_with("Filesystem features:"))
            .unwrap_or("Filesystem features: <unknown>")
            .trim()
            .to_string();
        let nr = l.contains("needs_recovery");
        (l, nr)
    };
    let st = |d: &str| -> String {
        d.lines()
            .find(|l| l.starts_with("Filesystem state:"))
            .unwrap_or("Filesystem state: <unknown>")
            .trim()
            .to_string()
    };

    let (_, dump) = sh("dumpe2fs", &["-h", copy.to_str().unwrap()]);
    let state = st(&dump);
    // `Filesystem state: clean` is NOT the recovery signal and reading it as one
    // is the "error codes are taxonomy, not mechanism" trap. The superblock stays
    // `clean` across an unclean shutdown; the fact that a journal replay is
    // pending lives in the FEATURE flag `needs_recovery`. Both are captured, and
    // the feature flag is the one that is believed.
    let (features, needs_recovery) = feat(&dump);

    // `-f` forces a full check even when the superblock says clean; `-n` answers
    // no to every repair, so nothing is mutated and the exit code is the verdict.
    //
    // WEAK INSTRUMENT, and named as such: on a dirty journal e2fsck prints
    // "skipping journal recovery because doing a read-only filesystem check" and
    // still exits 0. So this pass cannot answer "is the filesystem consistent" —
    // it only answers "is it consistent IGNORING the journal". The check that
    // means something is the second one below, after replay.
    let (fsck_rc, fsck_out) = sh("e2fsck", &["-fn", copy.to_str().unwrap()]);

    let (noload, noload_note) = scan_image(copy, mnt, "ro,noload,loop");
    // Separate file: see the doc comment on `forensics`.
    let (replayed, replayed_note) = scan_image(copy_b, mnt, "loop");

    let (_, dump2) = sh("dumpe2fs", &["-h", copy_b.to_str().unwrap()]);
    let state_after = st(&dump2);
    let (_, needs_recovery_after) = feat(&dump2);
    // THE meaningful structural verdict: a full check with the journal already
    // replayed and nothing left pending. A non-zero rc here is real corruption.
    let (fsck_after_rc, fsck_after_out) = sh("e2fsck", &["-fn", copy_b.to_str().unwrap()]);

    Forensics {
        state,
        features,
        needs_recovery,
        fsck_rc,
        fsck_out,
        noload,
        noload_note,
        replayed,
        replayed_note,
        state_after,
        needs_recovery_after,
        fsck_after_rc,
        fsck_after_out,
    }
}

// ===========================================================================
// args
// ===========================================================================

struct Args {
    cmd: String,
    arm: String,
    label: String,
    sync: u64,
    mem_mib: u64,
    touch_mib: u64,
    work_ms: u64,
    cut_at: u64,
    direct: u64,
    kernel: PathBuf,
    rootfs_src: PathBuf,
    vol_src: PathBuf,
    run_dir: PathBuf,
    snap_root: PathBuf,
    bench_mode: String,
    boot_ticks: u64,
}

fn arg(a: &[String], k: &str, d: &str) -> String {
    a.windows(2).find(|w| w[0] == k).map(|w| w[1].clone()).unwrap_or_else(|| d.to_string())
}

fn parse_args() -> Args {
    let a: Vec<String> = std::env::args().collect();
    Args {
        cmd: arg(&a, "--cmd", "arm"),
        arm: arg(&a, "--arm", "a"),
        label: arg(&a, "--label", "run"),
        sync: arg(&a, "--sync", "1").parse().unwrap(),
        mem_mib: arg(&a, "--mem-mib", "2048").parse().unwrap(),
        touch_mib: arg(&a, "--touch-mib", "1536").parse().unwrap(),
        work_ms: arg(&a, "--work-ms", "20").parse().unwrap(),
        cut_at: arg(&a, "--cut-at", "300").parse().unwrap(),
        direct: arg(&a, "--direct", "0").parse().unwrap(),
        kernel: arg(&a, "--kernel", "/var/tmp/spike-increment-l/kernel").into(),
        rootfs_src: arg(&a, "--rootfs-src", "/var/tmp/spike-increment-l/rootfs.ext4").into(),
        vol_src: arg(&a, "--vol-src", "/var/tmp/spike-increment-l/vol-blank.ext4").into(),
        run_dir: arg(&a, "--run-dir", "/run/spike-increment-l").into(),
        snap_root: arg(&a, "--snap-root", "/srv/vm/p14l").into(),
        bench_mode: arg(&a, "--bench-mode", "cold").parse().unwrap(),
        boot_ticks: arg(&a, "--boot-ticks", "60").parse().unwrap(),
    }
}

// ===========================================================================
// VMM launch
// ===========================================================================

struct Launch {
    child: Child,
    pid: i32,
}

#[allow(clippy::too_many_arguments)]
fn boot_vmm(
    a: &Args,
    api_sock: &Path,
    rootfs: &Path,
    vol: &Path,
    console: &Path,
    chlog: &Path,
    quiesce_at: u64,
    touch_mib: u64,
) -> Launch {
    // loglevel=7 on purpose: the kernel's own ext4 mount/recovery lines are
    // direct evidence for the journal question and cost nothing else.
    let cmdline = format!(
        "root=/dev/vda rw console=ttyS0 init=/init panic=-1 loglevel=7 \
spike.sync={} spike.touch={touch_mib} spike.work_ms={} spike.quiesce_at={quiesce_at}",
        a.sync, a.work_ms
    );
    let vol_arg = if a.direct == 1 {
        format!("path={},image_type=raw,direct=on", vol.display())
    } else {
        format!("path={},image_type=raw", vol.display())
    };
    let child = Command::new("cloud-hypervisor")
        .arg("--api-socket")
        .arg(format!("path={}", api_sock.display()))
        .arg("--cpus")
        .arg("boot=1,max=4")
        .arg("--memory")
        .arg(format!("size={}M", a.mem_mib))
        .arg("--kernel")
        .arg(&a.kernel)
        .arg("--cmdline")
        .arg(&cmdline)
        // image_type=raw is MANDATORY on v53: the auto-detect fallback disables
        // sector-0 writes and a bare-filesystem image then faults and reboots
        // two layers from the cause.
        .arg("--disk")
        .arg(format!("path={},image_type=raw", rootfs.display()))
        .arg("--disk")
        .arg(vol_arg)
        .arg("--serial")
        .arg(format!("file={}", console.display()))
        .arg("--console")
        .arg("off")
        .stdout(Stdio::from(std::fs::File::create(chlog).unwrap()))
        .stderr(Stdio::from(std::fs::File::create(chlog).unwrap()))
        .spawn()
        .expect("spawn cloud-hypervisor");
    let pid = child.id() as i32;
    Launch { child, pid }
}

/// Wait until the console reports a WORK seq at or above `floor`, or the guest
/// prints `QUIESCE`. Returns `Ok(text)` / `Err(reason)`.
fn wait_for(
    console: &Path,
    pid: i32,
    floor: i64,
    want_quiesce: bool,
    secs: u64,
) -> Result<String, String> {
    let t0 = Instant::now();
    loop {
        let txt = read_console(console);
        if want_quiesce && txt.contains("QUIESCE seq=") {
            return Ok(txt);
        }
        if !want_quiesce && max_work_seq(&txt) >= floor {
            return Ok(txt);
        }
        if !alive(pid) {
            return Err(format!("VMM pid {pid} died while waiting"));
        }
        if t0.elapsed() > Duration::from_secs(secs) {
            return Err(format!(
                "timeout after {secs}s (max_work_seq={} quiesce={})",
                max_work_seq(&txt),
                txt.contains("QUIESCE seq=")
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Wait for a literal line fragment to appear on the console.
///
/// Needed because the bench's volume is SEEDED — the guest recovers ~300
/// records and its very first WORK line is already `seq=301`, so a "wait until
/// seq >= N" gate fires instantly and the snapshot would be taken BEFORE the
/// memory touch finished. Waiting on `init: TOUCHED` is the only gate that
/// actually means "the guest is in steady state".
fn wait_for_line(console: &Path, pid: i32, needle: &str, secs: u64) -> Result<String, String> {
    let t0 = Instant::now();
    loop {
        let txt = read_console(console);
        if txt.contains(needle) {
            return Ok(txt);
        }
        if !alive(pid) {
            return Err(format!("VMM pid {pid} died waiting for `{needle}`"));
        }
        if t0.elapsed() > Duration::from_secs(secs) {
            return Err(format!("timeout {secs}s waiting for `{needle}`"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn cleanup_sock(api_sock: &Path) {
    let _ = std::fs::remove_file(api_sock);
    // v53 leaves a LOCK FILE beside the socket; removing only the socket is not
    // enough and the next VMM refuses with StartVmmThread(ApiSocketInUse) (P8).
    let _ = std::fs::remove_file(format!("{}.lock", api_sock.display()));
}

fn drop_caches(tag: &str) {
    // `drop_caches` CANNOT evict DIRTY pages. Without the sync(2) first, a
    // just-written 2 GiB snapshot stays resident and both arms restore out of
    // RAM while the log claims the cache was dropped (increment-k's harness
    // defect: `copy` "read" 2 GiB at 8.8 GB/s off a 2.6 GB/s device).
    let before =
        std::fs::read_to_string("/proc/meminfo").map(|s| kb_field(&s, "Cached:")).unwrap_or(0);
    unsafe { libc::sync() };
    let _ = std::fs::write("/proc/sys/vm/drop_caches", "3");
    std::thread::sleep(Duration::from_millis(250));
    let after =
        std::fs::read_to_string("/proc/meminfo").map(|s| kb_field(&s, "Cached:")).unwrap_or(0);
    println!(
        "### [{tag}] sync+drop_caches: Cached {before} -> {after} kB (delta {})",
        before as i64 - after as i64
    );
}

// ===========================================================================
// ARM
// ===========================================================================

fn run_arm(a: &Args) {
    let ev = format!("L-RESULT arm={} sync={} label={}", a.arm, a.sync, a.label);
    println!("##################################################################");
    println!(
        "### increment-l ARM {} sync={} mem={}MiB cut_at={} direct={}",
        a.arm, a.sync, a.mem_mib, a.cut_at, a.direct
    );
    println!("### cloud-hypervisor : {}", sh("cloud-hypervisor", &["--version"]).1.trim());
    println!(
        "### kernel/arch      : {} {}",
        sh("uname", &["-r"]).1.trim(),
        sh("uname", &["-m"]).1.trim()
    );
    println!("### date             : {}", sh("date", &["-Is"]).1.trim());
    println!(
        "### pre-flight foreign cloud-hypervisor pids: {:?} (NOT killed — trap 6)",
        foreign_vmms(&[])
    );
    println!("##################################################################");

    let rd = &a.run_dir;
    let _ = std::fs::create_dir_all(rd);
    let mnt = rd.join(format!("mnt-{}", a.label));
    let rootfs = rd.join(format!("rootfs-{}.ext4", a.label));
    let vol = rd.join(format!("vol-{}.ext4", a.label));
    let vol_copy = rd.join(format!("volcopyA-{}.ext4", a.label));
    let vol_copy_b = rd.join(format!("volcopyB-{}.ext4", a.label));
    let con1 = rd.join(format!("con1-{}.log", a.label));
    let con1_keep = rd.join(format!("con1keep-{}.log", a.label));
    let con2 = rd.join(format!("con2-{}.log", a.label));
    let chlog1 = rd.join(format!("ch1-{}.log", a.label));
    let chlog2 = rd.join(format!("ch2-{}.log", a.label));
    let api_sock = rd.join(format!("api-{}.sock", a.label));
    let snap = a.snap_root.join(format!("snap-{}", a.label));
    for p in [&rootfs, &vol, &vol_copy, &con1, &con1_keep, &con2, &chlog1, &chlog2] {
        let _ = std::fs::remove_file(p);
    }
    cleanup_sock(&api_sock);
    let _ = std::fs::remove_dir_all(&snap);
    let _ = std::fs::create_dir_all(&snap);
    let _ = std::fs::create_dir_all(&mnt);
    for (src, dst) in [(&a.rootfs_src, &rootfs), (&a.vol_src, &vol)] {
        let (rc, out) = sh("cp", &["--reflink=auto", src.to_str().unwrap(), dst.to_str().unwrap()]);
        if rc != 0 {
            println!("!!! cp {src:?} -> {dst:?} rc={rc} {out}");
        }
    }

    // Arm A quiesces AFTER the restore (cut_at+40) so the pre-cut buffered data
    // is genuinely still in RAM at snapshot time; arm D quiesces AT the cut,
    // which is the whole point of arm D. B and C never quiesce — that is what
    // makes C the no-warning crash.
    let q1 = match a.arm.as_str() {
        "a" => a.cut_at + 40,
        "d" => a.cut_at,
        _ => 0,
    };
    let mut l1 = boot_vmm(a, &api_sock, &rootfs, &vol, &con1, &chlog1, q1, a.touch_mib);
    println!("### boot VMM pid={}", l1.pid);
    if !wait_for_socket(&api_sock, 30) {
        kill_pid(l1.pid);
        println!("{ev} status=BOOT_NO_SOCKET");
        return;
    }

    // Wait until the guest has written enough to make "0 lost" a meaningful
    // statement rather than a vacuous one.
    let want_quiesce = a.arm == "d";
    let txt1 = match wait_for(&con1, l1.pid, a.cut_at as i64, want_quiesce, 180) {
        Ok(t) => t,
        Err(e) => {
            println!("!!! {e}");
            println!(
                "--- console head:\n{}",
                &read_console(&con1)[..read_console(&con1).len().min(4000)]
            );
            println!("--- ch log:\n{}", std::fs::read_to_string(&chlog1).unwrap_or_default());
            kill_pid(l1.pid);
            println!("{ev} status=PRECUT_FAIL");
            return;
        }
    };

    let nonce1 = last_field(&txt1, "WORK ", "nonce");
    let tick1 = last_field(&txt1, "WORK ", "tick");
    let claimed = max_work_seq(&txt1);
    let n_work1 = work_seqs(&txt1).len();
    let rss1 = vmm_rss_kb(l1.pid);
    // Sampled HERE, not right after the API socket appeared. The socket exists
    // before the VM's block devices are opened, so the earlier sample returned
    // an empty list and the O_DIRECT claim would have rested on nothing.
    let fdflags = disk_fd_flags(l1.pid, &vol);
    println!();
    println!("=========== PRE-CUT STATE ==========================");
    println!("  VMM fds on the volume image: {fdflags:?}");
    println!("      (`direct=on` is what decides whether the guest's fsync leaves");
    println!("       the HOST page cache. SIGKILLing the VMM does not test that.)");
    println!("  {}", line_containing(&txt1, "MOUNT /dev/vdb"));
    println!("  {}", line_containing(&txt1, "RECOVER "));
    println!("  {}", line_containing(&txt1, "CONTINUE "));
    println!("  {}", line_containing(&txt1, "READY_AT_MONO_MS"));
    println!("  {}", line_containing(&txt1, "TOUCHED "));
    println!("  ext4 kernel lines:");
    for l in txt1.lines().filter(|l| l.contains("EXT4-fs")).take(6) {
        println!("      {}", l.trim());
    }
    println!("  BOOT_NONCE (pre-cut)  {nonce1}");
    println!("  RAM tick   (pre-cut)  {tick1}");
    println!("  claimed durable seq   {claimed}   ({n_work1} WORK lines)");
    println!("  VMM RSS               {rss1} kB");
    if a.arm == "d" {
        println!("  {}", line_containing(&txt1, "QUIESCE seq="));
    }
    // P8 trap: a restored VM re-opens the serial path from the SNAPSHOT's
    // config.json and TRUNCATES it. Copy the before-half aside or it vanishes.
    let _ = std::fs::copy(&con1, &con1_keep);

    // ================= THE CUT =========================================
    let mut snap_bytes = (0u64, 0u64);
    let mut mem_ranges_bytes = 0u64;
    let mut df_before = 0u64;
    let mut df_after_snap = 0u64;
    let mut df_after_discard = 0u64;
    let mut restore_ms = -1.0f64;
    let mut resume_ms = -1.0f64;

    println!();
    println!("=========== THE CUT: arm {} ========================", a.arm);
    if a.arm == "a" || a.arm == "b" {
        df_before = fs_avail_bytes(&a.snap_root);
        let (pc, pb, _) = api(&api_sock, "PUT", "vm.pause", None);
        println!("  PUT vm.pause    -> {pc} {pb}");
        let (sc, sb, sd) = api(
            &api_sock,
            "PUT",
            "vm.snapshot",
            Some(&format!(r#"{{"destination_url":"file://{}"}}"#, snap.display())),
        );
        println!("  PUT vm.snapshot -> {sc} in {:.3} s {sb}", sd.as_secs_f64());
        if sc != 204 {
            kill_pid(l1.pid);
            println!("{ev} status=SNAPSHOT_FAIL code={sc}");
            return;
        }
        unsafe { libc::sync() };
        df_after_snap = fs_avail_bytes(&a.snap_root);
        snap_bytes = dir_bytes(&snap);
        for e in std::fs::read_dir(&snap).into_iter().flatten().flatten() {
            if let Ok(m) = e.metadata() {
                use std::os::unix::fs::MetadataExt;
                let nm = e.file_name().to_string_lossy().to_string();
                println!(
                    "    {nm:<16} {:>14} B apparent  {:>14} B on disk",
                    m.len(),
                    m.blocks() * 512
                );
                if nm == "memory-ranges" {
                    mem_ranges_bytes = m.len();
                }
            }
        }
        println!(
            "  df delta on {}: {} B consumed by the snapshot (guest RAM = {} B)",
            a.snap_root.display(),
            df_before as i64 - df_after_snap as i64,
            a.mem_mib * 1024 * 1024
        );
    }

    kill_pid(l1.pid);
    let _ = l1.child.wait();
    cleanup_sock(&api_sock);
    println!("  SIGKILL sent to VMM pid {} (only pids this harness spawned — trap 6)", l1.pid);

    if a.arm == "b" {
        // The memory-as-cache model executed literally: erase the bytes that
        // make a warm pool expensive and require the VM to come back from the
        // filesystem alone. Deleting is stronger evidence than merely declining
        // to restore — after this the pages do not exist anywhere.
        let mr = snap.join("memory-ranges");
        let rmrc = std::fs::remove_file(&mr).is_ok();
        // XFS frees extents through DEFERRED ops, so `df` right after the unlink
        // can still show the space as used — the first run reported `reclaim=0`
        // on 2 of 3 arm-B trials while reporting the exact 2 GiB on the third.
        // sync, settle, sync again before sampling.
        unsafe { libc::sync() };
        std::thread::sleep(Duration::from_millis(800));
        unsafe { libc::sync() };
        df_after_discard = fs_avail_bytes(&a.snap_root);
        println!(
            "  DISCARD memory-ranges: removed={rmrc} was {mem_ranges_bytes} B; df reclaimed {} B",
            df_after_discard as i64 - df_after_snap as i64
        );
        println!(
            "  snapshot dir now: {:?}",
            std::fs::read_dir(&snap)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect::<Vec<_>>()
        );
    }

    // TWO forensic copies taken the INSTANT the cut lands, before anything
    // mounts the real image and replays its journal (which would silently turn
    // "needed recovery" into "was clean"). Two rather than one because a
    // `ro,noload` loop mount poisons the loop device for a later rw mount of the
    // same file — see `forensics`.
    for dst in [&vol_copy, &vol_copy_b] {
        let (cprc, cpout) =
            sh("cp", &["--reflink=auto", vol.to_str().unwrap(), dst.to_str().unwrap()]);
        if cprc != 0 {
            println!("!!! forensic copy -> {dst:?} failed rc={cprc} {cpout}");
        }
    }

    println!();
    println!("=========== POST-CUT FORENSICS (on copies) =========");
    let f = forensics(&vol_copy, &vol_copy_b, &mnt);
    println!("  dumpe2fs  {}", f.state);
    println!("  dumpe2fs  {}", f.features);
    println!(
        "  feature `needs_recovery` set: {}   <- THIS is the recovery signal,",
        f.needs_recovery
    );
    println!("      not `Filesystem state`, which reads `clean` either way.");
    println!(
        "  e2fsck -fn (journal NOT replayed — weak; cannot see past a dirty journal) rc={}",
        f.fsck_rc
    );
    for l in f.fsck_out.lines().take(10) {
        println!("      {l}");
    }
    println!(
        "  scan ro,noload (journal replay SUPPRESSED — what is visible with no recovery at all):"
    );
    println!("      {}", f.noload.line());
    println!("      {}", f.noload_note);
    println!("  scan rw        (journal REPLAYED, i.e. an ORDINARY mount):");
    println!("      {}", f.replayed.line());
    println!("      {}", f.replayed_note);
    println!(
        "  dumpe2fs after replay: {}  needs_recovery={}",
        f.state_after, f.needs_recovery_after
    );
    println!(
        "  e2fsck -fn AFTER replay (the meaningful structural verdict) rc={}",
        f.fsck_after_rc
    );
    for l in f.fsck_after_out.lines().take(8) {
        println!("      {l}");
    }
    println!(
        "  journal recovered {} records ({} -> {})",
        f.replayed.records_ok as i64 - f.noload.records_ok as i64,
        f.noload.records_ok,
        f.replayed.records_ok
    );
    println!("  LOST vs the guest's own announcement (which is a LOWER bound on what");
    println!("  reached the device — the guest prints each seq only AFTER its write");
    println!("  and, in sync mode, after its fsync returns):");
    println!(
        "      claimed={claimed} durable_noload={} durable_replayed={} lost_noload={} lost_replayed={}",
        f.noload.highest_seq,
        f.replayed.highest_seq,
        claimed - f.noload.highest_seq,
        claimed - f.replayed.highest_seq
    );

    // ================= THE RECOVERY ====================================
    println!();
    println!("=========== RECOVERY: arm {} =======================", a.arm);
    let (nonce2, tick2, ban2, txt2, ok2);
    if a.arm == "a" {
        // Restore the memory. `Copy` (the default) rather than `OnDemand`: P13
        // showed OnDemand is refused under P5's uid-dropped shape, and this arm
        // is the CONTROL — it must use the mode the driver can actually ship.
        let mut v2 = Command::new("cloud-hypervisor")
            .arg("--api-socket")
            .arg(format!("path={}", api_sock.display()))
            .stdout(Stdio::from(std::fs::File::create(&chlog2).unwrap()))
            .stderr(Stdio::from(std::fs::File::create(&chlog2).unwrap()))
            .spawn()
            .expect("spawn restore VMM");
        let pid2 = v2.id() as i32;
        if !wait_for_socket(&api_sock, 30) {
            kill_pid(pid2);
            println!("{ev} status=RESTORE_NO_SOCKET");
            return;
        }
        let body =
            format!(r#"{{"source_url":"file://{}","memory_restore_mode":"Copy"}}"#, snap.display());
        let (rc, rb, rdur) = api(&api_sock, "PUT", "vm.restore", Some(&body));
        restore_ms = rdur.as_secs_f64() * 1e3;
        println!("  PUT vm.restore -> {rc} in {:.3} s {rb}", rdur.as_secs_f64());
        if rc != 204 {
            println!("--- ch log:\n{}", std::fs::read_to_string(&chlog2).unwrap_or_default());
            kill_pid(pid2);
            println!("{ev} status=RESTORE_REFUSED code={rc}");
            return;
        }
        let (uc, ub, ud) = api(&api_sock, "PUT", "vm.resume", None);
        resume_ms = ud.as_secs_f64() * 1e3;
        println!("  PUT vm.resume  -> {uc} in {:.3} s {ub}", ud.as_secs_f64());
        // The restored VM truncated con1; wait on the NEW content of that path.
        // +20, not +40: the guest quiesces at cut_at+40 and stops writing there,
        // so a floor AT the quiesce point would race the freeze and time out.
        match wait_for(&con1, pid2, claimed + 20, false, 120) {
            Ok(t) => {
                txt2 = t;
                ok2 = true;
            }
            Err(e) => {
                println!("!!! {e}");
                txt2 = read_console(&con1);
                ok2 = false;
            }
        }
        nonce2 = last_field(&txt2, "WORK ", "nonce");
        tick2 = last_field(&txt2, "WORK ", "tick");
        ban2 = banners(&txt2);
        // Give the post-restore quiesce a moment to land, then stop the guest.
        let _ = wait_for(&con1, pid2, i64::MAX, true, 20);
        println!("  {}", line_containing(&read_console(&con1), "QUIESCE seq="));
        kill_pid(pid2);
        let _ = v2.wait();
    } else {
        // COLD BOOT from the same rootfs + the same volume. No memory anywhere.
        let mut l2 =
            boot_vmm(a, &api_sock, &rootfs, &vol, &con2, &chlog2, claimed as u64 + 40, a.touch_mib);
        println!("  cold-boot VMM pid={}", l2.pid);
        if !wait_for_socket(&api_sock, 30) {
            kill_pid(l2.pid);
            println!("{ev} status=COLDBOOT_NO_SOCKET");
            return;
        }
        match wait_for(&con2, l2.pid, i64::MAX, true, 180) {
            Ok(t) => {
                txt2 = t;
                ok2 = true;
            }
            Err(e) => {
                println!("!!! {e}");
                println!("--- ch log:\n{}", std::fs::read_to_string(&chlog2).unwrap_or_default());
                txt2 = read_console(&con2);
                ok2 = false;
            }
        }
        nonce2 = last_field(&txt2, "WORK ", "nonce");
        tick2 = last_field(&txt2, "WORK ", "tick");
        ban2 = banners(&txt2);
        println!("  {}", line_containing(&txt2, "MOUNT /dev/vdb"));
        println!("  ext4 kernel lines on the COLD BOOT (journal recovery shows here):");
        for l in txt2.lines().filter(|l| l.contains("EXT4-fs")).take(8) {
            println!("      {}", l.trim());
        }
        println!("  {}", line_containing(&txt2, "RECOVER "));
        println!("  {}", line_containing(&txt2, "CONTINUE "));
        println!("  {}", line_containing(&txt2, "READY_AT_MONO_MS"));
        println!("  {}", line_containing(&txt2, "QUIESCE seq="));
        kill_pid(l2.pid);
        let _ = l2.child.wait();
    }
    cleanup_sock(&api_sock);

    // Final state of the REAL volume, after the second incarnation quiesced.
    let final_scan = {
        // A THIRD file: vol_copy was mounted `ro,noload` and its loop device is
        // read-only, so reusing it for an rw mount fails.
        let fin = rd.join(format!("volfinal-{}.ext4", a.label));
        let (rc, out) = sh("cp", &["--reflink=auto", vol.to_str().unwrap(), fin.to_str().unwrap()]);
        if rc != 0 {
            println!("!!! final copy rc={rc} {out}");
        }
        let (s, note) = scan_image(&fin, &mnt, "loop");
        let _ = std::fs::remove_file(&fin);
        println!();
        println!("=========== FINAL VOLUME (after the recovered guest quiesced) ===");
        println!("  {}", s.line());
        println!("  {note}");
        s
    };

    // ================= CORRECTNESS =====================================
    //
    // The polarity INVERTS between arm A and arms B/C/D and both directions are
    // asserted. An identical nonce in arm B would mean memory had accidentally
    // been restored and nothing was measured; a different nonce in arm A would
    // mean the restore silently rebooted.
    let memory_returned = a.arm == "a";
    let nonce_same = nonce2 == nonce1 && nonce2 != "<none>";
    // The FIRST post-cut tick, not the last — see `first_field`.
    let first_tick_after = first_field(&txt2, "WORK ", "tick");
    let tick_restarted = first_tick_after == "0";
    let seq_continued = final_scan.highest_seq > claimed;
    let mounts_ok = f.noload.mount_ok && f.replayed.mount_ok && final_scan.mount_ok;
    let correct_polarity = if memory_returned {
        nonce_same && ban2 == 0 && !tick_restarted
    } else {
        !nonce_same && ban2 == 1 && tick_restarted
    };
    println!();
    println!("=========== CORRECTNESS (polarity INVERTS per arm) ==");
    println!(
        "  arm {} expects: nonce {}  |  boot banners {}  |  RAM tick {}",
        a.arm,
        if memory_returned { "IDENTICAL" } else { "DIFFERENT" },
        if memory_returned { "0" } else { "1" },
        if memory_returned { "CONTINUED" } else { "RESTARTED" }
    );
    println!("  BOOT_NONCE before   {nonce1}");
    println!(
        "  BOOT_NONCE after    {nonce2}     -> {}",
        if nonce_same { "IDENTICAL" } else { "DIFFERENT" }
    );
    println!("  RAM tick, last before cut       {tick1}");
    println!(
        "  RAM tick, FIRST record after    {first_tick_after}     -> {}",
        if tick_restarted { "RESTARTED" } else { "CONTINUED mid-stream" }
    );
    println!("  RAM tick, last after            {tick2}   (climbs past {tick1} on a REBOOT too —");
    println!("      which is why the FIRST post-cut tick is the one that carries the signal)");
    println!("  boot banners after  {ban2}");
    println!("  all forensic mounts succeeded   {mounts_ok}   (false => HARNESS, not data loss)");
    println!("  durable seq claimed before cut  {claimed}");
    println!(
        "  durable seq at end              {}   -> {}",
        final_scan.highest_seq,
        if seq_continued { "CONTINUED" } else { "DID NOT ADVANCE" }
    );
    println!(
        "  gaps in the final journal       {} (first at {})",
        final_scan.gaps, final_scan.first_gap
    );
    println!(
        "  VERDICT  polarity={} volume_intact={}",
        if correct_polarity { "CORRECT" } else { "!!! WRONG — DISCARD" },
        if final_scan.gaps == 0 && final_scan.corrupt == 0 && seq_continued { "YES" } else { "NO" }
    );

    println!();
    println!(
        "{ev} mem_mib={} direct={} status={} polarity_ok={} claimed={claimed} \
durable_noload={} durable_replayed={} lost_noload={} lost_replayed={} \
journal_recovered={} fs_state=\"{}\" needs_recovery={} fsck_rc={} fsck_after_rc={} nr_after={} \
noload_ok={} replayed_ok={} noload_gaps={} replayed_gaps={} noload_corrupt={} \
torn_noload={} torn_replayed={} final_high={} final_gaps={} final_corrupt={} \
nonce_same={} first_tick_after={} tick_restarted={} banners={} mounts_ok={} \
noload_mount_ok={} replayed_mount_ok={} snap_apparent={} snap_ondisk={} \
mem_ranges_bytes={} df_snap_cost={} df_discard_reclaim={} restore_ms={:.1} resume_ms={:.1}",
        a.mem_mib,
        a.direct,
        if !mounts_ok {
            "HARNESS_DEFECT_MOUNT"
        } else if ok2 {
            "OK"
        } else {
            "RECOVERY_INCOMPLETE"
        },
        correct_polarity,
        f.noload.highest_seq,
        f.replayed.highest_seq,
        claimed - f.noload.highest_seq,
        claimed - f.replayed.highest_seq,
        f.replayed.records_ok as i64 - f.noload.records_ok as i64,
        f.state.replace("Filesystem state:", "").trim(),
        f.needs_recovery,
        f.fsck_rc,
        f.fsck_after_rc,
        f.needs_recovery_after,
        f.noload.records_ok,
        f.replayed.records_ok,
        f.noload.gaps,
        f.replayed.gaps,
        f.noload.corrupt,
        f.noload.torn_tail_bytes,
        f.replayed.torn_tail_bytes,
        final_scan.highest_seq,
        final_scan.gaps,
        final_scan.corrupt,
        nonce_same,
        first_tick_after,
        tick_restarted,
        ban2,
        mounts_ok,
        f.noload.mount_ok,
        f.replayed.mount_ok,
        snap_bytes.0,
        snap_bytes.1,
        mem_ranges_bytes,
        df_before as i64 - df_after_snap as i64,
        df_after_discard as i64 - df_after_snap as i64,
        restore_ms,
        resume_ms,
    );

    let _ = std::fs::remove_dir_all(&snap);
    let _ = std::fs::remove_file(&rootfs);
    let _ = std::fs::remove_file(&vol);
    let _ = std::fs::remove_file(&vol_copy);
    let _ = std::fs::remove_file(&vol_copy_b);
}

// ===========================================================================
// BENCH — cold boot vs restore, to the SAME observable event
// ===========================================================================

/// One timing trial.
///
/// The observable is identical across all three modes: **the guest's first
/// post-start WORK record**. Two numbers are reported for the restore modes and
/// the distinction matters:
///
///   * `call_ms`      — the `vm.restore` call alone. This is what P13 reported.
///   * `spawn_ready_ms` — from `Command::spawn` of the incarnation that will
///     serve, through socket wait, restore and resume, to that first record.
///     Cold boot has no other number to give, so this is the ONLY apples-to-
///     apples comparison, and it is the one the pool economics turn on.
fn run_bench_trial(a: &Args) {
    let rd = &a.run_dir;
    let _ = std::fs::create_dir_all(rd);
    let rootfs = rd.join(format!("rootfs-{}.ext4", a.label));
    let vol = rd.join(format!("vol-{}.ext4", a.label));
    let con = rd.join(format!("con-{}.log", a.label));
    let chlog = rd.join(format!("ch-{}.log", a.label));
    let chlog2 = rd.join(format!("ch2-{}.log", a.label));
    let api_sock = rd.join(format!("api-{}.sock", a.label));
    let snap = a.snap_root.join(format!("bench-{}", a.label));
    for p in [&rootfs, &vol, &con, &chlog, &chlog2] {
        let _ = std::fs::remove_file(p);
    }
    cleanup_sock(&api_sock);
    let _ = std::fs::remove_dir_all(&snap);
    let _ = std::fs::create_dir_all(&snap);
    for (src, dst) in [(&a.rootfs_src, &rootfs), (&a.vol_src, &vol)] {
        let _ = sh("cp", &["--reflink=auto", src.to_str().unwrap(), dst.to_str().unwrap()]);
    }

    let mode = a.bench_mode.clone();
    let mut call_ms = -1.0f64;
    let mut resume_ms = -1.0f64;
    let mut snap_ondisk = 0u64;

    if mode == "cold" {
        drop_caches(&format!("{}-cold", a.label));
        let t0 = Instant::now();
        let mut l = boot_vmm(a, &api_sock, &rootfs, &vol, &con, &chlog, 0, a.touch_mib);
        let r = wait_for(&con, l.pid, 0, false, 180);
        let ready = t0.elapsed().as_secs_f64() * 1e3;
        let txt = read_console(&con);
        let ok = r.is_ok();
        let boot_to_ready = last_field(&txt, "init: READY_AT_MONO_MS", "boot_to_ready_ms");
        let mount_ms = last_field(&txt, "init: MOUNT", "mount_ms");
        let scan_ms = last_field(&txt, "init: RECOVER", "scan_ms");
        let rec_high = last_field(&txt, "init: RECOVER", "highest_seq");
        println!(
            "L-BENCH label={} mode=cold mem_mib={} status={} spawn_ready_ms={:.1} call_ms=-1 resume_ms=-1 \
guest_boot_to_ready_ms={boot_to_ready} mount_ms={mount_ms} scan_ms={scan_ms} recovered_high={rec_high} \
first_seq={} banners={} snap_ondisk=0",
            a.label,
            a.mem_mib,
            if ok { "OK" } else { "FAIL" },
            ready,
            work_seqs(&txt).first().copied().unwrap_or(-1),
            banners(&txt)
        );
        if !ok {
            println!("    !!! {}", r.unwrap_err());
            println!(
                "    --- ch log: {}",
                std::fs::read_to_string(&chlog)
                    .unwrap_or_default()
                    .lines()
                    .take(6)
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
        }
        kill_pid(l.pid);
        let _ = l.child.wait();
        cleanup_sock(&api_sock);
    } else {
        // Boot, let the guest touch memory and work, snapshot, kill, drop cache,
        // then time the restore path from SPAWN.
        let mut l = boot_vmm(a, &api_sock, &rootfs, &vol, &con, &chlog, 0, a.touch_mib);
        if !wait_for_socket(&api_sock, 30) {
            kill_pid(l.pid);
            println!("L-BENCH label={} mode={mode} status=NO_SOCKET", a.label);
            return;
        }
        // The volume is SEEDED, so the guest's first WORK line already carries a
        // high seq — a bare "seq >= N" gate would fire before the touch finished
        // and the snapshot would hold zeros where the probe needs real pages.
        // Gate on TOUCHED first, THEN on boot_ticks further records.
        if let Err(e) = wait_for_line(&con, l.pid, "init: TOUCHED", 180) {
            kill_pid(l.pid);
            println!("L-BENCH label={} mode={mode} status=PRETOUCH_FAIL note=\"{e}\"", a.label);
            return;
        }
        let seq_after_touch = max_work_seq(&read_console(&con));
        let txt1 = match wait_for(&con, l.pid, seq_after_touch + a.boot_ticks as i64, false, 180) {
            Ok(t) => t,
            Err(e) => {
                kill_pid(l.pid);
                println!("L-BENCH label={} mode={mode} status=PREBOOT_FAIL note=\"{e}\"", a.label);
                return;
            }
        };
        let floor = max_work_seq(&txt1);
        let nonce1 = last_field(&txt1, "WORK ", "nonce");
        let (pc, _, _) = api(&api_sock, "PUT", "vm.pause", None);
        let (sc, _, sd) = api(
            &api_sock,
            "PUT",
            "vm.snapshot",
            Some(&format!(r#"{{"destination_url":"file://{}"}}"#, snap.display())),
        );
        if pc != 204 || sc != 204 {
            kill_pid(l.pid);
            println!("L-BENCH label={} mode={mode} status=SNAP_FAIL pause={pc} snap={sc}", a.label);
            return;
        }
        snap_ondisk = dir_bytes(&snap).1;
        kill_pid(l.pid);
        let _ = l.child.wait();
        cleanup_sock(&api_sock);
        let _ = std::fs::copy(&con, rd.join(format!("conkeep-{}.log", a.label)));
        drop_caches(&format!("{}-{}", a.label, mode));

        let t0 = Instant::now();
        let mut v2 = Command::new("cloud-hypervisor")
            .arg("--api-socket")
            .arg(format!("path={}", api_sock.display()))
            .stdout(Stdio::from(std::fs::File::create(&chlog2).unwrap()))
            .stderr(Stdio::from(std::fs::File::create(&chlog2).unwrap()))
            .spawn()
            .expect("spawn restore VMM");
        let pid2 = v2.id() as i32;
        if !wait_for_socket(&api_sock, 30) {
            kill_pid(pid2);
            println!("L-BENCH label={} mode={mode} status=RESTORE_NO_SOCKET", a.label);
            return;
        }
        let body = if mode == "restore-ondemand" {
            format!(
                r#"{{"source_url":"file://{}","memory_restore_mode":"OnDemand"}}"#,
                snap.display()
            )
        } else {
            format!(r#"{{"source_url":"file://{}","memory_restore_mode":"Copy"}}"#, snap.display())
        };
        let (rc, rb, rdur) = api(&api_sock, "PUT", "vm.restore", Some(&body));
        call_ms = rdur.as_secs_f64() * 1e3;
        if rc != 204 {
            println!(
                "L-BENCH label={} mode={mode} status=RESTORE_REFUSED code={rc} body=\"{rb}\"",
                a.label
            );
            kill_pid(pid2);
            let _ = v2.wait();
            cleanup_sock(&api_sock);
            return;
        }
        let (_, _, ud) = api(&api_sock, "PUT", "vm.resume", None);
        resume_ms = ud.as_secs_f64() * 1e3;
        let r = wait_for(&con, pid2, floor + 1, false, 120);
        let ready = t0.elapsed().as_secs_f64() * 1e3;
        let txt2 = read_console(&con);
        let nonce2 = last_field(&txt2, "WORK ", "nonce");
        // A reboot is FAST and would masquerade as a spectacular restore win.
        let genuine = nonce2 == nonce1 && nonce2 != "<none>" && banners(&txt2) == 0;
        println!(
            "L-BENCH label={} mode={mode} mem_mib={} status={} spawn_ready_ms={:.1} call_ms={:.1} \
resume_ms={:.1} genuine={genuine} nonce_before={nonce1} nonce_after={nonce2} banners={} \
first_seq={} snap_ondisk={snap_ondisk}",
            a.label,
            a.mem_mib,
            if r.is_ok() && genuine { "OK" } else { "FAIL" },
            ready,
            call_ms,
            resume_ms,
            banners(&txt2),
            work_seqs(&txt2).iter().find(|s| **s > floor).copied().unwrap_or(-1)
        );
        if let Err(e) = r {
            println!("    !!! {e}");
        }
        kill_pid(pid2);
        let _ = v2.wait();
        cleanup_sock(&api_sock);
    }
    let _ = std::fs::remove_dir_all(&snap);
    let _ = std::fs::remove_file(&rootfs);
    let _ = std::fs::remove_file(&vol);
}

// ===========================================================================

fn main() {
    let a = parse_args();
    match a.cmd.as_str() {
        "arm" => run_arm(&a),
        "bench" => run_bench_trial(&a),
        "seed" => seed_volume(&a),
        other => println!("unknown --cmd {other}"),
    }
}

/// Produce the seeded volume the bench uses: a volume that already holds a
/// realistic journal, so cold boot is charged for the recovery scan a real cold
/// boot would have to do, and all three bench modes start from one identical
/// on-disk state.
fn seed_volume(a: &Args) {
    let rd = &a.run_dir;
    let _ = std::fs::create_dir_all(rd);
    let rootfs = rd.join("rootfs-seed.ext4");
    let vol = a.vol_src.clone();
    let con = rd.join("con-seed.log");
    let chlog = rd.join("ch-seed.log");
    let api_sock = rd.join("api-seed.sock");
    for p in [&rootfs, &con, &chlog] {
        let _ = std::fs::remove_file(p);
    }
    cleanup_sock(&api_sock);
    let _ = sh("cp", &["--reflink=auto", a.rootfs_src.to_str().unwrap(), rootfs.to_str().unwrap()]);
    println!("### seeding {} with {} records + quiesce", vol.display(), a.cut_at);
    let mut l = boot_vmm(a, &api_sock, &rootfs, &vol, &con, &chlog, a.cut_at, a.touch_mib);
    match wait_for(&con, l.pid, i64::MAX, true, 180) {
        Ok(t) => {
            println!("### {}", line_containing(&t, "QUIESCE seq="));
            println!("### {}", line_containing(&t, "RECOVER "));
        }
        Err(e) => println!("!!! seed failed: {e}"),
    }
    kill_pid(l.pid);
    let _ = l.child.wait();
    cleanup_sock(&api_sock);
    let _ = std::fs::remove_file(&rootfs);
    let mnt = rd.join("mnt-seed");
    let (s, note) = scan_image(&vol, &mnt, "loop");
    println!("### seeded volume: {}", s.line());
    println!("### {note}");
}
