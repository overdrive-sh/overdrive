# Development Guidelines

Rust-specific implementation patterns that apply across the Overdrive codebase.
These are not architectural decisions (those live in the whitepaper); they are
conventions for how we write Rust in the reconciler, dataplane, and store paths.

---

## Type-driven design

Guiding principle: **make invalid states unrepresentable.** Lean on the type
system to enforce correctness at compile time rather than runtime
validation.

### Sum types over sentinels

Use `enum` to model mutually exclusive states explicitly. Do not use
sentinels (`None`, `-1`, empty `Vec`) to carry semantic meaning.

```rust
// Bad — ambiguous: does None mean "not yet computed" or "no match"?
struct Alloc {
    placement: Option<NodeId>,
}

// Good — every state is explicit
enum Placement {
    Pending,
    Scheduled(NodeId),
    Rejected { reason: String },
    Failed { error: DriverError },
}
```

This compounds with the newtype rules below: once a concept has a
dedicated type, its states should be a dedicated sum type.

---

## Allocation strategy

Two complementary tools. Pick the right one for the lifetime shape of the data.

### Arena allocation (bumpalo) — reconciler scratch

Use for short-lived intermediate state whose lifetime is bounded by a single
reconcile iteration, a single request, or a single workflow step.

```rust
fn reconcile(intent: &IntentNode) -> Result<Vec<Action>> {
    let bump = Bump::new();
    let parsed  = parse_spec_into(&bump, &intent.spec)?;
    let actual  = fetch_actual_into(&bump, &intent.id)?;
    let diff    = compute_diff(&bump, &parsed, &actual);
    emit_actions(&diff)     // only the returned actions escape
    // bump drops here — all intermediates freed in one pointer reset
}
```

**When to reach for it:**
- Reconciler hot path (diff buffers, derived intermediate state, per-iteration
  scratch).
- Per-request work in the gateway or sidecar handler chain.
- Per-investigation work in the SRE agent (tool-call buffers, prompt
  assembly).

**When NOT to use it:**
- Anything that needs to outlive the iteration. Cache entries, reconciler
  memory rows, workflow journal entries — these go on the global heap or in
  the per-primitive libSQL store.
- Types with non-trivial `Drop` (file handles, sockets, other RAII
  resources). Bump skips destructors by default; you'll leak.
- Async scopes that span multiple reconcile iterations or `.await` points
  outside the arena's scope. Keep the arena within a single synchronous span
  or single async task.
- I/O-bound work. If the bottleneck is the syscall, not the allocator,
  bumpalo buys nothing.

### Zero-copy deserialization (rkyv) — persistent inputs

Use for durable data that reconcilers *read* — IntentStore rows, Raft log
entries, Corrosion row payloads, incident-memory blobs. rkyv encodes the
in-memory layout directly; readers access `&ArchivedT` against mmap'd bytes
without a deserialization pass.

**When to reach for it:**
- Any hot-path read out of redb, the Raft log, or Corrosion row values.
- Archived telemetry events in-flight (pre-Parquet).
- Incident-memory blobs retrieved for LLM context assembly.

**When NOT to use it:**
- External wire formats (gRPC, REST, OTel export) — stay with serde +
  protobuf/JSON for interop and schema evolution.
- Data with rapidly evolving schemas under active design. rkyv's evolution
  story is stricter than serde; additive-only discipline works, breaking
  changes need a migration step.
- Small, cold reads where deserialization cost is in the noise.

### Composition

The two stack naturally:

```
rkyv    →  read ArchivedJob directly from redb bytes         (no alloc)
bumpalo →  build diff, candidate placements, action buffers   (arena alloc)
heap    →  return Vec<Action> through Raft                    (global alloc)
```

The borrow checker enforces the boundaries: arena references can't escape the
`Bump`, archived references can't escape the backing byte slice, and the
`Action` values returned to Raft must be owned.

---

## Lifetime discipline in internal APIs

Orthogonal to the allocator choice — applies even when bumpalo and rkyv are
not in play.

- **Prefer `&str` over `String`** in function signatures when the callee does
  not need ownership. Borrow from the deserialized input for the duration of
  the call.
- **Use `Cow<'_, T>`** for data that is usually borrowed but occasionally
  needs modification. Common in label / annotation / header handling — the
  fast path stays zero-copy.
- **Use `#[serde(borrow)]`** on serde-deserialized structs where the parsed
  struct can hold `&str` into the original input bytes. Kills allocation for
  every string field in JSON/YAML-heavy reconcilers.
- **Reserve `Arc<T>`** for genuinely long-lived state shared across tasks
  (engines, caches, connection pools). Do not reach for `Arc` to dodge
  lifetime annotations on per-request data.

---

## State-layer hygiene (repeats §18 discipline in code form)

The three state layers each map to a specific allocation / storage pattern.
Crossing them accidentally is the class of bug the type system exists to
prevent.

| Layer | Store | Reading | Writing |
|---|---|---|---|
| Intent — what should be | `IntentStore` (redb / openraft+redb) | `&ArchivedT` via rkyv | Only via typed Raft actions |
| Observation — what is | `ObservationStore` (Corrosion / CR-SQLite) | SQL subscriptions | Owner-writer only, full rows |
| Memory — what happened | per-primitive libSQL | SQL | SQL |
| Scratch — this iteration | `Bump` | arena refs | arena alloc, dies at iteration end |

Enforce this with distinct trait objects (`IntentStore`, `ObservationStore`)
and distinct types per layer. Do not expose a shared `put(key, value)`
surface that lets the wrong call go to the wrong place.

---

## Persist inputs, not derived state

**Anywhere a value will be read back later — libSQL row, redb entry,
Corrosion table, on-disk artifact, JSON config, audit row, cache —
persist the *inputs* to whatever logic consumes the value, not the
*output* of that logic.** Derived values get recomputed on every read
from the persisted inputs and the live policy / function / lookup
table. This is the rule that makes policy evolution, schema stability,
and audit replay all work; violating it ships a stale cache of your
own logic.

The State-layer hygiene table above governs *where* state lives
(IntentStore, ObservationStore, libSQL memory, scratch). This rule
governs *what shape* the value takes regardless of which layer it sits
in.

### What counts as "derived"

A value is derived if it would change when one of these changes,
without the *inputs to it* changing:

- A constant elsewhere in the codebase (timeout, threshold, weight,
  schedule entry).
- A function or lookup table consulted at compute time (a backoff
  schedule, a scoring weighting, a Regorus policy, a routing table).
- An operator-configurable knob (today often a constant; tomorrow a
  per-tenant / per-job override).
- A cross-version migration of the logic itself (the v1 algorithm
  produced X; the v2 algorithm produces Y from the same inputs).

