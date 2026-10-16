//! V-3 on the HOST-role stack (vhost-vsock transport) with evmod, inside the private QEMU/KVM host-role VM.
//! The synthetic guest peer (peer.rs) is credit-aware here: it sends only while the host's advertised window allows
//! (virtio-vsock credit: host_buf_alloc - (sent - host_fwd_cnt)) and asks for credit with CREDIT_REQUEST when blocked.
//!   v2t  synthetic guest sends a splitmix stream -> host vsock source -> evmod -> TCP target app (stalls / slow / fast, verifies)
//!   t2v  TCP source app writes a splitmix stream -> evmod -> host vsock target -> synthetic guest (stalls, then consumes + verifies)
//! usage: probe v3 <v2t|t2v> <schedule> <total_mib> <trials> <label> [max_s]
use super::{accept, buffers, event, Backend};
use crate::peer::Peer;
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    net::{TcpListener, TcpStream},
    os::fd::AsRawFd,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

fn splitmix(k: u64) -> u64 {
    let mut z = k.wrapping_add(0x9E37_79B9_7F4A_7C15).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
const HK: u64 = 0x9E37_79B9_7F4A_7C15;
struct Verifier {
    k: u64,
    carry: Vec<u8>,
    hash: u64,
    ehash: u64,
    first_bad: Option<u64>,
}
impl Verifier {
    fn new() -> Self {
        Verifier { k: 0, carry: vec![], hash: 0, ehash: 0, first_bad: None }
    }
    fn feed(&mut self, d: &[u8]) {
        self.carry.extend_from_slice(d);
        let full = self.carry.len() / 8 * 8;
        for c in self.carry[..full].chunks_exact(8) {
            let w = u64::from_le_bytes(c.try_into().unwrap());
            let e = splitmix(self.k);
            if w != e && self.first_bad.is_none() {
                self.first_bad = Some(self.k * 8);
            }
            self.hash = (self.hash.rotate_left(5) ^ w).wrapping_mul(HK);
            self.ehash = (self.ehash.rotate_left(5) ^ e).wrapping_mul(HK);
            self.k += 1;
        }
        self.carry.drain(..full);
    }
    fn ok(&self) -> bool {
        self.first_bad.is_none() && self.hash == self.ehash && self.carry.is_empty()
    }
}
fn gen_fill(buf: &mut [u8], first_word: u64) {
    for (i, c) in buf.chunks_exact_mut(8).enumerate() {
        c.copy_from_slice(&splitmix(first_word + i as u64).to_le_bytes());
    }
}
fn mono_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

#[derive(Clone, Debug)]
enum Phase {
    Stall(u64),
    Rate(u64, Option<u64>),
    Fast(Option<u64>),
}
fn parse_phases(s: &str) -> Vec<Phase> {
    s.split(',')
        .map(|p| {
            let (head, dur) = match p.split_once(':') {
                Some((h, d)) => (h, d.parse::<u64>().ok()),
                None => (p, None),
            };
            match head.chars().next() {
                Some('s') => Phase::Stall(head[1..].parse().unwrap()),
                Some('r') => Phase::Rate(head[1..].parse().unwrap(), dur),
                Some('f') => Phase::Fast(dur),
                _ => panic!("bad phase {p}"),
            }
        })
        .collect()
}
fn ioctl_i32(fd: i32, req: u64) -> i64 {
    let mut v: i32 = 0;
    if unsafe { libc::ioctl(fd, req as _, &mut v) } < 0 { -1 } else { v as i64 }
}
fn meminfo_kb(keys: &[&str]) -> Vec<u64> {
    let t = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    keys.iter().map(|k| t.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse().ok()).unwrap_or(0)).collect()
}
fn ev_stat(key: &str) -> i64 {
    fs::read_to_string("/proc/evmod")
        .unwrap_or_default()
        .split_whitespace()
        .find_map(|kv| kv.strip_prefix(&format!("{key}=")).and_then(|v| v.parse().ok()))
        .unwrap_or(0)
}

