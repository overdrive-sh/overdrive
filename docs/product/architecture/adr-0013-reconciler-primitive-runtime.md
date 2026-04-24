# ADR-0013 — Reconciler primitive: trait in `overdrive-core`, runtime in `overdrive-control-plane`, libSQL private memory, shipped whole

## Status

Accepted. 2026-04-23. Amended 2026-04-24 (pre-hydration
pattern — see Changelog). Amended 2026-04-24 (time injection via
`TickContext` parameter — see Changelog).

## Context

Whitepaper §18 defines the reconciler primitive as the first of two
orthogonal control-plane primitives (the other is the workflow
primitive, shipping in Phase 3). The defining property is a **pure
function** `reconcile(desired, actual, db) -> Vec<Action>` with an
**evaluation broker that collapses duplicates via a cancelable-eval-set**
— shipped native, not retrofitted after a Nomad-shaped incident.

Slice 4 effort estimate: 1–2 days (largest slice by risk). DISCUSS
Key Decision 7 + wave-decisions.md "What DESIGN wave should focus on"
items 5–7 roll up into this single ADR:

- Where does the `Reconciler` trait live?
- Where does the `ReconcilerRuntime` + `EvaluationBroker` live?
- What is the per-primitive private-memory story (libSQL), and how are
  paths derived to enforce isolation?
- Does slice 4 ship whole, or split into 4A (trait + runtime + libSQL +
  noop-heartbeat) + 4B (broker + DST invariants)?
- `async_trait` vs native async-in-trait for the new trait surfaces.

## Decision

### 1. Module ownership

**The `Reconciler` trait, the `Action` enum, the `Db` handle type, and
the `ReconcilerName` newtype live in `overdrive-core::reconciler`.**

**The `ReconcilerRuntime`, `EvaluationBroker`, `ReconcilerRegistry`,
and the per-primitive libSQL path provisioner live in
`overdrive-control-plane::reconciler_runtime`.**

The rationale: the trait is a *port* (every reconciler author depends
on it); the runtime is a *wiring adapter* (composes the ports into
something that runs). Authors in future phases depend on `overdrive-core`
only; the runtime is an implementation concern of the single-mode
control-plane binary.

### 2. Trait shape — pre-hydration pattern