If the answer to "would editing this constant / table / policy change
the persisted value's correctness?" is yes, the value is derived. The
field that should be persisted is the input that feeds the constant /
table / policy, not the output it produces.

### Why

A persisted derived value is a stale cache of the logic that produced
it. Three failure modes follow, in order of how often they bite in
practice:

1. **Policy / configuration evolution silently no-ops.** Today's
   constant becomes tomorrow's operator-configurable knob (Kubernetes
   `restartPolicy` / Nomad `restart` shape; per-tenant timeout
   overrides; per-region weighting; a schedule swap). When the change
   lands, every persisted derived value is still bound to the OLD
   logic until it ages out. The change appears to apply at the
   configuration boundary but is silently ignored at the read site —
   exactly the inverse of what the operator expects. Recovery requires
   a column migration (re-deriving the missing inputs from a value
   that has already lost them) or an explicit drain (waiting until
   every persisted derived value expires).
2. **Schema migrations multiply.** Every change to the producing
   function becomes a change to the persisted schema, because the
   persisted value embedded the old function's output. Persisting
   inputs decouples the schema from the function — `(attempts,
   last_failure)` is stable across every backoff schedule that
   consumes those two fields.
3. **Audit and replay degrade.** Persisted inputs let an investigation
   re-derive a past decision against any candidate policy. Persisted
   outputs collapse the input → output trajectory into the output
   alone; the original decision cannot be re-explained without
   reconstructing the lost inputs.

### Examples

**Reconciler memory (libSQL).** A reconciler View field carries a
recompute-on-read deadline by storing the inputs the deadline depends
on, not the deadline itself:

```rust
// Bad — persists the deadline (a derived value bound to today's policy)
pub struct JobLifecycleView {
    pub restart_counts:   BTreeMap<AllocationId, u32>,
    pub next_attempt_at:  BTreeMap<AllocationId, UnixInstant>,  // derived
}

// Good — persists the inputs; deadline recomputed on every read
pub struct JobLifecycleView {
    pub restart_counts:        BTreeMap<AllocationId, u32>,         // input
    pub last_failure_seen_at:  BTreeMap<AllocationId, UnixInstant>, // input
}
// reconcile() body computes `*seen_at + backoff_for_attempt(*count)`
// every tick — picks up backoff-policy changes for free.
```

**Observation rows.** An observation row carries the field the
authoritative writer observed, not a derived classification of it.
`alloc_status.state` is an observed input; a `health_grade ∈ {green,
amber, red}` column would be derived from `state` + a threshold table
and must not be persisted — compute it at read time.

**Cache rows.** When a value is genuinely expensive to recompute (the
narrow exception below), the cache row carries an *invalidation key*
derived from the inputs and the policy identity, so a policy change
automatically invalidates the cache. A cache without such a key
violates this rule and must not be persisted.

