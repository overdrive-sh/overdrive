//! `ServiceMapHydrator` reconciler — Phase 2 (Slice 08; ASR-2.2-04).
//!
//! Watches the `service_backends` ObservationStore rows for backend-set
//! drift (the desired side) and the `service_hydration_results` rows
//! for the dataplane's confirmed-state observation (the actual side).
//! Emits one `Action::DataplaneUpdateService` per service whose
//! fingerprint diverges, and reads the hydration-result row on the
//! next tick to advance the state machine.
//!
//! Per ADR-0035/0036:
//!
//! - Sync `reconcile`. No `.await`, no `Instant::now()`, no DB handle.
//!   Wall-clock only via `tick.now_unix`.
//! - Typed `State` (desired+actual per `ServiceId`) and typed `View`
//!   (per-service retry inputs only — `attempts`,
//!   `last_failure_seen_at`, `last_attempted_fingerprint`). NEVER a
//!   `next_attempt_at` field per `.claude/rules/development.md`
//!   § "Persist inputs, not derived state".
//!
//! The struct lives here (rather than in `overdrive-control-plane`)
//! because [`super::AnyReconciler`] holds the concrete type in its
//! `ServiceMapHydrator` variant — same layering as `WorkloadLifecycle`.
//! `overdrive-control-plane::reconcilers::service_map_hydrator`
//! re-exports the public surface.

use std::collections::BTreeMap;
use std::num::NonZeroU16;
use std::time::Duration;

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::dataplane::fingerprint::BackendSetFingerprint;
use overdrive_core::id::{ContentHash, CorrelationKey, ServiceId, ServiceVip};
use overdrive_core::reconcilers::{HydrateError, HydrationContext};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    ServiceHydrationResultRow, ServiceHydrationStatus,
};
use overdrive_core::wall_clock::UnixInstant;

use super::workload_lifecycle::backoff_for_attempt;
use super::{Action, Reconciler, ReconcilerName, TargetResource, TickContext};

/// Desired-side projection for a single service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDesired {
    /// Virtual IP the kernel-side XDP program matches incoming packets
    /// against.
    pub vip: ServiceVip,
    /// Service listener port — sourced from a listener-bearing fact
    /// (ADR-0060 site #8), never synthesised.
    pub port: NonZeroU16,
    /// L4 protocol — sourced from a listener-bearing fact, NEVER
    /// defaulted to `Tcp` (ADR-0060 C3).
    pub proto: Proto,
    /// Backend set, in deterministic `BTreeMap<BackendId, Backend>`
    /// iteration order.
    pub backends: Vec<Backend>,
    /// Content-hash of the `(vip, backends)` pair.
    pub fingerprint: BackendSetFingerprint,
}

/// Failure of the observation→desired projection
/// ([`project_service_desired`]). Per ADR-0060 C3 an unresolvable
/// listener protocol is a structured error — NEVER a silent `Proto::Tcp`
/// default.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServiceProjectionError {
    /// No listener-bearing fact resolves the service's L4 protocol.
    /// `ServiceBackendRow` carries neither port nor proto, so the proto
    /// MUST come from a `ListenerRow` per listener projection; when none is resolvable the projection
    /// fails rather than defaulting to `Tcp` (C3 guard).
    #[error(
        "no listener-bearing protocol fact for service {service_id} (vip {vip}); \
         refusing to default to Tcp (ADR-0060 C3)"
    )]
    NoListenerProto {
        /// Service whose proto could not be resolved.
        service_id: ServiceId,
        /// The service VIP, for operator-facing diagnostics.
        vip: std::net::Ipv4Addr,
    },
}

