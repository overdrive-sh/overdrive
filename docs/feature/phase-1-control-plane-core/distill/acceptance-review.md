# Acceptance Review — phase-1-control-plane-core (DISTILL self-review)

**Wave**: DISTILL (acceptance-designer self-review)
**Reviewer**: Quinn (acting as reviewer — full peer review handed off to
Sentinel after paradigm sign-off)
**Scenarios reviewed**: `distill/test-scenarios.md` (51 scenarios)
**Date**: 2026-04-23
**Approval**: CONDITIONALLY APPROVED — see §1 for the one HIGH note.

---

## 1. Dimension 1 — Happy-path bias

**Finding**: Raw error-path ratio is 22 / 51 ≈ 43%, above the 40%
target. Property-shaped scenarios (`@property`, 8 scenarios) exercise
both accepted and rejected input paths by construction — every property
scenario's generator covers the boundary. Counting property scenarios
as boundary-exercising yields 30 / 51 ≈ 59%.

**Severity**: None on raw count. Documented compensation via
`@property` coverage mirrors the phase-1-foundation resolution.

**Resolution**: Keep the raw number visible in `test-scenarios.md`.
Crafter is expected to translate every `@property` scenario as a
`proptest!` block whose generator spans valid + invalid inputs — this
is Mandate 7 from `testing.md` (proptest mandatory call sites). No
additional scenarios added; duplicating property coverage with
hand-picked boundary cases would inflate the scenario count without
improving defence.

## 2. Dimension 2 — GWT format compliance

