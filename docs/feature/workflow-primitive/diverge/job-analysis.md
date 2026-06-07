# Job Analysis — Workflow Primitive (workflow-primitive)

**Wave**: DIVERGE Phase 1 (JTBD) | **Agent**: Flux | **Date**: 2026-06-05
**Anchor**: GH #39 ([3.2] Workflow primitive); whitepaper §18

---

## 1. Raw request (verbatim)

> GH #39: "Workflow primitive — `Workflow` trait, durable journal in
> per-primitive libSQL, typed signals, workflow-lifecycle reconciler.
> First-class for platform and app code; SDK load-time version-skew
> rejection via code-graph hashing."

> User's explicit steer (from dispatch): **"we want the workflows to be
> durable like restate.dev."**

The Acceptance section of #39 is an unfilled TODO — this DIVERGE defines
the direction the DISCUSS wave converges on. The request names a
**solution shape** (trait + journal + signals + reconciler) and a
**store** (libSQL) and a **comparison** (restate.dev). Per JTBD
discipline, all three are guesses at a solution. Extract the job first.

**In-scope surface** (per dispatch): the platform-internal, first-party
Rust workflow primitive that cert rotation, multi-stage deployment,
cross-region migration, staged rollout, and microVM snapshot/restore
coordination are built on. **Deferred (do NOT diverge on)**: the WASM
Workflow SDK for third-party developers, and SDK load-time version-skew
rejection via code-graph hashing. Version-skew is noted as a forward
concern; options must not hinge on the SDK or code-graph hashing.

---

## 2. Job extraction — 5 Whys (tactical → strategic/physical)

| # | Q | A | Layer |
|---|---|---|---|
| 0 | "We want a `Workflow` trait + durable libSQL journal + typed signals." | (the proposed solution) | Tactical (feature) |
| 1 | *Why* a durable journal? | So a half-finished multi-step sequence (cert rotation, region migration) is not lost when the control-plane node running it crashes. | Operational |
| 2 | *Why* must it survive the crash and not just restart from scratch? | Because the sequence has *already taken externally-visible, non-idempotent-to-rerun steps* (issued a CSR to an ACME server, quiesced a source region, took a microVM snapshot). Re-running from the top would re-issue / double-quiesce / corrupt. The completed steps must be remembered and not repeated. | Operational |
| 3 | *Why* does the platform need a primitive for this rather than each subsystem hand-rolling its own state machine + recovery? | Because every multi-step platform operation otherwise re-implements: a persisted step cursor, crash-resume-anywhere, exactly-once external effects, and a correctness proof that the resume path matches the original. That is a hard, bug-prone wheel (the durable-execution problem) and re-inventing it per subsystem multiplies the bug surface and the DST-verification surface. | Strategic |
| 4 | *Why* does it matter that resume happens *anywhere* (any control-plane node) and that correctness is *proven*? | Because the platform's entire value proposition (whitepaper §21, J-PLAT-001) is "mechanically-checked correctness under crash/partition." A durable sequence that resumes on a *different* node after the original crashes — and provably reaches the same terminal result it would have — is the orchestration-layer expression of that promise. Without it, the platform's own lifecycle operations are the least-trustworthy code it runs. | Strategic |
| 5 | *Why* (physical / irreducible)? | The irreducible function: **drive a finite multi-step sequence with externally-visible side effects to a terminal result exactly once, such that a crash at any step resumes from that step on any node without repeating completed effects or losing committed ones.** | Physical |

**Stop condition met**: step 5 is the irreducible function (drive a side-effecting finite sequence to a terminal result, crash-exactly-once). Further "why" produces a platform-goal answer ("so the platform is trustworthy").

**Disruption check** — *Is there a higher-level job that makes this job unnecessary?* Candidate: "make every platform operation a single idempotent reconciler convergence, so no multi-step sequence ever exists." Examined and rejected as a disruptor (it is captured instead as a first-class **option**, Option F reconciler-as-workflow): genuinely-terminal, ordered, side-effecting sequences ("quiesce source → handoff → resume target") are *not* expressible as "keep X looking like Y" convergence without smuggling a step-cursor into View — which is the reconciler-as-workflow option, not a disruption of the job. The job survives the disruption check.

---

## 3. Job statements

### Functional (required)

