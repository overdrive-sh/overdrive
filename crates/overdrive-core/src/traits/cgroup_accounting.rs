//! [`CgroupAccounting`] — the post-mortem `memory.events` `oom_kill`
//! read (ADR-0082 §D8, ADR-0083 §A8, GH #42).
//!
//! A NEW port beside [`crate::traits::cgroup_fs::CgroupFs`] — **not** an
//! extension of it. `CgroupFs` is deliberately write-only (ADR-0083 §A8:
//! "the read side is unexposed by design"); widening it with a read
//! method would make that contract mean one thing at one call site and
//! another thing at the next. `CgroupAccounting` exists for exactly one
//! narrow read: the `oom_kill` counter out of a single, already-known
//! `memory.events` path, consulted once by the VM per-alloc exit watcher
//! immediately after the hypervisor process's own exit resolves and
//! before any teardown (`overdrive-worker::vm_driver::run_exit_watcher`).
//!
//! Production wires `overdrive_host::RealCgroupAccounting`; simulation
//! wires `overdrive_sim::adapters::cgroup_accounting::SimCgroupAccounting`.
//! Composed gated alongside [`crate::traits::vmm::Vmm`] (SD-5's
//! composition gate, ADR-0083 §D2) — `CgroupAccounting` is consulted only
//! by the VM exit-watcher, which exists only when `VmDriver` is composed.
//!
//! See ADR-0082 §D8 for the full design rationale (the D-3 fold-in that
//! closes deferral D-3's *reduced* form: a post-mortem read, not the live
//! `memory.events` subscription D-3 names as its eventual mechanism).

use std::path::Path;

use async_trait::async_trait;

