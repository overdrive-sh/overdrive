//! boot-cost driver: boots one guest per sample through the repo's guest-kit fixture
//! (production `CloudHypervisorVmm` path, same as the kvm fixtures), swapping only PID 1
//! (`GuestKit.init_bin`). Variants are interleaved round-robin so host drift hits all equally.
//! Usage: bootcost-driver <measured_rounds> <warmup_rounds> <label=init_path>...
//! stdout: one `BCJSON {...}` line per boot (+ `BCENV`, `BCFAIL`).
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use overdrive_core::guest_transport::{GUEST_CID_BASE, GUEST_NODE_GATEWAY};
use overdrive_core::vm::beacon::{BEACON_VSOCK_PORT, BeaconMessage};
use overdrive_testing::guest_kit::{GuestBoot, GuestImage, GuestImageSpec, GuestKit, WorkDir};
use overdrive_testing::guest_lane::{HostListeners, LANE_LAUNCH_TOKEN, lane_workload_addr};
use overdrive_testing::vm_fixture::default_staging_root;
use overdrive_testing::vsock::{VMADDR_CID_ANY, VsockListener, VsockStream};

fn read_line(stream: &VsockStream, bound: Duration) -> Option<String> {
    let deadline = Instant::now() + bound;
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match stream.read_within(&mut byte, left).ok()? {
            None | Some(0) => return None,
            Some(_) if byte[0] == b'\n' => return Some(String::from_utf8_lossy(&line).into_owned()),
            Some(_) => line.push(byte[0]),
        }
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args[1].parse().expect("measured rounds");
    let warmup: usize = args[2].parse().expect("warmup rounds");
    let variants: Vec<(String, String)> = args[3..]
        .iter()
        .map(|a| {
            let (l, p) = a.split_once('=').expect("label=path");
            (l.to_owned(), p.to_owned())
        })
        .collect();

    let staging = default_staging_root();
    let base_kit = GuestKit::provision(&staging).expect("guest kit provisions");
    let work = WorkDir::create(&staging, "bootcost").expect("work dir");
    println!(
        "BCENV {{\"kernel_release\":{:?},\"kernel\":{:?},\"ch\":{:?},\"real_init\":{:?}}}",
        base_kit.kernel_release, base_kit.kernel_path, base_kit.cloud_hypervisor_bin, base_kit.init_bin
    );

    let mut images = Vec::new();
    for (label, path) in &variants {
        let mut kit = base_kit.clone();
        kit.init_bin = path.into();
        let img = kit
            .image(work.path(), &format!("img-{label}"), &GuestImageSpec::workload())
            .expect("image builds");
        images.push((label.clone(), kit, img));
    }

    let beacon = VsockListener::bind(VMADDR_CID_ANY, BEACON_VSOCK_PORT.as_u32()).expect("beacon bind");
    let HostListeners { beacon: adv, stream, control } = HostListeners::bind().expect("listeners");
    let (adv_p, stream_p, control_p) = (adv.port(), stream.port(), control.port());
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    // Holds the guest owner's control session open (the lane's `Holder`).
    let hold = std::thread::spawn(move || {
        let mut held: Vec<VsockStream> = Vec::new();
        let mut buf = [0u8; 4096];
        while !stop2.load(Ordering::SeqCst) {
            if let Ok(Some((s, _))) = control.accept_within(Duration::from_millis(50)) {
                held.push(s);
            }
            held.retain(|s| !matches!(s.read_within(&mut buf, Duration::ZERO), Ok(Some(0)) | Err(_)));
        }
    });

    let base_cid = GUEST_CID_BASE + 0x2100;
    let mut seq = 0u32;
    for round in 0..(warmup + n) {
        for (label, kit, img) in &images {
            let cid = base_cid + (seq % 32);
            seq += 1;
            let token = format!(
                "overdrive.transport=vsock2,addr={},dns={GUEST_NODE_GATEWAY},launch={LANE_LAUNCH_TOKEN},beacon={adv_p},stream={stream_p},control={control_p}",
                lane_workload_addr(cid)
            );
            run_one(label, round, round < warmup, kit, img, cid, &token, &beacon, &adv, work.path());
        }
    }
    stop.store(true, Ordering::SeqCst);
    let _ = hold.join();
}

#[allow(clippy::too_many_arguments)]
fn run_one(
    label: &str,
    round: usize,
    warm: bool,
    kit: &GuestKit,
    img: &GuestImage,
    cid: u32,
    token: &str,
    beacon: &VsockListener,
    adv: &VsockListener,
    staging: &std::path::Path,
) {
    let run = WorkDir::create(staging, "bc-run").expect("run dir");
    let t0 = Instant::now();
    let vm = match kit.boot(&GuestBoot {
        image: img,
        cid,
        cmdline_extra: vec![token.to_owned()],
        memory_mib: 256,
        vcpus: 1,
        run_dir: run.path(),
    }) {
        Ok(vm) => vm,
        Err(e) => {
            println!("BCFAIL {label} {round} boot: {e}");
            return;
        }
    };
    let t_boot_ret = t0.elapsed();
    let mut conn: Option<VsockStream> = None;
    let mut t_conn = None;
    let deadline = t0 + Duration::from_secs(60);
    let t_ready = loop {
        if conn.is_none() {
            let acc = match beacon.accept_within(Duration::from_millis(25)) {
                Ok(None) => adv.accept_within(Duration::from_millis(25)),
                other => other,
            };
            match acc {
                Ok(Some((s, peer))) => {
                    eprintln!("BCDBG accepted peer cid={} port={} want cid={cid}", peer.cid, peer.port);
                    if peer.cid == cid {
                        t_conn = Some(t0.elapsed());
                        conn = Some(s);
                    }
                }
                Ok(None) => {}
                Err(e) => eprintln!("BCDBG accept err {e}"),
            }
        }
        if let Some(s) = &conn {
            if let Some(line) = read_line(s, Duration::from_millis(100)) {
                if line.parse::<BeaconMessage>().is_ok_and(|m| matches!(m, BeaconMessage::Ready { .. })) {
                    break t0.elapsed();
                }
                println!("BCFAIL {label} {round} sent {line:?} instead of READY");
                return;
            }
        }
        if vm.try_exit().is_some() {
            println!("BCFAIL {label} {round} exited before READY; console:\n{}", vm.console_tail(4000));
            return;
        }
        if Instant::now() >= deadline {
            println!("BCFAIL {label} {round} no READY in 60s; console:\n{}", vm.console_tail(4000));
            return;
        }
    };
    let mut bc = String::new();
    if label != "R" {
        if !vm.wait_console("BC_END", Duration::from_secs(15)) {
            println!("BCFAIL {label} {round} no BC_END; console:\n{}", vm.console_tail(4000));
            return;
        }
        for l in vm.console().lines() {
            if let Some(rest) = l.trim().strip_prefix("BC ") {
                let mut it = rest.split_whitespace();
                if let (Some(a), Some(b), Some(c)) = (it.next(), it.next(), it.next()) {
                    if !bc.is_empty() {
                        bc.push(',');
                    }
                    bc.push_str(&format!("[\"{a}\",{b},{c}]"));
                }
            }
        }
    }
    println!(
        "BCJSON {{\"variant\":\"{label}\",\"round\":{round},\"warm\":{warm},\"cid\":{cid},\"boot_call_ms\":{:.3},\"conn_ms\":{:.3},\"ready_ms\":{:.3},\"bc\":[{bc}]}}",
        ms(t_boot_ret),
        t_conn.map_or(-1.0, ms),
        ms(t_ready)
    );
    drop(vm);
}
