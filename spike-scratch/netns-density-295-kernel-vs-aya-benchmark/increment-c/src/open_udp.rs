//! Independent send pacing and receive service; common to both forwarding variants.
use super::*;
use std::collections::{BTreeMap, VecDeque};
fn bytes(n: usize, cid: u32, seq: u64, reply: bool) -> Vec<u8> {
    let mut x: Vec<u8> = (0..n)
        .map(|j| {
            ((j * 31 + cid as usize + seq as usize) % 251) as u8 ^ if reply { 0xa5 } else { 0 }
        })
        .collect();
    if n >= 16 {
        x[..8].copy_from_slice(&seq.to_le_bytes());
        x[8..12].copy_from_slice(&cid.to_le_bytes());
        x[12..16].copy_from_slice(&[0x29, 0x5a, if reply { 2 } else { 1 }, 0]);
    }
    x
}
fn identity(x: &[u8], cid: u32, reply: bool) -> u64 {
    assert!(x.len() >= 16);
    assert_eq!(u32::from_le_bytes(x[8..12].try_into().unwrap()), cid);
    let seq = u64::from_le_bytes(x[..8].try_into().unwrap());
    assert_eq!(x, bytes(x.len(), cid, seq, reply));
    seq
}
fn consume_wire(o: &mut Owner, flags: u32, data: Vec<u8>) -> Vec<Vec<u8>> {
    o.stream.extend(data);
    let mut out = vec![];
    if o.kernel {
        loop {
            if o.stream.len() < 12 {
                break;
            }
            let n = u32::from_be_bytes(o.stream[8..12].try_into().unwrap()) as usize;
            assert!(n <= 60000);
            if o.stream.len() < 12 + n {
                break;
            }
            assert_eq!(o.stream[0], 10);
            let port = u16::from_be_bytes(o.stream[2..4].try_into().unwrap());
            let ip: [u8; 4] = o.stream[4..8].try_into().unwrap();
            assert_eq!(SocketAddr::from((ip, port)), o.app.addr());
            out.push(o.stream[12..12 + n].to_vec());
            o.stream.drain(..12 + n);
        }
    } else if flags & 1 != 0 {
        assert!(o.stream.len() >= 8);
        assert_eq!(&o.stream[..4], b"ZUD1");
        let n = u32::from_be_bytes(o.stream[4..8].try_into().unwrap()) as usize;
        assert_eq!(n, o.stream.len() - 8);
        out.push(o.stream[8..].to_vec());
        o.stream.clear();
    }
    out
}
pub(super) fn run(
    owners: Vec<Owner>,
    active: usize,
    size: usize,
    rate: u64,
    secs: f64,
    rep: usize,
    variant: &str,
) -> Vec<Owner> {
    let id = format!("{variant}-udp-open-{}-{active}-{size}-{rate}-{rep}", owners.len());
    let perfpath = format!("/tmp/perf-{id}.txt");
    let mut perf = Command::new("/perf")
        .args([
            "stat",
            "-a",
            "-e",
            "cpu-clock,task-clock,context-switches,cpu-migrations,page-faults,cycles,instructions",
            "-o",
            &perfpath,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
    thread::sleep(Duration::from_millis(30));
    event(json!({"event":"window_begin","id":id}));
    let before = read("/proc/stat");
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(secs);
    let mut shards: Vec<Vec<(usize, Owner)>> = (0..4).map(|_| vec![]).collect();
    for (i, o) in owners.into_iter().enumerate() {
        shards[i % 4].push((i, o));
    }
    let tasks:Vec<_>=shards.into_iter().map(|mut owners|thread::spawn(move||{
  let indices:Vec<_>=owners.iter().enumerate().filter(|(_,x)|x.0<active).map(|(i,_)|i).collect();let ep=unsafe{OwnedFd::from_raw_fd(libc::epoll_create1(0))};assert!(ep.as_raw_fd()>=0);
  let mut pending:Vec<BTreeMap<u64,Instant>>=(0..owners.len()).map(|_|BTreeMap::new()).collect();let mut zero:Vec<VecDeque<Instant>>=(0..owners.len()).map(|_|VecDeque::new()).collect();let mut participating=HashSet::new();
  for &i in &indices{let o=&mut owners[i].1;let App::Udp(app)=&o.app else{panic!()};app.set_nonblocking(true).unwrap();for(fd,token)in [(app.as_raw_fd(),i as u64*2),(o.p.rx_call_fd(),i as u64*2+1)]{let mut ev=libc::epoll_event{events:libc::EPOLLIN as u32,u64:token};assert_eq!(unsafe{libc::epoll_ctl(ep.as_raw_fd(),libc::EPOLL_CTL_ADD,fd,&mut ev)},0);}while let Some((op,_,_))=o.p.try_rx(Duration::ZERO){assert_eq!(op,6,"unexpected pre-window payload");}}
  let interval=if rate==0{Duration::ZERO}else{Duration::from_secs_f64(active.min(4)as f64/rate as f64)};let mut due=start;let mut cursor=0;let mut offered=0u64;let mut sent=0u64;let mut guest_received=0u64;let mut guest_sent=0u64;let mut app_received=0u64;let mut send_backpressure=0;let mut latency=vec![];let mut max_outstanding=0;let mut current_outstanding=0usize;let mut events=vec![libc::epoll_event{events:0,u64:0};256];let mut drain_deadline=None;
  if !indices.is_empty(){loop{let now=Instant::now();if now>=deadline&&drain_deadline.is_none(){drain_deadline=Some(now+Duration::from_secs(2));}if let Some(end)=drain_deadline{if current_outstanding==0||now>=end{break}}
   if now<deadline&&now>=due{let i=indices[cursor%indices.len()];let o=&mut owners[i].1;let seq=SEQUENCE.fetch_add(1,Ordering::Relaxed);let data=bytes(size,o.p.cid,seq,false);let App::Udp(app)=&o.app else{panic!()};offered+=1;match app.send_to(&data,o.local){Ok(n)=>{assert_eq!(n,size);sent+=1;participating.insert(o.p.cid);if size==0{zero[i].push_back(Instant::now());}else{pending[i].insert(seq,Instant::now());}current_outstanding+=1;max_outstanding=max_outstanding.max(current_outstanding);},Err(e)if e.kind()==io::ErrorKind::WouldBlock=>send_backpressure+=1,Err(e)=>panic!("send {e}")};cursor+=1;due+=interval;}
   let wait=if Instant::now()<deadline&&rate>0{due.saturating_duration_since(Instant::now()).as_millis().min(1)as i32}else{0};let ready=unsafe{libc::epoll_wait(ep.as_raw_fd(),events.as_mut_ptr(),events.len()as i32,wait)};assert!(ready>=0||io::Error::last_os_error().kind()==io::ErrorKind::Interrupted);for ev in events.iter().take(ready.max(0)as usize){let token=ev.u64;let i=(token/2)as usize;let o=&mut owners[i].1;
    if token&1!=0{unsafe{let mut n=0u64;libc::read(o.p.rx_call_fd(),&mut n as *mut _ as *mut _,8);}while let Some((op,flags,data))=o.p.try_rx(Duration::ZERO){if op==6{continue}assert_eq!(op,5,"unexpected OP in open UDP");for data in consume_wire(o,flags,data){assert_eq!(data.len(),size);let seq=if size==0{0}else{identity(&data,o.p.cid,false)};guest_received+=1;let reply=bytes(size,o.p.cid,seq,true);o.guest_send(&reply);guest_sent+=1;}}}
    else{let App::Udp(app)=&o.app else{panic!()};let mut data=vec![0;65536];loop{match app.recv_from(&mut data){Ok((n,src))=>{assert_eq!(src,o.local);assert_eq!(n,size);if size==0{assert!(zero[i].pop_front().is_some());}else{let seq=identity(&data[..n],o.p.cid,true);let began=pending[i].remove(&seq).expect("duplicate/unoffered reply identity");latency.push(began.elapsed().as_nanos()as u64);}app_received+=1;current_outstanding-=1;},Err(e)if e.kind()==io::ErrorKind::WouldBlock=>break,Err(e)=>panic!("recv {e}")}}}
   }
  }}
  // All timed sends have stopped. Any unresolved message is recorded as loss;
  // receive cleanup stays bounded and never creates a payload relay.
  for &i in &indices{let App::Udp(app)=&owners[i].1.app else{panic!()};app.set_nonblocking(false).unwrap();}
  (owners,latency,json!({"offered":offered,"sent":sent,"guest_received":guest_received,"guest_sent":guest_sent,"app_received":app_received,"send_backpressure":send_backpressure,"participating_peers":participating.len(),"maximum_inflight_datagrams":max_outstanding,"unresolved_after_two_second_drain":current_outstanding}))
 })).collect();
    let mut returned = vec![];
    let mut samples = vec![];
    let mut actors = vec![];
    for t in tasks {
        let (o, l, s) = t.join().unwrap();
        returned.extend(o);
        samples.extend(l);
        actors.push(s)
    }
    let wall = start.elapsed().as_secs_f64();
    let after = read("/proc/stat");
    event(json!({"event":"window_end","id":id}));
    if let Some(p) = perf.as_mut() {
        unsafe {
            libc::kill(p.id() as i32, libc::SIGINT);
        }
        let _ = p.wait();
    }
    samples.sort_unstable();
    let sum = |k: &str| actors.iter().map(|v| v[k].as_u64().unwrap()).sum::<u64>();
    let q = |p: f64| samples.get(((samples.len().saturating_sub(1)) as f64 * p) as usize).copied();
    event(
        json!({"event":"udp_open_window","id":id,"variant":variant,"population":returned.len(),"eligible_active":active,"participating_peers":sum("participating_peers"),"application_size":size,"offered_target_per_s":rate,"unrestricted":rate==0,"repeat":rep,"offering_duration_s":secs,"total_delivery_duration_s":wall,"offered":sum("offered"),"sent":sum("sent"),"guest_delivered":sum("guest_received"),"guest_sent":sum("guest_sent"),"app_delivered":sum("app_received"),"host_to_guest_loss":sum("sent")-sum("guest_received"),"guest_to_host_loss":sum("guest_sent")-sum("app_received"),"send_backpressure":sum("send_backpressure"),"maximum_inflight_datagrams":sum("maximum_inflight_datagrams"),"unresolved_after_drain":sum("unresolved_after_two_second_drain"),"zero_latency_count_only":size==0,"latency_ns":samples,"p50_ns":q(0.5),"p95_ns":q(0.95),"p99_ns":q(0.99),"cpu_before":before,"cpu_after":after,"raw_perf":read(&perfpath),"actors":actors,"corrupt":0,"boundary_violations":0}),
    );
    returned.sort_by_key(|x| x.0);
    returned.into_iter().map(|x| x.1).collect()
}
