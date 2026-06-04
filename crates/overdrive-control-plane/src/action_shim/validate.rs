//! Reconcile-output invariant validator.
//!
//! Runtime defense against a buggy reconciler that emits two or more
//! write-actions targeting the SAME dataplane map slot in a single
//! `reconcile()` return.
//!
//! # Why this lives at the dispatch boundary
//!
//! The convergence loop dispatches actions sequentially through the
//! [`action_shim::dispatch`](super::dispatch) loop. Two write actions
//! targeting the same map slot produce non-deterministic post-state in
//! the dataplane: whichever wrote first is overwritten by whichever
//! wrote second, and the failure mode is silent (no error surfaces).
//! Sum-type-interior modelling on the [`Action`] enum is insufficient —
//! the enum admits valid actions whose Vec-level composition is a bug.
//! The runtime validator is the right layer: it inspects the
//! post-`reconcile` Vec and rejects the aggregate before any dispatch
//! fires.
//!
//! # Conflict granularity — `(route, key-tuple)`, never the shared VIP
//!
//! A conflict exists iff two write actions target the SAME map slot.
//! The unit of conflict is the owned `(route, key-tuple)`, not the
//! shared parent VIP — the same granularity Kubernetes Server-Side
//! Apply uses (conflict = collision on an owned field, never
//! co-residence on the object) and Cilium uses (socket-LB
//! `cgroup connect4` and the XDP/tc datapath are complementary,
//! "transparent" surfaces for one ClusterIP). See ADR-0053 revision
//! 2026-06-03 ("dispatch-boundary conflict granularity is
//! `(route, key-tuple)`") and
//! `docs/research/reconcilers/dispatch-boundary-validation-and-attempt-budget-backoff.md`.
//!
//! Two write actions conflict only when:
//!
//! 1. **Same route, same slot** — two cgroup writes to the same
//!    `LOCAL_BACKEND_MAP` `(vip, vip_port, proto)` slot, or two XDP
//!    writes to the same `SERVICE_MAP` `(vip, port, proto)` slot. The
//!    second write silently overwrites the first; a reconciler emitting
//!    both in one tick is non-deterministic in its intent. Step 02-01
//!    widened the XDP slot from VIP-only to `(vip, port, proto)`
//!    IPVS-style; step 02-02 widened the cgroup slot the same way.
//!    Distinct ports (tcp/8080 + tcp/8081) and distinct proto (tcp/53 +
//!    udp/53) are distinct slots on EITHER route and do NOT conflict.
//!
//! # Cross-route on one VIP is NOT a conflict (ADR-0053 § 4 dual-path)
//!
//! An XDP `SERVICE_MAP` write AND a cgroup `LOCAL_BACKEND_MAP` write for
//! the same VIP in one tick is the BLESSED dual-path of ADR-0053
//! Decisions 2/4/5, NOT a conflict. The XDP path serves remote
//! backends; the cgroup path serves local backends; the
//! `ServiceMapHydrator` classifier (ADR-0053 § 4) partitions each
//! backend into exactly one route. The two routes are disjoint kernel
//! maps consumed by different hooks with no precedence race —
//! `cgroup_connect4` rewrites the connect at `connect(2)` time, before
//! the kernel routes the SYN to XDP ingress. A VIP appearing on both
//! routes is the correct shape for a mixed local+remote service. The
//! validator MUST NOT reject it.
//!
//! # Provenance
//!
//! The Phase-16 D11 finding
//! (`docs/evolution/2026-05-23-backend-discovery-bridge-service-reachability.md`
//! § "Reconcile-output invariant at the action_shim boundary") governs
//! SAME-CLASS write conflicts only — two `WriteServiceBackendRow`
//! (observation-row) writes to one VIP with conflicting backend sets, a
//! genuine same-slot overwrite. D11 does NOT authorise a cross-route
//! (XDP-vs-cgroup) rule; the cross-route composition is the ADR-0053
//! § 4 dual-path described above. This validator originally
//! over-generalised D11 into a VIP-level cross-route rejection; that
//! rule is removed (see ADR-0053 revision 2026-06-03).
//!
//! # Fail-safe semantics
//!
//! On violation, the caller [`run_convergence_tick`](crate::reconciler_runtime::run_convergence_tick)
//! skips action dispatch for the tick and surfaces the violation on two
//! channels (Kubernetes Events model — a machine-queryable control
//! signal distinct from a best-effort human signal):
//!
//! - a queryable `reconcile_conflict` observation row written through
//!   the `ObservationStore` (the durable, machine-queryable surface —
//!   operators query
//!   [`ObservationStore::reconcile_conflict_rows`](overdrive_core::traits::observation_store::ObservationStore::reconcile_conflict_rows)
//!   for the conflicting `(service_id, vip, port, proto)` slot and the
//!   two routes), AND
//! - a structured `reconciler.output.invariant_violation` tracing event
//!   (the supplemental best-effort human signal).
//!
//! The View still persists (reconciler memory is independent of
//! dispatch success); convergence retries the next tick. The
//! control-plane does NOT panic on a buggy reconciler — the violation
//! is a soft failure surfaced to operators (RCA
//! `fix-mixed-backend-dispatch-spin` § Fix C).
//!
//! Per `.claude/rules/development.md` § "Distinct failure modes get
//! distinct error variants": the validator returns a typed
//! [`ReconcilerOutputViolation`] with named structural fields
//! (the conflicting route + the shared `(vip, port, proto)` slot) so
//! downstream `matches!` branches do not have to parse `Display`
//! strings.
//!
//! Per `.claude/rules/development.md` § "Ordered-collection choice":
//! the tracking sets are [`BTreeSet`]s so violation reproducibility
//! is deterministic across runs — the FIRST conflicting pair surfaced
//! does not depend on `HashSet` iteration order.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::ServiceId;
use overdrive_core::reconcilers::Action;