/// Project a `ServiceBackendRow` + its single listener-bearing fact into a
/// [`ServiceDesired`], sourcing `(port, proto)` from the listener fact.
///
/// Per ADR-0060 site #8 + C3: `ServiceBackendRow` carries neither port
/// nor proto, so the protocol MUST be sourced from the `ListenerRow` whose
/// `vip` matches the row's VIP. The caller pre-resolves the one fact for
/// this service via `ListenerFactStore::fact_for(row.service_id)` — an O(1)
/// keyed read on the full `(vip, port, proto)` identity — and passes it as
/// `Option<&ListenerRow>`. Taking a single optional fact (NOT a
/// `&[ListenerRow]`) makes a multi-listener input structurally
/// unrepresentable: a service co-locating protocols on one VIP (e.g.
/// tcp/53 + udp/53) can never have the wrong listener selected by slice
/// order, because `ServiceBackendRow` carries no proto discriminator the
/// function could use to disambiguate a slice. Per-listener fan-out
/// (US-05) derives a distinct `ServiceId` per listener, so each projection
/// still handles exactly one fact. When the fact is absent or carries a
/// non-matching VIP the projection returns
/// [`ServiceProjectionError::NoListenerProto`] — it NEVER defaults to
/// `Tcp`.
///
/// # Errors
///
/// Returns [`ServiceProjectionError::NoListenerProto`] when no matching
/// `ListenerRow` resolves the service's protocol.
pub fn project_service_desired(
    row: &overdrive_core::traits::observation_store::ServiceBackendRow,
    listener: Option<&overdrive_core::traits::observation_store::ListenerRow>,
) -> Result<ServiceDesired, ServiceProjectionError> {
    let vip = ServiceVip::new(std::net::IpAddr::V4(row.vip)).unwrap_or_else(|_| {
        unreachable!(
            "ServiceBackendRow.vip is a wire-shape Ipv4Addr; ServiceVip::new is total over IPv4"
        )
    });
    // Source `(port, proto)` from the single listener-bearing fact the
    // caller pre-resolved for this service. The fact's SSOT is the Service
    // intent's listeners (the allocator-issued VIP is stamped onto the
    // fact). The retained VIP guard rejects a fact handed in for a
    // different VIP rather than mis-stamping; an absent fact fails —
    // refusing to synthesise a `Proto::Tcp` default (C3).
    let listener =
        listener.filter(|l| l.vip == Some(vip)).ok_or(ServiceProjectionError::NoListenerProto {
            service_id: row.service_id,
            vip: row.vip,
        })?;
    let fingerprint = overdrive_core::dataplane::fingerprint::fingerprint(&vip, &row.backends);
    Ok(ServiceDesired {
        vip,
        port: listener.port,
        proto: listener.protocol,
        backends: row.backends.clone(),
        fingerprint,
    })
}

/// Hydrator state — split into `desired` and `actual` projections
/// merged by the runtime before `reconcile` per ADR-0036.
///
/// `BTreeMap` per `.claude/rules/development.md` § Ordered-collection
/// choice — deterministic iteration order is load-bearing for the
/// Maglev permutation generator that consumes the emitted action.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceMapHydratorState {
    /// Per-service desired backend set.
    pub desired: BTreeMap<ServiceId, ServiceDesired>,
    /// Per-service last-known hydration outcome.
    pub actual: BTreeMap<ServiceId, ServiceHydrationStatus>,
}

/// Per-service retry inputs — `attempts`,
/// `last_failure_seen_at`, `last_attempted_fingerprint` per
/// architecture.md § 8 *type View*.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RetryMemory {
    /// Number of `Action::DataplaneUpdateService` dispatches emitted
    /// for this service.
    #[serde(default)]
    pub attempts: u32,
    /// Wall-clock observation timestamp of the last failure.
    #[serde(default = "retry_memory_default_seen_at")]
    pub last_failure_seen_at: UnixInstant,
    /// Fingerprint of the most recently attempted backend set.
    #[serde(default)]
    pub last_attempted_fingerprint: Option<BackendSetFingerprint>,
}

/// Default `last_failure_seen_at` for serde — `UnixInstant` does not
/// implement `Default`, so we provide a sensible epoch-zero value
/// for new rows where no failure has been observed yet.
const fn retry_memory_default_seen_at() -> UnixInstant {
    UnixInstant::from_unix_duration(Duration::ZERO)
}

impl Default for RetryMemory {
    fn default() -> Self {
        Self {
            attempts: 0,
            last_failure_seen_at: retry_memory_default_seen_at(),
            last_attempted_fingerprint: None,
        }
    }
}

/// `ServiceMapHydrator` reconciler memory — `BTreeMap<ServiceId,
/// RetryMemory>` persisted by the runtime via `RedbViewStore` per
/// ADR-0035.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServiceMapHydratorView {
    /// Per-service retry inputs.
    #[serde(default)]
    pub retries: BTreeMap<ServiceId, RetryMemory>,
}

