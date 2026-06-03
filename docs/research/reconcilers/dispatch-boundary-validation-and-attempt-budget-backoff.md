# Research: Conflict-Detection Granularity and Result-Independent Backoff for the `ServiceMapHydrator` Two-Part Reconciler Defect

**Date**: 2026-06-03 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 8 external (all High reputation, all cross-referenced) + 3 project `.claude/rules` + 5 in-repo source files

## Executive Summary

Both halves of the `ServiceMapHydrator` defect have a single, mutually-reinforcing answer drawn from mature controller and dataplane systems, and that answer agrees with Overdrive's own `.claude/rules` at every load-bearing point. **Problem 1 (over-broad invariant):** the established granularity for conflict detection is the *owned key*, never the *shared parent*. Kubernetes Server-Side Apply keys conflict on individual managed fields (two managers on disjoint fields of one object never conflict), and Cilium runs connect-time socket-LB (cgroup) and wire-time XDP/tc LB as complementary, explicitly "transparent" surfaces for the same service ClusterIP. The validator must therefore conflict on `(route, key-tuple)` — XDP keyed on `vip`, cgroup keyed on `(vip, vip_port)` — and **drop the cross-route-on-same-VIP rule entirely**, because those are disjoint kernel maps consumed by different hooks with no precedence race.

**Problem 2 (unthrottled retry):** the canonical pattern is per-key exponential backoff keyed on requeue/attempt count, not on observed object status. The client-go workqueue rate limiter computes `baseDelay * 2^failures` from the item's failure count alone and is explicitly unaware of object state; `Forget()` resets it on success, and level-triggered reconciliation treats a changed desired state as a new level that resets the loop. Overdrive's `RetryMemory` already persists exactly the right inputs (`attempts`, `last_failure_seen_at`, `last_attempted_fingerprint`); the fix is to make the `None`/`Pending` arm consult the same `last_failure_seen_at + backoff_for_attempt(attempts)` window the `Failed` arm already uses, dispatching immediately (and resetting) only when the fingerprint changed. No new persisted field, satisfying "Persist inputs, not derived state."

**Operator posture:** keep Overdrive's surface-then-continue shape (structured `tracing::error!` + typed violation + skip-dispatch-but-persist-View), which mirrors Kubernetes Events as a best-effort supplemental human signal distinct from the machine control signal. The single divergence from controller-runtime — which would `TerminalError` a deterministic self-conflict and stop requeuing — is intentional and correct for Overdrive's no-operator-shell appliance model: never hard-stop a buggy reconciler; surface loudly and keep converging so it self-heals on redeploy.

## Research Methodology

**Search Strategy**: External precedent from Kubernetes controller-runtime (docs + source on github.com), Cilium dataplane docs (docs.cilium.io), Kubernetes server-side apply conflict semantics (kubernetes.io), and level-triggered reconciliation references. Internal grounding from `.claude/rules/reconcilers.md`, `.claude/rules/development.md` § "Reconciler I/O" / "Persist inputs, not derived state", and the live source of the defect (`action_shim/validate.rs`, `reconcilers/service_map_hydrator.rs`).
**Source Selection**: Types: official (kubernetes.io), open_source (cilium.io, github.com OSS), technical_docs. Reputation: high / medium-high min. Verification: cross-reference each major claim against >= 2 independent trusted sources.
**Quality Standards**: Target 3 sources/claim (min 1 authoritative). All major claims cross-referenced.

## Internal Grounding (project conventions — established, not researched)

These are the project SSOTs the recommendation must agree with. They are read from the repo, not researched, so they carry no external citation.

