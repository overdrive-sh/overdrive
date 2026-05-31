# DISCUSS Wave Decisions — service-health-check-probes

**Feature ID:** `service-health-check-probes`
**Wave:** DISCUSS
**Date:** 2026-05-24
**PO:** Luna
**Source brief:** GitHub issue #170 (supersedes #169, settle-window primitive rejected)
**Predecessor RCA:** `docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md`

## Decisions resolved by main (per dispatch)

| ID | Decision | Resolution |
|---|---|---|
| D1 | Feature type | Cross-cutting |
| D2 | Walking Skeleton | Depends — evaluate during slicing. Natural candidate: default TCP-connect startup probe end-to-end |
| D3 | UX research depth | Lightweight (operator-CLI surface only, no UI) |
| D4 | JTBD | Yes (mandatory default) — derive Service-honesty sub-job extending J-OPS-003; cite #170 RCA framing |
| D5 | Density | lean + ask-intelligent (from `~/.nwave/global-config.json`) |

## DIVERGE artifacts presence

**Status:** ABSENT. No `docs/feature/service-health-check-probes/diverge/recommendation.md` or `job-analysis.md` exists.

**Risk:** Without a DIVERGE wave, the design-direction selection rationale is implicit in GH #170's narrative ("supersedes #169 settle-window primitive — operator framing rejected the synthetic-timer shape in favour of declarative k8s-shape probes"). The job grounding for this feature derives directly from:

1. The RCA root cause A ("kernel-accepted exec is NOT operator-meaningful liveness") in `docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md`.
2. The existing J-OPS-003 ("Run my actual workload and trust the platform to converge") in `docs/product/jobs.yaml` — this feature extends J-OPS-003 to the Service workload kind with operator-declared liveness contracts.
3. The ADR-0047 per-kind protocol shape — Service-kind needs a non-trivial "Stable" predicate analogous to Job-kind's `Completed`/`Failed`.

**Mitigation.** The Service-honesty sub-job (J-OPS-004) is derived in this wave's `jobs.yaml` patch and ratified by user via approval of the DISCUSS handoff package. If main subsequently wants ODI-style outcome scoring or alternative design-direction analysis (e.g. continuous health vs probe-driven), a DIVERGE wave can be inserted before DESIGN.

## Scope Assessment (Phase 1.5 — Elephant Carpaccio Gate)

Run BEFORE journey visualization to detect oversized-feature signal early.

| Signal | Count / Assessment | Verdict |
|---|---|---|
| Estimated user stories | 6–8 LeanUX stories (probe types × roles × CLI surface × default-inference × kind-rejection × KPI instrumentation) | OK (≤10) |
| Bounded contexts touched | 3 — `overdrive-worker` (probe runner), `overdrive-control-plane` (Service reconciler + streaming wire), `overdrive-cli` (Probes section render) | OK (≤3) |
| Walking skeleton integration points | 4 — TOML parser; ServiceLifecycleReconciler; ProbeRunner (worker); CLI render. Borderline 5 if you count the ObservationStore probe-result row | At limit |
| Estimated effort | 6–10 days across all slices (~8 days median) | Within 2 weeks |
| Independent user outcomes | 4 distinct outcomes: (1) honest startup signal, (2) readiness flips Backend.healthy, (3) liveness triggers restart, (4) operator visibility via CLI | Justifies slicing, but each composes naturally |

**Verdict: PASS — right-sized for one feature, slices into ~5 thin end-to-end shards.**

The natural elephant-carpaccio shards (each delivering a working behaviour the operator can verify):