/// Reasons [`classify_backend_address`] rejects a backend address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendAddressRejection {
    /// `127.0.0.0/8`.
    Loopback,
    /// `169.254.0.0/16`.
    LinkLocal,
    /// `224.0.0.0/4`.
    Multicast,
    /// `255.255.255.255`.
    Broadcast,
    /// `0.0.0.0/8`.
    Reserved,
}

impl core::fmt::Display for BackendAddressRejection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Loopback => f.write_str("loopback (127.0.0.0/8)"),
            Self::LinkLocal => f.write_str("link-local (169.254.0.0/16)"),
            Self::Multicast => f.write_str("multicast (224.0.0.0/4)"),
            Self::Broadcast => f.write_str("broadcast (255.255.255.255)"),
            Self::Reserved => f.write_str("reserved (0.0.0.0/8)"),
        }
    }
}

/// Classify a candidate backend address.
pub const fn classify_backend_address(
    addr: std::net::Ipv4Addr,
) -> Result<(), BackendAddressRejection> {
    if addr.is_loopback() {
        return Err(BackendAddressRejection::Loopback);
    }
    if addr.is_link_local() {
        return Err(BackendAddressRejection::LinkLocal);
    }
    if addr.is_multicast() {
        return Err(BackendAddressRejection::Multicast);
    }
    if addr.is_broadcast() {
        return Err(BackendAddressRejection::Broadcast);
    }
    if addr.octets()[0] == 0 {
        return Err(BackendAddressRejection::Reserved);
    }
    Ok(())
}

/// The Phase 2 hydrator reconciler. Activates J-PLAT-004 (per
/// ADR-0042). Watches `service_backends` and `service_hydration_results`
/// observation rows; emits one `Action::DataplaneUpdateService` per
/// service whose backend-set fingerprint has drifted from the
/// confirmed-applied fingerprint.
pub struct ServiceMapHydrator {
    name: ReconcilerName,
    /// Per-host Path-A workload subnet (`WORKLOAD_SUBNET_BASE`,
    /// `10.99.0.0/16`). A backend whose V4 address is a member of this
    /// subnet is a Path-A/mesh backend: it is gated out of the
    /// load-balancer path (no `DataplaneUpdateService`) because nft-TPROXY owns its delivery
    /// (D-GATE / D-GATE-PRED; reconciles ADR-0053 ↔ ADR-0071). The
    /// SSOT is the provisioner's `WORKLOAD_SUBNET_BASE` const — the
    /// production wiring crate passes it; core takes the generic
    /// `Ipv4Net` value (core MUST NOT depend on the wiring crate).
    workload_subnet: ipnet::Ipv4Net,
}

impl ServiceMapHydrator {
    /// Construct the canonical `service-map-hydrator` instance.
    ///
    /// # Preconditions
    ///
    /// `workload_subnet` MUST be the same `WORKLOAD_SUBNET_BASE` the provisioner carves
    /// per-allocation `/30`s from — one source (D-GATE-PRED).
    ///
    /// # Panics
    ///
    /// Never — `Self::NAME` is a compile-time string literal
    /// satisfying every `ReconcilerName` validation rule.
    #[must_use]
    pub fn canonical(workload_subnet: ipnet::Ipv4Net) -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new(<Self as Reconciler>::NAME)
            .expect("'service-map-hydrator' is a valid ReconcilerName by construction");
        Self { name, workload_subnet }
    }

    /// The Path-A/mesh workload subnet the gate compares backends
    /// against.
    #[must_use]
    pub const fn workload_subnet(&self) -> ipnet::Ipv4Net {
        self.workload_subnet
    }
}

#[async_trait::async_trait]
impl Reconciler for ServiceMapHydrator {
    const NAME: &'static str = "service-map-hydrator";

    type State = ServiceMapHydratorState;
    type View = ServiceMapHydratorView;

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    // DISTILL scaffold: an empty target set, replaced by this reconciler's real enumeration.
    async fn managed_targets(
        &self,
        _ctx: &HydrationContext<'_>,
    ) -> Result<std::collections::BTreeSet<TargetResource>, HydrateError> {
        Ok(std::collections::BTreeSet::new())
    }

