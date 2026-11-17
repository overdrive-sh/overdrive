//! V-10 probe. One real `serve` (run_with_adapters), the guest-kit workload image
//! (`GuestImageSpec::workload()`, `overdrive-guest-peer idle`), a `Vmm` decorator that reads the
//! console log when the exit watch fires, a `BeaconIo` decorator that counts READY and EXIT
//! lines, and N deploy -> Running -> stop -> terminal cycles. The data dir is created under
//! umask 0o022 (DDD-11), as guest_listener_real_boot.rs does.
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use overdrive_cli::commands::deploy::{deploy, stop, DeployArgs, StopArgs};
use overdrive_cli::commands::serve::{run_with_adapters, ServeArgs};
use overdrive_cli::commands::workload::{describe, DescribeArgs};
use overdrive_control_plane::api::AllocStateWire;
use overdrive_control_plane::{NodeAdapters, VmCapability};
use overdrive_core::guest_transport::{GuestCid, NodeVsockPortConfig};
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmExitWatch, VmProcess, VmTermination, Vmm, VmmProbeError,
};
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo, BeaconSocket};
use overdrive_core::vm::config::{VmConfig, VsockPort};
use overdrive_host::{CloudHypervisorVmm, RealCgroupFs, VsockBeaconIo};
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::{SimDataplaneComposer, SimKek};
use overdrive_testing::guest_app::{subcommand, GUEST_PEER_GUEST_PATH};
use overdrive_testing::guest_kit::{GuestImageSpec, GuestKit};
use overdrive_testing::vm_fixture::default_staging_root;

#[derive(Default)]
struct Seen {
    console: Vec<String>,
    beacon_lines: Vec<String>,
}

struct ConsoleVmm {
    inner: CloudHypervisorVmm,
    seen: Arc<Mutex<Seen>>,
}

#[async_trait]
impl Vmm for ConsoleVmm {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }
    async fn probe(&self) -> Result<(), VmmProbeError> {
        self.inner.probe().await
    }
    async fn create(&self, config: &VmConfig, cid: GuestCid) -> VmmResult<VmProcess> {
        let VmProcess { control, mut exit, diagnostics } = self.inner.create(config, cid).await?;
        let console = config.run_dir.console_log();
        let seen = Arc::clone(&self.seen);
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            if let Some(ending) = exit.recv().await {
                let text = std::fs::read_to_string(&console)
                    .unwrap_or_else(|e| format!("<console {} unreadable: {e}>", console.display()));
                seen.lock().unwrap().console.push(text);
                let _ = tx.send(ending);
            }
        });
        Ok(VmProcess { control, exit: VmExitWatch::new(rx), diagnostics })
    }
    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

struct LoggingBeacon {
    inner: VsockBeaconIo,
    seen: Arc<Mutex<Seen>>,
}

#[async_trait]
impl BeaconIo for LoggingBeacon {
    async fn bind(
        &self,
        port: overdrive_core::guest_transport::VsockListenPort,
    ) -> Result<VsockPort, std::io::Error> {
        self.inner.bind(port).await
    }
    async fn accept(&self) -> Result<(BeaconSocket, BeaconConnKey), std::io::Error> {
        self.inner.accept().await
    }
    async fn read_line(&self, socket: &BeaconSocket) -> Result<Option<String>, std::io::Error> {
        let line = self.inner.read_line(socket).await?;
        if let Some(text) = &line {
            self.seen.lock().unwrap().beacon_lines.push(text.clone());
        }
        Ok(line)
    }
    async fn write_line(&self, socket: &BeaconSocket, line: &str) -> Result<(), std::io::Error> {
        self.inner.write_line(socket, line).await
    }
    async fn close(&self, socket: BeaconSocket) {
        self.inner.close(socket).await;
    }
}

fn stage_ms(text: &str, stage: &str) -> Option<u64> {
    text.lines()
        .filter_map(|l| {
            let rest = l.split("overdrive-init: ").nth(1)?;
            let (s, tail) = rest.split_once(" elapsed_ms=")?;
            (s == stage).then(|| tail.trim().parse::<u64>().ok()).flatten()
        })
        .last()
}

async fn state_of(cfg: &Path, id: &str) -> Option<AllocStateWire> {
    describe(DescribeArgs { id: id.to_owned(), config_path: cfg.to_path_buf() })
        .await
        .ok()
        .and_then(|o| o.snapshot.rows.first().map(|r| r.state))
}

