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

use crate::dataplane::backend_key::Proto;
use crate::dataplane::fingerprint::BackendSetFingerprint;
use crate::id::{ContentHash, CorrelationKey, ServiceId, ServiceVip};
use crate::traits::dataplane::Backend;
use crate::traits::observation_store::ServiceHydrationStatus;
use crate::wall_clock::UnixInstant;

use super::workload_lifecycle::backoff_for_attempt;
use super::{Action, Reconciler, ReconcilerName, TickContext};

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
    /// MUST come from a `ListenerRow` (or `BackendDiscoveryBridge`
    /// per-listener projection); when none is resolvable the projection
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

/// Project a `ServiceBackendRow` + its listener-bearing facts into a
/// [`ServiceDesired`], sourcing `(port, proto)` from the listener fact.
///
/// Per ADR-0060 site #8 + C3: `ServiceBackendRow` carries neither port
/// nor proto, so the protocol MUST be sourced from a `ListenerRow` whose
/// `vip` matches the row's VIP. The first matching listener wins (US-01
/// is single-listener; US-05 generalises to per-listener fan-out). When
/// no listener resolves the proto the projection returns
/// [`ServiceProjectionError::NoListenerProto`] — it NEVER defaults to
/// `Tcp`.
///
/// # Errors
///
/// Returns [`ServiceProjectionError::NoListenerProto`] when no
/// `ListenerRow` resolves the service's protocol.
pub fn project_service_desired(
    row: &crate::traits::observation_store::ServiceBackendRow,
    listeners: &[crate::traits::observation_store::ListenerRow],
) -> Result<ServiceDesired, ServiceProjectionError> {
    let vip = ServiceVip::new(std::net::IpAddr::V4(row.vip)).unwrap_or_else(|_| {
        unreachable!(
            "ServiceBackendRow.vip is a wire-shape Ipv4Addr; ServiceVip::new is total over IPv4"
        )
    });
    // Source `(port, proto)` from the listener-bearing fact whose `vip`
    // matches this service's VIP. The fact's SSOT is the Service intent's
    // listeners (the allocator-issued VIP is stamped onto the fact). When
    // no fact resolves, fail — refusing to synthesise a `Proto::Tcp`
    // default (C3).
    let listener = listeners.iter().find(|l| l.vip == Some(vip)).ok_or(
        ServiceProjectionError::NoListenerProto { service_id: row.service_id, vip: row.vip },
    )?;
    let fingerprint = crate::dataplane::fingerprint::fingerprint(&vip, &row.backends);
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

/// Reasons a backend address is rejected by the hydrator's
/// `Action::RegisterLocalBackend` precondition guard.
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
    /// Host's primary IPv4 — the classifier input per ADR-0053 § 4.
    host_ipv4: std::net::Ipv4Addr,
}

impl ServiceMapHydrator {
    /// Construct the canonical `service-map-hydrator` instance.
    ///
    /// # Preconditions
    ///
    /// `host_ipv4` MUST be the same value
    /// `BackendDiscoveryBridge` was constructed with.
    ///
    /// # Panics
    ///
    /// Never — `Self::NAME` is a compile-time string literal
    /// satisfying every `ReconcilerName` validation rule.
    #[must_use]
    pub fn canonical(host_ipv4: std::net::Ipv4Addr) -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new(<Self as Reconciler>::NAME)
            .expect("'service-map-hydrator' is a valid ReconcilerName by construction");
        Self { name, host_ipv4 }
    }

    /// The host IPv4 the classifier compares backends against.
    #[must_use]
    pub const fn host_ipv4(&self) -> std::net::Ipv4Addr {
        self.host_ipv4
    }
}

impl Reconciler for ServiceMapHydrator {
    const NAME: &'static str = "service-map-hydrator";

    type State = ServiceMapHydratorState;
    type View = ServiceMapHydratorView;

