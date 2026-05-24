//! [`CgroupFs`] — the sole source of cgroupfs side effects for the
//! worker subsystem.
//!
//! Production wires this to `overdrive_host::RealCgroupFs` (wrapping
//! `tokio::fs::*`); tests wire it to `overdrive_sim::SimCgroupFs`
//! (in-memory `BTreeMap` byte store with injectable per-method error
//! schedule). The trait surface is the boundary at which the worker
//! crate's cgroup manager performs every filesystem mutation against
//! `/sys/fs/cgroup/overdrive.slice/...`.
//!
//! See ADR-0054 for the full design rationale.

use std::path::Path;

use async_trait::async_trait;

/// Cgroupfs side-effect port. The driven port through which the worker
/// crate's cgroup manager performs every filesystem mutation on
/// `/sys/fs/cgroup/overdrive.slice/...`.
///
/// **Production binding**: `overdrive_host::RealCgroupFs`, wrapping
/// `tokio::fs::{create_dir_all, write, remove_dir}`. **Test binding**:
/// `overdrive_sim::adapters::cgroup_fs::SimCgroupFs`, an in-memory
/// `BTreeMap<PathBuf, Vec<u8>>` store with an injectable per-method
/// error schedule.
///
/// # Scope
///
/// The port covers the **byte-side effects** of cgroupfs writes — what
/// bytes appeared in what path. It does NOT cover the **kernel-side
/// effects** that real cgroupfs triggers as a consequence of those
/// writes (mass-kill on `cgroup.kill`; controller enablement
/// validation on `cgroup.subtree_control`; EBUSY when modifying a
/// parent whose descendants have live processes). Tests that exercise
/// kernel-side effects MUST run against real cgroupfs (Tier 3 / Lima
/// sudo); see ADR-0054 § D3 "Non-replacement contract".
///
/// # Preconditions, postconditions, edge cases
///
/// Per `.claude/rules/development.md` § "Trait definitions specify
/// behavior, not just signature", every method below pins:
/// - Preconditions on inputs (validated paths, byte payloads).
/// - Postconditions on caller-observable state.
/// - Edge cases (idempotency, NotFound, EBUSY).
/// - Observable invariants (`write` then `read` yields bytes
///   written, modulo kernel semantics on real cgroupfs).
///
/// # Earned Trust
///
/// Every adapter MUST implement [`probe`](Self::probe). The
/// composition root invariant is "wire then probe then use": the
/// binary calls `probe()` at startup; failure surfaces as a
/// structured `health.startup.refused` event and the process refuses
/// to start. Probe specifications for the two shipping adapters are
/// in the adapter rustdoc (see `overdrive_host::RealCgroupFs` and
/// `overdrive_sim::SimCgroupFs`).
#[async_trait]
pub trait CgroupFs: Send + Sync + 'static {
    /// Create the directory at `path`, including any missing parents
    /// (`mkdir -p` semantics).
    ///
    /// # Preconditions
    /// - `path` must be an absolute path (caller's responsibility;
    ///   `CgroupPath::resolve(&root)` always satisfies this).
    ///
    /// # Postconditions on Ok
    /// - `path` exists as a directory; subsequent
    ///   `write(path.join(...))` calls against children succeed
    ///   unless rejected by the substrate.
    /// - Re-invocation against an existing `path` is `Ok(())` (no-op
    ///   on Real; no-op on Sim).
    ///
    /// # Edge cases
    /// - Pre-existing directory at `path`: `Ok(())` (idempotent).
    /// - Pre-existing regular file at `path`: `Err(AlreadyExists)` on
    ///   Real (kernel-side `EEXIST`); on Sim the schedule determines
    ///   the outcome (default `Err(io::ErrorKind::AlreadyExists)`).
    ///
    /// # Observable invariants
    /// - Idempotent on existing directories. Two successive calls
    ///   against the same `path` are indistinguishable to the caller
    ///   (both return `Ok(())`).
    ///
    /// # Errors
    /// Returns the underlying [`std::io::Error`] from the substrate.
    /// Notable [`std::io::ErrorKind`] values callers should expect:
    /// - [`std::io::ErrorKind::PermissionDenied`] — delegation refused
    ///   by the substrate, or the caller lacks the capability to
    ///   create the directory.
    /// - [`std::io::ErrorKind::NotADirectory`] — a non-directory entry
    ///   already exists at one of the path components.
    /// - [`std::io::ErrorKind::AlreadyExists`] — a non-directory file
    ///   already exists at `path` (Real adapter only; Sim adapter's
    ///   behaviour depends on the injection schedule).
    async fn create_dir(&self, path: &Path) -> std::io::Result<()>;

    /// Write `bytes` to the file at `path`. Overwrites any existing
    /// contents; creates the file if absent (matches
    /// `tokio::fs::write`).
    ///
    /// # Preconditions
    /// - `path`'s parent directory must exist.
    ///
    /// # Postconditions on Ok
    /// - The file at `path` exists and its full contents equal
    ///   `bytes`.
    /// - On real cgroupfs, additional **kernel-side effects** may
    ///   follow as a consequence — these are NOT promised by this
    ///   port (see the trait-level § Scope and ADR-0054 § D3).
    ///
    /// # Edge cases
    /// - `bytes.is_empty()`: writes an empty file; not an error.
    /// - Path does not exist and parent does not exist:
    ///   `Err(NotFound)` (on Real; substrate-dependent on Sim).
    ///
    /// # Observable invariants
    /// - After `Ok(())`, a hypothetical read of `path` would yield
    ///   exactly `bytes` (the substrate is honest about the byte
    ///   payload — substrate dishonesty is detected by
    ///   [`probe`](Self::probe)).
    ///
    /// # Errors
    /// Returns the underlying [`std::io::Error`]. Notable
    /// [`std::io::ErrorKind`] values on real cgroupfs:
    /// - [`std::io::ErrorKind::ResourceBusy`] (EBUSY) — from
    ///   `cgroup.subtree_control` writes when descendants contain
    ///   live processes.
    /// - [`std::io::ErrorKind::PermissionDenied`] — delegation
    ///   refusal.
    /// - [`std::io::ErrorKind::InvalidInput`] — rejected control
    ///   values (e.g. malformed `cpu.weight`).
    /// - [`std::io::ErrorKind::NotFound`] — `path`'s parent does not
    ///   exist.
    async fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()>;

    /// Remove the empty directory at `path`. Matches
    /// `tokio::fs::remove_dir`.
    ///
    /// # Preconditions
    /// - `path` must be an absolute path.
    ///
    /// # Postconditions on Ok
    /// - `path` no longer exists.
    /// - Subsequent [`create_dir`](Self::create_dir) against `path`
    ///   succeeds (idempotent re-create).
    ///
    /// # Edge cases
    /// - `path` does not exist: returns
    ///   `Err(io::ErrorKind::NotFound)`. Callers responsible for
    ///   `NotFound` tolerance (the cgroup manager's
    ///   `remove_workload_scope` wrapper swallows `NotFound` as Ok).
    /// - `path` non-empty: `Err(DirectoryNotEmpty)` on Real. On real
    ///   cgroupfs this does NOT happen for workload scopes — the
    ///   kernel-managed pseudo-files inside a scope are reaped
    ///   automatically by `rmdir(2)`.
    ///
    /// # Observable invariants
    /// - After `Ok(())`, `path` is absent from the substrate's
    ///   directory namespace.
    ///
    /// # Errors
    /// Returns the underlying [`std::io::Error`]. Notable
    /// [`std::io::ErrorKind`] values:
    /// - [`std::io::ErrorKind::NotFound`] — `path` did not exist.
    /// - [`std::io::ErrorKind::DirectoryNotEmpty`] — `path` contains
    ///   non-kernel-reaped children (does not fire for workload
    ///   scopes on real cgroupfs).
    /// - [`std::io::ErrorKind::PermissionDenied`] — caller lacks
    ///   permission to remove `path`.
    async fn remove_dir(&self, path: &Path) -> std::io::Result<()>;

    /// Empirically demonstrate that this adapter can honor its
    /// contract against the real substrate. Called once at
    /// composition-root startup per Earned Trust (CLAUDE.md principle
    /// 12); failure causes the process to refuse to start with a
    /// structured `health.startup.refused` event.
    ///
    /// # Preconditions
    /// - The adapter is constructed; no other operations have been
    ///   issued (the probe is the first call at composition root).
    ///
    /// # Postconditions on Ok
    /// - The substrate honored a full round-trip
    ///   (`create_dir → write → read-back → remove_dir`) with the
    ///   expected byte payload.
    /// - Any probe-scoped scratch artifacts have been removed.
    ///
    /// # Production probe (RealCgroupFs)
    ///
    /// At `<cgroup_root>/.overdrive-probe-<uuid>/`:
    /// 1. `create_dir(&probe_dir)` — directory exists.
    /// 2. `write(&probe_dir.join("probe-file"), b"probe\n")` — byte
    ///    round-trip works.
    /// 3. (host adapter additionally reads back the file via
    ///    `tokio::fs::read` and asserts bytes match — Earned Trust
    ///    requires demonstrated round-trip, not just write-and-pray).
    /// 4. `remove_dir(&probe_dir.join("probe-file"))` then
    ///    `remove_dir(&probe_dir)` — teardown succeeds.
    ///
    /// On failure, the probe surfaces the originating
    /// [`std::io::Error`] in [`ProbeError::Substrate`]; the
    /// composition root emits `health.startup.refused` with the
    /// structured cause.
    ///
    /// # Sim probe (SimCgroupFs)
    ///
    /// Structural: invokes the four-step round-trip against the
    /// in-memory store and asserts the `BTreeMap` reflects each
    /// transition. Fault-injection mode (injected `Err` at any step)
    /// causes the probe to fail; this is the mechanism that
    /// validates the binary's "refuse to start" path under DST.
    ///
    /// # Errors
    /// - [`ProbeError::Substrate`] — the substrate itself returned an
    ///   [`std::io::Error`] at one of the four probe steps.
    /// - [`ProbeError::RoundTripMismatch`] — the substrate completed
    ///   the write but the read-back yielded different bytes. This is
    ///   the failure mode Earned Trust specifically defends against
    ///   (Docker overlayfs `fsync` no-op, WSL2 DrvFs caching, tmpfs
    ///   eviction).
    async fn probe(&self) -> Result<(), ProbeError>;

    /// Adapter discriminator for diagnostic logging.
    ///
    /// # Contract
    /// - Returns a `&'static str` (compile-time constant). Adapter
    ///   implementations MUST hard-code the returned literal; no
    ///   runtime formatting.
    /// - The value is **stable across versions** — operators grep on
    ///   this string in startup logs and structured events. Changing
    ///   the literal is a breaking change to the operator-visible
    ///   surface.
    /// - Real adapters return their crate-qualified name (e.g.
    ///   `"overdrive_host::RealCgroupFs"`); sim adapters return a
    ///   stable sim discriminator (e.g.
    ///   `"overdrive_sim::SimCgroupFs"`).
    ///
    /// Structurally defended at steps 01-02 (RealCgroupFs lands its
    /// stable literal) and 01-03 (SimCgroupFs lands its stable
    /// literal); the equivalence proptest at step 01-07 asserts both
    /// values are non-empty and distinct.
    fn kind(&self) -> &'static str;
}

/// Failure surface for [`CgroupFs::probe`].
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// Probe failed at a substrate-level operation. The originating
    /// [`std::io::Error`] carries the cause.
    #[error("CgroupFs probe failed: {source}")]
    Substrate {
        #[source]
        source: std::io::Error,
    },

    /// Probe succeeded structurally but the round-trip assertion
    /// failed (bytes written did not match bytes read back). Indicates
    /// the substrate is lying about the write — the failure mode
    /// Earned Trust specifically defends against (Docker overlayfs
    /// `fsync` no-op, WSL2 DrvFs caching, tmpfs eviction).
    #[error("CgroupFs probe round-trip mismatch: wrote {wrote:?}, read {read:?}")]
    RoundTripMismatch { wrote: Vec<u8>, read: Vec<u8> },
}
