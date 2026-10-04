//! Rust Aya controller never reads/writes either proxy-leg socket payload.
//! Synthetic guest virtqueues and ordinary network apps are traffic endpoints.
use aya::{
    maps::{Array, HashMap, PerCpuArray, SockMap},
    programs::SkSkb,
    EbpfLoader, VerifierLogLevel,
};
use serde_json::json;
use std::{
    fs,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream, UdpSocket},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::atomic::{fence, Ordering},
    thread,
    time::{Duration, Instant},
};
const SIZE: usize = 256 * 1024;
const N: u16 = 8;
const BUF: u64 = 0x8000;
fn ioctl<T>(fd: i32, req: u64, p: &T) {
    let rc = unsafe { libc::ioctl(fd, req as libc::c_ulong, p as *const T) };
    assert_eq!(rc, 0, "ioctl {req:x}: {}", io::Error::last_os_error());
}
#[repr(C)]
struct State {
    index: u32,
    num: u32,
}
#[repr(C)]
struct Addr {
    index: u32,
    flags: u32,
    desc: u64,
    used: u64,
    avail: u64,
    log: u64,
}
#[repr(C)]
struct VFile {
    index: u32,
    fd: i32,
}
#[repr(C)]
struct Memory {
    n: u32,
    pad: u32,
    gpa: u64,
    size: u64,
    user: u64,
    flags: u64,
}
struct Peer {
    typ: u16,
    fd: OwnedFd,
    mem: *mut u8,
    kicks: Vec<OwnedFd>,
    _calls: Vec<OwnedFd>,
    cid: u32,
    port: u32,
    rx_av: u16,
    rx_used: u16,
    tx_av: u16,
    tx_used: u16,
    fwd: u32,
    packets: u64,
}
impl Peer {
    fn new(cid: u32, port: u32, typ: u16) -> Self {
        let fd = fs::OpenOptions::new().read(true).write(true).open("/dev/vhost-vsock").unwrap();
        let fd: OwnedFd = fd.into();
        let mem = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                SIZE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        } as *mut u8;
        assert_ne!(mem as isize, -1);
        let mut offered = 0u64;
        assert_eq!(
            unsafe { libc::ioctl(fd.as_raw_fd(), 0x8008af00u64 as libc::c_ulong, &mut offered) },
            0
        );
        event(
            json!({"event":"vhost_feature_readback","cid":cid,"offered_hex":format!("{:016x}",offered),"seqpacket_offered":offered&(1<<1)!=0,"negotiated_hex":"0000000100000002"}),
        );
        ioctl(fd.as_raw_fd(), 0xaf01, &0u64);
        ioctl(fd.as_raw_fd(), 0x4008af00, &((1u64 << 32) | (1u64 << 1)));
        ioctl(
            fd.as_raw_fd(),
            0x4008af03,
            &Memory { n: 1, pad: 0, gpa: 0, size: SIZE as u64, user: mem as u64, flags: 0 },
        );
        let mut kicks = vec![];
        let mut calls = vec![];
        for q in 0..2u32 {
            ioctl(fd.as_raw_fd(), 0x4008af10, &State { index: q, num: N as u32 });
            ioctl(fd.as_raw_fd(), 0x4008af12, &State { index: q, num: 0 });
            let b = q as u64 * 0x4000;
            ioctl(
                fd.as_raw_fd(),
                0x4028af11,
                &Addr {
                    index: q,
                    flags: 0,
                    desc: mem as u64 + b,
                    avail: mem as u64 + b + 0x1000,
                    used: mem as u64 + b + 0x1800,
                    log: 0,
                },
            );
            let kick = unsafe { OwnedFd::from_raw_fd(libc::eventfd(0, libc::EFD_NONBLOCK)) };
            let call = unsafe { OwnedFd::from_raw_fd(libc::eventfd(0, libc::EFD_NONBLOCK)) };
            ioctl(fd.as_raw_fd(), 0x4008af20, &VFile { index: q, fd: kick.as_raw_fd() });
            ioctl(fd.as_raw_fd(), 0x4008af21, &VFile { index: q, fd: call.as_raw_fd() });
            kicks.push(kick);
            calls.push(call);
        }
        ioctl(fd.as_raw_fd(), 0x4008af60, &(cid as u64));
        ioctl(fd.as_raw_fd(), 0x4004af61, &1i32);
        let p = Self {
            typ,
            fd,
            mem,
            kicks,
            _calls: calls,
            cid,
            port,
            rx_av: 0,
            rx_used: 0,
            tx_av: 0,
            tx_used: 0,
            fwd: 0,
            packets: 0,
        };
        p.desc(0, BUF, 44, 3, 1);
        p.desc(16, BUF + 0x100, 65536, 2, 0);
        p
    }
    fn write(&self, a: u64, b: &[u8]) {
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), self.mem.add(a as usize), b.len()) }
    }
    fn read(&self, a: u64, n: usize) -> Vec<u8> {
        unsafe { std::slice::from_raw_parts(self.mem.add(a as usize), n).to_vec() }
    }
    fn w16(&self, a: u64, n: u16) {
        unsafe { std::ptr::write_volatile(self.mem.add(a as usize) as *mut u16, n.to_le()) }
    }
    fn r16(&self, a: u64) -> u16 {
        unsafe { u16::from_le(std::ptr::read_volatile(self.mem.add(a as usize) as *const u16)) }
    }
    fn desc(&self, a: u64, addr: u64, len: u32, flags: u16, next: u16) {
        let mut b = vec![];
        b.extend(addr.to_le_bytes());
        b.extend(len.to_le_bytes());
        b.extend(flags.to_le_bytes());
        b.extend(next.to_le_bytes());
        self.write(a, &b)
    }
    fn kick(&self, q: usize) {
        let n = 1u64;
        assert_eq!(
            unsafe { libc::write(self.kicks[q].as_raw_fd(), &n as *const u64 as *const _, 8) },
            8
        )
    }
    fn tx(&mut self, op: u16, flags: u32, b: &[u8]) {
        let mut h = vec![];
        h.extend((self.cid as u64).to_le_bytes());
        h.extend(2u64.to_le_bytes());
        h.extend(self.port.to_le_bytes());
        h.extend(29500u32.to_le_bytes());
        h.extend((b.len() as u32).to_le_bytes());
        h.extend(self.typ.to_le_bytes());
        h.extend(op.to_le_bytes());
        h.extend(flags.to_le_bytes());
        h.extend(65536u32.to_le_bytes());
        h.extend(self.fwd.to_le_bytes());
        self.write(0x20000, &h);
        self.write(0x20100, b);
        self.desc(0x4000, 0x20000, 44, 1, 1);
        self.desc(0x4010, 0x20100, b.len() as u32, 0, 0);
        self.w16(0x5004 + (self.tx_av % N) as u64 * 2, 0);
        self.tx_av = self.tx_av.wrapping_add(1);
        fence(Ordering::SeqCst);
        self.w16(0x5002, self.tx_av);
        self.kick(1);
        let until = Instant::now() + Duration::from_secs(4);
        while self.r16(0x5802) == self.tx_used {
            assert!(Instant::now() < until, "TX timeout CID {}", self.cid);
            thread::sleep(Duration::from_micros(100));
        }
        fence(Ordering::SeqCst);
        self.tx_used = self.tx_used.wrapping_add(1);
    }
    fn try_rx(&mut self, timeout: Duration) -> Option<(u16, u32, Vec<u8>)> {
        self.w16(0x1004 + (self.rx_av % N) as u64 * 2, 0);
        self.rx_av = self.rx_av.wrapping_add(1);
        fence(Ordering::SeqCst);
        self.w16(0x1002, self.rx_av);
        self.kick(0);
        let until = Instant::now() + timeout;
        while self.r16(0x1802) == self.rx_used {
            if Instant::now() >= until {
                return None;
            }
            thread::sleep(Duration::from_micros(100));
        }
        fence(Ordering::SeqCst);
        self.rx_used = self.rx_used.wrapping_add(1);
        let h = self.read(BUF, 44);
        assert_eq!(u64::from_le_bytes(h[8..16].try_into().unwrap()), self.cid as u64);
        assert_eq!(u32::from_le_bytes(h[20..24].try_into().unwrap()), self.port);
        let n = u32::from_le_bytes(h[24..28].try_into().unwrap()) as usize;
        assert!(n <= 65536);
        let op = u16::from_le_bytes(h[30..32].try_into().unwrap());
        let flags = u32::from_le_bytes(h[32..36].try_into().unwrap());
        let b = self.read(BUF + 0x100, n);
        self.fwd = self.fwd.wrapping_add(n as u32);
        self.packets += 1;
        if n > 0 {
            self.tx(6, 0, &[]);
        }
        Some((op, flags, b))
    }
    fn rx(&mut self) -> (u16, u32, Vec<u8>) {
        self.try_rx(Duration::from_secs(4)).expect("RX timed out")
    }
    fn connect(&mut self) {
        self.tx(1, 0, &[]);
        loop {
            let (op, _, b) = self.rx();
            if op == 2 {
                assert!(b.is_empty());
                break;
            }
            assert_ne!(op, 3, "connect rejected")
        }
    }
    fn bytes(&mut self, b: &[u8], split: usize) {
        for c in b.chunks(split) {
            self.tx(5, 0, c)
        }
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        ioctl(self.fd.as_raw_fd(), 0x4004af61, &0i32);
        unsafe {
            libc::munmap(self.mem as *mut _, SIZE);
        }
    }
}
fn event(v: serde_json::Value) {
    println!("{v}");
    io::stdout().flush().unwrap()
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
        assert!(f.as_raw_fd() >= 0);
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
            "bind {}",
            io::Error::last_os_error()
        );
        assert_eq!(libc::listen(f.as_raw_fd(), 32), 0);
        f
    }
}
fn accept(f: &OwnedFd) -> OwnedFd {
    unsafe {
        let n = libc::accept(f.as_raw_fd(), std::ptr::null_mut(), std::ptr::null_mut());
        assert!(n >= 0, "accept {}", io::Error::last_os_error());
        OwnedFd::from_raw_fd(n)
    }
}
fn cookie(fd: i32) -> u64 {
    let mut n = 0u64;
    let mut len = 8u32;
    assert_eq!(
        unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, 57, &mut n as *mut _ as *mut _, &mut len) },
        0
    );
    n
}
fn counters(bpf: &aya::Ebpf, n: u32) -> Vec<u64> {
    let a = PerCpuArray::<_, u64>::try_from(bpf.map("COUNTERS").unwrap()).unwrap();
    (0..n).map(|i| a.get(&i, 0).unwrap().iter().sum()).collect()
}
fn register(bpf: &mut aya::Ebpf, a: i32, b: i32, key: u32) -> bool {
    let mut m = SockMap::try_from(bpf.map_mut("SOCKETS").unwrap()).unwrap();
    for (k, fd) in [(key, a), (key + 1, b)] {
        if let Err(e) = m.set(k, &fd, 0) {
            event(json!({"event":"map_insert_failed","key":k,"fd":fd,"error":format!("{e:?}")}));
            return false;
        }
    }
    let mut r = HashMap::<_, u64, u32>::try_from(bpf.map_mut("ROUTES").unwrap()).unwrap();
    r.insert(cookie(a), key + 1, 0).unwrap();
    r.insert(cookie(b), key, 0).unwrap();
    event(
        json!({"event":"map_pair_registered","keys":[key,key+1],"fds":[a,b],"cookies":[cookie(a),cookie(b)]}),
    );
    true
}
fn receive(p: &mut Peer, n: usize) -> Vec<u8> {
    let mut all = vec![];
    while all.len() < n {
        let (op, flags, b) = p.rx();
        event(json!({"event":"wire_rx","cid":p.cid,"op":op,"flags":flags,"len":b.len()}));
        if op == 5 {
            all.extend(b)
        } else {
            assert_ne!(op, 3)
        }
    }
    all
}
fn failures(bpf: &aya::Ebpf) -> Vec<u64> {
    let a = PerCpuArray::<_, u64>::try_from(bpf.map("FAILURES").unwrap()).unwrap();
    (0..4).map(|i| a.get(&i, 0).unwrap().iter().sum()).collect()
}
fn udp_bound(port: u16) -> UdpSocket {
    unsafe {
        let fd = OwnedFd::from_raw_fd(libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0));
        let one = 1i32;
        assert_eq!(
            libc::setsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_REUSEADDR,
                &one as *const _ as *const _,
                4
            ),
            0
        );
        let a = libc::sockaddr_in {
            sin_family: libc::AF_INET as u16,
            sin_port: port.to_be(),
            sin_addr: libc::in_addr { s_addr: u32::from_ne_bytes([127, 0, 0, 1]) },
            sin_zero: [0; 8],
        };
        assert_eq!(
            libc::bind(fd.as_raw_fd(), &a as *const _ as *const _, 16),
            0,
            "bind {}",
            io::Error::last_os_error()
        );
        UdpSocket::from(fd)
    }
}
fn message(p: &mut Peer) -> Vec<u8> {
    let mut all = vec![];
    loop {
        let (op, flags, data) = p.rx();
        if op == 5 {
            all.extend(data);
            if flags & 1 != 0 {
                return all;
            }
        } else {
            assert_eq!(op, 6, "unexpected non-credit control packet");
            event(json!({"event":"message_control_credit","cid":p.cid,"op":op}));
        }
    }
}
fn main() {
    let initial = fs::read_dir("/proc/self/fd").unwrap().count();
    let mut bpf = EbpfLoader::new()
        .verifier_log_level(VerifierLogLevel::VERBOSE | VerifierLogLevel::STATS)
        .load_file("/program.o")
        .unwrap();
    let mapfd = SockMap::try_from(bpf.map("SOCKETS").unwrap()).unwrap().fd().try_clone().unwrap();
    let prog: &mut SkSkb = bpf.program_mut("route").unwrap().try_into().unwrap();
    prog.load().unwrap();
    let info = prog.info().unwrap();
    event(
        json!({"event":"bpf_loaded","program_tag":format!("{:016x}",info.tag()),"verified_instructions":info.verified_instruction_count(),"map_ids":info.map_ids().unwrap()}),
    );
    prog.attach(&mapfd).unwrap();
    let vl = listener(libc::SOCK_SEQPACKET);
    let mut owners = vec![];
    let mut server = 0;
    for (i, addr) in [[127, 0, 0, 2], [127, 0, 0, 3]].into_iter().enumerate() {
        let app = UdpSocket::bind((std::net::Ipv4Addr::from(addr), 0)).unwrap();
        let proxy = udp_bound(server);
        server = proxy.local_addr().unwrap().port();
        proxy.connect(app.local_addr().unwrap()).unwrap();
        app.connect(proxy.local_addr().unwrap()).unwrap();
        app.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let mut p = Peer::new(295200 + i as u32, 4200 + i as u32, 2);
        p.connect();
        let vs = accept(&vl);
        assert!(register(&mut bpf, proxy.as_raw_fd(), vs.as_raw_fd(), i as u32 * 2));
        owners.push((p, vs, proxy, app));
    }
    for (p, _, _, app) in &mut owners {
        let data = p.cid.to_le_bytes();
        app.send(&data).unwrap();
        let got = message(p);
        assert_eq!(got, data.to_vec());
        p.tx(5, 1, &data);
        let mut reply = [0; 32];
        let (n, a) = app.recv_from(&mut reply).unwrap();
        assert_eq!(n, 4);
        assert_eq!(&reply[..n], data);
        assert_eq!(a.port(), server);
        event(
            json!({"event":"udp_same_server_peer_association_pass","server_port":server,"actual_peer":app.local_addr().unwrap().to_string(),"cid":p.cid,"tag_bytes":data}),
        );
    }
    // Large frames are linearized by the BPF helper. Test endpoint sends one complete virtio seqpacket message.
    let (p, _, _, app) = &mut owners[0];
    let data: Vec<u8> = (0..59000).map(|i| (i % 251) as u8).collect();
    app.send(&data).unwrap();
    let mut flags = vec![];
    let mut lens = vec![];
    let mut got = vec![];
    while got.len() < data.len() {
        let (op, f, x) = p.rx();
        if op == 5 {
            flags.push(f);
            lens.push(x.len());
            got.extend(x)
        }
    }
    assert_eq!(got, data);
    p.tx(5, 1, &data);
    let mut reply = vec![0; 65536];
    let n = app.recv(&mut reply).unwrap();
    assert_eq!(&reply[..n], data);
    event(
        json!({"event":"udp_linearized_large_pass","input_len":59000,"forward_vsock_packet_lengths":lens,"forward_flags":flags,"reverse_udp_len":n,"reverse_one_datagram":n==59000,"flags_eom_last_only":flags.iter().take(flags.len()-1).all(|x|x&1==0)&&flags.last().unwrap()&1!=0}),
    );
    app.send(&vec![37; 1431]).unwrap();
    let x = message(p);
    assert_eq!(x.len(), 1431);
    p.tx(5, 1, &x);
    let mut tiny = [0; 8];
    let n = app.recv(&mut tiny).unwrap();
    assert_eq!(n, 8);
    assert_eq!(tiny, [37; 8]);
    app.send(b"after-truncation").unwrap();
    let x = message(p);
    p.tx(5, 1, &x);
    let n = app.recv(&mut reply).unwrap();
    assert_eq!(&reply[..n], b"after-truncation");
    event(
        json!({"event":"udp_endpoint_truncation_pass","input_len":1431,"receive_buffer":8,"next_datagram_preserved":true}),
    );
    // The final owner has no outstanding timed-out RX descriptor before each independent negative case.
    for mode in 0..3u32 {
        let app = UdpSocket::bind("127.0.0.2:0").unwrap();
        let proxy = udp_bound(0);
        proxy.connect(app.local_addr().unwrap()).unwrap();
        app.connect(proxy.local_addr().unwrap()).unwrap();
        let mut p = Peer::new(295210 + mode, 4210 + mode, 2);
        p.connect();
        let vs = accept(&vl);
        assert!(register(&mut bpf, proxy.as_raw_fd(), vs.as_raw_fd(), 4));
        if mode == 0 {
            HashMap::<_, u64, u32>::try_from(bpf.map_mut("ROUTES").unwrap())
                .unwrap()
                .remove(&cookie(proxy.as_raw_fd()))
                .unwrap()
        }
        if mode == 1 {
            HashMap::<_, u64, u32>::try_from(bpf.map_mut("ROUTES").unwrap())
                .unwrap()
                .insert(cookie(proxy.as_raw_fd()), 99, 0)
                .unwrap()
        }
        if mode == 2 {
            Array::<_, u64>::try_from(bpf.map_mut("FLAGS").unwrap()).unwrap().set(0, 1, 0).unwrap()
        }
        app.send(b"negative-owned-path").unwrap();
        let delivery = p.try_rx(Duration::from_millis(500));
        assert!(delivery.is_none());
        event(
            json!({"event":"negative_fail_closed_pass","mode":(["missing_source_route","missing_target_map_entry","vsock_target_ingress_flag"][mode as usize]),"delivered":false,"failures":failures(&bpf)}),
        );
        Array::<_, u64>::try_from(bpf.map_mut("FLAGS").unwrap()).unwrap().set(0, 0, 0).unwrap();
        let mut map = SockMap::try_from(bpf.map_mut("SOCKETS").unwrap()).unwrap();
        map.clear_index(&4).unwrap();
        map.clear_index(&5).unwrap();
        p.tx(3, 0, &[]);
        drop(vs);
        drop(proxy);
        drop(app);
        drop(p);
    }
    // Repeat zero through the exact linearized BPF object, with its own owner after a bounded no-delivery wait.
    let zero_app = UdpSocket::bind("127.0.0.3:0").unwrap();
    let zero_proxy = udp_bound(0);
    zero_proxy.connect(zero_app.local_addr().unwrap()).unwrap();
    zero_app.connect(zero_proxy.local_addr().unwrap()).unwrap();
    zero_app.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
    let mut zero_peer = Peer::new(295215, 4215, 2);
    zero_peer.connect();
    let zero_vs = accept(&vl);
    assert!(register(&mut bpf, zero_proxy.as_raw_fd(), zero_vs.as_raw_fd(), 4));
    let before = counters(&bpf, 6);
    zero_app.send(&[]).unwrap();
    let forward = zero_peer.try_rx(Duration::from_millis(500));
    zero_peer.tx(5, 1, &[]);
    let mut zero_buf = [0u8; 1];
    let reverse = zero_app.recv(&mut zero_buf);
    event(
        json!({"event":"linearized_bpf_zero_datagram_probe","forward_wire":forward.as_ref().map(|(op,flags,data)|json!({"op":op,"flags":flags,"len":data.len()})),"reverse_udp_receive":reverse.as_ref().ok(),"reverse_errno":reverse.err().and_then(|e|e.raw_os_error()),"counter_before":before,"counter_after":counters(&bpf,6),"failure_counters":failures(&bpf)}),
    );
    let mut zero_map = SockMap::try_from(bpf.map_mut("SOCKETS").unwrap()).unwrap();
    zero_map.clear_index(&4).unwrap();
    zero_map.clear_index(&5).unwrap();
    zero_peer.tx(3, 0, &[]);
    drop(zero_vs);
    drop(zero_proxy);
    drop(zero_app);
    drop(zero_peer);
    // Independent ordinary vsock endpoint baseline: this socket is never a proxy leg or in a BPF map.
    let mut baseline_peer = Peer::new(295220, 4220, 2);
    baseline_peer.connect();
    let baseline_endpoint = accept(&vl);
    let zero_send = unsafe { libc::send(baseline_endpoint.as_raw_fd(), std::ptr::null(), 0, 0) };
    let zero_wire = baseline_peer.try_rx(Duration::from_millis(500));
    event(
        json!({"event":"native_seqpacket_zero_endpoint_baseline","send_return":zero_send,"zero_wire_packet":zero_wire.as_ref().map(|(op,flags,payload)|json!({"op":op,"flags":flags,"len":payload.len()})),"socket_in_bpf_map":false}),
    );
    baseline_peer.tx(5, 1, &[]);
    thread::sleep(Duration::from_millis(50));
    let mut zero_receive_buf = [0u8; 1];
    let zero_receive = unsafe {
        libc::recv(
            baseline_endpoint.as_raw_fd(),
            zero_receive_buf.as_mut_ptr() as *mut _,
            1,
            libc::MSG_DONTWAIT,
        )
    };
    event(
        json!({"event":"native_seqpacket_zero_receive_baseline","receive_return":zero_receive,"errno":if zero_receive<0{io::Error::last_os_error().raw_os_error()}else{None},"socket_in_bpf_map":false}),
    );
    baseline_peer.tx(3, 0, &[]);
    drop(baseline_endpoint);
    drop(baseline_peer);
    let dgram = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_DGRAM, 0) };
    event(
        json!({"event":"vsock_dgram_socket_native","fd":dgram,"errno":if dgram<0{io::Error::last_os_error().raw_os_error()}else{None},"stock_vhost_dgram_route_proven":false}),
    );
    if dgram >= 0 {
        unsafe {
            libc::close(dgram);
        }
    }
    for (i, (mut p, vs, proxy, app)) in owners.into_iter().enumerate() {
        let mut map = SockMap::try_from(bpf.map_mut("SOCKETS").unwrap()).unwrap();
        map.clear_index(&(i as u32 * 2)).unwrap();
        map.clear_index(&(i as u32 * 2 + 1)).unwrap();
        p.tx(3, 0, &[]);
        drop(vs);
        drop(proxy);
        drop(app);
        drop(p);
    }
    drop(vl);
    drop(mapfd);
    drop(bpf);
    let final_fds = fs::read_dir("/proc/self/fd").unwrap().count();
    assert_eq!(final_fds, initial);
    event(
        json!({"event":"complete","initial_fds":initial,"final_fds":final_fds,"controller_payload_reads_writes":0}),
    );
}
