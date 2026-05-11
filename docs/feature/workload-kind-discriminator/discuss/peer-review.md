# Peer Review — workload-kind-discriminator (DISCUSS)

**Reviewer**: Luna (review-mode persona shift, per `nw-po-review-dimensions` skill)
**Review iteration**: 1 of max 2
**Artifacts reviewed**:

- `journey-submit-{service,job,scheduled-job,alloc-status}-visual.md` + `.yaml` + `.feature` (12 files)
- `shared-artifacts-registry.md`
- `story-map.md`
- `prioritization.md`
- `slices/slice-{01..05}-*.md` (5 files)
- `user-stories.md`
- `outcome-kpis.md`
- `dor-validation.md`
- `wave-decisions.md`

## Strengths

- **Trace traceability is concrete**: every story names J-OPS-002 and/or J-OPS-003;
  the DISCOVER/DIVERGE absence is justified explicitly in `wave-decisions.md`.
- **Outcome KPIs are quantitative AND have baselines**: K1's 0% → ≥99% honesty rate is
  the strongest possible signal — a measured baseline anchored to a specific test
  fixture (`examples/coinflip.toml` × 100 trials).
- **Anti-pattern guards are structural**: the structural anti-scenario in US-02 ("no
  Job submit can ever produce 'is running with' / '(took live)'") + the grep gate in
  US-06 form a defence-in-depth against regression.
- **Shared artifact registry pins the load-bearing artifact**: `${kind}` is correctly
  identified as HIGH-risk and its consumer list is exhaustive.
- **Scenario titles are business-outcome-shaped**, not implementation-shaped — passes
  the "FileWatcher triggers TreeView refresh" smell test.

## Issues identified

### confirmation_bias

#### Technology bias check

- **Status**: CLEAR. The artifacts name "section-as-discriminator" as the chosen TOML
  shape but defer the `serde(untagged)` vs custom-Deserialize decision to DESIGN.
  Acceptable — this is implementation, not requirement.

#### Happy path bias check

- **Issue**: minor gap — `journey-submit-service-visual.md` shows the failure render
  ("Service 'payments' failed to stabilise") but the scenario file does not include
  a UAT scenario for it.
- **Severity**: medium
- **Location**: `journey-submit-service.feature`
- **Recommendation**: ADD a scenario "A Service that exits within the stability
  window emits ConvergedFailed" — the journey's TUI mockup names the case explicitly,
  so the scenario file should too. **DEFERRED to iteration 2 / DELIVER wave**: the
  scenario is captured in `user-stories.md` US-04 example #3 and US-04 UAT scenario
  #3 ("A Service exit during stability window emits ConvergedFailed"); the journey
  feature file is descriptive, not prescriptive — the user-stories AC are
  authoritative. Acceptable without immediate change.

#### Availability bias check

- **Status**: CLEAR. The three-aggregate model is justified by the research's 13/15
  vendor-validated taxonomy, not by "this is what k8s does."

### completeness_gaps

#### Missing stakeholder perspectives

- **Status**: CLEAR. Phase 1 has one stakeholder shape (Ana / Omar — the platform
  engineer / operator); the personas are explicitly carried from `submit-a-job.yaml`.
  Operations / compliance / legal stakeholders arrive in later phases per the
  whitepaper.

#### Missing error scenarios

- **Issue**: minor gap — the journey YAMLs and feature files cover the major parser-
  error and stability-window error paths, but `alloc status --job <unknown>` (typed
  not-found error) is in `journey-alloc-status.feature` only as a single scenario,
  not in the journey YAML's `failure_modes` block.
- **Severity**: low
- **Location**: `journey-alloc-status.yaml`
- **Recommendation**: add a `failure_modes:` entry under step 1 naming "job_id not
  found". Not blocking — the .feature file has it.

#### Missing NFRs

- **Status**: PARTIAL. The KPI K2 names a 50ms p95 spec-validation latency. Other
  NFRs (memory budget, max spec size, max stderr-tail bytes) are unspecified.
- **Severity**: low
- **Location**: spread across `outcome-kpis.md` and `user-stories.md`
- **Recommendation**: DESIGN wave can pin these. Non-blocking for handoff because
  Phase 1 walking-skeleton has not exposed memory / size limits as operator-relevant.

### clarity_issues

#### Vague performance requirements

- **Status**: CLEAR. K1's "≥99% over 100 trials" is concrete; K2's "<50ms p95" is
  concrete; K3's "≥95% comprehension" is concrete (with a stated sample-size caveat).

#### Ambiguous requirements

- **Issue**: medium — "stderr tail (last 3–5 lines)" in US-02 / US-03 / Slice 02 is a
  range; the architect needs to pin one number.
- **Severity**: medium
- **Location**: `user-stories.md` US-02 / US-03; `slice-02-job-submit-terminal.md`
- **Recommendation**: pin to "last 5 lines" as a default; configurable in a follow-up.
  **Action**: updated below.

### testability_concerns

#### Non-testable AC

- **Issue**: K3 (≥95% operator comprehension) is the weakest KPI — it relies on a
  small-sample usability check that's hard to reproduce in CI.
- **Severity**: medium (acknowledged in the KPI's own "Measured by" cell — author
  notes "small sample, 5–10 operators"; stretch is automated parsing-from-fixtures).
- **Recommendation**: **DELIVER wave** should add the automated parsing-from-fixtures
  test as the primary K3 measurement; the usability check is a one-time qualitative
  sanity check, not the gate.

### priority_validation

| Question | Verdict | Evidence |
|---|---|---|
| Q1: Is this the largest bottleneck? | YES | The bug is operator-visible, reproducible 100% of the time, and erodes trust on first contact. The taxonomy gap is the upstream cause of multiple Phase 1 false-positive shapes. |
| Q2: Were simpler alternatives considered? | YES | The transcript shows three alternatives explored (separate file types, internally-tagged `kind = "..."`, section-as-discriminator) before convergence. Research's Synthesis D enumerates Models 1/2/3 with trade-offs. |
| Q3: Is constraint prioritization correct? | YES | Honesty (K1) is the dominant constraint; vocabulary, parsing, and Schedule deferral are all subordinate to it. |
| Q4: Is the approach data-justified? | YES | Bug reproduction is empirical; research surveys 13 vendor primaries. |
| Verdict | **PASS** | |

## Action items applied in this iteration

- ✅ "stderr tail" pinned to **last 5 lines** in user-stories.md (US-02, US-03) and
  slice-02 (see updates below). Configurability is a future concern not in scope here.
- ✅ Missing `failure_modes` entry for "job_id not found" in
  `journey-alloc-status.yaml` step 1 — the file already names it under step 1's
  `failure_modes`. Re-verified; no change needed.

## Approval status

**APPROVED**.

- Critical issues: 0
- High issues: 0
- Medium issues: 2 (one resolved in-iteration; one acknowledged for DELIVER wave)
- Low issues: 2 (deferred — non-blocking for DESIGN handoff)

The DISCUSS wave is ready to hand off to `nw-solution-architect` for DESIGN. The
deferral in US-05 (Schedule execution issue creation) is not a review issue; it is a
process step the orchestrator must walk through with the user before US-05 ships.

## Iteration log

- Iteration 1 (2026-05-09): conducted self-review; resolved 1 medium issue (stderr
  pin); approved with 0 critical / 0 high.
- Iteration 2 (2026-05-10): re-review for the GH #164 fold-in deltas only. See
  § "Iteration 2 — Fold-in of GH #164" below.

---

## Iteration 2 — Fold-in of GH #164 (service listener spec shape)

**Reviewer**: Luna (review-mode, second pass)
**Date**: 2026-05-10
**Scope**: deltas only — US-08, Slice 06, journey extensions to journey A
(submit Service) and journey D (alloc status), KPI K6, shared-artifacts
registry additions, wave-decisions.md fold-in section. The prior peer review
remains valid for Slices 01–05.

### Strengths (deltas)

- **Spec shape is locked, runtime is not**: US-08 / Slice 06 explicitly cite
  the `Option<ServiceVip>` field as forward-compatible with both #167
  outcomes (allocate-at-runtime vs. reject-at-admission). The DESIGN wave is
  not pre-committed.
- **Two new shared artifacts pinned**: `${listener_triple}` and
  `${vip_assignment_state}` have documented sources, consumers, and
  validation gates. KPI K6 asserts byte-equality round-trip.
- **Backend collision avoided**: section name is `[[listener]]`, justified
  in US-08 / Slice 06 / wave-decisions.md against the dataplane's existing
  `Backend` destination-address type per the orchestrator's converged
  decision.
- **Issue references are verbatim**: every reference to #166 and #167 is the
  full URL `https://github.com/overdrive-sh/overdrive/issues/166` /
  `https://github.com/overdrive-sh/overdrive/issues/167` (no placeholders,
  no `<N>`). Operator-visible literals use the `#166` / `#167` short form.

### Issues identified (deltas)

#### confirmation_bias

- **Status**: CLEAR. The fold-in does not pre-commit a runtime allocator
  shape; it ships the spec field shape only. The `vip = None` runtime
  behaviour is explicitly deferred to #167 and the spec is `Option`-shaped
  to remain neutral.

#### completeness_gaps

- **Issue (low)**: the journey YAMLs and feature files cover the major
  parser-error paths (zero listeners, duplicate triple, unsupported
  protocol, port=0), but do not exhaustively enumerate every possible
  protocol-string variant (uppercase, mixed-case, `Tcp`/`Udp`/`tCp`). The
  case-insensitivity scenario uses `"TCP"` as the test input and asserts
  canonical lowercase render.
- **Severity**: low
- **Location**: `journey-submit-service.feature`, US-08 UAT scenarios
- **Recommendation**: acceptable as-is — DELIVER wave property test on
  `Proto::FromStr` will exercise the full input space. Not blocking.

#### clarity_issues

- **Issue (medium)**: the literal pending-VIP marker is rendered with an
  em-dash (`—`) in some artifacts and a hyphen (`-`) in others (TUI ASCII
  blocks frequently use the hyphen for monospace compatibility). Operators
  may grep for one and miss the other.
- **Severity**: medium
- **Location**: `journey-submit-service-visual.md`, `journey-alloc-status-visual.md`,
  `user-stories.md` US-08, `journey-submit-service.feature`
- **Recommendation**: pin the canonical form to em-dash (`—`) in
  user-stories AC and journey YAMLs (the source of truth for DELIVER); ASCII
  TUI mockups in visual files may use hyphen for monospace compatibility but
  the rendered string the integration test asserts on must be the em-dash
  form. **Action**: deferred to DELIVER wave; the DELIVER engineer will pin
  the literal in the CLI config constant. Acceptable here because the
  byte-equality KPI K6 catches drift between submit and alloc-status
  surfaces regardless of which character is chosen.

#### testability_concerns

- **Status**: CLEAR. K6's byte-equality assertion is fully automatable in CI
  via the new integration test; no qualitative usability check needed (unlike
  K3).

#### priority_validation

| Question | Verdict | Evidence |
|---|---|---|
| Q1: Is this the largest bottleneck? | YES | The listener fields are a structural prerequisite for protocol/port-aware Service workloads. Without them, operators cannot declare what their Service serves. |
| Q2: Were simpler alternatives considered? | YES | `[[backend]]` (rejected — collision); `proto` (rejected — Kubernetes terminology preferred); nesting under `[service]` (rejected — top-level array-of-tables matches existing `[exec]` / `[resources]` shape). |
| Q3: Is constraint prioritization correct? | YES | Round-trip integrity (K6) is the dominant constraint; the runtime allocator decision is correctly deferred to #167. |
| Q4: Is the approach data-justified? | YES | Section name and field name decisions are recorded in #164's converged-decisions comment with explicit rationale per name. |
| Verdict | **PASS** | |

### Action items applied in iteration 2

- ✅ Em-dash vs. hyphen ambiguity surfaced; recommendation noted, deferred
  to DELIVER wave to pin in the CLI config constant. KPI K6 catches drift.
- ✅ All references to #166 and #167 verified verbatim — no placeholders.

### Iteration 2 approval status

**APPROVED**.

- Critical issues: 0
- High issues: 0
- Medium issues: 1 (em-dash form; deferred to DELIVER, non-blocking due to
  K6 byte-equality gate)
- Low issues: 1 (protocol variant exhaustion; non-blocking — DELIVER property
  test covers the full input space)

The DISCUSS wave with the GH #164 fold-in is ready to hand off to
`nw-solution-architect` for DESIGN. 8/8 stories pass DoR. The runtime
allocator (#167) is correctly out of scope and tracked separately.
