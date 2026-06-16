//! Tier-3 regression gate for D-MTLS-18 — the per-alloc transparent-mTLS
//! intercept install is FAIL-CLOSED.
//!
//! Before this fix, `MtlsInterceptWorker::start_alloc` returned `()` and
//! `warn!`+`return`ed on every install-step failure (the
//! `ProbeRunner`-fire-and-forget shape copied onto a security control). An
//! exec alloc whose `cgroup_connect4_mtls` attach could not be installed was
//! left running with cleartext egress — the exact inverse of the platform
//! guarantee ("the workload holds NOTHING; the kernel does mTLS; the
//! encryption cannot be bypassed").
//!
//! D-MTLS-18 (amends D-MTLS-17 item 4) pins the disposition: an alloc whose
//! intercept cannot be installed MUST NOT run. `start_alloc` now returns
//! `Result<(), MtlsInterceptInstallError>`; the action-shim drives the alloc
//! to terminal `Failed` on `Err`.
//!
//! This test pins the worker-level half of that contract: a deterministic
//! site-1 (OUTBOUND `attach_alloc`) failure — the alloc's `.scope` cgroup
//! does not exist, so `attach_alloc`'s `File::open` fails `NotFound` →
//! `MtlsDataplaneError::Attach` — surfaces as
//! `Err(MtlsInterceptInstallError::OutboundAttach(_))` rather than a
//! swallowed `warn!`. No running workload, no peer wire, no leg traffic is
//! needed: the attach is the FIRST install step and it fails on a
//! non-existent path, so the failure is reproducible on any kernel that can
//! load the mTLS BPF object.
//!
//! Port-to-port: the assertion enters through the worker's `start_alloc`
//! driving port and asserts on its returned `Result` (the observable
//! outcome of the install contract). Deleting the fail-closed conversion at
//! site 1 (reverting it to `warn! + return Ok-shaped`) MUST flip this RED —
//! the `Err` is produced by production code surfacing the discarded typed
//! error, not by the fixture.
//!
//! Requires `CAP_BPF` / `CAP_NET_ADMIN` + a mounted bpffs for the real
//! `MtlsDataplane::load` (its own `aya::Ebpf`). A runner without them SKIPs
//! at the load step (mirrors `mtls_production_activation.rs`'s
//! cap-less-runner SKIP discipline) — the load refusal is itself a
//! fail-closed boot path, just not the per-alloc install path under test.
//! Run via `cargo xtask lima run -- cargo nextest run -p overdrive-worker
//! --features integration-tests`.

#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    reason = "Test body; the no-CAP_BPF SKIP message goes to stderr; fixture \
              construction expects valid inputs and must panic with an \
              informative message otherwise"
)]

use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::Arc;

use overdrive_core::AllocationId;
use overdrive_core::SpiffeId;
use overdrive_core::aggregate::probe_descriptor::ProbeDescriptor;
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::driver::{AllocationSpec, Resources};
use overdrive_core::traits::mtls_enforcement::MtlsLimits;
use overdrive_dataplane::mtls::MtlsDataplane;
use overdrive_sim::adapters::SimIdentityRead;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_worker::mtls_intercept_worker::{MtlsInterceptInstallError, MtlsInterceptWorker};
use tempfile::TempDir;

/// D-MTLS-18 — an OUTBOUND `cgroup_connect4_mtls` attach failure (site 1)
/// makes `start_alloc` return `Err`, NOT swallow the failure and leave the
/// alloc to run uninstrumented.
///
/// The attach targets the alloc's `.scope` cgroup under a `cgroup_root` that
/// does not contain it (a fresh tempdir), so `attach_alloc`'s
/// `File::open(<scope>)` fails `NotFound` → `MtlsDataplaneError::Attach`.
/// Under the fail-closed contract that surfaces as
/// `MtlsInterceptInstallError::OutboundAttach`.
#[tokio::test]
async fn start_alloc_fails_closed_when_outbound_attach_fails() {
    // Real `MtlsDataplane::load` (its own `aya::Ebpf`). The `overdrive_bpf.o`
    // is `include_bytes!`-baked into the test binary, so the only environment
    // gate is capability; a cap-less runner refuses at load and is skipped.
    let pin_dir = TempDir::new().expect("pin-dir tempdir");
    let dataplane = match MtlsDataplane::load(pin_dir.path()) {
        Ok(dp) => dp,
        Err(source) => {
            eprintln!(
                "SKIP: MtlsDataplane::load refused (no CAP_BPF / bpffs on this runner); \
                 the per-alloc install fail-closed path was not reached. Cause: {source}"
            );
            return;
        }
    };

    // Enforcement port — required to construct the worker, but NOT exercised
    // on the site-1 path (the attach is the first install step and it fails
    // before any leg is acquired or `enforce` runs). An empty held set + no
    // bundle is sufficient.
    let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
    let enforcement = Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));

    // The cgroup root is a fresh tempdir that contains NO alloc `.scope`
    // subtree, so the OUTBOUND attach (site 1) fails deterministically with
    // `NotFound` regardless of kernel/privilege state once load succeeds.
    let cgroup_root = TempDir::new().expect("cgroup-root tempdir");
    let worker = Arc::new(MtlsInterceptWorker::new(
        enforcement,
        dataplane,
        cgroup_root.path().to_path_buf(),
        Arc::new(SimClock::new()),
    ));

    let spec = AllocationSpec {
        alloc: AllocationId::new("alloc-mtls-fail-closed-1").expect("valid AllocationId"),
        identity: SpiffeId::from_str("spiffe://overdrive.local/test/mtls-fail-closed")
            .expect("valid SpiffeId"),
        command: "/bin/true".to_owned(),
        args: vec![],
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::<ProbeDescriptor>::new(),
    };

    let result = worker.start_alloc(&spec);

    match result {
        Err(MtlsInterceptInstallError::OutboundAttach(source)) => {
            // The fail-closed contract fired: the discarded typed
            // `MtlsDataplaneError::Attach` is now SURFACED to the caller, so
            // the action-shim can drive the alloc to `Failed` instead of
            // letting it run with cleartext egress. This is the D-MTLS-18 PASS.
            eprintln!("PASS: start_alloc failed closed on outbound attach: {source}");
        }
        Err(other) => panic!(
            "expected MtlsInterceptInstallError::OutboundAttach for a missing alloc \
             .scope, got a different install-failure variant: {other:?}"
        ),
        Ok(()) => panic!(
            "start_alloc returned Ok despite a missing alloc .scope — the OUTBOUND \
             cgroup_connect4_mtls attach could NOT have succeeded, so this is the \
             FAIL-OPEN regression D-MTLS-18 forbids (the alloc would run with \
             cleartext egress and no transparent mTLS)"
        ),
    }
}
