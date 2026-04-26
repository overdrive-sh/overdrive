# DEVOPS Wave Decisions — phase-1-control-plane-core

**Wave**: DEVOPS (platform-architect)
**Owner**: Apex
**Mode**: Execute (scope + decisions pre-confirmed by user in task prompt)
**Date**: 2026-04-23
**Status**: COMPLETE — pending peer review by `nw-platform-architect-reviewer`.

---

## Infrastructure Summary

phase-1-control-plane-core extends phase-1-foundation's CI foundation
with a REST+OpenAPI server, a reconciler primitive runtime, and a
functional CLI client. There is **still no runtime infrastructure** —
the walking skeleton runs end-to-end on a contributor laptop against
a `tempfile::TempDir`. This DEVOPS wave produces exactly three
artifacts:

1. `environments.yaml` — retroactive environment inventory filling the
   DISTILL DWD-12 graceful-degradation gap.
2. `ci-cd-pipeline.md` — the **delta** over phase-1-foundation's
   existing `.github/workflows/ci.yml` + `nightly.yml`. One new
   required CI check (`openapi-check`); existing jobs absorb the rest
   of the new material automatically.
3. `kpi-instrumentation.md` — `tracing` span / structured-field design
   per outcome KPI (K1–K7). No dashboards, no alerting, no DuckLake
   ingestion pipeline. Instrumentation-ready for Phase 2.

All other typical DEVOPS artifacts (`platform-architecture.md`,
`monitoring-alerting.md`, `observability-design.md`,
`infrastructure-integration.md`, `continuous-learning.md`,
`branching-strategy.md`) are **skipped** — either already owned by
phase-1-foundation (branch strategy) or not applicable at Phase 1
scope (monitoring, alerting, integration).

---

## Inherit-from-phase-1-foundation rationale

The task prompt pins eight decisions as inherited from
phase-1-foundation. Each is recorded here explicitly so the peer
reviewer can audit the chain rather than re-derive it.

| # | Decision | Source | Why inherited, not re-opened |
|---|---|---|---|
| 1 | Deployment target = on-premise / laptop | phase-1-foundation Decision 1 | Overdrive IS the orchestrator; we do not deploy the orchestrator to a cloud service. The whitepaper §1 single-binary principle pins this for every phase. |
| 2 | No container orchestration | phase-1-foundation Decision 2 | Binary runs directly. No Docker image, no K8s manifest. |
| 3 | CI/CD platform = GitHub Actions | phase-1-foundation Decision 3 + existing `.github/workflows/ci.yml` | The repo lives on GitHub; switching platforms for one feature is pure cost. |
| 4 | Existing infrastructure = extend, don't replace | phase-1-foundation Decision 4 | `.github/workflows/ci.yml` + `nightly.yml` already carry five required checks (`fmt-clippy`, `test`, `dst`, `dst-lint`, `mutants-diff`). This feature adds one, leaves the rest untouched. |
| 5 | Observability = Rust `tracing` crate; DuckLake deferred | phase-1-foundation Decision 5 | whitepaper §12 DuckLake telemetry is Phase 2+. Phase 1 `tracing` subscriber is the upper bound on observability surface. |
| 6 | Deployment strategy = recreate (no rollout) | phase-1-foundation Decision 6 | Walking skeleton; no previous version running anywhere to roll against. |
| 7 | Continuous learning = no | phase-1-foundation Decision 7 | LLM-supervised canary analysis applies to live traffic. Phase 1 has zero traffic. |
| 8 | Git branching = GitHub Flow | phase-1-foundation Decision 8 + `.github/workflows/ci.yml` trigger block | Already codified. Feature branches → PR → `main`. No re-derivation. |
| 9 | Mutation testing = per-feature + nightly full-corpus | phase-1-foundation Decision 9 + `CLAUDE.md` §"Mutation Testing Strategy" + `.cargo/mutants.toml` | User memory explicitly says: **do NOT re-prompt, do NOT edit CLAUDE.md**. The strategy is in force; diff-scoped `mutants-diff` on PR with ≥80% kill rate, nightly workspace with -2pp drift alert. |

