//! [`Driver`] — a workload backend (exec, microVM, VM, unikernel, WASM).
//!
//! Each driver is a thin trait object owned by the node agent. Production
//! wires concrete drivers (`CloudHypervisorDriver`, `ExecDriver`,
//! `WasmDriver`); simulation wires `SimDriver` with configurable failure
//! modes for scheduler and reconciler tests.
//!
//! See `docs/whitepaper.md` §6 for the driver catalogue.

use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::net::Ipv4Addr;
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

use crate::aggregate::probe_descriptor::ProbeDescriptor;
use crate::id::NetnsName;
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
    /// The driver was configured with a target network namespace
    /// path (opt-in, mirroring the CNI spec's `CNI_NETNS`) but the
    /// `pre_exec` hook could not enter it before `execve` — either
    /// the path could not be opened (`netns_path` does not exist,
    /// caller lacks permission) or `setns(CLONE_NEWNET)` failed.
    /// Distinct from `StartRejected` because the failure mode is a
    /// pre-fork netns-targeting setup error, not a workload-spec
    /// rejection — callers can `matches!` on this variant when
    /// diagnosing test-fixture netns plumbing.
    #[error("driver {driver} could not enter netns {netns_path}: {source}")]
    NetnsEntry { driver: DriverType, netns_path: String, source: std::io::Error },
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
    /// Tagged per-driver invocation payload (ADR-0083 §D3, GH #42).
    /// Replaces the former flat `command: String` / `args: Vec<String>`
    /// fields — every read goes through [`DriverPayload::command`] /
    /// [`DriverPayload::args`] (or a full `match` where the driver-kind
    /// distinction matters, e.g. the `[vm]`-only fields). The routing
    /// key for [`DriverRegistry::get`] is [`DriverPayload::driver_type`].
    ///
    /// Per `.claude/rules/development.md` § "Type-driven design": a
    /// second, `Option`-shaped field alongside the existing flat ones
    /// (`vm: Option<VmPayload>`) was rejected — it would let `vm:
    /// Some(..)` coexist with a meaningful top-level `command`, the
    /// exact sentinel shape that rule forbids.
    pub driver: DriverPayload,
    pub resources: Resources,
    /// Validated health-check probe declarations per ADR-0054 §3.
    ///
    /// Carried from the reconciler-emitted `Action::StartAllocation`
    /// down to the worker-side `ExecDriver` so the driver's
    /// `on_alloc_running` lifecycle hook can hand them to
    /// `ProbeRunner::start_alloc`.
    ///
    /// For Phase 1 Job-kind and Schedule-kind workloads the
    /// reconciler constructs an empty `Vec` — those kinds have no
    /// probes per ADR-0054 §2 ("probes are a Service-kind concern").
    /// Service-kind workloads project the descriptors from
    /// `ServiceSpec.health_check` (per ADR-0057) into the
    /// reconciler-emitted `AllocationSpec`.
    pub probe_descriptors: Vec<ProbeDescriptor>,

    /// Target network namespace NAME this allocation's workload is spawned
    /// INTO (the `ExecDriver` `setns(CLONE_NEWNET)` seam ENTERS it; it must
    /// already exist — the action-shim C3 site provisions it before
    /// `Driver::start`). `Some(plan.netns)` only when the C3 site
    /// provisioned a per-workload netns (the production mTLS boot); `None`
    /// for every non-netns workload (every current test fixture, and any
    /// boot where the mTLS composition gate is off). The driver opens
    /// `/var/run/netns/<name>` (via [`NetnsName::as_str`]) when `Some`; a
    /// `None` spec yields the pre-join host-netns behaviour.
    ///
    /// `Option<NetnsName>` — [`NetnsName`] is an INTERNAL newtype (no
    /// serde, no rkyv, no `FromStr`) minted ONLY by
    /// `derive_workload_netns_plan` (`overdrive-control-plane`) from a
    /// validated `NetSlot`. **This field's type SUPERSEDES D-TME-12 /
    /// JOIN-1's prior `Option<String>` choice** — per ADR-0082 §D2
    /// (Amendment 2026-08-12, GH #42), the value gained a newtype while
    /// keeping JOIN-1's underlying reasoning intact: it has no parse
    /// surface, no operator-typed entry point, and no `FromStr`
    /// round-trip to defend (the JOIN-1 canonical home,
    /// `docs/feature/transparent-mtls-enrollment/design/wave-decisions.md`,
    /// no longer exists on disk — the archived feature's surviving
    /// references are `docs/architecture/transparent-mtls-enrollment/
    /// feature-delta.md` and `docs/evolution/
    /// 2026-06-22-transparent-mtls-enrollment.md`; ADR-0082 §D2 is the
    /// authoritative record of the supersession). Per
    /// `.claude/rules/development.md` § "Persist inputs, not derived
    /// state": `AllocationSpec` derives only `Debug, Clone, PartialEq,
    /// Eq` — NO serde, NO rkyv — and is recomputed each reconcile tick
    /// (never persisted), so this field is a pure in-memory channel with
    /// no schema-evolution discipline attached.
    pub netns: Option<NetnsName>,

    /// Host-side veth interface NAME for this allocation's per-workload veth
    /// pair (`ovd-hv-<4hex-slot>`), the `iifname` the outbound nft-TPROXY rule
    /// matches to redirect the workload's egress to leg-F
    /// (`MtlsInterceptWorker::start_alloc` →
    /// `install_outbound_tproxy(host_veth, leg_f_port)`). `Some(plan.host_veth)`
    /// ONLY when the action-shim C3 site provisioned a per-workload netns/veth
    /// (the production mTLS-composed boot); `None` for every non-netns workload
    /// (every current test fixture, and any boot where the mTLS composition gate
    /// is off) — the pre-join host-netns behaviour, exactly like `netns`.
    ///
    /// `Option<String>`, NOT a newtype — on its OWN rationale (JOIN-6; this
    /// field is not this step's subject): the value is already a validated,
    /// bounded, slot-derived name minted ONLY by `derive_workload_netns_plan`
    /// (a pure projection of the already-newtyped `NetSlot`); it has no parse
    /// surface, no operator-typed entry point, and no `FromStr` round-trip to
    /// defend.
    ///
    /// `netns` above carried the SAME JOIN-1 no-newtype rationale until
    /// ADR-0082 §D2 (Amendment 2026-08-12, GH #42) reversed it FOR `netns`
    /// ONLY — `host_veth` was not part of that reversal and stays
    /// `Option<String>`. JOIN-1 / JOIN-6's canonical archived home,
    /// `docs/feature/transparent-mtls-enrollment/design/wave-decisions.md`,
    /// no longer exists on disk; see `docs/architecture/
    /// transparent-mtls-enrollment/feature-delta.md` and `docs/evolution/
    /// 2026-06-22-transparent-mtls-enrollment.md` for the surviving record.
    pub host_veth: Option<String>,

    /// Canonical per-workload IPv4 address this allocation was provisioned
    /// INTO (the in-netns end of the per-workload veth, `plan.workload_addr`)
    /// for the canonical-workload-address inbound-TPROXY path (D-A1, GH
    /// #241). `Some(plan.workload_addr)` ONLY when the action-shim C3 site
    /// provisioned a per-workload netns/veth (the production mTLS-composed
    /// boot); `None` for every non-netns workload (every current test
    /// fixture, and any boot where the mTLS composition gate is off) — the
    /// pre-join host-netns behaviour, exactly like `netns` / `host_veth`.
    ///
    /// The third member of the slot-derived channel beside `netns` /
    /// `host_veth`, injected at the SAME C3 provision seam off the SAME
    /// `plan`. Per `.claude/rules/development.md` § "Persist inputs, not
    /// derived state": `AllocationSpec` derives only
    /// `Debug, Clone, PartialEq, Eq` — NO serde, NO rkyv — and is recomputed
    /// each reconcile tick (never persisted), so this is a pure in-memory
    /// channel with no schema-evolution discipline attached.
    pub workload_addr: Option<Ipv4Addr>,

    /// Declared Service listener ports projected from the live intent at
    /// hydrate-desired time via
    /// [`crate::reconcilers::project_service_listen_ports`] (D-A1 /
    /// D-BLOCKER1, GH #241). Consumed by the inbound-TPROXY rule install
    /// (step 03-01) — one `install_inbound_tproxy` per declared port, keyed
    /// on `workload_addr`. Empty for Job-kind / Schedule-kind / host-netns
    /// workloads (every current fixture); a Service projects its
    /// `listeners[].port` set in declaration order.
    ///
    /// D-BLOCKER1 — this is the SAME single source the
    /// `BackendDiscoveryBridge` advertise path reads (step 02-01); the two
    /// readers MUST agree, so the value bottoms out in `svc.listeners`.
    /// Threaded through `WorkloadLifecycleState.service_ports` and cloned
    /// into the emitted `AllocationSpec` at the IDENTICAL site/shape as
    /// `probe_descriptors`. Same pure-in-memory derive discipline as the
    /// fields above (no serde / rkyv).
    pub service_ports: Vec<NonZeroU16>,
}

