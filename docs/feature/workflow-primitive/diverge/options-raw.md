# Options (Raw) — Workflow Primitive (workflow-primitive)

**Wave**: DIVERGE Phase 3 (Brainstorming) | **Agent**: Flux | **Date**: 2026-06-05

> GENERATION ONLY. No evaluation, ranking, or DVF/taste language appears
> in this file. Scoring happens in `taste-evaluation.md`. (Separation
> principle, Osborn 1953.)

---

## 1. HMW question

> **How might we let a platform subsystem drive a finite, side-effecting,
> multi-step operation to a single terminal result that survives a crash
> and resumes correctly on any node — without each subsystem re-inventing
> a state machine, a crash-resume path, and a correctness proof?**

No embedded solution (no "journal", "trait", "libSQL"). Outcome-oriented.
Broad enough to admit replay-based, log-based, step-memoized,
explicit-state, embedded-engine, and reconciler-based answers.

---

## 2. SCAMPER — one option per lens

### S — Substitute: replace the core mechanism (journal-replay → DBOS-style step-memoized resume on libSQL)
**Core idea**: Author writes `async fn run(ctx)`. Each `ctx.step(...)`
commits its output to a per-instance libSQL step-table row before
returning. On crash, the runtime re-enters `run` from the top and each
recorded `ctx.step` returns its memoized output instantly until execution
reaches the first unrecorded step.
**Key mechanism**: Step-output memoization in libSQL (the whitepaper's
named store); re-execution-from-top with memoized short-circuit.
**Key assumption**: Authored control flow is deterministic between steps;
libSQL is the right journal store.
**SCAMPER origin**: S — substitutes the whitepaper's Temporal-style
event-history-replay with DBOS-style step-memoization (same store).
**Closest competitor**: DBOS.

### C — Combine: merge the workflow primitive INTO the reconciler primitive (reconciler-as-step-machine)
**Core idea**: No new primitive. A durable sequence is a specialized
reconciler whose typed `View` carries a `step_cursor` enum + per-step
recorded outputs + retry inputs. `reconcile` reads the cursor, emits the
Action for the current step, observes the result next tick, advances the
cursor, persists the View; a terminal cursor emits `TerminalCondition`
and stops.
**Key mechanism**: Persisted step-cursor on the existing reconcile loop;
crash-resume inherited from the runtime's ViewStore bulk-load.
**Key assumption**: Terminal sequences are few and small enough that a
hand-authored step enum is acceptable; the "two distinct primitives"
doctrine is worth overruling because the mechanism subsumes it (Argo
precedent).
**SCAMPER origin**: C — combines the workflow job with the adjacent
reconciler primitive.
**Closest competitor**: Argo Workflows controller; Crossplane sequenced
reconciliation.

### A — Adapt: borrow Restate's log-based suspend/resume with a server-as-proxy, in-binary
**Core idea**: A first-class `Workflow` trait (`async fn run(ctx)`) whose
`ctx.call/sleep/wait_for_signal` journal commands+completions to an
append-only invocation log owned by an in-binary "workflow runtime" that
sits as a proxy in front of the executing function. The invocation
**suspends** (freeing the task) while awaiting a durable promise / timer /
signal, then resumes by replay from the log when the awaited completion
lands.
**Key mechanism**: Command/completion append-only log + suspension;
replay-from-log on resume.
**Key assumption**: Long waits (cert DNS propagation, human ratification)
are common enough that suspend-to-free-resources earns the log+proxy
machinery; deterministic control flow is acceptable (inherits skew).
**SCAMPER origin**: A — adapts Restate's log-based durable-execution model
into an embedded in-binary runtime.
**Closest competitor**: Restate.

