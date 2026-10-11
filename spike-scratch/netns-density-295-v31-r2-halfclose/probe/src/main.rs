//! drainprobe: throwaway spike binary (netns-density-295, drain probes T1-T6).
mod apps;
mod bpfh;
mod fwd;
mod sys;

use std::collections::{BTreeMap, HashMap};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use apps::{CliRes, SrvMap, SrvRes, Trial};
use bpfh::{Attach, Bpf, CNT_NAMES};
use fwd::{ArmMode, Backstop, Chan, CtlLoss, Peer, Role, RoleCfg, VPORT};
use sys::*;

pub struct Opts(pub HashMap<String, String>);
impl Opts {
    pub fn parse(args: &[String]) -> Opts {
        let mut m = HashMap::new();
        let mut i = 0;
        while i < args.len() {
            if let Some(k) = args[i].strip_prefix("--") {
                let v = if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                    i += 1;
                    args[i].clone()
                } else {
                    "yes".to_string()
                };
                m.insert(k.to_string(), v);
            }
            i += 1;
        }
        Opts(m)
    }
    pub fn s(&self, k: &str, d: &str) -> String {
        self.0.get(k).cloned().unwrap_or_else(|| d.to_string())
    }
    pub fn u(&self, k: &str, d: u64) -> u64 {
        self.0.get(k).map(|v| v.parse().expect(k)).unwrap_or(d)
    }
    pub fn on(&self, k: &str) -> bool {
        self.0.get(k).map(|v| v == "yes" || v == "true" || v == "1").unwrap_or(false)
    }
}

fn arm_mode(o: &Opts) -> ArmMode {
    match o.s("arm", "owner").as_str() {
        "none" => ArmMode::None,
        "owner" => ArmMode::Owner,
        x => panic!("--arm {x}"),
    }
}

fn backstop_mode(o: &Opts) -> Backstop {
    match o.s("backstop", "full").as_str() {
        "off" => Backstop::Off,
        "rise" => Backstop::Rise,
        "full" => Backstop::Full,
        x => panic!("--backstop {x}"),
    }
}

fn ctl_loss_policy(o: &Opts) -> CtlLoss {
    match o.s("ctl-loss-policy", "unpaired").as_str() {
        "unpaired" => CtlLoss::Unpaired,
        "all" => CtlLoss::All,
        x => panic!("--ctl-loss-policy {x}"),
    }
}

/// Program the BPF-side configuration: kernel struct offsets (from the running kernel's BTF, passed by the
/// driver), the fault-injection knobs, the counter mode and the level-read hook.
fn apply_cfg(b: &Bpf, o: &Opts) {
    use bpfh::*;
    b.cfg_set(CFG_DIAG_SEQ, o.on("diag-seq") as u32);
    b.cfg_set(CFG_SKIP_A, o.u("skip-a", 0) as u32);
    b.cfg_set(CFG_SKIP_B, o.u("skip-b", 0) as u32);
    b.cfg_set(CFG_INCR, o.on("incr") as u32);
    for (slot, key) in [
        (CFG_OFF_WSEQ, "off-wseq"),
        (CFG_OFF_TRANS, "off-trans"),
        (CFG_OFF_TXCNT, "off-txcnt"),
        (CFG_OFF_FPRIV, "off-fpriv"),
        (CFG_OFF_SKT, "off-skt"),
        (CFG_OFF_COOKIE, "off-cookie"),
    ] {
        b.cfg_set(slot, o.u(key, 0) as u32);
    }
    b.cfg_set(CFG_POKE, (o.s("poke", "yes") == "yes") as u32);
}

pub fn setup_cgroup() -> String {
    let dir = format!("/sys/fs/cgroup/v31-drain-{}", std::process::id());
    std::fs::create_dir_all(&dir).expect("mkdir cgroup");
    std::fs::write(format!("{dir}/cgroup.procs"), std::process::id().to_string()).expect("join cgroup");
    dir
}

pub fn cleanup_cgroup(dir: &str) {
    let _ = std::fs::write("/sys/fs/cgroup/cgroup.procs", std::process::id().to_string());
    let _ = std::fs::remove_dir(dir);
}

// ---------------------------------------------------------------- trial generation
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            lo
        } else {
            lo + self.next() % (hi - lo + 1)
        }
    }
}

fn pick_size(spec: &str, r: &mut Rng) -> u64 {
    if spec == "small" {
        return match r.next() % 10 {
            0 => 0,
            1 => 1,
            _ => r.range(2, 70_000),
        };
    }
    if spec != "mix" {
        return spec.parse().expect("size");
    }
    match r.next() % 10 {
        0 => 0,
        1 => r.range(1, 100),
        2..=4 => r.range(100, 65_536),
        5..=7 => r.range(65_536, 524_288),
        _ => r.range(524_288, 2_097_152),
    }
}

