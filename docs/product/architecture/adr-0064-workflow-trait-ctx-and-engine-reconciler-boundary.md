# ADR-0064 — `Workflow` trait + `WorkflowCtx` in `overdrive-core`; durable-async engine in `overdrive-control-plane` driven off the action-shim; replay-equivalence as a DST invariant in `overdrive-sim`

## Status

Accepted. 2026-06-05. Decision-makers: Morgan (proposing, PROPOSE
mode); user ratification pending (subagent context — the
engine↔reconciler boundary and the ctx-surface granularity are
surfaced for ratification in the DESIGN return summary). Tags: phase-1,
workflow-primitive, application-arch, durable-execution, dst.

**Companion**: ADR-0063 (the redb journal — the engine's durable
backing). **Composes with**: ADR-0035 (`Reconciler` trait shape — the
peer primitive the `Workflow` trait mirrors in *placement* but not in
*purity*), ADR-0023 (action-shim — the engine's driving seam), ADR-0037
(`TerminalCondition` — the terminal-modelling precedent `WorkflowResult`
relates to), ADR-0003 (crate-class taxonomy — the `core`-has-no-tokio
rule the trait placement honours).

## Context

The locked "B′" direction (`workflow-primitive` feature, GH #39) ships a
distinct durable-async `Workflow` primitive whose execution is genuinely
`async` — the one place in the codebase where `.await` on real work is
the *correct* shape (`.claude/rules/development.md` § "Workflow
contract": "`async` is permitted in workflows. Only in workflows."). The
whitepaper §18 already fixes the trait shape:

```rust
trait Workflow: Send + Sync {
    async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult;
}
```

Three design questions inside the locked direction are this ADR's remit
(the journal store + codec are ADR-0063):

1. **Crate placement** — where do the `Workflow` trait + `WorkflowCtx`
   live, given `overdrive-core` is forbidden `tokio` / async-runtime
   deps by the dst-lint `core`-class rule (ADR-0003, CLAUDE.md)?
2. **`WorkflowCtx` surface granularity** — minimal (`ctx.call` only) for
   slice 01, or the full surface up front?
3. **The engine ↔ lifecycle-reconciler boundary** — the
   workflow-lifecycle reconciler is a *pure sync* `reconcile`
   (ADR-0035); the engine that runs `async fn run` is genuinely `async`.
   How do they compose without violating reconciler purity?

The headline constraint that shapes all three: **the reconciler primitive
is pure-sync (no `.await`, ADR-0035 §1); the workflow primitive is
durable-async**. These are the §18 peer primitives, and the boundary
between them is the subtlest part of this design.

## Decision

### 1. `Workflow` trait + `WorkflowCtx` live in `overdrive-core` (trait-only, no async-runtime dep)

The `Workflow` trait, the `WorkflowCtx` *type*, `WorkflowResult`, and
`WorkflowSpec` (replacing the placeholder at `reconcilers/mod.rs:562`)
live in a new module `overdrive-core::workflow`, mirroring how the
`Reconciler` trait lives in `overdrive-core::reconcilers` while its
*runtime* lives in `overdrive-control-plane` (ADR-0035 §1).

**This does NOT pull `tokio` into `overdrive-core`.** The trait uses
`async fn` via `async_trait` (already a core dep — the `Driver`,
`Transport`, `Llm` traits are async-trait today). `async fn` in a trait
declares a `Future`-returning signature; it does not require a runtime.
`WorkflowCtx`'s *fields* are the injected port traits (`Arc<dyn Clock>`,
`Arc<dyn Transport>`, `Arc<dyn Entropy>`) — already core trait objects —
plus a journal-cursor handle whose concrete async I/O is performed by the
engine in `overdrive-control-plane`, not inside core. The dst-lint gate
scans for `Instant::now` / `rand::*` / `tokio::net::*` / `tokio::time::sleep`
in `core`-class source; the `WorkflowCtx` *type definition* contains none
— its methods delegate to the injected traits, which is exactly the
substitution the ports layer exists for. **`WorkflowCtx` is the workflow
analogue of `TickContext`**: a core-owned bundle of injected
non-determinism, DST-controllable, with no runtime baked in.

The *engine* — the thing that actually drives `run`, polls the future,
writes the journal, and suspends/resumes — is async, holds `tokio`, and
lives in `overdrive-control-plane` (adapter-host), exactly where the
`ReconcilerRuntime` and `RedbViewStore` live. This is the same
trait-in-core / runtime-in-control-plane split ADR-0035 §2 used for
`ViewStore` ("Putting an `async fn` trait in `overdrive-core` would …
pull `tokio` into core" — note: that concern was about the *storage*
trait whose impls do real I/O; the `Workflow` trait itself is authored
by platform engineers and its async body's I/O flows through `ctx`'s
injected ports, so the trait declaration is core-safe; the engine that
executes it is not, and lives in control-plane).

**Crate map:**

| Surface | Crate | Class | Rationale |
|---|---|---|---|
| `Workflow` trait, `WorkflowCtx` type, `WorkflowResult`, `WorkflowSpec` | `overdrive-core::workflow` | core | Author-facing trait surface; no runtime; injected ports only |
| `WorkflowEngine` (drives `run`, journal cursor, suspend/resume) | `overdrive-control-plane::workflow_runtime` | adapter-host | Genuinely async; holds tokio; does journal I/O |
| `JournalStore` port + `RedbJournalStore` | `overdrive-control-plane::journal` | adapter-host | ADR-0063 |
| `workflow-lifecycle` reconciler | `overdrive-core::reconcilers` (state) + runtime registration | core (reconcile) / control-plane (registration) | Pure-sync `reconcile`, ADR-0035 shape |
| `SimJournalStore`, `replay_equivalence_*` invariant | `overdrive-sim` | adapter-sim | DST surface (ADR-0063 §6, §3 below) |

### 2. `WorkflowResult` is a new core enum; relates to but does not reuse `TerminalCondition`

`WorkflowResult` (in `overdrive-core::workflow`) models the workflow's
*terminal*:

```text
WorkflowResult =
  | Success                       // slice 01 — the ProvisionRecord terminal
  | Failed { reason: String }     // a workflow that ran to a failure terminal
  | Cancelled                     // operator/parent cancellation (forward; slice 03+)
```

`WorkflowResult` is **distinct from** `TerminalCondition` (ADR-0037):
`TerminalCondition` is the *reconciler's* claim about an *allocation's*
lifecycle (`BackoffExhausted`, `Stopped`, `Custom`), written onto
`AllocStatusRow` / `LifecycleEvent`. `WorkflowResult` is the *workflow's*
own terminal value, returned from `run`. They are **related by
composition, not by type reuse**: when the workflow-lifecycle reconciler
observes a workflow reach a `WorkflowResult`, it may emit a terminal claim
for the *workflow instance's* observation row — but the two enums model
different things (a reconciler's lifecycle decision vs a workflow's return
value) and are not substitutable. `WorkflowResult` follows the same
`#[non_exhaustive]` + K8s-`Condition`-style SemVer convention ADR-0037 §5
established (well-known variants stable; new variants additive minor;
renames major) so its evolution discipline is inherited, not its type.

The observable terminal surface (D4): the engine, on `run` returning a
`WorkflowResult`, writes a **terminal-result row to the ObservationStore**
keyed by the instance's `CorrelationKey` (US-WP-2 / slice-01 AC5), via the
same action-shim ObservationStore write path the reconciler runtime uses
— NOT a direct engine write that bypasses the sanctioned channels.

### 3. Replay mechanism — engine-owned journal cursor; `ctx.*` ops check-then-record

The engine drives replay with a **per-instance journal cursor** (the
`step` index, ADR-0063 §2). On (re)start of an instance the engine:

1. `journal.load_journal(workflow_id)` → the ordered `Vec<JournalEntry>`.
2. Constructs a `WorkflowCtx` carrying (a) the injected ports, (b) the
   loaded journal as a **replay buffer**, (c) a cursor at step 0.
3. Calls `run(&ctx).await` — a *fresh* execution of the author's `async fn`.

Every `ctx` await-op (`ctx.call`, `ctx.sleep`, `ctx.wait_for_signal`,
`ctx.emit_action`) is a **check-then-record** point:

- **Replay (cursor < journal length):** the op reads the recorded entry
  at the cursor instead of performing the effect — `ctx.call` returns the
  recorded response (re-derived from `response_digest`), `ctx.sleep`
  returns immediately if the recorded deadline has passed,
  `ctx.wait_for_signal` returns the recorded signal value if `SignalSeen`
  was recorded. The cursor advances. **No external effect re-fires** —
  this is the exactly-once guarantee (US-WP-3 AC1, K1: SimTransport call
  count == 1 on resume).
- **Live (cursor == journal length):** the op performs the real effect
  through the injected port, **appends the result entry to the journal
  with fsync BEFORE returning** (ADR-0063 §4 fsync-then-suspend), advances
  the cursor, and continues. For `ctx.sleep` / `ctx.wait_for_signal`, the
  "live" step writes the await-armed entry (deadline / signal-key) then
  **suspends** the future (parks on the injected `Clock` deadline / the
  ObservationStore signal subscription); the engine yields the instance
  back to the runtime until the wake condition fires.

This is the canonical durable-execution replay shape (Temporal / Restate /
DBOS all re-execute from the top and short-circuit completed awaits from
the journal). **Determinism is structural:** because every non-deterministic
input flows through `ctx`'s injected ports, and completed awaits are
replayed from the journal rather than re-performed, two replays of the same
journal produce a bit-identical trajectory (D-INH-5).

**How K4's `assert_replay_equivalent!` hooks in:** the
`replay_equivalence_provision_record` `SimInvariant` (ADR-0063 §6,
exported from `overdrive-sim::invariants::Invariant` — extending the
existing `ReplayEquivalentEmptyWorkflow` placeholder variant) drives the
engine through: (1) an uninterrupted run capturing the terminal trajectory;
(2) a crash-injected run (kill after step-N records, before terminal);
(3) a resumed run from the persisted journal. It asserts the resumed
trajectory is byte-identical to the uninterrupted one (replay-equivalence)
AND `assert_eventually!(is_terminal)` within the declared step budget
(bounded progress). The existing `evaluate_replay_equivalent_empty_workflow`
"two-SimEntropy-transcripts" placeholder is replaced by a real journal
replay against the engine + `SimJournalStore`.

### 4. `WorkflowCtx` surface — minimal for slice 01, additive per slice

The `WorkflowCtx` surface grows one method per await-surface slice; the
*type* and the journal-cursor machinery (§3) are established whole in
slice 01, and each later method is an additive entry-variant (ADR-0063 §2)
+ an additive `ctx` method:

| Method | Slice | Journal entry | Port consumed |
|---|---|---|---|
| `ctx.call(req) -> Result<Resp>` | 01 | `CallResult` | `Transport` |
| `ctx.sleep(Duration)` | 02 | `SleepArmed` (records deadline) | `Clock` |
| `ctx.wait_for_signal(SignalKey) -> SignalValue` | 03 | `SignalAwaited` / `SignalSeen` | `ObservationStore` (signal rows) |
| `ctx.emit_action(Action)` | 03 | `ActionEmitted` | Action channel → Raft |
| `ctx.activity(...)` | post-skeleton | (forward) | per-activity |

Slice 01 ships `ctx.call` only — the thinnest surface with a real,
non-idempotent-to-repeat external effect (the ProvisionRecord write,
US-WP-1). The journal cursor, replay buffer, suspend/resume engine, and
the `replay_equivalence_provision_record` invariant are all slice-01
(they are the abstraction every later slice builds on, per the carpaccio
"ship the abstraction first"). `ctx.emit_action` (slice 03) routes through
the **same Action channel the reconciler runtime consumes** (whitepaper
§18 *Primitive Composition* "Workflow → Reconciler"; development.md
Workflow contract rule 6 — no Raft bypass, no direct IntentStore write);
the `ActionEmitted` journal entry makes the emit idempotent on resume
(US-WP-5 AC3 / slice-03 AC3).

### 5. Engine ↔ lifecycle-reconciler boundary — reconciler stays pure-sync; engine runs off the action-shim

**The boundary (the subtlest decision):**

- **The `workflow-lifecycle` reconciler is a normal ADR-0035 pure-sync
  reconciler.** It owns *instance lifecycle* (spec → running → journaled →
  terminated) as desired-vs-actual convergence, NOT the durable-async
  execution. Its `reconcile(desired, actual, view, tick) -> (Vec<Action>,
  View)` is pure: `desired` carries the `WorkflowSpec`(s) that should be
  running (hydrated from the IntentStore via `Action::StartWorkflow`
  having been committed); `actual` carries the instances' observed states
  (running / terminal) from the ObservationStore; the `View` carries
  per-instance lifecycle memory (e.g. start attempts). It emits actions
  to *start* an instance and observes its *terminal* — it never `.await`s
  the workflow body.
- **The engine runs the async body off the action-shim**, exactly where
  the existing action-shim dispatches `Action::StartAllocation` to
  `Driver::start`. A new shim arm handles a workflow-start: when the shim
  dispatches the workflow-start action, it hands the instance to the
  `WorkflowEngine`, which spawns/drives the `async fn run` future as a
  tracked async task (the engine owns a `tokio::task` set, the same way
  the reconciler runtime owns its tick task — ADR-0023 §4). The engine's
  async work is downstream of `reconcile`'s pure return, exactly as
  `Driver::start` is (ADR-0023: "the shim is the async I/O boundary
  `reconcile` cannot cross").

**Concretely, the composition path:**

```
reconciler emits Action::StartWorkflow { spec, correlation }
  → reconciler runtime commits it (Raft / Phase-1 IntentStore)
  → action-shim dispatch picks it up
  → WorkflowEngine::start(spec, correlation):
       journal.probe-already-done-at-boot
       journal.load_journal(id)  (empty on first start; populated on resume)
       spawn run(&ctx) as a tracked async task
  → the task drives run; each ctx.* await check-then-records (§3)
  → on terminal, engine writes ObservationStore terminal-result row
       (keyed by correlation) via the shim's ObservationStore write path
  → workflow-lifecycle reconciler observes the terminal row next tick,
       converges the instance to `terminated`
```

This keeps **`reconcile` pure** (it emits `StartWorkflow`, observes
terminal rows — never awaits) and puts **all async durability in the
engine** (off the shim, ADR-0023's sanctioned async boundary). The engine
is to workflows what `Driver` is to allocations: the async executor the
pure reconciler drives through typed Actions and observes through the
ObservationStore. **On restart** (US-WP-3 AC4): the workflow-lifecycle
reconciler re-hydrates desired instances and re-emits the start for any
instance that is `running` in intent but has no live engine task; the
engine's `load_journal` finds the persisted journal and *resumes* rather
than restarting from scratch (§3 replay). The reconciler does not know or
care whether a start is a cold start or a crash-resume — the engine's
journal-load decides, which keeps the reconciler's desired-vs-actual logic
clean.

**Why the engine is NOT itself a reconciler** (the Option-C runner-up the
direction explicitly rejected, R3): a reconciler converges a single
desired/actual relationship per tick and cannot express the inner
await/suspension/signal execution surface ergonomically. The engine drives
*ordered multi-step orchestration with await-points* (issue → wait →
validate → result), which is the §18 discriminator for the distinct
primitive. The lifecycle reconciler manages *which instances should exist*;
the engine manages *how each instance's steps execute between start and
terminal*. Two concerns, two primitives — the upheld two-primitive
doctrine (R3).

### 6. DST invariants (extending the catalogue)

Added to `overdrive-sim::invariants::Invariant` (ADR-0063 §6 enumerates
them; this ADR pins their meaning):

- **`replay_equivalence_provision_record`** (replaces / supersedes the
  placeholder `ReplayEquivalentEmptyWorkflow`) — §3 above: uninterrupted
  vs crash-resumed trajectory byte-equality + `assert_eventually!
  (is_terminal)` bounded progress. **K4, the load-bearing KPI, on the CI
  critical path.**
- **`WorkflowJournalWriteOrdering`** — under `SimJournalStore` with
  injected fsync-failure on the next append: assert the engine does NOT
  advance the cursor / suspend, mirroring ADR-0035's
  `WriteThroughOrdering`.
- **`WorkflowExactlyOnceEffectOnResume`** — crash after `ctx.call`
  records but before terminal → resume → `SimTransport` call count == 1
  (US-WP-3 AC1 / K1).

The existing `ReplayEquivalentEmptyWorkflow` variant + its
`evaluate_replay_equivalent_empty_workflow` evaluator are the trust-the-sim
step-1 placeholder this feature graduates into a real journal replay; the
enum variant name evolves to the slice-specific `provision_record` form
(no inline string literal — house convention, US-WP-4 AC1).

## Considered alternatives

### Alternative A — Trait+ctx in core, async engine in control-plane off the shim (ACCEPTED)

Above.

### Alternative B — Put `Workflow` trait + engine both in `overdrive-control-plane`

Keep the entire workflow surface (trait, ctx, engine) in control-plane,
avoiding any core change.

**Rejected because:**

1. **It breaks the §18 symmetry and the author surface.** The `Reconciler`
   trait is core-owned (ADR-0035 §1); platform engineers author workflows
   the same way they author reconcilers, against a core trait. Putting the
   `Workflow` trait in control-plane makes the author surface
   adapter-host-class — every first-party workflow would depend on
   `overdrive-control-plane`, the heavy axum/rustls/tokio crate, just to
   `impl Workflow`. The trait belongs next to its peer in core.
2. **`WorkflowSpec` already lives in core** (`reconcilers/mod.rs:562`, the
   placeholder this feature replaces) because `Action::StartWorkflow`
   carries it and `Action` is core. The trait that consumes the spec
   belongs in the same crate.
3. **The dst-lint concern is unfounded for the trait** — see §1; the
   trait declaration uses injected ports, the engine (which does hold
   tokio) is correctly in control-plane.

### Alternative C — Engine IS a reconciler (Option C, the DIVERGE runner-up)

Model the workflow engine as a step-machine reconciler that advances one
journal step per tick.

**Rejected** — this is the explicitly-rejected matrix Option C (R3 of the
ratified direction). The two-primitive doctrine is upheld: a reconciler
converges a single desired/actual relationship and cannot express the
inner await/suspension/signal surface ergonomically; modelling
multi-await orchestration as per-tick convergence is the exact "hand-rolled
state machine per sequence" the primitive exists to eliminate (J-PLAT-005
/ O3). Recorded for traceability; the user selected B′ over C.

### Alternative D — Full `WorkflowCtx` surface (call+sleep+signal+emit+activity) in slice 01

Ship every `ctx` method in the first slice.

**Rejected** — violates the carpaccio slicing (DISCUSS scope assessment):
slice 01 is already the one heavy slice (engine + journal + replay). The
journal-cursor + suspend/resume *machinery* must ship whole in slice 01
(every later method needs it), but the *methods* beyond `ctx.call` are
additive entry-variants (ADR-0063 §2) that each carry their own slice's
learning hypothesis (sleep parks correctly under DST; signals/emit are
crash-safe). Shipping them all in slice 01 would bundle three learning
hypotheses into one slice and lose the de-risking the slicing buys.

## Consequences

### Positive

- **Author surface matches the reconciler surface.** Platform engineers
  `impl Workflow for X` against a core trait, exactly as they
  `impl Reconciler`. One mental model, one crate for both primitives.
- **`reconcile` stays pure (ADR-0035 invariant preserved).** The
  lifecycle reconciler emits `StartWorkflow` and observes terminal rows;
  all async durability lives in the engine off the shim (ADR-0023's
  sanctioned boundary). The `ReconcilerIsPure` DST invariant continues to
  hold unchanged.
- **`core` has no tokio.** The trait + ctx type use injected ports +
  `async_trait`; the runtime (tokio-holding) is control-plane. dst-lint
  scope unchanged.
- **Replay is structural, not disciplinary.** All non-determinism through
  `ctx`'s injected ports + journal-replay of completed awaits ⇒
  bit-identical replay (K4) by construction, the same way reconciler
  purity gives bit-identical reconcile.
- **Additive slice growth.** `ctx` methods + journal entry variants grow
  one per slice; the engine machinery is whole in slice 01.

### Negative

- **The engine is a new always-on async subsystem** (a `tokio::task` set
  for live instances), alongside the reconciler tick task. Bounded by live
  instance count; Phase-1 single-node footprint is small.
- **Two terminal-modelling enums** (`WorkflowResult` for the workflow's
  return, `TerminalCondition` for the reconciler's allocation claim). They
  model genuinely different things, but a reader must not conflate them;
  the ADR pins the distinction.
- **The engine↔reconciler handoff is a multi-hop path** (reconciler emit
  → shim → engine → ObservationStore → reconciler observe). This is the
  same level-triggered indirection the allocation path already has
  (ADR-0023); the cost is a known, accepted pattern, but the workflow path
  adds the engine task-set as a new moving part.

### Quality-attribute impact

- **Maintainability — testability**: positive. The engine takes injected
  ports + `Arc<dyn JournalStore>`; DST drives it with `Sim*` adapters and
  the replay invariant. No fixture theatre.
- **Maintainability — modifiability**: positive. Additive ctx methods +
  journal variants per slice.
- **Reliability — recoverability**: positive (single-node crash-resume via
  journal replay; pinned by the replay invariant + the ordering invariant).
- **Reliability — fault tolerance**: positive (exactly-once effect on
  resume, pinned by `WorkflowExactlyOnceEffectOnResume`).
- **Performance — time behaviour**: neutral-to-positive (replay is a
  range-scan + journal-buffer reads, no re-performed effects).
- **Security**: neutral (workflow→cluster mutations go through Raft via
  `ctx.emit_action`; no IntentStore bypass — slice-03 AC2).

## References

- ADR-0063 — workflow journal (the engine's durable backing; codec;
  fsync ordering; `JournalStore` port).
- ADR-0035 — `Reconciler` trait shape (the peer primitive's trait-in-core
  / runtime-in-control-plane placement this ADR mirrors; purity invariant
  preserved).
- ADR-0023 — action-shim (the engine's driving seam; the async boundary
  `reconcile` cannot cross).
- ADR-0037 — `TerminalCondition` (the terminal-modelling precedent
  `WorkflowResult` relates to but does not reuse; SemVer convention
  inherited).
- ADR-0003 — crate-class taxonomy (the `core`-has-no-tokio rule the trait
  placement honours).
- Whitepaper §18 *The Workflow Primitive*, *Primitive Composition*,
  *Correctness Guarantees* — the SSOT trait shape + replay obligation.
- `docs/feature/workflow-primitive/feature-delta.md` (D-INH-1, D-INH-3,
  D-INH-4, D-INH-5; US-WP-1..5; K1, K4, K6; slices 01–03).
- `.claude/rules/development.md` § "Workflow contract" (the async-only-in-
  workflows rule; ctx-only non-determinism; no Raft bypass; bounded step
  budget; journal replay bit-identical).

## Changelog

- 2026-06-05 — Initial accepted version. Companion to ADR-0063.
