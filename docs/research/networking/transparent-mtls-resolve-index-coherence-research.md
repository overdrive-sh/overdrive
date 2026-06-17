# Research: Building a coherent address-keyed resolve cache over a forward-only lossy watch stream — on a security boundary where staleness degrades to cleartext, and what Cilium does in production

**Date**: 2026-06-17 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (canonical pattern + Cilium grounding + verdict) | **Sources**: 24 (10 primary code [5 Overdrive + 5 Cilium] + 14 external; avg reputation ≈ 0.96)

> **Framing (the live DELIVER decision).** Overdrive's v1 transparent-mTLS host
> adapter (`ServiceBackendsResolve`) builds an in-RAM `addr → Backend` reverse
> index from a **forward-only, bounded, lazily-drained, lossy** observation
> watch stream (`ObservationStore::subscribe_all`). A miss classifies as
> `NonMesh` → **the connection proceeds in CLEARTEXT (fail-open, by design)**.
> Therefore **any staleness that turns a real mesh backend into a miss is a
> silent-cleartext security hole.** An adversarial review found three distinct
> silent-cleartext hazards (cold-start/#237, concurrency-race/F2,
> lag-drop/F4) all stemming from the same mechanism. This doc establishes the
> canonical pattern for the class ("coherent local cache over a lossy
> forward-only watch" = List-then-Watch + relist-on-loss + periodic resync),
> grounds what Cilium actually does in production (file:line), and takes a
> decisive position on Overdrive's Option (a) [fail-toward-handshake, remove the
> *security* consequence of staleness] vs Option (b) [add a bounded List/snapshot
> + resync, make the cache coherent].

## Executive Summary

**The three hazards are one classic problem with a textbook answer.** Overdrive's
v1 resolve adapter builds an in-RAM `addr → Backend` cache from a forward-only,
bounded, lazily-drained, lossy watch stream (`ObservationStore::subscribe_all`),
and a cache miss degrades to cleartext (`NonMesh`, fail-open). The adversarial
review's three findings — #237 cold-start, F2 concurrency, F4 lag-drop — are not
three bugs but three faces of the **"coherent local cache over a lossy
forward-only watch"** problem that distributed systems solved a decade ago. The
canonical solution, established here with primary sources from three independent
systems, is **List-then-Watch + relist-on-loss + periodic resync**: the
Kubernetes informer/reflector (`ListAndWatchWithContext` + `isExpiredError`-relist
+ `resyncChan`, B1), the etcd watch contract (`ErrCompacted`/`CompactRevision` →
re-list from a current snapshot, B2), and Envoy xDS (SotW complete-snapshot /
Delta `removed_resources` + version/nonce/ACK, B3). All three agree on the same
load-bearing fact, restated by tokio's own docs for the exact primitive Overdrive
uses (`broadcast::RecvError::Lagged`, B4): **a bare forward-only stream cannot
self-certify completeness or recover missed events; the consumer must re-acquire
an authoritative snapshot.** Overdrive's `subscribe_all` provides no snapshot and
no completeness marker, and the adapter discards the `Lagged` loss signal
(`ok_or_skip`, A3) — so **observe-only-without-resync is the known-broken shape
the pattern exists to fix**, and Overdrive v1 is built in exactly that shape.

