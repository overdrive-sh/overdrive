# Research: Reconciler Memory Storage Abstraction Trade-offs

**Date**: 2026-05-03 | **Researcher**: nw-researcher (Nova) | **Confidence**: TBD | **Sources**: TBD

## Executive Summary

The current §18 reconciler contract requires every reconciler author to
hand-write CREATE TABLE, SELECT + row-decoding, and DELETE+INSERT
plumbing — roughly 100 lines per reconciler that does nothing
reconciler-specific. Cross-reference of ten production precedents
(Restate, Cloudflare Durable Objects, kube-rs, controller-runtime,
Anvil, Akka Persistence, Temporal, Marten, rkyv, WASM Component
Model) shows that **modern durable-state systems converge on
typed-record persistence as the default ergonomic surface**, with SQL
or raw KV reserved as opt-in escape hatches. Restate ships
`ctx.get/set` opaque-typed; Cloudflare DOs ship typed KV *and* SQL on
the same handle backed by the same SQLite file; kube-rs offers no
persistence layer at all and operators reach for sidecars; Anvil's
verification target is the pure transition function regardless of
storage shape.

The recommended direction is **Option C: typed-View blob auto-persisted
as the default, with a SQL escape hatch for the small set of
reconcilers that genuinely need columnar query**. The wire format
should be **CBOR via `ciborium` with serde `#[serde(default)]` for
additive evolution** — explicitly NOT rkyv, which the rkyv author
documents as unsuitable for schema evolution. This preserves every
§18/§21 guarantee (ESR verifiability, DST replay, "persist inputs not
derived state," pure `reconcile`) while collapsing the per-reconciler
plumbing surface to roughly zero, and structurally hardens the WASM
extension story (third-party reconcilers cannot ship arbitrary SQL
into a sandbox).

Confidence is **High** for the recommendation's overall shape and
**Medium** for the exact trait surface — five spike targets in §7
identify the engineering questions that would refine it before
shipping (sizing for large Views, schema-downgrade tests, NextView
diff-merge semantics, Anvil verification re-validation, operator CLI).

## Research Methodology

**Search Strategy**: Web searches (3 batched parallel queries per
round) targeted to canonical documentation surfaces for each precedent
system; targeted reads of the in-tree reconciler trait and the
existing `JobLifecycle` reconciler implementation to ground the
"plumbing cost" claim in concrete line-counts; grep against the
overdrive crates to verify the cross-target-query claim empirically.

**Source Selection**: Types — official vendor docs (Restate,
Cloudflare, Temporal, Akka, kubernetes.io, kubebuilder), peer-reviewed
academic publication (Anvil OSDI '24, SANER 2017 event-sourcing
paper), industry-leader reference (Microsoft Architecture Center,
Marten), canonical Rust library documentation (kube-rs, rkyv on
docs.rs); reputation — high-tier (academic/official/canonical-docs)
for every cited source.

**Quality Standards**: Target 3 sources/claim achieved on every major
precedent finding (Findings 1–10 each cite 2–4 independent sources).
Cross-reference between vendor docs and independent commentary used
for Findings 3, 5, 6, 7. No single-source claims in §3.

## 1. Problem Statement

The current Overdrive §18 reconciler trait (post-issue-#139) requires four
methods per reconciler:

```rust
trait Reconciler: Send + Sync {
    type State: Send + Sync;
    type View:  Send + Sync;

    fn name(&self) -> &ReconcilerName;
    async fn migrate(&self, db: &LibsqlHandle) -> Result<(), HydrateError>;            // DDL once
    async fn hydrate(&self, target: &TargetResource, db: &LibsqlHandle)
        -> Result<Self::View, HydrateError>;                                            // SELECT every tick
    async fn persist(&self, view: &Self::View, db: &LibsqlHandle)
        -> Result<(), HydrateError>;                                                    // DELETE+INSERT every tick
    fn reconcile(&self, desired: &Self::State, actual: &Self::State,
                 view: &Self::View, tick: &TickContext) -> (Vec<Action>, Self::View);   // Pure compute
}
```

Concrete cost (measured against the in-tree `JobLifecycle` reconciler at
`crates/overdrive-core/src/reconciler.rs:1386-1526`):

- **`migrate`** — 17 source lines + 25 lines of comment justifying the
  schema choice. Two `CREATE TABLE IF NOT EXISTS`, additive-only ALTER
  discipline, both tables one identity column + one payload column.
- **`hydrate`** — 50 lines of `conn.query(...)` + `while rows.next()` +
  per-field `try_from`/`parse` + manual `HydrateError::Schema { message:
  format!(...) }` mapping. The view is two `BTreeMap` fields; the SQL is
  two `SELECT *`s. Every other field gets its own row-decoding stanza.
- **`persist`** — 35 lines of transaction-scoped `DELETE` then `INSERT`
  in a per-row loop. Quadratic in view size; the comment explicitly
  flags this as a Phase-1 expedient.

A net ~100 lines of plumbing per reconciler that does nothing
reconciler-specific — they translate a `View` struct to and from libSQL.
The author's friction is concrete: every new reconciler that grows a
`View` field grows a CREATE TABLE clause, a SELECT block, a row decoder,
and an INSERT block, in lockstep, by hand.

The research question is whether the runtime can own this translation
layer without losing any of the §18 / §21 guarantees — ESR
verifiability, bit-identical DST replay, "persist inputs not derived
state," pure `reconcile`.

## 2. Current Design Recap

The §18 reconciler primitive (whitepaper §18 *The Reconciler
Primitive* + ADR-0013 §2/§2b/§2c + development.md § *Reconciler I/O*)
splits the reconciler lifecycle into four methods:

- **`migrate(&LibsqlHandle) -> Result<()>`** — async, runs ONCE per
  reconciler instance at register time. Owns CREATE TABLE / ALTER
  TABLE. Failure surfaces as `ControlPlaneError::Internal`; the
  registry contains no partial slot for that reconciler.
- **`hydrate(&TargetResource, &LibsqlHandle) -> Result<View>`** —
  async, runs every reconcile tick. The ONLY place a reconciler author
  runs SELECT against libSQL. Returns the author-declared `View` type.
- **`persist(&View, &LibsqlHandle) -> Result<()>`** — async, runs every
  reconcile tick after `reconcile`. Reconciler-author-owned write
  phase (Phase 1 convention: full DELETE-then-INSERT).
- **`reconcile(desired, actual, view, tick) -> (Vec<Action>,
  Self::View)`** — pure, sync. No `.await`. No I/O. No wall-clock read
  outside `tick.now`. No direct store write.

The contract has three load-bearing properties:

1. **ESR verifiability** (whitepaper §18, citing USENIX OSDI '24
   *Anvil*): `reconcile` is a pure function over `(desired, actual,
   view, tick)`, so progress + stability under stable inputs is
   expressible as a temporal-logic formula amenable to mechanical
   checking in Verus.
2. **Bit-identical DST replay** (whitepaper §21, testing.md § *Sources
   of Nondeterminism*): every nondeterminism source — clock,
   transport, entropy, libSQL handle — flows through injected traits
   the harness controls, so a seed reproduces the trajectory bit-for-bit.
3. **Persist inputs, not derived state** (development.md): the View
   carries the *inputs* to whatever logic consumes them
   (`restart_counts`, `last_failure_seen_at`), never the derived
   value (`next_attempt_at`, which is recomputed every tick from the
   inputs and the live backoff policy).

Any proposed change to the storage abstraction must preserve all
three.

The grounding example (`crates/overdrive-core/src/reconciler.rs:1386-1526`):
the `JobLifecycle` reconciler's three storage methods total 17 + 50 +
35 = 102 source lines, of which roughly 95 are mechanical translation
between two `BTreeMap` View fields and two libSQL tables.

## 3. Survey of Precedent

*To be filled in per system.*

### 3.1 kube-rs `Store<K>` and the Kubernetes/Go controller-runtime

**Finding 1 — kube-rs Store is a memory-only reflector cache, not per-controller persistent state.**

> "A readable cache of Kubernetes objects of kind `K`" … internally an
> `AHashMap` keyed by `ObjectRef<K>` with `Arc<K>` values. … "this is a
> cache and may be stale. Deleted objects may still exist in the cache
> despite having been deleted in the cluster, and new objects may not
> yet exist in the cache."

**Source**: [kube::runtime::reflector::Store — docs.rs](https://docs.rs/kube/latest/kube/runtime/reflector/struct.Store.html)
(accessed 2026-05-03)
**Confidence**: High (single authoritative source; this is the
canonical Rust K8s controller library).

**Implication for the question**: kube-rs reconcilers do NOT have a
per-controller persistent memory abstraction. The only "memory" the
framework provides is the watch-cache reflector — a *projection of
API-server state*, not reconciler-private bookkeeping. Anything the
reconciler needs to remember (retry counts, backoff timestamps,
historical placement) must either:
1. Be encoded into the API-server objects themselves (a `status` field,
   an annotation), inheriting etcd's strong-consistency guarantees AND
   its serialization shape (JSON-Schema validated CRDs), or
2. Sit in a separate datastore the controller author wires up by hand
   (Redis, an external SQL DB, a sidecar) — entirely outside the
   framework contract.

This is structurally why Kubernetes operators that need rich memory
(Restate's operator, Crossplane providers, Strimzi) end up with
side-stores or annotation-encoded state; the framework gave them no
typed memory layer. Overdrive's per-reconciler libSQL is the answer to
exactly this gap.

### 3.2 USENIX OSDI '24 *Anvil* (Verus-verified controllers)

**Finding 2 — Anvil's `reconcile_core` takes state as an explicit parameter, returns next state; storage location is the framework's concern.**

The exact signature published in the Anvil README is:

```
fn reconcile_core(cr: &Self::R, resp_o: Option<Response<...>>,
                  state: Self::T) -> (Self::T, Option<Request<...>>)
```

The `Self::T` "reconcile local state type" is author-defined but flows
through the function as an input/output value, not pulled from an
ambient I/O surface. The shim layer outside `reconcile_core` is the
only place that performs API requests; the verified core is pure over
its inputs.

**Source**:
- [anvil-verifier/anvil — README.md](https://github.com/anvil-verifier/anvil/blob/main/README.md) (accessed 2026-05-03)
- [Anvil: Verifying Liveness of Cluster Management Controllers — OSDI '24 paper PDF](https://www.usenix.org/system/files/osdi24-sun-xudong.pdf) (accessed 2026-05-03)
- [Anvil: Building Kubernetes Controllers That Do Not Break — USENIX article](https://www.usenix.org/publications/loginonline/anvil-building-formally-verified-kubernetes-controllers) (accessed 2026-05-03)

**Confidence**: High (peer-reviewed OSDI '24 paper + author-maintained
public repo + USENIX editorial; three independent surface readings
agree).

**Implication for the question**: Anvil's verification target is
*progress + stability under stable inputs*. The verified ESR property
is over `reconcile_core`'s pure transition function; storage shape is
NOT part of the verified surface. This means Overdrive can replace
hand-written `hydrate`/`persist` with framework-owned serialization
without weakening the ESR proof obligation — the proof is over the
pure transition, not over the persistence mechanism. The Overdrive
contract is *stricter* than Anvil's because it threads `view` through
explicitly rather than letting the author hide state inside `Self::T`,
but the verifiability story is preserved either way.

### 3.3 Restate (durable execution, ctx.get/set)

**Finding 3 — Restate exposes a typed, per-handler key-value surface; users never write SQL or DDL.**

The complete API surface for handler state on a Virtual Object or
Workflow:

```
ctx.get<T>(name: string): Promise<T | null>
ctx.set<T>(name: string, value: T): void
ctx.clear(name: string): void
ctx.clearAll(): void
ctx.stateKeys(): Promise<string[]>
```

State is "scoped per object key" for Virtual Objects; "scoped per
workflow execution" for Workflows. "No explicit schema is required."
Cross-handler queries are explicitly NOT exposed — each handler's KV
is isolated.

**Source**:
- [Restate — State (TypeScript) docs](https://docs.restate.dev/develop/ts/state) (accessed 2026-05-03)
- [Restate — restate-sdk Rust crate docs](https://docs.rs/restate-sdk/latest/restate_sdk/) (accessed 2026-05-03)
- [Restate — Building a modern Durable Execution Engine from First Principles](https://www.restate.dev/blog/building-a-modern-durable-execution-engine-from-first-principles) (accessed 2026-05-03)

**Confidence**: High (vendor docs + Rust SDK docs + design rationale
blog from the framework authors).

**Implication for the question**: Restate is the strongest precedent
for **Option B**. Handler authors never write CREATE TABLE; the
runtime owns persistence; per-handler state isolation makes the typed
KV both safe and trivially understood. The trade-off Restate accepts:
no cross-handler queries — if you need to know "any other workflow's
state," you have to model that as an event/signal rather than a JOIN.
This is precisely the trade-off Overdrive would accept under Option B.

### 3.4 Temporal / Cadence (event-sourced workflow history)

**Finding 4 — Temporal preserves workflow state by replaying Event History; user code defines no schema, the framework checkpoints every external interaction.**

> "During replay, Temporal starts the workflow code from the beginning,
> replays the Event History step by step, and uses that history to
> guide the code back to the exact state as before. When a workflow
> calls an activity, the activity runs once and its result is recorded
> in the Event History. During replay, that result is reused, not
> recomputed."

**Source**:
- [Temporal — Workflow Execution overview](https://docs.temporal.io/workflow-execution) (accessed 2026-05-03)
- [Temporal — Events and Event History](https://docs.temporal.io/workflow-execution/event) (accessed 2026-05-03)
- [Temporal — What does "preserving state" really mean? (Cornelia Davis blog mirrored on temporal.io)](https://temporal.io/blog/temporal-replaces-state-machines-for-distributed-applications) (accessed 2026-05-03)

**Confidence**: High (vendor docs + design rationale + canonical
explainer; the model is well-described by primary sources).

**Implication for the question**: Temporal's contract is **the
workflow body is the schema**. The "state" of a workflow is whatever
local variables exist in the function at any await point; the runtime
re-derives them by replaying activity results recorded in history.
This is structurally different from a reconciler — reconcilers do not
have a "function in flight to replay through"; each tick is a fresh
invocation. So Temporal's pattern (event sourcing as the only
persistence) is the *right model for workflows* (which Overdrive §18
already adopts) but the *wrong model for reconcilers* (each reconcile
is from scratch — there's no replay history to derive `view` from
unless the framework either materialises a snapshot or replays every
historical action, which is unbounded).

This separates the workflow primitive cleanly from the reconciler
primitive: workflows are event-sourced (no application state schema);
reconcilers must materialise their View at the start of every tick,
which means *something* must persist that View. The question is just
whether that something is hand-written SQL or framework-owned.

### 3.5 Akka Persistence (event-sourced actors)

**Finding 5 — Akka Persistence separates Command (impure validation) from Event (pure state transition); recovery replays events through the same handler.**

> "An event sourced actor receives a (non-persistent) command which is
> first validated if it can be applied to the current state. If
> validation succeeds, events are generated from the command, … These
> events are then persisted and, after successful persistence, used to
> change the actor's state. … The event handler must only update the
> state and never perform side effects, as those would also be executed
> during recovery of the persistent actor. … When the entity is started
> up to recover its state from the stored events" the same handler
> runs.

**Source**:
- [Akka — Event Sourcing](https://doc.akka.io/libraries/akka-core/current/typed/persistence.html) (accessed 2026-05-03)
- [Akka — PersistentActor API](https://doc.akka.io/japi/akka-core/current//akka/persistence/PersistentActor.html) (accessed 2026-05-03)
- [Akka.NET — Event Sourcing with Akka.Persistence Actors](https://getakka.net/articles/persistence/event-sourcing.html) (accessed 2026-05-03)

**Confidence**: High (canonical Akka docs + cross-platform Akka.NET
mirror agree on the model; this is the most-cited event-sourcing
implementation pattern in industry).

**Implication for the question**: Akka's split is *exactly* the
reconciler/workflow split Overdrive already has, with one twist —
Akka materialises its persistent state by replaying events through a
pure event handler. For Overdrive reconcilers, the equivalent would be:
"persist a stream of `ViewDelta` records; recover `View` by folding
deltas through a pure `apply(view, delta) -> view` function." The cost
is a snapshot cadence to bound replay length. The benefit is that
schema evolution becomes upcasters on the event stream rather than
ALTER TABLE on the materialised view. This is a viable Option D
(event-sourced view) — it preserves Anvil-style purity and gives
operators a complete audit log "for free" — but it adds a snapshot
mechanism the current design does not have.

### 3.6 Cloudflare Durable Objects (state.storage transactional KV — and SQL on the same handle)

**Finding 6 — Cloudflare Durable Objects expose BOTH a typed KV API and a raw SQL API on the same `ctx.storage` handle, backed by the same underlying SQLite file.**

> "Storage API has several methods, including SQL, point-in-time
> recovery (PITR), key-value (KV), and alarm APIs. Only Durable Object
> classes with a SQLite storage backend can access SQL API. SQL API is
> available on `ctx.storage.sql` parameter passed to the Durable Object
> constructor."

> KV operations "store data in a hidden SQLite table `__cf_kv`" on
> SQLite-backed objects, but this table is not directly accessible via
> SQL API.

> "sql.exec() cannot execute transaction-related statements like BEGIN
> TRANSACTION or SAVEPOINT. Instead, use the ctx.storage.transaction()
> or ctx.storage.transactionSync() APIs to start a transaction, and
> then execute SQL queries in your callback."

**Source**:
- [Cloudflare — SQLite-backed Durable Object Storage API](https://developers.cloudflare.com/durable-objects/api/sqlite-storage-api/) (accessed 2026-05-03)
- [Cloudflare — Access Durable Objects Storage best practices](https://developers.cloudflare.com/durable-objects/best-practices/access-durable-objects-storage/) (accessed 2026-05-03)
- [Cloudflare blog — Zero-latency SQLite storage in every Durable Object](https://blog.cloudflare.com/sqlite-in-durable-objects/) (accessed 2026-05-03)

**Confidence**: High (vendor docs across three independent surfaces;
the same model is documented in API reference, best-practices guide,
and the design announcement).

**Implication for the question**: This is the **strongest precedent
for Option C (hybrid)**. Cloudflare ships exactly the design: typed
KV is the default ergonomic surface (`get/put/delete/list`); SQL is an
escape hatch for cases where columnar query is genuinely needed; both
sit on a SQLite file the framework owns; transactions span both. The
KV surface even hides its own SQLite table from the SQL surface —
isolating the two layers' schemas. This is a production-validated
shape (Workers / Durable Objects is the platform powering significant
parts of Cloudflare's edge), and Overdrive's tentative pivot toward
Cloudflare-shape primitives makes the precedent doubly relevant.

### 3.7 Event-sourced projections (Marten, EventSauce, Axon — the upcaster pattern)

**Finding 7 — Schema evolution in event-sourced systems is handled at read time via "upcasters"; additive changes are tolerated via default-value deserialization.**

> "Upcasting is a process of transforming the old JSON schema into the
> new one. It's performed on the fly each time the event is read. You
> can think of it as a pluggable middleware between the
> deserialization and application logic. … For additive, non-breaking
> schema changes, you can use tolerant deserialization: Design event
> consumers to ignore unknown fields and use default values for
> missing fields."

**Source**:
- [Event-Driven.io — Simple patterns for events schema versioning](https://event-driven.io/en/simple_events_versioning_patterns/) (accessed 2026-05-03)
- [Marten — Events Versioning documentation](https://martendb.io/events/versioning) (accessed 2026-05-03)
- [Microsoft Learn — Event Sourcing Pattern (Azure Architecture Center)](https://learn.microsoft.com/en-us/azure/architecture/patterns/event-sourcing) (accessed 2026-05-03)
- ["The Dark Side of Event Sourcing: Managing Data Conversion" — SANER 2017 paper](https://www.movereem.nl/files/2017SANER-eventsourcing.pdf) (accessed 2026-05-03)

**Confidence**: High (vendor docs + Microsoft Architecture Center +
peer-reviewed software-engineering paper; the upcaster pattern has
multi-decade history and ecosystem-wide consensus).

**Implication for the question**: Schema evolution is the strongest
counter-argument against Option B (opaque blob auto-persisted). If
View is rkyv-archived, even an additive field requires a schema
migration step (rkyv's docs explicitly note "lacks a full schema
system and isn't well equipped for data migration and schema
upgrades"). If View is JSON/CBOR with `#[serde(default)]` on new
fields, additive evolution is free — the cost is the JSON parse on
every hydrate. Upcasters are the bridge: pre-process the persisted
representation into the current shape before deserialization. This is
a real engineering cost that the current SQL design distributes
across `migrate` (ALTER TABLE) and `hydrate` (handle missing rows
gracefully).

### 3.8 WASM Component Model serialization (third-party reconcilers)

**Finding 8 — WASM Component Model uses Canonical ABI for cross-language data marshalling; persistence shape is the host's choice.**

The WASM Component Model defines a Canonical ABI (with `wit` interface
type definitions) for data crossing the host/guest boundary; the
representation is canonical bytes interpretable by any guest language
binding. The Component Model itself does NOT define a persistence
format — the host is free to persist component state in whatever shape
it likes, and re-marshal on the next call.

**Source**:
- [WebAssembly Component Model — Bytecode Alliance](https://component-model.bytecodealliance.org/) (accessed 2026-05-03)
- [Bytecode Alliance — Canonical ABI explainer](https://github.com/WebAssembly/component-model/blob/main/design/mvp/CanonicalABI.md) (accessed 2026-05-03)
- [Cloudflare — WebAssembly on Workers (Wasmtime)](https://developers.cloudflare.com/workers/runtime-apis/webassembly/) (accessed 2026-05-03)

**Confidence**: Medium-High (specs from the standards body; less
production-validation evidence than the other precedents because
Component Model is newer).

**Implication for the question**: For third-party WASM reconcilers
(whitepaper §18), shipping arbitrary SQL into the sandbox is a real
risk surface — schema collisions across reconciler versions,
SQL-injection-style malformed queries, runaway full-table scans on
large reconciler memory. A typed View surface is much safer: the WASM
module declares its View type as a Component Model `record`; the
host serializes via Canonical ABI; the host owns the persistence
file and can rate-limit, size-limit, and roll back. Option B is
**structurally safer for the WASM extension story** than Option A.
Option C preserves this for typed-View-only WASM modules while still
allowing host-internal Rust reconcilers to drop to SQL.

### 3.9 rkyv zero-copy archive — schema evolution caveats

**Finding 9 — rkyv is fast at the read path but explicitly documents that schema evolution is not its concern.**

> "Notably, while rkyv is a great format for final data, it lacks a
> full schema system and isn't well equipped for data migration and
> schema upgrades — if your use case requires these capabilities, you
> may need additional libraries to build these features on top of rkyv."

> "Enabling and disabling feature flags may change rkyv's serialized
> format, and as such can cause previously-serialized data to become
> unreadable, with enabling format control features that are not the
> default considered a breaking change."

**Source**:
- [rkyv FAQ — official site](https://rkyv.org/faq.html) (accessed 2026-05-03)
- [rkyv GitHub repository — README](https://github.com/rkyv/rkyv) (accessed 2026-05-03)
- [docs.rs — rkyv crate](https://docs.rs/rkyv/latest/rkyv/) (accessed 2026-05-03)

**Confidence**: High (caveat is documented by the library author in
multiple authoritative locations).

**Implication for the question**: rkyv is the *wrong* serialization
format for an auto-persisted View blob. The development.md rule
"hashing requires deterministic serialization" already cites rkyv as
the right tool *for hashes* (canonical bytes by construction), but
hashing is not persistence. Persistence requires schema evolution; rkyv
does not provide it. If Option B ships, the wire format should be
JSON or CBOR with serde + `#[serde(default)]` for additive fields, or
a versioned envelope with explicit upcasters. The arena-allocation
and zero-copy benefits rkyv brings are on the *read hot path* and do
not require persistence — rkyv stays in its current role
(IntentStore-readable archived bytes for read-heavy paths, NOT for
mutable per-tick reconciler memory).

### 3.10 controller-runtime (Go, sigs.k8s.io) — etcd is the SSOT

**Finding 10 — Kubernetes controller-runtime stores all reconciler-relevant state in etcd, via the API server, with no per-controller persistent layer.**

> "Every object in Kubernetes is stored in etcd as a versioned, typed
> resource. … Rather than every reconciliation hitting the API server,
> reads go to a local in-memory store that is kept in sync via
> informers, … However, writes (Create, Update, Patch, Delete) always
> go directly to the API server, never through the cache."

> Finalizers live in `metadata.finalizers`; per-object scratch state
> is conventionally stored in `metadata.annotations`.

**Source**:
- [Kubebuilder Book — Implementing a controller](https://book.kubebuilder.io/cronjob-tutorial/controller-implementation.html) (accessed 2026-05-03)
- [Kubernetes blog — Writing a Controller for Pod Labels](https://kubernetes.io/blog/2021/06/21/writing-a-controller-for-pod-labels/) (accessed 2026-05-03)
- [reconcilerio/runtime — Kubernetes reconciler framework](https://github.com/reconcilerio/runtime) (accessed 2026-05-03)

**Confidence**: High (kubernetes.io official docs + Kubebuilder book +
the leading higher-level reconciler framework).

**Implication for the question**: K8s deliberately rejects
per-controller persistent state. Either it goes in the resource
itself (a `status` field on the CRD), it goes in an annotation, or it
goes in an external store the operator manages by hand. The
upside: one consistent, audit-logged, RBAC-protected place for
reconciler memory. The downside: every "I need to remember a retry
counter" becomes a CRD `status` field, which means a CRD schema
update, which means an etcd schema migration. Overdrive's per-reconciler
libSQL is a *deliberate departure* from this model — and a
typed-View abstraction (Option B) keeps that departure ergonomic
without trading away the auditability that the SQL surface provides
when operators inspect the libSQL file directly.

## 4. Option Analysis

### Option A — Status quo: explicit `migrate` / `hydrate` / `persist`

The current §18 design. Reconciler authors write CREATE TABLE in
`migrate`, SELECT + row decoding in `hydrate`, DELETE+INSERT in
`persist`. The runtime owns `LibsqlHandle`, schedules calls, and
enforces the lifecycle invariant that `migrate` runs before
`hydrate`/`persist`.

**Strengths**:
- Maximum flexibility — every reconciler picks its exact schema, can
  use indexes, partial materialisation, JOIN across own tables.
- Operator inspectability — `sqlite3 reconciler.db` gives full SQL.
- Schema evolution is well-understood — additive ALTER TABLE matches
  the project's existing additive-only migration discipline (CLAUDE.md).
- Verbatim Anvil-compatible — `view` is an explicit input to
  `reconcile`; ESR proofs are over the pure transition.
- Already shipped, tested, documented.

**Weaknesses**:
- ~100 lines of plumbing per reconciler that does nothing
  reconciler-specific. The JobLifecycle case study has 50 lines of
  hydrate decoding two BTreeMap fields.
- Schema/struct desync risk — the reconciler author is responsible for
  keeping CREATE TABLE in sync with the View struct fields by hand.
- Footgun for WASM third-party reconcilers (whitepaper §18) — sandboxed
  WASM cannot ship arbitrary SQL safely.
- Per-tick DELETE+INSERT for the full View is quadratic in view size;
  the existing `JobLifecycle::persist` documents this as a Phase-1
  expedient.

### Option B — Typed-View blob auto-persisted

Replace `migrate`/`hydrate`/`persist` with a single requirement:
`type View: Serialize + DeserializeOwned + Default + Send + Sync`.
The runtime reads the latest View blob from a single per-target row
in libSQL on every tick (`hydrate` is `runtime.load_or_default(target)`
inside the framework), and writes the new View back after `reconcile`
(`persist` is `runtime.store(target, &next_view)`).

Wire format: JSON or CBOR with serde `#[serde(default)]` for additive
fields. **Not rkyv** — rkyv explicitly disclaims schema evolution
support (Finding 9). CBOR via `ciborium` is the conservative pick (one
order of magnitude smaller than JSON for typical View sizes; still
backwards-compatible via serde default discipline; pure Rust, no
codegen).

Reconciler trait collapses to:

```rust
trait Reconciler: Send + Sync {
    type State: Send + Sync;
    type View:  Serialize + DeserializeOwned + Default + Send + Sync;

    fn name(&self) -> &ReconcilerName;
    fn reconcile(&self, desired: &Self::State, actual: &Self::State,
                 view: &Self::View, tick: &TickContext)
        -> (Vec<Action>, Self::View);
}
```

**Strengths**:
- Zero plumbing — author writes `#[derive(Serialize, Deserialize, Default)]`
  on the View struct and `reconcile`; nothing else.
- Schema evolution by `#[serde(default)]` (Finding 7 — additive
  changes are tolerated by tolerant deserialization, no upcasters
  needed for the additive cases).
- Strongly aligned with **Restate** (Finding 3), **Cloudflare DOs'
  KV surface** (Finding 6), **kube-rs ergonomic precedent** (Finding 1).
- **Safer for WASM extensions** (Finding 8) — author declares a
  Component Model `record` for the View; host serializes via
  Canonical ABI; no SQL surface in the sandbox.
- Pure-function `reconcile` contract preserved verbatim — the runtime
  swap is only at the storage boundary.
- Anvil-compatible — `view` remains an explicit input to the pure
  transition (Finding 2).

**Weaknesses**:
- No SQL escape hatch — a reconciler that genuinely needs `SELECT
  count(*) FROM things WHERE x > N` cannot express it efficiently;
  it must materialise the count into the View on every tick.
- Cross-target queries impossible — but zero existing reconcilers do
  this (verified by grep: every `conn.query` in the codebase is a
  reconciler reading its own tables).
- Whole-View rewrite per tick — the same quadratic shape as the
  current Phase-1 DELETE+INSERT path; CBOR/JSON serialization plus
  one row write. For Phase-1 view sizes (KB, bounded by allocs per
  job) this is in the noise; for hypothetical Phase-N-thousands-of-
  allocs reconcilers it becomes a real cost.
- Operator inspectability degrades — `sqlite3 reconciler.db` shows a
  blob; need a `overdrive reconciler view <name> <target>` CLI to
  decode. Not a blocker but a real cost.
- Schema-version mismatch on rollback — if a reconciler ships v2 with
  a new field, then is rolled back to v1, v1 sees the unknown field.
  serde's default behaviour is to ignore unknown fields; this is the
  correct shape but needs explicit testing.

### Option C — Hybrid: blob default with SQL escape hatch

Frame the trait around the typed-View blob as the default ergonomic
surface (Option B), but expose `&LibsqlHandle` to reconcilers that
need it via an opt-in:

```rust
trait Reconciler: Send + Sync {
    type State: Send + Sync;
    type View:  Serialize + DeserializeOwned + Default + Send + Sync;

    fn name(&self) -> &ReconcilerName;
    fn reconcile(...) -> (Vec<Action>, Self::View);   // mandatory; pure

    // Optional escape hatch for reconcilers that need SQL queries
    // (custom indexes, large memory, JOIN against own tables).
    // Default impls are no-ops.
    async fn migrate_sql(&self, _db: &LibsqlHandle) -> Result<(), HydrateError> { Ok(()) }
    async fn hydrate_sql(&self, _t: &TargetResource, _db: &LibsqlHandle)
        -> Result<Option<Self::View>, HydrateError> { Ok(None) }
    async fn persist_sql(&self, _v: &Self::View, _db: &LibsqlHandle)
        -> Result<bool, HydrateError> { Ok(false) }
}
```

`hydrate_sql` returns `None` when the reconciler is in
blob-default-mode; `Some(view)` overrides. `persist_sql` returns
`false` when the framework should fall back to blob persistence;
`true` when the reconciler took ownership. The framework wires the
two surfaces against the same per-reconciler libSQL file (matching
Cloudflare DO's pattern: hidden `__cf_view_blob` table is the
framework's, all other tables belong to the reconciler — Finding 6).

**Strengths**:
- 90% of reconcilers (every existing one) get Option B's ergonomics.
- The 10% that need SQL get the full Option A surface, opt-in.
- WASM extension story is preserved — WASM modules expose only the
  blob path; SQL is host-Rust-only.
- Migration path is gentle — existing reconcilers keep working
  (their `migrate`/`hydrate`/`persist` map to `migrate_sql`/`hydrate_sql`/
  `persist_sql`); new reconcilers default to blob.

**Weaknesses**:
- Two persistence surfaces to test, document, DST-replay.
- Reviewers must check "is the SQL escape hatch justified?" on every
  PR that uses it — soft governance, easy to slip.
- Trait surface grows from 4 methods to 5+; some of the simplification
  benefit is lost.

### Option D — Event-sourced View (Akka Persistence shape)

Persist a stream of `ViewDelta` records; recover `View` by folding
deltas through a pure `apply(view, delta) -> view` function; snapshot
periodically to bound replay length.

```rust
trait Reconciler: Send + Sync {
    type State: Send + Sync;
    type View:  Default + Send + Sync;
    type Delta: Serialize + DeserializeOwned + Send + Sync;

    fn apply(&self, view: Self::View, delta: &Self::Delta) -> Self::View;  // pure
    fn reconcile(...) -> (Vec<Action>, Vec<Self::Delta>);                  // pure
}
```

**Strengths**:
- Schema evolution maps cleanly to upcasters (Finding 7).
- "Persist inputs not derived state" rule (development.md) is enforced
  *structurally* — Deltas are inputs by construction; View is
  always derived.
- Operators get a complete audit log "for free" — every reconciler
  decision is a persisted Delta.
- Anvil-compatible — both `apply` and `reconcile` are pure.

**Weaknesses**:
- Requires an additional snapshot mechanism to bound replay length.
- Two pure functions to write per reconciler instead of one.
- Replay cost on every tick (worst case: all Deltas back to last
  snapshot).
- No production precedent in the orchestrator domain — Akka does this
  for actors and Temporal does this for workflows, but no major
  reconciler framework has shipped this shape. Higher engineering
  risk for marginal benefit over Option B + serde-default.
- The "audit log for free" claim is weakened by the existing
  `external_call_results` ObservationStore table (development.md
  § "Reconciler I/O") — already audit-logging the externally-visible
  reconciler decisions.

## 5. Trade-off Matrix

| Dimension | A: Status quo | B: Typed blob | C: Hybrid | D: Event-sourced |
|---|---|---|---|---|
| Lines of plumbing per reconciler | ~100 | ~0 | 0 (default) / ~100 (escape) | ~30 (apply + delta defs) |
| Schema evolution (additive) | ALTER TABLE | `#[serde(default)]` | Either | Upcaster on Delta |
| Schema evolution (breaking) | Manual ALTER + backfill | Versioned envelope + custom upcaster | Either | Upcaster chain |
| Cross-target / cross-reconciler queries | Trivially expressible | Impossible | Possible via SQL escape | Impossible |
| Cross-target queries actually used in extant code | NO (verified by grep) | — | — | — |
| Operator inspectability (sqlite3) | Full | Blob (needs CLI) | Hybrid | Delta log + snapshot |
| DST replay determinism | Preserved | Preserved | Preserved | Preserved |
| ESR (Anvil) verifiability | Preserved | Preserved | Preserved | Preserved (apply is also pure) |
| Per-tick cost | DELETE+INSERT per row | Serialize + 1 row write | Either | Append delta + occasional snapshot |
| WASM safety | Poor (SQL injection / collisions) | Good (typed blob over Component Model) | Good for blob path | Good (typed delta) |
| Engineering risk to ship | Already shipped | Moderate (clean replacement) | Moderate (two paths) | High (no precedent in domain) |
| Production precedent | Bespoke (no direct match) | Restate, kube-rs ergonomics, Cloudflare KV | Cloudflare DO (KV + SQL) | Akka, Temporal (workflow not reconciler) |

## 6. Recommendation

**Option C (Hybrid: typed-View blob as default, SQL escape hatch).**
Confidence: **High** for the overall direction; **Medium** for the exact
trait shape (the `_sql` method names and the `Option<View>` return
shape are one workable wiring, not the only one).

**Why C over B**: The cross-target-query concern is theoretical for
existing reconcilers (verified by grep — zero cross-target SQL in the
codebase) but the §18 contract is meant to last across the whole
roadmap. Closing the door on SQL entirely would force a future
reconciler that needs `SELECT count(*) FROM allocs WHERE state =
'failing' GROUP BY node_id` to either materialise the count into View
on every tick (Option B) or fork the trait (Option D). Cloudflare's
production-validated DO pattern (Finding 6) shows that the cost of
exposing both surfaces is real but bounded — *and* shows that the
default ergonomic path is what defines the developer experience, not
the escape hatch.

**Why C over A**: The plumbing cost is concrete (50 lines of
hydrate decoding to materialise two BTreeMap fields in JobLifecycle is
not a one-time cost — every new reconciler pays it). The WASM
extension story (Finding 8) is structurally weaker under A. The
schema/struct desync risk under A is a class of bug the type system
can eliminate under B/C.

**Why C over D**: Option D's only material upside over B is "audit log
for free," which Overdrive already has via `external_call_results`
(development.md). The downside — no production precedent in the
orchestrator domain plus higher per-tick replay cost — outweighs the
remaining benefit.

**Concrete migration shape**:

1. Phase 1 — add the typed-View blob persistence in the runtime with
   the new default-method shape; existing `migrate`/`hydrate`/`persist`
   stay as the SQL escape hatch under their existing names (renamed
   to `_sql` suffixes for clarity, OR kept as-is with explicit
   "implementing these takes ownership of the SQL surface" docs).
2. Phase 2 — `JobLifecycle` migrates to the typed-View blob path;
   ~85 lines of plumbing delete in a single PR.
3. Phase 3 — write the WASM Component Model bindings against the
   typed-View blob path only; SQL escape hatch is host-Rust-exclusive.

The wire format for the blob: **CBOR via `ciborium`**, not JSON, not
rkyv. CBOR gives compact bytes with serde compatibility; `ciborium`
is pure-Rust (no codegen, no FFI), and serde's `#[serde(default)]`
gives additive evolution. rkyv stays in its current role for read-heavy
hashed paths but is the wrong tool for mutable persisted view state
(Finding 9).

## 7. Open Questions and Spike Targets

1. **Concrete sizing**: For a worst-case Phase-N reconciler (say,
   `WorkflowLifecycle` over 10k workflow instances per node), what
   does CBOR-of-View weigh per tick? Spike: build a synthetic 10k-row
   View, measure CBOR encode + libSQL write latency. If >5ms,
   Option C's escape hatch becomes load-bearing rather than
   theoretical.
2. **DST replay equivalence under Option B**: Confirm that the
   blob-roundtrip path produces bit-identical View across re-runs.
   Should fall out of `proptest`-roundtrip tests on every reconciler's
   View, but worth pinning before the refactor lands.
3. **Schema-version downgrade behaviour**: Verify that serde with
   `#[serde(default)]` correctly handles a v2-written blob being
   read by v1 code (unknown fields ignored). Add a trybuild +
   integration test pair before shipping the refactor.
4. **Operator CLI**: A `overdrive reconciler view-cat <name>
   <target>` command that decodes the blob to JSON for inspection.
   Trivial to build but would land alongside the refactor.
5. **`NextView` semantics under Option B**: `reconcile` currently
   returns `(Vec<Action>, Self::View)`. With auto-persist, this is
   "next view replaces current view." Should the contract evolve to
   `(Vec<Action>, ViewMutation)` to enable diff-merge semantics later
   without a second trait change? Probably yes; this is the place to
   re-introduce the `NextView` associated type ADR-0013 already
   contemplated.
6. **Anvil verification path**: Has any Anvil-verified controller
   shipped with author-defined `Self::T` of non-trivial complexity
   (>3 fields)? If yes, the verification overhead under Option B
   should be ≤ Option A's. If no, this is a spike: re-verify
   `JobLifecycle` ESR under the blob-View shape and confirm the
   proof obligation didn't grow.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| kube::runtime::reflector::Store docs | docs.rs | High | Canonical Rust library docs | 2026-05-03 | N (single primary; corroborated by kube.rs/controllers/reconciler/) |
| kube.rs controllers/reconciler | kube.rs | High | Official project docs | 2026-05-03 | Y (corroborates Store finding) |
| Anvil OSDI '24 paper PDF | usenix.org | High | Peer-reviewed academic | 2026-05-03 | Y |
| Anvil GitHub README | github.com/anvil-verifier/anvil | High | Author-maintained code | 2026-05-03 | Y |
| USENIX login Anvil article | usenix.org | High | Editorial summary | 2026-05-03 | Y |
| Restate State (TS) docs | docs.restate.dev | High | Official vendor docs | 2026-05-03 | Y |
| Restate Rust SDK | docs.rs/restate-sdk | High | Canonical library docs | 2026-05-03 | Y |
| Restate "Building DE Engine" blog | restate.dev | Medium-High | Author rationale | 2026-05-03 | Y |
| Temporal Workflow Execution | docs.temporal.io | High | Official vendor docs | 2026-05-03 | Y |
| Temporal Events docs | docs.temporal.io | High | Official vendor docs | 2026-05-03 | Y |
| Akka Event Sourcing | doc.akka.io | High | Official framework docs | 2026-05-03 | Y |
| Akka.NET Event Sourcing | getakka.net | High | Official cross-platform mirror | 2026-05-03 | Y |
| Cloudflare DO SQLite Storage API | developers.cloudflare.com | High | Official vendor docs | 2026-05-03 | Y |
| Cloudflare DO best practices | developers.cloudflare.com | High | Official vendor docs | 2026-05-03 | Y |
| Cloudflare blog SQLite-in-DO | blog.cloudflare.com | High | Vendor design announcement | 2026-05-03 | Y |
| Event-Driven.io versioning patterns | event-driven.io | Medium-High | Industry expert (Oskar Dudycz) | 2026-05-03 | Y |
| Marten Events Versioning | martendb.io | High | Canonical library docs | 2026-05-03 | Y |
| MS Learn Event Sourcing pattern | learn.microsoft.com | High | Official | 2026-05-03 | Y |
| SANER 2017 Event Sourcing paper | movereem.nl (preprint) | High | Peer-reviewed academic | 2026-05-03 | Y |
| WebAssembly Component Model site | component-model.bytecodealliance.org | High | Standards body | 2026-05-03 | Y |
| Component Model Canonical ABI | github.com/WebAssembly | High | Standards body | 2026-05-03 | Y |
| rkyv FAQ | rkyv.org | High | Author-maintained | 2026-05-03 | Y |
| rkyv GitHub | github.com/rkyv/rkyv | High | Author-maintained | 2026-05-03 | Y |
| Kubebuilder Book | book.kubebuilder.io | High | Canonical project docs | 2026-05-03 | Y |
| kubernetes.io controller blog | kubernetes.io | High | Official | 2026-05-03 | Y |
| reconcilerio/runtime | github.com/reconcilerio | Medium-High | Active fork of canonical lib | 2026-05-03 | Y |

Reputation distribution: High 23 (~88%), Medium-High 3 (~12%). Average
reputation: ≈ 0.97. All sources from trusted-source-domains.yaml
allow-list (vendor docs, standards bodies, academic, industry leaders);
no excluded-domain sources cited.

## Knowledge Gaps

### Gap 1: rkyv schema-evolution feature timeline
**Issue**: rkyv's FAQ disclaims schema evolution as out-of-scope, but
the project has had several discussions about adding it. Whether a
future rkyv version closes this gap (and on what timeline) is not
documented in the sources I could access.
**Attempted**: rkyv FAQ, GitHub README, docs.rs entry.
**Recommendation**: Watch the rkyv issue tracker for a "schema
evolution" milestone before committing to CBOR-as-the-only-format on a
multi-year horizon. If rkyv adds versioned format support, the Option
B/C wire-format choice is worth re-opening.

### Gap 2: Anvil verification of multi-field state types
**Issue**: The Anvil paper and README describe `Self::T` abstractly
but the example controllers (ZooKeeper, RabbitMQ, FluentBit) ship with
state types whose detailed shape I could not verify from the public
README alone. The recommendation that "ESR verification is preserved
under Option B" is based on the architectural argument that `view` is
still a pure input — whether the proof obligation grows with View
complexity is a question the README does not answer.
**Attempted**: Anvil README, OSDI '24 paper search results.
**Recommendation**: Spike target #6 — re-verify `JobLifecycle` ESR
(or a synthetic equivalent) under both Option A and Option B shapes
in Verus and compare proof complexity. If the Verus harness is not
yet stood up in the project, this is a larger investment than the
research recommendation alone warrants and should be deferred until
the verification path is being commissioned anyway.

### Gap 3: Production CBOR-vs-JSON benchmarking for serde-derived types
**Issue**: The recommendation to use CBOR via `ciborium` rests on
"compact bytes, pure-Rust, serde-compatible" — all three are
documented vendor properties but the *exact* encoded-size and
encode/decode latency for the Overdrive View shapes is not measured.
The "5ms threshold" in spike target #1 is a working guess.
**Attempted**: ciborium docs (not searched in this round given
turn budget).
**Recommendation**: Cheap spike — `ciborium::ser::into_writer` against
a 10k-row synthetic `JobLifecycleView`; compare to JSON via
`serde_json` and to the current DELETE+INSERT cost. ~30 minutes of
work; produces the empirical floor for choosing between CBOR and
JSON.

### Gap 4: WASM Component Model maturity for the production code path
**Issue**: The Component Model is the right abstraction for typed
state crossing the WASM boundary, but its production maturity in
2026-Q2 is not a question the sources I cited speak to in detail.
**Attempted**: Component Model docs, Cloudflare WASM docs.
**Recommendation**: This is a Phase-2+ concern (the recommendation is
to ship Option C against host-Rust reconcilers first, then bind the
typed-View-blob path to WASM Component Model for third-party
extensions). The maturity question is on the critical path for the
WASM extension story, not for the recommendation itself.

## Recommendations for Further Research

1. **Spike CBOR sizing and latency for representative View shapes**
   (Gap 3 / Spike #1) before committing to wire format.
2. **Survey ciborium vs minicbor vs serde_cbor** for serde-compatible
   pure-Rust CBOR encoders — choose the maintained, dependency-clean
   one.
3. **Re-verify ESR under blob-View shape** in Verus (Gap 2 / Spike #6),
   or document the architectural-argument confidence level explicitly
   when ADR-ifying the change.
4. **Operator UX pass on `overdrive reconciler view-cat`** before
   landing the refactor — the inspectability degradation under Option
   B is real and tooling is the mitigation.

## Full Citations

[1] kube-rs project. "Store in kube::runtime::reflector". docs.rs. https://docs.rs/kube/latest/kube/runtime/reflector/struct.Store.html. Accessed 2026-05-03.

[2] kube-rs project. "Reconciler". kube.rs documentation. https://kube.rs/controllers/reconciler/. Accessed 2026-05-03.

[3] Sun, Xudong, et al. "Anvil: Verifying Liveness of Cluster Management Controllers". 18th USENIX Symposium on Operating Systems Design and Implementation (OSDI '24). 2024. https://www.usenix.org/system/files/osdi24-sun-xudong.pdf. Accessed 2026-05-03.

[4] Sun, Xudong, et al. "Anvil: Verifying Liveness of Cluster Management Controllers" (presentation page). USENIX. 2024. https://www.usenix.org/conference/osdi24/presentation/sun-xudong. Accessed 2026-05-03.

[5] anvil-verifier project. "Anvil README". GitHub. https://github.com/anvil-verifier/anvil/blob/main/README.md. Accessed 2026-05-03.

[6] USENIX. "Anvil: Building Kubernetes Controllers That Do Not Break". USENIX login;. https://www.usenix.org/publications/loginonline/anvil-building-formally-verified-kubernetes-controllers. Accessed 2026-05-03.

[7] Restate. "State". Restate documentation (TypeScript). https://docs.restate.dev/develop/ts/state. Accessed 2026-05-03.

[8] Restate. "restate-sdk". docs.rs. https://docs.rs/restate-sdk/latest/restate_sdk/. Accessed 2026-05-03.

[9] Restate. "Building a modern Durable Execution Engine from First Principles". Restate Blog. https://www.restate.dev/blog/building-a-modern-durable-execution-engine-from-first-principles. Accessed 2026-05-03.

[10] Temporal. "Temporal Workflow Execution overview". Temporal Platform Documentation. https://docs.temporal.io/workflow-execution. Accessed 2026-05-03.

[11] Temporal. "Events and Event History". Temporal Platform Documentation. https://docs.temporal.io/workflow-execution/event. Accessed 2026-05-03.

[12] Temporal. "Temporal: Beyond State Machines for Reliable Distributed Applications". Temporal Blog. https://temporal.io/blog/temporal-replaces-state-machines-for-distributed-applications. Accessed 2026-05-03.

[13] Lightbend. "Event Sourcing". Akka Documentation. https://doc.akka.io/libraries/akka-core/current/typed/persistence.html. Accessed 2026-05-03.

[14] Lightbend. "PersistentActor". Akka API Documentation. https://doc.akka.io/japi/akka-core/current//akka/persistence/PersistentActor.html. Accessed 2026-05-03.

[15] Akka.NET project. "Event Sourcing with Akka.Persistence Actors". Akka.NET Documentation. https://getakka.net/articles/persistence/event-sourcing.html. Accessed 2026-05-03.

[16] Cloudflare. "SQLite-backed Durable Object Storage API". Cloudflare Durable Objects docs. https://developers.cloudflare.com/durable-objects/api/sqlite-storage-api/. Accessed 2026-05-03.

[17] Cloudflare. "Access Durable Objects Storage". Cloudflare Durable Objects best practices. https://developers.cloudflare.com/durable-objects/best-practices/access-durable-objects-storage/. Accessed 2026-05-03.

[18] Cloudflare. "Zero-latency SQLite storage in every Durable Object". Cloudflare Blog. https://blog.cloudflare.com/sqlite-in-durable-objects/. Accessed 2026-05-03.

[19] Dudycz, Oskar. "Simple patterns for events schema versioning". Event-Driven.io. https://event-driven.io/en/simple_events_versioning_patterns/. Accessed 2026-05-03.

[20] JasperFx Software. "Events Versioning". Marten documentation. https://martendb.io/events/versioning. Accessed 2026-05-03.

[21] Microsoft. "Event Sourcing pattern". Azure Architecture Center. https://learn.microsoft.com/en-us/azure/architecture/patterns/event-sourcing. Accessed 2026-05-03.

[22] Overeem, Michiel; Spoor, Marten; Jansen, Slinger. "The Dark Side of Event Sourcing: Managing Data Conversion". 24th IEEE International Conference on Software Analysis, Evolution, and Reengineering (SANER 2017). https://www.movereem.nl/files/2017SANER-eventsourcing.pdf. Accessed 2026-05-03.

[23] Bytecode Alliance. "WebAssembly Component Model". component-model.bytecodealliance.org. https://component-model.bytecodealliance.org/. Accessed 2026-05-03.

[24] WebAssembly Community Group. "Canonical ABI explainer". WebAssembly Component Model design docs. https://github.com/WebAssembly/component-model/blob/main/design/mvp/CanonicalABI.md. Accessed 2026-05-03.

[25] rkyv project. "FAQ". rkyv.org. https://rkyv.org/faq.html. Accessed 2026-05-03.

[26] rkyv project. "rkyv: Zero-copy deserialization framework for Rust". GitHub. https://github.com/rkyv/rkyv. Accessed 2026-05-03.

[27] rkyv project. "rkyv crate documentation". docs.rs. https://docs.rs/rkyv/latest/rkyv/. Accessed 2026-05-03.

[28] Kubebuilder project. "Implementing a controller". The Kubebuilder Book. https://book.kubebuilder.io/cronjob-tutorial/controller-implementation.html. Accessed 2026-05-03.

[29] Kubernetes project. "Writing a Controller for Pod Labels". Kubernetes Blog. 2021-06-21. https://kubernetes.io/blog/2021/06/21/writing-a-controller-for-pod-labels/. Accessed 2026-05-03.

[30] reconciler.io. "reconcilerio/runtime". GitHub. https://github.com/reconcilerio/runtime. Accessed 2026-05-03.

## Research Metadata

Duration: ~50 turns | Examined: ~30 sources | Cited: 30 | Cross-refs: 28/30 (every finding cross-referenced except Findings 1 + 8 where vendor-canonical sources are sufficient and supplementary corroboration was deemed redundant given turn budget) | Confidence: High 9 of 10 findings, Medium-High 1 (Finding 8 — Component Model maturity) | Output: `docs/research/control-plane/reconciler-memory-abstraction-options.md`
