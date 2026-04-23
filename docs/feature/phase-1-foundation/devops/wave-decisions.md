# DEVOPS Wave Decisions — phase-1-foundation

**Wave**: DEVOPS (platform-architect)
**Owner**: Apex
**Mode**: Execute (scope pre-confirmed by user)
**Date**: 2026-04-22
**Status**: COMPLETE — pending peer review

---

## Infrastructure Summary

Phase 1 is a Rust library + CLI + xtask workspace. There is **no
runtime infrastructure**. The DEVOPS wave produces exactly one thing:
the GitHub Actions workflow that gates PRs against `main` with the
checks ADR-0006 specifies, plus a scheduled nightly workflow for
full-workspace mutation trend tracking.

All other typical DEVOPS artifacts (platform architecture, observability
design, monitoring/alerting, infrastructure integration, continuous
learning, KPI dashboards) are skipped — they have nothing to design
against at Phase 1 scope. The six KPIs from DISCUSS are enforced by
`cargo xtask dst`, `cargo xtask dst-lint`, and `cargo test` running in
CI, not by external instrumentation.

---

## The 9 DEVOPS decisions

### Decision 1 — Deployment target

**Question**: What runtime deployment target does Phase 1 ship to?

**Answer**: N/A — library + CLI + xtask; `cargo publish` someday, no
runtime deploy.

**Rationale**: The workspace has no server, no service, no long-running
process. Phase 1's user is a platform engineer running `cargo xtask dst`
locally on their laptop, and CI running the same command on every PR.
There is nothing to deploy.

### Decision 2 — Container orchestration

**Question**: Kubernetes? Nomad? Cloud Run?

**Answer**: None.

**Rationale**: No containers to orchestrate. See Decision 1.

### Decision 3 — CI/CD platform

**Question**: GitHub Actions, GitLab CI, CircleCI, Buildkite?

**Answer**: **GitHub Actions**.

**Rationale**: The repo lives on GitHub (`github.com/overdrive-sh/overdrive`
per workspace Cargo.toml). The existing `.github/workflows/deploy-pages.yml`
already uses GHA for the docs site. Switching CI platforms for a single
new workflow has no upside.

### Decision 4 — Existing infrastructure

**Question**: What CI/CD infrastructure already exists to extend vs
build new?

**Answer**: `.github/workflows/` already contains `deploy-pages.yml`
(static-site deployment on push to `main`). **No PR-gating CI workflow
exists yet** — Phase 1 bootstraps `ci.yml` and `nightly.yml` from
scratch.

Local gates via `lefthook.yml` exist (pre-commit: fmt-check + toml-check
+ yaml-lima; pre-push: `cargo xtask ci`) and MUST be kept coherent with
the new `ci.yml`. The pre-push hook literally references
`.github/workflows/ci.yml` in its comment — this document makes that
reference true.

**Rationale**: The lefthook config is the correct client-side mirror;
the missing piece is the server-side authoritative gate. Adding CI
without disturbing lefthook is the minimum-risk shape.

### Decision 5 — Observability / logging

**Question**: Datadog? Prometheus? OpenTelemetry?

**Answer**: **Deferred to later phases**. Phase 1's only telemetry is:

- `cargo xtask dst` prints the seed on the first line of stdout and a
  structured summary at the end (green or red).
- On failure, a JSON summary artifact (`dst-summary.json`) and full text
  log (`dst-output.log`) are uploaded by CI per ADR-0006.

That is the entire observability surface for Phase 1.

**Rationale**: Per whitepaper §12, real telemetry (eBPF flow events,
DuckLake, LLM correlation) applies to the control-plane process that
Phase 1 deliberately does not ship. Instrumenting `cargo test` with
Prometheus would be design without a problem.

### Decision 6 — Deployment strategy

**Question**: Canary, blue-green, rolling?

**Answer**: N/A — nothing to roll out.

**Rationale**: See Decision 1.

### Decision 7 — Continuous learning

**Question**: LLM-supervised canary analysis, automated remediation?

**Answer**: No.

**Rationale**: This is an LLM observability capability from whitepaper
§12 tied to real traffic. Phase 1 has no traffic.

### Decision 8 — Git branching strategy

**Question**: GitFlow, GitHub Flow, trunk-based, release branching?

**Answer**: **GitHub Flow** — feature branches → PR → `main`.
Trunk-based is aspirational once CI is consistently green.

