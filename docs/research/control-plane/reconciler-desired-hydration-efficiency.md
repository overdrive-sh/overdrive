# Research: Avoiding Re-Derivation of Desired State on Every Reconcile Tick — Informer/Indexer Caches, Field Indexers, and Event-Driven Invalidation

**Date**: 2026-06-03 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 18 cited (~22 examined)

## Executive Summary

The industry-canonical answer to "don't re-derive desired state from durable storage on every reconcile tick" is the **Kubernetes informer/indexer model**: controllers read desired state from an **in-process cache kept current by watch events**, never by listing the durable store (etcd, via the API server) inside the reconcile path. This is corroborated by official controller-runtime, client-go, Kubebuilder, Cluster API, and Operator SDK documentation, and the same shape appears in etcd's watch/revision model and Argo CD's watch-backed live-state cache. The Cluster API tuning guide states the principle bluntly: "avoid uncached list calls, or make sure they happen only once in a reconcile loop," and "watch for events, so the controller is going to do the work only once when it is actually required" — explicitly preferring event-driven recompute over polling-to-detect-change. Argo CD's own tracker classifies "all reconciliations driven by a timer instead of watch events" as a regression. Overdrive's `ServiceMapHydrator` doing a full uncached intent-store scan + decode **once per target per ~100ms tick** is precisely the anti-pattern this literature names.

