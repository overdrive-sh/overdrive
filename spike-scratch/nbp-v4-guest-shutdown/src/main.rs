//! V-4 probe. One real `serve` (run_with_adapters, the serve_fixture composition minus the
//! forwarder-module check), a `Vmm` decorator over the production `CloudHypervisorVmm`
//! that keeps each VmProcess's `VmmDiagnostics` read handle and the console-log path,
//! then N deploy -> Running -> operator stop -> terminal cycles.
//!
//! Copied (by shape, not edited at source): serve composition from
//! crates/overdrive-cli/tests/integration/serve_fixture.rs:126-163; the Vmm decorator from
//! vm_stop_restart_and_vmm_death.rs:3150-3189 (GuestControlVmm); rootfs staging helpers from
//! the same file :342-545 (build_spin_binary, rustc_static_musl, with_mounted_rootfs_copy,
//! install_guest_binary); vm_job_toml from :298-306.
use std::net::{Ipv4Addr, SocketAddr};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use overdrive_cli::commands::deploy::{deploy, stop, DeployArgs, StopArgs};
use overdrive_cli::commands::serve::{run_with_adapters, ServeArgs};
use overdrive_cli::commands::workload::{describe, DescribeArgs};
use overdrive_control_plane::api::AllocStateWire;
use overdrive_control_plane::{NodeAdapters, VmCapability};
use overdrive_core::guest_transport::{GuestCid, NodeVsockPortConfig};
use overdrive_core::traits::driver::STDERR_TAIL_LINES;
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmExitWatch, VmProcess, VmTermination, Vmm, VmmDiagnostics,
    VmmProbeError, VMM_CONSOLE_TAIL_MAX_BYTES,
};
use overdrive_core::vm::config::VmConfig;
use overdrive_host::{CloudHypervisorVmm, RealCgroupFs};
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::{SimDataplaneComposer, SimKek};
use overdrive_testing::vm_fixture::{default_staging_root, VmFixture};

#[derive(Clone, Debug)]
struct Record {
    console_file: String,
    tail_at_exit: Option<String>,
    tail_later: Option<String>,
}

struct ProbeVmm {
    inner: CloudHypervisorVmm,
    records: Arc<Mutex<Vec<Record>>>,
}

#[async_trait]
impl Vmm for ProbeVmm {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }
    async fn probe(&self) -> Result<(), VmmProbeError> {
        self.inner.probe().await
    }
    async fn create(&self, config: &VmConfig, cid: GuestCid) -> VmmResult<VmProcess> {
        let VmProcess { control, mut exit, diagnostics } = self.inner.create(config, cid).await?;
        let console = config.run_dir.console_log();
        let records = Arc::clone(&self.records);
        let diag: VmmDiagnostics = diagnostics.clone();
        let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            if let Some(ending) = exit.recv().await {
                // "after the reaper event": the handle read at the moment the exit watch fires.
                let tail_at_exit = diag.console_tail();
                let console_file = std::fs::read_to_string(&console)
                    .unwrap_or_else(|e| format!("<console {} unreadable: {e}>", console.display()));
                let idx = {
                    let mut r = records.lock().unwrap();
                    r.push(Record { console_file, tail_at_exit, tail_later: None });
                    r.len() - 1
                };
                let _ = exit_tx.send(ending);
                // a later read of the SAME handle, in case the capture task drained after exit
                tokio::time::sleep(Duration::from_millis(500)).await;
                let later = diag.console_tail();
                records.lock().unwrap()[idx].tail_later = Some(later.unwrap_or_default());
            }
        });
        Ok(VmProcess { control, exit: VmExitWatch::new(exit_rx), diagnostics })
    }
    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

fn rustc_static_musl(src: &Path, out: &Path) {
    let status = Command::new("rustc")
        .args(["--edition", "2021", "-C", "opt-level=0", "-C", "target-feature=+crt-static"])
        .args(["--target", "x86_64-unknown-linux-musl"])
        .arg("-o")
        .arg(out)
        .arg(src)
        .status()
        .expect("spawn rustc");
    assert!(status.success(), "rustc static musl {}", out.display());
}