**Rationale**: See `branching-strategy.md`. GitHub Flow matches the
repo's single-main-branch shape, the per-feature workflow already in
use (current branch `marcus-sa/phase-1-foundation`), and the PR-gated
review culture that `ci.yml`'s required status checks reinforce.

### Decision 9 — Mutation testing strategy

**Question**: When and how does `cargo-mutants` run?

**Answer**: **Per-feature + nightly full-corpus**. Matches
`.claude/rules/testing.md`'s existing mandate exactly:

- Per-PR: `cargo mutants --in-diff origin/main` (gate ≥ 80% kill rate
  on the diff-scoped mutation set).
- Nightly: `cargo mutants --workspace` on a schedule; trend-tracked
  against `mutants-baseline/main/kill_rate.txt`. Drift > 2 pp below
  baseline soft-fails via annotation; absolute < 60% hard-fails as a
  critical regression.

Exclusions (documented in `.cargo/mutants.toml`, to be created by the
crafter when mutants first runs):
- `unsafe` blocks
- `aya-rs` eBPF programs (future Phase 2+ scope; Tier 2/3 covers them)
- Generated code (derives, `build.rs` output, proc-macro expansion)
- Async scheduling logic (`select!`, future polling — DST territory)

**Rationale**: Testing.md already mandates ≥ 80% kill rate on a
per-feature scope with the identical exclusion list. The DEVOPS wave
wires that mandate into CI; it does not invent a new strategy.

**CLAUDE.md append — pending user confirmation**. The task prompt
requires asking the user before writing to project CLAUDE.md. In
subagent mode this is not possible via AskUserQuestion; the strategy is
recorded here as the source of truth and the append is flagged in the
report-back. The exact text the user will be asked to approve is:

> **## Mutation Testing Strategy**
>
> This project uses **per-feature** mutation testing. Per-PR runs are
> diff-scoped via `cargo mutants --in-diff origin/main` with a kill-rate
> gate of ≥80%. A nightly job runs the full workspace against the
> baseline in `mutants-baseline/main/` to catch drift. Mutations to
> `unsafe` blocks, `aya-rs` eBPF programs, generated code, and async
> scheduling logic are excluded per `.claude/rules/testing.md`.

---

## Skipped deliverables

Per the scope-discipline guidance in the user's DEVOPS-wave
instructions, the following skill-template artifacts are explicitly
skipped:

| File | Why skipped |
|---|---|
| `platform-architecture.md` | No platform to architect. Phase 1 is a Rust workspace; platform architecture is the whitepaper (§3-§7, §17) and brief.md. |
| `observability-design.md` | Deferred. No control-plane process exists to observe. |
| `monitoring-alerting.md` | Deferred. No production service; no SLOs to alert on. |
| `infrastructure-integration.md` | Greenfield. No external system to integrate with (brief.md §12 — "no external integrations"). |
| `continuous-learning.md` | N/A. LLM observability is Phase 3+. |
| `kpi-instrumentation.md` | KPIs K1/K2/K3/K5/K6 are enforced by `xtask dst` + `dst-lint` + `cargo test` in CI, not by dashboards. K4 is deferred to Phase 2+ per `upstream-changes.md`. |

Each skipped deliverable returns when the underlying system it describes
exists. `observability-design.md` for example returns in Phase 2 when
the control-plane process first runs.

---

## Handoff to DISTILL — retrospective

DISTILL already ran for phase-1-foundation before DEVOPS (non-canonical
wave order for this feature because test strategy needed to stabilise
before platform decisions). This handoff is therefore retrospective:
DEVOPS confirms that the environment inventory it produces is
consistent with DISTILL's chosen Strategy C (Walking Skeleton §Demo
script; DWD-01).

**Confirmations**:

- **Real redb on `tempfile::TempDir`, not mocks.** The `test` and `dst`
  jobs in `ci.yml` run `cargo test --workspace` and `cargo xtask dst`,
  which exercise the real `overdrive-store-local::LocalStore` against
  redb in a tempdir per WS-1. No test mocks redb.
- **No `@requires_external` markers.** The workflow has no AWS creds,
  no LLM API keys, no paid-service tokens. Every secret the workflow
  needs is a GitHub-native runtime token (GITHUB_TOKEN for artifact
  upload; the default automatic permission).
- **DST runs as a subprocess entry point.** The `dst` job invokes
  `cargo xtask dst` directly — not `cargo test --features dst`. This
  matches WS-1 / WS-2 / WS-3 which all enter through the subprocess
  driver-port wrapper per Mandate-1.
