//! The per-module guest broker: boots ONE guest, serves RUN requests, exits
//! (killing the guest) when its anchor nextest process is gone.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use modguest_probe::{alive, netns_id};
use overdrive_testing::guest_kit::cid_bands::FWD_GUEST_BOUNDARY;
use overdrive_testing::guest_kit::{GuestFile, GuestImageSpec, GuestKit, ScriptedBootExtra, WorkDir};
use overdrive_testing::vm_fixture::default_staging_root;

const HARD_CAP: Duration = Duration::from_secs(30 * 60);

fn tool(name: &str) -> GuestFile {
    let host = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"]
        .into_iter()
        .map(|d| Path::new(d).join(name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("tool {name} missing"));
    GuestFile { host, guest: Path::new("/usr/bin").join(name), with_shared_libraries: true }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let sock = PathBuf::from(&a[1]);
    let exe = PathBuf::from(&a[2]);
    let anchor_pid: u32 = a[3].parse().unwrap();
    let anchor_start = a[4].clone();
    let begun = Instant::now();
    eprintln!("MODGUEST_BROKER start pid={} netns={} anchor={anchor_pid}", std::process::id(), netns_id());

    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("provision");
    let work = WorkDir::create(&staging, "modguest-broker").expect("workdir");
    let mut spec = GuestImageSpec::scripted();
    spec.extra = vec![
        GuestFile { host: exe.canonicalize().unwrap(), guest: exe.clone(), with_shared_libraries: true },
        tool("env"),
    ];
    let image = kit.image(work.path(), "modguest", &spec).expect("image");
    let staged = begun.elapsed();
    let mut guest = kit
        .boot_scripted_with(
            &image,
            FWD_GUEST_BOUNDARY.raw(14),
            &work.path().join("run"),
            &ScriptedBootExtra { cmdline: Vec::new(), memory_mib: Some(1024), vcpus: Some(2) },
        )
        .expect("boot");
    let booted = begun.elapsed();
    eprintln!(
        "MODGUEST_TIMING boots=1 staging_ms={} boot_ms={} total_ms={}",
        staged.as_millis(),
        (booted - staged).as_millis(),
        booted.as_millis()
    );
    let mut offset = guest.vm.console().len();

    let listener = UnixListener::bind(&sock).expect("bind");
    listener.set_nonblocking(true).unwrap();
    let mut runs = 0_u32;
    let reason = loop {
        if anchor_pid != 0 && !alive(anchor_pid, &anchor_start) {
            break "anchor nextest process gone";
        }
        if begun.elapsed() > HARD_CAP {
            break "hard cap";
        }
        let stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
        };
        stream.set_nonblocking(false).unwrap();
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        let Some(test) = line.trim_end().strip_prefix("RUN\t") else { continue };
        let t0 = Instant::now();
        let exe_s = exe.display().to_string();
        let code = guest
            .peer()
            .exec(
                &["/usr/bin/env", "OVERDRIVE_TEST_IN_GUEST=1", &exe_s, "--exact", test, "--nocapture"],
                Duration::from_secs(120),
            )
            .unwrap_or(-1);
        std::thread::sleep(Duration::from_millis(300)); // console flush
        let console = guest.vm.console();
        let delta = console.get(offset..).unwrap_or("").to_owned();
        offset = console.len();
        runs += 1;
        eprintln!("MODGUEST_BROKER ran test={test} exit={code} exec_ms={}", t0.elapsed().as_millis());
        let mut out = &stream;
        let _ = write!(
            out,
            "EXIT\t{code}\t{runs}\t{}\t{}\t{}\t{}\n",
            std::process::id(),
            netns_id(),
            t0.elapsed().as_millis(),
            delta.len()
        );
        let _ = out.write_all(delta.as_bytes());
    };
    eprintln!("MODGUEST_BROKER exiting reason={reason} runs={runs} lived_ms={}", begun.elapsed().as_millis());
    let _ = std::fs::remove_file(&sock);
    let _ = guest.vm.kill();
}
