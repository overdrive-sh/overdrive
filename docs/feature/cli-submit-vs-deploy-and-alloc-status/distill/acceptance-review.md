# Acceptance Review — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DISTILL
**Date**: 2026-04-30
**Reviewer**: self-review pass against `nw-ad-critique-dimensions`
(Dimensions 1–9). External `nw-acceptance-designer-reviewer`
dispatch deferred — subagent execution context cannot dispatch
sibling agents; the parent orchestrator is expected to dispatch the
reviewer as a separate Agent invocation against this wave's outputs.

---

## Strengths

- **Tier 1 / Tier 3 split is principled.** The split follows the
  rule from `.claude/rules/testing.md` — DST tier for property /
  ordering / cap-fires logic; real-kernel tier strictly for
  syscall-propagation regressions. Three Tier-3 scenarios
  (`S-WS-01`, `S-WS-02`, `S-CLI-03`) is the minimum that satisfies
  driving-adapter verification + the KPI-02 regression target without
  inflating CI runtime.
- **Single-source-of-truth is type-system-enforced, not
  discipline-enforced.** `S-AS-02` is a compile-time
  type-equivalence assertion; the same `TransitionReason` enum on
  both surfaces makes KPI-04 a structural property. The Tier-3
  byte-equality scenario (`S-WS-02`) is the structural-end-to-end
  proof of the same property under real I/O.
- **Property-shape scenarios are inventoried separately** (§5 of
  `test-scenarios.md`), making the proptest generator surface
  explicit for the crafter.
- **RED-vs-BROKEN scaffold scoping is explicit.** DWD-03 enumerates
  exactly which 5 of the 9 net-new types compile cleanly as
  scaffolds vs which 4 must wait for slice 01 GREEN's cross-cutting
  derive change. `cargo check` was run; clean.
- **Driving-adapter verification is met** with two scenarios that
  invoke the real CLI subprocess against the real HTTP API
  (`S-WS-01`, `S-WS-02`).

---

## Issues identified

### Dim 1 — Happy-path bias (note, not blocker)

- **Issue**: error-path scenario ratio is 10 / 26 ≈ 38%, marginally
  below the 40% target.
- **Severity**: low.
- **Mitigation**: several happy-path scenarios are property tests
  with generators that include error-shape inputs (`S-CP-09` covers
  every `AllocState` including the new `Failed`; `S-AS-07` covers
  every `TransitionReason` including the three failure-class
  variants). The effective error coverage is higher than the headline
  ratio suggests.
- **Recommendation**: accept as-is; the coverage matrix is exhaustive
  on AC bullets and KPIs. If a reviewer pass insists on raising the
  ratio, add boundary-edge scenarios for capacity calculations on the
  snapshot side — but those duplicate `S-AS-08` shape and add little
  signal.

### Dim 2 — GWT compliance

- **Result**: PASS. Every scenario has Given / When / Then; no
  multi-When in any scenario; Then steps assert observable user
  outcomes (exit codes, stdout content, response status, byte-equality
  across surfaces) rather than internal state.

### Dim 3 — Business language purity (note)

- **Issue**: the Gherkin in `S-CP-01`, `S-CP-08`, `S-CLI-01`,
  `S-CLI-02`, `S-CLI-06` uses the literal tokens
  `Accept: application/x-ndjson` / `Accept: application/json`.
  Strictly, these are protocol primitives, not pure business
  language.
- **Severity**: low. The tokens are part of the operator-visible
  contract: the user (operator) literally types `--detach` to flip
  the header. The header IS the domain primitive at this surface;
  abstracting it to "the streaming opt-in flag" loses precision and
  testability.
- **Mitigation**: technical tokens are confined to the precondition
  / observation slots; no `axum::Router::oneshot`, no
  `tokio::sync::broadcast`, no `serde_json` appears in any Gherkin
  block. Those technical surfaces appear ONLY in the per-scenario
  metadata ("Driving port", "Sim/real adapter substitutions",
  "Asserts").
- **Recommendation**: accept. The HTTP headers are the
  user-observable wire contract here; precision wins.

### Dim 4 — Coverage completeness

- **Result**: PASS. Every US-NN AC bullet and every KPI-NN binds to
  ≥ 1 scenario in `test-scenarios.md` § 1.

### Dim 5 — Walking-skeleton user-centricity

- **Result**: PASS. Both `S-WS-*` scenarios are titled around the
  operator's goal ("Operator submits a healthy spec and the verb
  tells the truth on success"; "Operator submits a broken-binary
  spec and the verb names the cause"), not around technical layer
  connectivity. A non-technical stakeholder can confirm "yes, that
  is what an operator needs" — see `walking-skeleton.md` for the
  demo session transcript.

### Dim 6 — Priority validation

