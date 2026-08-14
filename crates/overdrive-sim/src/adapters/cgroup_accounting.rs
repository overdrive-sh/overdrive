//! `SimCgroupAccounting` — test binding of the [`CgroupAccounting`] port
//! trait (ADR-0082 §D8).
//!
//! In-memory `BTreeMap<PathBuf, u64>` counter store, mirroring
//! [`super::cgroup_fs::SimCgroupFs`]'s shape. An injectable per-path
//! error schedule for [`CgroupAccounting::oom_kill_count`] and a
//! separate, one-shot fault slot for [`CgroupAccounting::probe`] (the
//! three ADR-0082 §D8 fault classes: `Substrate`, `SubstrateCorrupt`,
//! `MissingOomKillKey`) make "this VM's scope was OOM-killed" — and "the
//! `CgroupAccounting` substrate lies" — DST-controllable scenarios for
//! Tier 1.
//!
//! # Concurrency
//!
//! Every method body acquires `parking_lot::Mutex`, mutates the state,
//! and releases — no `.await` while holding a guard, per
//! `.claude/rules/development.md` § "Concurrency & async". The `async
//! fn` surface exists only to satisfy the trait signature.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use overdrive_core::traits::cgroup_accounting::{
    CgroupAccounting, CgroupAccountingError, CgroupAccountingProbeError, parse_oom_kill_line,
};
use parking_lot::Mutex;

/// Healthy default probe body: a well-formed `memory.events` shape
/// carrying `oom_kill 0` — structurally mirrors the real adapter's
/// successful round-trip (parses, has the key) without requiring a test
/// to pre-seed a probe-specific counter.
const HEALTHY_PROBE_BODY: &str = "low 0\nhigh 0\nmax 0\noom 0\noom_kill 0\noom_group_kill 0\n";

/// One-shot fault the NEXT [`CgroupAccounting::probe`] call surfaces,
/// mirroring the three variants of [`CgroupAccountingProbeError`].
#[derive(Debug, Clone)]
enum ProbeFault {
    Substrate(io::ErrorKind),
    SubstrateCorrupt(Vec<u8>),
    MissingOomKillKey(String),
}

/// One-shot fault the NEXT [`CgroupAccounting::oom_kill_count`] call
/// against a specific path surfaces, mirroring
/// [`CgroupAccountingError`]'s two variants.
#[derive(Debug, Clone)]
enum ReadFault {
    Io(io::ErrorKind),
    Malformed(String),
}

/// Sim binding of the [`CgroupAccounting`] port trait.
///
/// # Construction
///
/// ```
/// use overdrive_sim::adapters::cgroup_accounting::SimCgroupAccounting;
/// let sim = SimCgroupAccounting::new();
/// ```
///
/// # Clone semantics
///
/// Cloning shares the underlying `Arc<Mutex<...>>` state. Mirrors
/// `SimCgroupFs` / `SimClock` so callers can hand one clone to the
/// harness and another to the system under test and have both observe
/// the same mutations.
#[derive(Clone, Debug, Default)]
pub struct SimCgroupAccounting {
    counts: Arc<Mutex<BTreeMap<PathBuf, u64>>>,
    read_faults: Arc<Mutex<BTreeMap<PathBuf, ReadFault>>>,
    probe_fault: Arc<Mutex<Option<ProbeFault>>>,
    probe_calls: Arc<Mutex<u32>>,
    read_calls: Arc<Mutex<BTreeMap<PathBuf, u32>>>,
}

impl SimCgroupAccounting {
    /// Construct an empty `SimCgroupAccounting` — no counts set, no
    /// faults injected.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the `oom_kill` counter [`CgroupAccounting::oom_kill_count`]
    /// returns for `path`. `0` (the default for any unset path) is
    /// itself a real, positive fact — the same value production observes
    /// for a scope the kernel has never OOM-killed a process in.
    pub fn set_oom_kill_count(&self, path: PathBuf, count: u64) {
        self.counts.lock().insert(path, count);
    }

    /// Inject a ONE-SHOT [`CgroupAccountingError::Io`] for the next
    /// [`CgroupAccounting::oom_kill_count`] call against `path`.
    pub fn inject_read_io_error(&self, path: PathBuf, kind: io::ErrorKind) {
        self.read_faults.lock().insert(path, ReadFault::Io(kind));
    }

