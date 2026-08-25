//! Injectable trait boundaries.
//!
//! Every source of non-determinism in Overdrive — time, network, entropy,
//! kernel, drivers, storage, LLM — crosses one of the traits in this
//! module. Core logic depends on these traits, never on concrete
//! implementations. The wiring crates (`overdrive-node`,
//! `overdrive-control-plane`) pick the real impls; test crates pick the
//! `Sim*` impls.
//!
//! This is the seam Deterministic Simulation Testing (see
//! `docs/whitepaper.md` §21) stands on. A lint gate in CI forbids
//! `std::time::*::now`, `rand::{random, thread_rng}`, `tokio::net::*`, and
//! direct `aya-rs` / kernel calls from anywhere that is not a wiring
//! crate.

pub mod ca;
// microvm-driver-cloud-hypervisor step 01-09 (ADR-0082 §D8, GH #42). The
// post-mortem `memory.events` `oom_kill` read port — a NEW port beside
// `CgroupFs`, not a widening of it (ADR-0083 §A8). Composed gated
// alongside `Vmm`; consulted only by the VM per-alloc exit watcher.
pub mod cgroup_accounting;
pub mod cgroup_fs;
pub mod clock;
pub mod dataplane;
pub mod driver;
pub mod entropy;
// reconcilers-own-hydration (ADR-0086 D5). The four narrow driven read-ports
// the reconciler hydration boundary reads. Contracts live in core; production
// impls live UP (`ListenerFactStore` / `WorkflowEngine` / `IdentityMgr` in
// control-plane, `PersistentServiceVipAllocator` in dataplane); `Sim*` impls
// (step 02-05) make the hydration boundary DST-injectable.
pub mod held_svid_view;
pub mod identity_read;
pub mod intent_store;
pub mod listener_facts;
pub mod llm;
pub mod service_vip_view;
pub mod workflow_live_set;
// transparent-mtls-host-socket (ADR-0069, GH #26). The per-connection
// transparent-mTLS enforcement port + its supporting types (the accepted
// MtlsEnforcement contract). Pure trait + `#[async_trait]` boundary (a
// declarative macro, no runtime — off the `core` I/O surface, exactly as
// `Dataplane`). `HostMtlsEnforcement` extends `overdrive-dataplane`;
// `SimMtlsEnforcement` will extend `overdrive-sim`.
pub mod mtls_enforcement;
// transparent-mtls-enrollment (ADR-0071, GH #26 / #242). The per-connection
// enrollment-resolve driven port (the #242 anti-corruption boundary): resolve a
// captured connection's `orig_dst` into a 3-variant `MtlsResolution`
// (Mesh/NonMesh/MeshUnreachable), fail-closed not silent-cleartext. Pure trait +
// `#[async_trait]` boundary (a declarative macro, no runtime — off the `core`
// I/O surface, exactly as `MtlsEnforcement` / `Dataplane`).
pub mod mtls_resolve;
pub mod observation_store;
// SCAFFOLD: true — service-health-check-probes feature.
// Three port traits (`TcpProber` / `HttpProber` / `ExecProber`) per
// ADR-0054 §3. Lands GREEN across slices 01-03.
pub mod prober;
pub mod transport;
// microvm-driver-cloud-hypervisor (ADR-0082, GH #42). The hypervisor-
// process port: `Vmm::create`/`terminate` plus the `VmConfig`-shaped
// value family in `crate::vm::config`. No implementor lands until step
// 01-06 (`CloudHypervisorVmm` / `SimVmm`); the trait compiles with zero
// implementors, which is expected for this step.
pub mod vmm;
// microvm-driver-cloud-hypervisor step 02-01 (ADR-0083 §D7, GH #42). The
// host-observation-driven port `VmReclamation` hydrates its `actual` half
// from — a NEW port, not a widened `CgroupFs` (ADR-0083 §A8). Composed
// unconditionally, never gated on `Vmm`.
pub mod vm_host_state;

pub use ca::{
    Ca, CaCertDer, CaCertPem, CaError, CaKeyPem, IntermediateHandle, RootCaHandle, SvidMaterial,
    SvidRequest, TrustBundle, TrustBundlePem,
};
pub use cgroup_accounting::{CgroupAccounting, CgroupAccountingError, CgroupAccountingProbeError};
pub use cgroup_fs::{CgroupFs, ProbeError};
pub use clock::Clock;
pub use dataplane::Dataplane;
pub use driver::{Driver, DriverType};
pub use entropy::Entropy;
pub use held_svid_view::HeldSvidView;
pub use identity_read::IdentityRead;
pub use intent_store::IntentStore;
pub use listener_facts::ListenerFacts;
pub use llm::Llm;
pub use service_vip_view::ServiceVipView;
pub use workflow_live_set::WorkflowLiveSet;
pub use mtls_enforcement::{
    Direction, EnforcedConnection, EnforcedConnectionId, EnforcedConnectionIdParseError,
    InterceptedConnection, MtlsEnforcement, MtlsEnforcementError, MtlsLimits, ProbeSentinel,
    PumpLiveness, Routed,
};
pub use mtls_resolve::{MtlsResolution, MtlsResolve, MtlsResolveError, ResolvedBackend};
pub use observation_store::ObservationStore;
pub use transport::Transport;
pub use vm_host_state::{ScopeFacts, VmHostObservation, VmHostState, VmHostStateProbeError};