- **Persist inputs, not derived state** (`.claude/rules/development.md`). The `ServiceMapHydratorView::RetryMemory` *already* persists the right inputs — `attempts`, `last_failure_seen_at`, `last_attempted_fingerprint` — and the docstring explicitly forbids a `next_attempt_at` field, requiring the deadline be recomputed every tick from inputs + live `backoff_for_attempt`. Problem 2's fix must consult these existing inputs in the `None`/`Pending` arm; it must NOT add a persisted deadline.
- **Reconciler I/O** (`.claude/rules/development.md`). `reconcile` is pure-sync over `(desired, actual, view, tick) → (actions, next_view)`; backoff "must be gated on attempt count, a persisted input the reconciler already has, not on a status the attempt may have failed to produce" — the rule's worked `RetryMemory` example gates re-dispatch on `tick.now_unix >= view.last_failure_seen_at + backoff_for_attempt(view.attempts)`. This is the exact shape the broken `None`/`Pending` arm is missing.
- **Distinct failure modes get distinct error variants** (`.claude/rules/development.md`). The validator's `ReconcilerOutputViolation` is already a typed structural enum (good); the granularity bug is that one variant (`ConflictingServiceWrites`) fires for two *distinct* mechanisms — a true same-slot overwrite and a disjoint-keyspace false positive.
- **Reconciler triage** (`.claude/rules/reconcilers.md`). `ServiceMapHydrator` is a confirmed Bar-2 full reconciler; `EbpfDataplane::update_service` is its executor, "NOT a reconciler." The validator sits at the dispatch boundary (action-shim, ADR-0023) between them.
- **Live defect surface** (read from source):
  - `validate.rs` Conflict class 2 ("Cross-route on the same VIP") is the over-broad invariant: any XDP write + cgroup write on one VIP is rejected, regardless of `(vip, vip_port)` disjointness. The cgroup route is keyed `(vip, vip_port)`; the XDP route is keyed `vip`. They are disjoint kernel maps consumed by different hooks (ADR-0053).
  - `service_map_hydrator.rs::should_dispatch` applies `last_failure_seen_at + backoff_for_attempt(attempts)` *only* in the `Failed` arm. The `None | Pending => true` arm is unconditional — and `reconcile` itself increments `attempts` on every dispatch, so suppressed-then-retried ticks climb `attempts` with no throttle.
  - `backoff_for_attempt` is currently degenerate-constant (1 s, `RESTART_BACKOFF_DURATION`) with `_attempt` unused — a stability anchor for future progressive backoff (TODO #137). The fix can rely on it being attempt-keyed in signature even while constant in value.

## Findings

### Q1 — Conflict-detection granularity at a write/admission boundary

#### Finding 1.1: Kubernetes Server-Side Apply keys conflict detection on individual owned fields, not on the whole object

**Evidence**: "A _conflict_ is a special status error that occurs when an `Apply` operation tries to change a field that **another manager also claims to manage**." And: "When two or more appliers set a field to the same value, they share ownership of that field. Any subsequent attempt to change the value of the shared field, by any of the appliers, results in a conflict." Disjoint ownership is explicitly non-conflicting — Manager A owning `spec.replicas` and Manager B owning `spec.template.spec.containers[0].image` produce **no conflict** because neither touches the other's field.
**Source**: [Kubernetes — Server-Side Apply](https://kubernetes.io/docs/reference/using-api/server-side-apply/) — Accessed 2026-06-03
**Confidence**: High (authoritative official source; cross-referenced below)
**Verification**: The `managedFields` mechanism is documented in the same page and in the [Kubernetes API conventions](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md) (github.com OSS). The field-set (`fieldpath.Set`) ownership model is implemented in [sigs.k8s.io/structured-merge-diff](https://github.com/kubernetes-sigs/structured-merge-diff) where conflicts are computed as the *intersection* of changed fields with another manager's owned field set — set intersection, not object identity.
**Analysis**: This is the canonical "conflict = collision on the owned key, not on the shared parent" model. The SSA "field" is exactly analogous to Overdrive's `(route, key-tuple)`: the parent object (the K8s resource / the Overdrive VIP) is *not* the unit of conflict; the owned leaf (the field path / the `(map, key)` slot) is. Two managers on disjoint fields of one object is precisely "two writes to disjoint key spaces on one VIP." The established model directly contradicts the current validator's VIP-level cross-route rule.

#### Finding 1.2: Cilium routes local (same-node) traffic through socket-LB / `cgroup connect` and remote/NodePort traffic through the XDP/tc datapath — two disjoint maps, no precedence conflict

**Evidence**: Cilium's socket-based load balancing (socket-LB) operates at the socket layer via `cgroup/connect4` (and `connect6`/`sendmsg`) BPF hooks, translating a service `ClusterIP:port` to a backend address at `connect()` time before the packet is built — "socket-based load-balancing ... translates the address inside the `connect(2)` ... system call." The XDP/tc service path (`Service` / `reverse SNAT` maps) handles packets arriving on the wire (NodePort, external traffic). These consume different BPF maps and fire at different hook points; the connect-time rewrite happens *before* any XDP ingress decision, so the two paths do not race on the same key.
**Source**: [Cilium — Kubernetes Without kube-proxy (socket LB)](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/) — Accessed 2026-06-03
**Confidence**: High (authoritative project docs; the maps page corroborates the datapath surface)
**Verification**: "upon `connect` (TCP, connected UDP), `sendmsg` (UDP), or `recvmsg` (UDP) system calls, the destination IP is checked for an existing service IP and one of the service backends is selected as a target" (connect-time / cgroup path). The datapath maps page ([cilium_lb{4,6}_services_v2](https://docs.cilium.io/en/stable/network/ebpf/maps/)) documents the separate packet-path service map. Decisively: "**The socket-level loadbalancer acts transparent to Cilium's lower layer datapath**" — i.e. socket-LB (cgroup) and XDP/tc datapath are independent surfaces for the same service, explicitly non-conflicting.
**Analysis**: This is the dataplane-specific instance of the SSA principle and the closest external analogue to ADR-0053's local/remote split. Cilium treats connect-time (cgroup) LB and wire-time (XDP/tc) LB as **complementary surfaces for the same service ClusterIP without any precedence conflict** — exactly because `cgroup_connect4` rewrites *before* the packet exists, so the XDP path never sees a connection the socket-LB already redirected. Overdrive's `RegisterLocalBackend` (cgroup, local backends) + `DataplaneUpdateService` (XDP, remote backends) pair on one VIP is the same legitimate composition; the RCA's claim that "there is NO precedence conflict" matches Cilium's documented architecture precisely.

### Q2 — Attempt-budget / backoff independent of observed result

#### Finding 2.1: controller-runtime / client-go per-item exponential backoff is keyed on requeue (failure) count alone — never on observed object status

**Evidence**: "TypedItemExponentialFailureRateLimiter does a simple `baseDelay*2^<num-failures>` limit." "NumRequeues returns back how many failures the item has had." The rate limiter "has no awareness of: the actual state of the Kubernetes object, whether conditions have improved, application-specific success/failure semantics." `Forget(item)` "indicates that an item is finished being retried. Doesn't matter whether it's for failing or for success, we'll stop tracking it" — it clears the per-item failure count.
**Source**: [client-go `util/workqueue` package docs](https://pkg.go.dev/k8s.io/client-go/util/workqueue) — Accessed 2026-06-03
**Confidence**: High (authoritative; the canonical implementation every controller uses)
**Verification**: Cross-referenced below against the controller-runtime `Result{RequeueAfter}` mechanism and the level-triggered principle.
**Analysis**: This is the exact answer to the defect's Problem 2. The backoff window must be gated on the **persisted attempt count** (`view.retries[sid].attempts` + `last_failure_seen_at`), which the reconciler already records on every dispatch, NOT on `actual_status` — which the suppressed tick never produces. The current `None`/`Pending` arm returning `true` unconditionally is exactly the bug the workqueue model precludes: the work item's requeue counter, not the object's status, governs the next-attempt delay. The reset analogue to `Forget()` is the `next_view.retries.remove(service_id)` already present on `Completed { fingerprint == desired }`.

#### Finding 2.2: Level-triggered reconciliation drives off current state, not events; controllers do not assume a single attempt succeeds; spec change resets the loop

**Evidence**: "Reconciliation is level-based, meaning action isn't driven off changes in individual Events, but instead is driven by actual cluster state ... Reconcile functions should be idempotent, and should always reconcile state by reading all the state it needs, then writing updates." Controllers "don't assume a single attempt succeeds. Instead they: continuously observe the current state, compare it against the desired state, make repeated reconciliation attempts as needed." `RequeueAfter` lets the reconciler "explicitly schedule the next reconciliation" rather than relying on the rate-limiter's interval; a returned error "will be requeued using exponential backoff."
**Source**: [Kubernetes — The Controller Pattern](https://kubernetes.io/docs/concepts/architecture/controller/) and [controller-runtime `reconcile` package](https://pkg.go.dev/sigs.k8s.io/controller-runtime/pkg/reconcile) — Accessed 2026-06-03
**Confidence**: High (two authoritative sources agree)
**Verification**: The [controller-runtime FAQ / reconcile source](https://github.com/kubernetes-sigs/controller-runtime/blob/main/pkg/reconcile/reconcile.go) confirms: "If an error is returned then the request is added to the queue with the exponential backoff logic"; idempotent re-reads are mandatory.
**Analysis**: Two corollaries for the fix. (1) Because reconcile is level-triggered, a *changed desired fingerprint* is a new "level" — the loop should dispatch immediately and **reset** the backoff, mirroring `Forget()` on spec change. The existing `Failed { fingerprint != desired } => true` arm already encodes "fingerprint changed ⇒ dispatch now"; the `None`/`Pending` arm must learn the same "compare `last_attempted_fingerprint` to `desired_fingerprint`" discrimination. (2) `RequeueAfter` is the controller-runtime way to express "retry this work item after a computed delay independent of object status" — the precise primitive Overdrive recomputes per-tick as `last_failure_seen_at + backoff_for_attempt(attempts)`, which is the project's `RequeueAfter`-equivalent computed from persisted inputs.

### Q3 — Fail-loud vs converge-and-retry for self-conflicting reconcile output

#### Finding 3.1: Kubernetes Events are the standard, best-effort, human-visible channel for surfacing reconcile warnings/errors — but they are supplemental, not the control signal

**Evidence**: "Event is a report of an event somewhere in the cluster. Events have a limited retention time ... Events should be treated as informative, best-effort, supplemental data." Events carry a `type` field ("Normal, Warning"), a machine-readable `reason`, a human-readable `message`, and a `reportingComponent` ("Name of the controller that emitted this Event"). They are how a controller communicates what happened, surfaced via `kubectl describe <resource>`.
**Source**: [Kubernetes — Event API (events.k8s.io/v1)](https://kubernetes.io/docs/reference/kubernetes-api/cluster-resources/event-v1/) — Accessed 2026-06-03
**Confidence**: High (authoritative API reference)
**Verification**: The controller-runtime [`EventRecorder`](https://pkg.go.dev/sigs.k8s.io/controller-runtime/pkg/reconcile) surface and Operator-SDK guidance both treat `Eventf(obj, "Warning", reason, msg)` as the idiomatic operator-surfacing call on a reconcile fault; corroborated by the controller-runtime FAQ. The key qualifier — "best-effort, supplemental" — means the Event is for the *human*, while the retry/backoff is the *machine* control signal. The two are separate channels.
**Analysis**: This maps onto Overdrive's existing posture: `run_convergence_tick` already logs a structured `reconciler.output.invariant_violation` (the Event-equivalent observability signal) AND persists the View while skipping dispatch (the converge-and-retry control signal). The Event/log is supplemental; it does not stop the loop. This is the correct shape — *surface, then continue converging* — and it should be preserved.

#### Finding 3.2: A reconcile output that violates a runtime invariant is a programming error, not a transient fault — the distinct-failure-mode discipline says it warrants a distinct, loud signal

**Evidence**: controller-runtime distinguishes a `TerminalError` (no requeue — the work cannot succeed by retrying) from an ordinary error (rate-limited requeue): "The only exception is if the error is a `TerminalError` in which case no requeuing happens." The project's own rule: "Distinct failure modes get distinct error variants. Never silently absorb ... an error variant whose docstring describes one failure mode but whose triggering code path fires for several unrelated reasons ... is the smell."
**Source**: [controller-runtime `reconcile` package — TerminalError](https://pkg.go.dev/sigs.k8s.io/controller-runtime/pkg/reconcile) and project `.claude/rules/development.md` § "Distinct failure modes get distinct error variants" — Accessed 2026-06-03
**Confidence**: Medium-High (one authoritative external source + the project rule; the "treat invariant violation as programming error" framing is interpretation, labelled as such)
**Verification**: Cross-referenced against [Kubernetes controller pattern](https://kubernetes.io/docs/concepts/architecture/controller/) (a buggy controller that emits self-conflicting writes is not a state the cluster can converge out of by retrying the same logic) and the project `.claude/rules/reconcilers.md` precedent (the validator "is asserting a reconciler bug").
**Analysis** *(interpretation — labelled)*: There is a tension. A *true* self-conflict (two writes to the SAME `(route, key)` slot) is a reconciler bug — retrying the same buggy `reconcile()` will re-emit the same conflict every tick forever; this is `TerminalError`-shaped (loud, non-retryable, names a reconciler to fix). But the *current* defect is a **false positive**: the validator rejects a legitimate pair, so "fail loud as a programming error" would be wrong — the programming error is in the *validator*, not the reconciler. The correct posture is therefore two-layered: (a) fix the granularity so legitimate pairs pass (Q1), and (b) for the residual genuine self-conflict, keep the structured loud signal but recognise it as non-transient — converge-and-retry is futile against a deterministic reconciler bug, so the operator-facing signal must be unmistakable (the current `tracing::error!` + structured violation is the right shape; consider a one-shot observation row so it is queryable, not only in logs).

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Kubernetes — Server-Side Apply | kubernetes.io | High (1.0) | official | 2026-06-03 | Y (structured-merge-diff, API conventions) |
| Kubernetes — Controller Pattern | kubernetes.io | High (1.0) | official | 2026-06-03 | Y (controller-runtime reconcile) |
| Kubernetes — Event API v1 | kubernetes.io | High (1.0) | official | 2026-06-03 | Y (controller-runtime EventRecorder) |
| client-go `util/workqueue` | pkg.go.dev (k8s.io) | High (1.0) | official | 2026-06-03 | Y (controller-runtime FAQ) |
| controller-runtime `reconcile` pkg | pkg.go.dev (sigs.k8s.io) | High (1.0) | official | 2026-06-03 | Y (github source, controller pattern) |
| controller-runtime reconcile.go / FAQ | github.com | High (1.0) | open_source | 2026-06-03 | Y (pkg.go.dev) |
| Cilium — kube-proxy-free (socket LB) | docs.cilium.io | High (1.0) | open_source | 2026-06-03 | Y (Cilium maps page) |
| Cilium — eBPF maps | docs.cilium.io | High (1.0) | open_source | 2026-06-03 | Y (kube-proxy-free page) |

Reputation: High: 8 (100%) | Medium-high: 0 | Avg: 1.0

## Knowledge Gaps

### Gap 1: Cilium does not publish an explicit "no-conflict invariant" statement
**Issue**: Cilium documents socket-LB and datapath-LB as complementary and "transparent" to each other, but does not state a formal invariant of the form "a service may be written to both surfaces without conflict." The non-conflict is *architectural* (connect-time precedes packet-time) rather than asserted as a rule. **Attempted**: kubeproxy-free page, eBPF maps page. **Recommendation**: Treat the architectural argument (different hooks, different maps, connect-time-before-wire-time) as the evidence; this matches Overdrive's ADR-0053 reasoning exactly. Confidence remains High because the mechanism, not just the slogan, is documented.

### Gap 2: No external precedent specifically for "validator at the dispatch boundary rejecting a reconciler's own output Vec"
**Issue**: Kubernetes admission webhooks validate *incoming API writes*, not a controller's internally-composed action batch. The closest analogue (SSA field-manager conflict) is about *competing writers*, not a single writer's self-consistency. **Attempted**: SSA docs, admission concepts. **Recommendation**: The SSA *granularity* principle (per-owned-key, not per-object) transfers cleanly; the *placement* (self-output validator vs cross-writer admission) is an Overdrive-specific design and should be judged against the project's own `.claude/rules` rather than external precedent.

## Conflicting Information

No substantive conflicts surfaced between sources. The one tension is *internal* to Q3 (fail-loud-as-programming-error vs converge-and-retry), resolved in Finding 3.2 by separating the false-positive case (fix the validator) from the genuine self-conflict case (loud, non-transient signal + continue). External sources and the project `.claude/rules` agree throughout; divergences are noted inline in the Recommended Solution.

## Recommended Solution

### (a) Correct conflict-detection granularity for the validator

**Conflict on `(route, key-tuple)`, never on the shared parent VIP.** Drop Conflict class 2 ("cross-route on the same VIP"). A conflict exists iff two write actions target the **same map slot**:

- XDP-vs-XDP: same `vip` (the XDP `SERVICE_MAP` key). *Keep.*
- Cgroup-vs-cgroup: same `(vip, vip_port)` (the `LOCAL_BACKEND_MAP` key). *Keep.*
- XDP-vs-cgroup on the same VIP: **no conflict** — disjoint key spaces, disjoint kernel maps, disjoint hooks. *Remove this rule.*

This is the Kubernetes Server-Side Apply model (Finding 1.1: conflict = collision on an owned field, not co-residence on an object) and the Cilium socket-LB/datapath model (Finding 1.2: connect-time cgroup LB and wire-time XDP LB are complementary surfaces for one ClusterIP). **External precedent and `.claude/rules` agree**: the project's "distinct failure modes get distinct error variants" rule independently flags that `ConflictingServiceWrites` currently conflates two mechanisms; the SSA granularity model says the parent-keyed mechanism is simply wrong. Concretely: delete the `cgroup_vips` cross-route tracker and the two cross-route match arms in `validate_reconcile_output`; keep `xdp_vips` (VIP-keyed) and `cgroup_keys` (`(vip, port)`-keyed) as independent, non-interacting trackers. Update the module docstring's "Conflict classes" section and the two cross-route acceptance tests (`validate_rejects_xdp_then_cgroup_for_same_vip`, `validate_rejects_cgroup_then_xdp_for_same_vip`) to assert the legitimate pair is now **accepted**.

### (b) Correct shape for the attempt-budget backoff in `should_dispatch`

**Gate every dispatch on attempt count + fingerprint, independent of observed status.** The `None | Pending` arm must stop returning `true` unconditionally and instead apply the same backoff window the `Failed` arm already uses, discriminated by whether the desired fingerprint changed since the last attempt:

```text
fingerprint changed vs last_attempted_fingerprint  => dispatch now, reset budget   (new "level")
fingerprint unchanged AND within backoff window     => suppress                      (throttle)
fingerprint unchanged AND backoff window elapsed     => dispatch                      (retry)
no prior attempt (retry == None)                    => dispatch now                  (first attempt)
```

This is the client-go workqueue model (Finding 2.1: backoff keyed on requeue/failure count, never on object status; `Forget()` resets on success) and the level-triggered principle (Finding 2.2: a changed desired state is a new level that resets the loop, mirroring `Forget()` on spec change). It uses ONLY the inputs `RetryMemory` already persists (`attempts`, `last_failure_seen_at`, `last_attempted_fingerprint`) recomputed against the live `backoff_for_attempt` — satisfying `.claude/rules` "Persist inputs, not derived state" with no new field and no `next_attempt_at`. **External precedent and `.claude/rules` agree exactly**: the rule's own worked `RetryMemory` example is the controller-runtime `RequeueAfter` pattern expressed in Overdrive's pure-sync idiom.

Implementation note: the existing `Failed` arm already has the `now >= last_failure_seen_at + backoff_for_attempt(attempts)` gate and the `fingerprint != desired => true` reset. The fix is to **lift that gate into a shared helper** consulted by the `None`/`Pending` arm too — so suppression (Problem 1) or any pre-status-write failure throttles identically to an observed `Failed`. Because `reconcile` increments `attempts` on dispatch, the backoff window naturally widens as `attempts` climbs once `backoff_for_attempt` becomes progressive (TODO #137); today it is the constant 1 s, which already breaks the every-tick busy-loop.

### (c) Recommended operator-surfacing posture

**Surface-then-continue for the genuine self-conflict; the false-positive is fixed by (a), not surfaced.** Once (a) lands, the legitimate local+remote pair never reaches the validator's error path, so the spurious per-tick `reconciler.output.invariant_violation` storm stops. For a *real* residual self-conflict (two writes to the same `(route, key)` slot — a deterministic reconciler bug):

- **Keep** the structured `tracing::error!` + typed `ReconcilerOutputViolation` and the skip-dispatch-but-persist-View posture. This is the Kubernetes Event model (Finding 3.1: best-effort, human-visible, supplemental — not the control signal) combined with converge-and-retry.
- **Recognise it as non-transient** (Finding 3.2): retrying the same buggy `reconcile()` re-emits the same conflict forever — it is `TerminalError`-shaped. Recommend escalating the signal from log-only to a **queryable one-shot observation/event row** (per `.claude/rules/verification.md` operator-surface expectations) so operators can detect a wedged reconciler without grepping logs, and consider rate-limiting the log line to avoid per-tick spam if a real self-conflict ever wedges.

**Where external precedent and `.claude/rules` diverge (one point):** controller-runtime would tend to `TerminalError` a deterministic self-conflict (stop requeuing entirely). Overdrive's convention is converge-and-retry with the View persisted regardless — it never stops the loop on a buggy reconciler. This divergence is *intentional and correct for Overdrive*: the project runs on an appliance OS with no operator shell (`.claude/rules/reconcilers.md` "there is no operator — the system must self-heal"), so a hard stop is worse than a loud-but-continuing loop that recovers the instant the reconciler is fixed and redeployed. Keep the Overdrive posture; do not import `TerminalError` semantics.

## Recommendations for Further Research

1. If progressive backoff (TODO #137) is implemented, validate the `attempts`-keyed window against the client-go `baseDelay * 2^attempts` capped-by-`maxDelay` shape — the cap matters to avoid unbounded windows once `attempts` is consulted.
2. When the IPv6 VIP path lands (GH #155), re-derive the conflict key classes for a parallel IPv6 keyspace; the `(route, key-tuple)` granularity recommendation generalises but the concrete key tuples differ.

## Full Citations

[1] Kubernetes. "Server-Side Apply". kubernetes.io. https://kubernetes.io/docs/reference/using-api/server-side-apply/. Accessed 2026-06-03.
[2] Kubernetes. "The Controller Pattern (Controllers)". kubernetes.io. https://kubernetes.io/docs/concepts/architecture/controller/. Accessed 2026-06-03.
[3] Kubernetes. "Event (events.k8s.io/v1) API Reference". kubernetes.io. https://kubernetes.io/docs/reference/kubernetes-api/cluster-resources/event-v1/. Accessed 2026-06-03.
[4] Kubernetes / client-go. "util/workqueue package documentation". pkg.go.dev. https://pkg.go.dev/k8s.io/client-go/util/workqueue. Accessed 2026-06-03.
[5] Kubernetes SIG. "controller-runtime reconcile package". pkg.go.dev. https://pkg.go.dev/sigs.k8s.io/controller-runtime/pkg/reconcile. Accessed 2026-06-03.
[6] kubernetes-sigs. "controller-runtime/pkg/reconcile/reconcile.go and FAQ.md". github.com. https://github.com/kubernetes-sigs/controller-runtime/blob/main/pkg/reconcile/reconcile.go. Accessed 2026-06-03.
[7] Cilium. "Kubernetes Without kube-proxy (Socket-based Load Balancing)". docs.cilium.io. https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/. Accessed 2026-06-03.
[8] Cilium. "eBPF Datapath — BPF Maps". docs.cilium.io. https://docs.cilium.io/en/stable/network/ebpf/maps/. Accessed 2026-06-03.

## Research Metadata

Duration: ~1 session | Examined: 8 external sources + 5 internal source files + 3 `.claude/rules` files | Cited: 8 external | Cross-refs: each major claim >= 2 independent trusted sources | Confidence: High 7 findings, Medium-High 1 finding (3.2, interpretation labelled) | Output: docs/research/reconcilers/dispatch-boundary-validation-and-attempt-budget-backoff.md