**No decisions re-opened.** If a reviewer believes any of the above
should be re-litigated for this feature, the correct move is a new
ADR or an upstream-changes note — not an edit here.

---

## New decisions for this feature

### N1 — New CI gates: the `openapi-check` trio

**Question**: What new CI-side gates does this feature require?

**Answer**: Exactly one new required status check — `openapi-check` —
plus two "verify the existing jobs already cover it" items.

**Detail** (see ci-cd-pipeline.md for the full delta):

1. **`openapi-check` (NEW REQUIRED CHECK)**. Runs
   `cargo xtask openapi-check` per ADR-0009. Fails the PR if
   `api/openapi.yaml` diverges from the derived schema off the Rust
   `overdrive-control-plane::api` types. The xtask subcommand stub
   lands in this DEVOPS wave; DELIVER populates the body.
2. **`cargo check --workspace` coverage of new deps**. The existing
   `test` job runs `cargo nextest run --workspace --locked` which
   implicitly performs `cargo check` as part of build. The five new
   workspace dependencies (`axum`, `axum-server`, `utoipa`,
   `utoipa-axum`, `libsql`) resolve and compile there — no new job.
3. **`cargo nextest` affected-test routing picks up the new crate**.
   `overdrive-control-plane` added to workspace `members` → nextest
   auto-discovers its tests in the `test` and `integration` jobs.
   Lefthook's `nextest-affected` hook uses `rdeps(...)` filter syntax
   that expands transitively — editing `overdrive-control-plane`
   triggers CLI acceptance tests too. Zero lefthook.yml edits required.

**Rationale**: The single new risk surface vs phase-1-foundation is
schema drift — Rust types vs `api/openapi.yaml`. ADR-0009 names the
gate; DEVOPS wires it. Everything else in the test and build path
is automatic by virtue of the existing workspace-wide invocation
shape.

### N2 — New DST invariants absorbed by existing `dst` job

**Question**: Do the three new DST invariants
(`at_least_one_reconciler_registered`, `duplicate_evaluations_collapse`,
`reconciler_is_pure`) need a new CI job?

**Answer**: No. The existing `dst` job runs `cargo xtask dst`, which
iterates over the `ALL_VARIANTS` invariant list. DISTILL scaffold
inventory (§DWD-06) lands the three new variants into
`xtask/src/dst.rs` as panic-bodied stubs; the DELIVER crafter fills
in the bodies. CI picks up the new invariants on the next PR after
the scaffolds compile.

**Scaffold-phase CI expectation**: CI will be **RED** on any PR that
lands scaffolds before their DST evaluators are implemented. This is
**expected**, not a workflow bug — scaffold `panic!` bodies are the
intended "red" state per DWD-06 and phase-1-foundation
ADR-0001 scaffolding discipline.

### N3 — `--no-verify` discipline during DELIVER

**Question**: How should the crafter run pre-commit hooks while
replacing scaffold `panic!` bodies one at a time?

**Answer**: **Hooks stay enabled**. Lefthook's pre-commit `fmt` hook
auto-formats and re-stages (benign). `clippy`, `doctest`, and
`nextest-affected` may legitimately fail on a scaffold `panic!` that
has not yet been replaced — in which case the crafter either:

- Replaces the scaffold and commits a green result, or
- Uses `git commit --no-verify` for a deliberate WIP commit that will
  be rebased before PR.

**DO NOT** disable hooks globally for this feature. The `--no-verify`
escape hatch is documented in lefthook.yml itself; using it is a
signal to the reviewer that the commit is WIP, not that the hooks
are broken.

### N4 — KPI instrumentation: `tracing` spans, DuckLake-ready, no dashboards

**Question**: What observability surface do the seven outcome KPIs
(K1–K7) get in Phase 1?

