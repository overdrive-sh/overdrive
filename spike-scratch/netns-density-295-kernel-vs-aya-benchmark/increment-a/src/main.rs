//! Common producer/consumer harness. It never reads or writes a proxy-leg fd.
//! Each synthetic endpoint owns its application messages and private wire codec.
mod peer;
use aya::{
    maps::{HashMap as BpfHash, PerCpuArray, SockHash},
    programs::{SchedClassifier, SkSkb, TcAttachType},
    EbpfLoader,
};
use peer::Peer;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
fn event(v: Value) {
    println!("{v}");
    io::stdout().flush().unwrap()
}
fn read(p: &str) -> String {
    fs::read_to_string(p).unwrap_or_else(|e| format!("UNAVAILABLE: {e}"))
}
fn snapshot() -> Value {
    let proc = fs::read_dir("/proc").unwrap();
    let mut workers = serde_json::Map::new();
    let mut tasks = 0;
    for p in proc.filter_map(Result::ok) {
        if let Ok(pid) = p.file_name().to_string_lossy().parse::<u32>() {
            let comm = read(&format!("/proc/{pid}/comm"));
            for name in ["shmps-", "shmprx-", "vhost-", "shmp-accept"] {
                if comm.starts_with(name) {
                    *workers.entry(name).or_insert(json!(0)) =
                        json!(workers.get(name).and_then(Value::as_u64).unwrap_or(0) + 1);
                }
            }
            tasks += fs::read_dir(p.path().join("task")).map(|x| x.count()).unwrap_or(0);
        }
    }
    json!({"proc_stat":read("/proc/stat"),"uptime":read("/proc/uptime"),"meminfo":read("/proc/meminfo"),"slabinfo":read("/proc/slabinfo"),"sockstat":read("/proc/net/sockstat"),"sockstat6":read("/proc/net/sockstat6"),"self_status":read("/proc/self/status"),"self_stat":read("/proc/self/stat"),"self_smaps_rollup":read("/proc/self/smaps_rollup"),"self_tasks":fs::read_dir("/proc/self/task").unwrap().count(),"all_tasks":tasks,"self_fds":fs::read_dir("/proc/self/fd").unwrap().count(),"kernel_workers":workers,"modules":read("/proc/modules"),"module_stats":read("/proc/shmproxy")})
}
#[repr(C)]
struct VsockAddr {
    family: u16,
    reserved: u16,
    port: u32,
    cid: u32,
    flags: u8,
    zero: [u8; 3],
}
fn listener(typ: i32) -> OwnedFd {
    unsafe {
        let f = OwnedFd::from_raw_fd(libc::socket(libc::AF_VSOCK, typ, 0));
        let a = VsockAddr {
            family: libc::AF_VSOCK as u16,
            reserved: 0,
            port: 29500,
            cid: 2,
            flags: 0,
            zero: [0; 3],
        };
        assert_eq!(
            libc::bind(f.as_raw_fd(), &a as *const _ as *const _, 16),
            0,
            "{}",
            io::Error::last_os_error()
        );
        assert_eq!(libc::listen(f.as_raw_fd(), 32768), 0);
        f
    }
}
fn accept(f: &OwnedFd) -> OwnedFd {
    unsafe {
        let x = libc::accept(f.as_raw_fd(), std::ptr::null_mut(), std::ptr::null_mut());
        assert!(x >= 0);
        OwnedFd::from_raw_fd(x)
    }
}
fn cookie(fd: i32) -> u64 {
    let mut c = 0u64;
    let mut n = 8u32;
    assert_eq!(
        unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, 57, &mut c as *mut _ as *mut _, &mut n) },
        0
    );
    c
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Tuple {
    src: u32,
    dst: u32,
    sport: u16,
    dport: u16,
}
unsafe impl aya::Pod for Tuple {}
fn tuple(src: SocketAddr, dst: SocketAddr) -> Tuple {
    let (SocketAddr::V4(a), SocketAddr::V4(b)) = (src, dst) else { panic!() };
    Tuple {
        src: u32::from_ne_bytes(a.ip().octets()),
        dst: u32::from_ne_bytes(b.ip().octets()),
        sport: a.port().to_be(),
        dport: b.port().to_be(),
    }
}
fn buffers(fd: i32) {
    for option in [libc::SO_RCVBUF, libc::SO_SNDBUF] {
        let n = 262144i32;
        assert_eq!(
            unsafe {
                libc::setsockopt(fd, libc::SOL_SOCKET, option, &n as *const _ as *const _, 4)
            },
            0
        );
    }
}
fn nocheck(fd: i32) {
    let n = 1i32;
    assert_eq!(
        unsafe {
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_NO_CHECK, &n as *const _ as *const _, 4)
        },
        0
    );
}
fn counters(b: &aya::Ebpf) -> Vec<u64> {
    let a = PerCpuArray::<_, u64>::try_from(b.map("COUNTERS").unwrap()).unwrap();
    (0..32).map(|i| a.get(&i, 0).unwrap().iter().sum()).collect()
}
struct Backend {
    kernel: bool,
    bpf: Option<aya::Ebpf>,
    listener: Option<OwnedFd>,
}
impl Backend {
    fn new(kernel: bool, udp: bool) -> Self {
        if kernel {
            return Self { kernel, bpf: None, listener: None };
        }
        let mut b = EbpfLoader::new().load_file(if udp { "/udp.o" } else { "/tcp.o" }).unwrap();
        let fd = SockHash::<_, u32>::try_from(b.map("SOCKETS").unwrap())
            .unwrap()
            .fd()
            .try_clone()
            .unwrap();
        let p: &mut SkSkb = b.program_mut("route").unwrap().try_into().unwrap();
        p.load().unwrap();
        event(
            json!({"event":"program","protocol":if udp{"udp"}else{"tcp"},"id":p.info().unwrap().id(),"tag":format!("{:016x}",p.info().unwrap().tag()),"verified_instructions":p.info().unwrap().verified_instruction_count()}),
        );
        p.attach(&fd).unwrap();
        if udp {
            let p: &mut SchedClassifier = b.program_mut("packet").unwrap().try_into().unwrap();
            p.load().unwrap();
            p.attach("lo", TcAttachType::Egress).unwrap();
        }
        Self {
            kernel,
            bpf: Some(b),
            listener: Some(listener(if udp { libc::SOCK_SEQPACKET } else { libc::SOCK_STREAM })),
        }
    }
    fn register(&mut self, a: i32, v: i32, key: u32) {
        let b = self.bpf.as_mut().unwrap();
        let mut s = SockHash::<_, u32>::try_from(b.map_mut("SOCKETS").unwrap()).unwrap();
        s.insert(key, a, 0).unwrap();
        s.insert(key + 1, v, 0).unwrap();
        let mut r = BpfHash::<_, u64, u32>::try_from(b.map_mut("ROUTES").unwrap()).unwrap();
        r.insert(cookie(a), key + 1, 0).unwrap();
        r.insert(cookie(v), key, 0).unwrap();
    }
    fn receipt(&self) -> Value {
        match &self.bpf {
            None => json!({"module_stats":read("/proc/shmproxy")}),
            Some(b) => {
                json!({"occupied_sockhash":SockHash::<_,u32>::try_from(b.map("SOCKETS").unwrap()).unwrap().keys().count(),"route_keys":BpfHash::<_,u64,u32>::try_from(b.map("ROUTES").unwrap()).unwrap().keys().count(),"counters_0_31":counters(b)})
            }
        }
    }
    fn retire(&mut self, mut o: Owner) {
        if !self.kernel {
            let b = self.bpf.as_mut().unwrap();
            let proxy = o.proxy.as_ref().unwrap().fd();
            let vs = o.vs.as_ref().unwrap().as_raw_fd();
            let mut r = BpfHash::<_, u64, u32>::try_from(b.map_mut("ROUTES").unwrap()).unwrap();
            r.remove(&cookie(proxy)).unwrap();
            r.remove(&cookie(vs)).unwrap();
            let mut s = SockHash::<_, u32>::try_from(b.map_mut("SOCKETS").unwrap()).unwrap();
            s.remove(&o.key).unwrap();
            s.remove(&(o.key + 1)).unwrap();
            if o.udp {
                let mut f =
                    BpfHash::<_, Tuple, u32>::try_from(b.map_mut("FLOWS").unwrap()).unwrap();
                f.remove(&tuple(o.app.addr(), o.local)).unwrap();
                f.remove(&tuple(o.local, o.app.addr())).unwrap();
            }
        }
        o.p.tx(3, 0, &[]);
        drop(o);
    }
}
enum App {
    Tcp(TcpStream),
    Udp(UdpSocket),
}
impl App {
    fn fd(&self) -> i32 {
        match self {
            Self::Tcp(x) => x.as_raw_fd(),
            Self::Udp(x) => x.as_raw_fd(),
        }
    }
    fn addr(&self) -> SocketAddr {
        match self {
            Self::Tcp(x) => x.local_addr().unwrap(),
            Self::Udp(x) => x.local_addr().unwrap(),
        }
    }
}
struct Owner {
    p: Peer,
    vs: Option<OwnedFd>,
    proxy: Option<App>,
    app: App,
    local: SocketAddr,
    key: u32,
    udp: bool,
    kernel: bool,
    stream: Vec<u8>,
}
fn frame(p: &mut Peer, kind: u8, addr: SocketAddr, data: &[u8]) {
    let SocketAddr::V4(addr) = addr else { panic!() };
    let mut x = vec![kind, 0];
    x.extend(addr.port().to_be_bytes());
    x.extend(addr.ip().octets());
    x.extend((data.len() as u32).to_be_bytes());
    x.extend(data);
    p.bytes(&x, 32768);
}
impl Owner {
    fn new(b: &mut Backend, udp: bool, i: usize) -> Self {
        let mut p =
            Peer::new(300000 + i as u32, 4000 + i as u32, if !b.kernel && udp { 2 } else { 1 });
        let key = i as u32 * 2;
        let mut vs = None;
        let mut proxy = None;
        let app;
        let local;
        if udp {
            let a = UdpSocket::bind((
                Ipv4Addr::from(if i % 2 == 0 { [127, 0, 0, 2] } else { [127, 0, 0, 3] }),
                40000 + (i / 2) as u16,
            ))
            .unwrap();
            a.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            buffers(a.as_raw_fd());
            nocheck(a.as_raw_fd());
            let addr = SocketAddr::from(([127, 0, 0, 1], 20000 + i as u16));
            if b.kernel {
                p.connect();
                frame(&mut p, 3, addr, &[]);
                let mut o = Self {
                    p,
                    vs,
                    proxy,
                    app: App::Udp(a),
                    local: addr,
                    key,
                    udp,
                    kernel: true,
                    stream: vec![],
                };
                let (kind, src, data) = o.kernel_get();
                assert_eq!(kind, 20);
                assert_eq!(src, addr);
                assert!(data.is_empty());
                return o;
            } else {
                let x = UdpSocket::bind(addr).unwrap();
                x.connect(a.local_addr().unwrap()).unwrap();
                a.connect(addr).unwrap();
                buffers(x.as_raw_fd());
                nocheck(x.as_raw_fd());
                p.connect();
                let v = accept(b.listener.as_ref().unwrap());
                b.register(x.as_raw_fd(), v.as_raw_fd(), key);
                let bb = b.bpf.as_mut().unwrap();
                let mut f =
                    BpfHash::<_, Tuple, u32>::try_from(bb.map_mut("FLOWS").unwrap()).unwrap();
                f.insert(tuple(a.local_addr().unwrap(), addr), 1, 0).unwrap();
                f.insert(tuple(addr, a.local_addr().unwrap()), 2, 0).unwrap();
                vs = Some(v);
                proxy = Some(App::Udp(x));
                app = App::Udp(a);
                local = addr;
            }
        } else {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = l.local_addr().unwrap();
            if b.kernel {
                p.connect();
                frame(&mut p, 1, addr, &[]);
                let (mut a, _) = l.accept().unwrap();
                a.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                a.set_nodelay(true).unwrap();
                buffers(a.as_raw_fd());
                let mut o = Self {
                    p,
                    vs,
                    proxy,
                    app: App::Tcp(a),
                    local: addr,
                    key,
                    udp,
                    kernel: true,
                    stream: vec![],
                };
                let (kind, _, data) = o.kernel_get();
                assert_eq!(kind, 20);
                assert!(data.is_empty());
                return o;
            } else {
                p.connect();
                let v = accept(b.listener.as_ref().unwrap());
                let x = TcpStream::connect(addr).unwrap();
                x.set_nodelay(true).unwrap();
                let (a, _) = l.accept().unwrap();
                a.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                a.set_nodelay(true).unwrap();
                buffers(a.as_raw_fd());
                buffers(x.as_raw_fd());
                b.register(x.as_raw_fd(), v.as_raw_fd(), key);
                vs = Some(v);
                proxy = Some(App::Tcp(x));
                app = App::Tcp(a);
                local = addr;
            }
        }
        Self { p, vs, proxy, app, local, key, udp, kernel: b.kernel, stream: vec![] }
    }
    fn kernel_get(&mut self) -> (u8, SocketAddr, Vec<u8>) {
        loop {
            if self.stream.len() >= 12 {
                let n = u32::from_be_bytes(self.stream[8..12].try_into().unwrap()) as usize;
                assert!(n <= 60000);
                if self.stream.len() >= 12 + n {
                    let k = self.stream[0];
                    let port = u16::from_be_bytes(self.stream[2..4].try_into().unwrap());
                    let ip: [u8; 4] = self.stream[4..8].try_into().unwrap();
                    let data = self.stream[12..12 + n].to_vec();
                    self.stream.drain(..12 + n);
                    return (k, SocketAddr::from((ip, port)), data);
                }
            }
            let (op, _, x) = self.p.rx();
            if op == 5 {
                self.stream.extend(x)
            } else {
                assert_ne!(op, 3, "unexpected reset CID {}", self.p.cid)
            }
        }
    }
    fn guest_receive(&mut self, n: usize) -> Vec<u8> {
        if self.kernel {
            if self.udp {
                let (k, src, x) = self.kernel_get();
                assert_eq!(k, 10);
                assert_eq!(src, self.app.addr());
                x
            } else {
                let mut x = vec![];
                while x.len() < n {
                    let (k, _, d) = self.kernel_get();
                    assert_eq!(k, 10);
                    x.extend(d);
                }
                x
            }
        } else if self.udp {
            let mut x = vec![];
            loop {
                let (op, flags, d) = self.p.rx();
                if op == 5 {
                    x.extend(d);
                    if flags & 1 != 0 {
                        break;
                    }
                } else {
                    assert_eq!(op, 6)
                }
            }
            assert!(x.len() >= 8);
            assert_eq!(&x[..4], b"ZUD1");
            assert_eq!(u32::from_be_bytes(x[4..8].try_into().unwrap()) as usize, x.len() - 8);
            x[8..].to_vec()
        } else {
            let mut x = vec![];
            while x.len() < n {
                let (op, _, d) = self.p.rx();
                if op == 5 {
                    x.extend(d)
                } else {
                    assert_ne!(op, 3)
                }
            }
            x
        }
    }
    fn guest_send(&mut self, x: &[u8]) {
        if self.kernel {
            if self.udp {
                frame(&mut self.p, 10, self.app.addr(), x);
            } else {
                for chunk in x.chunks(59000) {
                    frame(&mut self.p, 10, self.local, chunk)
                }
            }
        } else if self.udp {
            let mut wire = b"ZUD1".to_vec();
            wire.extend((x.len() as u32).to_be_bytes());
            wire.extend(x);
            self.p.tx(5, 1, &wire);
        } else {
            self.p.bytes(x, 32768);
        }
    }
    fn roundtrip(&mut self, size: usize, seq: u64) -> u64 {
        let x: Vec<u8> = (0..size)
            .map(|i| ((i * 31 + self.p.cid as usize + seq as usize) % 251) as u8)
            .collect();
        let start = Instant::now();
        match &mut self.app {
            App::Tcp(a) => a.write_all(&x).unwrap(),
            App::Udp(a) => assert_eq!(a.send_to(&x, self.local).unwrap(), size),
        }
        let got = self.guest_receive(size);
        assert_eq!(got, x, "host-to-guest CID {} seq {}", self.p.cid, seq);
        // The guest consumer responds with a separately generated message it owns.
        let reply: Vec<u8> = x.iter().map(|x| x ^ 0xa5).collect();
        self.guest_send(&reply);
        match &mut self.app {
            App::Tcp(a) => {
                let mut data = vec![0; size];
                a.read_exact(&mut data).unwrap();
                assert_eq!(data, reply)
            }
            App::Udp(a) => {
                let mut data = vec![0; 65536];
                let mut poll = libc::pollfd { fd: a.as_raw_fd(), events: libc::POLLIN, revents: 0 };
                assert_eq!(unsafe { libc::poll(&mut poll, 1, 5000) }, 1);
                let (n, src) = a.recv_from(&mut data).unwrap();
                assert_eq!(src, self.local);
                assert_eq!(n, size);
                assert_eq!(&data[..n], reply);
            }
        }
        start.elapsed().as_nanos() as u64
    }
}
fn audit(owners: &mut [Owner], b: &Backend, tag: &str) {
    for o in owners.iter_mut() {
        o.roundtrip(64, SEQUENCE.fetch_add(1, Ordering::Relaxed));
    }
    let fds = fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .filter(|x| {
            fs::read_link(x.path())
                .ok()
                .is_some_and(|x| x == std::path::Path::new("/dev/vhost-vsock"))
        })
        .count();
    let memories: HashSet<usize> = owners.iter().map(|o| o.p.mem as usize).collect();
    assert_eq!(fds, owners.len());
    assert_eq!(memories.len(), owners.len());
    if let Some(b) = &b.bpf {
        assert_eq!(
            SockHash::<_, u32>::try_from(b.map("SOCKETS").unwrap()).unwrap().keys().count(),
            owners.len() * 2
        );
    }
    event(
        json!({"event":"audit","tag":tag,"actual_vhost_devices":fds,"independent_memory_contexts":memories.len(),"cid_first":owners.first().map(|x|x.p.cid),"cid_last":owners.last().map(|x|x.p.cid),"every_owner_tagged_bidirectional":true,"backend":b.receipt(),"snapshot":snapshot()}),
    );
}
fn window(
    owners: Vec<Owner>,
    active: usize,
    size: usize,
    rate: u64,
    secs: f64,
    rep: usize,
    variant: &str,
    protocol: &str,
) -> Vec<Owner> {
    let before = read("/proc/stat");
    let before_backend = if variant == "kernel" { read("/proc/shmproxy") } else { String::new() };
    let id = format!("{variant}-{protocol}-{}-{active}-{size}-{rate}-{rep}", owners.len());
    let perfpath = format!("/tmp/perf-{id}.txt");
    let mut perf = Command::new("/perf")
        .args([
            "stat",
            "-a",
            "-e",
            "cpu-clock,task-clock,context-switches,cpu-migrations,page-faults",
            "-o",
            &perfpath,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
    thread::sleep(Duration::from_millis(30));
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(secs);
    let mut shards: Vec<Vec<(usize, Owner)>> = (0..4).map(|_| vec![]).collect();
    for (i, o) in owners.into_iter().enumerate() {
        shards[i % 4].push((i, o));
    }
    let tasks: Vec<_> = shards
        .into_iter()
        .enumerate()
        .map(|(shard, mut xs)| {
            thread::spawn(move || {
                let mut samples = vec![];
                let mut delivered = 0u64;
                let interval = if rate == 0 {
                    Duration::ZERO
                } else {
                    Duration::from_secs_f64(4f64 / rate as f64)
                };
                let mut due = start;
                let mut idx = 0;
                let candidates: Vec<usize> =
                    xs.iter().enumerate().filter(|(_, x)| x.0 < active).map(|(i, _)| i).collect();
                if !candidates.is_empty() {
                    while Instant::now() < deadline {
                        if Instant::now() < due {
                            thread::sleep(due - Instant::now());
                        }
                        if Instant::now() >= deadline {
                            break;
                        }
                        let o = &mut xs[candidates[idx % candidates.len()]].1;
                        let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed);
                        let n = o.roundtrip(size, seq);
                        samples.push(n);
                        delivered += 1;
                        idx += 1;
                        due += interval;
                    }
                }
                (xs, samples, delivered, shard)
            })
        })
        .collect();
    let mut joined = vec![];
    let mut latency = vec![];
    let mut delivered = 0;
    for t in tasks {
        let (xs, s, n, _) = t.join().unwrap();
        joined.extend(xs);
        latency.extend(s);
        delivered += n;
    }
    let wall = start.elapsed().as_secs_f64();
    let after = read("/proc/stat");
    if let Some(p) = perf.as_mut() {
        unsafe {
            libc::kill(p.id() as i32, libc::SIGINT);
        }
        let _ = p.wait();
    }
    latency.sort_unstable();
    let q = |p: f64| {
        latency.get(((latency.len().saturating_sub(1)) as f64 * p) as usize).copied().unwrap_or(0)
    };
    event(
        json!({"event":"window","id":id,"variant":variant,"protocol":protocol,"population":joined.len(),"active":active,"application_bytes_each_direction":size,"offered_target_roundtrips_per_s":rate,"unlimited_closed_loop":rate==0,"repeat":rep,"duration_s":wall,"delivered_roundtrips":delivered,"sent_each_direction":delivered,"delivered_each_direction":delivered,"lost":0,"corrupt":0,"boundary_violations":0,"application_bytes_delivered":delivered*size as u64*2,"latency_ns":latency,"p50_ns":q(0.5),"p95_ns":q(0.95),"p99_ns":q(0.99),"cpu_before":before,"cpu_after":after,"kernel_counters_before":before_backend,"kernel_counters_after":if variant=="kernel"{read("/proc/shmproxy")}else{String::new()},"raw_perf":read(&perfpath),"generator_concurrency":4,"no_payload_uart_logging":true}),
    );
    joined.sort_by_key(|x| x.0);
    joined.into_iter().map(|x| x.1).collect()
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let variant = &args[1];
    let protocol = &args[2];
    let max: usize = args[3].parse().unwrap();
    let kernel = variant == "kernel";
    let udp = protocol == "udp";
    let limit = libc::rlimit { rlim_cur: 262144, rlim_max: 262144 };
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) }, 0);
    let baseline = snapshot();
    event(
        json!({"event":"begin","variant":variant,"protocol":protocol,"maximum_population":max,"clock_ticks":unsafe{libc::sysconf(libc::_SC_CLK_TCK)},"baseline":baseline,"queue_size":8,"memory_per_peer":262144,"socketbuf_requested":262144,"cpus_online":read("/sys/devices/system/cpu/online"),"cpus_possible":read("/sys/devices/system/cpu/possible")}),
    );
    let mut b = Backend::new(kernel, udp);
    event(json!({"event":"loaded","snapshot":snapshot()}));
    let mut owners = vec![];
    for population in [1, 64, 1024, 4096, 16384].into_iter().filter(|p| *p <= max) {
        let setup = Instant::now();
        let previous = owners.len();
        while owners.len() < population {
            let i = owners.len();
            let mut o = Owner::new(&mut b, udp, i);
            o.roundtrip(64, i as u64);
            owners.push(o);
        }
        event(
            json!({"event":"setup","population":population,"new_owners":population-previous,"duration_s":setup.elapsed().as_secs_f64(),"completion_boundary":"device started, proxy registered, tagged two-direction application message completed"}),
        );
        audit(&mut owners, &b, "interval-start");
        if population == 1 {
            for size in if udp { vec![0, 0, 64, 0, 1431, 59000, 0] } else { vec![64, 1024, 262144] }
            {
                let n = owners[0].roundtrip(size, 999);
                event(
                    json!({"event":"correctness_size","protocol":protocol,"size":size,"latency_ns":n,"byte_exact":true,"whole_message":udp,"actual_nonzero_recv_buffer":65536}),
                );
            }
        }
        let idle_before = snapshot();
        thread::sleep(Duration::from_secs(2));
        event(
            json!({"event":"idle","population":population,"duration_s":2,"before":idle_before,"after":snapshot()}),
        );
        let sizes = if udp { vec![0, 64, 1431, 59000] } else { vec![64, 1024, 65536] };
        let activities = if population <= 64 { vec![population] } else { vec![64, population] };
        for active in activities {
            for size in &sizes {
                owners = window(owners, active, *size, 0, 0.15, 99, variant, protocol);
                for rep in 0..3 {
                    for rate in if rep % 2 == 0 { vec![100, 1000, 0] } else { vec![0, 1000, 100] } {
                        owners = window(owners, active, *size, rate, 1.0, rep, variant, protocol);
                    }
                }
            }
        }
        event(
            json!({"event":"traffic_resources","population":population,"snapshot":snapshot(),"backend":b.receipt()}),
        );
        audit(&mut owners, &b, "interval-end");
    }
    let retire = Instant::now();
    let count = owners.len();
    for o in owners {
        b.retire(o)
    }
    event(
        json!({"event":"owned_retirement","owners":count,"duration_s":retire.elapsed().as_secs_f64(),"backend":b.receipt(),"snapshot":snapshot()}),
    );
    drop(b);
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let modules = read("/proc/modules");
        let usage = modules
            .lines()
            .find(|x| x.starts_with("vhost_vsock "))
            .and_then(|x| x.split_whitespace().nth(2))
            .and_then(|x| x.parse::<usize>().ok());
        if usage == Some(0) || Instant::now() >= until {
            event(
                json!({"event":"transport_drain_live_kernel","usage":usage,"confirmed_zero":usage==Some(0),"retirement_total_s":retire.elapsed().as_secs_f64(),"modules":modules,"snapshot":snapshot()}),
            );
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    event(
        json!({"event":"complete","variant":variant,"protocol":protocol,"controller_proxy_payload_reads_writes":0}),
    );
}
