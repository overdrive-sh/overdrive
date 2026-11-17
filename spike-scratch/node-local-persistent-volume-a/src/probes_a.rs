//! Probe A: durability. Throwaway.
use super::*;

#[derive(Clone)]
pub struct ACell {
    pub name: &'static str,
    pub ch: &'static str,        // "stock" | "spike"
    pub host_direct: bool,       // CH disk direct=on
    pub guest_cache_wt: bool,    // guest switches virtio-blk to write-through
    pub no_uring: bool,
    pub no_aio: bool,
    pub guest_odirect: bool,
    pub sync: &'static str,      // fdatasync | fsync | none
    pub n: u64,
    pub pace_ms: u64,
    pub kill_after_acks: Option<u64>,
}

pub struct CellResult { pub check: AckCheck, pub log_summary: String, pub acks: Vec<u64>, pub errs: Vec<String>, pub exit: String, pub extra_blocks_survived: usize }

fn disk_arg(path: &str, direct: bool, no_uring: bool, no_aio: bool, extra: &str) -> String {
    let mut a = format!("path={path},image_type=raw");
    if direct { a.push_str(",direct=on"); }
    if no_uring { a.push_str(",_disable_io_uring=on"); }
    if no_aio { a.push_str(",_disable_aio=on"); }
    if !extra.is_empty() { a.push(','); a.push_str(extra); }
    a
}

/// A1: CH opens a dm-log-writes device (block-device volume).
pub fn run_block_cell(c: &ACell) -> CellResult {
    let (data, logf) = (format!("/nlpv/{}.data", c.name), format!("/nlpv/{}.log", c.name));
    sparse(&data, 128 << 20);
    sparse(&logf, 1 << 30);
    let ld = loop_attach(&data, false);
    let lg = loop_attach(&logf, false);
    let sectors = blk_sectors(&ld);
    let dev = dm_create("log0", &format!("0 {sectors} log-writes {ld} {lg}"));
    let rootdir = format!("/nlpv/run/{}-img", c.name);
    fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(c.name, &ch_binary(c.ch), &rootfs);
    spec.disks.push(disk_arg(&dev, c.host_direct, c.no_uring, c.no_aio, ""));
    let nonce = 0xA11CE;
    spec.cmdline_params = vec![
        "nlpv.mode=ackraw".into(), format!("nlpv.n={}", c.n), format!("nlpv.pace_ms={}", c.pace_ms),
        format!("nlpv.nonce={nonce}"), format!("nlpv.sync={}", c.sync),
        format!("nlpv.odirect={}", c.guest_odirect as u8),
        format!("nlpv.cache={}", if c.guest_cache_wt { "wt" } else { "wb" }),
    ];
    let mut vmm = launch(&spec);
    let pid = vmm.pid();
    let mut acks: Vec<u64> = vec![];
    let mut errs: Vec<String> = vec![];
    let mut backend_noted = false;
    let kill_at = c.kill_after_acks;
    vmm.follow(Duration::from_secs(120), |line, v| {
        if !backend_noted && line.starts_with("READY") {
            backend_noted = true;
            ev(format!("guest {line}; ch pid {pid}: io_uring fds={} ; data-disk fds={}; {}",
                fds_on(pid, "anon_inode:[io_uring]"), fds_on(pid, &dev), proc_limit(pid, "Max file size")));
        }
        if let Some(n) = line.strip_prefix("ACK ") {
            let i: u64 = n.trim().parse().unwrap();
            dm_mark("log0", &format!("ACK{i}"));
            acks.push(i);
            if kill_at == Some(i) { ev(format!("SIGKILL VMM after ACK {i}")); v.kill9(); }
        } else if line.starts_with("ERR ") || line.starts_with("cache_type") || line.starts_with("FIRSTERR") {
            errs.push(line.to_owned()); ev(format!("guest: {line}"));
        }
    });
    let exit = match vmm.wait_exit(Duration::from_secs(15)) {
        Some(s) => format!("{s:?}"), None => { vmm.kill9(); "killed-by-test(timeout)".into() }
    };
    ev(format!("vmm exit: {exit}; stderr: {}", vmm.stderr_text().trim()));
    ev(format!("console tail:\n{}", vmm.console_text().lines().filter(|l| l.contains("NLPV-G")).collect::<Vec<_>>().join("\n")));
    vmm.cleanup();
    dm_mark("log0", "FINAL");
    std::thread::sleep(Duration::from_millis(300));
    dm_remove("log0");
    let log = Log::open(&logf);
    let summary = log.summary();
    ev(format!("log-writes: {summary}"));
    let check = check_acks(&log, &acks, 4096, nonce, 512);
    // How many blocks beyond the last ACK are in the log (informational: survived without an ACK)
    let last = acks.last().copied().unwrap_or(0);
    let mut extra = 0usize;
    for e in &log.entries { if e.flags & (LOG_MARK | LOG_DISCARD | LOG_FLUSH) == 0 && e.nr_sectors > 0 {
        let blk = e.sector * log.sectorsize / 4096; if blk > last { extra += 1; } } }
    ev(format!("RESULT cell={} ch={} host_direct={} guest_wt={} no_uring={} no_aio={} guest_odirect={} sync={} acks={} violations={} missing_marks={} extra_unacked_writes_logged={extra} exit={exit}",
        c.name, c.ch, c.host_direct, c.guest_cache_wt, c.no_uring, c.no_aio, c.guest_odirect, c.sync, check.acked, check.violations.len(), check.missing_marks));
    for v in check.violations.iter().take(20) { ev(format!("VIOLATION {v}")); }
    CellResult { check, log_summary: summary, acks, errs, exit, extra_blocks_survived: extra }
}

