# Feature Delta — `reconciler-framework-improvements` (GH #266)

Two landable pieces on the reconciler framework: **Piece A** — a per-reconciler
resync cadence hook (level-triggered safety net beside the edge-triggered
broker); **Piece B** — a reflector-`Store` warm materialized `actual` cache that
unifies event-interest declaration + hydration into one primitive.

This delta has **two authors**. The section below is the **SYSTEM** layer
(Titan / `nw-system-designer`): cache/scheduling/bounding mechanics, capacity
math, the broker-coalescing verdict, the DST ordering invariant, and the
system constraints the trait design must honour. `nw-solution-architect` (Morgan)
appends the **APPLICATION** layer after this — the exact `Reconciler` trait
surface, the `AnyReconciler`/`AnyState` erasure resolution, component boundaries,
and the ADR amendments. Nothing below locks a trait signature; where the system
design constrains a signature, it says so and hands the signature to Morgan.

---

## Wave: DESIGN / [SYSTEM] System Architecture (Titan)

### Prior-wave consultation checklist

There is no DISCUSS wave for this feature; the requirements source is the
research doc §6 (SSOT for scope) + the two GH issues + the ADRs/code anchors.

- ✓ `docs/research/architecture/cqrs-structural-mechanism-reconciler-framework-research.md` — read in full; §6 (6.1 A/B slicing, 6.2 caveats, 6.3 #265 coupling, 6.4 K8s-differences, 6.5 scope) is the scope SSOT.
- ✓ GH #266 — issue + both comments (event-interest comment; the two-pieces re-scope comment). Fetched via `gh issue view 266 --comments`.
- ✓ GH #265 — the reconciler-side `events` output-channel comment + the sharp-edge dedup note. Fetched via `gh issue view 265 --comments`. Confirmed OUT OF SCOPE (one inherited constraint recorded, §10).
- ✓ `docs/product/architecture/brief.md` — reconciler-runtime §24–28 (`AnyState`, hydrate match-dispatch, ActionShim, tick cadence); System Architecture section (owned by Titan) confirmed as the append target.
- ✓ ADR-0036 — read and confronted. It *explicitly decided AGAINST* a per-reconciler `hydrate(target, db)` surface (runtime owns all hydration). Piece B revisits *where hydration reads from* (a warm cache vs per-tick store read), not *who owns the async surface* — §5.6 reconciles the two.
- ✓ ADR-0035 (View/redb persistence; fsync-then-memory `WriteThroughOrdering`) — the invariant Piece B's cache invariant mirrors.
- ✓ ADR-0021 (`AnyState` per-reconciler typed projection) — the shape Piece B's cache feeds.
- ✓ ADR-0013 (reconciler runtime) — via ADR-0035/0036 lineage.
- ✓ ADR-0079 (`backend-discovery-bridge-converges-on-the-rows-it-manages`) — the canonical "materialized view of the rows you MANAGE, read them back; never a hash of what you emitted" precedent. Directly governs Piece B's invalidation model (§5.2).
- ✓ `crates/overdrive-core/src/eval_broker.rs` — the cancelable-eval-set broker; `submit`/`drain_pending`/`reap_cancelable`; LWW key-collapse on `(ReconcilerName, TargetResource)`. Traced for Open Question 5 (§4.3).
- ✓ `crates/overdrive-control-plane/src/reconciler_runtime.rs` — hydrate→reconcile→dispatch (~1520–1598), self-re-enqueue at :1587, `hydrate_actual` match-dispatch (~2673).
- ✓ `crates/overdrive-control-plane/src/lib.rs:2427` — `spawn_convergence_loop` (clean ~50-line broker-drain, loop-owned clock via `clock.sleep(cadence)`); confirmed **no** `VM_RECLAMATION_SWEEP_INTERVAL` on this branch (research Gap 3).
- ✓ `crates/overdrive-core/src/reconcilers/mod.rs` — `Reconciler` trait (:279), pure-sync signature guard (:271), `AnyState` (:335), `AnyReconciler` (:798), `TargetResource` + `CANONICAL_TARGET_PREFIXES` (:732, prefixes `workload/ node/ alloc/ service/ workflow/`).
- ✓ `crates/overdrive-core/src/traits/observation_store.rs` — the **watcher already exists**: `subscribe_all_events() -> LagAwareSubscription` (:1896) yielding `SubscriptionEvent::{Row, Lagged{missed}}` (:1740) with the etcd-`ErrCompacted`/k8s-reflector-`Gone` relist contract; `all_service_backends_rows` (:2135); line 1774 names the Phase-2 `prefix`/predicate filter as the future bounding knob.

---

### 0. Scope fence

**In scope (SYSTEM):** the scheduling mechanics of the cadence hook (Piece A);
the warm-cache system design — invalidation model, DST ordering invariant,
host-state exclusion partition, cardinality-bounding story (Piece B). The
broker-coalescing behaviour (Open Question 5). The `LocalNode → node/<id>` scope
resolution.

**Explicitly deferred to `nw-solution-architect`:** the exact `Reconciler` trait
method signatures; the `AnyReconciler`/`AnyState`/`AnyReconcilerView` erasure
resolution (central `match` vs erased-trait `downcast` vs per-type
monomorphization — research Open Questions 1/2/6); component/module boundaries;
ADR amendments to ADR-0021/0035/0036. This delta hands Morgan the **system
constraints** those decisions must satisfy, not the decisions.

**Out of scope entirely:** GH #265 durable `ObservationEvent` design (one
inherited constraint recorded, §10).

---

### 1. Back-of-envelope estimation

Numbers keep the "cache" claim honest. Assumptions stated; rounded to order of
magnitude.

**Reconciler cardinality.** 7 registered today; whitepaper full built-in set
~10–12. `O(reconcilers)` is small and bounded — this is the number that makes a
central-`match` erasure *cheap* (a counter-pressure Morgan weighs, not a system
blocker).

**Row cardinality & warm-cache RAM.**

| Row family | Key | Phase-1 single-node count | Est. row size (rkyv) | Warm subtotal |
|---|---|---|---|---|
| `alloc_status` | `AllocationId` | ≤ ~1000 allocs/node | ~500 B (incl. `LastTerminated` depth-1 snapshot) | ~500 KB |
| `service_backends` | `ServiceId` | ~100–1000 services | ~300 B + N·64 B backends | ~0.3–1 MB |
| `service_hydration_results` | `ServiceId` | ~100–1000 | ~300 B | ~0.3 MB |
| `node_health` | `NodeId` | **1** (single-node) | ~300 B | ~0.3 KB |
| `workflow_terminal`, audit rows | corr / serial | ~100s | ~200 B | ~0.1 MB |

**Phase-1 warm cache ≈ 1–5 MB for a busy node.** Negligible against the
process's existing redb mmap + working set. → **Caveat 3 confirmed: the cache
saves a *local SQL query*, not a network hop. Do not justify Piece B on
latency.** Its value is unification (one subscription serves interests +
hydration, collapsing the five wiring sites).

**Phase-2 gossip scale — the bounding cliff.** The ObservationStore is a
CR-SQLite *full local replica* — every node already holds the whole cluster's
gossiped rows *on disk*. The incremental cost of Piece B is **RAM for the warm
materialized subset**. Worst case, a naïve reflector-Store that materializes the
*entire* replica:

```
1,000 nodes × 100 allocs/node = 100,000 alloc_status rows × 500 B ≈ 50 MB
    + services + node_health(1,000 rows) + audit  →  order 100 MB – 1 GB / node
```