    fn reconcile(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        view: &Self::View,
        tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        let mut actions = Vec::new();
        let mut next_view = view.clone();

        for (service_id, desired_svc) in &desired.desired {
            // D-GATE / D-GATE-PRED — subnet-membership gate applied BEFORE
            // the VIP-family switch. A Path-A/mesh backend (V4 addr ∈
            // `workload_subnet`, 10.99.0.0/16) is gated out of the LB path:
            // nft-TPROXY owns its delivery (reconciles ADR-0053 ↔ ADR-0071).
            // The gate keys on the BACKEND's address, NOT the VIP's family, so
            // it applies uniformly to every VIP family.
            let workload_subnet = self.workload_subnet;
            let remote_survivors: Vec<Backend> = desired_svc
                .backends
                .iter()
                .filter(|b| match b.addr.ip() {
                    std::net::IpAddr::V4(v4) => !workload_subnet.contains(&v4),
                    std::net::IpAddr::V6(_) => true,
                })
                .cloned()
                .collect();

            // The post-gate survivors are the ONLY backends the dataplane
            // (XDP/REVERSE_NAT) is driven to program. The programmed
            // fingerprint is `fingerprint(vip, remote_survivors)` — byte-
            // identical to what `dataplane_update_service::dispatch` writes back
            // (`fingerprint(vip, action.backends)`) because the emitted action
            // carries exactly `remote_survivors`. Recomputed every tick from
            // inputs (`backends`, `workload_subnet`), NEVER persisted.
            let programmed_fingerprint = overdrive_core::dataplane::fingerprint::fingerprint(
                &desired_svc.vip,
                &remote_survivors,
            );

            let actual_status = actual.actual.get(service_id);
            let need_dispatch = should_dispatch(
                actual_status,
                programmed_fingerprint,
                view.retries.get(service_id),
                tick.now_unix,
            );

            if need_dispatch {
                let target_str = format!("service-map-hydrator/{service_id}");
                // `spec_hash` stays keyed on the FULL-set `desired_svc.fingerprint`
                // (convergence-model.md § 3.3): the correlation key identifies
                // "this service's desired-state revision"; the full-set
                // fingerprint is the stable identity of that revision (the
                // churn/identity signal). Do NOT re-key onto
                // `programmed_fingerprint`.
                let spec_hash = ContentHash::of(desired_svc.fingerprint.to_le_bytes().as_slice());

                // Emit the DataplaneUpdateService UNCONDITIONALLY on dispatch
                // (convergence-model.md § 3.3) — an empty `remote_survivors`
                // is the documented per-proto PURGE (traits/dataplane.rs
                // :197-204). This drives the dataplane to the empty state AND
                // produces the Completed{fingerprint(vip,[])} row that lets an
                // all-mesh / local-only service settle, AND tears down stranded
                // REVERSE_NAT/SERVICE_MAP entries on a non-mesh→all-mesh
                // transition (Finding 2, § 6). Uniform for V4 and V6 VIPs.
                actions.push(Action::DataplaneUpdateService {
                    service_id: *service_id,
                    vip: desired_svc.vip,
                    port: desired_svc.port,
                    proto: desired_svc.proto,
                    backends: remote_survivors.clone(),
                    correlation: CorrelationKey::derive(&target_str, &spec_hash, "update-service"),
                });

                // A dispatch ALWAYS emitted a DataplaneUpdateService now (incl.
                // the empty purge), so it always has a programmable fingerprint
                // to record. Record the PROGRAMMED fingerprint (matches what the
                // Completed row will carry), NOT the full-set fingerprint
                // (convergence-model.md § 3.4). The 02-02 "don't record a phantom
                // for an all-mesh service" guard is OBSOLETE — an all-mesh service
                // genuinely dispatches a purge, so recording its empty-set
                // programmed fingerprint is honest.
                let entry = next_view.retries.entry(*service_id).or_default();
                entry.attempts = entry.attempts.saturating_add(1);
                entry.last_failure_seen_at = tick.now_unix;
                entry.last_attempted_fingerprint = Some(programmed_fingerprint);
            } else if let Some(ServiceHydrationStatus::Completed { fingerprint, .. }) =
                actual_status
                && *fingerprint == programmed_fingerprint
            {
                next_view.retries.remove(service_id);
            }
        }

        // GC: drop retry memory for services no longer in `desired`.
        next_view.retries.retain(|service_id, _| desired.desired.contains_key(service_id));

        (actions, next_view)
    }

