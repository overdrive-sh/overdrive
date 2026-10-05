//! Rust Aya controller never reads/writes either proxy-leg socket payload.
//! Synthetic guest virtqueues and ordinary network apps are traffic endpoints.
use aya::{
    maps::{Array, HashMap, PerCpuArray, SockHash},
    programs::{SkSkb,SchedClassifier,TcAttachType},
    EbpfLoader, VerifierLogLevel,
};
use serde_json::json;
use std::{
    fs,
    io::{self, Write},
    net::UdpSocket,
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
        if op==5 {event(json!({"event":"actual_vhost_guest_tx","cid":self.cid,"header":self.read(0x20000,44),"wire_len":b.len(),"prefix":&b[..b.len().min(16)],"tx_avail":self.tx_av,"tx_used":self.tx_used,"descriptor_head":self.read(0x4000,32),"used_head":self.read(0x5804+((self.tx_used-1)%N) as u64*8,8)}));}
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
        event(json!({"event":"actual_vhost_used_rx","cid":self.cid,"header":h,"descriptor_head":self.read(0,32),"used_head":self.read(0x1804+((self.rx_used-1)%N) as u64*8,8),"rx_avail":self.rx_av,"rx_used":self.rx_used}));
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
    let mut m = SockHash::<_,u32>::try_from(bpf.map_mut("SOCKETS").unwrap()).unwrap();
    for (k, fd) in [(key, a), (key + 1, b)] {
        if let Err(e) = m.insert(k, fd, 0) {
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
#[repr(C)]
#[derive(Clone,Copy)]
struct Tuple { src:u32,dst:u32,sport:u16,dport:u16 }
unsafe impl aya::Pod for Tuple {}
fn tuple(src:std::net::SocketAddr,dst:std::net::SocketAddr)->Tuple {
 let (std::net::SocketAddr::V4(src),std::net::SocketAddr::V4(dst))=(src,dst) else{panic!("v4 only")};
 Tuple{src:u32::from_ne_bytes(src.ip().octets()),dst:u32::from_ne_bytes(dst.ip().octets()),sport:src.port().to_be(),dport:dst.port().to_be()}
}
fn nocheck(fd:i32){let one=1i32;assert_eq!(unsafe{libc::setsockopt(fd,libc::SOL_SOCKET,libc::SO_NO_CHECK,&one as *const _ as *const _,4)},0)}
fn udp_bound(port:u16)->UdpSocket {
 unsafe {
 let fd=OwnedFd::from_raw_fd(libc::socket(libc::AF_INET,libc::SOCK_DGRAM,0));let one=1i32;
 assert_eq!(libc::setsockopt(fd.as_raw_fd(),libc::SOL_SOCKET,libc::SO_REUSEADDR,&one as *const _ as *const _,4),0);
 let a=libc::sockaddr_in{sin_family:libc::AF_INET as u16,sin_port:port.to_be(),sin_addr:libc::in_addr{s_addr:u32::from_ne_bytes([127,0,0,1])},sin_zero:[0;8]};
 assert_eq!(libc::bind(fd.as_raw_fd(),&a as *const _ as *const _,16),0);UdpSocket::from(fd)
 }
}
fn encode(b:&[u8])->Vec<u8>{let mut wire=b"ZUD1".to_vec();wire.extend((b.len() as u32).to_be_bytes());wire.extend(b);wire}
fn decode(wire:&[u8])->Vec<u8>{assert!(wire.len()>=8);let n=wire.len()-8;assert_eq!(&wire[..4],b"ZUD1");assert_eq!(u32::from_be_bytes(wire[4..8].try_into().unwrap()) as usize,n);wire[8..].to_vec()}
fn message(p:&mut Peer)->Vec<u8>{let mut all=vec![];loop{let (op,flags,data)=p.rx();event(json!({"event":"actual_vhost_rx","cid":p.cid,"op":op,"flags":flags,"wire_len":data.len(),"prefix":&data[..data.len().min(16)],"footer":&data[data.len().saturating_sub(8)..],"rx_avail":p.rx_av,"rx_used":p.rx_used}));if op==5{all.extend(data);if flags&1!=0{return all}}else{assert_eq!(op,6)}}}
#[repr(C)] #[derive(Clone,Copy)] struct Witness{seq:u64,len:u32,mode:u32,bytes:[u8;48]}
unsafe impl aya::Pod for Witness{}
fn packet_witnesses(bpf:&aya::Ebpf)->Vec<serde_json::Value>{let map=Array::<_,Witness>::try_from(bpf.map("WITNESS").unwrap()).unwrap();(0..2).map(|i|{let x=map.get(&i,0).unwrap();let h=&x.bytes;let ip_len=u16::from_be_bytes([h[16],h[17]]);let udp_len=u16::from_be_bytes([h[38],h[39]]);let mut sum=0u32;for pair in h[14..34].chunks(2){sum+=u16::from_be_bytes([pair[0],pair[1]]) as u32}while sum>65535{sum=(sum&65535)+(sum>>16)}assert_eq!(sum,65535,"IP checksum");json!({"seq":x.seq,"skb_len":x.len,"mode":x.mode,"raw_header":&h[..42],"ipv4_total_length":ip_len,"udp_length":udp_len,"ipv4_checksum_valid":sum==65535,"udp_checksum":u16::from_be_bytes([h[40],h[41]]),"source_ip":&h[26..30],"dest_ip":&h[30..34],"source_port":u16::from_be_bytes([h[34],h[35]]),"dest_port":u16::from_be_bytes([h[36],h[37]])})}).collect()}
fn no_message(p:&mut Peer,bound:Duration)->bool{let end=Instant::now()+bound;loop{let now=Instant::now();if now>=end{return true}match p.try_rx(end-now){None=>return true,Some((6,_,_))=>event(json!({"event":"negative_credit_control_only","cid":p.cid})),Some((op,flags,data))=>{event(json!({"event":"negative_unexpected_packet","cid":p.cid,"op":op,"flags":flags,"len":data.len()}));return false}}}}
fn ready(fd:i32)->i32{let mut poll=libc::pollfd{fd,events:libc::POLLIN,revents:0};let n=unsafe{libc::poll(&mut poll,1,1000)};assert_eq!(n,1,"UDP readiness timeout");assert_ne!(poll.revents&libc::POLLIN,0);poll.revents as i32}
fn main(){
 let initial=fs::read_dir("/proc/self/fd").unwrap().count();
 let mut bpf=EbpfLoader::new().verifier_log_level(VerifierLogLevel::VERBOSE|VerifierLogLevel::STATS).load_file("/program.o").unwrap();
 let mapfd=SockHash::<_,u32>::try_from(bpf.map("SOCKETS").unwrap()).unwrap().fd().try_clone().unwrap();
 let prog:&mut SkSkb=bpf.program_mut("route").unwrap().try_into().unwrap();prog.load().unwrap();let info=prog.info().unwrap();event(json!({"event":"bpf_loaded","name":"route","id":info.id(),"program_tag":format!("{:016x}",info.tag()),"verified_instructions":info.verified_instruction_count(),"jitted_bytes":info.size_jitted(),"map_ids":info.map_ids().unwrap()}));let route_link=prog.attach(&mapfd).unwrap();
 let tc:&mut SchedClassifier=bpf.program_mut("packet").unwrap().try_into().unwrap();tc.load().unwrap();let info=tc.info().unwrap();event(json!({"event":"bpf_loaded","name":"packet","id":info.id(),"program_tag":format!("{:016x}",info.tag()),"verified_instructions":info.verified_instruction_count(),"jitted_bytes":info.size_jitted(),"map_ids":info.map_ids().unwrap()}));let tc_link=tc.attach("lo",TcAttachType::Egress).unwrap();let (revision,progs)=SchedClassifier::query_tcx("lo",TcAttachType::Egress).unwrap();event(json!({"event":"tcx_effective_attachment","ifname":"lo","attach_type":"egress","revision":revision,"program_ids":progs.iter().map(|p|p.id()).collect::<Vec<_>>() }));
 let vl=listener(libc::SOCK_SEQPACKET);let mut owners=vec![];let mut server=0;
 for (i,addr) in [[127,0,0,2],[127,0,0,3]].into_iter().enumerate(){
 let app=UdpSocket::bind((std::net::Ipv4Addr::from(addr),0)).unwrap();let proxy=udp_bound(server);server=proxy.local_addr().unwrap().port();proxy.connect(app.local_addr().unwrap()).unwrap();app.connect(proxy.local_addr().unwrap()).unwrap();nocheck(proxy.as_raw_fd());nocheck(app.as_raw_fd());app.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
 Array::<_,u32>::try_from(bpf.map_mut("SERVER").unwrap()).unwrap().set(0,server.to_be() as u32,0).unwrap();
 let mut flows=HashMap::<_,Tuple,u32>::try_from(bpf.map_mut("FLOWS").unwrap()).unwrap();flows.insert(tuple(app.local_addr().unwrap(),proxy.local_addr().unwrap()),1,0).unwrap();flows.insert(tuple(proxy.local_addr().unwrap(),app.local_addr().unwrap()),2,0).unwrap();
 let mut p=Peer::new(295300+i as u32,4300+i as u32,2);p.connect();let vs=accept(&vl);assert!(register(&mut bpf,proxy.as_raw_fd(),vs.as_raw_fd(),i as u32*2));
 event(json!({"event":"owner","cid":p.cid,"real_vhost_fd":p.fd.as_raw_fd(),"memory":format!("{:p}",p.mem),"actual_peer":app.local_addr().unwrap().to_string(),"actual_proxy":proxy.local_addr().unwrap().to_string(),"no_udp_tx_checksum_socket_option":true}));owners.push((p,vs,proxy,app));
 }
 let mut sent_empty=0;let mut received_empty=0;
 for (p,_,proxy,app) in &mut owners{
  for (order,len) in [0usize,0,1,0,1431,59000,0].into_iter().enumerate(){
   let data:Vec<u8>=(0..len).map(|i| ((i+order+p.cid as usize)%251) as u8).collect();
   let before=counters(&bpf,21);let send=app.send(&data).unwrap();assert_eq!(send,len);let frame=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||message(p))).unwrap_or_else(|e|{event(json!({"event":"native_receive_failure","logical_len":len,"counters":counters(&bpf,21)}));std::panic::resume_unwind(e)});let got=decode(&frame);assert_eq!(got,data);if len==0{sent_empty+=1}
   let reverse=encode(&data);p.tx(5,1,&reverse);let readiness=ready(app.as_raw_fd());let mut buf=vec![0;65536];let (n,source)=app.recv_from(&mut buf).unwrap();assert_eq!(n,len);assert_eq!(&buf[..n],data);assert_eq!(source,proxy.local_addr().unwrap());if len==0{received_empty+=1}
   event(json!({"event":"udp_datagram_roundtrip","cid":p.cid,"order":order,"logical_len":len,"send_return":send,"actual_vsock_wire_len":frame.len(),"private_header":&frame[..8],"packet_witnesses":packet_witnesses(&bpf),"reverse_tx_avail":p.tx_av,"reverse_tx_used":p.tx_used,"recv_from_return":n,"readiness_revents":readiness,"recv_source":source.to_string(),"actual_peer":app.local_addr().unwrap().to_string(),"counter_before":before,"counter_after":counters(&bpf,21)}));
  }
  let collision=b"ZUD1\0\0\0\0".to_vec();app.send(&collision).unwrap();let actual=message(p);assert_eq!(decode(&actual),collision);p.tx(5,1,&encode(&collision));let readiness=ready(app.as_raw_fd());let mut buf=[0;32];let (n,source)=app.recv_from(&mut buf).unwrap();assert_eq!(&buf[..n],collision);event(json!({"event":"marker_collision_preserved","cid":p.cid,"len":n,"actual_wire_prefix":&actual[..16],"readiness_revents":readiness,"source":source.to_string()}));
  // A final readable-state check distinguishes zero-length messages from EOF/no data.
  let mut poll=libc::pollfd{fd:app.as_raw_fd(),events:libc::POLLIN,revents:0};assert_eq!(unsafe{libc::poll(&mut poll,1,50)},0);event(json!({"event":"no_ghost_datagram","cid":p.cid}));
 }
 let foreign=UdpSocket::bind("127.0.0.4:0").unwrap();let foreign_server=UdpSocket::bind("127.0.0.1:0").unwrap();foreign.send_to(&[],foreign_server.local_addr().unwrap()).unwrap();let revents=ready(foreign_server.as_raw_fd());let mut byte=[0];let (n,src)=foreign_server.recv_from(&mut byte).unwrap();assert_eq!(n,0);assert_eq!(src,foreign.local_addr().unwrap());event(json!({"event":"foreign_udp_unmodified","recv_from_return":n,"readiness_revents":revents,"source":src.to_string(),"destination":foreign_server.local_addr().unwrap().to_string()}));drop(foreign);drop(foreign_server);
 event(json!({"event":"zero_message_counts","ordinary_app_empty_sends":sent_empty,"real_guest_empty_decodes":sent_empty,"ordinary_app_empty_receives":received_empty,"expected_each":8}));assert_eq!(sent_empty,8);assert_eq!(received_empty,8);
 let (p,_,proxy,app)=&mut owners[0];
 for (name,wire) in [("bad_magic",b"BAD!\0\0\0\0".to_vec()),("length_mismatch",b"ZUD1\0\0\0\x01".to_vec())]{p.tx(5,1,&wire);let mut poll=libc::pollfd{fd:app.as_raw_fd(),events:libc::POLLIN,revents:0};assert_eq!(unsafe{libc::poll(&mut poll,1,100)},0);event(json!({"event":"invalid_wire_fail_closed","case":name,"counters":counters(&bpf,21)}));}
 HashMap::<_,Tuple,u32>::try_from(bpf.map_mut("FLOWS").unwrap()).unwrap().remove(&tuple(proxy.local_addr().unwrap(),app.local_addr().unwrap())).unwrap();p.tx(5,1,&encode(&[]));let mut poll=libc::pollfd{fd:app.as_raw_fd(),events:libc::POLLIN,revents:0};assert_eq!(unsafe{libc::poll(&mut poll,1,100)},0);event(json!({"event":"removed_mapping_fail_closed","direction":"guest_to_host","counters":counters(&bpf,21)}));
 // Remove both socket routes before app→guest probe. Socket map entries still remain owned, so no fallback is possible.
 HashMap::<_,u64,u32>::try_from(bpf.map_mut("ROUTES").unwrap()).unwrap().remove(&cookie(proxy.as_raw_fd())).unwrap();app.send(&[]).unwrap();assert!(no_message(p,Duration::from_millis(150)));event(json!({"event":"removed_mapping_fail_closed","direction":"host_to_guest","counters":counters(&bpf,21)}));
 for (i,(mut p,vs,proxy,app)) in owners.into_iter().enumerate(){let mut map=SockHash::<_,u32>::try_from(bpf.map_mut("SOCKETS").unwrap()).unwrap();map.remove(&(i as u32*2)).unwrap();map.remove(&(i as u32*2+1)).unwrap();p.tx(3,0,&[]);drop(vs);drop(proxy);drop(app);drop(p);}
 event(json!({"event":"final_counters","values":counters(&bpf,21)}));let occupied=SockHash::<_,u32>::try_from(bpf.map("SOCKETS").unwrap()).unwrap().keys().count();assert_eq!(occupied,0);event(json!({"event":"owned_sockhash_removed","occupied_keys":occupied}));let tc:&mut SchedClassifier=bpf.program_mut("packet").unwrap().try_into().unwrap();tc.detach(tc_link).unwrap();let (revision,progs)=SchedClassifier::query_tcx("lo",TcAttachType::Egress).unwrap();assert!(progs.is_empty());event(json!({"event":"tcx_detached_readback","revision":revision,"program_ids":progs.iter().map(|p|p.id()).collect::<Vec<_>>() }));let prog:&mut SkSkb=bpf.program_mut("route").unwrap().try_into().unwrap();prog.detach(route_link).unwrap();drop(vl);drop(mapfd);drop(bpf);let final_fds=fs::read_dir("/proc/self/fd").unwrap().count();assert_eq!(final_fds,initial);event(json!({"event":"complete","initial_fds":initial,"final_fds":final_fds,"controller_payload_reads_writes":0}));
}