fn gen_trial(o: &Opts, seed: u64, i: u64) -> Trial {
    let mut r = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (i + 1).wrapping_mul(0xbf58_476d_1ce4_e5b9) | 1);
    for _ in 0..4 {
        r.next();
    }
    let n = pick_size(&o.s("n", "mix"), &mut r);
    let m = pick_size(&o.s("m", "mix"), &mut r);
    let pause = |k: &str, r: &mut Rng| -> u32 {
        let v = o.s(k, "0");
        if v == "mix" {
            match r.next() % 10 {
                0..=6 => 0,
                7..=8 => 5,
                _ => 50,
            }
        } else {
            v.parse().expect(k)
        }
    };
    let srv_pause_ms = pause("srv-pause", &mut r);
    let cli_pause_ms = pause("cli-pause", &mut r);
    let fin_data = match o.s("fin", "mix").as_str() {
        "data" => true,
        "only" => false,
        _ => r.next() % 2 == 0,
    };
    let rst_pm = o.u("srv-rst-permille", 0);
    let srv_rst_after = if rst_pm > 0 && r.next() % 1000 < rst_pm { Some((n / 2).max(1)) } else { None };
    let srv_first = o.on("srv-first") || (o.s("srv-first", "") == "mix" && r.next() % 4 == 0);
    Trial { id: i + 1, n, m, srv_pause_ms, cli_pause_ms, fin_data, srv_rst_after, srv_first }
}

struct Summary {
    total: u64,
    pass: u64,
    /// fault runs: trials that ended in a reset on at least one end and a violation on none
    reset: u64,
    fail: BTreeMap<String, u64>,
    examples: Vec<serde_json::Value>,
}

fn judge(t: &Trial, c: &CliRes, s: Option<&SrvRes>) -> Result<(), String> {
    let Some(s) = s else {
        return Err(if c.send_err.is_some() || c.recv_err.is_some() { "no_server_result_client_error".into() } else { "no_server_result".into() });
    };
    if t.srv_rst_after.is_some() {
        // reset trial: success = the reset was observed; the harness judges abort behaviour elsewhere
        return if s.rst_done { Err("expected_reset".into()) } else { Err("rst_trial_server_incomplete".into()) };
    }
    if let Some(e) = &s.recv_err {
        return Err(format!("srv_recv_err:{e}"));
    }
    if s.recv != t.n {
        return Err(if s.recv > t.n { "srv_excess_bytes".into() } else if s.eof { "srv_truncated".into() } else { "srv_short_no_eof".into() });
    }
    if !s.recv_ok {
        return Err("srv_payload_corrupt".into());
    }
    if !s.eof {
        return Err("srv_no_eof".into());
    }
    if let Some(e) = &s.send_err {
        return Err(format!("srv_send_err:{e}"));
    }
    if let Some(e) = &c.send_err {
        return Err(format!("cli_send_err:{e}"));
    }
    if let Some(e) = &c.recv_err {
        return Err(format!("cli_recv_err:{e}"));
    }
    if c.recv != t.m {
        return Err(if c.recv > t.m { "cli_excess_bytes".into() } else if c.eof { "cli_truncated".into() } else { "cli_short_no_eof".into() });
    }
    if !c.recv_ok {
        return Err("cli_payload_corrupt".into());
    }
    if !c.eof {
        return Err("cli_no_eof".into());
    }
    Ok(())
}

#[derive(PartialEq, Eq, Debug)]
pub enum Cat {
    Pass,
    Reset,
    Violation(String),
}

fn is_reset(e: &str) -> bool {
    // a refused connect is the kernel's RST to the SYN: the far end gets a reset
    e == "ECONNRESET" || e == "EPIPE" || e.starts_with("connect:ECONNRESET") || e.starts_with("connect:EPIPE") || e.starts_with("connect:Connection refused")
}
fn is_hang(e: &str) -> bool {
    e == "TIMEOUT" || e == "ETIMEDOUT" || e.contains("TIMEOUT")
}

