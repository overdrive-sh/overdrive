//! Shared helper for integration tests that load
//! `target/bpf/overdrive_bpf.o`.
//!
//! Provides a runtime freshness check that complements the
//! compile-time check in `crates/overdrive-dataplane/build.rs`. Both
//! exist to plug the workflow trap where `cargo nextest run -p
//! overdrive-bpf --features integration-tests` does NOT trigger
//! `cargo xtask bpf-build`, and the integration tests therefore load
//! whatever `.o` happens to be on disk — including artifacts older
//! than the source they're supposed to test.
//!
//! The compile-time check fires when building anything that depends
//! on `overdrive-dataplane`. The runtime check fires when the
//! `overdrive-bpf` integration test binary itself runs (which does
//! not pull in `overdrive-dataplane`'s build.rs, since
//! `overdrive-bpf` does not depend on it).
//!
//! Per ADR-0038 §3 the BPF compile is on-demand via `cargo xtask
//! bpf-build`; this helper does NOT shell out to that command. It
//! detects staleness and panics with a clear remediation message —
//! same posture as `overdrive-dataplane`'s build.rs.
//!
//! ## `OVERDRIVE_BPF_OBJECT` override
//!
//! When set, the env var bypasses the workspace-relative artifact
//! path AND the freshness check. CI and cargo-mutants both use this
//! to point at an externally-managed artifact whose mtime relative
//! to the workspace source tree is meaningless.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Resolve the BPF artifact path AND assert that the file exists
/// and is no older than any `*.rs` file under
/// `crates/overdrive-bpf/src/`. Panics with a clear remediation
/// when missing or stale.
///
/// Honours `OVERDRIVE_BPF_OBJECT` — when set, the override path is
/// returned and the freshness check is skipped.
pub fn path() -> PathBuf {
    let workspace_root = workspace_root();

    let override_path =
        std::env::var_os("OVERDRIVE_BPF_OBJECT").filter(|v| !v.is_empty()).map(PathBuf::from);

    let artifact =
        override_path.clone().unwrap_or_else(|| workspace_root.join("target/bpf/overdrive_bpf.o"));

    assert!(
        artifact.exists(),
        "BPF artifact missing at {}; run `cargo xtask bpf-build` first",
        artifact.display(),
    );

    if override_path.is_some() {
        // CI / cargo-mutants manage artifact freshness out-of-band;
        // mtime relative to the local source tree is meaningless.
        return artifact;
    }

    let bpf_src = workspace_root.join("crates/overdrive-bpf/src");
    if let Some(src_mtime) = newest_rs_mtime(&bpf_src)
        && let Ok(art_meta) = std::fs::metadata(&artifact)
        && let Ok(art_mtime) = art_meta.modified()
    {
        assert!(
            art_mtime >= src_mtime,
            "BPF artifact at {} is stale (older than `crates/overdrive-bpf/src/`); \
             run `cargo xtask bpf-build` first",
            artifact.display(),
        );
    }

    artifact
}

/// `crates/overdrive-bpf/<this-module>` -> workspace root. Pops
/// twice (crate name + `crates/`).
fn workspace_root() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut p = PathBuf::from(manifest);
    p.pop();
    p.pop();
    p
}

/// Newest mtime across `*.rs` files under `dir` (recursive). Returns
/// `None` when the dir cannot be walked or contains no `.rs` files
/// — in that case the caller skips the check rather than failing
/// for a reason unrelated to artifact freshness.
fn newest_rs_mtime(dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    walk(dir, &mut |mtime| {
        newest = Some(match newest {
            Some(cur) if cur >= mtime => cur,
            _ => mtime,
        });
    })
    .ok()?;
    newest
}

fn walk<F: FnMut(SystemTime)>(dir: &Path, f: &mut F) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let meta = entry.metadata()?;
        if meta.is_dir() {
            walk(&path, f)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs")
            && let Ok(mtime) = meta.modified()
        {
            f(mtime);
        }
    }
    Ok(())
}