#[derive(Default)]
struct RStats {
    read: AtomicU64,
    done: AtomicBool,
    stall_ends: Mutex<Vec<(u64, u64)>>,
    res: Mutex<Option<(bool, i64, u64)>>,
}
/// the target app: follows the schedule and verifies every byte
fn reader(mut s: TcpStream, total: u64, phases: Vec<Phase>, st: Arc<RStats>) {
    s.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    let mut v = Verifier::new();
    let mut buf = vec![0u8; 65536];
    let (mut got, mut idx) = (0u64, 0usize);
    let (mut ph_start, mut ph_bytes) = (Instant::now(), 0u64);
    let hard = Instant::now() + Duration::from_secs(300);
    while got < total && Instant::now() < hard {
        let ph = phases.get(idx).cloned().unwrap_or(Phase::Fast(None));
        let dur = match &ph {
            Phase::Stall(ms) => Some(*ms),
            Phase::Rate(_, d) | Phase::Fast(d) => *d,
        };
        if let Some(ms) = dur {
            if ph_start.elapsed() >= Duration::from_millis(ms) {
                if let Phase::Stall(_) = ph {
                    st.stall_ends.lock().unwrap().push((mono_ns(), got));
                }
                idx += 1;
                ph_start = Instant::now();
                ph_bytes = 0;
                continue;
            }
        }
        if let Phase::Stall(_) = ph {
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                v.feed(&buf[..n]);
                got += n as u64;
                ph_bytes += n as u64;
                st.read.store(got, Ordering::Relaxed);
                if let Phase::Rate(mibps, _) = ph {
                    let allowed = ph_bytes as f64 / (mibps as f64 * 1048576.0);
                    let el = ph_start.elapsed().as_secs_f64();
                    if allowed > el {
                        thread::sleep(Duration::from_secs_f64(allowed - el));
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(_) => break,
        }
    }
    *st.res.lock().unwrap() = Some((got == total && v.ok(), v.first_bad.map(|x| x as i64).unwrap_or(-1), got));
    st.done.store(true, Ordering::Relaxed);
}

struct Sample {
    t_ms: u128,
    avail_kb: u64,
    sunrec_kb: u64,
    slab_kb: u64,
    a: i64,
    b: i64,
    c: i64,
    moved: i64,
    stall_dst: i64,
}

fn one(b: &mut Backend, dir: &str, sched: &str, total: u64, trial: usize, label: &str, max_s: u64) -> Value {
    let phases = parse_phases(sched);
    let mut p = Peer::new(1_000_000 + (std::process::id() % 100_000) * 64 + trial as u32, 4000, 1);
    p.connect();
    let v = accept(b.listener.as_ref().unwrap());
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let x = TcpStream::connect(addr).unwrap();
    let (a, _) = l.accept().unwrap();
    buffers(a.as_raw_fd());
    let (vfd, xfd, afd) = (v.as_raw_fd(), x.as_raw_fd(), a.as_raw_fd());
    let m0 = meminfo_kb(&["MemAvailable", "SUnreclaim", "Slab"]);
    let ev0_moved = ev_stat("bytes_v2t") + ev_stat("bytes_t2v");
    let ev0_stall = ev_stat("stall_dst");
    let id = b.ev_attach(vfd, xfd);
    // sampler
    let stop = Arc::new(AtomicBool::new(false));
    let samples: Arc<Mutex<Vec<Sample>>> = Arc::new(Mutex::new(vec![]));
    let (st2, sm2) = (stop.clone(), samples.clone());
    let t0 = Instant::now();
    let dirc = dir.to_string();
    let sampler = thread::spawn(move || {
        while !st2.load(Ordering::Relaxed) {
            let m = meminfo_kb(&["MemAvailable", "SUnreclaim", "Slab"]);
            // v2t: a = vsock source queue, b = TCP target leg send queue, c = target app receive queue
            // t2v: a = TCP source receive queue, b = app writer send queue, c = vsock unsent bytes
            let (qa, qb, qc) = if dirc == "v2t" {
                (ioctl_i32(vfd, 0x541B), ioctl_i32(xfd, 0x5411), ioctl_i32(afd, 0x541B))
            } else {
                (ioctl_i32(xfd, 0x541B), ioctl_i32(afd, 0x5411), ioctl_i32(vfd, 0x5411))
            };
            sm2.lock().unwrap().push(Sample {
                t_ms: t0.elapsed().as_millis(),
                avail_kb: m[0],
                sunrec_kb: m[1],
                slab_kb: m[2],
                a: qa.max(0),
                b: qb.max(0),
                c: qc.max(0),
                moved: ev_stat("bytes_v2t") + ev_stat("bytes_t2v") - ev0_moved,
                stall_dst: ev_stat("stall_dst") - ev0_stall,
            });
            thread::sleep(Duration::from_millis(50));
        }
    });
    let rst = Arc::new(RStats::default());
    let (mut blocks, mut first_block_sent, mut sent): (Vec<(u128, u128)>, Option<u64>, u64) = (vec![], None, 0);
    let mut block_end_ns: Vec<u64> = vec![];
    let mut cons_ok = None;
    let mut timed_out = false;
    let t_start = Instant::now();
    let limit = Duration::from_secs(max_s);
    if dir == "v2t" {
        let (s2, ph) = (rst.clone(), phases.clone());
        let ac = a.try_clone().unwrap();
        let rd = thread::spawn(move || reader(ac, total, ph, s2));
        let mut buf = vec![0u8; 32768];
        while sent < total {
            let n = std::cmp::min(32768, total - sent) as usize;
            if p.credit_free() < n as i64 {
                let bs = Instant::now();
                let mut last_req = Instant::now();
                while p.credit_free() < n as i64 && t_start.elapsed() < limit {
                    if last_req.elapsed() > Duration::from_millis(50) {
                        p.credit_request();
                        last_req = Instant::now();
                    }
                    let _ = p.try_rx(Duration::from_millis(20));
                }
                if first_block_sent.is_none() && bs.elapsed() > Duration::from_millis(50) {
                    first_block_sent = Some(sent);
                }
                if bs.elapsed() > Duration::from_millis(50) && blocks.len() < 64 {
                    blocks.push((bs.duration_since(t_start).as_millis(), bs.elapsed().as_millis()));
                    block_end_ns.push(mono_ns());
                }
                if t_start.elapsed() >= limit {
                    timed_out = true;
                    break;
                }
            }
            gen_fill(&mut buf[..n], sent / 8);
            p.tx(5, 0, &buf[..n]);
            sent += n as u64;
        }
        let end = Instant::now() + limit;
        while !rst.done.load(Ordering::Relaxed) && Instant::now() < end {
            let _ = p.try_rx(Duration::from_millis(10));
        }
        rd.join().unwrap();
    } else {
        // t2v: the TCP source app writes; this thread is the synthetic guest consumer
        let ac = a.try_clone().unwrap();
        let wst = Arc::new(Mutex::new((0u64, 0u128, Vec::<(u128, u128)>::new(), 0u64, Vec::<u64>::new())));
        let w2 = wst.clone();
        let wr = thread::spawn(move || {
            let mut buf = vec![0u8; 65536];
            let (mut sent, mut max_ms, mut blocks, mut at_longest) = (0u64, 0u128, Vec::new(), 0u64);
            let mut ends: Vec<u64> = vec![];
            let t0 = Instant::now();
            while sent < total {
                let n = std::cmp::min(65536, total - sent) as usize;
                gen_fill(&mut buf[..n], sent / 8);
                let mut off = 0;
                while off < n {
                    let ts = Instant::now();
                    let r = unsafe { libc::send(ac.as_raw_fd(), buf[off..n].as_ptr() as *const _, n - off, libc::MSG_NOSIGNAL) };
                    let el = ts.elapsed().as_millis();
                    if r < 0 {
                        *w2.lock().unwrap() = (sent, max_ms, blocks, at_longest, ends);
                        return;
                    }
                    if el > max_ms {
                        max_ms = el;
                        at_longest = sent;
                    }
                    if el >= 50 && blocks.len() < 64 {
                        blocks.push((t0.elapsed().as_millis(), el));
                        ends.push(mono_ns());
                    }
                    off += r as usize;
                    sent += r as u64;
                }
            }
            *w2.lock().unwrap() = (sent, max_ms, blocks, at_longest, ends);
        });
        let mut ver = Verifier::new();
        let (mut got, mut idx) = (0u64, 0usize);
        let (mut ph_start, mut stall_ends) = (Instant::now(), Vec::<(u64, u64)>::new());
        while got < total && t_start.elapsed() < limit {
            let ph = phases.get(idx).cloned().unwrap_or(Phase::Fast(None));
            let dur = match &ph {
                Phase::Stall(ms) => Some(*ms),
                Phase::Rate(_, d) | Phase::Fast(d) => *d,
            };
            if let Some(ms) = dur {
                if ph_start.elapsed() >= Duration::from_millis(ms) {
                    if let Phase::Stall(_) = ph {
                        stall_ends.push((mono_ns(), got));
                    }
                    idx += 1;
                    ph_start = Instant::now();
                    continue;
                }
            }
            if let Phase::Stall(_) = ph {
                thread::sleep(Duration::from_millis(2));
                continue;
            }
            if let Some((op, _, data)) = p.try_rx(Duration::from_millis(5)) {
                if op == 5 && !data.is_empty() {
                    ver.feed(&data);
                    got += data.len() as u64;
                    rst.read.store(got, Ordering::Relaxed);
                }
            }
        }
        timed_out = got < total;
        cons_ok = Some((got == total && ver.ok(), ver.first_bad.map(|x| x as i64).unwrap_or(-1), got));
        // let a stuck writer finish or give up
        let _ = wr.join();
        let w = wst.lock().unwrap();
        sent = w.0;
        blocks = w.2.clone();
        first_block_sent = Some(w.3);
        event(json!({"event":"v3_writer","label":label,"trial":trial,"writer_sent":w.0,"max_send_ms":w.1,"blocks":w.2.iter().map(|(t,ms)|json!([t,ms])).collect::<Vec<_>>(),"sent_at_longest_block_start":w.3,"block_end_mono_ns":w.4,"stall_ends":stall_ends}));
    }
    let secs = t_start.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    sampler.join().unwrap();
    let ev1_moved = ev_stat("bytes_v2t") + ev_stat("bytes_t2v");
    let m1 = meminfo_kb(&["MemAvailable", "SUnreclaim", "Slab"]);
    let detach_start = Instant::now();
    b.ev_detach(id);
    let detach_ms = detach_start.elapsed().as_millis();
    drop(v);
    drop(x);
    drop(a);
    drop(p);
    thread::sleep(Duration::from_millis(1500));
    let m2 = meminfo_kb(&["MemAvailable", "SUnreclaim", "Slab"]);
    let ss = samples.lock().unwrap();
    let peak = |f: &dyn Fn(&Sample) -> i64| ss.iter().map(f).max().unwrap_or(0);
    let peak_pipe = ss.iter().map(|s| s.a + s.b + s.c).max().unwrap_or(0);
    let max_sun = ss.iter().map(|s| s.sunrec_kb).max().unwrap_or(0);
    let min_avail = ss.iter().map(|s| s.avail_kb).min().unwrap_or(0);
    let max_slab = ss.iter().map(|s| s.slab_kb).max().unwrap_or(0);
    let step = std::cmp::max(1, ss.len() / 40);
    let series: Vec<Value> = ss.iter().step_by(step).map(|s| json!([s.t_ms, s.a, s.b, s.c, s.moved, s.stall_dst, s.sunrec_kb, s.avail_kb])).collect();
    let (ok, first_bad, got) = if let Some(c) = cons_ok { c } else { rst.res.lock().unwrap().unwrap_or((false, -2, 0)) };
    json!({"event":"v3_trial","label":label,"dir":dir,"schedule":sched,"trial":trial,"total":total,"sent":sent,"consumed":got,"ok":ok,"first_bad":first_bad,"timed_out":timed_out,"secs":secs,
           "module_moved":ev1_moved-ev0_moved,"peak_pipe":peak_pipe,"peak_a":peak(&|s| s.a),"peak_b":peak(&|s| s.b),"peak_c":peak(&|s| s.c),
           "sunrec_start_kb":m0[1],"sunrec_max_kb":max_sun,"sunrec_end_kb":m1[1],"sunrec_after_close_kb":m2[1],"avail_start_kb":m0[0],"avail_min_kb":min_avail,"slab_start_kb":m0[2],"slab_max_kb":max_slab,
           "sender_blocks_ms":blocks.iter().map(|(t,ms)|json!([t,ms])).collect::<Vec<_>>(),"block_end_mono_ns":block_end_ns,"sent_at_first_block":first_block_sent,"detach_ms":detach_ms,"series_cols":"t_ms,a,b,c,moved,stall_dst,sunrec_kb,avail_kb","series":series,
           "stall_ends":rst.stall_ends.lock().unwrap().iter().map(|(t,b)|json!([t,b])).collect::<Vec<_>>()})
}

pub fn run(args: &[String]) {
    let limit = libc::rlimit { rlim_cur: 65536, rlim_max: 65536 };
    unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
    let dir = args[0].as_str();
    let sched = args[1].as_str();
    let total = args[2].parse::<u64>().unwrap() * 1048576;
    let trials: usize = args[3].parse().unwrap();
    let label = args[4].as_str();
    let max_s: u64 = args.get(5).map(|x| x.parse().unwrap()).unwrap_or(60);
    let mut b = Backend::new(false, true, false);
    event(json!({"event":"v3_begin","label":label,"dir":dir,"schedule":sched,"total":total,"trials":trials,"uname":super::read("/proc/sys/kernel/osrelease"),"evmod":super::read("/proc/evmod")}));
    for t in 0..trials {
        let r = one(&mut b, dir, sched, total, t, label, max_s);
        event(r);
    }
    event(json!({"event":"v3_done","label":label,"evmod":super::read("/proc/evmod")}));
}