/// Fault runs. A trial may end complete, or in a reset. It may not end with a clean end-of-stream that carries
/// fewer bytes than the application sent (a FIN after an abort), nor hang, nor deliver excess or corrupt bytes.
pub fn judge_fault(t: &Trial, c: &CliRes, s: Option<&SrvRes>) -> Cat {
    if t.srv_rst_after.is_some() {
        // the destination reset on purpose: the client must see a reset, never a clean end-of-stream, never a hang
        let ce: Vec<&String> = [&c.send_err, &c.recv_err].into_iter().flatten().collect();
        if ce.iter().any(|e| is_hang(e)) {
            return Cat::Violation("hang".into());
        }
        if ce.iter().any(|e| is_reset(e)) {
            return Cat::Reset;
        }
        if c.eof && c.recv_err.is_none() {
            return Cat::Violation("cli_clean_eof_after_dest_rst".into());
        }
        return Cat::Violation("cli_no_reset_no_eof".into());
    }
    let mut errs: Vec<&str> = vec![];
    for e in [&c.send_err, &c.recv_err] {
        if let Some(e) = e {
            errs.push(e);
        }
    }
    if let Some(s) = s {
        for e in [&s.recv_err, &s.send_err] {
            if let Some(e) = e {
                errs.push(e);
            }
        }
    }
    if errs.iter().any(|e| is_hang(e)) {
        return Cat::Violation("hang".into());
    }
    let mut any_reset = errs.iter().any(|e| is_reset(e));
    if errs.iter().any(|e| !is_reset(e)) {
        let other = errs.iter().find(|e| !is_reset(e)).unwrap().to_string();
        return Cat::Violation(format!("other_error:{other}"));
    }
    match s {
        None => {
            // the destination never recorded a result: fine only if the client was reset
            if !any_reset {
                return Cat::Violation("no_server_result_no_reset".into());
            }
        }
        Some(s) => {
            if s.recv > t.n {
                return Cat::Violation("srv_excess_bytes".into());
            }
            if s.recv > 0 && !s.recv_ok {
                return Cat::Violation("srv_payload_corrupt".into());
            }
            // a read that follows an error already reported by a send on the same socket returns 0: that is not a clean FIN
            let s_reset = s.recv_err.is_some() || s.send_err.is_some();
            if !s_reset && s.eof && s.recv != t.n {
                return Cat::Violation("srv_clean_short_eof".into());
            }
            if !s_reset && !s.eof && s.recv != t.n {
                return Cat::Violation("srv_short_no_eof_no_error".into());
            }
        }
    }
    if c.recv > t.m {
        return Cat::Violation("cli_excess_bytes".into());
    }
    if c.recv > 0 && !c.recv_ok {
        return Cat::Violation("cli_payload_corrupt".into());
    }
    let c_reset = c.recv_err.is_some() || c.send_err.is_some();
    if !c_reset && c.eof && c.recv != t.m {
        return Cat::Violation("cli_clean_short_eof".into());
    }
    if !c_reset && !c.eof && c.recv != t.m {
        return Cat::Violation("cli_short_no_eof_no_error".into());
    }
    let complete = c.send_err.is_none() && c.recv_err.is_none() && c.eof && c.recv == t.m && s.map(|s| s.eof && s.recv == t.n && s.recv_err.is_none() && s.send_err.is_none()).unwrap_or(false);
    if complete {
        Cat::Pass
    } else {
        any_reset = true;
        let _ = any_reset;
        Cat::Reset
    }
}

fn run_trials(o: &Opts, intake: SocketAddrV4, srv: &SrvMap, seed: u64, count: u64, conc: u64) -> Summary {
    let next = Arc::new(AtomicU64::new(0));
    let results: Arc<Mutex<Vec<(Trial, CliRes)>>> = Arc::new(Mutex::new(vec![]));
    let mut hs = vec![];
    let oo = Arc::new(Opts(o.0.clone()));
    for _ in 0..conc {
        let (next, results, oo) = (next.clone(), results.clone(), oo.clone());
        hs.push(std::thread::Builder::new().stack_size(512 * 1024).spawn(move || loop {
            let i = next.fetch_add(1, Ordering::SeqCst);
            if i >= count {
                return;
            }
            let t = gen_trial(&oo, seed, i);
            let c = apps::run_client(intake, &t);
            results.lock().unwrap().push((t, c));
        }).unwrap());
    }
    for h in hs {
        h.join().unwrap();
    }
    // let the server side finish publishing
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        let n = srv.lock().unwrap().len() as u64;
        if n >= count || Instant::now() > end {
            break;
        }
        sleep_ms(50);
    }
    let rs = results.lock().unwrap();
    let sm = srv.lock().unwrap();
    let mut sum = Summary { total: rs.len() as u64, pass: 0, reset: 0, fail: BTreeMap::new(), examples: vec![] };
    let fault = o.on("fault-judge");
    for (t, c) in rs.iter() {
        if fault {
            match judge_fault(t, c, sm.get(&t.id)) {
                Cat::Pass => sum.pass += 1,
                Cat::Reset => sum.reset += 1,
                Cat::Violation(why) => {
                    *sum.fail.entry(why.clone()).or_insert(0) += 1;
                    if sum.examples.iter().filter(|e| e["why"] == why.as_str()).count() < 4 && sum.examples.len() < 48 {
                        sum.examples.push(json!({"trial":t.id,"why":why,"n":t.n,"m":t.m,"srv_pause":t.srv_pause_ms,"cli_pause":t.cli_pause_ms,"fin_data":t.fin_data,
                            "cli":format!("{c:?}"),"srv":format!("{:?}", sm.get(&t.id))}));
                    }
                }
            }
            continue;
        }
        match judge(t, c, sm.get(&t.id)) {
            Ok(()) => sum.pass += 1,
            Err(why) => {
                *sum.fail.entry(why.clone()).or_insert(0) += 1;
                if sum.examples.iter().filter(|e| e["why"] == why.as_str()).count() < 4 && sum.examples.len() < 48 {
                    sum.examples.push(json!({"trial":t.id,"why":why,"n":t.n,"m":t.m,"srv_pause":t.srv_pause_ms,"cli_pause":t.cli_pause_ms,"fin_data":t.fin_data,
                        "cli":format!("{c:?}"),"srv":format!("{:?}", sm.get(&t.id))}));
                }
            }
        }
    }
    sum
}