- **Result**: PASS. KPI-02 (broken-binary surfaces failure inline)
  is the load-bearing KPI per `outcome-kpis.md` ("its failure means
  the feature has not delivered"); `S-WS-02` IS the boolean test
  for it. Priority is correctly inverted from "happy-path first" to
  "regression-target first" — the user's actual complaint
  session.

### Dim 7 — Observable behavior assertions

- **Result**: PASS. Every Then step asserts return values,
  observable outputs, or business outcomes (exit codes, response
  status, response body, stdout substrings, byte-equality across
  surfaces). No assertions on private fields or method-call counts.

### Dim 8 — Traceability coverage

- **Check A (story-to-scenario)**: PASS. Every US-NN has ≥ 1
  matching scenario; the coverage matrix in § 1 maps explicitly.
- **Check B (environment-to-scenario)**: degraded — no
  `docs/feature/.../devops/environments.yaml` exists for this
  feature (no DEVOPS wave was run; per project context there's no
  deployment-environment surface that this feature needs to
  enumerate against). Per the skill's graceful-degradation rule:
  warning logged, proceeded with default matrix
  `clean | with-pre-commit | with-stale-config`. The Tier-3
  scenarios (`S-WS-01`, `S-WS-02`, `S-CLI-03`) implicitly cover
  the `clean` environment — they spawn fresh tempdirs per run.
  The `with-pre-commit` and `with-stale-config` environments are
  not relevant to this feature (Phase 1 single-node, no
  pre-commit hooks in scope, no stale-config concept). NOT a
  blocker.

### Dim 9 — Walking-skeleton boundary proof

- **9a (WS strategy declared)**: PASS — DWD-01 declares WS waived;
  `walking-skeleton.md` records the rationale.
- **9b (WS implementation matches strategy)**: N/A — no formal WS;
  the structural-end-to-end S-WS-* scenarios are tagged correctly
  (`@walking_skeleton @driving_adapter @real-io`).
- **9c (every driven adapter has @real-io coverage)**: PASS — see
  the adapter coverage table in `test-scenarios.md` § 2. Every
  driven adapter has either a Tier-3 `@real-io` scenario or a
  rationale for Tier-1-only (`Clock`, broadcast channel, CLI
  rendering — no real I/O surface to validate).
- **9d (WS fixture tier)**: PASS — `S-WS-01` and `S-WS-02` use real
  `LocalIntentStore` (redb), real `LocalObservationStore`, real
  `ExecDriver` against real binaries, real `tokio::process` for
  the CLI subprocess, real `reqwest` for HTTP streaming. If you
  deleted the real adapter, these tests would not pass — the
  litmus test is satisfied.
- **9e (strategy drift detection)**: N/A — WS is waived; the
  question of "@in-memory under Strategy B/C/D" doesn't apply.

---

## Approval status

**conditionally_approved** — pending external reviewer dispatch by
the parent orchestrator. The 9-dimension self-review passes;
remaining concerns are notes, not blockers.

If the parent orchestrator dispatches `nw-acceptance-designer-
reviewer` as a separate Agent invocation, this document and the
catalogue are ready for review against the same dimensions.

---

## Mandate compliance proof

| Mandate | Evidence |
|---|---|
| **CM-A** Driving ports only | `test-scenarios.md` § 3 — every "Driving port" line names an `axum::Router::oneshot` invocation, a real subprocess `Command::new("overdrive")` invocation, or a pure CLI rendering function. Zero "Driving port" lines name internal types directly except for compile-time type-equivalence assertions (`S-AS-02`), which is a structural check, not a behaviour test. |
| **CM-B** Business-language Gherkin | `grep -E "axum::|tokio::|serde::|broadcast::|reqwest::"` against the Gherkin blocks of `test-scenarios.md` returns 0 hits in the Gherkin (these tokens appear ONLY in metadata blocks). |
| **CM-C** User journey completeness | `S-WS-01` / `S-WS-02` carry the operator from spec edit through commit through convergence to terminal exit code; `walking-skeleton.md` provides the literal demo transcript a stakeholder can confirm. |
| **CM-D** Pure functions before fixtures | The CLI rendering scenarios (`S-AS-04`, `S-AS-05`, `S-AS-06`) are written against pure rendering functions taking a typed `AllocStatusResponse` — no fixture-tier parametrisation. The Tier-3 scenarios are explicitly the adapter-layer tests; fixture parametrisation (real `tempfile::TempDir`, real `Command::new`) applies only to that adapter layer. |

---

## References

- `nw-ad-critique-dimensions/SKILL.md` (loaded).
- `distill/test-scenarios.md` (this wave).
- `distill/walking-skeleton.md` (this wave).
- `distill/wave-decisions.md` (this wave).
