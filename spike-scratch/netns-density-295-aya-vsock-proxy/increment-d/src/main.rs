//! Rust Aya controller never reads/writes either proxy-leg socket payload.
//! Synthetic guest virtqueues and ordinary network apps are traffic endpoints.
use std::{fs,io::{self,Read,Write},net::{TcpListener,TcpStream,UdpSocket},os::fd::{AsRawFd,FromRawFd,OwnedFd},sync::atomic::{fence,Ordering},thread,time::{Duration,Instant}};
use aya::{EbpfLoader,VerifierLogLevel,maps::{SockMap,HashMap,PerCpuArray},programs::SkSkb};
use serde_json::json;
const SIZE:usize=256*1024; const N:u16=8; const BUF:u64=0x8000;
fn ioctl<T>(fd:i32,req:u64,p:&T){ let rc=unsafe{libc::ioctl(fd,req as libc::c_ulong,p as *const T)};assert_eq!(rc,0,"ioctl {req:x}: {}",io::Error::last_os_error()); }
#[repr(C)] struct State{index:u32,num:u32}
#[repr(C)] struct Addr{index:u32,flags:u32,desc:u64,used:u64,avail:u64,log:u64}
#[repr(C)] struct VFile{index:u32,fd:i32}
#[repr(C)] struct Memory{n:u32,pad:u32,gpa:u64,size:u64,user:u64,flags:u64}
struct Peer{typ:u16,fd:OwnedFd,mem:*mut u8,kicks:Vec<OwnedFd>,_calls:Vec<OwnedFd>,cid:u32,port:u32,rx_av:u16,rx_used:u16,tx_av:u16,tx_used:u16,fwd:u32,packets:u64}
impl Peer{
 fn new(cid:u32,port:u32,typ:u16)->Self{
  let fd=fs::OpenOptions::new().read(true).write(true).open("/dev/vhost-vsock").unwrap();let fd:OwnedFd=fd.into();
  let mem=unsafe{libc::mmap(std::ptr::null_mut(),SIZE,libc::PROT_READ|libc::PROT_WRITE,libc::MAP_PRIVATE|libc::MAP_ANONYMOUS,-1,0)} as *mut u8; assert_ne!(mem as isize,-1);
  ioctl(fd.as_raw_fd(),0xaf01,&0u64);ioctl(fd.as_raw_fd(),0x4008af00,&((1u64<<32)|(1u64<<1)));
  ioctl(fd.as_raw_fd(),0x4008af03,&Memory{n:1,pad:0,gpa:0,size:SIZE as u64,user:mem as u64,flags:0});
  let mut kicks=vec![];let mut calls=vec![];
  for q in 0..2u32{
   ioctl(fd.as_raw_fd(),0x4008af10,&State{index:q,num:N as u32});ioctl(fd.as_raw_fd(),0x4008af12,&State{index:q,num:0});let b=q as u64*0x4000;
   ioctl(fd.as_raw_fd(),0x4028af11,&Addr{index:q,flags:0,desc:mem as u64+b,avail:mem as u64+b+0x1000,used:mem as u64+b+0x1800,log:0});
   let kick=unsafe{OwnedFd::from_raw_fd(libc::eventfd(0,libc::EFD_NONBLOCK))};let call=unsafe{OwnedFd::from_raw_fd(libc::eventfd(0,libc::EFD_NONBLOCK))};
   ioctl(fd.as_raw_fd(),0x4008af20,&VFile{index:q,fd:kick.as_raw_fd()});ioctl(fd.as_raw_fd(),0x4008af21,&VFile{index:q,fd:call.as_raw_fd()});kicks.push(kick);calls.push(call);
  }
  ioctl(fd.as_raw_fd(),0x4008af60,&(cid as u64));ioctl(fd.as_raw_fd(),0x4004af61,&1i32);
  let p=Self{typ,fd,mem,kicks,_calls:calls,cid,port,rx_av:0,rx_used:0,tx_av:0,tx_used:0,fwd:0,packets:0};
  p.desc(0,BUF,44,3,1);p.desc(16,BUF+0x100,65536,2,0);p
 }
 fn write(&self,a:u64,b:&[u8]){unsafe{std::ptr::copy_nonoverlapping(b.as_ptr(),self.mem.add(a as usize),b.len())}}
 fn read(&self,a:u64,n:usize)->Vec<u8>{unsafe{std::slice::from_raw_parts(self.mem.add(a as usize),n).to_vec()}}
 fn w16(&self,a:u64,n:u16){unsafe{std::ptr::write_volatile(self.mem.add(a as usize) as *mut u16,n.to_le())}}
 fn r16(&self,a:u64)->u16{unsafe{u16::from_le(std::ptr::read_volatile(self.mem.add(a as usize) as *const u16))}}
 fn desc(&self,a:u64,addr:u64,len:u32,flags:u16,next:u16){let mut b=vec![];b.extend(addr.to_le_bytes());b.extend(len.to_le_bytes());b.extend(flags.to_le_bytes());b.extend(next.to_le_bytes());self.write(a,&b)}
 fn kick(&self,q:usize){let n=1u64;assert_eq!(unsafe{libc::write(self.kicks[q].as_raw_fd(),&n as *const u64 as *const _,8)},8)}
 fn tx(&mut self,op:u16,flags:u32,b:&[u8]){
  let mut h=vec![];h.extend((self.cid as u64).to_le_bytes());h.extend(2u64.to_le_bytes());h.extend(self.port.to_le_bytes());h.extend(29500u32.to_le_bytes());h.extend((b.len() as u32).to_le_bytes());h.extend(self.typ.to_le_bytes());h.extend(op.to_le_bytes());h.extend(flags.to_le_bytes());h.extend(65536u32.to_le_bytes());h.extend(self.fwd.to_le_bytes());
  self.write(0x20000,&h);self.write(0x20100,b);self.desc(0x4000,0x20000,44,1,1);self.desc(0x4010,0x20100,b.len() as u32,0,0);
  self.w16(0x5004+(self.tx_av%N) as u64*2,0);self.tx_av=self.tx_av.wrapping_add(1);fence(Ordering::SeqCst);self.w16(0x5002,self.tx_av);self.kick(1);
  let until=Instant::now()+Duration::from_secs(4);while self.r16(0x5802)==self.tx_used{assert!(Instant::now()<until,"TX timeout CID {}",self.cid);thread::sleep(Duration::from_micros(100));}fence(Ordering::SeqCst);self.tx_used=self.tx_used.wrapping_add(1);
 }
 fn try_rx(&mut self,timeout:Duration)->Option<(u16,u32,Vec<u8>)>{
  self.w16(0x1004+(self.rx_av%N) as u64*2,0);self.rx_av=self.rx_av.wrapping_add(1);fence(Ordering::SeqCst);self.w16(0x1002,self.rx_av);self.kick(0);
  let until=Instant::now()+timeout;while self.r16(0x1802)==self.rx_used{if Instant::now()>=until{return None}thread::sleep(Duration::from_micros(100));}fence(Ordering::SeqCst);self.rx_used=self.rx_used.wrapping_add(1);
  let h=self.read(BUF,44);assert_eq!(u64::from_le_bytes(h[8..16].try_into().unwrap()),self.cid as u64);assert_eq!(u32::from_le_bytes(h[20..24].try_into().unwrap()),self.port);
  let n=u32::from_le_bytes(h[24..28].try_into().unwrap()) as usize;assert!(n<=65536);let op=u16::from_le_bytes(h[30..32].try_into().unwrap());let flags=u32::from_le_bytes(h[32..36].try_into().unwrap());let b=self.read(BUF+0x100,n);self.fwd=self.fwd.wrapping_add(n as u32);self.packets+=1;
  if n>0{self.tx(6,0,&[]);} Some((op,flags,b))
 }
 fn rx(&mut self)->(u16,u32,Vec<u8>){self.try_rx(Duration::from_secs(4)).expect("RX timed out")}
 fn connect(&mut self){self.tx(1,0,&[]);loop{let(op,_,b)=self.rx();if op==2{assert!(b.is_empty());break}assert_ne!(op,3,"connect rejected")}}
 fn bytes(&mut self,b:&[u8],split:usize){for c in b.chunks(split){self.tx(5,0,c)}}

}
impl Drop for Peer{fn drop(&mut self){ioctl(self.fd.as_raw_fd(),0x4004af61,&0i32);unsafe{libc::munmap(self.mem as *mut _,SIZE);}}}
fn event(v:serde_json::Value){println!("{v}");io::stdout().flush().unwrap()}
#[repr(C)] struct VsockAddr{family:u16,reserved:u16,port:u32,cid:u32,flags:u8,zero:[u8;3]}
fn listener(typ:i32)->OwnedFd{unsafe{let f=OwnedFd::from_raw_fd(libc::socket(libc::AF_VSOCK,typ,0));assert!(f.as_raw_fd()>=0);let a=VsockAddr{family:libc::AF_VSOCK as u16,reserved:0,port:29500,cid:2,flags:0,zero:[0;3]};assert_eq!(libc::bind(f.as_raw_fd(),&a as *const _ as *const _,16),0,"bind {}",io::Error::last_os_error());assert_eq!(libc::listen(f.as_raw_fd(),32),0);f}}
fn accept(f:&OwnedFd)->OwnedFd{unsafe{let n=libc::accept(f.as_raw_fd(),std::ptr::null_mut(),std::ptr::null_mut());assert!(n>=0,"accept {}",io::Error::last_os_error());OwnedFd::from_raw_fd(n)}}
fn cookie(fd:i32)->u64{let mut n=0u64;let mut len=8u32;assert_eq!(unsafe{libc::getsockopt(fd,libc::SOL_SOCKET,57,&mut n as *mut _ as *mut _,&mut len)},0);n}
fn counters(bpf:&aya::Ebpf,n:u32)->Vec<u64>{let a=PerCpuArray::<_,u64>::try_from(bpf.map("COUNTERS").unwrap()).unwrap();(0..n).map(|i|a.get(&i,0).unwrap().iter().sum()).collect()}
fn register(bpf:&mut aya::Ebpf,a:i32,b:i32,key:u32)->bool {
 let mut m=SockMap::try_from(bpf.map_mut("SOCKETS").unwrap()).unwrap();
 for (k,fd) in [(key,a),(key+1,b)]{if let Err(e)=m.set(k,&fd,0){event(json!({"event":"map_insert_failed","key":k,"fd":fd,"error":format!("{e:?}")}));return false}}
 let mut r=HashMap::<_,u64,u32>::try_from(bpf.map_mut("ROUTES").unwrap()).unwrap();r.insert(cookie(a),key+1,0).unwrap();r.insert(cookie(b),key,0).unwrap();event(json!({"event":"map_pair_registered","keys":[key,key+1],"fds":[a,b],"cookies":[cookie(a),cookie(b)]}));true
}
fn receive(p:&mut Peer,n:usize)->Vec<u8>{let mut all=vec![];while all.len()<n{let(op,flags,b)=p.rx();event(json!({"event":"wire_rx","cid":p.cid,"op":op,"flags":flags,"len":b.len()}));if op==5{all.extend(b)}else{assert_ne!(op,3)}}all}
fn main(){
 event(json!({"event":"begin","kernel":fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"initial_fds":fs::read_dir("/proc/self/fd").unwrap().count(),"controller_payload_reads_writes":0}));
 let mut bpf=EbpfLoader::new().verifier_log_level(VerifierLogLevel::VERBOSE|VerifierLogLevel::STATS).load_file("/program.o").unwrap();
 let mapfd=SockMap::try_from(bpf.map("SOCKETS").unwrap()).unwrap().fd().try_clone().unwrap();
 let prog:&mut SkSkb=bpf.program_mut("route").unwrap().try_into().unwrap();prog.load().unwrap();event(json!({"event":"bpf_loaded","info":format!("{:?}",prog.info().unwrap())}));prog.attach(&mapfd).unwrap();
 let vs_listener=listener(libc::SOCK_STREAM);let mut p=Peer::new(295101,4001,1);p.connect();let vs=accept(&vs_listener);
 let net_listener=TcpListener::bind("127.0.0.1:0").unwrap();let mut app=TcpStream::connect(net_listener.local_addr().unwrap()).unwrap();let (proxy,_)=net_listener.accept().unwrap();app.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
 if register(&mut bpf,proxy.as_raw_fd(),vs.as_raw_fd(),0){
  app.write_all(b"tcp-to-vsock-native").unwrap();assert_eq!(receive(&mut p,19),b"tcp-to-vsock-native");
  p.bytes(b"vsock-to-tcp-native",5);let mut got=[0u8;19];app.read_exact(&mut got).unwrap();assert_eq!(&got,b"vsock-to-tcp-native");event(json!({"event":"tcp_bidirectional_pass","counters":counters(&bpf,2),"real_vhost_fd":p.fd.as_raw_fd(),"cid":p.cid,"memory":format!("{:p}",p.mem)}));
 }
 drop(proxy);drop(vs);drop(vs_listener);p.tx(3,0,&[]);drop(p);
 let seq_listener=listener(libc::SOCK_SEQPACKET);let mut u=Peer::new(295102,4002,2);u.connect();let seq=accept(&seq_listener);
 let app=UdpSocket::bind("127.0.0.2:0").unwrap();let proxy=UdpSocket::bind("127.0.0.1:0").unwrap();proxy.connect(app.local_addr().unwrap()).unwrap();app.connect(proxy.local_addr().unwrap()).unwrap();app.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
 if register(&mut bpf,proxy.as_raw_fd(),seq.as_raw_fd(),2){
  app.send(b"udp-to-vsock-native").unwrap();assert_eq!(receive(&mut u,19),b"udp-to-vsock-native");
  u.tx(5,1,b"vsock-to-udp-native");let mut got=[0;65536];let n=app.recv(&mut got).unwrap();assert_eq!(&got[..n],b"vsock-to-udp-native");event(json!({"event":"udp_seqpacket_bidirectional_pass","counters":counters(&bpf,4),"peer":app.local_addr().unwrap().to_string(),"proxy":proxy.local_addr().unwrap().to_string()}));
 }
 drop(proxy);drop(seq);drop(seq_listener);u.tx(3,0,&[]);drop(u);drop(bpf);event(json!({"event":"complete","final_fds":fs::read_dir("/proc/self/fd").unwrap().count(),"controller_payload_reads_writes":0}));
}