fn role_cfg(o: &Opts, name: &'static str, peer_cid: u32, intake_port: u16, dest: SocketAddrV4, pin: Option<usize>) -> RoleCfg {
    RoleCfg {
        name,
        arm: arm_mode(o),
        nflows: if name == "host" { 1024 } else { o.u("flows", 512) as u32 },
        intake_port,
        dest,
        peer_cid,
        race_spin: o.on("race-spin"),
        v_rcvlowat: o.u("v-rcvlowat", 0) as i32,
        v_late: o.u("v-late", 0) as u32,
        a_rearm: o.s("a-rearm", "yes") == "yes",
        p2_kick: o.s("p2-kick", "yes") == "yes",
        intake_rcvbuf: o.u("intake-rcvbuf", 0) as i32,
        drop_notice: o.s("drop-notice", "yes") == "yes",
        backstop: backstop_mode(o),
        backstop_ms: o.u("backstop-ms", 20),
        ctl_loss: ctl_loss_policy(o),
        classify_abort: o.s("classify-abort", "yes") == "yes",
        tcp_end_check: o.s("tcp-end-check", "yes") == "yes",
        chaos_v_reset_permille: if name == "guest" { o.u("chaos-v-reset-permille", 0) as u32 } else { 0 },
        events: o.0.get("events").map(|p| format!("{p}.{name}.jsonl")),
        sample_every: o.u("events-sample", 200),
        pin,
        seed: o.u("seed", 1) ^ if name == "guest" { 11 } else { 22 },
    }
}

fn stats_json(s: &fwd::Stats) -> serde_json::Value {
    json!(s.0)
}

pub fn bpf_counters_pub(b: &Bpf) -> serde_json::Value {
    bpf_counters(b)
}

fn bpf_counters(b: &Bpf) -> serde_json::Value {
    let c = b.counters();
    let mut m = serde_json::Map::new();
    for (i, n) in CNT_NAMES.iter().enumerate() {
        m.insert(n.to_string(), json!(c[i]));
    }
    m.insert("send_err_hist".to_string(), json!(b.send_err_hist()));
    m.insert("fexit_recursion_misses".to_string(), json!(b.fexit_misses()));
    m.insert("fexit_b_recursion_misses".to_string(), json!(b.fexit_misses_b()));
    m.insert("fexit_run_cnt".to_string(), json!(b.fexit_run_cnt()));
    serde_json::Value::Object(m)
}

