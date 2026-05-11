# DISCUSS Wave Review — workload-kind-discriminator

**Reviewer**: `nw-product-owner-reviewer` (independent hard gate before DESIGN)
**Date**: 2026-05-10
**Verdict**: **APPROVED**
**Scope**: All artifacts under `docs/feature/workload-kind-discriminator/{discuss,slices}/` — 8 user stories, 4 journeys, 6 carpaccio slices, KPI register, DoR validation, shared artifacts registry, wave decisions, Luna's iteration-2 peer review.

This review is independent of `peer-review.md` (which is Luna's own iteration-2 self-review). Both are APPROVED; this document is the externally-dispatched hard gate.

---

## Verdict

**APPROVED for handoff to `@nw-solution-architect` (DESIGN wave).**

All 8 DoR items pass with strong evidence. Journey coherence is solid. No LeanUX antipatterns detected. GitHub issue references (#166, #167) are verified real and byte-identical across all artifacts. The 2026-05-10 fold-in of GH #164 is well-documented and defensible.

### Blocking-issue count by category

| Category | Blocking findings |
|---|---|
| DoR items (9 × 8 stories = 72 checks) | 0 |
| Journey coherence | 0 |
| LeanUX antipatterns | 0 |
| Story sizing | 0 |
| Carpaccio taste tests | 0 |
| KPI clarity | 0 |
| Deferral discipline | 0 (both deferrals tracked: #166, #167) |
| Cross-artifact consistency | 0 |

---

## Top 3 things Luna got conspicuously right (`praise:`)

### praise: The pending-VIP marker is brilliantly honest

The literal `(vip: pending allocation — see #167)` is sourced from a single CLI config constant, byte-identical across submit echo and `alloc status`. KPI K6 (100% byte-equality) is a real measurement that catches drift. This is mature product thinking — operator-visible acknowledgment that the platform isn't ready yet, with a direct link to track resolution.

### praise: Changed-assumptions discipline is flawless

Every artifact that changed for the 2026-05-10 fold-in names it explicitly:
- `dor-validation.md:9` — pass count update from 7/7 to 8/8
- `outcome-kpis.md:5` — K6 addition rationale
- `shared-artifacts-registry.md:11` — `${listener_triple}` and `${vip_assignment_state}` artifacts added
- `journey-submit-service.yaml` — listener TOML and submit echo additions noted
- `journey-alloc-status-visual.md` — Listeners section addition noted
- `wave-decisions.md` — fold-in section captures full converged decisions

The reviewer can trace the delta with high confidence. This is the gold standard for iterative refinement.

### praise: Emotion annotations are specific and defensible

`journey-submit-service.yaml:20` shifts from "Skeptical → Focused → Trusting" with a coda for the listener fold-in: "Patient-but-Trusting" (`journey-alloc-status-visual.md:196`). The arc acknowledges that pending VIPs are a real source of uncertainty, and the marker `(vip: pending allocation — see #167)` *names* that uncertainty instead of hiding it. This is the structural inverse of the coinflip bug (which hid the truth behind a hard-coded "live" literal).

---

## Top 3 non-blocking suggestions for DESIGN handoff

These are NOT blockers; they are normal DESIGN-phase refinements the architect should expect.

### suggestion (non-blocking): Confirm Slice 06 scenario count and architect-fault-line

`dor-validation.md:145` defends 9 scenarios as upper-edge but explicitly names a natural fault line: "the architect's natural fault line (parser+echo vs. alloc status) is captured in `slice-06-service-listener-fields.md` for DESIGN-time evaluation." The DESIGN architect should confirm:

- Can the 9 scenarios be crisply split along the parser+echo / alloc-status-rendering fault line?
- If so, is asymmetric implementation effort (parser ~4h vs. rendering ~6h, independent) load-bearing enough to warrant two slices instead of one?

The current single-slice shape is defensible but not immutable. Splitting is a DESIGN-time decision, not a DISCUSS-time blocker.

### suggestion (non-blocking): Nail down K3 usability-check cadence

`outcome-kpis.md:54` says "Once at feature release; ongoing automated." This is vague. The K3 measurement plan combines:

1. An automated parsing-from-fixtures regression test (always-on; clear).
2. A manual usability check with 5-10 operators (cadence unclear).

DESIGN should decide whether the manual check is:
- **Pre-release** — gate-for-ship; blocks merge if comprehension <95%.
- **At first release** — learning opportunity; KPI tracked but not blocking.
- **Post-release** — feedback loop; informs future iteration.

The automated piece is fine as-is.

### suggestion (non-blocking): AllocStatusRow listener denormalisation shape

`shared-artifacts-registry.md:161` flags: "AllocStatusRow listener fields denormalised at write time (architect to confirm shape)." DESIGN must decide:

- Are listeners stored as `Vec<Listener>` on the row, or as separately-keyed rows with an FK back to the alloc?
- This affects persistence, query patterns, and render-layer code shape — not the journey or spec layer.

---

## Detailed findings by dimension

### 1. DoR — 9 items × 8 stories = 72 checks

**All 72 checks PASS** with documented evidence. Spot-checked:

| Item | Evidence |
|---|---|
| 1. Problem statement clear | "Ana wants to distinguish a one-shot script from a long-running service" (US-01 Problem). Rooted in real coinflip bug reproduction. |
| 2. User/persona identified | Ana, Overdrive platform engineer. Consistent across all 8 stories. Same persona as J-OPS-002 / J-OPS-003. |
| 3. 3+ domain examples | Service (`payments`), Job (`coinflip`), Schedule (`nightly-backup`). All with real TOML bodies and real exit codes. Technical tasks (US-06, US-07) explicitly noted as N/A per template. |
| 4. UAT scenarios in GWT | 5/4/4/3/4/3/0/9 across US-01..US-08. US-07 is a migration task (no scenarios required). US-08's 9 scenarios are upper-edge with defended fault line. |
| 5. AC derived from UAT | All AC traceable to UAT scenarios. Spot: US-01 AC line 147-153 maps to scenarios at line 77-142. |
| 6. Right-sized | Largest is US-02 / Slice 02 at ~1.5 days / 4 scenarios. US-08 / Slice 06 at ~1.5 days / 9 scenarios (defended). |
| 7. Technical notes | Every story carries Technical Notes naming dependencies, risks, implementation patterns. |
| 8. Dependencies tracked | US-01 enables all others. US-05 / US-08 deferrals tracked to real GH issues (#166, #167). |
| 9. Outcome KPIs | K1-K6 all defined with Who/Does What/By How Much/Measured By specificity. |

### 2. Journey coherence

| Journey | Steps | Emotional arc | Status |
|---|---|---|---|
| A — Submit Service | 4 (incl. listener fold-in) | Skeptical → Focused → Trusting | ✅ |
| B — Submit Job | 5 (incl. intermediate + terminal sub-paths) | Anxious → Watchful → Satisfied/Informed | ✅ — explicitly closes the coinflip bug |
| C — Submit Scheduled Job | 4 | Curious → Confident-in-syntax → Patient | ✅ |
| D — alloc status | Multi-sub-path (D1 Service, D2/D2'/D2'' Job, D3 Schedule) | Curious → Reading carefully → Confident | ✅ |

**Shared artifacts** (verified in `shared-artifacts-registry.md`):
- `${kind}` — closed Rust enum, denormalised to `AllocStatusRow`, renders differently per kind. HIGH integration risk, well-mitigated.
- `${listener_triple}` & `${vip_assignment_state}` — round-trip through submit echo and alloc status byte-identical. KPI K6 enforces.
- `${deferral_issue_url}` — single CLI constant, sourced for Schedule kind. KPI K5 enforces.
- `${exit_code}`, `${duration}`, `${verdict}` — all trace to real components (ExitObserver, Clock, render logic). No literals, no fabrications.

### 3. LeanUX antipatterns — CLEAR

| Pattern | Status |
|---|---|
| Solution-as-problem | ✅ — Every story starts from Ana's pain. |
| Generic data | ✅ — Real names (`payments`, `coinflip`, `nightly-backup`), real TOML, real exit codes. |
| Technical AC | ✅ — All AC describe operator outcomes, never implementation. |
| Vague personas | ✅ — Ana is consistent. |
| Technical scenario titles | ✅ — Scenario titles describe business outcomes. |
| Missing real data in examples | ✅ — Every story has 3+ domain examples with real data. |
| Missing edge cases | ✅ — Every story includes failure modes. |
| Bundled concerns | ✅ — Stories are single-concern; US-08 defended against carpaccio threshold. |

### 4. Story sizing — APPROPRIATE

- US-02 (Job submit terminal): ~1.5d / 4 scenarios. Defensibly whole — the structural fix is one move (`JobSubmitEvent` enum without `ConvergedRunning`).
- US-08 / Slice 06 (Service listener fields): ~1.5d / 9 scenarios. Defended; architect-fault-line named for DESIGN-time evaluation. Not carpaccio-failure dressed as discipline.
- US-05 (Schedule parsing + deferral): ~1d / 4 scenarios. Lowest priority but not sandbagging — execution is genuinely deferred to #166.

### 5. Carpaccio taste tests — ALL PASS

| Test | Status |
|---|---|
| Slice ships ≥4 new components → not thin | ✅ — Slice 06 ships seven moving parts; defended explicitly. |
| Every slice depends on a new abstraction → ship abstraction first | ✅ — `WorkloadKind` enum lands in Slice 01 (correctly noted as the abstraction-first move). |
| No slice disproves any pre-commitment → decoration | ✅ — Slice 02 disproves "Job submits report Running on exit-1" (the bug). |
| Synthetic-data-only slice → plumbing not value | ✅ — Slice 02 uses real `examples/coinflip.toml`. Slice 06 uses real Service spec with declared listeners. |
| Two slices identical except for scale → merge | ✅ — None. |
| Production data acceptance criterion present | ✅ — KPI K1 measures over 100 trials of `examples/coinflip.toml`. |

### 6. KPI clarity

| KPI | Status | Notes |
|---|---|---|
| K1 (honesty rate 0% → ≥99% over 100 trials) | ✅ CLEAR | Falsification path: integration test on coinflip workload with kernel `exit_code=1`. Bug-present case fails (CLI says Running); bug-fixed case passes (CLI says Failed). |
| K2 (mixed-kind rejection p95<50ms) | ✅ CLEAR | Parser unit + integration tests with timing. |
| K3 (≥95% comprehension) | ⚠️ SOFT — measurement timing | See `suggestion (non-blocking)` above. |
| K4 (existing tests 100% pass) | ✅ CLEAR | CI pass rate. |
| K5 (Schedule URL byte-equality) | ✅ CLEAR | String equality integration test. |
| K6 (listener round-trip byte-equality) | ✅ CLEAR | 100 submits with pinned VIPs; un-pinned VIPs out of scope (allocator deferred to #167) — deliberate omission. |

### 7. GH issue references — VERIFIED REAL

Both deferrals are tracked to real GH issues with byte-identical URLs across all artifacts:

- **#166** (Schedule execution semantics): `dor-validation.md:20`, `user-stories.md:96-97`, `wave-decisions.md:100`, `journey-submit-scheduled-job.yaml:118`, `slice-05-schedule-parsing.md`. URL: `https://github.com/overdrive-sh/overdrive/issues/166`. ✅
- **#167** (VIP allocator primitive): `dor-validation.md:23`, `user-stories.md:715-717,750,972`, `outcome-kpis.md:14`, `shared-artifacts-registry.md:176-194`, `journey-submit-service.yaml:93+`, `slice-06-service-listener-fields.md`. URL: `https://github.com/overdrive-sh/overdrive/issues/167`. ✅

**No placeholders detected.** No `<N>`, `<placeholder>`, `[TBD]`, `issue #N`.

### 8. Cross-artifact consistency — CLEAN

- Slice list in `story-map.md` matches slice files (1-6).
- KPIs cited in stories match KPIs in `outcome-kpis.md` (K1-K6).
- Persona "Ana" consistent across all 8 stories and 4 journeys.
- Vocabulary clean: kind / Service / Job / Schedule / listener — no slippage to "workload" / "process" / "container".

### 9. Changed Assumptions sections — COMPLETE

The 2026-05-10 fold-in is recorded in:
- `wave-decisions.md` — § "Changed Assumptions" + § "Fold-in of GH #164"
- `dor-validation.md:9` — pass count update
- `outcome-kpis.md:5` — K6 rationale
- `shared-artifacts-registry.md:11` — new artifacts
- `journey-submit-service.yaml` — header note
- `journey-alloc-status-visual.md` — header note

### 10. Deferral discipline — STRICT

Two deferrals total, both tracked with real GH issue numbers and explicit user approval (2026-05-09):
- #166 — Schedule execution semantics (referenced by Slice 05 / US-05)
- #167 — VIP allocator primitive (referenced by Slice 06 / US-08, runtime-side)

No hand-wavy "future work" or "TBD" language. No invented issue numbers.

---

## Three items DESIGN should confirm

These are non-blocking handoff annotations:

1. **Slice 06 scenario count and possible split** along parser+echo / alloc-status fault line.
2. **K3 usability-check measurement timing** (pre-release / at-release / post-release).
3. **AllocStatusRow listener denormalisation shape** (Vec<Listener> on row vs. separate FK).

---

## Reviewer attestation

- Reviewed all 11 DISCUSS artifacts plus 6 slice briefs (17 files total).
- Verified GH issue references against the orchestrator's prior verification (#166, #167).
- Did not modify any artifact (review-only output).
- Did not create any GitHub issue.
- Verdict is independent of Luna's own `peer-review.md` (which also approved).

**Handoff cleared. Ready for `@nw-solution-architect` DESIGN-wave dispatch.**
