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
//! co-residence on the object). See ADR-0053 revision
//! 2026-06-03 ("dispatch-boundary conflict granularity is
//! `(route, key-tuple)`") and
//! `docs/research/reconcilers/dispatch-boundary-validation-and-attempt-budget-backoff.md`.
//!
//! Two write actions conflict only when they target the same XDP
//! `SERVICE_MAP` `(vip, port, proto)` slot. The second write silently
//! overwrites the first; a reconciler emitting both in one tick is
//! non-deterministic in its intent. Distinct ports (tcp/8080 + tcp/8081)
//! and distinct proto (tcp/53 + udp/53) are distinct slots and do NOT
//! conflict.
//!
//! # Provenance
//!
//! The Phase-16 D11 finding
//! (`docs/evolution/2026-05-23-backend-discovery-bridge-service-reachability.md`
//! § "Reconcile-output invariant at the action_shim boundary") governs
//! SAME-CLASS write conflicts only — two writes to one slot with
//! conflicting contents, a genuine same-slot overwrite.
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

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{AllocationId, ServiceId};
use overdrive_core::reconcilers::Action;

/// Route the action would take through the dataplane port boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteRoute {
    /// `Action::DataplaneUpdateService` — XDP `SERVICE_MAP` / Maglev
    /// path, keyed on `(vip, port, proto)` at the kernel-side program
    /// (step 02-01 widened from VIP-only).
    Xdp,
}

/// The allocation action a `ReclaimAllocationNetwork` conflicted with in one
/// `reconcile()` return: the other action names the same `alloc_id`.
///
/// GH #295 D-295-R11 (FD § "[REF] Lifecycle action — row-neutral reclaim (D-295-R11) — ACCEPTED 2026-09-24" (the reclaim action, shim arm, and validator rule)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReclaimConflictAction {
    /// `Action::StartAllocation` for the same `alloc_id`.
    StartAllocation,
    /// `Action::RestartAllocation` whose `alloc_id` (the predecessor) is the
    /// same allocation.
    RestartAllocation,
    /// `Action::StopAllocation` for the same `alloc_id`.
    StopAllocation,
    /// `Action::FinalizeFailed` for the same `alloc_id`.
    FinalizeFailed,
}

/// Violation surfaced by [`validate_reconcile_output`]. Per
/// `.claude/rules/development.md` § Errors / pass-through: typed
/// structural fields, not a flat string. New inter-action invariants land
/// as additional variants on this enum rather than as separate error types
/// so the dispatch-boundary caller can `matches!` on the structured cause
/// without re-parsing `Display`.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ReconcilerOutputViolation {
    /// Two write actions in the same `reconcile()` return target the
    /// same `(route, vip, port, proto)` map slot. The only conflict class
    /// is XDP-vs-XDP (a shared `(vip, port, proto)` slot).
    /// `first_route` and `second_route` are therefore always equal; both
    /// are retained so the operator-visible tracing event names exactly
    /// which pair conflicted and so a future cross-class invariant can
    /// populate them distinctly without a variant churn.
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
    /// A `ReclaimAllocationNetwork` and another allocation action name the
    /// same allocation in one `reconcile()` return (GH #295 D-295-R11).
    #[error(
        "conflicting allocation reclaim at alloc={alloc_id}: {other:?} names the same allocation"
    )]
    ConflictingAllocationReclaim {
        /// The allocation both actions name.
        alloc_id: AllocationId,
        /// The action that names it beside the reclaim.
        other: ReclaimConflictAction,
    },
}