- **Slice 01 (Walking Skeleton)** — Default TCP-connect startup probe: a Service with no explicit `[[health_check.*]]` declarations gets an implicit TCP probe against the first listener port; `ServiceSubmitEvent::Stable` emits once it passes OR `ServiceSubmitEvent::Failed { StartupProbeFailed }` if it never passes.
- **Slice 02** — Explicit HTTP startup probe declared in TOML; same wire outcome, different probe mechanic.
- **Slice 03** — Explicit Exec startup probe declared in TOML; runs inside workload cgroup; exit-0 = success.
- **Slice 04** — Readiness probe semantics: failure flips `Backend.healthy = false` (existing dataplane consumer at `crates/overdrive-core/src/dataplane/fingerprint.rs:95`); success flips it back.
- **Slice 05** — Liveness probe semantics: consecutive failures past threshold triggers `Action::RestartAllocation` for Service kind only.
- **Slice 06** — CLI `alloc status --job <id>` Service render gains a Probes section (ADR-0033 enrichment) showing each probe's last result + last-fail reason.
- **Slice 07** — Kind-rejection: TOML containing `[[health_check.*]]` under `[job]` or `[schedule]` is rejected at parse time with `ParseError::ProbesNotAllowedOnKind { kind, guidance }`.
- **Slice 08** — Early-exit detection within startup deadline: `ServiceSubmitEvent::Failed { EarlyExit { exit_code } }` emitted when workload exits before startup probe passes (Service-kind only; reuses ExitObserver).

Slices 01–05 each touch all three bounded contexts; 06–08 are surface-layer additions. The walking skeleton (Slice 01) closes RCA-A end-to-end for the most common case (operator declares no probes) — providing maximum value-per-effort.

**Sequencing rationale:**

- Slice 01 (WS) MUST land first — establishes the wire variant `Stable`, the reconciler-condition pathway, and the `ProbeRunner` trait surface.
- Slices 02 / 03 stack onto Slice 01's runner trait; pick either order based on operator demand (assume 02 first — HTTP is the dominant declared probe mechanic in k8s-shape ecosystems).
- Slices 04 / 05 require Slice 01's runner pathway AND extend the reconciler reaction (Backend.healthy flip, restart action) — sequence 04 before 05 because readiness has no destructive side effect.
- Slice 06 is operator-CLI surface — best landed after 01–05 so the Probes section has all probe types and roles to render.
- Slice 07 (kind-rejection) can land in parallel with 01 — pure parser change, no reconciler dependency.
- Slice 08 closes RCA-A for the **non-probe-declared early-exit** case (workload crashes within deadline before TCP-connect succeeds); requires Slice 01's `Failed` arm shape.

## Coherence validation (Phase 5 anticipated checks)

- CLI vocabulary: `probe` (noun), `health_check.{startup,readiness,liveness}` (TOML section), `Stable` (terminal condition), `StartupProbeFailed` (failure reason), `EarlyExit { exit_code }` (failure reason). Consistent across CLI output, TOML, wire events, ADRs.
- Emotional arc: operator submits Service → anxious (will it report honestly?) → focused (probes execute, results land in obs store) → confident (`Stable` event with `settled_in: Duration`) → trusting (subsequent `alloc status` renders Probes section with current state).
- Shared artifacts: `probe_idx` (TOML array position; used by ObservationStore row PK; rendered in CLI), `last_observed_at` (ObservationStore field; rendered in CLI Probes section), `settled_in` (computed by Service reconciler; emitted on `Stable`).

## Anticipated risks → DESIGN wave attention

1. **Probe runner concurrency model** — each alloc has N probes (typically 1–3 per role); runner must schedule them without head-of-line blocking. Resolution belongs in `solution-architect`'s DESIGN wave.
2. **Probe-result row cardinality** — N allocs × M probes × T ticks = unbounded if rows are append-only. Per `.claude/rules/development.md` § "Persist inputs, not derived state" the row should be the most recent observation per `(alloc_id, probe_idx)` (LWW), NOT a per-tick history. Surface to DESIGN.
3. **Cgroup-scoped exec probes** — exec probes MUST run inside the workload's cgroup (otherwise they observe the wrong process namespace). Worker-side concern; surface to DESIGN.
4. **Streaming-cap interplay** — startup probes may legitimately take >60s for slow-warming Services (LLMs, JVM warmup). The 60s `streaming_cap` may need to be configurable per spec. Flag to architect; may require ADR amendment.

