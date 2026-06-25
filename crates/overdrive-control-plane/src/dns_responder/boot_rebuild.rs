//! `boot_rebuild` — the empty-on-boot converge-on-boot rebuild that
//! re-populates the [`FrontendAddrAllocator`] from the currently-DECLARED
//! Service intent set (dial-by-name-responder step 01-05; ADR-0072 REV-3,
//! GH #243).
//!
//! # Why a boot rebuild
//!
//! The [`FrontendAddrAllocator`] is the EPHEMERAL
//! [`crate::veth_provisioner::NetSlotAllocator`] model — empty on every fresh
//! process boot, carrying NO cross-restart persistence. On a `serve` restart
//! the in-RAM `<job> → F` map is reconstructed EMPTY, but the declared-Service
//! intent rows SURVIVE in the [`IntentStore`] (a prior boot's
//! `submit_workload` wrote them). The DECLARED-SERVICE INTENT IS THE SSOT the
//! rebuild re-derives `F` from (`.claude/rules/development.md` § "Persist
//! inputs, not derived state": desired is the declared intent, NEVER inferred
//! from a prior allocator dump). Without this pass the allocator would stay
//! empty after a restart and the `name_index` (01-03) pure-reader would
//! WITHHOLD every declared `<job>` until its first new submit — reintroducing
//! the stale-answer hazard the stable-`F` design fights.
//!
//! # Bar-1 converge-on-boot (NOT a `Reconciler` trait impl)
//!
//! This is a Bar-1 idempotent observe → diff → converge-on-boot pass
//! (`.claude/rules/reconcilers.md` § "Bar 1 — converge-on-boot"), the EXACT
//! shape of [`crate::veth_provisioner::adopt_on_restart_recovery`] (the netns
//! restart precedent) and [`crate::listener_facts::ListenerFactStore::
//! rebuild_from_intent`] (the same `workloads/` intent scan): observe the
//! declared-Service set, idempotently `assign` each `<job>`. It runs ONCE at
//! boot — there is no continuous tick — so it is not promoted to a full
//! `Reconciler` trait impl.
//!
//! # Idempotent re-assign
//!
//! Each [`FrontendAddrAllocator::assign`] is idempotent per `<job>` (an
//! already-held `<job>` returns its EXISTING `F` unchanged, consuming no new
//! address — FRONTEND-02 at the allocator layer). The rebuild therefore needs
//! NO idempotency logic of its own: re-running the pass over an already-rebuilt
//! allocator re-assigns every `<job>` to the SAME `F` with no churn (S-DBN-
//! ASSIGN-03 idempotency). The pass is the writer's boot half; the
//! assign-on-declare half lives in the Service arm of `submit_workload`.

use std::path::Path;
use std::sync::Arc;

use overdrive_core::aggregate::WorkloadIntent;
use overdrive_core::id::MeshServiceName;
use overdrive_store_local::LocalIntentStore;
use thiserror::Error;

use super::frontend_addr_allocator::{FrontendAddrAllocator, FrontendAddrExhausted};

