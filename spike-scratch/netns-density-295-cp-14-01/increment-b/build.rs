use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let runner = manifest
        .join("../../..")
        .join("crates/overdrive-testing/bin/fwd_guest_suite.rs");
    println!("cargo::rerun-if-changed={}", runner.display());

    let source = fs::read_to_string(&runner).expect("read the existing GuestKit runner source");
    let first_item = source
        .find("use std::fs;")
        .expect("the existing runner keeps its first import");
    let mut generated = source[first_item..].to_owned();
    let canonical_workspace_root = r#"fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("overdrive-testing has a workspace-root grandparent"))
        .to_path_buf()
}"#;
    let isolated_workspace_root = r#"fn workspace_root() -> PathBuf {
    PathBuf::from(env!("OVERDRIVE_DIAGNOSTIC_WORKSPACE_ROOT"))
}"#;
    assert_eq!(
        generated.matches(canonical_workspace_root).count(),
        1,
        "the current runner keeps exactly one compile-time workspace-root helper"
    );
    generated = generated.replacen(canonical_workspace_root, isolated_workspace_root, 1);
    let workspace_root = manifest
        .join("../../..")
        .canonicalize()
        .expect("canonical workspace root for the isolated launcher");
    println!(
        "cargo::rustc-env=OVERDRIVE_DIAGNOSTIC_WORKSPACE_ROOT={}",
        workspace_root.display()
    );
    generated.push_str(
        r#"

pub(super) fn run_exact_control_plane_prefix() {
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging)
        .expect("the prepared GuestKit provisions on metal");
    let prefix = &STEP14_01_CONTROL_PLANE[..3];
    let passed = run_nested_vm_test_binary(
        &kit,
        "overdrive-control-plane",
        "integration",
        "step14-01-control-plane-prefix-diagnostic",
        15,
        prefix,
    )
    .expect("the exact existing first three control-plane cells execute in GuestKit");
    assert_eq!(passed, 3, "the exact existing control-plane prefix reports three passes");
}
"#,
    );

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"))
        .join("fwd_guest_suite_diagnostic.rs");
    fs::write(out, generated).expect("write the isolated diagnostic runner source");
}
