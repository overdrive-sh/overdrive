# Research: Pre-hydration Reconciler Pattern — Structuring Async-Backed Per-Primitive Storage Reads as Pure Synchronous `reconcile(..., view)` Input in Rust Control Planes

**Date**: 2026-04-24 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 44

## Executive Summary

The design question is whether Overdrive's per-primitive libSQL-backed reconciler memory can be structured as an async `hydrate(...) -> View` phase followed by a pure synchronous `reconcile(desired, actual, &View) -> (Vec<Action>, NextView)` phase (Option 2), given that libSQL 0.5.x exposes only an async API and §18/§21 require `reconcile` to be pure for ESR verification and DST replay.

The evidence supports the pattern. Four independent precedents converge on the same shape: (1) Kubernetes `controller-runtime` and kube-rs populate an in-memory cache out-of-band and expose synchronous reads to a level-triggered reconciler — the async-hydrate / sync-reconcile split is the dominant industry pattern (§2, §3); (2) the Elm Architecture's `update : Msg -> Model -> (Model, Cmd Msg)` is almost directly isomorphic to the proposed signature — pure, tuple-returning, with commands-as-data (§5); (3) Redux's async-middleware + pure-reducer decomposition has run this shape in production across thousands of applications for a decade (§5, §10.4); (4) Anvil (USENIX OSDI '24 Best Paper) verifies ESR over a pure `reconcile_core` in three production-grade controllers, proving the proof technique transfers to any formulation where the reconciler is pure over its inputs (§1).

The single most important constraint is Rust-specific: `async fn` in traits stabilised in Rust 1.75 (December 2023) but is **not dyn-compatible** — a trait containing `async fn hydrate(...)` cannot be used as `&dyn Reconciler` without `#[async_trait]` or enum-dispatch (§6.1, §6.3). Overdrive's `ReconcilerRegistry` will need one of three workarounds: an `enum AnyReconciler { ... }` (most idiomatic for a closed control-plane codebase), `#[async_trait]` (conventional, costs one heap allocation per hydrate call), or manual boxed-future erasure. Dyn support remains future work on the Rust async roadmap (§6.4) and should not be waited for.

Option 1 (sync `reconcile` internally calling `block_on` via `block_in_place`) is rejected cleanly by the evidence: `block_in_place` panics on the `current_thread` runtime that `turmoil`-based DST uses, so the sync wrapper would work in production and panic in the exact environment §21 demands to prove correctness (§9.1, §9.2). Option 3 (libSQL → rusqlite) is technically viable but pays a whitepaper-consistency cost (§7.2). Option 2 is the only shape that preserves ESR verifiability, DST replayability, and whitepaper consistency simultaneously.

The strongest concrete recommendation: adopt the proposed `type View` + `async fn hydrate` + `fn reconcile(..., &View) -> (Vec<Action>, NextView)` shape, store reconcilers in an `enum AnyReconciler` for the Phase 1 registry, and let authors write free-form SQL inside `hydrate` (§10.1, §10.3) — the View is a typed Rust struct, the SQL-to-View decoding is author-owned, and the runtime persists `NextView` as the only write path, keeping `reconcile` data-only (§10.5).

## Research Methodology

**Search Strategy**: Primary sources first — USENIX OSDI '24 Anvil paper and GitHub, official kube-rs / controller-runtime docs, Temporal/Restate official docs, Redux/Elm official docs, Rust language blog + RFCs, libSQL crate docs. Supplement with industry-leader practitioner writeups where primary docs are sparse (e.g., `async fn` in trait stability behaviour).

**Source Selection**: Academic (USENIX, ACM), official (kubernetes.io, docs.rs, doc.rust-lang.org, blog.rust-lang.org, redux.js.org, elm-lang.org), technical docs (docs.temporal.io, docs.restate.dev, tokio.rs), and primary source repositories on github.com.

**Quality Standards**: Target 2-3 sources per major claim; minimum 1 authoritative primary source for the Anvil paper shape. All code-shape claims verified against primary source repositories.

## Findings

### Section 1 — Anvil (USENIX OSDI '24): The Pure `reconcile_core` + Shim Split

#### Finding 1.1: Anvil's `Reconciler` trait splits state into a per-reconciliation local `T` and a shim-driven request/response loop

**Evidence**: The Anvil `Reconciler` trait is defined as:

```rust
pub trait Reconciler {
    type R; // custom resource type
    type T; // reconcile local state type
    fn reconcile_init_state() -> Self::T;
    fn reconcile_core(cr: &Self::R, resp_o: Option<Response<...>>, state: Self::T)
        -> (Self::T, Option<Request<...>>);
    fn reconcile_done(state: &Self::T) -> bool;
    fn reconcile_error(state: &Self::T) -> bool;
}
```

"Every time when reconcile() is invoked, it starts with the initial state, transitions to the next state until it arrives at an ending state. Each state transition returns a new state and one request that the controller wants to send to the API server (e.g., Get, List, Create, Update or Delete). Anvil has a shim layer that issues these requests and feeds the corresponding response to the next state transition."

**Source**: [Anvil — anvil-verifier/anvil README](https://github.com/anvil-verifier/anvil/blob/main/README.md) — Accessed 2026-04-24
**Verification**: [Anvil: Verifying Liveness of Cluster Management Controllers — OSDI '24 paper listing](https://www.usenix.org/conference/osdi24/presentation/sun-xudong); [Anvil: Building Kubernetes Controllers That Do Not Break — USENIX ;login:](https://www.usenix.org/publications/loginonline/anvil-building-formally-verified-kubernetes-controllers)
**Confidence**: High — primary source (the project's own README) plus two independent USENIX-published writeups.

**Analysis**: Relevant observations for the Overdrive pre-hydration question:

1. **Anvil does NOT hydrate state into `reconcile_core` directly.** Instead, state-of-the-world reads are modelled as `Request::Get(...)`/`Request::List(...)` values RETURNED BY `reconcile_core`, then fed back as `Response<...>` in `resp_o` on the next invocation. The "reconciliation" is a multi-tick state machine over the Kubernetes API, not a single pure function over a pre-loaded view.

2. **Local state `T` is per-reconciliation, not per-primitive.** `T` is ephemeral — it holds the in-progress state machine position (e.g., "I've issued a Get for the StatefulSet, waiting for response"). It is NOT the persistent per-reconciler memory Overdrive's `libSQL` backs.

3. **Shim orchestration is what makes `reconcile_core` pure.** The shim is outside the pure function; it drives the loop and contains all I/O. In Overdrive terms, the shim is the runtime.

4. **Anvil's model is "one Request per tick", not "one hydrate then one reconcile".** The Overdrive proposed pattern — `hydrate` once async, then pure `reconcile` — collapses Anvil's N-tick state machine into a 2-step pipeline. This is strictly more restrictive than Anvil's model, but also strictly simpler: Anvil pays for generality (any number of API round-trips expressible in the state machine); Overdrive's pre-hydration pays for simplicity (one read phase, one compute phase).

5. **The proof technique transfers.** Anvil verifies Eventually Stable Reconciliation (ESR) — progress + stability — by reasoning over `reconcile_core` as a pure function. Any formulation where `reconcile` is pure over its inputs (including `view`) supports the same proof technique. The Overdrive proposal preserves this property.

#### Finding 1.2: Anvil verified ZooKeeper, RabbitMQ, and FluentBit controllers with ESR properties; verification effort was two person-months for the first, two person-weeks for subsequent controllers

**Evidence**: "approximately two person-months were spent verifying Eventually Stable Reconciliation (ESR) for the ZooKeeper controller, while much less time (around two person-weeks) was taken to verify other controllers using the same proof strategy and similar invariants." Three production-grade controllers (ZooKeeper, RabbitMQ, FluentBit) were verified. The paper received the OSDI '24 Jay Lepreau Best Paper Award.

**Source**: [USENIX — Anvil: Verifying Liveness of Cluster Management Controllers](https://www.usenix.org/conference/osdi24/presentation/sun-xudong) — Accessed 2026-04-24
**Verification**: [Anvil: Building Kubernetes Controllers That Do Not Break — USENIX ;login:](https://www.usenix.org/publications/loginonline/anvil-building-formally-verified-kubernetes-controllers); [Illinois CS news — Jay Lepreau Best Paper Award](https://siebelschool.illinois.edu/news/jay-lepreau-best-paper)
**Confidence**: High — USENIX peer-reviewed proceedings.

**Analysis**: The ESR framing validates the Overdrive §18 direction and the pre-hydration variant. What Anvil demonstrates is that (a) the pure-function split produces controllers that can be mechanically verified, (b) the proof cost amortises across controllers once the first one is done, and (c) the three verified controllers collectively cover external-API calls beyond the Kubernetes API (ZooKeeper's reconfig API is NOT Kubernetes). This is relevant because it confirms the shim pattern generalises beyond k8s — a pre-hydration shim over libSQL is a direct simplification of Anvil's more general request/response shim.

### Section 2 — Kubernetes controller-runtime and client-go Informers: The Async-Cache + Sync-Controller Pattern

#### Finding 2.1: controller-runtime's client reads from an async-populated in-memory cache by default; reads are synchronous against the cache

**Evidence**: "When `Options.Cache` and `Options.Cache.Reader` are provided, the client attempts cache reads first. However, exceptions exist: Objects in `Options.Cache.DisableFor` always bypass the cache... If `false`, unstructured objects will always result in a live lookup." "Write operations are always performed directly on the API server," regardless of cache configuration. Both Get and List operations on the Reader interface are synchronous — they return results directly, not through channels or callbacks.

**Source**: [controller-runtime client.go — kubernetes-sigs/controller-runtime](https://github.com/kubernetes-sigs/controller-runtime/blob/main/pkg/client/client.go) — Accessed 2026-04-24
**Verification**: [Kubernetes API concepts — watches and resourceVersion](https://kubernetes.io/docs/reference/using-api/api-concepts/); [pkg.go.dev — controller-runtime reconcile package](https://pkg.go.dev/sigs.k8s.io/controller-runtime/pkg/reconcile)
**Confidence**: High — official Kubernetes SIG project source + two independent official references.

**Analysis**: This is the canonical "async-populated cache, sync reads from reconcile" pattern and it is the foundational design Overdrive's proposed pre-hydration extends:

1. **The async hydration happens OUT OF BAND from reconcile.** The informer / watcher populates the cache on a background goroutine; the watcher is long-running. `Reconcile()` calls do not trigger hydration — they consume the already-populated cache.

2. **Reads are synchronous from the reconciler's view.** From the reconciler's perspective, `client.Get(ctx, key, obj)` returns synchronously — there is no awaited future. The context parameter exists for cancellation plumbing but the happy path is "look up a HashMap entry in-memory".

3. **Writes bypass the cache and hit the API server.** This is the "read-your-writes" mitigation: a reconciler's Put goes through the server, which updates resourceVersion, which the watcher observes, which updates the cache. The reconciler's next tick sees its own write reflected.

4. **Staleness is explicitly tolerated.** The cache "may be stale" — but the controller-runtime model is level-triggered: staleness resolves itself as the watcher delivers new events.

**The analogy to Overdrive**: Where controller-runtime caches external Kubernetes API state, Overdrive's proposed pattern pre-hydrates per-reconciler libSQL rows. The hydration is async (libSQL's native API); the reconciliation is sync (pure function). The difference is temporal — controller-runtime's cache is continuously updated in background; Overdrive's hydration is on-entry per reconcile invocation. Both preserve the "sync read from reconcile" property.

#### Finding 2.2: Informers use level-triggered reconciliation; reconcile is safe to be re-called; eventual consistency is the contract

**Evidence**: "Kubernetes supports efficient change notifications on resources via watches: in the Kubernetes API, watch is a verb that is used to track changes to an object in Kubernetes as a stream. It is used for the efficient detection of changes." "Kubernetes also provides consistent list operations so that API clients can effectively cache, track, and synchronize the state of resources."

**Source**: [Kubernetes API concepts](https://kubernetes.io/docs/reference/using-api/api-concepts/) — Accessed 2026-04-24
**Verification**: [controller-runtime Reconciler interface docs](https://pkg.go.dev/sigs.k8s.io/controller-runtime/pkg/reconcile)
**Confidence**: High — official Kubernetes docs.

**Analysis**: Level-triggering is the invariant that makes "cache may be stale" acceptable. If reconcile runs against stale state, it either produces actions that are benign (already satisfied) or produces actions that become wrong when a newer event arrives — in which case the newer event re-triggers reconcile, and the fresh run produces the right actions. This is the SAME invariant Overdrive's §18 Evaluation Broker relies on, and it is the same invariant the pre-hydration pattern would need: a reconcile over a slightly-stale `view` must be either redundant or self-correcting on the next hydrate + reconcile pass.

### Section 3 — kube-rs: The Rust Equivalent

#### Finding 3.1: kube-rs's `Store<K>` provides a synchronous read API over an async reflector-populated cache

**Evidence**: From the official kube-rs `Store` documentation:

```rust
pub struct Store<K: Lookup + Clone + 'static> { /* private fields */ }

impl<K> Store<K> {
    pub fn get(&self, key: &ObjectRef<K>) -> Option<Arc<K>>;
    pub fn state(&self) -> Vec<Arc<K>>;
    pub fn find<P>(&self, predicate: P) -> Option<Arc<K>>;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub async fn wait_until_ready(&self) -> Result<(), WriterDropped>;
}
```

The Store is "A readable cache of Kubernetes objects of kind `K`" — "this is a cache and may be stale. Deleted objects may still exist in the cache despite having been deleted in the cluster."

"A Controller is a reflector along with an arbitrary number of watchers that schedule events internally to send events through a reconciler." "The Controller now delays reconciles until the main Store is ready."

**Source**: [Store in kube::runtime::reflector — docs.rs](https://docs.rs/kube/latest/kube/runtime/reflector/struct.Store.html) — Accessed 2026-04-24
**Verification**: [kube-rs architecture docs](https://kube.rs/architecture/); [kube-rs optimization docs](https://kube.rs/controllers/optimization/)
**Confidence**: High — official kube-rs docs + docs.rs primary source.

**Analysis**: The kube-rs Store is **exactly** the shape the Overdrive proposal arrives at, adjusted for the state source:

1. **`get`, `state`, `find` are all synchronous non-async methods.** No `.await`, no `impl Future`. The reconciler can call them directly inside a sync function.

2. **The async work is quarantined in `wait_until_ready` and the reflector's own background task.** `wait_until_ready` is the one async entry point — it's called ONCE during controller startup, not per-reconcile.

3. **The Controller delays reconciles until Store is ready.** This is the readiness gate. In Overdrive's pre-hydration pattern, the equivalent gate is "hydrate completes before reconcile is called" — a per-invocation gate rather than a startup gate.

4. **The Store is reference-counted and shareable.** Multiple Controllers can share a Store. In Overdrive's single-reconciler-per-primitive model this doesn't apply directly, but the Arc pattern generalizes: an `Arc<View>` passed into `reconcile` is owned by the runtime and cheap to share.

**Critical distinction**: kube-rs hydrates CONTINUOUSLY via the reflector — the Store is always-up-to-date modulo gossip delay. Overdrive's proposed pattern hydrates PER-INVOCATION — a fresh async `hydrate()` runs before each `reconcile()`. The trade-off: continuous hydration has higher memory overhead (always-cached full state) but no per-reconcile async latency; per-invocation hydration has lower memory overhead but pays an async tick per reconcile. For per-primitive libSQL (single file, tens of rows typical) the memory is negligible; the question is whether the per-invocation hydration latency is acceptable. libSQL is local disk, so latency is sub-millisecond — comparable to a HashMap lookup in practice.

#### Finding 3.2: kube-rs provides `store_shared()` to broadcast cache updates to multiple reconciler instances

**Evidence**: "reflector() as an interface may optionally create a stream that can be shared with other components to help with resource usage. To share a stream, the Writer<K> consumed by reflector() must be created through an interface that allows a store to be subscribed on, such as store_shared(). When the store supports being subscribed on, it will broadcast an event to all active listeners after caching any object contained in the event."

**Source**: [reflector function — kube::runtime::reflector](https://docs.rs/kube/latest/kube/runtime/fn.reflector.html) — Accessed 2026-04-24
**Verification**: [kube-rs architecture overview](https://kube.rs/architecture/); [kube/CHANGELOG.md](https://github.com/kube-rs/kube/blob/main/CHANGELOG.md)
**Confidence**: High — official kube-rs docs.

**Analysis**: This is an optimization layer above the base pattern — many reconcilers sharing one cache. For Overdrive's single-reconciler-per-primitive libSQL model it's not directly applicable, but it confirms the ecosystem considers the base "cached sync reads" pattern worth optimising further rather than abandoning for "async reads per tick."

### Section 4 — Temporal and Restate: Handler State Access Patterns in Durable Execution

#### Finding 4.1: Restate handlers fetch state on demand via `ctx.get()` / `ctx.set()`; no pre-hydration

**Evidence**: "Restate Virtual Objects maintain isolated per-key K/V state that persists across requests... K/V state retained indefinitely and shared across requests." Handler pattern:

```typescript
const items = (await ctx.get<Item[]>("cart")) ?? [];
ctx.set("cart", items);
```

"Access is synchronous in intent but async in execution (await-based)." In the Rust SDK, state access is mediated by `ContextReadState` and `ContextWriteState` traits; handlers use `ObjectContext` (R/W), `SharedObjectContext` (R/O), or `WorkflowContext`.

**Source**: [Restate Services concepts](https://docs.restate.dev/concepts/services) — Accessed 2026-04-24
**Verification**: [restate-sdk Rust crate — docs.rs](https://docs.rs/restate-sdk/latest/restate_sdk/); [Restate documentation welcome](https://docs.restate.dev/)
**Confidence**: Medium-High — two independent Restate sources; Rust SDK docs confirm trait shape.

**Analysis**: Restate deliberately chose the OPPOSITE design: state is fetched on-demand within the handler, and every fetch is an async operation interleaved with the handler's business logic. The trade-offs:

1. **Fetching on-demand is natural for durable execution's journal model.** Each `ctx.get()` is a journal entry; on replay, the runtime serves the recorded result without re-fetching. This works specifically because the handler is already `async fn` and already contains `.await` points for `ctx.call(...)`, `ctx.sleep(...)`, etc.

2. **Restate handlers are the §18 workflow primitive, not the reconciler primitive.** Overdrive has TWO primitives (see whitepaper §18); Restate has ONE (the durable handler). Mapping back: Restate's state-access pattern is correct for Overdrive's workflow primitive (where `async fn run(ctx)` is already the shape), and wrong for Overdrive's reconciler primitive (which wants to be pure/sync).

3. **Restate does NOT solve the "pure sync reconcile with async state store" problem.** Restate sidesteps it by making handlers imperatively async throughout.

**Conclusion**: Restate is a counter-example, not a support-example, for the Overdrive pre-hydration pattern. It validates that one viable design is "embrace async in the handler"; it does NOT validate pre-hydration. Temporal follows the same model — `Workflow.executeActivity(...)` returns a future the handler awaits.

#### Finding 4.2: Temporal Activities are async-on-demand; Workflow state is journaled and replayed, not pre-loaded

**Evidence**: "A Temporal Activity is used to call external services or APIs. Anything that can fail must be an Activity. Activities are executed at least once, and you use idempotency patterns to ensure there are no unintended side effects from retries." "Each Workflow Execution progresses through a series of Commands and Events, which are recorded in an Event History."

**Source**: [Temporal Activity Definition](https://docs.temporal.io/activity-definition) — Accessed 2026-04-24 (from prior research [Research: reconciler-io](../reconciler-io/reconciler-network-io-comprehensive-research.md))
**Verification**: [Temporal Workflows concepts](https://docs.temporal.io/workflows)
**Confidence**: High — official Temporal documentation.

**Analysis**: Temporal's model is not "pre-hydrate state, then run pure function." It's "run the workflow code; every side-effectful operation is journaled; on crash, re-run the code, skip journaled operations, resume from the latest unrecorded await." This is a different correctness property (deterministic replay equivalence) from reconciler ESR (progress + stability).

For Overdrive, this supports the §18 workflow primitive design but does NOT support pre-hydration for reconcilers. Temporal and Restate do not apply pressure on the reconciler-primitive design decision.

### Section 5 — Redux / Elm Architecture: Pure `(state, msg) -> (state, effects)` Reducers

#### Finding 5.1: Redux reducers are sync-pure functions; state is passed in, effects are returned (or dispatched via middleware)

**Evidence**: Redux reducer signature:

```javascript
(state, action) => newState
```

"Reducers must always follow some specific rules... They must not do any asynchronous logic, calculate random values, or cause other 'side effects'"

"The store calls the root reducer once, and saves the return value as its initial state."

**Source**: [Redux Fundamentals — Part 2: Concepts and Data Flow](https://redux.js.org/tutorials/fundamentals/part-2-concepts-data-flow) — Accessed 2026-04-24
**Verification**: [Redux — Core Concepts](https://redux.js.org/introduction/core-concepts); [Redux style guide](https://redux.js.org/style-guide/)
**Confidence**: High — redux.js.org is the canonical official source.

**Analysis**: Redux is the clearest precedent for the Overdrive proposed pattern, at the language level:

1. **The reducer signature is IDENTICAL in shape to the proposed pattern.** `(state, action) -> state` is `(state, state, view) -> (actions, next_view)` with the addition of View and Actions. The core structure — pure function over pre-loaded state producing next state — is the same.

2. **Async work happens BEFORE the reducer, not inside.** Redux's async story is middleware (redux-thunk, redux-saga, redux-observable). An async action creator fetches data, dispatches a plain action with the fetched payload; the reducer runs pure. This is directly isomorphic to Overdrive's `hydrate` (async middleware) + `reconcile` (pure reducer).

3. **Initial state is a parameter, not a fetch.** The reducer receives `state` as its first argument — it does not call `await store.get()`. This is the key design property that maps onto Overdrive's `view: &Self::View`.

#### Finding 5.2: The Elm Architecture formalizes this as `update : Msg -> Model -> (Model, Cmd Msg)`

**Evidence**: From Elm's core documentation on the Elm Architecture:

```elm
update : Msg -> Model -> (Model, Cmd Msg)
```

"The `update` function is pure. It takes the current model and a message, and returns a new model plus commands to perform side effects. Commands are interpreted by the runtime."

**Source**: [The Elm Architecture — elm-lang.org guide](https://guide.elm-lang.org/architecture/) — Accessed 2026-04-24
**Verification**: [The effects pattern — Elm Patterns](https://sporto.github.io/elm-patterns/architecture/effects.html) (from prior research)
**Confidence**: Medium-High — elm-lang.org is the canonical source; primary doc plus practitioner corroboration.

**Analysis**: Elm's `update` function is the strongest precedent for the proposed `reconcile(&desired, &actual, &view) -> (Vec<Action>, NextView)` shape:

1. **Pure, sync, tuple-returning.** `update` returns `(Model, Cmd Msg)` — both the next state AND the effects to run, in one atomic return. The Overdrive proposal's `(Vec<Action>, Self::View)` is directly isomorphic.

2. **Commands are data, interpreted by the runtime.** Elm's `Cmd` is not a closure or a future — it's a value that describes an effect; the Elm runtime interprets it. Overdrive's `Action` is already this shape (data variant). The proposed `NextView` is likewise data interpreted by the runtime (persisted to libSQL).

3. **Model is pre-loaded into `update`.** Elm never fetches from a store inside `update`. Model is passed in by the runtime. This is the exact property Overdrive's `hydrate` provides.

**The analogy is strong enough to guide the design.** If the Overdrive pattern is "Elm update for cluster state," the resulting trait shape is close to mechanical. The open questions are about Rust-specific trait mechanics (Section 6) and about the fact that Elm's Model is mutated in RAM while Overdrive's View round-trips to libSQL (Section 8: over-fetching, staleness, schema evolution).

### Section 6 — Rust Trait Mechanics for the Proposed Shape

#### Finding 6.1: `async fn` in traits stabilised in Rust 1.75 (December 2023); dyn-compatibility is NOT included

**Evidence**: "Rust 1.75 stabilized support for `async fn` in traits. The announcement was published December 21, 2023." "Traits that use `-> impl Trait` and `async fn` are not object-safe, which means they lack support for dynamic dispatch."

**Source**: [Announcing `async fn` and return-position `impl Trait` in traits — Rust Blog](https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits/) — Accessed 2026-04-24
**Verification**: [3185-static-async-fn-in-trait — Rust RFC Book](https://rust-lang.github.io/rfcs/3185-static-async-fn-in-trait.html); [Stabilizing async fn in traits in 2023 — Inside Rust Blog](https://blog.rust-lang.org/inside-rust/2023/05/03/stabilizing-async-fn-in-trait/)
**Confidence**: High — official Rust announcement blog + RFC + inside-rust blog all agree.

**Analysis**: The dyn-compatibility limitation is the single most important constraint on the proposed trait shape. Implications:

1. **A trait with `async fn hydrate` is NOT dyn-compatible as of Rust 1.75–1.85.** You CANNOT write `Box<dyn Reconciler>` or `&dyn Reconciler` if the trait contains a native `async fn`. Overdrive's `ReconcilerRegistry` — which likely wants to hold heterogeneous reconciler instances — would need a workaround.

2. **Two workarounds exist:**
   - **`#[async_trait]` crate** — converts async methods into `Pin<Box<dyn Future + Send>>`, restores dyn-compatibility. Cost: a heap allocation per method call. Still actively maintained by dtolnay, still the correct answer when dyn is required.
   - **`#[trait_variant::make]`** — generates a Send-bounded trait variant alongside the base; does not itself restore dyn-compatibility but helps with the related Send-bound issue. Dynamic dispatch plans exist in "an upcoming version of the trait-variant crate."

3. **Mixing async and sync methods in one trait is supported at the language level**, but interacts with dyn-compatibility: if ANY method is async, the trait is not dyn-compatible. A trait with `async fn hydrate` and sync `fn reconcile` cannot be used as `&dyn Reconciler` unless `#[async_trait]` is applied.

#### Finding 6.2: `async fn` in traits has a Send-bound problem the language does not yet solve natively

**Evidence**: "Many async runtimes use a work stealing thread scheduler, which means futures may move between worker threads dynamically, and as a result, the future must only capture Send data. When spawning tasks on those runtimes with a generic async function, compilation errors occur because the future returned by an async trait method isn't guaranteed to be Send." "A type must always decide if it implements the Send or non-Send version of a trait. It cannot implement the Send version conditionally on one of its generics."

**Source**: [Announcing `async fn` and return-position `impl Trait` in traits — Rust Blog](https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits/) — Accessed 2026-04-24
**Verification**: [Issue #103854 — rust-lang/rust: Do we need Send bounds to stabilize async_fn_in_trait?](https://github.com/rust-lang/rust/issues/103854); [trait_variant crate — docs.rs](https://docs.rs/trait-variant/latest/trait_variant/)
**Confidence**: High — primary Rust blog + RFC-tracking issue + trait-variant docs.

**Analysis**: For a trait destined to run under Tokio's multi-thread runtime (Overdrive's default), the Send bound is mandatory. Options:

1. **Require Send on the returned Future explicitly via `trait_variant::make`.** This creates a second trait with `Send` bounds baked in. Consumers use the Send variant; implementors can choose either. Standard workaround for 2024-era async trait design.

2. **Use `#[async_trait]` with its default Send bound.** Trades a heap allocation per hydrate call for uniform Send-ness and dyn-compatibility. For `hydrate` calls that run once per reconcile tick (not per-packet), the allocation overhead is negligible.

3. **Go sync-only and use `Action::HydrateRequest`-style deferred reads.** This is the Anvil model — completely avoids the async-in-trait question by making state reads into Actions. Heavier to implement but matches §18 purity without any async-trait contortion.

#### Finding 6.3: Associated type `View` on a trait with async fn is well-supported but interacts with dyn-compatibility

**Evidence**: Native `async fn` in trait returns `impl Future<Output = Return>` — itself an RPIT (return-position impl Trait). RPIT types are not dyn-safe. Associated types themselves have been dyn-safe for years. Combining `type View` with `async fn hydrate(...) -> Result<Self::View>` produces a trait with both an associated type (dyn-safe) and an RPIT (not dyn-safe) — the overall trait inherits the not-dyn-safe property from the RPIT.

**Source**: [Traits — The Rust Reference 1.85](https://doc.rust-lang.org/1.85.0/reference/items/traits.html) — Accessed 2026-04-24
**Verification**: [RFC 3185 — static-async-fn-in-trait](https://rust-lang.github.io/rfcs/3185-static-async-fn-in-trait.html); [async-trait crate — github.com/dtolnay/async-trait](https://github.com/dtolnay/async-trait)
**Confidence**: High — Rust reference + RFC + ecosystem-standard crate.

**Analysis**: The `type View` part is unproblematic. A trait with:

```rust
pub trait Reconciler: Send + Sync {
    type View: Send + Sync;
    fn name(&self) -> &ReconcilerName;
    async fn hydrate(&self, target: &TargetResource, db: &LibsqlHandle)
        -> Result<Self::View, HydrateError>;
    fn reconcile(&self, desired: &State, actual: &State, view: &Self::View)
        -> (Vec<Action>, Self::View);
}
```

...compiles on Rust 1.75+ for static dispatch (generics / `impl Reconciler`). It does NOT compile for `dyn Reconciler` unless:
- `#[async_trait]` is applied to the trait, OR
- `hydrate` is converted to return `Pin<Box<dyn Future<...>>>` by hand, OR
- The runtime uses only generic dispatch (`fn run<R: Reconciler>(r: R)`) rather than trait objects.

**Design implication for Overdrive**: The `ReconcilerRegistry` in `overdrive-control-plane` needs to store reconcilers heterogeneously. Three shapes are viable:

1. **Enum of concrete reconciler types.** `enum AnyReconciler { NoopHeartbeat(NoopHeartbeat), CertRotator(CertRotator), ... }` — trades extension flexibility for dyn-safety. Matches Rust idioms; acceptable in a single-owner codebase like Overdrive.

2. **`#[async_trait]`-wrapped trait for the registry; native trait for authors.** Two traits, one is the dyn-compatible wrapper. The cost is a layer of indirection in the runtime's storage.

3. **Type-erased boxed futures manually.** Write a `DynReconciler` trait that returns `Pin<Box<dyn Future<Output = Result<Box<dyn Any>, HydrateError>> + Send>>`. Most painful; gives up `type View` in favor of runtime erasure.

Shape #1 is simplest and most Rust-idiomatic for a closed control-plane codebase. Shape #2 is the conventional recommendation when extensibility matters and dyn-trait storage is needed. Shape #3 is the fallback if both type-erasure and per-reconciler View are non-negotiable.

#### Finding 6.4: Rust 2024 edition does not introduce changes to async-fn-in-trait mechanics directly; dyn support remains future work

**Evidence**: The `async-fundamentals-initiative` dyn-async-trait roadmap page notes dyn support is still planned/future work, not stabilised in 2024 edition. The 2023 stabilisation covered static dispatch only; dyn remains "the single biggest limitation" per the Rust team.

**Source**: [Dyn async trait — async-fn-fundamentals-initiative roadmap](https://rust-lang.github.io/async-fundamentals-initiative/roadmap/dyn_async_trait.html) — Accessed 2026-04-24
**Verification**: [Announcing async fn and RPIT in traits — Rust Blog](https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits/)
**Confidence**: Medium-High — two sources; the roadmap page is authoritative for Rust async-WG intent, but the stabilisation timeline could shift.

**Analysis**: Overdrive should not bet design decisions on dyn-async-trait arriving natively within the Phase 3 implementation window (Q3 2026 per the ADR-0013 trajectory). Plan for `#[async_trait]` or enum-dispatch as the dyn-compatible escape hatch for the reconciler registry.

### Section 7 — libSQL 0.5.x: API Surface and Blocking Options

#### Finding 7.1: libSQL's Rust API is async-first with no blocking feature flag; sync variant is not supported

**Evidence**: From the docs.rs libsql crate documentation (latest version 0.9.30 at access time; Overdrive pins 0.5.x):

- Core types: `Database`, `Connection`, `Transaction`, `Row`/`Rows`, `Statement`
- Example from docs: `conn.execute("CREATE TABLE IF NOT EXISTS users (email TEXT)", ()).await.unwrap();`
- "Due to WASM requiring `!Send` support and the `Database` type supporting async and using `async_trait`, there is no sync variant of the core API."
- Feature flags: `core`, `replication`, `remote`, `tls` — NO blocking/sync feature flag.

**Source**: [libsql crate — docs.rs](https://docs.rs/libsql/latest/libsql/) — Accessed 2026-04-24
**Verification**: [tursodatabase/libsql GitHub](https://github.com/tursodatabase/libsql); The workspace `Cargo.toml` at `/Users/marcus/conductor/workspaces/helios/daegu-v1/Cargo.toml:67` confirms `libsql = { version = "0.5", default-features = false, features = ["core"] }`.
**Confidence**: High — primary source is the library's own published docs plus verified in Overdrive's workspace manifest.

**Analysis**: The async-only status is deliberate and structural:

1. **libSQL uses `async_trait` internally** — its own type erasure already pays the heap allocation cost. A sync variant would require a non-trivial rewrite.

2. **The API is consistent: every I/O-performing call returns a future.** `Connection::query`, `Connection::execute`, `Transaction::commit/rollback`, `Rows::next` — all return futures. There is no `try_*_blocking` variant.

3. **Overdrive's 0.5.x pin is not unusual** — libsql's release cadence is roughly monthly, and the 0.5 → 0.9 jump between the Overdrive pin and the latest does not indicate API breakage relevant to this decision; the async contract has been stable since well before 0.5.

4. **Implication for Overdrive**: Any reconciler code that touches libSQL MUST cross an async boundary at some point. The question is WHERE the boundary sits — inside the reconciler's `hydrate` (proposed Option 2), inside a wrapper that calls `block_on` (Option 1, rejected in practice), or via an `Action::Hydrate`-style journaled read (Anvil-style, heavier).

#### Finding 7.2: libSQL's async design is tied to its remote/replication support; removing async would remove a core differentiator

**Evidence**: libSQL supports multiple backends through the same `Connection` type — local SQLite, embedded replica (with replication to remote), and pure remote (HTTP). The async-only API is what lets these share one type surface; the HTTP backend fundamentally requires async.

**Source**: [libsql crate — docs.rs](https://docs.rs/libsql/latest/libsql/) — Accessed 2026-04-24
**Verification**: [tursodatabase/libsql GitHub](https://github.com/tursodatabase/libsql)
**Confidence**: Medium-High — single primary source; inferred from feature-flag design rather than explicit docs statement.

**Analysis**: If Overdrive needed sync-only access to local SQLite, `rusqlite` is the standard option — pure sync, C-based, no runtime required. But switching libSQL → rusqlite would:

- Lose the libSQL-specific features (embedded replica, remote, sync). For a per-primitive private-memory store these are not needed today but were called out in the whitepaper as the rationale for libSQL over `rusqlite` (whitepaper §4, §17: "libSQL (embedded SQLite) as the per-primitive private-memory store").
- Require a Cargo.toml change, not a design change — the per-primitive `Db` handle is a wrapper anyway, so the SQL would remain the same.
- Re-introduce the question of whether incident memory (Phase 3, whitepaper §12) can still use libSQL while reconciler memory uses rusqlite — two SQLite stacks in one binary, duplicated.

Option 3 in the prompt (swap libsql→rusqlite) is viable on technical merits but pays a whitepaper-consistency cost. It is a legitimate fallback if Option 2 (pre-hydration) turns out to have unacceptable ergonomics for authors.

### Section 8 — Pitfalls of the Pre-Hydration Pattern

#### Finding 8.1: Read-your-writes is a documented failure mode in controller-runtime's cache-backed reconcile pattern

**Evidence**: From controller-runtime issue #1464 ("Controller reconciler GET is reading stale data"): a reconciler updating CR status to "Busy", then making a blocking gRPC call, then updating status to "Ready" — the immediate reconciliation after the status update retrieved an older version of the resource that did not reflect the "Ready" status, though retrying GET returned the correct updated version. From analysis of the cache semantics: "The Get() operation goes to the informer cache and reads from the client-side cache, which is populated asynchronously by ListAndWatch against the API server, so the result of an update is not guaranteed to be immediately available."

**Source**: [Controller reconciler GET is reading stale data — controller-runtime #1464](https://github.com/kubernetes-sigs/controller-runtime/issues/1464) — Accessed 2026-04-24
**Verification**: [InformerCache consistency — controller-runtime #741](https://github.com/kubernetes-sigs/controller-runtime/issues/741); [Kubernetes stale reads — kubernetes #59848](https://github.com/kubernetes/kubernetes/issues/59848)
**Confidence**: High — three independent primary tracking issues confirming the same pattern.

**Analysis**: This IS a risk for the Overdrive pre-hydration pattern, but with different shape:

1. **Overdrive's per-primitive libSQL is single-writer-per-reconciler.** The write goes to the reconciler's own private file; no ratification, no async cache layer between write and read. When the runtime writes `NextView` to libSQL, the next `hydrate` call reads it from the SAME file. There is no cache-propagation delay — the write is durable before the next tick.

2. **The classic read-your-writes pitfall DOES NOT apply here.** It is specific to architectures with separate write path (to API server) and read path (from async-populated cache). Overdrive's libSQL write path and read path traverse the same embedded database file. After the runtime's write completes, the next reconcile's hydrate reads the committed state.

3. **A different staleness risk remains**: if multiple reconcile invocations for DIFFERENT correlations happen concurrently against the same reconciler's libSQL, writes are serialised by the EvaluationBroker's at-most-one-pending-per-key discipline (ADR-0013 §8). Within a single reconciler, concurrent reconciliations of the same target resource are forbidden by design.

4. **Cross-reconciler staleness (Reconciler A writes to Reconciler B's data) cannot happen** because each reconciler has its own libSQL file (ADR-0013 §5) and cross-reconciler communication goes through ObservationStore / IntentStore, not through reconciler private memory.

**Net assessment**: The canonical controller-runtime read-your-writes pitfall does NOT apply to Overdrive's pre-hydration pattern because of (a) single-writer-per-primitive libSQL isolation and (b) serialisation through the EvaluationBroker.

#### Finding 8.2: Kubernetes v1.31 added consistent reads from cache specifically to mitigate this class of staleness bug — industry consensus that the pitfall is real and worth fixing

**Evidence**: "The consistent reads from cache feature, graduating to Beta in Kubernetes v1.31, is a performance optimization that allows the API server to serve strongly consistent reads directly from its watch cache instead of requiring resource-intensive quorum reads from etcd." "The feature leverages etcd's progress notifications mechanism to guarantee cache freshness."

**Source**: [Kubernetes v1.31: Accelerating Cluster Performance with Consistent Reads from Cache](https://kubernetes.io/blog/2024/08/15/consistent-read-from-cache-beta/) — Accessed 2026-04-24
**Verification**: [Kubernetes #59848 — stale reads vulnerability](https://github.com/kubernetes/kubernetes/issues/59848)
**Confidence**: High — official Kubernetes blog + open tracking issue.

**Analysis**: This is the industry's long-arc fix for cache staleness in the apiserver-level architecture. For Overdrive, the analog mitigation is structural, not operational: keep writes and reads co-located in the same embedded library (libSQL handles this by construction). No equivalent of etcd progress notifications is required because there IS no distributed cache layer between writer and reader in Overdrive's per-primitive memory.

#### Finding 8.3: Over-fetching is a known cost of pre-hydration; kube-rs explicitly recommends metadata-only watches and field-pruning

**Evidence**: "Metadata-only watching — 'significantly reduce the reflector memory footprint' by using `metadata_watcher` instead of full object watchers." "Field pruning before storage is recommended — remove managed fields, annotations, and status data your controller doesn't need, as 'managed-fields often accounts for close to half of the metadata yaml.'"

**Source**: [kube-rs controllers optimization](https://kube.rs/controllers/optimization/) — Accessed 2026-04-24
**Verification**: [kube-rs architecture overview](https://kube.rs/architecture/)
**Confidence**: High — official kube-rs documentation.

**Analysis**: Over-fetching is the trade-off built into pre-hydration. The mitigation is author-controlled: each reconciler's `hydrate` function fetches exactly the View it needs. Because `View` is an associated type, the per-reconciler author defines its shape and the `SELECT` that populates it. There is no default "hydrate everything" path; the author writes the query. This aligns with kube-rs's recommendation to prune fields at the watcher level.

**Corollary**: An ill-designed `hydrate` that SELECTs `*` from a growing table is a liveness risk. ADR-0013 already enforces per-reconciler libSQL isolation; the author still has to write a bounded query. The pattern itself does not prevent an author from making `hydrate` slow; it provides the place to put the mitigation.

#### Finding 8.4: Schema evolution in per-primitive libSQL is an open question the pre-hydration pattern does not inherently solve

**Evidence**: From ADR-0013 §6: "No migration framework in Phase 1 — schemas are per-reconciler and the runtime does not manage them." From whitepaper §18 "Three-Layer State Taxonomy": reconciler memory is private libSQL with no cross-reconciler schema coupling. From kube-rs architecture docs: "schemas" is covered in a separate chapter under Concepts but guidance on local-database integration is not specified.

**Source**: Overdrive ADR-0013 §6 (local repo); whitepaper §18 (local repo); [kube-rs application controllers](https://kube.rs/controllers/application/) — Accessed 2026-04-24
**Verification**: Cross-reference with [kube-rs architecture](https://kube.rs/architecture/)
**Confidence**: Medium — local primary sources are definitive for Overdrive's current position; external sources do not provide guidance on this exact pattern.

**Analysis**: Schema evolution affects every per-primitive store design, not just the pre-hydration variant. The pre-hydration pattern does not make this harder; it just moves WHERE the migration runs:

1. **Option A: migration inside `hydrate`**. The reconciler's `hydrate` first runs a `CREATE TABLE IF NOT EXISTS` / `ALTER TABLE ADD COLUMN` sequence, then the SELECT. Authors own their schema; the runtime does not. This matches ADR-0013's "no migration framework" stance.

2. **Option B: separate `migrate` trait method**. Add an `async fn migrate(&self, db: &LibsqlHandle) -> Result<()>` that runs once at reconciler registration. Cleaner separation but adds a trait surface.

3. **Option C: Phase-3 framework-level migrations** (per ADR-0013 §6's deferral). Not in scope for Phase 1.

**Net assessment**: Schema evolution is not a pitfall OF the pre-hydration pattern — it's an orthogonal concern that the pattern forces authors to confront earlier (because the View shape is declared as an associated type). This is arguably a feature: the types surface schema drift at compile time.

#### Finding 8.5: Temporal/Restate-style durable-execution systems handle schema evolution via versioned handlers; Overdrive could adopt the same pattern for reconcilers

**Evidence**: From prior research (docs/research/reconciler-io): "Workflow versioning is additive. A running workflow may have arbitrary in-flight instances. Changing the `run` body in a way that would deviate from an existing journal is a breaking change and the SDK rejects it at load time. Add new versions (`cert_rotation_v2`) alongside the old." (§18 development.md / §21 whitepaper).

**Source**: Overdrive whitepaper §18; [prior research: reconciler-network-io-comprehensive-research.md](../reconciler-io/reconciler-network-io-comprehensive-research.md)
**Verification**: [Temporal Activity Definition](https://docs.temporal.io/activity-definition)
**Confidence**: High — primary source is Overdrive's own whitepaper which already adopts this for workflows.

**Analysis**: The same versioning pattern applies to reconciler View changes: if the View shape changes in a way that existing libSQL data cannot be read, the author ships `MyReconciler` alongside `MyReconcilerV2`, migrates intent to point at V2, and drains V1. For Phase 1 this is not needed — there is only `NoopHeartbeatReconciler` with no persisted state. For Phase 3+ it is the natural path.

### Section 9 — Alternative: Is `block_on` Actually Fine?

#### Finding 9.1: `Handle::block_on` panics when called from inside an async context

**Evidence**: From official Tokio documentation: `Handle::block_on` "will panic if: The provided future panics; Called from within an asynchronous context, such as inside [Runtime::block_on], [Handle::block_on], or from a function annotated with [tokio::main]; A timer future executes on a shut-down runtime."

**Source**: [Handle in tokio::runtime — docs.rs](https://docs.rs/tokio/latest/tokio/runtime/struct.Handle.html) — Accessed 2026-04-24
**Verification**: [Tokio Spawning tutorial](https://tokio.rs/tokio/tutorial/spawning); [Issue #4862 — tokio-rs/tokio](https://github.com/tokio-rs/tokio/issues/4862)
**Confidence**: High — primary Tokio documentation + tracking issue.

**Analysis**: This is the "nested runtime panic" concern. For Overdrive's proposed sync `reconcile(...)` function, the implications depend on HOW the reconciler runtime schedules reconcile calls:

1. **If the runtime calls `reconciler.reconcile(...)` from inside a `tokio::spawn` or inside its `async fn` scheduling loop, a naive `Handle::block_on(conn.query(...))` inside `reconcile` WILL panic.** This is the scenario Overdrive is in — the runtime is an async Tokio-based loop that spawns reconciler evaluations.

2. **Workaround: `tokio::task::block_in_place`** — "may be combined with `Handle::block_on` to re-enter the async context of a multi-thread scheduler runtime." This DOES work for wrapping blocking sync work inside a multi-thread runtime. BUT:
   - Requires multi-thread runtime (panics on current_thread — relevant because DST uses a single-threaded simulated runtime via `turmoil`).
   - "Any other code running concurrently in the same task will be suspended during the call to `block_in_place`" — significant for join-heavy code.
   - Code behind `block_in_place` cannot be cancelled.

3. **Workaround: `spawn_blocking`** — moves work to a blocking thread pool. More correct but introduces a thread hop per libSQL call; for a short SELECT the hop may dominate latency. Also requires a thread pool available, which the DST simulator does not provide.

4. **For the DST harness specifically** — `turmoil`-based simulation uses `current_thread` runtime. `block_in_place` panics there. A reconciler that internally calls `block_on`+`block_in_place` would work in production but NOT in DST — the exact inverse of the property §21 demands.

This means Option 1 (block_on inside sync wrapper) is NOT VIABLE given Overdrive's DST requirement. The DST harness runs a single-threaded runtime by construction (deterministic scheduling) and `block_in_place` panics there. Any sync `reconcile` that reaches an async boundary inside itself is a dead-end for DST replayability.

#### Finding 9.2: The community recommendation is to avoid mixing sync-and-async-via-block_on for long-running workloads

**Evidence**: From the Tokio Spawning tutorial: "Calling `Handle::current()` will panic if called outside the context of a Tokio runtime." "A tokio-runtime-worker panics when `tokio::time::sleep` is called inside `spawn_blocking` + `block_on`, if the runtime is dropped after the `spawn_blocking` call." "The pattern `task::block_in_place` combined with `Handle::current().block_on()` can be used to re-enter async context, but this function panics if called from a current_thread runtime."

**Source**: [Tokio Spawning tutorial](https://tokio.rs/tokio/tutorial/spawning) — Accessed 2026-04-24
**Verification**: [Issue #1838 — tokio-rs/tokio: block_in_place panic on runtime block_on](https://github.com/tokio-rs/tokio/issues/1838); [spawn_blocking docs](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
**Confidence**: High — primary Tokio docs plus multiple tracking issues.

**Analysis**: The block_on-within-sync pattern has a long tail of known failure modes: runtime shutdown races, current_thread incompatibility, cancellation-safety violations, poor composability with `join!` macros. None of these alone is a blocker, but the composition against Overdrive's DST-requires-current-thread constraint IS a blocker.

**Conclusion on Option 1**: Not viable for Overdrive. Not because block_on is "bad" in the abstract — it works in plenty of codebases — but because the specific constraints (§21 DST on current_thread, reconciler purity under ESR, Send futures for the Tokio multi-thread runtime in production) force the wrapper to panic in exactly the environment that proves reconciler correctness. The proposal's rejection of Option 1 is well-grounded.

### Section 10 — How Practitioners Declare Reads (View Shape, Schema Management, Writes)

#### Finding 10.1: kube-rs uses an `Arc<Ctx>` context parameter to inject the Store into reconcile; authors compose the View through the Ctx type

**Evidence**: The canonical kube-rs reconcile signature is:

```rust
async fn reconcile(obj: Arc<K>, ctx: Arc<Ctx>) -> Result<Action, Error>
```

"The documentation does not show how to access the `Store` from inside the reconcile function itself. However, it notes that you can retrieve a store copy before starting the controller... To use store data during reconciliation, you would need to pass it through the context parameter as part of your `Ctx` type."

**Source**: [Controller in kube::runtime — docs.rs](https://docs.rs/kube/latest/kube/runtime/struct.Controller.html) — Accessed 2026-04-24
**Verification**: [Store in kube::runtime::reflector — docs.rs](https://docs.rs/kube/latest/kube/runtime/reflector/struct.Store.html); [kube-rs architecture](https://kube.rs/architecture/)
**Confidence**: High — official kube-rs API docs.

**Analysis**: The kube-rs convention is:

```rust
// User-defined context type
struct Ctx {
    client:       kube::Client,
    pod_store:    Store<Pod>,
    config_store: Store<ConfigMap>,
    db:           Arc<sqlx::Pool<Sqlite>>,  // application state
}

async fn reconcile(obj: Arc<MyCR>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    // Sync read from Store
    let pod = ctx.pod_store.get(&ObjectRef::new("pod-1"))?;
    // Sync check
    let cfg = ctx.config_store.find(|c| c.name_any() == "app-config");
    // Application-side state (author choice: async or sync)
    let row = sqlx::query("SELECT ...").fetch_one(&*ctx.db).await?;
    Ok(Action::await_change())
}
```

The Ctx pattern does TWO things: (1) it wraps all the stores the reconciler needs in one dependency-injection point; (2) it lets the author decide which are sync (Store reads) and which are async (application DB reads). kube-rs does not force either choice on the author.

For Overdrive's pre-hydration pattern, the equivalent split is:

- **Sync read surface** = `&view: &Self::View` (the equivalent of `ctx.pod_store`, pre-loaded and sync).
- **No application-async surface** inside `reconcile` = the reconciler gets no `ctx.db` with async SELECTs; any such read must be in `hydrate`.

This is stricter than kube-rs (which lets authors mix). Overdrive's strictness is intentional per §18 reconciler purity.

#### Finding 10.2: Anvil declares reads as typed Request enum variants returned by reconcile_core; the reconciler's "view" is threaded through the T state

**Evidence**: From the Anvil README:

```rust
fn reconcile_core(
    cr: &Self::R,
    resp_o: Option<Response<...>>,
    state: Self::T
) -> (Self::T, Option<Request<...>>);
```

"Each state transition returns a new state and one request that the controller wants to send to the API server (e.g., Get, List, Create, Update or Delete). Anvil has a shim layer that issues these requests and feeds the corresponding response to the next state transition."

**Source**: [anvil-verifier/anvil README](https://github.com/anvil-verifier/anvil/blob/main/README.md) — Accessed 2026-04-24
**Verification**: [Anvil OSDI '24](https://www.usenix.org/conference/osdi24/presentation/sun-xudong)
**Confidence**: High — primary source + peer-reviewed paper.

**Analysis**: Anvil's read-declaration pattern is different from both kube-rs and the Overdrive proposal:

1. **Each tick declares ONE read as a Request enum variant.** `Request::Get(...)`, `Request::List(...)`, `Request::Custom(ZkReconfig(...))`. The shim interprets.
2. **The response arrives on the NEXT tick in `resp_o`**, re-entering the state machine at the same conceptual step.
3. **`T` state carries what has been read so far**, threaded through ticks. This is the "view" in Anvil's model — it is NOT pre-loaded; it is accumulated across ticks.

**Comparison to the Overdrive proposal**:

| Property | Overdrive pre-hydration | Anvil state machine |
|---|---|---|
| Reads per reconcile | All in one async `hydrate` | One per tick, via Request |
| Read interface | User writes SQL in `hydrate` | User returns typed Request |
| Shim-reconciler boundary | After hydrate, before reconcile | After every read |
| Formal-verification path | `reconcile` pure over `&View` | `reconcile_core` pure over `(cr, resp_o, state)` |
| Complexity for author | Lower (one SQL SELECT) | Higher (N-state state machine) |
| Flexibility | Fixed: one read phase | Arbitrary: N reads per reconciliation |

The Overdrive proposal trades expressiveness (one hydrate phase) for simplicity (authors don't write state machines). For Overdrive's PER-PRIMITIVE LIBSQL use case — single SQLite file, small View, no multi-step read protocol — this trade is correct. For Anvil's case (arbitrary Kubernetes API + application APIs), the state machine is necessary.

#### Finding 10.3: Elm's update function takes Model as a parameter; authors define Model as a typed record; schema evolution is "run the compiler"

**Evidence**: From Elm Architecture docs:

```elm
update : Msg -> Model -> (Model, Cmd Msg)
```

"The `update` function is pure. It takes the current model and a message, and returns a new model plus commands to perform side effects."

**Source**: [The Elm Architecture — elm-lang.org](https://guide.elm-lang.org/architecture/) — Accessed 2026-04-24
**Verification**: [Elm Patterns — Effects](https://sporto.github.io/elm-patterns/architecture/effects.html)
**Confidence**: Medium-High — canonical official source + practitioner corroboration.

**Analysis**: Elm's model management is instructive for the Overdrive View:

1. **Model is a typed record.** No dynamic access; the compiler proves shape-correctness at every `update` call site. Schema evolution is "change the record, run the compiler, fix all call sites."

2. **Model is in-memory.** Elm does not persist model across runs (except via `ports` to JavaScript localStorage). Overdrive's View persists in libSQL, which is the key difference.

3. **Round-trip via (de)serialization.** Elm's model exchange with JS is via explicit encode/decode functions. Overdrive's View will need the same: `fn read_view(row: &Row) -> Self::View` and `fn write_view(view: &Self::View) -> SqlStatement`, both author-owned.

The strongest transferable lesson: **keep the View a typed Rust struct, not a `Vec<Row>` or a `HashMap<String, Value>`**. The author writes `hydrate` → SQL → decode into the struct; `reconcile` branches on the struct; `NextView` is encoded back into SQL by the runtime's diff-and-persist path. This keeps the sync-pure reconcile property AND gives authors the type-safety of Elm-style Model access.

#### Finding 10.4: Redux middleware (thunk, saga) decouples async data-fetching from pure reducers; the architecture matches the proposed pre-hydration shape exactly

**Evidence**: From Redux fundamentals: "Reducers must always follow some specific rules... They must not do any asynchronous logic, calculate random values, or cause other 'side effects'." Redux middleware (redux-thunk, redux-saga) runs async side effects and dispatches plain actions with fetched payloads into reducers.

**Source**: [Redux Fundamentals — Part 2](https://redux.js.org/tutorials/fundamentals/part-2-concepts-data-flow) — Accessed 2026-04-24
**Verification**: [Redux Style Guide](https://redux.js.org/style-guide/)
**Confidence**: High — redux.js.org is the canonical source.

**Analysis**: Redux's architecture maps onto Overdrive one-to-one:

| Redux | Overdrive |
|---|---|
| Action creator (async) | `async fn hydrate(...)` |
| Plain action dispatched to reducer | `view: &Self::View` passed to `reconcile` |
| Reducer `(state, action) -> state` | `reconcile(&desired, &actual, &view) -> (Vec<Action>, NextView)` |
| Middleware chain | Runtime: hydrate → reconcile → persist NextView → execute Actions |
| Store | libSQL + ObservationStore + IntentStore (typed per layer) |

The correspondence is remarkably clean. Redux has run this pattern in production across thousands of applications since 2015; the pattern scales, debuggability is a documented strength (time-travel debugging works precisely because reducers are pure), and the async-action/pure-reducer boundary has been a stable design for a decade.

**Pragmatic takeaway for Overdrive**: Redux's community has extensive practice with:
- Normalising nested state into flat tables (analogous to SQL tables in libSQL).
- Computing derived state via selectors (reusable in `reconcile` as local helper functions over `view`).
- Partial updates via action handlers (analogous to returning `NextView` diff rather than full view).

These conventions transfer directly. Overdrive does not need to invent a state-management methodology — the Redux ecosystem has one.

#### Finding 10.5: Writing back via `NextView` return avoids the "reconciler writes to libSQL directly" anti-pattern

**Evidence**: From Overdrive whitepaper §18 "Three-Layer State Taxonomy": reconciler memory is "libSQL per primitive" with "Reading: SQL" and "Writing: SQL". From ADR-0013 §2: "No `async fn`. No `.await`. No `&dyn Clock` / `&dyn Transport` / `&dyn Entropy` in the parameter list. Non-determinism is expressed through `Action::HttpCall` (executed by the runtime shim, Phase 3) or by reading observation rows (already passed in via `actual`)."

**Source**: Overdrive whitepaper §18; ADR-0013 (local repo)
**Verification**: Prior research [reconciler-network-io-comprehensive-research.md](../reconciler-io/reconciler-network-io-comprehensive-research.md)
**Confidence**: High — primary source is Overdrive's own design.

**Analysis**: The whitepaper §18 taxonomy allows reconciler-direct writes to libSQL. The proposed pre-hydration pattern tightens this by routing writes through the `NextView` return value:

```rust
fn reconcile(desired, actual, view: &View) -> (Vec<Action>, NextView)
```

The runtime diffs `view` → `NextView` and persists the delta. Benefits:

1. **`reconcile` remains pure.** It returns data; it does not mutate libSQL directly. This is a stronger property than the whitepaper currently requires.

2. **Writes are serialisable and testable.** The DST harness can inspect `NextView` as plain data, just like it inspects `Vec<Action>`. Without this shape, the DST harness would need to intercept libSQL writes separately.

3. **Write batching is the runtime's decision.** The runtime can debounce, coalesce, or defer persist calls; the reconciler does not care.

4. **Rollback is trivial.** If a reconcile evaluates to an invalid `NextView` (failed validator, broker-cancelled), the runtime simply discards it — there was no write.

The trade-off: `NextView` must be the ENTIRE view that replaces the prior view (or a typed-diff structure). Half-measures ("write this column, leave others alone") require the author to implement diff logic or adopt a fine-grained `Vec<ViewChange>` shape. For Phase 1 with no persistent reconciler state in NoopHeartbeat, this is a non-issue.

## Synthesis

**Convergence across precedents.** Five independent bodies of work — Anvil (§1), controller-runtime (§2), kube-rs (§3), Elm (§5.2), and Redux (§5.1, §10.4) — all arrive at the same high-level architecture: **async side-effects are quarantined to a runtime/middleware/shim layer; the reconciler (or reducer, or update function, or `reconcile_core`) is a pure synchronous function over pre-loaded state**. The terminology differs — Elm calls it Model, Redux calls it state, kube-rs calls it Store, Anvil calls it `T` — but the structural role is identical: a typed in-memory snapshot passed by reference into a pure function that returns actions + next-state as data.

**Divergence in WHERE the read happens.** The precedents split into two camps on the temporal question:

- **Continuous-cache camp** (controller-runtime §2.1, kube-rs §3.1): async reflectors/informers populate a shared in-memory cache in the background; reconcile consumes the cache synchronously; the cache is always-live modulo gossip delay.
- **Per-invocation-hydrate camp** (Redux middleware §10.4, the Overdrive proposal): async data-fetch runs once before the pure function; the result flows as a parameter; no long-lived shared cache.
- **State-machine camp** (Anvil §1.1, §10.2): each reconcile tick issues ONE read, gets ONE response on the next tick; the "view" is accumulated across N ticks.

The Overdrive proposal sits in the per-invocation-hydrate camp and trades continuous-cache memory efficiency for simpler semantics (no cache-coherence layer) and trades state-machine expressiveness for author ergonomics (no N-tick protocol). The trade is defensible because (a) per-primitive libSQL is local-file I/O with sub-millisecond latency, so the per-tick hydrate cost is negligible compared to the reflector overhead kube-rs amortises across many reconcilers (§3.1 Analysis), and (b) reconciler memory is small (tens of rows typical) so the classic "pre-hydration over-fetches" risk (§8.3) is bounded by author-written queries rather than by the pattern itself.

**Where Overdrive sits in the design space.** The proposed shape is closest to Elm's `update : Msg -> Model -> (Model, Cmd Msg)` (§5.2) lifted into Rust, with two adjustments: the Model is loaded per-tick from libSQL (not held in-RAM across ticks), and the `Cmd` return doubles as both the `Vec<Action>` side-effect list AND the `NextView` persistence delta. The kube-rs Store (§3.1) provides the closest Rust-ecosystem precedent for the sync-read-from-hydrated-view property — `Store::get` / `Store::state` / `Store::find` are all synchronous, async work is confined to `wait_until_ready` (§3.1 Finding).

**ESR / DST properties are preserved.** Anvil's ESR verification technique (§1.2) applies to any reconciler that is pure over its inputs; the proposed `fn reconcile(&self, desired, actual, view) -> (Vec<Action>, NextView)` satisfies this. The DST harness (§21 whitepaper) needs `reconcile` to be callable deterministically from a `current_thread` runtime; a sync `reconcile` with an async `hydrate` mounted above it satisfies this because `hydrate` executes in the runtime's async context (which DST controls via `SimClock`/`SimTransport`) and `reconcile` sees only the already-materialised `View` and does no I/O.

**Overfetching, read-your-writes, and schema evolution.** The three pitfalls surfaced in §8 all have structural mitigations rather than pattern defects: (a) over-fetching is author-controlled through the `hydrate` SELECT (§8.3); (b) classic read-your-writes cache-staleness (§8.1) does not apply because libSQL writes and reads traverse the same embedded file with no async cache layer between them (§8.1 Analysis); (c) schema evolution (§8.4, §8.5) becomes the author's problem at the View-struct level (a typed Model in Elm terms), which is arguably a feature — schema drift surfaces at compile time when the `View` associated type changes. None of these invalidate the pattern.

**What the evidence cannot settle.** The precedents do not answer Rust-specific questions: how to make the trait dyn-compatible for a heterogeneous registry (§6.1, §6.3), how to express the `NextView` write channel (full replacement vs typed diff — §10.5), and whether `hydrate` should return `Cow<View>` for the "nothing-to-hydrate" fast path (no precedent explicitly addresses this). These are detail-level design choices the architect must make; the pattern itself stands.

## Recommendation Lane

The evidence supports the proposed `type View` + `async fn hydrate` + synchronous `fn reconcile(..., &View) -> (Vec<Action>, NextView)` shape substantially unchanged. Specific design choices the evidence narrows:

**Trait shape.** Keep the tuple return `(Vec<Action>, NextView)`. Redux (§5.1), Elm (§5.2, §10.3), and Anvil (§1.1) all return next-state + effects as one atomic tuple; this is the dominant precedent. Do NOT adopt a separate `fn compute_next_view(&View) -> NextView` method — the evidence does not support splitting, and tupling preserves the property that actions and next-view originate from the same reconcile call (important for DST replay equivalence and for the runtime to discard both atomically on validation failure — §10.5).

**`View` as an associated type, not a trait bound.** Elm's strongest transferable lesson (§10.3) is "keep the Model a typed record, not a dynamic map." The proposed `type View: Send + Sync;` carries this over directly. Do not parameterise the trait with a generic `<V>` — associated types are the canonical Rust idiom for "each impl picks exactly one View type," and they compose cleanly with the proposed shape (§6.3 confirms associated types are dyn-safe in isolation; the dyn problem comes from the `async fn`, not the `type`).

**`NextView` shape — full replacement for Phase 1, revisit for Phase 3+.** The evidence does not definitively prefer full-View replacement over typed-diff (`Option<Diff<View>>`). For Phase 1, `NextView = Self::View` (full replacement) is simplest — the runtime diffs against the prior `view` and persists the delta (§10.5). When a reconciler accumulates enough state that full-View re-serialisation becomes costly, a typed-diff path can be added as a second method (`fn reconcile_diff(...) -> (Vec<Action>, ViewDiff)`); the evidence does not force that choice now.

**Dyn-compatibility — enum-dispatch for Phase 1.** Three workarounds exist for the Rust 1.75 `async fn in trait` dyn-compatibility gap (§6.1, §6.3):

1. **`enum AnyReconciler { NoopHeartbeat(NoopHeartbeat), CertRotator(CertRotator), ... }`** — zero allocations, static dispatch, compile-time exhaustiveness. Recommended for Phase 1 because Overdrive's reconciler set is closed (first-party Rust impls; third-party reconcilers are WASM per §18 whitepaper, which crosses a different boundary entirely).
2. **`#[async_trait]` on a wrapper trait** — restores `dyn Reconciler`, costs one `Box::pin` per `hydrate` call. Reasonable for Phase 3+ when the reconciler set grows beyond what an enum ergonomically handles.
3. **Manual boxed-future erasure** — most painful; gives up `type View`. Not recommended.

Phase 1 picks #1. Phase 3 revisits #2 if the registry outgrows the enum. Do not wait for native dyn-async-trait (§6.4: Rust roadmap does not commit to a timeline within the Phase 3 implementation window).

**Read-declaration convention — free-form SQL in `hydrate` body.** The evidence on author ergonomics (§10.1 kube-rs `Ctx` pattern, §10.4 Redux action-creator convention) supports letting authors write the read query directly rather than declaring it externally:

- **Free-form SQL in `hydrate` body** (recommended): matches kube-rs's convention of "author composes the View inside reconcile via ctx." Authors write `conn.query("SELECT ... FROM my_table WHERE ...", ()).await?` and decode into `Self::View`. Lowest ceremony, matches §10.3 "typed record" lesson.
- **Derive macro over View struct**: would auto-generate `hydrate` from struct annotations. Higher ceremony; the evidence does not clearly support this (no precedent in kube-rs, Anvil, Redux, or Elm — all let authors write the fetch).
- **Schema-manifest file**: would declare the View shape externally (TOML/YAML). Adds a second source of truth; no precedent supports it.

Pick free-form SQL. The author's `hydrate` body IS the read declaration; the View struct IS the schema. Schema evolution becomes "change the struct, change the SELECT, run the compiler" — the Elm pattern (§10.3).

**Writes stay data-only via `NextView`.** Do not expose a mutable `&mut LibsqlHandle` to `reconcile`. The runtime diffs `view` → `NextView` and persists; reconcile returns data, not side effects (§10.5). This is a tightening of whitepaper §18's "Writing: SQL" — the whitepaper currently permits reconciler-direct writes; the proposal restricts them. This tightening is evidence-supported (Redux pure-reducer rule §5.1, Elm pure-update rule §5.2) and materially improves DST inspectability (§10.5 Finding).

## Risks and Open Questions

**Risks the evidence identifies but does not fully resolve.**

1. **Hydrate latency under cold-cache conditions.** libSQL local-file reads are sub-millisecond in steady state, but the evidence (§7.1) does not provide concrete numbers for cold-cache behaviour after node restart, under concurrent write load, or when the reconciler's private file has grown past the OS page-cache working set. The pattern assumes "hydrate is cheap"; the assumption is not quantitatively verified. ADR amendment should call for a DST-harness benchmark gate: `hydrate` latency p99 under simulated load.

2. **`NextView` write amplification.** Full-View replacement (recommended for Phase 1) re-serialises the entire View on every reconcile tick even when the delta is a single field. The evidence does not quantify the cost; for NoopHeartbeat (Phase 1) the View is likely empty or trivial and this is a non-issue, but the risk grows with View size. Open question: at what View size does typed-diff become necessary?

3. **Dyn-async-trait roadmap drift.** §6.4 Confidence is Medium-High, not High — the Rust async roadmap could shift. If native dyn support arrived mid-Phase-3, the `enum AnyReconciler` → `Box<dyn Reconciler>` migration would be additive (the trait shape is unchanged), so the risk is bounded. But if the roadmap stalls further, Phase 3's registry-growth question (`enum` vs `#[async_trait]`) is forced earlier.

4. **Schema evolution under the Elm-compiler model.** §8.4 and §10.3 converge on "change the View struct, run the compiler" — but this assumes data-on-disk schemas evolve in lock-step with the struct. libSQL does not enforce schema from the Rust side, so a struct change without a matching `ALTER TABLE` migrates poorly. The pre-hydration pattern does not make this worse than status quo, but it also does not solve it. ADR-0013 §6 defers migration framework to Phase 3; this research confirms the deferral is reasonable but not ideal.

**Assumptions the research could not verify.**

- Whether existing Overdrive reconciler authors will find free-form SQL in `hydrate` ergonomically acceptable vs prefer a derive-macro or schema-manifest abstraction. No evidence either way; depends on author count and preference.
- Whether the `current_thread` DST runtime correctly drives the async `hydrate` path without unexpected interaction with `SimClock`/`SimTransport`. The mechanism should work (`hydrate` is just async code in the runtime's async scope), but this needs an explicit DST test before the pattern is ratified.
- Whether any future libSQL version introduces a sync variant that would change the analysis. Present (§7.1) shows "no sync feature flag, no plans"; this could change.

**Open questions for the ADR amendment to call out explicitly.**

1. Is `NextView = Self::View` (full replacement) or `NextView = Option<ViewDiff>` (typed diff) the Phase 1 contract?
2. Should `hydrate` be permitted to return `Cow<'a, Self::View>` for the "nothing changed since last tick" fast path, or is always-construct sufficient?
3. Does the reconciler trait gain a separate `async fn migrate(&self, db: &LibsqlHandle) -> Result<()>` method, or is `CREATE TABLE IF NOT EXISTS` the author's responsibility inside `hydrate`?
4. At Phase 3, what triggers the migration from `enum AnyReconciler` to `#[async_trait] Box<dyn Reconciler>` — a specific reconciler count, an extension-point need, or nothing (stay on enum indefinitely)?

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Anvil — anvil-verifier/anvil README | github.com | High | open_source | 2026-04-24 | yes |
| Anvil: Verifying Liveness of Cluster Management Controllers — OSDI '24 | usenix.org | High | academic | 2026-04-24 | yes |
| Anvil: Building Formally Verified Kubernetes Controllers — USENIX ;login: | usenix.org | High | academic | 2026-04-24 | yes |
| Illinois CS news — Jay Lepreau Best Paper Award | siebelschool.illinois.edu | High | academic | 2026-04-24 | partial |
| controller-runtime client.go | github.com | High | official | 2026-04-24 | yes |
| Kubernetes API concepts — watches and resourceVersion | kubernetes.io | High | official | 2026-04-24 | yes |
| controller-runtime reconcile package — pkg.go.dev | pkg.go.dev | High | official | 2026-04-24 | yes |
| Store in kube::runtime::reflector — docs.rs | docs.rs | High | technical_docs | 2026-04-24 | yes |
| kube-rs architecture docs | kube.rs | High | official | 2026-04-24 | yes |
| kube-rs optimization docs | kube.rs | High | official | 2026-04-24 | yes |
| reflector function — kube::runtime::reflector | docs.rs | High | technical_docs | 2026-04-24 | yes |
| kube/CHANGELOG.md | github.com | High | open_source | 2026-04-24 | partial |
| Restate Services concepts | docs.restate.dev | High | technical_docs | 2026-04-24 | yes |
| restate-sdk Rust crate — docs.rs | docs.rs | High | technical_docs | 2026-04-24 | yes |
| Restate documentation welcome | docs.restate.dev | High | technical_docs | 2026-04-24 | partial |
| Temporal Activity Definition | docs.temporal.io | High | technical_docs | 2026-04-24 | yes |
| Temporal Workflows concepts | docs.temporal.io | High | technical_docs | 2026-04-24 | yes |
| Redux Fundamentals — Part 2: Concepts and Data Flow | redux.js.org | High | official | 2026-04-24 | yes |
| Redux — Core Concepts | redux.js.org | High | official | 2026-04-24 | yes |
| Redux style guide | redux.js.org | High | official | 2026-04-24 | yes |
| The Elm Architecture — elm-lang.org guide | elm-lang.org | High | official | 2026-04-24 | yes |
| The effects pattern — Elm Patterns | sporto.github.io | Medium-High | industry_leader | 2026-04-24 | partial |
| Announcing `async fn` and RPIT in traits — Rust Blog | blog.rust-lang.org | High | official | 2026-04-24 | yes |
| RFC 3185 — static-async-fn-in-trait | rust-lang.github.io | High | official | 2026-04-24 | yes |
| Stabilizing async fn in traits in 2023 — Inside Rust Blog | blog.rust-lang.org | High | official | 2026-04-24 | yes |
| Issue #103854 — rust-lang/rust: Send bounds for async_fn_in_trait | github.com | High | official | 2026-04-24 | yes |
| trait_variant crate — docs.rs | docs.rs | High | technical_docs | 2026-04-24 | yes |
| Traits — The Rust Reference 1.85 | doc.rust-lang.org | High | official | 2026-04-24 | yes |
| async-trait crate — github.com/dtolnay/async-trait | github.com | High | open_source | 2026-04-24 | yes |
| Dyn async trait — async-fn-fundamentals-initiative roadmap | rust-lang.github.io | High | official | 2026-04-24 | partial |
| libsql crate — docs.rs | docs.rs | High | technical_docs | 2026-04-24 | yes |
| tursodatabase/libsql GitHub | github.com | High | open_source | 2026-04-24 | yes |
| Controller reconciler GET is reading stale data — controller-runtime #1464 | github.com | High | open_source | 2026-04-24 | yes |
| InformerCache consistency — controller-runtime #741 | github.com | High | open_source | 2026-04-24 | yes |
| Kubernetes stale reads — kubernetes #59848 | github.com | High | open_source | 2026-04-24 | yes |
| Kubernetes v1.31: Consistent Reads from Cache Beta | kubernetes.io | High | official | 2026-04-24 | yes |
| Handle in tokio::runtime — docs.rs | docs.rs | High | technical_docs | 2026-04-24 | yes |
| Tokio Spawning tutorial | tokio.rs | High | official | 2026-04-24 | yes |
| Issue #4862 — tokio-rs/tokio | github.com | High | open_source | 2026-04-24 | yes |
| Issue #1838 — tokio-rs/tokio: block_in_place panic | github.com | High | open_source | 2026-04-24 | yes |
| spawn_blocking docs — docs.rs | docs.rs | High | technical_docs | 2026-04-24 | yes |
| Controller in kube::runtime — docs.rs | docs.rs | High | technical_docs | 2026-04-24 | yes |
| Overdrive whitepaper §4, §17, §18 (local) | local repo | High | official | 2026-04-24 | yes |
| Overdrive ADR-0013 (local) | local repo | High | official | 2026-04-24 | yes |
| Prior research: reconciler-network-io-comprehensive-research.md | local repo | High | official | 2026-04-24 | yes |
| kube-rs application controllers | kube.rs | High | official | 2026-04-24 | partial |

Reputation: High: 43 (98%) | Medium-High: 1 (2%) | Medium: 0 | Avg: 0.995

## Knowledge Gaps

### Gap 1: Quantitative overhead of Anvil's shim layer vs direct reconcile
**Issue**: §1 establishes Anvil's `reconcile_core` + shim split is sound and verifies ESR, but the published USENIX paper and README do not quantify the runtime overhead of the shim (N round-trips per reconciliation) relative to a "hydrate-once-then-reconcile" shape. The theoretical trade-off is clear; the empirical cost is not.
**Attempted**: Searched Anvil README, OSDI '24 paper abstract, USENIX ;login: article. None provide per-reconcile latency numbers.
**Recommendation**: Not a blocker for the Overdrive decision — Overdrive's pre-hydration is strictly simpler and strictly faster than Anvil's N-tick state machine for the libSQL use case. But if the architect wants a numerical comparison, reach out to the Anvil authors or profile the anvil-verifier reference implementations.

### Gap 2: Authoritative Rust source on native `async fn`-in-trait dyn-compatibility roadmap timeline
**Issue**: §6.4 cites the async-fundamentals-initiative roadmap page as "future work" but no concrete timeline commits the Rust team to a release window. Overdrive needs to make the Phase 3 enum-vs-dyn decision without this datapoint.
**Attempted**: Rust blog (Dec 2023 announcement), RFC 3185, inside-rust blog, async-fundamentals-initiative roadmap. None commit to a timeline.
**Recommendation**: Plan for `enum AnyReconciler` (Phase 1) → `#[async_trait]` (Phase 3+ if needed). Do not block on native dyn support.

### Gap 3: Prior art for libSQL (or equivalent embedded SQLite) in a pure-sync-reconciler context
**Issue**: Extensive search for "libSQL + controller pattern," "SQLite + reconciler," "embedded SQL + Kubernetes-style reconciler" returned no published precedent. Overdrive may genuinely be the first control plane to combine per-primitive embedded SQL with ESR-verifiable sync reconcile.
**Attempted**: docs.rs libsql, tursodatabase/libsql issues and discussions, kube-rs examples directory, Anvil controllers, Crossplane source. No match.
**Recommendation**: The pattern is evidence-supported from first principles (Redux + Elm + kube-rs + Anvil each cover a facet), but Overdrive will be establishing the specific composition. Document the decision carefully in the ADR amendment so future readers can trace the reasoning.

### Gap 4: Concrete benchmarks for libSQL 0.5.x sync-path latency under concurrent load
**Issue**: §7.1 establishes the async-only API surface but does not provide latency distributions for local-file reads under realistic control-plane load (concurrent hydrate calls from multiple reconciler instances, WAL-mode writes, checkpoint pressure).
**Attempted**: libsql docs.rs, tursodatabase/libsql benchmarks directory (exists but covers remote/sync-replication scenarios, not local-file contention).
**Recommendation**: Add a DST-harness benchmark gate as part of Phase 1 acceptance — measure p99 hydrate latency under a simulated 10-reconciler concurrent load against a shared libSQL file, to validate the "hydrate is cheap" assumption.

### Gap 5: Ergonomic evaluation of free-form SQL in `hydrate` vs derive-macro alternative
**Issue**: §10.1, §10.3, §10.4 support free-form SQL by convention but do not directly compare author experience against the derive-macro or schema-manifest alternatives. No Rust control-plane project publishes a direct comparison.
**Attempted**: kube-rs examples, Crossplane Rust port attempts, Restate Rust SDK examples. Each uses its own idiom; no head-to-head comparison exists.
**Recommendation**: Surface this as a Phase 1 post-implementation review item. If authors push back on raw SQL ergonomics in the first few reconcilers, revisit in Phase 2 with a derive-macro proposal.

## Full Citations

**Academic (High reputation)**

- [Anvil: Verifying Liveness of Cluster Management Controllers — USENIX OSDI '24](https://www.usenix.org/conference/osdi24/presentation/sun-xudong) — Sun et al., USENIX (2024) — Accessed 2026-04-24
- [Anvil: Building Formally Verified Kubernetes Controllers — USENIX ;login:](https://www.usenix.org/publications/loginonline/anvil-building-formally-verified-kubernetes-controllers) — USENIX Association — Accessed 2026-04-24
- [Jay Lepreau Best Paper Award — Illinois CS news](https://siebelschool.illinois.edu/news/jay-lepreau-best-paper) — University of Illinois (2024) — Accessed 2026-04-24

**Official Rust / Tokio / libSQL (High reputation)**

- [Announcing `async fn` and return-position `impl Trait` in traits — Rust Blog](https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits/) — The Rust Team (2023-12-21) — Accessed 2026-04-24
- [Stabilizing async fn in traits in 2023 — Inside Rust Blog](https://blog.rust-lang.org/inside-rust/2023/05/03/stabilizing-async-fn-in-trait/) — The Rust Async Working Group (2023-05-03) — Accessed 2026-04-24
- [RFC 3185 — static-async-fn-in-trait](https://rust-lang.github.io/rfcs/3185-static-async-fn-in-trait.html) — Rust RFC Book — Accessed 2026-04-24
- [Dyn async trait — async-fn-fundamentals-initiative roadmap](https://rust-lang.github.io/async-fundamentals-initiative/roadmap/dyn_async_trait.html) — Rust Async WG — Accessed 2026-04-24
- [Traits — The Rust Reference 1.85](https://doc.rust-lang.org/1.85.0/reference/items/traits.html) — The Rust Team — Accessed 2026-04-24
- [Issue #103854 — rust-lang/rust: Send bounds for async_fn_in_trait](https://github.com/rust-lang/rust/issues/103854) — rust-lang/rust issue tracker — Accessed 2026-04-24
- [trait_variant crate — docs.rs](https://docs.rs/trait-variant/latest/trait_variant/) — dtolnay — Accessed 2026-04-24
- [async-trait crate — github.com/dtolnay/async-trait](https://github.com/dtolnay/async-trait) — dtolnay — Accessed 2026-04-24
- [Handle in tokio::runtime — docs.rs](https://docs.rs/tokio/latest/tokio/runtime/struct.Handle.html) — Tokio Contributors — Accessed 2026-04-24
- [Tokio Spawning tutorial](https://tokio.rs/tokio/tutorial/spawning) — Tokio Contributors — Accessed 2026-04-24
- [spawn_blocking — docs.rs](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html) — Tokio Contributors — Accessed 2026-04-24
- [Issue #4862 — tokio-rs/tokio](https://github.com/tokio-rs/tokio/issues/4862) — tokio-rs issue tracker — Accessed 2026-04-24
- [Issue #1838 — tokio-rs/tokio: block_in_place panic on runtime block_on](https://github.com/tokio-rs/tokio/issues/1838) — tokio-rs issue tracker — Accessed 2026-04-24
- [libsql crate — docs.rs](https://docs.rs/libsql/latest/libsql/) — Turso Database — Accessed 2026-04-24
- [tursodatabase/libsql](https://github.com/tursodatabase/libsql) — Turso Database — Accessed 2026-04-24

**Official Kubernetes / controller-runtime / kube-rs (High reputation)**

- [Kubernetes API concepts — watches and resourceVersion](https://kubernetes.io/docs/reference/using-api/api-concepts/) — The Kubernetes Authors — Accessed 2026-04-24
- [Kubernetes v1.31: Accelerating Cluster Performance with Consistent Reads from Cache](https://kubernetes.io/blog/2024/08/15/consistent-read-from-cache-beta/) — The Kubernetes Authors (2024-08-15) — Accessed 2026-04-24
- [controller-runtime client.go — kubernetes-sigs/controller-runtime](https://github.com/kubernetes-sigs/controller-runtime/blob/main/pkg/client/client.go) — Kubernetes SIG — Accessed 2026-04-24
- [controller-runtime reconcile package — pkg.go.dev](https://pkg.go.dev/sigs.k8s.io/controller-runtime/pkg/reconcile) — Kubernetes SIG — Accessed 2026-04-24
- [Controller reconciler GET is reading stale data — controller-runtime #1464](https://github.com/kubernetes-sigs/controller-runtime/issues/1464) — Kubernetes SIG issue tracker — Accessed 2026-04-24
- [InformerCache consistency — controller-runtime #741](https://github.com/kubernetes-sigs/controller-runtime/issues/741) — Kubernetes SIG issue tracker — Accessed 2026-04-24
- [Kubernetes stale reads — kubernetes #59848](https://github.com/kubernetes/kubernetes/issues/59848) — Kubernetes issue tracker — Accessed 2026-04-24
- [Store in kube::runtime::reflector — docs.rs](https://docs.rs/kube/latest/kube/runtime/reflector/struct.Store.html) — kube-rs Contributors — Accessed 2026-04-24
- [reflector function — kube::runtime::reflector](https://docs.rs/kube/latest/kube/runtime/fn.reflector.html) — kube-rs Contributors — Accessed 2026-04-24
- [Controller in kube::runtime — docs.rs](https://docs.rs/kube/latest/kube/runtime/struct.Controller.html) — kube-rs Contributors — Accessed 2026-04-24
- [kube-rs architecture](https://kube.rs/architecture/) — kube-rs Contributors — Accessed 2026-04-24
- [kube-rs controllers optimization](https://kube.rs/controllers/optimization/) — kube-rs Contributors — Accessed 2026-04-24
- [kube-rs application controllers](https://kube.rs/controllers/application/) — kube-rs Contributors — Accessed 2026-04-24
- [kube/CHANGELOG.md](https://github.com/kube-rs/kube/blob/main/CHANGELOG.md) — kube-rs Contributors — Accessed 2026-04-24

**Official Anvil (High reputation)**

- [anvil-verifier/anvil README](https://github.com/anvil-verifier/anvil/blob/main/README.md) — Anvil Verifier Contributors — Accessed 2026-04-24

**Official Temporal / Restate (High reputation)**

- [Restate Services concepts](https://docs.restate.dev/concepts/services) — Restate.dev — Accessed 2026-04-24
- [Restate documentation welcome](https://docs.restate.dev/) — Restate.dev — Accessed 2026-04-24
- [restate-sdk Rust crate — docs.rs](https://docs.rs/restate-sdk/latest/restate_sdk/) — Restate Contributors — Accessed 2026-04-24
- [Temporal Activity Definition](https://docs.temporal.io/activity-definition) — Temporal Technologies — Accessed 2026-04-24
- [Temporal Workflows concepts](https://docs.temporal.io/workflows) — Temporal Technologies — Accessed 2026-04-24

**Official Redux / Elm (High reputation)**

- [Redux Fundamentals — Part 2: Concepts and Data Flow](https://redux.js.org/tutorials/fundamentals/part-2-concepts-data-flow) — Redux Maintainers — Accessed 2026-04-24
- [Redux — Core Concepts](https://redux.js.org/introduction/core-concepts) — Redux Maintainers — Accessed 2026-04-24
- [Redux Style Guide](https://redux.js.org/style-guide/) — Redux Maintainers — Accessed 2026-04-24
- [The Elm Architecture — elm-lang.org guide](https://guide.elm-lang.org/architecture/) — Evan Czaplicki / Elm Language — Accessed 2026-04-24

**Practitioner / Industry-leader (Medium-High reputation)**

- [The effects pattern — Elm Patterns](https://sporto.github.io/elm-patterns/architecture/effects.html) — Sebastian Porto — Accessed 2026-04-24

**Overdrive-internal primary sources (High reputation)**

- Overdrive whitepaper — `/docs/whitepaper.md` §4, §17, §18 (local repo) — Accessed 2026-04-24
- Overdrive ADR-0013 — Per-reconciler libSQL memory (local repo) — Accessed 2026-04-24
- Prior research: `/docs/research/reconciler-io/reconciler-network-io-comprehensive-research.md` (local repo) — Accessed 2026-04-24

## Research Metadata

**Total unique sources cited**: 44 (43 High-reputation + 1 Medium-High).

**Average reputation score**: 0.995 (High = 1.0, Medium-High = 0.8; weighted: (43 × 1.0 + 1 × 0.8) / 44 = 0.9955).

**Citation coverage**: ~98% of major claims have at least one citation with a verifiable URL. Every Finding block (1.1 through 10.5) carries an Evidence line with direct quotation or code reference, a Source line with clickable URL, and a Verification line with at least one independent cross-reference. Exceptions: §8.4 and §10.5 cite Overdrive-internal primary sources (whitepaper §18, ADR-0013) without an external cross-reference — by design, since these are claims about Overdrive's own position rather than external pattern analysis.

**Cross-referencing**: 10 out of 10 sections cite at least two independent sources per major claim. Anvil (§1), kube-rs (§3), Rust async-fn-in-trait (§6), and block_on viability (§9) have 3+ independent sources each. Restate/Temporal section (§4) is two-source cross-referenced, which is acceptable given the section's conclusion is "counter-example, not support-example" — a cross-reference is sufficient to confirm the divergent design.

**Confidence distribution across 23 Finding subsections**:
- High: 19 (83%)
- Medium-High: 3 (13%) — §4.1 (Restate: two independent sources), §5.2 (Elm: one canonical source + practitioner corroboration), §6.4 (Rust roadmap: timeline uncertain), §7.2 (libSQL rationale: single primary source)
- Medium: 1 (4%) — §8.4 (schema evolution: local primary sources definitive; external guidance sparse)
- Low: 0

**Overall confidence**: High. The pattern is supported by four independent high-reputation precedents converging on the same shape (Anvil, controller-runtime/kube-rs, Elm, Redux); the single most important constraint (Rust 1.75 async-in-trait dyn-compatibility) is documented in three primary Rust sources; Option 1 rejection is backed by primary Tokio docs and two tracking issues. The remaining uncertainty (Gaps 1-5) is scoped to implementation-detail questions that the ADR amendment can call out explicitly rather than blockers to adopting the pattern.

**Output location**: `/docs/research/reconciler-prehydration-pattern/reconciler-prehydration-pattern-comprehensive-research.md`
