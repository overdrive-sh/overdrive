//! vgw loader: load vgw-bpf ELF with map_pin_path, attach one cgroup/connect4 program, hold.
use std::{fs::File, path::PathBuf, time::Duration};

use aya::{
    programs::{CgroupAttachMode, CgroupSockAddr},
    EbpfLoader,
};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.get(1).map(String::as_str) == Some("info") {
        for p in aya::programs::loaded_programs().flatten() {
            let n = p.name_as_str().unwrap_or("");
            if n.starts_with("cgroup_") || n.starts_with("vgw_") {
                println!("PROG id={} name={} verified_insns={:?} jited={:?}", p.id(), n, p.verified_instruction_count(), p.size_jitted());
            }
        }
        return;
    }
    if a.len() < 5 {
        eprintln!("usage: vgw <elf> <prog> <cgroup> <pin-dir>");
        std::process::exit(2);
    }
    let bytes = std::fs::read(&a[1]).expect("read elf");
    let mut ebpf = EbpfLoader::new()
        .map_pin_path(PathBuf::from(&a[4]))
        .allow_unsupported_maps()
        .load(&bytes)
        .expect("load elf (maps)");
    let prog: &mut CgroupSockAddr = ebpf.program_mut(&a[2]).expect("program").try_into().unwrap();
    if let Err(e) = prog.load() {
        eprintln!("VERIFIER/LOAD FAILED: {e:?}");
        std::process::exit(1);
    }
    let info = prog.info().expect("prog info");
    println!(
        "LOADED prog={} id={} verified_insns={:?} jited_len={:?} uname={}",
        a[2],
        info.id(),
        info.verified_instruction_count(),
        info.size_jitted(),
        std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim()
    );
    let cg = File::open(&a[3]).expect("open cgroup");
    let _link = prog.attach(&cg, CgroupAttachMode::Single).unwrap_or_else(|e| {
        eprintln!("ATTACH FAILED({}): {e:?}", a[3]);
        std::process::exit(1)
    });
    println!("ATTACHED prog={} cgroup={} mode=Single", a[2], a[3]);
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
