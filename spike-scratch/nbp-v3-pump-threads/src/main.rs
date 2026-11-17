//! V-3 probe. Drives the PRODUCTION `HostMtlsEnforcement::enforce(Routed::Outbound)` (the call the
//! worker's egress path makes; it spawns the two outbound pump threads in
//! crates/overdrive-dataplane/src/mtls/splice.rs: spawn_encrypt / spawn_decrypt) with a plaintext
//! workload-side client on leg F and a rustls mesh peer in a CHILD process, for a sustained
//! single-connection transfer, forward-heavy then return-heavy, and samples
//! /proc/self/task/<tid>/stat at the window edges.
//!
//! Composition: NOT the full worker egress rig (`MtlsInterceptWorker` + `HostMtlsIntercept` +
//! `start_alloc`, vsock_mesh_egress_mtls.rs). It is the narrower shape of
//! crates/overdrive-dataplane/tests/integration/mtls_guardrails.rs:125-380 (AC5: outbound `enforce`
//! over a loopback peer). Both reach the same `spawn_*` sites, so the pump-thread accounting is the
//! same instrument; what the narrower rig omits is the worker's own leg-F accept loop.
//! Helpers included BY PATH, not edited: helpers/mtls_pki.rs, helpers/guest_dial.rs.
//!
//! Usage:  nbp-v3-pump-threads run <secs> <replicates> <out.csv>
//!         nbp-v3-pump-threads peer <dir> <sink|source>      (child; internal)
use std::io::{Read, Write};
use std::net::{SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::OwnedFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::mtls_enforcement::{
    InterceptedConnection, MtlsEnforcement, MtlsLimits, Routed,
};
use overdrive_core::traits::IdentityRead;
use overdrive_core::AllocationId;
use overdrive_dataplane::mtls::HostMtlsEnforcement;

#[path = "../../../crates/overdrive-dataplane/tests/integration/helpers/guest_dial.rs"]
#[allow(dead_code, unused)]
mod guest_dial;
#[path = "../../../crates/overdrive-dataplane/tests/integration/helpers/mtls_pki.rs"]
#[allow(dead_code, unused)]
mod mtls_pki;
use mtls_pki::TestPki;

struct HeldIdentities {
    svids: std::collections::BTreeMap<AllocationId, SvidMaterial>,
    bundle: Option<TrustBundle>,
}
impl IdentityRead for HeldIdentities {
    fn svid_for(&self, a: &AllocationId) -> Option<SvidMaterial> {
        self.svids.get(a).cloned()
    }
    fn current_bundle(&self) -> Option<TrustBundle> {
        self.bundle.clone()
    }
}

// ----------------------------------------------------------------- peer (child process)
fn peer_main(dir: &Path, mode: &str) {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use rustls::server::WebPkiClientVerifier;
    use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};
    let _ = rustls::crypto::ring::default_provider().install_default();
    let rd = |n: &str| std::fs::read(dir.join(n)).unwrap();
    let cert = CertificateDer::from(rd("peer.der"));
    let inter = CertificateDer::from(rd("inter.der"));
    let key = PrivateKeyDer::try_from(rd("peer.key")).unwrap();
    let ca = String::from_utf8(rd("ca.pem")).unwrap();
    let mut roots = RootCertStore::empty();
    for c in rustls_pemfile::certs(&mut ca.as_bytes()).flatten() {
        roots.add(c).unwrap();
    }
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots)).build().unwrap();
    let mut cfg = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![cert, inter], key)
        .unwrap();
    // As every production mesh peer and sibling Tier-3 peer does (tls_config.rs:337, outbound.rs:498):
    // a post-handshake NewSessionTicket makes the kTLS-RX return splice fail (observed here as EINVAL
    // in results/strace-1.log before this line was added).
    cfg.send_tls13_tickets = 0;
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    println!("PORT {}", l.local_addr().unwrap().port());
    std::io::stdout().flush().unwrap();
    let (tcp, _) = l.accept().unwrap();
    tcp.set_nodelay(true).ok();
    let mut tls = StreamOwned::new(ServerConnection::new(Arc::new(cfg)).unwrap(), tcp);
    let mut buf = vec![0u8; 64 * 1024];
    match mode {
        "sink" => {
            let mut total = 0u64;
            loop {
                match tls.read(&mut buf) {
                    Ok(0) => {
                        eprintln!("PEER sink: EOF after {total} bytes");
                        break;
                    }
                    Err(e) => {
                        eprintln!("PEER sink: read error after {total} bytes: {e} (kind {:?})", e.kind());
                        break;
                    }
                    Ok(n) => total += n as u64,
                }
            }
        }
        _ => {
            // handshake completes on first IO; read one byte-or-EOF-free: just write.
            let chunk = vec![0x5au8; 64 * 1024];
            let mut total = 0u64;
            loop {
                match tls.write_all(&chunk) {
                    Ok(()) => total += chunk.len() as u64,
                    Err(e) => {
                        eprintln!("PEER source: write error after {total} bytes: {e}");
                        break;
                    }
                }
            }
        }
    }
}

