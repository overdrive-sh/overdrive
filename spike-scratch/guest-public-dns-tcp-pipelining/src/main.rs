//! Spike harness: the PRODUCTION DNS responder (HostGuestDnsFactory::host()) bound to
//! 100.95.0.1:53 inside a private netns, with a scripted upstream at 127.0.0.2:53.
//! usage: harness <out-dir> <scripted|tc> <seconds-to-serve>
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use overdrive_control_plane::dns_responder::frontend_addr_allocator::FrontendAddrAllocator;
use overdrive_control_plane::dns_responder::owned_ranges::OwnedAddressRanges;
use overdrive_control_plane::dns_responder::{GuestDnsDeps, GuestDnsFactory, HostGuestDnsFactory};
use overdrive_core::id::NodeId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_dataplane::allocators::VipRange;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_testing::dns_upstream::{Behaviour, ScriptedDnsUpstream};

const GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
const UPSTREAM: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);

fn parse_q(q: &[u8]) -> Option<(usize, u16)> {
    let mut at = 12;
    loop {
        let l = *q.get(at)? as usize;
        at += 1;
        if l == 0 { break; }
        at += l;
    }
    Some((at + 4, u16::from_be_bytes([*q.get(at)?, *q.get(at + 1)?])))
}

/// reply: tc=true -> header only, TC set, no answers (UDP); else A -> 1 record, AAAA -> empty NOERROR.
fn reply(q: &[u8], tc: bool, log: &Mutex<Vec<String>>, via: &str) -> Option<Vec<u8>> {
    let (end, qtype) = parse_q(q)?;
    log.lock().unwrap().push(format!("{via} qtype={qtype} tc={tc}"));
    let mut r = Vec::new();
    r.extend_from_slice(&q[0..2]);
    r.push(0x80 | (q[2] & 1) | if tc { 0x02 } else { 0 });
    r.push(0x80);
    r.extend_from_slice(&1u16.to_be_bytes());
    let an: u16 = if !tc && qtype == 1 { 1 } else { 0 };
    r.extend_from_slice(&an.to_be_bytes());
    r.extend_from_slice(&[0, 0, 0, 0]);
    r.extend_from_slice(&q[12..end]);
    if an == 1 { r.extend_from_slice(&[0xC0, 0x0C, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 93, 184, 216, 34]); }
    Some(r)
}

/// An upstream whose UDP replies are always truncated (TC) and whose TCP serves many queries per connection.
fn tc_upstream(log: Arc<Mutex<Vec<String>>>) {
    let udp = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).unwrap();
    udp.set_reuse_address(true).unwrap();
    udp.bind(&SocketAddrV4::new(UPSTREAM, 53).into()).unwrap();
    let udp: std::net::UdpSocket = udp.into();
    let l1 = log.clone();
    std::thread::spawn(move || {
        let mut b = [0u8; 1500];
        while let Ok((n, p)) = udp.recv_from(&mut b) {
            if let Some(r) = reply(&b[..n], true, &l1, "upstream-udp") { let _ = udp.send_to(&r, p); }
        }
    });
    let tcp = TcpListener::bind(SocketAddrV4::new(UPSTREAM, 53)).unwrap();
    std::thread::spawn(move || {
        for s in tcp.incoming().flatten() {
            let log = log.clone();
            std::thread::spawn(move || {
                let mut s = s;
                loop {
                    let mut l = [0u8; 2];
                    if s.read_exact(&mut l).is_err() { return; }
                    let mut q = vec![0u8; u16::from_be_bytes(l) as usize];
                    if s.read_exact(&mut q).is_err() { return; }
                    if let Some(r) = reply(&q, false, &log, "upstream-tcp") {
                        let _ = s.write_all(&(r.len() as u16).to_be_bytes());
                        let _ = s.write_all(&r);
                    }
                }
            });
        }
    });
}

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    let out = std::path::PathBuf::from(&a[1]);
    let mode = a[2].clone();
    let secs: u64 = a[3].parse().unwrap();
    let store = Arc::new(SimObservationStore::single_peer(NodeId::new("local").unwrap(), 0));
    let frontend = FrontendAddrAllocator::new();
    let deps = GuestDnsDeps {
        store: store as Arc<dyn ObservationStore>,
        clock: Arc::new(SimClock::new()) as Arc<dyn Clock>,
        gateway: GATEWAY,
        frontend,
        owned_ranges: OwnedAddressRanges::platform(&VipRange::default()),
        forward_permits: 64,
    };
    let dns = HostGuestDnsFactory::host().responder(deps);
    dns.probe().await.expect("probe binds the responder sockets");
    tokio::spawn(Arc::clone(&dns).serve());
    let log = Arc::new(Mutex::new(Vec::new()));
    let scripted;
    if mode == "tc" {
        tc_upstream(log.clone());
        scripted = None;
    } else {
        let u = ScriptedDnsUpstream::start(UPSTREAM).expect("upstream");
        u.script_default(Behaviour::Answer(vec![Ipv4Addr::new(93, 184, 216, 34)]));
        scripted = Some(u);
    }
    std::fs::write(out.join("ready"), "ok").unwrap();
    println!("harness: responder up on {GATEWAY}:53, upstream mode={mode}");
    let started = std::time::Instant::now();
    while !out.join("stop").exists() && started.elapsed() < Duration::from_secs(secs) {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let mut seen = log.lock().unwrap().clone();
    if let Some(u) = &scripted {
        for q in u.seen() { seen.push(format!("upstream-{:?} name={} qtype={}", q.transport, q.name, q.qtype)); }
    }
    std::fs::write(out.join("upstream-seen.txt"), seen.join("\n")).unwrap();
    println!("harness: upstream saw {} queries", seen.len());
    dns.stop();
    std::process::exit(0);
}
