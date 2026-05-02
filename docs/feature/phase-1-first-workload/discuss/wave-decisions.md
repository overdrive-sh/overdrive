# DISCUSS Wave Decisions — phase-1-first-workload

**Wave**: DISCUSS (product-owner)
**Owner**: Luna
**Date**: 2026-04-27
**Status**: COMPLETE — handoff-ready for DESIGN (solution-architect),
pending peer review.

---

## Scope correction (2026-04-27)

The first DISCUSS pass committed a scope error: it framed Phase 1 as
multi-node-shaped (operator-facing node registration, scheduler
taint/toleration support, default `control-plane:NoSchedule` taint)
even though Phase 1 is single-node — control plane and worker run
co-located on one machine.

The correction (per user feedback, 2026-04-27):

- **Phase 1 is SINGLE-NODE.** There is exactly one node (the local
  host), implicit, not operator-facing. The control plane writes its
  own `node_health` row at startup as an implementation detail; there
  is no CLI verb, no handler, no separate "register a node" activity
  for the operator.
- **No taint, no toleration, no default `control-plane:NoSchedule`.**
  With one node there is no placement choice for a taint to gate
  against, so taint/toleration logic delivers no Phase 1 value.
- **GH issue #20 (Control-plane cgroup isolation + scheduler
  taint/toleration support) is split across phases**: the cgroup-
  isolation half is in Phase 1 scope (US-04 / Slice 4); the
  taint/toleration half is deferred to a later phase (alongside
  multi-node + Raft). The user is expected to split GH #20 into two
  separate issues to track this independently.
- **Control-plane cgroup isolation stays in scope for Phase 1.** Even
  though there is exactly one node, the control plane and the
  workload run on the same machine — exactly the topology §4 calls
  out as needing kernel-enforced isolation. The cgroup-isolation
  story validates that the kernel actually enforces the split under
  real CPU pressure.
- **Scheduler stays in scope for Phase 1.** The placement function
  is trivially "this one node, if capacity covers, else reject" at
  runtime, but the determinism contract still has to hold so Phase
  2+ multi-node is a content change rather than a structural one.
- **`Node` aggregate stays single-row in Phase 1.** Already exists
  from closed issue #7; one row in `node_health` at runtime. The
  `Node::taints` and `Job::tolerations` field plumbing originally
  proposed in this wave was pulled.

What was removed:

- Slice 1 (Node registration) — file emptied; no operator-facing
  registration verb in Phase 1.
- Slice 5 (Taint/toleration) — file emptied; the `job stop`
  affordance from this slice was folded into the lifecycle
  reconciler slice (now Slice 3) since stop is the inverse of start
  through the same lifecycle path.
- US-01 (Node registration) and US-05 (Taint/toleration) user
  stories.
- `node_taints` and `tolerations` artifacts from the shared
  artifacts registry.
- Every reference to "register a node", "default
  `control-plane:NoSchedule` taint", "Taint" / "Toleration" newtype,
  "tolerations field on Job", etc.

What stayed and was tightened to single-node phrasing:

- US-01 (formerly US-02) — first-fit scheduler scaffold.
- US-02 (formerly US-03) — ProcessDriver.
- US-03 (formerly US-04) — Job-lifecycle reconciler + action shim,
  now bundling `overdrive job stop` end-to-end (folded in from the
  deleted US-05).
- US-04 (formerly US-06) — Control-plane cgroup isolation. Note:
  GH #20's taint/toleration half deferred.

The 4-slice plan replaces the prior 6-slice plan. Re-running the
elephant-carpaccio taste tests (see `story-map.md`) confirms the
4-slice plan PASSES.

## Wizard decisions honoured (from invocation)

- **Decision 1 — Feature type**: Backend / Infrastructure (process driver, scheduler, reconciler, cgroup isolation). No UI surface beyond the existing CLI.
- **Decision 2 — Walking skeleton**: NO new walking skeleton. This feature IS the execution-layer extension of the prior walking skeleton (`phase-1-control-plane-core`). Sliced via elephant-carpaccio only.
- **Decision 3 — UX research depth**: Lightweight. Journey already established in `docs/product/journeys/submit-a-job.yaml`; this feature extends it via `journey-submit-a-job-extended.yaml` rather than re-deriving it.
- **Decision 4 — JTBD analysis**: Skipped. `docs/product/jobs.yaml` already carries J-OPS-002 (active) and J-PLAT-001/002/003. **However, this wave adds J-OPS-003 to `jobs.yaml`** as the execution-layer counterpart of J-OPS-002 (motivation explicit in whitepaper §4, §6, §18). Addition is additive — no edits to existing entries, only a new entry plus changelog rows. J-OPS-003's situation/motivation/outcome was tightened to single-node phrasing per the 2026-04-27 scope correction.

