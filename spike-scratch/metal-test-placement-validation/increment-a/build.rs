use std::process::Command;
fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let out = Command::new(rustc).arg("-V").output().expect("rustc -V");
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    println!("cargo:warning=RUSTC_VERSION {version}");
    println!("cargo:rerun-if-changed=build.rs");
}