```rust
// in overdrive-core::reconciler
pub trait Reconciler: Send + Sync {
    /// Author-declared projection of the reconciler's private memory.
    /// Serves as the pure, sync input to `reconcile`. Typed Rust
    /// struct; author owns both the shape and the SQL that populates
    /// it. See `development.md` §Reconciler I/O.
    type View: Send + Sync;

    /// Canonical name — used for libSQL path derivation and evaluation
    /// broker keying. Newtype-validated (see `ReconcilerName`).
    fn name(&self) -> &ReconcilerName;

    /// Async read phase. The ONLY place a reconciler author touches
    /// libSQL. Runs under the runtime's async context. Returns an
    /// immutable snapshot that `reconcile` sees as pure data.
    ///
    /// Authors write free-form SQL against `db` inside this body and
    /// decode into `Self::View`. Schema management
    /// (`CREATE TABLE IF NOT EXISTS`, `ALTER TABLE ADD COLUMN`) is the
    /// author's responsibility here — no framework migrations Phase 1.
    async fn hydrate(
        &self,
        target: &TargetResource,
        db:     &LibsqlHandle,
    ) -> Result<Self::View, HydrateError>;

    /// Pure function over `(desired, actual, &view, &tick) →
    /// (Vec<Action>, NextView)`. Sync. No `.await`. No I/O. Wall-clock
    /// access is only via `tick.now` — never `Instant::now()` /
    /// `SystemTime::now()`. See §2c.
    ///
    /// `view` is the pre-hydrated snapshot produced by `hydrate`. The
    /// returned `NextView` is the author-declared replacement state;
    /// the runtime diffs `view` vs `NextView` and persists the delta
    /// to libSQL. Reconcilers DO NOT write libSQL directly — writes
    /// are expressed as data in the return value, not as side effects
    /// in the body. See whitepaper §18 and `development.md`
    /// §Reconciler I/O.
    ///
    /// `tick` carries the runtime's snapshot of `Clock::now()` taken
    /// once at evaluation start, plus a monotonic tick counter and
    /// the per-tick deadline. Time is input state, not a side
    /// channel. See §2c.
    fn reconcile(
        &self,
        desired: &State,
        actual:  &State,
        view:    &Self::View,
        tick:    &TickContext,
    ) -> (Vec<Action>, Self::View);
}
```

The split is load-bearing. `hydrate` is async because the chosen
backing store (`libsql 0.5.x`) exposes only an async API — the async
boundary has to live somewhere, and burying it inside a sync `reconcile`
via `block_on` is rejected (see Alternative G). Pre-hydration moves
the async to its natural place: the read phase. `reconcile` stays
pure and synchronous, satisfying the whitepaper §18 contract and the
Anvil (OSDI '24) `reconcile_core` shape for ESR verification.

This is the same architectural split every mature precedent converges
on: kube-rs `Store<K>` (sync reads out of an async-populated cache),
controller-runtime's cache-backed Reader, Anvil's pure `reconcile_core`
+ async shim, the Elm Architecture's `update : Msg -> Model ->
(Model, Cmd Msg)`, and Redux's middleware + pure-reducer. Cite
`docs/research/reconciler-prehydration-pattern/reconciler-prehydration-pattern-comprehensive-research.md`
§1, §2, §3, §5 for the four-way convergence.

Non-determinism stays out of `reconcile`: no `&dyn Clock` / `&dyn
Transport` / `&dyn Entropy` in its parameter list. External effects
are expressed through `Action::HttpCall` (executed by the runtime
shim, Phase 3) or by reading observation rows (already passed in via
`actual`) or by reading the pre-hydrated `view`.

### 2a. Dyn-compatibility strategy — `enum AnyReconciler`

`async fn` in traits stabilised in Rust 1.75 (December 2023) **but is
not dyn-compatible** (research §6.1, §6.3). A trait with `async fn
hydrate` cannot be stored as `Box<dyn Reconciler>` or `&dyn
Reconciler` on Rust 1.75–1.85; associated-type + async-fn combination
inherits the not-dyn-safe property from the RPIT (return-position
impl Trait) that `async fn` desugars to.

**Decision: Phase 1 replaces `HashMap<ReconcilerName, Box<dyn
Reconciler>>` with `HashMap<ReconcilerName, AnyReconciler>` where
`AnyReconciler` is a hand-rolled enum in `overdrive-core::reconciler`**:

```rust
// in overdrive-core::reconciler
pub enum AnyReconciler {
    NoopHeartbeat(NoopHeartbeat),
    // future variants land one-per-reconciler as Phase 2+ adds them:
    //   CertRotation(CertRotation),
    //   JobLifecycle(JobLifecycle),
    //   ...
}
```

`AnyReconciler` dispatches `name`, `hydrate`, and `reconcile` via a
match arm per variant. Static dispatch; zero heap allocation on the
hot path; compile-time exhaustiveness across every registered
reconciler kind.

The cost is that each new first-party reconciler adds one enum
variant and one match arm per dispatch method. For Phase 1 with
exactly one reconciler (`noop-heartbeat`) and Phase 2/3 adding
single-digit more, the boilerplate is acceptable. Third-party
reconcilers land through the WASM extension path (whitepaper §18
"Extension Model"), which is a separate subsystem and does not go
through `AnyReconciler`.

Two rejected dyn-compatibility alternatives:

- **`#[async_trait]` wrapper** (research §6.1): restores `dyn
  Reconciler`, costs one `Box::pin` per `hydrate` call. Viable but
  premature for a closed Phase 1 registry where `enum AnyReconciler`
  suffices. Revisit when / if the registry outgrows enum ergonomics
  (arbitrary threshold; likely ≥10 first-party reconcilers). The
  migration is additive — the trait shape does not change.
- **Manual boxed-future erasure** (research §6.3 option 3): write a
  `DynReconciler` with `Pin<Box<dyn Future<...>>>` returns. Gives up
  `type View`. Not recommended; rejected at DESIGN.

The `enum AnyReconciler` shape also deletes the need for the current
`Box<dyn Reconciler>` indirection in `ReconcilerRuntime::register`
and `reconcilers_iter` — the runtime stores enum values directly.

### 2b. Runtime hydrate-then-reconcile contract

The runtime's tick loop for each dispatched `Evaluation`:

```
1. Pick reconciler from registry by name           (enum dispatch)
2. Open (or reuse cached) LibsqlHandle for name    (path from §5)
3. tick <- TickContext { now: clock.now(),         (snapshot once;
                          tick: counter,             see §2c)
                          deadline: now + budget }