The canonical *scoped-lookup* remedy is the **field indexer** (`IndexField` + `MatchingFields`): the Kubebuilder Book introduces it verbatim because "as our number of cronjobs increases, looking these up can become quite slow as we have to filter through all of them." The canonical *invalidation key* is a **writer-bumped monotonic generation** (`metadata.generation` vs `observedGeneration`; etcd's revision counter) — not a reader-recomputed content digest, and not a TTL (which the caching literature flags as the inconsistent strategy). These two — cache-from-events and version-keyed invalidation — are exactly Overdrive's own stated rules ("Views served from RAM, durable storage touched only on write-through/cold boot"; "invalidation key tied to inputs + policy identity"; pure `reconcile` per Anvil/ESR).

Mapped to the candidate fixes: **(d) fold listener facts into the bulk-load + write-through in-memory view path is the architecturally-aligned destination** (ranks first — it is the informer-cache pattern, reuses machinery the runtime already has, and listener facts are too small for the memory objection to matter). **(c) a reverse index for scoped lookup ranks second** (the field-indexer remedy; the right choice if no intent-write hook exists for (d) or the cached set were large). **(b) hoist the scan to once-per-tick ranks third** as a minimal-risk interim that kills the quadratic but still polls. **(a) memoise by recomputed `spec_digest` is discouraged as specified** — the digest must be recomputed (decoded) every tick to validate the cache, re-incurring the avoided cost; it is only acceptable if re-keyed on a writer-published generation, at which point it converges onto (d). Recommended path: ship (b) if a fast bleed-stop is needed, then land (d).

## Research Methodology

**Search Strategy**: Targeted searches against official Kubernetes docs (kubernetes.io), controller-runtime godoc/source (pkg.go.dev, github.com/kubernetes-sigs/controller-runtime), client-go architecture docs, etcd docs (etcd.io), Argo CD / Flux docs (argo-cd.readthedocs.io, fluxcd.io), and academic reconciler-verification work (usenix.org / arxiv.org).
**Source Selection**: Types: official, open_source, industry_leaders, academic | Reputation: high (1.0) preferred, medium-high (0.8) cross-referenced | Verification: each major claim cross-referenced against 2-3 independent sources, official docs preferred as primary.
**Quality Standards**: Target 3 sources/claim (min 1 authoritative) | All major claims cross-referenced | Avg reputation target ≥ 0.8

## Problem Context (grounding)

Overdrive's `ServiceMapHydrator` reconciler hydrates desired state (`gather_service_listener_facts`) by performing a full scan of the `workloads/` intent-store prefix + rkyv-decode of every Service intent + per-intent mutex lock, **once per target, per ~100ms tick**. With one target per service (S services), this is **O(S²) decodes + O(S²) lock acquisitions per tick**. The derived value (`ListenerRow = (vip, port, protocol)`) only changes on operator intent submission — it is stable between submissions.

Candidate fixes under evaluation:
- **(a)** Memoise `spec_digest → Vec<ListenerRow>` in a side-cache invalidated on intent submit/stop.
- **(b)** Hoist the cluster-wide scan to once per tick (not once per target): O(S²) → O(S).
- **(c)** Scope the read to the target service only (single keyed lookup), possibly via a reverse index `vip/service_id → owning intent`.
- **(d)** Fold listener facts into the bulk-load + write-through in-memory view path so steady-state pays zero durable reads.

## Findings

### Finding 1: Kubernetes controllers read state from a local cache populated by watches, not by listing the API server on every reconcile
**Evidence**: controller-runtime's `cache` package provides "object caches that act as caching `client.Reader` instances and help drive Kubernetes-object-based event handlers." By default, controller-runtime controllers use a **cache-backed client for reads** — `client.Get()` / `client.List()` inside `Reconcile()` resolve against the in-process informer cache, not the API server. Reconciliation is "level-based, meaning action isn't driven off changes in individual Events, but instead is driven by actual cluster state read from the apiserver or a local cache."
**Source**: [cache package — sigs.k8s.io/controller-runtime/pkg/cache](https://pkg.go.dev/sigs.k8s.io/controller-runtime/pkg/cache) - Accessed 2026-06-03
**Confidence**: High
**Verification**: [controller-runtime issue #498 — Understanding Client/Informer/Indexer interaction in Reconcile](https://github.com/kubernetes-sigs/controller-runtime/issues/498), [reconcile package godoc](https://pkg.go.dev/sigs.k8s.io/controller-runtime/pkg/reconcile)
**Analysis**: This is the canonical precedent for Overdrive's own architectural rule — "Views are bulk-loaded once at register time into an in-memory `BTreeMap` and served from RAM; durable storage is touched only on write-through and cold boot." Kubernetes controllers do not re-read the durable store (etcd via the API server) on every tick; they read a watch-maintained local cache. Overdrive's listener-fact hydration path bypasses this discipline by scanning the durable intent store every tick.

### Finding 2: The control loop is "watch state → diff → act," and Kubernetes uses a watch (not poll) model to observe state
**Evidence**: "In Kubernetes, controllers are control loops that **watch the state** of your cluster, then make or request changes where needed. Each controller tries to move the current cluster state closer to the desired state." A controller "tracks at least one Kubernetes resource type. These objects have a spec field that represents the desired state."
**Source**: [Kubernetes Documentation — Controllers](https://kubernetes.io/docs/concepts/architecture/controller/) - Accessed 2026-06-03
**Confidence**: High
**Verification**: [cache package godoc — "drive Kubernetes-object-based event handlers"](https://pkg.go.dev/sigs.k8s.io/controller-runtime/pkg/cache), [controller-runtime source/source.go (watch sources feed the workqueue)](https://github.com/kubernetes-sigs/controller-runtime/blob/main/pkg/source/source.go)
**Analysis**: The desired state lives in the `spec` of watched objects; the watch keeps a local cache current, so "reading desired state" is a cache read, not a store scan. This is the precedent for candidate (d): listener facts (a projection of the Service intent's desired spec) should be maintained in-cache and re-projected on intent-change, not re-derived from the store on a timer.

### Finding 3: The canonical fix for "find objects matching field X without listing everything" is a field indexer (`IndexField` + `MatchingFields`) — and listing-then-filtering is explicitly called out as not scaling
**Evidence**: The Kubebuilder Book states verbatim: "As our number of cronjobs increases, looking these up can become quite slow **as we have to filter through all of them**. For a more efficient lookup, these jobs will be **indexed locally on the controller's name**." controller-runtime's `FieldIndexer.IndexField` "adds an index with the given field name on the given object type by using the given function to extract the value for that field," after which "you can then make use of the index by specifying a field selector (`MatchingFields`) on calls to `List`." The godoc example: "A Secret controller might have an index on the `.spec.volumes.secret.secretName` field in Pod objects, so that it could easily look up all pods that reference a given secret."
**Source**: [The Kubebuilder Book — Implementing a controller (Indexing)](https://book.kubebuilder.io/cronjob-tutorial/controller-implementation) - Accessed 2026-06-03
**Confidence**: High
**Verification**: [client package godoc — FieldIndexer / MatchingFields](https://pkg.go.dev/sigs.k8s.io/controller-runtime/pkg/client), [controller-runtime issue #1941 — IndexField documentation](https://github.com/kubernetes-sigs/controller-runtime/issues/1941)
**Analysis**: This is the direct precedent for Overdrive candidate (c) — a reverse index `vip/service_id → owning intent` so the hydrator does a single keyed lookup instead of a full prefix scan. The Kubernetes ecosystem treats "list all then filter in the reconcile body" as the named scaling pitfall and the field index as the standard remediation.

### Finding 4: The event-driven watch → cache → workqueue model exists precisely so a controller does work only when something changes, not on a polling timer
**Evidence**: client-go's `SharedIndexInformer` consumes a `DeltaFIFO` producer-consumer queue fed by a `Reflector`; "the default event handler enqueues Adds to the workqueue, enqueues Updates to the workqueue if they match a predicate function, and enqueues Deletes to the workqueue. The architecture utilizes work queues to decouple event detection from processing." The Cluster API tuning guide states verbatim: "instead of re-queuing every few seconds when a controller is waiting for something to happen, which leads to the controller to do some work to check if something changed in the system, it is always better to **watch for events, so the controller is going to do the work only once when it is actually required**."
**Source**: [The Cluster API Book — Tuning controllers](https://cluster-api.sigs.k8s.io/developer/core/tuning) - Accessed 2026-06-03
**Confidence**: High
**Verification**: [client-go cache package godoc (SharedIndexInformer / DeltaFIFO / Indexer)](https://pkg.go.dev/k8s.io/client-go/tools/cache), [kubernetes/sample-controller — controller-client-go.md (informer/workqueue architecture)](https://github.com/kubernetes/sample-controller/blob/master/docs/controller-client-go.md)
**Analysis**: This is the precedent for candidate (a)'s invalidation trigger and candidate (d): recompute the derived projection when the intent changes (a watch/event), not on every 100ms tick. The literature is explicit that polling-to-detect-change is the inferior pattern. Overdrive's tick-driven re-scan of the intent store is exactly the "re-queuing every few seconds to check if something changed" anti-pattern named here.

### Finding 5: The controller-runtime guidance is "avoid work entirely; avoid uncached list calls, or do them at most once per reconcile" — directly indicting the O(S²) per-tick scan
**Evidence**: "The best optimization that can be done is to avoid any work at all for controllers." "Whenever possible, you should **avoid uncached list calls, or make sure they happen only once in a reconcile loop** and possibly only under specific circumstances." The default split client "reads (Get and List) from the Cache and writes (Create, Update, Delete) to the API server"; reading from the cache is "a huge boost of performance (**microseconds vs. seconds**) that everyone gets at the cost of some memory allocation and the need of considering stale reads."
**Source**: [The Cluster API Book — Tuning controllers](https://cluster-api.sigs.k8s.io/developer/core/tuning) - Accessed 2026-06-03
**Confidence**: High
**Verification**: [Operator SDK — Controller Runtime Client API (split client: reads from Cache, writes to API server)](https://sdk.operatorframework.io/docs/building-operators/golang/references/client/), [controller-runtime issue #498](https://github.com/kubernetes-sigs/controller-runtime/issues/498)
**Analysis**: Overdrive's hydrator does an *uncached* list (full prefix scan + decode) of the durable store, and does it once **per target per tick** — the opposite of "at most once per reconcile" and "from the cache." Candidate (b) (hoist the scan to once per tick) brings it into partial compliance with "only once in a reconcile loop"; candidates (c)/(d) eliminate the uncached list entirely, which is what the guidance ultimately recommends ("avoid any work at all").

### Finding 6: The watch primitive underlying every K8s cache — etcd's revision/MVCC watch — observes changes incrementally from a revision instead of re-listing
**Evidence**: "An etcd watch waits for changes to keys by continuously watching from a given revision, either current or historical, and streams key updates back to the client." MVCC retains historical revisions so "watchers simply resume from the last observed historical revision," and a cluster-wide 64-bit counter increments per modification, "providing a global logical ordering of all updates." This is the change-detection substrate that makes the informer cache event-driven rather than poll-driven.
**Source**: [etcd Documentation v3.6 — etcd API (Watch / Revisions)](https://etcd.io/docs/v3.6/learning/api/) - Accessed 2026-06-03
**Confidence**: High
**Verification**: [client-go cache godoc (Reflector consumes the watch stream into DeltaFIFO)](https://pkg.go.dev/k8s.io/client-go/tools/cache), [kubernetes/kubernetes issue #82655 (reflector list/watch + revision)](https://github.com/kubernetes/kubernetes/issues/82655)
**Analysis**: The revision counter is the cross-reference for candidate (a)'s invalidation key. The literature's canonical change-detection unit is a monotonic revision/generation, not a content digest computed by re-deriving the value. Overdrive's intent store has an analogous notion (intent submit/stop is a discrete write event with an ordering); the architecturally-correct invalidation key is the intent generation/version, mirroring etcd's revision, rather than a recomputed `spec_digest` (which still requires decoding the intent to compute it — the exact cost being avoided).

### Finding 7: Cross-reference (GitOps reconcilers) — Argo CD maintains a watch-backed cluster cache specifically to avoid querying Kubernetes during reconciliation; only Git (the desired-state source it cannot watch cheaply) is polled
**Evidence**: "The argocd-application-controller's live state cache properly implements the List&Watch pattern when tracking state of cluster resources, where it issues a LIST API call from the watch cache and follows it with WATCH requests"; this "allows it to avoid querying Kubernetes during app reconciliation and significantly improve performance." Notably, the desired-state source it *cannot* watch (Git) is the only thing it polls: "By default, Argo CD checks (polls) Git repositories every 3 minutes."
**Source**: [Argo CD — FAQ (Git polling interval)](https://argo-cd.readthedocs.io/en/latest/faq/) - Accessed 2026-06-03
**Confidence**: Medium-High
**Verification**: [argoproj/argo-cd issue #18838 — live state cache List&Watch pattern](https://github.com/argoproj/argo-cd/issues/18838), [argoproj/argo-cd issue #27192 — reconciliations should be watch-event-driven, regression when they became timer-driven](https://github.com/argoproj/argo-cd/issues/27192)
**Analysis**: Argo CD's own bug tracker treats "all reconciliations driven by a timer instead of watch events" (#27192) as a **regression**, not an acceptable design — direct independent corroboration that timer-polling to detect change is the inferior pattern. The split is instructive for Overdrive: state you own and can observe cheaply (the intent store) should be watch/event-driven (candidate d); only state you genuinely cannot watch falls back to a timer. Overdrive owns its intent store and *can* observe writes, so polling it on a tick is the avoidable case.

### Finding 8: Cross-reference (formal reconciler work) — Anvil models `reconcile()` as a deterministic state-transition function and proves Eventually Stable Reconciliation; the project already cites this lineage
**Evidence**: "one has to write `reconcile()` as a state machine that defines initial state, ending state and state transitions … The reason for this style is to enable formal verification." The target correctness property is "**Eventually Stable Reconciliation (ESR)**, a liveness property stating that a controller should eventually manage the system to its desired state, and stays in that desired state, despite failures and network issues." Anvil's verified controllers are written in Rust.
**Source**: [anvil-verifier/anvil — README](https://github.com/anvil-verifier/anvil/blob/main/README.md) - Accessed 2026-06-03
**Confidence**: High
**Verification**: [USENIX OSDI '24 — Anvil: Verifying Liveness of Cluster Management Controllers (paper)](https://www.usenix.org/system/files/osdi24-sun-xudong.pdf), [ACM Digital Library — Anvil (OSDI '24 proceedings)](https://dl.acm.org/doi/10.5555/3691938.3691973)
**Analysis**: This corroborates Overdrive's own rule that `reconcile()` is a pure, deterministic function over `(desired, actual, view, tick)` — the project's CLAUDE rules cite "ESR specifications" and "USENIX OSDI '24 Anvil" by name. A pure `reconcile` *cannot* do I/O (the prefix scan + rkyv decode + mutex lock), so the desired state MUST be hydrated *before* `reconcile` is called — which is exactly the runtime's `hydrate_desired` step. The fix therefore belongs in the hydration layer feeding `reconcile`, not inside `reconcile`; candidates (c)/(d) make `hydrate_desired` cheap, keeping the purity contract intact.

### Finding 9: The canonical invalidation key for derived desired-state in Kubernetes is `metadata.generation` (a monotonic version of the spec), compared against `observedGeneration` — version-keyed, not digest-keyed, not TTL
**Evidence**: API conventions define generation verbatim as "a sequence number representing a specific generation of the desired state. Set by the system and **monotonically increasing, per-resource**," and `observedGeneration` as "the `generation` most recently observed by the component responsible for acting upon changes to the desired state of the resource." Controllers compare them to know whether work is needed: "When `metadata.generation` and `status.observedGeneration` differ, it indicates the spec has changed and the controller needs to reconcile. When they match, no spec change has occurred since the last reconciliation." `metadata.generation` "only changes when spec changes" (not on metadata/status edits).
**Source**: [Kubernetes API Conventions — generation / observedGeneration](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md) - Accessed 2026-06-03
**Confidence**: High
**Verification**: [kubernetes/kubernetes PR #69059 — generation incremented on every spec write](https://github.com/kubernetes/kubernetes/pull/69059), [knative/serving issue #4937 — observedGeneration semantics across CRDs](https://github.com/knative/serving/issues/4937)
**Analysis**: This is the literature's verdict on *what the invalidation key should be*: a monotonic per-resource version of the inputs (the spec), incremented by the writer on change. It aligns precisely with Overdrive's "invalidation key tied to inputs + policy identity" rule and with etcd's revision (Finding 6). Critically, it does NOT recompute a content digest of the derived value — the key is bumped by the *write*, cheaply, with no decode. This directly weakens candidate (a) as specified ("memoise `spec_digest → Vec<ListenerRow>`"): computing `spec_digest` requires decoding the intent every tick to know whether the cache is valid, which re-incurs the very decode cost being eliminated unless the digest/version is itself published by the writer at submit time.

### Finding 10: Cache-invalidation taxonomy — event/key-based (writer-driven) invalidation is the accurate strategy; TTL is explicitly the inconsistent one; "generations" (revision bump) invalidate many keys with one increment
**Evidence**: TTL "might result in inconsistency if the data changes before its TTL expires." Event-based invalidation "is updated when specific events occur in the data source, ensuring data accuracy." Key-based: "Whenever relevant data changes in the source, the corresponding key is marked as invalid or is simply removed." The generations technique: "Whenever the data in the database changes in a way which should invalidate a whole generation, a revision number is incremented … if the data stored in cache belongs to an old generation, it is ignored. This technique allows invalidating multiple keys in the cache using a single increment operation."
**Source**: [Devinterview-io/caching-interview-questions (caching strategy taxonomy)](https://github.com/Devinterview-io/caching-interview-questions) - Accessed 2026-06-03
**Confidence**: Medium-High
**Verification**: [Kubernetes API conventions — generation as monotonic revision (Finding 9)](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md), [etcd revision/MVCC model (Finding 6)](https://etcd.io/docs/v3.6/learning/api/)
**Analysis**: The taxonomy ranks the candidate invalidation strategies for Overdrive: writer-driven event/key invalidation (intent submit/stop bumps a generation) is the accurate one and matches what K8s actually does. A side-cache that must *poll to validate itself* (re-deriving `spec_digest` each tick to check freshness) collapses to the TTL/poll failure mode the literature warns against. The "generations" pattern is the cleanest fit: one revision counter on the intent store, bumped on any intent write, invalidates all derived listener facts with a single increment — and the hydrator compares the cached revision to the live one with no decode.

### Finding 11: Indexer (scoped lookup) vs full materialized cache is a memory-vs-coverage trade-off; controller-runtime offers both, and selective caching exists precisely to bound the memory cost of caching everything
**Evidence**: controller-runtime exposes a full informer cache by default but provides selective caching via `ByObject` / `DefaultLabelSelector` / `DefaultFieldSelector` settings; the stated solutions "for tackling cache memory consumption include configuring cache limitations by-namespace and/or by-label, and caching only partial meta-data." Field indexes are the orthogonal mechanism for efficient *lookup within* whatever is cached ("specify a field selector on calls to List on the cache Reader"). The feature exists because caching all objects of a type has a memory cost worth bounding (issue title: "Provide a way to create more selective cache to **tackle memory consumption**").
**Source**: [controller-runtime issue #2570 — selective cache to tackle memory consumption](https://github.com/kubernetes-sigs/controller-runtime/issues/2570) - Accessed 2026-06-03
**Confidence**: Medium-High
**Verification**: [controller-runtime designs/cache_options.md (ByObject / selectors config surface)](https://github.com/kubernetes-sigs/controller-runtime/blob/main/designs/cache_options.md), [cache package godoc (FieldIndexer + selectors)](https://pkg.go.dev/sigs.k8s.io/controller-runtime/pkg/cache)
**Analysis**: This is the decision rule for candidate (c) vs (d). A **field indexer / scoped lookup (c)** keeps the durable store as the SSOT and only adds a reverse index — minimal memory, but each lookup is still a (cheap, keyed) read against the index/store and depends on the index being maintained on write. A **full materialized in-memory cache (d)** pays zero reads at steady state but holds every listener fact in RAM and must be kept current by the same write-through path the project already uses for Views. The K8s ecosystem ships *both* and picks per-workload: full cache when the working set is small and read-hot (Overdrive's listener facts — `(vip, port, protocol)` per service — are tiny and read every tick), scoped index when the cardinality/size makes full caching expensive. For Overdrive's listener facts the memory cost of (d) is negligible (a few bytes × S), so the memory argument that normally favors (c) does not bite here.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Kubernetes Docs — Controllers | kubernetes.io | High (1.0) | official | 2026-06-03 | Y |
| Kubernetes API Conventions (generation/observedGeneration) | github.com/kubernetes/community | High (1.0)* | official | 2026-06-03 | Y |
| controller-runtime cache godoc | pkg.go.dev | High (1.0) | official | 2026-06-03 | Y |
| controller-runtime client godoc (FieldIndexer) | pkg.go.dev | High (1.0) | official | 2026-06-03 | Y |
| client-go cache godoc (SharedIndexInformer/DeltaFIFO) | pkg.go.dev | High (1.0) | official | 2026-06-03 | Y |
| The Kubebuilder Book — controller indexing | book.kubebuilder.io | High (1.0)* | official | 2026-06-03 | Y |
| The Cluster API Book — Tuning controllers | cluster-api.sigs.k8s.io | High (1.0)* | official | 2026-06-03 | Y |
| Operator SDK — Client API (split client) | sdk.operatorframework.io | High (1.0)* | official | 2026-06-03 | Y |
| etcd Docs — API (Watch/Revisions) | etcd.io | High (1.0) | open_source | 2026-06-03 | Y |
| Anvil — README | github.com | Medium-High (0.8) | open_source | 2026-06-03 | Y |
| Anvil — USENIX OSDI '24 paper | usenix.org | High (1.0) | academic | 2026-06-03 | Y |
| Anvil — ACM Digital Library | dl.acm.org | High (1.0) | academic | 2026-06-03 | Y |
| Argo CD — FAQ / live state cache | argo-cd.readthedocs.io | High (1.0) | open_source | 2026-06-03 | Y |
| controller-runtime / kubernetes GitHub issues (#498, #1941, #2570, #27192, #18838, #82655) | github.com | Medium-High (0.8) | open_source | 2026-06-03 | Y |
| Caching strategy taxonomy (interview compendium) | github.com | Medium (0.6) | open_source | 2026-06-03 | Y |

*Reputation note: the Kubebuilder Book, Cluster API Book, Operator SDK, and the kubernetes/community API-conventions doc are official sub-project documentation of the CNCF Kubernetes project (sigs.k8s.io / operatorframework.io), treated here as High alongside kubernetes.io rather than at github.com's medium-high tier, because they are the projects' authoritative docs. GitHub *issues* (community discussion) are scored at github.com's 0.8 tier and used only as corroboration, never as a sole source.

Reputation: High: 11 (~73%) | Medium-High: 3 (~20%) | Medium: 1 (~7%) | Avg: ~0.93

## Knowledge Gaps

### Gap 1: No single source pins the literal "O(N²) list-inside-per-item-reconcile" phrase
**Issue**: The Kubebuilder Book names the O(N) "filter through all of them" cost per reconcile and the field-index remedy, but no consulted source frames the *per-item × per-list* product as "O(N²)" in those words. The O(S²) characterization here is the project's own RCA arithmetic (one full scan per target, S targets), corroborated by — not lifted from — the literature's O(N)-per-reconcile + "avoid uncached lists / do at most once per loop" guidance.
**Attempted**: searches for "O(N^2) reconcile", "quadratic controller list" against kubernetes.io / github.com.
**Recommendation**: Treat the O(S²) figure as Overdrive-internal (from the RCA), with the literature supporting each factor (per-reconcile list cost is O(N); doing it per-target multiplies it). Confidence on the *industry remedy* is High; confidence on the literal "O(N²)" framing being an industry term is Low — it is descriptive, not a cited term of art.

### Gap 2: Overdrive's intent store has no built-in watch/generation primitive surfaced in this research
**Issue**: The K8s model assumes a watch stream + monotonic generation from the API server/etcd. Whether Overdrive's `redb`-backed `IntentStore` (and the future `RaftStore`) already exposes a write-event hook or a monotonic revision per intent was not verified from source in this research pass (it is an implementation question, not a literature question).
**Attempted**: out of scope for an external-literature research task; flagged for the implementer.
**Recommendation**: Before choosing (d) (write-through cache) over (c) (reverse index), confirm the intent-write path can emit an invalidation signal or expose a per-intent generation — the architecturally-correct invalidation key per Findings 9–10. If no write hook exists, (b)+(c) is the lower-coupling interim.

## Conflicting Information

No substantive conflicts among authoritative sources. All converge on: read desired state from a watch-maintained cache, not by re-listing the store; index for scoped lookup; invalidate on write-event/generation, not on a timer. The one *apparent* tension — Argo CD polling Git every 3 minutes (Finding 7) — is not a counterexample: Git is an external desired-state source Argo CD cannot watch cheaply, whereas it *does* watch the cluster state it owns. Overdrive owns its intent store and can observe writes, so the analogy points to event-driven (not polled) hydration.

## Ranked Recommendation (mapping literature → candidates a–d)

**The industry-canonical answer is the informer/indexer model: read desired state from an in-process cache kept current by write events, and use a field index for scoped lookups — never re-list the durable store inside the reconcile path.** Mapped onto Overdrive's candidates, ranked best-to-worst as the *end state*, with a pragmatic sequencing note:

**Rank 1 — (d) fold listener facts into the bulk-load + write-through in-memory view path (zero steady-state durable reads).**
This is the literal Kubernetes informer-cache pattern (Findings 1, 4, 5) and the etcd-watch/Argo-CD live-state-cache pattern (Findings 6, 7): desired state is served from RAM, the durable store is touched only on write-through and cold boot — which is *already* Overdrive's documented contract for reconciler Views. It also keeps `reconcile` pure per Anvil/ESR (Finding 8). The usual objection to a full cache — memory (Finding 11) — does not bite: listener facts are a few bytes × S. **This is the architecturally-aligned destination and the project's own runtime already implements the machinery for it.** The one prerequisite (Gap 2): the intent-write path must drive the cache update (event-driven), which is the canonical invalidation discipline (Findings 9, 10).

**Rank 2 — (c) scope the read to the target via a reverse index `vip/service_id → owning intent`.**
This is the field-indexer remedy (Finding 3) — the *named* canonical fix for "find objects matching field X without listing everything." It eliminates the full scan (O(S²) → O(S) keyed lookups, each cheap) while keeping the durable store as SSOT and adding minimal memory (Finding 11). It is the correct choice if the working set were large enough that full caching (d) cost real memory, or if the intent-write hook needed for (d)'s event-driven update does not yet exist (Gap 2). For Overdrive's tiny listener-fact set, (c) is strictly a stepping stone to (d) rather than a different destination.

**Rank 3 — (b) hoist the cluster-wide scan to once per tick (O(S²) → O(S)).**
Defensible **only as a minimal-risk interim**. It brings the code into partial compliance with "make uncached list calls happen at most once in a reconcile loop" (Finding 5) and removes the quadratic factor with a near-trivial change. But it still does an *uncached full scan + full decode every tick* — exactly the "re-queue every few seconds to check if something changed" / poll-to-detect-change pattern the literature calls inferior (Findings 4, 7) and Argo CD treats as a regression (#27192). Ship it to stop the bleeding; do not call it done.

**Rank 4 (discouraged) — (a) memoise `spec_digest → Vec<ListenerRow>` in a side-cache invalidated on intent submit/stop.**
The *intent* (invalidate on write) is correct and matches event/key-based invalidation (Finding 10). The **specified key is the problem**: computing `spec_digest` requires decoding the intent on every tick to check cache validity, re-incurring the decode cost the cache exists to avoid (Finding 9) — unless the digest/generation is *published by the writer at submit time* rather than recomputed by the reader. As written ("memoise by `spec_digest`"), it risks the self-polling-cache failure mode the invalidation literature warns against (Finding 10: a cache that polls to validate itself degrades to TTL/poll). If pursued, replace the recomputed digest key with a **writer-bumped monotonic generation/revision** (the `metadata.generation`/etcd-revision pattern, Findings 6, 9) — at which point (a) converges onto (d)'s invalidation mechanism and loses its distinct identity.

**Recommended path for this codebase**: ship **(b)** immediately if a fast bleed-stop is needed (one-line-ish, removes the quadratic), then land **(d)** as the real fix — it is the informer-cache pattern, it reuses the runtime's existing bulk-load + write-through machinery, its memory cost is negligible for listener facts, and it restores the project's own "zero steady-state durable reads" contract. Use **(c)** instead of (d) only if Gap 2 reveals no intent-write hook to drive event-driven invalidation, or if a future surface makes the cached set large. Avoid **(a)** as specified; if its ergonomics are wanted, re-key it on a writer-published generation so it becomes (d)-with-a-side-table rather than a self-validating digest cache.

## Recommendations for Further Research

1. **Verify the intent-store write path (Gap 2).** Source-read `LocalStore` / `IntentStore` write surface for an existing event hook or per-intent monotonic version; this decides (d) vs (c) and whether (a) can be salvaged into a generation-keyed form.
2. **DST invariant for hydration cost.** Per the project's testing tiers, add an assertion that steady-state ticks perform zero durable reads for listener-fact hydration (mirrors the `WriteThroughOrdering` View invariant) so a regression to the scan-every-tick shape fails loudly.
3. **kube-rs precedent.** A Rust-native cross-reference: kube-rs `reflector`/`Store` and `Controller` implement the informer-cache pattern in Rust (kube-rs/kube #148 surfaced in search) and may offer a closer API shape to Overdrive than the Go controller-runtime; worth a focused read if implementation borrows are wanted.

## Full Citations

[1] The Kubernetes Authors. "Controllers". Kubernetes Documentation. https://kubernetes.io/docs/concepts/architecture/controller/. Accessed 2026-06-03.
[2] Kubernetes SIG Architecture. "Kubernetes API Conventions — Generation / ObservedGeneration". kubernetes/community. https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md. Accessed 2026-06-03.
[3] Kubernetes SIGs. "cache package — sigs.k8s.io/controller-runtime/pkg/cache". Go Packages. https://pkg.go.dev/sigs.k8s.io/controller-runtime/pkg/cache. Accessed 2026-06-03.
[4] Kubernetes SIGs. "client package — FieldIndexer / MatchingFields". Go Packages. https://pkg.go.dev/sigs.k8s.io/controller-runtime/pkg/client. Accessed 2026-06-03.
[5] Kubernetes. "cache package — k8s.io/client-go/tools/cache (SharedIndexInformer, DeltaFIFO, Indexer)". Go Packages. https://pkg.go.dev/k8s.io/client-go/tools/cache. Accessed 2026-06-03.
[6] The Kubebuilder Authors. "Implementing a controller — Indexing for efficient lookup". The Kubebuilder Book. https://book.kubebuilder.io/cronjob-tutorial/controller-implementation. Accessed 2026-06-03.
[7] The Cluster API Authors. "Tuning controllers". The Cluster API Book. https://cluster-api.sigs.k8s.io/developer/core/tuning. Accessed 2026-06-03.
[8] The Operator Framework Authors. "Controller Runtime Client API (split client)". Operator SDK. https://sdk.operatorframework.io/docs/building-operators/golang/references/client/. Accessed 2026-06-03.
[9] The etcd Authors. "etcd API — Watch and Revisions". etcd Documentation v3.6. https://etcd.io/docs/v3.6/learning/api/. Accessed 2026-06-03.
[10] anvil-verifier. "Anvil — README". GitHub. https://github.com/anvil-verifier/anvil/blob/main/README.md. Accessed 2026-06-03.
[11] Sun, Xudong et al. "Anvil: Verifying Liveness of Cluster Management Controllers". USENIX OSDI '24. https://www.usenix.org/system/files/osdi24-sun-xudong.pdf. Accessed 2026-06-03.
[12] Sun, Xudong et al. "Anvil". Proceedings of the 18th USENIX OSDI. ACM Digital Library. https://dl.acm.org/doi/10.5555/3691938.3691973. Accessed 2026-06-03.
[13] The Argo CD Authors. "FAQ / live state cache". Argo CD Documentation. https://argo-cd.readthedocs.io/en/latest/faq/. Accessed 2026-06-03.
[14] kubernetes-sigs/controller-runtime. "Issue #498 — Understanding Client/Informer/Indexer interaction in Reconcile". GitHub. https://github.com/kubernetes-sigs/controller-runtime/issues/498. Accessed 2026-06-03.
[15] kubernetes-sigs/controller-runtime. "Issue #2570 — Provide a way to create more selective cache to tackle memory consumption". GitHub. https://github.com/kubernetes-sigs/controller-runtime/issues/2570. Accessed 2026-06-03.
[16] argoproj/argo-cd. "Issue #27192 — reconciliations timer-driven, no watch events fired (regression)". GitHub. https://github.com/argoproj/argo-cd/issues/27192. Accessed 2026-06-03.
[17] argoproj/argo-cd. "Issue #18838 — live state cache List&Watch pattern". GitHub. https://github.com/argoproj/argo-cd/issues/18838. Accessed 2026-06-03.
[18] Devinterview-io. "Caching interview questions (strategy taxonomy: TTL / event / key / generations)". GitHub. https://github.com/Devinterview-io/caching-interview-questions. Accessed 2026-06-03.

## Research Metadata

Duration: ~1 session | Examined: ~22 sources | Cited: 18 | Cross-refs: every major finding ≥2 independent sources (most ≥3) | Confidence distribution: High ~80%, Medium-High ~15%, Low ~5% (the literal "O(N²)" framing only) | Output: docs/research/control-plane/reconciler-desired-hydration-efficiency.md