/// The post-mortem `oom_kill` read port (ADR-0082 §D8). One read method
/// plus the Earned-Trust [`probe`](Self::probe) every port trait in this
/// codebase carries (CLAUDE.md principle 13 — "wire → probe → use").
///
/// # Scope
///
/// This port owns exactly one fact: the current value of the `oom_kill`
/// key inside a `memory.events` pseudo-file at a caller-resolved path.
/// It does NOT own path resolution (the caller joins `scope.resolve(&
/// cgroup_root).join("memory.events")` — the same convention
/// [`crate::traits::cgroup_fs::CgroupFs::write`] uses for a
/// fully-resolved file path), does NOT subscribe to live `memory.events`
/// updates (deferred — the unreduced half of D-3), and does NOT read any
/// OTHER key in the file (`memory.max`, `low`, `high`, `max`, `oom`,
/// `oom_group_kill` are all out of scope for [`oom_kill_count`](Self::oom_kill_count);
/// [`probe`](Self::probe) reads the whole file but only asserts on the
/// `oom_kill` key's presence).
#[async_trait]
pub trait CgroupAccounting: Send + Sync + 'static {
    /// Read the `oom_kill` counter out of the `memory.events` pseudo-file
    /// at `memory_events_path`.
    ///
    /// # Preconditions
    /// - `memory_events_path` is a fully-resolved path to a
    ///   `memory.events` pseudo-file (or, on the Sim adapter, a key into
    ///   its in-memory store addressed the same way) — the caller
    ///   resolves and joins the path; this method does no path
    ///   construction of its own.
    ///
    /// # Postconditions on Ok
    /// Returns the current value of the `oom_kill` key, parsed from
    /// `key value\n` lines. `0` is a real, positive fact ("the kernel has
    /// never OOM-killed a process in this scope") — never a default
    /// substituted for an error.
    ///
    /// # Edge cases
    /// - Called against a path with no prior `oom_kill` event: `Ok(0)`.
    /// - Called twice against the same path with no intervening kernel
    ///   event: both calls return the same value (this port has no
    ///   caching layer of its own to go stale — every call re-reads the
    ///   substrate, or the Sim adapter's current in-memory value).
    /// - Multiple `key value` lines with extra whitespace or a trailing
    ///   line with no value: parsed permissively (`split_whitespace`);
    ///   only a genuinely absent `oom_kill` key or unparseable value is
    ///   an error (see below).
    ///
    /// # Errors
    /// - [`CgroupAccountingError::Io`] — the substrate `read` failed
    ///   (`NotFound`, `PermissionDenied`, …). At the ONE call site this
    ///   port is used from (immediately after the VM exit-watcher's
    ///   `wait`/`recv()` resolves, before any teardown), `NotFound` is an
    ///   anomaly, not a benign race — the scope should still exist at
    ///   that point.
    /// - [`CgroupAccountingError::Malformed`] — the content parsed as
    ///   valid UTF-8 but had no `oom_kill` line (cgroup v2 guarantees the
    ///   key when the `memory` controller is enabled; its absence means
    ///   the controller was never enabled for this scope, or the path is
    ///   not `memory.events` at all), OR the content is not valid UTF-8
    ///   at all (kernel-generated `memory.events` is always text; this
    ///   port carries no separate non-UTF-8 variant for this read path —
    ///   contrast [`probe`](Self::probe), which distinguishes
    ///   `SubstrateCorrupt` for the stricter boot-time Earned-Trust
    ///   check).
    async fn oom_kill_count(&self, memory_events_path: &Path)
    -> Result<u64, CgroupAccountingError>;

    /// Empirically demonstrate that this adapter can honor its contract
    /// against the real substrate. Called once at composition-root
    /// startup per Earned Trust (CLAUDE.md principle 13), gated alongside
    /// [`crate::traits::vmm::Vmm`]'s own probe (ADR-0082 §D8's
    /// "Composition" section) — failure causes the VM driver to not be
    /// composed (capability absence) or, when an adapter was explicitly
    /// substituted at the port boundary declaring presence, refuses the
    /// boot with a structured `health.startup.refused` event, mirroring
    /// every other Earned-Trust probe in this codebase.
    ///
    /// # Preconditions
    /// - The adapter is constructed; no other operation has been issued
    ///   (the probe is the first call at composition-root startup).
    ///
    /// # Postconditions on Ok
    /// - The substrate's `memory.events` (at this adapter's configured
    ///   probe path — the control-plane's own delegated root scope,
    ///   already created by the control-plane's cgroup bootstrap before
    ///   this probe runs) was read successfully, parsed as valid UTF-8,
    ///   and contains a parseable `oom_kill` line.
    ///
    /// # Edge cases
    /// - Called twice: idempotent — a read-only probe leaves no residue
    ///   to clean up (unlike [`crate::traits::cgroup_fs::CgroupFs::probe`],
    ///   which round-trips a write; this probe never writes).
    ///
    /// # Errors
    /// Returns [`CgroupAccountingProbeError`] naming the specific
    /// substrate lie the probe caught — see that type's per-variant
    /// docs for the three fault classes (ADR-0082 §D8's probe fault
    /// table).
    async fn probe(&self) -> Result<(), CgroupAccountingProbeError>;

    /// Adapter discriminator for diagnostic logging.
    ///
    /// # Contract
    /// - Returns a `&'static str` compile-time constant; no runtime
    ///   formatting.
    /// - Stable across versions — operators grep on this string in
    ///   startup logs and structured events.
    /// - `"overdrive_host::RealCgroupAccounting"` for the production
    ///   adapter, `"overdrive_sim::SimCgroupAccounting"` for the DST
    ///   binding.
    fn kind(&self) -> &'static str;
}

/// Failure surface for [`CgroupAccounting::oom_kill_count`].
#[derive(Debug, thiserror::Error)]
pub enum CgroupAccountingError {
    /// The substrate `read` failed. Wraps the originating
    /// [`std::io::Error`] without reinterpretation.
    #[error("cgroup accounting read failed: {source}")]
    Io {
        #[source]
        source: std::io::Error,
    },
    /// The content parsed as valid UTF-8 (or, for content that is not
    /// valid UTF-8 at all, was lossily converted) but had no parseable
    /// `oom_kill` line. `raw` carries the content this port tried to
    /// parse — the caller's diagnostic context for "the substrate is not
    /// what I expected."
    #[error("memory.events has no parseable 'oom_kill' line: {raw:?}")]
    Malformed { raw: String },
}