**What Cilium actually does (file:line, the code is the authority).** Cilium's
kvstore watcher does List-before-Watch with a one-time `EventTypeListDone` signal
that gates a `synced` flag (the `HasSynced()` analog) — the cache is never
empty-but-trusted during convergence (A5, `etcd.go:697-779`,
`watchstore.go:161-165`). On `ErrCompacted` **or any other watch error**, it
`goto reList` — a full re-List — and reconciles deletions missed during the gap
via stale mark/sweep (A6, `etcd.go:823-844`, `watchstore.go:153-225`). It has a
producer-side periodic resync (A7, `store.go:228-238`). Its concurrency model is
a single `RWMutex`: per-connection readers `RLock`, the event writer `Lock` (A8) —
the same shape Overdrive already uses. **On the security crux: Cilium fail-CLOSES
on a stale/incomplete cache for enforced flows** — an auth-required flow with an
auth-map miss returns `DROP_POLICY_AUTH_REQUIRED`, never cleartext (A9,
`bpf/lib/auth.h:45-53`). This is the exact inverse of Overdrive v1's
`NonMesh`-cleartext-on-miss. Crucially, even ztunnel — the mesh Overdrive's #236
model is shaped on — has a real fail-open-on-missing-destination path that leaks
plaintext (istio/ztunnel #369, B5), confirming the hazard is genuine and that the
planned #236 fail-toward-handshake is a real hardening, not paranoia.

**Verdict (decisive).** Cache coherence and miss-meaning are **two orthogonal
levers**, and Overdrive v1 is the only system examined that engages *neither*
(bare observe × fail-open). The fix is a **sequenced hybrid**: (1) fix F2's TOCTOU
race now, unconditionally; (2) adopt **Option (a) fail-toward-handshake (#236) as
the stated v1 security invariant** — "a resolve miss must never silently emit
cleartext to a should-be-mesh peer" — because as long as a miss means cleartext,
*no* coherence engineering fully closes the irreducible convergence window
(cold-boot, backend-just-came-up); only changing what a miss MEANS removes the
security consequence (this is precisely what Cilium's auth-drop and ztunnel's
source-drop do); (3) for **single-node v1 shipping before #236**, close #237 with
a **bounded List-at-probe** — a minimal slice of Option (b): a keyless
`service_backends_rows()` snapshot drained once at probe before the Earned-Trust
gate opens, consistent with the `alloc_status_rows()`/`node_health_rows()`
enumerators that already exist (A2) and with the in-house bulk-load precedent that
the C4 text itself cited (A4); (4) defer the full relist-on-`Lagged` (the rest of
Option b) and (a)'s handshake machinery to the #236 multi-node arc. **Do not ship
the current bare-observe + fail-open-on-miss as the permanent v1 story — it is the
one shape no production mesh uses.**

## Research Methodology

**Search Strategy**: Two mandatory evidence streams. **Stream A — codebase
grounding**: read Cilium (`/Users/marcus/git/cilium/cilium`) and Overdrive
(this repo) with file:line citations. **Stream B — external evidence**: web
search across the embedded trusted-domain config (kubernetes.io, etcd.io,
envoyproxy.io, docs.rs/tokio, cilium.io, istio.io, linkerd.io, github.com
primary sources).

**Source Selection**: Primary sources strongly preferred — k8s/etcd/Envoy/tokio
official docs + client-go reflector source over blogs. Each load-bearing external
claim cross-referenced against ≥2 trusted sources (ideally 3).

**Quality Standards**: 3 sources/claim ideal, 2 acceptable, 1 authoritative
minimum. Where Cilium code and docs disagree, trust the code (cite the line).

## Findings

### Stream A — Codebase grounding

#### A1. Overdrive: the current resolve-index mechanism (the thing under review)

**Evidence (file:line, this repo):** `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs`.

The adapter holds three pieces of state (lines 174–197):
- `store: Arc<dyn ObservationStore>` — the injected observation surface;
- `index: RwLock<BackendIndex>` — the in-RAM `addr → Backend` reverse index
  (`by_addr: BTreeMap<SocketAddrV4, Backend>` + `addrs_by_service: BTreeMap<ServiceId, Vec<SocketAddrV4>>`, lines 116–127);
- `subscription: tokio::sync::Mutex<Option<ObservationSubscription>>` — a
  **single, persistent, forward-only** `subscribe_all` subscription opened
  lazily on first probe/resolve and held for the adapter's lifetime (lines 184–196).

The refresh path (`refresh_index`, lines 232–268) is the mechanism under review:

```rust
let taken = self.subscription.lock().await.take();          // line 245 — TAKE out of the mutex
let mut subscription = match taken {
    Some(existing) => existing,
    None => self.store.subscribe_all().await.map_err(...)?,  // line 248 — open on first call
};
...
while let Some(Some(row)) = subscription.next().now_or_never() {  // line 256 — LAZY drain, ready-only
    if let ObservationRow::ServiceBackend(row) = row {
        self.index.write().apply_row(row.service_id, &row.backends);  // line 258
        ingested += 1;
    }
}
*self.subscription.lock().await = Some(subscription);        // line 266 — RESTORE
```

Three observable properties confirm the mechanism is **observe-only, with no
List/snapshot and no resync**:
1. **No bulk-load / List.** The index starts empty (`new`, line 208–214: "The
   index starts empty and no subscription is open yet") and is only ever
   populated by draining the held subscription. There is no initial enumeration
   of the existing `service_backends` rows — confirmed by the module rustdoc
   (lines 36–46): "built and refreshed from the EXISTING `subscribe_all`
   observation surface … NOT a per-`ServiceId` point query, and WITHOUT adding
   any new trait method."
2. **Lazy, ready-only drain.** `now_or_never()` (line 256, `futures::FutureExt`)
   pulls only rows *already buffered* on the subscription and stops at the first
   pending poll. Rows arrive between calls and sit in the broadcast buffer until
   the next `resolve`/`probe` drains them — there is no background task draining
   continuously.
3. **Take-then-restore is a non-atomic check-and-act (F2).** Lines 245–266: the
   guard is released after `.take()`, the subscription is owned locally across
   the drain, then re-stored. The rustdoc (lines 235–249) frames this as keeping
   the mutex scope tight, but it means two concurrent `resolve`s that both find
   `None` (or whose `take()` interleaves) each open a *fresh* `subscribe_all`,
   and the loser's drained-but-not-restored subscription is dropped — a churn
   that also loses any rows that arrived on the dropped subscription. This is F2.

The classification (`classify`, lines 160–168) is where staleness becomes a
**security** property: `by_addr.get(&orig_dst)` returning `None` → `NonMesh`
(line 166), which the module rustdoc (lines 56–60) and the port contract define
as **cleartext pass-through, by design**. So a backend that *should* be in the
index but is missing because of cold-start/race/lag is indistinguishable from a
genuinely non-mesh address — both yield `NonMesh` → cleartext.

**Source**: `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:116-168, 174-268`
(this repo, read 2026-06-17). **Confidence**: High (primary source, the code under review).

#### A2. Overdrive: the ObservationStore surface asymmetry (no keyless `service_backends` enumerate)

**Evidence (file:line):** `crates/overdrive-core/src/traits/observation_store.rs`.

The trait deliberately exposes **keyless (List-shaped) enumerators for some row
classes but NOT for `service_backends`**:
- `subscribe_all(&self) -> Result<ObservationSubscription, …>` (line 1201) — the
  forward-only watch. Its rustdoc is exactly one line: "Subscribe to every
  observation row written to this peer" (line 1200) — no replay contract stated
  on the trait itself; the replay/future-only semantics live in the adapter
  rustdoc (A3).
- `alloc_status_rows(&self) -> Result<Vec<AllocStatusRow>, …>` (line 1214) — a
  **keyless List**: "Read a deterministic snapshot of every `alloc_status` row
  this peer currently holds as LWW winner … Iteration order is deterministic"
  (lines 1203–1213). This is precisely a List/snapshot surface.
- `node_health_rows(&self) -> Result<Vec<NodeHealthRow>, …>` (line 1239) — also
  a keyless List.
- `issued_certificate_rows(&self) -> …` (line 1267) — keyless List.
- `service_backends_rows(&self, service_id: &ServiceId) -> …` (line 1383) — the
  **only** backend-read surface, and it is **keyed by `ServiceId`**. There is no
  `service_backends_rows()` (keyless) enumerate.

This asymmetry is the crux of the architecture choice. The resolve adapter is
handed an arbitrary `orig_dst: SocketAddrV4` and holds **no `ServiceId`** (module
rustdoc, `mtls_resolve_adapter.rs:36-46`), so the keyed point query is the wrong
surface — it cannot enumerate all services to build the reverse index. The
informer "List" step (Option b) would require adding a keyless
`service_backends_rows()` to this trait (+ both adapters), which is exactly the
surface the design (ADR-0071 C4 / D-TME-11) forbade adding. **The keyless
enumerators already present for `alloc_status`/`node_health` prove the pattern is
not foreign to the trait — the design chose not to extend it to `service_backends`.**

**Source**: `crates/overdrive-core/src/traits/observation_store.rs:1198-1239, 1267-1383`
(this repo, read 2026-06-17). **Confidence**: High (primary source).

#### A3. Overdrive: both store adapters silently drop `Lagged` — the F4 root

**Evidence (file:line):** the sim and local store adapters both implement
`subscribe_all` over a **bounded `tokio::sync::broadcast` channel (capacity
1024)** and **silently discard `Lagged`**.

`crates/overdrive-sim/src/adapters/observation_store.rs`:
- `const DEFAULT_FANOUT_CAPACITY: usize = 1024;` (line 75); the fan-out is a
  `broadcast::channel(DEFAULT_FANOUT_CAPACITY)` (line 164).
- `subscribe_all` wraps the receiver in `BroadcastStream::new(rx).filter_map(ok_or_skip)`
  (lines 494–498).
- `ok_or_skip` (lines 657–658) is `futures::future::ready(item.ok())` — i.e. it
  **maps every `Err` (including `BroadcastStreamRecvError::Lagged(n)`) to `None`
  and drops it**. The rustdoc (lines 651–656) states this verbatim: "drops any
  `Lagged` signal emitted by `BroadcastStream` when the subscriber has fallen
  behind the `DEFAULT_FANOUT_CAPACITY` window … surfacing it as a stream value
  would force every caller to handle a variant they cannot do anything about."

`crates/overdrive-store-local/src/observation_backend.rs`:
- `const SUBSCRIPTION_CHANNEL_CAPACITY: usize = 1024;` (line 270);
  `broadcast::channel(SUBSCRIPTION_CHANNEL_CAPACITY)` (line 374).
- The module rustdoc (lines 29–38) is explicit: "Subscribers that lag past the
  broadcast capacity **drop the lagged notifications silently** and continue
  delivering subsequent events; the stream does not close, **so a caller relying
  on end-of-stream as a catch-up trigger will miss the lost events.** Phase 2's
  Corrosion replacement recovers via CR-SQLite gossip catch-up."

**This is the F4 root, confirmed in code.** The resolve adapter's single held
subscription is the *whole observation firehose* (every `ObservationRow` class —
alloc_status, node_health, service_backends, probe results, …) over one 1024-deep
buffer. Because the adapter drains lazily (A1), a quiet adapter between two bursts
of >1024 writes will silently lose `service_backends` updates: the broadcast
overwrites the oldest buffered messages, `BroadcastStream` would yield
`Err(Lagged(n))`, and `ok_or_skip` discards it. The adapter has **no resync path**
— it never re-reads an authoritative snapshot — so the dropped `service_backends`
row leaves the index **permanently stale** until that service's *next* write
happens to be drained in time. A permanently-stale index entry that should be a
mesh backend → `NonMesh` → cleartext.

Critically, the store-local rustdoc names the exact anti-pattern from the tokio
docs (B4): a consumer "relying on end-of-stream as a catch-up trigger will miss
the lost events" — i.e. the broadcast does NOT signal loss in a way the current
adapter observes; the loss is silent by construction.

**Source**: `crates/overdrive-sim/src/adapters/observation_store.rs:75, 164, 494-498, 651-658`;
`crates/overdrive-store-local/src/observation_backend.rs:29-38, 270, 374`
(this repo, read 2026-06-17). **Confidence**: High (primary source, two independent adapters agree).

#### A4. Overdrive: the reconciler-runtime bulk-load precedent — does it transfer?

**Evidence:** `.claude/rules/development.md` § "Reconciler I/O" → "Runtime
mechanics — bulk-load + write-through". The in-house precedent the C4 text
invoked as "bulk-load-then-observe" is the reconciler runtime's `ViewStore`
handling:

> "**Boot / register-time** (once per reconciler at process start): … `views =
> view_store.bulk_load::<R::View>(name)` … The runtime calls `bulk_load` once per
> reconciler and materialises every persisted `(reconciler_name, target) → View`
> blob into a per-reconciler `BTreeMap<TargetResource, View>` held in RAM. From
> that moment on the in-memory map is the steady-state read SSOT."
> "**Steady-state tick** … `view_store.write_through(name, target, &next_view)`
> (fsync) … `views.insert(...)` (after fsync OK)."

**Assessment: the precedent is structurally the SAME pattern the informer uses
(List-then-Watch), but it does NOT transfer to the resolve adapter as-is — and
that is the whole problem.** Two reasons:
1. **It bulk-loads the runtime's OWN `ViewStore`, not `service_backends`.** The
   `ViewStore` exposes a `bulk_load` (List) surface by design; `ObservationStore`
   exposes `bulk_load`-shaped enumerators for `alloc_status`/`node_health` but
   **not** for `service_backends` (A2). So the precedent demonstrates Overdrive
   *already does* List-then-Watch where the List surface exists — it just was not
   given one for `service_backends`.
2. **The reconciler runtime never relies on the broadcast for correctness of its
   own state.** Its steady-state read SSOT is the in-RAM `BTreeMap` populated by
   `bulk_load` at boot + `write_through` after each tick — both authoritative,
   loss-free, fsync-ordered paths. It does *not* reconstruct its `View` state from
   `subscribe_all`. The resolve adapter, by contrast, has *only* the lossy
   broadcast as its source — no bulk-load at boot, no authoritative write-through.

So the precedent actually **argues for Option (b)**: the in-house pattern that the
C4 text cited as justification is itself a List(bulk-load)-then-observe(write-through)
loop over an authoritative surface — i.e. exactly the informer shape — and the
resolve adapter is the one place that was built *without* the List half.

**Source**: `.claude/rules/development.md` § "Reconciler I/O" → "Runtime
mechanics — bulk-load + write-through" (this repo). **Confidence**: High (primary,
in-repo SSOT rule doc).

#### A5. Cilium: startup cold-start — List-before-Watch with a one-time `ListDone` sync signal (the #237 analog)

**Evidence (file:line, `/Users/marcus/git/cilium/cilium`):** Cilium's kvstore
watcher does **List-before-Watch**, not observe-only. `pkg/kvstore/etcd.go`,
`func (e *etcdClient) watch(...)` (line 666):

- A top-level `reList:` loop label (line 683) wraps the whole list-then-watch
  cycle.
- Each iteration FIRST does a full enumeration:
  `kvs, revision, err := e.paginatedList(ctx, scopedLog, prefix)` (line 697),
  logging "Successfully listed keys before starting watcher" with the count
  (lines 721–725).
- It emits every listed key as a `Create`/`Modify` event into the consumer
  stream (lines 727–750), then emits a **one-time** `EventTypeListDone` signal:
  "Only send the list signal once … `events.emit(ctx, KeyValueEvent{Typ: EventTypeListDone})`"
  (lines 773–779).
- ONLY THEN does it open the watch from the post-list revision:
  `etcdWatch := e.client.Watch(..., client.WithRev(nextRev))` (lines 799–800,
  with `nextRev := revision + 1` at line 752).

The consumer side gates "the cache is coherent" on that `ListDone` signal.
`pkg/kvstore/store/watchstore.go`, `restartableWatchStore.Watch` (line 126):

```go
for event := range events {
    if event.Typ == kvstore.EventTypeListDone {   // line 161
        rws.log.Debug("Initial synchronization completed")
        rws.drainKeys(true)                       // line 163 — sweep stale
        syncedMetric.Set(...true)                 // line 164
        rws.synced.Store(true)                    // line 165 — cache now coherent
        for _, callback := range rws.onSyncCallbacks { callback(ctx) }  // line 167
        ...
    }
    ...
}
```

`Synced()` returns `rws.synced.Load()` (lines 199–202). This is the direct
analog of a Kubernetes informer's `HasSynced()` — a boolean "the initial List
completed; the in-RAM cache now reflects the full backend state" gate. **Cilium
never serves an empty cache as authoritative during convergence: it lists first,
signals `ListDone`, and only then is `synced`.** This is precisely the mechanism
Overdrive's resolve adapter lacks (#237 cold-start).

**Source**: `pkg/kvstore/etcd.go:666, 683, 697, 721-779, 799-800`;
`pkg/kvstore/store/watchstore.go:126, 153-176, 199-202` (Cilium checkout,
read 2026-06-17). **Confidence**: High (primary source, the production code).

#### A6. Cilium: missed/dropped events — relist-on-loss with stale mark/sweep reconciliation (the F4 analog)

**Evidence (file:line):** Cilium recovers from BOTH etcd history compaction AND
any other watch error by **re-listing the full keyspace** and reconciling
additions and deletions missed during the gap.

`pkg/kvstore/etcd.go`, inside the watch loop (lines 815–845):

```go
if err := r.Err(); err != nil {
    switch {
    ...
    case errors.Is(err, v3rpcErrors.ErrCompacted):           // line 823
        scopedLog.Info("Tried watching on compacted revision. Triggering relist of all keys", ...)  // line 828
    default:
        scopedLog.Info("Etcd watcher errored. Triggering relist of all keys", ...)  // line 833
    }
    // mark all local keys in state for deletion unless the upcoming GET marks them alive
    localCache.MarkAllForDeletion()                          // line 842
    goto reList                                              // line 844
}
```

Two properties make this a *correct* resync, not just a reconnect:
1. **`ErrCompacted` → relist (lines 823–828).** When etcd has compacted away the
   revision the watch wanted (the direct analog of `tokio::broadcast::Lagged` —
   "you missed events, they're gone"), Cilium does NOT ignore it and continue
   from the next event; it `goto reList` (line 844) back to the full
   `paginatedList`. This is the etcd-canonical recovery (B2).
2. **`default:` → relist (lines 832–833) — ANY watch error triggers a full
   relist.** Cilium does not even need a *specific* loss signal: any watcher
   error path falls back to a full List.
3. **Stale mark/sweep reconciles deletions (line 842 + watchstore.go).**
   `MarkAllForDeletion()` flags every cached key; the re-List re-marks the
   survivors alive; keys NOT seen in the new List are emitted as deletions. The
   consumer mirrors this: `restartableWatchStore.Watch` marks all known keys
   `stale = true` on (re)start (lines 153–156), and on `ListDone` calls
   `drainKeys(true)` (line 163) which "Emit[s] deletion event for stale key"
   (line 225) for every entry still stale after the relist (lines 221–225). So
   a backend that was DELETED during the watch gap is correctly evicted by the
   relist — the cache cannot strand a stale entry.

`restartableWatchStore.Watch` is explicitly designed to be re-run: "It might be
executed multiple times, granted that the previous execution already terminated"
(line 125). The watch restart and the cache resync are one mechanism.

**This is the exact mechanism Overdrive's resolve adapter lacks (F4).** On
`Lagged`, Overdrive's `ok_or_skip` (A3) maps the loss to `None` and *continues
from the next event* with no relist — the opposite of Cilium's `goto reList`.

**Source**: `pkg/kvstore/etcd.go:815-845`; `pkg/kvstore/store/watchstore.go:125,
153-156, 161-176, 221-225` (Cilium checkout, read 2026-06-17). **Confidence**:
High (primary source).

#### A7. Cilium: periodic resync (producer-side)

**Evidence (file:line):** `pkg/kvstore/store/store.go`. The `SharedStore` config
carries a `SynchronizationInterval` (lines 49–52): "the interval in which locally
owned keys are synchronized with the kvstore. Defaults to 0 (i.e., no periodic
synchronization is performed) if unset." When set (>0), a controller periodically
re-pushes locally owned keys (lines 228–238):

```go
if s.conf.SynchronizationInterval > 0 {
    controllers.UpdateController(s.controllerName, controller.ControllerParams{
        DoFunc:      func(ctx context.Context) error { return s.syncLocalKeys(ctx, true) },
        RunInterval: s.conf.SynchronizationInterval,
    })
}
```

This is a *producer-side* periodic resync (each agent periodically re-asserts the
keys it owns into the kvstore, repairing any drift), complementing the
*consumer-side* relist-on-error (A6). Together they give the informer triad:
List-then-Watch (A5) + relist-on-loss (A6) + periodic resync (A7). Note: the
**read** side's primary drift repair is the relist-on-error of A6; the periodic
interval here is the write-ownership reassertion. Cilium thus implements all
three legs of the canonical pattern, distributed across producer and consumer.

**Source**: `pkg/kvstore/store/store.go:49-52, 228-238` (Cilium checkout, read
2026-06-17). **Confidence**: High (primary source). **Caveat**: this is the
`SharedStore` periodic write-sync, not a read-side periodic re-List; the read
side's drift repair is A6's relist-on-error. Flagged so the architect does not
over-read it as a read-side resync.

#### A8. Cilium: concurrency model (per-connection reader vs event writer)

**Evidence (file:line):** `pkg/ipcache/ipcache.go`. The `IPCache` struct holds a
single `mutex lock.SemaphoredMutex` (line 119, constructed line 165). The model
is a standard reader/writer split over one lock:
- **Readers take `RLock`.** `LookupByIP` (lines 812–819): `ipc.mutex.RLock();
  defer ipc.mutex.RUnlock(); return ipc.lookupByIPRLocked(IP)` — a point lookup
  against `ipc.ipToIdentityCache[IP]` (lines 824–827). Public `RLock`/`RUnlock`
  helpers (lines 185–192) let the datapath hold the read lock across a lookup.
  `getHostIPCacheRLocked`, `getEndpointFlagsRLocked` etc. are all RLock readers
  (lines 220–257).
- **The event-apply / upsert writer takes the write lock.** `ipc.mutex.Lock()`
  (line 199, line 277) in the mutating paths (`upsertLocked` line 297 and the
  metadata-injection path). The semaphored mutex even supports `UnlockToRLock()`
  (line 205) to downgrade a write lock to a read lock without a gap.

So the per-connection reader (`LookupByIP`, RLock) and the event-ingest writer
(`upsertLocked`, Lock) share one `RWMutex`-shaped lock — reads run concurrently,
writes are exclusive. This is the same shape Overdrive's adapter uses
(`index: RwLock<BackendIndex>`, `parking_lot::RwLock`, `mtls_resolve_adapter.rs:183`)
— so the concurrency *model* is already aligned; the gap is purely the coherence
mechanism (List/relist), not the locking.

**Source**: `pkg/ipcache/ipcache.go:119, 165, 185-192, 199-205, 220-257,
277-297, 812-827` (Cilium checkout, read 2026-06-17). **Confidence**: High
(primary source).

#### A9. Cilium: fail-open vs fail-closed on cache miss / stale cache (THE CRUX)

**Evidence (file:line):** This is the load-bearing comparison. Cilium's security
boundary is **fail-CLOSED on a missing/stale auth entry** — the opposite of
Overdrive's `NonMesh`-cleartext-on-miss.

Two layers:

1. **ipcache miss → `WORLD_ID`, then POLICY decides (not auto-allow).** When the
   ipcache has no entry for a remote IP, the datapath assigns the reserved
   `world` identity rather than failing open to "allow". `bpf/lib/identity.h`
   (lines 270–283): an unidentified packet's `*identity` falls to `WORLD_IPV4_ID`
   / `WORLD_IPV6_ID` / `WORLD_ID`. `world` is a real identity that policy
   evaluates — it is NOT a bypass. (This re-confirms and sharpens the prior
   sibling doc's finding.)

2. **Mutual-auth required + auth-map miss → `DROP`, NOT cleartext.** This is the
   decisive evidence. `bpf/lib/auth.h`, `auth_lookup` (lines 21–54):

```c
auth = map_lookup_elem(&cilium_auth_map, &key);   // line 45
if (likely(auth)) {
    if (utime_get_time() < auth->expiration)      // line 48
        return CTX_ACT_OK;                        // line 49 — authenticated, allow
}
send_signal_auth_required(ctx, &key);             // line 52 — kick the agent to auth
return DROP_POLICY_AUTH_REQUIRED;                 // line 53 — DROP on miss/expiry
```

When a flow requires mutual auth and the `cilium_auth_map` has **no entry**
(the auth state is incomplete/stale) OR the entry has **expired** (line 48), the
program returns `DROP_POLICY_AUTH_REQUIRED` (line 53) and signals the agent to
establish authentication — **it drops the packet; it does not let it through in
cleartext.** A missing/stale auth-cache entry **fails closed**.

**The sharp contrast with Overdrive:**

| | Cache state | Cilium (auth-required flow) | Overdrive v1 resolve |
|---|---|---|---|
| Hit, authenticated/healthy | present | `CTX_ACT_OK` (encrypted) | `Mesh` (mTLS) |
| Present but unhealthy/expired | present, bad | `DROP` (auth.h:48,53) | `MeshUnreachable` (fail-closed) ✓ |
| **MISS (true or stale)** | **absent** | **`DROP_POLICY_AUTH_REQUIRED` (fail-closed)** | **`NonMesh` → CLEARTEXT (fail-OPEN)** ✗ |

For the *enforced* (auth-required) flows, Cilium **never** silently allows
cleartext on a cache miss or stale cache — it drops and triggers re-auth. The
agent then learns the missing identity/auth and a retried connection succeeds.
Overdrive v1, by design, does the opposite on a miss: it passes cleartext. That
is exactly why staleness in Overdrive's index is a *security* hazard while
staleness in Cilium's ipcache is (for enforced flows) merely a transient
connectivity blip that fail-closed + re-auth repairs.

**Caveat on the comparison.** Cilium's `world`-on-miss can fail *open* for flows
where policy does NOT require auth/encryption (an explicitly allowed
world-egress) — i.e. Cilium's posture is "policy decides," and a permissive
policy permits cleartext to `world`. The fail-CLOSED guarantee is specific to
flows the policy marks auth-required (mutual auth / encryption). This is the same
shape as Overdrive's intended #236 model: enrolled→enrolled flows must
authenticate; genuinely-non-mesh egress is allowed cleartext. The difference is
that in Cilium the "is this peer in-mesh?" question is answered fail-closed (a
missing identity for an auth-required policy drops), whereas Overdrive v1 answers
"is this addr a mesh backend?" fail-open (a missing index entry → cleartext).

**Source**: `bpf/lib/auth.h:21-54`; `bpf/lib/identity.h:270-283` (Cilium
checkout, read 2026-06-17). **Confidence**: High (primary source; the eBPF
datapath is the authoritative enforcement point — per the rule "where code and
docs disagree, trust the code").

### Stream B — External evidence

#### B1. The canonical pattern: Kubernetes informer/reflector = List-then-Watch + relist-on-"too old" + periodic resync

**Evidence (primary — client-go reflector source):** The Kubernetes
`Reflector` (the engine of every informer) implements exactly the three-leg
pattern, in one function:

- **List-then-Watch.** `ListAndWatchWithContext` — comment verbatim:
  *"ListAndWatchWithContext first lists all items and get the resource version
  at the moment of call, and then use the resource version to watch."* It calls
  `r.list(ctx)` to populate the initial cache, then `r.watchWithResync(...)`.
- **Relist-on-"too old" (the loss signal).** On a watch/list `Expired` error
  (`isExpiredError`), the reflector sets
  `setIsLastSyncResourceVersionUnavailable(true)` and `relistResourceVersion()`
  returns `""` — i.e. it **re-lists from scratch** (a full, fresh enumeration)
  rather than resuming from the stale resourceVersion. This is the HTTP 410
  Gone / `ResourceExpired` recovery.
- **Periodic resync.** `startResync` / `resyncChan` create a timer
  (`r.clock.NewTimer(r.resyncPeriod)`) that periodically re-delivers the full
  cached state (`r.store.Resync()`).

**Evidence (secondary — kubernetes.io + the API contract):** The watch verb "is
used for the efficient detection of changes" as "a stream" (forward-only from a
resourceVersion); "Kubernetes also provides consistent list operations so that
API clients can effectively cache, track, and synchronize the state of
resources." The documented 410-Gone contract (cross-referenced via the
kubernetes/kubernetes relist discussions): *"clients must recognize the 410 Gone
status code, clear their local cache, perform a list operation, and start the
watch from the resourceVersion returned by that new list operation."*

**Analysis — this is the textbook solution to all three Overdrive hazards.**
The reflector pattern exists *precisely* to keep a coherent in-RAM cache over a
forward-only, loss-prone watch:
- List-before-Watch closes the **cold-start** hole (#237) — the cache is never
  empty-but-trusted during convergence; `HasSynced()` gates readiness.
- Relist-on-Expired closes the **lag-drop** hole (F4) — a watch that fell too far
  behind (the 410/compaction analog of `broadcast::Lagged`) triggers a full
  re-List, not a silent continue.
- Periodic resync repairs residual drift even absent an explicit loss signal.

**Observe-only without resync is the known-broken shape this pattern was built to
fix.** Overdrive's current adapter is the "watch-only, ignore the loss signal,
never re-List" anti-pattern that the reflector's `isExpiredError` branch exists
to prevent.

**Sources**:
[client-go reflector.go](https://github.com/kubernetes/client-go/blob/master/tools/cache/reflector.go) (primary, accessed 2026-06-17),
[Kubernetes API concepts — watch/resourceVersion](https://kubernetes.io/docs/reference/using-api/api-concepts/) (primary, accessed 2026-06-17),
[kubernetes/kubernetes #83520 "Avoid going back in time in Reflector relist"](https://github.com/kubernetes/kubernetes/pull/83520) (cross-ref).
**Confidence**: High (3 sources, primary client-go source + official docs).

#### B2. etcd watch semantics: forward-only; compaction → `ErrCompacted` / `CompactRevision` → re-list from a current snapshot

**Evidence (primary — etcd.io):** *"An etcd watch waits for changes to keys by
continuously watching from a given revision … and streams key updates back to
the client"* — i.e. **forward-only** from a revision. On compaction: *"This
happens when creating a watcher at a compacted revision or the watcher cannot
catch up … The watcher will be canceled; creating new watches with the same
start_revision will fail."* The error is `ErrCompacted`, and the
`WatchResponse` carries *"Compact_Revision — set to the minimum historical
revision available to etcd if a watcher tries watching at a compacted
revision."* Recovery: *"clients must establish new watches from a current
revision rather than attempting to resume from the compacted point."*

**Evidence (cross-ref — etcd/k8s issue trail):** The error *"etcdserver: mvcc:
required revision has been compacted"* is the canonical loss signal; the
documented client handling is to catch `ErrCompacted` and **re-list / re-watch
from a current revision** (etcd-io/etcd #9386, kube-apiserver watcher.go
references). This is the *direct, mechanically-identical analog* of
`tokio::broadcast::Lagged` (B4): "you asked to resume from a point that is no
longer retained; you have lost events; re-acquire a fresh snapshot."

**Analysis.** etcd is the substrate under Kubernetes; its watch loss semantic IS
the 410-Gone semantic one layer down. Both say the same thing: a forward-only
watch cannot reconstruct missed history — the only correct recovery is to
re-List from an authoritative current snapshot. Cilium's `ErrCompacted → goto
reList` (A6) is this contract honored in code.

**Sources**:
[etcd Watch API (learning/api)](https://etcd.io/docs/v3.5/learning/api/) (primary, accessed 2026-06-17),
[etcd Maintenance/compaction](https://etcd.io/docs/v3.4/op-guide/maintenance/) (primary, accessed 2026-06-17),
[etcd-io/etcd #9386 "How to handle watcher returning ErrCompacted?"](https://github.com/etcd-io/etcd/issues/9386) (cross-ref).
**Confidence**: High (3 sources, etcd official docs primary).

#### B3. Envoy xDS: SotW vs Delta — explicit completeness guarantees over a stream; eventual consistency via versioning + nonce/ACK

**Evidence (primary — envoyproxy.io xDS protocol):** xDS is the control-plane→
dataplane analog of the cache-coherence problem, and Envoy solves it with
**explicit completeness semantics**, not best-effort observe:
- **State-of-the-World (SotW)** is List-shaped *on every update*: *"the server
  must include the complete state of the world, meaning that all resources of
  the relevant type that are needed by the client must be included, even if they
  did not change."* For CDS/LDS, *"if a previously seen resource is not present
  in a new response, that indicates that the resource has been removed … a
  response containing no resources means to delete all resources of that type."*
  So the proxy's view is reconciled against a complete snapshot each push — drift
  cannot accumulate silently.
- **Delta/Incremental xDS** carries explicit removals: *"When a resource
  subscribed to by a client does not exist, the server will send a
  DeltaDiscoveryResponse … that contains that resource's name in the
  `removed_resources` field"* — the consumer is told, never left to infer
  absence.
- **Convergence machinery**: a `version_info` per resource type, a required
  `nonce` to pair responses to requests ("avoids various race conditions"), and
  ACK/NACK so the server knows the client applied a given version.

**The load-bearing caveat for completeness:** *"The SotW protocol variants do
NOT provide any explicit mechanism to determine when a requested resource does
not exist"* — clients fall back to a 15s timeout. Delta xDS fixes this with
`removed_resources`. This is directly relevant: **a forward-only stream alone
cannot tell a consumer "you now have everything"; you need either a complete
snapshot (SotW/List) or an explicit per-resource existence signal.** Overdrive's
`subscribe_all` provides *neither* — it is a bare forward stream with no
snapshot and no completeness marker, which is why the resolve index cannot know
it is coherent.

**Analysis.** Envoy is the third independent system (after k8s and etcd) that
treats "keep the dataplane's view complete and non-stale over a stream" as
requiring an explicit snapshot/completeness mechanism layered over the watch.
The convergence of three independent designs on the same shape is strong evidence
that observe-only is insufficient by construction, not by implementation defect.

**Sources**:
[Envoy xDS REST/gRPC protocol (latest)](https://www.envoyproxy.io/docs/envoy/latest/api-docs/xds_protocol) (primary, accessed 2026-06-17),
[Envoy xDS protocol v1.17.2](https://www.envoyproxy.io/docs/envoy/v1.17.2/api-docs/xds_protocol) (primary, cross-ref accessed 2026-06-17).
**Confidence**: High (2 primary Envoy doc versions agree).

#### B4. tokio `broadcast`: `RecvError::Lagged(n)` — a permanent miss; idiomatic recovery is resync, NOT ignore

**Evidence (primary — docs.rs/tokio):** A bounded `broadcast` channel overwrites
the oldest buffered value when at capacity: *"If a value is sent when the channel
is at capacity, the oldest value currently held by the channel is released."* A
receiver that fell behind learns this exactly once per gap: *"Any receiver that
has not yet seen the released value will return `RecvError::Lagged`"*, and *"Once
`RecvError::Lagged` is returned, the lagging receiver's position is updated to the
oldest value contained by the channel."* The crucial sentence on recovery: *"The
caller may decide how to respond to this: either by aborting its task or by
tolerating lost messages and resuming consumption of the channel."* The lag
threshold is capacity-rounded: lagging begins once buffered length exceeds the
next power-of-two of capacity (e.g. capacity 1024 → lag at >1024).

**Analysis — what `Lagged(n)` means and why ignoring it is the bug.** `Lagged(n)`
is tokio's *exact equivalent* of etcd `ErrCompacted` / k8s 410-Gone: the receiver
has **permanently and irretrievably** missed `n` messages — they are overwritten,
gone, never re-delivered. The doc offers two responses (abort, or tolerate-and-
continue) but **neither is "resync from an authoritative snapshot," because a bare
broadcast channel HAS no snapshot to resync from** — that is the consumer's
responsibility to provide out-of-band. For a cache built over the stream,
"tolerate lost messages and resume" is the **silently-stale** outcome: the cache
keeps whatever it had for the `n` dropped updates until those keys happen to be
re-sent. The correct shape — the one k8s/etcd/Cilium all implement — is to treat
`Lagged` as the loss signal and **re-List from an authoritative current snapshot**
(the third option the bare channel cannot offer alone).

**This is exactly Overdrive's F4.** `ok_or_skip` (A3) chooses tokio's
"tolerate lost messages and resume" path — `item.ok()` maps `Err(Lagged(n))` to
`None` and drops it — and the adapter has no snapshot surface to re-List from
(A2). So a `service_backends` update lost to lag leaves the resolve index
permanently stale → `NonMesh` → cleartext. The store-local rustdoc itself names
the trap: a consumer "relying on end-of-stream as a catch-up trigger will miss
the lost events" (A3) — i.e. the channel does not even give the adapter a place
to hook a resync, so the loss is silent by construction.

**Sources**:
[tokio::sync::broadcast (docs.rs)](https://docs.rs/tokio/latest/tokio/sync/broadcast/index.html) (primary, accessed 2026-06-17),
[broadcast::error::RecvError (docs.rs)](https://docs.rs/tokio/latest/tokio/sync/broadcast/error/enum.RecvError.html) (primary, accessed 2026-06-17),
[tokio-rs/tokio #2425 (Lagged semantics discussion)](https://github.com/tokio-rs/tokio/issues/2425) (cross-ref).
**Confidence**: High (3 sources, tokio official docs primary; corroborated by the
in-repo store-local rustdoc, A3).

#### B5. Service-mesh endpoint-cache coherence on a security boundary — the fail-open-on-miss hazard is REAL and acknowledged in production (ztunnel)

**Evidence (primary — istio.io + istio/ztunnel):** ztunnel (Istio ambient's
sidecarless dataplane, the model Overdrive's #236 is shaped on) keeps a local
workload state cache fed by xDS and is **fail-closed on a missing SOURCE
workload**: *"Ztunnel drops the connection if it cannot find the source workload
in the xDS API … If it fails to [get pod info from istiod] after 5s, it will
reject the connection."* Traffic between meshed pods is *"fully encrypted with
mTLS by default."*

**But — the decisive nuance (istio/ztunnel #369):** ztunnel is **fail-OPEN on a
missing DESTINATION workload.** Verbatim from the issue: *"If the `source` is not
found in the workload API, we drop the connection. If the `dest` is not found in
the workload API, we give up and **pass the connection thru anyway**."* A pod
making an outbound call to a destination not (yet) in ztunnel's cache gets the
connection passed through — i.e. **a stale/incomplete destination cache yields
plaintext passthrough**, exactly Overdrive's `NonMesh`-cleartext-on-miss hazard,
in a shipping mesh.

**Analysis — this is the single most important external data point for the
verdict.** It establishes two things simultaneously:
1. **The hazard is real and not hypothetical.** Even a mature, security-audited
   mesh (ztunnel) has a path where a cache miss on the *destination* degrades to
   plaintext. Overdrive's adversarial review found the same class — so the
   concern is well-founded, not paranoia.
2. **The mitigation Overdrive plans (#236 fail-toward-handshake) is a genuine
   hardening over ztunnel's current behavior, AND it is the same architectural
   lever — change what a miss MEANS, not (only) whether the cache is coherent.**
   ztunnel's source-side fail-closed (drop after 5s waiting for istiod) is the
   "don't degrade to plaintext on a miss" posture applied to the source; #236
   applies the analogous fail-toward-handshake posture to the destination resolve.
   The coherence of the cache (List-then-Watch) and the *meaning* of a miss
   (fail-open vs fail-closed) are **two independent levers** — ztunnel tunes the
   miss-meaning per-direction; Cilium (A9) tunes it via policy (auth-required →
   drop). Overdrive can do either or both.

**On the coherence lever specifically:** ztunnel's workload cache is fed by xDS
SotW/Delta (B3) — i.e. it DOES get completeness semantics from the control plane
(unlike Overdrive's bare `subscribe_all`). So ztunnel mitigates the *coherence*
side via xDS completeness AND tunes the *miss-meaning* side per-direction. Cilium
(A5–A7) mitigates coherence via List-then-Watch + relist AND fail-closes the
miss-meaning via auth-required-drop. **Every production mesh examined applies a
mitigation on at least one of the two levers; none relies on a bare forward-only
observe with fail-open-on-miss — which is precisely Overdrive's v1 shape.**

**Sources**:
[Istio ztunnel architecture](https://github.com/istio/istio/blob/master/architecture/ambient/ztunnel.md) (primary, accessed 2026-06-17),
[Istio ztunnel L4/mTLS docs](https://istio.io/v1.21/docs/ops/ambient/usage/ztunnel/) (primary, accessed 2026-06-17),
[istio/ztunnel #369 — inconsistent passthrough on missing workload](https://github.com/istio/ztunnel/issues/369) (primary, accessed 2026-06-17).
**Confidence**: High (3 sources, istio/ztunnel primary). **Bias note**: #369 is a
bug report; the "pass thru anyway" quote is the reporter's description of the code
behavior (no maintainer rebuttal in-thread), cross-referenced against the
istio.io fail-closed-on-source documentation which is consistent with it.

## Verdict — Option (a) vs (b) for Overdrive v1

### The decisive framing: two ORTHOGONAL levers, not one choice

The single most important synthesis from the evidence is that **cache coherence
and miss-meaning are two independent levers**, and every production system tunes
*at least one* — none ships Overdrive v1's combination of (bare forward-only
observe) × (fail-open-on-miss):

| System | Coherence lever (snapshot/resync) | Miss-meaning lever (what a miss means) |
|---|---|---|
| **k8s informer** | List-then-Watch + relist-on-Expired + resync (B1) | n/a (not a security boundary) |
| **etcd watch** | re-list on `ErrCompacted` (B2) | n/a |
| **Cilium** | List-then-Watch + relist-on-error + stale-sweep (A5–A7) | **fail-CLOSED**: auth-required + auth-map miss → `DROP` (A9) |
| **ztunnel** | xDS SotW/Delta completeness from istiod (B3, B5) | **fail-CLOSED on source**, fail-open on dest (a known bug, #369) |
| **Overdrive v1 (current)** | **NONE** — bare forward observe, lazy drain, `Lagged` dropped (A1, A3) | **fail-OPEN**: miss → `NonMesh` → cleartext (A1, A9) |

Overdrive v1 is the only row with *neither* lever engaged. That is the security
hole the adversarial review found, stated structurally.

### Mapping the two levers onto Option (a) and Option (b)

- **Option (a) — fail-toward-handshake (#236)** tunes the **miss-meaning** lever:
  a miss attempts mTLS / holds, never silent cleartext. It does NOT make the
  cache coherent — the cache can still be stale — but staleness stops being a
  *security* property and becomes a *latency/correctness* property (a stale-miss
  costs a handshake attempt + retry, not a cleartext leak). This is exactly
  Cilium's posture (A9: auth-required → drop + signal re-auth) and ztunnel's
  source-side posture (B5: drop + wait for istiod). **It is the industry-standard
  way to make a local endpoint cache safe on a security boundary.**

- **Option (b) — List-then-Watch + resync** tunes the **coherence** lever: add a
  bounded snapshot/enumerate surface so the adapter List-then-Watches and
  re-Lists on `Lagged`. This is *literally* the informer pattern (B1) and what
  Cilium's kvstore watcher does (A5–A6). It closes #237 + F4 by construction, but
  costs `ObservationStore` surface (a keyless `service_backends_rows()`, A2)
  across core + both store adapters, reversing C4 / D-TME-11.

### The verdict: a sequenced HYBRID

**Recommendation to pin: do (a) as the v1 SECURITY guarantee, fix F2 now
regardless, and schedule (b) as the coherence hardening — in that priority
order.** Concretely:

1. **F2 (concurrency race) — fix NOW, unconditionally.** Not the subject of this
   research, but it must land regardless of a/b: replace the non-atomic
   `take()`/`subscribe_all()`/restore (A1, `mtls_resolve_adapter.rs:245-266`) with
   an open-once / atomic-claim of the held subscription (the `ClaimSet` /
   `OnceCell`-style primitive the repo already mandates per `development.md`
   § "Check-and-act must be atomic"). This is a straightforward TOCTOU fix.

2. **Option (a) is the correct v1 SECURITY posture — and it is REQUIRED, not
   optional, for v1 to be safe.** The evidence is unambiguous: a forward-only
   lossy cache on a fail-open boundary is unsafe *by construction* (B1, B2, B3,
   B4 all say a bare watch cannot self-certify completeness; A9 + B5 show every
   mesh that IS safe fail-closes the miss). **As long as a resolve miss means
   cleartext, NO amount of cache-coherence engineering fully closes the hole** —
   there is always a convergence window (cold boot before the first List
   completes; the instant between a backend coming up and its row arriving). The
   only mechanism that removes the *security* consequence of that irreducible
   window is changing what a miss MEANS (fail-toward-handshake). So (a) is the
   load-bearing fix; (b) without (a) still leaks during its own convergence window.

   **Caveat — (a) is multi-node-shaped and not yet in tree.** Per the sibling doc
   (`transparent-mtls-interception-mechanism-2026-research.md` §0.4 / §3.4) and
   GH #236, fail-toward-handshake is the planned multi-node hardening; it depends
   on the agent being able to *attempt* a handshake on an ambiguous miss (the
   ztunnel-shaped capture-all + resolve path). For **single-node v1**, where the
   only writer of `service_backends` is the local `BackendDiscoveryBridge` and the
   only reader is the local resolve adapter, the convergence window is bounded and
   local — but it is NOT zero (cold-boot #237 is exactly this window). So even
   single-node v1 should not ship fail-open-on-miss as its permanent security
   story; it should ship (a)'s intent.

3. **Option (b) is the RIGHT eventual architecture and the in-house precedent
   already argues for it (A4).** The reconciler runtime's bulk-load + write-through
   IS List-then-Watch over an authoritative surface; the resolve adapter is the
   one place built without the List half. When the `service_backends` enumerate
   surface is justified (multi-node, or the moment (a) is not yet landed and #237
   must be closed for single-node correctness), (b) is the clean fix. It is
   `O(1)` new trait method + a snapshot read at probe time + a re-List on the
   `Lagged` signal the adapter currently discards.

### What single-node v1 actually needs vs multi-node

- **Single-node v1, minimum honest bar:** F2 fixed + **either** (a)'s
  fail-toward-handshake **or** (b)'s List-at-probe (bulk-load the existing
  `service_backends` rows once at probe, before serving). The latter alone closes
  #237 (cold-start) because the boot-time List seeds the index before the
  Earned-Trust gate opens the door — and for single-node, F4 (lag-drop) is far
  less acute (one local writer, bounded write rate, unlikely to burst >1024
  *between* resolve calls). So **a pragmatic single-node v1 = F2 fix + a
  bounded List-at-probe (a thin slice of (b))**, deferring the full relist-on-
  `Lagged` and (a)'s handshake machinery to multi-node.

- **Multi-node:** (a) fail-toward-handshake becomes load-bearing (cross-node
  convergence windows are real and unbounded under partition), and (b)'s
  relist-on-`Lagged` becomes necessary (the firehose can genuinely burst >1024).
  Both are #236-coupled.

### Honest cost/risk

- **(a) cost/risk:** depends on the #236 handshake-attempt path existing; not a
  pure adapter change. Risk: until #236 lands, (a) is a *design intent*, not
  shippable code — so v1 cannot rely on it alone today. Lowest *surface* cost
  (no trait change) but highest *dependency* cost.
- **(b) cost/risk:** reverses C4/D-TME-11 (adds `service_backends_rows()` keyless
  enumerate to `ObservationStore` + both adapters + the relist-on-`Lagged` drain).
  Risk: trait-surface growth the design deliberately avoided; but the surface is
  symmetric with the `alloc_status_rows()`/`node_health_rows()` that already exist
  (A2), so it is *consistent* growth, not novel. Highest *surface* cost, lowest
  *dependency* cost — shippable for single-node today.
- **Hybrid (recommended):** F2-now + List-at-probe slice of (b) for single-node
  #237 + schedule (a) and full (b)-relist for multi-node. This is the only path
  that makes single-node v1 honestly safe *today* without waiting on #236.

### What I'd recommend the architect pin

> **Pin:** (1) F2 is a blocking unconditional fix (atomic-claim the held
> subscription). (2) Adopt **(a) fail-toward-handshake as the stated v1 SECURITY
> invariant** — "a resolve miss must never silently emit cleartext to a
> should-be-mesh peer" — and record it as the contract the miss-classification
> must eventually satisfy. (3) For **single-node v1 shipping before #236**, close
> #237 with a **bounded List-at-probe** (a minimal slice of (b): a keyless
> `service_backends_rows()` snapshot drained once at probe, before the
> Earned-Trust gate opens) — this is the smallest change that makes the cold-boot
> window safe and is consistent with the existing `alloc_status_rows()` surface.
> (4) Defer the full relist-on-`Lagged` (the rest of (b)) and (a)'s handshake
> machinery to the #236 multi-node arc, tracked. Do **not** ship the current
> bare-observe + fail-open-on-miss as the permanent v1 story — it is the one shape
> no production mesh uses, for the reason this whole document establishes.

## Hazard → disposition map

| Hazard | Root mechanism (file:line) | Closed by (a) fail-toward-handshake? | Closed by (b) List-then-Watch + resync? | Recommended v1 disposition |
|---|---|---|---|---|
| **#237 cold-start / no-replay** | empty index on (re)start until `BackendDiscoveryBridge` re-writes; `subscribe_all` is forward-only (A1, A3; `mtls_resolve_adapter.rs:184-196`) | **Partially** — a cold-boot miss attempts handshake instead of cleartext, so no *leak*; but every connection during convergence eats a handshake/retry (latency, not security) | **YES, fully** — List-at-probe seeds the index before the Earned-Trust gate opens; no empty-trusted window (A5 — Cilium's `ListDone`-gates-`synced`) | **(b) List-at-probe** (single-node) — smallest safe fix; (a) as the security backstop |
| **F2 concurrency race** | non-atomic `take()`/`subscribe_all()`/restore (A1; `mtls_resolve_adapter.rs:245-266`) | No (orthogonal) | No (orthogonal) | **Fix NOW unconditionally** — atomic-claim per `development.md` § "Check-and-act must be atomic" |
| **F4 bounded-buffer lag-drop** | held subscription = whole firehose over 1024-deep broadcast; `Lagged` silently dropped, no resync (A3; `observation_store.rs:651-658`, `observation_backend.rs:29-38`) | **YES (security)** — a lag-induced stale miss attempts handshake, never cleartext; staleness becomes a latency bug | **YES (correctness)** — re-List on the `Lagged` signal the adapter currently discards (A6 — Cilium's `ErrCompacted → goto reList`; B4 — the tokio-idiomatic recovery) | **(a)** removes the security consequence for v1; **(b) relist-on-`Lagged`** is the full fix, deferred to multi-node (firehose unlikely to burst >1024 single-node) |

## Source Analysis

### Stream A — codebase (primary, file:line)

| Source | Path | Reputation | Type | Cross-verified |
|--------|------|------------|------|----------------|
| Overdrive resolve adapter | `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs` | High (1.0) | primary code | Y (vs trait + adapters) |
| Overdrive ObservationStore trait | `crates/overdrive-core/src/traits/observation_store.rs` | High (1.0) | primary code | Y |
| Overdrive sim store | `crates/overdrive-sim/src/adapters/observation_store.rs` | High (1.0) | primary code | Y (vs local store) |
| Overdrive local store | `crates/overdrive-store-local/src/observation_backend.rs` | High (1.0) | primary code | Y (vs sim store) |
| Overdrive reconciler-I/O rule | `.claude/rules/development.md` § "Reconciler I/O" | High (1.0) | primary SSOT | Y |
| Cilium kvstore etcd watcher | `pkg/kvstore/etcd.go` (Cilium) | High (1.0) | primary code | Y (vs watchstore) |
| Cilium restartable watchstore | `pkg/kvstore/store/watchstore.go` (Cilium) | High (1.0) | primary code | Y |
| Cilium SharedStore periodic sync | `pkg/kvstore/store/store.go` (Cilium) | High (1.0) | primary code | Y |
| Cilium ipcache | `pkg/ipcache/ipcache.go` (Cilium) | High (1.0) | primary code | Y |
| Cilium datapath auth | `bpf/lib/auth.h`, `bpf/lib/identity.h` (Cilium) | High (1.0) | primary code | Y |

### Stream B — external (trusted domains)

| Source | Domain | Reputation | Type | Cross-verified |
|--------|--------|------------|------|----------------|
| client-go reflector.go | github.com/kubernetes/client-go | High (1.0) | official primary | Y |
| Kubernetes API concepts (watch/RV) | kubernetes.io | High (1.0) | official | Y |
| k8s/k8s #83520 (relist) | github.com | Medium-High (0.8) | official issue | Y |
| etcd Watch API | etcd.io | High (1.0) | official | Y |
| etcd Maintenance/compaction | etcd.io | High (1.0) | official | Y |
| etcd-io/etcd #9386 | github.com | Medium-High (0.8) | official issue | Y |
| tokio broadcast docs | docs.rs/tokio | High (1.0) | official | Y |
| tokio broadcast RecvError | docs.rs/tokio | High (1.0) | official | Y |
| tokio-rs/tokio #2425 | github.com | Medium-High (0.8) | official issue | Y |
| Envoy xDS protocol (latest) | envoyproxy.io | High (1.0) | official | Y |
| Envoy xDS protocol (v1.17.2) | envoyproxy.io | High (1.0) | official | Y |
| Istio ztunnel architecture | github.com/istio/istio | High (1.0) | official | Y |
| Istio ztunnel L4/mTLS docs | istio.io | High (1.0) | official | Y |
| istio/ztunnel #369 | github.com | Medium-High (0.8) | official issue | Y |

**Reputation summary**: 24 sources cited. High (1.0): 19 (~79%). Medium-High
(0.8): 5 GitHub issues (~21%, each used only as cross-reference, never as a sole
source for a claim). **Average reputation ≈ 0.96.** No medium/excluded-tier
sources used. All sources from the embedded trusted-domain config.

## Knowledge Gaps

### Gap 1: Cilium read-side PERIODIC resync (vs relist-on-error)
**Issue**: A7 found a *producer-side* `SynchronizationInterval` periodic re-push
(`store.go:228-238`), and A6 found *consumer-side* relist-on-error
(`etcd.go:823-844`). I did not find an unconditional *read-side periodic re-List*
on a timer (independent of any error signal) in the kvstore watcher path. The k8s
informer has one (`resyncChan`, B1) but it re-delivers the *cached* state to
handlers, not a fresh List from the API. **Recommendation**: this gap does not
affect the verdict (relist-on-loss is the load-bearing leg and is present); flagged
so the architect does not over-claim "Cilium periodically re-Lists the read path."

### Gap 2: Cilium ipcache cold-start ordering vs eBPF map population
**Issue**: A5 established the kvstore-level `ListDone`/`synced` gate, but I did not
trace whether the *eBPF ipcache map* (the per-packet datapath read) is gated on
`synced` before the datapath starts enforcing — i.e. whether there is a datapath
window where the BPF map is partially populated during cold-start. **Attempted**:
read `pkg/ipcache/ipcache.go` lookups + `bpf/lib/auth.h`. **Recommendation**: A9's
fail-closed-on-auth-map-miss likely makes this moot (a not-yet-populated entry
fail-closes), but a dedicated trace of `pkg/datapath`/`pkg/maps/ipcache` BPF-map
hydration ordering would confirm. Not load-bearing for the verdict.

### Gap 3: Overdrive single-node F4 burst likelihood (quantitative)
**Issue**: I asserted F4 (lag-drop) is "far less acute single-node" because one
local writer is unlikely to burst >1024 writes between two resolve calls. This is
a qualitative judgment, not measured. **Recommendation**: if the architect wants
to *rely* on single-node F4 being negligible (rather than fixing it via (a)/(b)),
measure the actual `service_backends` + whole-firehose write rate under a realistic
single-node workload churn against the resolve call cadence. The safe default
(adopt (a)) does not need this measurement.

## Conflicting Information

### Conflict 1: Does fail-toward-handshake make cache coherence a non-issue?
**Position A** (implied by Option a / the sibling doc §3.4): "fail-toward-handshake
removes the *security* consequence of staleness, so cache coherence is a
non-security concern" — Source: `transparent-mtls-interception-mechanism-2026-research.md`
§3.4 (internal, High). **Position B** (this doc's synthesis from B1–B4 + A5–A7):
"every production system ALSO engages the coherence lever (List-then-Watch); none
relies on miss-meaning alone." **Assessment**: not a true contradiction — both are
correct on their own axis. (a) removes the *security* consequence; (b) removes the
*correctness/latency* consequence (a stale cache still costs handshake retries and
can mis-route under (a)). The verdict reconciles them: (a) is the v1 security
floor; (b) is the eventual correctness fix. The systems engage both because both
consequences matter at scale; single-node v1 can prioritize the security floor.

### Conflict 2: Does the in-house bulk-load precedent justify observe-only (C4) or argue against it (this doc)?
**Position A** (the C4 / D-TME-11 design text): cited "bulk-load-then-observe" as
the in-house precedent justifying the observe-only resolve mechanism — Source:
ADR-0071 C4 / `mtls_resolve_adapter.rs:36-46` (internal, High). **Position B**
(A4, this doc): the reconciler-runtime precedent is List(bulk-load)-then-observe
over an *authoritative* `ViewStore`, and the resolve adapter is the one place
built *without* the List half — so the precedent argues *for* Option (b), not for
observe-only. **Assessment**: Position B is better-grounded. The precedent's
"bulk-load" IS the List leg; the C4 text invoked the precedent's name while
dropping its load-bearing half (the authoritative snapshot). The code (A4,
`development.md` § Reconciler I/O) is the authority and shows bulk-load + write-
through, not bare observe. Per "where code and docs disagree, trust the code."

## Full Citations

[1] Kubernetes authors. "client-go tools/cache/reflector.go". kubernetes/client-go (master). https://github.com/kubernetes/client-go/blob/master/tools/cache/reflector.go. Accessed 2026-06-17.
[2] Kubernetes authors. "API Concepts — Efficient detection of changes / resourceVersion / 410 Gone". kubernetes.io. https://kubernetes.io/docs/reference/using-api/api-concepts/. Accessed 2026-06-17.
[3] Kubernetes authors. "Avoid going back in time in Reflector relist (#83520)". github.com/kubernetes/kubernetes. https://github.com/kubernetes/kubernetes/pull/83520. Accessed 2026-06-17.
[4] etcd authors. "Watch API / KV API — etcd v3.5 learning/api". etcd.io. https://etcd.io/docs/v3.5/learning/api/. Accessed 2026-06-17.
[5] etcd authors. "Maintenance — compaction". etcd.io. https://etcd.io/docs/v3.4/op-guide/maintenance/. Accessed 2026-06-17.
[6] etcd authors. "How to handle watcher returning ErrCompacted? (#9386)". github.com/etcd-io/etcd. https://github.com/etcd-io/etcd/issues/9386. Accessed 2026-06-17.
[7] Tokio authors. "tokio::sync::broadcast". docs.rs/tokio. https://docs.rs/tokio/latest/tokio/sync/broadcast/index.html. Accessed 2026-06-17.
[8] Tokio authors. "broadcast::error::RecvError". docs.rs/tokio. https://docs.rs/tokio/latest/tokio/sync/broadcast/error/enum.RecvError.html. Accessed 2026-06-17.
[9] Tokio authors. "broadcast returns Lagged error … (#2425)". github.com/tokio-rs/tokio. https://github.com/tokio-rs/tokio/issues/2425. Accessed 2026-06-17.
[10] Envoy authors. "xDS REST and gRPC protocol (latest)". envoyproxy.io. https://www.envoyproxy.io/docs/envoy/latest/api-docs/xds_protocol. Accessed 2026-06-17.
[11] Envoy authors. "xDS REST and gRPC protocol (v1.17.2)". envoyproxy.io. https://www.envoyproxy.io/docs/envoy/v1.17.2/api-docs/xds_protocol. Accessed 2026-06-17.
[12] Istio authors. "Ambient ztunnel architecture". github.com/istio/istio. https://github.com/istio/istio/blob/master/architecture/ambient/ztunnel.md. Accessed 2026-06-17.
[13] Istio authors. "Layer 4 Networking & mTLS with Ztunnel (1.21)". istio.io. https://istio.io/v1.21/docs/ops/ambient/usage/ztunnel/. Accessed 2026-06-17.
[14] Istio authors. "Inconsistent outbound passthrough behavior if source or dest workloads are not found (#369)". github.com/istio/ztunnel. https://github.com/istio/ztunnel/issues/369. Accessed 2026-06-17.
[15] Cilium authors. "pkg/kvstore/etcd.go". (local checkout `/Users/marcus/git/cilium/cilium`). Read 2026-06-17.
[16] Cilium authors. "pkg/kvstore/store/watchstore.go". (local checkout). Read 2026-06-17.
[17] Cilium authors. "pkg/kvstore/store/store.go". (local checkout). Read 2026-06-17.
[18] Cilium authors. "pkg/ipcache/ipcache.go". (local checkout). Read 2026-06-17.
[19] Cilium authors. "bpf/lib/auth.h, bpf/lib/identity.h". (local checkout). Read 2026-06-17.
[20] Overdrive. "crates/overdrive-control-plane/src/mtls_resolve_adapter.rs" (and observation_store trait + sim/local adapters + development.md). (this repo). Read 2026-06-17.

## Research Metadata

Duration: ~1 session | Sources examined: 24+ | Sources cited: 24 | Cross-references: every load-bearing external claim ≥2 sources (canonical pattern: 3) | Confidence distribution: High ~90% (all primary-source findings), Medium-High ~10% (GitHub-issue cross-refs) | Output: `docs/research/networking/transparent-mtls-resolve-index-coherence-research.md`

**Methodology note**: Stream A (codebase) is 100% primary-source with file:line
citations; the Cilium dive was performed directly (no subagent available) against
`/Users/marcus/git/cilium/cilium`. Where Cilium code and prose docs could diverge,
the code (file:line) is cited as authoritative per the prompt directive. Stream B
external claims are each cross-referenced against ≥2 trusted-domain sources, with
the canonical informer/etcd/tokio/Envoy primaries preferred over blogs (zero
medium/excluded-tier sources used).