// ---------------------------------------------------------------- loop: everything on one host (vsock loopback)
fn cmd_loop(o: &Opts) {
    let obj = o.s("obj", "/tmp/v31-work/out/drain-bpf.o");
    let cg = setup_cgroup();
    let bpf = Bpf::load(&obj, Attach { forwarder: true, tracepoint: true }, Some(&cg)).expect("bpf load");
    apply_cfg(&bpf, o);
    let intake_port = o.u("intake-port", 17001) as u16;
    let dest_port = o.u("dest-port", 17002) as u16;
    let dest = SocketAddrV4::new(Ipv4Addr::LOCALHOST, dest_port);
    let intake = SocketAddrV4::new(Ipv4Addr::LOCALHOST, intake_port);
    let pin_g = if o.on("pin") { Some(o.u("pin-guest", 2) as usize) } else { None };
    let pin_h = if o.on("pin") { Some(o.u("pin-host", 3) as usize) } else { None };
    let gchan = Chan::new();
    let hchan = Chan::new();
    let next_id = Arc::new(AtomicU32::new(1));
    let stop = Arc::new(AtomicBool::new(false));
    let srv: SrvMap = Arc::new(Mutex::new(HashMap::new()));
    let srv_stop = Arc::new(AtomicBool::new(false));
    let _srv_h = apps::spawn_server(dest, srv.clone(), srv_stop.clone());
    let live_g = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let live_h = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (bg, bh) = (bpf.clone(), bpf.clone());
    let gcfg = role_cfg(o, "guest", 1, intake_port, dest, pin_g);
    let hcfg = role_cfg(o, "host", 1, intake_port, dest, pin_h);
    let (gc, hc, gc2, hc2) = (gchan.clone(), hchan.clone(), gchan.clone(), hchan.clone());
    let (nid_g, nid_h) = (next_id.clone(), next_id.clone());
    let (stop_g, stop_h) = (stop.clone(), stop.clone());
    let (lg, lh) = (live_g.clone(), live_h.clone());
    // host role first so the guest can connect
    let host_t = std::thread::spawn(move || {
        let mut r = Role::new(hcfg, bh, hc, Peer::Local(gc2), None, nid_h);
        r.live = lh;
        r.run(stop_h);
        r
    });
    sleep_ms(200);
    let guest_t = std::thread::spawn(move || {
        let mut r = Role::new(gcfg, bg, gc, Peer::Local(hc2), None, nid_g);
        r.live = lg;
        r.run(stop_g);
        r
    });
    sleep_ms(500 + o.u("flows", 512) / 2); // park pools
    let seed = o.u("seed", 1);
    let count = o.u("trials", 1000);
    let conc = o.u("conc", 64);
    let t0 = Instant::now();
    // control-session loss injected at a seeded random point of the run
    let loss_at_ms = if o.u("ctl-loss-ms-max", 0) > 0 {
        let lo = o.u("ctl-loss-ms-min", 0);
        let hi = o.u("ctl-loss-ms-max", 0);
        let mut r = Rng(seed ^ 0xc71_1055);
        for _ in 0..3 {
            r.next();
        }
        let at = r.range(lo, hi);
        let (g, h) = (gchan.clone(), hchan.clone());
        std::thread::spawn(move || {
            sleep_ms(at);
            g.lose();
            h.lose();
        });
        Some(at)
    } else {
        None
    };
    let sum = run_trials(o, intake, &srv, seed, count, conc);
    let wall = t0.elapsed().as_secs_f64();
    // roles drain
    let end = Instant::now() + Duration::from_secs(o.u("settle-s", 20));
    while Instant::now() < end && (live_g.load(Ordering::Relaxed) + live_h.load(Ordering::Relaxed)) > 0 {
        sleep_ms(100);
    }
    sleep_ms(300);
    stop.store(true, Ordering::Relaxed);
    let host = host_t.join().unwrap();
    let guest = guest_t.join().unwrap();
    srv_stop.store(true, Ordering::Relaxed);
    let out = json!({
        "cmd":"loop","uname":uname(),"args":o.0,"wall_s":wall,"ctl_loss_at_ms":loss_at_ms,
        "trials":{"total":sum.total,"pass":sum.pass,"reset":sum.reset,"fail":sum.fail,"examples":sum.examples},
        "guest":stats_json(&guest.stats),"host":stats_json(&host.stats),
        "guest_live_flows_at_end":guest.live_flows(),"host_live_flows_at_end":host.live_flows(),"guest_cells":guest.cell_accounting(),"host_cells":host.cell_accounting(),
        "guest_stuck":guest.snapshot(),"host_stuck":host.snapshot(),
        "bpf_counters":bpf_counters(&bpf),
    });
    println!("{out}");
    cleanup_cgroup(&cg);
}

pub fn uname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string()
}

// ---------------------------------------------------------------- split processes (loopback or microVM)
// owner-host | dest | owner-guest | clients are four separate processes, so any one of them can be killed.
// The control session between the owners is a vsock stream (host listens VPORT+1).
static LOSE_CTL: AtomicBool = AtomicBool::new(false);
extern "C" fn on_usr1(_: i32) {
    LOSE_CTL.store(true, Ordering::SeqCst);
}

fn wait_stop_file(o: &Opts, ctl_raw: Option<RawFd>) {
    let sf = o.s("stop-file", "/tmp/v31-work/stop");
    let mut lost = false;
    loop {
        if std::path::Path::new(&sf).exists() {
            break;
        }
        if let Some(fd) = ctl_raw {
            if LOSE_CTL.load(Ordering::SeqCst) && !lost {
                lost = true;
                // inject the loss of the control session from this end: both ends see the end of the stream
                unsafe { libc::shutdown(fd, libc::SHUT_RDWR) };
                eprintln!("CTL_LOSS_INJECTED");
            }
        }
        sleep_ms(50);
    }
}

fn cmd_owner_host(o: &Opts) {
    let obj = o.s("obj", "/tmp/v31-work/out/drain-bpf.o");
    let cg = setup_cgroup();
    let bpf = Bpf::load(&obj, Attach { forwarder: true, tracepoint: true }, Some(&cg)).expect("bpf load");
    apply_cfg(&bpf, o);
    unsafe { libc::signal(libc::SIGUSR1, on_usr1 as usize) };
    let dest_port = o.u("dest-port", 17002) as u16;
    let dest = SocketAddrV4::new(Ipv4Addr::LOCALHOST, dest_port);
    let ctl_l = vsock_listen(libc::VMADDR_CID_ANY, VPORT + 1, libc::SOCK_STREAM).expect("ctl listen");
    println!("OWNER_HOST_LISTENING");
    let ctl = accept(ctl_l.as_raw_fd()).expect("ctl accept");
    let ctl_raw = ctl.as_raw_fd();
    let chan = Chan::new();
    let live = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let cfg = role_cfg(o, "host", 0, 0, dest, if o.on("pin") { Some(o.u("pin-host", 3) as usize) } else { None });
    let (b2, l2, s2) = (bpf.clone(), live.clone(), stop.clone());
    let h = std::thread::spawn(move || {
        let mut r = Role::new(cfg, b2, chan, Peer::Vsock(ctl_raw), Some(ctl_raw), Arc::new(AtomicU32::new(1)));
        r.live = l2;
        r.run(s2);
        r
    });
    println!("OWNER_HOST_READY");
    wait_stop_file(o, Some(ctl_raw));
    let end = Instant::now() + Duration::from_secs(o.u("settle-s", 25));
    while Instant::now() < end && live.load(Ordering::Relaxed) > 0 {
        sleep_ms(100);
    }
    sleep_ms(300);
    stop.store(true, Ordering::Relaxed);
    let host = h.join().unwrap();
    let out = json!({"cmd":"owner-host","uname":uname(),"args":o.0,"host":stats_json(&host.stats),"host_live_flows_at_end":host.live_flows(),
        "host_stuck":host.snapshot(),"host_cells":host.cell_accounting(),"bpf_counters":bpf_counters(&bpf)});
    std::fs::write(o.s("out", "/tmp/v31-work/owner-host-out.json"), out.to_string()).unwrap();
    println!("OWNER_HOST_DONE");
    drop(ctl);
    cleanup_cgroup(&cg);
}