## Research Alignment Review (2026-05-24)

**Reviewer:** Eclipse (nw-product-owner-reviewer)
**Research:** `docs/research/orchestration/service-health-check-probes-comprehensive-research.md` (16 sources, High confidence, Nova / 2026-05-24)
**Initial verdict:** NEEDS_REVISION (4 blocking, 4 non-blocking)
**Final verdict:** APPROVED (verification pass, 2026-05-24)

### Verification pass — APPROVED

All 4 blocking (B1–B4) and 4 non-blocking (R1–R4) findings landed correctly with no semantic drift. B1–B2 (feature-delta.md P2-Q8/P2-Q9) capture Kubernetes `successThreshold` defaults and the K8s issue #66230 cascading-restart risk per research § 5.1 and § 7.2 D1/D6. B3–B4 (US-02 AC + Technical Notes + slice-02 brief) accurately reflect research § 6.1 Pitfall 5 (HTTP 3xx → Fail, no redirect following) and the Phase-1 GET-only scope. R1–R4 (US-01 Technical Notes + System Constraints C11/C12/C13) ground in research § 2.1 (initial-delay deferral, startup budget formula), § 6.2 best practice 1 (liveness checks app-internal only), and § 6.1 Pitfall 4 (declare readiness to prevent premature traffic). US-02 DoR re-validated: 9/9 PASS. Research-citation accuracy verified at every fix site. **Feature is ready for DESIGN-wave handoff.**

### Blocking findings actioned