/// Failure surface for [`CgroupAccounting::probe`] — ADR-0082 §D8's
/// three fault-injection scenarios, one variant each.
#[derive(Debug, thiserror::Error)]
pub enum CgroupAccountingProbeError {
    /// The probe's `read` of `memory.events` failed at the substrate
    /// level (`ENOENT` — the `memory` controller was never enabled for
    /// this scope, `EACCES` — delegation refused). Wraps the originating
    /// [`std::io::Error`] without reinterpretation.
    #[error("cgroup accounting probe failed: {source}")]
    Substrate {
        #[source]
        source: std::io::Error,
    },
    /// The read succeeded but the content is not valid UTF-8.
    /// `memory.events` is kernel-generated text; non-UTF-8 indicates
    /// substrate corruption, a non-cgroupfs mount, or something
    /// impersonating cgroupfs.
    #[error("cgroup accounting probe substrate corrupt: {} bytes are not valid UTF-8: {read:?}", read.len())]
    SubstrateCorrupt { read: Vec<u8> },
    /// The content is valid UTF-8 but carries no `oom_kill` line.
    /// cgroup v2 guarantees the key when the `memory` controller is
    /// enabled; its absence at the control-plane's own delegated root
    /// scope means the controller was never enabled there, or the probe
    /// path is not `memory.events` at all. `raw` carries the observed
    /// content for diagnosis.
    #[error("memory.events at probe path has no parseable 'oom_kill' line: {raw:?}")]
    MissingOomKillKey { raw: String },
}

impl CgroupAccountingError {
    #[must_use]
    pub const fn io(source: std::io::Error) -> Self {
        Self::Io { source }
    }

    #[must_use]
    pub fn malformed(raw: impl Into<String>) -> Self {
        Self::Malformed { raw: raw.into() }
    }
}

impl CgroupAccountingProbeError {
    #[must_use]
    pub const fn substrate(source: std::io::Error) -> Self {
        Self::Substrate { source }
    }

    #[must_use]
    pub const fn substrate_corrupt(read: Vec<u8>) -> Self {
        Self::SubstrateCorrupt { read }
    }

    #[must_use]
    pub fn missing_oom_kill_key(raw: impl Into<String>) -> Self {
        Self::MissingOomKillKey { raw: raw.into() }
    }
}

/// Parse the `oom_kill` key's value out of `memory.events`-shaped
/// content (`key value\n` lines, whitespace-separated). Shared by both
/// adapters (and reusable by a future probe-path parse) so the parsing
/// rule lives in exactly one place — the port module, not duplicated
/// per adapter.
///
/// `None` when no line's first whitespace-separated token is exactly
/// `"oom_kill"`, or when that line's second token does not parse as
/// `u64`.
#[must_use]
pub fn parse_oom_kill_line(content: &str) -> Option<u64> {
    content.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let key = fields.next()?;
        if key != "oom_kill" {
            return None;
        }
        fields.next()?.parse::<u64>().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::parse_oom_kill_line;

    #[test]
    fn finds_oom_kill_among_sibling_keys() {
        let content = "low 0\nhigh 0\nmax 0\noom 0\noom_kill 3\noom_group_kill 0\n";
        assert_eq!(parse_oom_kill_line(content), Some(3));
    }

    #[test]
    fn zero_is_a_real_value_not_absence() {
        let content = "low 0\nhigh 0\nmax 0\noom 0\noom_kill 0\noom_group_kill 0\n";
        assert_eq!(parse_oom_kill_line(content), Some(0));
    }

    #[test]
    fn missing_key_returns_none() {
        let content = "low 0\nhigh 0\nmax 0\noom 0\noom_group_kill 0\n";
        assert_eq!(parse_oom_kill_line(content), None);
    }

    #[test]
    fn unparseable_value_returns_none() {
        let content = "oom_kill not-a-number\n";
        assert_eq!(parse_oom_kill_line(content), None);
    }

    #[test]
    fn empty_content_returns_none() {
        assert_eq!(parse_oom_kill_line(""), None);
    }
}