fn cmd_dest(o: &Opts) {
    let dest_port = o.u("dest-port", 17002) as u16;
    let dest = SocketAddrV4::new(Ipv4Addr::LOCALHOST, dest_port);
    let srv: SrvMap = Arc::new(Mutex::new(HashMap::new()));
    let srv_stop = Arc::new(AtomicBool::new(false));
    let _h = apps::spawn_server(dest, srv.clone(), srv_stop.clone());
    println!("DEST_READY");
    wait_stop_file(o, None);
    srv_stop.store(true, Ordering::Relaxed);
    sleep_ms(300);
    let sm = srv.lock().unwrap();
    let mut rows = vec![];
    for (id, s) in sm.iter() {
        rows.push(json!([id, s.recv, s.recv_ok, s.eof, s.recv_err, s.sent, s.send_err, s.rst_done]));
    }
    std::fs::write(o.s("out", "/tmp/v31-work/dest-out.json"), json!({"cmd":"dest","srv_results":rows}).to_string()).unwrap();
    println!("DEST_DONE");
}

fn cmd_owner_guest(o: &Opts) {
    let obj = o.s("obj", "/vm-drain-bpf.o");
    let cg = setup_cgroup();
    let bpf = Bpf::load(&obj, Attach { forwarder: true, tracepoint: true }, Some(&cg)).expect("bpf load");
    apply_cfg(&bpf, o);
    unsafe { libc::signal(libc::SIGUSR1, on_usr1 as usize) };
    let peer_cid = o.u("peer-cid", 2) as u32;
    let intake_port = o.u("intake-port", 17001) as u16;
    let ctl = vsock_connect(peer_cid, VPORT + 1, libc::SOCK_STREAM).expect("ctl connect");
    let ctl_raw = ctl.as_raw_fd();
    let chan = Chan::new();
    let live = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let cfg = role_cfg(o, "guest", peer_cid, intake_port, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0), if o.on("pin") { Some(o.u("pin-guest", 2) as usize) } else { None });
    let (b2, l2, s2) = (bpf.clone(), live.clone(), stop.clone());
    let h = std::thread::spawn(move || {
        let mut r = Role::new(cfg, b2, chan, Peer::Vsock(ctl_raw), Some(ctl_raw), Arc::new(AtomicU32::new(1)));
        r.live = l2;
        r.run(s2);
        r
    });
    sleep_ms(500 + o.u("flows", 512) / 2);
    println!("OWNER_GUEST_READY");
    wait_stop_file(o, Some(ctl_raw));
    let end = Instant::now() + Duration::from_secs(o.u("settle-s", 25));
    while Instant::now() < end && live.load(Ordering::Relaxed) > 0 {
        sleep_ms(100);
    }
    sleep_ms(300);
    stop.store(true, Ordering::Relaxed);
    let guest = h.join().unwrap();
    let out = json!({"cmd":"owner-guest","uname":uname(),"args":o.0,"guest":stats_json(&guest.stats),
        "guest_live_flows_at_end":guest.live_flows(),"guest_stuck":guest.snapshot(),"guest_cells":guest.cell_accounting(),"bpf_counters":bpf_counters(&bpf)});
    std::fs::write(o.s("out", "/tmp/v31-owner-guest-out.json"), out.to_string()).unwrap();
    println!("OWNER_GUEST_DONE");
    drop(ctl);
    cleanup_cgroup(&cg);
}

fn cmd_clients(o: &Opts) {
    let intake_port = o.u("intake-port", 17001) as u16;
    let intake = SocketAddrV4::new(Ipv4Addr::LOCALHOST, intake_port);
    let seed = o.u("seed", 1);
    let count = o.u("trials", 1000);
    let conc = o.u("conc", 32);
    let t0 = Instant::now();
    let (rows, crash_rows) = run_client_only(o, intake, seed, count, conc);
    let out = json!({"cmd":"clients","uname":uname(),"args":o.0,"wall_s":t0.elapsed().as_secs_f64(),"cli_results":rows,"crash_results":crash_rows});
    std::fs::write(o.s("out", "/tmp/v31-clients-out.json"), out.to_string()).unwrap();
    println!("CLIENTS_DONE");
}

