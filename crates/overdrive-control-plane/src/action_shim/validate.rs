//! Reconcile-output invariant validator.
//!
//! Runtime defense against a buggy reconciler that emits two or more
//! write-actions targeting the same service-LB VIP in a single
//! `reconcile()` return.
//!
//! # Why this lives at the dispatch boundary
//!
//! The convergence loop dispatches actions sequentially through the
//! [`action_shim::dispatch`](super::dispatch) loop. Two write actions
//! targeting the same key produce non-deterministic post-state in the
//! dataplane: whichever map wrote first is overwritten by whichever
//! wrote second, and the failure mode is silent (no error surfaces).
//! Per the Phase 16 D11 finding, sum-type-interior modelling on the
//! [`Action`] enum is insufficient — the enum admits valid actions
//! whose Vec-level composition is a bug. The runtime validator is the
//! right layer: it inspects the post-`reconcile` Vec and rejects the
//! aggregate before any dispatch fires.
//!
//! # Conflict classes
//!
//! Two write actions conflict when either:
//!
//! 1. **Same route, same `(vip, vip_port)`** — two cgroup writes to
//!    the same `LOCAL_BACKEND_MAP` slot, or two XDP writes for the
//!    same VIP. The second write silently overwrites the first; the
//!    reconciler emitting both in one tick is non-deterministic in
//!    its intent.
//! 2. **Cross-route on the same VIP** — an XDP `SERVICE_MAP` write
//!    AND a cgroup `LOCAL_BACKEND_MAP` write for the same VIP. The
//!    XDP path is keyed on VIP alone (no port — per-port routing
//!    happens inside `SERVICE_MAP`); the cgroup path is keyed on
//!    `(vip, vip_port)`. A backend served by both paths is
//!    reachable via two distinct kernel-side maps with
//!    non-deterministic precedence — the silhouette of the original
//!    defect.
//!
//! # Fail-safe semantics
//!
//! On violation, the caller [`run_convergence_tick`](crate::reconciler_runtime::run_convergence_tick)
//! skips action dispatch for the tick and logs a structured
//! `reconciler.output.invariant_violation` tracing event. The View
//! still persists (reconciler memory is independent of dispatch
//! success); convergence retries the next tick. The control-plane
//! does NOT panic on a buggy reconciler — the violation is a soft
//! failure surfaced to operators.
//!
//! Per `.claude/rules/development.md` § "Distinct failure modes get
//! distinct error variants": the validator returns a typed
//! [`ReconcilerOutputViolation`] with named structural fields
//! (the two conflicting routes + the shared VIP) so downstream
//! `matches!` branches do not have to parse `Display` strings.
//!
//! Per `.claude/rules/development.md` § "Ordered-collection choice":
//! the tracking maps are [`BTreeMap`]s so violation reproducibility
//! is deterministic across runs — the FIRST conflicting pair
//! surfaced does not depend on `HashMap` iteration order.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

use overdrive_core::reconciler::Action;

/// Route the action would take through the dataplane port boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteRoute {
    /// `Action::DataplaneUpdateService` — XDP `SERVICE_MAP` / Maglev
    /// path, keyed on `vip` alone at the kernel-side program.
    Xdp,
    /// `Action::RegisterLocalBackend` / `Action::DeregisterLocalBackend`
    /// — cgroup `connect4` rewrite path, keyed on `(vip, vip_port)`
    /// at the kernel-side program.
    Cgroup,
}

/// Violation surfaced by [`validate_reconcile_output`]. Per
/// `.claude/rules/development.md` § Errors / pass-through: typed
/// structural fields, not a flat string. Phase 1 has one variant; new
/// inter-action invariants land as additional variants on this enum
/// rather than as separate error types so the dispatch-boundary
/// caller can `matches!` on the structured cause without re-parsing
/// `Display`.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ReconcilerOutputViolation {
    /// Two write actions in the same `reconcile()` return target the
    /// same service VIP. `vip_port` is `Some(port)` for cgroup-vs-cgroup
    /// conflicts (the literal shared port) and `None` for any conflict
    /// involving the XDP path (which is per-VIP, not per-port).
    /// `first_route` is the route the FIRST-emitted action took;
    /// `second_route` is the offending (later-emitted) action's route.
    /// Both are captured so the operator-visible tracing event names
    /// exactly which pair conflicted.
    #[error(
        "conflicting service-LB writes at vip={vip} port={vip_port:?}: first={first_route:?}, second={second_route:?}"
    )]
    ConflictingServiceWrites {
        /// Virtual IP both writes target.
        vip: Ipv4Addr,
        /// VIP port both writes target. `Some(port)` for two cgroup
        /// writes to the same `(vip, port)` map slot; `None` for any
        /// conflict that includes an XDP-route write (which has no
        /// port).
        vip_port: Option<u16>,
        /// Route the FIRST emitted action takes.
        first_route: WriteRoute,
        /// Route the SECOND (conflicting) emitted action takes.
        second_route: WriteRoute,
    },
}

