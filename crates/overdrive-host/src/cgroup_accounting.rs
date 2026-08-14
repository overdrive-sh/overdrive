//! Host [`CgroupAccounting`] binding — `tokio::fs::read` over real
//! cgroupfs `memory.events`.
//!
//! Production binding of the [`CgroupAccounting`] port trait (ADR-0082
//! §D8). See `overdrive_core::traits::cgroup_accounting::CgroupAccounting`
//! for the full port-trait contract (preconditions, postconditions, edge
//! cases) — this adapter implements that contract; it does not restate
//! it.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use overdrive_core::traits::cgroup_accounting::{
    CgroupAccounting, CgroupAccountingError, CgroupAccountingProbeError, parse_oom_kill_line,
};

/// Default probe target — the control-plane's own delegated root scope's
/// `memory.events`, already created by the control-plane's cgroup
/// bootstrap (`create_and_enrol_control_plane_slice`) before this probe
/// runs. Overridable via [`RealCgroupAccounting::with_probe_path`].
const DEFAULT_PROBE_PATH: &str = "/sys/fs/cgroup/overdrive.slice/memory.events";

/// Production [`CgroupAccounting`] binding backed by `tokio::fs::read`.
///
/// The sim counterpart is
/// `overdrive_sim::adapters::cgroup_accounting::SimCgroupAccounting`.
/// Swap at the wiring boundary; no call site should need both.
///
/// # Concurrency
///
/// Carries no mutable state beyond the configured probe path. Cloning is
/// cheap (single `PathBuf`); `Send + Sync + 'static` per the
/// [`CgroupAccounting`] supertrait requirement.
#[derive(Debug, Clone)]
pub struct RealCgroupAccounting {
    probe_path: PathBuf,
}

impl Default for RealCgroupAccounting {
    fn default() -> Self {
        Self::new()
    }
}

impl RealCgroupAccounting {
    /// Construct a `RealCgroupAccounting` with the default probe path
    /// (`/sys/fs/cgroup/overdrive.slice/memory.events`).
    #[must_use]
    pub fn new() -> Self {
        Self { probe_path: PathBuf::from(DEFAULT_PROBE_PATH) }
    }

    /// **TEST-ONLY scoping.** Override the probe path used by
    /// [`probe`](Self::probe). Consumes `self` and returns `Self`
    /// (builder shape) so the override can chain off [`new`](Self::new).
    /// An internal adapter knob on a single `PathBuf` field — NOT a
    /// port-trait injection builder (see `RealCgroupFs::with_probe_root`'s
    /// docs for the same distinction).
    #[must_use]
    pub fn with_probe_path(mut self, path: PathBuf) -> Self {
        self.probe_path = path;
        self
    }
}

/// Read `path`, decode as UTF-8 (lossily — kernel `memory.events` is
/// always text; the strict Earned-Trust encoding check lives on the
/// [`probe`](CgroupAccounting::probe) path, not this shared read
/// helper), and parse the `oom_kill` key. Shared by both
/// [`CgroupAccounting::oom_kill_count`] and
/// [`CgroupAccounting::probe`] so the parse rule is applied identically.
async fn read_and_parse(path: &Path) -> Result<u64, CgroupAccountingError> {
    let bytes = tokio::fs::read(path).await.map_err(CgroupAccountingError::io)?;
    let content = String::from_utf8_lossy(&bytes);
    parse_oom_kill_line(&content)
        .ok_or_else(|| CgroupAccountingError::malformed(content.into_owned()))
}

#[async_trait]
impl CgroupAccounting for RealCgroupAccounting {
    async fn oom_kill_count(
        &self,
        memory_events_path: &Path,
    ) -> Result<u64, CgroupAccountingError> {
        read_and_parse(memory_events_path).await
    }

    async fn probe(&self) -> Result<(), CgroupAccountingProbeError> {
        let bytes = tokio::fs::read(&self.probe_path)
            .await
            .map_err(CgroupAccountingProbeError::substrate)?;
        let Ok(content) = std::str::from_utf8(&bytes) else {
            return Err(CgroupAccountingProbeError::substrate_corrupt(bytes));
        };
        match parse_oom_kill_line(content) {
            Some(_) => Ok(()),
            None => Err(CgroupAccountingProbeError::missing_oom_kill_key(content.to_owned())),
        }
    }

    fn kind(&self) -> &'static str {
        "overdrive_host::RealCgroupAccounting"
    }
}