/// Child process of an application-crash trial (killed by the parent).
fn cmd_crash_child(o: &Opts) {
    let intake = SocketAddrV4::new(Ipv4Addr::LOCALHOST, o.u("intake-port", 17001) as u16);
    let t = Trial {
        id: o.u("trial", 1),
        n: o.u("n", 1 << 21),
        m: o.u("m", 0),
        srv_pause_ms: 0,
        cli_pause_ms: 0,
        fin_data: false,
        srv_rst_after: None,
        srv_first: o.on("srv-first"),
    };
    apps::run_crash_child(intake, &t, &o.s("progress", "/tmp/v31-crash.bin"), o.u("chunk", 16384) as usize, o.u("gap-us", 100));
}

/// Guest-side client runs. Rows: [trial, n, m, sent, send_err, recv, recv_ok, eof, recv_err, ms, srv_pause, fin_data, rst_after]
/// Crash rows (a fraction of trials, `--crash-permille`): [trial, n, m, kind, lo, hi, done, kill_delay_us]
fn run_client_only(o: &Opts, intake: SocketAddrV4, seed: u64, count: u64, conc: u64) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let next = Arc::new(AtomicU64::new(0));
    let rows: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(vec![]));
    let crows: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(vec![]));
    let oo = Arc::new(Opts(o.0.clone()));
    let mut hs = vec![];
    for _ in 0..conc {
        let (next, rows, crows, oo) = (next.clone(), rows.clone(), crows.clone(), oo.clone());
        hs.push(std::thread::Builder::new().stack_size(512 * 1024).spawn(move || loop {
            let i = next.fetch_add(1, Ordering::SeqCst);
            if i >= count {
                return;
            }
            let mut t = gen_trial(&oo, seed, i);
            let mut r = Rng(seed.wrapping_mul(0x2545_f491_4f6c_dd1d) ^ (i + 7).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
            for _ in 0..3 {
                r.next();
            }
            let pm = oo.u("crash-permille", 0);
            if pm > 0 && r.next() % 1000 < pm {
                let kind = oo.s("crash-kind", "fin");
                t.n = oo.u("crash-n", 1 << 21);
                if kind == "rst" {
                    t.srv_first = true;
                    t.m = 65536;
                } else {
                    t.srv_first = false;
                    t.m = 0;
                }
                t.srv_pause_ms = 0;
                let path = format!("/tmp/v31-crash-{}-{}.bin", std::process::id(), t.id);
                let _ = std::fs::remove_file(&path);
                let exe = std::env::current_exe().unwrap();
                let mut c = std::process::Command::new(exe)
                    .args(["crash-child", "--intake-port", &intake.port().to_string(), "--trial", &t.id.to_string(), "--n", &t.n.to_string(), "--m", &t.m.to_string(),
                        "--progress", &path, "--gap-us", &oo.u("crash-gap-us", 100).to_string()])
                    .args(if t.srv_first { vec!["--srv-first"] } else { vec![] })
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .expect("spawn crash child");
                // the kill clock starts when the child reports it is connected and (RST variant) holds an unread banner
                let ready_end = Instant::now() + Duration::from_secs(8);
                while apps::read_progress(&path).3 == 0 && Instant::now() < ready_end {
                    std::thread::sleep(Duration::from_micros(300));
                }
                let delay_us = r.range(0, oo.u("crash-ms-max", 25) * 1000);
                sleep_ms(delay_us / 1000);
                std::thread::sleep(Duration::from_micros(delay_us % 1000));
                unsafe { libc::kill(c.id() as i32, libc::SIGKILL) };
                let _ = c.wait();
                let (lo, hi, done, _ready) = apps::read_progress(&path);
                let _ = std::fs::remove_file(&path);
                crows.lock().unwrap().push(json!([t.id, t.n, t.m, kind, lo, hi, done, delay_us]));
                continue;
            }
            let c = apps::run_client(intake, &t);
            rows.lock().unwrap().push(json!([t.id, t.n, t.m, c.sent, c.send_err, c.recv, c.recv_ok, c.eof, c.recv_err, c.ms, t.srv_pause_ms, t.fin_data, t.srv_rst_after, c.lport]));
        }).unwrap());
    }
    for h in hs {
        h.join().unwrap();
    }
    let r = rows.lock().unwrap().clone();
    let c = crows.lock().unwrap().clone();
    (r, c)
}

