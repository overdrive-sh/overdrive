//! listen-hooks spike harness (throwaway). Loads the copied guest object (+ listen
//! arm), attaches in a dedicated child cgroup (never the host root), runs
//! unmodified `wl` workloads inside that cgroup, prints PREDICT / ACTUAL per case.
mod storage;

use std::ffi::CString;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use aya::maps::{Array, HashMap as AyaHashMap, MapData, PerCpuArray, RingBuf};
use aya::programs::{CgroupAttachMode, CgroupSock, CgroupSockAddr, SockOps};
use aya::{Ebpf, EbpfLoader};

type R<T> = Result<T, Box<dyn std::error::Error>>;
static SEQ: AtomicU32 = AtomicU32::new(0);

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Ev {
    kind: u32,
    family: u32,
    port: u32,
    pid: u32,
    addr4: u32,
    addr6: [u32; 4],
    protocol: u32,
    sock_type: u32,
    aux: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Child {
    state: u32,
    pad: u32,
    cookie: u64,
    ip: u32,
    port: u16,
    p: u16,
}
unsafe impl aya::Pod for Child {}

fn show(e: &Ev) -> String {
    let kind = match e.kind {
        1 => "TCP_LISTEN_CB",
        2 => "UDP_POST_BIND",
        3 => "OTHER_POST_BIND",
        _ => "?",
    };
    let fam = match e.family {
        2 => "AF_INET".to_string(),
        10 => "AF_INET6".to_string(),
        f => format!("fam{f}"),
    };
    let v4 = std::net::Ipv4Addr::from(e.addr4.to_ne_bytes());
    let mut b6 = [0u8; 16];
    for (i, w) in e.addr6.iter().enumerate() {
        b6[i * 4..i * 4 + 4].copy_from_slice(&w.to_ne_bytes());
    }
    let v6 = std::net::Ipv6Addr::from(b6);
    let addr = if e.family == 2 { v4.to_string() } else { format!("[{v6}]") };
    let aux = if e.kind == 1 && e.family == 10 {
        format!(" getsockopt(V6ONLY)_rc={}", e.aux as i32)
    } else {
        String::new()
    };
    format!(
        "{kind:<16} {fam:<8} local={addr}:{} pid={} proto={} type={}{aux}",
        e.port, e.pid, e.protocol, e.sock_type
    )
}

struct Rig {
    label: String,
    bpf: Ebpf,
    cg: PathBuf,
    ring: Option<RingBuf<MapData>>,
    has_listen: bool,
}

impl Drop for Rig {
    fn drop(&mut self) {
        // The cgroup is removed once the workloads have exited; the programs detach when
        // `bpf` (declared before use here) drops its links just after this body.
        let cg = self.cg.clone();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = fs::remove_dir(&cg);
    }
}

fn mk_cg(label: &str) -> R<PathBuf> {
    let p = PathBuf::from(format!(
        "/sys/fs/cgroup/lh-{}-{}-{label}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&p)?;
    Ok(p)
}

impl Rig {
    fn load(obj: &Path, label: &str, ring_bytes: Option<u32>) -> R<Rig> {
        let bytes = fs::read(obj)?;
        let staging = PathBuf::from(format!(
            "/sys/fs/bpf/lh-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&staging)?;
        storage::create_active_storage(&staging.join("ACTIVE_CORRELATION"))?;
        let mut l = EbpfLoader::new();
        l.allow_unsupported_maps().map_pin_path(&staging);
        if let Some(b) = ring_bytes {
            l.set_max_entries("LISTEN_EVENTS", b);
        }
        let res = l.load(&bytes);
        for n in ["ACTIVE_CORRELATION", "CHILD_CORRELATION"] {
            let _ = fs::remove_file(staging.join(n));
        }
        let _ = fs::remove_dir(&staging);
        let mut bpf = res?;
        let has_listen = bpf.map("LISTEN_EVENTS").is_some();
        let ring = match bpf.take_map("LISTEN_EVENTS") {
            Some(m) => Some(RingBuf::try_from(m)?),
            None => None,
        };
        let cg = mk_cg(label)?;
        Ok(Rig { label: label.to_string(), bpf, cg, ring, has_listen })
    }

    fn cgfile(&self) -> R<File> {
        Ok(File::open(&self.cg)?)
    }

    fn attach_sockops(&mut self, name: &str, mode: CgroupAttachMode) -> R<()> {
        let cg = self.cgfile()?;
        let p: &mut SockOps = self.bpf.program_mut(name).ok_or("no prog")?.try_into()?;
        if p.fd().is_err() {
            p.load()?;
        }
        p.attach(cg, mode)?;
        Ok(())
    }
    fn attach_cgsock(&mut self, name: &str, mode: CgroupAttachMode) -> R<()> {
        let cg = self.cgfile()?;
        let p: &mut CgroupSock = self.bpf.program_mut(name).ok_or("no prog")?.try_into()?;
        if p.fd().is_err() {
            p.load()?;
        }
        p.attach(cg, mode)?;
        Ok(())
    }
    fn attach_cgaddr(&mut self, name: &str, mode: CgroupAttachMode) -> R<()> {
        let cg = self.cgfile()?;
        let p: &mut CgroupSockAddr = self.bpf.program_mut(name).ok_or("no prog")?.try_into()?;
        if p.fd().is_err() {
            p.load()?;
        }
        p.attach(cg, mode)?;
        Ok(())
    }
    /// The guest's six aya programs exactly as overdrive-init attaches them (`Single`),
    /// plus the two post_bind programs when the object has them.
    fn attach_guest_set(&mut self) -> R<()> {
        let m = CgroupAttachMode::Single;
        self.attach_sockops("tcp_correlation", m)?;
        for n in ["connect4", "sendmsg4", "getpeername4", "recvmsg4"] {
            self.attach_cgaddr(n, m)?;
        }
        self.attach_cgsock("sock_release", m)?;
        if self.has_listen {
            self.attach_cgsock("post_bind4", m)?;
            self.attach_cgsock("post_bind6", m)?;
        }
        Ok(())
    }
    fn insns(&self) -> Vec<String> {
        let mut v = Vec::new();
        for (name, p) in self.bpf.programs() {
            let n = p.info().ok().and_then(|i| i.verified_instruction_count());
            v.push(format!("{name}={}", n.map_or("?".to_string(), |n| n.to_string())));
        }
        v.sort();
        v
    }
    fn counters(&self) -> [u64; 4] {
        let mut out = [0u64; 4];
        if let Some(m) = self.bpf.map("LISTEN_COUNTERS") {
            if let Ok(a) = PerCpuArray::<_, u64>::try_from(m) {
                for i in 0..4u32 {
                    if let Ok(v) = a.get(&i, 0) {
                        out[i as usize] = v.iter().sum();
                    }
                }
            }
        }
        out
    }
    fn drain(&mut self) -> Vec<Ev> {
        let mut v = Vec::new();
        if let Some(r) = self.ring.as_mut() {
            while let Some(item) = r.next() {
                assert_eq!(item.len(), std::mem::size_of::<Ev>());
                v.push(unsafe { std::ptr::read_unaligned(item.as_ptr().cast::<Ev>()) });
            }
        }
        v
    }
}

fn cg_command(cg: &Path, wl: &Path, args: &[String]) -> Command {
    let procs = CString::new(cg.join("cgroup.procs").to_str().unwrap()).unwrap();
    let mut c = Command::new(wl);
    c.args(args);
    unsafe {
        c.pre_exec(move || {
            let fd = libc::open(procs.as_ptr(), libc::O_WRONLY);
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let w = libc::write(fd, b"0".as_ptr().cast(), 1);
            libc::close(fd);
            if w < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
        });
    }
    c
}

fn run_wl(cg: &Path, wl: &Path, args: &[String]) -> (i32, String, String) {
    let out = cg_command(cg, wl, args).output().expect("spawn wl");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn strs(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

fn case(rig: &mut Rig, wl: &Path, label: &str, predict: &str, args: &[&str]) {
    println!("--- {label}   [wl {}]", args.join(" "));
    println!("PREDICT: {predict}");
    let _ = rig.drain();
    let (rc, out, err) = run_wl(&rig.cg, wl, &strs(args));
    for l in out.lines() {
        println!("  wl> {l}");
    }
    if !err.trim().is_empty() {
        println!("  wl stderr> {}", err.trim());
    }
    let evs = rig.drain();
    println!("ACTUAL: wl exit={rc}; {} event(s)", evs.len());
    for e in &evs {
        println!("  ev> {}", show(e));
    }
}

fn primary_ip() -> String {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    s.connect("192.0.2.1:9").unwrap();
    s.local_addr().unwrap().ip().to_string()
}

fn v32(rig: &mut Rig, wl: &Path, ip: &str) {
    println!("\n================ V32 TCP listen ================");
    println!("(post_bind4/6 are attached too, so a TCP bind() also yields an OTHER_POST_BIND event; only TCP_LISTEN_CB lines are listener reports)");
    case(rig, wl, "T1 v4 wildcard listen", "1 TCP_LISTEN_CB AF_INET 0.0.0.0:<assigned port printed by wl>", &["tcp4_wild"]);
    case(rig, wl, "T2 v4 loopback listen", "1 TCP_LISTEN_CB local=127.0.0.1:<port> (address readable, so a loopback exclusion is implementable)", &["tcp4_loop"]);
    case(rig, wl, "T3 v4 specific-address listen", "1 TCP_LISTEN_CB local=<primary ip>:<port>", &["tcp4_specific", ip]);
    case(rig, wl, "T4 v6 [::1] listen", "1 TCP_LISTEN_CB AF_INET6 local=[::1]:<port>", &["tcp6_loop"]);
    case(rig, wl, "T5 v6 wildcard V6ONLY=1", "1 TCP_LISTEN_CB AF_INET6 [::]:<port>; V6ONLY not readable (getsockopt rc = -EINVAL = -22, filter.c:5562-5573)", &["tcp6_wild_v6only1"]);
    case(rig, wl, "T6 v6 wildcard V6ONLY=0 (dual-stack)", "1 TCP_LISTEN_CB AF_INET6 [::]:<port>, indistinguishable from T5", &["tcp6_wild_v6only0"]);
    case(rig, wl, "T7 listen() x3 on the same socket", "exactly 1 TCP_LISTEN_CB (backlog-only calls, af_inet.c:211)", &["tcp_relisten"]);
    case(rig, wl, "T8 listen, shutdown(SHUT_RD), listen again", "2 TCP_LISTEN_CB if the second listen succeeds (state returns to CLOSE then LISTEN), else 1", &["tcp_shutdown_relisten", "0.0.0.0", "47050"]);
    case(rig, wl, "T9 forked child creates+listens", "1 TCP_LISTEN_CB with the CHILD pid (cgroup inherited across fork)", &["tcp_fork_child"]);
    case(rig, wl, "T10 SO_REUSEPORT x2 same port", "2 TCP_LISTEN_CB, same port, one per socket", &["tcp_reuseport2"]);
    case(rig, wl, "T11 listener + client connect + accept", "1 TCP_LISTEN_CB only (accepted child is not a listen transition)", &["tcp_accept_children"]);
    case(rig, wl, "T12 bind without listen", "0 TCP_LISTEN_CB (1 OTHER_POST_BIND)", &["tcp4_bind_only"]);
    case(rig, wl, "T13 listen() with no prior bind (autobind)", "1 TCP_LISTEN_CB with the autobound port (nonzero)", &["tcp4_listen_unbound"]);
}

fn v33(rig: &mut Rig, wl: &Path, ip: &str) {
    println!("\n================ V33 UDP post_bind ================");
    case(rig, wl, "U1 v4 wildcard explicit bind", "1 UDP_POST_BIND 0.0.0.0:47101", &["udp4_wild", "0.0.0.0", "47101"]);
    case(rig, wl, "U2 v4 specific-address bind", "1 UDP_POST_BIND <primary ip>:47102", &["udp4_specific", ip, "47102"]);
    case(rig, wl, "U3 v4 loopback bind", "1 UDP_POST_BIND 127.0.0.1:47103 (address readable)", &["udp4_loop", "127.0.0.1", "47103"]);
    case(rig, wl, "U4 v4 bind(port 0)", "1 UDP_POST_BIND with the kernel-assigned (nonzero) port equal to wl's getsockname", &["udp4_port0"]);
    case(rig, wl, "U5 v4 autobind via sendto (no bind)", "0 events", &["udp4_autobind_sendto"]);
    case(rig, wl, "U6 connected UDP client, no bind", "0 events", &["udp4_connected_client"]);
    case(rig, wl, "U7 IP_BIND_ADDRESS_NO_PORT + bind(0)", "0 events (get_port and post_bind share one branch: af_inet.c:541-551)", &["udp4_bind_no_port"]);
    case(rig, wl, "U8 SO_REUSEPORT x2 same port", "2 UDP_POST_BIND, port 47108", &["udp4_reuseport2", "0.0.0.0", "47108"]);
    case(rig, wl, "U9 second bind to a taken port (EADDRINUSE)", "1 UDP_POST_BIND (first bind) only; the failing bind is not reported", &["udp4_bind_conflict", "0.0.0.0", "47109"]);
    case(rig, wl, "U10 v6 wildcard V6ONLY=1", "1 UDP_POST_BIND AF_INET6 [::]:47110", &["udp6_wild_v6only1", "0.0.0.0", "47110"]);
    case(rig, wl, "U11 v6 wildcard V6ONLY=0", "1 UDP_POST_BIND AF_INET6 [::]:47111, identical shape to U10 (V6ONLY not visible)", &["udp6_wild_v6only0", "0.0.0.0", "47111"]);
    case(rig, wl, "U12 v6 loopback bind", "1 UDP_POST_BIND [::1]:47112", &["udp6_loop", "0.0.0.0", "47112"]);
    case(rig, wl, "U13 v6 bind(port 0)", "1 UDP_POST_BIND AF_INET6 with assigned port", &["udp6_port0"]);
}

fn ck_cg_show(cg: &Path) {
    match Command::new("bpftool").args(["cgroup", "show", cg.to_str().unwrap()]).output() {
        Ok(o) => println!("{}", String::from_utf8_lossy(&o.stdout)),
        Err(e) => println!("bpftool: {e}"),
    }
}

fn intake_regress(rig: &mut Rig, wl: &Path) {
    {
        let mut cfg = Array::<_, u32>::try_from(rig.bpf.map_mut("CFG").unwrap()).unwrap();
        cfg.set(0, u32::from_ne_bytes([127, 0, 0, 1]), 0).unwrap();
        cfg.set(1, 47900, 0).unwrap();
    }
    let mut c = cg_command(&rig.cg, wl, &strs(&["tcp_intake_hold", "127.0.0.1", "47900"]));
    c.stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut ch = c.spawn().expect("spawn hold");
    let mut rd = BufReader::new(ch.stdout.take().unwrap());
    let mut line = String::new();
    while rd.read_line(&mut line).unwrap() > 0 {
        if line.contains("HOLD") {
            break;
        }
        line.clear();
    }
    let states: Vec<u32> = {
        let m = AyaHashMap::<_, u64, Child>::try_from(rig.bpf.map("CHILD_CORRELATION").unwrap()).unwrap();
        m.iter().map(|kv| kv.unwrap().1.state).collect()
    };
    let _ = writeln!(ch.stdin.take().unwrap());
    let _ = ch.wait();
    println!(
        "  [{}] CHILD_CORRELATION while the connection is alive: {} entr(ies), states={:?} (3 = REJECTED: no TCP option on a plain connect)",
        rig.label, states.len(), states
    );
}

fn v35(listen_obj: &Path, base_obj: &Path, wl: &Path) -> R<()> {
    println!("\n================ V35 coexistence / verifier / multi-attach ================");
    let mut base = Rig::load(base_obj, "baseline", None)?;
    let mut lis = Rig::load(listen_obj, "listen", Some(1 << 20))?;
    base.attach_guest_set()?;
    lis.attach_guest_set()?;
    println!("verified_insns baseline: {}", base.insns().join(" "));
    println!("verified_insns listen  : {}", lis.insns().join(" "));
    println!("\nbpftool cgroup show (listen rig):");
    ck_cg_show(&lis.cg);
    println!("--- correlation regression (same scenario on both objects)");
    println!("PREDICT: identical CHILD_CORRELATION shape on both: 1 entry, state=3 (REJECTED) for the accepted child");
    intake_regress(&mut base, wl);
    intake_regress(&mut lis, wl);
    let evs = lis.drain();
    println!("  listen rig events during the regression scenario: {}", evs.len());
    for e in &evs {
        println!("  ev> {}", show(e));
    }
    drop(base);
    drop(lis);

    println!("\n--- multi-attach semantics (sockops at one cgroup)");
    for (tag, first, second, predict) in [
        ("M1 Single then Single", CgroupAttachMode::Single, CgroupAttachMode::Single, "raw BPF_PROG_ATTACH would replace (cgroup.c:630-634); but aya attaches via bpf_link_create which forces BPF_F_ALLOW_MULTI (cgroup.c:1505) => both coexist"),
        ("M2 Multi then Multi", CgroupAttachMode::AllowMultiple, CgroupAttachMode::AllowMultiple, "EINVAL as for M3"),
        ("M3 Single then Multi", CgroupAttachMode::Single, CgroupAttachMode::AllowMultiple, "link_create rejects a caller-supplied BPF_F_ALLOW_MULTI (not in BPF_F_LINK_ATTACH_MASK, cgroup.c:1484) => EINVAL on first attach"),
    ] {
        let mut r = Rig::load(listen_obj, tag, Some(1 << 20))?;
        println!("{tag}\nPREDICT: {predict}");
        let r1 = r.attach_sockops("tcp_correlation", first);
        println!("  first attach result: {:?}", r1.as_ref().map_err(|e| e.to_string()));
        if r1.is_err() {
            continue;
        }
        let res = r.attach_sockops("listen_only", second);
        println!("  second attach result: {:?}", res.as_ref().map_err(|e| e.to_string()));
        ck_cg_show(&r.cg);
        let before = r.counters();
        let (_, out, _) = run_wl(&r.cg, wl, &strs(&["tcp4_wild"]));
        let after = r.counters();
        let evs = r.drain();
        println!("  wl> {}", out.trim());
        println!(
            "  ACTUAL: tcp_correlation-arm events delivered={} (counter seen+{} drops+{}); listen_only invocations+{}",
            evs.len(),
            after[0] - before[0],
            after[1] - before[1],
            after[2] - before[2]
        );
    }
    Ok(())
}

fn parse_stat(out: &str, key: &str) -> Option<(u64, u64, u64)> {
    for l in out.lines() {
        if let Some(i) = l.find(key) {
            let f = |name: &str| -> u64 {
                let s = &l[i..];
                let j = s.find(name).unwrap() + name.len();
                s[j..].split_whitespace().next().unwrap().parse().unwrap()
            };
            return Some((f("mean="), f("p99="), f("max=")));
        }
    }
    None
}

type Stat = (u64, u64, u64);

fn flood(listen_obj: &Path, base_obj: &Path, wl: &Path) -> R<()> {
    println!("\n================ V35 flood + latency ================");
    let n = "20000";
    let mut ctl = Rig::load(base_obj, "ctl", None)?; // cgroup with NO program attached
    let mut base = Rig::load(base_obj, "base", None)?;
    base.attach_guest_set()?;
    let mut big = Rig::load(listen_obj, "big", Some(4 << 20))?;
    big.attach_guest_set()?;
    let mut small = Rig::load(listen_obj, "small", Some(4096))?;
    small.attach_guest_set()?;
    println!("config: ctl = cgroup with no BPF; base = copied guest programs, no listen arm; big = +listen arm, 4 MiB ring; small = +listen arm, 4096 B ring, not drained during the run");
    for (wcase, tag, mult) in [("flood_udp", "UDP bind", 1u64), ("flood_tcp", "TCP bind+listen", 2u64)] {
        println!("\n### {tag} x {n} distinct sockets held open");
        let mut acc: Vec<(String, Vec<Stat>, Vec<Stat>)> = Vec::new();
        for rep in 0..3 {
            for rig in [&mut ctl, &mut base, &mut big, &mut small] {
                let before = rig.counters();
                let (rc, out, err) = run_wl(&rig.cg, wl, &strs(&[wcase, n]));
                if rc != 0 {
                    println!("  [{}] rep{rep} wl failed rc={rc} {err}", rig.label);
                    continue;
                }
                let after = rig.counters();
                let delivered = rig.drain().len() as u64;
                let b = parse_stat(&out, "bind_ns").unwrap();
                let l = parse_stat(&out, "listen_ns");
                let sockets_line = out.lines().find(|l| l.contains("distinct_ports")).unwrap_or("").to_string();
                println!(
                    "  [{:<5}] rep{rep} {}  bind mean/p99/max ns={:?}{}  ring: seen+{} drops+{} delivered={}",
                    rig.label,
                    sockets_line.trim(),
                    b,
                    l.map_or(String::new(), |l| format!("  listen={l:?}")),
                    after[0] - before[0],
                    after[1] - before[1],
                    delivered
                );
                if rig.has_listen {
                    let seen = after[0] - before[0];
                    let drops = after[1] - before[1];
                    println!(
                        "          accounting: seen={seen} == delivered+drops={}  expected_seen={} -> {}",
                        delivered + drops,
                        mult * 20000,
                        if seen == delivered + drops && seen == mult * 20000 { "OK" } else { "MISMATCH" }
                    );
                }
                match acc.iter_mut().find(|a| a.0 == rig.label) {
                    Some(a) => {
                        a.1.push(b);
                        if let Some(l) = l {
                            a.2.push(l)
                        }
                    }
                    None => acc.push((rig.label.clone(), vec![b], l.into_iter().collect())),
                }
            }
        }
        println!("  mean-of-3 summary: config  bind(mean_ns p99_ns max_ns)  [listen(mean_ns p99_ns max_ns)]");
        for (label, bs, ls) in &acc {
            let avg = |v: &Vec<Stat>, i: usize| -> u64 {
                v.iter().map(|t| [t.0, t.1, t.2][i]).sum::<u64>() / v.len().max(1) as u64
            };
            print!("    {label:<6} bind  {:>8} {:>8} {:>10}", avg(bs, 0), avg(bs, 1), avg(bs, 2));
            if !ls.is_empty() {
                print!("   | listen {:>8} {:>8} {:>10}", avg(ls, 0), avg(ls, 1), avg(ls, 2));
            }
            println!();
        }
    }
    Ok(())
}

fn main() -> R<()> {
    let a: Vec<String> = std::env::args().collect();
    let (mode, listen_obj, base_obj, wl) =
        (a[1].as_str(), PathBuf::from(&a[2]), PathBuf::from(&a[3]), PathBuf::from(&a[4]));
    assert_eq!(unsafe { libc::geteuid() }, 0, "must run as root");
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    unsafe { libc::uname(&mut u) };
    let rel = unsafe { std::ffi::CStr::from_ptr(u.release.as_ptr()) }.to_string_lossy().to_string();
    println!("uname -r: {rel}");
    let _ = Command::new("mount").args(["-t", "bpf", "bpf", "/sys/fs/bpf"]).output();
    assert!(
        Path::new("/sys/fs/cgroup/cgroup.controllers").exists(),
        "cgroup2 not mounted at /sys/fs/cgroup"
    );
    let ip = primary_ip();
    println!("primary ip: {ip}");

    if matches!(mode, "v32" | "v33" | "all") {
        let mut rig = Rig::load(&listen_obj, "main", Some(1 << 20))?;
        rig.attach_guest_set()?;
        println!("attached in child cgroup {} (not the host root); bpftool cgroup show:", rig.cg.display());
        ck_cg_show(&rig.cg);
        if matches!(mode, "v32" | "all") {
            v32(&mut rig, &wl, &ip);
        }
        if matches!(mode, "v33" | "all") {
            v33(&mut rig, &wl, &ip);
        }
    }
    if matches!(mode, "v35" | "all") {
        v35(&listen_obj, &base_obj, &wl)?;
        flood(&listen_obj, &base_obj, &wl)?;
    }
    println!("\nDONE");
    Ok(())
}