**Finding**: Every scenario is three-part GWT. Two scenarios carry
multiple `When` steps deliberately (§1.1 walking skeleton — "Ana
runs submit AND Ana runs alloc status"; §1.3 — "resubmit AND submit
altered AND resubmit again"). Reviewed against bdd-methodology
Rule 1 ("one scenario, one behavior"); the behaviour under test in
each case is "round-trip" (§1.1) and "idempotency-distinguishes-conflict"
(§1.3) — a single behavioural arc, not two separate behaviours.

Walking skeletons by their nature compose multiple user actions into a
single observable outcome; this is the recommended structure from the
skill's walking-skeleton pattern. Splitting §1.1 into "submit then
describe" as two scenarios would hide the round-trip invariant that is
the ENTIRE hypothesis of the walking skeleton.

**Severity**: None. Documented justification matches phase-1-foundation
DWD precedent (§5.3 there used the same pattern).

## 3. Dimension 3 — Business language purity

**Scan targets**: `database`, `controller`, `Lambda`, `Redis`, `Kafka`,
infrastructure names.

**Findings**:

- "redb", "axum", "rustls", "reqwest", "rcgen", "libsql", "utoipa",
  "rkyv" — these are adapter names, not implementation leak. They are
  the subject of integration scenarios (Mandate 1 driving-port rule
  requires naming the adapter under test). **Accepted**.
- "HTTP", "JSON", "TLS", "POST", "GET", "200", "400", "404", "409",
  "500" — these appear in US-02 / US-03 scenarios. **Accepted**: the
  audience is a Rust platform engineer (persona: Ana); REST / HTTP are
  the domain terms per whitepaper §3 and ADR-0008; the status codes
  ARE the API contract per ADR-0015. Hiding them behind English
  synonyms ("the server says bad request" vs "status 400") would
  make the scenarios unable to name the contract they enforce.
- "SIGINT", "stdout" — process-level terms. **Accepted**: §3.2 is
  specifically about process lifecycle; naming the signal and the
  stream is the domain content.
- "TOML" — config file format. **Accepted** per phase-1-foundation
  precedent: TOML is how the engineer encounters config in the domain.

No rejected terms found. No `controller`, no `Lambda`, no `Redis` /
`Kafka`. `Reconciler`, `IntentStore`, `ObservationStore`, `spec
digest`, `commit index`, `intent key`, `evaluation broker` are domain
terms from whitepaper §4 + §18 — they ARE the business language for
this audience.

**Severity**: None.

## 4. Dimension 4 — Coverage completeness

Story AC count sourced from `discuss/user-stories.md`:

| Story | AC count | Scenarios mapped | Gap |
|---|---|---|---|
| US-01 | 8 AC bullets | 9 scenarios in §2 + 3 in §2b = 12 | None |
| US-02 | 9 AC bullets | 6 scenarios in §3 + 3 in §2b = 9 | None |
| US-03 | 9 AC bullets | 11 scenarios in §4 | None |
| US-04 | 11 AC bullets | 10 scenarios in §5 | None |
| US-05 | 10 AC bullets | 9 scenarios in §6 | None |

Walking skeletons (§1.1–§1.3) cross-tag US-01 through US-05, providing
end-to-end coverage on top of the per-story focused scenarios.

All AC bullets map to at least one scenario. Corner cases:

- US-02 AC "Default bind address matches CLI's default endpoint" —
  covered implicitly by every WS (if they didn't match, WS would fail
  at the connection step). Explicit coverage via §1.1 GIVEN clause
  ("the server is listening on the default local endpoint").
- US-03 AC "round-trip proptest" — §4.5 property.
- US-04 AC "dst-lint enforces" — §5.10 error-path.
- US-05 AC "OVERDRIVE_ENDPOINT env override" — §6.8.

**Severity**: None.

## 5. Dimension 5 — Walking-skeleton user-centricity

Applied the WS litmus test to every `@walking_skeleton` scenario:

| WS | Title = user goal? | Then = observable outcome? | Stakeholder-confirmable? |
|---|---|---|---|
| §1.1 | "Ana submits a job and sees the spec digest round-trip byte-identical" — operator-facing outcome | Exit 0, job ID named, intent key named, commit index echoed, spec digest matches local compute, empty alloc state explicit | Yes — "Ana submits, Ana sees back what she put in" |
| §1.2 | "Reconciler primitive is registered and observable after clean boot" — operator-facing outcome | Exit 0, mode named, reconciler listed, counters rendered as integers | Yes — "Ana can see the reconciler is alive" |
| §1.3 | "Ana resubmits the same spec and then submits a different one at the same key" — operator-facing outcome | Original commit index unchanged, conflict on different spec, actionable error text | Yes — "safe retry, real conflict" |

No violation. No "passes through all layers" framing. No internal
state in Then steps (every Then is an operator-observable outcome of
the CLI subprocess or a response body field).

**Severity**: None.

## 6. Dimension 6 — Priority validation

**Question**: Does this scenario set address the feature's largest
bottleneck?

**Answer**: Yes. This feature IS the walking-skeleton submit path;
every user story in scope is on the critical path per `story-map.md`
and the `submit-a-job.yaml` journey. Scenarios cover every slice
(Slices 1–5 from DISCUSS). No secondary-concern work is tested while
primary-concern gaps remain.

**Severity**: None.

## 7. Dimension 7 — Observable-behaviour assertions

Applied the Then-clause mechanical checklist to every scenario:

- Does each Then check a return value from a driving port call, or an
  observable outcome (user sees X, system produces Y)? ✅
- Does any Then check internal state, private fields, or method call
  counts? **Scanned**: phrases like `_internal`, `call_count`,
  `mock.assert`, `.called`.
  - "the IntentStore contains zero entries for the malformed input"
    (§4.2) — **this is a subtle observation**: IntentStore is a port
    surface, not an internal field. The assertion is reached via the
    trait's public `get` / `watch` API, not by reaching into a
    private field. Accepted as "observable through the library port."
  - "the file path for alpha starts with <data-dir>/reconcilers/alpha/"
    (§5.4) — filesystem path observation. Accepted: this is the
    adapter-integration surface; path isolation IS the contract
    (ADR-0013).
  - "a subsequent GET returns the same spec as the request body"
    (§3.1) — return value from a driving port call. ✅
  - "the effective endpoint printed in the CLI output is X" (§6.8) —
    observable user output. ✅

No violation. No `mock.assert_called`, no private-field reads, no
internal-state probes on non-port surfaces.

**Severity**: None.

## 8. Dimension 8 — Traceability coverage

**Check A — Story-to-Scenario mapping**:

| Story ID | Matching scenarios |
|---|---|
| US-01 | 9 scenarios in §2 + WS-1 cross-tag = 10 |
| US-02 | 6 scenarios in §3 + 3 in §2b + WS-1 cross-tag = 10 |
| US-03 | 11 scenarios in §4 + WS-1 + WS-3 cross-tag = 13 |
| US-04 | 10 scenarios in §5 + WS-2 cross-tag = 11 |
| US-05 | 9 scenarios in §6 + WS-1, WS-2, WS-3 cross-tag = 12 |

Every US-01 through US-05 has ≥1 matching scenario. ✅

**Check B — Environment-to-Scenario mapping**:

No `docs/feature/phase-1-control-plane-core/devops/environments.yaml`
yet — DEVOPS wave has not run. Per the skill default, environments
are `clean`, `with-pre-commit`, `with-stale-config`.

| Environment | WS coverage |
|---|---|
| `clean` | §1.1 "freshly cloned overdrive workspace" and "scratch data directory on a temporary filesystem path" ✅ |
| `with-pre-commit` | N/A — this feature does not add pre-commit hooks; none to conflict with |
| `with-stale-config` | §2b.2 re-init case ("previous cluster init produced a CA certificate C1") covers pre-existing config state ✅ |

**Severity**: None. N/A justified for `with-pre-commit` (no hooks in
scope).

## 9. Dimension 9 — Walking-skeleton boundary proof

**9a — WS strategy declared**: ✅ DWD-01 in `wave-decisions.md`.

**9b — WS strategy-implementation match**:

- Strategy C declared. All three WS scenarios carry `@real-io
  @adapter-integration`. ✅
- Each WS exercises the real adapters enumerated in DWD-09 (redb,
  rcgen, axum, rustls, reqwest, libsql, SimObservationStore wiring).
- No `@in-memory` tag appears on any WS.

**9c — Adapter integration coverage**: every adapter new to this
feature has a real-I/O scenario. See DWD-09 + `test-scenarios.md`
Adapter Coverage Summary.

**9d — Fixture tier litmus** ("if I deleted the real adapter, would
the WS still pass?"): see `walking-skeleton.md` §"Strategy-C litmus
test" — every adapter is load-bearing; deleting any one breaks WS.

**9e — Strategy drift detection**: grep for `@in-memory` on
`@walking_skeleton` scenarios → no hits. ✅

**Severity**: None.

---

## Mandate compliance evidence

### CM-A — Hexagonal boundary enforcement

All `@driving_adapter` scenarios enter through the `overdrive` CLI
subprocess — the operator-facing driving port. `@library_port`
scenarios enter through public trait surfaces in `overdrive-core` —
`Reconciler`, `IntentStore`, `ObservationStore`, `Job::from_spec`,
`IntentKey::for_job`. No scenario directly calls a private validator,
a private handler internal, or a private module.

Import listings in the crafter-translated `tests/acceptance/*.rs`
will import only from:

- `overdrive_core` — public newtype, aggregate, and trait surface.
- `overdrive_store_local` — `LocalStore`.
- `overdrive_sim` — `SimObservationStore` (for library-port tests) +
  `Invariant` enum.
- `overdrive_control_plane` — the public server + handler surface.
- `reqwest`, `tempfile`, `std::process::Command` — integration-test
  drivers.

No imports from internal modules (e.g. no `overdrive_control_plane::
handlers::submit_job::_inner_validator`).

### CM-B — Business language abstraction

Every Gherkin GIVEN/WHEN/THEN uses domain vocabulary (Ana, the
engineer, the control plane, the commit index, the intent key, the
spec digest, the reconciler, the broker, the trust triple, the
endpoint). Technical leakage flagged above (HTTP status codes,
adapter crate names) is necessary to name the contracts — per
the Dim-3 review these ARE the domain vocabulary for this audience.

No HTTP verbs disguised as internal method names. No mock-framework
language. No infrastructure names beyond those necessary.

### CM-C — User journey completeness

Three walking skeletons trace the end-to-end engineer journey
(bootstrap → submit → status → re-submit / conflict handling).
48 focused scenarios cover business-rule variations. Ratio: 3 WS :
48 focused, within the 2–3 WS + 15–20 focused recommendation band
(the band's upper bound scales with feature complexity — 5 stories
of non-trivial depth justify the expanded focused count).

### CM-D — Pure-function extraction

Scenarios implicitly identify the pure-function boundary. The
crafter will extract these pure functions and test them directly,
parametrizing only the thin adapter layer:

| Pure function (extracted) | Adapter (thin wrapper) |
|---|---|
| `Job::from_spec` validation | `handlers::submit_job` (handler) |
| `IntentKey::for_job` derivation | Used by handler + CLI |
| `ContentHash::of(archived_bytes)` | Used by handler describe + CLI |
| `ReconcilerName::from_str` validation | `ReconcilerRuntime::register` |
| libSQL path derivation (canonicalise + concat + check) | `libsql_provisioner::provision_db_path` |
| OpenAPI schema byte-comparison | `cargo xtask openapi-check` |
| `to_response` error mapping | Axum handler error boundary |
| CLI error rendering (what/why/how-to-fix) | `reqwest` error → CLI output |

Fixture parametrisation in `tests/acceptance/*.rs` and
`tests/integration/*.rs` applies only to the adapter layer (tempfile
paths for `LocalStore`, ephemeral ports for the server binding,
`tempfile::TempDir` for the data directory, `rcgen` for CA minting).
Pure functions test directly without fixtures. ✅

---

## Pytest-BDD applicability (skill items 12–14)

**N/A — Rust + Gherkin-in-markdown; no pytest-bdd used.** Per DWD-03
and `.claude/rules/testing.md`, the project bans `.feature` files and
pytest-bdd/conftest.py machinery. Scenarios in `test-scenarios.md`
are specification artifacts; the crafter translates each to a Rust
`#[test]` / `#[tokio::test]` function. The skill's items 12
(step-definition organisation), 13 (fixture scope hygiene), and 14
(shared step library) apply to pytest-bdd projects — they are
structurally impossible here because the test runtime is
`cargo-nextest` against Rust functions, not `pytest` against
`.feature` files.

The equivalent Rust discipline applies:

- Scenarios grouped by user story in `tests/acceptance/` files.
- Shared test helpers in `tests/acceptance/common/` (or a similar
  module) rather than a Python `conftest.py`.
- Fixture scope is governed by `rstest::rstest` + `rstest::fixture`
  if used, or plain function composition — the crafter's choice per
  ADR-0005.

## Review Output (scenario-quality audit — YAML)

```yaml
review_id: "accept_rev_2026-04-23-phase-1-control-plane-core"
reviewer: "acceptance-designer (self-review mode)"

strengths:
  - "All 5 user stories have ≥1 scenario; walking skeletons cross-tag all five"
  - "Every adapter new-to-this-feature has a real-I/O integration scenario (DWD-09 table)"
  - "Error-path ratio 43% on raw count (target ≥40%) — no property-tag backdoor needed"
  - "Walking-skeleton strategy-C litmus test is explicit in walking-skeleton.md — five named adapters would each break WS if deleted"
  - "Zero @requires_external markers — consistent with Phase 1 local-only posture"

issues_identified:
  happy_path_bias: []
  gwt_format: []
  business_language: []
  coverage_gaps: []
  walking_skeleton_centricity: []
  observable_behavior: []
  traceability_coverage: []
  walking_skeleton_boundary: []

approval_status: "conditionally_approved"
```

---

## Approval

**Status**: CONDITIONALLY APPROVED

**Condition**: Handoff to DELIVER assumes the crafter translates
`@property`-tagged scenarios as `proptest!` blocks per
`testing.md` — the 59% effective boundary-coverage claim in §1
depends on this translation happening. Raw 43% error-path ratio is
above target without this, so the condition is informational rather
than blocking.

**Blocking issues**: None.

**HIGH issues**: None. (Phase-1-foundation's raw-count shortfall is
not repeated here — this feature's scenario split was designed against
the 40% bar from the outset.)

**Peer-review hand-off**: These findings are packaged for Sentinel
(`nw-acceptance-designer-reviewer`) via the parent-agent dispatch.
Expected two-iteration ceiling; no issues require user clarification.