async fn wait_state(cfg: &Path, id: &str, want: AllocStateWire, max: Duration) -> Option<AllocStateWire> {
    let t0 = std::time::Instant::now();
    let mut last = None;
    while t0.elapsed() < max {
        if let Some(s) = state_of(cfg, id).await {
            last = Some(s);
            if s == want || matches!(s, AllocStateWire::Terminated | AllocStateWire::Failed) {
                return Some(s);
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    last
}

#[tokio::main]
async fn main() {
    let cycles: usize = std::env::args().nth(1).map_or(60, |s| s.parse().unwrap());
    println!("# uname -r: {}", String::from_utf8_lossy(&std::process::Command::new("uname").arg("-r").output().unwrap().stdout).trim());
    // SAFETY: umask has no preconditions.
    unsafe { libc::umask(0o022) };
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("guest kit (run kmod-build and guest-kit-prepare)");
    let image_dir = tempfile::Builder::new().prefix("nbp-v10-img-").tempdir_in(&staging).unwrap();
    std::fs::set_permissions(image_dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let image = kit.image(image_dir.path(), "workload", &GuestImageSpec::workload()).expect("workload image");
    println!("# kernel={} rootfs={}", kit.kernel_path.display(), image.rootfs.display());

    let seen = Arc::new(Mutex::new(Seen::default()));
    let tmp = tempfile::Builder::new().prefix("v10-serve-").tempdir_in(&staging).unwrap();
    std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    for p in [&data_dir, &config_dir] {
        std::fs::create_dir_all(p).unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let args = ServeArgs {
        bind: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        data_dir,
        config_dir,
        vsock_ports: NodeVsockPortConfig::kernel_assigned(),
        gateway_address: None,
        gateway_certified_key_id: None,
        gateway_certificate_chain: None,
        gateway_private_key: None,
    };
    let (vmm, beacon) = (
        Arc::new(ConsoleVmm { inner: CloudHypervisorVmm::new(), seen: Arc::clone(&seen) }) as Arc<dyn Vmm>,
        Arc::new(LoggingBeacon { inner: VsockBeaconIo::new(), seen: Arc::clone(&seen) }) as Arc<dyn BeaconIo>,
    );
    let server = run_with_adapters(args, Arc::new(RealCgroupFs::new()), move |config, fs| {
        let mut a = NodeAdapters::host(config, fs, Arc::new(SimKek::for_boot()))?;
        a.boot.dataplane = Arc::new(SimDataplaneComposer::new(Arc::new(SimDataplane::new()), Ipv4Addr::LOCALHOST));
        a.vm.vmm = vmm;
        a.vm.beacon = beacon;
        a.vm.capability = VmCapability::Required;
        Ok(a)
    })
    .await
    .expect("serve");
    let cfg = tmp.path().join("conf").join(".overdrive").join("config");

    let (mut ready, mut exit_seen, mut both, mut running_ok) = (0usize, 0usize, 0usize, 0usize);
    let mut durations: BTreeMap<i64, usize> = BTreeMap::new();
    for i in 0..cycles {
        let id = format!("nbp-v10-{i:03}");
        let spec = tmp.path().join(format!("{id}.toml"));
        std::fs::write(
            &spec,
            format!(
                "[job]\nid = \"{id}\"\n\n[vm]\ncommand = \"{GUEST_PEER_GUEST_PATH}\"\nargs = [\"{}\"]\nkernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = 500\nmemory_bytes = 134217728\n",
                subcommand::IDLE,
                kit.kernel_path.display(),
                image.rootfs.display()
            ),
        )
        .unwrap();
        let (c0, b0) = {
            let s = seen.lock().unwrap();
            (s.console.len(), s.beacon_lines.len())
        };
        let sub = match deploy(DeployArgs { spec, config_path: cfg.clone() }).await {
            Ok(s) => s,
            Err(e) => {
                println!("CYCLE {id}: deploy failed: {e}");
                continue;
            }
        };
        let run = wait_state(&cfg, &sub.workload_id, AllocStateWire::Running, Duration::from_secs(90)).await;
        if run == Some(AllocStateWire::Running) {
            running_ok += 1;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = stop(StopArgs { id: sub.workload_id.clone(), config_path: cfg.clone() }).await;
        let term = wait_state(&cfg, &sub.workload_id, AllocStateWire::Terminated, Duration::from_secs(40)).await;
        for _ in 0..60 {
            if seen.lock().unwrap().console.len() > c0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let (console, lines) = {
            let s = seen.lock().unwrap();
            (s.console.get(c0).cloned().unwrap_or_default(), s.beacon_lines[b0..].to_vec())
        };
        let has_ready = lines.iter().any(|l| l.starts_with("READY"));
        let has_exit = lines.iter().any(|l| l.starts_with("EXIT"));
        let sr = stage_ms(&console, "shutdown-received");
        let gc = stage_ms(&console, "group-complete");
        ready += usize::from(has_ready);
        exit_seen += usize::from(has_exit);
        both += usize::from(sr.is_some() && gc.is_some());
        if let Some(d) = sr.zip(gc).map(|(a, b)| b as i64 - a as i64) {
            *durations.entry(d).or_default() += 1;
        }
        println!("CYCLE {id}: running={run:?} terminal={term:?} READY={has_ready} EXIT={has_exit} sr={sr:?} gc={gc:?} beacon_lines={lines:?}");
        if i == 0 {
            println!("----- RAW console ({id}) -----\n{console}\n-----");
        }
        if sr.is_none() || gc.is_none() || !has_ready {
            println!("----- DIAG console tail ({id}) -----\n{}", console.lines().rev().take(25).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"));
        }
    }
    println!("--- SUMMARY ---");
    println!("cycles={cycles} running={running_ok} READY={ready} EXIT={exit_seen} both_shutdown_lines={both} durations_ms(value:count)={durations:?}");
    let _ = server.shutdown().await;
}
