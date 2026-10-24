use std::{fs,io::{Read,Write},path::{Path,PathBuf},os::fd::{AsFd,AsRawFd},sync::{Arc,atomic::{AtomicBool,Ordering}},time::{Duration,Instant}};
use overdrive_testing::{guest_kit::{GuestKit,GuestImageSpec,GuestFile,GuestBoot,WorkDir,cid_bands},guest_lane::HostListeners,vsock::{VsockListener,VsockStream,VMADDR_CID_ANY}};
use overdrive_core::{vm::beacon::{BeaconMessage,BEACON_VSOCK_PORT},guest_transport::wire::{FlowRequest,ControlMessage}};
fn line(s:&VsockStream,seconds:u64)->String{let end=Instant::now()+Duration::from_secs(seconds);let mut out=Vec::new();let mut b=[0];loop{match s.read_within(&mut b,end.saturating_duration_since(Instant::now())).unwrap(){Some(1) if b[0]==b'\n'=>return String::from_utf8(out).unwrap(),Some(1)=>out.push(b[0]),x=>panic!("beacon line={out:?} read={x:?}")}}}
fn main(){
 unsafe { let limit=libc::rlimit{rlim_cur:65536,rlim_max:65536};assert_eq!(libc::setrlimit(libc::RLIMIT_NOFILE,&limit),0); }
 println!("SOURCE {}",fs::read_to_string(".overdrive-metal-source").unwrap());
 let staging=overdrive_testing::vm_fixture::default_staging_root();
 let kit=GuestKit::provision(&staging).unwrap();println!("KIT kernel={} init={}",kit.kernel_path.display(),kit.init_bin.display());
 let work=WorkDir::create(&staging,"review-11-06-udp").unwrap();
 let guest=PathBuf::from(std::env::args().nth(1).unwrap());
 let mut extra=vec![GuestFile{host:guest,guest:PathBuf::from("/opt/review-guest"),with_shared_libraries:false}];
 if std::env::args().any(|x|x=="with-diag"){for name in ["inet_diag","tcp_diag","udp_diag"] {extra.push(overdrive_testing::guest_kit::host_module_file(&kit,work.path(),&format!("net/ipv4/{name}"),&PathBuf::from(format!("/lib/overdrive-testing/modules/{name}.ko"))).unwrap());}}
 let image=kit.image(work.path(),"udp-recycle",&GuestImageSpec{loopback_module:std::env::args().any(|x|x=="with-loopback"),extra,..GuestImageSpec::workload()}).unwrap();
 let beacon=VsockListener::bind(VMADDR_CID_ANY,BEACON_VSOCK_PORT.as_u32()).unwrap();
 let listeners=HostListeners::bind().unwrap();
 let cid=cid_bands::INIT_IN_GUEST_LANE.raw(15);
 let token=listeners.transport_token(cid);
 let vm=kit.boot(&GuestBoot{image:&image,cid,cmdline_extra:vec![token],memory_mib:512,vcpus:2,run_dir:&work.path().join("run")}).unwrap();
 let (beacon,_)=beacon.accept_within(Duration::from_secs(60)).unwrap().unwrap();
 let ready=std::panic::catch_unwind(||line(&beacon,60)); if ready.is_err(){let _=vm.wait_exit(Duration::from_secs(5));println!("GUEST_BOOT_REFUSED\n{}",vm.console());fs::write(".context/netns-density-295/review-11-06-spike/boot-refusal-console.txt",vm.console()).unwrap();std::process::exit(4);} println!("READY {}",ready.unwrap());
 let (session,_)=listeners.control.accept_within(Duration::from_secs(60)).unwrap().unwrap();
 let mut open=[0;24];(&session).read_exact(&mut open).unwrap();
 let stop=Arc::new(AtomicBool::new(false));let done=stop.clone();
 let echoes=std::thread::spawn(move||{
  let mut flows:Vec<(VsockStream,Vec<u8>)>=Vec::new(); let mut count=0;
  while !done.load(Ordering::Relaxed){
   let mut p=vec![libc::pollfd{fd:listeners.stream.as_fd().as_raw_fd(),events:libc::POLLIN,revents:0}];
   for (s,_) in &flows{p.push(libc::pollfd{fd:s.as_fd().as_raw_fd(),events:libc::POLLIN,revents:0});}
   let n=unsafe{libc::poll(p.as_mut_ptr(),p.len() as _,100)};if n<0{panic!("poll {}",std::io::Error::last_os_error());}
   if p[0].revents & libc::POLLIN!=0 {
    let (s,_)=listeners.stream.accept().unwrap(); let mut req=[0;32];(&s).read_exact(&mut req).unwrap();
    let request=FlowRequest::decode(&req).unwrap(); assert!(matches!(request,FlowRequest::DatagramStream{..}));
    (&session).write_all(&ControlMessage::Paired{flow:request.flow()}.encode()).unwrap();
    flows.push((s,Vec::new()));count+=1;
   }
   for i in (0..p.len()-1).rev(){
    let flags=p[i+1].revents;if flags==0{continue;}
    let mut b=[0;4096];
    let got=unsafe{libc::recv(flows[i].0.as_fd().as_raw_fd(),b.as_mut_ptr().cast(),b.len(),libc::MSG_DONTWAIT)};
    if got==0{flows.swap_remove(i);continue;}if got<0{if std::io::Error::last_os_error().kind()==std::io::ErrorKind::WouldBlock{continue;}flows.swap_remove(i);continue;}
    let (s,buf)=&mut flows[i];buf.extend_from_slice(&b[..got as usize]);
    while buf.len()>=8{assert_eq!(&buf[..4],b"OVD1");let len=u32::from_be_bytes(buf[4..8].try_into().unwrap()) as usize;if buf.len()<8+len{break;}(&*s).write_all(&buf[..8+len]).unwrap();buf.drain(..8+len);}
   }
  }
  println!("HOST_REQUESTS={count}");
 });
 let mut argv=vec!["/opt/review-guest".into()];if std::env::args().any(|x|x=="with-diag"){argv.push("with-diag".into());} if std::env::args().any(|x|x=="after-release-wait"){argv.push("after-release-wait".into());} let exec=BeaconMessage::Exec{argv};
 (&beacon).write_all(format!("{exec}\n").as_bytes()).unwrap();
 let exit=line(&beacon,240);println!("GUEST {exit}");
 let _=vm.wait_exit(Duration::from_secs(10));stop.store(true,Ordering::Relaxed);let echo_result=echoes.join();
 println!("GUEST_CONSOLE\n{}",vm.console());
 fs::write(Path::new(".context/netns-density-295/review-11-06-spike/native-console.txt"),vm.console()).unwrap();
 echo_result.unwrap();assert_eq!(exit,"EXIT 0");
}