/// Tagged per-driver invocation payload (ADR-0083 §D3). The routing key
/// [`DriverPayload::driver_type`] is what [`DriverRegistry::get`] indexes
/// on; `command()` / `args()` are the two fields every variant carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverPayload {
    Exec(ExecPayload),
    Vm(VmPayload),
}

/// `[exec]` invocation fields — the native-binary-under-cgroups-v2 driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecPayload {
    pub command: String,
    pub args: Vec<String>,
}

/// `[vm]` invocation fields — the Cloud Hypervisor microVM driver
/// (ADR-0082 / ADR-0083, GH #42).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmPayload {
    /// Command run INSIDE the guest.
    pub command: String,
    /// Argv passed verbatim to the in-guest command.
    pub args: Vec<String>,
    /// Operator-supplied kernel artifact path (BYO).
    pub kernel: PathBuf,
    /// Operator-supplied rootfs artifact path (BYO).
    pub rootfs: PathBuf,
    /// Per-VM volume mounts. `vec![]` for every Slice 01-03 allocation —
    /// `[[vm.volume]]` is a Slice 04 concern (design not yet written);
    /// the field is threaded now so `VmPayload`'s shape does not change
    /// again when Slice 04 lands. `AllocationSpec` derives neither serde
    /// nor rkyv, so widening [`VmVolume`] later is not a schema-evolution
    /// event.
    pub volumes: Vec<VmVolume>,
}

