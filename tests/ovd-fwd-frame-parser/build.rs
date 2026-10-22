//! Build the exact shipped C frame parser into a userspace test archive.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let Ok(target_os) = std::env::var("CARGO_CFG_TARGET_OS") else {
        panic!("Cargo did not set CARGO_CFG_TARGET_OS for the build script");
    };
    if target_os != "linux" {
        // The source intentionally includes Linux UAPI types. The project
        // executes this evidence under `cargo xtask lima run --`.
        return;
    }

    let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") else {
        panic!("Cargo did not set CARGO_MANIFEST_DIR for the build script");
    };
    let manifest_dir = PathBuf::from(manifest_dir);
    let Some(workspace_root) = manifest_dir.parent().and_then(|path| path.parent()) else {
        panic!(
            "expected tests/ovd-fwd-frame-parser below the workspace root: {}",
            manifest_dir.display()
        );
    };
    let module_dir = workspace_root.join("kmod/ovd_fwd");
    let source = module_dir.join("ovd_fwd_frame.c");
    let header = module_dir.join("ovd_fwd_frame.h");
    let uapi = module_dir.join("include/uapi/linux/ovd_fwd.h");

    for path in [&source, &header, &uapi] {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let mut build = cc::Build::new();
    build
        .file(&source)
        .include(module_dir.join("include/uapi"))
        .flag("-std=c11")
        .warnings(true)
        .warnings_into_errors(true);

    if std::env::var_os("CARGO_FEATURE_C_SANITIZERS").is_some() {
        // cargo-fuzz supplies the final executable's userspace ASan runtime.
        // Undefined behavior traps, so no separate UBSan runtime is needed.
        build
            .compiler("clang")
            .flag("-fsanitize=address,undefined")
            .flag("-fsanitize-trap=undefined")
            .flag("-fno-sanitize-recover=all")
            .flag("-fno-omit-frame-pointer");
    }

    // Pulling this object through the FFI reference below also makes any
    // accidental kernel API reference an unresolved userspace link error.
    build.compile("ovd_fwd_frame_parser");
}