4. view <- reconciler.hydrate(target, db).await    (async; runtime owns the .await)
5. (actions, next_view) =
       reconciler.reconcile(&desired, &actual,     (sync; pure function)
                            &view, &tick)
6. Persist diff(view, next_view) to libsql         (runtime owns the write)
7. Dispatch actions to the runtime's action shim   (Phase 3)
```

The runtime never hands `&LibsqlHandle` to `reconcile`. Writes are
expressed as data in `NextView`, persisted by the runtime. Reconcile
remains pure over its inputs — DST-replayable and ESR-verifiable
(research §1.1, §10.5).

Phase 1 convention: `NextView = Self::View` (full replacement). The
runtime diffs against the prior view and persists the delta.
Full-View replacement is simplest and imposes no per-author
diff-protocol. A typed-diff shape (`NextView = ViewDiff<View>`) is an
additive future extension when View size makes re-serialisation
costly; deferred until a real reconciler drives the need (research
Recommendation Lane).

### 2c. Time injection — `TickContext` parameter

The pre-hydration amendment (§2, §2b) closed the question of how
`reconcile` reads private memory: through a pre-computed `&Self::View`
populated by `hydrate`. It did not close the symmetric question of how
`reconcile` reads **time**. The amendment banned `&dyn Clock` from
`reconcile`'s parameter list (Alternative D); this section specifies
the positive answer.

#### Problem

The purity contract (§2, Enforcement) forbids `Instant::now()` /
`SystemTime::now()` inside a `reconcile` body — they are the canonical
sources of non-determinism the DST harness exists to control. But
many real reconcilers genuinely need wall-clock to make decisions:
"has the retry budget elapsed?", "is this allocation past its
deadline?", "should the lease be renewed yet?". Without a path for
`reconcile` to see the current time, those reconcilers cannot be
written purely.

Re-introducing `&dyn Clock` as a `reconcile` parameter would solve the
read problem at the cost of re-introducing a non-data dependency:
`reconcile` would call `clock.now()`, which under DST is fine but
under any future refactor invites the same `Instant::now()` slip the
ban exists to prevent. It also re-couples `reconcile` to a trait
object, defeating the dyn-compatibility win in §2a.

#### Resolution

Time becomes another **pure input** to `reconcile`, plumbed through a
dedicated `TickContext` parameter the runtime constructs once per
evaluation:

```rust
// in overdrive-core::reconciler
pub struct TickContext {
    /// Snapshot of the injected `Clock::now()` at evaluation start.
    /// `SystemClock` in production, `SimClock` (turmoil-controlled,
    /// seeded) under DST.
    pub now:      Instant,

    /// Monotonic per-runtime counter, incremented once per dispatched
    /// evaluation. For debugging, telemetry, and tie-breaking in
    /// reconciler memory.
    pub tick:     u64,