/// Placeholder for a future per-VM volume mount (Slice 04 — not yet
/// designed). Carries no fields; [`VmPayload::volumes`] stays `vec![]`
/// until that slice's design lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmVolume {}

impl DriverPayload {
    /// The routing key — [`DriverRegistry::get`] indexes on this.
    #[must_use]
    pub const fn driver_type(&self) -> DriverType {
        match self {
            Self::Exec(_) => DriverType::Exec,
            Self::Vm(_) => DriverType::Vm,
        }
    }

    /// Both variants carry a command; borrow it regardless of kind.
    #[must_use]
    pub fn command(&self) -> &str {
        match self {
            Self::Exec(e) => &e.command,
            Self::Vm(v) => &v.command,
        }
    }

    /// Both variants carry argv; borrow it regardless of kind.
    #[must_use]
    pub fn args(&self) -> &[String] {
        match self {
            Self::Exec(e) => &e.args,
            Self::Vm(v) => &v.args,
        }
    }
}

/// The set of workload drivers this node composed (ADR-0022's
/// pre-committed registry migration; ADR-0083 §D1, GH #42).
///
/// **Absence of a key is a first-class answer**, not an error state: a
/// node with no `cloud-hypervisor` installed simply has no [`DriverType::Vm`]
/// entry, and that absence *is* SD-5's capability gate.
pub struct DriverRegistry {
    by_type: BTreeMap<DriverType, Arc<dyn Driver>>,
}

