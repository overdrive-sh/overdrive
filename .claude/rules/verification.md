# Verification Guidelines — Expectation-Driven Development (EDD)

Overdrive runs an **executed-evidence catalogue** at `verification/` for
operator-observable and qualitative behaviour. This file is the EDD discipline
that governs its meaning and lifecycle. `verification/README.md` documents the
catalogue layout and the existing harness; read it when using that machinery,
but harness mechanics cannot turn a point-in-time expectation into a recurring
test. This rule pins **when** the catalogue is touched in the wave lifecycle
and **what** a reviewer rejects.

EDD is **not** a test tier. The four tiers in `.claude/rules/testing.md`
(DST + seed reproduction, proptest, trybuild, the 80% mutation gate) prove the
*code* is correct in-process and **fail loudly forever**. The verification
catalogue proves the *operator surface* behaves and captures the qualitative
expectations no `assert!` holds — and it is a **snapshot pinned to a SHA**,
silent when stale. The two are complements, not substitutes. An expectation is
the natural-language **`why`**; a Tier 1/Tier 3 test is the **`what,
forever`**. If you find yourself reaching for the catalogue to get regression
protection, you want a test instead — and the catalogue's `Stabilize` step
(below) is how you get there.

---

## Boundary: expectation, E2E test, and benchmark

An EDD expectation answers: **“what did this feature claim, and what did we
actually observe at delivery?”** It records one point-in-time capture with one
or a small number of deliberately contrasting cases. It does not establish a
success rate, exercise a large matrix, or measure a distribution.

An E2E/conformance test answers: **“does the assembled system continue to
satisfy this deterministic contract?”** It is automated and rerunnable in the
appropriate test lane. A repeated cohort that checks correctness or reliability
is an E2E, conformance, stress, or soak test—not an expectation.

A benchmark answers: **“how fast, how much, or how variable is the system
under this declared profile?”** It requires repeated samples, a controlled
substrate, raw measurements, and statistical methodology. It is neither a
correctness test nor an expectation.

In particular, launching hundreds or thousands of microVMs to see whether they
work, to find a failure rate, or to measure latency is never an EDD expectation.
Classify it as the relevant test or benchmark and retain only a concise feature
claim plus point-in-time receipt in this catalogue.

---

## How this slots into nWave

An EDD expectation is a point-in-time feature-verification record, not a test
gate. A feature team may author one during DISTILL, capture executed evidence
when the feature is ready to verify, retain the reviewed record at FINALIZE,
and decide whether any critical part merits a separately maintained regression
test. The absence of an expectation is not equivalent to a skipped mutation or
test run; a feature's accepted design determines whether an EDD record is
needed for its operator-visible, qualitative, or systemic claim.

### DISTILL — author expectations

Expectations are authored in natural language during DISTILL, **before
implementation evidence exists**. They state the coherent feature claim, its
meaningful edge cases and non-effects, and what would count as an observable
refutation. They may be anchored to a user story, scenario, ADR, wave decision,
or roadmap acceptance criterion, but they are not test scenarios that
"graduate" into another format.

Operator-visible, qualitative, and systemic claims are the natural candidates:
for example, an error is actionable rather than cryptic, a rendered status is
honest, or a real-kernel deployment exhibits the intended operator journey.
Pure in-process properties normally belong only in the test tiers. The
acceptance designer and feature reviewers decide which claims need an EDD
record; they do not create a catalogue stub merely because a scenario has an
operator-facing tag.

### DELIVER / DEVOPS — capture evidence

Evidence is captured against the **built `overdrive` binary** during DELIVER
or DEVOPS, when the feature team makes its point-in-time verification claim.
The existing `verification/harness/run-expectation.sh <ID>` and a per-
expectation `runner.sh` are useful capture aids when their environment matches
the claim, but neither is what makes an expectation valid. A one-time,
well-recorded command sequence and its retained real output can be sufficient;
do not manufacture a permanent runner merely to make an expectation look like
a test.

The implementation author does not hand-write successful evidence or set an
expectation to `satisfied`. They retain the actual commands, output, substrate,
SHA, dirty state, and any relevant seed; a different reviewer then evaluates
the expectation and the captured evidence. An expectation capture is one
feature-verification event, not an obligation to add a recurring DELIVER gate.

### FINALIZE — retain the canonical catalogue

At FINALIZE, expectations and their evidence remain at their permanent
repository-root home: `verification/expectations/<ID>/`. Do **not** copy or
move them into `docs/evolution/{slug}/verification/`: that creates competing
snapshots and obscures the canonical point-in-time record. The evolution record
links to the relevant expectation(s) and summarizes their status;
`verification/` remains the "what did this feature claim, what was observed,
and how do we know?" record.