**Audit / forensic rows.** Audit rows record the *observed inputs and
the decision that was made*, not a re-derived view of why. The
decision is the input ("we restarted alloc X at time T after attempt
N"); a "would have been retried again at T+5s" prediction is a
derived value and belongs in a viewer that recomputes from the audit
row, not in the audit row itself.

**Operator-facing config files / UI state.** A schematic file
(§Image Factory, whitepaper §23) is hashed by content. The hash is the
ID of the schematic. Persisting `schematic_id_v1: "abc..."` AND the
canonicalised schematic body is fine because the ID is *defined as* a
function of the body — they are the same datum, encoded twice for
different purposes. But persisting "this schematic produces a 247 MB
image" alongside the schematic is a derived cache; the next change to
the build system makes the number a lie.

### Codebase precedent

`crates/overdrive-control-plane/src/worker/exit_observer.rs:291`:

```rust
let backoff = RETRY_BACKOFFS.get((attempts - 1) as usize).copied();
```

The persisted input is `attempts: u32`. The policy is the indexed
`RETRY_BACKOFFS` table. Swap the table — the next attempt picks up the
new schedule with no migration, no drain, no inconsistency window.

### When NOT to apply

The rule has one narrow exception: when recomputing the derived value
is genuinely intractable on the read hot path (an expensive ML
inference, a cross-cluster correlation that costs a network
round-trip, a cryptographic operation measured in seconds). That is a
*cache*, not the primary value — and it MUST carry an invalidation
key derived from the inputs and the policy identity. If the cache
cannot carry such a key (because the policy is opaque), it must not
be persisted; recompute on every read or move the work into a workflow
that produces the value as observable state.

If you find yourself reaching for this exception for an in-process
arithmetic computation (a duration addition, a comparison, a small
table lookup, a string format), you are mistaken about the cost —
the recomputation is free. Persist the inputs.

### Symptoms during review

- A persisted field whose name describes a *future event*
  (`next_attempt_at`, `expires_at`, `scheduled_for`, `valid_until`,
  `next_run_after`, `eligible_at`). These are almost always derived
  from a "last X" timestamp + a policy.
- A persisted field whose value would change if a constant elsewhere
  in the codebase were edited. The constant IS the policy; the field
  is the cached output of applying it.
- A read site that uses a persisted field directly in a comparison or
  branch without consulting a policy table or function. The
  silhouette `if now < persisted.deadline` is the canonical smell —
  the deadline IS the cache; replace with `if now < persisted.seen_at
  + policy.backoff()`.
- A persisted field whose docstring contains the word "computed"
  or "derived" or "calculated."

If the field genuinely does NOT depend on a policy (a remote system's
externally-assigned identifier, a user-supplied free-text comment, a
fingerprint of immutable content), it is an input by construction and
the rule does not apply.

---

## Port-trait dependencies — `overdrive-host` is production, `overdrive-sim` is tests

`overdrive-core` declares the port traits (`Clock`, `Transport`, `Entropy`,
`Dataplane`, `Driver`, `IntentStore`, `ObservationStore`, `Llm`); two
sibling crates implement them.

| Crate | Class | Use | Cargo.toml placement |
|---|---|---|---|
| `overdrive-host` | `adapter-host` | Production bindings (`SystemClock`, `OsEntropy`, `TcpTransport`, …) | `[dependencies]` of crates that ship a production wiring |
| `overdrive-sim` | `adapter-sim` | Simulation bindings (`SimClock`, `SimTransport`, `SimEntropy`, `SimDriver`, `SimObservationStore`, `SimLlm`) | `[dev-dependencies]` of any crate whose tests need DST controllability |

**Rules:**

- **Never put `overdrive-host` in `[dev-dependencies]`** — it is the
  production binding crate. Tests do not run production wiring; they
  inject sim adapters. If a test reaches for `SystemClock` /
  `OsEntropy` / `TcpTransport`, the test is wrong, not the dep
  placement: replace the production binding with its sim counterpart.
- **Never put `overdrive-sim` in `[dependencies]`** — it carries
  `turmoil`, `StdRng`, and other DST machinery that must not be
  reachable from production binaries. The dst-lint gate scans for
  this.
- **Required, not defaulted, at the call site.** Types that depend on
  a port trait take the implementation as an explicit constructor
  parameter (`fn new(clock: Arc<dyn Clock>, ...)` or similar). Never
  default the field to a production binding inside the constructor —
  that silently inherits wall-clock / OS-entropy / real-network
  behaviour into tests that forgot to override, which is the exact
  failure mode the trait surface exists to prevent.
- **Builder-pattern overrides (`with_clock`, `with_transport`) are an
  anti-pattern for these traits.** A builder makes the dependency
  optional — and "optional" means "tests can forget." Make the
  dependency mandatory in `new()`; tests pass `Arc::new(SimClock::new())`,
  production passes `Arc::new(SystemClock)`, the compiler enforces
  every call site is explicit.
- **Production wiring is composed at the binary boundary.** A library
  crate's tests compose sim adapters; the CLI / control-plane binary
  composes host adapters. The library crate itself does not pick
  sides — it depends only on the trait surface in `overdrive-core`.

The compile-time consequence is the load-bearing one: a test that
forgets to inject a clock fails to compile rather than silently
running on `SystemClock`. The dst-lint gate catches the residual
"clock leaked into a `core` compile path" cases; this rule is the
upstream prevention.

---

## Production code is not shaped by simulation

**Production code MUST NOT carry extra logic, extra arms, extra yields,
extra polling, or extra structural concessions whose only purpose is to
make a `Sim*` adapter behave correctly under DST.** The port-trait
contract is the boundary: production wires `Host*`, tests wire `Sim*`,
and production code is written to the contract, not to the test
double's quirks. If a production call site needs a workaround to make
`Sim*` work, the bug is in the `Sim*` adapter, not in the production
code.

This is the rule that the `Clock` / `Transport` / `Entropy` / `Driver`
trait surface exists to enforce. A production `tokio::select!` arm that
exists "so DST can advance time" is the canonical violation — it
means the sim adapter has imposed a shape on the production hot path
that the production hot path would not otherwise need. The cost is
real (busy-loop CPU, redundant polls, defensive yields, wasted resource
budget) and it compounds: every future call site copies the workaround
because "that's how we use the trait here."

### Symptoms

- A `tokio::select!` in production whose only resolving arm is
  `tokio::task::yield_now()` or `clock.sleep(Duration::from_millis(1))`
  with a comment explaining "so the deadline check at the top can fire
  under SimClock."
- A production loop with a `clock.unix_now() >= deadline` check at the
  top *and* a separate timer arm in the select, where the comment
  explains the deadline check is for DST and the timer arm is for
  production.
- A production function that takes a `_dst_yield_hook: Option<...>`
  parameter, or any other signature contortion whose only caller is a
  test.
- A docstring that says some piece of production code is "redundant
  but harmless" in production and exists "to play nicely with SimClock."
- A `// dst:` or `// sim:` comment on a production line of code
  explaining behavior that would not be there absent the test double.

If you find yourself writing the comment, stop. The comment is the
admission that the rule is being broken.

### What to do instead

Fix the `Sim*` adapter to model the real-world primitive faithfully,
then write production against the trait shape that production
genuinely needs. Concretely, if a `Sim*` `sleep` / `recv` / `connect`
forces a workaround in production, the adapter is doing the wrong
thing — usually conflating two concerns (e.g. "yield" and "advance
time"), or auto-progressing state that the harness should drive
explicitly.

Reference shape: `SimClock::sleep` should *park on a deadline* (register
a waker; the harness's `tick()` wakes timers whose deadline has passed),
not auto-advance time as a side effect of being polled. Every credible
DST framework (turmoil, madsim, FoundationDB flow sim) implements
`sleep` this way; only the harness drives logical time.

### Audit checklist for any sim-adapter change

Before landing a `Sim*` change that requires modifying a production
call site, answer:

1. Could the sim adapter be reshaped so the production call site stays
   unchanged? If yes — do that instead.
2. Does the production change degrade production behavior in any way
   (extra CPU, extra latency, extra polls, defensive resource use)? If
   yes — the change is rejected; the sim adapter must be reshaped.
3. Would a future engineer reading the production code, with no
   knowledge of the sim adapter, understand why the code is shaped this
   way? If no — the shape is wrong, even if a comment papers over it.

The boundary between production and simulation is load-bearing. Erode
it once and every future call site inherits the erosion.

---

## Ordered-collection choice

`core` and control-plane hot paths default to `BTreeMap` for keyed maps
whose iteration order is observed (drain, snapshot, JSON output,
invariant evaluation). `HashMap` is a first-class nondeterminism source
on the same footing as `Clock` / `Transport` / `Entropy` and must be
treated with the same discipline.

| Iteration shape | Choice | Notes |
|---|---|---|
| Drained / iterated / snapshotted | `BTreeMap<K, V>` | Default. Order is `Ord` on `K` — deterministic across processes, runs, seeds. |
| Serialised (JSON, rkyv archived field, audit log) | `BTreeMap<K, V>` | Output bytes must be canonical for content hashing and trace-equivalence DST assertions. |
| Walked by an invariant or property test | `BTreeMap<K, V>` | Reproduction requires bit-identical traversal under the seed. |
| Point-accessed only (`get` / `insert` / `remove`, never iterated) | `HashMap<K, V>` with `// dst-lint: hashmap-ok <reason>` | Allowed in `core` only with the justification comment. |

The escape hatch — `HashMap` in a `core`-class crate — requires an
explicit `// dst-lint: hashmap-ok <one-line reason>` comment on (or
immediately above) the use site. Without the comment the dst-lint gate
rejects the file at PR time. The comment is the load-bearing artifact:
it documents *why* iteration nondeterminism cannot surface here, and a
reviewer who disagrees with the reason has a single line to push back
on.

### Why

`std::collections::HashMap`'s default `RandomState` is per-process
random-seeded — two seeded DST runs produce divergent dispatch
orderings the moment ≥2 distinct keys are held. That violates the K3
*seed → bit-identical trajectory* property documented in whitepaper §21
and `.claude/rules/testing.md` § "Sources of Nondeterminism": every
source of nondeterminism in core logic must be injectable, and `RandomState`
is the one the type system silently smuggles past every other gate.

The defect was discovered in
`crates/overdrive-control-plane/src/eval_broker.rs`, where
`EvaluationBroker::drain_pending` returned evaluations in
non-deterministic order. The fix landed as a `BTreeMap` swap in commit
`8cf9119` (`fix(eval-broker): switch pending map to BTreeMap for
deterministic drain order`). The structural rule prevents the class
from recurring; the dst-lint clause enforces it.

### When NOT to apply

Bounded-cardinality maps that are NEVER iterated — point-accessed only
via `get` / `contains_key` / `insert` / `remove`, with no observable
drop order or iteration call site — MAY use `HashMap` with the
justification comment, since the iteration nondeterminism never
surfaces. Examples: per-allocation handle caches keyed by
`AllocationId` where the cache is consulted only by point lookup and
never enumerated, or per-request memo tables whose lifetime ends before
any reduction over their entries.

If you find yourself reaching for the escape hatch and the cardinality
is small (say, <16), prefer `BTreeMap` anyway — the constant-factor
cost is in the noise and the `// dst-lint: hashmap-ok` comment is
upkeep that future contributors must justify.

### Marker comment syntax

The dst-lint scanner accepts exactly one escape form. Other shapes —
`#[allow(dst_lint::hashmap)]` attributes, `// SAFETY:`-style prose,
crate-level `#![allow(...)]` — are NOT recognised; the scanner will
still reject the file.

**Form**:

```
// dst-lint: hashmap-ok <one-line reason>
```

- The literal prefix is `// dst-lint: hashmap-ok` (single space after
  the colon, single space before `hashmap-ok`). Casing matters — the
  scanner is case-sensitive on the marker tokens.
- A one-line reason is **required** in human-readable code review
  contexts even though the scanner does not enforce reason text. A
  marker without a reason will be rejected at code review time, not at
  lint time. Put the *why* on the line; the *what* is obvious from the
  next line of source.
- **Placement**: on the line **immediately above** the use site, OR as
  a trailing comment **on the same line** as the use site. Both are
  recognised:

  ```rust
  // dst-lint: hashmap-ok per-allocation handle cache, point access only
  let cache: HashMap<AllocationId, Handle> = HashMap::new();
  ```

  ```rust
  let cache: HashMap<AllocationId, Handle> = HashMap::new(); // dst-lint: hashmap-ok per-allocation handle cache, point access only
  ```

- The marker suppresses violations on the marked line only. Multiple
  use sites in the same function each need their own marker. Do not
  put one marker at the top of a function and expect it to cover the
  whole body.
- The marker covers `HashMap` and `HashSet` together — a single
  `// dst-lint: hashmap-ok` suppresses both type families on the
  marked line. There is no separate `hashset-ok` form.

**What the marker does not cover**:

- `std::collections::hash_map::HashMap` — the scanner walks
  `TypePath` and `ExprPath` and catches the type by last segment
  regardless of qualifying path; the marker still applies.
- `BuildHasherDefault<...>` / `RandomState` / custom hashers — these
  are different concerns and require their own justification at the
  use site (typically a `// SAFETY:`-style prose comment, since they
  do not flow through dst-lint).
- Type aliases (`type Cache = HashMap<...>;`) — the alias declaration
  IS the use site; the marker goes on the alias line. Subsequent
  references to the alias type do not need their own markers (the
  alias's marker is the load-bearing artifact).

**Why the marker has to be precise**: ad-hoc patterns (`// hashmap-ok`,
`// dst: hashmap-ok`, `// allow hashmap`) would silently slip past the
scanner without a clear failure mode. The strict syntax is the
trade-off for catching the rule's enforcement gap mechanically.

### Sim-internal exception

`adapter-sim` and `adapter-host` crates are NOT scanned by the dst-lint
clause (only `core` is), but the principle applies as guidance —
multi-step DST harnesses that observe iteration order should still
prefer `BTreeMap`. The precedent at
`crates/overdrive-sim/src/adapters/observation_store.rs:215-218`
documents this choice for `BTreeMap<AllocationId, AllocStatusRow>` with
the same rationale: when the harness asserts on the row stream, the
stream must be deterministic across seeds.

### Cross-references

- Whitepaper §21 — *Deterministic Simulation Testing*; the K3
  reproducibility property (seed → bit-identical trajectory).
- `.claude/rules/testing.md` § *Sources of Nondeterminism* — every
  nondeterminism source must be injectable behind a trait; `HashMap`
  iteration order is the one the trait surface cannot intercept.

---

## Reconciler I/O

**`reconcile` does not perform I/O.** The §18 contract splits the
reconciler into two methods: async `hydrate(target, &LibsqlHandle) ->
Result<Self::View, HydrateError>` and sync pure
`reconcile(desired, actual, &view, &tick) -> (Vec<Action>, NextView)`.
All libSQL access lives exclusively in `hydrate`. No `.await` inside
`reconcile`; no network, no subprocess spawn, no direct libSQL /
IntentStore / ObservationStore write anywhere in `reconcile`.
Wall-clock reads come from `tick.now` (a field on the
`TickContext` parameter the runtime constructs once per evaluation),
never `Instant::now()` / `SystemTime::now()`. This is what makes DST
(§21) and ESR verification (§18) possible; it is not optional.

When a reconciler needs to talk to an external service (a Restate admin
API, an AWS account, a webhook, a custom internal service), the shape is:

```rust
// Bad — violates §18 purity; no DST support; no ESR reasoning
async fn reconcile(&self, /* ... */) -> Vec<Action> {
    let resp = self.http.post("https://svc/register").await?;
    // ...
}

// Good — hydrate reads retry memory from libsql (the ONLY place this
// reconciler author touches libsql); reconcile is sync + pure, emits
// an HttpCall action, and returns a NextView the runtime persists.
// The response arrives as observation on the next tick via `actual`.
// Wall-clock comes from `tick.now`, never `Instant::now()`.
impl Reconciler for RegisterReconciler {
    type View = RetryMemory;

    fn name(&self) -> &ReconcilerName { &self.name }

    async fn hydrate(
        &self,
        target: &TargetResource,
        db:     &LibsqlHandle,
    ) -> Result<Self::View, HydrateError> {
        // Free-form SQL in hydrate. Schema management
        // (CREATE TABLE IF NOT EXISTS, ALTER TABLE ADD COLUMN) lives
        // here too — no framework migrations Phase 1.
        let row = db.query_one(
            "SELECT attempts, last_correlation, next_attempt_at \
             FROM register_memory WHERE target = ?",
            &[target.as_str()],
        ).await?;
        Ok(RetryMemory::from_row(row))
    }

    fn reconcile(
        &self,
        desired: &State,
        actual:  &State,
        view:    &Self::View,
        tick:    &TickContext,
    ) -> (Vec<Action>, Self::View) {
        let correlation = CorrelationKey::from((desired.id, desired.spec_hash, "register"));
        let actions = match actual.external_call(&correlation).latest_status() {
            None => vec![Action::HttpCall {
                correlation: correlation.clone(),
                target: desired.endpoint.clone(),
                method: Method::POST,
                body: build_register_payload(desired),
                timeout: Duration::from_secs(30),
                idempotency_key: Some(correlation.to_string()),
            }],
            Some(Status::Pending) | Some(Status::InFlight) => vec![],
            Some(Status::Completed { response }) => converge_from_response(actual, response),
            // Retry-budget gate: only re-dispatch once the backoff
            // window has elapsed. `tick.now` is the runtime's
            // single-snapshot of wall-clock for this evaluation —
            // pure input, DST-controllable.
            Some(Status::Failed { .. } | Status::TimedOut { .. })
                if tick.now >= view.next_attempt_at =>
            {
                handle_failure(view)
            }
            Some(Status::Failed { .. } | Status::TimedOut { .. }) => vec![],
        };
        // NextView carries the updated retry memory; the runtime
        // diffs (view → next_view) and persists the delta to libsql.
        // Reconcile never writes libsql directly. Compute the next
        // backoff deadline from `tick.now`, not from `Instant::now()`.
        let next_view = view.bump_if_dispatched(&actions, tick.now);
        (actions, next_view)
    }
}
```

Rules:

1. **Every external call carries an `idempotency_key`** when the remote
   API supports one. The runtime executes `HttpCall` at-least-once;
   idempotency on the remote side is what makes the effect
   exactly-once.
2. **Correlation, not request ID, links cause to response.** A
   `CorrelationKey` newtype derived from
   `(reconciliation_target, spec_hash, purpose)` lets the next reconcile
   find the prior response deterministically. Do not embed the
   `request_id` in reconcile logic — it changes per attempt; the
   correlation does not.
3. **Retry budgets live in reconciler libSQL.** The runtime does not
   auto-retry a failed `HttpCall` — that policy belongs to the
   reconciler. Track attempts in the private DB via `hydrate` reads
   and `NextView` writes; emit a new `HttpCall` action until the
   budget is exhausted; then surface the failure to status.
4. **Multi-step external sequences become workflows, not chains of
   `HttpCall`s.** If the reconciler would need to coordinate three or
   more external calls that must complete as a unit, emit
   `Action::StartWorkflow` and read the workflow's result on completion.
   Reconcilers converge; workflows orchestrate.
5. **`HttpCall` responses are observation, not intent.** The
   `external_call_results` table lives in the ObservationStore and is
   gossiped like any other observation row. Reconcilers read it
   locally, same as `alloc_status` or `service_backends`.
6. **Reading wall-clock: use `tick.now` from the `TickContext`
   parameter.** Never call `Instant::now()` / `SystemTime::now()`
   inside `reconcile`. The dst-lint gate catches violations at PR
   time. Time is input state, injected by the runtime — the same
   `Clock` trait DST already controls (`SystemClock` in production,
   `SimClock` under simulation). The `tick.now` snapshot is taken
   once per evaluation; every `reconcile` call sees one consistent
   "now," which is what makes the function pure over its inputs.
   `tick.deadline` is the per-tick budget (consult to checkpoint
   bounded work into `NextView`); `tick.tick` is a monotonic counter
   useful as a deterministic tie-breaker.

The reference case: a Restate-operator-equivalent in Overdrive is a
reconciler whose `hydrate` reads retry memory, whose `reconcile` emits
`HttpCall { target: restate_admin_url, method: POST, body:
deployment_spec, idempotency_key: Some(...) }` on registration when
`tick.now >= view.next_attempt_at`, and which reads
`external_call_results` on the next tick to advance its state
machine. No `async fn` in `reconcile`; no direct HTTP client; no
direct wall-clock read; fully DST-replayable.

---

## Workflow contract

Workflows are the §18 peer primitive to reconcilers. The rules are
different from reconcilers — and different in specific ways that matter.

**`async` is permitted in workflows. Only in workflows.** Anywhere else
in the codebase — reconcilers, policies, sidecars — `async fn` that
performs I/O is a violation. Workflow handlers are the one place where
`.await` on real work is the *correct* shape.

```rust
trait Workflow: Send + Sync {
    async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult;
}

// Good — all non-determinism flows through ctx; durable at each await
#[overdrive::workflow]
async fn cert_rotation(ctx: &WorkflowCtx, spec: CertSpec) -> Result<Cert> {
    let challenge = ctx.call("acme", acme::new_order(&spec)).await?;
    ctx.sleep(Duration::from_secs(spec.dns_propagation_seconds)).await;
    let validated = ctx.call("acme", acme::validate(&challenge)).await?;
    ctx.signal_all("workflow.cert_ready", validated.clone()).await?;
    Ok(validated)
}
```

Rules:

1. **All non-determinism goes through `ctx`.** Clock, network, RNG,
   signals, inter-workflow coordination — every source of non-determinism
   is a method on `WorkflowCtx`, which consumes the same injected
   `Clock` / `Transport` / `Entropy` traits the rest of the platform uses
   under DST. No `Instant::now()`, no `reqwest::get()`, no
   `tokio::time::sleep`, no `rand::random()` inside a workflow body.
2. **No side effects outside `ctx`.** Writing to a file, calling a
   non-`ctx` async function, spawning a task, mutating a global — these
   break journal replay. The SDK lints them; the runtime rejects
   workflows whose call graph references forbidden hosts.
3. **Journal replay is bit-identical.** A workflow run twice against the
   same journal must produce the same trajectory. The DST harness asserts
   this as `assert_replay_equivalent!` (§21); treat replay-equivalence
   failures as logic bugs, not flakes.
4. **Workflow versioning is additive.** A running workflow may have
   arbitrary in-flight instances. Changing the `run` body in a way that
   would deviate from an existing journal is a *breaking* change and the
   SDK rejects it at load time. Add new versions (`cert_rotation_v2`)
   alongside the old; migrate in-flight instances explicitly via
   `ctx.upgrade_to(...)` or let them drain.
5. **Bounded step budget.** Every workflow declares a maximum number of
   `await` points. Unbounded loops inside a workflow are a bug; if you
   genuinely need indefinite lifecycle, use a reconciler, not a workflow.
   The reconciler primitive exists exactly for "runs forever."
6. **Workflow → cluster mutations go through Actions.** Workflows write
   intent the same way reconcilers do: by emitting typed Actions that
   the runtime commits through Raft. Workflows do not bypass Raft to
   write IntentStore directly — `ctx` does not expose a `.put()` surface
   on IntentStore; it exposes `ctx.emit_action(...)`.

**When to reach for a workflow vs a reconciler:**

```
Runs forever; converges desired vs actual?           → Reconciler
Terminates with a Result; orchestrates a sequence?   → Workflow

"Keep N replicas running"                            → Reconciler
"Roll the certificate through 4 steps"               → Workflow
"Maintain BPF map == policy verdict"                 → Reconciler
"Migrate allocation X from region A to region B"     → Workflow
"Reach desired replica count from queue depth"       → Reconciler (rule-based)
                                                       OR Workflow (predictive,
                                                       multi-step rollout)
```

When in doubt: if the operation has a natural terminal `Ok(result)`,
it's a workflow. If it's "the cluster should always look like X," it's
a reconciler.

---

## Compile-checking

> **Use `cargo check`, not `cargo build`.** `cargo build` is **blocked** by
> a pre-tool hook (`.claude/hooks/block-cargo-build.ts`). For iterative
> typecheck-and-diagnose loops — the overwhelming majority of what the
> agent actually does — `cargo check` skips codegen and linking and is
> dramatically faster on this workspace.
>
> **Rewrite your command before submitting it:**
>
> | ❌ don't | ✅ do |
> |---|---|
> | `cargo build` | `cargo check` |
> | `cargo build -p CRATE` | `cargo check -p CRATE` |
> | `cargo build --workspace` | `cargo check --workspace` |
> | `cargo build --all-targets` | `cargo check --all-targets` |
> | `cargo build --features X` | `cargo check --features X` |
> | `cargo build --release` | `cargo check --release` |
>
> `cargo check` catches every `rustc` diagnostic `cargo build` would —
> trait resolution, borrow checker, type inference, macro expansion,
> lints via `cargo clippy`. It does NOT produce a binary or run
> `build.rs` link steps.
>
> **Legitimate `cargo build` shapes** (the hook allows these):
> - Producing a binary for a real execution target — xtask, CLI, a
>   real-kernel integration test that must boot an artifact, anything
>   about to be `exec`'d.
> - `cargo xtask ...` subcommands that internally invoke `cargo build`
>   as part of their own compilation pipeline.
> - Tier 3 / Tier 4 harness flows where the build artifact is the
>   point (`cargo xtask integration-test vm`, `cargo xtask xdp-perf`).
>
> If you find yourself needing `cargo build` for "just to see the
> errors," you want `cargo check`. If you need the binary, reach for
> the xtask wrapper that already knows how to produce it.

## Committing a focused subset

The lefthook pre-commit pipeline auto-stages modified files into the
in-flight commit (it runs `cargo fmt`, clippy, and nextest-affected
across the working tree, and re-stages whatever they touch). When the
working tree carries unrelated modifications and you only want to
commit a focused subset, `git add <one-file>` is **not** sufficient —
the hook will silently bundle every other modified file into your
commit on its way through. The previous commit you just landed will
contain changes you never staged.

**Pattern: stash the unrelated paths first, then commit.**

```bash
git stash push -m "unrelated-{summary}" -- \
  <path-1> <path-2> ... <path-N>     && \
git add <path-you-actually-want>     && \
git commit -m "..."                  ; \
git stash pop
```

Three things to note:

- **`;` before `git stash pop`, not `&&`.** The stash must be restored
  even if the commit fails (e.g. a pre-commit gate rejects it),
  otherwise the unrelated work is left only in the stash and the
  working tree appears empty. The `git stash` pre-tool hook in this
  repo enforces the matched-pair shape — see the message it prints
  when blocking.
- **Path-scoped stash, not `git stash -u`.** Stashing the whole tree
  would also stash the file you want to commit; pass paths to `git
  stash push -- <paths>` so only the unrelated changes move.
- **Verify with `git show --stat HEAD` after the commit.** "1 file
  changed" should match what you intended to land. If the commit
  bundles more, the stash scope was wrong — soft-reset and retry.

This pattern is the only safe shape for landing a focused fix when
the working tree is dirty with parallel work. Do not reach for
`--no-verify` to bypass the lefthook auto-staging — the hook is also
running clippy and tests, and skipping it lands unverified code.

**Always-include path: `.nwave/des-config.json`.** When this file shows
as modified, untracked, or otherwise affected in `git status`, it MUST
be staged into the current commit — never stashed under the
focused-subset pattern, never deferred to a follow-up commit. The file
captures the active nWave rigor profile (lean / standard / thorough /
exhaustive / custom / inherit) and is the SSOT for how subsequent wave
runs in this repo behave; leaving it out of a commit that touched it
silently desyncs the committed profile from the one the next agent
will read. If the commit you are landing is otherwise unrelated, add
`.nwave/des-config.json` to it anyway — it is the one file exempt from
the "focused subset" discipline above.

## Deletion discipline

When production code becomes unused — typically after a refactor that
collapses or replaces a subsystem — **delete the production code AND
its tests in the same commit**. Do not gate, annotate, salvage, or
relocate.

Specifically, when you see a `dead_code` warning (or `unused_imports`,
`unused_variables`) after a deletion pass, the warning is the signal
that **more code needs deleting**, not that the existing code needs a
gate or an allow. The wrong moves:

- `#[cfg(test)]` on a helper that's now only called from tests.
- Moving a helper into `mod tests { … }` to keep the same effect.
- `#[allow(dead_code)]` to silence the warning.
- "Rewriting" the existing tests to test something else so they keep
  earning their keep.

The right move is a single commit that removes the production code
*and* every test that was defending it. A test exists to defend
production code; if the production code is gone, the test is gone too.
You cannot defend something that doesn't exist, and preserving the
test by repurposing it just hides the deletion in the git log — a
future reviewer reading the test name expects it to be telling them
something about a function whose name no longer resolves.

A genuinely new requirement that needs a genuinely new test (e.g. a
convention to enforce after a sweeping deletion) is a separate matter
— write it from scratch, with a name and assertions that describe
the new requirement. Don't pretend the salvaged-and-rewritten old
test was "already" testing the new thing.

The deleted code does not get a stub, a deprecation comment, a
`// removed in PR #N` marker, or a re-export shim. None of these
forms exist in this codebase; per CLAUDE.md and
`feedback_single_cut_greenfield_migrations.md`, removed is removed.

The corollary: **a test file shrinking is the correct shape of a
deletion PR**. If your "deletion" PR adds tests on net, double-check
you actually deleted what you set out to delete.

## Rust patterns

### Errors

- **Use `thiserror` for typed errors** in all library / core crates. Typed
  errors provide structured data for audit trails, reconciler retry logic,
  and investigation-agent tool outputs.
- **Use `eyre` only at CLI / binary boundaries** for user-facing messages.
  `eyre` is a fork of `anyhow` with pluggable report handlers — pair it
  with `color-eyre` in binaries to get backtraces, `tracing-error`
  spantraces, and `Help` suggestions in one formatted report. Prefer
  `eyre::Result<T>` over `anyhow::Result<T>` in new code; do not mix the
  two in one crate.
- **Library code never returns `eyre::Report` (or `anyhow::Error`).** The
  caller loses the ability to branch on variant, and re-exporting a
  `Report` as part of a public API ties your SemVer to eyre's — an
  `eyre` major bump in a downstream app becomes a breaking change you
  cannot control. Return a `thiserror` enum; let the binary convert at
  the boundary via `?` (`eyre::Report: From<E>` for any `E: Error`).
- **`wrap_err` / `wrap_err_with` for context**, not `Display` string
  concatenation. The returned `Report` preserves the full error chain;
  `color-eyre`'s formatter renders it as `Caused by:` sections. Do not
  use `.map_err(|e| format!("...: {e}"))` — it collapses the chain to a
  string and breaks downcasting.
- **`eyre!` and `bail!` for one-off errors** at the boundary only.
  Inside a library, construct the typed variant.
- **Consistent constructors.** Every error enum variant should have an
  associated constructor method (`Error::validation(...)`,
  `Error::internal(...)`, `Error::not_found(...)`). Call sites read as
  English; variant shape can evolve without a breaking grep.
- **Pass-through embedding, not duplication.** When a higher-level error
  wraps a lower-level error, embed via `#[from]` rather than redefining
  variants. Preserves the full nested structure (and its queryable
  fields) through audit logs and investigation outputs.

  ```rust
  // Bad — duplicates lower variants; manual From impls; loses fields
  pub enum ReconcilerError {
      IntentPutFailed { source: DbError },
      DriverStartFailed { message: String, alloc_id: Option<String> },
  }

  // Good — pass-through via #[from]; nested structure preserved
  pub enum ReconcilerError {
      Validation { message: String, field: Option<String> },
      Intent { #[from] source: IntentStoreError },
      Driver { #[from] source: DriverError },
  }
  ```

  Use service-specific variants for local concerns (validation, business
  logic); use pass-through for errors from lower layers that need no
  transformation.

- **Distinct failure modes get distinct error variants. Never silently
  absorb a `Result<_, io::Error>` (or any other fallible boundary read)
  into a default value.** Using `.unwrap_or_default()`, `.ok()`, or
  `.unwrap_or(_)` on a boundary I/O / parse / env read collapses every
  distinguishable failure (PermissionDenied, EIO, broken procfs, missing
  mount, malformed input, ...) into the same neutral value. The next
  downstream check then misdiagnoses the cause and prescribes the wrong
  remediation — and the cost is paid by the operator, who follows
  guidance that does not fix the actual problem. "Boot a newer kernel"
  does not repair a permissions error on `/proc/filesystems`; "the JSON
  field is missing" is not the same diagnosis as "the JSON file is
  unparseable." **Default to propagation**: `.map_err(...)?` into a
  discrete typed variant whose `Display` form names the actual cause
  and the actual fix. Absorbing a specific `ErrorKind` into a default
  is allowed only when the application semantics legitimately treat
  that kind the same as the default — `NotFound` on `/proc/filesystems`
  IS the cgroup-v1-host signal, but `PermissionDenied` is not, even
  when the downstream check happens to fire in both cases.

  ```rust
  // Bad — every io::Error becomes the empty string, which then
  // triggers NoCgroupV2 with a "boot a newer kernel" remediation
  // regardless of the actual cause (permission denied, EIO, broken
  // procfs, /proc unmounted).
  let proc_fs = std::fs::read_to_string(proc_filesystems).unwrap_or_default();
  if !proc_fs.lines().any(|l| l.contains("cgroup2")) {
      return Err(NoCgroupV2 { kernel: uname_release() });
  }

  // Good — NotFound flows to the v1-host signal because that IS the
  // application semantics; every other ErrorKind surfaces as its own
  // discrete variant with its own Display message and its own
  // remediation.
  let proc_fs = match std::fs::read_to_string(proc_filesystems) {
      Ok(s) => s,
      Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
      Err(err) => return Err(ProcFilesystemsUnreadable { source: err }),
  };
  ```

  Symptom to watch for during review: an error variant whose docstring
  describes one failure mode but whose *triggering code path* fires for
  several unrelated reasons. That is the smell — a variant has become a
  catch-all for everything not explicitly handled, and operators
  downstream receive the wrong remediation. The structural fix is
  always the same: split the catch-all into discrete variants, propagate
  the originating error via `.map_err(...)?`, and let `Display` carry
  the cause-specific guidance.

### Concurrency & async

- **Tokio is the standard runtime.** Do not reach for `async-std`,
  `smol`, or a hand-rolled executor. Consistency matters more than
  marginal performance differences.
- **`Send + Sync` on core data structures.** Shared long-lived state
  (engines, caches, pools) must be safely sendable across threads. If a
  type is not `Send + Sync`, justify it in a comment.
- **Cancellation safety.** Async tasks must tolerate being cancelled at
  any `.await` point. A task holding a partially-applied mutation
  across an `.await` is a bug.
- **Never hold a lock across `.await`.** Grab the lock, mutate or clone,
  drop the guard, then `await`. Holding `parking_lot` across `.await` is
  a deadlock waiting for an unfair scheduler tick; holding `tokio::sync`
  across `.await` is a latency spike waiting to happen.
- **Use `parking_lot::RwLock` / `Mutex`** over `std::sync::RwLock` /
  `Mutex` for synchronous critical sections. Avoids lock poisoning,
  faster uncontended path, smaller. Use `tokio::sync::RwLock` / `Mutex`
  only when the critical section *must* cross `.await` — and per the
  rule above, try hard not to need that.
- **`.expect()` in CLI binaries.** In `main()` and CLI entry points, use
  `.expect("description")` instead of verbose `match` / `unwrap_or_else`
  + `process::exit()` patterns. `expect` already prints and panics;
  wrapping `process::exit` around fallible constructors adds noise with
  no benefit.
- **No blocking `std::fs::*` inside `async fn`.** Filesystem I/O inside
  an `async fn` body in an `adapter-host`-class crate goes through
  `tokio::fs::*` (preferred — same syscall surface, async API) or
  `tokio::task::spawn_blocking` (escape hatch — the sync closure runs
  on the blocking pool). Sync `std::fs::*` blocks the tokio worker
  thread and stalls every other future scheduled on it until the
  syscall returns. The dst-lint gate enforces this at PR time:
  `xtask/src/dst_lint.rs::scan_source_async_fs` walks every `async fn`
  body (plus `async {}` blocks and `async fn` inside `#[async_trait]`
  impls) in `adapter-host` crate `src/` and flags any path under
  `std::fs::*`. Two exemptions:
  - **Sync helper fns** are allowed to use `std::fs::*` directly. The
    lint only fires when the *enclosing* fn / closure / async block
    is async — sync helpers called from an `async fn` are still a
    smell, but if you genuinely cannot make the helper async, wrap
    its call site in `tokio::task::spawn_blocking`.
  - **`#[cfg(test)]` items.** Tests may use sync `std::fs` for fixture
    setup without penalty. The lint detects `#[cfg(test)]` on modules
    and on individual fns and skips both.
  Note: `tokio::fs::*` itself dispatches each call onto the blocking
  pool internally — the *kernel* still does blocking I/O. The
  difference is that the `async fn` body is never the one blocked.

### Hashing requires deterministic serialization

When a hash is used as an identity, address, or integrity check (content
hashes in Garage, schematic IDs, Raft log digests, investigation-trace
reproducibility), the serialization that feeds the hash MUST be
deterministic.

- **Internal data → rkyv.** rkyv's archived bytes are canonical by
  construction. Hash the archived slice directly.
- **External / JSON data → RFC 8785 (JCS).** If a hash must be computed
  over JSON (interop requirement, external-facing audit log), use a JCS
  implementation — never `serde_json::to_string()`. `{"a":1,"b":2}` and
  `{"b":2,"a":1}` must produce the same hash; serde does not guarantee
  that.
- **TOML / YAML schematics → canonicalize, then hash.** Round-trip
  through a canonical form before SHA-256. The schematic ID is a content
  hash; non-deterministic input means non-deterministic ID.

```rust
// Bad — key ordering is not guaranteed; hash varies run to run
let digest = sha256(&serde_json::to_string(&record)?);

// Good — archived bytes are canonical
let archived = rkyv::to_bytes::<_, 256>(&record)?;
let digest = sha256(&archived);
```

### Dependencies

- **Workspace dependencies always.** Use `foo.workspace = true` in
  per-crate `Cargo.toml`; never hardcode versions in a leaf crate. Version
  drift across crates is a merge-conflict generator and an audit
  nightmare.
- **Use standard crates.** Don't roll custom base64 / hex / crypto / UUID
  / time formatting. Use `base64`, `hex`, `ring` / `aws-lc-rs`, `uuid`,
  `time` / `chrono` — whichever is already in the workspace graph.

### Cargo.toml conventions

- **Every workspace member declares `integration-tests = []`** in its
  `[features]` block, even crates with no integration tests of their
  own. The declaration is a no-op for the latter and the actual gate
  for the former. This makes `cargo {check,test,mutants} --features
  integration-tests` resolve uniformly under per-package scoping —
  cargo refuses the bare feature on packages that don't declare it,
  which historically broke mutation testing's per-mutant invocations.
  See `.claude/rules/testing.md` § "Integration vs unit gating" /
  "Workspace convention" for the full story; an xtask `#[test]`
  enforces the rule mechanically (`xtask::mutants::tests::every_
  workspace_member_declares_integration_tests_feature`).

### Newtypes — STRICT by default

Raw primitives (`String`, `&str`, `u64`, `i64`, `[u8; 32]`) for domain
concepts are blocking violations. All identifiers and domain-bearing
values MUST use newtypes from `overdrive-core`:

| Concept | Newtype |
|---|---|
| Workload identity | `SpiffeId` |
| Job | `JobId` |
| Allocation | `AllocationId` |
| Node | `NodeId` |
| Policy | `PolicyId` |
| Region | `Region` |
| Investigation | `InvestigationId` |
| Correlation | `CorrelationKey` |
| Image schematic | `SchematicId` |
| WASM module / chunk | `ContentHash` (SHA-256) |
| Certificate serial | `CertSerial` |

**Only exception** — an explicitly approved, issue-tracked deferral with
scope and exit criteria. Outside a tracked deferral, do not accept
"follow-up" language in review — the types exist, use them now.

**Symptom signals.** A new `normalize_spiffe_id()`, `normalize_node_id()`,
or similar helper is almost always a symptom of a missing newtype
constructor. If you find yourself writing one, the fix is to move the
normalization into the newtype's constructor, not to ship the helper.

### Newtype completeness

Every newtype must implement:

- `FromStr` — with validation; returns `Result<Self, ParseError>`.
- `Display` — the canonical string form.
- `Serialize` / `Deserialize` — matching `Display` / `FromStr` exactly.
- Constructors that **validate and return `Result`**. No infallible
  `new()` that silently accepts garbage.

**Case-insensitive parsing.** `FromStr` for identifiers that humans type
or paste — SPIFFE IDs, region codes, schematic IDs — must be
case-insensitive. The canonical form emitted by `Display` is lowercase.
SHA-256-style content hashes stay case-sensitive (they are not
human-typed).

### Documentation

- **Rustdoc `///` on every public item.** If the public API is not worth
  documenting, it probably should not be public.
- **Doctests for usage examples.** Examples in rustdoc fenced blocks run
  as tests — code that rots in a `README` is unverifiable; code in a
  doctest fails the build when the API drifts.
- **No aspirational docs.** Never document behaviour that is not
  implemented. An empty doc comment is strictly better than a lie.

### Import style

Import types directly. Do not use fully-qualified paths in function
signatures or struct fields.

```rust
// Bad
fn reconcile(id: &overdrive_core::SpiffeId) -> Result<(), Error> {
    let seen: HashSet<overdrive_core::SpiffeId> = HashSet::new();
    // ...
}

// Good
use overdrive_core::SpiffeId;

fn reconcile(id: &SpiffeId) -> Result<(), Error> {
    let seen: HashSet<SpiffeId> = HashSet::new();
    // ...
}
```

Exception: full paths only to disambiguate two types with the same name:

```rust
use overdrive_core::JobId;
use legacy::JobId as LegacyJobId;
```

---

## Why this matters

The reconcile loop runs constantly, per object, across thousands of objects.
Its allocator behaviour is one of the largest determinants of tail latency
and steady-state memory use. The goal is not a micro-optimization; it is a
predictable hot path:

- Arena allocation removes malloc/free overhead on the transient middle and
  eliminates heap fragmentation across iterations.
- Zero-copy deserialization removes the deserialization pass entirely for
  the durable inputs.
- Lifetime-bound references make "this data cannot escape this iteration" a
  compile-time guarantee, which is the only way the pattern survives across
  a team.

In a GC language this pattern is approximated with object pools and
discipline. In C++ it is arenas plus hope. Rust makes the invariants
mechanical.
