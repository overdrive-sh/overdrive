//! PROBE increment-k — the host harness for `memory_restore_mode=ondemand`.
//!
//! increment-g drove CH's API from bash with `curl`. That was fine for a binary
//! "did it restore" verdict and is NOT fine here, for two reasons that are both
//! the point of this probe:
//!
//!   * **The headline is a latency**, and a `curl` fork is worth more than the
//!     difference we are trying to resolve.
//!   * **The control is an RSS TRAJECTORY** — /proc/<vmm>/status sampled on a
//!     fixed schedule relative to the resume, WHILE simultaneously watching the
//!     guest console for its first post-restore tick. bash can do one or the
//!     other, badly.
//!
//! So this speaks HTTP-over-unix directly (CH's API is plain HTTP/1.1 on a unix
//! socket; `ch-remote` is not installed on the box) and runs one 2 ms poll loop
//! that services both the console watch and the sample schedule.
//!
//! ## Why the RSS trajectory is the control, and not a nice-to-have
//!
//! P11 nearly published 2958 MiB/s on a device whose ceiling is 1163 MiB/s. It
//! survived 5 interleaved trials, a disjoint-ranges check and a THP sweep,
//! because none of those can detect a dropped flush — only an explicit control
//! that REMOVES the mechanism could. The equivalent trap here is exact:
//! `ondemand` could report a fast restore because it never faulted anything in
//! *yet*, or because it silently fell back to `copy` and something else got
//! faster, or because the VM rebooted. Latency alone cannot tell those apart.
//!
//! RSS can. Under `copy` the VMM must read the whole `memory-ranges` file
//! before the guest runs, so RSS is ~guest RAM essentially at restore-return.
//! Under a genuinely lazy `ondemand` it starts near zero and climbs as the
//! guest touches pages — at a rate the guest's own WALK makes known. **If the
//! two trajectories are indistinguishable, `ondemand` did not happen**, whatever
//! the latency says.
//!
//! ## Trap 6 (the box is shared)
//!
//! A previous probe on this box was corrupted when a concurrent agent's
//! `pkill -9 -x cloud-hyperviso` SIGKILLed its restoring VMM mid-run, producing
//! an `HTTP 000` with an empty log that read exactly like a genuine restore
//! refusal. So this harness:
//!   * kills ONLY pids it spawned itself, by pid, never by name;
//!   * `kill(pid, 0)`-checks its VMM at every single sample; and
//!   * on finding its VMM gone, labels the run `HARNESS_DEFECT=EXTERNAL_KILL`
//!     so the bench discards it rather than averaging it in.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Minimal HTTP/1.1 over a unix socket.
// ---------------------------------------------------------------------------