    /// Runtime-imposed wall-clock budget for this `reconcile` call.
    /// `now + reconcile_budget` at construction; `reconcile`
    /// implementations MAY consult it to short-circuit work, but the
    /// runtime does not preempt — overrun surfaces as a back-pressure
    /// counter (see §8 Evaluation Broker).
    pub deadline: Instant,
}
```

The runtime's responsibility (extending §2b step 3): snapshot
`Clock::now()` exactly once per evaluation via the same injected
`Clock` trait DST already controls (whitepaper §21,
`testing.md` §"Sources of nondeterminism"). Package `now`, `tick`, and
`deadline` into a fresh `TickContext`. Pass `&TickContext` as the
fourth parameter to `reconcile`. The runtime never re-reads the clock
mid-tick: every `reconcile` call sees one consistent snapshot of "the
time at which this evaluation started," which is what makes the
function pure over its inputs.

Time is now in the same posture as observation: an input, not a side
channel. The two have different consistency models and different
origins, and the type system reflects that:

| Input | Origin | Consistency |
|---|---|---|
| `actual: &State` | CRDT-gossiped from peers | Eventually consistent, seconds-fresh |
| `view: &Self::View` | This node's libSQL via `hydrate` | Local, single-tick snapshot |
| `tick: &TickContext` | This node's `Clock` at evaluation start | Local, single-tick snapshot |

#### Why `TickContext` carries more than just `now`

Adding `tick` and `deadline` alongside `now` avoids a second signature
migration when the evaluation broker grows back-pressure-aware
behaviour:

- `tick: u64` is a monotonic counter useful for telemetry correlation
  ("which dispatch produced this action?") and for reconciler authors
  who want a deterministic tie-breaker that does not depend on
  wall-clock granularity.
- `deadline: Instant` is the natural home for the per-tick reconcile
  budget mentioned in §8 ("Evaluation Broker — Storm-Proof Ingress")
  and whitepaper §18. Reconcilers that do bounded work per tick can
  consult `deadline` to checkpoint progress into `NextView` without
  spawning a follow-up evaluation themselves.

Both fields are populated by the runtime; both are pure inputs to
`reconcile`. Adding them now costs one struct definition and avoids a
later breaking signature change.

#### DST property

`SimClock` is already the `Clock` implementation under DST (per
`testing.md` §"Sources of nondeterminism"). A seeded run produces a
deterministic sequence of `now` snapshots; replay against the same
seed produces bit-identical `TickContext` values; `reconcile_is_pure`
(twin invocation, §Enforcement) sees the same `tick` value on both
calls and asserts byte-identical `(Vec<Action>, NextView)` outputs.
Time-dependent logic in `reconcile` is therefore as DST-replayable as
storage-dependent logic.

#### Prior art

The pattern matches the convergence already cited in §2 for
pre-hydration. Each precedent answers the time question the same way:
inject it as data, never read it from inside the pure function.

- **controller-runtime** — `Reconcile(ctx context.Context, req
  Request)` threads time through `ctx.Deadline()` rather than
  letting controllers call `time.Now()` directly. The deadline is the
  runtime's contract with the reconciler.
- **Anvil (USENIX OSDI '24)** — models time as an external observable:
  resources carry `expires_at` fields; a separate observer mutates
  observation as time passes. The reconciler reads the resulting
  state; it never reads a clock. Overdrive's `TickContext` is lighter
  — time arrives as a per-tick input rather than as a live observable
  — but the principle ("pure reconcile never reads wall-clock") is the
  same.
- **Elm Architecture** — `Time.now` is a `Task Never Posix` (a
  command), not a function returning the current time inside `update`.
  The runtime resolves the command and feeds the result back into
  `update` as a `Msg` payload. Same shape: time is data the runtime
  hands to the pure function.

The research grounding lives in
`docs/research/reconciler-prehydration-pattern/reconciler-prehydration-pattern-comprehensive-research.md`
§6 (Clock trait injection pattern) and the §1 / §2 / §5 precedent
chains for controller-runtime, Anvil, and Elm respectively. No new
research pass was needed for this extension; the existing document
already establishes that every mature reconciler precedent treats
time as injected state.

### 3. Action enum — Phase 1 shape

```rust
// in overdrive-core::reconciler
pub enum Action {
    Noop,
    HttpCall {
        correlation:     CorrelationKey,
        target:          http::Uri,       // or a newtype wrapping Uri
        method:          http::Method,
        body:            Bytes,
        timeout:         Duration,
        idempotency_key: Option<String>,
    },
    StartWorkflow {
        spec:        WorkflowSpec,        // placeholder type; workflow
        correlation: CorrelationKey,      // runtime lands Phase 3
    },
}
```

`HttpCall` variant is shipped with the primitive surface even though
the runtime shim lands in Phase 3 (#3.11). Per development.md
§Reconciler I/O, authors writing reconcilers in Phase 1 can legitimately
only ship reconcilers whose actions are already executable (currently
`Noop`; `StartWorkflow` once #3.2 ships). The variant's presence in
Phase 1 locks the surface so a Phase 2 author does not *have* to
`async fn` their way around the purity contract.

### 4. `ReconcilerName` newtype

```rust
// in overdrive-core::reconciler
pub struct ReconcilerName(String);  // validated