impl DriverRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self { by_type: BTreeMap::new() }
    }

    /// Keyed on `driver.r#type()` — the driver names itself; no second
    /// source of truth to drift.
    pub fn insert(&mut self, driver: Arc<dyn Driver>) {
        self.by_type.insert(driver.r#type(), driver);
    }

    #[must_use]
    pub fn get(&self, t: DriverType) -> Option<&Arc<dyn Driver>> {
        self.by_type.get(&t)
    }

    #[must_use]
    pub fn supports(&self, t: DriverType) -> bool {
        self.by_type.contains_key(&t)
    }

    /// `BTreeMap`, not `HashMap`: iterated for the admission-rejection
    /// message and `health.startup` logging — ordering is observed
    /// (`.claude/rules/development.md` § "Ordered-collection choice").
    pub fn kinds(&self) -> impl Iterator<Item = DriverType> + '_ {
        self.by_type.keys().copied()
    }
}

impl Default for DriverRegistry {
    fn default() -> Self {
        Self::new()
    }
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
    /// Newline-joined tail of the last [`STDERR_TAIL_LINES`] lines the
    /// workload wrote to stderr before exiting. `None` when the driver
    /// could not capture stderr (no `Stdio::piped()` wiring, the pipe
    /// was closed by the workload, or the watcher detected an I/O
    /// error mid-stream). Per ADR-0033 Amendment 2026-05-10.
    ///
    /// Producers:
    /// - `ExecDriver` (production): consumes `child.stderr` line-by-
    ///   line into a bounded ring buffer of capacity
    ///   `STDERR_TAIL_LINES`; on `child.wait()` resolution emits the
    ///   ring contents joined by `\n` (no trailing newline).
    /// - `SimDriver` (tests): emits `None` by default; tests that
    ///   want to exercise the tail-rendering path inject explicit
    ///   stderr via the sim driver's tail-injection API.
    pub stderr_tail: Option<String>,
}

/// Number of trailing stderr lines `ExecDriver` retains for inclusion
/// on the [`ExitEvent`]. The constant is the project-wide SSOT so the
/// driver-side ring buffer (which fills it) and the renderer (which
/// displays it as "stderr (last N lines):") read from one source.
/// Per ADR-0033 Amendment 2026-05-10 / step 02-05 of
/// `workload-kind-discriminator`.
pub const STDERR_TAIL_LINES: usize = 5;

