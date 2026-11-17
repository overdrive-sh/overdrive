//! V-2 probe: does the DDD-12 `QueryObserver` (two `Clock::now()` reads + one dynamic call per
//! datagram) cost less than the A/A noise floor on a UDP query?
//!
//! COPIED SHAPE (not linked): `DnsResponder::serve_one_socket`, crates/overdrive-control-plane/
//! src/dns_responder/responder.rs:354-420 -- blocking `recvmsg` with `IP_PKTINFO` cmsg space and
//! `SO_RCVTIMEO` 500 ms, stop flag check per loop, reply `sendmsg` source-pinned through an
//! `Ipv4PacketInfo` cmsg. The decode/answer/encode step (`answer_datagram`, hickory) is replaced
//! by a fixed 70-byte reply carrying the query id: that REMOVES real per-query work (several
//! microseconds), so the observer's share of the service time here is overstated (conservative).
//!
//! Arms (one fresh server per replicate, one serving thread):
//!   A   the responder as it is today: no observer code at all
//!   A2  the same as A (the A/A control)
//!   B   production composition after DDD-12: `NoQueryObserver` (empty body), two clock reads + dyn call
//!   C   a recording sink (appends (outcome, ns) to its own preallocated Vec)
//! Load: scheduled arrival, fixed schedule, latency measured from the SCHEDULED send time, replies
//! matched by DNS id (first two bytes). Usage: nbp-v2-dns-observer <seed> <replicates> <secs> <rate> <csv>
use std::net::{SocketAddr, SocketAddrV4, UdpSocket};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nix::sys::socket::sockopt::{Ipv4PacketInfo, ReceiveTimeout, ReuseAddr};
use nix::sys::socket::{
    bind, recvmsg, sendmsg, setsockopt, socket, AddressFamily, ControlMessage,
    ControlMessageOwned, MsgFlags, SockFlag, SockProtocol, SockType, SockaddrIn,
};
use nix::sys::time::TimeVal;

// ---- the DDD-12 contract (copied from the design text) ----
trait QueryObserver: Send + Sync {
    fn observe(&self, outcome: QueryOutcome, service_time: Duration);
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
enum QueryOutcome {
    Answered,
    NoData,
    NameError,
    Dropped,
}
struct NoQueryObserver;
impl QueryObserver for NoQueryObserver {
    fn observe(&self, _: QueryOutcome, _: Duration) {}
}
struct Recorder(Mutex<Vec<(QueryOutcome, u64)>>);
impl QueryObserver for Recorder {
    fn observe(&self, outcome: QueryOutcome, d: Duration) {
        self.0.lock().unwrap().push((outcome, d.as_nanos() as u64));
    }
}
/// Stand-in for the injected `Clock` port (a dynamic call per read, as production).
trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}
struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Arm {
    A,
    A2,
    B,
    C,
}
impl Arm {
    fn label(self) -> &'static str {
        match self {
            Arm::A => "A",
            Arm::A2 => "A2",
            Arm::B => "B",
            Arm::C => "C",
        }
    }
}

fn pin(cpu: usize) {
    // SAFETY: plain libc affinity call on the calling thread with a zeroed, correctly sized set.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(cpu, &mut set);
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
    }
}

fn bind_one(port: u16) -> OwnedFd {
    let fd = socket(AddressFamily::Inet, SockType::Datagram, SockFlag::SOCK_CLOEXEC, SockProtocol::Udp).unwrap();
    setsockopt(&fd, ReuseAddr, &true).unwrap();
    setsockopt(&fd, Ipv4PacketInfo, &true).unwrap();
    setsockopt(&fd, ReceiveTimeout, &TimeVal::new(0, 500_000)).unwrap();
    bind(fd.as_raw_fd(), &SockaddrIn::from(SocketAddrV4::new("127.0.0.1".parse().unwrap(), port))).unwrap();
    fd
}

