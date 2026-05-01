//! [`Driver`] — a workload backend (exec, microVM, VM, unikernel, WASM).
//!
//! Each driver is a thin trait object owned by the node agent. Production
//! wires concrete drivers (`CloudHypervisorDriver`, `ExecDriver`,
//! `WasmDriver`); simulation wires `SimDriver` with configurable failure
//! modes for scheduler and reconciler tests.
//!
//! See `docs/whitepaper.md` §6 for the driver catalogue.

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

use crate::{AllocationId, SpiffeId};

/// Driver class — the `driver` field in a job spec maps 1:1 to a variant.
///
/// Stable: new drivers are appended; existing variants never change their
/// wire form. [`Display`] and [`FromStr`] emit `exec`, `microvm`, `vm`,
/// `unikernel`, `wasm` — matching `docs/whitepaper.md` §6 exactly. The
/// `exec` vocabulary aligns with Nomad's `exec` task driver and Talos's
/// terminology (see ADR-0029 amendment 2026-04-28).
///
/// Carries `utoipa::ToSchema` so the wire-typed `TransitionSource::Driver`
/// variant in `overdrive-control-plane::api` can register the schema
/// transitively (DWD-03). `utoipa` is a declarative-derive crate with no
/// runtime I/O — the dst-lint banned-API list does not enumerate it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum DriverType {
    /// Native binary under cgroups v2 (`tokio::process`).
    Exec,
    /// Fast-boot Cloud Hypervisor microVM.
    MicroVm,
    /// Full Cloud Hypervisor VM (hotplug, virtiofs, any OS).
    Vm,
    /// Cloud Hypervisor + Unikraft unikernel.
    Unikernel,
    /// Wasmtime — serverless WASM functions.
    Wasm,
}

impl DriverType {
    /// Canonical string — matches the job-spec `driver` field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exec => "exec",
            Self::MicroVm => "microvm",
            Self::Vm => "vm",
            Self::Unikernel => "unikernel",
            Self::Wasm => "wasm",
        }
    }
}

impl Display for DriverType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DriverType {
    type Err = UnknownDriverType;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw {
            "exec" => Ok(Self::Exec),
            "microvm" => Ok(Self::MicroVm),
            "vm" => Ok(Self::Vm),
            "unikernel" => Ok(Self::Unikernel),
            "wasm" => Ok(Self::Wasm),
            other => Err(UnknownDriverType(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown driver type: {0:?}")]
pub struct UnknownDriverType(pub String);

#[derive(Debug, Error)]
pub enum DriverError {
    #[error("driver {driver} rejected start: {reason}")]
    StartRejected { driver: DriverType, reason: String },
    #[error("allocation {alloc} not found")]
    NotFound { alloc: AllocationId },
    #[error("driver I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Resource envelope for an allocation — cgroup limits for processes,
/// virtio-mem / hotplug target for VMs.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct Resources {
    pub cpu_milli: u32,
    pub memory_bytes: u64,
}

/// What the scheduler handed to the node agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocationSpec {
    pub alloc: AllocationId,
    pub identity: SpiffeId,
    /// Host filesystem path to the binary the driver execs (e.g. `/bin/sleep`).
    /// Container drivers (Phase 2+ MicroVm/Wasm) carry their own
    /// `ContentHash`-typed image field on per-driver-type spec types.
    pub command: String,
    /// Argv passed verbatim to the binary; the driver invokes
    /// `Command::new(&self.command).args(&self.args)`.
    pub args: Vec<String>,
    pub resources: Resources,
}

/// Opaque handle returned by the driver at start. The node agent does not
/// inspect its contents — it is the driver's private tracking state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocationHandle {
    pub alloc: AllocationId,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocationState {
    Pending,
    Running,
    Draining,
    Terminated,
    Failed { reason: String },
}

/// Classified exit observed by the driver-internal watcher when the
/// underlying workload process exits naturally (the watcher's
/// `child.wait()` resolves) or under a stop signal.
///
/// `CleanExit` covers `wait()` returning `ExitStatus::success()` (exit
/// code 0). `Crashed` carries either an `exit_code` (non-zero `wait()`
/// result) or a `signal` (the kernel killed the process); never both
/// populated, never both `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitKind {
    /// Process exited with status 0 — successful natural completion.
    CleanExit,
    /// Process exited non-zero, was signalled by the kernel, or both.
    /// Watcher constructs from `ExitStatus::code()` /
    /// `ExitStatus::signal()`.
    Crashed { exit_code: Option<i32>, signal: Option<i32> },
}

/// Exit observation for a driver-owned allocation.
///
/// Emitted by the driver's per-alloc watcher task on `child.wait()`
/// resolution and consumed by the worker-side `exit_observer`
/// subsystem, which classifies the event into an `AllocStatusRow` and
/// writes it to the `ObservationStore`.
///
/// `intentional_stop` is the load-bearing discriminator that
/// distinguishes operator-driven termination (`Driver::stop` was
/// called — the workload's exit, even if it was via SIGTERM/SIGKILL,
/// is `Terminated`) from natural crashes (the watcher saw
/// `child.wait()` resolve without a prior `stop` call —
/// classified as `Failed`). Per RCA §Approved fix item 4.
///
/// # Invariants
///
/// - **LWW dominance.** When the worker subsystem writes an obs row
///   in response to this event, it MUST source the row's
///   `LogicalTimestamp` from the same `Clock` instance the action
///   shim uses for its `Running` writes. A watcher that mints
///   timestamps from a divergent clock loses LWW races against
///   stale shim writes (see RCA §Risk).
/// - **`intentional_stop` ordering.** `Driver::stop` MUST set the
///   shared `intentional_stop` flag to `true` BEFORE delivering
///   SIGTERM. Otherwise the watcher reads the flag while it is
///   still `false`, classifies the SIGTERM-induced `wait()`
///   resolution as `Crashed`, and writes a `Failed` row for an
///   operator-stop. Per RCA §Risk "`stop` vs natural exit race".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitEvent {
    pub alloc: AllocationId,
    pub kind: ExitKind,
    pub intentional_stop: bool,
}

#[async_trait]
pub trait Driver: Send + Sync + 'static {
    /// Which driver this is. Stable — the `driver` field of a job spec
    /// deserialises to the same variant.
    fn r#type(&self) -> DriverType;

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError>;

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError>;

