# Taste Evaluation — Workflow Primitive (workflow-primitive)

**Wave**: DIVERGE Phase 4 | **Agent**: Flux | **Date**: 2026-06-05

> Weights are LOCKED before any score is written (anti-pattern guard:
> weight manipulation / retroactive justification). Scores are assigned
> per criterion across ALL surviving options before the recommendation
> is formed.

---

## Phase 1 — DVF filter (primary triage)

For a platform-internal primitive, the IDEO lenses are interpreted for
*this* domain (the "user" is a first-party Overdrive platform engineer
authoring durable sequences, and the "business" is the platform's own
maintainability + correctness posture):

- **Desirability** — does it serve the validated job + ODI outcomes
  (exactly-once O1/O2/O4, fast-to-author O3, provable O5, minimal-
  mechanism O6)?
- **Feasibility** — buildable in-binary, pure-Rust (whitepaper §2
  principle 7), DST-replayable under turmoil, with available skills/time?
- **Viability** — composes with the reconciler runtime + Action channel +
  ObservationStore, sustainable to operate (O6) and to maintain, and
  honest about the **deferred version-skew hazard** (an option that
  *inherits* version-skew ships an unmitigated, explicitly-out-of-scope
  hazard into first-party platform sequences — a viability cost).

| Option | Desirability | Feasibility | Viability | DVF total | Verdict |
|---|---|---|---|---|---|
| A — Restate-style log+suspend, in-binary | 4 | 3 | 3 | 10 | survives |
| B — DBOS-style step-memoized on libSQL | 4 | 4 | 3 | 11 | survives |
| C — Reconciler-as-step-machine | 4 | 5 | 4 | 13 | survives |
| D — Idempotent-step + completion-gating | 3 | 4 | 3 | 10 | survives |
| E — Event-sourced fold (`apply`/`decide`) | 3 | 3 | 3 | 9 | survives |
| F — Macro-lowered explicit-state | 5 | 2 | 4 | 11 | survives |

**DVF rationale (the load-bearing scores):**
- **A — Feasibility 3 / Viability 3**: a suspension+log+replay runtime is
  substantial net-new machinery; replay model **inherits version-skew**
  (the deferred hazard) → Viability capped at 3.
- **B — Feasibility 4**: DBOS's SQL-table journal maps cleanly to libSQL
  (already in graph); but its signature transactional-step value
  evaporates (our steps don't write the journal's DB), and it **inherits
  version-skew** → Viability 3.
- **C — Feasibility 5 / Viability 4**: reuses the existing reconciler
  runtime + redb ViewStore + `ReconcilerIsPure` DST invariant — minimal
  net-new code; **dodges version-skew**; O6 maximal. Viability docked to
  4 only by the doctrinal cost (overrules reconcilers.md / whitepaper §18
  "neither is expressible as the other").
- **D — Desirability 3**: serves crash-resume but **does not serve O3
  ergonomics** (author hand-wires idempotency keys + completion gating)
  and ordering/visibility is implicit; correctness is harder to reason
  about than an explicit cursor. Survives but weakly desirable.
- **E — Desirability 3 / Feasibility 3 / Viability 3 = 9** (closest to the
  <6 floor but survives): event-sourced fold dodges skew and gives audit-
  for-free, but imposes a **paradigm shift** (events+reducer+decider) that
  fights the project's reconciler/`reconcile`-pure-function mental model;
  net-new event-log machinery. Lowest DVF; survives the filter, will be
  pressure-tested in taste.
- **F — Desirability 5 / Feasibility 2**: ordinary-control-flow authoring
  (best O3) **and** explicit-state recovery (dodges skew) is the most
  desirable shape — but a proc-macro lowering `async fn` control flow to
  a correct pure `advance` is the **highest build risk** (Feasibility 2)
  and a maintained component the project owns forever.

**No option eliminated** (all DVF ≥ 6; lowest is E at 9). The DVF filter
did its job — it documents that A/B inherit the deferred hazard (Viability
3) and that F carries real build risk (Feasibility 2), which the taste
criteria then weigh.

---

## Phase 2 — Locked weights

**Profile: Developer Tool** (this is a platform-internal primitive for
first-party Rust authors and the DST harness — not a consumer app). Per
the taste-evaluation skill's Developer-Tool column, with **two documented
adjustments** for this specific primitive, locked before scoring:

