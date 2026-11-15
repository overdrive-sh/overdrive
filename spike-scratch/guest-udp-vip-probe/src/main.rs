//! Throwaway harness (spike): mirrors `Lane::boot` of vsock_support.rs — the real
//! `run_with_adapters` composition with `NodeAdapters::host` (SimKek injected) and
//! the host dataplane — then deploys through the public deploy handler a UDP+TCP
//! VM Service (peer) and a VM client Service. Nothing is hand-installed.
use std::io::Write as _;
use std::net::{Ipv4Addr, SocketAddr};
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

const PROBE_GUEST_PATH: &str = "/usr/local/bin/udp-probe";
const PORT: u16 = 9000;
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

fn toml(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

async fn rows(cfg: &Path, id: &str) -> overdrive_control_plane::api::AllocStatusResponse {
    describe(DescribeArgs { id: id.to_owned(), config_path: cfg.to_owned() }).await.unwrap().snapshot
}

async fn wait_running(cfg: &Path, id: &str, secs: u64) -> String {
    let end = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let s = rows(cfg, id).await;
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

fn dumps(ev: &Evidence, tag: &str) {
    ev.sh(&format!("{tag}: uname/bpftool map list (all maps whose name mentions LOCAL/SERVICE/REVERSE)"), "uname -r; bpftool map show 2>&1 | grep -i -E 'local|service|reverse|backend' ");
    ev.sh(&format!("{tag}: dump LOCAL_BACKEND_M"), "bpftool map dump name LOCAL_BACKEND_M 2>&1");
    ev.sh(&format!("{tag}: dump REVERSE_LOCAL (name prefix)"), "bpftool map show 2>&1 | grep -i reverse_local | head -3; for id in $(bpftool map show 2>/dev/null | grep -i 'name REVERSE_LOCAL' | cut -d: -f1); do bpftool map dump id $id 2>&1; done");
    ev.sh(&format!("{tag}: dump other service maps for positive control"), "for n in SERVICE_MAP BACKEND_MAP; do echo --- $n; bpftool map dump name $n 2>&1 | head -40; done; bpftool map show 2>&1 | grep -i -E 'name (SERVICE|BACKEND|MAGLEV|REVERSE_NAT)' ");
    ev.sh(&format!("{tag}: cgroup attachments"), "bpftool cgroup tree /sys/fs/cgroup/overdrive.slice 2>&1 | head -60; echo ---; bpftool cgroup show /sys/fs/cgroup/overdrive.slice 2>&1");
}

#[tokio::main]
async fn main() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |i| { prev(i); eprintln!("harness: panic -> exiting process so the exclusive lease is released"); std::process::exit(101); }));
    std::thread::spawn(|| { std::thread::sleep(Duration::from_secs(900)); eprintln!("harness: 900s watchdog"); std::process::exit(102); });
    let a: Vec<String> = std::env::args().collect();
    let probe_bin = PathBuf::from(&a[1]);
    let evdir = PathBuf::from(&a[2]);
    std::fs::create_dir_all(&evdir).unwrap();
    let ev = Evidence { dir: evdir.clone() };
    ev.sh("env", &format!("uname -a; cd {} && git rev-parse HEAD && git status --short | head; date -u", env!("CARGO_MANIFEST_DIR")));

    overdrive_testing::forwarder_module::expect_forwarder_module();
    let kit = GuestKit::provision(&overdrive_testing::vm_fixture::default_staging_root()).expect("guest kit provisions");
    let server_tmp = tempfile::Builder::new().prefix("vm-serve-").tempdir_in(overdrive_testing::vm_fixture::default_staging_root()).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        // the VMM runs as an unprivileged uid; a 0700 root-owned tempdir blocks it from the rootfs clone
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

    let mut spec = GuestImageSpec::workload();
    spec.extra.push(GuestFile { host: probe_bin, guest: PathBuf::from(PROBE_GUEST_PATH), with_shared_libraries: false });
    let image = kit.image(server_tmp.path(), "udp-vip", &spec).expect("image");
    let (kernel, rootfs) = (kit.kernel_path.display().to_string(), image.rootfs.display().to_string());

    // ---- peer: VM Service, tcp+udp listeners on PORT, echoing both.
    let peer_body = format!(
        "[service]\nid = \"udp-peer\"\nreplicas = 1\n\n[vm]\ncommand = \"{PROBE_GUEST_PATH}\"\nargs = [\"echo\", \"{PORT}\"]\n\
         kernel = \"{kernel}\"\nrootfs = \"{rootfs}\"\n\n[resources]\ncpu_milli = 500\nmemory_bytes = {VM_MEM}\n\n\
         [[listener]]\nport = {PORT}\nprotocol = \"tcp\"\n\n[[listener]]\nport = {PORT}\nprotocol = \"udp\"\n");
    ev.log("peer spec", &peer_body);
    let spec_p = toml(server_tmp.path(), "udp-peer.toml", &peer_body);
    deploy(DeployArgs { spec: spec_p, config_path: cfg.clone() }).await.expect("deploy peer");
    let peer_alloc = wait_running(&cfg, "udp-peer", 120).await;
    let listening = wait_console(&peer_alloc, 40, "probe-echo: listening").await;
    ev.log("peer console after Running", &format!("listening-seen={listening}\n{}", console(&peer_alloc)));
    // let reconcilers publish rows
    tokio::time::sleep(Duration::from_secs(8)).await;
    let snap = rows(&cfg, "udp-peer").await;
    ev.log("peer snapshot", &format!("{snap:#?}"));
    let vip: Ipv4Addr = snap.vip.as_deref().expect("peer has VIP").parse().unwrap();
    let backend = snap.rows.iter().find(|r| r.alloc_id == peer_alloc).and_then(|r| r.workload_addr).expect("workload_addr");
    ev.log("addresses", &format!("VIP={vip} backend_guest_addr={backend} port={PORT}"));
    dumps(&ev, "T0 (peer Running, before client)");

    // ---- client VM
    let client_body = format!(
        "[service]\nid = \"udp-client\"\nreplicas = 1\n\n[vm]\ncommand = \"{PROBE_GUEST_PATH}\"\nargs = [\"client\", \"{vip}\", \"{PORT}\", \"{backend}:{PORT}\"]\n\
         kernel = \"{kernel}\"\nrootfs = \"{rootfs}\"\n\n[resources]\ncpu_milli = 500\nmemory_bytes = {VM_MEM}\n\n\
         [[listener]]\nport = 8081\nprotocol = \"tcp\"\n");
    let spec_c = toml(server_tmp.path(), "udp-client.toml", &client_body);
    deploy(DeployArgs { spec: spec_c, config_path: cfg.clone() }).await.expect("deploy client");
    let client_alloc = wait_running(&cfg, "udp-client", 120).await;
    ev.log("client alloc", &client_alloc);

    // strace the in-process serve (owner of the host datagram socket)
    let pid = std::process::id();
    let strace_out = evdir.join("strace-serve.txt");
    let mut strace = Command::new("strace")
        .args(["-f", "-ttt", "-yy", "-s", "64", "-e", "trace=connect,getpeername,getsockname,sendto,sendmsg,sendmmsg,recvfrom,recvmsg,write,shutdown", "-o"])
        .arg(&strace_out).arg("-p").arg(pid.to_string())
        .spawn().map_err(|e| ev.log("strace spawn failed", &e.to_string())).ok();

    let reached = wait_console(&client_alloc, 120, "probe: dump-point").await;
    ev.log("client reached dump-point", &reached.to_string());
    dumps(&ev, "T1 (client mid-run, after phase-1 UDP)");
    let done = wait_console(&client_alloc, 90, "probe: done").await;
    ev.log("client done", &done.to_string());
    dumps(&ev, "T2 (client done)");
    if let Some(s) = strace.as_mut() { let _ = Command::new("kill").arg("-INT").arg(s.id().to_string()).status(); let _ = s.wait(); }

    let cc = console(&client_alloc);
    ev.log("CLIENT CONSOLE", &cc);
    ev.log("PEER CONSOLE", &console(&peer_alloc));
    ev.sh("strace: connect()/send of interest (VIP/backend/port)", &format!("grep -E '{vip}|{backend}|:{PORT}|sin_port=htons\\({PORT}\\)' {} | head -80", strace_out.display()));
    ev.sh("strace: size", &format!("wc -l {}", strace_out.display()));

    stop(StopArgs { id: "udp-client".into(), config_path: cfg.clone() }).await.ok();
    stop(StopArgs { id: "udp-peer".into(), config_path: cfg.clone() }).await.ok();
    tokio::time::sleep(Duration::from_secs(5)).await;
    handle.shutdown().await.ok();
}