/// Issue one request and return `(status, body, elapsed)`.
///
/// The response is parsed by HEADERS rather than read-to-EOF. Read-to-EOF would
/// work only if CH honours `Connection: close`, and if it does not, every call
/// would block until the read timeout — which would silently become the
/// "latency" this probe reports. Parsing Content-Length (and treating 204 as
/// bodyless, which is CH's success code for most PUTs) removes that dependency.
fn api(sock: &Path, method: &str, path: &str, body: Option<&str>) -> (u16, String, Duration) {
    let t0 = Instant::now();
    let mut s = match UnixStream::connect(sock) {
        Ok(s) => s,
        Err(e) => return (0, format!("<connect failed: {e}>"), t0.elapsed()),
    };
    // Generous: a 4 GiB `copy` restore reads 4 GiB off XFS inside this call.
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
    // 1. headers
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
        let l = l.trim();
        let (k, v) = l.split_once(':')?;
        if k.eq_ignore_ascii_case("content-length") { v.trim().parse().ok() } else { None }
    });
    // 2. body
    let want = match clen {
        Some(n) => n,
        // 204 No Content is CH's success code for most PUTs: no body, and
        // waiting for one would hang until the read timeout.
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
    let elapsed = t0.elapsed();
    let body_txt = String::from_utf8_lossy(&buf[head_end..]).trim().to_string();
    (status, body_txt, elapsed)
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

// ---------------------------------------------------------------------------
// /proc sampling — the control.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default, Debug)]
struct Rss {
    alive: bool,
    vm_rss_kb: u64,
    anon_kb: u64,
    file_kb: u64,
    shmem_kb: u64,
    host_avail_kb: u64,
}

fn kb_field(s: &str, key: &str) -> u64 {
    s.lines()
        .find(|l| l.starts_with(key))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn host_mem_available_kb() -> u64 {
    std::fs::read_to_string("/proc/meminfo").map(|s| kb_field(&s, "MemAvailable:")).unwrap_or(0)
}

fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

fn sample_rss(pid: i32) -> Rss {
    let a = alive(pid);
    let st = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    Rss {
        alive: a,
        vm_rss_kb: kb_field(&st, "VmRSS:"),
        anon_kb: kb_field(&st, "RssAnon:"),
        file_kb: kb_field(&st, "RssFile:"),
        shmem_kb: kb_field(&st, "RssShmem:"),
        host_avail_kb: host_mem_available_kb(),
    }
}

// ---------------------------------------------------------------------------
// Console parsing.
// ---------------------------------------------------------------------------

/// Read the console transcript, DISCARDING any trailing partial line.
///
/// HARNESS DEFECT, caught on the first real run and fixed here. The guest is
/// writing this file continuously; a read that lands mid-write returns a
/// truncated final line. The first api-probe run reported
/// `nonce=360349041693d9c6c8e5290d5b3e` — 28 hex chars of a 32-char nonce —
/// and `walked_mib=<none>`, both from parsing a half-written line.
///
/// Left unfixed this corrupts the probe in BOTH directions:
///   * a truncated `nonce_before` can never equal a complete `nonce_after`, so
///     a genuine restore reports as a FALSE NEGATIVE; and worse
///   * a truncated `TICK n=123` parses as `n=12` or `n=1`, so
///     `first_tick_above(floor)` can fire on a PRE-snapshot tick and report a
///     resume latency that never happened. That error flatters whichever mode
///     is under test.
///
/// Truncating at the last newline makes every parser below see only whole
/// lines, which is the property they all assume.
fn read_console(p: &Path) -> String {
    let raw = std::fs::read(p).map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default();
    match raw.rfind('\n') {
        Some(i) => raw[..=i].to_string(),
        None => String::new(),
    }
}

/// Highest `TICK n=<n>` in the transcript.
fn last_tick(txt: &str) -> Option<u64> {
    txt.lines()
        .filter_map(|l| l.strip_prefix("TICK n="))
        .filter_map(|r| r.split_whitespace().next())
        .filter_map(|v| v.parse::<u64>().ok())
        .max()
}

fn first_tick_above(txt: &str, floor: u64) -> Option<u64> {
    txt.lines()
        .filter_map(|l| l.strip_prefix("TICK n="))
        .filter_map(|r| r.split_whitespace().next())
        .filter_map(|v| v.parse::<u64>().ok())
        .find(|n| *n > floor)
}

fn field_of_last_tick(txt: &str, key: &str) -> String {
    txt.lines()
        .filter(|l| l.starts_with("TICK n="))
        .last()
        .and_then(|l| {
            l.split_whitespace().find_map(|t| t.strip_prefix(&format!("{key}=")).map(str::to_owned))
        })
        .unwrap_or_else(|| "<none>".into())
}

fn nonce_of(txt: &str) -> String {
    field_of_last_tick(txt, "nonce")
}

// ---------------------------------------------------------------------------
// Args.
// ---------------------------------------------------------------------------

struct Args {
    label: String,
    mode: String,
    mem_mib: u64,
    touch_mib: u64,
    walk_mib: u64,
    kernel: PathBuf,
    rootfs_src: PathBuf,
    run_dir: PathBuf,
    snap_dir: PathBuf,
    boot_ticks: u64,
    api_probe: bool,
    /// Skip the `drop_caches` before restore. Default is to DROP: the snapshot
    /// was written seconds earlier and is entirely in page cache, so a run that
    /// does not drop measures how fast `copy` can memcpy from RAM, and whichever
    /// arm ran first would have warmed the cache for the next one.
    warm_cache: bool,
}

fn arg(a: &[String], k: &str, d: &str) -> String {
    a.windows(2).find(|w| w[0] == k).map(|w| w[1].clone()).unwrap_or_else(|| d.to_string())
}

fn flag(a: &[String], k: &str) -> bool {
    a.iter().any(|x| x == k)
}

fn parse_args() -> Args {
    let a: Vec<String> = std::env::args().collect();
    Args {
        label: arg(&a, "--label", "run"),
        mode: arg(&a, "--mode", "copy"),
        mem_mib: arg(&a, "--mem-mib", "2048").parse().unwrap(),
        touch_mib: arg(&a, "--touch-mib", "1536").parse().unwrap(),
        walk_mib: arg(&a, "--walk-mib", "2").parse().unwrap(),
        kernel: arg(&a, "--kernel", "/var/tmp/spike-increment-k/kernel").into(),
        rootfs_src: arg(&a, "--rootfs-src", "/var/tmp/spike-increment-k/rootfs.ext4").into(),
        run_dir: arg(&a, "--run-dir", "/run/spike-increment-k").into(),
        snap_dir: arg(&a, "--snap-dir", "/srv/vm/p13k/snap").into(),
        boot_ticks: arg(&a, "--boot-ticks", "80").parse().unwrap(),
        api_probe: flag(&a, "--api-probe"),
        warm_cache: flag(&a, "--warm-cache"),
    }
}

/// The JSON body for `PUT /api/v1/vm.restore`.
///
/// Field names come from the binary's own serde string table
/// (`struct RestoreConfig with 5 elements` / `source_url` `memory_restore_mode`
/// `net_fds` `resume`) plus `--help`'s
/// `source_url=..,prefault=on|off,memory_restore_mode=copy|ondemand,..`.
/// `prefault` is COPY-ONLY: the binary carries the literal refusal
/// `'prefault' cannot be combined with 'memory_restore_mode=ondemand'`.
///
/// **The JSON values are NOT the CLI values.** `--help` documents
/// `memory_restore_mode=copy|ondemand`; the API rejects both of those and
/// accepts only `Copy` / `OnDemand`. Established empirically by sending a
/// deliberately-invalid value and reading serde's own enumeration back:
///
/// ```text
/// 400 {"...","memory_restore_mode":"ondemand"}
///     ["Failed to deserialize JSON",
///      "unknown variant `ondemand`, expected `Copy` or `OnDemand` ..."]
/// ```
///
/// The `copy-prefault` arm therefore sends `prefault` with NO
/// `memory_restore_mode` at all: `Copy` is the default, and pairing the two
/// explicitly is only noise.
fn restore_body(mode: &str, snap: &Path) -> String {
    let url = format!("file://{}", snap.display());
    match mode {
        "ondemand" => format!(r#"{{"source_url":"{url}","memory_restore_mode":"OnDemand"}}"#),
        "copy-prefault" => format!(r#"{{"source_url":"{url}","prefault":true}}"#),
        _ => format!(r#"{{"source_url":"{url}","memory_restore_mode":"Copy"}}"#),
    }
}

fn kill_pid(pid: i32) {
    unsafe { libc::kill(pid, libc::SIGKILL) };
}

fn wait_for_socket(p: &Path, secs: u64) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(secs) {
        if p.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn dir_bytes(p: &Path) -> (u64, u64) {
    // (apparent, on-disk) — S-7 asked exactly this question at 512 MiB; it is
    // re-asked here at 2 and 4 GiB because warm-pool storage is the whole
    // economic argument.
    let mut app = 0u64;
    let mut disk = 0u64;
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

fn main() {
    let a = parse_args();

    // --- pre-flight: is the box quiet? (trap 6) --------------------------
    let foreign = foreign_vmms();
    println!(
        "### increment-k  label={} mode={} mem={}MiB touch={}MiB walk={}MiB/tick",
        a.label, a.mode, a.mem_mib, a.touch_mib, a.walk_mib
    );
    println!("### pre-flight foreign cloud-hypervisor pids: {foreign:?}");

    let _ = std::fs::create_dir_all(&a.run_dir);
    let _ = std::fs::create_dir_all(&a.snap_dir);
    // A stale snapshot from a previous trial would be restored instead of this
    // trial's, and every number would describe the wrong VM.
    let _ = std::fs::remove_dir_all(&a.snap_dir);
    let _ = std::fs::create_dir_all(&a.snap_dir);

    let rootfs = a.run_dir.join(format!("rootfs-{}.ext4", a.label));
    let console = a.run_dir.join(format!("console-{}.log", a.label));
    let api_sock = a.run_dir.join(format!("api-{}.sock", a.label));
    let chlog_a = a.run_dir.join(format!("ch-before-{}.log", a.label));
    let chlog_b = a.run_dir.join(format!("ch-after-{}.log", a.label));
    for p in [&rootfs, &console, &api_sock, &chlog_a, &chlog_b] {
        let _ = std::fs::remove_file(p);
    }
    let _ = std::fs::remove_file(format!("{}.lock", api_sock.display()));
    // reflink where possible (P4); falls back to a full copy.
    let _ = Command::new("cp").args(["--reflink=auto"]).arg(&a.rootfs_src).arg(&rootfs).status();

    let cmdline = format!(
        "root=/dev/vda rw console=ttyS0 init=/init panic=1 loglevel=4 spike.touch={} spike.walk={}",
        a.touch_mib, a.walk_mib
    );

    // ================= [1] boot =========================================
    let mut boot: Child = Command::new("cloud-hypervisor")
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
        .arg("--disk")
        .arg(format!("path={},image_type=raw", rootfs.display()))
        .arg("--serial")
        .arg(format!("file={}", console.display()))
        .arg("--console")
        .arg("off")
        .stdout(Stdio::from(std::fs::File::create(&chlog_a).unwrap()))
        .stderr(Stdio::from(std::fs::File::create(&chlog_a).unwrap()))
        .spawn()
        .expect("spawn cloud-hypervisor");
    let boot_pid = boot.id() as i32;
    println!("### boot VMM pid={boot_pid}");

    if !wait_for_socket(&api_sock, 30) {
        println!("!!! API socket never appeared");
        kill_pid(boot_pid);
        println!("K-RESULT label={} mode={} status=BOOT_FAIL", a.label, a.mode);
        return;
    }

    // Wait for the guest to finish TOUCHing and tick past the floor.
    let t_boot = Instant::now();
    let before_txt = loop {
        let txt = read_console(&console);
        if last_tick(&txt).unwrap_or(0) >= a.boot_ticks {
            break txt;
        }
        if !alive(boot_pid) {
            println!(
                "!!! boot VMM died. ch log:\n{}",
                std::fs::read_to_string(&chlog_a).unwrap_or_default()
            );
            println!("K-RESULT label={} mode={} status=BOOT_DIED", a.label, a.mode);
            return;
        }
        if t_boot.elapsed() > Duration::from_secs(180) {
            println!("!!! guest never reached tick {}", a.boot_ticks);
            println!("--- console:\n{}", &txt[..txt.len().min(3000)]);
            kill_pid(boot_pid);
            println!("K-RESULT label={} mode={} status=BOOT_TIMEOUT", a.label, a.mode);
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let rss_running = sample_rss(boot_pid);
    let nonce_before = nonce_of(&before_txt);
    let tick_before = last_tick(&before_txt).unwrap_or(0);
    let touched_line = before_txt
        .lines()
        .find(|l| l.contains("TOUCHED"))
        .unwrap_or("<no TOUCHED line>")
        .to_string();
    println!("### {touched_line}");
    println!(
        "### pre-snapshot: nonce={nonce_before} tick={tick_before} walked_mib={} bad={} zero={}",
        field_of_last_tick(&before_txt, "walked_mib"),
        field_of_last_tick(&before_txt, "bad"),
        field_of_last_tick(&before_txt, "zero")
    );
    println!(
        "### pre-snapshot VMM RSS: VmRSS={} kB RssAnon={} RssFile={} RssShmem={}",
        rss_running.vm_rss_kb, rss_running.anon_kb, rss_running.file_kb, rss_running.shmem_kb
    );
    // The restored VM re-opens the serial path from the SNAPSHOT's config.json
    // and TRUNCATES it (P8 trap 3). Copy the "before" half aside or it vanishes.
    let _ = std::fs::copy(&console, a.run_dir.join(format!("console-before-{}.log", a.label)));

    // ================= [2] pause + snapshot =============================
    let (pc, pb, pd) = api(&api_sock, "PUT", "vm.pause", None);
    println!("### vm.pause -> {pc} ({:.1} ms) {}", pd.as_secs_f64() * 1e3, pb);
    let snap_url = format!(r#"{{"destination_url":"file://{}"}}"#, a.snap_dir.display());
    let (sc, sb, sd) = api(&api_sock, "PUT", "vm.snapshot", Some(&snap_url));
    println!("### vm.snapshot -> {sc} ({:.3} s) {}", sd.as_secs_f64(), sb);
    if sc != 204 && sc != 200 {
        kill_pid(boot_pid);
        println!("K-RESULT label={} mode={} status=SNAPSHOT_FAIL code={sc}", a.label, a.mode);
        return;
    }
    let (app, disk) = dir_bytes(&a.snap_dir);
    println!(
        "### snapshot bytes: apparent={app} on_disk={disk} (guest RAM = {} B)",
        a.mem_mib * 1024 * 1024
    );
    if let Ok(rd) = std::fs::read_dir(&a.snap_dir) {
        for e in rd.flatten() {
            if let Ok(m) = e.metadata() {
                use std::os::unix::fs::MetadataExt;
                println!(
                    "###   {:<16} {:>14} B  {:>14} B on disk",
                    e.file_name().to_string_lossy(),
                    m.len(),
                    m.blocks() * 512
                );
            }
        }
    }

    // ================= [3] the checkpoint's temporal gap ================
    kill_pid(boot_pid);
    let _ = boot.wait();
    let _ = std::fs::remove_file(&api_sock);
    // v53 leaves a LOCK FILE beside the socket; removing only the socket is not
    // enough and the next VMM refuses with StartVmmThread(ApiSocketInUse) (P8).
    let _ = std::fs::remove_file(format!("{}.lock", api_sock.display()));

    // Drop the host page cache for the snapshot so `copy` genuinely reads from
    // the device rather than replaying its own just-written pages. Without this
    // the two modes are compared across different cache states and the whole
    // latency number is an artifact of who warmed the cache.
    // HARNESS DEFECT, caught by comparing against the device. The first cut
    // wrote `3` to drop_caches WITHOUT a preceding sync(2), and `drop_caches`
    // cannot evict DIRTY pages. `vm.snapshot` had just written 2 GiB in 0.19 s
    // — 11 GB/s, which no NVMe on this box can do — so the whole
    // `memory-ranges` file was still dirty in page cache and BOTH arms were
    // restoring out of RAM while the log claimed the cache had been dropped.
    // `copy` then "read" 2 GiB in 0.23 s = 8.8 GB/s against a measured cold
    // device rate of 2.6 GB/s. That is the P11 shape exactly: a number faster
    // than the hardware beneath it.
    //
    // sync() first, then drop, then PROVE it by printing Cached across the
    // operation. A drop that did not drop is invisible otherwise.
    let cached_before =
        std::fs::read_to_string("/proc/meminfo").map(|s| kb_field(&s, "Cached:")).unwrap_or(0);
    if a.warm_cache {
        println!("### page cache NOT dropped (--warm-cache): Cached={cached_before} kB.");
        println!("###   The memory-ranges file was written seconds ago and is resident.");
        println!("###   This is the best case for `copy`, not a warm pool's steady state.");
    } else {
        let t_sync = Instant::now();
        unsafe { libc::sync() };
        let sync_s = t_sync.elapsed().as_secs_f64();
        let _ = std::fs::write("/proc/sys/vm/drop_caches", "3");
        std::thread::sleep(Duration::from_millis(300));
        let cached_after =
            std::fs::read_to_string("/proc/meminfo").map(|s| kb_field(&s, "Cached:")).unwrap_or(0);
        println!("### sync() took {sync_s:.3} s  <- the REAL durability cost of vm.snapshot;");
        println!(
            "###   vm.snapshot's own {:.3} s was page-cache speed, not durability (P8 said",
            sd.as_secs_f64()
        );
        println!("###   this at 512 MiB; it holds at {} MiB).", a.mem_mib);
        println!(
            "### drop_caches=3: Cached {cached_before} -> {cached_after} kB (delta {} kB)",
            cached_before as i64 - cached_after as i64
        );
    }

    if a.api_probe {
        api_probe(&a, &api_sock, &chlog_b);
        return;
    }

    // ================= [4] restore ======================================
    let mut vmm2: Child = Command::new("cloud-hypervisor")
        .arg("--api-socket")
        .arg(format!("path={}", api_sock.display()))
        .stdout(Stdio::from(std::fs::File::create(&chlog_b).unwrap()))
        .stderr(Stdio::from(std::fs::File::create(&chlog_b).unwrap()))
        .spawn()
        .expect("spawn restore VMM");
    let pid2 = vmm2.id() as i32;
    println!("### restore VMM pid={pid2}");
    if !wait_for_socket(&api_sock, 30) {
        println!("!!! restore VMM API socket never appeared");
        kill_pid(pid2);
        println!("K-RESULT label={} mode={} status=RESTORE_VMM_FAIL", a.label, a.mode);
        return;
    }

    let body = restore_body(&a.mode, &a.snap_dir);
    println!("### PUT vm.restore body={body}");
    let (rc, rb, rd) = api(&api_sock, "PUT", "vm.restore", Some(&body));
    let rss_at_restore = sample_rss(pid2);
    println!("### vm.restore -> {rc} ({:.4} s) {}", rd.as_secs_f64(), rb);
    if rc != 204 && rc != 200 {
        println!("--- ch log:\n{}", std::fs::read_to_string(&chlog_b).unwrap_or_default());
        kill_pid(pid2);
        let _ = vmm2.wait();
        println!(
            "K-RESULT label={} mode={} status=RESTORE_REFUSED code={rc} body={rb}",
            a.label, a.mode
        );
        return;
    }
    println!("### RSS immediately after vm.restore RETURNED, BEFORE resume:");
    println!(
        "###   VmRSS={} kB  RssAnon={}  RssFile={}  RssShmem={}",
        rss_at_restore.vm_rss_kb,
        rss_at_restore.anon_kb,
        rss_at_restore.file_kb,
        rss_at_restore.shmem_kb
    );

    // ================= [5] resume + the sampled window ==================
    let (uc, ub, ud) = api(&api_sock, "PUT", "vm.resume", None);
    let t_resume_ret = Instant::now();
    println!("### vm.resume -> {uc} in {:.4} s {}", ud.as_secs_f64(), ub);

    // One 2 ms loop services BOTH the console watch and the sample schedule.
    let offsets_ms: [u64; 12] = [0, 100, 200, 300, 500, 750, 1000, 1500, 2000, 3000, 6000, 10_000];
    let mut samples: Vec<(u64, Rss, String)> = Vec::new();
    let mut next = 0usize;
    let mut first_tick_at: Option<(u64, f64)> = None;
    let mut external_kill = false;
    let mut threads: Vec<String> = Vec::new();

    loop {
        let el = t_resume_ret.elapsed();
        if !alive(pid2) {
            external_kill = true;
            break;
        }
        if first_tick_at.is_none() {
            let txt = read_console(&console);
            if let Some(n) = first_tick_above(&txt, tick_before) {
                first_tick_at = Some((n, el.as_secs_f64() * 1e3));
            }
        }
        while next < offsets_ms.len() && el.as_millis() as u64 >= offsets_ms[next] {
            let r = sample_rss(pid2);
            let txt = read_console(&console);
            samples.push((offsets_ms[next], r, field_of_last_tick(&txt, "walked_mib")));
            // Mid-fill thread dump. The WALK=0 control shows RSS climbing while
            // the guest touches nothing, so SOMETHING in the VMM is populating
            // memory on its own. Naming the thread that burns the CPU turns
            // "there must be a background fill" into an observation.
            if offsets_ms[next] == 750 {
                threads = thread_dump(pid2);
            }
            next += 1;
        }
        if next >= offsets_ms.len() && first_tick_at.is_some() {
            break;
        }
        if el > Duration::from_secs(30) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    let after_txt = read_console(&console);
    let nonce_after = nonce_of(&after_txt);
    let tick_after = last_tick(&after_txt).unwrap_or(0);
    let banners = after_txt.matches("OND PROBE up").count();
    let bad = field_of_last_tick(&after_txt, "bad");
    let zero = field_of_last_tick(&after_txt, "zero");

    // --- Can an ondemand-restored VM be re-checkpointed? -----------------
    // The binary carries a VmError whose Display is "VM on-demand memory
    // restore is still in progress", so CH clearly has a state in which some
    // operations are refused. A warm pool CHAINS checkpoints — restore, run,
    // re-snapshot — so a mode that cannot be re-snapshotted while paging is
    // outstanding is a driver constraint, not a footnote. Runs after the whole
    // sampled window, so it cannot perturb any number above.
    let (ic, ib, _) = api(&api_sock, "GET", "vm.info", None);
    let state = ib
        .split("\"state\":")
        .nth(1)
        .map(|s| s.chars().take(20).collect::<String>())
        .unwrap_or_else(|| "<unparsed>".into());
    println!();
    println!("=========== RE-CHECKPOINT after a {} restore ===========", a.mode);
    println!("  GET vm.info -> {ic}  state={state}");
    let (p2c, p2b, _) = api(&api_sock, "PUT", "vm.pause", None);
    println!("  PUT vm.pause -> {p2c} {p2b}");
    let resnap = a.snap_dir.with_file_name(format!("resnap-{}", a.label));
    let _ = std::fs::remove_dir_all(&resnap);
    let _ = std::fs::create_dir_all(&resnap);
    let (s2c, s2b, s2d) = api(
        &api_sock,
        "PUT",
        "vm.snapshot",
        Some(&format!(r#"{{"destination_url":"file://{}"}}"#, resnap.display())),
    );
    let (r_app, r_disk) = dir_bytes(&resnap);
    println!(
        "  PUT vm.snapshot -> {s2c} in {:.3} s  apparent={r_app} on_disk={r_disk} {s2b}",
        s2d.as_secs_f64()
    );
    let _ = std::fs::remove_dir_all(&resnap);

    println!();
    println!("=========== RSS TRAJECTORY (the laziness control) ===========");
    println!(
        "  {:>8}  {:>12} {:>12} {:>12} {:>12} {:>6} {:>12}   {:>10}",
        "t_ms",
        "VmRSS_kB",
        "RssAnon_kB",
        "RssFile_kB",
        "RssShmem_kB",
        "alive",
        "hostAvail_kB",
        "guest_MiB"
    );
    for (t, r, w) in &samples {
        println!(
            "  {t:>8}  {:>12} {:>12} {:>12} {:>12} {:>6} {:>12}   {:>10}",
            r.vm_rss_kb, r.anon_kb, r.file_kb, r.shmem_kb, r.alive, r.host_avail_kb, w
        );
    }
    println!("  (t is ms since vm.resume RETURNED; guest_MiB is the guest's own");
    println!("   cumulative verified WALK at that instant)");
    println!();
    println!("--- VMM threads at t=750ms, by CPU (who is populating the memory?)");
    for t in threads.iter().take(6) {
        println!("      {t}");
    }
    println!();

    println!("=========== CORRECTNESS (mandatory; a reboot is FAST) =======");
    println!("  BOOT_NONCE before   {nonce_before}");
    println!("  BOOT_NONCE after    {nonce_after}");
    println!("  last tick before    {tick_before}");
    println!("  last tick after     {tick_after}");
    println!("  boot banners after  {banners}   (must be 0)");
    println!("  walk bad pages      {bad}      (must be 0)");
    println!("  walk zero pages     {zero}      (must be 0 — a page served as ZERO");
    println!("                              is lazy paging that never fetched content)");
    let genuine = !external_kill
        && nonce_after == nonce_before
        && nonce_after != "<none>"
        && banners == 0
        && tick_after > tick_before
        && bad == "0";
    println!(
        "  VERDICT             {}",
        if genuine {
            "+++ GENUINE MEMORY RESTORE"
        } else {
            "!!! NOT a genuine memory restore — DISCARD this trial"
        }
    );

    let (n_first, ms_first) = first_tick_at.unwrap_or((0, -1.0));
    println!();
    println!(
        "K-RESULT label={} mode={} mem_mib={} status={} genuine={} \
restore_s={:.4} resume_s={:.4} first_tick_n={} resume_to_tick_ms={:.1} \
user_visible_ms={:.1} rss_at_restore_kb={} rss_t0_kb={} rss_t200_kb={} \
rss_t1000_kb={} rss_t3000_kb={} rss_t10000_kb={} bad={} zero={} banners={} \
snap_apparent={} snap_ondisk={} walk_mib={} traj={}",
        a.label,
        a.mode,
        a.mem_mib,
        if external_kill {
            "EXTERNAL_KILL"
        } else if genuine {
            "OK"
        } else {
            "NOT_GENUINE"
        },
        genuine,
        rd.as_secs_f64(),
        ud.as_secs_f64(),
        n_first,
        ms_first,
        rd.as_secs_f64() * 1e3 + ud.as_secs_f64() * 1e3 + ms_first,
        rss_at_restore.vm_rss_kb,
        samples.first().map(|s| s.1.vm_rss_kb).unwrap_or(0),
        samples.iter().find(|s| s.0 == 200).map(|s| s.1.vm_rss_kb).unwrap_or(0),
        samples.iter().find(|s| s.0 == 1000).map(|s| s.1.vm_rss_kb).unwrap_or(0),
        samples.iter().find(|s| s.0 == 3000).map(|s| s.1.vm_rss_kb).unwrap_or(0),
        samples.iter().find(|s| s.0 == 10_000).map(|s| s.1.vm_rss_kb).unwrap_or(0),
        bad,
        zero,
        banners,
        app,
        disk,
        a.walk_mib,
        samples
            .iter()
            .map(|(t, r, w)| format!("{t}:{}:{w}", r.vm_rss_kb))
            .collect::<Vec<_>>()
            .join(",")
    );

    if external_kill {
        println!("!!! HARNESS_DEFECT=EXTERNAL_KILL — my VMM pid {pid2} vanished mid-run.");
        println!("!!! The box is SHARED. This trial is INVALID and must be discarded,");
        println!("!!! not averaged in. (A previous probe was corrupted exactly this way.)");
    }
    println!("--- post-restore ch log (first 20 lines):");
    for l in std::fs::read_to_string(&chlog_b).unwrap_or_default().lines().take(20) {
        println!("      {l}");
    }

    kill_pid(pid2);
    let _ = vmm2.wait();
    let _ = std::fs::remove_file(&api_sock);
    let _ = std::fs::remove_file(format!("{}.lock", api_sock.display()));
    let _ = std::fs::remove_file(&rootfs);
}

/// Per-thread name + CPU jiffies, sorted by CPU descending.
///
/// Fields 14/15 of /proc/<tid>/stat are utime/stime. `comm` can contain spaces
/// and parentheses, so the fields are counted from the LAST ')' rather than by
/// splitting the whole line — the classic /proc/stat parsing trap.
fn thread_dump(pid: i32) -> Vec<String> {
    let mut v: Vec<(u64, String)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(format!("/proc/{pid}/task")) {
        for e in rd.flatten() {
            let tid = e.file_name().to_string_lossy().to_string();
            let comm = std::fs::read_to_string(format!("/proc/{pid}/task/{tid}/comm"))
                .unwrap_or_default()
                .trim()
                .to_string();
            let stat =
                std::fs::read_to_string(format!("/proc/{pid}/task/{tid}/stat")).unwrap_or_default();
            let after = stat.rfind(')').map(|i| &stat[i + 1..]).unwrap_or("");
            let f: Vec<&str> = after.split_whitespace().collect();
            let ut: u64 = f.get(11).and_then(|s| s.parse().ok()).unwrap_or(0);
            let st: u64 = f.get(12).and_then(|s| s.parse().ok()).unwrap_or(0);
            v.push((ut + st, format!("tid={tid:<8} comm={comm:<18} cpu_jiffies={}", ut + st)));
        }
    }
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().map(|(_, s)| s).collect()
}

/// Any cloud-hypervisor on the box that is not ours. `comm` is truncated to 15
/// chars by the kernel, so the name to match is `cloud-hyperviso` — matching
/// `cloud-hypervisor` NEVER matches anything (P8 trap 3).
fn foreign_vmms() -> Vec<i32> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/proc") {
        for e in rd.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            if let Ok(pid) = n.parse::<i32>() {
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

/// Discovery: what does `vm.restore` actually accept for `memory_restore_mode`?
/// The CLI spells the values `copy|ondemand`, but the CLI parser and serde are
/// different code paths and the JSON spelling could be `OnDemand`, `on_demand`,
/// or an integer. Rather than guess, send candidates and read serde's own
/// rejection text — which enumerates the accepted variants.
fn api_probe(a: &Args, sock: &Path, chlog: &Path) {
    let url = format!("file://{}", a.snap_dir.display());
    let candidates = vec![
        format!(r#"{{"source_url":"{url}","memory_restore_mode":"BOGUS_VALUE"}}"#),
        format!(r#"{{"source_url":"{url}","memory_restore_mode":"ondemand"}}"#),
        format!(r#"{{"source_url":"{url}","memory_restore_mode":"OnDemand"}}"#),
        format!(r#"{{"source_url":"{url}","memory_restore_mode":"on_demand"}}"#),
        format!(r#"{{"source_url":"{url}","memory_restore_mode":"copy"}}"#),
        format!(r#"{{"source_url":"{url}","memory_restore_mode":"Copy"}}"#),
        format!(r#"{{"source_url":"{url}","prefault":true}}"#),
        format!(r#"{{"source_url":"{url}","prefault":true,"memory_restore_mode":"copy"}}"#),
        format!(r#"{{"source_url":"{url}","prefault":true,"memory_restore_mode":"ondemand"}}"#),
        format!(r#"{{"source_url":"{url}","memory_restore_mode":"ondemand","resume":true}}"#),
        format!(r#"{{"source_url":"{url}","bogus_field":1}}"#),
    ];
    println!();
    println!("=========== API PROBE: what does vm.restore ACCEPT? =========");
    for c in candidates {
        // A fresh VMM per candidate: a successful restore leaves the VMM
        // holding a VM, and every later attempt would then be rejected for
        // "VM is already created" rather than for the field under test.
        let mut v: Child = Command::new("cloud-hypervisor")
            .arg("--api-socket")
            .arg(format!("path={}", sock.display()))
            .stdout(Stdio::from(std::fs::File::create(chlog).unwrap()))
            .stderr(Stdio::from(std::fs::File::create(chlog).unwrap()))
            .spawn()
            .expect("spawn probe VMM");
        let pid = v.id() as i32;
        wait_for_socket(sock, 20);
        let (code, body, d) = api(sock, "PUT", "vm.restore", Some(&c));
        println!("  {code}  {:>8.3}s  {c}", d.as_secs_f64());
        if !body.is_empty() {
            println!("        body: {body}");
        }
        let log = std::fs::read_to_string(chlog).unwrap_or_default();
        for l in log.lines().filter(|l| l.contains("rror") || l.contains("estore")).take(3) {
            println!("        log : {l}");
        }
        kill_pid(pid);
        let _ = v.wait();
        let _ = std::fs::remove_file(sock);
        let _ = std::fs::remove_file(format!("{}.lock", sock.display()));
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("=========== END API PROBE ===================================");
}
