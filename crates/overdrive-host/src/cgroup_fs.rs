//! Host [`CgroupFs`] binding — `tokio::fs::*` over real cgroupfs.
//!
//! Production binding of the [`CgroupFs`] port trait per ADR-0054
//! § Real adapter. Wraps `tokio::fs::{create_dir_all, write,
//! remove_dir}` for the three I/O methods; implements `probe` per
//! ADR-0054 § Production probe (amended 2026-05-24 — round-trip on
//! the kernel-managed `cgroup.subtree_control` pseudo-file:
//! `create_dir → write empty → read → remove_dir`).
//!
//! See `overdrive_core::traits::cgroup_fs::CgroupFs` for the full
//! port-trait contract (preconditions, postconditions, edge cases,
//! observable invariants).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use overdrive_core::traits::{CgroupFs, ProbeError};

/// Production [`CgroupFs`] binding backed by `tokio::fs::*`.
///
/// The sim counterpart is `overdrive_sim::adapters::SimCgroupFs`,
/// an in-memory `BTreeMap<PathBuf, Vec<u8>>` store with injectable
/// per-method error schedule. Swap at the wiring boundary; no call
/// site should need both.
///
/// # Probe root
///
/// The probe root defaults to `/sys/fs/cgroup` — the canonical
/// production substrate location. Tests that need to scope the
/// probe away from real cgroupfs (e.g. CI runners without sudo)
/// override via [`with_probe_root`](Self::with_probe_root).
///
/// # Concurrency
///
/// `RealCgroupFs` carries no mutable state. Cloning is cheap (single
/// `PathBuf`); the adapter is `Send + Sync + 'static` per the
/// [`CgroupFs`] supertrait requirement.
#[derive(Debug, Clone)]
pub struct RealCgroupFs {
    probe_root: PathBuf,
}

impl Default for RealCgroupFs {
    fn default() -> Self {
        Self::new()
    }
}

impl RealCgroupFs {
    /// Construct a `RealCgroupFs` with the default probe root
    /// (`/sys/fs/cgroup`).
    ///
    /// Production composition root calls this directly. Tests that
    /// need a non-cgroupfs probe target chain
    /// [`with_probe_root`](Self::with_probe_root).
    #[must_use]
    pub fn new() -> Self {
        Self { probe_root: PathBuf::from("/sys/fs/cgroup") }
    }

    /// **TEST-ONLY scoping**. Override the probe-root directory used
    /// by [`probe`](Self::probe). Consumes `self` and returns `Self`
    /// (builder shape) so the override can chain off [`new`](Self::new).
    ///
    /// # Not a port-trait injection builder
    ///
    /// Per `.claude/rules/development.md` § "Port-trait dependencies",
    /// builder-pattern overrides on injected port traits (`Clock`,
    /// `Transport`, `Entropy`, ...) are an anti-pattern — every port
    /// trait MUST be a mandatory `new()` parameter so the compiler
    /// catches missing injections at every call site. This builder
    /// is the opposite shape: an *internal adapter knob* on a single
    /// `PathBuf` field. It does NOT relax the port-trait-mandatory
    /// rule at `ExecDriver::new` (step 01-05); the
    /// `Arc<dyn CgroupFs>` parameter there stays mandatory.
    ///
    /// # Use cases
    ///
    /// - Tier 3 acceptance test against `tempfile::TempDir` to prove
    ///   the override genuinely scopes (see
    ///   `crates/overdrive-worker/tests/integration/real_cgroup_fs/
    ///   probe_with_custom_root.rs`).
    /// - CI runners without sudo where `/sys/fs/cgroup` is not
    ///   writable.
    #[must_use]
    pub fn with_probe_root(mut self, root: PathBuf) -> Self {
        self.probe_root = root;
        self
    }
}

#[async_trait]
impl CgroupFs for RealCgroupFs {
    async fn create_dir(&self, path: &Path) -> std::io::Result<()> {
        tokio::fs::create_dir_all(path).await
    }

    async fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        tokio::fs::write(path, bytes).await
    }

    async fn remove_dir(&self, path: &Path) -> std::io::Result<()> {
        tokio::fs::remove_dir(path).await
    }

    async fn probe(&self) -> Result<(), ProbeError> {
        // Per ADR-0054 § Production probe (amended 2026-05-24) —
        // round-trip on `cgroup.subtree_control`, a kernel-managed
        // pseudo-file production cgroup-management code already
        // touches via every controller-enablement write. The
        // amendment replaced the original regular-file approach
        // (rejected — cgroupfs forbids regular-file creation inside
        // cgroup directories; see ADR § Alternatives → Alternative F).
        //
        // Earned Trust contract: substrate is healthy iff write+read
        // both succeed and the kernel returns valid UTF-8 bytes on
        // read. Bytes-equality with the empty write payload is NOT
        // asserted — the kernel's canonical response is its own
        // (possibly empty, possibly containing inherited controllers).
        let probe_dir = self.probe_root.join(format!(".overdrive-probe-{}", uuid::Uuid::new_v4()));
        let subtree_control = probe_dir.join("cgroup.subtree_control");

        // (1) create_dir — on real cgroupfs the kernel synthesises
        // `cgroup.subtree_control` (and siblings) inside the new
        // leaf cgroup as a side effect.
        self.create_dir(&probe_dir).await.map_err(|source| ProbeError::Substrate { source })?;

        // (2) write — empty payload is a no-op controller-diff the
        // kernel parses and applies (no controller enabled, no
        // controller disabled). On a non-cgroupfs substrate
        // (e.g. test routing to tempdir), `tokio::fs::write` will
        // CREATE the file (it is not kernel-synthesised), so the
        // probe proceeds to step (3) and only fails at step (4)
        // `remove_dir` with `DirectoryNotEmpty`.
        if let Err(source) = self.write(&subtree_control, b"").await {
            best_effort_remove_dir(&probe_dir).await;
            return Err(ProbeError::Substrate { source });
        }

        // (3) read — the kernel returns the current controller list
        // (may be empty for a fresh leaf cgroup with no enabled
        // controllers). Assertion is "read returned Ok AND bytes are
        // valid UTF-8", NOT byte-equality with the empty payload we
        // wrote. Non-UTF-8 is the substrate-lying signal (kernel
        // bug, corruption, something pretending to be cgroupfs).
        let read_back = match tokio::fs::read(&subtree_control).await {
            Ok(bytes) => bytes,
            Err(source) => {
                best_effort_remove_dir(&probe_dir).await;
                return Err(ProbeError::Substrate { source });
            }
        };
        if std::str::from_utf8(&read_back).is_err() {
            best_effort_remove_dir(&probe_dir).await;
            return Err(ProbeError::SubstrateCorrupt { read: read_back });
        }

        // (4) teardown — no `remove_file` step: the kernel forbids
        // unlinking its own pseudo-files; `remove_dir` on an empty
        // leaf cgroup reaps the synthesised pseudo-files
        // automatically.
        self.remove_dir(&probe_dir).await.map_err(|source| ProbeError::Substrate { source })?;

        Ok(())
    }

    fn kind(&self) -> &'static str {
        "overdrive_host::RealCgroupFs"
    }
}

/// Best-effort teardown of the probe scratch directory after a probe
/// step failed. Per ADR-0054 § Production probe — the primary cause
/// is what matters; partial leftover under a non-cgroupfs probe root
/// is acceptable per the amended contract.
async fn best_effort_remove_dir(probe_dir: &Path) {
    let _ = tokio::fs::remove_dir(probe_dir).await;
}
