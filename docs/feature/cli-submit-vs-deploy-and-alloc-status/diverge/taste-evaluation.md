# Taste Evaluation — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DIVERGE / Phase 4
**Owner**: Flux (`nw-diverger`)
**Date**: 2026-04-30
**Discipline**: Generation is complete. This phase is **evaluation
only**. Weights are locked **before** scoring per the skill.

---

## Phase 1 — DVF Filter

Apply IDEO's three-lens filter. Any option failing two or more lenses
or scoring DVF Total < 6 is eliminated before taste scoring.

| Option | Desirability | Feasibility | Viability | Total | Status |
|---|---|---|---|---|---|
| **S — Submit-streams-default** | 5 — matches user's literal ask "should submit wait"; J-OPS-003's emotional arc culminates in "trusting the platform" | 4 — HTTP long-poll/SSE on submit; tractable on axum/reqwest; backend filter to job-id is a SQL subscription | 5 — no commercial cost; aligns with all priors (single-node, ESR, journey extension TUI mockup) | **14** | PASS |
| **A — Submit-async + status-follow** | 4 — requires user to learn a second verb; familiar shape (systemctl + journalctl) | 5 — smaller change; reuses existing endpoints; just add `--follow` mode and richer snapshot | 5 | **14** | PASS |
| **M — Submit-async + dense-status** | 3 — only fixes half the user's complaint; submit still says `Accepted.` for the broken-binary case | 5 — smallest change; pure status enrichment; no new transport protocol | 5 | **13** | PASS |
| **P — `deploy` verb** | 4 — matches fly mental model; adds a verb to the surface | 4 — mostly same backend as S, mounted on a different path; submit kept | 5 | **13** | PASS |
| **E — One-verb (submit absorbs status)** | 3 — radical; senior SREs may resist losing snapshot capability | 3 — significant rework; submit-handler must subsume status's discoverability; `cluster status` and `logs` must absorb the second-day inspection role | 4 — commits to a model that may not extend to logs/exec/etc. when those land | **10** | PASS |
| **R — Plan/Apply split** | 3 — plan is value-add but doesn't directly answer "should submit wait"; user explicitly framed the question as submit-vs-deploy, plan/apply is a different fork from a different mental model | 3 — dry-run scheduler tractable; dry-run driver `stat` requires Phase 1 ProcessDriver to expose a pre-flight check; whitepaper §18 reconciler purity intact | 4 — plan/apply is a strong long-term shape but Phase 1 may not be the right time | **10** | PASS |

**All six survive the DVF filter** (none below 6). Options E and R sit
at the threshold (10/15) — flagged for taste-phase scrutiny.

---

## Phase 2 — Weights (locked before scoring)

This is a **developer tool used in CI**. Per the skill, the
developer-tool weight column is the starting point. I am keeping that
column with **one explicit adjustment**, documented here so the
adjustment is auditable rather than retroactive:

| Criterion | Default (dev tool) | Adjustment | Locked weight | Rationale |
|---|---|---|---|---|
| DVF (avg) | 25% | 0 | **25%** | Default — DVF is the foundational filter and we already used it once; keeping its post-filter influence proportionate. |
| T1 Subtraction | 15% | 0 | **15%** | Default. |
| T2 Concept Count | 20% | **+5pp** | **25%** | The user's framing is "is this one verb or two; how many concepts must I learn?" Concept count is the lens most diagnostic of the verb-soup risk that this divergence has to navigate (six options with up to three verbs each). T2 should carry more weight than the dev-tool default. |
| T3 Progressive Disclosure | 15% | 0 | **15%** | Default. |
| T4 Speed-as-Trust | 25% | **−5pp** | **20%** | The user's complaint is *not* "submit is slow." It's "submit lies and status is empty." Perceived honesty dominates over perceived speed for this divergence. T4's rubric still captures the relevant concern (does the option introduce latency/friction?) but at slightly lower weight than the dev-tool default. |

Total: 25 + 15 + 25 + 15 + 20 = **100%**. Locked.

---

## Phase 3 — Taste scoring

### Option S — Submit-streams-default