/// Typed errors for the converge-on-boot frontend-address rebuild.
///
/// Two distinct failure modes get two distinct variants
/// (`.claude/rules/development.md` § "Distinct failure modes get distinct error
/// variants"; § "Never flatten a typed error to `Internal(String)`"): an
/// unreadable intent store ([`Self::IntentScan`]) and a frontend-address
/// exhaustion mid-rebuild ([`Self::Exhausted`]). The boot caller in `run_server`
/// converts via `#[from]` on [`crate::error::ControlPlaneError`] so the
/// composition root can `matches!` on the cause for structured
/// `health.startup.refused` diagnostics rather than `Display`-grepping.
#[derive(Debug, Error)]
pub enum FrontendRebuildError {
    /// The `workloads/` intent prefix scan failed — the declared-Service SSOT
    /// could not be read, so the allocator cannot be re-derived. The node
    /// refuses to start.
    #[error("declared-workload intent scan failed during frontend rebuild: {0}")]
    IntentScan(#[from] overdrive_core::traits::intent_store::IntentStoreError),

    /// A declared `<job>` could not be assigned a stable frontend address
    /// because [`super::frontend_addr_allocator::WORKLOAD_FRONTEND_BASE`] is
    /// exhausted. Refusing the boot is the fail-closed posture: a declared
    /// Service with no resolvable `F` would be silently undialable.
    #[error("frontend address exhausted rebuilding {job}: {source}")]
    Exhausted {
        /// The declared `<job>` whose assignment was refused.
        job: MeshServiceName,
        /// The underlying allocator exhaustion cause.
        #[source]
        source: FrontendAddrExhausted,
    },
}

/// Re-populate `allocator` from the currently-DECLARED Service intent set —
/// the empty-on-boot converge-on-boot rebuild (Bar-1).
///
/// Scans the `workloads/` intent prefix (MIRRORING
/// [`crate::listener_facts::ListenerFactStore::rebuild_from_intent`]), decodes
/// each canonical `workloads/<id>` record, and for every
/// [`WorkloadIntent::Service`] idempotently
/// [`assign`](FrontendAddrAllocator::assign)s the `<job>`'s stable frontend
/// address. Non-Service intents, the `workloads/<id>/stop` and
/// `workloads/<id>/kind` sub-keys, and undecodable payloads contribute nothing
/// (skip — they declare no resolvable `<job>`).
///
/// The `<job>` key is derived `MeshServiceName::new(format!("{id}.{SUFFIX}"))`
/// — byte-identical to the `name_index` reader's `job_of` derivation (OQ-1), so
/// the rebuilt binding is the SAME key the reader looks up. A Service whose
/// `id` is not a valid v1 single-label mesh name (dotted, out-of-class, over
/// 63 octets) is not mesh-dialable by name and is skipped, exactly as in the
/// reader (`name_index::job_of` returns `None`).
///
/// **Idempotent.** Re-running over an already-rebuilt allocator re-assigns
/// every `<job>` to its EXISTING `F` (FRONTEND-02), so the pass is safe to run
/// on every boot and a double-invocation produces no churn.
///
/// **Driven from `run_server` GATED on `mtls_worker.is_some()`** — the SAME
/// composition gate the netns
/// [`adopt_on_restart_recovery`](crate::veth_provisioner::adopt_on_restart_recovery)
/// uses, and the same gate the 02-01 responder + its `name_index` reader are
/// themselves built behind (feature-delta DDN-6). On a non-mTLS boot there is no
/// responder and no reader to serve, so the allocator the rebuild would populate
/// has no consumer — gating to match keeps the rebuild and its only reader
/// behind one gate (roadmap 01-05 pin). This function itself is gate-agnostic;
/// the gating lives at the `run_server` call site.
///
/// # Errors
///
/// Returns [`FrontendRebuildError::IntentScan`] when the `workloads/` prefix
/// scan fails (the SSOT is unreadable — refuse to boot), or
/// [`FrontendRebuildError::Exhausted`] when a declared `<job>` cannot be
/// assigned because the frontend block is full.
pub async fn rebuild_frontend_addrs_from_intent(
    store: &Arc<LocalIntentStore>,
    intent_redb_path: &Path,
    allocator: &FrontendAddrAllocator,
) -> Result<(), FrontendRebuildError> {
    use overdrive_core::traits::intent_store::IntentStore;

    let rows = store.scan_prefix(b"workloads/").await?;
    for (key_bytes, value_bytes) in rows {
        // Only the canonical `workloads/<id>` records carry a Service payload;
        // skip the `workloads/<id>/stop` and `workloads/<id>/kind` sub-keys
        // (an empty suffix or one containing '/' is a sub-key, not the
        // canonical record). Mirrors `rebuild_from_intent`.
        let Ok(key_str) = std::str::from_utf8(&key_bytes) else { continue };
        let suffix = &key_str["workloads/".len()..];
        if suffix.is_empty() || suffix.contains('/') {
            continue;
        }

        // A non-intent payload under the prefix (or a decode failure) declares
        // no `<job>` — skip it.
        let Ok(intent) =
            WorkloadIntent::from_store_bytes(value_bytes.as_ref(), intent_redb_path, Some(key_str))
        else {
            continue;
        };
        // Frontends are a Service-name concern (mirrors the VIP allocate's
        // Service-only guard): a Job / Schedule intent declares no resolvable
        // `<job>` and assigns no frontend addr.
        let WorkloadIntent::Service(service_v1) = &intent else { continue };

        // OQ-1: derive the `<job>` key byte-identically to the `name_index`
        // reader (`name_index::job_of`) — `MeshServiceName::new("<id>.<SUFFIX>")`.
        // A Service whose id is not a valid v1 single-label mesh name is not
        // mesh-dialable by name in v1 and is skipped (the reader skips it too).
        let Ok(job) = MeshServiceName::new(&format!(
            "{}.{}",
            service_v1.id.as_str(),
            MeshServiceName::SUFFIX
        )) else {
            continue;
        };

        // Idempotent re-assign — an already-held `<job>` returns its existing F
        // (FRONTEND-02), so a re-run produces no churn. Fail the boot closed on
        // exhaustion rather than leave a declared Service silently undialable.
        allocator
            .assign(&job)
            .map_err(|source| FrontendRebuildError::Exhausted { job: job.clone(), source })?;
    }
    Ok(())
}