/// Walk `actions` in emission order; return `Err` on the first
/// inter-action conflict. The contract is *first-conflict-wins* — the
/// validator does NOT enumerate every conflict in a single error
/// because the dispatch boundary's response is the same regardless
/// (skip dispatch this tick, log, retry next tick). Surfacing the
/// first-conflicting pair gives operators a concrete VIP to grep for
/// in the broken reconciler's implementation.
///
/// # Errors
///
/// Returns [`ReconcilerOutputViolation::ConflictingServiceWrites`]
/// when two or more emitted actions target the same VIP via the
/// service-LB write surface, whether same-route at the same
/// `(vip, vip_port)` or cross-route on the same VIP.
pub fn validate_reconcile_output(actions: &[Action]) -> Result<(), ReconcilerOutputViolation> {
    // BTreeSet per `.claude/rules/development.md` § "Ordered-collection
    // choice" — error reproducibility requires deterministic
    // first-conflict surfacing across seeds; the structural defense
    // against `HashSet`'s `RandomState` iteration nondeterminism
    // applies equally to set-shaped trackers.
    //
    // Three trackers — XDP keyed by VIP, cgroup keyed by (vip, port)
    // for same-route, and cgroup-touched VIPs for cross-route. On
    // each new action we check ALL collision shapes: same-route
    // (look up in its own tracker) and cross-route (look up VIP in
    // the OTHER route's tracker).
    let mut xdp_vips: BTreeSet<Ipv4Addr> = BTreeSet::new();
    let mut cgroup_keys: BTreeSet<(Ipv4Addr, u16)> = BTreeSet::new();
    // Track all VIPs that have been touched by ANY cgroup write so
    // that a later XDP write on that VIP fires the cross-route
    // conflict. Separate from `cgroup_keys` because the cross-route
    // check is VIP-only, not (vip, port)-keyed.
    let mut cgroup_vips: BTreeSet<Ipv4Addr> = BTreeSet::new();

    for action in actions {
        let Some(WriteKey { vip, port_opt, route }) = service_write_key(action) else {
            continue;
        };
        match (route, port_opt) {
            // XDP-vs-XDP at same VIP.
            (WriteRoute::Xdp, _) if xdp_vips.contains(&vip) => {
                return Err(ReconcilerOutputViolation::ConflictingServiceWrites {
                    vip,
                    vip_port: None,
                    first_route: WriteRoute::Xdp,
                    second_route: WriteRoute::Xdp,
                });
            }
            // XDP-after-cgroup at same VIP.
            (WriteRoute::Xdp, _) if cgroup_vips.contains(&vip) => {
                return Err(ReconcilerOutputViolation::ConflictingServiceWrites {
                    vip,
                    vip_port: None,
                    first_route: WriteRoute::Cgroup,
                    second_route: WriteRoute::Xdp,
                });
            }
            (WriteRoute::Xdp, _) => {
                xdp_vips.insert(vip);
            }
            // Cgroup-vs-cgroup at same (vip, port).
            (WriteRoute::Cgroup, Some(port)) if cgroup_keys.contains(&(vip, port)) => {
                return Err(ReconcilerOutputViolation::ConflictingServiceWrites {
                    vip,
                    vip_port: Some(port),
                    first_route: WriteRoute::Cgroup,
                    second_route: WriteRoute::Cgroup,
                });
            }
            // Cgroup-after-XDP at same VIP.
            (WriteRoute::Cgroup, _) if xdp_vips.contains(&vip) => {
                return Err(ReconcilerOutputViolation::ConflictingServiceWrites {
                    vip,
                    vip_port: None,
                    first_route: WriteRoute::Xdp,
                    second_route: WriteRoute::Cgroup,
                });
            }
            (WriteRoute::Cgroup, Some(port)) => {
                cgroup_keys.insert((vip, port));
                cgroup_vips.insert(vip);
            }
            (WriteRoute::Cgroup, None) => {
                unreachable!(
                    "service_write_key always returns Some(port) for the Cgroup route; \
                     None here indicates a regression in service_write_key"
                );
            }
        }
    }

    Ok(())
}