This is precisely the "informer memory is a known K8s operational pain"
(research Finding 3e: *"the size of the local cache and the set of indexes
directly drive memory consumption"*). → **Caveat 4 is real and is the deferral
in §5.5.** The bounding lever *already exists in the trait rustdoc*: line 1774
of `observation_store.rs` names a Phase-2 `prefix`/predicate filter on
`subscribe_all_events`. The design owed before gossip is: **materialize only the
interest-scoped subset (O(local targets)) rather than the full replica
(O(cluster))** — the node still filters the full change stream, but holds warm
only what its reconcilers declare interest in.

**Cadence / resync burst (feeds Open Question 5).** A `LocalNode`-scope resync
resolves to **one** target (`node/<local_node_id>`) → **one** eval per period.
A hypothetical whole-set resync over M managed targets → **M** evals per period,
M distinct keys, drained in one tick's `for eval in pending` loop. At sub-ms
per pure-sync tick over node-local reads, M=1000 ⇒ ~1 s work / 30 s period ≈ 3%
duty cycle. **Bounded by managed cardinality, never by event rate** — the
distinction that settles §4.3.

---

### 2. C4 — System Context

Where the two new mechanisms sit relative to the operator, the two stores, and
the convergence loop. Both new pieces are **runtime-internal** and read-only
against the Observation side.

```mermaid
C4Context
    title System Context — Reconciler framework (#266): cadence hook + reflector-Store

    Person(operator, "Operator", "overdrive deploy / describe / status via CLI (mTLS)")

    System_Boundary(node, "overdrive serve — single control-plane node") {
        System(runtime, "Reconciler Runtime", "Convergence loop + EvaluationBroker + reflector-Store + cadence scheduler. Drives pure reconcile() toward desired==actual.")
    }

    SystemDb(intent, "IntentStore", "redb / openraft. Desired state (Job, Node, Service). rkyv zero-copy. Type-non-substitutable.")
    SystemDb(obs, "ObservationStore", "CR-SQLite local replica. Observed state (alloc_status, service_backends, node_health). Emits a lag-aware change feed.")
    System_Ext(host, "Host / kernel", "cgroups, VMM, getifaddrs, bpftool — the actual for host-state reconcilers (vm-reclamation, veth, XDP). Emits NO rows.")

    Rel(operator, runtime, "deploy / observe", "CLI mTLS")
    Rel(runtime, intent, "reads desired (per tick)", "rkyv")
    Rel(runtime, obs, "reads actual + SUBSCRIBES to change feed", "subscribe_all_events → SubscriptionEvent")
    Rel(runtime, host, "hydrates host-state actual LIVE (resync-only)", "getifaddrs/bpftool")
    Rel(runtime, intent, "commits Actions via ActionShim → Raft", "typed Action")

    UpdateRelStyle(runtime, obs, $offsetY="-40", $offsetX="-90")
    UpdateRelStyle(runtime, host, $offsetY="10")
```

Load-bearing reads of this diagram:

- The reflector-`Store` is a **materialized view of the Observation side only**
  (research §6.4 row 1). It never touches Intent and is never a write path.
- Host-state reconcilers hydrate their `actual` **live from the host**, not from
  the cache — they are a structurally distinct population (§5.3).

---

### 3. C4 — Container

Decomposition of the Reconciler Runtime. **Bold = new (#266).** The
`subscribe_all_events` watcher and the `EvaluationBroker` already exist; the
reflector-`Store` and the cadence scheduler are the two new runtime components.

```mermaid
C4Container
    title Container — Reconciler Runtime internals (#266 additions in the Runtime boundary)

    SystemDb(obs, "ObservationStore", "CR-SQLite; subscribe_all_events() → LagAwareSubscription (Row | Lagged{missed})")
    SystemDb(intent, "IntentStore", "redb; desired")
    System_Ext(host, "Host/kernel", "cgroup/VMM/netdev")

    Container_Boundary(rt, "Reconciler Runtime") {
        Container(loop, "Convergence Loop", "tokio task", "Owns the injected Clock + local NodeId. Drains broker, runs hydrate→reconcile→dispatch, self-re-enqueues.")
        Container(sched, "Cadence Scheduler [NEW]", "pure fn + loop bookkeeping", "Per-reconciler next-wake table. On period elapse, resolves scope→target(s) and submits Evaluations. Hook is PURE (now passed in); loop owns the clock.")
        Container(broker, "EvaluationBroker", "in-proc workqueue", "Cancelable-eval-set. LWW key-collapse on (ReconcilerName, TargetResource). Coalesces edge + resync + self-re-enqueue submits.")
        Container(store, "reflector-Store [NEW]", "warm materialized cache", "Folds SubscriptionEvent::Row into per-target actual view; relists on Lagged. Serves hydration reads AND drives interest fan-out → broker.submit. ONE subscription.")
        Container(hydr, "Hydration (desired/actual)", "async, runtime-owned", "desired ← IntentStore. actual ← reflector-Store (row-backed) OR host LIVE (host-state).")
        Container(recon, "Reconciler registry", "AnyReconciler enum", "Pure sync reconcile(desired, actual, view, tick) → (Vec<Action>, View).")
        Container(views, "ViewStore", "redb, warm BTreeMap", "Per-reconciler View. fsync-then-memory (WriteThroughOrdering).")
        Container(shim, "ActionShim", "async dispatch", "Commits Actions → Raft / Driver / ObservationStore.")
    }

    Rel(obs, store, "change feed", "subscribe_all_events")
    Rel(store, broker, "interest fan-out: row-change → submit(Evaluation)", "edge trigger")
    Rel(sched, broker, "resync: submit(Evaluation) per resolved target", "level backstop")
    Rel(loop, broker, "drain_pending() per tick", "")
    Rel(loop, sched, "advance next-wake table (now)", "")
    Rel(store, hydr, "serves warm actual (row-backed reconcilers)", "read")
    Rel(host, hydr, "LIVE actual (host-state reconcilers, resync-only)", "getifaddrs/bpftool")
    Rel(intent, hydr, "serves desired", "rkyv")
    Rel(hydr, recon, "desired + actual", "")
    Rel(views, recon, "warm View", "")
    Rel(recon, shim, "Vec<Action>", "")
    Rel(recon, views, "next View (fsync-then-memory)", "")
```

The single most important structural claim of Piece B is the double edge from
`reflector-Store`: it **both** feeds `hydration` (the read model) **and** drives
`broker.submit` (the interest fan-out) from **one** subscription. That fusion is
the whole point — it is why interests + cache + hydration are "three faces of one
informer" (research §6.1), and collapsing them onto one component is what removes
the five-wiring-sites smell.

---

### 4. Piece A — cadence scheduler (system design)

#### 4.1 Model: level-triggered resync as a safety net beside the edge path

Direct port of the K8s two-knob split (research Finding 3c): the edge-triggered
broker path is the primary trigger (fast, responsive); a periodic
level-triggered resync is the **safety net** — because edge-only is fragile
(one dropped/missed `SubscriptionEvent` ⇒ permanent divergence). Prior art:
controller-runtime `SyncPeriod` (whole-set resync, default 10 h) + `RequeueAfter`
(per-object next-evaluation); kube-rs `Action::requeue_after(Duration)`. The
reconciler **declares** its cadence; the **loop owns the clock**.

#### 4.2 HARD constraints (must survive the trait design)

- **The loop owns the clock.** `spawn_convergence_loop` already sources the
  injected `Clock` (`SimClock` under DST) via `clock.now()`/`clock.sleep(cadence)`
  (`lib.rs:2436`, `:2472`). Piece A adds a **per-reconciler next-wake table**
  *in the loop*, read/advanced against `now`. No cadence constant, reconciler
  name, or target scheme leaks into the loop after this — they move behind the
  declaration.
- **The hook is PURE.** It receives `now` (or reads nothing at all, for a static
  period) and returns a **declaration** (data). It performs no I/O, holds no
  clock handle. This preserves the pure-`reconcile`/DST-replay story and mirrors
  the object-safety win the research flags (Facet 1 is object-safe: no associated
  types, one `AnyReconciler` arm).
- **The loop owns scope resolution** (§4.4).

#### 4.3 Open Question 5 — VERDICT: the broker coalesces resync submits correctly

**Question:** does a reconciler-declared cadence that re-submits its target(s)
coalesce through the cancelable-eval-set, or does it risk the Nomad eval-storm
shape?

**Trace of `eval_broker.rs`.** `submit(eval)` does
`pending.insert((reconciler, target), eval)`; a second submit at the *same key*
evicts the prior into `cancelable` (LWW) and bumps `cancelled` — **at most one
pending entry per distinct `(ReconcilerName, TargetResource)` key**.
`drain_pending` empties the pending map; each distinct key drains **once** per
tick.

**Verdict: YES — coalesces correctly; no storm.** Three independent reasons,
each grounded in the key-collapse:

1. **Redundant resync-vs-edge-vs-self-re-enqueue collapses.** A resync submit for
   a target already pending — from an edge `SubscriptionEvent`, or from the
   `has_work` self-re-enqueue at `reconciler_runtime.rs:1587` — lands on the same
   key and collapses to **one** dispatch. Resync never *adds* a dispatch for a
   target the edge path already queued this tick.
2. **A whole-set resync produces exactly one eval per *distinct* managed target
   per period.** Those M submits are M *distinct* keys — they don't collapse with
   *each other*, but they don't *multiply* either. This is **bounded by managed
   cardinality M, not by event rate.** The Nomad eval-storm shape is *unbounded
   redundant* evals at the *same logical key* from a flap; the broker's whole
   reason for existing is collapsing exactly those (`eval_broker.rs:1-8`:
   "60 000 redundant evaluations from a single flap collapse to one dispatch per
   distinct target"). Resync's M-distinct-key burst is categorically not that
   shape.
3. **Resync fires once per period, not every tick** — the loop's next-wake table
   re-arms on period elapse, so a 30 s resync submits M evals every 30 s, not
   every 100 ms tick.

**Two design constraints the trait/loop must honour to keep this verdict true**
(handed to Morgan):

- **(C-A1) The resync submit MUST go through `broker.submit`** — the same
  coalescing path — never a side channel that bypasses the key-collapse. A resync
  that pushed directly onto the dispatch path would reintroduce the storm.
- **(C-A2) The next-wake table MUST re-arm at most once per period**, not
  re-enqueue every tick. (A per-tick re-arm degenerates to a busy resync = the
  storm by another name.)

**Residual (ties to §5.5):** for a future whole-set resync over a *large* target
set at gossip scale, the per-period O(M) burst is bounded but can be large. That
is the same cardinality concern as Caveat 4, not a coalescing defect — flagged
there, not here.

#### 4.4 `LocalNode` scope resolution

The reconciler declares **period + scope**; the **loop resolves scope → concrete
target(s)** using state the loop already owns. Specifically
`ResyncScope::LocalNode` resolves to `node/<local_node_id>` using the loop's
`NodeId` (the single-node id from ADR-0025 §28; `TargetResource` already accepts
the `node/` prefix — `reconcilers/mod.rs:733`). This is the exact vm-reclamation
motivating case: it needs one `node/<id>` sweep and nothing else. The reconciler
never names `node/<id>` — it names the *intent* (`LocalNode`), the loop supplies
the id. This keeps the "no target scheme in the loop" constraint honest: the
loop resolves a *scope enum* it understands, not a hardcoded per-reconciler
string.

Scope shapes the system design needs to support (the *set*, not the wire form):
`LocalNode` (→ `node/<local_node_id>`, one target) and — only if a future
reconciler needs it — a whole-set scope (→ all managed targets, bounded by §5.5).
Per-target `RequeueAfter`-style deadlines are **not** needed by any current
reconciler; recommend deferring that expressiveness (RN-1).

---

### 5. Piece B — reflector-`Store` (system design)

> **DEFERRED to #270 — NOT shipping in this feature (RN-2 = B-2, interests-only).**
> This entire §5 (the warm reflector-`Store`: Caveat-1 invalidation, the
> `ReflectorApplyBeforeHydrate` SD-7 ordering invariant, and Caveat-4 cardinality
> bounding) is the **B-1 forward design for GH #270**. What ships in this feature
> is Piece B **interests-only** over the existing `subscribe_all_events` watcher —
> see the APPLICATION section (§A2) and ADR-0084. Read §5 as the #270 design, not
> this feature's scope.

#### 5.1 Model: `watcher → reflector → Store`, one subscription

kube-rs shape (research Findings 3a/3e): a `watcher` yields a change stream; a
`reflector` folds it into a `Store`; the controller **both reads the `Store` for
reconcile AND is woken from the same stream.** Overdrive **already ships the
`watcher`**: `ObservationStore::subscribe_all_events() → LagAwareSubscription`
(`observation_store.rs:1896`) yielding `SubscriptionEvent::{Row, Lagged{missed}}`
with an explicit etcd-`ErrCompacted`/k8s-reflector-`Gone` relist contract
(`:1720–1757`). Piece B builds the **`reflector → Store` half**: a runtime-owned
component that (a) folds `Row` into a warm per-target materialized `actual` view,
(b) relists on `Lagged`, (c) serves hydration reads, and (d) drives the interest
fan-out (`row-change → broker.submit`). **One subscription, two consumers** —
that fusion is the unification win.

This makes Piece B **additive over an existing, DST-tested primitive**, not a
from-scratch build.

#### 5.2 Caveat 1 — invalidation model (THE crux) — RATIFICATION NEEDED (RN-2)

**The cache MUST be a materialized view invalidated by the ObservationStore's own
change feed — never a parallel truth.** A warm cache that could lag or diverge
from the store re-introduces, *as a framework feature*, the exact drift hazard
`.claude/rules/reconcilers.md` exists to prevent: a reconciler reading a stale
`actual` from the cache converges against a stale snapshot of the very rows it
manages — the adopt-and-skip / fingerprint anti-pattern, one layer up. **ADR-0079
is the governing precedent:** "converge on the rows you MANAGE, read them back;
never a hash of what you emitted." Piece B must extend that discipline to the
framework's own cache: the cache is *derived state* (a projection of
ObservationStore rows), and per "Persist inputs, not derived state" the store —
not the cache — is the source of truth. The cache is a *faithful, always-catching-
up materialization*, structurally guaranteed by:

1. **Change-feed as sole writer.** The cache is written **only** by folding
   `SubscriptionEvent::Row`. No reconciler, no other path, writes it. (A
   reconciler that wants to change observed state emits an `Action`; the write
   goes through the ObservationStore, which re-emits a `Row`, which updates the
   cache — never a direct cache write.)
2. **Relist-on-`Lagged` rebuildability.** On any `SubscriptionEvent::Lagged`, the
   reflector **re-acquires an authoritative snapshot** from the store and
   rebuilds/merges the materialized view — the reflector-`Store`'s "disposable,
   rebuildable from source" property (research Finding 3f: Microsoft Materialized
   View). The primitive for this already exists and is *mandatory to handle*
   (`observation_store.rs:1734–1738`: no lossy `Row`-only sibling; every consumer
   must handle `Lagged`). This is what makes "cache" honest — it can never be a
   permanently-diverged parallel truth.
3. **A DST-pinned update-relative-to-tick ordering invariant** (§5.4).

**RATIFICATION NEEDED (RN-2).** This is *the* value judgment of the feature.
Options:

| # | Option | Trade |
|---|---|---|
| **B-1** (recommended) | Warm reflector-`Store` invalidated by the change feed, relist-on-`Lagged`, DST-pinned ordering | Unifies interests+hydration onto one primitive; removes the five-sites smell; **structurally** (not "carefully") prevents the drift hazard via change-feed-sole-writer + relist. Cost: a new runtime component + the Caveat-4 bounding debt (§5.5). |
| **B-2** | Interests-only: land the declarative interest fan-out over the *existing* `subscribe_all_events`, but keep per-tick re-hydration from the store (no warm cache) | Lower risk, no bounding debt, no new stale-cache surface. Loses the unification (hydration stays a separate per-tick read); the "warm cache" is deferred. Latency cost is ~nil (Caveat 3), so this is a *cheap, honest cut*. |
| **B-3** | Warm cache for the *fat* cross-store reconcilers only (`WorkflowLifecycle`, `ServiceMapHydrator`); per-tick for the thin ones | Matches the "read≠write ⇒ materialize" heuristic (Finding 3f). But a per-reconciler split reintroduces exactly the "some reconcilers wired differently" smell #266 objects to; not recommended. |

**Titan's recommendation: B-1**, because the invalidation model is not a
"be-careful" mitigation but a *structural* one — the same mechanism ADR-0079
mandates for a single reconciler, generalized to the framework — and the
unification win is the actual point of the feature. **BUT** the honest lower-risk
fallback is B-2: if DESIGN judges the Caveat-4 bounding story (§5.5) too immature
to commit the warm-cache contract at Phase 1, land Piece A + the interest
declaration over the existing watcher, and defer the warm materialization to the
Phase-2 bounding slice. Either is defensible; the value call (unification-now vs
bounding-debt-deferred) is what needs ratifying.

#### 5.3 Caveat 2 — host-state exclusion (structural, not a special-case)

`vm-reclamation` (cgroup scopes / VMM liveness), veth-provisioner, XDP-attach
hydrate `actual` from the host (`getifaddrs`/`bpftool`/cgroup), **not** from
rows. They **cannot** be cache-served — there is no `SubscriptionEvent` for an
orphaned cgroup scope. Make the split **structural**: it is the *same partition*
as empty event-interest.

> **The interest declaration IS the partition key.** A reconciler either
> declares interests (⟺ its `actual` is row-backed ⟺ cache-served ⟺ event-woken,
> resync as backstop) or declares **none** (⟺ host-backed ⟺ hydrated LIVE from
> the host ⟺ **resync-only**, never cache-served). Piece A and Piece B are
> consistent *by construction*: a reconciler with empty `interests()` has no
> subscription in the reflector-`Store` and is driven solely by the Piece-A
> cadence; a reconciler with non-empty `interests()` is cache-served and
> event-woken with resync as the safety net.

The loop needs **no special-case** — "empty interests" is handled uniformly as
"resync-only; hydrate `actual` live." Accurate status note (research Gap 3):
**no host-state reconciler is on `main` today** — all 7 current reconcilers are
row-backed. `vm-reclamation` is on the microvm feature branch; veth/XDP are
converge-on-boot (Bar 1), tracked as Bar-2 promotions (#197/#199). So the
host-state exclusion is a **forward provision** for the incoming reconcilers,
expressed via the declaration, not a fix for existing code. Landing the partition
*before* vm-reclamation's 8th reconciler lands is exactly why #266 is pre-emptive
(research Gap 3).

#### 5.4 Caveat 1 → the DST cache-ordering invariant — **`ReflectorApplyBeforeHydrate`**

Mirrors the View's load-bearing fsync-then-memory ordering (`WriteThroughOrdering`;
`development.md` § Reconciler I/O STEP 7→8). The reflector-`Store` update relative
to the tick must be a **deterministic point in the tick sequence**, or DST
replay-equivalence breaks (the cache is impure runtime state feeding the pure
`reconcile`).

**Named invariant: `ReflectorApplyBeforeHydrate`.** Two coupled sub-properties:

1. **Ordering.** Change-feed items are folded into the materialized view at a
   **fixed loop point — a tick boundary — before** that tick's `hydrate_actual`
   read. The loop drains the reflector's pending change-feed items into the
   `Store` *before* `drain_pending()` + hydrate, so the tick reads an
   **as-of-tick-start coherent snapshot**. (Analogue: fsync precedes the
   in-memory `BTreeMap::insert`; here, cache-apply precedes hydrate.)
2. **Snapshot stability across a tick.** Within a single reconcile tick, the
   cache does **not** mutate — `hydrate_actual` reads a snapshot that is stable
   for the tick's duration. This is what keeps `reconcile` deterministic over its
   `(desired, actual, view, tick)` inputs: the `actual` drawn from the cache is a
   fixed value, not a moving target.

**DST assertion shape (handed to the acceptance-designer / Morgan):** for a given
`(seed, change-feed delivery order)`, the trajectory of `(cache state at each
tick boundary, hydration result per tick, emitted Actions)` is **bit-identical**
across replays — `assert_replay_equivalent!`-style. A companion safety invariant:
after a `SubscriptionEvent::Lagged` + relist, the rebuilt cache **equals** a fresh
full materialization of the store at that point (the "disposable/rebuildable"
property is testable, not aspirational).

This invariant is the structural teeth behind Caveat 1: it forbids a torn or
mid-tick-mutated cache read, which is the concrete mechanism by which a "warm
cache" would otherwise become a lagging parallel truth.

#### 5.5 Caveat 4 — cardinality bounding — RATIFICATION NEEDED + DEFERRAL (needs user approval)

Per §1: Phase-1 single-node warm cache ≈ 1–5 MB (fine, no bounding needed).
Phase-2 gossip scale ≈ 100 MB–1 GB/node if the reflector materializes the *full*
replica (the K8s informer-memory pain). **The bounding story owed before
multi-node gossip (cf. GH #36):**

- **Bound the warm set by interest scope, not by total cardinality.** Materialize
  only the O(local-target) subset the node's reconcilers declare interest in;
  filter (don't materialize) the rest of the change stream. The lever already
  exists in the trait rustdoc: the Phase-2 `prefix`/predicate filter on
  `subscribe_all_events` (`observation_store.rs:1774`). This is a *design owed*,
  not designed here.

**RATIFICATION NEEDED (RN-3) + DEFERRAL.** The Phase-1 design commits to an
**unbounded (full-replica) warm cache** and records the bounding as an explicit
Phase-2 constraint. This is safe at single-node scale (the math backs it) but is
a real forward debt. **Per CLAUDE.md I am NOT creating a GH issue** — this is
surfaced as a **blocker in the return message** for the orchestrator to relay to
the user for approval before any tracking issue is created or deferral language
hardens. The recommendation: **accept the unbounded Phase-1 cache with the
bounding constraint recorded**, gated on the user approving the deferral (or
electing option B-2 in §5.2, which sidesteps the debt entirely by not
materializing a warm cache at Phase 1).

---

### 6. Driven ports (system-level shapes — signatures are `nw-solution-architect`'s)

Presented as **system contracts / options**, not locked signatures (per CLAUDE.md
"don't invent API surface"). Morgan pins the exact Rust.

- **P-1 — ObservationStore change-feed (EXISTS).** `subscribe_all_events() →
  LagAwareSubscription` yielding `SubscriptionEvent::{Row, Lagged{missed}}`. The
  reflector-`Store` consumes this. **System constraint:** the reflector MUST
  handle `Lagged` by relisting (Caveat 1); a Phase-2 `prefix`/predicate filter
  variant is the bounding lever (§5.5). No *new* port trait method is required
  for Phase 1 — this is the additive-over-existing property.
- **P-2 — reflector-`Store` read surface (NEW, runtime-internal).** A warm,
  per-target `actual` read that `hydrate_actual` consults for row-backed
  reconcilers. **System constraint:** read-only over the Observation projection;
  presents a snapshot stable across a tick (`ReflectorApplyBeforeHydrate`). This
  is a runtime component surface, likely *not* a core port trait — Morgan decides
  placement.
- **P-3 — interest declaration (NEW, on `Reconciler`).** A declaration of which
  observation-row kinds/targets wake a reconciler (research candidate E). **System
  constraints:** (a) it is **data** (object-safe; returns a static routing
  descriptor), preserving pure-`reconcile`; (b) **empty ⟺ host-backed ⟺
  resync-only** (§5.3 — the partition key); (c) the runtime owns the
  `interest → broker.submit` fan-out. **Exact shape is Morgan's** — the research
  floats `&'static [Interest]`; do not lock it here.
- **P-4 — cadence declaration (NEW, on `Reconciler`).** A declaration of resync
  **period + scope**. **System constraints:** (a) **pure** (`now` passed in or
  nothing read; returns data — §4.2); (b) the loop owns the clock + `NodeId` +
  scope resolution (§4.4); (c) resync submits go through `broker.submit`
  (constraint C-A1) at most once per period (C-A2). **Exact signature is Morgan's**
  — options are `resync_period() -> Option<Duration>`,
  `next_evaluation(now) -> Option<Evaluation>`, or
  `Option<ResyncSchedule { period, scope }>` (RN-1).

---

### 7. Component decomposition (cache + scheduler)

| Component | New? | Owns | Reads | Writes | Purity |
|---|---|---|---|---|---|
| **Cadence Scheduler** | NEW | per-reconciler next-wake table | `now` (via loop-owned clock); reconciler cadence declaration | `broker.submit` (resync evals) | table is loop-owned mutable state; the *hook* is pure |
| **reflector-`Store`** | NEW | warm per-target `actual` materialized view | `subscribe_all_events` change feed; store (on relist) | its own view; `broker.submit` (interest fan-out) | impure runtime state; DST-pinned by `ReflectorApplyBeforeHydrate` |
| Convergence Loop | existing (+wiring) | injected Clock, local NodeId, tick counter | broker, scheduler, hydration | — | orchestration |
| EvaluationBroker | existing | cancelable-eval-set | — | pending/cancelable | LWW key-collapse (unchanged) |
| Hydration (actual) | existing (+routing) | — | reflector-`Store` (row-backed) OR host (host-state) | — | async, runtime-owned |
| `reconcile` | existing (unchanged) | — | desired, actual, view, tick | Vec<Action>, View | **PURE + SYNC (invariant)** |

The two NEW components share **one** subscription (the reflector's) and **one**
clock (the loop's). That single-subscription / single-clock discipline is what
keeps the DST story (single-loop/single-clock) intact while adding the two
mechanisms.

---

### 8. Decisions table

| ID | Decision | Status | Titan's recommendation |
|---|---|---|---|
| SD-1 | Piece A models level-triggered resync as a safety net beside the edge broker path (K8s `SyncPeriod`/`RequeueAfter`; kube-rs `Action::requeue_after`). | Locked (system model) | — |
| SD-2 | The cadence hook is **pure**; the loop owns the clock + NodeId + scope resolution; resync submits go through `broker.submit` (C-A1) at most once per period (C-A2). | Locked (system constraint) | — |
| SD-3 | **Open Question 5:** the broker **coalesces** resync submits correctly; no eval-storm. Bounded by managed cardinality, not event rate (§4.3). | Locked (verdict) | — |
| SD-4 | `ResyncScope::LocalNode` resolves to `node/<local_node_id>` in the loop (§4.4). | Locked (system model) | — |
| SD-5 | Piece B = reflector-`Store` built on the existing `subscribe_all_events` watcher; one subscription serves hydration + interest fan-out. | Locked (system model) | — |
| SD-6 | The host-state exclusion is **structural**: empty `interests()` ⟺ host-backed ⟺ resync-only ⟺ never cache-served (§5.3). | Locked (system model) | — |
| SD-7 | DST cache-ordering invariant named **`ReflectorApplyBeforeHydrate`** (ordering + snapshot-stability; §5.4). | Locked (named) | — |
| SD-8 | Piece B justified on **unification**, not latency (Caveat 3; §1). | Locked | — |
| **RN-1** | Cadence declaration shape (system implication: scope expressiveness). Options: `resync_period()` / `next_evaluation(now)` / `ResyncSchedule{period,scope}`. | **LOCKED (Morgan, APPLICATION §)** | `Option<ResyncSchedule{period, scope}>`; `scope` = single-variant `{LocalNode}` at Phase 1 (Titan recommended `{LocalNode, WholeManaged}` — `WholeManaged` dropped as an unimplementable/unused arm, added additively later; Morgan's signature-lock call). Enough for vm-reclamation; no per-target `RequeueAfter` deadline. Exact Rust locked in ADR-0084 § 1–2. |
| **RN-2** | Cache invalidation model (THE crux). B-1 warm change-feed-invalidated cache / B-2 interests-only, no warm cache / B-3 fat-only. | **RATIFIED: B-2 (user)** | Interests-only, **no warm cache**. The warm reflector-`Store` (B-1) is deferred to **GH #270** (gated on observed need). Consequences: no `ReflectorApplyBeforeHydrate` (SD-7), no cache-served hydration; SD-5's "one subscription serves hydration + fan-out" collapses to "one subscription drives the fan-out" (hydration stays per-tick from the replica, ADR-0036 unchanged). SD-6 stands as the interest partition key. |
| **RN-3** | Phase-2 cardinality bounding for the warm cache. | **MOOT under B-2** | No warm cache ⇒ nothing to bound ⇒ the Caveat-4 debt dissolves. Recorded, not carried. |

> **Reconciliation banner (Morgan, 2026-08-22).** RN-2 is ratified to **B-2**;
> RN-1 is locked; RN-3 is moot. The reflector-`Store` / warm-cache portions of §5
> above (Caveat 1 invalidation-as-a-cache, `ReflectorApplyBeforeHydrate` SD-7, the
> Caveat-4 bounding) describe the **deferred B-1** design (GH #270) and do **not**
> ship in this feature; read them as the #270 forward design. What ships is Piece A
> (cadence) + Piece B **interests-only** — see the APPLICATION section below and
> ADR-0084. A **third deferral** (the Facet-2 hydration-erasure rework, GH #272) is
> recorded in the APPLICATION section; hydration ownership is unchanged.

---

### 9. Open questions handed to `nw-solution-architect`

Research Open Questions 1/2/6 are Morgan's (the erasure resolution + keeping
hydration provably distinct from `reconcile`). The **system implications** Titan
records for them:

- **OQ-1/2 (where the type-match is paid).** System note: a **per-type
  reflector** (kube-rs monomorphization, research candidate D) would mean **N
  caches / N subscriptions / N clocks** — a direct system cost that *sacrifices
  the single-loop/single-clock DST design*. Titan flags this as a **system reason
  to prefer keeping the single loop** (one subscription, one clock — §7), i.e.
  the erasure resolution should stay within the closed-world-enum or
  erased-trait-`downcast` options, both of which preserve one loop. The *choice
  between* those two is Morgan's; the *constraint* (don't fragment into N
  subscriptions/clocks) is Titan's.
- **OQ-6 (hydration stays distinct from `reconcile`).** System note: the
  reflector-`Store` read (P-2) is the *impure runtime* side; `reconcile` stays
  pure/sync (`mod.rs:271` signature guard). `ReflectorApplyBeforeHydrate` is the
  DST guard that keeps the impure cache from leaking nondeterminism into the pure
  function. The signature guard must continue to forbid a store/async/`&Libsql`
  parameter on `reconcile` — Piece B does not change that.
- **OQ-4 (shared read-model worth it only for fat reconcilers?).** Answered
  structurally by SD-5/SD-6: the reflector-`Store` serves *all* row-backed
  reconcilers uniformly and degrades to near-zero ceremony for the thin ones
  (empty-ish `actual`), avoiding the "tax the majority" over-engineering the
  counter-pressure (research §5.1) warns about.

---

### 10. Cross-feature constraint that GH #265 inherits (NOT designed here)

Recorded per the task, one line, not designed: **Piece A's resync re-runs
`reconcile` with no row change** (the deliberate level-triggered safety-net
sweep). If a reconciler emits #265 `ObservationEvent`s, a *standing* condition
would emit one event **per resync fire** → **#265's occurrence-dedup MUST hold
under resync, not merely under edge-triggering.** This amplifies the
"convergent record cannot answer did-it-happen" producer-dedup problem #265 owns.
#265's DISCUSS/DESIGN inherits this as a hard constraint; Piece A does not
mitigate it (and should not — the sweep is correct).

---

### 11. Hard-constraints checklist (research §6.4 — must survive)

- ✓ `reconcile` stays **PURE + SYNC** `(desired, actual, view, tick) → (Vec<Action>, View)`. Cache/hydration is the runtime's impure work, OUTSIDE `reconcile` (§7). DST replay-equivalence preserved; the cache update is a deterministic point in the tick (`ReflectorApplyBeforeHydrate`, §5.4).
- ✓ **Single-loop / single-clock DST preserved.** One subscription, one clock (§7). Titan explicitly flags per-type monomorphization (candidate D) as a *system* regression of this property (§9) — not proposed. The `AnyReconciler` type-match *resolution* is Morgan's; Titan only names the system implication.
- ✓ **Intent/Observation type-non-substitutability preserved.** The reflector-`Store` is a materialized view of the **Observation side ONLY**, never a write path, never Intent (§2, §5.1).
- ✓ **ADR-0036 confronted, not ignored.** ADR-0036 removed the per-reconciler *async hydration surface* (runtime owns hydration). Piece B does **not** re-introduce a reconciler-owned async surface — it changes *where the runtime's hydration reads from* (warm cache vs per-tick store read). The runtime still owns hydration; `reconcile` still receives pre-computed `actual`. The ADR-0036 signature guard (no `&Libsql`/async/store param on `reconcile`) is untouched.

---

## Wave: DESIGN / [APPLICATION] Application Architecture (Morgan)

Authored after the SYSTEM layer. This layer locks the exact `Reconciler` trait
surface, the loop/runtime wiring, the Reuse Analysis, and the ADR. **Ratified
inputs:** RN-2 = **B-2** (interests-only, no warm cache; B-1 → GH #270); Open
Question 5 resolved; RN-1 is Morgan's to lock. Full record:
`design/wave-decisions.md`; the ADR: **ADR-0084**.

### A0. Scoping verdict — CONFIRM the core; REFINE the site list

**CONFIRMED.** With B-2 ratified, **neither piece touches the
`AnyState`/`AnyReconciler` erasure, and neither amends ADR-0036.** Piece A
(`resync_schedule → Option<ResyncSchedule>`) and Piece B (`interests → &'static
[ObservationRowKind]`) each return a *concrete* type (no associated types) → one
`AnyReconciler` forwarding arm apiece, touching no `AnyState`/`AnyReconcilerView`.
Hydration stays runtime-owned, per-tick from the CR-SQLite replica — no warm cache,
no reflector-`Store`, no `ReflectorApplyBeforeHydrate`. The `mod.rs:271` signature
guard still passes. The **Facet-2 hydration-erasure rework (research candidates
B/C/D, OQ 1/2/6) becomes a THIRD deferral** (§A6, GH #272) — hydration ownership is
unchanged.

**REFINED (submit-site list).** Verified against live code, the fan-out replaces
**only the observation-row-change producers**:

- `exit_observer.rs` has **×4** submits (234/254/295/320), not ×3 — fire on an
  accepted `alloc_status` write, name 4 consumers → **replaced** by the fan-out.
- `handlers.rs:76` fires on an **IntentStore** write (operator `deploy`/`stop`),
  not an observation-row change → the fan-out (Observation feed only) **cannot**
  subsume it → **stays**.
- `action_shim/enqueue_evaluation.rs:58` dispatches **reconciler-emitted**
  `Action::EnqueueEvaluation` (emitters in `workload_lifecycle.rs`,
  `backend_discovery_bridge.rs`) → reconcile-body logic + first-tick-latency +
  test blast radius → **RN-A1** (recommend keep; §A5).
- `reconciler_runtime.rs:1591` (`has_work` self-re-enqueue) re-arms *itself* →
  **stays**.

### A1. Piece A — locked cadence surface (RN-1)

```rust
// overdrive-core: pure data; label enums own as_str
pub struct ResyncSchedule { pub period: Duration, pub scope: ResyncScope }
pub enum   ResyncScope    { LocalNode }   // single-variant at Phase 1
// trait Reconciler (additive, default None):
fn resync_schedule(&self) -> Option<ResyncSchedule> { None }
// AnyReconciler: one forwarding match (7 arms), like name().
```

The hook reads nothing (purest form; not `next_evaluation(now)`). The loop owns a
`BTreeMap<ReconcilerName, UnixInstant>` next-wake table built at registration; each
iteration, for any reconciler whose next-wake ≤ `clock.now()`, it resolves
`LocalNode → [node/<local_node_id>]` (from the `NodeId` it owns,
`lib.rs:1701/233`) and `broker.submit`s, then re-arms `next_wake = now + period`
(C-A2). After this the loop carries **no reconciler name, no cadence constant, no
hardcoded target scheme** — only the generic table + a total scope resolver.
**`WholeManaged` is dropped from the Phase-1 enum** (diverging from Titan's RN-1
`{LocalNode, WholeManaged}` recommendation, within Morgan's signature-lock
authority): its resolver needs the #270 managed-target set, so shipping it now
forces an unimplementable `todo!` arm — the unused-surface smell the project
forbids. Single-variant keeps `resolve_scope` total and fully exercised;
`WholeManaged` is added additively (one variant + one arm) when a reconciler
declares it.

### A2. Piece B — locked interest surface + fan-out wiring

> **Reconciliation banner (2026-08-23 — lean surface, user design review).** The
> Piece B surface below is re-cut to a single `&'static [ObservationRowKind]`
> keyed off a **complete, `ObservationRow`-owned** discriminant; `Interest`,
> `RowKind`, `TargetFrom`, `classify`, `derive_target` are dropped and
> `ObservationRow::kind()` + inline router-local target derivation replace them
> (ADR-0084 § Amendment 2026-08-23). Piece A (§A1) and the SYSTEM layer are
> unchanged.

```rust
// overdrive-core: the discriminant lives BESIDE ObservationRow in
// crates/overdrive-core/src/traits/observation_store.rs — the type owns it.
// ObservationRow itself is NOT modified (zero rkyv/layout/discriminant impact);
// only this sibling enum + a read-only kind() projection are added.
pub enum ObservationRowKind {   // complete: one variant per ObservationRow family
    AllocStatus, NodeHealth, ServiceHydration, ServiceBackend,
    ReconcileConflict, IssuedCertificate, WorkflowTerminal, Signal,
}   // derives Ord (keys BTreeMap<ObservationRowKind, Vec<ReconcilerName>>) + Hash; owns as_str
impl ObservationRow {
    // total, no-wildcard projection — 8 arms, no `_`; a new variant fails to compile here
    pub const fn kind(&self) -> ObservationRowKind { /* exhaustive match */ }
}
// trait Reconciler (additive, default &[]):
fn interests(&self) -> &'static [ObservationRowKind] { &[] }
// AnyReconciler: one forwarding match (7 arms), like name().
```

A **new runtime task** `spawn_interest_router` is **List-then-Watch**: (1) open the
**existing** `subscribe_all_events()` watcher *first* (so no accepted write is
missed in the boot window — a `tokio::broadcast` subscriber does not see
pre-subscription sends); (2) **list** the interested snapshot families and submit
per row (so an interested reconciler wakes without waiting for a change); (3)
**watch** — per `SubscriptionEvent::Row(row)` take `row.kind()` (the total,
no-wildcard `ObservationRow::kind()` projection; a new `ObservationRow` variant
fails to compile at `kind()` until consciously mapped — the drift-closure now lives
**on the row type it describes**, stronger than the old partial `classify` and with
no `Option`). If `interest_table.get(&row.kind())` is non-empty, derive the
`TargetResource` **inline from the row** (`ObservationRow::AllocStatus(r) →
workload/<r.workload_id>`) and `broker.submit` per interested reconciler. The
"how to derive the target" lives router-local, keyed by row kind — correct for
Phase 1, where every routed kind → a workload target and all four consumers key
identically. A *per-interest* target strategy (the dropped `TargetFrom`) is
re-introduced additively **only if** a future reconciler needs a *different* target
from the *same* row kind — deferred as speculative surface, like `WholeManaged`.
The watcher emits a `Row` only for an *accepted* write (LWW winner), so the fan-out
fires on genuine changes — matching the exit-observer's "nudge only on change"
gate. On `SubscriptionEvent::Lagged` it **relists** (repeats the list step) — no
warm cache to rebuild under B-2. The four current-cut consumers
(`workload-lifecycle`, `backend-discovery-bridge`, `service-lifecycle`,
`svid-lifecycle`) each declare `&[ObservationRowKind::AllocStatus]`; default `&[]`
⟺ host-backed ⟺ resync-only (SD-6).

**Single-cut migration (greenfield, no shim):** delete the four `exit_observer`
submits + add the four `interests()` overrides + add `spawn_interest_router`, in
one change. The exit-observer keeps writing the `AllocStatusRow` and broadcasting
its `LifecycleEvent`; it stops naming consumers. **Design rule:** a reconciler MUST
NOT declare interest in a family it authors unless it reads that row back to
converge (ADR-0079); the four `AllocStatus` consumers author no `alloc_status`, so
the cut is loop-free.

### A3. C4 — Component (cadence scheduler + interest fan-out)

```mermaid
C4Component
    title Component — Reconciler Runtime (#266 APPLICATION: Piece A + Piece B, B-2)

    SystemDb(obs, "ObservationStore", "subscribe_all_events() → Row | Lagged")
    System_Ext(host, "Host/kernel", "getifaddrs/bpftool (host-backed actual)")

    Container_Boundary(rt, "Reconciler Runtime") {
        Component(loop, "Convergence Loop", "tokio task", "Owns Clock + NodeId + next-wake table. Drains broker, hydrate→reconcile→dispatch.")
        Component(cad, "Cadence next-wake table [Piece A]", "loop-owned BTreeMap", "Per-reconciler next-wake from resync_schedule(). On elapse: resolve scope→target, submit. Re-arm once/period (C-A2).")
        Component(router, "Interest router [Piece B, NEW]", "tokio task", "One subscribe_all_events. Row → row.kind() → derive target inline → submit. Lagged → relist. NO warm cache.")
        Component(reg, "Registration", "register()", "Builds cadence table + interest table from trait methods at boot.")
        Component(broker, "EvaluationBroker", "workqueue", "LWW key-collapse on (Reconciler, Target). Coalesces edge + resync + fan-out + self-re-enqueue.")
        Component(recon, "AnyReconciler", "enum dispatch", "Pure sync reconcile(). +resync_schedule()/+interests() forwarders. AnyState/AnyReconcilerView UNCHANGED.")
        Component(hydr, "Hydration", "runtime-owned, per-tick", "actual ← replica per tick (ADR-0036 unchanged; NO cache). desired ← IntentStore.")
    }

    Rel(reg, cad, "resync_schedule() → cadence table", "boot")
    Rel(reg, router, "interests() → interest table", "boot")
    Rel(obs, router, "change feed", "subscribe_all_events")
    Rel(router, broker, "fan-out submit", "edge")
    Rel(cad, broker, "resync submit (C-A1)", "level backstop")
    Rel(loop, broker, "drain_pending()", "per tick")
    Rel(loop, cad, "advance next-wake (now)", "")
    Rel(obs, hydr, "actual per tick", "replica read")
    Rel(hydr, recon, "desired + actual", "")
    Rel(host, hydr, "host-backed actual (empty-interest reconcilers)", "resync-only")
```

### A4. Reuse Analysis — outcome

**EXTEND**: `Reconciler` trait (2 additive methods), `AnyReconciler` (2
forwarders), `spawn_convergence_loop` (next-wake table), registration path
(build the two tables), `ObservationRow` (a read-only `kind()` projection beside
the type — the row layout is **not** modified, zero rkyv/discriminant impact).
**REUSE unchanged**: `subscribe_all_events`, `EvaluationBroker`,
`Evaluation`/`TargetResource`/`ReconcilerName`, snapshot reads. **DELETE**: the 4
`exit_observer` submits. **CREATE NEW** (each justified "no existing
alternative"): `ObservationRowKind`, `ResyncSchedule`/`ResyncScope`,
`spawn_interest_router`. (`Interest`/`RowKind`/`TargetFrom`/`classify`/
`derive_target` are **not** created — dropped by the 2026-08-23 lean re-cut;
`ObservationRowKind` + inline router-local target derivation replace them.) No
CREATE NEW reimplements an existing capability. Full table in
`design/wave-decisions.md`.

### A5. RN-A1 (NEW) — RATIFICATION NEEDED

Does the single cut also migrate the **reconciler-emitted**
`Action::EnqueueEvaluation` (Mechanism 2) to `interests()`? **Recommendation:
KEEP / defer.** It is the deeper #266 smell, but removing it changes reconcile
bodies + acceptance tests + a first-tick-latency behaviour — exceeding the ratified
"surgical, separable, cheap" mandate. Keep it as the explicit reconciler→reconciler
handoff primitive; migrate producer-push→consumer-pull in a later, independently
drivable slice only if judged worth the blast radius. **Deferral tracked at GH
#271.**

### A6. Third deferral — Facet-2 hydration-erasure rework

Untouched by this design (hydration stays runtime-owned, ADR-0036). **Trigger /
forcing function:** open-world extensibility actually needed (third-party / WASM
reconcilers, >1yr out — the scoping forcing function), OR the
`AnyState`/`AnyReconcilerView`/`hydrate_actual` five-sites edit becomes a
demonstrated maintenance sink at a materially larger reconciler count. **Deferral
tracked at GH #272** (scoped around open-world / WASM third-party reconcilers).

### A7. Hard-constraints re-confirmation (B-2 restatement)

- ✓ `reconcile` PURE + SYNC; the two methods are additive/pure/declarative; the
  `mod.rs:271` guard passes.
- ✓ Loop owns the clock; `resync_schedule` reads nothing.
- ✓ Interest wiring reads the **Observation** feed only; Intent/Observation stay
  non-substitutable.
- ✓ No warm cache, no ADR-0036 amendment, no erasure rework.
- ✓ Single-loop / single-clock DST preserved: the fan-out is a deterministic
  broker-submit source given the DST-controlled change-feed order; because there is
  no warm cache, **no `ReflectorApplyBeforeHydrate` is needed** — the accepted
  write persists to the replica *before* it emits its `Row`, so a fan-out submit
  always trails already-persisted state, and the loop hydrates ≥ that state.

---

## Wave: DISTILL / [REF] Acceptance Scenarios (Quinn, `nw-acceptance-designer`)

Authored after DESIGN (ADR-0084 APPROVE, opus re-review). **Spec-only wave** —
per `.claude/rules/testing.md` there are **no `.feature` files**; the executable
scenario SSOT is `distill/test-scenarios.md` (20 GIVEN/WHEN/THEN blocks), which
the DELIVER crafter translates into Rust `#[test]`/`#[tokio::test]`. No `crates/`
edits, no Rust test files, no tests run in this wave.

**Prior-wave gate:** DESIGN present → proceed. DISCUSS absent → WARN (ACs derived
from ADR-0084 + SYSTEM/APPLICATION delta). DEVOPS absent → WARN (in-process
framework change; no env/deployment matrix). Reconciliation HARD GATE: only
`design/wave-decisions.md` present; **0 contradictions → PASS**.
`deliverable_type = application`.

**Scope:** exactly the two shipped hooks — Piece A cadence + Piece B interests
(B-2, interests-only). Deferred scope (#270 warm cache, #271 `EnqueueEvaluation`
migration, #272 Facet-2/WASM) is OUT — **no scenarios authored for it**.

### [REF] Scenario list + tags

| ID | Scenario | Tags |
|---|---|---|
| S-266-01 | Walking skeleton — watched rows change, reconciler converges via the production loop | `@walking_skeleton @dst @piece-a @piece-b @contract-shape:bounded-change` |
| S-266-02 | Scheduled reconciler resynced once per period | `@dst @piece-a @property @contract-shape:bounded-change` |
| S-266-03 | Resync re-arms at the period boundary, not before, not every tick | `@dst @piece-a @property @error @contract-shape:bounded-change` |
| S-266-04 | Reconciler with no schedule is never resynced | `@dst @piece-a @error @contract-shape:bounded-change` |
| S-266-05 | Two reconcilers, distinct periods, own cadence (loop no-hardcode proxy) | `@dst @piece-a @property @contract-shape:bounded-change` |
| S-266-06 | Cadence hook pure; `reconcile` stays pure/sync | `@unit @property @contract-shape:pure-function` |
| S-266-07 | `LocalNode` scope resolves to exactly the local node target, totally | `@unit @property @contract-shape:pure-function` |
| S-266-08 | Interested reconciler wakes when its observed rows change | `@dst @piece-b @property @contract-shape:bounded-change` |
| S-266-09 | Host-state reconciler (empty interests) never event-woken | `@dst @piece-b @error @contract-shape:bounded-change` |
| S-266-10 | Migration preserves behaviour — four consumers wake as deleted submits did | `@dst @piece-b @property @contract-shape:bounded-change` |
| S-266-11 | `ObservationRow::kind()` totally discriminates all 8 row variants to their `ObservationRowKind` | `@unit @property @contract-shape:pure-function` |
| S-266-12 | `Workload` interest derives the workload-scoped target | `@dst @piece-b @property @contract-shape:bounded-change` |
| S-266-13 | Fan-out fires on every accepted `alloc_status` write (equal-or-broader) | `@dst @piece-b @property @contract-shape:bounded-change` |
| S-266-14 | After a lag gap the router relists — no interested target un-woken | `@dst @piece-b @error @contract-shape:bounded-change` |
| S-266-15 | List-then-Watch — pre-existing rows wake without waiting for a change | `@dst @piece-b @error @contract-shape:bounded-change` |
| S-266-16 | Non-accepted (LWW-loser / no-op) write wakes nobody | `@dst @piece-b @error @contract-shape:bounded-change` |
| S-266-17 | Interest hook is pure static routing metadata | `@unit @property @contract-shape:pure-function` |
| S-266-18 | Convergence reaches a fixpoint — no infinite re-wake | `@dst @property @piece-a @piece-b @error @contract-shape:bounded-change` |
| S-266-19 | No resync-storm — redundant resync submit coalesces at the already-pending resync key | `@dst @property @piece-a @error @contract-shape:bounded-change` |
| S-266-20 | Single-clock determinism — seed → bit-identical trajectory | `@dst @property @piece-a @piece-b @contract-shape:bounded-change` |
| S-266-21 | **[REMOVED — 2026-08-23 lean rework; router-local, covered by S-266-12 + S-266-10]** | — |
| S-266-22 | No fan-out storm — write-flood coalesces to one eval per distinct interested target | `@dst @property @piece-b @error @contract-shape:bounded-change` |

Error/edge: S-03, S-04, S-09, S-14, S-15, S-16, S-18, S-19, S-22 = **9/21 = 43%** (≥40%).
Contract shapes: `pure-function` ×4 (S-06/07/11/17), `bounded-change` ×17, **no
`unbounded-preservation`** (no preview/dry-run surface exists — design-confirmed).

### [REF] Tier mapping

- **Tier-1 DST (PRIMARY, default lane, `Sim*` traits)** — 17 scenarios: S-01
  (WS-composition), 02, 03, 04, 05, 08, 09, 10, 12, 13, 14, 15, 16, 18, 19, 20, 22.
  `SimClock` + observation store; `assert_eventually`/`assert_always`;
  seed-reproducible.
- **Unit / proptest / compile-time** — 4 scenarios: S-06 & S-17 (trait
  signature/purity guards), S-07 (`resolve_scope` proptest), S-11
  (`ObservationRow::kind()` parametrize over all 8 `ObservationRow` variants —
  closed-world → parametrize, not PBT, per the falsifier-gate).
- **Walking skeleton / vertical slice** — S-01 only.

### [REF] Driving-surface coverage (run_server vertical slice)

No CLI/HTTP driving port. The driving surface is the **convergence loop /
`run_server` composition**. S-266-01 **boots the production composition entry**
`run_server_with_obs_and_driver` (`lib.rs:1447`) — the same entry that spawns the
convergence loop at `lib.rs:2315` — with Sim obs + `SimClock`, and **asserts that
this entry spawns `spawn_interest_router`** (the router is wired by `run_server`,
not by the test; no hand-call of the spawn fn, no hand-assembled router). The
spawn-wiring teeth are *does `run_server_with_obs_and_driver` spawn the router?*,
not *does the spawn fn exist / compose with the loop?*. This honours the
vertical-slice rule ("no test installs the one production call site the feature
omitted"). **Primary lane:** Tier-1 DST booting the production entry with Sim
adapters; **fallback:** a full `run_server` Lima boot under `integration-tests`.

### [REF] Mutation surface (DELIVER mandatory targets)

1. `ObservationRow::kind()` — **every match arm** (S-266-11).
2. Cadence next-wake `<=` decision + `next_wake += period` re-arm (S-266-02/03).
3. Interest-router routing — the `ObservationRowKind → interested reconcilers`
   table lookup + the inline `AllocStatus(row) → workload/<row.workload_id>`
   derivation, DST-covered by S-266-08/10/12 (the pure-fn target-derivation sibling
   is gone — derivation is router-local).
4. `resolve_scope(LocalNode, n) → node/<n>` derivation (S-266-07).
5. Broker LWW key-collapse on `(ReconcilerName, TargetResource)`, exercised on
   **both paths**: the resync side (`node/…` key) by S-266-19 and the fan-out side
   (`workload/…` key) by S-266-22.

### [REF] Test-placement plan

| Concern | Crate / dir | Lane |
|---|---|---|
| `ObservationRow::kind()` exhaustive (S-11) — beside `ObservationRow` + `ObservationRowKind`, with the compile-fail drift-closure; `resolve_scope` (S-07) — pure fns over core types, dst-lint-clean, mutation-testable without `integration-tests` | `overdrive-core` (`ObservationRow::kind()` in `crates/overdrive-core/src/traits/observation_store.rs`; `resolve_scope` in `crates/overdrive-core/src/reconcilers/mod.rs`) co-located unit | default |
| Trait purity/signature (S-06, S-17) | `overdrive-core/tests/` beside `reconciler_trait_signature_is_synchronous_no_async_no_clock_param` | default |
| Broker coalescing — resync side (S-19) + fan-out side (S-22) | `overdrive-core/src/eval_broker.rs` co-located + `overdrive-control-plane` DST driving `spawn_interest_router` (S-22) | default |
| Cadence DST (S-02/03/04/05/20) | `overdrive-control-plane` DST driving `spawn_convergence_loop` under `SimClock` (or `overdrive-sim` invariant catalogue) | default (Tier-1) |
| Interest DST (S-08…S-16, S-18) | `overdrive-control-plane` DST driving `spawn_interest_router` + broker + `Sim`/`Local` observation store under `SimClock` | default (Tier-1) |
| Walking skeleton (S-01) | `overdrive-control-plane/tests/` booting `run_server_with_obs_and_driver` (`lib.rs:1447`) with Sim obs + `SimClock`, asserting it spawns `spawn_interest_router`; full `run_server` Lima boot is the fallback | default (Tier-1), else `integration-tests` (Lima) |

Migration cut: deleting `exit_observer.rs:234/254/295/320` rewrites the pre-existing
`exit_observer` "enqueues bridge/service/svid" acceptance tests to assert fan-out
equivalence (S-266-10) in the **same commit** (deletion discipline).

### [REF] RED-scaffold convention (crafter applies in DELIVER — documented, not applied here)

Per `.claude/rules/testing.md` — **not** `.feature`, **not** `NotImplementedError`:
test-side `#[should_panic(expected = "RED scaffold")]` with a
`panic!("Not yet implemented -- RED scaffold (S-266-NN / …)")` body; production-side
`todo!("RED scaffold: …")` gated by `#[expect(clippy::todo, reason = "RED scaffold;
lands GREEN in step <id>")]`. `#[ignore = "reason"]` only for genuinely-external
blockers.

### [REF] AT-completeness + verification-catalogue

- **AT-completeness (15-item mechanical checklist): 15/15 → COMPLETE.** All gaps
  `AT_GAP_IN_DELIVERY_SCOPE` (filled); **zero `SPECIFICATION_AMBIGUITY` blockers**
  (C2 state machines, C5 partition key, C6 closed `ObservationRow::kind()`/`Lagged`
  contract all fully specified in ADR-0084). C6b/C6c/C7a/C7c counted passing with documented
  rationale (no typed error-return surface on the fan-out; single-threaded broker).
  Full matrix in `distill/test-scenarios.md`.
- **`verification/` operator catalogue: none.** Internal reconciler-framework
  wiring with no operator-observable surface (no CLI verb, no HTTP, no
  `describe`/`status` change). Proven by the DST + unit tiers; no qualitative
  operator expectation to graduate (manufacturing one would dilute the catalogue).