fn install(host_bin: &Path, dest: &Path) {
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::copy(host_bin, dest).unwrap();
    let mut p = std::fs::metadata(dest).unwrap().permissions();
    p.set_mode(0o755);
    std::fs::set_permissions(dest, p).unwrap();
}

fn with_mounted_rootfs_copy(tmp: &Path, master: &Path, edit: impl FnOnce(&Path)) -> PathBuf {
    let rootfs_copy = tmp.join("rootfs.ext4");
    std::fs::copy(master, &rootfs_copy).unwrap();
    let mnt = tmp.join("rootfs-mnt");
    std::fs::create_dir_all(&mnt).unwrap();
    let out = Command::new("losetup").args(["--find", "--show"]).arg(&rootfs_copy).output().unwrap();
    assert!(out.status.success(), "losetup: {}", String::from_utf8_lossy(&out.stderr));
    let loop_dev = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    assert!(Command::new("mount").arg(&loop_dev).arg(&mnt).status().unwrap().success());
    edit(&mnt);
    assert!(Command::new("umount").arg(&mnt).status().unwrap().success());
    let _ = Command::new("losetup").arg("-d").arg(&loop_dev).status();
    rootfs_copy
}

fn job_toml(id: &str, command: &str, args: &[&str], kernel: &Path, rootfs: &Path) -> String {
    let a = args.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", ");
    format!(
        "[job]\nid = \"{id}\"\n\n[vm]\ncommand = \"{command}\"\nargs = [{a}]\nkernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = 500\nmemory_bytes = 134217728\n",
        kernel.display(),
        rootfs.display()
    )
}

async fn state_of(cfg: &Path, id: &str) -> Option<AllocStateWire> {
    describe(DescribeArgs { id: id.to_owned(), config_path: cfg.to_path_buf() })
        .await
        .ok()
        .and_then(|o| o.snapshot.rows.first().map(|r| r.state))
}

