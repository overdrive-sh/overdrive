use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let Some(manifest_dir) = std::env::var_os("CARGO_MANIFEST_DIR") else {
        panic!("Cargo did not set CARGO_MANIFEST_DIR for the build script");
    };
    let manifest_dir = PathBuf::from(manifest_dir);
    let Some(workspace) = manifest_dir.parent().and_then(|crates| crates.parent()) else {
        panic!("overdrive-init is expected under the workspace crates directory");
    };
    let object = workspace.join("target/bpf/overdrive_guest_bpf.o");
    let candidate = workspace.join("target/bpf/overdrive_guest_tcp_correlation_candidate_bpf.o");
    println!("cargo:rerun-if-changed={}", object.display());
    println!("cargo:rerun-if-changed={}", candidate.display());
    assert!(
        object.is_file(),
        "error: BPF object not found at {}; run `cargo xtask bpf-build` first",
        object.display()
    );
    assert!(
        candidate.is_file(),
        "error: TCP correlation candidate BPF object not found at {}; run `cargo xtask bpf-build` first",
        candidate.display()
    );
    println!("cargo:rustc-env=OVERDRIVE_GUEST_BPF_OBJECT_PATH={}", object.display());
    println!(
        "cargo:rustc-env=OVERDRIVE_GUEST_TCP_CORRELATION_CANDIDATE_BPF_OBJECT_PATH={}",
        candidate.display()
    );
}
