//! Sim adapters — one module per injectable port trait.
//!
//! Each sub-module contains the `Sim*` implementation of one
//! `overdrive_core::traits::*` trait:
//!
//! * [`clock`] — `SimClock`, logical-time clock driven by harness ticks.
//! * [`transport`] — `SimTransport`, in-process datagram router with
//!   injectable partition matrix.
//! * [`entropy`] — `SimEntropy`, seeded `StdRng`.
//! * [`dataplane`] — `SimDataplane`, in-memory policy / service /
//!   flow-event storage.
//! * [`driver`] — `SimDriver`, in-memory allocation table with
//!   configurable failure modes.
//! * [`llm`] — `SimLlm`, transcript-replay adapter.
//! * [`observation_store`] — `SimObservationStore` + gossip cluster.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc, dead_code)]

pub mod ca;
// microvm-driver-cloud-hypervisor step 01-09 (ADR-0082 §D8, GH #42) —
// `SimCgroupAccounting`, the in-memory
// `overdrive_core::traits::cgroup_accounting::CgroupAccounting` double.
// The `cgroup_accounting_equivalence` structural guard (`overdrive-host`
// tests) drives both this and the host `RealCgroupAccounting` adapter
// through the same call sequence.
pub mod cgroup_accounting;
pub mod cgroup_fs;
pub mod clock;
// built-in-ca-operator-composition step 02-02 — `SimKek`, the in-memory
// `overdrive_core::ca::kek::Kek` double. The pure in-process counterpart to
// the host `SystemdCredsKeyring`; injected through `ServerConfig.kek` by every
// `run_server` integration/acceptance fixture so `boot_ca`'s KEK-resolve probe
// succeeds hermetically (feature-delta § C1-AMEND, crafter obligation C-3).
pub mod kek;
// workload-identity-manager step 02-02 — `SimIdentityRead`, the in-memory
// `overdrive_core::traits::identity_read::IdentityRead` double over a preloaded
// held set + trust bundle. The sim counterpart to the host `IdentityMgr`
// (`overdrive-control-plane`); the `identity_read_equivalence` structural guard
// drives both adapters through the same calls (ADR-0067 D7/D9).
pub mod dataplane;
pub mod driver;
pub mod entropy;
pub mod identity_read;
pub mod llm;
// transparent-mtls-host-socket step 02-02 — `SimMtlsEnforcement`, the in-memory
// `overdrive_core::traits::mtls_enforcement::MtlsEnforcement` double. Models the
// handshake OUTCOME (Established vs fail-closed) driven by a preloaded
// `SimIdentityRead`; the `mtls_enforcement_equivalence` structural guard drives
// both this and the host adapter through the same sequence (ADR-0069 F3).
pub mod mtls_enforcement;
// mtls-intercept-install-fault-seam step 03-01 (GH #250) — `SimMtlsIntercept`,
// the in-memory `overdrive_worker::mtls_intercept_port::MtlsIntercept` double
// with per-method fault scripting. Materialises an armed `SimInterceptFault`
// into the REAL `InterceptError` shape the production substrate produces,
// short-circuiting before any syscall, so the fail-closed install paths are
// exercisable on demand (ADR-0076 § 4.6).
pub mod mtls_intercept;
// transparent-mtls-enrollment step 01-02 — `SimMtlsResolve`, the in-memory
// `overdrive_core::traits::mtls_resolve::MtlsResolve` double. Classifies each
// `orig_dst` against a scripted `BTreeMap<SocketAddrV4, MtlsResolution>` table;
// the `mtls_resolve_equivalence` structural guard (DELIVER) drives both this and
// the v1 host `ServiceBackendsResolve` adapter through the same sequence
// (ADR-0071; GH #242 anti-corruption boundary).
pub mod mtls_resolve;
pub mod observation_store;
// reconcilers-own-hydration step 02-05 (ADR-0086 D5/D8) — `SimListenerFacts`,
// `SimServiceVipView`, `SimWorkflowLiveSet`, `SimHeldSvidView`: the four
// in-memory doubles for the new core hydration read-ports. They make the
// reconciler hydration boundary DST-injectable for the first time (each wraps a
// preloaded `BTreeMap`/`BTreeSet`; no substrate, degenerate Earned-Trust probe).
pub mod read_ports;
pub mod transport;
// reconciler-memory-redb step 01-03 — `SimViewStore` impl of
// `overdrive_control_plane::view_store::ViewStore` per ADR-0035 §2.
pub mod view_store;
// workflow-primitive step 01-03 — `SimJournalStore` impl of
// `overdrive_control_plane::journal::JournalStore` per ADR-0066. In-memory
// `BTreeMap<(WorkflowId, u32), Vec<u8>>` with injectable fsync-failure.
pub mod journal;
// SCAFFOLD: true — service-health-check-probes feature.
// Sim bindings for `TcpProber` / `HttpProber` / `ExecProber` per
// ADR-0054 §2. Queue-driven outcome injection. Lands GREEN across
// slices 01-03.
pub mod probers;
// microvm-driver-cloud-hypervisor step 01-06 (GH #42) — `SimVmm`, the
// in-memory `overdrive_core::traits::vmm::Vmm` double. The
// `vmm_equivalence` structural guard (`overdrive-host` tests) drives both
// this and the host `CloudHypervisorVmm` adapter through the same call
// sequence (ADR-0082 §D6).
pub mod vmm;
// microvm-driver-cloud-hypervisor step 02-01 (ADR-0083 §D7, GH #42) —
// `SimVmHostState`, the in-memory
// `overdrive_core::traits::vm_host_state::VmHostState` double. The
// `vm_host_state_equivalence` structural guard (`overdrive-host` tests,
// S-VM-91) drives both this and the host `RealVmHostState` adapter
// through the same call sequence.
pub mod vm_host_state;

pub use ca::SimCa;
pub use cgroup_accounting::SimCgroupAccounting;
pub use cgroup_fs::{SimCgroupFs, SimEntry, SimOp};
pub use identity_read::SimIdentityRead;
pub use kek::SimKek;
pub use mtls_enforcement::{ScriptedTrip, SimMtlsEnforcement};
pub use mtls_intercept::{SimInterceptFault, SimMtlsIntercept};
pub use mtls_resolve::SimMtlsResolve;
pub use read_ports::{SimHeldSvidView, SimListenerFacts, SimServiceVipView, SimWorkflowLiveSet};
pub use vm_host_state::SimVmHostState;
pub use vmm::{SimVmm, SimVmmProbeFault};