### M — Modify/Magnify: amplify DST-provable-correctness as the primary dimension (explicit-state + replay-equivalence-by-construction)
**Core idea**: The primitive is an **explicit typed state machine**:
author declares `enum State`, `enum Step`, and a pure
`advance(state, observed) -> (State, Vec<Action>)` transition function.
There is no authored imperative control flow to replay — recovery reads
the persisted `State` value and dispatches `advance`. The replay-
equivalence property the whitepaper requires (O5) becomes **trivially
true by construction** (there is no control-flow trajectory to diverge),
and the same `ReconcilerIsPure`-shaped twin-invocation DST invariant
proves it.
**Key mechanism**: Pure `advance` transition over a persisted typed
`State`; correctness-by-construction (no control-flow replay).
**Key assumption**: Author will enumerate states explicitly; "ordinary
control flow authoring" is sacrificed for maximal provability + dodged
version-skew.
**SCAMPER origin**: M — magnifies the DST-provability / version-skew-
avoidance dimension above authoring ergonomics.
**Closest competitor**: AWS Step Functions (ASL state machine); hand-
authored sagas.

### P — Put to other use: expose the SAME durable-execution machinery to the deferred WASM SDK from day one (uniform ABI)
**Core idea**: Build the first-party Rust primitive as a thin Rust
front-end over a durable-execution **core** whose interface is the exact
serializable command/completion ABI the future WASM SDK will use. First-
party Rust workflows and (later) third-party WASM workflows run on one
engine, one journal format, one replay path — the platform never ships
two parallel workflow systems (whitepaper §18 "Overdrive does not ship
two parallel workflow systems").
**Key mechanism**: A serializable command/completion ABI core; Rust trait
is one front-end, WASM is the next.
**Key assumption**: Designing the ABI now (even though the SDK is
deferred) is cheaper than retrofitting it; the WASM-component execution
unit can be anticipated without building it.
**SCAMPER origin**: P — puts the primitive to use for a second audience
(WASM third parties) by designing the shared ABI up front.
**Closest competitor**: Golem / Obelisk (WASM-component durable execution).

### E — Eliminate: remove the journal/replay machinery entirely (idempotent-step + correlation, no durable control flow)
**Core idea**: There is no persisted control-flow position and no replay.
Each sequence is a set of **named idempotent steps keyed by a correlation
key**; the "workflow" is just a reconciler/handler that, on each tick,
asks the ObservationStore "which correlation keys for this sequence have
completed?" and fires the next not-yet-completed step (every step carries
an idempotency key so re-firing is safe). Recovery is "fire everything
not yet observed-complete"; ordering is enforced by only firing step N+1
once step N's completion row exists.
**Key mechanism**: Idempotency-key + completion-observation; no journal,
no cursor, no replay — order emerges from completion gating.
**Key assumption**: Every step can be made idempotent and given a stable
correlation/idempotency key; the ObservationStore completion rows are a
sufficient substitute for a journal.
**SCAMPER origin**: E — eliminates the most complex part (durable control
flow / journal / replay).
**Closest competitor**: Saga-via-idempotent-effects; the existing
`external_call_results` + `Action::HttpCall` pattern, generalized.

### R — Reverse: invert who drives — the journal drives the code, not the code driving the journal (event-sourced/CQRS workflow)
**Core idea**: Reverse the control direction. Instead of an imperative
function emitting journal entries, the **journal of events is primary**
and the workflow is a pure **fold**: `apply(state, event) -> state` plus
`decide(state) -> Vec<Command>`. Commands produce events (step
completions, signals, timer fires) appended to the log; the fold re-runs
on every new event. The "workflow" is a reducer over an event stream, not
a script.
**Key mechanism**: Event-sourced fold (`apply`) + command-deriver
(`decide`); the event log is the source of truth, code is a pure reducer.
**Key assumption**: Authors accept event-sourced modeling (events +
reducer + decider) over imperative scripting; the audit-log-for-free and
replay-equivalence-for-free benefits justify the paradigm shift.
**SCAMPER origin**: R — reverses code-drives-journal into
journal-drives-code.
**Closest competitor**: Akka Persistence / EventSourcing; Marten;
Temporal's history-as-source-of-truth taken to its logical extreme.

---

## 3. Crazy 8s supplements (1-minute ideas, structurally distinct)

