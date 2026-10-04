//! Endpoint-only synthetic virtio driver. TCP/UDP adaptation is in shmproxy.ko.
use std::{collections::HashSet,fs,io::{self,Read,Write},net::{TcpListener,UdpSocket,SocketAddr},os::fd::{AsRawFd,FromRawFd,OwnedFd},sync::atomic::{fence,Ordering},thread,time::{Duration,Instant}};
use serde_json::json;
const SIZE:usize=256*1024; const N:u16=8; const BUF:u64=0x8000;
fn ioctl<T>(fd:i32,req:u64,p:&T){ let rc=unsafe{libc::ioctl(fd,req as libc::c_ulong,p as *const T)};assert_eq!(rc,0,"ioctl {req:x}: {}",io::Error::last_os_error()); }
#[repr(C)] struct State{index:u32,num:u32}
#[repr(C)] struct Addr{index:u32,flags:u32,desc:u64,used:u64,avail:u64,log:u64}
#[repr(C)] struct VFile{index:u32,fd:i32}
#[repr(C)] struct Memory{n:u32,pad:u32,gpa:u64,size:u64,user:u64,flags:u64}
struct Peer{fd:OwnedFd,mem:*mut u8,kicks:Vec<OwnedFd>,calls:Vec<OwnedFd>,cid:u32,port:u32,rx_av:u16,rx_used:u16,tx_av:u16,tx_used:u16,fwd:u32,stream:Vec<u8>,packets:u64}
impl Peer{
 fn new(cid:u32,port:u32)->Self{
  let fd=fs::OpenOptions::new().read(true).write(true).open("/dev/vhost-vsock").unwrap();let fd:OwnedFd=fd.into();
  let mem=unsafe{libc::mmap(std::ptr::null_mut(),SIZE,libc::PROT_READ|libc::PROT_WRITE,libc::MAP_PRIVATE|libc::MAP_ANONYMOUS,-1,0)} as *mut u8; assert_ne!(mem as isize,-1);
  ioctl(fd.as_raw_fd(),0xaf01,&0u64);ioctl(fd.as_raw_fd(),0x4008af00,&(1u64<<32));
  ioctl(fd.as_raw_fd(),0x4008af03,&Memory{n:1,pad:0,gpa:0,size:SIZE as u64,user:mem as u64,flags:0});
  let mut kicks=vec![];let mut calls=vec![];
  for q in 0..2u32{
   ioctl(fd.as_raw_fd(),0x4008af10,&State{index:q,num:N as u32});ioctl(fd.as_raw_fd(),0x4008af12,&State{index:q,num:0});let b=q as u64*0x4000;
   ioctl(fd.as_raw_fd(),0x4028af11,&Addr{index:q,flags:0,desc:mem as u64+b,avail:mem as u64+b+0x1000,used:mem as u64+b+0x1800,log:0});
   let kick=unsafe{OwnedFd::from_raw_fd(libc::eventfd(0,libc::EFD_NONBLOCK))};let call=unsafe{OwnedFd::from_raw_fd(libc::eventfd(0,libc::EFD_NONBLOCK))};
   ioctl(fd.as_raw_fd(),0x4008af20,&VFile{index:q,fd:kick.as_raw_fd()});ioctl(fd.as_raw_fd(),0x4008af21,&VFile{index:q,fd:call.as_raw_fd()});kicks.push(kick);calls.push(call);
  }
  ioctl(fd.as_raw_fd(),0x4008af60,&(cid as u64));ioctl(fd.as_raw_fd(),0x4004af61,&1i32);
  let mut p=Self{fd,mem,kicks,calls,cid,port,rx_av:0,rx_used:0,tx_av:0,tx_used:0,fwd:0,stream:vec![],packets:0};
  p.desc(0,BUF,44,3,1);p.desc(16,BUF+0x100,65536,2,0);p
 }
 fn write(&self,a:u64,b:&[u8]){unsafe{std::ptr::copy_nonoverlapping(b.as_ptr(),self.mem.add(a as usize),b.len())}}
 fn read(&self,a:u64,n:usize)->Vec<u8>{unsafe{std::slice::from_raw_parts(self.mem.add(a as usize),n).to_vec()}}
 fn w16(&self,a:u64,n:u16){unsafe{std::ptr::write_volatile(self.mem.add(a as usize) as *mut u16,n.to_le())}}
 fn r16(&self,a:u64)->u16{unsafe{u16::from_le(std::ptr::read_volatile(self.mem.add(a as usize) as *const u16))}}
 fn desc(&self,a:u64,addr:u64,len:u32,flags:u16,next:u16){let mut b=vec![];b.extend(addr.to_le_bytes());b.extend(len.to_le_bytes());b.extend(flags.to_le_bytes());b.extend(next.to_le_bytes());self.write(a,&b)}
 fn kick(&self,q:usize){let n=1u64;assert_eq!(unsafe{libc::write(self.kicks[q].as_raw_fd(),&n as *const u64 as *const _,8)},8)}
 fn tx(&mut self,op:u16,flags:u32,b:&[u8]){
  let mut h=vec![];h.extend((self.cid as u64).to_le_bytes());h.extend(2u64.to_le_bytes());h.extend(self.port.to_le_bytes());h.extend(29500u32.to_le_bytes());h.extend((b.len() as u32).to_le_bytes());h.extend(1u16.to_le_bytes());h.extend(op.to_le_bytes());h.extend(flags.to_le_bytes());h.extend(65536u32.to_le_bytes());h.extend(self.fwd.to_le_bytes());
  self.write(0x20000,&h);self.write(0x20100,b);self.desc(0x4000,0x20000,44,1,1);self.desc(0x4010,0x20100,b.len() as u32,0,0);
  self.w16(0x5004+(self.tx_av%N) as u64*2,0);self.tx_av=self.tx_av.wrapping_add(1);fence(Ordering::SeqCst);self.w16(0x5002,self.tx_av);self.kick(1);
  let until=Instant::now()+Duration::from_secs(20);while self.r16(0x5802)==self.tx_used{assert!(Instant::now()<until,"TX timeout CID {}",self.cid);thread::sleep(Duration::from_micros(100));}fence(Ordering::SeqCst);self.tx_used=self.tx_used.wrapping_add(1);
 }
 fn rx(&mut self)->(u16,u32,Vec<u8>){
  self.w16(0x1004+(self.rx_av%N) as u64*2,0);self.rx_av=self.rx_av.wrapping_add(1);fence(Ordering::SeqCst);self.w16(0x1002,self.rx_av);self.kick(0);
  let until=Instant::now()+Duration::from_secs(20);while self.r16(0x1802)==self.rx_used{assert!(Instant::now()<until,"RX timeout CID {} stats {}",self.cid,fs::read_to_string("/proc/shmproxy").unwrap());thread::sleep(Duration::from_micros(100));}fence(Ordering::SeqCst);self.rx_used=self.rx_used.wrapping_add(1);
  let h=self.read(BUF,44);assert_eq!(u64::from_le_bytes(h[8..16].try_into().unwrap()),self.cid as u64);assert_eq!(u32::from_le_bytes(h[20..24].try_into().unwrap()),self.port);
  let n=u32::from_le_bytes(h[24..28].try_into().unwrap()) as usize;assert!(n<=65536);let op=u16::from_le_bytes(h[30..32].try_into().unwrap());let flags=u32::from_le_bytes(h[32..36].try_into().unwrap());let b=self.read(BUF+0x100,n);self.fwd=self.fwd.wrapping_add(n as u32);self.packets+=1;
  if n>0{self.tx(6,0,&[]);} (op,flags,b)
 }
 fn connect(&mut self){self.tx(1,0,&[]);loop{let(op,_,b)=self.rx();if op==2{assert!(b.is_empty());break}assert_ne!(op,3,"connect rejected")}}
 fn bytes(&mut self,b:&[u8],split:usize){for c in b.chunks(split){self.tx(5,0,c)}}
 fn frame(&mut self,kind:u8,port:u16,addr:[u8;4],b:&[u8],split:usize){let mut x=vec![kind,0];x.extend(port.to_be_bytes());x.extend(addr);x.extend((b.len() as u32).to_be_bytes());x.extend(b);self.bytes(&x,split)}
 fn get(&mut self)->(u8,u16,[u8;4],Vec<u8>){
  loop{if self.stream.len()>=12{let n=u32::from_be_bytes(self.stream[8..12].try_into().unwrap()) as usize;assert!(n<=60000);if self.stream.len()>=12+n{let kind=self.stream[0];let port=u16::from_be_bytes(self.stream[2..4].try_into().unwrap());let addr=self.stream[4..8].try_into().unwrap();let data=self.stream[12..12+n].to_vec();self.stream.drain(..12+n);return(kind,port,addr,data)}}
   let(op,_,b)=self.rx();if op==5{self.stream.extend(b)}else{assert_ne!(op,3,"unexpected reset")}
  }
 }
 fn setup(&mut self,kind:u8,port:u16)->u16{self.connect();self.frame(kind,port,[127,0,0,1],&[],1);let(k,p,_,b)=self.get();assert_eq!(k,20);assert!(b.is_empty());p}
}
impl Drop for Peer{fn drop(&mut self){ioctl(self.fd.as_raw_fd(),0x4004af61,&0i32);unsafe{libc::munmap(self.mem as *mut _,SIZE);}}}
fn event(v:serde_json::Value){println!("{v}");io::stdout().flush().unwrap()}
fn stats()->String{fs::read_to_string("/proc/shmproxy").unwrap()}
fn main(){
 let start=Instant::now();event(json!({"event":"begin","kernel":fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"initial_fds":fs::read_dir("/proc/self/fd").unwrap().count(),"initial_tasks":fs::read_dir("/proc/self/task").unwrap().count(),"placement":"host-role isolated QEMU VM; module adapts; Rust endpoints only"}));
 let listener=TcpListener::bind("127.0.0.1:0").unwrap();let tcp_port=listener.local_addr().unwrap().port();
 let echo=thread::spawn(move||{let(mut s,_)=listener.accept().unwrap();s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();let mut b=[0;1733];loop{let n=s.read(&mut b).unwrap();if n==0{break}s.write_all(&b[..n]).unwrap()}s.write_all(b"after-half-close").unwrap();});
 let mut p=Peer::new(295001,4001);p.setup(1,tcp_port);
 // Split wire headers across packets; reassemble a byte stream independent of writes.
 let data:Vec<u8>=(0..262144usize).map(|i|((i*31+17)%251) as u8).collect();
 for part in data.chunks(50000){p.frame(10,0,[0;4],part,3071)}
 let held=stats();thread::sleep(Duration::from_millis(300));let stalled=stats();event(json!({"event":"backpressure_hold","rx_descriptors_posted_during_hold":0,"before":held.trim(),"after":stalled.trim()}));
 let mut got=vec![];while got.len()<data.len(){let(k,_,_,b)=p.get();assert_eq!(k,10);got.extend(b)}assert_eq!(got,data);
 p.frame(11,0,[0;4],&[],2);let mut tail=vec![];loop{let(k,_,_,b)=p.get();if k==11{break}assert_eq!(k,10);tail.extend(b)}assert_eq!(tail,b"after-half-close");echo.join().unwrap();
 event(json!({"event":"tcp_pass","cid":p.cid,"vhost_fd":p.fd.as_raw_fd(),"guest_memory":format!("{:p}",p.mem),"bytes_each_direction":data.len(),"virtqueue_rx_packets":p.packets,"split_sizes":[1,2,3071,1733],"half_close_tail":String::from_utf8(tail).unwrap(),"stats":stats().trim()}));drop(p);
 let udp=UdpSocket::bind("127.0.0.1:0").unwrap();let echo_addr=udp.local_addr().unwrap();udp.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
 let udp_echo=thread::spawn(move||{let mut b=vec![0;65536];for _ in 0..4{let(n,a)=udp.recv_from(&mut b).unwrap();assert_eq!(udp.send_to(&b[..n],a).unwrap(),n)}});
 let mut a=Peer::new(295002,4002);let outbound_port=a.setup(2,0);
 for (i,n) in [0,1,1431,59000].into_iter().enumerate(){let b:Vec<u8>=(0..n).map(|j|((j+i*19)%251) as u8).collect();a.frame(10,echo_addr.port(),[127,0,0,1],&b,4079);let(k,port,addr,got)=a.get();assert_eq!((k,port,addr),(10,echo_addr.port(),[127,0,0,1]));assert_eq!(got,b);event(json!({"event":"udp_outbound_pass","len":n,"remote_port":port,"kernel_local_port":outbound_port,"datagram_boundary":true}));}udp_echo.join().unwrap();
 let mut b=Peer::new(295003,4003);let inbound_port=b.setup(3,0);let x=UdpSocket::bind("127.0.0.1:0").unwrap();let y=UdpSocket::bind("127.0.0.1:0").unwrap();x.set_read_timeout(Some(Duration::from_secs(20))).unwrap();y.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
 event(json!({"event":"held_transport_owners","actual_vhost_devices":2,"cids":[a.cid,b.cid],"fds":[a.fd.as_raw_fd(),b.fd.as_raw_fd()],"memory":[format!("{:p}",a.mem),format!("{:p}",b.mem)],"kernel_udp_ports":[outbound_port,inbound_port],"process_fds":fs::read_dir("/proc/self/fd").unwrap().count(),"status":fs::read_to_string("/proc/self/status").unwrap(),"stats":stats().trim()}));
 for (endpoint,n) in [(&x,0usize),(&y,17),(&x,59000),(&y,1500)]{let payload:Vec<u8>=(0..n).map(|i|((i*13)%251) as u8).collect();endpoint.send_to(&payload,("127.0.0.1",inbound_port)).unwrap();let(k,port,addr,got)=b.get();assert_eq!(k,10);assert_eq!(port,endpoint.local_addr().unwrap().port());assert_eq!(addr,[127,0,0,1]);assert_eq!(got,payload);b.frame(10,port,addr,&got,2041);let mut reply=vec![0;65536];let(len,source)=endpoint.recv_from(&mut reply).unwrap();assert_eq!(source,SocketAddr::from(([127,0,0,1],inbound_port)));assert_eq!(&reply[..len],payload);event(json!({"event":"udp_inbound_pass","len":n,"source_peer_port":port,"server_port":inbound_port,"datagram_boundary":true}));}
 // Oversized UDP is consumed whole with MSG_TRUNC and reported, never partial-forwarded.
 x.send_to(&vec![7;61000],("127.0.0.1",inbound_port)).unwrap();let(k,port,_,got)=b.get();assert_eq!(k,30);assert_eq!(port,x.local_addr().unwrap().port());assert!(got.is_empty());event(json!({"event":"udp_truncation_pass","input_len":61000,"bound":60000,"partial_payload_delivered":false,"stats":stats().trim()}));
 a.frame(12,0,[0;4],&[],12);b.tx(3,0,&[]);drop(a);drop(b);drop(x);drop(y);
 let until=Instant::now()+Duration::from_secs(5);while !stats().contains("closed=3 "){assert!(Instant::now()<until,"cleanup timeout {}",stats());thread::sleep(Duration::from_millis(10));}
 let mut c=Peer::new(295004,4004);c.connect();c.frame(99,0,[0;4],&[],3);thread::sleep(Duration::from_millis(100));drop(c);event(json!({"event":"negative_bad_registration","stats":stats().trim()}));
 // Population is actual started vhost objects, one kernel UDP adapter socket per CID.
 let mut limit=libc::rlimit{rlim_cur:0,rlim_max:0};assert_eq!(unsafe{libc::getrlimit(libc::RLIMIT_NOFILE,&mut limit)},0);limit.rlim_cur=limit.rlim_max;assert_eq!(unsafe{libc::setrlimit(libc::RLIMIT_NOFILE,&limit)},0);
 let endpoint=UdpSocket::bind("127.0.0.1:0").unwrap();let destination=endpoint.local_addr().unwrap().port();endpoint.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
 let echo_stop=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));let stop_echo=echo_stop.clone();
 let echo_task=thread::spawn(move||{let mut data=[0;4096];while !stop_echo.load(Ordering::SeqCst){match endpoint.recv_from(&mut data){Ok((n,peer))=>{endpoint.send_to(&data[..n],peer).unwrap();},Err(e) if e.kind()==io::ErrorKind::WouldBlock||e.kind()==io::ErrorKind::TimedOut=>{},Err(e)=>panic!("{e}")}}});
 let capacity_started=Instant::now();let mut peers=Vec::new();let mut local_ports=HashSet::new();let mut maps=HashSet::new();let mut fds=HashSet::new();
 for stage in [16,64,256,1024,4096]{
  while peers.len()<stage{let i=peers.len();let mut peer=Peer::new(300000+i as u32,5000);let local=peer.setup(2,0);assert!(local_ports.insert(local));assert!(maps.insert(peer.mem as usize));assert!(fds.insert(peer.fd.as_raw_fd()));peers.push(peer);}
  let check_started=Instant::now();
  for peer in &mut peers{let data=format!("cid={} held-stage={stage} bounded-UDP-kernel-proxy",peer.cid).into_bytes();peer.frame(10,destination,[127,0,0,1],&data,31);let(k,port,addr,reply)=peer.get();assert_eq!((k,port,addr),(10,destination,[127,0,0,1]));assert_eq!(reply,data);}
  event(json!({"event":"kernel_capacity_stage","actual_started_vhost_devices":peers.len(),"actual_kernel_udp_adapter_sockets":local_ports.len(),"independent_guest_memory_contexts":maps.len(),"distinct_open_vhost_fds":fds.len(),"every_owner_tagged_bidirectional_datagram":true,"first_cid":peers.first().unwrap().cid,"last_cid":peers.last().unwrap().cid,"held_process_fd_count":fs::read_dir("/proc/self/fd").unwrap().count(),"held_process_task_count":fs::read_dir("/proc/self/task").unwrap().count(),"meminfo":fs::read_to_string("/proc/meminfo").unwrap(),"process_status":fs::read_to_string("/proc/self/status").unwrap(),"kernel_adapter_stats":stats().trim(),"elapsed_s":capacity_started.elapsed().as_secs_f64(),"pool_traffic_check_s":check_started.elapsed().as_secs_f64()}));
 }
 for mut peer in peers{peer.frame(12,0,[0;4],&[],12);drop(peer);}
 echo_stop.store(true,Ordering::SeqCst);let wake=UdpSocket::bind("127.0.0.1:0").unwrap();wake.send_to(b"wake",("127.0.0.1",destination)).unwrap();echo_task.join().unwrap();drop(wake);
 let drain_until=Instant::now()+Duration::from_secs(10);while !stats().contains("closed=4100 "){assert!(Instant::now()<drain_until,"capacity drain timeout {}",stats());thread::sleep(Duration::from_millis(10));}
 event(json!({"event":"kernel_capacity_cleanup","released_vhost_devices":4096,"adapter_stats":stats().trim(),"process_fds":fs::read_dir("/proc/self/fd").unwrap().count(),"process_tasks":fs::read_dir("/proc/self/task").unwrap().count(),"capacity_elapsed_s":capacity_started.elapsed().as_secs_f64()}));
 event(json!({"event":"complete","functional_transport_owners_total":4,"max_concurrent_vhost_devices":2,"kernel_proxy_sessions":3,"application_forwarding_userspace_processes":0,"guest_ordinary_socket_adaptation_validated":false,"elapsed_s":start.elapsed().as_secs_f64(),"final_fds":fs::read_dir("/proc/self/fd").unwrap().count(),"final_tasks":fs::read_dir("/proc/self/task").unwrap().count(),"stats":stats().trim()}));
}