#[async_trait]
pub trait Driver: Send + Sync + 'static {
    /// Which driver this is. Stable — the `driver` field of a job spec
    /// deserialises to the same variant.
    fn r#type(&self) -> DriverType;

    /// Spawn the workload described by `spec` and return an opaque
    /// `AllocationHandle` the operator uses to address it.
    ///
    /// # Running-confirmed gate (post-condition)
    ///
    /// Drivers that emit `ExitEvent`s via [`Driver::take_exit_receiver`]
    /// (i.e. those whose `start` spawns a per-alloc watcher / scheduled
    /// emitter) MUST stash a `tokio::sync::oneshot::Sender<()>` whose
    /// receiver is awaited by the watcher BEFORE its first
    /// `ExitEvent` send on the channel. The action shim fires the
    /// gate via [`Driver::release_for_exit_emission`] exactly once,
    /// after one of:
    ///
    /// - the corresponding `obs.write(AllocStatus::Running)` has
    ///   committed Ok, or
    /// - the May-2 retry path has exhausted retries and degraded to a
    ///   `LifecycleEvent`-only emission (the gate must STILL fire on
    ///   that liveness rail — otherwise the watcher leaks forever
    ///   waiting on a oneshot that nothing will ever send).
    ///
    /// This is the structural happens-before edge between the action
    /// shim's "Running row committed" and the watcher's first
    /// `ExitEvent` emission. Without it, a sub-millisecond-lifetime
    /// workload's exit event can race the action shim's `Running`
    /// write and be silently dropped by the observer's
    /// `find_prior_row → NoPriorRow` arm. See
    /// `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
    /// (Solution 1') for the full RCA.
    ///
    /// # Idempotency
    ///
    /// `release_for_exit_emission` is idempotent: a call against an
    /// alloc whose gate has already fired (or whose alloc is unknown
    /// to the driver) is a no-op, NOT a panic. This protects against
    /// future call sites firing the gate twice (e.g. a successful
    /// retry AND the May-2 degraded-escalation path both firing).
    /// `tokio::sync::oneshot::Sender::send` consumes the sender, so
    /// double-fire is structurally impossible at the type level —
    /// the idempotency guarantee here is about the lookup-and-take
    /// path in this method, not the channel itself.
    ///
    /// # Observable invariant
    ///
    /// No `ExitEvent` reaches the consumer of
    /// [`Driver::take_exit_receiver`] before the corresponding gate
    /// has fired (or, equivalently, before the gate's
    /// `oneshot::Sender` has been dropped — see "Sender drop"
    /// below).
    ///
    /// # Sender drop (orphan path)
    ///
    /// If the action shim crashes or otherwise panics between
    /// `driver.start` resolving and the gate firing, the stashed
    /// `oneshot::Sender` is eventually dropped (when the driver is
    /// dropped, or when the alloc's slot is evicted). The watcher's
    /// `oneshot::Receiver::await` then resolves to `Err(RecvError)`,
    /// which the watcher treats as "proceed and emit the event". The
    /// observer's `find_prior_row` then handles the present-or-absent
    /// row case as today (May-2 RCA). This matches the today-reality
    /// orphan-process condition the predecessor RCA accepts as out of
    /// scope.
    ///
    /// # Default implementation
    ///
    /// Drivers that have no watcher (no `ExitEvent` emission path)
    /// do not need to override [`Driver::release_for_exit_emission`];
    /// the default no-op is correct for them.
    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError>;

    /// Fire the Running-confirmed gate for `handle.alloc`. See the
    /// post-condition section on [`Driver::start`] for the full
    /// contract: idempotent, exactly-once-per-alloc by construction
    /// of the `oneshot::Sender::send` consume-self semantics, no-op
    /// for unknown allocs.
    ///
    /// The action shim calls this after `obs.write(Running)`
    /// resolves Ok, OR after the May-2 retry-exhaustion-degraded
    /// `LifecycleEvent` path runs. Either firing site is sufficient
    /// for the watcher's gate-await to release.
    ///
    /// Default: no-op (drivers with no watcher).
    fn release_for_exit_emission(&self, _handle: &AllocationHandle) {}

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

    /// Lifecycle hook fired by the action shim when an allocation
    /// transitions to `Running` (i.e. immediately after the action
    /// shim writes the `AllocStatusRow { state: Running, .. }` row
    /// via `obs.write`).
    ///
    /// Production [`crate::traits::driver::Driver`] implementations
    /// that hold a reference to the worker's `ProbeRunner` (today:
    /// `overdrive_worker::ExecDriver`) override this to call
    /// `probe_runner.start_alloc(&spec.alloc, spec.probe_descriptors.clone())`,
    /// handing the validated probe descriptors to the per-alloc
    /// supervisor per ADR-0054 § 3.
    ///
    /// Default no-op for drivers that do not run probes
    /// (`overdrive_sim::SimDriver`, future Phase-2 driver types).
    /// Per ADR-0054 § 2 probes are a worker-level concern; sim /
    /// future drivers that delegate to other observation surfaces
    /// continue to compile.
    fn on_alloc_running(&self, _spec: &AllocationSpec) {}

    /// Lifecycle hook fired by the action shim when an allocation
    /// transitions to a terminal state (Terminated / Failed) — i.e.
    /// immediately after the action shim writes the terminal
    /// `AllocStatusRow` via `obs.write`.
    ///
    /// Production [`crate::traits::driver::Driver`] implementations
    /// that hold a reference to the worker's `ProbeRunner` override
    /// this to call `probe_runner.stop_alloc(alloc_id)`, cooperatively
    /// shutting down every per-probe task spawned under the
    /// allocation's supervisor per ADR-0054 § 2.
    ///
    /// Default no-op — symmetric with [`Self::on_alloc_running`].
    fn on_alloc_terminal(&self, _alloc_id: &AllocationId) {}

    /// Lifecycle hook fired by the action shim when an allocation is
    /// announced `Stable` (ADR-0055 — a NON-terminal condition).
    ///
    /// Startup probing is bounded by the startup window and is
    /// complete at Stable; readiness and liveness are continuous
    /// post-Stable per [`crate::observation::ProbeRole`]'s contract.
    /// Implementations stop ONLY the startup-role tasks and leave the
    /// supervisor alive.
    ///
    /// Per ADR-0080 § D4 this hook exists because routing `Stable`
    /// through [`Self::on_alloc_terminal`] was a category error: it
    /// tore down the whole probe supervisor, making readiness and
    /// liveness structurally unreachable for their entire intended
    /// lifetime.
    ///
    /// Default no-op — symmetric with [`Self::on_alloc_running`] /
    /// [`Self::on_alloc_terminal`].
    fn on_alloc_stable(&self, _alloc_id: &AllocationId) {}

    /// The allocations for which this driver currently holds a live
    /// supervision handle — an authorship claim on that allocation's
    /// ending, in EITHER phase (claimed-and-running, or
    /// ending-being-authored). Reporting only the running phase is
    /// exactly the defect this method's contract refuses (microvm-driver
    /// brief §105a.3 DD-1(b.i)): a reclamation-shaped consumer that reads
    /// this set to decide whether it may safely kill a survivor must see
    /// an allocation whose ending is already in flight as SUPERVISED,
    /// not as abandoned.
    ///
    /// `None` = "this driver does not report supervision" — the caller
    /// MUST read that as "supervision is unavailable", never as
    /// "supervises nothing". An empty `Some(vec![])` and a `None` are
    /// different claims; conflating them authorises a kill-capable
    /// consumer to act on absence of evidence as if it were evidence of
    /// absence.
    ///
    /// Sync deliberately: the live map behind this accessor is a
    /// `parking_lot` guard, and a lock must not be held across
    /// `.await` (`.claude/rules/development.md` § "Concurrency &
    /// async").
    ///
    /// Default: `None`, for drivers that do not report supervision
    /// (`overdrive_worker::ExecDriver` keeps the default — correct
    /// rather than an omission, since a reclamation-shaped consumer
    /// only ever acts on VM allocations).
    fn live_allocations(&self) -> Option<Vec<AllocationId>> {
        None
    }

    /// Retire this driver's claim to author `alloc`'s ending. The
    /// reporter of a claim ([`Self::live_allocations`]) is its releaser
    /// — symmetric by design.
    ///
    /// Called strictly AFTER the ending has been authored (a terminal
    /// row committed) or authorship has been abandoned as impossible;
    /// never at process death, never at a watcher's return, and never
    /// while an exit report is still in flight. The intended callers
    /// are: an exit-observer-shaped consumer, once per exit event, on
    /// every retry-outcome arm — not only a successful write — and any
    /// ending-authoring path that writes a terminal row, strictly after
    /// that write resolves `Ok`.
    ///
    /// IDEMPOTENT: an unknown or already-released `alloc` is a no-op,
    /// never a panic. This is load-bearing — both an exit-observer-shaped
    /// caller and an ending-authoring caller may race to release the
    /// same allocation's claim, and both must be safe to call.
    ///
    /// Sync, for the same reason [`Self::live_allocations`] is: the live
    /// map is a `parking_lot` guard.
    ///
    /// Default: no-op, for drivers that do not report supervision.
    fn release_supervision(&self, _alloc: &AllocationId) {}
}