| Criterion | Score | Reasoning |
|---|---|---|
| T1 Subtraction | **5** | Nothing to subtract — submit is already the canonical verb, streaming makes the wait visible without a separate command. Removing the separate `alloc status --follow` step is itself a subtraction relative to A. |
| T2 Concept Count | **4** | One concept (submit) plus one minor concept (`--detach` for CI). Maps to `nomad job run` muscle memory; `fly deploy` muscle memory; `docker run` muscle memory. The TTY-detection sub-feature means many operators won't even encounter the flag. |
| T3 Progressive Disclosure | **5** | First interaction = type submit, watch convergence happen. Depth (status, logs, cluster status, retry budget) is revealed only on demand — none of it is required for the first inner-loop turn. |
| T4 Speed-as-Trust | **4** | Streaming visible feedback is honest about what's happening. Minor risk: a slow lifecycle reconciler tick could feel frozen — mitigated with progress dots / heartbeat ticks. The wait IS the work; perceived latency is bounded by the lifecycle reconciler tick budget. |

DVF mean = 14/3 = 4.67.

### Option A — Submit-async + status-follow

| Criterion | Score | Reasoning |
|---|---|---|
| T1 Subtraction | **3** | Two verbs survive. Could be smaller (E does smaller). |
| T2 Concept Count | **3** | Two concepts: submit (commits) and `alloc status --follow` (the canonical post-submit move). The operator must learn that the second is the canonical companion to the first. Mitigated by submit's hint pointing to it; not eliminated. |
| T3 Progressive Disclosure | **4** | First interaction is two commands but they're sequenced; the second builds on the first. The `--follow` flag is depth on an existing verb, not a distinct concept. |
| T4 Speed-as-Trust | **4** | Stream surface is honest. The "submit returns immediately and you might forget to follow" failure mode is real; the hint catches most of it but some operators will paste the output, see "Accepted." and a hint, and move on without following. |

DVF mean = 14/3 = 4.67.

### Option M — Submit-async + dense-status snapshot

| Criterion | Score | Reasoning |
|---|---|---|
| T1 Subtraction | **4** | Doesn't add anything new (no new verb, no new flag if the optional `--wait` is deferred). Could be smaller in absolute terms (E removes a verb), but additions are zero. |
| T2 Concept Count | **4** | One existing concept (`alloc status`) made denser; no new mental model. The optional `--wait` would add one new concept, dropping this to 3 if shipped — Phase 1 can defer it. |
| T3 Progressive Disclosure | **4** | First interaction is submit (no change); second is `alloc status` (richer). Sequencing is unchanged from today; depth (events, restart-budget, last-tick timestamp) is revealed in the snapshot output but the operator can ignore most of it on the happy path. |
| T4 Speed-as-Trust | **2** | **The core concern.** Doesn't fix the user's first question — submit *still* returns "Accepted." for a broken binary. Operators still discover failure asynchronously. Snapshot output is denser but the operator still has to know to look. The user's literal first question ("shouldn't submit wait") is structurally unaddressed. |

DVF mean = 13/3 = 4.33.

### Option P — `deploy` verb

| Criterion | Score | Reasoning |
|---|---|---|
| T1 Subtraction | **2** | Adds a verb. Submit kept; deploy kept; alloc status kept. Verb count grows from 2 to 3. |
| T2 Concept Count | **2** | Three concepts now: submit (raw), deploy (the "make-it-real" inner-loop verb), alloc status (the snapshot). The operator must learn when to use which. Fly deploys justify this with their build-and-push complexity (deploy = build + push + roll out); Overdrive doesn't have that complexity to justify the verb split. |
| T3 Progressive Disclosure | **3** | First interaction has to *choose* between submit and deploy. Help text can guide ("use `deploy` for interactive workflows, `submit` for raw automation"); muscle memory will eventually settle. But the choice is exposed at the start. |
| T4 Speed-as-Trust | **4** | Same streaming backend as S; same honesty. Slightly less than S because the verb choice is itself a friction in the first-time experience. |

DVF mean = 13/3 = 4.33.

### Option E — One-verb (submit absorbs status)

| Criterion | Score | Reasoning |
|---|---|---|
| T1 Subtraction | **5** | Maximum subtraction — `alloc status` is gone. One verb total for the inner loop. |
| T2 Concept Count | **3** | Submit becomes idempotent and stateful (re-submit attaches). The "submit is sometimes a commit, sometimes an attach" dual semantic IS a new concept the operator must learn. Two concepts compressed into one verb is denser, not always simpler. |
| T3 Progressive Disclosure | **3** | First interaction is fine. Second-day interaction ("how do I check on this job I deployed yesterday?") forces the operator to re-run submit to inspect — surprising. Cluster status and logs are still there but the journey-defined "I want to see allocation state" needs `alloc status` or its replacement. |
| T4 Speed-as-Trust | **4** | Streaming is honest; same backend as S. Slightly less than S because the dual-mode semantic creates a small uncertainty on first inspect-via-resubmit ("am I commiting again?"). |

