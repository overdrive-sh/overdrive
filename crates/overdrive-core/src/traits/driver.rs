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

use crate::{AllocationId, SpiffeId};

/// Driver class — the `driver` field in a job spec maps 1:1 to a variant.
///
/// Stable: new drivers are appended; existing variants never change their
/// wire form. [`Display`] and [`FromStr`] emit `exec`, `microvm`, `vm`,
/// `unikernel`, `wasm` — matching `docs/whitepaper.md` §6 exactly. The
/// `exec` vocabulary aligns with Nomad's `exec` task driver and Talos's
/// terminology (see ADR-0029 amendment 2026-04-28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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

#[async_trait]
pub trait Driver: Send + Sync + 'static {
    /// Which driver this is. Stable — the `driver` field of a job spec
    /// deserialises to the same variant.
    fn r#type(&self) -> DriverType;

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError>;

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError>;

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError>;

    async fn resize(
        &self,
        handle: &AllocationHandle,
        resources: Resources,
    ) -> Result<(), DriverError>;
}
