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
2. **`WorkflowCtx` surface granularity** — minimal (`ctx.run<T>` only) for
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
`WorkflowStart` (replacing the placeholder at `reconcilers/mod.rs:562`)
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
| `Workflow` trait, `WorkflowCtx` type, `WorkflowResult`, `WorkflowStart` | `overdrive-core::workflow` | core | Author-facing trait surface; no runtime; injected ports only |
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

### 3. Replay mechanism — engine-owned journal cursor; partition-at-construction; `ctx.*` ops check-then-record

The engine drives replay with a **per-instance journal cursor**. On
(re)start of an instance the engine:

1. `journal.load_journal(workflow_id)` → the flat ordered
   `Vec<LoadedEntry>` (commands + notifications interleaved in append
   order — ADR-0063 §3; the store is a dumb ordered log).
2. Constructs a `WorkflowCtx` carrying (a) the injected ports, (b) a
   `JournalCursorHandle` built via `JournalCursorHandle::new` /
   `new_with_channels`, which **partitions the loaded run ONCE at
   construction** (Q2, amended 2026-06-06 — see § "Changed Assumptions"):
   - `Vec<JournalCommand>` — the **positional replay walk**, in command
     append order. The cursor advances over this, one command at a time.
     `Started` is command-index 0; the first re-executed `await`-point
     reads command-index 1.
   - `BTreeMap<SignalKey, JournalNotification>` — the **correlated
     notification lookup** (`BTreeMap`, not `HashMap`, per
     `.claude/rules/development.md` § "Ordered-collection choice"; the
     map is iterated by DST invariants and must be deterministic across
     seeds). `SignalSeen` notifications land here, keyed by `SignalKey`,
     **off** the positional walk.
3. Calls `run(&ctx).await` — a *fresh* execution of the author's `async fn`.

**The partition lives at the cursor, not the store** (Q2). The store
returns the flat `Vec<LoadedEntry>`; the cursor classifies into the two
collections at construction. This RETIRES the pre-amendment
two-positional-entry signal walk (the `*cursor += 2` advance that treated
`SignalAwaited` + `SignalSeen` as two consecutive positional entries —
see § "Changed Assumptions" CA-5). Under the typed model:

- `SignalAwaited` is a **command** — it advances the command-cursor by 1
  (exactly like every other command).
- `SignalSeen` is a **notification** — it is matched by `SignalKey`
  lookup in the `BTreeMap`, never by cursor position.
- **"Crashed while still blocked"** is now a *structural* condition,
  type-checkable: a `SignalAwaited` command is present at the cursor with
  **no matching `SignalSeen` notification** in the lookup map → the wait
  did not complete → re-block on the same `SignalKey`. (Before: a lone
  `SignalAwaited` with no following positional `SignalSeen`; the
  positional check is replaced by the keyed-absence check.)

Every `ctx` await-op (`ctx.run<T>`, `ctx.sleep`, `ctx.wait_for_signal`,
`ctx.emit_action`) is a **check-then-record** point. The generic
durable-step primitive is `ctx.run<T>(name, f)` (the Restate `ctx.run`
model — wrap ANY side-effecting future `f`, journal its result, replay the
journaled result on resume without re-running `f`):

```rust
pub async fn run<T, F>(&self, name: &str, f: F) -> Result<T, WorkflowCtxError>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Send,
    F: std::future::Future<Output = T> + Send;
```

- **Replay (cursor < journal length):** the op reads the recorded entry
  at the cursor instead of performing the effect. For `ctx.run<T>`, it
  deserializes the recorded CBOR `result_bytes` at the cursor into `T`,
  returns it, and **drops `f` WITHOUT polling it** — the effect never
  re-fires. `ctx.sleep` returns immediately if the recorded deadline has
  passed, `ctx.wait_for_signal` returns the recorded signal value if
  `SignalSeen` was recorded. The cursor advances. **No external effect
  re-fires** on the replay path — this is the **exactly-once-on-replay**
  guarantee (US-WP-3 AC1, K1: SimTransport call count == 1 on resume from
  a journaled step).
- **Live (cursor == journal length):** the op performs the real effect
  through the injected port. For `ctx.run<T>`, it `f.await`s, CBOR-serializes
  the `T`, and **durably appends + fsyncs the journal entry BEFORE returning**
  (ADR-0063 §4 fsync-then-suspend), advances the cursor, and returns `T`.
  For `ctx.sleep` / `ctx.wait_for_signal`, the "live" step writes the
  await-armed entry (deadline / signal-key) then **suspends** the future
  (parks on the injected `Clock` deadline / the ObservationStore signal
  subscription); the engine yields the instance back to the runtime until
  the wake condition fires.