### The "different fox" audit — review evidence, never code

Adversarial evidence review is dispatched to a **`*-reviewer` agent (Haiku)**
or a **small adversarial-verify Workflow**, pointed at the expectation and its
captured evidence rather than at implementation code as a substitute for
evidence. This is the structural defense against the fox-guarding-the-henhouse
failure: the same agent (and the same reasoning flaw) that wrote the code will
overlook the bug in its own evidence. A *different* agent asks whether the
actual capture refutes or supports the expectation's claim.

The audit prompt is adversarial by construction — "try to refute that this
evidence satisfies the expectation; default to refuted if the capture is
narrated rather than executed, if the numbers don't add up, or if a sub-claim
was dodged." The adversarial-verify Workflow shape (N independent skeptics per
expectation, kill if a majority refute) is the same pattern the review
discipline uses elsewhere; reuse it here against `verification/` evidence. Do
**not** let the authoring agent stamp its own expectation `satisfied`.

---

## Enforcement — what a reviewer rejects

- **Narrated evidence.** A capture that describes what the agent *believes*
  would happen rather than real `stdout`/`stderr` from a command that ran.
  `verification.yaml` with `executed_in_lima: false` cannot back a `satisfied`
  status. "I'm pretty sure it works like this" is a second assertion by the
  same entity that made the first — reject it.
- **Unanchored claims.** An expectation `README.md` with no `- Anchor:` line,
  or an anchor that does not resolve to a real `S-*` scenario / ADR /
  `wave-decisions.md` entry / roadmap AC that *predates* verification. Tag
  `unanchored-claim` regardless of pass/fail — identical discipline to
  CLAUDE.md's "deferrals require a real issue number; no hand-wavy forward
  pointers."
- **Self-audited `satisfied`.** A status set to `satisfied` by the same agent
  that wrote the implementation or captured the evidence, with no
  different-fox review recorded. Bounce it to the audit.
- **Historical evidence treated as a claim about HEAD.** An expectation is
  satisfied only for its recorded SHA and environment. A later source change
  does not invalidate or require re-running that historical feature-verification
  record; it means the record must not be cited as proof of the later state. If
  the team needs a claim about that later state, it captures a new expectation
  event (or relies on a stabilized test), without overwriting the old receipt.
- **A test scenario duplicated as an expectation.** In-process logic that the
  test tiers already cover, copy-pasted into `verification/`. The catalogue is
  for the operator/qualitative slice the tiers under-serve; duplication dilutes
  the signal.
- **A cohort or benchmark disguised as an expectation.** Hundreds or thousands
  of VM launches, a repeated failure-rate/matrix exercise, or any collection of
  latency, throughput, resource, percentile, or distribution measurements.
  Reclassify it as an E2E/conformance/stress test or a benchmark; retain only a
  concise point-in-time feature receipt here.
- **Crate dependency in a capture.** `verification/` is black-box — a capture
  drives the built binary and observes surfaces (CLI output, `bpftool map dump`,
  `ss -K`, observation rows). Any capture tool that imports or links an
  `overdrive-*` crate has become a fifth test tier and forfeited the
  independence that makes the evidence worth trusting. Reject it.

---

## Stabilize — when an expectation earns a test

The catalogue is design-time and acceptance-time; it is not a regression alarm.
An expectation is **not meant to be rerun**. Its pinning records what was
actually verified at one feature point in time; a saved command or seed can aid
forensic investigation, but does not turn the expectation into a test.

For **critical paths**, convert the expectation + evidence into a separately
maintained automated test (the captured scenario supplies candidate inputs and
outcomes; the test must be made deterministic and rerunnable). The expectation
stays as the `why`; the test becomes the `what, forever` and fails loudly in CI
when the surface drifts. An expectation guarding a KPI (K1..K5) or a
known-incident regression (an `RCA-*` guard) is a stabilize candidate by
default.

---

## Cross-references

- `verification/README.md` — catalogue layout, status legend, surface taxonomy
  (`O`/`R`/`D`/`E`/`X`), and existing harness invocation.
- `.claude/rules/testing.md` — the four test tiers; the executed-evidence
  catalogue is a complement, and the Lima-only execution discipline is shared.
- `.claude/rules/debugging.md` § "Refresh measurements when source changes" —
  why stale evidence is a load-bearing-premise hazard.
- `CLAUDE.md` § "Deferrals require GitHub issues" — the same anchor discipline,
  applied to forward pointers rather than evidence claims.