/// Route the action would take through the dataplane port boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteRoute {
    /// `Action::DataplaneUpdateService` — XDP `SERVICE_MAP` / Maglev
    /// path, keyed on `(vip, port, proto)` at the kernel-side program
    /// (step 02-01 widened from VIP-only).
    Xdp,
    /// `Action::RegisterLocalBackend` / `Action::DeregisterLocalBackend`
    /// — cgroup `connect4` rewrite path, keyed on `(vip, vip_port, proto)`
    /// at the kernel-side program (step 02-02 widened from
    /// `(vip, vip_port)`).
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
    /// same `(route, vip, port, proto)` map slot. The only conflict
    /// classes after the ADR-0053 revision 2026-06-03 are same-route
    /// same-slot: cgroup-vs-cgroup (a shared `(vip, port, proto)` slot,
    /// since step 02-02) and XDP-vs-XDP (a shared `(vip, port, proto)`
    /// slot, since step 02-01). Cross-route (XDP + cgroup) on one VIP
    /// is the blessed § 4 dual-path and is NOT a conflict.
    /// `first_route` and `second_route` are therefore always equal in
    /// Phase 1; both are retained so the operator-visible tracing event
    /// names exactly which pair conflicted and so a future cross-class
    /// invariant can populate them distinctly without a variant churn.
    #[error(
        "conflicting service-LB writes at vip={vip} port={vip_port:?}: first={first_route:?}, second={second_route:?}"
    )]
    ConflictingServiceWrites {
        /// Identity of the service both writes target. Carried so the
        /// dispatch-boundary caller can populate the queryable
        /// `reconcile_conflict` observation row's primary key without
        /// re-deriving it from the VIP (Fix C, RCA
        /// `fix-mixed-backend-dispatch-spin`). The two conflicting
        /// actions both carry this `service_id`.
        service_id: ServiceId,
        /// Virtual IP both writes target.
        vip: Ipv4Addr,
        /// VIP port both writes target. Always `Some(port)` in Phase 1
        /// — every surviving conflict is same-route same-slot and so
        /// carries the shared `(vip, port, proto)` slot's port. Kept as
        /// `Option<u16>` to avoid churning the variant + downstream
        /// `matches!` if a future port-less conflict class lands.
        vip_port: Option<u16>,
        /// L4 protocol of the conflicting `(vip, port, proto)` slot.
        /// Carried so the dispatch-boundary caller can populate the
        /// queryable `reconcile_conflict` observation row's slot key
        /// directly, without re-deriving it from the action list (Fix C).
        proto: Proto,
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
/// when two or more emitted actions target the same same-route map
/// slot — two XDP writes to one `(vip, port, proto)` `SERVICE_MAP`
/// slot, or two cgroup writes to one `(vip, vip_port, proto)`
/// `LOCAL_BACKEND_MAP` slot. Cross-route (XDP + cgroup) co-residence on
/// one VIP is the ADR-0053 § 4 dual-path and is accepted.
pub fn validate_reconcile_output(actions: &[Action]) -> Result<(), ReconcilerOutputViolation> {
    // BTreeSet per `.claude/rules/development.md` § "Ordered-collection
    // choice" — error reproducibility requires deterministic
    // first-conflict surfacing across seeds; the structural defense
    // against `HashSet`'s `RandomState` iteration nondeterminism
    // applies equally to set-shaped trackers.
    //
    // Two trackers, one per route, each holding the full
    // `(vip, port, proto)` slot tuple:
    //   - `xdp_keys`: SERVICE_MAP outer-key slots for XDP-vs-XDP
    //     same-slot detection. Two XDP writes conflict only when ALL
    //     THREE match. Distinct ports (tcp/8080 + tcp/8081) and distinct
    //     proto (tcp/53 + udp/53) are distinct slots → no conflict.
    //   - `cgroup_keys`: LOCAL_BACKEND_MAP slots for cgroup-vs-cgroup
    //     same-slot detection (step 02-02 carries proto).
    //
    // There is NO cross-route tracker. An XDP write and a cgroup write
    // for the same VIP are the ADR-0053 § 4 dual-path (disjoint kernel
    // maps, disjoint hooks, no precedence race), not a conflict — see
    // the module doc. Conflict granularity is `(route, key-tuple)`,
    // never the shared VIP.
    let mut xdp_keys: BTreeSet<(Ipv4Addr, u16, Proto)> = BTreeSet::new();
    let mut cgroup_keys: BTreeSet<(Ipv4Addr, u16, Proto)> = BTreeSet::new();

    for action in actions {
        let Some(WriteKey { service_id, vip, port_opt, proto_opt, route }) =
            service_write_key(action)
        else {
            continue;
        };
        match (route, port_opt, proto_opt) {
            // XDP-vs-XDP at same (vip, port, proto) — genuine duplicate
            // outer-map slot. Reports the shared port (the slot is now
            // port-specific).
            (WriteRoute::Xdp, Some(port), Some(proto))
                if xdp_keys.contains(&(vip, port, proto)) =>
            {
                return Err(ReconcilerOutputViolation::ConflictingServiceWrites {
                    service_id,
                    vip,
                    vip_port: Some(port),
                    proto,
                    first_route: WriteRoute::Xdp,
                    second_route: WriteRoute::Xdp,
                });
            }
            (WriteRoute::Xdp, Some(port), Some(proto)) => {
                xdp_keys.insert((vip, port, proto));
            }
            (WriteRoute::Xdp, _, _) => {
                unreachable!(
                    "service_write_key always returns Some(port) + Some(proto) for the Xdp \
                     route; None here indicates a regression in service_write_key"
                );
            }
            // Cgroup-vs-cgroup at same (vip, port, proto) — step 02-02
            // widened the cgroup slot to carry proto, mirroring the
            // LOCAL_BACKEND_MAP key. Two writes only conflict when ALL
            // THREE match; tcp/53 + udp/53 are distinct slots (same-host
            // DNS unlocked) and do NOT conflict.
            (WriteRoute::Cgroup, Some(port), Some(proto))
                if cgroup_keys.contains(&(vip, port, proto)) =>
            {
                return Err(ReconcilerOutputViolation::ConflictingServiceWrites {
                    service_id,
                    vip,
                    vip_port: Some(port),
                    proto,
                    first_route: WriteRoute::Cgroup,
                    second_route: WriteRoute::Cgroup,
                });
            }
            (WriteRoute::Cgroup, Some(port), Some(proto)) => {
                cgroup_keys.insert((vip, port, proto));
            }
            (WriteRoute::Cgroup, _, _) => {
                unreachable!(
                    "service_write_key always returns Some(port) + Some(proto) for the Cgroup \
                     route (step 02-02 widened the key); None here indicates a regression in \
                     service_write_key"
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
    service_id: ServiceId,
    vip: Ipv4Addr,
    port_opt: Option<u16>,
    proto_opt: Option<Proto>,
    route: WriteRoute,
}

/// Returns `Some(WriteKey)` for actions that write to the service-LB
/// dataplane; `None` for non-write actions.
///
/// Step 02-01: `Action::DataplaneUpdateService` carries `vip`, `port`
/// AND `proto` — the XDP write-key is now the full `(vip, port, proto)`
/// tuple matching the widened SERVICE_MAP outer key. Two such writes
/// only conflict when all three match; distinct ports or distinct
/// proto are distinct outer-map slots.
///
/// `Action::RegisterLocalBackend` / `DeregisterLocalBackend` carry
/// `vip` + `vip_port` + `proto` — step 02-02 widened the cgroup
/// write-key to the full `(vip, vip_port, proto)` tuple matching the
/// widened `LOCAL_BACKEND_MAP` key. Two such writes only conflict when
/// all three match; tcp/53 + udp/53 are distinct slots.
///
/// IPv6 VIPs (`ServiceVip::try_as_ipv4() == None`) are out of scope
/// for the cgroup path per ADR-0053 § 1 and structurally unreachable
/// in Phase 1 per ADR-0049 § 5 (the allocator's `VipRange` is
/// IPv4-only). An IPv6 VIP here is treated as "non-write" by the
/// validator — when the IPv6 path lands (GH #155) the conflict
/// surface will need a parallel IPv6 key class.
fn service_write_key(action: &Action) -> Option<WriteKey> {
    match action {
        Action::DataplaneUpdateService { service_id, vip, port, proto, .. } => {
            vip.try_as_ipv4().map(|v4| WriteKey {
                service_id: *service_id,
                vip: v4,
                port_opt: Some(port.get()),
                proto_opt: Some(*proto),
                route: WriteRoute::Xdp,
            })
        }
        Action::RegisterLocalBackend { service_id, vip, vip_port, proto, .. }
        | Action::DeregisterLocalBackend { service_id, vip, vip_port, proto, .. } => {
            Some(WriteKey {
                service_id: *service_id,
                vip: *vip,
                port_opt: Some(*vip_port),
                proto_opt: Some(*proto),
                route: WriteRoute::Cgroup,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};

    use overdrive_core::dataplane::backend_key::Proto;
    use overdrive_core::id::{ContentHash, CorrelationKey, ServiceId, ServiceVip};
    use overdrive_core::reconcilers::Action;

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
        register_proto(vip, vip_port, Proto::Tcp)
    }

    fn register_proto(vip: Ipv4Addr, vip_port: u16, proto: Proto) -> Action {
        Action::RegisterLocalBackend {
            service_id: service_id(),
            vip,
            vip_port,
            proto,
            backend: SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 5), 9090),
            correlation: correlation("register-local-backend"),
        }
    }

    fn deregister(vip: Ipv4Addr, vip_port: u16) -> Action {
        Action::DeregisterLocalBackend {
            service_id: service_id(),
            vip,
            vip_port,
            proto: Proto::Tcp,
            correlation: correlation("deregister-local-backend"),
        }
    }

    fn update_service(o1: u8) -> Action {
        update_service_pp(o1, overdrive_core::dataplane::backend_key::Proto::Tcp, 8080)
    }

    fn update_service_pp(
        o1: u8,
        proto: overdrive_core::dataplane::backend_key::Proto,
        port: u16,
    ) -> Action {
        Action::DataplaneUpdateService {
            service_id: service_id(),
            vip: service_vip(o1),
            port: std::num::NonZeroU16::new(port).expect("non-zero"),
            proto,
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
                service_id: sid,
                vip,
                vip_port,
                proto,
                first_route,
                second_route,
            } => {
                assert_eq!(sid, service_id());
                assert_eq!(vip, vip_v4(7));
                assert_eq!(vip_port, Some(5000));
                assert_eq!(proto, Proto::Tcp);
                assert_eq!(first_route, WriteRoute::Cgroup);
                assert_eq!(second_route, WriteRoute::Cgroup);
            }
        }
    }

    /// XDP-vs-XDP — two `DataplaneUpdateService`s for the same
    /// `(vip, port, proto)` slot in one tick. Same-route same-slot
    /// conflict; step 02-01 reports the shared port (the slot is now
    /// port+proto-specific).
    #[test]
    fn validate_rejects_two_xdp_writes_for_same_slot() {
        let actions = vec![update_service(2), update_service(2)];
        match validate_reconcile_output(&actions) {
            Err(ReconcilerOutputViolation::ConflictingServiceWrites {
                service_id: sid,
                vip,
                vip_port,
                proto,
                first_route,
                second_route,
            }) => {
                assert_eq!(sid, service_id());
                assert_eq!(vip, vip_v4(2));
                assert_eq!(vip_port, Some(8080), "XDP-vs-XDP now reports the shared port");
                assert_eq!(proto, Proto::Tcp);
                assert_eq!(first_route, WriteRoute::Xdp);
                assert_eq!(second_route, WriteRoute::Xdp);
            }
            other => panic!("expected XDP-vs-XDP conflict, got {other:?}"),
        }
    }

    /// S-02-01 — same VIP, DIFFERENT ports via the XDP path now pass.
    /// Before 02-01 the XDP write-key was VIP-only so these falsely
    /// conflicted. The widened `(vip, port, proto)` key makes them
    /// distinct slots.
    #[test]
    fn validate_accepts_xdp_same_vip_different_ports() {
        use overdrive_core::dataplane::backend_key::Proto;
        let actions =
            vec![update_service_pp(1, Proto::Tcp, 8080), update_service_pp(1, Proto::Tcp, 8081)];
        assert!(
            validate_reconcile_output(&actions).is_ok(),
            "same VIP different ports must not conflict"
        );
    }

    /// S-02-01 — DNS co-location: same `(vip, port)`, DIFFERENT proto
    /// via the XDP path now passes (tcp/53 + udp/53).
    #[test]
    fn validate_accepts_xdp_same_vip_port_different_proto() {
        use overdrive_core::dataplane::backend_key::Proto;
        let actions =
            vec![update_service_pp(1, Proto::Tcp, 53), update_service_pp(1, Proto::Udp, 53)];
        assert!(
            validate_reconcile_output(&actions).is_ok(),
            "same (vip,port) different proto must not conflict"
        );
    }

    /// S-02-02 — cgroup DNS co-location: same `(vip, port)`, DIFFERENT
    /// proto via the cgroup path now passes (tcp/53 + udp/53). Step
    /// 02-02 widened the cgroup write-key to `(vip, port, proto)`.
    #[test]
    fn validate_accepts_cgroup_same_vip_port_different_proto() {
        let actions = vec![
            register_proto(vip_v4(1), 53, Proto::Tcp),
            register_proto(vip_v4(1), 53, Proto::Udp),
        ];
        assert!(
            validate_reconcile_output(&actions).is_ok(),
            "same (vip,port) different proto on the cgroup path must not conflict"
        );
    }

    /// S-02-02 — genuine cgroup duplicate slot is STILL caught. Two
    /// `RegisterLocalBackend` for IDENTICAL `(vip, port, proto)` remain
    /// a conflict reporting the shared port.
    #[test]
    fn validate_rejects_cgroup_identical_vip_port_proto() {
        let actions = vec![
            register_proto(vip_v4(1), 53, Proto::Tcp),
            register_proto(vip_v4(1), 53, Proto::Tcp),
        ];
        match validate_reconcile_output(&actions) {
            Err(ReconcilerOutputViolation::ConflictingServiceWrites {
                service_id: sid,
                vip,
                vip_port,
                proto,
                first_route,
                second_route,
            }) => {
                assert_eq!(sid, service_id());
                assert_eq!(vip, vip_v4(1));
                assert_eq!(vip_port, Some(53), "cgroup-vs-cgroup reports the shared port");
                assert_eq!(proto, Proto::Tcp);
                assert_eq!(first_route, WriteRoute::Cgroup);
                assert_eq!(second_route, WriteRoute::Cgroup);
            }
            other => panic!("expected cgroup-vs-cgroup conflict, got {other:?}"),
        }
    }

    /// ADR-0053 § 4 dual-path — an XDP `SERVICE_MAP` write AND a cgroup
    /// `LOCAL_BACKEND_MAP` write for the SAME VIP in one tick is the
    /// blessed mixed local+remote shape, NOT a conflict. The two routes
    /// are disjoint kernel maps consumed by different hooks with no
    /// precedence race (`cgroup_connect4` rewrites the connect before
    /// the SYN routes to XDP ingress). The validator MUST accept it.
    /// See ADR-0053 revision 2026-06-03.
    #[test]
    fn validate_accepts_xdp_and_cgroup_for_same_vip() {
        let actions = vec![update_service(1), register(vip_v4(1), 8080)];
        assert!(
            validate_reconcile_output(&actions).is_ok(),
            "XDP + cgroup writes for the same VIP are the ADR-0053 § 4 dual-path, not a conflict"
        );
    }

    /// ADR-0053 § 4 dual-path with distinct proto on one VIP+port —
    /// an XDP write at tcp/53 and a cgroup write at udp/53 for the same
    /// VIP are distinct slots on disjoint routes; accepted.
    #[test]
    fn validate_accepts_xdp_and_cgroup_distinct_proto_same_vip_port() {
        use overdrive_core::dataplane::backend_key::Proto;
        let actions =
            vec![update_service_pp(1, Proto::Tcp, 53), register_proto(vip_v4(1), 53, Proto::Udp)];
        assert!(
            validate_reconcile_output(&actions).is_ok(),
            "XDP tcp/53 + cgroup udp/53 on one VIP are disjoint-route slots, not a conflict"
        );
    }

    /// Empty action vec is trivially valid — no writes, no
    /// conflicts.
    #[test]
    fn validate_accepts_empty_vec() {
        assert!(validate_reconcile_output(&[]).is_ok());
    }
}
