# Recommendation — Workflow Primitive (workflow-primitive)

**Wave**: DIVERGE | **Agent**: Flux | **Date**: 2026-06-05
**Anchor**: GH #39; whitepaper §18; traces to
`diverge/taste-evaluation.md` scoring matrix.
**Hands off to**: nw-product-owner (DISCUSS wave).

> **⚠️ SUPERSEDED BY RATIFIED DECISION (2026-06-05).** The matrix below
> recommended **Option C (reconciler-as-step-machine)**. In the
> post-DIVERGE design dialogue the user **selected the "B′" synthesis: a
> distinct durable-async `Workflow` primitive journaled in redb** (Option
> B's authoring model, redb store instead of libSQL). The selection rests
> on three premises the matrix did not have — (R1) version-skew is an
> SDK-era concern, not an architectural driver, so the [D3] penalty on
> replay options is withdrawn; (R2) the journal lives in redb, not libSQL;
> (R3) the two-primitive doctrine is **upheld** (the discriminator is the
> await/suspension/signal surface, not termination — Jobs already
> run-to-completion on the reconcile loop). See `wave-decisions.md` §
> "RATIFIED DIRECTION" for the full record. **The matrix analysis below is
> retained for the trail; Option C is the runner-up, not the call.**

---

## Validated job (from `diverge/job-analysis.md`)

> When a platform subsystem must perform a finite, ordered, multi-step
> operation whose steps take externally-visible side effects unsafe to
> repeat (issue a cert, quiesce a region, snapshot a microVM, ratify a
> rollout), express the sequence as ordinary control flow and have the
> platform persist progress, resume on **any node** after a crash from
> the first incomplete step, and drive it to a single terminal result
> **exactly once** — without hand-rolling a state machine, a crash-resume
> path, and a correctness proof for each one.

Load-bearing ODI outcomes: **O1** no repeated side effect on resume,
**O2** no lost committed step on cross-node resume, **O4** resumed
terminal == uninterrupted terminal, **O3** fast to author, **O5** provable
resume-equivalence before ship, **O6** minimize # of distinct
persistence/recovery mechanisms.

---

## Top 3 options

### 1. Option C — Reconciler-as-step-machine — Score 4.50

A durable terminal sequence is modeled as a specialized reconciler whose
typed `View` carries a `step_cursor` enum + per-step recorded outputs +
retry inputs. `reconcile` reads the cursor, emits the Action for the
current step (existing `Action::HttpCall` / cluster-mutation / signal
channel), observes the result on the next tick via the ObservationStore,
advances the cursor, and persists the View through the runtime-owned redb
ViewStore. A terminal cursor emits a typed `TerminalCondition` (ADR-0037)
and stops. **Prior art: Argo Workflows' controller is exactly this** — a
reconcile-loop driving a node-phase state machine with requeue until
terminal.

- **Why it scores well**: Maximal on the two heaviest criteria — DVF
  (Feasibility 5 / Viability 4: reuses the existing reconciler runtime,
  redb ViewStore, `ReconcilerIsPure` DST invariant — near-zero net-new
  code) and T2 Concept Count (5: **zero new mental concepts** for an
  author who already writes reconcilers). **Dodges the deferred
  version-skew hazard** (recovery keys on a persisted cursor value under
  additive-only CBOR evolution, not on control-flow replay). O6 is
  maximally served — no new store, no new recovery mechanism, no new DST
  machinery.
- **Core trade-off**: Authoring ergonomics (O3). The author writes a step
  enum + a transition `match`, not an ordinary `async fn run`. The "magic
  writes the state machine for you" half of the job is unserved.
- **Key risk**: The doctrinal one. `.claude/rules/reconcilers.md`
  explicitly classifies "genuinely-terminal sequences (workflow-shaped)"
  as **NOT a reconciler candidate**, and whitepaper §18 asserts
  "Reconcilers converge; workflows orchestrate. Neither is expressible as
  the other." Choosing C is choosing to **overrule that doctrine** on the
  evidence that the *mechanism* (persisted cursor on a reconcile loop)
  genuinely subsumes terminal sequences (Argo proves it at scale). If the
  doctrine is load-bearing for reasons beyond mechanism (e.g. the WASM
  extension story, suspension/await ergonomics, parent-child workflow
  composition), C is the wrong call.
- **Hire criteria**: Choose C when the set of first-party durable
  sequences is **small, single-digit-step, and rare** (cert rotation,
  region migration, staged rollout, microVM snapshot coordination — the
  whitepaper's own list), when **O6 (one mechanism) and dodging the
  deferred version-skew hazard outweigh authoring ergonomics**, and when
  the team is willing to amend the two-primitive doctrine in the
  whitepaper + reconcilers.md.

### 2. Option F — Macro-lowered explicit-state — Score 4.05

Author writes `#[workflow] async fn run(ctx) -> WorkflowResult` in
ordinary control flow; a build-time proc-macro **lowers** that control
flow into an explicit typed `enum State` + a pure
`advance(state, observed) -> (State, Vec<Action>)` transition function.
Recovery reads the persisted `State` value (redb typed blob,
peer-primitive precedent) and dispatches `advance` — **no replay of
authored control flow**.

- **Why it scores well**: Best Desirability (5 — ordinary-control-flow
  authoring, the felt experience of "durable like restate") *and* dodges
  version-skew (recovery on a persisted state value), *and* best T3/T4
  (first interaction is one ordinary `async fn`; recovery is O(1) state
  read). It is the option that delivers the user's "write normal code, it
  survives crashes" intent **without** inheriting the replay-skew tax.
- **Core trade-off**: Build risk and ownership. A proc-macro that
  correctly lowers `async fn` control flow to a pure `advance` is the
  highest-feasibility-risk option (Feasibility 2) and a maintained
  compiler component the project owns forever.
- **Key risk**: The macro can reliably lower only **bounded** control-flow
  shapes. If a platform sequence needs a loop, a dynamic branch count, or
  a shape the macro can't lower, the abstraction leaks and the author
  drops to hand-written explicit state (which is F's no-macro fallback,
  = the folded-in Option M). The assumption that platform sequences stay
  within the macro's lowerable subset must hold.
- **Hire criteria**: Choose F when **authoring ergonomics (O3) is
  non-negotiable** — when platform engineers will write enough durable
  sequences that hand-authoring state enums is a real tax — and the team
  accepts owning a proc-macro to get ordinary-control-flow authoring
  while still dodging version-skew.

### 3. Option B — DBOS-style step-memoized resume on libSQL — Score 3.51

A `Workflow` trait (`async fn run(ctx)`); each `ctx.step(...)` commits its
output to a per-instance libSQL journal row before returning. On crash the
runtime re-enters `run` from the top; recorded steps return memoized
outputs instantly until execution reaches the first unrecorded step. This
is **the closest faithful realization of the whitepaper's current sketch**
(durable journal in libSQL) with DBOS's lighter step-memoize model
substituted for Temporal's event-history replay.

- **Why it scores well**: Highest-feasibility of the *imperative-
  authoring* options (libSQL is already in the dep graph; step-table
  journal maps 1:1). Good T3 (`ctx.step` calls, depth on demand). Serves
  O3 (ordinary `async fn`) directly.
- **Core trade-off**: It **inherits the deferred version-skew hazard** —
  DBOS docs are explicit that the workflow function must be deterministic
  ("if non-deterministic, it may execute different steps during recovery")
  — and #39 defers the code-graph-hash mitigation. It also adopts libSQL
  for the journal even though the **peer reconciler primitive was
  deliberately moved OFF libSQL to redb (ADR-0035)** for O6 reasons.
- **Key risk**: Shipping an unmitigated, explicitly-out-of-scope
  version-skew hazard into first-party platform sequences; and a second
  storage engine for the same small-record workload that ADR-0035 argued
  against.
- **Hire criteria**: Choose B if **fidelity to the whitepaper's existing
  sketch** ("durable like restate", journal in libSQL, `async fn run`) is
  valued above dodging the deferred hazard, and the team intends to bring
  the deferred code-graph-hash version-skew rejection **back into scope**
  to mitigate the inherited hazard.

*(Options D — idempotent-step gating (3.35), A — Restate-style log+suspend
(2.95), and E — event-sourced fold (2.75) rank 4–6; full scoring in
`diverge/taste-evaluation.md`.)*

---

## Recommendation

**Proceed with Option C (reconciler-as-step-machine) as the primary
direction, contingent on the DISCUSS wave ratifying the doctrinal
amendment it requires.**

The matrix is unambiguous: C wins on the two heaviest-weighted criteria
(DVF 30%, T2 Concept Count 25%), which are precisely the criteria that
encode the platform's stated O6 ("minimize distinct mechanisms") and the
version-skew-avoidance viability concern that the deferred-mitigation
scope makes acute. C delivers the job's exactly-once + crash-resume-
anywhere core (O1/O2/O4) **for free** by reusing the proven reconciler
runtime + redb ViewStore + `ReconcilerIsPure` DST invariant — no new
primitive, no new store, no new recovery path, no new correctness-proof
machinery (O5/O6 maximally served) — and it **structurally dodges the
deferred version-skew hazard** rather than inheriting it.

The recommendation is **explicitly contingent**, because C's single
critical weakness is not technical but doctrinal: it requires overruling
the whitepaper §18 / reconcilers.md "two distinct primitives, neither
expressible as the other" claim. The research shows the *mechanism* claim
is overstatable (Argo runs terminal sequences on a reconcile loop in
production), but the *ergonomics* and *extension-story* reasons behind the
doctrine are real. **DISCUSS must decide whether the two-primitive
doctrine is load-bearing beyond mechanism.** If it is — if suspension/
await ergonomics, parent-child workflow composition, or the WASM extension
trait-surface genuinely demand a distinct primitive — then **Option F is
the fallback**: it preserves a distinct `Workflow` authoring surface and
ordinary-control-flow ergonomics while still dodging version-skew, at the
cost of owning a proc-macro.

---

## Dissenting case

**The scoring almost chose F (4.05 vs C's 4.50), and a single defensible
weight change flips them.** F outscores C on Desirability (ordinary-
control-flow authoring, O3) and on T3/T4. The matrix ranks C first mainly
because **T2 Concept Count is weighted 25%** and C adds *zero* new
concepts while F adds one. **If the team believes authoring ergonomics
(O3) is the dominant outcome** — because platform engineers will write
many durable sequences and the per-sequence authoring tax compounds — then
T-something should be reweighted toward authoring ergonomics, F's
Desirability-5 dominates, and **F becomes the recommendation.** The
honest framing for DISCUSS: *C is right if durable sequences are few and
O6 dominates; F is right if durable sequences are many and O3 dominates.*

There is also a **second, sharper dissent that does not come from the
matrix**: the user's literal steer was "durable like restate.dev," which
most faithfully points at **Option A/B** (journal-replay, `async fn run`).
The matrix ranks those 5th/3rd because they **inherit the deferred
version-skew hazard**. If DISCUSS decides to **bring the deferred
code-graph-hash version-skew mitigation back into scope**, the viability
penalty on A/B lifts, and the "most faithful to the whitepaper + the
user's steer" option (B) becomes far more competitive. This is the one
case where the recommendation should be revisited from the top rather than
adjusted at the margin — it is a **scope decision, not a taste decision**,
and it belongs to DISCUSS.

---

## Decision for DISCUSS wave

> **Proceed with Option C (reconciler-as-step-machine), assuming the
> DISCUSS wave ratifies amending the whitepaper §18 / reconcilers.md
> "two distinct primitives" doctrine to permit terminal sequences on the
> reconcile loop via a persisted step-cursor.** If that doctrine is judged
> load-bearing beyond mechanism (suspension ergonomics, parent-child
> composition, the deferred WASM extension surface), fall back to
> **Option F (macro-lowered explicit-state)**, which preserves a distinct
> ordinary-control-flow `Workflow` surface while still dodging the
> deferred version-skew hazard.
>
> **Two scope questions DISCUSS must answer first**, because they can flip
> the recommendation:
> 1. **Is the two-primitive doctrine load-bearing beyond mechanism?**
>    (C vs F.)
> 2. **Does the deferred version-skew mitigation (code-graph hashing)
>    stay deferred?** If it comes back into scope, re-open Option B
>    (whitepaper-faithful, "durable like restate.dev") as a live
>    contender — this is a scope decision, not a taste decision.
>
> **Journal-store note** (applies to whichever option wins): the
> whitepaper's "journal in libSQL" is an assumption to validate, not a
> given. The peer reconciler primitive was deliberately moved OFF libSQL
> to redb (ADR-0035) for O6 reasons, and a crash-resume journal is
> append-mostly small-record point-access — closer to redb / an
> append-only log than to a query-heavy SQL workload. C answers this for
> free (existing redb ViewStore); B/A/F should each justify their store
> rather than inherit libSQL.

---

## Forward concern to surface (NOT a created issue)

**Version-skew hazard for the inheriting options (A, B, E to a degree).**
GH #39 defers SDK load-time version-skew rejection via code-graph hashing.
The research establishes that **any replay-of-authored-control-flow option
(A, B, and the WASM-engine options) inherits this hazard unmitigated.**
The recommended option (C) and the fallback (F) **structurally avoid** it,
so no new tracking issue is needed *if C or F is chosen*. **If DISCUSS
chooses B (or reopens A), the deferred version-skew mitigation must be
tracked** — but per repo policy this agent does **not** create GitHub
issues. Surfacing it here for the orchestrator to relay: *should DISCUSS
land an inheriting option, a tracked issue for the version-skew mitigation
becomes a prerequisite, pending user approval to create it.* (#39's own
Acceptance section is still an unfilled TODO; this DIVERGE is the input
that lets it be filled.)