// ----------------------------------------------------------------- /proc sampling
#[derive(Clone, Debug)]
struct TaskStat {
    tid: u32,
    comm: String,
    ticks: u64, // utime+stime
    utime: u64,
    stime: u64,
}
fn parse_stat(s: &str) -> Option<(String, u64, u64)> {
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    let comm = s[open + 1..close].to_owned();
    let rest: Vec<&str> = s[close + 2..].split_whitespace().collect();
    // after ")" : state(0) ppid(1) ... utime is field 14 overall => index 11 here, stime index 12
    let utime: u64 = rest.get(11)?.parse().ok()?;
    let stime: u64 = rest.get(12)?.parse().ok()?;
    Some((comm, utime, stime))
}
fn tasks() -> Vec<TaskStat> {
    let mut v = Vec::new();
    for e in std::fs::read_dir("/proc/self/task").unwrap().flatten() {
        let tid: u32 = e.file_name().to_str().unwrap().parse().unwrap();
        if let Ok(s) = std::fs::read_to_string(e.path().join("stat")) {
            if let Some((comm, u, st)) = parse_stat(&s) {
                v.push(TaskStat { tid, comm, ticks: u + st, utime: u, stime: st });
            }
        }
    }
    v.sort_by_key(|t| t.tid);
    v
}
fn proc_cpu(path: &str) -> (u64, u64) {
    let s = std::fs::read_to_string(path).unwrap_or_default();
    parse_stat(&s).map(|(_, u, st)| (u, st)).unwrap_or((0, 0))
}
/// /proc/stat first line: user nice system idle iowait irq softirq steal
fn host_stat() -> [u64; 8] {
    let s = std::fs::read_to_string("/proc/stat").unwrap();
    let l = s.lines().next().unwrap();
    let mut a = [0u64; 8];
    for (i, f) in l.split_whitespace().skip(1).take(8).enumerate() {
        a[i] = f.parse().unwrap();
    }
    a
}

struct Out {
    dir: &'static str,
    mbps: f64,
    process_ms: f64,
    pump_ms: [f64; 2],
    pump_sys_share: [f64; 2],
    fixture_ms: f64,
    other_agent_ms: f64,
    ratio: f64,
    raw_share: f64,
    agent_share: f64,
    peer_ms: f64,
    softirq_ms: f64,
    host_busy_ms: f64,
}