    /// Hydrate the `desired` service→backend-set map (ADR-0086 D1; moved off the
    /// central `reconciler_runtime::hydrate_desired` `ServiceMapHydrator` arm).
    /// Sources each service's `(port, proto)` from its listener-bearing fact via
    /// the `ListenerFacts` read-port — NEVER defaulting to `Tcp` (ADR-0060 C3):
    /// a service with no resolvable proto fact is SKIPPED.
    async fn hydrate_desired(
        &self,
        ctx: &HydrationContext<'_>,
        target: &TargetResource,
    ) -> Result<Self::State, HydrateError> {
        let service_id = super::service_id_from_target(target)?;
        let rows = ctx
            .observation_store
            .service_backends_rows(&service_id)
            .await
            .map_err(|e| HydrateError::ObservationRead(e.to_string()))?;
        let mut desired = BTreeMap::new();
        for row in rows {
            // O(1) keyed listener-fact read through the injected read-port
            // (was `state.listener_facts.lock().await.fact_for(...)`).
            let fact = ctx.listener_facts.fact_for(row.service_id).await;
            match project_service_desired(&row, fact.as_ref()) {
                Ok(desired_svc) => {
                    desired.insert(row.service_id, desired_svc);
                }
                Err(e) => {
                    tracing::warn!(
                        name: "service_map_hydrator.desired.unresolvable_proto",
                        service_id = %row.service_id,
                        error = %e,
                        "skipping service-map desired projection: no listener-bearing \
                         protocol fact; refusing to default to Tcp (ADR-0060 C3)"
                    );
                }
            }
        }
        Ok(ServiceMapHydratorState { desired, actual: BTreeMap::new() })
    }

    /// Hydrate the `actual` service→hydration-status map from
    /// `service_hydration_results` (ADR-0086 D1; moved off the central
    /// `hydrate_actual` `ServiceMapHydrator` arm). Retains the LWW-dominating row
    /// per `service_id`.
    async fn hydrate_actual(
        &self,
        ctx: &HydrationContext<'_>,
        target: &TargetResource,
    ) -> Result<Self::State, HydrateError> {
        let service_id = super::service_id_from_target(target)?;
        let rows = ctx
            .observation_store
            .service_hydration_results_rows(&service_id)
            .await
            .map_err(|e| HydrateError::ObservationRead(e.to_string()))?;
        let mut actual = BTreeMap::new();
        let mut latest: Option<ServiceHydrationResultRow> = None;
        for row in rows {
            let take = match latest.as_ref() {
                None => true,
                Some(current) => row.updated_at.dominates(&current.updated_at),
            };
            if take {
                latest = Some(row);
            }
        }
        if let Some(row) = latest {
            actual.insert(row.service_id, row.status);
        }
        Ok(ServiceMapHydratorState { desired: BTreeMap::new(), actual })
    }
}

/// Pure decision: has the attempt-budget backoff window elapsed for
/// the recorded retry memory at `now`?
///
/// Recomputes the deadline `last_failure_seen_at +
/// backoff_for_attempt(attempts)` from the persisted inputs on every
/// call (honors `.claude/rules/development.md` § "Persist inputs, not
/// derived state" — the deadline is never persisted, only its
/// `attempts` + `last_failure_seen_at` inputs). With no retry memory
/// the window is vacuously elapsed (never throttle a service that has
/// never been attempted).
fn backoff_window_elapsed(retry: Option<&RetryMemory>, now: UnixInstant) -> bool {
    retry.is_none_or(|r| now >= r.last_failure_seen_at + backoff_for_attempt(r.attempts))
}

