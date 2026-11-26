//! Throwaway harness (spike): boots the real `serve` composition in-process
//! (`run_with_adapters` with `NodeAdapters::host`, SimKek injected; same shape as
//! spike-scratch/guest-udp-vip-probe) and deploys, through the public deploy
//! handler, one VM Service whose guest program probes public-name DNS. Nothing is
//! hand-installed: no resolver, route, rule or map entry.
mod pktinfo;
use std::io::{Read as _, Write as _};
use std::sync::atomic::{AtomicBool, Ordering};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::{ServeArgs, run_with_adapters};
use overdrive_cli::commands::workload::{DescribeArgs, describe};
use overdrive_control_plane::NodeAdapters;
use overdrive_control_plane::api::AllocStateWire;
use overdrive_core::guest_transport::NodeVsockPortConfig;
use overdrive_core::vm::config::VmRunDir;
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::SimKek;
use overdrive_testing::guest_kit::{GuestFile, GuestImageSpec, GuestKit};

const PROBE_GUEST_PATH: &str = "/usr/local/bin/dns-probe";
const GLIBC_GUEST_PATH: &str = "/usr/local/bin/dns-probe-glibc";
const GATEWAY: &str = "100.95.0.1";
const SERVICE: &str = "dns-probe";
const VM_MEM: u64 = 128 * 1024 * 1024;

struct Evidence { dir: PathBuf }
impl Evidence {
    fn log(&self, label: &str, body: &str) {
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("evidence.log")).unwrap();
        writeln!(f, "===== [{ts}] {label} =====\n{body}").unwrap();
        println!("===== {label} =====\n{body}");
    }
    fn sh(&self, label: &str, cmd: &str) {
        let out = Command::new("bash").arg("-c").arg(cmd).output().expect("bash");
        let body = format!("$ {cmd}\n(exit {:?})\n{}{}", out.status.code(), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        self.log(label, &body);
    }
}

fn console(alloc: &str) -> String {
    let p = VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), &alloc.parse().unwrap()).console_log();
    String::from_utf8_lossy(&std::fs::read(p).unwrap_or_default()).into_owned()
}