DVF mean = 10/3 = 3.33.

### Option R — Plan/Apply split

| Criterion | Score | Reasoning |
|---|---|---|
| T1 Subtraction | **2** | Adds plan as a new verb. Apply is a renamed submit + wait. Status survives. Three verbs net. |
| T2 Concept Count | **2** | Three concepts: plan, apply, status. The plan/apply terraform mental model is foreign to senior SREs from the ops world (they expect kubectl/nomad shape, not terraform shape). New mental model required. |
| T3 Progressive Disclosure | **2** | First interaction *forces* the operator to learn plan-before-apply. They cannot "just submit and see what happens" — that path is gone or relegated. |
| T4 Speed-as-Trust | **5** | Best of all options at speed-of-feedback — failures surface *before* any state mutation. Pre-flight = no async failure for the common case. The "binary not found" failure the user actually hit would be caught at plan time, not after submit returns. |

DVF mean = 10/3 = 3.33.

---

## Phase 4 — Weighted scoring matrix

Weights locked: DVF 25%, T1 15%, T2 25%, T3 15%, T4 20%.

| Option | DVF | T1 | T2 | T3 | T4 | Weighted Total |
|---|---|---|---|---|---|---|
| **S — Submit-streams-default** | 4.67 | 5 | 4 | 5 | 4 | **4.47** |
| **A — Submit-async + status-follow** | 4.67 | 3 | 3 | 4 | 4 | **3.77** |
| **M — Submit-async + dense-status** | 4.33 | 4 | 4 | 4 | 2 | **3.68** |
| **E — One-verb (submit absorbs status)** | 3.33 | 5 | 3 | 3 | 4 | **3.58** |
| **P — `deploy` verb** | 4.33 | 2 | 2 | 3 | 4 | **3.13** |
| **R — Plan/Apply split** | 3.33 | 2 | 2 | 2 | 5 | **2.96** |

### Computation audit (Option S)

`0.25 × 4.67 + 0.15 × 5 + 0.25 × 4 + 0.15 × 5 + 0.20 × 4`
= 1.168 + 0.75 + 1.00 + 0.75 + 0.80
= **4.468 → 4.47**

### Computation audit (Option R, the bottom)

`0.25 × 3.33 + 0.15 × 2 + 0.25 × 2 + 0.15 × 2 + 0.20 × 5`
= 0.833 + 0.30 + 0.50 + 0.30 + 1.00
= **2.933 → 2.93**

---

## Ranking

1. **S — Submit-streams-default** — 4.47 ← winner by 0.70
2. **A — Submit-async + status-follow** — 3.77
3. **M — Submit-async + dense-status** — 3.68
4. **E — One-verb (submit absorbs status)** — 3.58
5. **P — `deploy` verb** — 3.13
6. **R — Plan/Apply split** — 2.96

**Top 3** for the recommendation: S, A, M.

The gap from S to A is 0.70 — a clear winner, not a coin flip. The
gap from A to M is 0.09 — they are effectively tied for runner-up;
the decision between them depends on whether the team is willing to
ship a streaming surface in Phase 1 (A) or refuses streaming entirely
in Phase 1 (M).

---

## Anti-pattern self-audit

Per the skill's "Anti-Patterns in Taste Evaluation" table:

| Anti-pattern | Detection check | This evaluation |
|---|---|---|
| Cherry-picking criteria | Some options evaluated on fewer criteria | All six scored on all four criteria. PASS. |
| Retroactive justification | Scores given after recommendation chosen | Scores were entered before the ranking sort. PASS. |
| Weight manipulation | Weights shifted to favor pre-chosen winner | Weights locked before scoring; T2 raised by 5pp and T4 lowered by 5pp with explicit rationale tied to the user's complaint shape, not to a pre-chosen winner. PASS. |
| "It feels right" override | Recommendation contradicts scores | Recommendation will follow the matrix (see `recommendation.md`). PASS. |
| Feasibility as tie-breaker only | Low-feasibility options kept for aesthetics | E and R are scored honestly; both ranked low. PASS. |

---

## Phase 4 gate verdict — G4 PASS

- [x] DVF filter applied; all surviving above the 6-threshold.
- [x] Weights documented and locked before scoring (T2 +5pp,
  T4 −5pp with rationale).
- [x] All surviving options scored on all four taste criteria.
- [x] Weighted ranking complete, with computation audit on top and
  bottom options.
- [x] Top 3 + dissenting case identified for the recommendation
  (`recommendation.md`).
- [x] Anti-pattern self-audit passed.
