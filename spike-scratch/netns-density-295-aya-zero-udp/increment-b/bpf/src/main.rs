#![no_std]
#![no_main]
use aya_ebpf::{bindings::{TC_ACT_OK,TC_ACT_SHOT},helpers::{bpf_get_socket_cookie,bpf_skb_change_tail},macros::{map,stream_verdict,classifier},maps::{Array,HashMap,PerCpuArray,SockMap},programs::{SkBuffContext,TcContext}};
#[repr(C)] #[derive(Clone,Copy)] pub struct Tuple { pub src:u32,pub dst:u32,pub sport:u16,pub dport:u16 }
#[map] static SOCKETS:SockMap=SockMap::with_max_entries(16,0);
#[map] static ROUTES:HashMap<u64,u32>=HashMap::with_max_entries(16,0);
#[map] static FLOWS:HashMap<Tuple,u32>=HashMap::with_max_entries(16,0);
#[map] static SERVER:Array<u32>=Array::with_max_entries(1,0);
#[map] static COUNTERS:PerCpuArray<u64>=PerCpuArray::with_max_entries(32,0);
fn count(i:u32){if let Some(p)=COUNTERS.get_ptr_mut(i){unsafe{*p=(*p).wrapping_add(1)}}}
const MAGIC:[u8;4]=*b"ZUD1";
#[stream_verdict] pub fn route(ctx:SkBuffContext)->u32 {
 let cookie=unsafe{bpf_get_socket_cookie(ctx.skb.skb as *mut _)};
 let Some(key)=(unsafe{ROUTES.get(&cookie)}) else{count(8);return 0};let key=*key;count(key);
 if ctx.pull_data(ctx.len()).is_err(){count(9);return 0}
 let n=ctx.len();if n<8{count(10);return 0}
 let Ok(footer)=ctx.load::<[u8;8]>((n-8) as usize) else{count(10);return 0};
 if footer[0]!=MAGIC[0] || footer[1]!=MAGIC[1] || footer[2]!=MAGIC[2] || footer[3]!=MAGIC[3] || u32::from_be_bytes([footer[4],footer[5],footer[6],footer[7]])!=n-8{count(10);return 0}
 let result=unsafe{SOCKETS.redirect_skb(&ctx,key,0) as u32};if result==0{count(11)}result
}
#[classifier] pub fn packet(ctx:TcContext)->i32 {match transform(ctx){Ok(x)=>x,Err(_)=>{count(20);TC_ACT_SHOT}}}
fn transform(mut ctx:TcContext)->Result<i32,i64>{
 let ip=14usize;let udp=34usize;
 if ctx.load::<u8>(ip)?!=0x45 || ctx.load::<u8>(ip+9)?!=17{return Ok(TC_ACT_OK)}
 let src=ctx.load::<u32>(ip+12)?;let dst=ctx.load::<u32>(ip+16)?;let sport=ctx.load::<u16>(udp)?;let dport=ctx.load::<u16>(udp+2)?;let tuple=Tuple{src,dst,sport,dport};
 let server=unsafe{SERVER.get(0).copied().unwrap_or(0)} as u16;
 let proxy=u32::from_ne_bytes([127,0,0,1]);let peer2=u32::from_ne_bytes([127,0,0,2]);let peer3=u32::from_ne_bytes([127,0,0,3]);
 let owned=(dst==proxy && (src==peer2 || src==peer3) && dport==server) || (src==proxy && (dst==peer2 || dst==peer3) && sport==server);
 if !owned{return Ok(TC_ACT_OK)}
 let Some(mode)=(unsafe{FLOWS.get(&tuple).copied()}) else{count(19);return Ok(TC_ACT_SHOT)};
 let old_ip=ctx.load::<u16>(ip+2)?;let old_udp=ctx.load::<u16>(udp+4)?;let plen=u16::from_be(old_udp) as u32;let ilen=u16::from_be(old_ip) as u32;
 if plen<8 || ilen!=plen+20 || ctx.len()!=ilen+14{return Err(-1)}
 let logical;if mode==1 {
  logical=plen-8;let tail=[MAGIC[0],MAGIC[1],MAGIC[2],MAGIC[3],(logical>>24) as u8,(logical>>16) as u8,(logical>>8) as u8,logical as u8];
  let end=ctx.len();let rc=unsafe{bpf_skb_change_tail(ctx.skb.skb,end+8,0)};if rc!=0{return Err(rc)}ctx.store(end as usize,&tail,0)?;count(12);if logical==0{count(14)}
 }else if mode==2{
  if plen<16{return Err(-2)}let tail=ctx.load::<[u8;8]>((ctx.len()-8) as usize)?;logical=plen-16;
  if tail[0]!=MAGIC[0] || tail[1]!=MAGIC[1] || tail[2]!=MAGIC[2] || tail[3]!=MAGIC[3] || u32::from_be_bytes([tail[4],tail[5],tail[6],tail[7]])!=logical{return Err(-3)}
  let rc=unsafe{bpf_skb_change_tail(ctx.skb.skb,ctx.len()-8,0)};if rc!=0{return Err(rc)}count(13);if logical==0{count(15)}
 }else{return Err(-4)}
 let new_ip=if mode==1{(ilen+8) as u16}else{(ilen-8) as u16};let new_udp=if mode==1{(plen+8) as u16}else{(plen-8) as u16};
 ctx.l3_csum_replace(ip+10,old_ip as u64,new_ip.to_be() as u64,2)?;ctx.store(ip+2,&new_ip.to_be(),0)?;ctx.store(udp+4,&new_udp.to_be(),0)?;ctx.store(udp+6,&0u16,0)?;
 if ctx.load::<u16>(udp+4)?!=new_udp.to_be(){return Err(-5)}Ok(TC_ACT_OK)
}
#[panic_handler] fn panic(_: &core::panic::PanicInfo)->!{loop{}}