    fn name(&self) -> &ReconcilerName {
        &self.name
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
            let actual_status = actual.actual.get(service_id);
            let need_dispatch = should_dispatch(
                actual_status,
                desired_svc.fingerprint,
                view.retries.get(service_id),
                tick.now_unix,
            );

            if need_dispatch {
                let target_str = format!("service-map-hydrator/{service_id}");
                let spec_hash = ContentHash::of(desired_svc.fingerprint.to_le_bytes().as_slice());

                // ADR-0053 § 4 — per-backend Local-vs-Remote classification.
                let host_ipv4 = self.host_ipv4;
                let vip_v4 = match desired_svc.vip.get() {
                    std::net::IpAddr::V4(v4) => v4,
                    std::net::IpAddr::V6(_) => {
                        actions.push(Action::DataplaneUpdateService {
                            service_id: *service_id,
                            vip: desired_svc.vip,
                            port: desired_svc.port,
                            proto: desired_svc.proto,
                            backends: desired_svc.backends.clone(),
                            correlation: CorrelationKey::derive(
                                &target_str,
                                &spec_hash,
                                "update-service",
                            ),
                        });
                        let entry = next_view.retries.entry(*service_id).or_default();
                        entry.attempts = entry.attempts.saturating_add(1);
                        entry.last_failure_seen_at = tick.now_unix;
                        entry.last_attempted_fingerprint = Some(desired_svc.fingerprint);
                        continue;
                    }
                };

                let (local, remote): (Vec<&Backend>, Vec<&Backend>) =
                    desired_svc.backends.iter().partition(|b| match b.addr.ip() {
                        std::net::IpAddr::V4(v4) => v4 == host_ipv4,
                        std::net::IpAddr::V6(_) => false,
                    });

                let remote_is_empty = remote.is_empty();
                let local_is_empty = local.is_empty();

                if !remote_is_empty {
                    actions.push(Action::DataplaneUpdateService {
                        service_id: *service_id,
                        vip: desired_svc.vip,
                        port: desired_svc.port,
                        proto: desired_svc.proto,
                        backends: remote.into_iter().cloned().collect(),
                        correlation: CorrelationKey::derive(
                            &target_str,
                            &spec_hash,
                            "update-service",
                        ),
                    });
                }

                push_register_local_backend_actions(
                    &mut actions,
                    &local,
                    *service_id,
                    vip_v4,
                    desired_svc.port.get(),
                    &target_str,
                    &spec_hash,
                );

                let _ = (local_is_empty, remote_is_empty);

                let entry = next_view.retries.entry(*service_id).or_default();
                entry.attempts = entry.attempts.saturating_add(1);
                entry.last_failure_seen_at = tick.now_unix;
                entry.last_attempted_fingerprint = Some(desired_svc.fingerprint);
            } else if let Some(ServiceHydrationStatus::Completed { fingerprint, .. }) =
                actual_status
            {
                if *fingerprint == desired_svc.fingerprint {
                    next_view.retries.remove(service_id);
                }
            }
        }

        // GC: drop retry memory for services no longer in `desired`.
        next_view.retries.retain(|service_id, _| desired.desired.contains_key(service_id));

        (actions, next_view)
    }
}