/// Pure decision: dispatch a `DataplaneUpdateService` action this tick?
fn should_dispatch(
    actual_status: Option<&ServiceHydrationStatus>,
    desired_fingerprint: BackendSetFingerprint,
    retry: Option<&RetryMemory>,
    now: UnixInstant,
) -> bool {
    match actual_status {
        // No status row (dispatch suppressed before any result was
        // written) or a Pending row. Without a backoff gate this arm
        // spins every tick (RCA §Root cause B). Throttle on the
        // attempt-budget window ONLY when a prior attempt targeted the
        // SAME desired fingerprint; a fingerprint change (or no retry
        // memory) is a level-triggered spec change → dispatch
        // immediately and let the next emission reset the window
        // (client-go workqueue precedent, RCA §Fix B).
        None | Some(ServiceHydrationStatus::Pending) => {
            match retry.and_then(|r| r.last_attempted_fingerprint) {
                Some(last) if last == desired_fingerprint => backoff_window_elapsed(retry, now),
                _ => true,
            }
        }
        Some(ServiceHydrationStatus::Completed { fingerprint, .. }) => {
            *fingerprint != desired_fingerprint
        }
        Some(ServiceHydrationStatus::Failed { fingerprint, .. }) => {
            if *fingerprint != desired_fingerprint {
                return true;
            }
            backoff_window_elapsed(retry, now)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- should_dispatch attempt-budget gate (RCA §Fix B) ----
    //
    // `should_dispatch` is a pure decision over `(actual_status,
    // desired_fingerprint, retry, now)`. The fixtures below partition
    // the `None | Pending` arm by (fingerprint match/mismatch ×
    // now-vs-backoff-window). Time enters only via `now: UnixInstant`
    // — no `Instant::now()` (dst-lint stays green).

    /// Base wall-clock instant for the gate fixtures.
    fn t0() -> UnixInstant {
        UnixInstant::from_unix_duration(Duration::from_secs(100))
    }

    /// RCA §Root cause B (closure) / Regression test 2 (B half) — the
    /// load-bearing RED→GREEN pin. With a prior attempt at the SAME
    /// fingerprint, no status row (`actual_status = None`), and `now`
    /// inside the 1s backoff window, the gate must NOT re-dispatch
    /// (returns `false`). The current `None | Some(Pending) => true`
    /// arm returns `true` unconditionally → this fails against current
    /// code and passes after the fix.
    #[test]
    fn mixed_backend_does_not_re_dispatch_within_backoff_window_with_no_status_row() {
        let fingerprint: BackendSetFingerprint = 0xABCD_1234;
        let retry = RetryMemory {
            attempts: 1,
            last_failure_seen_at: t0(),
            last_attempted_fingerprint: Some(fingerprint),
        };
        let now = t0() + Duration::from_millis(500);

        assert!(
            !should_dispatch(None, fingerprint, Some(&retry), now),
            "within the backoff window at the same fingerprint, a suppressed dispatch \
             must NOT re-emit (no spin) — gate returns false"
        );
    }

    /// RCA §Fix B — level-triggered reset on spec change. With a prior
    /// attempt at a DIFFERENT fingerprint (the desired backend set
    /// changed), the gate dispatches immediately even inside the
    /// window — the backoff is keyed on the attempt budget for a given
    /// fingerprint, and a fingerprint change resets it (client-go
    /// workqueue precedent).
    #[test]
    fn mixed_backend_re_dispatches_immediately_on_fingerprint_change() {
        let old_fingerprint: BackendSetFingerprint = 0x0000_1111;
        let new_fingerprint: BackendSetFingerprint = 0x0000_2222;
        let retry = RetryMemory {
            attempts: 1,
            last_failure_seen_at: t0(),
            last_attempted_fingerprint: Some(old_fingerprint),
        };
        let now = t0() + Duration::from_millis(1);

        assert!(
            should_dispatch(None, new_fingerprint, Some(&retry), now),
            "a desired fingerprint differing from last_attempted_fingerprint must \
             re-dispatch immediately (spec changed → backoff reset)"
        );
    }

    /// RCA §Fix B — once the backoff window elapses for the same
    /// fingerprint, the gate re-dispatches. Mirrors the `Failed`-arm
    /// semantics via the shared `backoff_window_elapsed` helper.
    #[test]
    fn mixed_backend_re_dispatches_after_window_elapses() {
        let fingerprint: BackendSetFingerprint = 0xABCD_1234;
        let attempts = 1;
        let retry = RetryMemory {
            attempts,
            last_failure_seen_at: t0(),
            last_attempted_fingerprint: Some(fingerprint),
        };
        // now is exactly at the window boundary (>= triggers dispatch).
        let now = t0() + backoff_for_attempt(attempts);

        assert!(
            should_dispatch(None, fingerprint, Some(&retry), now),
            "at/after the backoff window boundary the gate must re-dispatch"
        );
    }
}