impl FromStr for ReconcilerName {
    // Regex: ^[a-z][a-z0-9-]{0,62}$
    // Rejects: empty, uppercase, leading digit, '.', '..', '/', '\', ':'
}
```

Kebab-case with the same validation shape as other Overdrive newtypes
(see `development.md` §Newtype completeness). The strict character set
is what lets the libSQL path provisioner safely concatenate the name
into a filesystem path — no sanitisation layer required.

### 5. libSQL per-primitive path derivation

```
<data_dir>/reconcilers/<reconciler_name>/memory.db
```

- `data_dir` — defaults to `~/.local/share/overdrive` (XDG), overridable
  by `--data-dir` at control-plane startup.
- `reconciler_name` — validated `ReconcilerName`; by construction
  cannot contain `..`, `/`, `\`, or other path-traversal characters.
- `memory.db` — the libSQL database file. One per reconciler, full
  filesystem isolation.

The path provisioner in `overdrive-control-plane::reconciler_runtime`:

- Canonicalises `data_dir` via `std::fs::canonicalize` at startup (or
  creates it if missing) to resolve symlinks once.
- Concatenates `<canonicalised_data_dir>/reconcilers/<name>/memory.db`.
- Asserts the resulting path starts with `<canonicalised_data_dir>/reconcilers/`
  — defence-in-depth in case the newtype regex ever regresses.
- Creates the directory tree and opens the libSQL file.
- Returns an `Arc<Db>` handle exclusive to that reconciler.

The DST scenario `per_primitive_libsql_isolated` asserts that two
reconcilers `alpha` and `beta` get distinct paths and that `alpha`'s
`&Db` handle cannot read `beta`'s data.

### 6. Per-primitive storage — libSQL via `LibsqlHandle`

**Workspace adds `libsql` as the per-primitive private-memory backend.**

- License: MIT (Turso fork of SQLite). Pure Rust.
- Version: 0.5.x lineage per workspace pin; exact revision chosen at
  implementation time.
- Usage: one libSQL connection per reconciler, owned by the runtime,
  exposed to `hydrate` as `&LibsqlHandle` (a newtype wrapping the
  live `libsql::Connection`). The handle is **only** visible inside
  `hydrate`; `reconcile` never sees it. Writes produced by
  `reconcile` flow through the returned `NextView`, diffed and
  persisted by the runtime.
- No migration framework in Phase 1 — schemas are per-reconciler and
  the runtime does not manage them. The `noop-heartbeat` reconciler
  uses `type View = ()` and writes nothing.

`LibsqlHandle` is a real newtype over `Arc<libsql::Connection>` (or
equivalent) — not a placeholder. The type exists from step 04-01a
onward even though `noop-heartbeat`'s `hydrate` ignores it (its View
is the unit type).

libSQL (rather than `rusqlite` or `sqlx-sqlite`) matches whitepaper §4
and §17 naming explicitly: "libSQL (embedded SQLite) as the per-primitive
private-memory store." Same crate the incident memory will use in
Phase 3. The async-only API surface is libSQL's deliberate
structural choice (research §7.1, §7.2) — it is what necessitates
the pre-hydration pattern.

Schema evolution is the author's responsibility, at the View-struct
level. Changing `Self::View` is the compile-time trigger to revisit
the `CREATE TABLE` / `ALTER TABLE` statements inside `hydrate`. This
matches the Elm-style "typed Model, compiler proves shape-correctness"
pattern (research §10.3). Framework-level migrations are deferred to
Phase 3+.

### 7. Ship slice 4 whole

**Slice 4 ships as one work unit. It is NOT split into 4A / 4B.**

Rationale: the broker's storm-mitigation value is meaningless without
the DST invariants that prove it works. The `duplicate_evaluations_collapse`
invariant is the executable proof of the whitepaper §18 claim — separating
it from the broker implementation produces two half-useful PRs.

The DISCUSS wave pre-described a 4A / 4B split as a *crafter-time
fallback* if material complexity surfaces during implementation. That
escape hatch remains open — the crafter can split on their own judgment.
DESIGN does not pre-split.

### 8. Evaluation broker shape

```rust
// in overdrive-control-plane::reconciler_runtime
pub struct EvaluationBroker {
    // Key: (reconciler_name, target_resource).
    // Pending: the in-flight evaluation for a key (at most one).
    // Cancelable: evaluations displaced by a later duplicate at the
    //             same key, awaiting bulk reap.
    pending:    HashMap<(ReconcilerName, TargetResource), Evaluation>,
    cancelable: Vec<Evaluation>,
    counters:   BrokerCounters,  // queued / cancelled / dispatched
}
```

- Keyed on `(reconciler_name, target_resource)` per whitepaper §18.
- `submit()`: if no pending entry for the key, insert → `queued++`.
  If one exists, move the prior to `cancelable`, replace → `cancelled++`,
  `queued` unchanged.
- `drain_pending()`: empties `pending`, dispatches each via the
  runtime's invocation path → `dispatched++`.
- `reap_cancelable()`: empties `cancelable` in bulk. Phase 1 runs the
  reaper as an in-runtime loop every N ticks (N = 16, arbitrary but
  bounded). Phase 2+ promotes it to a proper reconciler
  (`evaluation-broker-reaper` per whitepaper §18 built-in primitives).
- Counters `queued`, `cancelled`, `dispatched` are exposed via
  `ClusterStatus` JSON body (ADR-0015).

### 9. `noop-heartbeat` reconciler

Registered at control-plane boot. Always returns `vec![Action::Noop]`
from `reconcile(...)`. Its purpose is living proof of the contract:
the DST invariant `at_least_one_reconciler_registered` asserts a
non-empty registry; `reconciler_is_pure` exercises the twin-invocation
contract against it.

## Considered alternatives

### Alternative A — Runtime in `overdrive-core`

**Rejected.** Would force `overdrive-core` to depend on `libsql`,
`tokio` schedulers, and wiring-layer concerns. `overdrive-core`
stays port-only; the runtime is adapter wiring.

### Alternative B — Separate `overdrive-reconciler-runtime` crate

**Rejected.** In Phase 1 the runtime has exactly one consumer (the
control-plane binary). Promoting it to a separate crate before a
second consumer exists is premature modularisation. Revisit if /
when a second runtime consumer appears (e.g. a test harness that
wants the runtime without the full control-plane stack).

### Alternative C — Native async-in-trait for `Reconciler`

**Rejected.** The trait is synchronous by design (purity contract).
There is no `async fn` in the trait surface to decide between
`async_trait` and native async-in-trait. The runtime's internal
scheduling loop uses native `async fn` against concrete types —
the `dyn`-compatibility concern does not apply.

### Alternative D — Async `Reconciler` with injected `&dyn Clock`

**Rejected.** Directly contradicts whitepaper §18 and
development.md §Reconciler I/O. The DST invariant
`reconciler_is_pure` would immediately fail. This is the bug the
primitive exists to prevent.

### Alternative E — `rusqlite` or `sqlx-sqlite` instead of libSQL

**Rejected.** Whitepaper §4 and §17 name libSQL as the per-primitive
private-memory backend. The incident memory (Phase 3) and the
per-primitive reconciler memory share the library — one dependency,
one SQLite flavour across the platform.

### Alternative F — Ship slice 4 split 4A + 4B

**Rejected at DESIGN; left as a crafter-time escape hatch.** The
broker-without-invariants option is half-baked. If the crafter
hits material complexity, splitting is still on the table.

### Alternative G — Sync `reconcile` with `block_on` over libsql's async API

**Rejected.** Considered: keep `fn reconcile(&self, desired, actual,
&Db) -> Vec<Action>` synchronous (the prior shape), and have `Db`
internally call `Handle::block_on(conn.query(...).await)` to bridge
libsql's async API into the sync body.

Research §9 (two primary Tokio sources plus two tracking issues)
rejects this on DST-compatibility grounds:

- `Handle::block_on` panics if called from inside an async context —
  which the reconciler runtime's scheduling loop IS, by construction.
- `tokio::task::block_in_place` (the workaround) panics on the
  `current_thread` runtime that turmoil-based DST uses. A sync
  wrapper using `block_on` + `block_in_place` would work in
  production and panic in the exact environment whitepaper §21
  requires to prove correctness — the inverse of the intended
  property.
- `spawn_blocking` avoids the panic but introduces a thread hop per
  libSQL call and needs a blocking thread pool the DST simulator
  does not provide.

Pre-hydration (Alternative vs the new §2 shape) is cleaner and
matches the four-way precedent convergence (Anvil, controller-runtime,
kube-rs, Elm/Redux) cited in §2. Swapping libsql → rusqlite (Option 3
in the prompt) is technically viable but pays a whitepaper-
consistency cost against §4 and §17, and leaves two SQLite stacks in
one binary once incident memory (Phase 3) lands. Rejected.

## Consequences

### Positive

- `Reconciler` trait is a port; portable across crates; testable in
  isolation.
- Runtime / broker / path-provisioner in one crate — cohesion wins.
- libSQL per-primitive isolation is by construction (newtype regex
  + canonicalised path); no runtime sanitisation.
- Synchronous trait sidesteps the async-in-trait debate entirely.
- Slice 4 shipping whole means the Nomad-shaped incident mitigation
  is live from day one with DST proof.

### Negative

- `libsql` is a new workspace dep. Build time + binary size impact
  accepted as the cost of the §18 memory story.
- Phase 1 runtime is single-threaded; broker drain happens on one
  async task. Phase 2+ may parallelise — not a Phase 1 concern.
- Adding a new reconciler requires choosing a kebab-case name; the
  regex is strict. Documented in the reconciler-author's README.

### Quality-attribute impact

- **Reliability — fault tolerance**: positive. Storm-proof ingress
  by construction.
- **Maintainability — testability**: positive. Pure trait + DST
  invariants + lint gate triple-defend the purity contract.
- **Maintainability — modularity**: positive. Port-adapter split
  maps the runtime boundary cleanly.

### Enforcement

The purity contract is enforced at three layers, all scoped to
`reconcile` — `hydrate` is explicitly outside the purity contract
because it is async and performs libsql reads by design:

1. **Trait-level** — the synchronous
   `reconcile(desired, actual, &view, &tick) -> (Vec<Action>, NextView)`
   signature forbids `.await` in implementations, so an impl cannot
   directly invoke async nondeterminism. The absence of any
   trait-object-shaped clock parameter, combined with the explicit
   `tick: &TickContext` plumbing (§2c), removes the legitimate
   reason an author would reach for `Instant::now()` in the body.
2. **Compile-time** — `dst-lint` (phase-1-foundation ADR-0006) scans
   any core-class crate that imports the trait for banned
   nondeterminism APIs (`Instant::now`, `SystemTime::now`, `rand::*`,
   `tokio::time::sleep`, raw `tokio::net::*`). `dst-lint` does NOT
   flag `async fn hydrate` bodies in the same crate — the banned-API
   gate excludes the hydrate path because its explicit purpose is
   async libsql I/O. Wall-clock reads inside `reconcile` are caught
   here: the only legitimate path to "now" is `tick.now`, which is
   a struct field access, not a banned API call.
3. **Runtime** — the DST invariant `reconciler_is_pure` catches any
   `reconcile` implementation that smuggles nondeterminism through
   the trait boundary (interior mutability, TLS statics, FFI) via a
   twin-invocation equivalence test. The predicate evaluates
   `r.reconcile(&desired, &actual, &view, &tick)` twice against an
   identical `(desired, actual, view, tick)` 4-tuple and asserts
   `Vec<Action>` and `NextView` are bit-identical between runs.
   Both invocations share the **same** `TickContext` instance — the
   invariant is "same inputs produce byte-identical outputs," and
   `tick` is one of the inputs. `hydrate` is NOT covered by
   `reconciler_is_pure` — it is async and may read libsql; its
   correctness is a separate concern.

Enforcement details:

- `Reconciler` trait is in `overdrive-core`; the `dst-lint` gate
  (phase-1-foundation ADR-0006) scans any core-class crate that
  imports it. Banned APIs in a `reconcile(...)` body fail the lint.
  This is the gate that catches `Instant::now()` /
  `SystemTime::now()` slips inside `reconcile` — the only legitimate
  source of "now" is the `tick: &TickContext` parameter.
- DST invariant `at_least_one_reconciler_registered` — live on
  every run.
- DST invariant `duplicate_evaluations_collapse` — fires N (≥3)
  concurrent evaluations at the same key; expects 1 dispatched,
  N-1 cancelled.
- DST invariant `reconciler_is_pure` — twin invocation with
  identical `(desired, actual, &view, &tick)` inputs expects
  bit-identical `(Vec<Action>, NextView)` outputs across both calls.
  Both calls receive the same `TickContext` reference; the runtime
  does not refresh the clock between them.
- A compile-time trybuild fixture asserts passing `&LibsqlHandle`
  through `reconcile`'s parameter list fails to compile — the
  handle's only visibility path is `hydrate`.
- A unit test asserts the libSQL path provisioner rejects
  path-traversal attempts via `ReconcilerName`'s constructor.
- A unit test asserts the canonicalised path is under the
  configured data_dir.

## References

- `docs/whitepaper.md` §18 (Reconciler and Workflow Primitives)
- `.claude/rules/development.md` §Reconciler I/O, §Workflow contract
- `.claude/rules/testing.md` §Tier 1 (DST) — invariant catalogue
- `docs/feature/phase-1-control-plane-core/discuss/user-stories.md`
  US-04
- `docs/feature/phase-1-control-plane-core/slices/slice-4-reconciler-primitive.md`
- `docs/feature/phase-1-control-plane-core/discuss/wave-decisions.md`
  Key Decision 7
- `docs/research/reconciler-prehydration-pattern/reconciler-prehydration-pattern-comprehensive-research.md`
  — 2026-04-24 research grounding the pre-hydration amendment
  (Anvil §1, controller-runtime §2, kube-rs §3, Temporal/Restate §4,
  Elm/Redux §5, Rust trait mechanics §6, libSQL §7,
  pre-hydration pitfalls §8, `block_on` rejection §9, author-ergonomics
  §10)

## Changelog

- 2026-04-23 — Remediation pass (Atlas peer review, APPROVED-WITH-NOTES):
  added summary lead-in to `### Enforcement` naming the three layers of
  purity enforcement (trait-level, compile-time, runtime) so a standalone
  reader can parse the enforcement chain without threading it together
  from Trait Design + Testing sections. No semantic change; bullets
  unchanged.