> **When** a platform subsystem must perform a finite, ordered, multi-step
> operation whose steps take externally-visible side effects that are
> unsafe to repeat (issue a cert, quiesce a region, snapshot a microVM,
> ratify a rollout), **I want** to express the sequence as ordinary
> control flow and have the platform persist its progress, resume it on
> any node after a crash from the first incomplete step, and drive it to
> a single terminal result — **so I can** rely on the operation completing
> exactly once without hand-rolling a state machine, a step cursor, a
> crash-resume path, and a correctness proof for each one.

### Emotional

> I want to *trust that a half-finished cert rotation or region migration
> will not silently corrupt or double-execute* when the node running it
> dies — the same confidence the DST harness gives me about reconcilers,
> extended to terminal sequences.

### Social

> I want the platform's own lifecycle operations to be visibly as
> rigorous as the convergence engine — so that "Overdrive's orchestration
> is mechanically-checked, not hand-rolled" is a true and defensible
> claim, not an aspiration that stops at the reconciler boundary.

---

## 4. ODI Outcome Statements

Format: `[Direction] + [Metric] + [Object] + [Context]`. Direction =
Minimize (the side-effecting-sequence domain is all about *avoiding* bad
occurrences). Forbidden words / solution references excluded.

| # | Outcome statement |
|---|---|
| O1 | Minimize the likelihood of a completed, externally-visible step being repeated when the node executing a multi-step sequence crashes and the sequence resumes. |
| O2 | Minimize the likelihood of a committed step's result being lost when a multi-step sequence resumes on a different node than the one that started it. |
| O3 | Minimize the time it takes to author a new crash-resumable multi-step platform sequence (cert rotation, region migration, staged rollout) from "I know the steps" to "it resumes correctly after a crash." |
| O4 | Minimize the likelihood that the resumed execution of a sequence reaches a terminal result different from the one the uninterrupted execution would have reached. |
| O5 | Minimize the effort required to prove, before shipping, that a sequence's crash-resume path is equivalent to its uninterrupted path. |
| O6 | Minimize the number of distinct persistence/recovery mechanisms the platform must operate and verify to support its multi-step lifecycle operations. |

### Opportunity candidates (most under-served)

This is a greenfield platform primitive — there is no incumbent Overdrive
mechanism serving these outcomes, so *satisfaction is ~1* across the
board and *importance is high* for O1, O2, O4 (the exactly-once + correct-
terminal core). No survey data exists (consistent with the existing
jobs.yaml note that JTBD here is distilled from the whitepaper, not user
interviews); opportunity is assessed qualitatively:

| Outcome | Importance (qual.) | Satisfaction today | Opportunity |
|---|---|---|---|
| O1 — no repeated side effect | Very high | None (each subsystem hand-rolls) | **Under-served** |
| O2 — no lost committed step on cross-node resume | Very high | None | **Under-served** |
| O4 — resumed terminal == uninterrupted terminal | Very high | None | **Under-served** |
| O5 — provable resume-equivalence before ship | High | Partial (DST exists for reconcilers, not sequences) | **Under-served** |
| O3 — fast to author a new sequence | High | None | **Under-served** |
| O6 — minimize # of persistence/recovery mechanisms | Medium-high | redb already serves reconcilers; libSQL already in graph | Contested (the libSQL-vs-redb store tension) |

O1, O2, O4 are the load-bearing exactly-once-correctness triple and the
heart of "durable like restate.dev." O5 ties the primitive to the
platform's DST identity. **O6 is the outcome that surfaces the
libSQL-vs-redb store tension** — it is a real outcome (operators and the
platform pay for every distinct recovery mechanism), and it is the axis
on which the journal-store decision is evaluated, not assumed.

---

## 5. Gate check (G1)

- [x] Job at **strategic/physical** level (step 4–5 of the 5-Whys; the
  irreducible "drive a side-effecting finite sequence to a terminal
  result exactly once, crash-resumable on any node").
- [x] **No feature references** in the functional job statement (no
  "trait", "journal", "libSQL", "signal" — those are solution guesses).
- [x] **≥ 3 ODI outcome statements** — 6 produced.
- [x] Functional + emotional + social job statements all present.
- [x] Disruption check performed (reconciler-as-workflow disruptor
  rejected-as-disruptor, retained-as-option).

**G1: PASS.**
