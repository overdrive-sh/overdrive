<!-- markdownlint-disable MD013 MD024 -->

# Overdrive Verification — Expectation-Driven Development (EDD) catalogue

A **black-box** evidence catalogue for operator-observable and qualitative
behaviour. It sits beside the code; it does **not** contain or link any
`overdrive-*` crate. Each expectation is a plain-text statement of intent
paired with **executed evidence** — real output captured from the built
`overdrive` binary running on a real kernel (Lima), pinned to a commit and a
DST seed.

## What this is — and is NOT

This is **not** a fifth test tier. Overdrive already has a stronger
regression net than this catalogue aspires to: the four tiers in
`.claude/rules/testing.md` (DST + seed reproduction, proptest, trybuild,
80% mutation gate). Those prove the *code* is correct in-process and **fail
loudly forever**.

EDD covers the gap those tiers under-serve, exactly where the methodology
wins (see `docs/research/` / the EDD essay):

- **Operator-observable behaviour** — what `overdrive` the CLI actually
  prints on `submit → converge → serve`. Tier 1 proves reconciler logic; it
  does not produce a human-readable proof that the *operator surface*
  behaves.
- **Qualitative expectations** — "the cgroup-preflight error names the
  actual cause and the actual fix", "the Service-honest render never claims
  `(took live)` for a workload that exited". These are real requirements
  that have no home in an `assert!` (cf. `development.md` § "Distinct
  failure modes get distinct error variants" — a pile of qualitative
  expectations).
- **Systemic / end-to-end** — "this holds end-to-end through a real kernel",
  captured as pinned evidence, not just a green CI line.

> An EDD expectation is the natural-language **`why`**. A Tier 3 / Lima test
> is the **`what, forever`**. The expectation is a **snapshot** pinned to a
> SHA — it is silent when it goes stale. That is why critical expectations
> get **stabilized** into automated tests (Step 6 below); the test is the
> regression alarm, the expectation is the design-time conversation.

## Governing rules

1. **No claim without a verifiable pointer.** An expectation is `satisfied`
   only when its evidence points to one of: a `path:line` citation from the
   Overdrive tree quoted verbatim; a command we ran with stdout/stderr saved
   verbatim under `evidence/`; a test we ran with the exact invocation and
   full output saved. **Executed evidence, not narration.** "I'm pretty sure
   it would work like this" is a second assertion by the same agent that
   made the first — it is not evidence.

2. **Pin everything.** Every verification records: the Overdrive commit SHA,
   working-tree dirty state (diff preserved), the DST seed used, the date,
   and the harness invocation. Reproduce with the same SHA + `SEED=<N>`.

3. **Claims require external anchors.** Every expectation references an
   independent contract that **predates** verification — a `S-*` scenario
   ID, an ADR, a `wave-decisions.md` entry, or a roadmap acceptance
   criterion. A passing claim with no such anchor is tagged
   `unanchored-claim` regardless of pass/fail (same discipline CLAUDE.md
   applies to deferrals: no hand-wavy forward pointers).

## "Executed, not narrated" — the Overdrive guarantee

Evidence runs through `cargo xtask lima run --` (a real kernel + cgroup v2),
never bare on the macOS host. A macOS-host capture resolves
`#[cfg(target_os = "linux")]` differently and is narration, not execution.
The harness refuses to mark evidence `satisfied` from a non-Lima run.

## Observable surfaces

| Tag | Surface |
|---|---|
| **O** | Operator CLI (`overdrive` binary) — submit / alloc status / serve |
| **R** | Reconciler / control-plane convergence (observable via observation store) |
| **D** | Dataplane / kernel-observable (`bpftool map dump`, `ss -K`, `tcpdump`) |
| **E** | End-to-end (submit → converge → serve, through a real deployment) |
| **X** | Build / supply chain (xtask gates, image provenance) |

## Status legend

| Status | Definition |
|---|---|
| `pending` | Not yet verified |
| `satisfied` | Verified with complete executed evidence (Lima run, pinned) |
| `partial` | Multiple sub-claims; some pass, some fail; issue linked |
| `broken` | Regression from a prior `satisfied`; issue linked |
| `unanchored-claim` | Passes but lacks an external contract anchor |
| `out-of-scope` | Deliberately removed; reason documented |

## Directory layout

```
verification/
  README.md                         # this file
  expectations/
    INDEX.md                        # master status table
    <SURFACE><NN>-<slug>/
      README.md                     # scenario + anchor + verification block + status
      runner.sh                     # optional per-expectation driver (real commands)
      evidence/                     # pinned: verification.yaml, verbatim stdout/stderr, lima logs
  issues/
    INDEX.md                        # open/closed tracker
    <NNN>-<slug>.md                 # one per failed expectation
  harness/
    run-expectation.sh              # pins SHA+seed+dirty, runs runner.sh in Lima, captures evidence
    lima-helpers.sh                 # `od` (CLI-in-Lima) + `capture` helpers for runner.sh
```

## Running a verification

```bash
verification/harness/run-expectation.sh O03            # default SEED=1
SEED=42 verification/harness/run-expectation.sh E01     # pin a different seed
```

The runner pins commit + dirty state + seed, executes the expectation's
`runner.sh` through Lima, captures verbatim output to `evidence/`, validates
the anchor, and writes `evidence/verification.yaml`. It does **not** fabricate
evidence — absent a `runner.sh` it records `pending` and tells you manual
capture is required.

## How this slots into nWave

- **DISTILL** — expectations are authored (you already do this in
  `docs/feature/{slug}/distill/test-scenarios.md`; the `O`/`E` operator-surface
  ones graduate into this catalogue).
- **DELIVER / DEVOPS** — evidence is captured here against the built binary.
- **FINALIZE** — the catalogue archives into `docs/evolution/{slug}/` as the
  feature's "what does it do, and how do we know?" artifact.
- **The "different fox" audit** — adversarial evidence review is dispatched to
  a `*-reviewer` agent (Haiku) or a small adversarial-verify Workflow against
  the *evidence*, never the code that produced it.