**Answer**: Each KPI maps to a named `tracing` span with structured
fields. See `kpi-instrumentation.md` for per-KPI span design.

- **Scope in Phase 1**: instrument only. No subscriber other than the
  default `fmt::Layer` writing to stderr. No external collector. No
  Prometheus, no OTLP export, no DuckLake.
- **Shape ready for Phase 2**: field names and span boundaries chosen
  so the Phase 2 DuckLake work (whitepaper §12) can ingest them
  without re-instrumenting.
- **Dashboard / SLO design**: **explicitly out of scope** per wizard
  decision. Phase 2 DuckLake work owns dashboards, alerting,
  retention, and query patterns.

---

## Skipped deliverables

Per the user's DEVOPS-wave scope directive in the task prompt:

| File | Why skipped |
|---|---|
| `platform-architecture.md` | The platform architecture IS `docs/product/architecture/brief.md` §14-§23 + 8 ADRs (0008-0015). Duplicating here would guarantee drift. |
| `monitoring-alerting.md` | No SLOs / alerts in Phase 1 per wizard decision. Phase 2+ DuckLake work owns this. |
| `observability-design.md` | `kpi-instrumentation.md` covers per-KPI span design; a separate observability-design file would redundantly re-state the same content. |
| `infrastructure-integration.md` | brief.md §12 — no external integrations. Nothing to integrate with. |
| `continuous-learning.md` | LLM-supervised canary is whitepaper §12/§15; Phase 3+. |
| `branching-strategy.md` | Phase-1-foundation documented GitHub Flow + `.github/workflows/ci.yml` trigger block. This feature changes neither. |

Each skipped file returns when the underlying system it describes
exists. `monitoring-alerting.md` for example returns in Phase 2 when
DuckLake comes online.

---

## Relationship to DISTILL

DISTILL for phase-1-control-plane-core ran in
**graceful-degradation mode** — DWD-12 explicitly notes "DEVOPS wave
had not run when DISTILL ran; default environment matrix applied
(`clean`, `with-pre-commit`, `with-stale-config`)".

This DEVOPS wave's `environments.yaml` retroactively fills that gap
with a matrix that is **additive and consistent with DISTILL's
Strategy C choice**:

- `clean` → covered by `dev-local` (fresh `tempfile::TempDir`
  + fresh `~/.overdrive/config` per test).
- `with-pre-commit` → covered by `dev-local` + lefthook coexistence
  matrix entry.
- `with-stale-config` → covered by DISTILL §2.4 scenario
  ("pre-existing `~/.overdrive/config` from a previous cluster") —
  the behavioural handling lives in the scenario; the DEVOPS wave
  confirms the environment precondition (write access to
  `$HOME/.overdrive/`).

**No DISTILL scenarios invalidated by DEVOPS wave.** The
acceptance-designer-reviewer's APPROVED-WITH-NOTES posture on DISTILL
stands unchanged.

---

## Handoff to DELIVER (software-crafter)

The crafter needs the following from this wave before DELIVER begins:

1. **`.github/workflows/ci.yml` gains exactly one new job — `openapi-check`.**
   See ci-cd-pipeline.md for the exact YAML shape. The job invokes
   `cargo xtask openapi-check` and fails on non-empty diff.
2. **`xtask/src/main.rs` gains two new subcommands — `openapi-gen`
   and `openapi-check`.** DEVOPS scaffolds the subcommand enum
   entries; DELIVER populates the bodies per ADR-0009. Until
   populated, both subcommands should exit with a
   `"not yet implemented"` error so CI fails loudly rather than
   pass silently.
3. **`api/openapi.yaml`** is checked in at the **workspace root** per
   ADR-0009. First version lands alongside the first green
   `openapi-gen` run. Until then, the file contains a
   placeholder comment referencing ADR-0009 so the `openapi-check`
   CI job has a file to diff against (the diff will be massive on
   first implementation PR, and that is correct — `openapi-check`
   is not satisfied until `openapi-gen` + `openapi-check` round-trip
   to an empty diff).
