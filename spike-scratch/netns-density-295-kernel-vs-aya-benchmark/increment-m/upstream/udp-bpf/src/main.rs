#![no_std]
#![no_main]
use aya_ebpf::{bindings::{TC_ACT_OK,TC_ACT_SHOT},helpers::{bpf_get_socket_cookie,bpf_skb_adjust_room,bpf_skb_load_bytes},macros::{map,stream_verdict,classifier},maps::{Array,HashMap,PerCpuArray,SockHash},programs::{SkBuffContext,TcContext}};
#[repr(C)] #[derive(Clone,Copy)] pub struct Tuple { pub src:u32,pub dst:u32,pub sport:u16,pub dport:u16 }
#[map] static SOCKETS:SockHash<u32>=SockHash::with_max_entries(16,0);
#[map] static ROUTES:HashMap<u64,u32>=HashMap::with_max_entries(16,0);
#[map] static FLOWS:HashMap<Tuple,u32>=HashMap::with_max_entries(16,0);
#[map] static SERVER:Array<u32>=Array::with_max_entries(1,0);
#[repr(C)] #[derive(Clone,Copy)] pub struct Witness { pub seq:u64,pub len:u32,pub mode:u32,pub bytes:[u8;48] }
#[map] static WITNESS:Array<Witness>=Array::with_max_entries(2,0);
#[map] static COUNTERS:PerCpuArray<u64>=PerCpuArray::with_max_entries(32,0);
fn count(i:u32){if let Some(p)=COUNTERS.get_ptr_mut(i){unsafe{*p=(*p).wrapping_add(1)}}}
const MAGIC:[u8;4]=*b"ZUD1";
#[stream_verdict] pub fn route(mut ctx:SkBuffContext)->u32 {
 let cookie=unsafe{bpf_get_socket_cookie(ctx.skb.skb as *mut _)};
 let Some(key)=(unsafe{ROUTES.get(&cookie)}) else{count(8);return 0};let key=*key;count(key);
 let n=ctx.len();
 if key&1==1 {
  let header=[MAGIC[0],MAGIC[1],MAGIC[2],MAGIC[3],(n>>24) as u8,(n>>16) as u8,(n>>8) as u8,n as u8];
  let rc=unsafe{bpf_skb_adjust_room(ctx.skb.skb,8,0,0)};if rc!=0{count(9);return 0}
  if ctx.skb.store(0,&header,0).is_err(){count(9);return 0}count(12);if n==0{count(14)}
 }else{
  if n<8{count(10);return 0}
  let Ok(header)=ctx.load::<[u8;8]>(0) else{count(10);return 0};
  if header[0]!=MAGIC[0] || header[1]!=MAGIC[1] || header[2]!=MAGIC[2] || header[3]!=MAGIC[3] || u32::from_be_bytes([header[4],header[5],header[6],header[7]])!=n-8{count(10);return 0}
 }
 if ctx.pull_data(ctx.len()).is_err(){count(9);return 0}
 let mut target=key;let result=SOCKETS.redirect_skb(&ctx,&mut target,0) as u32;if result==0{count(11)}result
}
#[classifier] pub fn packet(ctx:TcContext)->i32 {match transform(ctx){Ok(x)=>x,Err(_)=>{count(20);TC_ACT_SHOT}}}
fn transform(mut ctx:TcContext)->Result<i32,i64>{
 let ip=14usize;let udp=34usize;
 if ctx.load::<u8>(ip).unwrap_or(0)!=0x45 || ctx.load::<u8>(ip+9).unwrap_or(0)!=17 || ctx.len()<42{return Ok(TC_ACT_OK)}
 let src=ctx.load::<u32>(ip+12)?;let dst=ctx.load::<u32>(ip+16)?;let sport=ctx.load::<u16>(udp)?;let dport=ctx.load::<u16>(udp+2)?;let tuple=Tuple{src,dst,sport,dport};
 let server=unsafe{SERVER.get(0).copied().unwrap_or(0)} as u16;
 let proxy=u32::from_ne_bytes([127,0,0,1]);let peer2=u32::from_ne_bytes([127,0,0,2]);let peer3=u32::from_ne_bytes([127,0,0,3]);
 let owned=(dst==proxy && (src==peer2 || src==peer3) && dport==server) || (src==proxy && (dst==peer2 || dst==peer3) && sport==server);
 if !owned{return Ok(TC_ACT_OK)}
 let Some(mode)=(unsafe{FLOWS.get(&tuple).copied()}) else{count(19);return Ok(TC_ACT_SHOT)};
 let old_ip=ctx.load::<u16>(ip+2)?;let old_udp=ctx.load::<u16>(udp+4)?;let plen=u16::from_be(old_udp) as u32;let ilen=u16::from_be(old_ip) as u32;
 if plen<8 || ilen!=plen+20 || ctx.len()!=ilen+14{return Err(-1)}
 let logical;if mode==1 {
  // Ordinary app→proxy packets stay untouched; SK_SKB encodes their payload.
  if let Some(ptr)=WITNESS.get_ptr_mut(0){unsafe{let p=&mut *ptr;p.seq=p.seq.wrapping_add(1);p.len=ctx.len();p.mode=mode;let rc=bpf_skb_load_bytes(ctx.skb.skb as *const _,0,p.bytes.as_mut_ptr() as *mut _,42);if rc!=0{return Err(rc)}}}
  return Ok(TC_ACT_OK)
 }else if mode==2{
  if plen<16{return Err(-2)}let head=ctx.load::<[u8;8]>(udp+8)?;logical=plen-16;
  if head[0]!=MAGIC[0] || head[1]!=MAGIC[1] || head[2]!=MAGIC[2] || head[3]!=MAGIC[3] || u32::from_be_bytes([head[4],head[5],head[6],head[7]])!=logical{return Err(-3)}
  let udp_header=ctx.load::<[u8;8]>(udp)?;
  ctx.adjust_room(-8,0,0)?;ctx.store(udp,&udp_header,0)?;count(13);if logical==0{count(15)}
 }else{return Err(-4)}
 let new_ip=(ilen-8) as u16;let new_udp=(plen-8) as u16;
 ctx.l3_csum_replace(ip+10,old_ip as u64,new_ip.to_be() as u64,2)?;ctx.store(ip+2,&new_ip.to_be(),0)?;ctx.store(udp+4,&new_udp.to_be(),0)?;ctx.store(udp+6,&0u16,0)?;
 if ctx.load::<u16>(udp+4)?!=new_udp.to_be(){return Err(-5)}
 if let Some(ptr)=WITNESS.get_ptr_mut(1){unsafe{let p=&mut *ptr;p.seq=p.seq.wrapping_add(1);p.len=ctx.len();p.mode=mode;let rc=bpf_skb_load_bytes(ctx.skb.skb as *const _,0,p.bytes.as_mut_ptr() as *mut _,42);if rc!=0{return Err(rc)}}}
 Ok(TC_ACT_OK)
}
#[panic_handler] fn panic(_: &core::panic::PanicInfo)->!{loop{}}