async fn one(pki: &TestPki, dir: &Path, direction: &'static str, secs: u64) -> Out {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["peer", dir.to_str().unwrap(), if direction == "forward" { "sink" } else { "source" }])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let port: u16 = {
        use std::io::{BufRead, BufReader};
        let mut line = String::new();
        BufReader::new(child.stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
        line.trim().strip_prefix("PORT ").unwrap().parse().unwrap()
    };
    let mut svids = std::collections::BTreeMap::new();
    svids.insert(pki.client_alloc.clone(), pki.client_svid_material());
    let identity: Arc<dyn IdentityRead> = Arc::new(HeldIdentities { svids, bundle: Some(pki.trust_bundle()) });
    let adapter = HostMtlsEnforcement::new(
        identity,
        MtlsLimits::default(),
        overdrive_core::guest_transport::GUEST_NODE_PREFIX,
        guest_dial::not_serving(),
    );
    // leg F: a loopback TCP pair; the accepted end goes to the agent, the connecting end is the workload.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let workload = TcpStream::connect(l.local_addr().unwrap()).unwrap();
    let (agent_tcp, _) = l.accept().unwrap();
    workload.set_nodelay(true).ok();
    agent_tcp.set_nodelay(true).ok();
    let before: std::collections::BTreeSet<u32> = tasks().iter().map(|t| t.tid).collect();
    let conn = InterceptedConnection {
        leg: OwnedFd::from(agent_tcp),
        routed: Routed::Outbound { peer: SocketAddrV4::new("127.0.0.1".parse().unwrap(), port) },
        alloc: pki.client_alloc.clone(),
        expected_peer: None,
    };
    let handle = adapter.enforce(conn).await.expect("enforce outbound");
    let after = tasks();
    let new_tids: Vec<u32> = after.iter().map(|t| t.tid).filter(|t| !before.contains(t)).collect();

    // load generators (named so they are separable from the agent's own threads)
    let stop = Arc::new(AtomicBool::new(false));
    let bytes = Arc::new(AtomicU64::new(0));
    let mut lg = Vec::new();
    if direction == "forward" {
        let (stop, bytes, mut w) = (stop.clone(), bytes.clone(), workload.try_clone().unwrap());
        lg.push(std::thread::Builder::new().name("lg-writer".into()).spawn(move || {
            let chunk = vec![0xa5u8; 64 * 1024];
            while !stop.load(Ordering::Relaxed) {
                match w.write(&chunk) {
                    Ok(n) => {
                        bytes.fetch_add(n as u64, Ordering::Relaxed);
                    }
                    Err(e) => {
                        println!("  lg-writer error: {e}");
                        break;
                    }
                }
            }
        }).unwrap());
    } else {
        let (stop, bytes, mut r) = (stop.clone(), bytes.clone(), workload.try_clone().unwrap());
        r.set_read_timeout(Some(Duration::from_millis(200))).ok();
        lg.push(std::thread::Builder::new().name("lg-reader".into()).spawn(move || {
            let mut buf = vec![0u8; 256 * 1024];
            while !stop.load(Ordering::Relaxed) {
                match r.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        bytes.fetch_add(n as u64, Ordering::Relaxed);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(_) => break,
                }
            }
        }).unwrap());
    }
    for i in 0..5 {
        tokio::time::sleep(Duration::from_millis(400)).await;
        let now = tasks();
        println!(
            "  DIAG t+{}ms liveness={:?} bytes={} threads={:?} peer_alive={}",
            (i + 1) * 400,
            adapter.liveness(&handle),
            bytes.load(Ordering::Relaxed),
            now.iter().filter(|t| !before.contains(&t.tid)).map(|t| (t.tid, t.comm.clone(), t.ticks)).collect::<Vec<_>>(),
            child.try_wait().map(|x| x.is_none()).unwrap_or(false),
        );
    }
    tokio::time::sleep(Duration::from_millis(0)).await; // (warm-up already elapsed above: 2 s)
    let (t_a, p_a, h_a, c_a, b_a, w_a) = (tasks(), proc_cpu("/proc/self/stat"), host_stat(), proc_cpu(&format!("/proc/{}/stat", child.id())), bytes.load(Ordering::Relaxed), Instant::now());
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let (t_b, p_b, h_b, c_b, b_b, w_b) = (tasks(), proc_cpu("/proc/self/stat"), host_stat(), proc_cpu(&format!("/proc/{}/stat", child.id())), bytes.load(Ordering::Relaxed), Instant::now());
    stop.store(true, Ordering::Relaxed);
    let secs_f = w_b.duration_since(w_a).as_secs_f64();
    let tick_ms = 10.0; // CLK_TCK=100

    let find = |v: &[TaskStat], tid: u32| v.iter().find(|t| t.tid == tid).cloned();
    let mut rows: Vec<(u32, String, f64, f64, bool, bool)> = Vec::new(); // tid, comm, total ms, sys ms, new, fixture
    for tb in &t_b {
        let ta = find(&t_a, tb.tid);
        let (dt, ds) = match ta {
            Some(ta) => (tb.ticks - ta.ticks, tb.stime - ta.stime),
            None => (tb.ticks, tb.stime),
        };
        rows.push((tb.tid, tb.comm.clone(), dt as f64 * tick_ms, ds as f64 * tick_ms, new_tids.contains(&tb.tid), tb.comm.starts_with("lg-")));
    }
    println!("--- {direction}: per-thread CPU over a {secs_f:.2}s window (ms; thread_stat granularity 10 ms); 'new' = created by enforce ---");
    println!("new_tids_created_by_enforce={new_tids:?} (comm: {:?})", after.iter().filter(|t| new_tids.contains(&t.tid)).map(|t| (t.tid, t.comm.clone())).collect::<Vec<_>>());
    let mut sorted = rows.clone();
    sorted.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    for r in sorted.iter().take(8) {
        println!("  tid={} comm={:<18} cpu={:>8.0} ms sys={:>8.0} ms new={} fixture={}", r.0, r.1, r.2, r.3, r.4, r.5);
    }
    let process_ms = ((p_b.0 + p_b.1) - (p_a.0 + p_a.1)) as f64 * tick_ms;
    let newrows: Vec<&(u32, String, f64, f64, bool, bool)> = rows.iter().filter(|r| r.4 && !r.5).collect();
    let mut pump: Vec<&&(u32, String, f64, f64, bool, bool)> = newrows.iter().collect();
    pump.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    let top = |i: usize| pump.get(i).map_or((0.0, 0.0), |r| (r.2, r.3));
    let (p0, p1) = (top(0), top(1));
    let fixture_ms: f64 = rows.iter().filter(|r| r.5).map(|r| r.2).sum();
    let agent_ms = process_ms - fixture_ms;
    let pump_sum = p0.0 + p1.0;
    let hostd: Vec<u64> = (0..8).map(|i| h_b[i] - h_a[i]).collect();
    let host_busy = (hostd[0] + hostd[1] + hostd[2] + hostd[5] + hostd[6] + hostd[7]) as f64 * tick_ms;
    let out = Out {
        dir: direction,
        mbps: (b_b - b_a) as f64 / secs_f / 1e6,
        process_ms,
        pump_ms: [p0.0, p1.0],
        pump_sys_share: [if p0.0 > 0.0 { p0.1 / p0.0 } else { 0.0 }, if p1.0 > 0.0 { p1.1 / p1.0 } else { 0.0 }],
        fixture_ms,
        other_agent_ms: (agent_ms - pump_sum).max(0.0),
        ratio: if p1.0 > 0.0 { p0.0 / p1.0 } else { f64::INFINITY },
        raw_share: pump_sum / process_ms,
        agent_share: pump_sum / agent_ms,
        peer_ms: ((c_b.0 + c_b.1) - (c_a.0 + c_a.1)) as f64 * tick_ms,
        softirq_ms: hostd[6] as f64 * tick_ms,
        host_busy_ms: host_busy,
    };
    println!(
        "  throughput={:.0} MB/s process_cpu={:.0} ms pump0={:.0} pump1={:.0} (sys share {:.2}/{:.2}) fixture_threads={:.0} other_agent={:.0} peer_child={:.0} host_softirq={:.0} host_busy={:.0}",
        out.mbps, out.process_ms, out.pump_ms[0], out.pump_ms[1], out.pump_sys_share[0], out.pump_sys_share[1], out.fixture_ms, out.other_agent_ms, out.peer_ms, out.softirq_ms, out.host_busy_ms
    );
    println!("  RESULT {direction}: pumps/process_delta={:.3}  pumps/(process_delta - fixture threads)={:.3}  heavy:light={:.2}", out.raw_share, out.agent_share, out.ratio);
    // which creation-order thread is heavier (tid order = creation order)
    let mut by_tid: Vec<&&(u32, String, f64, f64, bool, bool)> = newrows.iter().collect();
    by_tid.sort_by_key(|r| r.0);
    println!("  pump threads in creation order (tid, cpu ms): {:?}", by_tid.iter().take(4).map(|r| (r.0, r.2)).collect::<Vec<_>>());
    let _ = adapter.teardown(handle.clone()).await;
    let _ = child.kill();
    let _ = child.wait();
    out
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.get(1).map(String::as_str) == Some("peer") {
        peer_main(Path::new(&a[2]), &a[3]);
        return;
    }
    let secs: u64 = a.get(2).map_or(10, |s| s.parse().unwrap());
    let reps: usize = a.get(3).map_or(3, |s| s.parse().unwrap());
    let csv = a.get(4).cloned().unwrap_or_else(|| "runs.csv".into());
    let _ = rustls::crypto::ring::default_provider().install_default();
    println!("# uname -r: {}", String::from_utf8_lossy(&std::process::Command::new("uname").arg("-r").output().unwrap().stdout).trim());
    let _ = std::process::Command::new("modprobe").arg("tls").status();
    let pki = TestPki::mint();
    let dir = std::env::temp_dir().join(format!("nbp-v3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    use rustls::pki_types::PrivateKeyDer;
    std::fs::write(dir.join("peer.der"), pki.peer_leaf.cert_der.as_ref()).unwrap();
    std::fs::write(dir.join("inter.der"), pki.intermediate_cert_der().as_ref()).unwrap();
    std::fs::write(dir.join("peer.key"), match &pki.peer_leaf.key_der { PrivateKeyDer::Pkcs8(k) => k.secret_pkcs8_der().to_vec(), _ => panic!("peer key is not PKCS#8") }).unwrap();
    std::fs::write(dir.join("ca.pem"), pki.ca_cert_pem()).unwrap();
    let mut out = String::from("rep,direction,mbps,process_ms,pump0_ms,pump1_ms,fixture_ms,peer_ms,ratio,raw_share,agent_share\n");
    for rep in 0..reps {
        for d in ["forward", "return"] {
            let o = one(&pki, &dir, d, secs).await;
            out.push_str(&format!("{rep},{},{:.0},{:.0},{:.0},{:.0},{:.0},{:.0},{:.2},{:.3},{:.3}\n", o.dir, o.mbps, o.process_ms, o.pump_ms[0], o.pump_ms[1], o.fixture_ms, o.peer_ms, o.ratio, o.raw_share, o.agent_share));
        }
    }
    std::fs::write(&csv, &out).unwrap();
    println!("{out}");
    let _ = std::fs::remove_dir_all(&dir);
}