4. **New workspace dependency entries** in the root `Cargo.toml`
   `[workspace.dependencies]`:
   - `axum = "0.7"` (feature set chosen by DELIVER per ADR-0008)
   - `axum-server = { version = "0.7", features = ["tls-rustls"] }`
   - `utoipa = { version = "5", features = ["axum_extras", "yaml"] }`
   - `utoipa-axum = "0.1"`
   - `libsql = "0.5"` (feature set chosen by DELIVER per ADR-0013)

   Exact versions are DELIVER's call against current crates.io, but
   the workspace-level pin is required (DEVOPS mandate per
   development.md §Dependencies — "workspace dependencies always").
5. **New crate scaffold**: `crates/overdrive-control-plane/Cargo.toml`
   with `package.metadata.overdrive.crate_class = "adapter-host"` per
   ADR-0016 rename. DISTILL DWD-06 scaffold inventory covers the
   module layout.
6. **KPI instrumentation** (per kpi-instrumentation.md) lands
   alongside the Rust code for each KPI. No separate "observability
   PR" — the span decorations go in the same PR as the code path
   they trace.
7. **Pre-push lefthook continues to run the full workspace suite.**
   DELIVER may legitimately push WIP commits with `--no-verify` while
   scaffolds are un-replaced; these MUST be squashed / rebased
   before the PR opens. CI is the authoritative gate.

---

## Quality gates

- [x] Environment inventory documents `ci` + `dev-local` — no new
      environment introduced
- [x] CI delta names exactly one new required status check
      (`openapi-check`)
- [x] DST job absorbs the three new invariants without a new job
- [x] Mutation testing strategy unchanged — `.cargo/mutants.toml`
      already scopes correctly for the new crate
- [x] Lefthook coexistence preserved — no new pre-commit or pre-push
      hook introduced
- [x] KPI-to-`tracing`-span map complete for K1–K7 (see
      kpi-instrumentation.md)
- [x] Rollback procedure: N/A for walking skeleton (wizard decision
      Deployment = Recreate) — documented explicitly so a reviewer
      does not expect one
- [x] `.claude/rules/testing.md` four-tier model respected — Phase 1
      still Tier 1 only; Tier 2/3/4 remain future-scoped
- [x] All ADRs from DESIGN referenced: 0008, 0009, 0010, 0013, 0014
      cited in the delta-analysis + instrumentation design
- [x] Skipped deliverables listed with reason
- [ ] Peer review by `nw-platform-architect-reviewer` — **pending**;
      parent task dispatches post-DEVOPS

---

## Residual stressors inherited / added

Residual stressors from phase-1-foundation that still apply:

- **turmoil upstream drift** — the DST harness depends on
  `turmoil`; a major-version bump could break the three new
  invariants in addition to the existing ones. Mitigation unchanged:
  workspace-level pin, covered by `cargo update` discipline.

Residual stressors added by this feature (inherited from DESIGN
wave-decisions §"Residuality / stressor posture"):

- **`axum` / `utoipa` major-version upgrade** — new workspace deps
  whose churn could break the router-derivation pipeline and the
  `openapi-check` gate. Mitigation: workspace-level pin + the
  `openapi-check` gate catches schema drift on any version bump;
  a breaking annotation change would surface as a nextest compile
  failure.

No other residual stressors rise to DEVOPS attention. The
observability / monitoring stressor ("what if Phase 2 DuckLake
schema differs from Phase 1 tracing span shape?") is deliberately
out-of-scope — that question lands when Phase 2 DuckLake work
lands.

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-23 | Initial DEVOPS wave decisions for phase-1-control-plane-core. 4 new decisions (N1-N4); 9 inherited from phase-1-foundation. One new CI required check (`openapi-check`). Retroactive environment inventory fills DISTILL DWD-12 graceful-degradation gap. |