pub fn fallocate(path: &str, bytes: u64) {
    use std::os::unix::io::AsRawFd;
    let f = fs::OpenOptions::new().write(true).create(true).open(path).unwrap();
    let r = unsafe { libc::fallocate(f.as_raw_fd(), 0, 0, bytes as i64) };
    assert!(r == 0, "fallocate {path}: {}", std::io::Error::last_os_error());
}

impl Log {
    /// Replay entries[..=end] over a copy of `base` into `out`.
    pub fn replay_onto(&self, base: &str, end_inclusive: usize, out: &str) {
        use std::os::unix::fs::FileExt;
        must(&["cp", "--sparse=always", base, out]);
        let f = fs::OpenOptions::new().write(true).open(out).unwrap();
        for e in &self.entries[..=end_inclusive] {
            if e.flags & (LOG_MARK | LOG_DISCARD) != 0 || e.nr_sectors == 0 { continue; }
            f.write_all_at(&self.read_data(e), e.sector * self.sectorsize).unwrap();
        }
    }
}

/// A2: file volume on XFS on dm-log-writes; CH opens the file.
pub fn run_xfs_file_cell(c: &ACell, prealloc: bool) -> CellResult {
    let (data, logf) = (format!("/nlpv/{}.data", c.name), format!("/nlpv/{}.log", c.name));
    sparse(&data, 320 << 20);
    sparse(&logf, 1 << 30);
    let ld = loop_attach(&data, false);
    must(&["mkfs.xfs", "-q", "-f", "-m", "reflink=1", &ld]);
    let base = format!("/nlpv/{}.base", c.name);
    must(&["cp", "--sparse=always", &data, &base]);
    let lg = loop_attach(&logf, false);
    let sectors = blk_sectors(&ld);
    let dev = dm_create("log0", &format!("0 {sectors} log-writes {ld} {lg}"));
    let mnt = "/nlpv/mnt"; fs::create_dir_all(mnt).unwrap();
    must(&["mount", "-t", "xfs", "-o", "noatime", &dev, mnt]);
    let vol = format!("{mnt}/vol.img");
    let vbytes = 64u64 << 20;
    if prealloc { fallocate(&vol, vbytes); } else { sparse(&vol, vbytes); }
    chown(&vol, VMM_UID, VMM_UID);
    unsafe { libc::sync(); }
    sh("xfs-state", &format!("busybox ls -ls {vol}; /usr/bin/xfs_io -c 'fiemap -v' {vol} 2>&1 | busybox head -5"));
    let rootdir = format!("/nlpv/run/{}-img", c.name);
    fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(c.name, &ch_binary(c.ch), &rootfs);
    spec.fsize = vbytes.max(256 << 20); // >= volume: the RLIMIT_FSIZE question is probe C
    spec.disks.push(disk_arg(&vol, c.host_direct, c.no_uring, c.no_aio, ""));
    let nonce = 0xB0B;
    spec.cmdline_params = vec![
        "nlpv.mode=ackraw".into(), format!("nlpv.n={}", c.n), format!("nlpv.pace_ms={}", c.pace_ms),
        format!("nlpv.nonce={nonce}"), format!("nlpv.sync={}", c.sync),
        format!("nlpv.odirect={}", c.guest_odirect as u8),
        format!("nlpv.cache={}", if c.guest_cache_wt { "wt" } else { "wb" }),
    ];
    let mut vmm = launch(&spec);
    let mut acks: Vec<u64> = vec![];
    let kill_at = c.kill_after_acks;
    vmm.follow(Duration::from_secs(120), |line, v| {
        if let Some(n) = line.strip_prefix("ACK ") {
            let i: u64 = n.trim().parse().unwrap();
            dm_mark("log0", &format!("ACK{i}"));
            acks.push(i);
            if kill_at == Some(i) { ev(format!("SIGKILL VMM after ACK {i}")); v.kill9(); }
        } else if line.starts_with("ERR ") { ev(format!("guest: {line}")); }
    });
    let exit = match vmm.wait_exit(Duration::from_secs(15)) { Some(s) => format!("{s:?}"), None => { vmm.kill9(); "killed-by-test(timeout)".into() } };
    ev(format!("vmm exit: {exit}; stderr: {}", vmm.stderr_text().trim()));
    vmm.cleanup();
    dm_mark("log0", "FINAL");
    std::thread::sleep(Duration::from_millis(300));
    let _ = run(&["umount", mnt]);
    std::thread::sleep(Duration::from_millis(300));
    dm_remove("log0");
    let log = Log::open(&logf);
    let summary = log.summary();
    ev(format!("log-writes: {summary}"));
    // Sample ACKs; replay to the last FLUSH/FUA entry before each mark and read the file out of the recovered XFS.
    let mut mark_idx: BTreeMap<u64, usize> = BTreeMap::new();
    for e in &log.entries { if let Some(n) = e.mark.as_deref().and_then(|m| m.strip_prefix("ACK")) { if let Ok(n) = n.parse() { mark_idx.insert(n, e.idx); } } }
    let nacks = acks.len();
    let mut sample: Vec<u64> = if nacks == 0 { vec![] } else { vec![acks[0], acks[nacks / 4], acks[nacks / 2], acks[3 * nacks / 4], acks[nacks - 1]] };
    sample.sort(); sample.dedup();
    let mut violations = vec![];
    for &i in &sample {
        let Some(&mi) = mark_idx.get(&i) else { violations.push(format!("ACK{i}: no mark")); continue };
        let commit = log.entries[..mi].iter().rposition(|e| e.flags & (LOG_FLUSH | LOG_FUA) != 0 && e.flags & LOG_MARK == 0);
        let Some(ci) = commit else { violations.push(format!("ACK{i}: no flush/FUA entry before its mark")); continue };
        let img = format!("/nlpv/{}.replay", c.name);
        let _ = fs::remove_file(&img);
        log.replay_onto(&base, ci, &img);
        let lr = loop_attach(&img, false);
        let rm = format!("/nlpv/replay-mnt"); fs::create_dir_all(&rm).unwrap();
        let (code, out) = run(&["mount", "-t", "xfs", &lr, &rm]);
        if code != 0 { violations.push(format!("ACK{i}: replayed image (to entry {ci}) does not mount: {out}")); loop_detach(&lr); continue; }
        let content = fs::read(format!("{rm}/vol.img"));
        let _ = run(&["umount", &rm]); loop_detach(&lr);
        match content {
            Ok(b) if b.len() as u64 >= (i + 1) * 4096 => {
                let got = fnv(&b[(i * 4096) as usize..((i + 1) * 4096) as usize]);
                if got != expected_hash(i, nonce, 4096) {
                    violations.push(format!("ACK{i}: replayed to commit entry {ci}: block {i} content mismatch (hash {got:x}); first bytes {:?}", &b[(i * 4096) as usize..(i * 4096) as usize + 8]));
                } else { ev(format!("ACK{i}: replayed to entry {ci}: block {i} present and correct")); }
            }
            Ok(b) => violations.push(format!("ACK{i}: replayed vol.img too short: {}", b.len())),
            Err(e) => violations.push(format!("ACK{i}: cannot read vol.img after replay: {e}")),
        }
    }
    let check = AckCheck { acked: nacks, violations, missing_marks: 0 };
    ev(format!("RESULT cell={} backing=xfs-file prealloc={prealloc} ch={} host_direct={} guest_wt={} no_uring={} no_aio={} sync={} acks={} sampled={:?} violations={} exit={exit}",
        c.name, c.ch, c.host_direct, c.guest_cache_wt, c.no_uring, c.no_aio, c.sync, nacks, sample, check.violations.len()));
    for v in check.violations.iter().take(20) { ev(format!("VIOLATION {v}")); }
    CellResult { check, log_summary: summary, acks, errs: vec![], exit, extra_blocks_survived: 0 }
}

