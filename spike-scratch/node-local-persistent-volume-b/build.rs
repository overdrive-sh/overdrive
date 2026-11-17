//! V-1 probe build script (throwaway). Runs on the Metal box. Produces, in OUT_DIR:
//!  * `nlpv-v1-init`  static-musl PID 1 of the INNER guest (the minimal node);
//!  * `lvm-bundle.bin` the distro `lvm` binary plus its `ldd` closure, at the paths the
//!                    inner guest's root filesystem carries them;
//!  * `lvm-manifest.txt` which binary, its version and sha256, and the closure.
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
        let t = line.trim();
        let path = if let Some((_, rhs)) = t.split_once("=>") { rhs.split_whitespace().next().unwrap_or("") } else { t.split_whitespace().next().unwrap_or("") };
        if path.starts_with('/') && Path::new(path).exists() { v.push(PathBuf::from(path)); }
    }
    v
}

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed=guest/nlpv_v1_init.rs");
    println!("cargo:rerun-if-changed=build.rs");

    run(Command::new("rustc").args(["--edition", "2021", "-C", "opt-level=2", "-C", "target-feature=+crt-static", "--target", "x86_64-unknown-linux-musl", "-o"])
        .arg(out.join("nlpv-v1-init")).arg(manifest.join("guest/nlpv_v1_init.rs")));

    // The LVM binary: a static one if the host has it, else the distro binary plus its closure.
    let (lvm, is_static) = if Path::new("/usr/sbin/lvm.static").exists() { ("/usr/sbin/lvm.static", true) } else { ("/usr/sbin/lvm", false) };
    let mut b = Bundle(Vec::new());
    let mut man = String::new();
    let bytes = fs::read(lvm).unwrap();
    b.add("/usr/sbin/lvm", 0o755, &bytes);
    man.push_str(&format!("binary: {lvm} (static={is_static}) -> installed in the node at /usr/sbin/lvm\n"));
    man.push_str(&format!("sha256: {}", run(Command::new("sha256sum").arg(lvm))));
    man.push_str(&format!("file: {}", run(Command::new("file").arg("-L").arg(lvm))));
    man.push_str(&format!("dpkg: {}\n", Command::new("dpkg").args(["-S", lvm]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default()));
    man.push_str(&format!("dpkg-query lvm2: {}\n", Command::new("dpkg-query").args(["-W", "lvm2", "libdevmapper1.02.1"]).output().map(|o| String::from_utf8_lossy(&o.stdout).replace('\n', " | ")).unwrap_or_default()));
    man.push_str(&format!("lvm version:\n{}\n", Command::new(lvm).arg("version").output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()));
    if !is_static {
        let mut seen = std::collections::BTreeSet::new();
        for lib in ldd_closure(lvm) {
            let p = fs::canonicalize(&lib).unwrap_or(lib.clone());
            let gp = lib.to_string_lossy().into_owned();
            if seen.insert(gp.clone()) {
                let data = fs::read(&p).unwrap();
                b.add(&gp, 0o755, &data);
                man.push_str(&format!("lib: {gp} ({} bytes)\n", data.len()));
            }
        }
    }
    fs::write(out.join("lvm-bundle.bin"), &b.0).unwrap();
    fs::write(out.join("lvm-manifest.txt"), &man).unwrap();
    let release = run(Command::new("uname").arg("-r")).trim().to_owned();
    println!("cargo:rustc-env=NLPV_KERNEL_RELEASE={release}");
}
