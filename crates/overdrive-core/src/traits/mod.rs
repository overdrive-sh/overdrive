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
pub mod cgroup_fs;
pub mod clock;
pub mod dataplane;
pub mod driver;
pub mod entropy;
pub mod identity_read;
pub mod intent_store;
pub mod llm;
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

pub use ca::{
    Ca, CaCertDer, CaCertPem, CaError, CaKeyPem, IntermediateHandle, RootCaHandle, SvidMaterial,
    SvidRequest, TrustBundle, TrustBundlePem,
};
pub use cgroup_fs::{CgroupFs, ProbeError};
pub use clock::Clock;
pub use dataplane::Dataplane;
pub use driver::{Driver, DriverType};
pub use entropy::Entropy;
pub use identity_read::IdentityRead;
pub use intent_store::IntentStore;
pub use llm::Llm;
pub use mtls_enforcement::{
    Direction, EnforcedConnection, EnforcedConnectionId, EnforcedConnectionIdParseError,
    InterceptedConnection, MtlsEnforcement, MtlsEnforcementError, MtlsLimits, ProbeSentinel,
    PumpLiveness, Routed,
};
pub use mtls_resolve::{MtlsResolution, MtlsResolve, MtlsResolveError, ResolvedBackend};
pub use observation_store::ObservationStore;
pub use transport::Transport;