| ID | Finding | Fix landed in |
|---|---|---|
| B1 | Missing `successThreshold` P2 open question (research D1) | feature-delta.md P2-Q8 |
| B2 | Missing cascading-failure protection P2 open question (research D6, K8s #66230) | feature-delta.md P2-Q9 |
| B3 | US-02 redirect handling unspecified (research Pitfall 5) | user-stories.md US-02 AC |
| B4 | US-02 HTTP method (GET-only Phase 1) undocumented | user-stories.md US-02 AC + Technical Notes |

### Non-blocking findings actioned

| ID | Finding | Fix landed in |
|---|---|---|
| R1 | Initial-delay deferral undocumented (research § 2.1) | user-stories.md C12 |
| R2 | Startup budget calc transparency (research D3) | user-stories.md US-01 Technical Notes |
| R3 | Liveness-vs-dependency check guidance (research Pitfall 1+2) | user-stories.md C11 |
| R4 | Readiness-prevents-premature-traffic guidance (research Pitfall 4) | user-stories.md C13 |

### Alignment summary (post-edit)

- Three-role startup/readiness/liveness sequencing matches Kubernetes K1 reference (high confidence)
- "Honest by default" divergence from K8s permissive default is justified by RCA-A and consistently documented
- All 8 research recommendations (D1–D8) now covered: D2/D4/D7/D8 confirmed by DISCUSS choices; D5 in P1-Q1; D3 in US-01 Technical Notes; D1 in P2-Q8; D6 in P2-Q9
- All 5 research pitfalls now mitigated by constraint or AC

## Design Decisions Summary (2026-05-24, appended by DESIGN wave)

The DESIGN wave (`nw-solution-architect`, Morgan) consumed all 9
open questions surfaced by DISCUSS and produced the artifact tree
captured in `feature-delta.md` § "Wave: DESIGN / REF Artifact
index". Below is a compact summary; the canonical content lives in
`feature-delta.md` and ADRs 0054–0059.

### Open questions resolution (9 of 9)

| ID | Resolution | ADR |
|---|---|---|
| P1-Q1 (ProbeRunner placement / task graph) | `overdrive-worker`; per-alloc supervisor + per-probe-instance tokio task; matches K8s prober.Manager shape | ADR-0054 |
| P1-Q2 (Exec-probe cgroup placement) | `cgroup.procs` write Phase 1 (reuses `place_pid_in_scope` from ADR-0030); `clone3 + CLONE_INTO_CGROUP` deferred Phase 2+ pending `nix-rust/nix#2120` | ADR-0059 |
| P1-Q3 (ServiceFailureReason SemVer) | Single per-kind enum (not per-condition); `#[non_exhaustive]`; additive variants per ADR-0037 §5; wire projection kept in lockstep via property test | ADR-0056 |
| P2-Q4 (TOML defaults) | timeout 5s (diverges from K8s 1s, justified); intervals 2/2/10s (startup/readiness/liveness); max_attempts 30; failure_threshold 1/3 (readiness/liveness); success_threshold 1 | ADR-0057 |
| P2-Q5 (Streaming cap interplay) | 60s cap unchanged; deliberate non-decision in Phase 1; operator workflow is submit → cap → `alloc status` for slow-warming Services; per-spec knob deferred behind future operator-UX iteration | ADR-0056 |
| P2-Q6 (`--json` Probes shape) | `ProbeResultRowJson` via `utoipa::ToSchema` per ADR-0009 / ADR-0033 enrichment convention | ADR-0056 |
| P2-Q7 (Multi-probe AND/OR) | AND-of-all (every startup probe must Pass for Stable); witness names last-to-pass; OR-combinator reserved as future knob | ADR-0055 |
| P2-Q8 (Readiness successThreshold) | Default 1 (matches K8s); configurable upward; counter persisted as input in `View::readiness_consecutive_successes`; gate recomputed every tick | ADR-0055 |
| P2-Q9 (Cascading-restart rate-limiting) | Phase 1 single-node has no cascading surface; `Action::RestartAllocation` emitted unconditionally; future Phase 2+ `LivenessRestartGovernor` reconciler is non-breaking addition | ADR-0055 |

### New ADRs

- **ADR-0054** — ProbeRunner subsystem
- **ADR-0055** — ServiceLifecycleReconciler
- **ADR-0056** — ServiceSubmitEvent Stable/Failed evolution
- **ADR-0057** — `[[health_check.*]]` TOML spec
- **ADR-0058** — Default-probe inference ("honest by default")
- **ADR-0059** — Exec-probe cgroup placement

### SSOT artifacts touched

- `docs/product/architecture/brief.md` — appended §§ 75–87
  (Application Architecture extension)
- `docs/product/architecture/c4-diagrams.md` — appended Service
  Health-Check Probes section (L2 annotation + L3 ProbeRunner
  subsystem)

### New deferrals surfaced (5 P3, all non-blocking)

P3-Q10 (nwave-ai outcomes CLI tool absence), P3-Q11 (Phase 2+
cascading-restart governor), P3-Q12 (Phase 2+ clone3 migration),
P3-Q13 (Phase 2+ per-spec streaming-cap knob), P3-Q14 (Phase 2+
OR-combinator knob). None promised to operators; no `gh issue
create` required per CLAUDE.md § "Deferrals require GitHub
issues — AND user approval BEFORE creation". Each is captured in
`feature-delta.md` § "Wave: DESIGN / REF Open questions deferred"
with an explicit trigger for when to revisit.

### Verdict

READY-FOR-DEVOPS-AND-DISTILL.

## Changelog

- 2026-05-24 — DISCUSS wave decisions captured. Walking skeleton candidate confirmed as Slice 01 (default TCP-connect startup probe). DIVERGE wave absent — risk noted, mitigation is direct grounding in RCA-A + J-OPS-003 extension.
- 2026-05-24 — Research-alignment review actioned. See "Research Alignment Review (2026-05-24)" section above.
- 2026-05-24 — DESIGN wave appended. Six new ADRs (0054–0059); brief.md §§ 75–87; c4-diagrams.md Service Health-Check Probes section. All 9 open questions resolved. See "Design Decisions Summary" section above.
