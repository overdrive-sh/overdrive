//! Standalone stock transport-device capacity probe; peers are synthetic virtio drivers.
//! No KVM VM, guest CPU, Linux guest, virtio PCI bus or guest AF_VSOCK runs here.
use std::{fs, io::{self, Read, Write, BufRead}, os::unix::{io::AsRawFd, net::UnixStream}, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicU8,AtomicUsize,Ordering}}, time::{Instant, Duration}};
use virtio_devices::{ActivationContext, VirtioDevice, VirtioInterrupt, VirtioInterruptType};
use virtio_devices::vsock::{Vsock, VsockUnixBackend, VsockEpollListener};
use virtio_queue::{Queue, QueueT};
use vm_memory::{GuestAddress, GuestMemoryAtomic, GuestMemoryMmap, Bytes, bitmap::AtomicBitmap};
use vmm_sys_util::eventfd::EventFd;
use serde_json::json;
type Mem = GuestMemoryMmap<AtomicBitmap>;
struct Irq(AtomicUsize);
impl VirtioInterrupt for Irq {
 fn trigger(&self, _:VirtioInterruptType)->io::Result<()> { self.0.fetch_add(1,Ordering::SeqCst); Ok(()) }
 fn set_notifier(&self,_:u32,_:Option<EventFd>,_:&dyn hypervisor::Vm)->io::Result<()> { Err(io::Error::other("synthetic driver has no VM IRQ route")) }
}
struct Peer {
 cid:u32, device:Vsock<VsockUnixBackend>, mem:Mem, evts:Vec<EventFd>,
 irq:Arc<Irq>, status:Arc<AtomicU8>, host:Option<UnixStream>,
 rx_avail:u16, rx_used:u16, tx_avail:u16, tx_used:u16, host_port:u32,
 path:PathBuf, backend_epoll:i32, exit_fd:i32,
}
fn write16(m:&Mem,a:u64,v:u16) {m.write_slice(&v.to_le_bytes(),GuestAddress(a)).unwrap()}
fn read16(m:&Mem,a:u64)->u16 {let mut b=[0;2];m.read_slice(&mut b,GuestAddress(a)).unwrap();u16::from_le_bytes(b)}
fn descriptor(m:&Mem,a:u64,addr:u64,len:u32,flags:u16,next:u16) {
 let mut b=Vec::new();b.extend(addr.to_le_bytes());b.extend(len.to_le_bytes());b.extend(flags.to_le_bytes());b.extend(next.to_le_bytes());m.write_slice(&b,GuestAddress(a)).unwrap();
}
impl Peer {
 fn new(index:usize,dir:&Path)->io::Result<Self> {
  let cid=100_000+index as u32;let path=dir.join(format!("p{index:05}"));
  let backend=VsockUnixBackend::new(cid,path.to_str().unwrap().into()).map_err(io::Error::other)?;
  let backend_epoll=backend.get_polled_fd();let exit=EventFd::new(libc::EFD_NONBLOCK)?;let exit_fd=exit.as_raw_fd();
  let mut device=Vsock::new(format!("capacity-{index}"),cid,path.clone(),backend,false,seccompiler::SeccompAction::Allow,exit,None)?;
  let mut config=[0;8];device.read_config(0,&mut config);assert_eq!(u64::from_le_bytes(config),cid as u64);
  assert_eq!(device.device_type(),19);assert_eq!(device.queue_max_sizes(),[256,256,256]);
  device.ack_features(device.features());
  let mem=Mem::from_ranges(&[(GuestAddress(0),128*1024)]).map_err(io::Error::other)?;
  let irq=Arc::new(Irq(AtomicUsize::new(0)));let status=Arc::new(AtomicU8::new(15));let mut evts=Vec::new();let mut queues=Vec::new();
  for i in 0..3 {let b=(i*0x4000) as u64;let mut q=Queue::new(256).unwrap();q.set_size(256);q.set_ready(true);q.set_desc_table_address(Some(b as u32),Some(0));q.set_avail_ring_address(Some((b+0x1000) as u32),Some(0));q.set_used_ring_address(Some((b+0x1800) as u32),Some(0));let evt=EventFd::new(libc::EFD_NONBLOCK)?;queues.push((i,q,evt.try_clone()?));evts.push(evt);}
  // Two separate descriptors: 44-byte virtio_vsock header and payload buffer.
  descriptor(&mem,0,0xc000,44,3,1);descriptor(&mem,16,0xc100,4096,2,0);
  descriptor(&mem,0x4000,0xe000,44,1,1);descriptor(&mem,0x4010,0xe100,4096,0,0);
  device.activate(ActivationContext{mem:GuestMemoryAtomic::new(mem.clone()),interrupt_cb:irq.clone(),queues,device_status:status.clone()}).map_err(io::Error::other)?;
  Ok(Self{cid,device,mem,evts,irq,status,host:None,rx_avail:0,rx_used:0,tx_avail:0,tx_used:0,host_port:0,path,backend_epoll,exit_fd})
 }
 fn rx_arm(&mut self) {let pos=self.rx_avail%256;write16(&self.mem,0x1004+u64::from(pos)*2,0);self.rx_avail=self.rx_avail.wrapping_add(1);write16(&self.mem,0x1002,self.rx_avail);self.evts[0].write(1).unwrap();}
 fn rx(&mut self)->io::Result<(u16,Vec<u8>)> {
  let until=Instant::now()+Duration::from_secs(5);
  while read16(&self.mem,0x1802)==self.rx_used {if Instant::now()>until {return Err(io::Error::new(io::ErrorKind::TimedOut,format!("RX cid={} status={}",self.cid,self.status.load(Ordering::SeqCst))))}std::thread::sleep(Duration::from_micros(20));}
  self.rx_used=self.rx_used.wrapping_add(1);let mut hdr=[0;44];self.mem.read_slice(&mut hdr,GuestAddress(0xc000)).unwrap();
  assert_eq!(u64::from_le_bytes(hdr[8..16].try_into().unwrap()),self.cid as u64);assert_eq!(u64::from_le_bytes(hdr[0..8].try_into().unwrap()),2);
  self.host_port=u32::from_le_bytes(hdr[16..20].try_into().unwrap());assert_eq!(u32::from_le_bytes(hdr[20..24].try_into().unwrap()),5001);
  let n=u32::from_le_bytes(hdr[24..28].try_into().unwrap()) as usize;let op=u16::from_le_bytes(hdr[30..32].try_into().unwrap());let mut bytes=vec![0;n];self.mem.read_slice(&mut bytes,GuestAddress(0xc100)).unwrap();Ok((op,bytes))
 }
 fn tx(&mut self,op:u16,bytes:&[u8])->io::Result<()> {
  let mut h=Vec::new();h.extend((self.cid as u64).to_le_bytes());h.extend(2u64.to_le_bytes());h.extend(5001u32.to_le_bytes());h.extend(self.host_port.to_le_bytes());h.extend((bytes.len() as u32).to_le_bytes());h.extend(1u16.to_le_bytes());h.extend(op.to_le_bytes());h.extend(0u32.to_le_bytes());h.extend(65536u32.to_le_bytes());h.extend(0u32.to_le_bytes());
  self.mem.write_slice(&h,GuestAddress(0xe000)).unwrap();self.mem.write_slice(bytes,GuestAddress(0xe100)).unwrap();descriptor(&self.mem,0x4010,0xe100,bytes.len() as u32,0,0);
  let pos=self.tx_avail%256;write16(&self.mem,0x5004+u64::from(pos)*2,0);self.tx_avail=self.tx_avail.wrapping_add(1);write16(&self.mem,0x5002,self.tx_avail);self.evts[1].write(1)?;
  let until=Instant::now()+Duration::from_secs(5);while read16(&self.mem,0x5802)==self.tx_used {if Instant::now()>until {return Err(io::Error::new(io::ErrorKind::TimedOut,"TX"))}std::thread::sleep(Duration::from_micros(20));}self.tx_used=self.tx_used.wrapping_add(1);Ok(())
 }
 fn connect(&mut self)->io::Result<()> {self.rx_arm();let mut h=UnixStream::connect(&self.path)?;h.set_read_timeout(Some(Duration::from_secs(5)))?;h.write_all(b"CONNECT 5001\n")?;let (op,b)=self.rx()?;assert_eq!(op,1);assert!(b.is_empty());self.tx(2,&[])?;let mut ack=Vec::new();let mut x=[0];while !ack.ends_with(b"\n") {h.read_exact(&mut x)?;ack.push(x[0]);}assert_eq!(String::from_utf8(ack).unwrap(),format!("OK {}\n",self.host_port));self.host=Some(h);Ok(())}
 fn exchange(&mut self,epoch:usize)->io::Result<()> {
  let a=format!("host-to-virtqueue-cid-{}-epoch-{epoch}",self.cid).into_bytes();let b=format!("virtqueue-to-host-cid-{}-epoch-{epoch}",self.cid).into_bytes();
  self.rx_arm();self.host.as_mut().unwrap().write_all(&a)?;let (op,got)=self.rx()?;assert_eq!(op,5);assert_eq!(got,a);self.tx(5,&b)?;let mut got=vec![0;b.len()];self.host.as_mut().unwrap().read_exact(&mut got)?;assert_eq!(got,b);assert_eq!(self.status.load(Ordering::SeqCst),15);Ok(())
 }
 fn identity(&self)->serde_json::Value {let mut c=[0;8];self.device.read_config(0,&mut c);json!({"cid_from_device_config":u64::from_le_bytes(c),"cid_confirmed_by_rx_packet":self.cid,"device_address":format!("{:p}",&self.device),"guest_memory_context_address":format!("{:p}",&self.mem),"backend_epoll_fd":self.backend_epoll,"backend_path":self.path,"exit_fd":self.exit_fd,"driver_queue_eventfds":self.evts.iter().map(AsRawFd::as_raw_fd).collect::<Vec<_>>(),"host_connection_fd":self.host.as_ref().unwrap().as_raw_fd(),"irq_count":self.irq.0.load(Ordering::SeqCst),"device_status":self.status.load(Ordering::SeqCst),"negotiated_queues":3,"queue_size":256,"guest_memory_mapping_bytes":128*1024,"synthetic_driver":true})}
}
impl Drop for Peer {fn drop(&mut self) {self.host=None;self.device.reset();self.device.shutdown();}}
fn main()->io::Result<()> {
 let a:Vec<_>=std::env::args().collect();let dir=PathBuf::from(&a[1]);let out=PathBuf::from(&a[2]);let stages:Vec<usize>=a[3].split(',').map(|s|s.parse().unwrap()).collect();fs::create_dir(&dir)?;
 let initial_fds=fs::read_dir("/proc/self/fd")?.count();let initial_tasks=fs::read_dir("/proc/self/task")?.count();let start=Instant::now();let mut peers=Vec::new();let mut stdin=io::stdin().lock();
 for n in stages {while peers.len()<n {let mut p=match Peer::new(peers.len(),&dir) {Ok(p)=>p,Err(e)=>{println!("{}",json!({"event":"capacity_error","held":peers.len(),"next_index":peers.len(),"error":e.to_string(),"errno":e.raw_os_error()}));return Err(e)}};p.connect()?;peers.push(p);}
  let recheck=Instant::now();for p in &mut peers {p.exchange(n)?;}
  fs::write(out.join(format!("identities-{n}.json")),serde_json::to_vec(&peers.iter().map(Peer::identity).collect::<Vec<_>>())?)?;
  println!("{}",json!({"event":"stage_held","actual_stock_activated_devices":peers.len(),"actual_stock_muxers":peers.len(),"independent_synthetic_driver_contexts":peers.len(),"actual_linux_guests":0,"actual_kvm_vms":0,"held_transport_connections":peers.len(),"every_peer_tagged_bidirectional_exchange_passed":true,"process_fd_count":fs::read_dir("/proc/self/fd")?.count(),"process_task_count":fs::read_dir("/proc/self/task")?.count(),"elapsed_s":start.elapsed().as_secs_f64(),"full_pool_recheck_s":recheck.elapsed().as_secs_f64()}));io::stdout().flush()?;let mut s=String::new();stdin.read_line(&mut s)?;if s.trim()=="stop" {break}
 }
 println!("{}",json!({"event":"final_recheck_begin","held":peers.len()}));io::stdout().flush()?;for p in &mut peers {p.exchange(999_999)?;}let count=peers.len();drop(peers);let remaining=fs::read_dir(&dir)?.count();assert_eq!(remaining,0);fs::remove_dir(&dir)?;
 println!("{}",json!({"event":"cleaned","released_devices":count,"remaining_socket_paths":remaining,"initial_fds":initial_fds,"final_fds":fs::read_dir("/proc/self/fd")?.count(),"initial_tasks":initial_tasks,"final_tasks":fs::read_dir("/proc/self/task")?.count(),"elapsed_s":start.elapsed().as_secs_f64()}));Ok(())
}
