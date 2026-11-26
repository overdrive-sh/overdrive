//! Throwaway harness (spike): boots the real `serve` composition in-process
//! (`run_with_adapters` with `NodeAdapters::host`, SimKek injected; same shape as
//! spike-scratch/guest-udp-vip-probe) and deploys, through the public deploy
//! handler, one VM Service whose guest program probes public-name DNS. Nothing is
//! hand-installed: no resolver, route, rule or map entry.
use std::io::Write as _;
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

    let mut spec = GuestImageSpec::workload();
    spec.extra.push(GuestFile { host: probe_bin, guest: PathBuf::from(PROBE_GUEST_PATH), with_shared_libraries: false });
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
    let done = wait_console(&alloc, 240, "probe: done").await;
    ev.log("probe reached done", &done.to_string());
    ev.log("GUEST CONSOLE", &console(&alloc));

    stop(StopArgs { id: SERVICE.into(), config_path: cfg.clone() }).await.ok();
    tokio::time::sleep(Duration::from_secs(5)).await;
    handle.shutdown().await.ok();
}
