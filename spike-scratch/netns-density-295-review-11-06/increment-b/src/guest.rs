use std::net::{UdpSocket,SocketAddrV4,Ipv4Addr};
use std::time::Duration;
fn socket() -> UdpSocket {
 let s=UdpSocket::bind((Ipv4Addr::UNSPECIFIED,0)).unwrap();
 s.set_read_timeout(Some(Duration::from_millis(800))).unwrap(); s
}
#[derive(Debug,PartialEq)]
enum Exchange{Echoed,Refused,AdmittedButLost}
fn exchange(s:&UdpSocket,seq:u32)->Exchange {
 let p=seq.to_be_bytes();
 match s.send_to(&p,SocketAddrV4::new(Ipv4Addr::new(203,0,113,99),7001)){Ok(4)=>{},Err(e) if e.raw_os_error()==Some(1)=>{println!("SLOT_STILL_BUSY seq={seq} errno=EPERM");return Exchange::Refused;},other=>panic!("unexpected UDP send {other:?}")}
 let mut b=[0;64];
 match s.recv_from(&mut b){Ok((n,a)) if b[..n]==p=> {if seq<3{println!("UDP_ECHO seq={seq} peer={a}");} Exchange::Echoed},other=>{println!("ADMITTED_DATAGRAM_LOST seq={seq} result={other:?}");Exchange::AdmittedButLost}}
}
fn main(){
 println!("REVIEW_SPIKE production PID1; live UDP first-send + full-slot recycle"); if std::env::args().any(|x|x=="with-diag"){load_diag();}
 let mut sockets=Vec::new();
 for i in 0..1024 {let s=socket(); if exchange(&s,i)!=Exchange::Echoed{println!("INITIAL_UDP_FAILURE at={i}");diagnose();std::process::exit(2);} sockets.push(s);}
 println!("ALL_1024_SLOTS_ECHOED");
 let mut lost=0;
 for i in 0..32 {
  let old=std::mem::replace(&mut sockets[i],socket());
  drop(old);
  if std::env::args().any(|x|x=="after-release-wait"){std::thread::sleep(Duration::from_millis(100));}
  let first=exchange(&sockets[i],1024+i as u32);
  if first!=Exchange::Echoed{if first==Exchange::AdmittedButLost{lost+=1;}std::thread::sleep(Duration::from_millis(100));let again=exchange(&sockets[i],2048+i as u32);println!("RECYCLE_OBSERVATION index={i} first={first:?} retry={again:?}");assert_eq!(again,Exchange::Echoed,"release must finish within the bounded wait");}
 }
 println!("RECYCLE_RESULT lost={lost}/32");
 if lost!=0{std::process::exit(3);}
}

fn diagnose(){
 println!("MODULES\n{}",std::fs::read_to_string("/proc/modules").unwrap());
 println!("UDP_TABLE\n{}",std::fs::read_to_string("/proc/net/udp").unwrap());
 unsafe extern "C" {fn socket(a:i32,b:i32,c:i32)->i32;fn bind(fd:i32,a:*const u8,n:u32)->i32;fn sendto(fd:i32,b:*const u8,n:usize,f:i32,a:*const u8,l:u32)->isize;fn recv(fd:i32,b:*mut u8,n:usize,f:i32)->isize;fn close(fd:i32)->i32;}
 for proto in [6u8,17u8]{unsafe{
 let fd=socket(16,3,4);assert!(fd>=0);let addr=[16u8,0,0,0,0,0,0,0,0,0,0,0];assert_eq!(bind(fd,addr.as_ptr(),12),0);
 let mut req=vec![0u8;72];req[0..4].copy_from_slice(&72u32.to_ne_bytes());req[4..6].copy_from_slice(&20u16.to_ne_bytes());req[6..8].copy_from_slice(&0x301u16.to_ne_bytes());req[8..12].copy_from_slice(&1u32.to_ne_bytes());req[16]=2;req[17]=proto;req[20..24].copy_from_slice(&u32::MAX.to_ne_bytes());
 let sent=sendto(fd,req.as_ptr(),req.len(),0,addr.as_ptr(),12);let mut r=[0u8;32768];let n=recv(fd,r.as_mut_ptr(),r.len(),0);println!("DIAG proto={proto} send={sent} received={n} data={:?}",&r[..n.max(0).min(100) as usize]);close(fd);
 }}
}

fn load_diag(){
 use std::os::fd::AsRawFd;
 unsafe extern "C" {fn syscall(n:i64,...)->i64;}
 for m in ["inet_diag","tcp_diag","udp_diag"] {let file=std::fs::File::open(format!("/lib/overdrive-testing/modules/{m}.ko")).unwrap();let r=unsafe{syscall(313,file.as_raw_fd(),c"".as_ptr(),0)};println!("DIAG_LOAD {m} ret={r} errno={}",std::io::Error::last_os_error());assert_eq!(r,0);}
}