/// Fault injection: dm-linear -> dm-error after ACK `fail_after`; classify what the guest sees.
pub struct FaultResult { pub acks_after: usize, pub errs_after: usize, pub detail: Vec<String>, pub exit: String }
pub fn run_fault_cell(name: &str, ch: &str, wt: bool, no_uring: bool, no_aio: bool, host_direct: bool, sync: &str, fail_after: u64, n: u64) -> FaultResult {
    let data = format!("/nlpv/{name}.data");
    sparse(&data, 128 << 20);
    let ld = loop_attach(&data, false);
    let sectors = blk_sectors(&ld);
    let dev = dm_create("data0", &format!("0 {sectors} linear {ld} 0"));
    let rootdir = format!("/nlpv/run/{name}-img"); fs::create_dir_all(&rootdir).unwrap();
    let rootfs = build_inner_rootfs(&rootdir, 24);
    let mut spec = VmmSpec::new(name, &ch_binary(ch), &rootfs);
    spec.disks.push(disk_arg(&dev, host_direct, no_uring, no_aio, ""));
    spec.cmdline_params = vec!["nlpv.mode=ackraw".into(), format!("nlpv.n={n}"), "nlpv.pace_ms=60".into(), "nlpv.nonce=3".into(),
        format!("nlpv.sync={sync}"), format!("nlpv.cache={}", if wt { "wt" } else { "wb" }), format!("nlpv.odirect={}", wt as u8)];
    spec.ch_extra = vec!["-v".into(), "--log-file".into(), format!("/nlpv/run/{name}/ch.log")];
    let mut vmm = launch(&spec);
    let (mut acks_after, mut errs_after) = (0usize, 0usize);
    let mut detail = vec![];
    let mut injected = false;
    vmm.follow(Duration::from_secs(120), |line, _v| {
        if let Some(i) = line.strip_prefix("ACK ").and_then(|x| x.trim().parse::<u64>().ok()) {
            if i == fail_after && !injected {
                injected = true;
                dm_reload("data0", &format!("0 {sectors} error"));
                ev(format!("FAULT INJECTED after ACK {i}: data0 table is now `error`"));
            } else if injected { acks_after += 1; detail.push(format!("ACK {i} (after injection)")); }
        } else if line.starts_with("ERR ") && injected { errs_after += 1; detail.push(line.to_owned()); }
    });
    let exit = match vmm.wait_exit(Duration::from_secs(15)) { Some(s) => format!("{s:?}"), None => { vmm.kill9(); "killed".into() } };
    ev(format!("guest console (NLPV-G + I/O errors):\n{}", vmm.console_text().lines().filter(|l| l.contains("NLPV-G") || l.contains("I/O error") || l.contains("vdb")).collect::<Vec<_>>().join("\n")));
    sh("chlog", &format!("busybox grep -iE 'fsync|Failed to process|Request failed|WARN|ERROR' /nlpv/run/{name}/ch.log | busybox head -6 | busybox cut -c1-260; busybox grep -ciE 'Failed to process' /nlpv/run/{name}/ch.log"));
    vmm.cleanup();
    dm_remove("data0");
    ev(format!("FAULTRESULT cell={name} ch={ch} wt={wt} no_uring={no_uring} no_aio={no_aio} direct={host_direct} sync={sync} acks_after_injection={acks_after} errs_after_injection={errs_after} exit={exit}"));
    FaultResult { acks_after, errs_after, detail, exit }
}