// ---------------------------------------------------------------- guest agent (as the capture spike's test agent)
fn cmd_agent() {
    let l = vsock_listen(libc::VMADDR_CID_ANY, 4000, libc::SOCK_STREAM).expect("agent listen");
    loop {
        let Ok(c) = accept(l.as_raw_fd()) else { continue };
        std::thread::spawn(move || {
            let raw = c.as_raw_fd();
            let mut len = [0u8; 4];
            if recv_exact(raw, &mut len).is_err() {
                return;
            }
            let n = u32::from_be_bytes(len) as usize;
            let mut cmd = vec![0u8; n];
            if recv_exact(raw, &mut cmd).is_err() {
                return;
            }
            let cmd = String::from_utf8_lossy(&cmd).to_string();
            let out = std::process::Command::new("/bin/sh").arg("-c").arg(&cmd).stdin(std::process::Stdio::null()).output();
            let (rc, mut text) = match out {
                Ok(o) => (o.status.code().unwrap_or(-1), [o.stdout, o.stderr].concat()),
                Err(e) => (-1, e.to_string().into_bytes()),
            };
            text.extend_from_slice(format!("\nAGENT_RC={rc}\n").as_bytes());
            let _ = send_all_fd(raw, &text);
        });
    }
}

fn recv_exact(fd: RawFd, buf: &mut [u8]) -> Result<(), ()> {
    let mut off = 0;
    while off < buf.len() {
        let n = recv_blocking(fd, &mut buf[off..]);
        if n <= 0 {
            return Err(());
        }
        off += n as usize;
    }
    Ok(())
}

fn send_all_fd(fd: RawFd, buf: &[u8]) -> Result<(), ()> {
    let mut off = 0;
    while off < buf.len() {
        let n = send_blocking(fd, &buf[off..]);
        if n <= 0 {
            return Err(());
        }
        off += n as usize;
    }
    Ok(())
}

// ---------------------------------------------------------------- selftest
fn cmd_selftest(o: &Opts) {
    let mut res = serde_json::Map::new();
    res.insert("uname".into(), json!(uname()));
    // 1. tcp_info offset + FIN accounting: a client sends 1000 bytes + FIN; the server reads them
    let l = tcp_listen(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = local4(l.as_raw_fd()).unwrap().port();
    let c = socket(libc::AF_INET, libc::SOCK_STREAM).unwrap();
    connect4(c.as_raw_fd(), SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).unwrap();
    let s = accept(l.as_raw_fd()).unwrap();
    send_blocking(c.as_raw_fd(), &[7u8; 1000]);
    shutdown_wr(c.as_raw_fd()).unwrap();
    sleep_ms(50);
    let mut b = [0u8; 4096];
    let n1 = recv_blocking(s.as_raw_fd(), &mut b);
    let n2 = recv_blocking(s.as_raw_fd(), &mut b);
    res.insert("tcp_info".into(), json!({"read1":n1,"read2_eof":n2,"bytes_received":tcp_bytes_received(s.as_raw_fd()),"expect":1001}));
    // FIN-only
    let c = socket(libc::AF_INET, libc::SOCK_STREAM).unwrap();
    connect4(c.as_raw_fd(), SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).unwrap();
    let s = accept(l.as_raw_fd()).unwrap();
    shutdown_wr(c.as_raw_fd()).unwrap();
    sleep_ms(50);
    res.insert("tcp_info_fin_only".into(), json!({"bytes_received":tcp_bytes_received(s.as_raw_fd()),"expect":1}));
    // 2. BPF load + cells mmap coherence
    let cg = setup_cgroup();
    let obj = o.s("obj", "/tmp/v31-work/out/drain-bpf.o");
    match Bpf::load(&obj, Attach { forwarder: true, tracepoint: o.on("tp") }, Some(&cg)) {
        Ok(bpf) => {
            bpf.a_sent(4095).store(5, Ordering::SeqCst);
            res.insert("cells_mmap".into(), json!({"ok":true,"sent":bpf.a_sent(4095).load(Ordering::SeqCst)}));
            res.insert("bpf_load".into(), json!("ok"));
        }
        Err(e) => {
            res.insert("bpf_load".into(), json!(e));
        }
    }
    cleanup_cgroup(&cg);
    println!("{}", serde_json::Value::Object(res));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().cloned().unwrap_or_default();
    let o = Opts::parse(&args[1.min(args.len())..]);
    apps::IO_TIMEOUT_S.store(o.u("io-timeout", 60), Ordering::Relaxed);
    apps::BANNER.store(o.u("srv-banner", 0), Ordering::Relaxed);
    match cmd.as_str() {
        "selftest" => cmd_selftest(&o),
        "loop" => cmd_loop(&o),
        "owner-host" => cmd_owner_host(&o),
        "dest" => cmd_dest(&o),
        "owner-guest" => cmd_owner_guest(&o),
        "clients" => cmd_clients(&o),
        "crash-child" => cmd_crash_child(&o),
        "agent" => cmd_agent(),
        _ => {
            eprintln!("usage: drainprobe selftest|loop|owner-host|dest|owner-guest|clients|agent [--k v]");
            std::process::exit(2);
        }
    }
}
