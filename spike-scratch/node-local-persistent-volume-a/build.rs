//! Probe build script (throwaway). Runs on the Metal box, where the guest
//! kernel's modules and the host tools live. It produces, in OUT_DIR:
//!  * `nlpv-init`   static-musl PID 1 / workload of the INNER guest;
//!  * `bundle.bin`  files the OUTER guest lacks (dmsetup, mkfs.xfs, xfs_io,
//!                  blkdiscard + their shared-library closure, and the
//!                  decompressed kernel modules dm-log-writes, dm-flakey),
//!                  at the paths they are unpacked to;
//!  * `ch-spike`    the spike-branch Cloud Hypervisor (static musl) when it
//!                  has been built at the conventional path, else empty.
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn run(cmd: &mut Command) -> String {
    let out = cmd.output().unwrap_or_else(|e| panic!("spawn {cmd:?}: {e}"));
    assert!(out.status.success(), "{cmd:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct Bundle(Vec<u8>);
impl Bundle {
    fn add(&mut self, guest_path: &str, mode: u32, data: &[u8]) {
        self.0.extend((guest_path.len() as u32).to_le_bytes());
        self.0.extend(guest_path.as_bytes());
        self.0.extend(mode.to_le_bytes());
        self.0.extend((data.len() as u64).to_le_bytes());
        self.0.extend(data);
    }
}

fn ldd_closure(bin: &str) -> Vec<PathBuf> {
    let text = run(Command::new("ldd").arg(bin));
    let mut v = Vec::new();
    for line in text.lines() {
        // "libfoo.so => /lib/x86_64-linux-gnu/libfoo.so (0x...)" or "/lib64/ld-linux... (0x..)"
        let t = line.trim();
        let path = if let Some((_, rhs)) = t.split_once("=>") {
            rhs.split_whitespace().next().unwrap_or("")
        } else {
            t.split_whitespace().next().unwrap_or("")
        };
        if path.starts_with('/') && Path::new(path).exists() {
            v.push(PathBuf::from(path));
        }
    }
    v
}

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed=guest/nlpv_init.rs");
    println!("cargo:rerun-if-changed=build.rs");

    // 1. inner-guest workload, static musl, std only.
    run(Command::new("rustc").args([
        "--edition", "2021", "-C", "opt-level=2", "-C", "target-feature=+crt-static",
        "--target", "x86_64-unknown-linux-musl", "-o",
    ]).arg(out.join("nlpv-init")).arg(manifest.join("guest/nlpv_init.rs")));

    // 2. bundle of tools + libs + modules.
    let mut b = Bundle(Vec::new());
    let mut seen = std::collections::BTreeSet::new();
    for tool in ["/usr/sbin/dmsetup", "/usr/sbin/mkfs.xfs", "/usr/sbin/xfs_io", "/usr/sbin/blkdiscard", "/usr/bin/strace"] {
        if !Path::new(tool).exists() { continue; }
        let guest = format!("/usr/bin/{}", Path::new(tool).file_name().unwrap().to_str().unwrap());
        if seen.insert(guest.clone()) {
            b.add(&guest, 0o755, &fs::read(tool).unwrap());
        }
        for lib in ldd_closure(tool) {
            let p = fs::canonicalize(&lib).unwrap_or(lib.clone());
            // keep the *requested* path (symlink name) and the real file's bytes
            let gp = lib.to_string_lossy().into_owned();
            if seen.insert(gp.clone()) {
                b.add(&gp, 0o755, &fs::read(&p).unwrap());
            }
        }
    }
    let release = run(Command::new("uname").arg("-r")).trim().to_owned();
    for (m, sub) in [("dm-log-writes", "md"), ("dm-flakey", "md"), ("dm-delay", "md"), ("brd", "block")] {
        let z = format!("/lib/modules/{release}/kernel/drivers/{sub}/{m}.ko.zst");
        let plain = format!("/lib/modules/{release}/kernel/drivers/{sub}/{m}.ko");
        let data = if Path::new(&z).exists() {
            let o = Command::new("zstd").args(["-dc", &z]).output().expect("zstd");
            assert!(o.status.success(), "zstd -dc {z}");
            o.stdout
        } else {
            fs::read(&plain).unwrap_or_else(|e| panic!("{plain}: {e}"))
        };
        b.add(&format!("/nlpv-modules/{m}.ko"), 0o644, &data);
    }
    fs::write(out.join("bundle.bin"), &b.0).unwrap();

    // 3. spike-branch Cloud Hypervisor, if built.
    let spike = manifest.join("../node-local-persistent-volume-ch-target/x86_64-unknown-linux-musl/release/cloud-hypervisor");
    println!("cargo:rerun-if-changed={}", spike.display());
    let data = fs::read(&spike).unwrap_or_default();
    fs::write(out.join("ch-spike"), data).unwrap();
    println!("cargo:rustc-env=NLPV_KERNEL_RELEASE={release}");
}