    /// Inject a ONE-SHOT [`CgroupAccountingError::Malformed`] for the
    /// next [`CgroupAccounting::oom_kill_count`] call against `path`.
    pub fn inject_read_malformed(&self, path: PathBuf, raw: impl Into<String>) {
        self.read_faults.lock().insert(path, ReadFault::Malformed(raw.into()));
    }

    /// Inject a ONE-SHOT [`CgroupAccountingProbeError::Substrate`] for
    /// the next [`CgroupAccounting::probe`] call.
    pub fn inject_probe_substrate_error(&self, kind: io::ErrorKind) {
        *self.probe_fault.lock() = Some(ProbeFault::Substrate(kind));
    }

    /// Inject a ONE-SHOT [`CgroupAccountingProbeError::SubstrateCorrupt`]
    /// for the next [`CgroupAccounting::probe`] call.
    pub fn inject_probe_substrate_corrupt(&self, read: Vec<u8>) {
        *self.probe_fault.lock() = Some(ProbeFault::SubstrateCorrupt(read));
    }

    /// Inject a ONE-SHOT [`CgroupAccountingProbeError::MissingOomKillKey`]
    /// for the next [`CgroupAccounting::probe`] call.
    pub fn inject_probe_missing_oom_kill_key(&self, raw: impl Into<String>) {
        *self.probe_fault.lock() = Some(ProbeFault::MissingOomKillKey(raw.into()));
    }

    /// **TEST-HOOK-ONLY**. Total number of [`CgroupAccounting::probe`]
    /// calls observed so far.
    #[must_use]
    pub fn probe_call_count(&self) -> u32 {
        *self.probe_calls.lock()
    }

    /// **TEST-HOOK-ONLY**. Number of [`CgroupAccounting::oom_kill_count`]
    /// calls observed so far against `path` — the read-once-semantics
    /// witness S-VM-93 asserts on (ADR-0082 §D8: "no adapter caches or
    /// re-reads the value across calls").
    #[must_use]
    pub fn read_call_count(&self, path: &Path) -> u32 {
        self.read_calls.lock().get(path).copied().unwrap_or(0)
    }
}

#[async_trait]
impl CgroupAccounting for SimCgroupAccounting {
    async fn oom_kill_count(
        &self,
        memory_events_path: &Path,
    ) -> Result<u64, CgroupAccountingError> {
        {
            let mut calls = self.read_calls.lock();
            *calls.entry(memory_events_path.to_path_buf()).or_insert(0) += 1;
        }
        let pending_fault = self.read_faults.lock().remove(memory_events_path);
        if let Some(fault) = pending_fault {
            return Err(match fault {
                ReadFault::Io(kind) => CgroupAccountingError::io(io::Error::from(kind)),
                ReadFault::Malformed(raw) => CgroupAccountingError::malformed(raw),
            });
        }
        Ok(self.counts.lock().get(memory_events_path).copied().unwrap_or(0))
    }

    async fn probe(&self) -> Result<(), CgroupAccountingProbeError> {
        *self.probe_calls.lock() += 1;
        let pending_fault = self.probe_fault.lock().take();
        if let Some(fault) = pending_fault {
            return Err(match fault {
                ProbeFault::Substrate(kind) => {
                    CgroupAccountingProbeError::substrate(io::Error::from(kind))
                }
                ProbeFault::SubstrateCorrupt(read) => {
                    CgroupAccountingProbeError::substrate_corrupt(read)
                }
                ProbeFault::MissingOomKillKey(raw) => {
                    CgroupAccountingProbeError::missing_oom_kill_key(raw)
                }
            });
        }
        // Healthy default: a well-formed `memory.events` body with
        // `oom_kill 0` — structurally mirrors the real adapter's
        // successful round-trip (parses, has the key) without requiring
        // a test to pre-seed a probe-specific counter.
        match parse_oom_kill_line(HEALTHY_PROBE_BODY) {
            Some(_) => Ok(()),
            None => unreachable!("HEALTHY_PROBE_BODY is a constant that always carries oom_kill"),
        }
    }

    fn kind(&self) -> &'static str {
        "overdrive_sim::SimCgroupAccounting"
    }
}
