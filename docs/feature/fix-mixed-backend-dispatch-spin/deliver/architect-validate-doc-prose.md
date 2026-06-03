# Architect-supplied verbatim module-doc for `validate.rs` (Fix A1)

Apply VERBATIM during the code fix. Replaces the doc-comment header
(`//! Reconcile-output invariant validator.` through the end of the
"Ordered-collection choice" paragraph, ~lines 1-63). The crafter owns the
LOGIC change (delete cross-route trackers/arms, widen same-route trackers to
`(vip, port, proto)` / `(vip, vip_port, proto)`); this is the doc only.

```rust
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
//! (the conflicting route + the shared `(vip, port, proto)` slot) so
//! downstream `matches!` branches do not have to parse `Display`
//! strings.
//!
//! Per `.claude/rules/development.md` § "Ordered-collection choice":
//! the tracking sets are [`BTreeSet`]s so violation reproducibility
//! is deterministic across runs — the FIRST conflicting pair surfaced
//! does not depend on `HashSet` iteration order.
```

## Note on Fix C (observation row)

The "Fail-safe semantics" paragraph above says "logs a structured
`reconciler.output.invariant_violation` tracing event." Per user decision
2026-06-03 (escalate to a queryable observation row), Fix C ADDS a queryable
observation row alongside the tracing event. When Fix C lands, extend this
paragraph to name the observation row as the queryable surface (the tracing
event stays as the supplemental human signal — Kubernetes Events model:
best-effort human signal distinct from the machine control signal).