### Crazy-8 #1 — Vendor/embed a WASM durable-execution engine (Golem/Obelisk) behind a Rust facade
**Core idea**: Don't build the durable core; embed an existing
wasmtime-based durable-execution engine (Golem or Obelisk) and present a
first-party Rust facade over it, accepting that first-party "Rust"
workflows compile to WASM components the engine drives.
**Key mechanism**: Reuse a third-party oplog-replay WASM engine.
**Key assumption**: The deferred WASM-component execution unit is
acceptable to pull forward; engine licensing/embeddability is workable.
**SCAMPER origin**: Crazy 8s (buy-not-build).
**Closest competitor**: Golem, Obelisk.

### Crazy-8 #2 — Two-tier: explicit-state core + optional macro-generated control-flow front-end
**Core idea**: Ship the explicit-state `advance` core (Option M) as the
durable substrate, and provide a `#[workflow] async fn run(ctx)` **proc-
macro** that compiles ordinary control flow DOWN to the explicit-state
machine at build time — the author writes ordinary control flow, the
macro generates the typed `State`/`Step` enums and the `advance` fn.
Recovery is on the generated explicit state (dodges skew); ergonomics are
ordinary-control-flow.
**Key mechanism**: Compile-time lowering of `async fn` control flow to an
explicit state machine + pure `advance`.
**Key assumption**: A proc-macro can reliably lower the bounded control-
flow shapes platform sequences use; build-time codegen is acceptable.
**SCAMPER origin**: Crazy 8s (have-both via codegen).
**Closest competitor**: Rust async state-machine lowering (the compiler
itself); ASL-from-code generators.

### Crazy-8 #3 — Journal-store-agnostic primitive: a `WorkflowJournal` port with redb/libSQL/append-log adapters
**Core idea**: Define the workflow primitive against a `WorkflowJournal`
PORT (append record, read instance records in order, mark terminal) and
ship multiple adapters — `RedbWorkflowJournal` (peer-primitive
precedent, O6-minimal), `LibsqlWorkflowJournal` (whitepaper default,
SQL replay queries), `LogWorkflowJournal` (append-only log). The
execution model is decided separately; the store is pluggable and the
default is chosen by the same reasoning ADR-0035 used for reconcilers.
**Key mechanism**: Storage-engine port + adapters; execution model
orthogonal to journal store.
**Key assumption**: The store choice is genuinely contested and worth a
port abstraction; the execution model can be specified independently of
the store.
**SCAMPER origin**: Crazy 8s (resolve the store tension by abstracting it).
**Closest competitor**: The project's own `IntentStore` / `ObservationStore`
/ `ViewStore` port-adapter pattern.

---

## 4. Curation to 6 (diversity test applied)

Candidate pool: S, C, A, M, P, E, R + Crazy-8 #1/#2/#3 = 10 options.

**Merges / eliminations (exact or genuine-variation only):**
- **P (uniform WASM ABI) ⊃ Crazy-8 #1 (vendor WASM engine)** — both pull
  the deferred WASM-component unit forward; #1 is the buy-variant of P's
  build-the-ABI. P is the stronger representative (it keeps the engine
  first-party). **Merge #1 into P** (noted as P's buy-variant).
- **Crazy-8 #3 (journal-store port)** is an *orthogonal* axis (store, not
  execution model) — it does not answer "which execution mechanism," so
  it is not a peer option to the execution-model options. **Fold its
  insight into every option's store discussion** rather than carry it as
  a 7th execution option (the store question is evaluated per-option in
  taste).
- **Crazy-8 #2 (macro-lowered explicit-state)** is a genuine third
  mechanism (ordinary-control-flow authoring + explicit-state recovery) —
  structurally distinct from both M (hand-written state) and S (replay).
  **Keep as Option G.**

**Curated 6** (final set carried to taste): A, S, C, E, R/M-merged, G.

> One adjustment for diversity: **M and R both occupy the
> "explicit-non-imperative" region** (M = explicit state machine, R =
> event-sourced fold). They differ in mechanism (transition-fn vs
> reducer-over-event-log) and cost profile (a state record vs an event
> stream + audit log), so they pass the 3-point test as distinct — BUT
> carrying both M and R *and* G (macro-lowered explicit-state) would put
> three explicit-state-family options in a six-slot set, crowding out
> mechanism diversity. **Resolution: keep R (event-sourced fold — most
> structurally distinct, code-reversed, audit-log-native) and G
> (macro-lowered — distinct authoring model), and FOLD M's "hand-written
> explicit state machine" into G as G's no-macro fallback** (G with the
> macro removed IS M). This keeps the six options maximally diverse in
> mechanism.**

