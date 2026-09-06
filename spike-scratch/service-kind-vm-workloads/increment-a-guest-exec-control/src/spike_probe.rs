use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::process::Command;
use std::time::Duration;

fn counter_value(path: &str) -> u64 {
    fs::read_to_string(path).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

fn increment(path: &str) {
    let mut file = OpenOptions::new().create(true).read(true).write(true).truncate(false)
        .open(path).expect("open counter");
    let mut current = String::new();
    file.read_to_string(&mut current).expect("read counter");
    let next = current.trim().parse::<u64>().unwrap_or(0) + 1;
    file.set_len(0).expect("truncate counter");
    file.seek(SeekFrom::Start(0)).expect("seek counter");
    writeln!(file, "{next}").expect("write counter");
    file.sync_all().expect("sync counter");
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    match args.get(1).map(String::as_str) {
        Some("ok") => {}
        Some("fail") => std::process::exit(7),
        Some("signal") => {
            nix::sys::signal::raise(nix::sys::signal::Signal::SIGTERM).expect("raise SIGTERM");
        }
        Some("sleep") => std::thread::sleep(Duration::from_secs(30)),
        Some("fork") => {
            let child = std::env::current_exe().expect("current executable");
            let _descendant = Command::new(child).arg("descendant").spawn().expect("spawn descendant");
            std::thread::sleep(Duration::from_secs(30));
        }
        Some("descendant") => std::thread::sleep(Duration::from_secs(30)),
        Some("count") => {
            increment("/probe-count");
            std::thread::sleep(Duration::from_secs(2));
        }
        Some("check1") if counter_value("/probe-count") == 1 => {}
        Some("check2") if counter_value("/probe-count") == 2 => {}
        Some("check1" | "check2") => std::process::exit(42),
        _ => std::process::exit(127),
    }
}
