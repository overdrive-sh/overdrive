# ADR-0013 — Reconciler primitive: trait in `overdrive-core`, runtime in `overdrive-control-plane`, libSQL private memory, shipped whole

## Status

Accepted. 2026-04-23.

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

### 2. Trait shape

```rust
// in overdrive-core::reconciler
pub trait Reconciler: Send + Sync {
    /// Canonical name — used for libSQL path derivation and evaluation
    /// broker keying. Newtype-validated (see `ReconcilerName`).
    fn name(&self) -> &ReconcilerName;

    /// Pure function over (desired, actual, private-memory) →
    /// Vec<Action>. See whitepaper §18 and `.claude/rules/development.md`
    /// §Reconciler I/O.
    fn reconcile(
        &self,
        desired: &State,
        actual:  &State,
        db:      &Db,
    ) -> Vec<Action>;
}
```

No `async fn`. No `.await`. No `&dyn Clock` / `&dyn Transport` / `&dyn
Entropy` in the parameter list. Non-determinism is expressed through
`Action::HttpCall` (executed by the runtime shim, Phase 3) or by reading
observation rows (already passed in via `actual`).

The trait is synchronous — which sidesteps the entire `async_trait` vs
native-async-in-trait question for the reconciler surface. Async is
confined to the runtime's scheduling loop, which is internal wiring
and uses native `async fn` against concrete types.

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

### 6. Per-primitive storage — libSQL

**Workspace adds `libsql` as the per-primitive private-memory backend.**

- License: MIT (Turso fork of SQLite). Pure Rust.
- Version: latest stable at implementation time (workspace pin to be
  chosen by the crafter; ≥0.5 lineage).
- Usage: one libSQL connection per reconciler, held in an `Arc<Mutex<…>>`
  owned by the runtime and exposed as a `&Db` handle to `reconcile(...)`.
- No migration framework in Phase 1 — schemas are per-reconciler and
  the runtime does not manage them. The `noop-heartbeat` reconciler
  does not write anything.

libSQL (rather than `rusqlite` or `sqlx-sqlite`) matches whitepaper §4
and §17 naming explicitly: "libSQL (embedded SQLite) as the per-primitive
private-memory store." Same crate the incident memory will use in Phase 3.

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

The purity contract is enforced at three layers: (1) **trait-level** —
the synchronous `reconcile(desired, actual, db) -> Vec<Action>` signature
forbids `.await` in implementations, so an impl cannot directly invoke
async nondeterminism; (2) **compile-time** — `dst-lint` (phase-1-foundation
ADR-0006) scans any core-class crate that imports the trait for banned
nondeterminism APIs (`Instant::now`, `SystemTime::now`, `rand::*`,
`tokio::time::sleep`, raw `tokio::net::*`); (3) **runtime** — the DST
invariant `reconciler_is_pure` catches any implementation that smuggles
nondeterminism through the trait boundary (interior mutability, TLS
statics, FFI) via a twin-invocation equivalence test.

- `Reconciler` trait is in `overdrive-core`; the `dst-lint` gate
  (phase-1-foundation ADR-0006) scans any core-class crate that
  imports it. Banned APIs in a `reconcile(...)` body fail the lint.
- DST invariant `at_least_one_reconciler_registered` — live on
  every run.
- DST invariant `duplicate_evaluations_collapse` — fires N (≥3)
  concurrent evaluations at the same key; expects 1 dispatched,
  N-1 cancelled.
- DST invariant `reconciler_is_pure` — twin invocation with
  identical inputs expects bit-identical `Vec<Action>` outputs.
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

## Changelog

- 2026-04-23 — Remediation pass (Atlas peer review, APPROVED-WITH-NOTES):
  added summary lead-in to `### Enforcement` naming the three layers of
  purity enforcement (trait-level, compile-time, runtime) so a standalone
  reader can parse the enforcement chain without threading it together
  from Trait Design + Testing sections. No semantic change; bullets
  unchanged.