**Identity is positional, not content-correlated.** The command identity
is the monotonic command-index (= the cursor's position in
`Vec<JournalCommand>`), NOT a content correlation and NOT the storage
append-position (ADR-0063 §3). The in-entry `step` field is gone (Q5,
ADR-0063 § "Changed Assumptions" CA-2); position in the partitioned
command vector IS the identity.

**Determinism gate — Layers 1+2, fail-closed (Q4, amended 2026-06-06).**
On replay, each `await`-op the resumed body performs is checked against
the `JournalCommand` recorded at the current command-index, in two
layers (the Restate RT0016 shape — `docs/research/workflow/restate-journal-replay-model.md`
Finding 4a; "the match is on the *sequence of command types at each
position*"):

- **Layer 1 — type-at-index.** The recorded `JournalCommand` variant at
  command-index N must match the await-op kind the resumed body performs
  at N (a `RunResult` for a `ctx.run`, a `SleepArmed` for a `ctx.sleep`,
  a `SignalAwaited` for a `ctx.wait_for_signal`, an `ActionEmitted` for a
  `ctx.emit_action`). A mismatch is a nondeterministic body → the engine
  **fails closed** with `WorkflowCtxError::NonDeterministic { expected,
  actual }`. This is the **structural twin of the trap**: before this
  amendment a variant mismatch at the cursor silently fell through to the
  live path (re-performing the effect against a divergent journal); the
  fail-closed gate makes a divergent journal an error, not a silent
  re-execution.
- **Layer 2 — name (within `RunResult`).** When Layer 1 confirms a
  `RunResult` at command-index N, the recorded `name` must equal the
  `name` the resumed body passed to `ctx.run` at N. A mismatch → the same
  `WorkflowCtxError::NonDeterministic` fail-closed. `name` is a diagnostic
  label AND this determinism check; it is not identity (position is).

**Layer 3 — content/digest comparison — is DEFERRED**
([#214](https://github.com/overdrive-sh/overdrive/issues/214)). The gate
does **not** compare the recorded `result_digest` / `value_digest` /
`action_digest` against a re-derived digest of the replaying body's
effect. Layers 1+2 (type-at-index + name) are the Phase-1 gate; the
optional Layer-3 content comparison (catching "same operation shape, same
name, but the closure now produces different bytes") is tracked in #214
and is not built here.

**Honest semantics — at-least-once effect, exactly-once on replay.**
Because the journal record happens AFTER the effect runs (fire → fsync), a
crash in the window between `f` firing and the fsync completing re-runs `f`
on resume. The honest guarantee is therefore: **at-least-once for the
effect; exactly-once on the replay path** — once a step's result is
journaled, the recorded result is returned and `f` is never re-polled.
This is the same caveat Restate's `ctx.run` carries; effects that must be
exactly-once at the remote side carry their own idempotency key (the
existing `Action::HttpCall` idempotency-key machinery is the precedent).

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

| Method | Slice | Journal entry (class) | Port consumed |
|---|---|---|---|
| `ctx.run<T>(name, f) -> Result<T>` | 01 | `RunResult` (command) | any (via the closure `f`; transport via `ctx.transport()`) |
| `ctx.sleep(Duration)` | 02 | `SleepArmed` (command — records deadline) | `Clock` |
| `ctx.wait_for_signal(SignalKey) -> SignalValue` | 03 | `SignalAwaited` (command, advances cursor) + `SignalSeen` (notification, `SignalKey`-keyed) | `ObservationStore` (signal rows) |
| `ctx.emit_action(Action)` | 03 | `ActionEmitted` (command) | Action channel → Raft |
| `ctx.activity(...)` | post-skeleton | (forward) | per-activity |

The `ctx.wait_for_signal` row is the one await-surface that produces both
classes: a `SignalAwaited` **command** (advances the command-cursor by 1
when the wait is armed — Q2/§3) and, when satisfied, a `SignalSeen`
**notification** (matched by `SignalKey` in the cursor's
`BTreeMap<SignalKey, JournalNotification>`, never by position). Every
other await-surface produces a single command.

Slice 01 ships `ctx.run<T>` only — the general durable-step primitive,
the thinnest surface that wraps a real, non-idempotent-to-repeat external
effect (the ProvisionRecord write, US-WP-1, expressed as a `ctx.run` whose
closure performs a `Transport` datagram send via the `ctx.transport()`
accessor and returns a `T`). The `Transport` port stays on `WorkflowCtx`
(exposed via `ctx.transport()`) so closures can perform transport effects;
transport errors fold into the user's `T` (e.g. `T = Result<usize, String>`),
not into `WorkflowCtxError`. The journal cursor, replay buffer,
suspend/resume engine, and the `replay_equivalence_provision_record`
invariant are all slice-01 (they are the abstraction every later slice
builds on, per the carpaccio "ship the abstraction first"). `ctx.emit_action` (slice 03) routes through
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
  View)` is pure: `desired` carries the `WorkflowStart`(s) that should be
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
reconciler emits Action::StartWorkflow { start, correlation }
  → reconciler runtime commits it (Raft / Phase-1 IntentStore)
  → action-shim dispatch picks it up
  → WorkflowEngine::start(spec, correlation):
       journal.probe-already-done-at-boot
       journal.load_journal(id)  (empty on first start; populated on resume)
       IF the loaded run is empty (first start, not a resume):
         journal.append(Started { spec_digest, input_digest })  [fsync]
         — the command-index-0 entry (ADR-0063 §2 CA-4); the cursor's
           positional walk begins at this command. On resume the loaded
           run already contains Started at command-index 0, so it is
           NOT re-appended (idempotent first-entry write).
       partition the loaded run at the cursor (§3): commands → the
         positional Vec<JournalCommand>; SignalSeen → the SignalKey map
       spawn run(&ctx) as a tracked async task
  → the task drives run; each ctx.* await check-then-records (§3)
       (ctx.run<T> journals the CBOR-encoded T at the await-point;
        the determinism gate Layers 1+2 fail-closed on a divergent journal)
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
  - **Cursor-advance guard (NEW IN SCOPE, Q6, amended 2026-06-06).** The
    SAME invariant (the verbatim `replay_equivalence_provision_record`
    name — NOT a new invariant family) is **extended** to drive a run
    whose journal has `Started` at command-index 0, crash after step-N,
    resume, and assert: **(a)** the resumed `ctx.run` effect fires **0
    times** (K1 — the recorded command is replayed, the closure is not
    re-polled); AND **(b)** the resumed command sequence is
    byte-identical *including* `Started` at command-index 0, with **zero
    re-executions caused by a non-command (the `SignalSeen` notification)
    being consumed as a command** (K4). This is the guard that would have
    caught the trap: a `Started` command-index-0 entry that the cursor
    correctly walks, and a notification that never enters the positional
    command sequence. The DST replay-equivalence invariant is the
    structural enforcement of the typed split — a regression that let a
    notification leak into the command walk, or that dropped the
    `Started` write, fails this invariant.
- **`WorkflowJournalWriteOrdering`** — under `SimJournalStore` with
  injected fsync-failure on the next append: assert the engine does NOT
  advance the cursor / suspend, mirroring ADR-0035's
  `WriteThroughOrdering`.
- **`WorkflowExactlyOnceEffectOnResume`** — asserts the **replay-path**
  guarantee: once a `ctx.run<T>` step's result is journaled, the recorded
  result is returned on resume and the effect closure `f` is NOT re-fired.
  Crash AFTER `ctx.run` records but before terminal → resume → `SimTransport`
  call count == 1 (US-WP-3 AC1 / K1). This is *exactly-once on the replay
  path*, not an unconditional exactly-once: a crash in the fire→fsync window
  (before the step journals) re-runs `f` on resume (at-least-once for the
  effect, per §3 "Honest semantics"). The invariant pins the
  result-is-journaled → effect-not-re-fired property, which is the one the
  durable replay machinery guarantees.

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
2. **`WorkflowStart` already lives in core** (`reconcilers/mod.rs:562`, the
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
(every later method needs it), but the *methods* beyond `ctx.run<T>` are
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
- **Reliability — fault tolerance**: positive (exactly-once effect on the
  replay path — once journaled, the effect is not re-fired — pinned by
  `WorkflowExactlyOnceEffectOnResume`; at-least-once in the fire→fsync
  window per §3 honest semantics, mitigated by remote idempotency keys).
- **Performance — time behaviour**: neutral-to-positive (replay is a
  range-scan + journal-buffer reads, no re-performed effects).
- **Security**: neutral (workflow→cluster mutations go through Raft via
  `ctx.emit_action`; no IntentStore bypass — slice-03 AC2).

## Changed Assumptions

Amendment **2026-06-06 — workflow-journal command/notification split**
(feature `workflow-journal-command-notification-split`; GUIDE-mode
decisions Q1–Q6, all user-ratified). Companion to ADR-0063's amendment
of the same date. This block quotes the superseded original ADR-0064
text verbatim (with its location in this file) and pins the replacement
+ rationale (the back-propagation contract). The §3 / §4 / §5 / §6 text
above has been edited in place to match.

### CA-5 — the `*cursor += 2` two-positional-entry signal walk → command-advance-1 + `SignalKey` notification lookup (Q2)

**Original (ADR-0064 §3 "Replay mechanism", as accepted 2026-06-05):**

> the loaded journal as a **replay buffer** … On (re)start of an instance
> the engine: 1. `journal.load_journal(workflow_id)` → the ordered
> `Vec<JournalEntry>`. 2. Constructs a `WorkflowCtx` carrying … (b) the
> loaded journal as a **replay buffer**, (c) a cursor at step 0.

The pre-amendment `JournalCursorHandle` held a flat `replay_buffer:
Vec<JournalEntry>` and walked it positionally; `replay_signal` advanced
`*cursor += 2` to skip a `SignalAwaited` + `SignalSeen` *pair* treated as
two consecutive positional entries.

**Replacement:** the cursor is built by `JournalCursorHandle::new` /
`new_with_channels`, which **partition the loaded `Vec<LoadedEntry>` once
at construction** into `Vec<JournalCommand>` (the positional walk) +
`BTreeMap<SignalKey, JournalNotification>` (correlated lookup) — §3.
`SignalAwaited` is a command (advances the cursor by 1, like every other
command); `SignalSeen` is a notification matched by `SignalKey`, never by
position. The `*cursor += 2` advance is RETIRED.
"Crashed while blocked" becomes the structural condition "`SignalAwaited`
command present at the cursor with no matching `SignalSeen` notification
in the lookup map → re-block."

**Rationale:** the two-positional-entry walk conflated two semantic
classes (an armed-command and a satisfied-notification) into one
positional sequence — exactly the conflation the trap exploits. The
typed split removes the notification from the positional walk entirely
(`SignalKey`-keyed), so the cursor advances over commands only and a
notification can never be consumed as a command. The partition lives at
the cursor (not the store) to keep `JournalStore` a dumb ordered log
(ADR-0063 §3) — a future HA adapter (#205) re-implements the log without
re-deriving replay semantics.

### CA-6 — determinism gate: silent fall-to-live on variant mismatch → Layers 1+2 fail-closed (Q4)

**Original (ADR-0064 §3, as accepted 2026-06-05):** the only determinism
check pinned was the `name`-mismatch fail-closed within `ctx.run`. A
*variant* mismatch at the cursor (the recorded entry's type not matching
the await-op the resumed body performs) was NOT a pinned fail-closed case
— the positional cursor implementation fell through to the live path
(re-performing the effect against a divergent journal). This is the
trap's twin.

**Replacement:** an explicit two-layer determinism gate (§3): **Layer 1**
(type-at-index — the recorded `JournalCommand` variant at command-index N
must match the resumed body's await-op kind) and **Layer 2** (name within
`RunResult`), both fail-closed with `WorkflowCtxError::NonDeterministic {
expected, actual }`. **Layer 3** (content/digest comparison) is DEFERRED
→ [#214](https://github.com/overdrive-sh/overdrive/issues/214).

**Rationale:** a divergent journal must be an error, not a silent
re-execution against the wrong recorded state. Layer 1 closes the
silent-fall-to-live path; the Restate RT0016 "command-type-at-position"
check is the precedent (research Finding 4a). Layer 3 (catching "same
shape, same name, different bytes") is real but optional for Phase 1 and
is tracked, not built.

### CA-7 — `Started` write obligation + the cursor-advance DST guard (Q6)

**Original (ADR-0064 §5 "composition path", as accepted 2026-06-05):**
`WorkflowEngine::start` was specified as `load_journal(id)` → `spawn
run(&ctx)`; **no clause obligated writing `Started`**, and the engine
code never did (ADR-0063 § "Changed Assumptions" CA-4 — the trap).

**Replacement:** §5's composition path now obligates
`WorkflowEngine::start` to `journal.append(Started { spec_digest,
input_digest })` [fsync] on first start (empty loaded run) — the
command-index-0 entry — and to skip the re-append on resume (the loaded
run already contains it; idempotent first-entry write). §6's
`replay_equivalence_provision_record` invariant is **extended** (verbatim
name, NOT a new invariant family) to drive a run whose journal has
`Started` at command-index 0, crash after step-N, resume, and assert (a)
the resumed `ctx.run` effect fires 0 times (K1) AND (b) the resumed
command sequence is byte-identical incl. `Started` at index 0, with zero
re-executions caused by a notification consumed as a command (K4).

**Rationale:** the DST replay-equivalence invariant is the structural
enforcement of the typed split (per the methodology: "DST replay-
equivalence is the structural guard"). The extended invariant is the
guard that would have caught the trap — a regression that drops the
`Started` write or lets a notification leak into the command walk fails
it. Minimal notification model (Q6): only `BTreeMap<SignalKey,
JournalNotification>` is built; no general `NotificationId` correlation
model and no forward pointer for one (single-node Phase-1 has exactly one
notification shape).

## References

- ADR-0063 — workflow journal (the engine's durable backing; codec;
  fsync ordering; `JournalStore` port). **Companion amendment
  2026-06-06** (the entry taxonomy CA-1..CA-4 this ADR's cursor +
  determinism + DST amendments CA-5..CA-7 pair with).
- `docs/research/workflow/restate-journal-replay-model.md` — the Restate
  journal-v2 `Command`/`Notification` split + RT0016 determinism check
  (Findings 3b, 4a) the cursor partition (CA-5) and the determinism gate
  (CA-6) are modelled on.
- [#205](https://github.com/overdrive-sh/overdrive/issues/205) — HA
  cross-node crash-resume (the cursor partition + the dumb-log store
  leave it un-precluded; §5).
- [#214](https://github.com/overdrive-sh/overdrive/issues/214) —
  determinism Layer-3 (content/digest) comparison, deferred (the gate is
  Layers 1+2; §3 / CA-6).
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
- 2026-06-05 — Replaced the slice-01 `ctx.call(CallRequest) -> CallResponse`
  await-surface with the general `ctx.run<T: Serialize + DeserializeOwned>(name,
  impl Future<Output = T>) -> T` durable-step primitive (Restate `ctx.run`
  model): §2 surface-granularity question, §3 check-then-record narrative (added
  the `run<T>` signature, positional-identity / `name`-as-determinism-check, and
  the honest at-least-once-effect / exactly-once-on-replay semantics), the §4
  await-surface table row 1, the §5 composition path, and the
  `WorkflowExactlyOnceEffectOnResume` invariant (reframed to the replay-path
  guarantee). `CallRequest`/`CallResponse`/`CALL_PURPOSE`/`WorkflowCtxError::Transport`
  removed; `Transport` stays on `ctx` via a `ctx.transport()` accessor. User-pinned
  contract decision; greenfield single-cut (no deprecation shim).
- 2026-06-06 — **Command/Notification split — cursor + determinism + DST**
  (see § "Changed Assumptions" CA-5..CA-7; feature
  `workflow-journal-command-notification-split`; GUIDE-mode Q1–Q6,
  user-ratified; companion to ADR-0063's same-date amendment). §3:
  cursor partitions the loaded `Vec<LoadedEntry>` once at construction
  into `Vec<JournalCommand>` (positional walk) + `BTreeMap<SignalKey,
  JournalNotification>` (correlated lookup); retired the `*cursor += 2`
  two-positional-entry signal walk (CA-5, Q2). §3: explicit determinism
  gate Layers 1+2 (type-at-index + name) fail-closed with
  `WorkflowCtxError::NonDeterministic`; Layer 3 (content/digest) deferred
  → [#214](https://github.com/overdrive-sh/overdrive/issues/214) (CA-6,
  Q4). §5: `WorkflowEngine::start` obligated to write `Started` at
  command-index 0 on first start (idempotent on resume) — closes the
  trap (CA-7, Q6). §6: `replay_equivalence_provision_record` extended
  (verbatim name) with the `Started`-at-index-0 + notification-not-as-
  command cursor-advance guard (CA-7, Q6). §4 await-surface table:
  `SignalAwaited` (command) / `SignalSeen` (notification) classes pinned.
  Minimal notification model — no general `NotificationId`. User-pinned 2026-06-06.
