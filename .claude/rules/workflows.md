# Workflow Discipline

When — and whether — a piece of code should run as a durable workflow,
and what the minimum bar is when it coordinates a crash-sensitive
sequence.

This doc governs the **triage decision**: *should this be a workflow, and
which bar must it meet?* The **implementation contract** — the `Workflow`
trait shape (`async fn run(ctx, input) → Result<Output, TerminalError>`),
the `ctx` await-surface (`run` / `sleep` / `wait_for_signal` /
`emit_action`), journal-and-replay mechanics, and the result/error model
(terminal vs. transient) — lives in `.claude/rules/development.md` §
"Workflow contract" (and ADR-0063/0064/0065) and is the SSOT for *how* to
write one. This file is the SSOT for *when* and *whether*. The
reconciler-vs-workflow split lives in `development.md` § "Workflow
contract" (the decision table) and `.claude/rules/reconcilers.md`; this
file points at them rather than restating them.

The rule below was extracted from the two-primitive doctrine ratified in
ADR-0064 — specifically the decision to ship a distinct durable-async
`Workflow` primitive rather than model durable sequences as a reconciler
with a step-cursor `View`. The precedent section at the end distils the
boundary it defends.

---

## The decision rule

**A workflow candidate is a terminating, ordered sequence of two or more
side-effecting steps that must run once to a result and must survive a
crash mid-sequence without repeating the steps that already completed.**
It coordinates external effects (network calls, cluster mutations, timed
waits, signal rendezvous) as a unit, and the unit has a natural
`Ok(output)` / `Err(terminal)` end.

It is a candidate when ALL of these hold:

1. **There is a natural terminal result.** The operation ends — it is not
   "keep X looking like Y forever." If it never terminates, it is a
   reconciler, not a workflow (per `reconcilers.md` and the
   `development.md` decision table).
2. **It is a sequence of ≥2 ordered steps**, at least one with an external
   side effect or a wait. A single idempotent operation is not a workflow
   (see "Not a candidate").
3. **A crash mid-sequence must not repeat completed steps.** Re-running the
   whole thing from the top would re-fire a side effect that is expensive
   or incorrect to repeat (a provision, a registration, an external
   `POST`). Progress must be *journaled and replayed*, not recomputed from
   scratch.
4. **Every source of non-determinism is injectable through `ctx`.** Clock,
   network, randomness, signals, and cluster mutations all flow through
   `WorkflowCtx` (`ctx.run` / `ctx.sleep` / `ctx.wait_for_signal` /
   `ctx.emit_action`) — nothing is read directly. A body that needs
   `Instant::now()` / `reqwest` / `tokio::time::sleep` in its own code is
   not yet a workflow body; route the effect through `ctx` first (per
   `development.md` § "Workflow contract").

---

## The two bars

"Should this be a workflow?" is two questions, not one. Conflating them
produces both over-engineering (a full `Workflow` for a single idempotent
call) and the failure this rule exists to prevent (a hand-rolled durable
state machine smeared across reconciler ticks, or a bare
`tokio::spawn`).

### Bar 1 — durability is not hand-rolled (the floor; non-negotiable)

A crash-sensitive multi-step sequence (criteria 1–3) MUST get its
durability from the journal-and-replay engine — never from a reconciler
whose `View` carries a step cursor, and never from a bare `async` task
that re-runs from the top (or not at all) after a crash. This is the
minimum bar. Simulating durable execution with a step-counter `View`
re-derives the whole sequence every tick, has nowhere to park on an
external wait, and reimplements — badly — exactly what the engine already
provides. Reaching for `tokio::spawn` to fire a multi-step external
sequence leaves a half-applied effect and no resume on crash. If it is
workflow-shaped, it goes through the `Workflow` primitive.

### Bar 2 — a first-class `Workflow` impl on the engine (the destination)

Author one ordinary `async fn run` over `ctx` against the `Workflow`
trait (`development.md` § "Workflow contract"), register it in the
engine's `WorkflowRegistry`, and trigger an instance with
`Action::StartWorkflow` from a reconciler. The engine owns the journal,
replay, retry/backoff, and crash-resume; the `WorkflowLifecycle`
reconciler re-emits `StartWorkflow` for an instance that should be
running but has no live task (ADR-0064 §5). The author writes the body
and the trigger; nothing else.

### A single idempotent effect stays a reconciler action

The valid "below the line" case, mirroring converge-on-boot for
reconcilers: a *single* external call that is safe to re-issue (an
idempotent `PUT`, a `POST` with an idempotency key) does NOT need a
workflow. It is a reconciler `Action::HttpCall` with the retry inputs
(`attempts`, `last_failure_seen_at`) in the `View` — the worked example in
`development.md` § "Reconciler I/O". Only when the reconciler would need
to coordinate **three or more** external calls that must complete *as a
unit* does the sequence cross into workflow territory (`development.md` §
"Reconciler I/O" rule 4). Do NOT promote a one-shot call to a workflow to
"be safe"; do NOT smear a three-step sequence across reconciler ticks to
avoid writing a workflow.

---

## Not a candidate

- **Forever-converging / a standing invariant.** "Keep N replicas
  running," "keep the BPF map equal to policy," "hold a service at its
  declared health" — these never terminate, so each is a *reconciler*
  (`reconcilers.md`), not a workflow. This is the inverse of the
  reconciler doc's "genuinely-terminal sequences are workflow-shaped."
- **A single idempotent external call.** One step is not a sequence — it
  is a reconciler `HttpCall` action with retry memory in the `View`, per
  Bar 1's "below the line" case above.
- **Pure computation.** No side effects and no crash sensitivity — a
  `#[test]` / proptest is the tool; durability and replay buy nothing.