fn serve_one_socket(
    fd: &OwnedFd,
    stop: &AtomicBool,
    arm: Arm,
    clock: &Arc<dyn Clock>,
    observer: &Arc<dyn QueryObserver>,
) {
    let mut buf = [0u8; 1500];
    let mut cmsg_space = nix::cmsg_space!(libc::in_pktinfo);
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let received: Option<(Vec<u8>, SockaddrIn, Option<std::net::Ipv4Addr>)> = {
            let mut iov = [std::io::IoSliceMut::new(&mut buf)];
            match recvmsg::<SockaddrIn>(fd.as_raw_fd(), &mut iov, Some(&mut cmsg_space), MsgFlags::MSG_CMSG_CLOEXEC) {
                Ok(recvd) => {
                    let Some(peer) = recvd.address else { continue };
                    let bytes = recvd.bytes;
                    let spec_dst = recvd.cmsgs().ok().and_then(|mut c| {
                        c.find_map(|m| match m {
                            ControlMessageOwned::Ipv4PacketInfo(i) => Some(std::net::Ipv4Addr::from(u32::from_be(i.ipi_spec_dst.s_addr))),
                            _ => None,
                        })
                    });
                    Some((iov[0][..bytes].to_vec(), peer, spec_dst))
                }
                Err(nix::errno::Errno::EINTR | nix::errno::Errno::EAGAIN) => continue,
                Err(_) => return,
            }
        };
        // DDD-12: service time runs "from the instant the receive returned"
        let started = if matches!(arm, Arm::B | Arm::C) { Some(clock.now()) } else { None };
        let Some((datagram, peer, spec_dst)) = received else { continue };
        // stand-in for answer_datagram: 70-byte reply carrying the id
        let mut reply = [0u8; 70];
        reply[0] = datagram[0];
        reply[1] = datagram[1];
        let pktinfo = spec_dst.map(|ip| libc::in_pktinfo {
            ipi_ifindex: 0,
            ipi_spec_dst: libc::in_addr { s_addr: u32::from(ip).to_be() },
            ipi_addr: libc::in_addr { s_addr: 0 },
        });
        let send_iov = [std::io::IoSlice::new(&reply)];
        let cmsgs: Vec<ControlMessage> = pktinfo.as_ref().map(|pi| vec![ControlMessage::Ipv4PacketInfo(pi)]).unwrap_or_default();
        let _ = sendmsg(fd.as_raw_fd(), &send_iov, &cmsgs, MsgFlags::empty(), Some(&peer));
        if let Some(t0) = started {
            let d = clock.now().duration_since(t0);
            observer.observe(QueryOutcome::Answered, d);
        }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}
fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    if n % 2 == 1 { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2.0 }
}
fn quantile(v: &[f64], q: f64) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pos = q * (s.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    s[lo] + (s[hi] - s[lo]) * (pos - lo as f64)
}
fn boot_diff(a: &[f64], b: &[f64], rng: &mut Rng) -> (f64, f64, f64) {
    let point = median(a) - median(b);
    let mut d = Vec::with_capacity(10_000);
    for _ in 0..10_000 {
        let ra: Vec<f64> = (0..a.len()).map(|_| a[rng.below(a.len())]).collect();
        let rb: Vec<f64> = (0..b.len()).map(|_| b[rng.below(b.len())]).collect();
        d.push(median(&ra) - median(&rb));
    }
    (point, quantile(&d, 0.025), quantile(&d, 0.975))
}

struct Result1 {
    p50_ns: f64,
    p99_ns: f64,
    lost: usize,
    sent: usize,
    svc_mean_ns: f64,
}