| Criterion | Skill default (Dev Tool) | LOCKED weight | Adjustment rationale |
|---|---|---|---|
| DVF (avg) | 25% | **30%** | +5: for a *correctness primitive*, "does it serve the exactly-once/provable job and compose with the platform" is more load-bearing than for a typical dev tool. The deferred-version-skew viability cost must carry weight. |
| T1 Subtraction | 15% | **15%** | unchanged |
| T2 Concept Count | 20% | **25%** | +5: this primitive is judged on how few NEW mental concepts/mechanisms it adds to a platform that already has reconcilers, the Action channel, redb ViewStore, and DST invariants. O6 ("minimize # of distinct mechanisms") is a primary outcome; T2 is its taste expression. |
| T3 Progressive Disclosure | 15% | **10%** | −5: a platform-internal primitive authored by experts has less need for staged first-interaction disclosure than a user-facing tool. |
| T4 Speed-as-Trust | 25% | **20%** | −5: reinterpreted for this domain as *recovery/crash-resume responsiveness + tick/replay latency*, not UI latency. Still material (it is the durable-execution hot path), but the −5 funds the T2 +5 where the real differentiation lives. |

**Total = 100%.** Weights are now frozen. T4 is interpreted for this
domain as: how fast/clean is crash-recovery + steady-state execution, and
does the mechanism avoid pathological re-execution cost (e.g.
re-executing a long sequence from the top on every resume)?

---

## Phase 3 — Scoring matrix (all options, all criteria)

Each cell carries a one-clause justification below the table.

| Option | DVF (avg/5) | T1 Sub | T2 Concept | T3 Prog | T4 Speed | **Weighted** |
|---|---|---|---|---|---|---|
| **C** Reconciler-as-step-machine | 4.33 | 5 | 5 | 4 | 4 | **4.50** |
| **F** Macro-lowered explicit-state | 3.67 | 3 | 4 | 5 | 5 | **4.05** |
| **B** DBOS-style step-memoized (libSQL) | 3.67 | 4 | 3 | 4 | 3 | **3.51** |
| **A** Restate-style log+suspend | 3.33 | 3 | 2 | 3 | 3 | **2.95** |
| **D** Idempotent-step gating | 3.33 | 4 | 3 | 2 | 4 | **3.35** |
| **E** Event-sourced fold | 3.00 | 3 | 2 | 2 | 3 | **2.75** |

> DVF/5 = (D+F+V)/3 from Phase 1, rescaled to a 1–5 cell (e.g. C: 13/15×5
> = 4.33; F: 11/15×5 = 3.67; B: 11/15×5 = 3.67; A: 10/15×5 = 3.33; D:
> 10/15×5 = 3.33; E: 9/15×5 = 3.00).
> Weighted = DVF×0.30 + T1×0.15 + T2×0.25 + T3×0.10 + T4×0.20, /5 implicit
> in the per-cell 1–5 scale (max 5.0).

### Per-criterion justifications

**T1 — Subtraction** ("nothing can be removed without breaking core value"):
- **C = 5**: there is nothing TO add — it is the existing reconciler with
  a cursor field; the new-primitive, new-store, new-runtime, new-recovery
  are all *removed*. Maximal subtraction.
- **B = 4**, **D = 4**: each removes a layer (B removes event-history
  replay → bare step memoize; D removes the journal entirely) but keeps a
  dedicated workflow surface.
- **F = 3**, **A = 3**, **E = 3**: each carries a substantial dedicated
  apparatus (F: proc-macro + state runtime; A: log + suspension + replay;
  E: event log + fold + decide + audit) where parts could plausibly be
  staged/removed.

**T2 — Concept Count** (NEW mental concepts a first-time author learns) —
*the heaviest-weighted differentiator, expressing O6*:
- **C = 5**: **zero new concepts.** The author already knows `reconcile`,
  `View`, `Action`, `TerminalCondition`, the redb ViewStore, the DST
  invariants. A step-cursor is one field in a View they already write.