/// Emit one `Action::RegisterLocalBackend` per local backend whose
/// address passes the ADR-0053 § 4 classifier guard. Backends with an
/// IPv6 address or a guard-rejected IPv4 (loopback / link-local /
/// multicast / broadcast / reserved) are skipped with a structured warn.
/// Extracted from `reconcile` to keep that method under the 100-line cap.
///
/// `vip_port` is the service's declared VIP listener port (the port a
/// client uses in `connect(vip:vip_port)`), NOT the backend's own
/// listening port. The `cgroup_connect4` hook keys `LOCAL_BACKEND_MAP`
/// on `(vip, vip_port)` and rewrites matching connects to the backend's
/// real address (ADR-0053 § 3); a service with VIP:53 → backend:5353
/// must register the entry under port 53 or the client's connect never
/// hits the map. See the `Dataplane::register_local_backend` contract.
fn push_register_local_backend_actions(
    actions: &mut Vec<Action>,
    local: &[&Backend],
    service_id: ServiceId,
    vip_v4: std::net::Ipv4Addr,
    vip_port: u16,
    target_str: &str,
    spec_hash: &ContentHash,
) {
    for backend in local {
        let backend_v4 = match backend.addr {
            std::net::SocketAddr::V4(s4) => s4,
            std::net::SocketAddr::V6(_) => continue,
        };
        if let Err(reason) = classify_backend_address(*backend_v4.ip()) {
            tracing::warn!(
                name: "service_map_hydrator.register_local_backend.rejected",
                service_id = %service_id,
                vip = %vip_v4,
                vip_port = vip_port,
                backend = %backend_v4,
                reason = %reason,
                "skipping RegisterLocalBackend: backend address rejected by classifier"
            );
            continue;
        }
        actions.push(Action::RegisterLocalBackend {
            service_id,
            vip: vip_v4,
            vip_port,
            backend: backend_v4,
            correlation: CorrelationKey::derive(target_str, spec_hash, "register-local-backend"),
        });
    }
}

