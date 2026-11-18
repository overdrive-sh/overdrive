//! `ServiceMapHydrator` reconciler — Phase 2 (Slice 08; ASR-2.2-04).
//!
//! Watches the `service_backends` ObservationStore rows for backend-set
//! drift (the desired side) and reads the programmed service map through
//! `ProgrammedServiceView` (the actual side: the fingerprint of the backend
//! set the dataplane carries per service). Emits one
//! `Action::DataplaneUpdateService` per service that is not in place. The
//! executor's `service_hydration_results` row is a receipt the hydrator
//! never reads.
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

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU16;
use std::time::Duration;

use overdrive_core::aggregate::WorkloadIntent;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::dataplane::fingerprint::BackendSetFingerprint;
use overdrive_core::id::{ContentHash, CorrelationKey, ServiceId, ServiceVip};
use overdrive_core::reconcilers::{HydrateError, HydrationContext};
use overdrive_core::traits::dataplane::Backend;
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
    /// Per-service fingerprint of the backend set the dataplane has
    /// programmed. A service with no entry has nothing programmed.
    pub actual: BTreeMap<ServiceId, BackendSetFingerprint>,
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
/// ADR-0042). Reads the desired backend set from the `service_backends`
/// observation rows and the programmed fingerprint (`actual`) through
/// `ProgrammedServiceView`; emits one `Action::DataplaneUpdateService` per
/// service whose survivor-set fingerprint differs from the programmed one.
pub struct ServiceMapHydrator {
    name: ReconcilerName,
    /// Per-host Path-A workload subnet (`WORKLOAD_SUBNET_BASE`,
    /// `10.99.0.0/16`). A backend whose V4 address is a member of this
    /// subnet is a Path-A/mesh backend: it is gated out of the
    /// load-balancer path (no `DataplaneUpdateService`) because nft-TPROXY owns its delivery
    /// (D-GATE / D-GATE-PRED). The
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

    /// The backends that pass the mesh gate: the only ones the dataplane is
    /// driven to program. A Path-A/mesh backend (V4 addr in `workload_subnet`)
    /// is gated out of the load-balancer path (D-GATE / D-GATE-PRED); the gate
    /// keys on the BACKEND's address, never on the VIP's family.
    fn remote_survivors(&self, desired_svc: &ServiceDesired) -> Vec<Backend> {
        desired_svc
            .backends
            .iter()
            .filter(|b| match b.addr.ip() {
                std::net::IpAddr::V4(v4) => !self.workload_subnet.contains(&v4),
                std::net::IpAddr::V6(_) => true,
            })
            .cloned()
            .collect()
    }

    /// The survivor set the dataplane is driven to program, its fingerprint, and
    /// whether the programmed fingerprint already places it. Pure over its
    /// inputs; shared by `reconcile` and `next_evaluation_at` so both judge
    /// "in place" identically.
    fn survivor_placement(
        &self,
        desired_svc: &ServiceDesired,
        programmed: Option<BackendSetFingerprint>,
    ) -> (Vec<Backend>, BackendSetFingerprint, bool) {
        let survivors = self.remote_survivors(desired_svc);
        let survivor_fingerprint =
            overdrive_core::dataplane::fingerprint::fingerprint(&desired_svc.vip, &survivors);
        let placed = in_place(&survivors, survivor_fingerprint, programmed);
        (survivors, survivor_fingerprint, placed)
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

    /// Every `ServiceId` a deployed Service intent declares through an assigned
    /// VIP (the id `hydrate_desired` derives from VIP, port and protocol),
    /// unioned with every `ServiceId` that has a `service_backends` row (D-21).
    /// A `service_hydration_results` row alone adds no target. A read failure
    /// is the whole call's failure.
    async fn managed_targets(
        &self,
        ctx: &HydrationContext<'_>,
    ) -> Result<BTreeSet<TargetResource>, HydrateError> {
        let mut targets: BTreeSet<TargetResource> = ctx
            .observation_store
            .all_service_backends_rows()
            .await
            .map_err(|e| HydrateError::ObservationRead(e.to_string()))?
            .iter()
            .map(|row| crate::service_target(row.service_id))
            .collect();
        for (_workload_id, intent) in crate::workload_intents(ctx).await? {
            let WorkloadIntent::Service(service) = &intent else { continue };
            let digest =
                intent.spec_digest().map_err(|e| HydrateError::IntentRead(e.to_string()))?;
            let Some(vip) = ctx.service_vip_view.assigned_vip(&digest).await else { continue };
            targets.extend(service.listeners.iter().map(|listener| {
                let service_id =
                    ServiceId::derive(&vip, listener.port, listener.protocol, "service-map");
                crate::service_target(service_id)
            }));
        }
        Ok(targets)
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
            // The fingerprint is of the set the dataplane is driven to program:
            // byte-identical to what the dataplane writes back
            // (`fingerprint(vip, action.backends)`) because the emitted action
            // carries exactly `remote_survivors`. Recomputed every tick from
            // inputs, NEVER persisted.
            let (remote_survivors, survivor_fingerprint, placed) =
                self.survivor_placement(desired_svc, actual.actual.get(service_id).copied());
            let need_dispatch = !placed
                && pacing_allows(view.retries.get(service_id), survivor_fingerprint, tick.now_unix);

            if need_dispatch {
                let target_str = format!("service-map-hydrator/{service_id}");
                // `spec_hash` stays keyed on the FULL-set `desired_svc.fingerprint`
                // (convergence-model.md § 3.3): the correlation key identifies
                // "this service's desired-state revision". Do NOT re-key onto
                // `survivor_fingerprint`.
                let spec_hash = ContentHash::of(desired_svc.fingerprint.to_le_bytes().as_slice());

                // Emitted UNCONDITIONALLY on dispatch: an empty `remote_survivors`
                // is the documented per-proto PURGE (traits/dataplane.rs
                // :197-204), tearing down a still-programmed slot when a service
                // turns all-mesh. Uniform for V4 and V6 VIPs.
                actions.push(Action::DataplaneUpdateService {
                    service_id: *service_id,
                    vip: desired_svc.vip,
                    port: desired_svc.port,
                    proto: desired_svc.proto,
                    backends: remote_survivors,
                    correlation: CorrelationKey::derive(&target_str, &spec_hash, "update-service"),
                });

                // Retry memory only decides when to act again: it records the
                // attempted set so a changed desired set dispatches at once.
                let entry = next_view.retries.entry(*service_id).or_default();
                entry.attempts = entry.attempts.saturating_add(1);
                entry.last_failure_seen_at = tick.now_unix;
                entry.last_attempted_fingerprint = Some(survivor_fingerprint);
            } else if placed {
                next_view.retries.remove(service_id);
            }
        }

        // GC: drop retry memory for services no longer in `desired`.
        next_view.retries.retain(|service_id, _| desired.desired.contains_key(service_id));

        (actions, next_view)
    }