fn replicate(arm: Arm, secs: u64, rate: u64) -> Result1 {
    let fd = bind_one(0);
    let port = nix::sys::socket::getsockname::<SockaddrIn>(fd.as_raw_fd()).unwrap().port();
    let stop = Arc::new(AtomicBool::new(false));
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let recorder = Arc::new(Recorder(Mutex::new(Vec::with_capacity(secs as usize * rate as usize + 1024))));
    let observer: Arc<dyn QueryObserver> = match arm {
        Arm::C => recorder.clone(),
        _ => Arc::new(NoQueryObserver),
    };
    let server = {
        let (stop, clock, observer) = (stop.clone(), clock.clone(), observer.clone());
        std::thread::spawn(move || {
            pin(2);
            serve_one_socket(&fd, &stop, arm, &clock, &observer);
        })
    };
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.connect(SocketAddr::from(([127, 0, 0, 1], port))).unwrap();
    let rsock = sock.try_clone().unwrap();
    rsock.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let n = (secs * rate) as usize;
    let period_ns = 1_000_000_000 / rate;
    let t0 = Instant::now() + Duration::from_millis(50);
    let recv_ns: Arc<Vec<AtomicU64>> = Arc::new((0..n).map(|_| AtomicU64::new(0)).collect());
    let receiver = {
        let recv_ns = recv_ns.clone();
        std::thread::spawn(move || {
            pin(4);
            let mut b = [0u8; 256];
            let deadline = t0 + Duration::from_secs(secs) + Duration::from_millis(1500);
            while Instant::now() < deadline {
                if let Ok(len) = rsock.recv(&mut b) {
                    let now = Instant::now().duration_since(t0).as_nanos() as u64;
                    if len >= 2 {
                        let id = u16::from_be_bytes([b[0], b[1]]) as usize;
                        // ids wrap at 65536; the schedule is <= 65535 queries per replicate
                        if id < n {
                            recv_ns[id].store(now.max(1), Ordering::Relaxed);
                        }
                    }
                }
            }
        })
    };
    pin(3);
    let q = {
        let mut q = [0u8; 40];
        q[2] = 1;
        q[5] = 1;
        q
    };
    for i in 0..n {
        let sched = t0 + Duration::from_nanos(i as u64 * period_ns);
        while Instant::now() < sched {
            std::hint::spin_loop();
        }
        let mut m = q;
        m[0..2].copy_from_slice(&(i as u16).to_be_bytes());
        let _ = sock.send(&m);
    }
    receiver.join().unwrap();
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();
    let mut lat: Vec<f64> = Vec::with_capacity(n);
    let mut lost = 0;
    for i in 0..n {
        let r = recv_ns[i].load(Ordering::Relaxed);
        if r == 0 {
            lost += 1;
        } else {
            lat.push(r as f64 - (i as u64 * period_ns) as f64);
        }
    }
    let svc: Vec<(QueryOutcome, u64)> = recorder.0.lock().unwrap().clone();
    let svc_mean = if svc.is_empty() { f64::NAN } else { svc.iter().map(|x| x.1 as f64).sum::<f64>() / svc.len() as f64 };
    Result1 { p50_ns: quantile(&lat, 0.5), p99_ns: quantile(&lat, 0.99), lost, sent: n, svc_mean_ns: svc_mean }
}

