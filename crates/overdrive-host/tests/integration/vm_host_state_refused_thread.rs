//! E23 (user decision 2 of 2026-09-30): a `kill_scope` whose blocking-pool
//! thread is refused by the OS returns the refusal as an `io::Error` and never
//! panics.
//!
//! The process runs under the release profile's `panic = "abort"`, so a panic on
//! this path ends the node instead of returning a failure the caller can act on.
//! `RealVmHostState::kill_scope` writes `cgroup.kill` and removes the scope
//! through `tokio::fs`, which dispatches each call onto the blocking pool; the
//! fixture caps the process's task count (`pids.max`) so the pool's next
//! `clone` is refused with `EAGAIN`.
//!
//! # Owner path, fault stimulus, oracle
//!
//! The production `RealVmHostState::kill_scope` over the real cgroup v2
//! filesystem. The fault enters at the kernel only: the `pids` controller refuses
//! the thread (`overdrive_testing::pids_max`). Oracle: the call returns (an
//! `Ok` or an `io::Error`) instead of unwinding.
//!
//! # Tier and placement
//!
//! Tier 3, a `cgroup-default` guest (real root, cgroup v2, `pids` delegated to
//! the root's children). Example-only at this layer (Mandate 9).
//!
//! PENDING (RED): today the blocking-pool spawn panics ("OS can't spawn worker
//! thread") inside `kill_scope`.

#![allow(
    clippy::doc_markdown,
    reason = "the Outcome/CONTRACT_SHAPE header lines use the exact mandated #295 tokens, \
              which carry no backticks by convention"
)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use overdrive_core::AllocationId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_host::RealVmHostState;
use overdrive_testing::overdrive_test;
use serial_test::serial;

/// The cgroup root the scope resolves under (a scratch root, not the node's).
const ROOT: &str = "/sys/fs/cgroup/e23-test";

/// Removes the scope and its scratch parents, whatever the outcome.
struct ScopeCleanup(PathBuf);

impl Drop for ScopeCleanup {
    fn drop(&mut self) {
        for dir in self.0.ancestors().take_while(|dir| dir.starts_with(ROOT)) {
            let _removed = std::fs::remove_dir(dir);
        }
    }
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// E23 — A refused OS thread is a typed failure, never a panic.
/// CONTRACT_SHAPE: unbounded-preservation.
#[overdrive_test(guest = "cgroup-default")]
#[test]
#[serial(cgroup)]
fn a_kill_scope_under_thread_refusal_returns_instead_of_panicking() {
    let alloc = AllocationId::new("alloc-e23kill-0").expect("valid allocation id");
    let scope = CgroupPath::for_alloc(&alloc).resolve(Path::new(ROOT));
    let _cleanup = ScopeCleanup(scope.clone());
    let host = RealVmHostState::new(
        PathBuf::from(ROOT),
        PathBuf::from(ROOT).join("run"),
        PathBuf::from(ROOT).join("index"),
    );

    // The scope is created before the cap: the empty scope is the whole
    // precondition, `cgroup.kill` and `cgroup.events` are core interface files.
    std::fs::create_dir_all(&scope).expect("create the scope");
    // A current-thread runtime built before the cap owns no blocking thread yet.
    let runtime =
        tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let _refused = overdrive_testing::pids_max::refuse_thread_creation()
        .expect("the guest delegates the pids controller to the cgroup root");

    let outcome = catch_unwind(AssertUnwindSafe(|| {
        runtime.block_on(host.kill_scope(&CgroupPath::for_alloc(&alloc)))
    }));

    // Reaching a return proves the call did not unwind; it either landed or
    // returned a typed `io::Error`.
    let returned = outcome.unwrap_or_else(|_unwound| {
        panic!("kill_scope returns instead of panicking when its blocking thread is refused")
    });
    if let Err(error) = returned {
        assert!(
            error.raw_os_error().is_some() || error.kind() != std::io::ErrorKind::Other,
            "a refused thread is a typed io::Error, got {error:?}",
        );
    }
}
