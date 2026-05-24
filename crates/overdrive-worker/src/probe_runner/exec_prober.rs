//! `CgroupExecProber` — production binding of `ExecProber` that
//! places the spawned PID inside the workload's cgroup scope via
//! `place_pid_in_scope` (reuse from `cgroup_manager` per ADR-0030 /
//! ADR-0026).
//!
//! Per ADR-0059 + DDD-17: `cgroup.procs` write (NOT `clone3 +
//! CLONE_INTO_CGROUP`; deferred to Phase 2+ pending
//! `nix-rust/nix#2120`). Per DDD-18: `cgroup.kill` (Linux 5.14+)
//! with PID-loop fallback for 5.10–5.13.
//!
//! RED scaffold — `probe()` body lands in slice 03 (Linux integration
//! only). Sim adapter at `crates/overdrive-sim/src/adapters/
//! probers.rs::SimExecProber` does NOT assert cgroup membership —
//! that's a Tier 3 concern.
// SCAFFOLD: true

#![allow(dead_code)]
#![expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice-03")]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::struct_field_names,
    reason = "DISTILL RED scaffold; per `.claude/rules/testing.md` § 'RED scaffolds' lints land when DELIVER replaces todo!() bodies + rewrites docs"
)]

use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::traits::prober::{ExecProber, ProbeFailure, ProbeOutcome};

/// Production `ExecProber` over `tokio::process::Command` +
/// `place_pid_in_scope` (per ADR-0026 / ADR-0030).
pub struct CgroupExecProber;

impl CgroupExecProber {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for CgroupExecProber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExecProber for CgroupExecProber {
    async fn probe(
        &self,
        command: &[String],
        cgroup_scope_path: &str,
        timeout: Duration,
    ) -> Result<ProbeOutcome, ProbeFailure> {
        let _ = (command, cgroup_scope_path, timeout);
        todo!("RED scaffold: CgroupExecProber::probe — spawn + cgroup placement in slice-03")
    }
}
