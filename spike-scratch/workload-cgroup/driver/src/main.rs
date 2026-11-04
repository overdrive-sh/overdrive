//! workload-cgroup driver (V42): boots one real Cloud Hypervisor guest per sample through the repo's guest-kit
//! fixture (the production `CloudHypervisorVmm` path), swapping only PID 1 (`GuestKit.init_bin`) for the
//! generated instrumented copy of `overdrive-init`. After READY it sends `EXEC ["/wcwl", <mode>]`, reads the
//! beacon until `EXIT`, and prints one `WCJSON` line per boot with the guest console's `WC_*` / `WL*` lines.
//! Variants are interleaved round-robin so host drift hits all of them equally.
//!
//! usage: wcdriver <warmup_per_variant> <init_main> <init_plus> <init_copy> <label=ovdwc_mode,wl_mode,count[,m|p|c]>...
//! (`m` = init over the production guest object; `p` = over a copy of it that also carries the listen programs;
//! `c` = over the byte-faithful copy of it without them.)
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use overdrive_core::guest_transport::{GUEST_CID_BASE, GUEST_NODE_GATEWAY};
use overdrive_core::vm::beacon::{BEACON_VSOCK_PORT, BeaconMessage};
use overdrive_testing::guest_kit::{GuestBoot, GuestFile, GuestImage, GuestImageSpec, GuestKit, WorkDir};
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

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

struct Variant {
    label: String,
    ovdwc: String,
    wl: String,
    count: usize,
    init: char,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let warmup: usize = args[1].parse().expect("warmup per variant");
    let init_main = args[2].clone();
    let init_plus = args[3].clone();
    let init_copy = args[4].clone();
    let variants: Vec<Variant> = args[5..]
        .iter()
        .map(|a| {
            let (label, rest) = a.split_once('=').expect("label=ovdwc,wl,count");
            let mut it = rest.split(',');
            Variant {
                label: label.to_owned(),
                ovdwc: it.next().expect("ovdwc").to_owned(),
                wl: it.next().expect("wl").to_owned(),
                count: it.next().expect("count").parse().expect("count"),
                init: it.next().and_then(|x| x.chars().next()).unwrap_or('m'),
            }
        })
        .collect();

    let staging = default_staging_root();
    let mut kit = GuestKit::provision(&staging).expect("guest kit provisions");
    println!(
        "WCENV {{\"kernel_release\":\"{}\",\"ch\":\"{}\",\"real_init\":\"{}\",\"swapped_init\":\"{}\",\"swapped_init_plus\":\"{}\",\"swapped_init_copy\":\"{}\"}}",
        kit.kernel_release,
        kit.cloud_hypervisor_bin.display(),
        kit.init_bin.display(),
        init_main,
        init_plus,
        init_copy
    );
    let work = WorkDir::create(&staging, "wcdrv").expect("work dir");
    let mut spec = GuestImageSpec::workload();
    spec.extra.push(GuestFile {
        host: std::env::var("WC_WL_BIN").expect("WC_WL_BIN").into(),
        guest: "/wcwl".into(),
        with_shared_libraries: false,
    });
    kit.init_bin = init_main.into();
    let kit_main = kit.clone();
    let img_main = kit_main.image(work.path(), "img-main", &spec).expect("image builds");
    kit.init_bin = init_plus.into();
    let kit_plus = kit.clone();
    let img_plus = kit_plus.image(work.path(), "img-plus", &spec).expect("image builds");
    kit.init_bin = init_copy.into();
    let kit_copy = kit.clone();
    let img_copy = kit_copy.image(work.path(), "img-copy", &spec).expect("image builds");

    let beacon = VsockListener::bind(VMADDR_CID_ANY, BEACON_VSOCK_PORT.as_u32()).expect("beacon bind");
    let HostListeners { beacon: adv, stream, control } = HostListeners::bind().expect("listeners");
    let (adv_p, stream_p, control_p) = (adv.port(), stream.port(), control.port());
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    // Holds the guest owner's control session and any guest-flow stream open (the lane's `Holder`).
    let streams: Arc<Mutex<Vec<(u32, Vec<u8>)>>> = Arc::new(Mutex::new(Vec::new()));
    let streams2 = Arc::clone(&streams);
    let hold = std::thread::spawn(move || {
        let mut held: Vec<VsockStream> = Vec::new();
        let mut flows: Vec<(u32, VsockStream)> = Vec::new();
        let mut buf = [0u8; 4096];
        while !stop2.load(Ordering::SeqCst) {
            if let Ok(Some((s, _))) = control.accept_within(Duration::from_millis(25)) {
                held.push(s);
            }
            if let Ok(Some((s, peer))) = stream.accept_within(Duration::from_millis(25)) {
                flows.push((peer.cid, s));
            }
            held.retain(|s| !matches!(s.read_within(&mut buf, Duration::ZERO), Ok(Some(0)) | Err(_)));
            // Guest-flow streams: record what the guest owner sends (the first bytes carry the resolved flow).
            for (cid, s) in &flows {
                if let Ok(Some(n)) = s.read_within(&mut buf, Duration::ZERO) {
                    if n > 0 {
                        streams2.lock().unwrap().push((*cid, buf[..n].to_vec()));
                    }
                }
            }
        }
    });