## Pinned scope (from GitHub roadmap + wizard + scope correction)

The Phase 1 issues this feature delivers:

- **GH #14 [1.7]** Process driver (`tokio::process` + cgroups v2) — whitepaper §6.
- **GH #15 [1.8]** Basic scheduler (first-fit) — whitepaper §4.
- **GH #20 [1.11] (cgroup-isolation half only)** Control-plane cgroup isolation — whitepaper §4. **The taint/toleration half of GH #20 is explicitly DEFERRED to a later phase** (when multi-node + Raft lands). The user is expected to split GH #20 into two separate issues.
- **GH #21 [1.12]** Job-lifecycle reconciler (start/stop/migrate/restart; convergence to declared replica count) — whitepaper §18. **`MigrateAllocation` is explicitly out-of-scope for Phase 1** per the dependency on `overdrive-fs` migration tooling that lands in Phase 3+.

Dependencies (#7, #10, #12, #17) all CLOSED — confirmed by the
codebase research dated 2026-04-27.

## Constraints established

This feature is bounded by the following constraints; they are not
debatable within this wave and must be carried forward into DESIGN /
DISTILL / DELIVER:

- **Phase 1 is single-node — no node registration, no taint/toleration, no multi-region.** Control plane and worker run co-located on one machine. There is exactly one node (the local host), implicit. No operator-facing node-registration verb. No taints, no tolerations, no `control-plane:NoSchedule` default. No `Node::taints` or `Job::tolerations` aggregate fields.
- **Reconciler purity is non-negotiable.** The lifecycle reconciler MUST satisfy the existing `ReconcilerIsPure` DST invariant — no `.await`, no wall-clock reads, no direct store writes inside `reconcile`. `tick.now` only.
- **Scheduler determinism is load-bearing.** All internal collections driving iteration are BTreeMap. A `HashMap` in the scheduler hot path is a blocking violation. Phase 1 N=1 doesn't change this — Phase 2+ multi-node is a content change, not a structural one.
- **STRICT newtypes for new identifiers.** `CgroupPath` is the only new newtype shipped by this feature. Full FromStr / Display / serde / rkyv / proptest discipline.
- **No new fields on existing aggregates.** `Node` and `Job` ship unchanged from `phase-1-control-plane-core`. The aggregate-roundtrip proptest continues to pass byte-identical with no extension.
- **Real-infrastructure tests gated `integration-tests`.** Default lane uses `SimDriver`; real processes / cgroups / sockets live behind the feature flag.
- **Action shim is the single I/O boundary in the convergence loop.** Lifecycle reconciler emits Actions (data); shim dispatches to `Driver::start` / `Driver::stop` (I/O).
- **Linux-only for cgroups.** macOS / Windows hosts run default-lane `SimDriver`; integration tests require a Linux VM.

## Aggregate field changes do NOT emerge here

The earlier version of this wave proposed two aggregate field additions
(`Node::taints`, `Job::tolerations`); both were pulled per the scope
correction. **This feature ships zero schema changes on `Node` or
`Job`.** The existing `aggregate_roundtrip` proptest in
`crates/overdrive-core/tests/acceptance/aggregate_roundtrip.rs`
continues to pass byte-identical. No rkyv schema migration is
required for Phase 1.

## Artifacts produced

### Product SSOT (additive)

- `docs/product/jobs.yaml` — added J-OPS-003 as `served_by_phase: 1, status: active`. Tightened to single-node phrasing per scope correction; new changelog row noting the tightening.
- `docs/product/journeys/submit-a-job.yaml` — one-line changelog row added pointing to the journey extension and noting Phase 1 single-node.

### Feature artifacts (this directory)

- `docs/feature/phase-1-first-workload/discuss/journey-submit-a-job-extended.yaml` — structured journey extension with embedded Gherkin per step (NO standalone `.feature` file consumer; Gherkin file below is specification-only).
- `docs/feature/phase-1-first-workload/discuss/journey-submit-a-job-extended-visual.md` — ASCII flow + emotional annotations + TUI mockups for the new and extended steps.
- `docs/feature/phase-1-first-workload/discuss/journey-submit-a-job-extended.feature` — **specification only**. The preamble disclaims any tooling consumption; the crafter translates these scenarios to Rust `#[test]` / `#[tokio::test]` per `.claude/rules/testing.md`.
- `docs/feature/phase-1-first-workload/discuss/shared-artifacts-registry.md` — 8 artifacts (`node_id` as a single-node precondition, `node_capacity`, `placement_decision`, `alloc_id`, `alloc_state`, `cgroup_path`, `restart_count`, `driver_handle`) tracked, each with SSOT + consumers + integration risk + validation; explicit inheritance from `phase-1-control-plane-core` registry.
- `docs/feature/phase-1-first-workload/discuss/story-map.md` — 4-activity backbone, walking-skeleton extension identified, 4 carpaccio slices, priority rationale, scope assessment PASS, slice taste-tests against the 4-slice plan all green.
- `docs/feature/phase-1-first-workload/discuss/prioritization.md` — release priority and intra-release ordering with the GH #20 split note.
- `docs/feature/phase-1-first-workload/slices/slice-{1..4}-*.md` — one brief per carpaccio slice (≤120 lines each), with slice taste-tests applied.
- `docs/feature/phase-1-first-workload/discuss/user-stories.md` — four LeanUX stories with System Constraints header and embedded BDD.
- `docs/feature/phase-1-first-workload/discuss/outcome-kpis.md` — four feature-level KPIs + measurement plan + handoff to DEVOPS.
- `docs/feature/phase-1-first-workload/discuss/dor-validation.md` — 9-item DoR PASS on 3/4 stories; **US-03 has a HARD DESIGN dependency on the `State` placeholder and is flagged as conditionally-PASSED**.
- `docs/feature/phase-1-first-workload/discuss/wave-decisions.md` (this file).

## Key decisions

### 1. No DIVERGE artifacts — grounded directly in whitepaper + prior feature precedent + codebase research

No `docs/feature/phase-1-first-workload/diverge/` directory present.
Wizard decision "JTBD: Skip". Jobs grounded in whitepaper §4, §6, §18
and in the existing job register (J-OPS-002 still active, J-OPS-003
added by this wave). **Risk**: operator motivation is inferred, not
interview-validated. **Mitigation**: DIVERGE can be retrofitted if
any of the walking-skeleton-extension commands turns out wrong.

### 2. No `.feature` files consumed by tooling

Per `.claude/rules/testing.md` and the wizard prompt. The
`journey-submit-a-job-extended.feature` file is specification-only;
its preamble explicitly disclaims tooling consumption. The crafter
translates the scenarios into Rust `#[test]` / `#[tokio::test]`
functions, gated behind the `integration-tests` feature where they
touch real infrastructure (real processes, real cgroups), or in
`tests/acceptance/<scenario>.rs` for in-memory acceptance shapes.

### 3. Walking skeleton is INHERITED — this feature is one extension, sliced into 4 carpaccio slices

Per the wizard decision. The product-level walking skeleton lives in
`submit-a-job.yaml` and was landed by `phase-1-control-plane-core`.
This feature extends step 4 of that journey (empty allocation rows
become real Running rows) and adds steps 5, 6, 7 (crash recovery,
control-plane responsiveness under pressure, stop and drain). The 4
internal carpaccio slices are NOT a new walking skeleton; they're the
execution-layer extension mechanics.

### 4. Scheduler is a module, not a reconciler variant

The codebase research's Q2 asked whether the scheduler should be its
own `AnyReconciler::Scheduler` variant or a module called from the
lifecycle reconciler. **DISCUSS pre-decides: the scheduler is a pure
synchronous function module, called from inside the lifecycle
reconciler's pure body.** Rationale:

- Two competing reconcilers writing to the same target would race;
  one reconciler with one helper module does not.
- The scheduler is a pure function over `(nodes, job, allocs)`; making
  it a reconciler would force synthetic State/View shapes for the
  pure-function inputs.
- Anvil's `reconcile_core` pattern (USENIX OSDI '24) supports this
  — pure helpers called from pure reconcile bodies, not separate
  reconcilers.