async fn wait_state(
    cfg: &Path,
    id: &str,
    want: &dyn Fn(AllocStateWire) -> bool,
    max: Duration,
) -> Option<AllocStateWire> {
    let t0 = std::time::Instant::now();
    let mut last = None;
    while t0.elapsed() < max {
        if let Some(s) = state_of(cfg, id).await {
            last = Some(s);
            if want(s) || matches!(s, AllocStateWire::Terminated | AllocStateWire::Failed) {
                return Some(s);
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    last
}

/// `overdrive-init: <stage> elapsed_ms=<n>` -> n, for the LAST occurrence of `stage`.
fn stage_ms(text: &str, stage: &str) -> Option<u64> {
    text.lines()
        .filter_map(|l| {
            let rest = l.split("overdrive-init: ").nth(1)?;
            let (s, tail) = rest.split_once(" elapsed_ms=")?;
            if s == stage {
                tail.trim().parse::<u64>().ok()
            } else {
                None
            }
        })
        .last()
}

struct Row {
    id: String,
    f_sr: bool,
    f_gc: bool,
    dur_f: Option<i64>,
    t_sr: bool,
    t_gc: bool,
}

#[tokio::main]
async fn main() {
    let cycles: usize = std::env::args().nth(1).map_or(30, |s| s.parse().unwrap());
    println!(
        "# uname -r: {}",
        String::from_utf8_lossy(&Command::new("uname").arg("-r").output().unwrap().stdout).trim()
    );
    let cpu = std::fs::read_to_string("/proc/cpuinfo").unwrap();
    println!("# {}", cpu.lines().find(|l| l.starts_with("model name")).unwrap_or("model name: ?"));
    println!("# VMM_CONSOLE_TAIL_MAX_BYTES={VMM_CONSOLE_TAIL_MAX_BYTES} STDERR_TAIL_LINES={STDERR_TAIL_LINES}");

    let staging = default_staging_root();
    let fixture = VmFixture::provision(&staging).expect("provision VM fixture");
    println!("# cloud-hypervisor: {}", fixture.cloud_hypervisor_version.trim());
    let work = tempfile::Builder::new().prefix("nbp-v4-").tempdir_in(&staging).unwrap();
    let spin_src = work.path().join("spin.rs");
    std::fs::write(
        &spin_src,
        "fn main() { loop { std::thread::sleep(std::time::Duration::from_secs(3600)); } }",
    )
    .unwrap();
    let spin = work.path().join("spin");
    rustc_static_musl(&spin_src, &spin);
    // slow: installs a SIGTERM handler; on TERM sleeps argv[1] ms then exits 0.
    let slow_src = work.path().join("slow.rs");
    std::fs::write(
        &slow_src,
        r#"use std::sync::atomic::{AtomicBool, Ordering};
static TERM: AtomicBool = AtomicBool::new(false);
extern "C" fn on_term(_: i32) { TERM.store(true, Ordering::SeqCst); }
extern "C" { fn signal(sig: i32, h: extern "C" fn(i32)) -> usize; }
fn main() {
    let ms: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    unsafe { signal(15, on_term); }
    while !TERM.load(Ordering::SeqCst) { std::thread::sleep(std::time::Duration::from_millis(1)); }
    std::thread::sleep(std::time::Duration::from_millis(ms));
    std::process::exit(0);
}"#,
    )
    .unwrap();
    let slow = work.path().join("slow");
    rustc_static_musl(&slow_src, &slow);
    // The VmFixture-staged rootfs lacks the guest transport modules (smoke4.log: "creating the
    // transport dummy device failed: Not supported"); use the guest-kit kernel and rootfs the Metal
    // runner forwards (OVERDRIVE_METAL_KERNEL / OVERDRIVE_METAL_ROOTFS, prepared by guest-kit-prepare).

    // Test-init wrapper PID 1 (the shape of the guest kit's `overdrive.testinit=` PID 1,
    // overdrive-testing/src/guest_kit.rs:111-115): load the stock `dummy` link driver the production
    // guest transport setup needs, then exec the production init (renamed /sbin/init-real).
    let wrap_src = work.path().join("wrap.rs");
    std::fs::write(
        &wrap_src,
        r#"extern "C" {
    fn syscall(n: i64, ...) -> i64;
    fn open(path: *const u8, flags: i32, ...) -> i32;
    fn execv(path: *const u8, argv: *const *const u8) -> i32;
}
fn main() {
    unsafe {
        for m in [b"/modules/dummy.ko\0".as_ref()] {
            let fd = open(m.as_ptr(), 0);
            if fd >= 0 { let r = syscall(313, fd as i64, b"\0".as_ptr(), 0i64); eprintln!("nbp-wrap: finit_module -> {r}"); } else { eprintln!("nbp-wrap: open failed"); }
        }
        let argv = [b"/sbin/init-real\0".as_ptr(), std::ptr::null()];
        execv(b"/sbin/init-real\0".as_ptr(), argv.as_ptr());
    }
}"#,
    )
    .unwrap();
    let wrap = work.path().join("wrap");
    rustc_static_musl(&wrap_src, &wrap);
    let dummy_ko = work.path().join("dummy.ko");
    {
        let uname = String::from_utf8(Command::new("uname").arg("-r").output().unwrap().stdout).unwrap();
        let zst = format!("/lib/modules/{}/kernel/drivers/net/dummy.ko.zst", uname.trim());
        let st = Command::new("zstd").args(["-d", "-f", "-q", "-o"]).arg(&dummy_ko).arg(&zst).status().unwrap();
        assert!(st.success(), "zstd -d {zst}");
    }
    let fwd_ko = {
        let uname = String::from_utf8(Command::new("uname").arg("-r").output().unwrap().stdout).unwrap();
        let p = std::env::current_dir().unwrap().join("target/kmod").join(uname.trim()).join("ovd_fwd.ko");
        assert!(p.is_file(), "built guest forwarder module missing at {} (run `cargo xtask kmod-build`)", p.display());
        p
    };
    let kit_kernel = PathBuf::from(std::env::var("OVERDRIVE_METAL_KERNEL").expect("OVERDRIVE_METAL_KERNEL"));
    let kit_rootfs = PathBuf::from(std::env::var("OVERDRIVE_METAL_ROOTFS").expect("OVERDRIVE_METAL_ROOTFS"));
    println!("# kernel={} rootfs={}", kit_kernel.display(), kit_rootfs.display());
    let rootfs = with_mounted_rootfs_copy(work.path(), &kit_rootfs, |m| {
        for init in ["sbin/init", "init"] {
            std::fs::rename(m.join(init), m.join("sbin/init-real")).ok();
        }
        // /sbin/init and /init are the same binary: the first rename moved it; restore the second path as wrapper
        install(&wrap, &m.join("sbin/init"));
        install(&wrap, &m.join("init"));
        install(&dummy_ko, &m.join("modules/dummy.ko"));
        install(&fwd_ko, &m.join("lib/overdrive/ovd_fwd.ko"));
        install(&spin, &m.join("sbin/nbp-spin"));
        install(&slow, &m.join("sbin/nbp-slow"));
    });

    let records: Arc<Mutex<Vec<Record>>> = Arc::new(Mutex::new(Vec::new()));
    let vmm = Arc::new(ProbeVmm { inner: CloudHypervisorVmm::new(), records: Arc::clone(&records) });
    let beacon: Arc<dyn overdrive_core::vm::beacon::BeaconIo> =
        Arc::new(overdrive_host::VsockBeaconIo::new());
    let tmp = tempfile::Builder::new().prefix("vm-serve-").tempdir_in(&staging).unwrap();
    // tempfile makes 0700 dirs; the confined (uid-dropped) VMM must traverse every ancestor of
    // the clone-staging dir, as it can under a production data_dir.
    std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();
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
    let vmm2: Arc<dyn Vmm> = vmm.clone();
    let server = run_with_adapters(args, Arc::new(RealCgroupFs::new()), move |config, fs| {
        let mut a = NodeAdapters::host(config, fs, Arc::new(SimKek::for_boot()))?;
        a.boot.dataplane =
            Arc::new(SimDataplaneComposer::new(Arc::new(SimDataplane::new()), Ipv4Addr::LOCALHOST));
        a.vm.vmm = vmm2;
        a.vm.beacon = beacon;
        a.vm.capability = VmCapability::Required;
        Ok(a)
    })
    .await
    .expect("serve");
    let cfg = tmp.path().join("conf").join(".overdrive").join("config");
    // ENVIRONMENT ADAPTATION: serve hardens `<data_dir>` to root:root 0750 (observed via namei in
    // results/smoke3.log) while only `<data_dir>/vm` and its clone-staging root get the 4200
    // traverse grant, so the uid-dropped VMM cannot reach its disk clone on a native host.
    // Traverse-only for "other" on the one ancestor lets the native boot proceed.
    std::fs::set_permissions(tmp.path().join("data"), std::fs::Permissions::from_mode(0o711)).unwrap();

    let delays = [0u64, 2, 5, 10, 20, 40];
    let mut rows: Vec<Row> = Vec::new();
    for variant in ["spin", "slow"] {
        for i in 0..cycles {
            let id = format!("nbp-{variant}-{i:03}");
            let (cmd, a): (&str, Vec<String>) = if variant == "spin" {
                ("/sbin/nbp-spin", vec![])
            } else {
                ("/sbin/nbp-slow", vec![delays[i % delays.len()].to_string()])
            };
            let a_ref: Vec<&str> = a.iter().map(String::as_str).collect();
            let spec = tmp.path().join(format!("{id}.toml"));
            std::fs::write(&spec, job_toml(&id, cmd, &a_ref, &kit_kernel, &rootfs)).unwrap();
            let before = records.lock().unwrap().len();
            let sub = match deploy(DeployArgs { spec, config_path: cfg.clone() }).await {
                Ok(s) => s,
                Err(e) => {
                    println!("CYCLE {id}: deploy failed: {e}");
                    continue;
                }
            };
            let run = wait_state(
                &cfg,
                &sub.workload_id,
                &|s| s == AllocStateWire::Running,
                Duration::from_secs(90),
            )
            .await;
            if run != Some(AllocStateWire::Running) {
                println!("CYCLE {id}: never Running (last {run:?})");
                let dbg = tmp.path().join("data/vm/clone-staging");
                for (prog, a) in [("namei", vec!["-ln".to_owned(), dbg.display().to_string()]), ("ls", vec!["-lan".to_owned(), dbg.display().to_string()]), ("cat", vec!["/sys/kernel/security/lsm".to_owned()])] {
                    let o = Command::new(prog).args(&a).output().unwrap();
                    println!("DEBUG {prog} {a:?}:\n{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            let _ = stop(StopArgs { id: sub.workload_id.clone(), config_path: cfg.clone() }).await;
            let term = wait_state(&cfg, &sub.workload_id, &|_| false, Duration::from_secs(40)).await;
            for _ in 0..60 {
                let g = records.lock().unwrap();
                if g.len() > before && g[before].tail_later.is_some() {
                    break;
                }
                drop(g);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let rec = records.lock().unwrap().get(before).cloned();
            if run != Some(AllocStateWire::Running) {
                if let Some(r) = &rec {
                    let lines: Vec<&str> = r.console_file.lines().collect();
                    println!("DEBUG console tail:\n{}", lines[lines.len().saturating_sub(15)..].join("\n"));
                }
            }
            let Some(rec) = rec else {
                println!("CYCLE {id}: no decorator record (terminal {term:?})");
                continue;
            };
            let f_sr = stage_ms(&rec.console_file, "shutdown-received");
            let f_gc = stage_ms(&rec.console_file, "group-complete");
            let t = rec.tail_at_exit.clone().unwrap_or_default();
            let tl = rec.tail_later.clone().unwrap_or_default();
            let t_sr = stage_ms(&t, "shutdown-received");
            let t_gc = stage_ms(&t, "group-complete");
            let tl_sr = stage_ms(&tl, "shutdown-received");
            let tl_gc = stage_ms(&tl, "group-complete");
            let dur_f = f_sr.zip(f_gc).map(|(a, b)| b as i64 - a as i64);
            let dur_t = t_sr.zip(t_gc).map(|(a, b)| b as i64 - a as i64);
            println!(
                "CYCLE {id}: terminal={term:?} file[sr={f_sr:?} gc={f_gc:?} dur={dur_f:?}] tail_at_exit[len={} sr={t_sr:?} gc={t_gc:?} dur={dur_t:?}] tail_later[len={} sr={tl_sr:?} gc={tl_gc:?}] console_file_len={}",
                t.len(),
                tl.len(),
                rec.console_file.len()
            );
            rows.push(Row {
                id: id.clone(),
                f_sr: f_sr.is_some(),
                f_gc: f_gc.is_some(),
                dur_f,
                t_sr: t_sr.is_some(),
                t_gc: t_gc.is_some(),
            });
            if i == 0 {
                println!(
                    "----- RAW console file ({id}) -----\n{}\n----- RAW tail_at_exit ({id}) -----\n{t}\n----- RAW tail_later ({id}) -----\n{tl}\n-----",
                    rec.console_file
                );
            }
        }
    }
    println!("--- SUMMARY ---");
    for name in ["spin", "slow"] {
        let r: Vec<&Row> = rows.iter().filter(|x| x.id.starts_with(&format!("nbp-{name}-"))).collect();
        let n = r.len();
        let file_both = r.iter().filter(|x| x.f_sr && x.f_gc).count();
        let tail_both = r.iter().filter(|x| x.t_sr && x.t_gc).count();
        let durs_f: Vec<i64> = r.iter().filter_map(|x| x.dur_f).collect();
        let mut dist: Vec<i64> = durs_f.clone();
        dist.sort_unstable();
        dist.dedup();
        let le1 = durs_f.iter().filter(|d| **d <= 1).count();
        println!(
            "{name}: cycles={n} console_file_has_both={file_both} diagnostics_tail_has_both={tail_both} distinct_file_durations={dist:?} durations<=1ms={le1}/{}",
            durs_f.len()
        );
        println!("{name}: file durations={durs_f:?}");
    }
    let _ = server.shutdown().await;
}