    /// Pre-existing observation surface — retained for tests and
    /// status queries that prefer point lookup over the obs row
    /// stream. The supported observation seam for crash detection is
    /// the watcher-emitted `ExitEvent` consumed by the
    /// `exit_observer` worker subsystem; production callers do not
    /// poll `status()` to detect crashes.
    ///
    /// # Post-stop contract
    ///
    /// After [`Driver::stop`] returns `Ok(())`, a subsequent
    /// `status()` against the same handle returns
    /// `Err(DriverError::NotFound)`. Drivers do not retain
    /// terminal-state memory for stopped allocations — durable
    /// terminal-state truth lives in the `ObservationStore`
    /// (`AllocStatusRow`), per the §18 three-layer state taxonomy
    /// and `.claude/rules/development.md` § State-layer hygiene.
    /// A driver that retained a `Terminated` slot would duplicate
    /// observation's job and accumulate one entry per finally-stopped
    /// allocation across a long-running node session.
    ///
    /// Tests that assert on the post-stop state must therefore expect
    /// `Err(DriverError::NotFound { alloc })`, not
    /// `Ok(AllocationState::Terminated)`.
    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError>;

    async fn resize(
        &self,
        handle: &AllocationHandle,
        resources: Resources,
    ) -> Result<(), DriverError>;

    /// Take the single `Receiver<ExitEvent>` this driver emits to.
    /// Returns `Some` on first call, `None` on every subsequent call.
    ///
    /// The `exit_observer` subsystem (in
    /// `overdrive-control-plane::worker::exit_observer`) consumes this
    /// receiver at startup. Drivers that emit exit events
    /// (`ExecDriver`, `SimDriver`) override this to return their
    /// internal receiver exactly once.
    ///
    /// Default: `None` — drivers that have no watcher (e.g.
    /// future implementations that do not buffer events) continue to
    /// compile with no extra surface to override.
    ///
    /// # Invariants
    ///
    /// - **LWW dominance.** When the worker subsystem writes an obs
    ///   row in response to an event from this receiver, it MUST
    ///   source the row's `LogicalTimestamp` from the same `Clock`
    ///   instance the action shim uses for its `Running` writes —
    ///   otherwise a watcher write may lose to a stale shim write
    ///   under last-write-wins (RCA §Risk).
    /// - **`intentional_stop` ordering.** `Driver::stop` MUST set the
    ///   shared `intentional_stop` flag to `true` BEFORE delivering
    ///   any termination signal. The watcher reads this when
    ///   classifying the exit so an operator-stop is not
    ///   misclassified as a crash.
    fn take_exit_receiver(&self) -> Option<tokio::sync::mpsc::Receiver<ExitEvent>> {
        None
    }
}