DESIGN may revise via ADR if convergence-time analysis reveals
material issues; the slicing supports either path.

### 5. Action shim is a NEW runtime layer in `overdrive-control-plane`

Per `.claude/rules/development.md` §Reconciler I/O and the codebase
research's Q4. The shim is the async boundary that the lifecycle
reconciler cannot cross. Lives alongside the existing
`reconciler_runtime` module in `overdrive-control-plane`. Holds an
`Arc<dyn Driver>` reference (production: ProcessDriver from Slice 2;
DST: SimDriver from `overdrive-sim`).

`AppState` extends with `driver: Arc<dyn Driver>`. **This is the
Phase 1 simplest-possible wiring** — Phase 2+ may introduce a
`DriverRegistry` selecting per-`DriverType` (Process / MicroVm / Wasm).

### 6. State shape is a HARD DESIGN dependency for US-03

The codebase research flagged this as the single largest design
decision for issue #21. The current `pub struct State;` placeholder
cannot be dereferenced by the lifecycle reconciler. **DESIGN MUST
deliver an ADR clarifying the State shape before DELIVER can begin
on US-03.** Three options listed in US-03 Technical Notes; Luna's
recommendation (per the codebase research suggestion) is per-reconciler
typed state matching the existing `AnyReconcilerView` pattern (e.g.
`AnyState::JobLifecycle(JobLifecycleState)`).