    /// D-3 retention: the earliest strictly-future retry deadline
    /// (`last_failure_seen_at + backoff_for_attempt(attempts)`) over the retry
    /// entries of services that are not in place. `None` when no service is
    /// waiting on a backoff. Reads only its inputs.
    fn next_evaluation_at(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        next_view: &Self::View,
        tick: &TickContext,
    ) -> Option<UnixInstant> {
        next_view
            .retries
            .iter()
            .filter(|(service_id, _)| {
                desired.desired.get(service_id).is_some_and(|desired_svc| {
                    let programmed = actual.actual.get(service_id).copied();
                    let (_, _, placed) = self.survivor_placement(desired_svc, programmed);
                    !placed
                })
            })
            .map(|(_, retry)| retry.last_failure_seen_at + backoff_for_attempt(retry.attempts))
            .filter(|deadline| *deadline > tick.now_unix)
            .min()
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

    /// Hydrate the `actual` map: the fingerprint of the backend set the
    /// dataplane has programmed per service, read through
    /// `ProgrammedServiceView` (ADR-0299). The target's frontend is re-projected
    /// exactly as `hydrate_desired` does (the `service_backends` row plus its
    /// listener fact through [`project_service_desired`]); a service whose
    /// frontend cannot be projected, or whose VIP is not IPv4, has nothing
    /// programmed and gets no entry. A failed read of the programmed map ends
    /// the evaluation as [`HydrateError::DataplaneRead`].
    async fn hydrate_actual(
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
        let mut actual = BTreeMap::new();
        for row in rows {
            let fact = ctx.listener_facts.fact_for(row.service_id).await;
            let Ok(projected) = project_service_desired(&row, fact.as_ref()) else {
                continue;
            };
            let Ok(frontend) = ServiceFrontend::new(projected.vip, projected.port, projected.proto)
            else {
                continue;
            };
            let programmed = ctx
                .programmed_services
                .programmed_fingerprint(&frontend)
                .await
                .map_err(|e| HydrateError::DataplaneRead(e.to_string()))?;
            if let Some(fingerprint) = programmed {
                actual.insert(row.service_id, fingerprint);
            }
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

/// Pure decision: is the service's slot already carrying its desired survivor
/// set? An empty survivor set is in place only when nothing is programmed
/// (a present entry is purged); a set holding an IPv6 backend is never in
/// place, because `update_service` refuses it.
fn in_place(
    survivors: &[Backend],
    survivor_fingerprint: BackendSetFingerprint,
    programmed: Option<BackendSetFingerprint>,
) -> bool {
    if survivors.is_empty() {
        return programmed.is_none();
    }
    let has_ipv6 = survivors.iter().any(|b| b.addr.is_ipv6());
    !has_ipv6 && programmed == Some(survivor_fingerprint)
}

/// Pure decision: may a not-in-place service dispatch now? A changed desired
/// set (or no retry memory) dispatches at once; the same set attempted before
/// waits for its backoff window.
fn pacing_allows(
    retry: Option<&RetryMemory>,
    survivor_fingerprint: BackendSetFingerprint,
    now: UnixInstant,
) -> bool {
    match retry.and_then(|r| r.last_attempted_fingerprint) {
        Some(last) if last == survivor_fingerprint => backoff_window_elapsed(retry, now),
        _ => true,
    }
}
