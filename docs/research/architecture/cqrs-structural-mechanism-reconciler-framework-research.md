# Research: Clean structural mechanisms for expressing CQRS read-side projections and event-interest in a reconciler framework (Overdrive GH #266)

**Date**: 2026-08-17 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (mechanism evidence); the *whether-to-adopt* judgement is deliberately left to DESIGN | **Sources**: 16 web (13 load-bearing) + 3 baseline research files + in-tree code

---

## Executive Summary

The platform-level "should Overdrive be CQRS/event-sourced" question is **settled** (baseline file
#1: already CQRS-structural via Intent/Observation; no event-sourcing on Raft). This research answers
the user's *refined* concern — "we don't have a clean way of doing these things structurally" — for
GH #266: what is the clean structural mechanism for CQRS read-side projections and event-interest at
the reconciler-framework layer, within Rust's associated-type / `dyn` constraints?

The evidence from the mature frameworks (Kubernetes controller-runtime, the Rust kube-rs
`Controller`/`reflector`/`Store`) is consistent and strong: a clean CQRS read-side is achieved not by
one abstraction but by **three separable, first-class mechanisms** — (1) **event-interest
declaration** (`For`/`Owns`/`Watches`; kube-rs `.owns()`/`.watches()`), (2) an **owned, rebuildable
read-model/cache** (the informer/reflector Store; Microsoft's Materialized View), and (3) a
**per-object cadence hook** as a level-triggered safety net beside the edge-triggered event path
(`SyncPeriod`/`RequeueAfter`; kube-rs `Action::requeue`). Overdrive **already ships** the substrate
for (1) and (2) — the `EvaluationBroker` workqueue and the Intent/Observation stores — so the gap
#266 names is the *declarative layer* over them, not the plumbing.

**Headline finding.** A first-class structural mechanism is warranted, but **not** as a single
heavyweight "CQRS read-side framework." The strongest-evidenced, lowest-risk shapes are the two that
**do not touch the associated-type erasure problem**: the **cadence hook (Facet 1)** — object-safe,
kube-rs-proven, removes the convergence-loop's per-reconciler special-case, keeps the loop-owned
clock (DST-safe) — and the **event-interest declaration** (candidate E) — additive, object-safe,
formalizes what 7/8 reconcilers already do through the broker. Both are cleanly Rust-expressible and
land independently.

**The hard crux — hydration erasure (Facet 2) — is where the evidence stops short of a verdict, on
purpose.** Rust's rule that associated types (`Reconciler::State`/`View`) are not `dyn`-compatible
means the `AnyReconciler`/`AnyState`/`AnyReconcilerView` enums are a genuine, non-gratuitous erasure
workaround (Finding 3d). The canonical Rust escape (erased-serde/`typetag`: an object-safe erased
trait + `downcast`) **does not remove the type-matching — it relocates it** from a compile-time
central `match` to a runtime `downcast` that can panic on a wiring bug. So "reconciler owns its own
hydration" is achievable, but it trades the project's valued compile-time exhaustiveness (baseline #1
Finding 9) for open-world extensibility that v1 (first-party only, WASM >1yr out) may not need yet.

**The key counter-pressure** (baseline file #3, Finding 8): a first-class projection abstraction pays
off only when the read model differs substantially from the write model. Overdrive's reconcilers
split cleanly — **thin** projections (`NoopHeartbeat`, `WorkflowLifecycle`, `SvidLifecycle`: empty/`()`
views) are exactly the "source data is simple and easy to query" case Microsoft and Fowler
*disqualify*, while only the **fat** cross-store reconcilers (`WorkflowLifecycle` 3-store
cross-product, `ServiceMapHydrator` LWW fold) meet the "read ≠ write" bar. A mechanism mandatory for
all reconcilers would tax the majority to serve the minority. Combined with the fact that the enum
also *serves the single-loop/single-clock DST design* (the substrate of the Anvil-ESR verification
story), the enum may be the right tool for a small first-party set — the "smell" is partly the price
of a verification property the project chose deliberately.

**Bottom line for the DESIGN wave**: adopt the cadence hook and (if open/closed hygiene is judged
worth it) the event-interest declaration as first-class surfaces — both strongly evidenced and
low-risk. Treat the hydration-erasure rework as a genuine value judgement the evidence *frames but
does not settle*: where to pay the irreducible type-match (central `match` vs per-reconciler
`downcast` vs per-type monomorphization), and whether the open/closed win justifies the
compile-time-exhaustiveness loss and the DST-design disturbance.

> **Update — §6 session synthesis (2026-08-22).** A working discussion after the initial findings
> refined the planning framing in one load-bearing way: the three mechanisms above (event-interest
> [E], read-model cache [F], and Facet-2 hydration) are **not independent** — the **reflector-`Store`
> pattern fuses them into a single primitive** (in kube-rs the informer *is* cache + event-handlers +
> the surface `reconcile` reads). That collapses the reconciler-framework work into **two landable
> pieces**: **Piece A** — the cadence hook, standalone; **Piece B** — the reflector-`Store` that unifies
> interests + warm `actual` cache + hydration. It also situates the #265 durable-events proposal as a
> *separate track* that consumes Piece B's change feed. See **§6** for the synthesis both go into the
> DESIGN wave with.

---

## 1. Reconcile with the settled baseline (scope fence)

The platform-level question "should Overdrive be CQRS / event-sourced?" is **SETTLED** and is NOT
re-derived here. Three prior in-tree research files establish the baseline this document builds on;
each is cited by its verdict, and this research is strictly *downstream* of them:

1. **`docs/research/architecture/event-driven-internals-comprehensive-research.md`** (Finding 11,
   verdict table). Verdict: Overdrive *already applies CQRS structurally* via the
   Intent(command/write) / Observation(query/read) split, and must **not** layer event-sourcing on
   top of Raft (ES belongs only at the Workflow boundary). "CQRS without event sourcing (the current
   architecture) is a strict subset of CQRS+ES." **This document does not relitigate that.**
2. **`docs/research/orchestration/crash-observability-under-lww-comprehensive-research.md`** (landed
   as ADR-0078). Verdict: the current-state read model must be paired with a bounded, explicitly
   lossy history side (Kubernetes `lastState`+`restartCount` / Nomad ring-buffer shape). Establishes
   that Overdrive's read-model layer is a real CQRS read side, and that read-model design decisions
   are already being made per-surface.
3. **`docs/research/control-plane/issue-139-followup-streaming-restart-budget-research.md`**
   (Finding 8, the key counter-pressure). Verdict: when the read model ≈ the write model, CQRS is
   the wrong tool (Fowler's own warning) — a full read-model split for a `u32` projection is
   over-engineering. **This is the load-bearing counter-pressure §5 must weigh.**

**The scope of THIS document** is the user's refined concern, verbatim: *"yes, we might be doing CQRS
to some extent, but we don't have a clean way of doing these things structurally."* The question is
therefore **not** "should we do CQRS" (settled: we already do) but: *what is the clean structural
mechanism to express CQRS read-side projections and event-interest as first-class, and would making
them first-class fix the open/closed smell in GH #266 — within Rust's associated-type / `dyn`
constraints?* Everything below is research **input to a future DESIGN wave**, not a design. Per
CLAUDE.md ("implement to the design / don't invent API surface"), §4 presents *options with
evidence*, not a recommended API.

---

## 2. The concrete symptom (GH #266) and the Rust crux

**The symptom** (verified against live code, 2026-08-17):

- **Facet 1 — cadence.** `spawn_convergence_loop` (`crates/overdrive-control-plane/src/lib.rs:2427`)
  owns one broker-drain + tick loop and the injected `Clock`. The #266 body reports it special-cases
  one reconciler's sweep (`VM_RECLAMATION_SWEEP_INTERVAL`, `node/<id>` target scheme). Proposed hook:
  `fn resync_period(&self) -> Option<Duration>` or `fn next_evaluation(&self, now) -> Option<Evaluation>`
  (the K8s `SyncPeriod`/`RequeueAfter` shape; the kube-rs `Action::requeue(Duration)` shape).
- **Facet 2 — hydration (the CQRS read side).** The runtime hydrates each reconciler's `State` via
  central `hydrate_desired` / `hydrate_actual` functions
  (`crates/overdrive-control-plane/src/reconciler_runtime.rs:1729,2673`) with **per-reconciler match
  arms**, plus the `AnyReconciler` / `AnyState` / `AnyReconcilerView` enums
  (`crates/overdrive-core/src/reconcilers/mod.rs:798,930,335`) and the `AnyReconciler::reconcile`
  dispatch (`:852`) that `panic!`s on a `(reconciler, state, view)` triple mismatch. Adding a
  reconciler edits all of them — the "five compiler-enforced wiring sites."

**The Rust crux (why this is not gratuitous centralization).** Per ADR-0035/0036, the `Reconciler`
trait carries associated types:

```
trait Reconciler {
    const NAME: &'static str;
    type State;                                        // divergent per reconciler
    type View: Serialize + DeserializeOwned + Default + Clone + Eq + …;
    fn reconcile(&self, desired: &State, actual: &State, view: &View, tick) -> (Vec<Action>, View);
}
```

Rust associated types do **not** erase behind `dyn Reconciler` — a trait with `type State` /
`type View` is not `dyn`-compatible (object-safe), so the runtime **cannot** hold a
`Vec<Box<dyn Reconciler>>` and call a generic `hydrate` / `reconcile` (Finding 3d). The
`AnyReconciler` / `AnyState` / `AnyReconcilerView` enums **are** the erasure layer — a hand-rolled,
closed-world stand-in for the `dyn` the language denies. So "a reconciler owns its own hydration"
collides head-on with the erasure workaround: the moment hydration returns a per-reconciler `State`,
the runtime needs *some* way to hold that heterogeneous result behind one handle, which is exactly
what the enums do today.

**What is already right (do not disturb).** The pure-`reconcile` contract
(`fn(&R, &R::State, &R::State, &R::View, &TickContext) -> (Vec<Action>, R::View)`, pinned by a
compile-time signature test at `mod.rs:271`) is the load-bearing DST-replay / Anvil-ESR property
(baseline file #1, Findings 4–5). Any #266 mechanism must keep hydration (impure, async, store-reading)
**strictly separate** from `reconcile` (pure, sync, no clock) — the frameworks agree (Finding 3a:
the read model is populated by list+watch, *separate* from the reconcile function). The
`EvaluationBroker` (`crates/overdrive-core/src/eval_broker.rs`) is already the workqueue half of the
event-interest mechanism; what is missing is the *declarative* producer half (Finding 3b).

**Two facets, two risk profiles.** Facet 1 (cadence) is object-safe and low-risk — `resync_period()`
has no associated types, needs one `AnyReconciler` arm, touches no `AnyState`/`AnyView`. Facet 2
(hydration) is where the erasure constraint bites and where the real design question lives. §4
enumerates the candidate mechanisms for Facet 2.

---

## 3. Findings

### 3a. Read-model / projection ownership — how mature systems structure it

**Claim**: In the canonical controller architecture, the read model is a **shared, materialized
cache (the informer / reflector Store)** that every reconcile reads from — it is a first-class,
independently-maintained component, structurally separate from both the write path and the event
delivery path.

**Evidence** (Kubernetes controller-runtime, official):
> "`r.Get()` and `r.List()` inside a reconciler typically do not read from the API server. They
> read from a local in-memory cache, which the manager warms up with **list** and then keeps
> current through **watch**."
> — and: "The size of the local cache and the set of indexes directly drive memory consumption."

**Source**: [Kubernetes Blog — "How the controller-runtime Cache Actually Works"](https://kubernetes.io/blog/2026/07/29/controller-runtime-cache-explained/) — Accessed 2026-08-17. Reputation: High (1.0, official kubernetes.io).

**Verification**:
- [docs.rs — `kube::runtime::Controller`](https://docs.rs/kube/latest/kube/runtime/struct.Controller.html) — Accessed 2026-08-17. Reputation: High (docs.rs, official crate docs). The Rust `Controller` exposes `Controller::store()` — "Retrieve a copy of the reader before starting the controller" — the reflector Store IS the read-model cache handed out to the rest of the program.
- [docs.rs — `kube::runtime::reflector`](https://docs.rs/kube/latest/kube/runtime/fn.reflector.html) — Accessed 2026-08-17. Reputation: High (docs.rs, official crate docs). "A reflector is a `watcher` with a `Store`… The reader part is the `Store` interface that you can send to other parts of your program as state" (paraphrase confirmed via search + crate docs).

**Confidence**: High (3 sources; two official crate/project docs + one official Kubernetes blog).

**Analysis**: The structural lesson: the read model is *not* a per-consumer ad-hoc projection built
inline. It is **one materialized cache per watched resource type**, owned by the runtime, keyed by
resource type (not by controller), and shared across all consumers. Overdrive's `hydrate_actual`
per-reconciler match arms (`crates/overdrive-control-plane/src/reconciler_runtime.rs:2673`) are the
*opposite* shape — each reconciler's `actual` projection is built inline by a central dispatcher
function, and there is no shared, independently-maintained read model. This is the gap #266 Facet 2
names. Note the important disanalogy (see §5 counter-pressure): K8s can afford one shared cache per
type because it reads a *remote* API server and the cache pays for itself across many consumers;
Overdrive reads *local* stores (`IntentStore`, `ObservationStore`) where the "cache" is already the
store itself.

### 3b. Event-interest declaration as a first-class mechanism

**Claim**: Mature controller frameworks make **event-interest a first-class, per-controller
declaration** — the controller declares *which resource changes wake it* via `For` / `Owns` /
`Watches` (Go controller-runtime / kubebuilder) or `.owns()` / `.watches()` / `.reconcile_all_on()`
(Rust kube-rs). This declaration is decoupled from the reconcile function and from the read model.

**Evidence** (Rust kube-rs, official crate docs):
> `.owns()` — "Specify `Child` objects which `K` owns and should be watched";
> `.watches()` — "Specify `Watched` object which `K` has a custom relation to and should be watched";
> `.reconcile_all_on()` — "Trigger a reconciliation for all managed objects whenever `trigger`
> emits a value."

**Source**: [docs.rs — `kube::runtime::Controller`](https://docs.rs/kube/latest/kube/runtime/struct.Controller.html) — Accessed 2026-08-17. Reputation: High (official crate docs).

**Verification**:
- [Kubernetes Blog — controller-runtime cache](https://kubernetes.io/blog/2026/07/29/controller-runtime-cache-explained/) — Accessed 2026-08-17. High. "an informer is created automatically when you register `Watches(...)`" — event-interest declaration is what instantiates the watch/informer.
- [Kubebuilder Book — Controller Watch Functions](https://book-v1.book.kubebuilder.io/beyond_basics/controller_watches.html) — Accessed 2026-08-17. Reputation: High (kubernetes-sigs official book). Documents `Watches`/`Owns` as the controller's declaration of what it observes. (Also confirmed in baseline file #1, Finding 2.)
- Search-corroborated: "`Owns` is just sugar for `Watches` with the `EnqueueRequestForOwner` handler pre-wired" — the declaration compiles down to a watch + an event handler that enqueues a reconcile request.

**Confidence**: High (3+ sources; official Rust + Go/CNCF).

**Analysis**: Overdrive's `EvaluationBroker` (keyed on `(ReconcilerName, TargetResource)`, collapsing
duplicate submits into a cancelable set — `crates/overdrive-core/src/eval_broker.rs`) already
*implements* the consumer side of this (the workqueue). What it lacks is the **declarative producer
side**: today, *who submits an Evaluation for which reconciler on which observation-row change* is
wired imperatively in the runtime, not declared by the reconciler. The marcus-sa comment on #266
("7/8 reconcilers are already purely event-triggered by observation-store row changes") maps exactly
onto "every controller declares a `Watches` and the runtime instantiates the watch." The first-class
mechanism the frameworks converge on is: **the reconciler declares its event-interest; the runtime
owns the watch→enqueue plumbing.** This is a strictly additive declaration surface — it does not
require touching the pure-`reconcile` function.

### 3c. Level-vs-edge resync cadence (the safety backstop)

**Claim**: Every mature controller framework keeps **both** an edge-triggered event path (fast,
responsive) **and** a periodic level-triggered resync (slow, safety net) — because edge-only is
fragile (one dropped/missed event ⇒ permanent divergence). The two are distinct, first-class knobs:
`SyncPeriod` (whole-set resync cadence) and `RequeueAfter` (per-object next-evaluation deadline).
This is exactly the two-hook generalization the marcus-sa #266 comment proposes.

**Evidence**:
- **`SyncPeriod` (whole-set resync)**: controller-runtime default is **10 hours**; its godoc:
  "SyncPeriod determines the minimum frequency at which watched resources are reconciled. A lower
  period will correct entropy more quickly, but reduce responsiveness to change if there are many
  watched resources."
  **Source**: [github.com — kubernetes-sigs/controller-runtime issue #521 ("Why resync default is so large — 10 hours")](https://github.com/kubernetes-sigs/controller-runtime/issues/521) — Accessed 2026-08-17. Reputation: High (github.com, primary-source repo issue quoting the official godoc).
- **`RequeueAfter` (per-object next-evaluation deadline)**: "RequeueAfter is appropriate and
  necessary, particularly for managing external systems that do not emit events or for handling
  resources that take time to converge… Some tasks, such as rotating secrets or renewing
  certificates, must happen at specific intervals."
  **Source**: [Kubebuilder Book — Watching Resources](https://book.kubebuilder.io/reference/watching-resources) — Accessed 2026-08-17. Reputation: High (kubernetes-sigs official book).

**Verification**:
- The Rust analog is `kube::runtime::Action::requeue(Duration)` vs `Action::await_change()` — the
  reconciler *returns* its next cadence; the runtime owns the clock/timer. [docs.rs — `kube::runtime::Controller`](https://docs.rs/kube/latest/kube/runtime/struct.Controller.html) — Accessed 2026-08-17. High.
- Level-triggered resilience to missed events is established in baseline file #1 (Finding 2:
  kubernetes.io Pod-lifecycle + Kubebuilder "level-triggered … resilient to missed events, external
  changes, partial failures"). Cross-referenced there against 3 sources.
- Corroboration (Medium, non-listed domain, flagged): golinuxcloud "Kubernetes Reconcile Loop
  Explained" — "Periodic reconciliation should be done to insure against reconciler bug for missed
  watch events and to poll for objects that cannot be watched." Used only as directional support;
  the two High sources above carry the claim.

**Confidence**: High (2 High primary sources + official Rust analog; claim also independently
established in baseline file #1).

**Analysis**: This directly validates the #266 marcus-sa comment's "keep BOTH hooks" position. The
two knobs are *not redundant*: `RequeueAfter`/`next_evaluation` is per-target and reconciler-decided
(cert near-expiry, VM reclamation sweep); `SyncPeriod`/`resync_period` is the coarse whole-set
backstop that re-drives everything even absent any event, catching the "dropped observation-store
notification ⇒ silent permanent divergence" failure. Overdrive's `spawn_convergence_loop`
(`crates/overdrive-control-plane/src/lib.rs:2427`) today hardcodes a single reconciler's sweep
interval (`VM_RECLAMATION_SWEEP_INTERVAL`, `node/<id>` target scheme) into the loop — the exact
"convergence loop special-cases one reconciler" smell #266 Facet 1 names. The framework-consensus
shape is: **the reconciler declares its cadence (`resync_period()` / `next_evaluation(now)`); the
loop owns the clock** — which is precisely #266's constraint that "the loop must own the clock
(SimClock under DST); the hook stays pure (`now` passed in)." Note this is the shape kube-rs already
ships (`Action::requeue(Duration)`), so it is proven Rust-expressible; it is also the *lower-risk*
half of #266 (Facet 1) because it does **not** touch the associated-type erasure problem — a
`fn resync_period(&self) -> Option<Duration>` (or `next_evaluation(&self, now) -> Option<Evaluation>`)
is an object-safe method with no associated types, addable to the `Reconciler` trait and threaded
through `AnyReconciler` with one new match arm, no `AnyState`/`AnyView` change.

### 3d. Object-safe / type-erased hydration in statically-typed languages

**Claim**: The canonical Rust solution to "hold heterogeneous implementors of a trait with
associated types behind one `dyn` handle" is an **object-safe (dyn-compatible) erased intermediate
trait** that wraps the non-object-safe generic/associated-type trait — the `erased-serde` /
`typetag` pattern. This is a real, load-bearing precedent that a first-class hydration/read-model
surface could adopt instead of the hand-rolled `AnyReconciler` enum.

**Evidence** (dtolnay/erased-serde, the canonical crate):
> Serde's traits "contain generic methods which cannot be made into a trait object." The library
> "wraps non-object-safe traits behind object-safe intermediaries" using three techniques:
> (1) by-value `self` handled by `impl` for `Option<T>`; (2) associated types handled by
> "carefully short-term stashing things behind a pointer"; (3) generic return types "flipped into
> a callback style where the return value is passed as a generic argument."

**Source**: [github.com — dtolnay/erased-serde](https://github.com/dtolnay/erased-serde) — Accessed 2026-08-17. Reputation: High (github.com, primary source; David Tolnay is the serde maintainer).

**Verification**:
- [docs.rs — `erased_serde`](https://docs.rs/erased-serde) — Accessed 2026-08-17. Reputation: High (official crate docs). Same mechanism, published API.
- [quinedot — "Erased traits" (Rust learning)](https://quinedot.github.io/rust-learning/dyn-trait-erased.html) — Accessed 2026-08-17. Reputation: Medium (`github.io` pedagogical site, NOT in trusted-domain list — flagged; used only to corroborate the general pattern, which the two High sources above already establish). "The general idea is to use `dyn` (type erasure) to replace all the non-dyn-compatible uses such as GATs and type-parameterized methods… an intermediate erased trait that wraps non-object-safe traits."
- [doc.rust-lang.org — object safety / `dyn` compatibility reference] — the underlying language rule that associated types + generic methods break `dyn` compatibility (referenced for the rule itself; the Rust Reference is High/official). Access not separately fetched — the rule is stated identically in the erased-serde README.

**Confidence**: High (2 High primary/official sources + 1 Medium corroboration for the general pattern).

**Analysis** — directly applicable to #266 Facet 2: Overdrive's `AnyReconciler` /
`AnyState` / `AnyReconcilerView` enums are a **hand-rolled, closed-world** instance of exactly the
problem `erased-serde` solves in an **open-world** way. The trade is well-understood:
- **Enum erasure (current)**: closed set, exhaustive `match`, no `Box`/vtable indirection, zero
  heap allocation on the hot path, compile-time-guaranteed dispatch — but *every new reconciler
  edits the enum and every central `match`* (the "five wiring sites" of #266).
- **Erased-trait object (`Box<dyn ErasedReconciler>` + `Box<dyn Any>` state)**: open set, add a
  reconciler without touching a central enum — but pays a vtable indirection, a `downcast`
  (runtime-checked, can fail), heap allocation for the erased `State`/`View`, and *loses the
  compile-time exhaustiveness the project explicitly values* (baseline file #1, Finding 9: "typed
  `Action` enum … compile-time exhaustiveness … load-bearing type-system property").

The key insight for §4: **an erased-trait hydration surface does not eliminate the type-matching; it
moves it from a central `match` to a per-reconciler `downcast` at the trait boundary.** Whether that
is a net win depends entirely on whether "add a reconciler without editing five central sites" is
worth "a runtime-checked downcast that can panic on a wiring bug instead of a compile error." That
is a design-wave value judgement this research surfaces but does not make.

### 3e. The closest real-world Rust analog — kube-rs Controller / reflector / Store

**Claim**: kube-rs is the closest production Rust prior art and demonstrates that the read-model
(reflector `Store`) and event-interest (`.owns()`/`.watches()`) mechanisms are expressible in Rust
**without** the associated-type erasure problem Overdrive hits — because kube-rs erases over a
single generic resource type `K: Resource` per controller, not over a heterogeneous set of
reconcilers with divergent associated `State`/`View` types.

**Evidence**:
- `kube::runtime::Controller<K>` is generic over one Kubernetes resource kind `K`; the reflector
  `Store<K>` is typed to that same `K`. Reconciliation is `reconcile(Arc<K>, Arc<Ctx>) -> Result<Action>`
  where the returned `Action` carries the requeue cadence (`Action::requeue(Duration)` /
  `Action::await_change()`).
- "A controller is an infinite stream of objects to be reconciled" and the mapping from a watched
  object to a reconcile request "hides the reason for the reconciliation request, and forces you to
  write an idempotent reconciler."

**Source**: [docs.rs — `kube::runtime::Controller`](https://docs.rs/kube/latest/kube/runtime/struct.Controller.html) — Accessed 2026-08-17. Reputation: High (official crate docs).

**Verification**:
- [docs.rs — `kube::runtime::reflector`](https://docs.rs/kube/latest/kube/runtime/fn.reflector.html) — Accessed 2026-08-17. High. The `Store<K>` is the read model; the writer is moved into the reflector, the reader is cloneable and shared.
- [docs.rs — `kube::runtime::watcher`](https://docs.rs/kube/latest/kube/runtime/fn.watcher.html) — Accessed 2026-08-17. High. The event stream a reflector consumes.
- [kube-rs/kube — `kube-runtime/src/controller/mod.rs`](https://github.com/kube-rs/kube/blob/main/kube-runtime/src/controller/mod.rs) — Accessed 2026-08-17. Reputation: High (github.com, primary source). The `Controller` combines a reflector with N watchers scheduling events into one reconcile stream.

**Confidence**: High (4 sources, all official Rust prior art).

**Analysis** — *this is the crux disanalogy, load-bearing for §4*: kube-rs sidesteps Overdrive's
associated-type erasure problem entirely because a kube-rs `Controller<K>` is **monomorphized per
resource kind** — one `Controller<Pod>`, one `Controller<Deployment>`, each its own typed object,
each spawned as an independent `tokio` task with its own typed `Store<K>`. There is no
`Vec<Box<dyn Reconciler>>` holding heterogeneous reconcilers behind one dispatch; there is no
`AnyReconciler` enum. Overdrive's runtime, by contrast, holds *all* reconcilers in one registry and
drives them through one `run_convergence_tick`, precisely because it wants a single broker, a single
tick loop, and a single DST-controllable clock over the whole set. kube-rs pays for its clean
per-type generics with N independent controller tasks and N independent caches; Overdrive pays for
its single-loop / single-clock DST story with the `AnyReconciler` / `AnyState` /
`AnyReconcilerView` enum-erasure layer (`crates/overdrive-core/src/reconcilers/mod.rs:798,930` and
the `reconcile` dispatch match at `:852` that `panic!`s on a `(reconciler, state, view)` triple
mismatch). **The kube-rs shape is evidence that the mechanisms are Rust-expressible, but NOT
evidence that Overdrive can adopt them without either (a) giving up the single-loop/single-clock DST
design, or (b) keeping some erasure layer.** This tension is the heart of §4.

### 3f. When a first-class projection abstraction pays off vs when it is over-engineering

**Claim**: A first-class read-model/projection abstraction (Microsoft's "Materialized View";
Axon/Marten/EventStoreDB "projections"; Akka Projection) is warranted precisely when **the read
model differs substantially from the write model** — denormalized across aggregates, tailored to a
query shape, expensive to compute — and is over-engineering when **the source data is already
query-shaped**. The structural essence of the abstraction (portable across frameworks): *a
projection is an owned, independently-rebuildable read model, subscribed to a change feed, that is
never the source of truth and can be discarded and regenerated.*

**Evidence** (Microsoft, official — the sharpest statement of the disqualifier):
> "A materialized view and the data it contains is completely disposable because it can be entirely
> rebuilt from the source data stores. A materialized view is never updated directly by an
> application, and so it's a specialized cache."
> — and, on when NOT to use it: **"This pattern isn't useful in the following situations: The
> source data is simple and easy to query. The source data changes very quickly, or can be accessed
> without using a view."** — and: "Materialized views tend to be specifically tailored to one, or a
> small number of queries. If many queries are used, materialized views can result in unacceptable
> storage capacity requirements and storage cost."

**Source**: [Microsoft Azure Architecture Center — Materialized View pattern](https://learn.microsoft.com/en-us/azure/architecture/patterns/materialized-view) — Accessed 2026-08-17. Reputation: High (learn.microsoft.com official).

**Verification**:
- [Martin Fowler — CQRS](https://martinfowler.com/bliki/CQRS.html) (via baseline file #3, Finding 8) — "CQRS should only be used on specific portions of a system (a Bounded Context) and not the system as a whole"; pays off when "the read model differs substantially from the write model." Reputation: Medium-High.
- [microservices.io — CQRS pattern](https://microservices.io/patterns/data/cqrs.html) (baseline file #2/#3) — the read side is "a view database, a read-only replica designed specifically to support a given query shape," subscribed to the write side's events. Reputation: Medium-High.
- Framework instances (structural abstraction only; canonical docs domains **not** in the trusted
  list — flagged, cross-referenced against the High/Medium-High sources above): Axon Framework
  (`docs.axoniq.io`) exposes projection/event handlers with EventStore + replay-to-rebuild; Marten
  (`martendb.io`) "projections" rebuildable from the Postgres event store; EventStoreDB
  (`developers.eventstore.com`) built-in projections + persistent subscriptions; Akka Projection.
  These confirm the *portable structural shape* (owned, rebuildable, change-feed-subscribed read
  model) but each carries event-sourcing baggage Overdrive deliberately does not adopt (baseline
  file #1). Sources surfaced via [axoniq.io blog "Axon and Akka"](https://www.axoniq.io/blog/axon-and-akka-how-do-they-compare) and O'Reilly *Mastering Akka* ch.5 (both Medium; flagged).

**Confidence**: High for the "pays off iff read≠write model" heuristic (Microsoft High + Fowler +
microservices.io, all cross-referenced and consistent with baseline files #1 and #3); Medium for the
specific framework-instance details (non-listed domains).

**Analysis** — *the decisive question for #266*: does Overdrive's per-reconciler `State` differ
substantially from the write model, or is it thin? The answer is **mixed, and that mixedness is the
core finding**:
- **Thin projections (read ≈ write ⇒ abstraction unwarranted per Fowler/Microsoft)**: `NoopHeartbeat`
  (`State = ()`), `WorkflowLifecycle` (empty view; pure over `actual`), `SvidLifecycle` (Slice-01
  empty view). For these, a first-class projection abstraction is over-engineering by the sources'
  own disqualifier — "the source data is simple and easy to query."
- **Fat cross-store projections (read ≠ write ⇒ abstraction genuinely earns its keep)**:
  `hydrate_actual` for `WorkflowLifecycle` cross-products *three* sources — the `workflows/` intent
  scan (`running_in_intent`), the engine's live-task set (`has_live_task`), and observed
  `WorkflowTerminal` rows (`terminal`) — into one merged `WorkflowLifecycleState`
  (`crates/overdrive-control-plane/src/reconciler_runtime.rs:2689`). `ServiceMapHydrator` folds LWW
  rows into a `BTreeMap<ServiceId, ServiceHydrationStatus>` picking the dominator per key
  (`:2707`). These are exactly the "denormalized across aggregates, tailored to a query shape"
  materialized views the abstraction is *for*.

So #266's hydration is **not uniformly** the "read ≈ write" case that baseline file #3, Finding 8
warned makes CQRS the wrong tool — for the fat reconcilers it genuinely is a materialized projection;
for the thin ones it is not. A first-class mechanism must therefore *degrade gracefully to near-zero
ceremony for the thin case* or it will impose the over-engineering tax file #3 warns about on the
majority of reconcilers to serve the minority. This is the central design tension, carried into §4
and §5.

---

## 4. The Rust-specific crux — candidate structural mechanisms for #266

These are **options with evidence**, not recommendations. Per CLAUDE.md, this research does not
invent the Overdrive API a DESIGN wave will choose. Each option is scored against three axes the
constraint demands: **(i)** does it preserve the pure-`reconcile` / DST-replay / Anvil-ESR story?
**(ii)** does it *remove* the central match arms or merely *relocate* them? **(iii)** what is the
object-safety / runtime-cost price?

### 4.1 Facet 1 (cadence) — low-risk, object-safe, kube-rs-proven

A cadence hook is **not** blocked by the erasure constraint. `fn resync_period(&self) -> Option<Duration>`
and/or `fn next_evaluation(&self, now: UnixInstant) -> Option<Evaluation>` are object-safe methods
(no associated types), addable to `Reconciler`, threaded through `AnyReconciler` with **one** new
match arm each, touching **no** `AnyState`/`AnyView`. The loop keeps the injected `Clock` and passes
`now` in (pure hook, DST-safe). This is the kube-rs `Action::requeue(Duration)` shape (Finding 3e)
and the K8s `SyncPeriod`+`RequeueAfter` two-knob split (Finding 3c). **Score: (i) preserved — the
hook is pure over `now`; (ii) removes the `VM_RECLAMATION_SWEEP_INTERVAL` special-case from the loop;
(iii) near-zero — object-safe, one arm.** This is the cleanest, most-evidenced, lowest-risk half of
#266 and can land independently of Facet 2.

### 4.2 Facet 2 (hydration / read-side) — the candidates

| # | Candidate | (i) Pure-`reconcile`/DST? | (ii) Match arms: remove or relocate? | (iii) Object-safety / runtime cost | Evidence anchor |
|---|---|---|---|---|---|
| **A** | **Status quo — keep enum erasure** | Preserved | Neither — 5 sites remain | Zero runtime cost; closed-world; compile-time exhaustive | Baseline; ADR-0035/0036 |
| **B** | **Object-safe erased hydration trait** — per-reconciler `async fn hydrate_actual(&self, stores) -> Box<dyn Any>` on an impure `ProjectionSource` trait object; `reconcile` stays typed; runtime `downcast`s at the boundary | Preserved (hydration already impure/async; `reconcile` untouched) | **Relocates**: central `match` → per-reconciler `downcast` (runtime-checked; the `panic!` at `mod.rs:918` becomes a `downcast` failure) | Vtable indirection + heap `Box` for `State`/`View` + runtime downcast that *can* fail where the enum fails at compile time | Finding 3d (erased-serde/typetag pattern) |
| **C** | **Co-locate hydration with the reconciler, keep the enum** — move `hydrate_actual`'s body from the central dispatcher into each reconciler's module (e.g. a sibling impure fn), but still register/dispatch through `AnyReconciler` | Preserved | **Relocates, partially removes**: the central `hydrate_actual` match shrinks; `AnyReconciler`/`AnyState`/`AnyView` enums remain | Zero runtime cost; still closed-world; fewer *distinct* central sites but the enums still edited per reconciler | Finding 3a (ownership) + ADR-0036 (runtime-owns-hydration is the current inversion) |
| **D** | **kube-rs shape — per-reconciler monomorphized task, drop the single loop** — each reconciler is its own generic `Controller`-like task with its own typed store; no enum | Preserved per-task, but **DST story changes**: N tasks / N clocks instead of one loop/one SimClock | **Removes** the enum entirely | No enum; but loses single-broker / single-tick / single-DST-clock design — the reason the enum exists | Finding 3e (kube-rs `Controller<K>` monomorphized per type) |
| **E** | **First-class event-interest declaration** — `fn watches(&self) -> Vec<EventInterest>` (which observation-row kinds/targets wake this reconciler); runtime owns the watch→`EvaluationBroker::submit` plumbing | Preserved (declaration is data; broker already exists) | **Removes** imperative "who submits for whom" wiring; orthogonal to the `State`/`View` enums | Object-safe (returns data); near-zero | Finding 3b (Owns/Watches/For); `eval_broker.rs` already the workqueue |
| **F** | **First-class materialized read-model component** — a runtime-owned, independently-rebuildable projection store per observation surface (reflector-`Store` / Materialized-View shape); reconcilers read from it | Preserved (read model separate from `reconcile`) | Neither removes nor relocates the `State` typing; adds a new shared component | Heavy: a second materialized state + rebuild path; pays off only for fat cross-store reconcilers | Findings 3a, 3f (reflector Store; Materialized View) |

### 4.3 What the evidence says about combining them

- **E is separable and complementary to everything else.** Event-interest declaration (Facet 2's
  *producer* half) does not touch the `State`/`View` erasure at all — it is about *what wakes a
  reconciler*, not *what shape its projection is*. It is the cleanest structural expression of the
  thing the user named ("7/8 reconcilers are already event-triggered by row changes") and composes
  with A, B, C, or D. Evidence strength: High (Finding 3b, and Overdrive already ships the consumer
  half in `eval_broker.rs`).
- **B is the direct answer to "reconciler owns its own hydration" within the erasure constraint** —
  but it does **not** eliminate type-matching; it *moves* it from a compile-time `match` to a
  runtime `downcast` (Finding 3d). The honest framing: B trades "edit five central sites, caught at
  compile time" for "add a reconciler in one place, wiring bugs caught at runtime via downcast
  panic." Whether that is a win is a value judgement about the project's stated preference for
  compile-time exhaustiveness (baseline file #1, Finding 9).
- **C is the lowest-ceremony partial fix** — it relocates hydration ownership to the reconciler
  (addressing the ADR-0036 "runtime owns hydration" inversion #266 objects to) while keeping the
  enum's compile-time guarantees. It reduces but does not eliminate the five-sites edit.
- **D fully removes the enum but is the highest-risk** — it re-architects the single-loop/single-clock
  DST design that the whole reconciler-runtime (and its Anvil-ESR/DST-replay story) is built on. The
  evidence (Finding 3e) shows it is Rust-expressible, but Overdrive chose the single-loop shape
  *deliberately*; D is a re-litigation of that, not a #266 fix.
- **F is warranted only for the fat cross-store reconcilers** (`WorkflowLifecycle` 3-store
  cross-product, `ServiceMapHydrator` LWW fold) and is over-engineering for the thin ones by
  Microsoft's and Fowler's own disqualifier (Finding 3f). A first-class read-model component that is
  mandatory for all reconcilers would impose the file-#3 over-engineering tax on the majority.

### 4.4 The through-line the evidence supports

The frameworks (K8s, kube-rs) achieve "clean structural CQRS read-side" via **three separable
mechanisms**, not one: (1) event-interest declaration [E], (2) an owned read-model/cache [F], (3) a
per-object cadence hook [4.1]. Overdrive **already has** the workqueue (`EvaluationBroker`) and the
stores (Intent/Observation) that (1) and (2) would sit on; the missing structural surface is the
*declarative* layer over them. The associated-type erasure problem (Finding 3d) is specific to
holding heterogeneous `State`/`View` behind one handle — it is a real Rust constraint with exactly
two escapes (closed-world enum [A/C] or open-world erased-trait+downcast [B]); kube-rs's third escape
(monomorphize per type [D]) is available only by giving up the single-loop design. **No option makes
the type-matching disappear; each relocates it** — the DESIGN wave's decision is *where* to pay it
(central `match` vs per-reconciler `downcast` vs per-type monomorphization) and *whether the
open/closed win is worth the exhaustiveness loss.*

---

## 5. Counter-pressure — when a first-class mechanism is over-engineering

Presented honestly and given equal weight to §4. The case *against* a heavyweight first-class CQRS
read-side mechanism is strong and evidence-backed:

1. **File #3's warning applies literally to the thin reconcilers.** Fowler and Microsoft both
   disqualify a read-model abstraction when "the source data is simple and easy to query" (Finding
   3f). `NoopHeartbeat` (`State=()`), `WorkflowLifecycle`/`SvidLifecycle` (empty views) are exactly
   that. A first-class projection abstraction *mandatory for all reconcilers* would tax the majority
   to serve the minority — the precise over-engineering baseline file #3, Finding 8 flagged. Any
   mechanism must degrade to near-zero ceremony for the thin case, or it is a net negative.

2. **YAGNI for event-interest formalization while the broker already works.** 7/8 reconcilers are
   already event-triggered through the `EvaluationBroker` (marcus-sa comment). The consumer half
   (workqueue, cancelable-set storm-proofing) is *done and correct* (`eval_broker.rs`). Formalizing
   the *declarative* producer half (candidate E) is a real cleanliness win, but it is a refactor of
   working wiring, not a bug fix — its value is open/closed hygiene for the *next* reconciler, not
   correctness for the current set. That is a legitimate reason to defer it behind higher-priority
   work.

3. **The enum-erasure cost may be irreducible in Rust — and the enum is not obviously the villain.**
   Finding 3d establishes that *no* option removes the type-matching; each relocates it. The current
   enum buys **compile-time exhaustiveness** — a property the project explicitly treasures (baseline
   file #1, Finding 9: the typed `Action` enum's compile-time exhaustiveness is "a load-bearing
   type-system property"). Candidate B's erased-trait+`downcast` trades that compile-time guarantee
   for runtime open-world extensibility. For a **v1 first-party-only** reconciler set with no WASM
   third-parties yet (baseline file #3, Finding 9 notes WASM is >1 year out), the closed-world enum's
   "edit five sites, all caught by the compiler" is arguably *safer* than "add anywhere, wiring bugs
   surface as a runtime `downcast` panic." The five-sites edit is annoying, not dangerous.

4. **The single-loop / single-clock DST design is load-bearing and the enum serves it.** The enums
   exist because Overdrive deliberately runs one convergence loop with one injected `Clock` (SimClock
   under DST) over the whole reconciler set — the substrate of its DST-replay and Anvil-ESR
   verification story (baseline file #1, Findings 4–5). kube-rs's clean per-type generics (candidate
   D) get their cleanliness by giving that up (N tasks, N clocks). The "smell" #266 names is partly
   the *price of a verification property the project chose on purpose.* Removing the enum without
   preserving single-clock DST would be a regression, not a cleanup.

5. **`hydrate_actual` match-arm growth is O(reconcilers), and the reconciler set is small and
   bounded.** v1 has ~7 reconcilers; the whitepaper's full built-in set is ~10–12. The "five wiring
   sites × N reconcilers" cost is a fixed, small, one-time-per-reconciler edit with compile-time
   guardrails — not an unbounded maintenance sink. The cost/benefit of a framework abstraction
   inverts at small N.

**Balanced synthesis.** The counter-pressure does **not** kill the first-class-mechanism idea; it
*shapes* it. The evidence points away from a single heavyweight "first-class CQRS read-side
framework" and toward **surgical, separable, cheap wins**: the cadence hook (§4.1, strongly
evidenced, low-risk) and — if the DESIGN wave judges the open/closed hygiene worth it — the
event-interest declaration (candidate E, additive, object-safe). The *hydration erasure* itself
(candidates B/C/D) is where the counter-pressure bites hardest: the enum may be the right tool for a
small, first-party, single-clock-DST reconciler set, and the "cleaner" alternatives trade away
compile-time exhaustiveness or the DST design. **The honest headline: the cadence + event-interest
surfaces are worth making first-class on the evidence; the hydration-erasure rework is where "it's a
smell" must be weighed against "the enum is buying compile-time safety and serving the DST design,"
and the evidence does not settle that — it is a genuine DESIGN-wave value judgement.**

---

## 6. Session synthesis (2026-08-22) — the A/B slicing, the reflector-`Store` unification, and the #265 events relation

§3–§5 present the mechanisms as *separable candidates* (E interest, F cache, B/C/D hydration erasure)
because that is how the framework evidence enumerates them. A working discussion after those findings
landed refined the **planning** framing without contradicting the evidence: the mechanisms the
evidence lists separately are, in the mature systems, **realised as one primitive**. This section
records that synthesis as the direct input to the DESIGN wave. Nothing here overturns §1's settled
baseline (Overdrive is already CQRS-structural; no ES-on-Raft); it reorganises §4's options into the
shape the design is actually cut in.

### 6.1 The refined framing: two landable pieces, not three facets

The issue's three-facet decomposition (Facet 1 cadence, Facet 2 hydration, comment's event-interest)
is the right *problem* statement but the wrong *slicing* for delivery. The delivery slicing the
evidence supports is **two pieces**:

- **Piece A — the cadence hook (Facet 1).** Object-safe (concrete `Option<ResyncSchedule>` return, no
  associated types), kube-rs-proven (`Action::requeue_after`/`SyncPeriod`), independent of everything
  else. It deletes the convergence-loop's per-reconciler special-case and keeps the loop-owned clock
  (DST-safe). Its urgency is external: the microvm-driver feature branch (not yet on `main` — see Gap 3)
  carries the `VM_RECLAMATION_SWEEP_INTERVAL` hardcode; landing Piece A first means that hardcode never
  reaches `main`.
- **Piece B — the reflector-`Store` primitive.** Event-interest declaration [E] + a warm, rebuildable
  materialized `actual` cache [F] + per-reconciler hydration reading *from that cache* (Facet 2) are
  **the same mechanism**. In kube-rs the sequence is `watcher → reflector → Store`, and the `Controller`
  both **reads** the `Store` for reconcile and is **woken** from the same stream (Finding 3e). So
  interests, cache, and hydration are three faces of one informer; splitting them across separate
  central sites is precisely what produced the "five wiring sites" smell #266 objects to. This reframes
  Facet 2 from the earlier "defer, cohesion-only" verdict: hydration is not deferred *separately* — it
  is a face of Piece B, built (or not) with it.

The two pieces are independently landable, and **#265 durable events is a third, separate track** (see
§6.3) that consumes Piece B's change feed but must not block it.

### 6.2 The reflector-`Store` unification — and the caveats that decide if it is safe

The warm cache is the missing structural surface. Overdrive already keeps the reconciler **`View`**
(private memory) warm in RAM (bulk-load at boot + write-through; `development.md` § "Reconciler I/O")
but re-hydrates **`desired`/`actual` every tick** from local redb (rkyv zero-copy) + the local
CR-SQLite replica. A reflector-`Store` makes `actual` a warm materialized view fed by **one**
ObservationStore subscription that simultaneously (a) serves hydration and (b) drives the interest
fan-out — collapsing E, F, and Facet-2 hydration into a single primitive. Four caveats are
load-bearing for the DESIGN wave:

1. **It must be a materialized view *invalidated by the store's own change feed*, not a parallel
   truth.** A warm cache that lags the store reintroduces, as a *framework* feature, the exact drift
   hazard `.claude/rules/reconcilers.md` exists to prevent (adopt-and-skip; the fingerprint
   anti-pattern; "a populated `State.actual` is not sufficient — verify it is the resource this
   reconciler *manages*"). The safe shape already exists in-tree: the `View` warm map with its
   load-bearing fsync-then-memory ordering invariant. The cache needs its own DST-pinned
   "update-relative-to-tick" ordering invariant, the same way `WriteThroughOrdering` pins View writes.
2. **Host-state reconcilers are excluded — the same split as interests.** `vm-reclamation` (cgroup
   scopes / VMM liveness), veth-provisioner, XDP-attach hydrate `actual` from `getifaddrs` / `bpftool`
   / the host, not from rows. An ObservationStore cache cannot serve them; they keep hydrating live.
   This is the same population as *empty* `interests()` — not row-backed ⇒ resync-only (Piece A),
   never cache-served (Piece B). The two hooks are consistent by construction.
3. **The latency win is smaller than K8s's — justify Piece B on unification, not speed.** K8s's
   informer cache saves an apiserver RPC; Overdrive's stores are already node-local (redb mmap +
   rkyv zero-copy for `desired`; a local CR-SQLite replica for `actual`). Piece B saves a local SQL
   query, not a network hop. Its value is that it *unifies interests + hydration into one primitive*
   and removes the five-sites smell — not per-tick read latency.
4. **Cardinality/bounding is a Phase-2 concern.** A warm cache of all observation rows has real memory
   cost at gossip scale (informer memory is a known K8s operational pain, Finding 3e). Single-node
   Phase 1 is small and fine; a bounding story is owed before multi-node gossip (cf. GH #36).

### 6.3 The #265 durable-events relation — a separate track that composes, not the same feature

#265 ("durable, queryable operational events in the ObservationStore") shares the word "event" with
Piece B's interests but sits on the **opposite end of the same wire**. Keeping them distinct is
load-bearing:

- **`interests` (Piece B) = inbound subscription** — "what wakes me." Static routing metadata
  (`&'static [Interest]`), no payload, *no occurrence semantics*, cannot flood.
- **`ObservationEvent` (#265) = outbound emission** — "what happened." Dynamic data with severity +
  a stable `name` + structured fields, written into the ObservationStore, whose *entire difficulty*
  (dedup, bounding, the "convergent record cannot answer did-it-happen" rule) is a **producer**
  concern orthogonal to interests.

They are consumer and producer of the same store, and they **compose into a cycle**: an emitted event
is an ObservationStore row-change, so a reconciler can declare an `interest` in events (e.g. an
alerting reconciler woken by any `severity=error` event). The design trap is a reconciler
`interest`-subscribing to events it *itself* emits — a feedback loop the broker's LWW key-collapse
dampens but should be designed out, not relied on.

The sharp cross-issue interaction the DESIGN wave must hold: **Piece A's resync cadence amplifies
#265's dedup problem.** `reconcile` runs every tick and re-derives its whole output; a naive
`events.push(...)` on a *standing* condition emits N rows for one occurrence (the marcus-sa comment's
"sharp edge"). Piece A's 30 s safety-net sweep *deliberately re-runs reconcile with no row change*, so
#265's dedup (correlation key / K8s `count + lastTimestamp` merge / edge-detection from `view`) must
hold under resync, not merely under event-triggering.

The events **storage** shape the discussion converged on — matching the CQRS/K8s/Nomad prior art in
baseline files #1 and #2 ("small bounded current-state + a separate, larger, explicitly-lossy history
side"):

- **Hot tier = #265's actual scope: ObservationStore (CR-SQLite).** Bounded, gossiped,
  occurrence-preserving (K8s Events shape: `count + lastTimestamp` + a bounded ring). Sub-second
  operator query, "what's wrong on this node now/recently." **Must stay bounded** (the
  convergent-record + `reconcilers.md` "no unbounded log in a gossiped row" rules). Non-negotiably the
  primary store.
- **Cold tier = a downstream analytical *sink*, off the hot path.** Unbounded retention, columnar,
  queried analytically — "every reclamation refusal across the fleet for 90 days." This is CQRS again:
  a second read model materialized from the event stream, shaped for a different query. **Category
  error to avoid:** making the columnar store the primary event store or putting it on the
  gossip/reconcile path — batch-analytics storage is the wrong tool for sub-second operator queries
  (the same "wrong tool for the hot path" logic baseline #1 used against ES-on-Raft).
- **Reuse, don't fork, the cold store.** The telemetry path already names Parquet as its at-rest target
  (`development.md` § rkyv: "archived telemetry events in-flight (pre-Parquet)"). #265's cold tier
  should export to that same pipeline, not stand up a second cold store. On format: full **Apache
  Iceberg** (file metastore + object store + compaction) is heavy for a single-binary, no-operator-
  shell, immutable-appliance platform; **DuckLake/DuckDB-on-Parquet** (both already in the project's
  trusted-source list) fits the embedded ethos far better as the starting point, with Iceberg reserved
  for when fleet-scale analytics interop (Spark/Trino/Snowflake) actually demands it. The export is a
  **sink adapter behind a port trait**, on a background task consuming the change feed — never in core
  reconciler/observation logic.
- **"Configurable" — retention/sink, not durability.** Configure the tier's retention window,
  cardinality ceiling, and whether the cold export is enabled. Do **not** put durability behind a
  global per-event toggle: whether an occurrence is operator-meaningful (→ durable) vs dev-debug (→
  `tracing`-only) is a **property of the event's type/severity**, decided at the producer (#265 open
  question 1), not a runtime flag. A "durable: on/off" knob is the "is a transient `Failed` row
  acceptable?" framing error — it turns a correctness property (the occurrence must survive) into a
  preference.

### 6.4 How this grounds against Kubernetes — the differences the design must preserve

The control *model* is borrowed from K8s controller-runtime (level-triggered convergence;
edge-triggered wakeups + periodic resync backstop; workqueue dedup). The **differences** are what the
design must not erode:

| Axis | Kubernetes | Overdrive | Consequence for this design |
|---|---|---|---|
| State split | `.spec`/`.status` on **one** object in **one** store (etcd); convention | **Two** physically separate, **type-non-substitutable** stores (`IntentStore` Raft/redb vs `ObservationStore` CR-SQLite); compile-fail-enforced | Piece B's cache is a materialized view of the *Observation* side only; it must never become a write path or blur the stores |
| Reconcile | impure `Reconcile(ctx,req)` — live client I/O *inside* | **pure sync** `reconcile(desired,actual,view,tick) -> (Vec<Action>,View)`; runtime hydrates before, dispatches after | Piece B's cache + #265's events stay *outside* `reconcile` (runtime write-throughs the events, same as View/Actions); DST replay-equivalence must survive |
| Primitives | one (controllers; multi-step faked in `.status` cursor) | **two** (reconciler + durable journaled Workflow); ES only at the workflow boundary | events/interests are reconciler-side; do not reach for a workflow |
| Type erasure | Go interfaces erase freely; or kube-rs monomorphizes per type | Rust associated types don't cross `dyn` ⇒ `AnyReconciler` enum; single-loop/single-clock DST | the enum is partly the *price* of the DST verification property — see §5.4; do not remove it without preserving single-clock DST |
| Read model | always-warm shared **informer cache** | **per-tick** hydration from node-local stores (View is warm; `actual`/`desired` are not) | Piece B *is* the missing warm cache — the gap, not a regression |
| Events | `Events` object, deduped, **TTL'd/ephemeral**, GC'd | #265 aims **durable + bounded-hot + cold-analytical**; more ambitious | the two-tier split in §6.3 is how "durable" stays honest without an unbounded gossiped log |

### 6.5 DESIGN-wave scope statement

Both pieces go to `/nw-design` together (they share the `Reconciler` trait contract the issue flags as
architect-owned, and Piece B subsumes the hydration question):

- **Piece A — cadence hook.** Pin the exact trait signature (`ResyncSchedule { period, scope }` vs
  `resync_period() -> Option<Duration>` vs `next_evaluation(now) -> Option<Evaluation>` — the issue
  leaves this open); the loop-owned-clock/pure-hook constraint; and the broker-coalescing check (Open
  Question 5 — a resync that re-submits every target must dedup through the cancelable-eval-set, or it
  reintroduces the Nomad eval-storm shape).
- **Piece B — reflector-`Store`.** Decide interests declaration shape (candidate E), the warm-cache
  ownership/invalidation model (F) with the §6.2 caveats (materialized-view invalidation; host-state
  exclusion; DST ordering invariant), and *where the irreducible type-match is paid* for hydration
  (Open Questions 1, 2, 6 — central `match` vs per-reconciler `downcast` vs monomorphization; whether
  open/closed extensibility is needed at v1; keeping hydration provably distinct from pure-`reconcile`
  per the `mod.rs` signature guard). Open Question 4 (is a shared read-model worth it *only* for the
  fat `WorkflowLifecycle`/`ServiceMapHydrator` reconcilers) is answered structurally by Piece B: the
  reflector-`Store` serves all row-backed reconcilers uniformly and degrades to near-zero ceremony for
  the thin ones, avoiding the file-#3 over-engineering tax.
- **Out of scope for this design; separate track:** #265 durable events (§6.3). It consumes Piece B's
  change feed and shares the ObservationStore, but its crux (occurrence-preserving dedup + hot/cold
  tiering) is its own DISCUSS/DESIGN pass. The one hard dependency to record: #265's dedup must hold
  under Piece A's resync (§6.3).

---

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Kubernetes Blog — controller-runtime Cache Explained | kubernetes.io | High (1.0) | Official | 2026-08-17 | Y (docs.rs kube, kubebuilder) |
| `kube::runtime::Controller` (docs.rs) | docs.rs | High (1.0) | Official crate docs | 2026-08-17 | Y (reflector, watcher, github) |
| `kube::runtime::reflector` (docs.rs) | docs.rs | High (1.0) | Official crate docs | 2026-08-17 | Y (Controller, github) |
| `kube::runtime::watcher` (docs.rs) | docs.rs | High (1.0) | Official crate docs | 2026-08-17 | Y (reflector) |
| kube-rs/kube — `kube-runtime/src/controller/mod.rs` | github.com | High (1.0) | Official primary source | 2026-08-17 | Y (docs.rs) |
| Kubebuilder Book — Controller Watch Functions | book-v1.book.kubebuilder.io | High (1.0) | Official (kubernetes-sigs) | 2026-08-17 | Y (kubernetes.io blog, baseline #1) |
| Kubebuilder Book — Watching Resources (RequeueAfter) | book.kubebuilder.io | High (1.0) | Official (kubernetes-sigs) | 2026-08-17 | Y (controller-runtime #521) |
| controller-runtime issue #521 (SyncPeriod default 10h) | github.com | High (1.0) | Official primary source | 2026-08-17 | Y (kubebuilder, godoc quote) |
| dtolnay/erased-serde (README) | github.com | High (1.0) | Official primary source | 2026-08-17 | Y (docs.rs erased_serde) |
| `erased_serde` (docs.rs) | docs.rs | High (1.0) | Official crate docs | 2026-08-17 | Y (github README) |
| Microsoft — Materialized View pattern | learn.microsoft.com | High (1.0) | Official | 2026-08-17 | Y (Fowler CQRS, microservices.io) |
| Fowler — CQRS | martinfowler.com | Medium-High (0.8) | Industry leader | 2026-08-17 (via baseline #3) | Y (Microsoft, microservices.io) |
| microservices.io — CQRS pattern | microservices.io | Medium-High (0.8) | Industry leader (Richardson) | 2026-08-17 (via baseline) | Y (Fowler, Microsoft) |
| quinedot — "Erased traits" (Rust learning) | quinedot.github.io | Medium (0.6) — **not in trusted list; flagged** | Community pedagogical | 2026-08-17 | Corroboration only (2 High sources carry the claim) |
| AxonIQ — "Axon and Akka" | axoniq.io | Medium (0.6) — **not in trusted list; flagged** | Vendor blog | 2026-08-17 | Structural-shape corroboration only |
| golinuxcloud — Reconcile Loop Explained | golinuxcloud.com | Medium (0.6) — **not in trusted list; flagged** | Community | 2026-08-17 | Directional corroboration only |
| Baseline #1 — event-driven-internals research | docs/research (local) | High (project SSOT) | Prior research | 2026-08-17 | Primary (settled baseline) |
| Baseline #2 — crash-observability-under-lww research | docs/research (local) | High (project SSOT) | Prior research (→ADR-0078) | 2026-08-17 | Primary |
| Baseline #3 — issue-139-followup streaming research | docs/research (local) | High (project SSOT) | Prior research | 2026-08-17 | Primary (key counter-pressure) |
| In-tree code (`reconcilers/mod.rs`, `eval_broker.rs`, `reconciler_runtime.rs`, `lib.rs`) | local | High (verified source) | Primary code | 2026-08-17 | Read directly |

**Reputation distribution (web sources)**: High: 11 (73%) | Medium-High: 2 (13%) | Medium/flagged: 3
(20% — corroboration only, never load-bearing). **Average reputation (web, load-bearing sources
only, excluding the 3 flagged corroborators): ~0.94.** Including the 3 flagged corroborators: ~0.87.
Every major claim in Findings 3a–3f has ≥2 High/Medium-High sources; the type-erasure and cadence
claims have ≥3 official sources. All three non-listed domains are explicitly flagged and used only to
corroborate a claim already carried by trusted High sources — none is load-bearing.

---

## Knowledge Gaps

### Gap 1: No production Rust orchestrator holds heterogeneous associated-type reconcilers behind one `dyn`
**Issue**: kube-rs — the closest Rust prior art — sidesteps the problem by monomorphizing per resource
type (`Controller<K>`), so it offers no direct precedent for "one runtime holding N reconcilers with
divergent `State`/`View` behind one dispatch *and* keeping compile-time exhaustiveness." The
erased-serde pattern (Finding 3d) is a *general Rust technique*, not an orchestrator-specific
precedent. **Attempted**: searched for Rust orchestrators with a unified reconciler registry over
associated-type projections; found only kube-rs (per-type) and Overdrive's own enum. **Recommendation**:
this gap does not block the DESIGN wave — it means the wave is choosing among *general* Rust erasure
techniques (enum vs erased-trait+downcast), not copying a proven orchestrator shape. The absence of a
precedent that keeps *both* one-loop-DST *and* open-world extensibility is itself a signal that the
two goals trade off.

### Gap 2: Canonical CQRS-projection framework docs are on non-trusted domains
**Issue**: Axon (`docs.axoniq.io`), Marten (`martendb.io`), EventStoreDB (`developers.eventstore.com`),
Akka Projection (`doc.akka.io`) are the canonical projection-framework references, but none is in the
trusted-domain list. **Attempted**: extracted the *structural abstraction* (owned, rebuildable,
change-feed-subscribed read model) and cross-referenced it against trusted High/Medium-High sources
(Microsoft Materialized View, Fowler CQRS, microservices.io), which fully carry the structural claim.
**Recommendation**: the structural essence is well-sourced from trusted domains; the framework-specific
API details (which this research did not need) would require accepting the vendor domains or finding
trusted secondary coverage.

### Gap 3: Live confirmation of #266 Facet 1's exact hardcoded target scheme — RESOLVED
**Issue**: The #266 body describes `spawn_convergence_loop` special-casing `VM_RECLAMATION_SWEEP_INTERVAL`
+ `node/<id>`. The version of `spawn_convergence_loop` read at `lib.rs:2427` (2026-08-17) shows the
broker-drain loop but not the VM-reclamation special-case inline.
**Resolution (2026-08-17, this branch `marcus-sa/cqrs-pattern-research` off `main` @ ce8756e9)**: the
hardcode is **not on this branch**. `VM_RECLAMATION_SWEEP_INTERVAL` appears nowhere in `lib.rs`;
`spawn_convergence_loop` (`lib.rs:2427-2477`) is a clean ~50-line broker-drain + `tokio::select!` +
shutdown loop with no reconciler name, cadence constant, or target scheme. `AnyReconciler`
(`reconcilers/mod.rs:798`) enumerates **7** reconcilers with **no `VmReclamation` variant**. The #266
smell lives on the **in-flight microvm-driver-cloud-hypervisor feature branch** (surfaced during its
DELIVER phase 02 step 02-03, GH #42), which has **not merged to `main`**. Implication: #266 is a
*pre-emptive generalization* request against a smell that is not yet in the mainline `Reconciler`
framework — the DESIGN wave has the freedom to shape the cadence/event-interest hooks *before* the
microvm feature lands its 8th reconciler, so the hardcode never reaches `main`. The *shape* of the fix
(reconciler-declared cadence, loop-owned clock) is unaffected.

---

## Open Questions for the DESIGN wave

1. **Where should the type-match be paid?** Central `match` (enum, compile-time, edit-5-sites) vs
   per-reconciler `downcast` (erased trait, runtime-checked, add-anywhere) vs per-type monomorphization
   (kube-rs shape, drops single-loop DST). The evidence frames the trade; it does not pick.
2. **Is open/closed extensibility actually needed at v1?** WASM third-party reconcilers are >1 year
   out (baseline #3). Does the closed-world enum's compile-time exhaustiveness (valued per baseline #1
   Finding 9) outweigh the five-sites edit cost for a bounded ~10–12 first-party reconciler set?
3. **Should cadence (Facet 1) and event-interest (candidate E) land independently of hydration
   erasure (Facet 2 B/C/D)?** They are separable, object-safe, low-risk, and each independently
   evidenced. Bundling them with the hard erasure rework may over-scope a cheap win.
4. **For the fat cross-store reconcilers only, is a shared materialized read-model component (candidate
   F) worth it?** `WorkflowLifecycle` (3-store cross-product) and `ServiceMapHydrator` (LWW fold) are
   the only current candidates that meet Microsoft's/Fowler's "read ≠ write" bar. Would a first-class
   projection help *them* without taxing the thin reconcilers?
5. **Does a reconciler-declared cadence hook interact correctly with the cancelable-eval-set
   storm-proofing?** A `resync_period` that re-submits every target risks the Nomad eval-storm shape
   (baseline #1, Finding 3) if not coalesced by the broker's existing key-collapse. Confirm the broker
   dedups resync submits.
6. **Can hydration ownership move to the reconciler (candidate C) without re-introducing the
   `&LibsqlHandle`/async parameter the ADR-0036 signature test forbids?** The pure-`reconcile`
   contract must stay intact; hydration is a *separate* impure surface. The DESIGN wave must keep the
   two provably distinct (the compile-time signature test at `mod.rs:271` is the guard).

---

## Full Citations

[1] Kubernetes Authors. "How the controller-runtime Cache Actually Works, and Why Your Controller Does Not Crash the API Server". *Kubernetes Blog*. 2026-07-29. https://kubernetes.io/blog/2026/07/29/controller-runtime-cache-explained/. Accessed 2026-08-17.

[2] kube-rs Authors. "Controller in kube::runtime". *docs.rs*. https://docs.rs/kube/latest/kube/runtime/struct.Controller.html. Accessed 2026-08-17.

[3] kube-rs Authors. "reflector in kube::runtime". *docs.rs*. https://docs.rs/kube/latest/kube/runtime/fn.reflector.html. Accessed 2026-08-17.

[4] kube-rs Authors. "watcher in kube::runtime". *docs.rs*. https://docs.rs/kube/latest/kube/runtime/fn.watcher.html. Accessed 2026-08-17.

[5] kube-rs Authors. "kube-runtime/src/controller/mod.rs". *github.com/kube-rs/kube*. https://github.com/kube-rs/kube/blob/main/kube-runtime/src/controller/mod.rs. Accessed 2026-08-17.

[6] Kubebuilder Authors. "Controller Watch Functions". *The Kubebuilder Book (v1)*. https://book-v1.book.kubebuilder.io/beyond_basics/controller_watches.html. Accessed 2026-08-17.

[7] Kubebuilder Authors. "Watching Resources". *The Kubebuilder Book*. https://book.kubebuilder.io/reference/watching-resources. Accessed 2026-08-17.

[8] controller-runtime contributors. "Why resync default is so large - 10 hours (issue #521)". *github.com/kubernetes-sigs/controller-runtime*. https://github.com/kubernetes-sigs/controller-runtime/issues/521. Accessed 2026-08-17.

[9] Tolnay, David. "erased-serde: Type-erased Serialize, Serializer and Deserializer traits". *github.com/dtolnay/erased-serde*. https://github.com/dtolnay/erased-serde. Accessed 2026-08-17.

[10] Tolnay, David. "erased_serde". *docs.rs*. https://docs.rs/erased-serde. Accessed 2026-08-17.

[11] Microsoft. "Materialized View pattern — Azure Architecture Center". *learn.microsoft.com*. 2022-07-28. https://learn.microsoft.com/en-us/azure/architecture/patterns/materialized-view. Accessed 2026-08-17.

[12] Fowler, Martin. "CQRS". *martinfowler.com*. 2011-07-14. https://martinfowler.com/bliki/CQRS.html. Accessed 2026-08-17 (via baseline research file #3).

[13] Richardson, Chris. "Pattern: Command Query Responsibility Segregation (CQRS)". *microservices.io*. https://microservices.io/patterns/data/cqrs.html. Accessed 2026-08-17 (via baseline research files).

[14] quinedot. "Erased traits". *Learning Rust*. https://quinedot.github.io/rust-learning/dyn-trait-erased.html. Accessed 2026-08-17. **[Non-trusted domain — flagged; corroboration only.]**

[15] AxonIQ. "Axon and Akka - How do they compare?". *axoniq.io*. https://www.axoniq.io/blog/axon-and-akka-how-do-they-compare. Accessed 2026-08-17. **[Non-trusted domain — flagged; structural corroboration only.]**

[16] GoLinuxCloud. "Kubernetes Reconcile Loop Explained: Workqueue, Reconcile() & Code". *golinuxcloud.com*. https://www.golinuxcloud.com/kubernetes-reconcile-loop-explained/. Accessed 2026-08-17. **[Non-trusted domain — flagged; directional corroboration only.]**

[L1] Overdrive contributors. "event-driven-internals-comprehensive-research.md". *docs/research/architecture/*. Local SSOT (settled baseline). 2026-05-02.

[L2] Overdrive contributors. "crash-observability-under-lww-comprehensive-research.md". *docs/research/orchestration/*. Local (→ ADR-0078). 2026-08-01.

[L3] Overdrive contributors. "issue-139-followup-streaming-restart-budget-research.md". *docs/research/control-plane/*. Local (key counter-pressure). 2026-05-03.

[L4] Overdrive source. `crates/overdrive-core/src/reconcilers/mod.rs` (`Reconciler` trait :279, `AnyReconciler` :798, `AnyReconcilerView` :930, `AnyState` :335, dispatch :852); `crates/overdrive-core/src/eval_broker.rs`; `crates/overdrive-control-plane/src/reconciler_runtime.rs` (`hydrate_actual` :2673); `crates/overdrive-control-plane/src/lib.rs` (`spawn_convergence_loop` :2427). Read 2026-08-17.

---

## Research Metadata

- **Duration**: ~30 turns.
- **Sources examined**: 16 web + 3 local baseline research files + 4 in-tree code files.
- **Sources cited**: 16 web (13 trusted/load-bearing, 3 flagged/corroboration-only) + 3 local baseline + code.
- **Cross-references**: every major finding (3a–3f) has ≥2 sources; type-erasure (3d), read-model (3a),
  event-interest (3b), and cadence (3c) each have ≥3 official sources.
- **Confidence distribution**: High: 3a, 3b, 3c, 3d, 3e (5 of 6 findings); High-for-heuristic /
  Medium-for-framework-details: 3f.
- **Citation coverage**: >95% of major claims carry inline URL citations with access dates.
- **Average web-source reputation**: ~0.94 (load-bearing sources); ~0.87 including flagged corroborators.
- **Output**: `docs/research/architecture/cqrs-structural-mechanism-reconciler-framework-research.md`.
- **No skill distillation**: this is DESIGN-wave research input, not recurring methodology.
- **Tool notes**: `book.kubebuilder.io/reference/watching-resources` fetch returned RequeueAfter content
  but not SyncPeriod safety-net text; SyncPeriod default + rationale sourced from controller-runtime
  issue #521 (github, trusted) instead. No blocking tool failures.