/// Internal representation of a write-action's key. Avoids a tuple
/// return so the match-on-route below reads as named bindings rather
/// than `.0` / `.1` / `.2`.
struct WriteKey {
    vip: Ipv4Addr,
    port_opt: Option<u16>,
    route: WriteRoute,
}

/// Returns `Some((vip, port_opt, route))` for actions that write to
/// the service-LB dataplane; `None` for non-write actions.
///
/// `Action::DataplaneUpdateService` carries `vip: ServiceVip` only;
/// the validator surfaces `port_opt = None` because the XDP path is
/// per-VIP, not per-port. `Action::RegisterLocalBackend` /
/// `DeregisterLocalBackend` carry both `vip` + `vip_port`.
///
/// IPv6 VIPs (`ServiceVip::try_as_ipv4() == None`) are out of scope
/// for the cgroup path per ADR-0053 § 1 and structurally unreachable
/// in Phase 1 per ADR-0049 § 5 (the allocator's `VipRange` is
/// IPv4-only). An IPv6 VIP here is treated as "non-write" by the
/// validator — when the IPv6 path lands (GH #155) the conflict
/// surface will need a parallel IPv6 key class.
fn service_write_key(action: &Action) -> Option<WriteKey> {
    match action {
        Action::DataplaneUpdateService { vip, .. } => {
            vip.try_as_ipv4().map(|v4| WriteKey { vip: v4, port_opt: None, route: WriteRoute::Xdp })
        }
        Action::RegisterLocalBackend { vip, vip_port, .. }
        | Action::DeregisterLocalBackend { vip, vip_port, .. } => {
            Some(WriteKey { vip: *vip, port_opt: Some(*vip_port), route: WriteRoute::Cgroup })
        }
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};

    use overdrive_core::id::{ContentHash, CorrelationKey, ServiceId, ServiceVip};
    use overdrive_core::reconciler::Action;

    use super::{ReconcilerOutputViolation, WriteRoute, validate_reconcile_output};

    fn correlation(purpose: &str) -> CorrelationKey {
        let hash = ContentHash::of(purpose.as_bytes());
        CorrelationKey::derive("service-map-hydrator/1", &hash, purpose)
    }

    fn service_id() -> ServiceId {
        ServiceId::new(1).expect("ServiceId")
    }

    fn vip_v4(o1: u8) -> Ipv4Addr {
        Ipv4Addr::new(10, 96, 0, o1)
    }

    fn service_vip(o1: u8) -> ServiceVip {
        ServiceVip::new(IpAddr::V4(vip_v4(o1))).expect("ServiceVip")
    }

    fn register(vip: Ipv4Addr, vip_port: u16) -> Action {
        Action::RegisterLocalBackend {
            service_id: service_id(),
            vip,
            vip_port,
            backend: SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 5), 9090),
            correlation: correlation("register-local-backend"),
        }
    }

    fn deregister(vip: Ipv4Addr, vip_port: u16) -> Action {
        Action::DeregisterLocalBackend {
            service_id: service_id(),
            vip,
            vip_port,
            correlation: correlation("deregister-local-backend"),
        }
    }

    fn update_service(o1: u8) -> Action {
        Action::DataplaneUpdateService {
            service_id: service_id(),
            vip: service_vip(o1),
            backends: vec![],
            correlation: correlation("update-service"),
        }
    }

    /// Happy path — a mix of non-conflicting writes and non-write
    /// actions returns `Ok(())`. Two cgroup writes for distinct VIPs,
    /// one XDP write for a third VIP, plus Noops.
    #[test]
    fn validate_accepts_distinct_writes_and_noops() {
        let actions = vec![
            Action::Noop,
            update_service(1),           // XDP at 10.96.0.1
            register(vip_v4(2), 8080),   // Cgroup at 10.96.0.2:8080
            register(vip_v4(2), 9090),   // Cgroup at 10.96.0.2:9090 — same VIP different port ok
            deregister(vip_v4(3), 7000), // Cgroup at 10.96.0.3:7000
            Action::Noop,
        ];
        assert!(validate_reconcile_output(&actions).is_ok());
    }

    /// Cgroup-vs-XDP conflict (the canonical defect class the task
    /// describes): the same VIP is authored by both the XDP path
    /// (`DataplaneUpdateService`) and the cgroup path
    /// (`RegisterLocalBackend`) in the same tick. Cross-route
    /// conflict on the shared VIP fires regardless of cgroup port.
    #[test]
    fn validate_rejects_xdp_then_cgroup_for_same_vip() {
        let actions = vec![update_service(1), register(vip_v4(1), 8080)];
        let err = validate_reconcile_output(&actions)
            .expect_err("XDP + cgroup writes for same VIP must conflict");
        match err {
            ReconcilerOutputViolation::ConflictingServiceWrites {
                vip,
                vip_port,
                first_route,
                second_route,
            } => {
                assert_eq!(vip, vip_v4(1));
                assert_eq!(vip_port, None, "cross-route conflict reports vip-only");
                assert_eq!(first_route, WriteRoute::Xdp);
                assert_eq!(second_route, WriteRoute::Cgroup);
            }
        }
    }

    /// Mirror of the cross-route conflict with the actions in the
    /// opposite emission order — cgroup first, then XDP. The
    /// validator reports the FIRST-emitted route as `first_route`.
    #[test]
    fn validate_rejects_cgroup_then_xdp_for_same_vip() {
        let actions = vec![register(vip_v4(1), 8080), update_service(1)];
        let err = validate_reconcile_output(&actions)
            .expect_err("cgroup + XDP writes for same VIP must conflict");
        match err {
            ReconcilerOutputViolation::ConflictingServiceWrites {
                vip,
                vip_port,
                first_route,
                second_route,
            } => {
                assert_eq!(vip, vip_v4(1));
                assert_eq!(vip_port, None);
                assert_eq!(first_route, WriteRoute::Cgroup);
                assert_eq!(second_route, WriteRoute::Xdp);
            }
        }
    }

    /// Register-vs-Deregister conflict — two cgroup-path writes to
    /// the same `(vip, vip_port)` slot in one tick are a bug even
    /// though both share the route. Whichever the dispatcher
    /// happens to apply second wins; the reconciler emitting both
    /// in one return is non-deterministic in its intent.
    #[test]
    fn validate_rejects_register_and_deregister_for_same_key() {
        let actions = vec![register(vip_v4(7), 5000), deregister(vip_v4(7), 5000)];
        let err = validate_reconcile_output(&actions)
            .expect_err("register+deregister at same (vip, port) must conflict");
        match err {
            ReconcilerOutputViolation::ConflictingServiceWrites {
                vip,
                vip_port,
                first_route,
                second_route,
            } => {
                assert_eq!(vip, vip_v4(7));
                assert_eq!(vip_port, Some(5000));
                assert_eq!(first_route, WriteRoute::Cgroup);
                assert_eq!(second_route, WriteRoute::Cgroup);
            }
        }
    }

    /// XDP-vs-XDP — two `DataplaneUpdateService`s for the same VIP
    /// in one tick. Same-route same-VIP conflict.
    #[test]
    fn validate_rejects_two_xdp_writes_for_same_vip() {
        let actions = vec![update_service(2), update_service(2)];
        assert!(matches!(
            validate_reconcile_output(&actions),
            Err(ReconcilerOutputViolation::ConflictingServiceWrites {
                first_route: WriteRoute::Xdp,
                second_route: WriteRoute::Xdp,
                ..
            })
        ));
    }

    /// Empty action vec is trivially valid — no writes, no
    /// conflicts.
    #[test]
    fn validate_accepts_empty_vec() {
        assert!(validate_reconcile_output(&[]).is_ok());
    }
}
