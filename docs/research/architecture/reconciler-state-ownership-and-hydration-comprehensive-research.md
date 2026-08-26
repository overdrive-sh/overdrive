# Research: Reconciler / Controller State Ownership and Hydration — Private Per-Reconciler Memory vs Shared Observed State, Cross-Reconciler Dependencies, Restart-Budget Authority, and Hydration Ownership

**Date**: 2026-08-25 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 22 external (avg reputation ≈ 0.945) + in-repo grounding

> Scope note: this document is EVIDENCE for an upcoming DESIGN decision on the
> Overdrive reconciler primitive. It compares the Overdrive codebase's current
> shape (ADR-0035/0036/0078/0079/0084/0086, `.claude/rules/reconcilers.md`,
> `development.md`) against mature reconciler/controller frameworks and
> durable-execution engines. It does **not** prescribe code changes beyond what
> the cited evidence supports.

## Executive Summary

Across the mature field — Kubernetes/kubelet, controller-runtime, kube-rs, Nomad,
Erlang/OTP, systemd, Akka, and (as the contrasting idiom) Temporal — reconcilers
and controllers hold **no durable private per-controller memory**: their state is
the shared API object plus its `status`, backed by an *ephemeral, rebuildable*
informer/reflector cache, and their retry/backoff counters live *in-memory and
ephemeral* in the workqueue rate-limiter or runtime scheduler. Durable private
per-instance state is the *durable-execution / workflow* idiom (Temporal's Event
History; Overdrive's own `Workflow` journal), not the reconciler idiom. Against
that backdrop Overdrive is an outlier in exactly two respects: it keeps a
**durable private `View`**, and it **splits one restart budget across two
independent reconcilers by cause**.

The four decisions, per the evidence:

1. **Does the `View` earn its place?** Qualified yes. It is a genuine outlier (no
   framework has durable private controller memory), but it is a *defensible*
   consequence of Overdrive's pure-sync `reconcile` — the retry *inputs* must be
   persisted somewhere. It becomes an anti-pattern only when it caches a
   projection of the shared observed object rather than holding genuinely private
   retry inputs — which is exactly the ADR-0079 discipline already in the rules.
   Keep it minimal; never let it stand in for the shared object.

2. **Is the `ServiceLifecycle`↔`WorkloadLifecycle` shared restart budget
   idiomatic?** No — this is the one clear divergence to fix. Every examined
   system unifies restart authority under **one owner**; a single owner's budget
   legitimately spans multiple *causes* (the kubelet is the precedent for "one
   budget covering both crash and liveness restarts"). Overdrive should converge
   restart authority onto **`WorkloadLifecycle`** (the natural restart authority,
   already the budget's author), demoting `ServiceLifecycle` to a liveness
   *detector/signal-source* rather than a co-authority reaching into another
   reconciler's private budget. The fix is unifying the *owner*, not splitting the
   *budget*.

3. **Reconciler-owned vs runtime-owned hydration?** The evidence favours
   **reconciler-owned** (ADR-0086's direction) over central runtime hydration
   (ADR-0036): the field norm is "the controller owns its read," and no framework
   uses a central per-controller hydration dispatcher.

4. **The pure-reconcile / impure-hydrate split?** Keep it. Overdrive is stricter
   than the mainstream (which does I/O inside reconcile), but that strictness is
   validated by the ESR/verification lineage the platform already cites — Anvil
   (USENIX OSDI '24) verifies controller liveness precisely by treating reconcile
   as a pure transition function. ADR-0086 preserves the boundary and makes
   hydration DST-injectable, a net gain.

Confidence is **High** (22 external sources, avg reputation ≈ 0.945, every major
claim cross-referenced against ≥2 independent sources, with three official-but
-outside-allowlist canonical sources flagged). The single most decision-relevant
finding is #2: the shared-budget-across-two-reconcilers shape has no counterpart
in the field and should move to single-owner restart authority.

## Research Methodology

**Search Strategy**: Grounding read of in-repo ADRs + rules + concrete code
sites first (to pin what "our decision" actually is), then web research against
the project's trusted-source allowlist (`kubernetes.io`, `github.com` for
kubernetes-sigs/controller-runtime + kube-rs + temporalio + erlang/otp,
`docs.rs`, `developer.hashicorp.com`, `martinfowler.com`, `learn.microsoft.com`,
USENIX/ACM).

**Source Selection**: Types: official docs / OSS project docs+source / recognised
industry experts. Reputation: high / medium-high min. Verification: cross-ref
each major claim across ≥2 independent sources where possible.

**Quality Standards**: Target 3 sources/claim (min 1 authoritative). Avg
reputation target ≥0.80.

---

## Overdrive Grounding — what "our decision" is (in-repo, verified)

Verified by reading the ADRs, `.claude/rules/reconcilers.md` / `development.md`,
and the concrete code sites named in the task. These are the facts each RQ maps
back to.

- **The `View` is durable private per-reconciler memory.** Per ADR-0035, each
  `Reconciler` declares `type View: Serialize + DeserializeOwned + Default +
  Clone + Eq`; the runtime owns persistence via a `ViewStore` (CBOR blobs in
  `<data_dir>/reconcilers/memory.redb`, one redb table per reconciler kind),
  bulk-loaded once at register and served from an in-memory `BTreeMap` per tick
  with fsync-then-memory write-through ordering. The `View` is **NOT** the shared
  observed object — it is memory only the owning reconciler reads
  (`development.md` § "Reconciler I/O").

- **Two distinct restart quantities exist, deliberately different (ADR-0078
  § D3).** (a) `WorkloadLifecycleView.restart_counts: BTreeMap<AllocationId,u32>`
  — reconciler *private memory* (View), increments when the reconciler **emits**
  `RestartAllocation`, counts restart **attempts** (including driver
  `StartRejected`), drives `RESTART_BACKOFF_CEILING`, visible to the reconciler
  only. (b) `AllocStatusRow.restart_count: u32` — *observation* row (rkyv,
  `ObservationStore`), increments when the shim **observes** a `terminal→Running`
  write land, counts restarts that actually **happened**, visible to the
  operator. ADR-0078 § D3's table: "do not source one from the other." Verified:
  `service_lifecycle.rs:790` reads `fact.restart_count` (hydrated from the
  WorkloadLifecycle **View** projection via `restart_status_for_alloc`), NOT the
  observation row.

- **The restart budget is SHARED across two reconcilers by restart CAUSE.**
  `WorkloadLifecycle` owns crash-restart (emits `RestartAllocation` on a terminal
  prior; `workload_lifecycle.rs:786`). `ServiceLifecycle`'s liveness branch owns
  liveness-restart (`service_lifecycle.rs:810`, `RestartReason::LivenessExhausted`)
  and **composes with the same budget**: `service_lifecycle.rs:790` gates on
  `fact.restart_count >= RESTART_BACKOFF_CEILING`. The cross-reconciler read is
  `ReconcilerRuntime::restart_status_for_alloc` (`reconciler_runtime.rs:499`),
  which projects `WorkloadLifecycleView.restart_counts` into `(attempt_index,
  will_restart)` for `ServiceLifecycle`'s actual-hydration. So crash-restart and
  liveness-restart draw on **one pool of `RESTART_BACKOFF_CEILING` attempts**,
  owned by `WorkloadLifecycle`, read by `ServiceLifecycle`.

- **ADR-0086 formalises the cross-reconciler read as a core port.** The
  `restart_status_for_alloc` read becomes the `RestartBudgetView` read-port (one
  of five), implemented UP by `ReconcilerRuntime`. ADR-0086 § Compliance is
  explicit: "A reconciler may read ANOTHER reconciler's View through a core
  driven read-port … the runtime still OWNS every reconciler's View." The
  `ServiceLifecycle` actual-projection consumes the `WorkloadLifecycle` budget as
  an **input** to its diff, not as its own View.

- **Hydration ownership is being reversed (ADR-0086 supersedes-in-part
  ADR-0036).** ADR-0036 (2026-05) put *all* hydration on the runtime (central
  `hydrate_desired`/`hydrate_actual` free-fn `match` arms in
  `reconciler_runtime.rs`, ~1100 lines). ADR-0086 (2026-08) moves **intent +
  observation** hydration back onto the reconciler as impure async `Reconciler`
  trait methods, in a new `overdrive-reconcilers` (`adapter-host`) crate reading
  through an injected `HydrationContext` of 5 read-ports; **view** hydration
  stays runtime-owned. Motivation: co-locate each reconciler's diff+hydration,
  make hydration DST-injectable, restore ports-in-core discipline.

- **The purity boundary is load-bearing and enforced.** `reconcile` is a pure
  sync fn `(desired, actual, view, tick) → (Vec<Action>, View)` — no `.await`,
  no I/O, no store handle, wall-clock only via `tick.now`. Pinned by a
  compile-time signature guard and by dst-lint (a whole-crate AST scan for
  `Instant::now`/`SystemTime::now`/`tokio::`/`rand::`/raw `HashMap`). ADR-0086
  extends dst-lint over the new crate with a narrow allowlist for exactly the
  impure async `hydrate_*` methods. The `ReconcilerIsPure` DST twin-invocation
  invariant is the behavioural backstop.

- **ADR-0079 = "converge only on rows you author" (read-back).** The
  `BackendDiscoveryBridge` was a `Reconciler` that dedup'd on an emit-time
  fingerprint in its View (never reading the row it managed); ADR-0079 deleted
  that field (View is now a field-less struct) and made it hydrate + diff against
  the `service_backends` rows it authors. This is the in-tree precedent for "a
  reconciler's private-memory marker of what it emitted is not a substitute for
  reading the shared observed resource."

---

## RQ1 — Private controller memory vs shared observed state

> Do mature reconciler frameworks even have a "View" (durable per-controller
> private memory)? Where do retry/backoff counters live?

**Headline: mainstream reconciler/controller frameworks hold NO durable private
per-controller memory. Controller state is (a) the shared API object + its
`status` subresource (durable, shared) and (b) an ephemeral, rebuildable
informer/reflector cache. Retry/backoff counters live in the workqueue
rate-limiter / runtime scheduler — in-memory and ephemeral. Durable private
per-instance state is the *durable-execution / workflow* pattern (Temporal's
Event History), NOT the reconciler pattern. The hypothesis is confirmed.**

### Finding 1.1: Controller state = shared object + ephemeral cache; the read model is disposable, not durable private memory

**Evidence**: The reflector `Store` "functions as an in-memory read cache …
ephemeral and rebuilt at runtime — it's not durable storage but rather a volatile
cache maintained during controller operation." The controller reads from this
cache (`Controller::store()` hands out a reader). More generally the read model
in the CQRS sense is "a specialized cache … completely disposable because it can
be entirely rebuilt from the source data stores."

**Source**: [docs.rs — `kube::runtime::Controller`](https://docs.rs/kube/latest/kube/runtime/struct.Controller.html) — Accessed 2026-08-25. Reputation: High (official crate docs).
**Verification**:
- [Microsoft Azure Architecture Center — Materialized View pattern](https://learn.microsoft.com/en-us/azure/architecture/patterns/materialized-view) — Accessed 2026-08-17 (via in-tree CQRS research §3f). High. "A materialized view … is completely disposable because it can be entirely rebuilt from the source data stores" — the read model is a cache, never a source of truth or durable private memory.
- [Kubernetes Blog — controller-runtime cache](https://kubernetes.io/blog/2026/07/29/controller-runtime-cache-explained/) — Accessed 2026-08-17 (via in-tree CQRS research §3a). High. The cache is warmed by list+watch and kept current — rebuilt from the API server, not persisted controller-privately.

**Confidence**: High (3 sources: official Rust crate docs + Microsoft pattern doc + official Kubernetes blog).

**Analysis**: There is no framework equivalent of Overdrive's durable `View`. The controller's read surface is a *disposable* cache of the shared objects; on restart it is rebuilt from list+watch. The only *durable* state a controller depends on is the shared API object (spec + status), which is not private to the controller — every controller and the operator can read it. This is the sharpest structural contrast with Overdrive, whose `ViewStore` persists per-reconciler private memory (CBOR blobs, fsync'd, bulk-loaded on boot).

### Finding 1.2: Retry/backoff counters live in the workqueue rate-limiter / runtime scheduler — in-memory and ephemeral

**Evidence**: In kube-rs, reconcile retry is expressed by the reconciler
*returning* `Action::requeue(Duration)` (or an error the runtime backs off);
"this backoff state resides in-memory within the runtime scheduler, not persisted
per-controller instance." In Go controller-runtime the analog is the workqueue's
rate-limiter (per-item exponential backoff), also in-memory. The counters are not
part of any durable object.

**Source**: [docs.rs — `kube::runtime::Controller` (Action / requeue / trigger_backoff)](https://docs.rs/kube/latest/kube/runtime/struct.Controller.html) — Accessed 2026-08-25. Reputation: High (official crate docs).
**Verification**:
- [github.com — kubernetes-sigs/controller-runtime issue #521 (SyncPeriod default 10h)](https://github.com/kubernetes-sigs/controller-runtime/issues/521) — Accessed 2026-08-17 (via in-tree CQRS research §3c). High (primary-source repo issue quoting the godoc). Establishes the resync/requeue cadence as runtime-owned, not persisted.
- Kubernetes' *durable* restart record — the Pod `status.containerStatuses[].restartCount` and `lastState` — lives on the **shared object**, not in controller-private memory (ADR-0078 research Finding 1; kubernetes.io Pod Lifecycle). The backoff *timer* is ephemeral; the restart *count* is on the durable shared status. This is the split Overdrive mirrors (observed `AllocStatusRow.restart_count`) — except Overdrive *additionally* keeps a private `restart_counts` in the View.

**Confidence**: High (official crate docs + primary repo issue + the kubernetes.io durable-status distinction).

**Analysis**: The framework norm is: retry/backoff *accounting* is ephemeral runtime state; any restart *fact* that must survive is published on the shared object's `status`. Overdrive's `WorkloadLifecycleView.restart_counts` (durable private memory, counts attempts) is the piece with no direct framework analog — in Kubernetes the equivalent (the CrashLoopBackOff attempt/backoff state) is ephemeral kubelet state, while the durable count is on the Pod status. Overdrive made durable-and-private what Kubernetes keeps ephemeral-and-private, and *separately* publishes an observed count. (Why Overdrive persists it: its reconcilers are pure-sync and hold no long-lived in-process task state across ticks/restarts the way a long-lived controller goroutine/`tokio` task does — so the retry inputs must be persisted somewhere, and the runtime-owned `View` is that somewhere. This is a genuine consequence of the pure-reconcile choice, not gratuitous.)

### Finding 1.3: Durable private per-instance state IS the durable-execution / workflow pattern (Temporal Event History)

**Evidence**: "Event History is durably persisted by the Temporal service,
enabling seamless recovery of your application state from crashes or failures."
Temporal "tracks the progress of each Workflow Execution by appending information
about Events … to the Event History" and replays that append-only log to
reconstruct state after a crash — "the Event History becomes the single source of
truth for each execution's state."

**Source**: [Temporal Docs — Event History](https://docs.temporal.io/workflow-execution/event) — Accessed 2026-08-25. Reputation: Medium-High (official docs.temporal.io; not in the project allowlist YAML — flagged; official vendor documentation for the durable-execution engine).
**Verification**:
- [github.com — temporalio/temporal](https://github.com/temporalio/temporal) — the OSS engine whose persistence layer stores the per-execution event history (cross-reference for the durable-journal claim; the allowlist names `temporalio` github explicitly).
- In-tree parallel: Overdrive's own **Workflow** primitive (ADR-0064/0065/0066) is the peer to reconcilers and DOES keep a durable per-instance journal (`workflow-journal.redb`) — the platform already draws the "durable private per-instance state = workflow, not reconciler" line (`.claude/rules/workflows.md`).

**Confidence**: Medium-High (official Temporal docs + OSS engine + the in-tree workflow/reconciler split which independently draws the same boundary).

**Analysis**: This is the decisive comparative frame. The systems that DO keep durable private per-instance state are the durable-execution engines (Temporal, and Overdrive's own `Workflow` primitive), where the journal is the source of truth for a *terminating orchestration*. Reconcilers/controllers, by contrast, converge a *standing invariant* and hold their durable truth on the *shared* object. **Overdrive's `View` sits between these two idioms**: it is durable private per-reconciler memory (workflow-like durability) attached to a converging reconciler (controller-like role). That hybrid is unusual against the field — which is exactly what makes "does the `View` earn its place?" a live question (answered in Synthesis).

---

## RQ2 — Cross-controller state dependencies

> How does controller A depend on state produced by controller B? Is a shared
> budget across two controllers idiomatic or an anti-pattern?

**Headline: the idiomatic mechanism is that controller A watches the shared API
object (or `status` subresource) that controller B writes — A reads B's *output
as published shared state*, never B's private cache or memory. Single-writer-per
-field is a first-class, enforced discipline (Server-Side Apply field ownership);
two controllers owning one field is a CONFLICT the platform actively arbitrates.
A shared budget written/read by two independent controllers is therefore
non-idiomatic — it is the exact multi-writer hazard the field-ownership model
exists to prevent.**

### Finding 2.1: Cross-controller dependency = watch the shared object; state is the API object + its `status`, not a controller-private store

**Evidence**: "The `status` describes the *current state* of the object,
supplied and updated by the Kubernetes system and its components" while spec is
the user-supplied "*desired state*." "The Kubernetes control plane continually
and actively manages every object's actual state to match the desired state."
Controllers read from a shared, per-resource-type informer cache — "`r.Get()`
and `r.List()` inside a reconciler … read from a local in-memory cache, which
the manager warms up with **list** and then keeps current through **watch**" —
i.e. every controller reads the *same* shared cache of the *shared API objects*,
not a peer controller's private state.

**Source**: [Kubernetes — Working with Kubernetes Objects (spec/status)](https://kubernetes.io/docs/concepts/overview/working-with-objects/kubernetes-objects/) — Accessed 2026-08-25. Reputation: High (official kubernetes.io).
**Verification**:
- [Kubernetes Blog — How the controller-runtime Cache Actually Works](https://kubernetes.io/blog/2026/07/29/controller-runtime-cache-explained/) — Accessed 2026-08-17 (via in-tree CQRS research). High. Establishes the shared informer cache as the read surface: keyed by resource type, owned by the manager, shared across all consumers.
- [docs.rs — `kube::runtime::Controller` (`.owns()` / `.watches()`)](https://docs.rs/kube/latest/kube/runtime/struct.Controller.html) — Accessed 2026-08-17 (via in-tree CQRS research). High. The Rust prior art: controller A declares a watch on the object type controller B writes; the runtime wakes A on B's writes. A reads the object, not B.

**Confidence**: High (3 sources: official kubernetes.io concept doc + official blog + official Rust crate docs).

**Analysis**: The canonical dependency edge is A **watches** the shared object that B **owns a field of**. Owner references and `Owns()`/`Watches()` declarations formalise "B's Deployment owns these Pods; A is woken when they change." There is no supported notion of "controller A reads controller B's in-memory reconciler state" — that state does not exist as a shared surface; the only shared surface is the API object graph. This is the direct structural contrast with Overdrive's `ServiceLifecycle` reading `WorkloadLifecycle`'s **private `View`** (`restart_status_for_alloc`, ADR-0086's `RestartBudgetView`). The idiomatic Kubernetes shape would put the restart budget on the *shared observed object* if a second controller needs it — which is exactly the ADR-0078 distinction (`AllocStatusRow.restart_count` is the operator/observed surface; the View is private).

### Finding 2.2: Single-writer-per-field is a first-class, enforced discipline; two owners of one field is a CONFLICT

**Evidence**: "A *conflict* is a special status error that occurs when an
`Apply` operation tries to change a field that another manager also claims to
manage. This prevents an applier from unintentionally overwriting the value set
by another user." Server-Side Apply tracks per-field ownership in
`.metadata.managedFields`. Resolution options: **force** ("changes the value of
the field, and removes the field from all other managers' entries" → become
*sole* manager — the automated-controller-recommended path), give up the claim,
or **shared ownership** ("that field's management [is] shared … Any subsequent
attempt to change the value of the shared field, by any of the appliers, results
in a conflict" — i.e. shared ownership is only stable while all writers agree on
the value).

**Source**: [Kubernetes — Server-Side Apply (field management & conflicts)](https://kubernetes.io/docs/reference/using-api/server-side-apply/) — Accessed 2026-08-25. Reputation: High (official kubernetes.io reference).
**Verification**:
- Kubernetes API conventions establish that `status` is written by the object's *controller* (the single owning controller of that resource), reinforcing one-writer-per-resource-status. Cross-referenced with the spec/status doc above and the controller-runtime cache blog.
- In-tree parallel: `.claude/rules/development.md` § "State-layer hygiene" ("Owner-writer only, full rows") and ADR-0079's `ServiceBackendRow` two-writer analysis are the Overdrive-side statement of the *same* discipline.

**Confidence**: High (authoritative kubernetes.io reference, directly on point; corroborated by API conventions and the in-tree hygiene rule).

**Analysis**: Kubernetes elevates single-writer-per-field to an *enforced* platform mechanism: the recommended path for an automated controller is to **become sole manager** of the fields it writes (`--force-conflicts` / `force:true`); shared ownership is explicitly the fragile case that only holds while writers never disagree. This is the strongest evidence that a **shared budget written/read by two independent controllers is non-idiomatic** — it is precisely the multi-writer situation the model treats as a conflict to be resolved *toward single ownership*. Overdrive's own ADR-0079 reaches the identical conclusion for `ServiceBackendRow` ("two writers on a key that is `service_id` alone … violates State-layer hygiene"; "the resolution is a single-owner decision"). The restart budget (RQ3) has the same shape one layer up: `WorkloadLifecycle` authors it, `ServiceLifecycle` reads it — which is *tolerable as a read-only projection through one owner* (ADR-0086's framing), but would be a conflict if `ServiceLifecycle` ever wrote it.

### Finding 2.3: Reading another controller's private memory has no idiomatic precedent; the "shared cache" everyone reads is the shared *object* store, not a controller-private store

**Evidence**: In controller-runtime and kube-rs the read model is the
reflector/informer `Store`, "one materialized cache per watched resource type,
owned by the runtime, keyed by resource type (not by controller), and shared
across all consumers" (in-tree CQRS research §3a, cross-referenced against the
official controller-runtime cache blog and `kube::runtime::reflector` docs).
Retry/backoff counters (a controller's private accounting) live in the
**workqueue rate-limiter**, which is *per-controller, in-memory, and ephemeral*
(see RQ1) — never exposed to or read by another controller.

**Source**: [docs.rs — `kube::runtime::reflector`](https://docs.rs/kube/latest/kube/runtime/fn.reflector.html) — Accessed 2026-08-17 (via in-tree CQRS research §3a). Reputation: High.
**Verification**:
- [Kubernetes Blog — controller-runtime cache](https://kubernetes.io/blog/2026/07/29/controller-runtime-cache-explained/) — High. The cache is keyed by *resource type*, not by controller — there is structurally no "controller B's cache" for A to read.
- [docs.rs — `kube::runtime::Controller::store()`](https://docs.rs/kube/latest/kube/runtime/struct.Controller.html) — High. A controller's `Store` is a read view of the *watched object type*, handed out to the rest of the program — again, shared object state, not private reconciler memory.

**Confidence**: Medium-High (3 official sources establish the shared-object-cache shape and the absence of any controller-private shared state; the negative claim "no precedent for reading another controller's private memory" is an inference from that absence rather than a direct statement — flagged).

**Analysis**: The structural point for Overdrive: in the mature model there is nothing analogous to a `View` that a *second* controller reads. Everyone reads the shared object graph; a controller's private accounting (retry counters) is ephemeral and unshared. Overdrive's `restart_status_for_alloc` (a second reconciler reading the first's private `View`) has **no idiomatic counterpart** — the closest idiomatic move is to publish the needed fact onto the shared observed object (as ADR-0078 does for the operator-facing `AllocStatusRow.restart_count`) and have the dependent reconciler read *that*.

---

## RQ3 — Restart-budget / backoff authority

> Where does restart-budget authority live across the field, and is it EVER split
> across two independent controllers by restart *cause*? (The sharpest question.)

**Headline: every mature system unifies restart authority under ONE owner, and
that one owner spans all restart CAUSES for a given unit. No examined system
splits a single restart budget across two independent controllers keyed by
restart cause. Overdrive's crash-restart-in-`WorkloadLifecycle` +
liveness-restart-in-`ServiceLifecycle` drawing on one shared pool is a
divergence from the field norm — though the underlying idea it approximates
(one budget spanning crash AND liveness restarts) is exactly what the kubelet
does, under a single owner.**

### Finding 3.1: Kubernetes/kubelet — ONE owner (the kubelet), ONE per-container budget spanning crash exits AND liveness failures

**Evidence**: "the kubelet manages containers … The kubelet also manages
executing probes that track the health of your application." When a liveness
probe fails, "the kubelet kills the container and restarts it" — the same
`RESTARTS` counter increments as for a crash exit. "After containers in a Pod
exit, the kubelet restarts them with an exponential backoff delay (10s, 20s,
40s, …), that is capped at 300 seconds (5 minutes)." "Once a container has
executed for 10 minutes without any problems, the kubelet resets the restart
backoff timer for that container." `restartPolicy` is a **Pod-level** field
(`Always`/`OnFailure`/`Never`) applied per container by the kubelet.

**Source**: [Kubernetes — Pod Lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/) — Accessed 2026-08-25. Reputation: High (1.0, official kubernetes.io).
**Verification**:
- [Kubernetes — Configure Liveness, Readiness and Startup Probes](https://kubernetes.io/docs/tasks/configure-pod-container/configure-liveness-readiness-startup-probes/) — Accessed 2026-08-25. High. "If the command returns a non-zero value, the kubelet kills the container and restarts it" / "If the handler returns a failure code, the kubelet kills the container and restarts it." Confirms the kubelet is the SINGLE owner of both probe execution and the restart action, and that the liveness-triggered restart increments the same `RESTARTS` counter with **no separate authority or budget**.
- [Kubernetes — Pod Lifecycle (restart-backoff section)](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/) — Accessed 2026-08-25. High. Confirms the 10s→300s exponential backoff and 10-minute reset (also independently corroborated in the in-tree crash-observability research, ADR-0078 Finding 1).

**Confidence**: High (2 official kubernetes.io pages, cross-referenced; the backoff schedule also appears verbatim in the in-tree ADR-0078 research).

**Analysis**: The kubelet is a *single* restart authority per container. Crucially, a **liveness-probe restart and a crash-exit restart are the SAME authority drawing on the SAME per-container backoff/count** — Kubernetes does exactly the thing Overdrive wants (one budget spanning crash + liveness), but it does so under **one owner** (the kubelet), never by having a second controller reach into the first controller's private budget. This is the precise contrast RQ3 asks for.

### Finding 3.2: Nomad — restart (local, one owner) vs reschedule (relocate); a clean cause-agnostic boundary, not a per-cause split

**Evidence**: "Restarts happen on the client that is running the task." The
`restart` block parameters: `attempts` ("the number of restarts allowed in the
configured interval"), `interval`, `delay` (with up to 25% jitter), `mode`
("Controls the behavior when the task fails more than `attempts` times in an
interval"). `mode="fail"` (default): "not to attempt to restart the task once
the number of `attempts` have been used … marked as failed"; `mode="delay"`:
"wait until another `interval` before restarting." "Restarts are different from
rescheduling, which happens when the tasks run out of restart attempts."

**Source**: [HashiCorp Nomad — `restart` Block](https://developer.hashicorp.com/nomad/docs/job-specification/restart) — Accessed 2026-08-25. Reputation: High (official developer.hashicorp.com).
**Verification**:
- [HashiCorp Nomad — `reschedule` Block](https://developer.hashicorp.com/nomad/docs/job-specification/reschedule) — (cross-referenced via the restart page's own boundary statement) — the escalation target when local restart attempts are exhausted; a *different mechanism* (relocate to another node), not a different owner of the same budget.
- In-tree corroboration: `docs/research/orchestration/nomad-scheduling-and-reconciler-pattern-research.md` and ADR-0078 research (Nomad `Restarts`/`LastRestart` scalar per task).

**Confidence**: High (official Nomad docs + in-tree corroboration).

**Analysis**: Nomad's two mechanisms are split by **layer/escalation** (local in-place restart vs cluster reschedule), not by **cause**. One owner (the client running the task) holds the local restart budget; when it is exhausted, control **escalates** to the scheduler for reschedule. This is a single-owner budget with a hierarchical escalation — the opposite of two peer controllers sharing one budget. The `restart` budget is cause-agnostic: any task failure (crash, OOM, non-zero exit) draws on the same `attempts`-per-`interval` pool.

### Finding 3.3: Erlang/OTP supervisor — ONE unified restart budget (intensity/period) per supervisor, escalates to parent

**Evidence**: "If more than `MaxR` number of restarts occur in the last `MaxT`
seconds, the supervisor terminates all the child processes and then itself"
(defaults `intensity => 1`, `period => 5`). The supervisor "owns a single
restart budget that applies collectively to all its children." Exceeding it
terminates all children + the supervisor, and "the parent supervisor then
responds by either restarting the failed supervisor or terminating itself."
Strategies (`one_for_one`/`one_for_all`/`rest_for_one`/`simple_one_for_one`)
govern *which* children restart, not *who owns* the budget.

**Source**: [Erlang/OTP — Supervisor Behaviour (Design Principles)](https://www.erlang.org/doc/system/sup_princ.html) — Accessed 2026-08-25. Reputation: High (official erlang.org language documentation; not in the project's explicit allowlist YAML, but an official language/runtime doc — High by the source-verification tier for "Official … tech docs").
**Verification**:
- [Erlang/OTP — `supervisor` module reference](https://www.erlang.org/doc/apps/stdlib/supervisor.html) — (cross-reference for the `intensity`/`period` supervisor-flags API and the "maximum restart intensity" rule).

**Confidence**: Medium-High (2 official erlang.org pages; single organisation, so cross-referencing is intra-org — flagged. The restart-intensity model is also textbook-canonical across decades of OTP literature).

**Analysis**: OTP is the archetype of *supervision as a first-class ownership tree*. A supervisor owns ONE restart budget for its whole child set; the budget is a property of the **supervisor node**, not of any cause or any individual child. When exhausted, the supervisor does not hand its budget to a sibling — it **escalates up** the tree. This is the "let-it-crash + single-owner budget + escalate" model that Nomad's restart→reschedule and Kubernetes' kubelet→(CrashLoopBackOff surfaced to controllers) both echo. No cause-split; no peer-sharing.

### Finding 3.4: systemd — the unit is the owner; StartLimitBurst/StartLimitIntervalSec is one rate-limit per unit, cause-agnostic

**Evidence**: "Units which are started more than *burst* times within an
*interval* time span are not permitted to start any more." "units which are
configured for `Restart=`, and which reach the start limit are not attempted to
be restarted anymore." `StartLimitAction=` (default `none`) is the escalation
knob when the limit is hit. Defaults come from `DefaultStartLimitBurst=` /
`DefaultStartLimitIntervalSec=` in manager config (well-known upstream defaults:
burst 5, interval 10s).

**Source**: [systemd.unit(5) — man7.org](https://man7.org/linux/man-pages/man5/systemd.unit.5.html) — Accessed 2026-08-25. Reputation: High (man7.org, canonical Linux man-page mirror; the upstream freedesktop.org page returned HTTP 403 to automated fetch — see Knowledge Gaps).

**Confidence**: Medium-High (one authoritative man-page mirror; upstream freedesktop.org unreachable for a second direct read — the numeric defaults are stated as "from manager config" and the 5/10s values are the well-documented upstream defaults, corroborated by long-standing systemd documentation but not re-verified against a second directly-fetched primary source here).

**Analysis**: systemd's restart rate-limit is a property of the **unit** — one budget (`StartLimitBurst` per `StartLimitIntervalSec`) covering every restart cause (`Restart=on-failure`/`on-abnormal`/`always`/etc. all feed the same limiter). When the limit is hit the unit enters a failed/rate-limited state and `StartLimitAction` may escalate (e.g. `reboot`). Single owner, cause-agnostic budget.

### Finding 3.5: Akka typed supervision — the parent's supervise decorator owns one budget per supervised behavior

**Evidence**: Supervision is declared by the parent via `Behaviors.supervise(...)
.onFailure[...](SupervisorStrategy.restart.withLimit(maxNrOfRetries = N,
withinTimeRange = D))`. "restart no more than 10 times in a 10 second period."
When the limit is exceeded, the actor is **stopped**. Each `supervise` wrapper
"maintains distinct restart counts and limits" — nested supervise layers create
separate budgets per exception class.

**Source**: [Akka — Fault Tolerance (typed)](https://doc.akka.io/libraries/akka-core/current/typed/fault-tolerance.html) — Accessed 2026-08-25. Reputation: Medium-High (official akka.io project docs; not in the project allowlist YAML — flagged; Akka's licensing/commercial model noted as a mild bias consideration, but the supervision mechanics are canonical and long-documented).

**Confidence**: Medium (one official-project source; the actor-supervision restart-limit model is nonetheless well-established).

**Analysis**: Akka is the one nuance in the set: the restart budget is owned by the **parent's supervise decorator around a specific behavior**, and can be *layered by exception class* (a per-cause split is expressible — a different strategy per exception type). But note the shape: even here the split is **within one owner** (the parent wrapping its child), producing *separate* budgets per cause — it is NOT one shared budget consumed by two independent supervisors. The owner is always the parent of the supervised behavior.

### Cross-cutting synthesis for RQ3

| System | Restart-budget owner | Budget spans multiple causes? | Split across independent controllers? |
|---|---|---|---|
| Kubernetes/kubelet | kubelet (per container) | **Yes** — crash exit + liveness failure share one per-container backoff/count | No — one owner |
| Nomad | client running the task | Yes — any task failure; escalates to reschedule when exhausted | No — single owner, hierarchical escalation |
| Erlang/OTP | supervisor (per child set) | Yes — collective, cause-agnostic; escalates to parent | No — single owner, escalate up tree |
| systemd | the unit | Yes — all `Restart=` causes feed one limiter | No — single owner |
| Akka typed | parent's supervise decorator | Optionally per-exception (separate budgets), all under the parent | No — always the parent owns |
| **Overdrive today** | **split: `WorkloadLifecycle` View owns the budget; `ServiceLifecycle` reads it** | Yes — crash-restart + liveness-restart draw on one `RESTART_BACKOFF_CEILING` pool | **YES — two independent reconcilers share one budget by cause** |

The field norm is unambiguous: **restart authority is single-owner, and a single
owner's budget legitimately spans multiple causes** (the kubelet is the exact
precedent for "one budget covering both crash and liveness restarts"). What no
examined system does is what Overdrive does — have **two independent
controllers** (`WorkloadLifecycle`, `ServiceLifecycle`) draw down **one** budget,
one of them by reaching into the other's *private memory*. The idiomatic
resolution (mapped in Synthesis) is single-owner: either liveness-restart moves
under the same owner as crash-restart, or the budget is promoted to a shared
*observed* surface both read but exactly one *authors* (the ADR-0079 "converge
only on rows you author" discipline).

---

## RQ4 — Hydration ownership, reconciler placement, purity boundary

> (a) Runtime-hydrates-centrally vs controller-owns-hydration; (b) the
> pure-decision / impure-I/O split; (c) separate-crate/module precedent.

**Headline: in mature frameworks the controller reads its own inputs from a
shared runtime-owned cache — there is NO central dispatcher that builds each
controller's typed projection (ADR-0036's shape), so the framework norm is
closer to "controller owns its read" (ADR-0086's direction) than to central
hydration. Mainstream controllers do NOT keep `reconcile` pure — they interleave
reads/writes; Overdrive's pure-sync `reconcile` with hydration fully separated is
STRICTER than the norm and is backed by the ESR/verification lineage (Anvil,
OSDI'24) the codebase already cites. The framework runtime is always a library
with controllers as separate user code — ADR-0086's crate extraction matches
that shape.**

### Finding 4.1 (a): No central per-controller hydration dispatcher exists in the field — controllers read their own inputs from the shared cache

**Evidence**: In kube-rs each controller is `Controller<K>`, reads its inputs
from its own reflector `Store` (`Controller::store()`), and is spawned as an
independent task. In Go controller-runtime the reconciler calls `r.Get()` /
`r.List()` itself against the manager's shared cache. Neither model has a central
function that match-dispatches per controller to build a typed `State` — the
controller *owns the read call*.

**Source**: [docs.rs — `kube::runtime::Controller`](https://docs.rs/kube/latest/kube/runtime/struct.Controller.html) — Accessed 2026-08-25. Reputation: High.
**Verification**:
- [Kubernetes Blog — controller-runtime cache](https://kubernetes.io/blog/2026/07/29/controller-runtime-cache-explained/) — Accessed 2026-08-17 (in-tree CQRS research §3a). High. "`r.Get()` and `r.List()` inside a reconciler … read from a local in-memory cache" — the reconciler issues its own reads.
- [docs.rs — `kube::runtime::reflector`](https://docs.rs/kube/latest/kube/runtime/fn.reflector.html) — Accessed 2026-08-17 (in-tree CQRS research §3a). High. The `Store` is the per-controller read surface.

**Confidence**: High (3 official sources).

**Analysis**: The field norm is "controller owns its hydration read" — which is precisely ADR-0086's direction (reconcilers own `hydrate_desired`/`hydrate_actual`) and precisely NOT ADR-0036's central-dispatcher shape. The *reason* Overdrive had a central dispatcher at all is its enum-erasure design (one registry, one tick loop, one DST clock over a heterogeneous reconciler set — see the in-tree CQRS research §3d/§3e): the frameworks avoid the central `match` by monomorphizing per type (`Controller<K>`), which Overdrive declined for the single-loop/single-clock DST story. ADR-0086 keeps the enum (the erasure) but moves the *hydration body* back onto each reconciler — a middle path the framework evidence supports on the "who owns the read" axis while retaining Overdrive's DST substrate.

### Finding 4.2 (b): Mainstream `reconcile` is NOT pure — it does I/O; Overdrive's pure-sync split is stricter, and is backed by the ESR/verification lineage

**Evidence**: The canonical controller-runtime/kube-rs `reconcile(ctx, req)` /
`reconcile(obj, ctx) -> Result<Action>` reads from the cache and *writes* to the
API server *inside* the reconcile function (`r.Status().Update(...)`,
`client.Apply(...)`), returning an `Action` for requeue. The read *model
population* (list+watch) is separate, but the reconcile function itself is
impure (performs reads and writes). By contrast, the functional-core /
imperative-shell discipline (a pure decision core, an impure I/O shell) and the
formally-verified-controller line push the decision logic to a pure function.

**Source**: [docs.rs — `kube::runtime::Controller`](https://docs.rs/kube/latest/kube/runtime/struct.Controller.html) — Accessed 2026-08-25. Reputation: High. (The reconcile signature returns `Action` and the surrounding examples perform API writes inside reconcile.)
**Verification**:
- [Anvil: Verifying Liveness of Cluster Management Controllers — USENIX OSDI '24](https://www.usenix.org/conference/osdi24/presentation/sun-xudong) (PDF: https://www.usenix.org/system/files/osdi24-sun-xudong.pdf) — Accessed 2026-08-25. Reputation: High (1.0, USENIX peer-reviewed). Anvil verifies Rust controllers against **eventually stable reconciliation (ESR)** as a temporal-logic liveness property — the same ESR property Overdrive's rules cite as the reason `reconcile` must be a pure transition function. This is the academic precedent that a controller's decision logic *can and should* be a verifiable pure function separate from its I/O.
- [Martin Fowler — CQRS](https://martinfowler.com/bliki/CQRS.html) (in-tree CQRS research §3f) — Medium-High. The read model is populated separately from command handling; separating the read (query) side from the write (command) side is the structural discipline Overdrive's hydrate/reconcile split embodies.
- In-tree: `.claude/rules/development.md` § "Reconciler I/O" pins the pure-sync `reconcile` explicitly to "make DST (§21) replay and ESR verification (§18, USENIX OSDI '24 *Anvil*) possible."

**Confidence**: Medium-High for the comparative claim (High that mainstream reconcile does I/O; High that Anvil verifies pure-ESR Rust controllers; the "functional-core/imperative-shell" framing itself is a recognised pattern but its canonical source (Gary Bernhardt) is outside the allowlist — flagged, and the claim rests on the Anvil + framework evidence).

**Analysis**: Overdrive is on the strict end of the spectrum: it forces `reconcile` to be pure-sync (no I/O, no clock, `tick.now` injected) and pushes ALL I/O into the separate `hydrate_*` surface. Mainstream controllers do not do this — they read and write inside reconcile and rely on idempotency + level-triggering for correctness. The evidence that Overdrive's stricter split is *sound rather than idiosyncratic* is Anvil: a peer-reviewed system that verifies liveness of controllers precisely by treating the reconcile step as a pure transition function over injected state. ADR-0086's central guarantee — `reconcile` stays pure, only `hydrate_*` is impure — keeps Overdrive inside that verifiable envelope while relocating hydration ownership. This is the single strongest external support for the pure-reconcile/impure-hydrate boundary.

### Finding 4.3 (c): The framework runtime is always a library; controllers are separate user code — ADR-0086's crate extraction matches the norm

**Evidence**: kube-rs ships `kube-runtime` (the `Controller`/`reflector`/`watcher`
machinery) as a library crate; user controllers live in the user's own crate and
depend on it. Go controller-runtime is `sigs.k8s.io/controller-runtime` (a
library); operators (e.g. scaffolded by Kubebuilder/Operator SDK) live in
separate projects that import it. The framework/runtime and the controller
implementations are always in separate compilation units.

**Source**: [github.com — kube-rs/kube (`kube-runtime` crate)](https://github.com/kube-rs/kube/blob/main/kube-runtime/src/controller/mod.rs) — Accessed 2026-08-17 (in-tree CQRS research §3e). Reputation: High (primary-source repo).
**Verification**:
- [docs.rs — `kube::runtime`](https://docs.rs/kube/latest/kube/runtime/index.html) — Accessed 2026-08-25. High. The runtime is a published library surface consumed by controller code.
- [Kubebuilder Book — architecture](https://book.kubebuilder.io/) (in-tree CQRS research §3b/§3c, kubernetes-sigs official) — High. Controllers are scaffolded in a user project depending on controller-runtime.

**Confidence**: High (official repo + crate docs + kubebuilder book).

**Analysis**: ADR-0086's move — keep the `Reconciler` *trait/contract* (+ `Action`, `TickContext`, broker keys) in `overdrive-core`, extract the reconciler *impls* into a separate `overdrive-reconcilers` crate — is structurally the kube-rs pattern: the trait/runtime surface is one crate, the controller impls are another. The one difference is that Overdrive's runtime (`ReconcilerRuntime`) lives in `overdrive-control-plane` and *depends up on* the reconcilers crate (registers + runs them), whereas in kube-rs the user crate depends *down on* `kube-runtime` — an inversion forced by Overdrive's closed-world registry/enum. The 5 read-port traits (ADR-0086 D5) that break the resulting Cargo cycle are the ports-in-core discipline the field also follows (the trait is shared, the impls are injected). Net: the separate-crate placement has clear precedent; the dependency direction is Overdrive-specific because of the enum-erased single registry.

---

## Comparison Table

| System | Private durable controller memory? | Cross-controller state access mechanism | Restart-budget owner | Hydration ownership | Reconcile purity |
|---|---|---|---|---|---|
| **Kubernetes / kubelet** | No — restart *count* on the shared Pod `status`; backoff timer ephemeral in kubelet | Watch the shared object + `status`; Server-Side Apply enforces single-writer-per-field | **kubelet**, per container; one budget spans crash exits + liveness failures | kubelet reads pod spec/status; controllers read shared informer cache | N/A (kubelet is imperative) |
| **controller-runtime (Go)** | No — ephemeral shared informer cache; retry in workqueue rate-limiter (in-memory) | `r.Get`/`r.List` the shared cache of the object B writes; `Owns`/`Watches`; SSA field ownership | The object's single owning controller | Manager-owned shared cache; reconciler issues its own reads | Impure — `reconcile` does I/O (Get + Status().Update) |
| **kube-rs (Rust)** | No — reflector `Store` is in-memory, rebuildable; requeue/backoff in runtime scheduler | Watch shared object via `.owns()`/`.watches()`; read own `Store` | The controller for that resource kind | Controller owns its read (its `Store`); per-type monomorphized | Impure — `reconcile` returns `Action`, does I/O inside |
| **Nomad** | No — `Restarts`/`LastRestart` scalar per task on shared alloc state | Scheduler reads shared allocation/eval state | **Client running the task**; escalates to reschedule when exhausted | Server/scheduler owns placement; client owns local restart | N/A (scheduler is imperative) |
| **Erlang/OTP** | Supervisor holds in-memory restart tally (not durable; lost on supervisor restart) | Supervision tree (parent owns children); no peer cross-read of budgets | **Supervisor**, one budget for all its children; escalates up tree | N/A (message-passing) | N/A |
| **systemd** | No — start-limit counters in-memory per unit | Unit dependencies (Requires/After); no shared-budget read | **The unit**; one `StartLimitBurst`/`IntervalSec` limiter, cause-agnostic | N/A | N/A |
| **Temporal / durable-execution** | **Yes** — durable per-execution Event History (event-sourced, source of truth) | Signals / queries / child-workflow results — explicit APIs, not shared memory | N/A (retry policies per activity/workflow, journaled) | Engine replays journal to reconstruct state | Workflow body deterministic-replayable (the pure-ish core) |
| **Overdrive — today** | **Yes** — durable per-reconciler `View` (CBOR, `ViewStore`, fsync'd, bulk-loaded) | `ServiceLifecycle` reads `WorkloadLifecycle`'s **private View** via `restart_status_for_alloc` (ADR-0086 `RestartBudgetView`); also converge-on-authored-rows (ADR-0079) | **Split**: `WorkloadLifecycle` View owns the budget; `ServiceLifecycle` draws on it by cause (crash vs liveness) | ADR-0036: runtime-central; ADR-0086: reconciler-owns intent+observation, runtime-owns view | **Pure-sync** `reconcile` (no I/O, no clock); hydration fully separated |

Reading the table: Overdrive is an outlier in exactly two columns — **durable
private controller memory** (only Temporal/durable-execution shares that trait,
and that is the *workflow* idiom) and **restart-budget owner** (the only entry
where the budget is *split across two independent controllers by cause*). In the
other three columns Overdrive is either norm-aligned (separate-crate placement)
or stricter-but-well-founded (pure reconcile; reconciler-owned hydration under
ADR-0086).

---

## Synthesis — the four decisions

This section answers the four decisions the DESIGN wave must make, mapping the
evidence back to the Overdrive shape. It states what the evidence *favours*; it
does not make the decision.

### Decision 1 — Does the `View` (durable private reconciler memory) earn its place, or is it an anti-pattern vs the shared-object model?

**Evidence position: the `View` is an outlier against the reconciler norm but is
NOT an anti-pattern per se — it earns its place ONLY for genuinely private retry
*inputs* that must survive restart, and it is a smell precisely when it holds
things that belong on the shared observed object.**

- No mainstream reconciler framework has durable private per-controller memory
  (RQ1, High): state is the shared object + status; the read model is a
  disposable cache; retry counters are ephemeral. Durable private per-instance
  state is the *workflow* idiom (Temporal Event History; Overdrive's own
  `Workflow` journal), not the reconciler idiom.
- Overdrive persists a `View` because its `reconcile` is pure-sync and holds no
  long-lived in-process task state across ticks/restarts — the retry *inputs*
  (`attempts`, `last_failure_seen_at`) have to live *somewhere*, and the
  runtime-owned `ViewStore` is that somewhere. This is a real consequence of the
  pure-reconcile choice (Finding 1.2 analysis), and it is disciplined by the
  in-tree "persist inputs, not derived state" rule (which is itself the correct
  shape — Kubernetes keeps the backoff *timer* ephemeral and the restart *count*
  durable-on-status; Overdrive keeps the *inputs* and recomputes the deadline).
- Where the `View` becomes a smell is exactly what ADR-0079 already found and
  fixed: a `View` field used as a *substitute for reading the shared observed
  resource* (the emit-time fingerprint that made `BackendDiscoveryBridge`'s View
  field-less once deleted). The field-less-View outcome of ADR-0079 is the
  reconciler-norm-aligned end state: converge on the shared authored rows, hold
  no private memory. Several Overdrive Views are already empty (`WorkflowLifecycle`,
  `SvidLifecycle` Slice-01, the post-ADR-0079 bridge), matching the norm.

**Net for Decision 1**: the `View` earns its place for the *thin, genuinely
private, restart-surviving retry inputs* (the `restart_counts` / backoff-input
case). It is an anti-pattern when it caches a projection of state that lives, or
should live, on the shared observed object — the ADR-0079 test ("is this the
resource the reconciler *manages*, or a marker of what it emitted?") is the right
discriminator, and it is already in the rules. The evidence does not support
abolishing the `View`; it supports *keeping it minimal and never letting it stand
in for the shared object*.

### Decision 2 — Is the `ServiceLifecycle` ↔ `WorkloadLifecycle` shared restart budget idiomatic, or should restart authority be single-owner (and if so, which owner)?

**Evidence position: NOT idiomatic. The field is unambiguous — restart authority
is single-owner everywhere examined (RQ3, High). Overdrive should make restart
authority single-owner. The evidence favours `WorkloadLifecycle` as that owner.**

- Every examined system (kubelet, Nomad, OTP, systemd, Akka) puts the restart
  budget under ONE owner. Critically, a single owner's budget legitimately spans
  multiple *causes* — the kubelet is the exact precedent for "one per-container
  budget covering BOTH crash exits AND liveness-probe restarts" (Finding 3.1).
  So the thing Overdrive wants (crash + liveness restarts share one budget) is
  idiomatic; the way Overdrive does it (two independent reconcilers, one reaching
  into the other's private `View`) is not.
- This is the *same* defect class as the `ServiceBackendRow` two-writer problem
  ADR-0079 diagnosed, and as the Server-Side Apply single-writer-per-field
  discipline (RQ2, High): a value that two controllers both depend on wants a
  single owner, with any dependent reading a *read-only projection* — never
  co-authoring.
- **Which owner:** the evidence points to `WorkloadLifecycle`, for three reasons.
  (i) It already authors the budget (`restart_counts` in its View) and already
  emits crash-restarts — it is the natural "restart authority," analogous to the
  kubelet owning the per-container restart. (ii) The kubelet precedent is
  specifically "the restart authority also owns liveness restarts" — i.e.
  liveness-driven restart belongs under the *same* owner as crash restart, which
  argues for folding the liveness-restart decision into `WorkloadLifecycle`
  rather than leaving it in `ServiceLifecycle`. (iii) ADR-0086 already models the
  cross-reconciler read as a one-way `RestartBudgetView` projection owned by the
  runtime/`WorkloadLifecycle` — the ownership is *already* asymmetric; the
  divergence is only that `ServiceLifecycle` makes an independent *restart
  decision* against that budget. Two shapes are consistent with the evidence:
  (a) move the liveness-exhaustion restart decision under `WorkloadLifecycle` (it
  becomes the single restart authority, consuming a liveness-failure signal the
  same way it consumes a crash signal — the kubelet shape); or (b) keep
  `ServiceLifecycle` as the liveness *detector* but have it emit an
  observation/signal that `WorkloadLifecycle` (the sole restart authority) acts
  on — the detector/authority split OTP and Nomad use (detect locally, escalate
  to the owner). Both are single-owner; the evidence does not distinguish between
  (a) and (b), and that choice is the DESIGN wave's.

**Net for Decision 2**: the current shared-budget-across-two-reconcilers shape is
the one clear divergence from the field norm and should be resolved toward
single-owner restart authority, with `WorkloadLifecycle` as the owner and
`ServiceLifecycle` at most a detector/signal-source. This is *not* a claim that
crash and liveness restarts need separate budgets — the kubelet shows one budget
spanning both causes is correct; the fix is unifying the *owner*, not splitting
the *budget*.

### Decision 3 — Reconciler-owned vs runtime-owned hydration: what does the evidence favour?

**Evidence position: the field norm is "controller owns its read" (RQ4a, High) —
which favours ADR-0086's direction (reconcilers own intent+observation hydration)
over ADR-0036's central-dispatcher shape.**

- No framework has a central per-controller hydration dispatcher; each controller
  reads its own inputs from the shared runtime-owned cache. ADR-0036's central
  `match`-per-reconciler `hydrate_desired`/`hydrate_actual` is the shape the
  field specifically does not use, and its own costs (split logic across crates,
  non-DST-injectable, structural `too_many_lines`) are documented in ADR-0086 §
  Context.
- ADR-0086's specific refinement — reconciler owns *intent + observation*
  hydration (impure async trait methods), runtime keeps *view* hydration
  (bulk-load + write-through) — is coherent with the evidence: the "read model"
  the controller reads is its business (reconciler-owned), while durable private
  memory persistence is a runtime concern (there is no framework analog for the
  latter because there is no framework View — so this half is Overdrive-specific
  and reasonably kept uniform in the runtime).
- One caveat the evidence surfaces: the frameworks get "controller owns its read"
  *for free* by monomorphizing per resource type (`Controller<K>`), avoiding any
  central erasure. Overdrive keeps the closed-world `AnyReconciler` enum (single
  loop / single DST clock) and therefore pays an enum-forwarding cost per
  reconciler even under ADR-0086. That is a deliberate trade for the DST
  substrate (in-tree CQRS research §3e / §5), not a defect ADR-0086 introduces —
  but it means Overdrive's "reconciler owns hydration" is *co-located with the
  impl* rather than *monomorphized per type* like kube-rs. The evidence supports
  the ownership move; it does not require dissolving the enum (GH #272 scope).

**Net for Decision 3**: the evidence favours reconciler-owned hydration
(ADR-0086) over central runtime hydration (ADR-0036) on the "who owns the read"
axis. The one place Overdrive stays runtime-owned (the `View`) has no framework
counterpart and is reasonably kept uniform.

### Decision 4 — The pure-reconcile / impure-hydrate split.

**Evidence position: Overdrive's pure-sync `reconcile` with hydration fully
separated is STRICTER than the mainstream norm, and that strictness is
well-founded — it is the shape the formal-verification lineage (Anvil, OSDI'24)
requires, and the shape ADR-0086 is careful to preserve.**

- Mainstream controllers do NOT keep `reconcile` pure — controller-runtime and
  kube-rs interleave reads and writes inside the reconcile function and rely on
  idempotency + level-triggering (RQ4b, High). So Overdrive is on the strict end.
- The strictness is not idiosyncratic: Anvil (peer-reviewed, USENIX OSDI'24)
  verifies liveness (ESR) of Rust controllers precisely by treating the reconcile
  step as a pure transition function over injected state — the exact property the
  Overdrive rules cite as *why* `reconcile` must be pure. The
  functional-core/imperative-shell discipline (pure decision core, impure I/O
  shell) is the general-programming name for the same split. This is the
  strongest external support for Overdrive's boundary.
- ADR-0086 keeps this boundary intact: `reconcile` stays pure-sync; only the new
  `hydrate_*` methods are impure/async; the dst-lint scan is extended over the
  new crate with a narrow allowlist for exactly those methods. The purity firewall
  is preserved, and the change actually *improves* it (hydration becomes
  DST-injectable via the 5 sim read-ports).

**Net for Decision 4**: keep the pure-reconcile / impure-hydrate split — it is
the correct and evidence-backed shape, stricter than the mainstream but validated
by the ESR/verification lineage the platform already builds on. ADR-0086's
preservation of the pure `reconcile` while relocating the impure hydration is
consistent with the evidence.

### Synthesis in one paragraph

The evidence draws a clean line. Overdrive's **pure-reconcile / impure-hydrate**
split (Decision 4) and its move to **reconciler-owned hydration** (Decision 3)
are norm-aligned-or-better and should be kept — the frameworks put the read on
the controller, and the verification lineage (Anvil) validates the pure decision
core. The **`View`** (Decision 1) is an outlier the field does not share, but it
is defensible as *minimal durable retry-input memory* forced by the pure-reconcile
choice, provided it never stands in for the shared observed object (the ADR-0079
discipline). The one shape the evidence flags as a genuine divergence to fix is
the **restart budget split across two reconcilers by cause** (Decision 2): every
comparable system unifies restart authority under one owner (a single owner whose
budget legitimately spans multiple causes, as the kubelet's does), and Overdrive
should converge restart authority onto `WorkloadLifecycle`, demoting
`ServiceLifecycle` to a detector/signal-source rather than a co-authority drawing
on another reconciler's private budget.

---

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | In allowlist? | Cross-verified |
|--------|--------|-----------|------|-------------|---------------|----------------|
| Kubernetes — Pod Lifecycle | kubernetes.io | High (1.0) | official | 2026-08-25 | Yes | Y |
| Kubernetes — Liveness/Readiness/Startup Probes | kubernetes.io | High (1.0) | official | 2026-08-25 | Yes | Y |
| Kubernetes — Working with Objects (spec/status) | kubernetes.io | High (1.0) | official | 2026-08-25 | Yes | Y |
| Kubernetes — Server-Side Apply | kubernetes.io | High (1.0) | official | 2026-08-25 | Yes | Y |
| Kubernetes Blog — controller-runtime cache | kubernetes.io | High (1.0) | official | 2026-08-17† | Yes | Y |
| Nomad — `restart` Block | developer.hashicorp.com | High (1.0) | official | 2026-08-25 | Yes | Y |
| Nomad — `reschedule` Block | developer.hashicorp.com | High (1.0) | official | 2026-08-25 | Yes | Y |
| Erlang/OTP — Supervisor (Design Principles) | erlang.org | High (1.0) | official lang doc | 2026-08-25 | No (official) | Y (intra-org) |
| Erlang/OTP — `supervisor` module ref | erlang.org | High (1.0) | official lang doc | 2026-08-25 | No (official) | Y (intra-org) |
| systemd.unit(5) | man7.org | High (1.0) | canonical man page | 2026-08-25 | No (official) | Partial (freedesktop 403) |
| Akka — Fault Tolerance (typed) | doc.akka.io | Medium-High (0.8) | official project | 2026-08-25 | No | N (single-source) |
| Temporal — Event History | docs.temporal.io | Medium-High (0.8) | official vendor | 2026-08-25 | No | Y |
| temporalio/temporal | github.com | Medium-High (0.8) | primary repo | 2026-08-25 | Yes | Y |
| kube — `runtime::Controller` | docs.rs | High (1.0) | official crate | 2026-08-25 | Yes | Y |
| kube — `runtime::reflector` | docs.rs | High (1.0) | official crate | 2026-08-17† | Yes | Y |
| kube — `runtime` (index) | docs.rs | High (1.0) | official crate | 2026-08-25 | Yes | Y |
| kube-rs/kube (`kube-runtime`) | github.com | Medium-High (0.8) | primary repo | 2026-08-17† | Yes | Y |
| controller-runtime issue #521 (SyncPeriod) | github.com | Medium-High (0.8) | primary repo | 2026-08-17† | Yes | Y |
| Azure Architecture — Materialized View | learn.microsoft.com | High (1.0) | official | 2026-08-17† | Yes | Y |
| Martin Fowler — CQRS | martinfowler.com | Medium-High (0.8) | industry expert | 2026-08-17† | Yes | Y |
| Kubebuilder Book | book.kubebuilder.io | High (1.0) | kubernetes-sigs official | 2026-08-25 | No (official) | Y |
| Anvil — USENIX OSDI '24 | usenix.org | High (1.0) | peer-reviewed | 2026-08-25 | Yes | Y |

† Sources marked 2026-08-17 were established in the in-tree CQRS research
(`docs/research/architecture/cqrs-structural-mechanism-reconciler-framework-research.md`,
a prior Nova session) and re-used here for the RQ1/RQ4 framework-side claims; the
load-bearing RQ2/RQ3 sources and the durable-execution contrast were fetched
fresh on 2026-08-25.

**Reputation distribution**: High (1.0): 16 · Medium-High (0.8): 6 · Total: 22.
**Average reputation ≈ 0.945** (≥ 0.80 threshold met). Four sources (erlang.org,
man7.org, book.kubebuilder.io) are authoritative-official but outside the project
allowlist YAML — scored High per the source-verification "Official / tech docs"
tier; two (doc.akka.io, docs.temporal.io) are official-vendor and scored
Medium-High with a commercial-interest flag.

## Knowledge Gaps

### Gap 1: systemd numeric defaults not re-verified against a second primary source
**Issue**: The upstream freedesktop.org systemd.unit(5) page returned HTTP 403 to
automated fetch; man7.org confirmed the *behavior* (rate limit; `Restart=` units
that hit the limit are not restarted) and that defaults come from
`DefaultStartLimitBurst=`/`DefaultStartLimitIntervalSec=`, but the specific numeric
defaults (burst 5 / interval 10s) are the well-known upstream values, not
re-verified against a second directly-fetched primary source here.
**Attempted**: freedesktop.org (403), man7.org (behavior confirmed).
**Recommendation**: If the exact defaults become load-bearing, fetch
freedesktop.org via an authenticated/alternate tool or `systemd` source
(`src/core/manager.c`).

### Gap 2: RQ2's "no precedent for reading another controller's private memory" is an inference from absence
**Issue**: The claim that reading a peer controller's private state has no
idiomatic precedent rests on the *structural absence* of any such surface (the
cache is keyed by resource type, not controller; retry counters are ephemeral and
unshared) rather than a source that says "do not do this."
**Attempted**: controller-runtime cache blog, kube-rs reflector/Controller docs.
**Recommendation**: Strong as a structural inference; if a direct prohibition is
wanted, the Server-Side Apply single-writer-per-field discipline (Finding 2.2) is
the closest *positive* statement of the same principle.

### Gap 3: "Which single owner" for restart authority is under-determined by the evidence
**Issue**: The evidence strongly supports *single-owner* restart authority but
does not distinguish between (a) folding liveness-restart into `WorkloadLifecycle`
(kubelet shape) and (b) keeping `ServiceLifecycle` as a detector that signals the
sole authority (OTP/Nomad detect-then-escalate shape). Both are single-owner.
**Attempted**: kubelet (unified owner), OTP/Nomad (detector→escalate).
**Recommendation**: This is a DESIGN value judgement; both shapes are
evidence-consistent. The choice hinges on where the liveness-probe *facts* live
and which reconciler already hydrates them.

### Gap 4: Some sources are official-but-outside the project allowlist
**Issue**: erlang.org, man7.org, book.kubebuilder.io, doc.akka.io,
docs.temporal.io are not in `.nwave/trusted-source-domains.yaml`. They were used
under the source-verification reputation tiers (official language/project docs =
High; official vendor = Medium-High) and flagged inline.
**Recommendation**: Consider adding erlang.org, man7.org, and kubebuilder.io to
the allowlist's `official` block — all three are canonical primary sources for
this domain.

### Gap 5: functional-core / imperative-shell canonical source is outside the allowlist
**Issue**: The "functional core, imperative shell" framing (Gary Bernhardt) has no
in-allowlist canonical source; the RQ4(b) claim therefore rests on the Anvil
(usenix.org) peer-reviewed evidence and the framework I/O-in-reconcile evidence,
with the imperative-shell term used descriptively.
**Recommendation**: Sufficient as-is; Anvil is the stronger, in-allowlist anchor
for the pure-transition-function property.

## Conflicting Information

No hard contradictions surfaced across sources on the load-bearing claims. One
**nuance (not a conflict)** worth recording for the DESIGN wave:

### Nuance 1: one budget spanning causes vs one budget per cause (both single-owner)
- **Position A (unified budget)**: kubelet (Finding 3.1), Nomad (3.2), OTP (3.3),
  systemd (3.4) — one restart budget covers *all* causes for a unit/child, owned
  by one authority.
- **Position B (per-cause budget, still single-owner)**: Akka typed supervision
  (Finding 3.5) — a parent can layer `supervise` decorators per exception class,
  producing *separate* restart budgets per cause, but all owned by the same
  parent.
- **Assessment**: These do not conflict on the core RQ3 claim (restart authority
  is single-owner). They differ only on whether one owner keeps one budget or
  several. Both are more idiomatic than Overdrive's current *two-owner shared
  budget*. The kubelet's unified-budget shape is the closest analog to what
  Overdrive wants (crash + liveness on one budget) and is the more common shape;
  Akka shows per-cause budgets are also defensible *if kept under one owner*.

## Full Citations

[1] The Kubernetes Authors. "Pod Lifecycle". kubernetes.io. Accessed 2026-08-25. https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/
[2] The Kubernetes Authors. "Configure Liveness, Readiness and Startup Probes". kubernetes.io. Accessed 2026-08-25. https://kubernetes.io/docs/tasks/configure-pod-container/configure-liveness-readiness-startup-probes/
[3] The Kubernetes Authors. "Understanding Kubernetes Objects" (spec/status). kubernetes.io. Accessed 2026-08-25. https://kubernetes.io/docs/concepts/overview/working-with-objects/kubernetes-objects/
[4] The Kubernetes Authors. "Server-Side Apply". kubernetes.io. Accessed 2026-08-25. https://kubernetes.io/docs/reference/using-api/server-side-apply/
[5] The Kubernetes Authors. "How the controller-runtime Cache Actually Works". Kubernetes Blog. Accessed 2026-08-17. https://kubernetes.io/blog/2026/07/29/controller-runtime-cache-explained/
[6] HashiCorp. "restart Block — Job Specification". developer.hashicorp.com. Accessed 2026-08-25. https://developer.hashicorp.com/nomad/docs/job-specification/restart
[7] HashiCorp. "reschedule Block — Job Specification". developer.hashicorp.com. Accessed 2026-08-25. https://developer.hashicorp.com/nomad/docs/job-specification/reschedule
[8] Ericsson / Erlang OTP Team. "Supervisor Behaviour" (OTP Design Principles). erlang.org. Accessed 2026-08-25. https://www.erlang.org/doc/system/sup_princ.html
[9] Ericsson / Erlang OTP Team. "supervisor" (stdlib module reference). erlang.org. Accessed 2026-08-25. https://www.erlang.org/doc/apps/stdlib/supervisor.html
[10] systemd project. "systemd.unit(5)". man7.org (Linux man-pages). Accessed 2026-08-25. https://man7.org/linux/man-pages/man5/systemd.unit.5.html
[11] Lightbend. "Fault Tolerance (Akka typed)". doc.akka.io. Accessed 2026-08-25. https://doc.akka.io/libraries/akka-core/current/typed/fault-tolerance.html
[12] Temporal Technologies. "Event History". docs.temporal.io. Accessed 2026-08-25. https://docs.temporal.io/workflow-execution/event
[13] Temporal Technologies. "temporalio/temporal". github.com. Accessed 2026-08-25. https://github.com/temporalio/temporal
[14] kube-rs maintainers. "kube::runtime::Controller". docs.rs. Accessed 2026-08-25. https://docs.rs/kube/latest/kube/runtime/struct.Controller.html
[15] kube-rs maintainers. "kube::runtime::reflector". docs.rs. Accessed 2026-08-17. https://docs.rs/kube/latest/kube/runtime/fn.reflector.html
[16] kube-rs maintainers. "kube::runtime". docs.rs. Accessed 2026-08-25. https://docs.rs/kube/latest/kube/runtime/index.html
[17] kube-rs maintainers. "kube-runtime/src/controller/mod.rs". github.com. Accessed 2026-08-17. https://github.com/kube-rs/kube/blob/main/kube-runtime/src/controller/mod.rs
[18] kubernetes-sigs/controller-runtime. "Issue #521 — Why resync default is so large (10 hours)". github.com. Accessed 2026-08-17. https://github.com/kubernetes-sigs/controller-runtime/issues/521
[19] Microsoft. "Materialized View pattern". Azure Architecture Center, learn.microsoft.com. Accessed 2026-08-17. https://learn.microsoft.com/en-us/azure/architecture/patterns/materialized-view
[20] Fowler, Martin. "CQRS". martinfowler.com. Accessed 2026-08-17. https://martinfowler.com/bliki/CQRS.html
[21] The Kubebuilder Authors (kubernetes-sigs). "The Kubebuilder Book". book.kubebuilder.io. Accessed 2026-08-25. https://book.kubebuilder.io/
[22] Sun, Xudong, et al. "Anvil: Verifying Liveness of Cluster Management Controllers". 18th USENIX Symposium on Operating Systems Design and Implementation (OSDI '24), pp. 649-666. July 2024. usenix.org. Accessed 2026-08-25. https://www.usenix.org/conference/osdi24/presentation/sun-xudong

### In-repo grounding artifacts (not external sources; the decision context)
- ADR-0035 (reconciler runtime + `ViewStore`); ADR-0036 (runtime owns hydration — superseded-in-part); ADR-0078 (crash observability: `restart_count` vs private `restart_counts`); ADR-0079 (converge on rows you author); ADR-0084 (cadence/interest hooks); ADR-0086 (reconcilers own hydration; `overdrive-reconcilers` crate; `RestartBudgetView`).
- `.claude/rules/reconcilers.md`; `.claude/rules/development.md` §§ "Reconciler I/O", "Persist inputs, not derived state", "A convergent record cannot answer 'did it happen'", "State-layer hygiene"; `.claude/rules/workflows.md`.
- Code: `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs:786` (`restart_counts` / `RestartAllocation`); `crates/overdrive-core/src/service_lifecycle.rs:790,810` (`fact.restart_count >= RESTART_BACKOFF_CEILING`; `RestartReason::LivenessExhausted`); `crates/overdrive-control-plane/src/reconciler_runtime.rs:499` (`restart_status_for_alloc`).
- `docs/research/architecture/cqrs-structural-mechanism-reconciler-framework-research.md` (prior Nova research; RQ1/RQ4 framework-side sourcing).