/// Isolated cost of the observer instrumentation itself (single thread, tight loop).
fn microbench() {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let none: Arc<dyn QueryObserver> = Arc::new(NoQueryObserver);
    let rec = Arc::new(Recorder(Mutex::new(Vec::with_capacity(20_000_000))));
    let recd: Arc<dyn QueryObserver> = rec.clone();
    const N: u32 = 10_000_000;
    pin(2);
    for (name, obs) in [("baseline-empty", None), ("NoQueryObserver(2 now + dyn call)", Some(none)), ("Recorder(2 now + dyn call + Vec push)", Some(recd))] {
        let mut best = f64::MAX;
        for _ in 0..5 {
            let t = Instant::now();
            for _ in 0..N {
                if let Some(o) = &obs {
                    let s = clock.now();
                    let d = clock.now().duration_since(s);
                    o.observe(QueryOutcome::Answered, std::hint::black_box(d));
                } else {
                    std::hint::black_box(());
                }
            }
            best = best.min(t.elapsed().as_nanos() as f64 / f64::from(N));
        }
        println!("MICRO {name}: {best:.1} ns per datagram (best of 5 x {N})");
        rec.0.lock().unwrap().clear();
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let seed: u64 = a.get(1).map_or(1, |s| s.parse().unwrap());
    let reps: usize = a.get(2).map_or(30, |s| s.parse().unwrap());
    let secs: u64 = a.get(3).map_or(10, |s| s.parse().unwrap());
    let rate: u64 = a.get(4).map_or(5000, |s| s.parse().unwrap());
    let csv_path = a.get(5).cloned().unwrap_or_else(|| "runs.csv".into());
    assert!(secs * rate < 65_536, "DNS id is 16-bit");
    println!("# uname -r: {}", String::from_utf8_lossy(&std::process::Command::new("uname").arg("-r").output().unwrap().stdout).trim());
    println!("# seed={seed} replicates={reps} secs={secs} rate={rate}/s serve cpu2 send cpu3 recv cpu4 (SMT siblings 10,11,12 idle under exclusive)");
    println!("# governor: {}", std::fs::read_to_string("/sys/devices/system/cpu/cpu2/cpufreq/scaling_governor").unwrap_or_default().trim());
    microbench();
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut res: Vec<(Arm, Result1)> = Vec::new();
    let mut csv = String::from("block,pos,arm,p50_ns,p99_ns,lost,sent,svc_mean_ns\n");
    // warm-up replicate, discarded
    let _ = replicate(Arm::A, 2, rate);
    for b in 0..reps {
        let mut order = [Arm::A, Arm::A2, Arm::B, Arm::C];
        for i in (1..4).rev() {
            order.swap(i, rng.below(i + 1));
        }
        for (pos, arm) in order.into_iter().enumerate() {
            let r = replicate(arm, secs, rate);
            let row = format!("{b},{pos},{},{:.0},{:.0},{},{},{:.1}", arm.label(), r.p50_ns, r.p99_ns, r.lost, r.sent, r.svc_mean_ns);
            println!("{row}");
            csv.push_str(&row);
            csv.push('\n');
            res.push((arm, r));
        }
    }
    std::fs::write(&csv_path, csv).unwrap();
    for (metric, f) in [("client p50 latency ns", (&|r: &Result1| r.p50_ns) as &dyn Fn(&Result1) -> f64), ("client p99 latency ns", &|r: &Result1| r.p99_ns)] {
        println!("# --- {metric} (delta of per-replicate medians, 95% percentile bootstrap, 10000 resamples) ---");
        let col = |arm: Arm| -> Vec<f64> { res.iter().filter(|x| x.0 == arm).map(|x| f(&x.1)).collect() };
        let (a, a2, b, c) = (col(Arm::A), col(Arm::A2), col(Arm::B), col(Arm::C));
        println!("# median of replicates: A={:.0} A2={:.0} B={:.0} C={:.0}", median(&a), median(&a2), median(&b), median(&c));
        let aa = boot_diff(&a2, &a, &mut rng);
        println!("# A/A  (A2-A): {:.1} CI95=[{:.1},{:.1}]", aa.0, aa.1, aa.2);
        let floor = aa.1.abs().max(aa.2.abs());
        for (name, v) in [("B-A", &b), ("C-A", &c)] {
            let d = boot_diff(v, &a, &mut rng);
            let excl = d.1 > 0.0 || d.2 < 0.0;
            let verdict = if excl && d.0.abs() > floor { "distinguishable" } else { "not distinguishable from noise" };
            println!("# {name}: {:.1} CI95=[{:.1},{:.1}] A/A floor bound {:.1} -> {verdict}", d.0, d.1, d.2, floor);
        }
    }
    let lost: usize = res.iter().map(|x| x.1.lost).sum();
    let sent: usize = res.iter().map(|x| x.1.sent).sum();
    println!("# total lost={lost} of {sent}");
    let svc: Vec<f64> = res.iter().filter(|x| x.0 == Arm::C).map(|x| x.1.svc_mean_ns).collect();
    println!("# arm C observer-recorded mean service time (recv-return to send-complete), median over replicates: {:.0} ns", median(&svc));
}
