//! V-4 / V-5 / V-10 probes. Every test is placed `guest-declared` through the
//! repo wrapper's ELF record, so it runs with the same local-mode vsock
//! network namespace a shared-guest test would get.
use std::fs;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use ownerprobe::{
    OwnerSpec, declare_placement, ensure_owner, hello, ns_link, nspid_of, pidfd_open, pidfd_probe, proc_state,
    readable, request, scan_procs,
};
use overdrive_testing::guest_kit::cid_bands::FWD_GUEST_BOUNDARY;

const OWNER: &str = env!("CARGO_BIN_EXE_ownerprobe");

macro_rules! placed {
    ($(fn $name:ident() $body:block)*) => { $(
        #[test]
        fn $name() {
            declare_placement!(::core::stringify!($name));
            $body
        }
    )* };
}

fn run_id() -> String {
    std::env::var("OVERDRIVE_METAL_RUN_ID").expect("OVERDRIVE_METAL_RUN_ID (ordinary metal run)")
}

/// `/run/ovd-gk/<run id>/shared`, mode 0700, created idempotently.
fn shared_dir() -> PathBuf {
    let dir = PathBuf::from("/run/ovd-gk").join(run_id()).join("shared");
    let mut b = fs::DirBuilder::new();
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    if let Err(e) = b.create(&dir) {
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists, "mkdir {}: {e}", dir.display());
    }
    dir
}

fn ppid() -> u32 {
    std::os::unix::process::parent_id()
}

fn anchor() -> u32 {
    std::env::var("OVERDRIVE_TEST_RUN_ANCHOR_PID").ok().and_then(|v| v.parse().ok()).unwrap_or_else(ppid)
}

fn spec(dir: &Path, key: &str, vm: bool, pidns: bool, cid: u32, startup_ms: u64, anchor: u32) -> OwnerSpec {
    OwnerSpec { dir: dir.to_path_buf(), key: key.into(), anchor, vm, pidns, cid, startup_ms }
}

fn members(spec: &OwnerSpec) -> Vec<(u32, String, String)> {
    fs::read_to_string(spec.file("members"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            Some((f.first()?.parse().ok()?, f.get(1)?.to_string(), f.get(2)?.to_string()))
        })
        .collect()
}

#[derive(Default, Debug)]
struct Gone {
    zombie_or_gone_ms: Option<u128>,
    gone_ms: Option<u128>,
}