// Returns the action's allocation key and conflict category for the R11 pair rule.
fn reclaim_owner(action: &Action) -> Option<(&AllocationId, ReclaimConflictAction)> {
    match action {
        Action::StartAllocation { alloc_id, .. } => {
            Some((alloc_id, ReclaimConflictAction::StartAllocation))
        }
        Action::RestartAllocation { alloc_id, .. } => {
            Some((alloc_id, ReclaimConflictAction::RestartAllocation))
        }
        Action::StopAllocation { alloc_id, .. } => {
            Some((alloc_id, ReclaimConflictAction::StopAllocation))
        }
        Action::FinalizeFailed { alloc_id, .. } => {
            Some((alloc_id, ReclaimConflictAction::FinalizeFailed))
        }
        _ => None,
    }
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
/// slot. Returns
/// [`ReconcilerOutputViolation::ConflictingAllocationReclaim`] when a
/// `ReclaimAllocationNetwork` and one of `StartAllocation`,
/// `RestartAllocation`, `StopAllocation`, or `FinalizeFailed` name the
/// same allocation.
pub fn validate_reconcile_output(actions: &[Action]) -> Result<(), ReconcilerOutputViolation> {
    // BTreeSet per `.claude/rules/development.md` § "Ordered-collection
    // choice" — error reproducibility requires deterministic
    // first-conflict surfacing across seeds; the structural defense
    // against `HashSet`'s `RandomState` iteration nondeterminism
    // applies equally to set-shaped trackers.
    //
    // `xdp_keys` holds the SERVICE_MAP outer-key slots for XDP-vs-XDP
    // same-slot detection. Two XDP writes conflict only when ALL THREE of
    // `(vip, port, proto)` match. Distinct ports (tcp/8080 + tcp/8081) and
    // distinct proto (tcp/53 + udp/53) are distinct slots → no conflict.
    let mut xdp_keys: BTreeSet<(Ipv4Addr, u16, Proto)> = BTreeSet::new();
    let mut reclaim_keys = BTreeSet::new();
    let mut reclaim_owners = BTreeMap::new();

    for action in actions {
        if let Action::ReclaimAllocationNetwork { alloc_id } = action {
            if let Some(other) = reclaim_owners.get(alloc_id) {
                return Err(ReconcilerOutputViolation::ConflictingAllocationReclaim {
                    alloc_id: alloc_id.clone(),
                    other: *other,
                });
            }
            reclaim_keys.insert(alloc_id.clone());
        } else if let Some((alloc_id, other)) = reclaim_owner(action) {
            if reclaim_keys.contains(alloc_id) {
                return Err(ReconcilerOutputViolation::ConflictingAllocationReclaim {
                    alloc_id: alloc_id.clone(),
                    other,
                });
            }
            reclaim_owners.entry(alloc_id.clone()).or_insert(other);
        }

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
/// IPv6 VIPs (`ServiceVip::try_as_ipv4() == None`) are structurally unreachable
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
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use overdrive_core::dataplane::backend_key::Proto;
    use overdrive_core::id::{ContentHash, CorrelationKey, ServiceId, ServiceVip};
    use overdrive_core::reconcilers::Action;

    use super::{ReconcilerOutputViolation, WriteRoute, validate_reconcile_output};
    use overdrive_testing::placement;

    placement!(host_pure);

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
    /// actions returns `Ok(())`: XDP writes for distinct VIPs, plus Noops.
    #[test]
    fn validate_accepts_distinct_writes_and_noops() {
        let actions = vec![
            Action::Noop,
            update_service(1), // XDP at 10.96.0.1
            update_service(2), // XDP at 10.96.0.2
            Action::Noop,
        ];
        assert!(validate_reconcile_output(&actions).is_ok());
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

    /// Empty action vec is trivially valid — no writes, no
    /// conflicts.
    #[test]
    fn validate_accepts_empty_vec() {
        assert!(validate_reconcile_output(&[]).is_ok());
    }

    fn alloc(name: &str) -> overdrive_core::id::AllocationId {
        overdrive_core::id::AllocationId::new(name).expect("allocation id")
    }

    fn allocation_spec(name: &str) -> overdrive_core::traits::driver::AllocationSpec {
        use overdrive_core::traits::driver::{DriverPayload, Resources, VmPayload};
        overdrive_core::traits::driver::AllocationSpec {
            declared_listeners: Vec::new(),
            alloc: alloc(name),
            identity: overdrive_core::SpiffeId::new(&format!(
                "spiffe://overdrive.local/workload/nd295/alloc/{name}"
            ))
            .expect("SPIFFE ID"),
            driver: DriverPayload::Vm(VmPayload {
                command: "/bin/true".to_owned(),
                args: Vec::new(),
                kernel: std::path::PathBuf::from("/nd295/kernel"),
                rootfs: std::path::PathBuf::from("/nd295/rootfs.ext4"),
            }),
            resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            probe_descriptors: Vec::new(),
            transport: None,
        }
    }

    /// The four actions that already own an allocation's cleanup in one
    /// evaluation (FD § "[REF] Lifecycle action — row-neutral reclaim (D-295-R11) — ACCEPTED 2026-09-24" (the validator)), each naming `name`.
    fn owning_actions(name: &str) -> [(&'static str, Action); 4] {
        [
            (
                "StartAllocation",
                Action::StartAllocation {
                    alloc_id: alloc(name),
                    workload_id: overdrive_core::id::WorkloadId::new("nd295-workload")
                        .expect("workload id"),
                    node_id: overdrive_core::id::NodeId::new("nd295-node").expect("node id"),
                    spec: allocation_spec(name),
                    kind: overdrive_core::aggregate::WorkloadKind::Service,
                },
            ),
            (
                "RestartAllocation",
                Action::RestartAllocation {
                    alloc_id: alloc(name),
                    spec: allocation_spec(&format!("{name}-successor")),
                    kind: overdrive_core::aggregate::WorkloadKind::Service,
                },
            ),
            ("StopAllocation", Action::StopAllocation { alloc_id: alloc(name), terminal: None }),
            ("FinalizeFailed", Action::FinalizeFailed { alloc_id: alloc(name), terminal: None }),
        ]
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-55 — Every leased, unowned, finished allocation is reclaimed from every reconcile path
    /// CONTRACT_SHAPE: pure-function.
    ///
    /// A reclaim may never name an allocation another action names in the
    /// same evaluation: `ReclaimAllocationNetwork` beside `StartAllocation`,
    /// `RestartAllocation` (its predecessor), `StopAllocation`, or
    /// `FinalizeFailed` for the same `alloc_id` is rejected in either order;
    /// beside the same actions for another allocation, alone, or beside a
    /// reclaim of another allocation, it is accepted (FD § "[REF] Lifecycle action — row-neutral reclaim (D-295-R11) — ACCEPTED 2026-09-24" (the validator)).
    #[test]
    fn a_reclaim_beside_another_action_for_the_same_allocation_is_rejected() {
        use super::ReclaimConflictAction;
        let reclaim = |name: &str| Action::ReclaimAllocationNetwork { alloc_id: alloc(name) };
        assert!(
            validate_reconcile_output(&[reclaim("nd295-a")]).is_ok(),
            "a reclaim alone is valid"
        );
        assert!(
            validate_reconcile_output(&[reclaim("nd295-a"), reclaim("nd295-b")]).is_ok(),
            "reclaims of distinct allocations are valid together"
        );
        let expected_conflict = |kind: &str| match kind {
            "StartAllocation" => ReclaimConflictAction::StartAllocation,
            "RestartAllocation" => ReclaimConflictAction::RestartAllocation,
            "StopAllocation" => ReclaimConflictAction::StopAllocation,
            "FinalizeFailed" => ReclaimConflictAction::FinalizeFailed,
            other => panic!("unexpected owning action kind {other}"),
        };
        for (kind, owning) in owning_actions("nd295-a") {
            for actions in
                [vec![reclaim("nd295-a"), owning.clone()], vec![owning.clone(), reclaim("nd295-a")]]
            {
                match validate_reconcile_output(&actions) {
                    Err(ReconcilerOutputViolation::ConflictingAllocationReclaim {
                        alloc_id,
                        other,
                    }) => {
                        assert_eq!(
                            alloc_id,
                            alloc("nd295-a"),
                            "the conflict names the shared allocation, not the {kind} successor"
                        );
                        assert_eq!(
                            other,
                            expected_conflict(kind),
                            "the conflict names {kind} as the action beside the reclaim, in \
                             either emission order"
                        );
                    }
                    other => panic!(
                        "a reclaim beside {kind} for the same allocation must be \
                         ConflictingAllocationReclaim, got {other:?}"
                    ),
                }
            }
            for actions in
                [vec![reclaim("nd295-b"), owning.clone()], vec![owning.clone(), reclaim("nd295-b")]]
            {
                assert!(
                    validate_reconcile_output(&actions).is_ok(),
                    "a reclaim beside {kind} for another allocation is valid"
                );
            }
        }
    }
}