- 2026-04-24 — Pre-hydration amendment. The `Reconciler` trait shape in
  §2 is split into an async `hydrate(target, &LibsqlHandle) ->
  Result<Self::View, HydrateError>` read phase and a sync pure
  `reconcile(desired, actual, &view) -> (Vec<Action>, NextView)`
  compute phase. The placeholder `Db` struct is removed; its role
  (read from libsql) is now discharged exclusively inside `hydrate`;
  its role (write to libsql) is replaced by the `NextView` return
  value, which the runtime diffs against the input view and persists.
  §2a introduces `AnyReconciler` (enum-dispatch) as the Phase 1
  registry shape — the `async fn hydrate` makes the trait
  non-dyn-compatible on Rust 1.75–1.85 (research §6.1, §6.3), so
  `Box<dyn Reconciler>` is replaced with `HashMap<ReconcilerName,
  AnyReconciler>`. §2b documents the runtime's hydrate-then-reconcile
  contract. §6 specifies `LibsqlHandle` as a real newtype (no longer
  a placeholder). Alternative G is added, recording the rejection of
  sync `reconcile` with `block_on` — research §9 establishes that the
  `block_in_place` workaround panics on turmoil's `current_thread`
  runtime, the exact environment DST runs in. Enforcement is updated
  to scope the purity contract to `reconcile` and to add a trybuild
  fixture that forbids `&LibsqlHandle` in the `reconcile` parameter
  list. Cites
  `docs/research/reconciler-prehydration-pattern/reconciler-prehydration-pattern-comprehensive-research.md`
  (879 lines, 44 sources, avg reputation 0.995).