- **End-to-end-idempotent fire-and-forget.** If re-running the entire
  sequence from the top on a crash is genuinely harmless (every step a
  no-op on re-apply), you do not need a journal — a reconciler that
  re-emits the action each tick already gives crash-safety for free. Reach
  for a workflow only when *repeating a completed step is the hazard*.
- **Unbounded lifecycle.** A body that loops indefinitely or has no
  terminal violates the bounded-step-budget rule (`development.md` §
  "Workflow contract" rule 5) — it is a reconciler, not a workflow.
- **Writing intent directly.** A workflow does not bypass Raft to write
  the `IntentStore`; if the operation is "commit this desired state," that
  is an `Action`, and the deciding logic is a reconciler. `ctx` exposes
  `emit_action`, not a store handle.

---

## Symptoms during review

The shapes that signal the boundary is being violated:

- **A reconciler `View` carrying a step cursor / phase enum** —
  `enum Phase { Requested, Validated, Published }` — whose `reconcile`
  switches on "which step am I on" and emits a different single external
  action each tick. That is a hand-rolled durable state machine; it should
  be a `Workflow`. (The exact anti-pattern the two-primitive doctrine
  rejects — ADR-0064.)
- **A chain of 3+ `Action::HttpCall`s** sequenced across ticks with
  correlation bookkeeping to remember "which call already went out." The
  bookkeeping is a journal in disguise — use the real one.
- **A bare `tokio::spawn` (or detached task) running a multi-step external
  sequence** with no journal, where a crash mid-sequence leaves a
  half-applied effect and no resume path.
- **An `async fn` doing real `.await`-ed I/O outside a workflow body** — in
  core, reconciler, sidecar, or policy code. `async` on real work is
  correct *only* in a workflow body (`development.md` § "Workflow
  contract"); elsewhere the I/O belongs behind a port trait or an
  `Action`.
- **A workflow body that loops forever / has no `Ok(_)` terminus** — it is
  a reconciler wearing a workflow's clothes.
- **A workflow body reading `Instant::now()` / `rand::*` /
  `tokio::time::sleep`, or calling an HTTP client directly** instead of
  going through `ctx` — breaks journal replay; the DST
  replay-equivalence harness diverges.
- **A workflow reaching for the `IntentStore`** instead of
  `ctx.emit_action` — bypasses Raft and the action boundary.

---

## Codebase precedent

- **The primitive (Bar 2 destination):** the `Workflow` trait +
  `WorkflowCtx` (`crates/overdrive-core/src/workflow/mod.rs`); the engine
  (`crates/overdrive-control-plane/src/workflow_runtime/`); the durable
  journal (`crates/overdrive-control-plane/src/journal/`, ADR-0063,
  `workflow-journal.redb`); the result/error model (`StepError` /
  `TerminalError` / `WorkflowStatus`, ADR-0065). Driven by
  `Action::StartWorkflow` through the action-shim; resumed by the
  `WorkflowLifecycle` reconciler.
- **Reference workflows (test fixtures, not production):**
  `ProvisionRecord`, `ProvisionRecordWithSleep`,
  `ProvisionRecordWithSignalEmit`
  (`crates/overdrive-core/src/testing/workflow.rs`) — the canonical
  `ctx.run → terminal`, `ctx.run → sleep → ctx.run`, and
  `wait_for_signal → emit_action → terminal` shapes.
- **The bridge reconciler:** `WorkflowLifecycle`
  (`crates/overdrive-core/src/reconcilers/workflow_lifecycle.rs`) —
  observes workflow instances, re-emits `StartWorkflow` for one that
  should be running with no live task, and converges on the observed
  terminal `WorkflowStatus`. The integration boundary, not a workflow
  itself.
- **A single external call stays a reconciler action, NOT a workflow:** the
  retry-memory `HttpCall` pattern in `development.md` § "Reconciler I/O" →
  "Worked example" (`RetryMemory { attempts, last_failure_seen_at }` in the
  `View`). The boundary: one idempotent call = action; 3+
  coordinated-as-a-unit = workflow.
- **No production workflow registered yet:** `overdrive serve` builds
  `WorkflowRegistry::new()` empty
  (`crates/overdrive-control-plane/src/lib.rs`); the engine is wired and
  exercised end-to-end under test, but no first-party workflow ships in a
  live deploy. The canonical first workflow is **certificate rotation**
  ([#40](https://github.com/overdrive-sh/overdrive/issues/40), DST
  replay-equivalence gated) — request → wait for DNS propagation →
  validate → publish, four ordered steps each with an effect, the textbook
  Bar-2 case.

---

## Cross-references

- `.claude/rules/development.md` § "Workflow contract" — the `Workflow`
  trait, the `ctx` await-surface, the six workflow rules, and the
  reconciler-vs-workflow decision table (SSOT for *how*).
- `.claude/rules/development.md` § "Reconciler I/O" — rule 4 (3+
  coordinated external calls → workflow) and the worked single-call
  `HttpCall` example (the boundary below which a workflow is overkill).
- `.claude/rules/reconcilers.md` — the mirror discipline: the
  terminal-sequence disqualifier that sends forever-converging code the
  other way.
- `.claude/rules/testing.md` § "Tier 1 — Deterministic Simulation
  Testing" — `assert_replay_equivalent!` is the canonical workflow target;
  workflow `run` bodies are a mandatory mutation-testing surface.
- ADR-0063 (workflow journal, redb), ADR-0064 (Workflow trait + ctx +
  engine↔reconciler boundary), ADR-0065 (result/error model: typed output
  + terminal error).
- Whitepaper §18 (workflows as the peer primitive to reconcilers), §21
  (DST replay-equivalence).