/// Polls `pids` every ~1 ms from `t0` until each is gone or `bound` passes.
fn watch_gone(pids: &[u32], t0: Instant, bound: Duration) -> Vec<(u32, Gone)> {
    let mut seen: Vec<(u32, Gone)> = pids.iter().map(|p| (*p, Gone::default())).collect();
    while t0.elapsed() < bound {
        for (pid, g) in &mut seen {
            let st = proc_state(*pid);
            let ms = t0.elapsed().as_micros();
            if g.zombie_or_gone_ms.is_none() && matches!(st, None | Some('Z' | 'X')) {
                g.zombie_or_gone_ms = Some(ms);
            }
            if g.gone_ms.is_none() && st.is_none() {
                g.gone_ms = Some(ms);
            }
        }
        if seen.iter().all(|(_, g)| g.gone_ms.is_some()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    seen
}

fn report_gone(label: &str, seen: &[(u32, Gone)]) {
    for (pid, g) in seen {
        eprintln!(
            "OWNERPROBE_V4 {label} pid={pid} zombie_or_gone_us={:?} gone_us={:?}",
            g.zombie_or_gone_ms, g.gone_ms
        );
    }
}

fn kill9(pid: u32) {
    assert!(pid > 1 && pid != std::process::id() && pid != ppid(), "refusing to signal pid {pid}");
    // SAFETY: signalling a pid this test just learned from its own owner.
    let r = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    assert_eq!(r, 0, "kill -9 {pid}: errno {}", ownerprobe::errno());
}

fn vm_owner(key: &str, cid_slot: u32, pidns: bool, anchor_pid: u32) -> (OwnerSpec, ownerprobe::Hello) {
    let sp = spec(&shared_dir(), key, true, pidns, FWD_GUEST_BOUNDARY.raw(cid_slot), 0, anchor_pid);
    let t = Instant::now();
    let (s, started) = ensure_owner(&sp, Path::new(OWNER), true).unwrap_or_else(|e| panic!("{e}"));
    drop(s);
    let h = hello(&sp.sock()).expect("hello");
    let exec = request(&sp.sock(), "EXEC").expect("exec");
    eprintln!(
        "OWNERPROBE_V4 {key} elected={started} elect_to_exec_ms={} hello={h:?} exec={exec:?} sock_len={}",
        t.elapsed().as_millis(),
        sp.sock().as_os_str().len()
    );
    assert_eq!(exec, "EXIT\t0", "the guest exec through the production launcher must succeed");
    (sp, h)
}

/// Starts a 40 s exec inside the guest (owner busy), returns once it is running.
fn start_slow_exec(sp: &OwnerSpec) -> std::thread::JoinHandle<Result<String, String>> {
    let sock = sp.sock();
    let h = std::thread::spawn(move || request(&sock, "EXECSLEEP"));
    std::thread::sleep(Duration::from_millis(1500));
    h
}

placed! {
    fn v10_wrapper_parent_is_nextest() {
        let me = std::process::id();
        let parent = ppid();
        let read = |p: String| fs::read_to_string(p).map(|s| s.trim().to_owned()).unwrap_or_else(|e| format!("?{e}"));
        let comm = read(format!("/proc/{parent}/comm"));
        let exe = fs::read_link(format!("/proc/{parent}/exe")).map(|p| p.display().to_string()).unwrap_or_else(|e| format!("?{e}"));
        let cmdline = read(format!("/proc/{parent}/cmdline")).replace('\0', " ");
        let gp = fs::read_to_string(format!("/proc/{parent}/stat")).ok()
            .and_then(|s| s.rsplit_once(')').map(|(_, r)| r.split_whitespace().nth(1).unwrap_or("0").to_owned()))
            .unwrap_or_default();
        let gp_comm = read(format!("/proc/{gp}/comm"));
        let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
        let cap = status.lines().find(|l| l.starts_with("CapEff")).unwrap_or("").to_owned();
        let uid = status.lines().find(|l| l.starts_with("Uid")).unwrap_or("").to_owned();
        eprintln!("OWNERPROBE_V10 test_pid={me} ppid={parent} parent_comm={comm} parent_exe={exe} grandparent={gp}/{gp_comm}");
        eprintln!("OWNERPROBE_V10 parent_cmdline={cmdline}");
        eprintln!("OWNERPROBE_V10 {uid} {cap} pidns={} netns={} env_anchor={:?}", ns_link("pid"), ns_link("net"),
            std::env::var("OVERDRIVE_TEST_RUN_ANCHOR_PID").ok());
        // what the shim recorded: the wrapper's own parent, and that the
        // wrapper pid is the test pid (the exec chain keeps it).
        let log = fs::read_to_string(shared_dir().join("wrapper-observe.log")).unwrap_or_default();
        let mine = log.lines().find(|l| l.contains("test=v10_wrapper_parent_is_nextest ")).unwrap_or("").to_owned();
        eprintln!("OWNERPROBE_V10 shim_line={mine}");
        assert!(mine.contains(&format!("wrapper_pid={me} ")), "wrapper pid == test pid (exec chain): {mine}");
        assert!(mine.contains(&format!("wrapper_ppid={parent} ")), "wrapper parent == test parent: {mine}");
        assert!(mine.contains("parent_comm=cargo-nextest"), "wrapper parent is the nextest process: {mine}");

        // pidfd_open on the parent, as root, in this namespace set.
        let fd = pidfd_open(parent).unwrap_or_else(|e| panic!("pidfd_open({parent}) errno={e}"));
        eprintln!("OWNERPROBE_V10 pidfd_open({parent}) OK fd={}", fd.as_raw_fd());
        eprintln!("OWNERPROBE_V10 pidfd readable now (anchor exited?) = {}", readable(fd.as_raw_fd(), 0));
        eprintln!("OWNERPROBE_V10 pidfd_send_signal(sig 0) = {:?}", pidfd_probe(&fd));
        assert!(comm.contains("nextest"));
        assert!(!readable(fd.as_raw_fd(), 0));
        pidfd_probe(&fd).expect("signal-0 probe on the anchor pidfd");
    }

    fn v4_owner_pidns_sigkill_leaves_no_vmm() {
        let (sp, h) = vm_owner("v4a", 1, true, anchor());
        assert_eq!(h.ns_pid, 1, "the owner proper is init of its PID namespace");
        assert!(h.host_pid > 1 && h.host_pid != h.ns_pid, "host pid must come from /proc/self, got {}", h.host_pid);
        assert_ne!(h.pidns, ns_link("pid"), "owner pid namespace differs from the test's");
        let mem = members(&sp);
        for m in &mem {
            eprintln!("OWNERPROBE_V4 v4a member host_pid={} nspid={} comm={}", m.0, m.1, m.2);
        }
        let vmm: Vec<u32> = mem.iter().filter(|m| m.2.contains("cloud-hyperv")).map(|m| m.0).collect();
        assert!(!vmm.is_empty(), "a VMM must be a descendant of the owner");
        let mut watch: Vec<u32> = mem.iter().map(|m| m.0).collect();
        watch.push(h.host_pid);
        eprintln!("OWNERPROBE_V4 v4a owner_log:\n{}", fs::read_to_string(sp.file("owner.log")).unwrap_or_default());
        let slow = start_slow_exec(&sp);
        eprintln!("OWNERPROBE_V4 v4a killing owner {} while a 40 s exec runs in the guest", h.host_pid);
        let t0 = Instant::now();
        kill9(h.host_pid);
        let seen = watch_gone(&watch, t0, Duration::from_secs(15));
        report_gone("v4a SIGKILL owner", &seen);
        let slowest = seen.iter().filter_map(|(_, g)| g.gone_ms).max();
        eprintln!("OWNERPROBE_V4 v4a all_gone_us={slowest:?} still_alive={:?}",
            seen.iter().filter(|(_, g)| g.gone_ms.is_none()).map(|(p, _)| *p).collect::<Vec<_>>());
        eprintln!("OWNERPROBE_V4 v4a client of the killed owner saw: {:?}", slow.join().unwrap());
        assert!(seen.iter().all(|(_, g)| g.gone_ms.is_some()), "every member of the owner's namespace is gone");
        // a replacement owner (next arrival elects) boots on the SAME CID
        let (sp2, h2) = vm_owner("v4a", 1, true, anchor());
        eprintln!("OWNERPROBE_V4 v4a replacement owner host_pid={} (old {})", h2.host_pid, h.host_pid);
        assert_ne!(h2.host_pid, h.host_pid);
        let mem2: Vec<u32> = members(&sp2).iter().map(|m| m.0).chain([h2.host_pid]).collect();
        let t1 = Instant::now();
        let _ = request(&sp2.sock(), "QUIT");
        let seen2 = watch_gone(&mem2, t1, Duration::from_secs(15));
        report_gone("v4a QUIT owner (normal exit, guest NOT killed by owner)", &seen2);
        assert!(seen2.iter().all(|(_, g)| g.gone_ms.is_some()));
    }

    fn v4_control_without_pidns_orphans_the_vmm() {
        let (sp, h) = vm_owner("v4c", 2, false, anchor());
        let mem = members(&sp);
        let vmm: Vec<u32> = mem.iter().filter(|m| m.2.contains("cloud-hyperv")).map(|m| m.0).collect();
        eprintln!("OWNERPROBE_V4 v4c owner_host_pid={} members={mem:?}", h.host_pid);
        assert!(!vmm.is_empty());
        let slow = start_slow_exec(&sp);
        let t0 = Instant::now();
        kill9(h.host_pid);
        let seen = watch_gone(&[h.host_pid], t0, Duration::from_secs(5));
        eprintln!("OWNERPROBE_V4 v4c client of the killed owner saw: {:?}", slow.join().unwrap());
        report_gone("v4c SIGKILL owner (no pidns)", &seen);
        std::thread::sleep(Duration::from_secs(3));
        let alive: Vec<(u32, Option<char>)> = vmm.iter().map(|p| (*p, proc_state(*p))).collect();
        eprintln!("OWNERPROBE_V4 v4c vmm state 3s+ after owner death: {alive:?}");
        // tidy: kill only the VMM pids recorded above
        for p in &vmm { kill9(*p); }
        let seen = watch_gone(&vmm, Instant::now(), Duration::from_secs(10));
        report_gone("v4c cleanup (test killed its own orphan VMM)", &seen);
        assert!(alive.iter().any(|(_, st)| matches!(st, Some(c) if *c != 'Z' && *c != 'X')),
            "control: without the PID namespace the VMM outlives the owner");
    }

    fn v4_anchor_pidfd_fires_and_namespace_takes_the_vmm() {
        let mut standin = Command::new("sleep").arg("600").spawn().expect("stand-in anchor");
        let (sp, h) = vm_owner("v4d", 3, true, standin.id());
        let mem = members(&sp);
        let mut watch: Vec<u32> = mem.iter().map(|m| m.0).collect();
        watch.push(h.host_pid);
        let t0_wall = ownerprobe::now_ms();
        let t0 = Instant::now();
        standin.kill().expect("kill stand-in anchor");
        let seen = watch_gone(&watch, t0, Duration::from_secs(15));
        let _ = standin.wait();
        report_gone("v4d anchor killed", &seen);
        let log = fs::read_to_string(sp.file("owner.log")).unwrap_or_default();
        let fired = log.lines().find(|l| l.contains("ANCHOR FIRED")).unwrap_or("").to_owned();
        eprintln!("OWNERPROBE_V4 v4d anchor_kill_wall_ms={t0_wall} owner_line={fired}");
        eprintln!("OWNERPROBE_V4 v4d owner_log:\n{log}");
        assert!(!fired.is_empty(), "the owner must observe the anchor pidfd firing");
        assert!(seen.iter().all(|(_, g)| g.gone_ms.is_some()), "owner and VMM gone after the anchor died");
    }

    fn v4_dead_anchor_refuses_to_start() {
        let mut c = Command::new("true").spawn().expect("true");
        let dead = c.id();
        c.wait().expect("wait");
        let sp = spec(&shared_dir(), "v4e", false, true, 0, 0, dead);
        let r = ensure_owner(&sp, Path::new(OWNER), true);
        eprintln!("OWNERPROBE_V4 v4e dead anchor {dead}: ensure_owner = {:?}", r.as_ref().map(|_| "started").map_err(String::as_str));
        assert!(r.is_err(), "an owner whose anchor is already gone must refuse to start");
    }

    fn v4_real_anchor_hold() {
        // Hold the run open on a real nextest anchor so the box-side script can
        // SIGKILL nextest and watch the owner and its VMM.
        let (sp, h) = vm_owner("v4r", 4, true, anchor());
        let mem = members(&sp);
        eprintln!("OWNERPROBE_V4 v4r owner_host_pid={} anchor={} members={mem:?}", h.host_pid, anchor());
        fs::write(shared_dir().join("v4r.ready"), format!("{}\n", h.host_pid)).unwrap();
        eprintln!("OWNERPROBE_V4 v4r READY owner={} (holding 120 s for the external kill)", h.host_pid);
        std::thread::sleep(Duration::from_secs(120));
    }
}

/// Boots a VM owner on `slot`, starts the slow exec, SIGKILLs the owner and
/// returns (microseconds until every member and the owner are gone, owner log).
fn kill_cycle(key: &str, slot: u32) -> u128 {
    let (sp, h) = vm_owner(key, slot, true, anchor());
    let mem: Vec<u32> = members(&sp).iter().map(|m| m.0).chain([h.host_pid]).collect();
    assert!(h.ns_pid == 1 && h.host_pid > 1);
    let slow = start_slow_exec(&sp);
    let t0 = Instant::now();
    kill9(h.host_pid);
    let seen = watch_gone(&mem, t0, Duration::from_secs(15));
    let _ = slow.join();
    assert!(seen.iter().all(|(_, g)| g.gone_ms.is_some()), "{key}: members left: {seen:?}");
    let us = seen.iter().filter_map(|(_, g)| g.gone_ms).max().unwrap();
    eprintln!("OWNERPROBE_V4 bound {key} members={} all_gone_us={us}", mem.len());
    us
}

placed! {
    fn v4_bound_sequential() {
        let all: Vec<u128> = (5..11).map(|i| kill_cycle(&format!("v4s{i}"), i)).collect();
        eprintln!("OWNERPROBE_V4 bound sequential n={} all_gone_us={all:?} max_ms={}", all.len(), all.iter().max().unwrap() / 1000);
    }

    fn v4_bound_simultaneous() {
        let hs: Vec<_> = [11_u32, 12, 13, 15].into_iter()
            .map(|i| std::thread::spawn(move || kill_cycle(&format!("v4p{i}"), i))).collect();
        let all: Vec<u128> = hs.into_iter().map(|h| h.join().unwrap()).collect();
        eprintln!("OWNERPROBE_V4 bound simultaneous n={} all_gone_us={all:?} max_ms={}", all.len(), all.iter().max().unwrap() / 1000);
    }
}

/// Eight tests arrive at once (barrier), then race to reach the owner.
fn v5_family(fam: &str, n: usize, use_lock: bool) {
    let dir = shared_dir();
    let bar = dir.join(format!("{fam}-barrier"));
    let _ = fs::create_dir_all(&bar);
    fs::write(bar.join(format!("arrive-{}", std::process::id())), b"").unwrap();
    let wait = |d: &Path, want: usize| {
        let t = Instant::now();
        while fs::read_dir(d).map(|r| r.count()).unwrap_or(0) < want {
            assert!(t.elapsed() < Duration::from_secs(90), "barrier {} never reached {want}", d.display());
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    wait(&bar, n);
    let sp = spec(&dir, &format!("{fam}-owner"), false, true, 0, 1500, anchor());
    let t = Instant::now();
    let (s, started) = ensure_owner(&sp, Path::new(OWNER), use_lock).unwrap_or_else(|e| panic!("{e}"));
    drop(s);
    let h = hello(&sp.sock()).expect("hello");
    let mine = ns_link("net");
    let res = dir.join(format!("{fam}-results"));
    let _ = fs::create_dir_all(&res);
    fs::write(
        res.join(format!("r-{}", std::process::id())),
        format!("owner_host_pid={} owner_netns={} my_netns={mine} started_owner={started} attach_ms={}\n", h.host_pid, h.netns, t.elapsed().as_millis()),
    )
    .unwrap();
    wait(&res, n);
    let all: Vec<String> = fs::read_dir(&res).unwrap().flatten().map(|e| fs::read_to_string(e.path()).unwrap()).collect();
    let owners: std::collections::BTreeSet<&str> =
        all.iter().filter_map(|l| l.split_whitespace().next()).collect();
    let started_count = all.iter().filter(|l| l.contains("started_owner=true")).count();
    let starts_logged = fs::read_to_string(sp.file("owners")).unwrap_or_default().lines().count();
    let my_netns_set: std::collections::BTreeSet<String> =
        all.iter().filter_map(|l| l.split_whitespace().find(|w| w.starts_with("my_netns=")).map(str::to_owned)).collect();
    let crossed = all.iter().filter(|l| {
        let g = |p: &str| l.split_whitespace().find(|w| w.starts_with(p)).unwrap_or("").to_owned();
        g("owner_netns=").trim_start_matches("owner_netns=") != g("my_netns=").trim_start_matches("my_netns=")
    }).count();
    eprintln!(
        "OWNERPROBE_V5 fam={fam} lock={use_lock} clients={n} distinct_owner_pids={} elected_by_client={started_count} owner_start_lines={starts_logged} distinct_client_netns={} clients_in_other_netns_than_owner={crossed}",
        owners.len(), my_netns_set.len()
    );
    eprintln!("OWNERPROBE_V5 fam={fam} sock={} len={} mode={:?}", sp.sock().display(), sp.sock().as_os_str().len(),
        fs::metadata(&dir).map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions())));
    if use_lock {
        assert_eq!(starts_logged, 1, "exactly one owner started");
        assert_eq!(started_count, 1, "exactly one client elected");
        assert_eq!(owners.len(), 1, "every client reached the same owner");
        assert!(crossed >= n - 1, "clients connected from other network namespaces");
    } else {
        assert!(starts_logged > 1, "control: without the lock several owners start (saw {starts_logged})");
    }
}

placed! {
    fn v5_elect_0() { v5_family("v5e", 8, true) }
    fn v5_elect_1() { v5_family("v5e", 8, true) }
    fn v5_elect_2() { v5_family("v5e", 8, true) }
    fn v5_elect_3() { v5_family("v5e", 8, true) }
    fn v5_elect_4() { v5_family("v5e", 8, true) }
    fn v5_elect_5() { v5_family("v5e", 8, true) }
    fn v5_elect_6() { v5_family("v5e", 8, true) }
    fn v5_elect_7() { v5_family("v5e", 8, true) }
    fn v5_nolock_0() { v5_family("v5c", 8, false) }
    fn v5_nolock_1() { v5_family("v5c", 8, false) }
    fn v5_nolock_2() { v5_family("v5c", 8, false) }
    fn v5_nolock_3() { v5_family("v5c", 8, false) }
    fn v5_nolock_4() { v5_family("v5c", 8, false) }
    fn v5_nolock_5() { v5_family("v5c", 8, false) }
    fn v5_nolock_6() { v5_family("v5c", 8, false) }
    fn v5_nolock_7() { v5_family("v5c", 8, false) }
}

#[allow(dead_code)]
fn unused() {
    let _ = (scan_procs(), nspid_of(1));
}
