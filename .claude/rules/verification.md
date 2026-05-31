# Verification Guidelines — Expectation-Driven Development (EDD)

Overdrive runs an **executed-evidence catalogue** at `verification/` for
operator-observable and qualitative behaviour. This file is the *discipline*
that governs it; `verification/README.md` is the *operational SSOT* (the three
governing rules, the status legend, the surface taxonomy, the harness
invocation). Read that first — this file does not repeat it; it pins **when**
the catalogue is touched in the wave lifecycle and **what** a reviewer rejects.

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

## How this slots into nWave

The catalogue is touched at three points in the wave lifecycle, and audited at
a fourth. These are enforced — an agent working a wave that skips its
verification obligation is incomplete, the same way a DELIVER step that skips
its mutation run is incomplete.

### DISTILL — author expectations

Expectations are authored during DISTILL. You already do this in
`docs/feature/{slug}/distill/test-scenarios.md` (the GIVEN/WHEN/THEN
specification companion). The **operator-surface and end-to-end** scenarios —
tagged `@driving_port` / `@walking_skeleton`, and anything whose correctness is
*qualitative* ("the error is actionable, not cryptic"; "the render is honest")
— **graduate into `verification/expectations/`** as `O`- and `E`-surface
expectations. In-process logic scenarios (`@in-memory`, reconciler purity, wire
roundtrips) stay in the test tiers and do **not** graduate; duplicating them in
the catalogue is noise.

The acceptance designer (`nw-acceptance-designer`) is responsible for marking
which scenarios graduate. A DISTILL artifact that produces operator-surface
scenarios with no corresponding `verification/expectations/<ID>/` stub is
incomplete.

### DELIVER / DEVOPS — capture evidence

Evidence is captured here, against the **built `overdrive` binary**, during
DELIVER (per-slice, as the operator surface lands) or DEVOPS (production
readiness). Capture runs through `verification/harness/run-expectation.sh
<ID>`, which executes the expectation's `runner.sh` inside Lima and pins the
SHA + dirty state + DST seed + harness SHA.

The crafter does not hand-write evidence files. The crafter writes (or extends)
the expectation's `runner.sh` to drive the real binary, runs the harness, and
sets the status in the expectation's `README.md` **after** an adversarial read
of the captured output. A slice that lands an operator surface without
capturing or updating its expectation's evidence is incomplete.

### FINALIZE — archive the catalogue

At FINALIZE the feature's expectations + evidence archive into
`docs/evolution/{slug}/verification/` alongside the rest of the feature's
lasting artifacts. The archived catalogue is the feature's **"what does it do,
and how do we know?"** record — it answers the six-months-later question
("does the system handle X correctly?") with a pinned expectation and its
concrete proof, not a grep through test files. `nw-finalize` migrates it; a
FINALIZE that drops the verification catalogue on the floor loses the evidence
trail the feature was built to produce.

### The "different fox" audit — review evidence, never code

Adversarial evidence review is dispatched to a **`*-reviewer` agent (Haiku)**
or a **small adversarial-verify Workflow**, pointed at the *evidence*, never at
the code that produced it. This is the structural defense against the
fox-guarding-the-henhouse failure: the same agent (and the same reasoning flaw)
that wrote the code will overlook the bug in its own evidence. A *different*
agent, reading only the captured `evidence/` and the expectation's anchor,
breaks that loop.

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
  that wrote the implementation and the runner, with no different-fox review
  recorded. Bounce it to the audit.
- **Stale evidence treated as live.** Evidence captured at an old SHA, cited
  after the operator surface changed. Per `.claude/rules/debugging.md` §
  "Refresh measurements when source changes" — re-run the capture at current
  HEAD before relying on it. The catalogue is a snapshot; a snapshot of the
  wrong commit is misleading, not reassuring.
- **A test scenario duplicated as an expectation.** In-process logic that the
  test tiers already cover, copy-pasted into `verification/`. The catalogue is
  for the operator/qualitative slice the tiers under-serve; duplication dilutes
  the signal.
- **Crate dependency in the catalogue.** `verification/` is black-box — it
  drives the built binary and observes surfaces (CLI output, `bpftool map
  dump`, `ss -K`, observation rows). A `runner.sh` that imports or links an
  `overdrive-*` crate has become a fifth test tier and forfeited the
  independence that makes the evidence worth trusting. Reject it.

---

## Stabilize — when an expectation earns a test

The catalogue is design-time and acceptance-time; it is not a regression alarm.
For **critical paths**, convert the expectation + evidence into an automated
test (the captured scenario gives you the inputs and expected outputs for
free — you make it deterministic and rerunnable). The expectation stays as the
`why`; the test becomes the `what, forever`. This is the answer to "is the
expectation rerunnable?" — the seed-pinned Lima capture reproduces with
`SEED=<N>`, and the stabilized test fails loudly in CI when the surface drifts.
An expectation guarding a KPI (K1..K5) or a known-incident regression (an
`RCA-*` guard) is a stabilize candidate by default.

---

## Cross-references

- `verification/README.md` — operational SSOT: the three governing rules,
  status legend, surface taxonomy (`O`/`R`/`D`/`E`/`X`), harness invocation.
- `.claude/rules/testing.md` — the four test tiers; the executed-evidence
  catalogue is a complement, and the Lima-only execution discipline is shared.
- `.claude/rules/debugging.md` § "Refresh measurements when source changes" —
  why stale evidence is a load-bearing-premise hazard.
- `CLAUDE.md` § "Deferrals require GitHub issues" — the same anchor discipline,
  applied to forward pointers rather than evidence claims.