- **F = 4**: **one** new concept ("write `#[workflow] async fn`; it
  lowers to a state machine") — well-anchored to ordinary async Rust.
- **B = 3**, **D = 3**: two concepts each (B: journal + memoized-step
  re-entry; D: idempotency-key + completion-gating ordering).
- **A = 2**: three+ interdependent concepts (durable journal, suspension,
  command/completion replay, signals) — a whole new durable-execution
  mental model alongside reconcilers.
- **E = 2**: three+ (events, reducer `apply`, decider `decide`, the
  reversed control direction) — a paradigm shift from the project's
  pure-`reconcile` model.

**T3 — Progressive Disclosure** (first interaction = only what's needed):
- **F = 5**: first interaction is writing one ordinary `async fn`; all
  durability machinery is hidden behind the macro.
- **C = 4**, **B = 4**: first interaction is a small step enum + one
  `reconcile`/`run` (C) or `ctx.step` calls (B); depth (retries,
  signals) revealed on demand.
- **A = 3**: first interaction already exposes journal+suspend+signal
  surfaces.
- **D = 2**, **E = 2**: first interaction forces choosing the gating/
  correlation scheme (D) or the event+reducer+decider triad (E) up front.

**T4 — Speed-as-Trust** (crash-recovery + steady-state execution cost;
avoid pathological re-execution):
- **F = 5**, **A = 3 (suspension helps) ...** recovery cost:
  - **F = 5**: recovery reads one persisted `State` value and dispatches
    `advance` — O(1), no re-execution of prior steps.
  - **C = 4**: recovery is the runtime's existing bulk-load + a cursor
    read — O(1) per instance, on the proven reconciler hot path; tick
    cadence (100ms) is the steady-state latency floor.
  - **D = 4**: recovery fires not-yet-completed steps — cheap, but a
    burst of completion-observations adds tick churn.
  - **B = 3**: re-enters `run` from the top and replays memoized steps —
    O(steps) re-entry cost on every resume (memoized steps are cheap but
    the control flow re-runs).
  - **A = 3**: replay-from-log on resume is O(journal); suspension avoids
    holding a process during long waits (a plus) but resume still replays.
  - **E = 3**: fold re-runs `apply` over the event prefix on each event —
    O(events) per advance unless snapshotted.

**DVF/5** cells carry the Phase-1 rationale (version-skew viability cost
sinks A/B; build risk sinks F's feasibility; doctrinal cost docks C's
viability to 4; E is lowest overall).

---

## Phase 4 — Weighted ranking

| Rank | Option | Weighted score |
|---|---|---|
| 1 | **C — Reconciler-as-step-machine** | **4.50** |
| 2 | **F — Macro-lowered explicit-state** | **4.05** |
| 3 | B — DBOS-style step-memoized (libSQL) | 3.51 |
| 4 | D — Idempotent-step gating | 3.35 |
| 5 | A — Restate-style log+suspend | 2.95 |
| 6 | E — Event-sourced fold | 2.75 |

**The ranking follows the matrix with no override.** C wins on the two
heaviest criteria (DVF 30% via Feasibility/Viability, T2 25% via zero-new-
concepts) — the same two criteria that encode the platform's O6
("minimize distinct mechanisms") and the version-skew-avoidance viability
concern. F is a clear, distinct second: it *maximizes* authoring
ergonomics (O3) and dodges skew, but pays for it in build risk
(Feasibility 2) and a maintained proc-macro.

### The "durable like restate.dev" tension, made explicit

The user's literal steer ("durable like restate.dev") most directly
describes **Option A** — which the matrix ranks **5th**. This is not the
matrix ignoring the steer; it is the matrix surfacing that the steer and
the scope collide:

- Restate's *durability mechanism* is journal-replay, which **inherits
  the version-skew hazard that #39 explicitly defers** (no code-graph
  hashing in scope). Shipping A means shipping that unmitigated hazard
  into first-party cert-rotation/region-migration sequences.
- The matrix reads the steer as **"give me Restate's exactly-once,
  crash-resume-anywhere, ordinary-authoring durability"** — the
  *outcomes* (O1/O2/O3/O4) — not necessarily Restate's *replay
  mechanism*. **Options C and F deliver those outcomes while dodging the
  deferred hazard.** F in particular gives the *ordinary-control-flow
  authoring* that is the felt experience of "durable like restate" (write
  normal code, it just survives crashes) **without** the replay-skew tax.

This tension is the central decision for DISCUSS and is called out in the
recommendation's dissenting case.

---

## Anti-pattern self-audit

- [x] All 6 options scored on all 4 taste criteria + DVF — no cherry-pick.
- [x] Weights locked (Phase 2) before any Phase-3 score written — no weight
  manipulation; the two adjustments are documented with rationale.
- [x] Recommendation (next file) follows the matrix; no "feels right"
  override — C is top by score.
- [x] DVF used as a filter (Phase 1), not a tie-breaker; its rescaled
  value is a weighted input, with the elimination threshold honored
  (none eliminated, lowest = 9 > 6).

**Gate G4: PASS** — all surviving options scored on all criteria; weights
documented and locked; ranking complete; recommendation (with dissent) in
`../recommendation.md` traces to this matrix.