This is the one DoR item that does not pass cleanly without
DESIGN's intervention. Luna does NOT propose a State shape herself —
that belongs in the DESIGN ADR.

### 7. `overdrive job stop` lives in the lifecycle reconciler slice

`job stop` is the inverse of `job submit` through the same lifecycle
reconciler + action shim path. Splitting them across two slices would
force the I/O machinery to land twice. They land together in Slice 3.
The DoR's right-sizing WARN flags the bundle but accepts it.

### 8. Linux-only for cgroup work; macOS dev hosts use SimDriver

Slices 2 and 4 are gated behind `integration-tests` feature and run
only on the Linux Tier 3 matrix. Default-lane `cargo nextest run`
uses `SimDriver` — no real processes, no real cgroups. macOS / Windows
developers run integration tests in a Linux VM (matches the existing
testing pattern from `.claude/rules/testing.md`).

### 9. `MigrateAllocation` is explicitly out of Phase 1

Per whitepaper §6 + §18. Migration depends on `overdrive-fs` cross-region
metadata handoff, which is Phase 3+ work. The Action variant is NOT
landed in this feature — only `StartAllocation`, `StopAllocation`,
`RestartAllocation` are.

### 10. Scenario titles are business outcomes, not implementation

Every scenario title in the embedded BDD describes operator-observable
behaviour ("Job-lifecycle reconciler converges to declared replica
count", "ProcessDriver places the child process in the workload
cgroup scope"). None name internal method signatures, trait object
types, or protocol tokens as the subject. Luna's contract with
DISTILL.

### 11. No regression on prior feature guardrails

Every guardrail from `phase-1-foundation` and `phase-1-control-plane-core`
applies verbatim:

- DST wall-clock < 60s.
- Lint-gate false-positive rate at 0.
- Snapshot round-trip byte-identical.
- CLI round-trip < 100ms on localhost.
- OpenAPI schema-drift gate green.

The three new DST invariants introduced by US-03
(`JobScheduledAfterSubmission`, `DesiredReplicaCountConverges`,
`NoDoubleScheduling`) compose with the existing catalogue, they do
not replace it. The existing `ReconcilerIsPure` invariant must
continue to pass with `JobLifecycle` in the catalogue — this is a
guardrail.

## Scope assessment result

- **Stories**: 4 (well below the 10-story ceiling).
- **Bounded contexts / crates touched**: 4 (`overdrive-core`, `overdrive-host`, `overdrive-control-plane`, `overdrive-cli`). `overdrive-sim` adds invariants only. **At the edge of the ≤3-bounded-context oversized signal**, but this is structurally unavoidable: the feature spans application-level (scheduler), host-adapter (driver), control-plane (reconciler + shim + handlers + cgroup-isolation bootstrap), and CLI surface. No two of these can collapse into one without contradicting the hexagonal split or the binary-boundary discipline.
- **Walking-skeleton-extension integration points**: 4 (lifecycle reconciler → scheduler, lifecycle reconciler → action shim, action shim → ProcessDriver, server bootstrap → cgroup hierarchy). Within the ≤5 oversized-signal threshold; each integration point follows a known-shape from the prior feature.
- **Estimated effort**: 4-6 focused days. Slices 1 and 2 parallelisable. Slice 3 may stretch to 1-2 days due to bundling `job stop`.
- **Multiple independent user outcomes worth shipping separately**: no — the four slices are sequential on the same walking skeleton.
- **Verdict**: **RIGHT-SIZED** — 4 stories, 4 crates well-bounded, 4 integration points, all familiar shapes from the prior feature.

## Risks surfaced

| # | Risk | Probability | Impact | Mitigation |
|---|---|---|---|---|
| 1 | The `State` placeholder must be replaced with a real shape before US-03 can ship; DESIGN's ADR is on the critical path | High | High | Flagged as a HARD DoR dependency in `dor-validation.md`. DELIVER cannot start US-03 without the ADR. Three options listed in US-03 Technical Notes. |
| 2 | US-03 scope may exceed 2 days once DESIGN digs into the State + AppState + new variants + new invariants + `job stop` end-to-end concentration | Medium | Low | Pre-described split in DoR (3A = StartAllocation only + JobScheduledAfterSubmission; 3B = StopAllocation + RestartAllocation + backoff + DesiredReplicaCountConverges + NoDoubleScheduling + `job stop`). DESIGN can split if material complexity surfaces. |
| 3 | `cgroups-rs` (or chosen cgroup-management crate) introduces a non-trivial Linux-only dep into `overdrive-host`; default-lane builds on macOS may break | Low | Medium | The dep is platform-conditional via Cargo target gates. macOS dev hosts run default lane with `SimDriver`; integration tests are Linux-only. |
| 4 | Pre-flight cgroup delegation check refuses to start on developer machines without delegated cgroup v2 (e.g. unconfigured Linux dev box) | Medium | Low | DESIGN may relax pre-flight to an actionable warning + dev-mode escape hatch. Documented as Slice 4 Technical Note. |
| 5 | DST `JobScheduledAfterSubmission` is an *eventually* invariant — depends on bounded reconciler tick budget; if the broker drains slowly, the invariant flakes | Medium | Medium | Bound the eventually window to ≥10 turmoil ticks (well above scheduler determinism + driver dispatch + observation row write). DST seed identifies any flake. |
| 6 | `Action::StartAllocation` carrying the full `AllocationSpec` couples reconciler size to spec size; large specs (Phase 2+ when specs grow) bloat the Action enum | Low | Low | Acknowledged. Phase 2+ may interpose an `AllocationSpecRef` (content-hashed reference) to keep Actions small. Phase 1 specs are tiny; not a blocker. |
| 7 | Scheduler's `Resources` arithmetic is in the hot path of every reconciler tick; saturating-sub vs explicit-error handling is unspecified | Low | Low | DESIGN owns; see US-01 Technical Notes. Phase 1 single-node has at most a handful of running allocs to subtract; performance is in the noise. |

## What DESIGN wave should focus on

**Priority Zero (blocks DELIVER on US-03):**

1. **`State` shape ADR**. The codebase research's flagged structural blocker. Three options in US-03 Technical Notes; recommendation is per-reconciler typed state matching `AnyReconcilerView`. This is the one item that MUST land before any DELIVER work on US-03 starts.

**Priority One (architectural, gates downstream slices):**

2. **`AppState::driver` extension**: `Arc<dyn Driver>` field; how it's threaded through `run_server_with_obs`; how test fixtures inject `SimDriver`.
3. **Action shim placement**: which module in `overdrive-control-plane`; what its function signature is; how it consumes `Vec<Action>` from the reconciler runtime drain path.
4. **Scheduler crate boundary**: stay in `overdrive-control-plane::scheduler` (lightweight default; recommendation) vs move to a dedicated `overdrive-scheduler` crate. ADR if the choice warrants it.
5. **Single-node startup wiring**: how the local node's `node_health` row is written at server startup. Recommendation: server bootstrap writes one row keyed by a deterministic local NodeId (e.g. hostname-derived or config-driven). DESIGN owns the exact mechanism.

**Priority Two (mechanical, wireable per slice):**

6. **`cgroups-rs` vs direct cgroupfs**: Slice 2 dep choice. Both viable; pick based on dep-graph cost vs verbose unsafe-feeling code.
7. **`overdrive job stop` HTTP shape**: `POST /v1/jobs/{id}:stop` (recommendation, idempotent semantics clear) vs `DELETE /v1/jobs/{id}` (idempotent semantics differ).
8. **Pre-flight cgroup check level**: hard refusal (recommendation) vs warn-with-escape-hatch.
9. **Resource enforcement on cgroup scope**: write `cpu.weight` / `memory.max` in Slice 2 (recommendation, since `Resources` is on `AllocationSpec`) vs defer to a §14 follow-on.

## What is NOT being decided in this wave (deferred to DESIGN)

- Exact Rust module layouts inside `overdrive-control-plane::scheduler` and `overdrive-control-plane::reconciler::job_lifecycle`.
- Error variant taxonomy beyond "thiserror + pass-through `#[from]`".
- Trait method signatures beyond what the AC semantically require.
- Concrete libSQL schema for `JobLifecycleView` private DB.
- Whether `MigrateAllocation` is added as a placeholder variant (Phase 3+ implementation).

## Handoff package for DESIGN (solution-architect)

- `docs/product/jobs.yaml` — updated with J-OPS-003 (single-node-tightened).
- `docs/product/journeys/submit-a-job.yaml` — changelog row pointing to extension and noting Phase 1 single-node.
- `docs/feature/phase-1-first-workload/discuss/journey-submit-a-job-extended.yaml` — journey extension structure.
- `docs/feature/phase-1-first-workload/discuss/journey-submit-a-job-extended-visual.md` — visual / mockups.
- `docs/feature/phase-1-first-workload/discuss/journey-submit-a-job-extended.feature` — Gherkin (specification only).
- `docs/feature/phase-1-first-workload/discuss/shared-artifacts-registry.md` — 8 artifacts + inheritance.
- `docs/feature/phase-1-first-workload/discuss/story-map.md` — 4 carpaccio slices + scope assessment.
- `docs/feature/phase-1-first-workload/discuss/prioritization.md` — release priority + intra-release ordering + GH #20 split note.
- `docs/feature/phase-1-first-workload/discuss/user-stories.md` — four LeanUX stories with AC + per-story BDD.
- `docs/feature/phase-1-first-workload/discuss/outcome-kpis.md` — four measurable KPIs.
- `docs/feature/phase-1-first-workload/discuss/dor-validation.md` — 9-item DoR PASS on 3/4 (US-03 conditional on State shape ADR).
- `docs/feature/phase-1-first-workload/slices/slice-{1..4}-*.md` — slice briefs.
- Reference: `docs/whitepaper.md` §4 (workload isolation), §6 (process driver), §18 (job-lifecycle reconciler).
- Reference: `docs/product/architecture/brief.md` — existing Application Architecture; DESIGN extends.
- Reference: `docs/product/architecture/adr-0001..0020` — existing ADRs.
- Reference: `docs/research/phase-1-first-workload-codebase-research.md` — codebase research with gap analysis per issue.
- Reference: `docs/feature/phase-1-control-plane-core/discuss/wave-decisions.md` — prior feature precedent.

## Open questions surfaced for user

None blocking handoff. The single hard blocker — the `State` shape —
is explicitly flagged as a Priority Zero ADR the DESIGN wave must
produce before DELIVER can start on US-03. The deferred half of
GH #20 (taint/toleration) is the user's tracking concern, not a
DESIGN concern.

## Changelog

| Date | Change |
|---|---|
| 2026-04-27 | Initial DISCUSS wave decisions for `phase-1-first-workload`. |
| 2026-04-27 | **Scope correction (single-node)**. Removed Slice 1 (Node registration) and Slice 5 (Taint/toleration), the corresponding US-01 and US-05 user stories, the `node_taints` and `tolerations` shared artifacts, the `Node::taints` and `Job::tolerations` aggregate field plumbing, the default `control-plane:NoSchedule` taint, and every reference to multi-node, taint, or toleration. Re-numbered the remaining four stories to US-01..US-04. Folded `overdrive job stop` end-to-end into the lifecycle-reconciler slice (now Slice 3) since stop is the inverse of start through the same lifecycle path. Re-validated DoR against the 4-story plan. Noted the GH #20 split (cgroup-isolation in Phase 1; taint/toleration deferred to multi-node phase) — user opened GH #134 for the deferred half. |
| 2026-04-27 | **Peer review APPROVED** (Eclipse / nw-product-owner-reviewer). Verdict: APPROVED with 0 blocking, 3 HIGH-severity advisory enhancements, 3 MEDIUM, 2 LOW. Stale-reference scope-correction scan: CLEAN. See Review Metadata below. |

---

## Review Metadata

**Review ID**: por-review-20260427-phase1-firstworkload
**Reviewer**: Eclipse (`nw-product-owner-reviewer`)
**Review Date**: 2026-04-27
**Artifact reviewed**: `docs/feature/phase-1-first-workload/discuss/` (full DISCUSS-wave artifact set, baseline review)
**Verdict**: **APPROVED**

### Findings by severity

| Severity | Count | Blocking? |
|---|---|---|
| Critical | 0 | — |
| High | 3 | No (advisory enhancements) |
| Medium | 3 | No |
| Low | 2 | No |

### Blocking issues

**None.** The single hard gate (US-03 cannot start DELIVER without a DESIGN ADR for the `State` shape) is already explicitly flagged in DoR item 8, called out in this document's "What DESIGN wave should focus on" section as Priority Zero, and labelled CONDITIONAL PASS in `dor-validation.md`. Reviewer noted that visibility could be strengthened with a top-of-document BLOCKERS callout (Finding 1) but did not gate approval on it.

### High-severity advisory enhancements (non-blocking)

1. **State ADR visibility** — Add a top-of-document BLOCKERS callout naming the `State` shape ADR as the DELIVER critical-path dependency. Current wording is correct but downstream-reader ergonomic. (`wave-decisions.md` summary.)
2. **US-03 scenario title polish** — Two scenarios slip into implementation mechanics ("reconciler does not call wall-clock or mint randomness"; "reconciler is pure"). Re-cast as outcome-focused ("output is deterministic under DST replay"; "scheduler placement is wall-clock-independent"). (`user-stories.md` US-03.)
3. **Action enum variants explicit in AC** — AC item 1 mentions "lifecycle Actions" generically; enumerate the three variants (`StartAllocation`, `StopAllocation`, `RestartAllocation`) and the deferred `MigrateAllocation` placeholder inline. (`user-stories.md` US-03 AC.)

### Medium-severity advisory enhancements

4. **`job stop` HTTP endpoint phrasing** — AC bullet 5 reads as decided (`POST /v1/jobs/{id}:stop`) when DESIGN owns the choice between that and `DELETE /v1/jobs/{id}`. Reword as recommendation. (`user-stories.md` US-03.)
5. **KPI K3 numeric bound** — `N` reconciler ticks is unbounded; pin to ≥10 ticks per the story-map remark. (`outcome-kpis.md` K3, or US-03 AC.)
6. **`driver_handle` forward-compat note** — Phase 2+ multi-driver dispatch will need `DriverRegistry` or enum-dispatch; flag this in the artifact entry so AppState extension is forward-aware. (`shared-artifacts-registry.md` driver_handle.)

### Low-severity editorial

7. **Taste-test "BORDERLINE" terminology** — Re-cast as "AT UPPER LIMIT" with a sub-note. (`story-map.md`.)
8. **Journey step 4 walking-skeleton-closure banner** — Optional: add a one-line banner naming step 4 as the moment the prior feature's empty rows materialize as Running. (`journey-submit-a-job-extended.yaml` step 4.)

### Scope-correction verification

Reviewer ran the stale-reference grep across all 13 feature artifacts for: `taint`, `toleration`, `Toleration`, `Taint`, `NoSchedule`, `tolerate-control-plane`, `node register`, `register a node`, `multi-node`, `cross-node`. **Result: CLEAN.** All hits are within explicit deferral frames (`Phase 2+ multi-node`, `GH #20 split`, `out of scope`, `non-goal`). Zero scope leakage into happy-path scope, AC, or System Constraints.

### Handoff readiness

- **DESIGN wave**: READY. Priority Zero ADR is the `State` shape (US-03 dependency).
- **DELIVER wave**: PROVISIONALLY READY. Slices 1 and 2 (scheduler scaffold, ProcessDriver) are parallelisable and unblocked. Slice 3 (lifecycle reconciler) is gated on DESIGN's State ADR. Slice 4 (cgroup isolation) is unblocked but depends on Slices 2 and 3 for end-to-end demonstrability.