async fn wait_running(cfg: &Path, id: &str, secs: u64) -> String {
    let end = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let s = describe(DescribeArgs { id: id.to_owned(), config_path: cfg.to_owned() }).await.unwrap().snapshot;
        if let Some(r) = s.rows.iter().find(|r| r.state == AllocStateWire::Running) { return r.alloc_id.clone(); }
        assert!(tokio::time::Instant::now() < end, "{id} not Running: {:#?}", s.rows);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn wait_console(alloc: &str, secs: u64, needle: &str) -> bool {
    let end = tokio::time::Instant::now() + Duration::from_secs(secs);
    while tokio::time::Instant::now() < end {
        if console(alloc).contains(needle) { return true; }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

#[tokio::main]
async fn main() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |i| { prev(i); eprintln!("harness: panic -> exiting process so the exclusive lease is released"); std::process::exit(101); }));
    std::thread::spawn(|| { std::thread::sleep(Duration::from_secs(600)); eprintln!("harness: 600s watchdog"); std::process::exit(102); });
    let a: Vec<String> = std::env::args().collect();
    let probe_bin = PathBuf::from(&a[1]);
    let glibc_bin = PathBuf::from(&a[3]);
    let evdir = PathBuf::from(&a[2]);
    std::fs::create_dir_all(&evdir).unwrap();
    let ev = Evidence { dir: evdir.clone() };
    ev.sh("env", &format!("uname -a; cd {} && git rev-parse HEAD && git status --short | head; date -u", env!("CARGO_MANIFEST_DIR")));
    ev.sh("host resolver (control: does the HOST resolve public names?)", "cat /etc/resolv.conf; getent hosts example.com api.anthropic.com");

    overdrive_testing::forwarder_module::expect_forwarder_module();
    let kit = GuestKit::provision(&overdrive_testing::vm_fixture::default_staging_root()).expect("guest kit provisions");
    let server_tmp = tempfile::Builder::new().prefix("vm-serve-").tempdir_in(overdrive_testing::vm_fixture::default_staging_root()).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(server_tmp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let data_dir = server_tmp.path().join("data");
    let config_dir = server_tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();
    let args = ServeArgs {
        bind: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        data_dir, config_dir,
        vsock_ports: NodeVsockPortConfig::kernel_assigned(),
        gateway_address: None, gateway_certified_key_id: None, gateway_certificate_chain: None, gateway_private_key: None,
    };
    let handle = run_with_adapters(args, Arc::new(RealCgroupFs::new()), move |config, fs| {
        NodeAdapters::host(config, fs, Arc::new(SimKek::for_boot()))
    }).await.expect("serve boots (host dataplane)");
    let cfg = server_tmp.path().join("conf").join(".overdrive").join("config");
    ev.sh("local routes / addrs / rules after serve boot", "ip -4 addr; echo; ip -4 route show table local; echo; ip rule; echo; ss -ulnp | grep ':53 ' || true; ss -tlnp | grep ':53 ' || true");
    ev.sh("host dual-stack control (dig/getent)", "for n in www.google.com cloudflare.com example.com github.com; do echo $n A: $(dig +short A $n @1.1.1.1 | tr '\\n' ' ') AAAA: $(dig +short AAAA $n @1.1.1.1 | tr '\\n' ' '); done; echo resolvers:; cat /etc/resolv.conf");
    // V-2 observer: a second wildcard :53 socket (SO_REUSEADDR + IP_PKTINFO, like the production responder)
    // purely to OBSERVE the pktinfo of real guest queries. It never replies.
    let stop_obs = Arc::new(AtomicBool::new(false));
    let obs = {
        let (stop, d) = (stop_obs.clone(), evdir.clone());
        std::thread::spawn(move || {
            let e = Evidence { dir: d };
            match pktinfo::PktSock::bind_wildcard(53, 1000) {
                Err(err) => e.log("V2 observer :53 bind FAILED", &err.to_string()),
                Ok(sock) => {
                    e.log("V2 observer :53 bound", "wildcard, SO_REUSEADDR, IP_PKTINFO");
                    while !stop.load(Ordering::Relaxed) {
                        if let Ok(Some(p)) = sock.recv() {
                            e.log("V2 OBSERVER got guest query on :53", &format!("src={} spec_dst={} ipi_addr(hdr daddr)={} ifindex={} len={}", p.src, p.spec_dst, p.ipi_addr, p.ifindex, p.data.len()));
                        }
                    }
                }
            }
        })
    };

    let mut spec = GuestImageSpec::workload();
    spec.extra.push(GuestFile { host: probe_bin, guest: PathBuf::from(PROBE_GUEST_PATH), with_shared_libraries: false });
    spec.extra.push(GuestFile { host: glibc_bin, guest: PathBuf::from(GLIBC_GUEST_PATH), with_shared_libraries: true });
    let image = kit.image(server_tmp.path(), "dns-probe", &spec).expect("image");
    let (kernel, rootfs) = (kit.kernel_path.display().to_string(), image.rootfs.display().to_string());

    let mesh = format!("{SERVICE}.svc.overdrive.local");
    let body = format!(
        "[service]\nid = \"{SERVICE}\"\nreplicas = 1\n\n[vm]\ncommand = \"{PROBE_GUEST_PATH}\"\nargs = [\"{mesh}\"]\n\
         kernel = \"{kernel}\"\nrootfs = \"{rootfs}\"\n\n[resources]\ncpu = \"500m\"\nmemory = \"{VM_MEM}B\"\n\n\
         [[listener]]\nport = 8081\nprotocol = \"tcp\"\n");
    ev.log("spec", &body);
    let spec_p = server_tmp.path().join("dns-probe.toml");
    std::fs::write(&spec_p, &body).unwrap();
    deploy(DeployArgs { spec: spec_p, config_path: cfg.clone() }).await.expect("deploy");
    let alloc = wait_running(&cfg, SERVICE, 120).await;
    ev.log("alloc", &alloc);
    v2_host_probe(&ev);
    // marker-driven host actions while the guest runs
    let mut listener_started = false;
    let mut obs_stopped = false;
    let end = tokio::time::Instant::now() + Duration::from_secs(300);
    let mut done = false;
    while tokio::time::Instant::now() < end {
        let c = console(&alloc);
        if !obs_stopped && c.contains("probe: V2 end") { stop_obs.store(true, Ordering::Relaxed); obs_stopped = true; }
        if !listener_started && c.contains("probe: V5 nolistener done") {
            listener_started = true;
            start_tcp_listener(&evdir);
            ev.sh("after spike TCP listener bound", "ss -tlnp | grep ':53 ' || true");
        }
        if c.contains("probe: done") { done = true; break; }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    stop_obs.store(true, Ordering::Relaxed);
    let _ = obs.join();
    ev.log("probe reached done", &done.to_string());
    ev.log("GUEST CONSOLE", &console(&alloc));

    stop(StopArgs { id: SERVICE.into(), config_path: cfg.clone() }).await.ok();
    tokio::time::sleep(Duration::from_secs(5)).await;
    handle.shutdown().await.ok();
}

/// V-2 host-side probe on a spare port: what `ipi_spec_dst` does a wildcard-bound IP_PKTINFO socket see for
/// datagrams addressed to the guest gateway versus other node addresses? Runs in the host namespace AFTER serve
/// has installed the production routes (no hand-installed route).
fn v2_host_probe(ev: &Evidence) {
    const PORT: u16 = 5399;
    let primary = std::process::Command::new("bash").arg("-c").arg("ip -4 route get 1.1.1.1 | grep -o 'src [0-9.]*' | cut -d' ' -f2")
        .output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
    ev.log("V2a host primary IPv4", &primary);
    let sock = match pktinfo::PktSock::bind_wildcard(PORT, 1500) { Ok(s) => s, Err(e) => { ev.log("V2a bind FAILED", &e.to_string()); return; } };
    let cases: Vec<(&str, String, Option<&str>)> = vec![
        ("connected->gateway (ADR-0152 owner shape)", GATEWAY.into(), None),
        ("connected->host-primary", primary.clone(), None),
        ("connected->127.0.0.1", "127.0.0.1".into(), None),
        ("connected->gateway, explicit src bind = host-primary", GATEWAY.into(), Some(&primary)),
        ("unconnected sendto->gateway", GATEWAY.into(), Some("sendto")),
        ("unconnected sendto->host-primary", primary.clone(), Some("sendto")),
    ];
    for (label, dst, mode) in cases {
        let bind = match mode { Some(m) if m != "sendto" => format!("{m}:0"), _ => "0.0.0.0:0".into() };
        let s = match std::net::UdpSocket::bind(&bind) { Ok(s) => s, Err(e) => { ev.log(&format!("V2a {label}"), &format!("sender bind {bind} FAILED {e}")); continue; } };
        let target = format!("{dst}:{PORT}");
        let sent = if mode == Some("sendto") { s.send_to(label.as_bytes(), &target).map(|_| ()) } else { s.connect(&target).and_then(|_| s.send(label.as_bytes()).map(|_| ())) };
        if let Err(e) = sent { ev.log(&format!("V2a {label}"), &format!("send to {target} FAILED {e}")); continue; }
        match sock.recv() {
            Ok(Some(p)) => ev.log(&format!("V2a {label}"), &format!("dst={dst} -> src={} spec_dst={} ipi_addr(hdr daddr)={} ifindex={} spec_dst==gateway:{}", p.src, p.spec_dst, p.ipi_addr, p.ifindex, p.spec_dst.to_string() == GATEWAY)),
            Ok(None) => ev.log(&format!("V2a {label}"), &format!("dst={dst}: NO DATAGRAM within 1.5s")),
            Err(e) => ev.log(&format!("V2a {label}"), &format!("recv error {e}")),
        }
    }
}

/// Spike stand-in for the proposed GPD-5 listener: a TCP listener on gateway:53, started AFTER serve booted.
/// Answers a length-framed query with a framed reply carrying a recognisable TXT litmus.
fn start_tcp_listener(evdir: &Path) {
    let e = Evidence { dir: evdir.to_owned() };
    let l = match std::net::TcpListener::bind(format!("{GATEWAY}:53")) {
        Ok(l) => l, Err(err) => { e.log("V5 listener bind FAILED", &err.to_string()); return; }
    };
    e.log("V5 listener bound", &format!("{:?}", l.local_addr()));
    let d = evdir.to_owned();
    std::thread::spawn(move || {
        let e = Evidence { dir: d };
        for conn in l.incoming() {
            let mut c = match conn { Ok(c) => c, Err(err) => { e.log("V5 accept error", &err.to_string()); continue; } };
            let peer = format!("{:?} -> local {:?}", c.peer_addr(), c.local_addr());
            c.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut len = [0u8; 2];
            if let Err(err) = c.read_exact(&mut len) { e.log("V5 conn", &format!("{peer}: read len failed {err}")); continue; }
            let mut q = vec![0u8; u16::from_be_bytes(len) as usize];
            if let Err(err) = c.read_exact(&mut q) { e.log("V5 conn", &format!("{peer}: read body failed {err}")); continue; }
            let mut r = vec![q[0], q[1], 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
            r.extend_from_slice(&q[12..]); // echo question
            r.extend_from_slice(&[0xc0, 0x0c, 0, 16, 0, 1, 0, 0, 0, 0, 0, 13, 12]);
            r.extend_from_slice(b"GPD-V5-REPLY");
            let mut f = (r.len() as u16).to_be_bytes().to_vec();
            f.extend_from_slice(&r);
            let w = c.write_all(&f);
            e.log("V5 conn", &format!("{peer}: query_len={} first_bytes={:02x?} reply_written={w:?}", q.len(), &q[..q.len().min(16)]));
        }
    });
}