    // Build the interleaved schedule: round r runs every variant that still has samples; warmups first.
    let max = variants.iter().map(|v| v.count).max().unwrap_or(0);
    let mut schedule: Vec<(usize, usize, bool)> = Vec::new();
    for r in 0..warmup {
        for (i, _) in variants.iter().enumerate() {
            schedule.push((i, r, true));
        }
    }
    for r in 0..max {
        for (i, v) in variants.iter().enumerate() {
            if r < v.count {
                schedule.push((i, r, false));
            }
        }
    }
    let base_cid = GUEST_CID_BASE + 0x2200;
    for (seq, (i, round, warm)) in schedule.into_iter().enumerate() {
        let v = &variants[i];
        let cid = base_cid + (seq as u32 % 32);
        let token = format!(
            "overdrive.transport=vsock2,addr={},dns={GUEST_NODE_GATEWAY},launch={LANE_LAUNCH_TOKEN},beacon={adv_p},stream={stream_p},control={control_p}",
            lane_workload_addr(cid)
        );
        let (kit, img) = match v.init {
            'p' => (&kit_plus, &img_plus),
            'c' => (&kit_copy, &img_copy),
            _ => (&kit_main, &img_main),
        };
        run_one(v, round, warm, kit, img, cid, &[token, format!("ovdwc={}", v.ovdwc)], &beacon, &adv, work.path(), &streams);
    }
    stop.store(true, Ordering::SeqCst);
    let _ = hold.join();
}

#[allow(clippy::too_many_arguments)]
fn run_one(
    v: &Variant,
    round: usize,
    warm: bool,
    kit: &GuestKit,
    img: &GuestImage,
    cid: u32,
    cmdline: &[String],
    beacon: &VsockListener,
    adv: &VsockListener,
    staging: &std::path::Path,
    streams: &Arc<Mutex<Vec<(u32, Vec<u8>)>>>,
) {
    let label = &v.label;
    // a CID is reused every 32 boots: drop what an earlier guest at this CID sent on the flow stream
    streams.lock().unwrap().retain(|(c, _)| *c != cid);
    let run = WorkDir::create(staging, "wc-run").expect("run dir");
    let t0 = Instant::now();
    let vm = match kit.boot(&GuestBoot {
        image: img,
        cid,
        cmdline_extra: cmdline.to_vec(),
        memory_mib: 256,
        vcpus: 1,
        run_dir: run.path(),
    }) {
        Ok(vm) => vm,
        Err(e) => {
            println!("WCFAIL {label} {round} boot: {e}");
            return;
        }
    };
    let mut conn: Option<VsockStream> = None;
    let deadline = t0 + Duration::from_secs(60);
    let t_ready = loop {
        if conn.is_none() {
            let acc = match beacon.accept_within(Duration::from_millis(25)) {
                Ok(None) => adv.accept_within(Duration::from_millis(25)),
                other => other,
            };
            if let Ok(Some((s, peer))) = acc {
                if peer.cid == cid {
                    conn = Some(s);
                }
            }
        }
        if let Some(s) = &conn {
            if let Some(line) = read_line(s, Duration::from_millis(100)) {
                if line.parse::<BeaconMessage>().is_ok_and(|m| matches!(m, BeaconMessage::Ready { .. })) {
                    break t0.elapsed();
                }
                println!("WCFAIL {label} {round} sent {line:?} instead of READY");
                return;
            }
        }
        if vm.try_exit().is_some() {
            println!("WCFAIL {label} {round} exited before READY; console:\n{}", vm.console_tail(4000));
            return;
        }
        if Instant::now() >= deadline {
            println!("WCFAIL {label} {round} no READY in 60s; console:\n{}", vm.console_tail(4000));
            return;
        }
    };
    let conn = conn.expect("connected once READY was read");
    let exec = BeaconMessage::Exec { argv: vec!["/wcwl".into(), v.wl.clone()] }.to_string();
    if let Err(e) = (&conn).write_all(format!("{exec}\n").as_bytes()) {
        println!("WCFAIL {label} {round} EXEC write: {e}");
        return;
    }
    let mut exit_status: Option<i32> = None;
    let exit_deadline = Instant::now() + Duration::from_secs(30);
    while exit_status.is_none() && Instant::now() < exit_deadline {
        match read_line(&conn, Duration::from_millis(200)) {
            Some(line) => {
                if let Ok(BeaconMessage::Exit { status }) = line.parse::<BeaconMessage>() {
                    exit_status = Some(status);
                }
            }
            None => {
                if vm.try_exit().is_some() {
                    break;
                }
            }
        }
    }
    let have_end = vm.wait_console("WC_END", Duration::from_secs(10));
    let console = vm.console();
    let mut kept = Vec::new();
    for l in console.lines() {
        let l = l.trim();
        if l.starts_with("WC_") || l.starts_with("WL") {
            kept.push(format!("\"{}\"", esc(l)));
        }
    }
    let stream_hex: Vec<String> = streams
        .lock()
        .unwrap()
        .iter()
        .filter(|(c, _)| *c == cid)
        .map(|(_, b)| format!("\"{}\"", b.iter().map(|x| format!("{x:02x}")).collect::<String>()))
        .collect();
    println!(
        "WCJSON {{\"variant\":\"{label}\",\"ovdwc\":\"{}\",\"wl\":\"{}\",\"round\":{round},\"warm\":{warm},\"cid\":{cid},\"ready_ms\":{:.3},\"exit\":{},\"wc_end\":{have_end},\"stream_hex\":[{}],\"lines\":[{}]}}",
        v.ovdwc,
        v.wl,
        ms(t_ready),
        exit_status.map_or("null".to_owned(), |s| s.to_string()),
        stream_hex.join(","),
        kept.join(",")
    );
    if !have_end || exit_status.is_none() {
        println!("WCFAIL {label} {round} incomplete (wc_end={have_end} exit={exit_status:?}); console tail:\n{}", vm.console_tail(3000));
    }
    drop(vm);
}