- **K1 wall-clock ceiling enforced.** Job timeout is 10 minutes; the
  K1 target is <60s on a warm laptop. CI runners are slower than an
  Apple Silicon M-class laptop, so the ceiling is a multi-x safety
  margin, not the target.

No DISTILL artifact needs updating based on DEVOPS-wave findings.

---

## Handoff to DELIVER (crafter)

The software-crafter needs the following before DELIVER begins:

1. **`.github/workflows/ci.yml` is the authoritative CI definition.**
   Any future Phase 1 additions (new xtask subcommands, new required
   gates) land as edits here.
2. **`lefthook.yml` and `ci.yml` must stay coherent.** The pre-commit
   and pre-push hooks mirror the CI commit stage. Changes to one require
   the other be updated in lockstep. The lefthook pre-push command is
   `cargo xtask ci`, which is expected to run fmt-check + clippy + test
   — the xtask `ci` subcommand must actually do that (the crafter
   verifies / implements).
3. **`cargo xtask dst` must write to the paths CI expects.** CI reads
   `target/xtask/dst-output.log` and `target/xtask/dst-summary.json`.
   The crafter's implementation of `xtask dst` must write to exactly
   those paths. If `$CARGO_TARGET_DIR` is set, both paths resolve to
   `$CARGO_TARGET_DIR/xtask/...` — CI does not set the variable, so the
   default `./target/` applies.
4. **`cargo xtask dst-lint` must read `package.metadata.overdrive.crate_class`
   per ADR-0003** and fail if any workspace crate is missing the key.
   The dst-lint job in CI treats a non-zero exit as a failure; the
   crafter owns the exit-code contract.
5. **`.cargo/mutants.toml`** — committed at repo root as part of this
   DEVOPS wave. The exclusion list mechanises testing.md §Mutation
   testing (cargo-mutants) "What it's NOT for"; see
   `ci-cd-pipeline.md` §Mutation-testing gate §Exclusions for the
   rationale. The crafter does not need to create this file; updates
   flow through PRs that also update testing.md.
6. **`mutants-baseline/main/kill_rate.txt`** — the nightly workflow
   seeds this file on its first run. The parent directory is committed
   with `mutants-baseline/main/.gitkeep` so the first nightly run has a
   place to write; without the placeholder, the `if-no-files-found:
   warn` setting on the artifact-upload step would silently swallow
   the trend-tracking signal. Do not delete the `.gitkeep`.

---

## Quality gates

- [x] CI workflow gates the five required checks on every PR
- [x] DST seed surfaced on first line of stdout + as GHA annotation on
      failure + in job summary (ADR-0006)
- [x] Text log + JSON summary uploaded as artifacts on every DST run
      (ADR-0006 `if: always()`)
- [x] Mutation testing diff-scoped per PR; nightly full-corpus
      separately; trend tracking against baseline
- [x] Concurrency group cancels stale runs on force-push
- [x] Branch-protection expectations documented (user-action to
      configure on GitHub Settings)
- [x] No Tier 2/3/4 noise — Phase 1 runs Tier 1 only
- [x] Skipped deliverables listed with reason
- [ ] Peer review by `@nw-platform-architect-reviewer` — **pending**;
      runs after this file is written

---

## Peer review

Reviewer verdict recorded below after dispatch.

| Iteration | Date | Verdict | Blockers addressed |
|---|---|---|---|
| 1 | 2026-04-22 | CONDITIONALLY_APPROVED | 3 blockers + 1 nitpick addressed in iter-2 (see Changelog 2026-04-22 second entry). |

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-22 | Initial DEVOPS wave decisions for phase-1-foundation. |
| 2026-04-22 | Review iter-1 fixes: (1) created `mutants-baseline/main/.gitkeep` so the nightly trend-tracking target exists in a fresh clone; (2) created `.cargo/mutants.toml` with the exclusion list from `.claude/rules/testing.md` (unsafe, aya-rs / overdrive-bpf, async scheduling, generated code, tests/benches); (3) added §Branch-protection setup recipe to `ci-cd-pipeline.md` with the exact five `gh api` required-check names mapped to `ci.yml` jobs and a `jq`-based verify step; (4) removed redundant job-level `PROPTEST_CASES` override in `ci.yml` (workflow-level declaration is the single source of truth). |