- 2026-04-24 — Time-injection extension. Added §2c specifying
  `TickContext { now, tick, deadline }` as a new (fourth) parameter
  to `reconcile`. Closes the "how does `reconcile` access time"
  question left open by the pre-hydration amendment: time is injected
  state, not read from a trait or a `&dyn Clock` parameter. The
  runtime snapshots `Clock::now()` once per evaluation via the same
  injected `Clock` trait DST already controls, packages it into a
  `TickContext` alongside a monotonic tick counter and a per-tick
  deadline, and passes it by reference to `reconcile`. §2 trait
  shape adds `tick: &TickContext` to the `reconcile` signature; §2b
  runtime contract gains a "snapshot once" step before `hydrate`;
  Enforcement updates `reconciler_is_pure` to share one
  `TickContext` across both twin invocations (same inputs, same
  outputs). Adding `tick` and `deadline` alongside `now` avoids a
  second signature migration when the evaluation broker grows
  back-pressure-aware behaviour (§8). Prior art: controller-runtime's
  `ctx.Deadline()`, Anvil's expires_at-as-observation pattern (USENIX
  OSDI '24), Elm's `Time.now : Task Never Posix`. References
  `docs/research/reconciler-prehydration-pattern/reconciler-prehydration-pattern-comprehensive-research.md`
  §1, §2, §5, §6.
