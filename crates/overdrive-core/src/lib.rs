//! Overdrive core types.
//!
//! Single source of truth for Overdrive's domain identifiers, cross-cutting
//! error types, and the [`traits`] module that defines every injectable
//! boundary the rest of the platform depends on — `Clock`, `Transport`,
//! `Entropy`, `Dataplane`, `Driver`, `IntentStore`, `ObservationStore`,
//! `Llm`.
//!
//! # Design rules
//!
//! * Every domain identifier is a **newtype** — never a raw `String`, `u64`,
//!   or `[u8; 32]`. See [`id`] for the full catalogue.
//! * Newtypes are `Serialize` / `Deserialize` via canonical `Display` /
//!   `FromStr` round-trip. Construction is always fallible and returns
//!   [`IdParseError`] on invalid input.
//! * Library code returns [`Error`] (or a crate-local `thiserror` enum). No
//!   `anyhow::Error` / `eyre::Report` in library return types — those are
//!   binary-boundary concerns.
//! * The [`traits`] module is the DST seam (see `docs/whitepaper.md` §21).
//!   Wiring crates pick real impls; test crates pick `Sim*` impls. Core
//!   logic depends only on the trait surface.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), warn(clippy::expect_used, clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
// Phase 2.2 RED scaffolds in `dataplane/*` (DropClass, MaglevTableSize)
// and `id::BackendId` carry short docstrings on draft type definitions.
// Per `.claude/rules/testing.md` § "Production-side scaffolds", crates
// with many concurrent scaffolds gate the relevant lints crate-level
// via `expect` (NOT `allow`) so the gate self-removes the moment
// every scaffold goes GREEN. Slice 08-01 closed the
// `BackendSetFingerprint` `todo!()` — `clippy::todo` is therefore
// dropped from this expect block. Remove the rest once the remaining
// scaffolds go GREEN.
#![expect(
    clippy::doc_markdown,
    clippy::too_long_first_doc_paragraph,
    reason = "Phase 2.2 RED scaffolds; lints will self-trip when scaffolds go GREEN"
)]

pub mod aggregate;
// `api::submit` — wire-shape `SubmitSpecInput` enum + per-kind payloads
// per ADR-0051 (Accepted 2026-05-15). The wire-side member of the
// three-layer Rust type universe (parser-side `WorkloadSpec` / wire-side
// `SubmitSpecInput` / persisted `WorkloadIntent`).
pub mod api;
// `codec::envelope::{VersionedEnvelope, EnvelopeError}` — shared
// envelope contract for every rkyv-persisted type that crosses a
// durable-storage boundary. Per-type envelopes (e.g.
// `AllocStatusRowEnvelope`, `JobEnvelope`) live co-located with
// their domain types and implement this trait. See ADR-0048 +
// `.claude/rules/development.md` § "rkyv schema evolution".
pub mod codec;
pub mod eval_broker;
// Phase 2.2 dataplane-internal types — `MaglevTableSize`, `DropClass`,
// `BackendSetFingerprint` + computation helpers. Workload-identifier
// newtypes (`ServiceVip`, `ServiceId`, `BackendId`) live in
// [`id`] alongside the existing identifier catalogue.
// RED scaffolds per `docs/feature/phase-2-xdp-service-map/distill/
// wave-decisions.md` DWD-4. Bodies panic until DELIVER fills them.
pub mod dataplane;
pub mod error;
pub mod id;
// `maglev::{permutation, table}` — pure userspace consistent-hashing
// primitives over `BackendId` + `MaglevTableSize`. Lives here (rather
// than in `overdrive-dataplane` where it originated) because it has
// two consumers — the production BPF map handle in `overdrive-dataplane`
// AND the `MaglevDistributionEven` / `MaglevDeterministic` DST
// invariants in `overdrive-sim` — and both already depend on
// `overdrive-core`. Hosting it here breaks the `overdrive-sim →
// overdrive-dataplane` edge that previously dragged dataplane's
// `build.rs` (and its missing-BPF-object hard-fail) into xtask's
// compile chain. See § "xtask is build / test / dev orchestration"
// in `.claude/rules/development.md` for the layering rule.
pub mod maglev;
pub mod reconcilers;
pub mod traits;
// `UnixInstant` — portable wall-clock instant for persistable
// deadlines. See `docs/research/control-plane/issue-139-followup-portable-deadline-representation-research.md`
// for the design rationale; subsequent steps under issue #141 wire it
// through `TickContext` and `WorkloadLifecycleView`.
pub mod wall_clock;
// `TransitionReason` is the SSOT enum carried on streaming
// `SubmitEvent::LifecycleTransition` and snapshot
// `AllocStatusRow.reason`. Locked under ADR-0032 §3 (Amendment
// 2026-04-30, cause-class refactor): 5 progress markers + 9 Phase 1
// cause-class failure variants + 2 Phase 2 emit-deferred forward-compat
// variants (16 total). See
// `docs/feature/cli-submit-vs-deploy-and-alloc-status/distill/wave-decisions.md`
// DWD-03.
pub mod transition_reason;

/// Trait-conformance harnesses exposed to adapter test suites.
///
/// Gated behind `cfg(any(test, feature = "test-utils"))` so the module
/// never enters production builds — adapter `dev-dependencies` opt in
/// via `overdrive-core = { ..., features = ["test-utils"] }`. See
/// `docs/feature/fix-observation-lww-merge/deliver/rca.md` for the
/// motivation: every adapter implementing a trait whose contract
/// constrains semantics (LWW domination on `ObservationStore::write`)
/// invokes the same harness so divergence between adapters is caught
/// at trait level rather than per-implementation.
#[cfg(any(test, feature = "test-utils"))]
pub mod testing;

pub use error::{Error, Result};
pub use id::{
    AllocationId, CertSerial, ContentHash, CorrelationKey, IdParseError, InvestigationId, NodeId,
    PolicyId, Region, SchematicId, SpiffeId, WorkloadId,
};
pub use traits::{
    Clock, Dataplane, Driver, DriverType, Entropy, IntentStore, Llm, ObservationStore, Transport,
};
pub use wall_clock::UnixInstant;
// Re-exported from `transition_reason` for convenience; the snapshot
// wire surface in `overdrive-control-plane::api` further re-exports
// with a `ToSchema` derive (locked in ADR-0032 §3 Amendment).
// `TerminalCondition` is the ADR-0037 reconciler-emitted classification
// of *why* an allocation reached a terminal state — the publication
// boundary between reconciler-private View state and downstream
// consumers.
pub use transition_reason::{TerminalCondition, TransitionReason};
