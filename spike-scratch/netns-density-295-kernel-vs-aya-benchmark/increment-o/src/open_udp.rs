//! Independent send pacing and receive service; common to both forwarding variants.
use super::*;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::AtomicUsize;
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
fn consume_wire(o: &mut Owner, _flags: u32, data: Vec<u8>) -> Vec<Vec<u8>> {
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
    } else {
        loop {
            if o.stream.len() < 8 { break; }
            assert_eq!(&o.stream[..4], b"ZUD1");
            let n = u32::from_be_bytes(o.stream[4..8].try_into().unwrap()) as usize;
            assert!(n <= 59000);
            if o.stream.len() < 8+n { break; }
            out.push(o.stream[8..8+n].to_vec());
            o.stream.drain(..8+n);
        }
    }
    out
}
fn cpu_usage() -> libc::rusage {
    let mut x = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_THREAD, &mut x) }, 0);
    x
}
fn cpu_seconds(a: libc::timeval, b: libc::timeval) -> f64 {
    (b.tv_sec - a.tv_sec) as f64 + (b.tv_usec - a.tv_usec) as f64 / 1e6
}
fn drain_ready(
    owners: &mut [(usize, Owner)],
    events: &[libc::epoll_event],
    reply: bool,
    size: usize,
    pending: &mut [BTreeMap<u64, Instant>],
    zero: &mut [VecDeque<Instant>],
    samples: &mut Vec<u64>,
    counts: &mut [u64; 4],
    outstanding: &AtomicUsize,
    peers: &AtomicUsize,
    epoch: u64,
) -> u64 {
    let mut late = 0;
    for ev in events {
        let token = ev.u64;
        let i = (token / 2) as usize;
        let o = &mut owners[i].1;
        if token & 1 != 0 {
            unsafe {
                let mut n = 0u64;
                libc::read(o.p.rx_call_fd(), &mut n as *mut _ as *mut _, 8);
            }
            while let Some((op, flags, data)) = o.p.try_rx(Duration::ZERO) {
                if op == 6 {
                    continue;
                }
                assert_eq!(op, 5, "unexpected vhost op during UDP load");
                for data in consume_wire(o, flags, data) {
                    if !reply {
                        if !data.is_empty() {
                            identity(&data, o.p.cid, false);
                        }
                        late += 1;
                        continue;
                    }
                    let seq = if data.is_empty() { 0 } else { identity(&data, o.p.cid, false) };
                    if !data.is_empty() && seq < epoch {
                        late += 1;
                        continue;
                    }
                    assert_eq!(data.len(), size);
                    counts[0] += 1;
                    o.guest_send(&bytes(size, o.p.cid, seq, true));
                    counts[1] += 1;
                }
            }
        } else {
            let App::Udp(app) = &o.app else { panic!() };
            let mut data = vec![0; 65536];
            loop {
                match app.recv_from(&mut data) {
                    Ok((n, src)) => {
                        assert_eq!(src, o.local);
                        if !reply {
                            if n > 0 {
                                identity(&data[..n], o.p.cid, true);
                            }
                            late += 1;
                            continue;
                        }
                        let oldseq =
                            if n > 0 { Some(identity(&data[..n], o.p.cid, true)) } else { None };
                        if oldseq.is_some_and(|s| s < epoch) {
                            late += 1;
                            continue;
                        }
                        assert_eq!(n, size);
                        if size == 0 {
                            assert!(zero[i].pop_front().is_some());
                        } else {
                            let seq = identity(&data[..n], o.p.cid, true);
                            if seq < epoch {
                                late += 1;
                                continue;
                            }
                            let began=pending[i].remove(&seq).unwrap_or_else(||panic!("duplicate/unoffered same-window reply cid={} nonce={seq} epoch={epoch}",o.p.cid));
                            samples.push(began.elapsed().as_nanos() as u64);
                        }
                        counts[2] += 1;
                        outstanding.fetch_sub(1, Ordering::Relaxed);
                        if zero[i].is_empty() && pending[i].is_empty() {
                            peers.fetch_sub(1, Ordering::Relaxed);
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => panic!("UDP receive: {e}"),
                }
            }
        }
    }
    late
}
/// Endpoint queues settle outside the timed interval. Late validated old messages
/// are discarded by their owning consumer and retained as boundary telemetry.
fn settle(owners: &mut [(usize, Owner)], indices: &[usize], ep: i32) -> u64 {
    let mut fenced_late = 0u64;
    for &i in indices {
        let o = &mut owners[i].1;
        let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let fence = bytes(64, o.p.cid, seq, false);
        let App::Udp(app) = &o.app else { panic!() };
        assert_eq!(app.send_to(&fence, o.local).unwrap(), 64);
        let until = Instant::now() + Duration::from_secs(5);
        let mut found = false;
        while !found {
            assert!(Instant::now() < until, "guest endpoint epoch fence timeout");
            let (op, flags, data) =
                o.p.try_rx(Duration::from_millis(100)).unwrap_or((6, 0, vec![]));
            if op == 6 {
                continue;
            }
            assert_eq!(op, 5);
            for data in consume_wire(o, flags, data) {
                if !data.is_empty() {
                    let actual = identity(&data, o.p.cid, false);
                    if actual == seq {
                        assert_eq!(data.len(), 64);
                        found = true;
                        break;
                    }
                }
                fenced_late += 1;
            }
        }
        o.guest_send(&bytes(64, o.p.cid, seq, true));
        let App::Udp(app) = &o.app else { panic!() };
        let mut data = vec![0; 65536];
        loop {
            assert!(Instant::now() < until, "ordinary endpoint epoch fence timeout");
            match app.recv_from(&mut data) {
                Ok((n, src)) => {
                    assert_eq!(src, o.local);
                    if n > 0 && identity(&data[..n], o.p.cid, true) == seq {
                        assert_eq!(n, 64);
                        break;
                    }
                    fenced_late += 1;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_micros(20))
                }
                Err(e) => panic!("endpoint epoch fence {e}"),
            }
        }
    }
    let mut events = vec![libc::epoll_event { events: 0, u64: 0 }; 256];
    let mut ignored_pending: Vec<BTreeMap<u64, Instant>> =
        (0..owners.len()).map(|_| BTreeMap::new()).collect();
    let mut ignored_zero: Vec<VecDeque<Instant>> =
        (0..owners.len()).map(|_| VecDeque::new()).collect();
    let mut ignored_samples = vec![];
    let mut counts = [0; 4];
    let mut late = fenced_late;
    let until = Instant::now() + Duration::from_secs(10);
    let mut quiet = Instant::now();
    for &i in indices {
        let o = &mut owners[i].1;
        while let Some((op, flags, data)) = o.p.try_rx(Duration::ZERO) {
            if op == 6 {
                continue;
            }
            assert_eq!(op, 5);
            for data in consume_wire(o, flags, data) {
                if !data.is_empty() {
                    identity(&data, o.p.cid, false);
                }
                late += 1;
                quiet = Instant::now();
            }
        }
    }
    loop {
        let n = unsafe { libc::epoll_wait(ep, events.as_mut_ptr(), events.len() as i32, 20) };
        assert!(n >= 0);
        let received = drain_ready(
            owners,
            &events[..n as usize],
            false,
            0,
            &mut ignored_pending,
            &mut ignored_zero,
            &mut ignored_samples,
            &mut counts,
            &AtomicUsize::new(0),
            &AtomicUsize::new(0),
            0,
        );
        late += received;
        if received > 0 {
            quiet = Instant::now();
        }
        if quiet.elapsed() >= Duration::from_millis(200) {
            return late;
        }
        assert!(Instant::now() < until, "endpoint queues did not settle within 10s");
    }
}
pub(super) fn run(
    owners: Vec<Owner>,
    active: usize,
    size: usize,
    rate: u64,
    secs: f64,
    rep: usize,
    variant: &str,
) -> (Vec<Owner>, bool) {
    use std::sync::{mpsc, Arc};
    let id = format!("{variant}-udp-open-{}-{active}-{size}-{rate}-{rep}", owners.len());
    let perfpath = format!("/tmp/perf-{id}.txt");
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let outstanding = Arc::new(AtomicUsize::new(0));
    let max_outstanding = Arc::new(AtomicUsize::new(0));
    let peers = Arc::new(AtomicUsize::new(0));
    let max_peers = Arc::new(AtomicUsize::new(0));
    let all_peer_burst = active > 64 && active == owners.len();
    let mut shards: Vec<Vec<(usize, Owner)>> = (0..4).map(|_| vec![]).collect();
    for (i, o) in owners.into_iter().enumerate() {
        shards[i % 4].push((i, o));
    }
    let (event_tx, event_rx) = mpsc::channel::<(usize, &'static str, String)>();
    let mut starts = vec![];
    let mut releases = vec![];
    let burst_ready = std::sync::Arc::new(AtomicUsize::new(0));
    let mut tasks = vec![];
    for (index, mut owners) in shards.into_iter().enumerate() {
        let (start_tx, start_rx) = mpsc::channel::<Option<Instant>>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        starts.push(start_tx);
        releases.push(release_tx);
        let tx = event_tx.clone();
        let cancel = cancel.clone();
        let burst_ready = burst_ready.clone();
        let outstanding = outstanding.clone();
        let max_outstanding = max_outstanding.clone();
        let peers = peers.clone();
        let max_peers = max_peers.clone();
        tasks.push(thread::spawn(move||{let indices:Vec<_>=owners.iter().enumerate().filter(|(_,x)|x.0<active).map(|(i,_)|i).collect();let mut samples=vec![];let mut metadata=json!({});
  let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
   let ep=unsafe{OwnedFd::from_raw_fd(libc::epoll_create1(0))};assert!(ep.as_raw_fd()>=0);
   for &i in &indices{let o=&mut owners[i].1;let App::Udp(app)=&o.app else{panic!()};app.set_nonblocking(true).unwrap();for(fd,token)in[(app.as_raw_fd(),i as u64*2),(o.p.rx_call_fd(),i as u64*2+1)]{let mut ev=libc::epoll_event{events:libc::EPOLLIN as u32,u64:token};assert_eq!(unsafe{libc::epoll_ctl(ep.as_raw_fd(),libc::EPOLL_CTL_ADD,fd,&mut ev)},0);}}
   let late_before=settle(&mut owners,&indices,ep.as_raw_fd());tx.send((index,"ready",String::new())).unwrap();let Some(start)=start_rx.recv().unwrap()else{return};let deadline=start+Duration::from_secs_f64(secs);let epoch=SEQUENCE.load(Ordering::Relaxed);let u0=cpu_usage();let mut pending:Vec<BTreeMap<u64,Instant>>=(0..owners.len()).map(|_|BTreeMap::new()).collect();let mut zero:Vec<VecDeque<Instant>>=(0..owners.len()).map(|_|VecDeque::new()).collect();let mut participating=HashSet::new();let mut offered=0u64;let mut sent=0u64;let mut blocked=0u64;let mut counts=[0;4];let mut late_during=0u64;let interval=if rate==0{Duration::ZERO}else{Duration::from_secs_f64(active.min(4)as f64/rate as f64)};let mut due=start;let mut cursor=0;let mut burst_left=if all_peer_burst{indices.len()}else{0};let mut burst_announced=false;let mut drain_end=None;let mut events=vec![libc::epoll_event{events:0,u64:0};256];
   if !indices.is_empty(){loop{if cancel.load(Ordering::Relaxed){break}let now=Instant::now();if now>=deadline&&burst_left==0&&drain_end.is_none(){drain_end=Some(now+Duration::from_secs(2));}if let Some(end)=drain_end{let pending_count=(sent-counts[2])as usize;if pending_count==0||now>=end{break}}
    if burst_left>0||(now<deadline&&now>=due){let i=indices[cursor%indices.len()];let o=&mut owners[i].1;let seq=SEQUENCE.fetch_add(1,Ordering::Relaxed);let data=bytes(size,o.p.cid,seq,false);let App::Udp(app)=&o.app else{panic!()};offered+=1;let began=Instant::now();match app.send_to(&data,o.local){Ok(n)=>{assert_eq!(n,size);sent+=1;participating.insert(o.p.cid);let first=pending[i].is_empty()&&zero[i].is_empty();if size==0{zero[i].push_back(began);}else{pending[i].insert(seq,began);}if first{let n=peers.fetch_add(1,Ordering::Relaxed)+1;max_peers.fetch_max(n,Ordering::Relaxed);}let n=outstanding.fetch_add(1,Ordering::Relaxed)+1;max_outstanding.fetch_max(n,Ordering::Relaxed);},Err(e)if e.kind()==io::ErrorKind::WouldBlock=>blocked+=1,Err(e)=>panic!("UDP send: {e}")};cursor+=1;due+=interval;if burst_left>0{burst_left-=1;}}
    if all_peer_burst&&!burst_announced&&burst_left==0{burst_announced=true;burst_ready.fetch_add(1,Ordering::SeqCst);while burst_ready.load(Ordering::SeqCst)<4&&!cancel.load(Ordering::Relaxed){thread::yield_now();}}
    if burst_left>0{continue;}
    let n=unsafe{libc::epoll_wait(ep.as_raw_fd(),events.as_mut_ptr(),events.len()as i32,0)};assert!(n>=0);late_during+=drain_ready(&mut owners,&events[..n as usize],true,size,&mut pending,&mut zero,&mut samples,&mut counts,&outstanding,&peers,epoch);
   }}
   let u1=cpu_usage();let unresolved=pending.iter().map(BTreeMap::len).sum::<usize>()+zero.iter().map(VecDeque::len).sum::<usize>();metadata=json!({"offered":offered,"sent":sent,"guest_received":counts[0],"guest_sent":counts[1],"app_received":counts[2],"send_backpressure":blocked,"participating_peers":participating.len(),"unresolved_after_two_second_drain":unresolved,"late_discarded_before_window":late_before,"late_previous_window_during_interval":late_during,"actor_user_cpu_s":cpu_seconds(u0.ru_utime,u1.ru_utime),"actor_system_cpu_s":cpu_seconds(u0.ru_stime,u1.ru_stime)});tx.send((index,"done",String::new())).unwrap();
  }));
  let error=if let Err(e)=&result{cancel.store(true,Ordering::Relaxed);let message=e.downcast_ref::<String>().cloned().or_else(||e.downcast_ref::<&str>().map(|s|s.to_string())).unwrap_or_else(||"non-string actor panic".into());let _=tx.send((index,"failed",message.clone()));Some(message)}else{None};let _=release_rx.recv();
  for &i in &indices{let App::Udp(app)=&owners[i].1.app else{panic!()};app.set_nonblocking(false).unwrap();}(owners,samples,metadata,error)
 }));
    }
    drop(event_tx);
    let mut ready = HashSet::new();
    let mut completed = HashSet::new();
    let mut failures = vec![];
    while ready.len() + completed.len() < 4 {
        let (i, state, reason) = event_rx
            .recv_timeout(Duration::from_secs(20))
            .expect("actor preparation status timeout");
        if state == "ready" {
            ready.insert(i);
        } else {
            completed.insert(i);
            failures.push(reason);
            cancel.store(true, Ordering::Relaxed);
        }
    }
    let mut perf = None;
    let mut before = String::new();
    let mut after = String::new();
    let mut wall = 0.0;
    if failures.is_empty() {
        perf=Command::new("/perf").args(["stat","-a","-e","cpu-clock,task-clock,context-switches,cpu-migrations,page-faults,cycles,instructions","-o",&perfpath]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok();
        thread::sleep(Duration::from_millis(30));
        event(json!({"event":"window_begin","id":id}));
        before = read("/proc/stat");
        let start = Instant::now();
        for tx in &starts {
            let _ = tx.send(Some(start));
        }
        while completed.len() < 4 {
            let (i, state, reason) = event_rx
                .recv_timeout(Duration::from_secs_f64(secs + 12.0))
                .expect("actor completion status timeout");
            if state == "done" {
                completed.insert(i);
            } else if state == "failed" {
                completed.insert(i);
                failures.push(reason);
                cancel.store(true, Ordering::Relaxed);
            }
        }
        wall = start.elapsed().as_secs_f64();
        after = read("/proc/stat");
        event(json!({"event":"window_end","id":id}));
    } else {
        for tx in &starts {
            let _ = tx.send(None);
        }
    }
    if let Some(p) = perf.as_mut() {
        unsafe {
            libc::kill(p.id() as i32, libc::SIGINT);
        }
        let _ = p.wait();
    }
    for tx in releases {
        let _ = tx.send(());
    }
    let mut returned = vec![];
    let mut samples = vec![];
    let mut actors = vec![];
    for task in tasks {
        let (o, l, s, error) = task.join().unwrap();
        returned.extend(o);
        samples.extend(l);
        if let Some(error) = error {
            if !failures.contains(&error) {
                failures.push(error)
            }
        }
        actors.push(s);
    }
    let valid = failures.is_empty()
        && actors.iter().all(|a| a["late_previous_window_during_interval"].as_u64() == Some(0));
    let sum = |k: &str| actors.iter().map(|a| a[k].as_u64().unwrap_or(0)).sum::<u64>();
    samples.sort_unstable();
    let q = |p: f64| samples.get(((samples.len().saturating_sub(1)) as f64 * p) as usize).copied();
    let base = samples.first().copied().unwrap_or(0);
    let deltas: Vec<u64> = samples.windows(2).map(|s| s[1] - s[0]).collect();
    event(
        json!({"event":if failures.is_empty(){"udp_open_window"}else{"udp_window_fixture_failure"},"id":id,"variant":variant,"population":returned.len(),"eligible_active":active,"initial_all_peer_burst":all_peer_burst,"all_peer_burst_service_barrier":all_peer_burst,"participating_peers":sum("participating_peers"),"application_size":size,"offered_target_per_s":rate,"unrestricted":rate==0,"repeat":rep,"offering_duration_s":secs,"total_delivery_duration_s":wall,"offered":sum("offered"),"sent":sum("sent"),"guest_delivered":sum("guest_received"),"guest_sent":sum("guest_sent"),"app_delivered":sum("app_received"),"host_to_guest_loss":sum("sent").saturating_sub(sum("guest_received")),"guest_to_host_loss":sum("guest_sent").saturating_sub(sum("app_received")),"send_backpressure":sum("send_backpressure"),"maximum_inflight_datagrams":max_outstanding.load(Ordering::Relaxed),"maximum_inflight_peers":max_peers.load(Ordering::Relaxed),"late_discarded_before_window":sum("late_discarded_before_window"),"late_previous_window_during_interval":sum("late_previous_window_during_interval"),"valid_timing_comparison":valid,"failures":failures,"actor_user_cpu_s":actors.iter().map(|a|a["actor_user_cpu_s"].as_f64().unwrap_or(0.0)).sum::<f64>(),"actor_system_cpu_s":actors.iter().map(|a|a["actor_system_cpu_s"].as_f64().unwrap_or(0.0)).sum::<f64>(),"unresolved_after_drain":sum("unresolved_after_two_second_drain"),"zero_latency_count_only":size==0,"latency_ns_encoding":"sorted exact nanoseconds, base plus successive deltas","latency_ns_base":base,"latency_ns_deltas":deltas,"latency_sample_count":samples.len(),"p50_ns":q(0.5),"p95_ns":q(0.95),"p99_ns":q(0.99),"cpu_before":before,"cpu_after":after,"raw_perf":read(&perfpath),"actors":actors,"corrupt":0,"boundary_violations":0}),
    );
    returned.sort_by_key(|o| o.0);
    (returned.into_iter().map(|o| o.1).collect(), valid)
}