### The final six

| # | Option | Mechanism (one line) | Store posture | Skew posture |
|---|---|---|---|---|
| **A** | Restate-style log-based suspend/resume, in-binary | append-only command/completion log + suspension + replay | append-only log | **inherits** |
| **B** | DBOS-style step-memoized resume (`ctx.step` on libSQL) | re-enter + memoize steps in journal | libSQL (whitepaper default) | **inherits** |
| **C** | Reconciler-as-step-machine (no new primitive) | persisted cursor on the existing reconcile loop | existing redb ViewStore | **dodges** |
| **D** | Idempotent-step + completion-gating (eliminate the journal) | idempotency-key + observation of completions; no cursor/replay | ObservationStore rows only | **dodges** |
| **E** | Event-sourced fold (`apply`/`decide`) | journal-drives-code reducer over an event log | append-only event log | **dodges** |
| **F** | Macro-lowered explicit-state (`#[workflow] async fn`) | compile-time lowering of control flow to a pure `advance` over a persisted typed `State` | redb typed-State blob (peer-primitive precedent) | **dodges** |

### 3-point diversity test

| Pair-vs-set | Different mechanism? | Different assumption about authoring? | Different cost profile? |
|---|---|---|---|
| **A** (log+replay+suspend) | Yes — replay-from-log w/ suspension | Assumes long waits + deterministic imperative code | Builds a log + suspension runtime + replay path |
| **B** (step-memoize) | Yes — re-enter + memoize (no suspension) | Assumes deterministic imperative code, no resource-free waits | Builds a journal + memoize path on libSQL |
| **C** (reconciler-as-step-machine) | Yes — cursor on existing reconcile loop | Assumes terminal sequences are few/small; overrules two-primitive doctrine | Near-zero new code (reuses runtime + ViewStore) |
| **D** (idempotent-step gating) | Yes — no cursor/replay at all; order via completion gating | Assumes every step idempotent + correlation-keyable | Builds gating + idempotency conventions; no journal |
| **E** (event-sourced fold) | Yes — journal-drives-code reducer | Assumes authors accept event-sourced modeling | Builds event log + fold/decide + audit surface |
| **F** (macro-lowered explicit-state) | Yes — build-time control-flow→state-machine lowering | Assumes ordinary control flow + a reliable proc-macro | Builds a proc-macro + explicit-state runtime |

All six answer **yes** to all three. No two share mechanism + assumption +
cost. **Diversity gate G3: PASS.**

### Eliminated / merged (with reason)

- **P (uniform WASM ABI) + Crazy-8 #1 (vendor WASM engine)** — merged
  into the cross-cutting note: pulling the deferred WASM-component
  execution unit forward is out of scope per the dispatch (the WASM SDK
  is explicitly deferred). Both options *hinge* on the deferred surface,
  which the dispatch forbids ("options must NOT hinge on the SDK or
  code-graph hashing"). Recorded here as researched-and-set-aside, not
  carried to taste. *(They remain documented in competitive-research.md
  §2.6 for traceability.)*
- **M (hand-written explicit state machine)** — folded into **F** as F's
  no-macro fallback (F minus the proc-macro IS M). Avoids three
  explicit-state-family options crowding the six-slot set.
- **Crazy-8 #3 (journal-store port)** — not an execution-model peer;
  folded into per-option store evaluation in taste (the store tension is
  scored per option, not as a standalone option).

> Note on count: the dispatch asked for "≥6 structurally diverse
> options." Six are carried to taste (A–F). The two WASM-hinged options
> (P, Crazy-8 #1) are deliberately excluded per the explicit scope
> constraint and recorded as researched alternatives. Reconciler-as-
> workflow (**C**) is present as required.

**Gate G3: PASS — 6 curated options, each passes the 3-point diversity
test, no evaluative language in the option bodies above.**