/// Pure decision: dispatch a `DataplaneUpdateService` action this tick?
fn should_dispatch(
    actual_status: Option<&ServiceHydrationStatus>,
    desired_fingerprint: BackendSetFingerprint,
    retry: Option<&RetryMemory>,
    now: UnixInstant,
) -> bool {
    match actual_status {
        None | Some(ServiceHydrationStatus::Pending) => true,
        Some(ServiceHydrationStatus::Completed { fingerprint, .. }) => {
            *fingerprint != desired_fingerprint
        }
        Some(ServiceHydrationStatus::Failed { fingerprint, .. }) => {
            if *fingerprint != desired_fingerprint {
                return true;
            }
            retry.is_none_or(|r| now >= r.last_failure_seen_at + backoff_for_attempt(r.attempts))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::SpiffeId;
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

    fn spiffe(suffix: &str) -> SpiffeId {
        let raw = format!("spiffe://overdrive.local/job/svc/alloc/{suffix}");
        SpiffeId::new(&raw).expect("derived SpiffeId is valid by construction")
    }

    fn backend(addr: SocketAddr) -> Backend {
        Backend { alloc: spiffe("a"), addr, weight: 1, healthy: true }
    }

    fn service_id() -> ServiceId {
        ServiceId::new(42).expect("ServiceId accepts any u64")
    }

    fn spec_hash() -> ContentHash {
        ContentHash::of(b"service-map-hydrator-test-spec")
    }

    /// A valid local IPv4 backend yields exactly one
    /// `Action::RegisterLocalBackend` carrying the service identity, VIP,
    /// the declared VIP listener port as `vip_port`, the narrowed
    /// `SocketAddrV4`, and the `register-local-backend` correlation.
    /// Here the declared port (8080) and the backend's own port coincide;
    /// the VIP≠backend-port case is pinned separately below. Default-lane proxy
    /// for the Tier-3 reverse-NAT registration path — pins the emission
    /// the body owns so mutating the body to a no-op is caught here, not
    /// only behind the real-veth gate.
    #[test]
    fn push_register_local_backend_emits_action_for_valid_local_backend() {
        let vip_v4 = Ipv4Addr::new(10, 0, 0, 1);
        let target = "service/42";
        let hash = spec_hash();
        let backend_v4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 50), 8080);
        let local = backend(SocketAddr::V4(backend_v4));
        let local_refs: Vec<&Backend> = vec![&local];

        let mut actions = Vec::new();
        push_register_local_backend_actions(
            &mut actions,
            &local_refs,
            service_id(),
            vip_v4,
            8080,
            target,
            &hash,
        );

        assert_eq!(
            actions.len(),
            1,
            "exactly one RegisterLocalBackend must be emitted for a valid local backend"
        );
        let expected_correlation = CorrelationKey::derive(target, &hash, "register-local-backend");
        match &actions[0] {
            Action::RegisterLocalBackend {
                service_id: sid,
                vip,
                vip_port,
                backend: emitted,
                correlation,
            } => {
                assert_eq!(*sid, service_id(), "service_id must be threaded through");
                assert_eq!(*vip, vip_v4, "vip must be the host VIP");
                assert_eq!(
                    *vip_port, 8080,
                    "vip_port is the declared VIP listener port (here == backend port)"
                );
                assert_eq!(*emitted, backend_v4, "backend must be the narrowed SocketAddrV4");
                assert_eq!(
                    *correlation, expected_correlation,
                    "correlation must derive from (target, spec_hash, purpose)"
                );
            }
            other => panic!("expected RegisterLocalBackend, got {other:?}"),
        }
    }

    /// Regression (bugfix): when the service's declared VIP listener
    /// port differs from the backend's own listening port (e.g. a DNS
    /// service VIP:53 → backend:5353), the emitted `vip_port` MUST be
    /// the declared listener port (53), NOT the backend port (5353).
    /// The `cgroup_connect4` hook keys `LOCAL_BACKEND_MAP` on the port
    /// a client connects to (the declared VIP port); registering under
    /// the backend port leaves every `connect(vip:53)` a lookup miss
    /// with no rewrite. Pins the threading of `desired_svc.port`
    /// through to the action so a body that reverts to
    /// `backend.addr.port()` is caught here.
    #[test]
    fn push_register_local_backend_uses_declared_vip_port_not_backend_port() {
        let vip_v4 = Ipv4Addr::new(10, 0, 0, 1);
        let declared_vip_port: u16 = 53;
        let backend_port: u16 = 5353;
        let target = "service/42";
        let hash = spec_hash();
        let backend_v4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 50), backend_port);
        let local = backend(SocketAddr::V4(backend_v4));
        let local_refs: Vec<&Backend> = vec![&local];

        let mut actions = Vec::new();
        push_register_local_backend_actions(
            &mut actions,
            &local_refs,
            service_id(),
            vip_v4,
            declared_vip_port,
            target,
            &hash,
        );

        assert_eq!(actions.len(), 1, "exactly one RegisterLocalBackend must be emitted");
        match &actions[0] {
            Action::RegisterLocalBackend { vip_port, backend: emitted, .. } => {
                assert_eq!(
                    *vip_port, declared_vip_port,
                    "vip_port must be the declared VIP listener port (53), not the backend port"
                );
                assert_ne!(
                    *vip_port, backend_port,
                    "vip_port must NOT carry the backend's own port (5353)"
                );
                assert_eq!(
                    emitted.port(),
                    backend_port,
                    "the backend address still carries the backend's own port"
                );
            }
            other => panic!("expected RegisterLocalBackend, got {other:?}"),
        }
    }

    /// IPv6 and guard-rejected (loopback) backends are skipped — the fn
    /// emits nothing. Pins the two `continue` arms so a body that drops
    /// the guard cannot silently register a loopback or IPv6 backend.
    #[test]
    fn push_register_local_backend_skips_ipv6_and_guard_rejected() {
        let vip_v4 = Ipv4Addr::new(10, 0, 0, 1);
        let hash = spec_hash();
        let v6 = backend(SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 8080, 0, 0)));
        let loopback = backend(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)));
        let local_refs: Vec<&Backend> = vec![&v6, &loopback];

        let mut actions = Vec::new();
        push_register_local_backend_actions(
            &mut actions,
            &local_refs,
            service_id(),
            vip_v4,
            8080,
            "service/42",
            &hash,
        );

        assert!(
            actions.is_empty(),
            "IPv6 and guard-rejected backends must not produce RegisterLocalBackend actions"
        );
    }
}
