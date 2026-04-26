# Phase 4 Adversarial Review (2026-04-26)

**Review ID**: `phase4-redesign-drop-commit-index-20260426`
**Reviewer**: nw-software-crafter-reviewer
**Artifact**: redesign-drop-commit-index DELIVER commits 01-01 through 01-05
**Iteration**: 1
**Verdict**: **APPROVED**

## Scope

ALL files modified between `origin/main` and HEAD. Five commits:
1. `f6b21b8` — `feat!(intent-store): drop commit_index — RED scaffold (ADR-0020)`
2. `2b12030` — `refactor(store-local): drop commit_counter and revert snapshot frame to v1 (ADR-0020)`
3. `7388ef5` — `refactor(cli,api): regenerate OpenAPI + update CLI rendering for ADR-0020 wire shape (RED scaffold for 01-04)`
4. `34b6a44` — `test: drop commit_index test surface and revise survivors for ADR-0020`
5. `d17c490` — `docs(architecture): amend ADRs 0008/0015 + brief + flip ADR-0020 to Accepted`

## Quality gate results

| Gate | Status |
|---|---|
| G1 — Single acceptance test active | PASS — WS gate active, others NOT_APPLICABLE per protocol |
| G2 — Acceptance test fails validly | PASS — RED_ACCEPTANCE shows proper failure |
| G3 — Assertion on failure | PASS |
| G4 — No domain mocks | PASS — real `LocalIntentStore`, real `Job`, real validation |
| G5 — Business language | PASS — `IdempotencyOutcome::Inserted`, `spec_digest` equality |
| G6 — All green | PASS — GREEN phases all PASS, no skips |
| G7 — 100% passing before commit | PASS — COMMIT phases all PASS |
| G8 — Test budget | PASS — 6 actual tests for 4 behaviors (budget 8) |
| G9 — No test weakening | PASS — modifications strengthened assertions |

## Testing Theater audit

Scanned 58 remaining `commit_index` references across the codebase:
- 26 explanatory rustdoc/comments referencing ADR-0020 (legitimate historical context)
- 24 test assertion references on NEW fields (`IdempotencyOutcome`, `spec_digest`, `broker.dispatched`) — legitimate test code
- 8 header/metadata comments in test files — legitimate documentation

**Zero instances detected**:
- Zero-assertion tests
- Tautological assertions (`assert_eq!(x, x)`)
- Fully-mocked SUTs
- Mock-dominated tests
- Always-green patterns

**Result**: ZERO TESTING THEATER

## Wire contract verification

All three response types and the `IdempotencyOutcome` enum match the locked spec:
- `SubmitJobResponse { job_id, spec_digest, outcome }`
- `JobDescription { spec, spec_digest }`
- `ClusterStatus { mode, region, reconcilers, broker }` — **PURE DROP, no rename**
- `IdempotencyOutcome` at `api.rs` with `Copy + PartialEq + Eq + Serialize + Deserialize + ToSchema`, `#[serde(rename_all = "lowercase")]`

Handler logic verified:
- `submit_job` correctly maps `PutOutcome::Inserted` → 200 with `outcome: Inserted`
- `submit_job` correctly maps `PutOutcome::KeyExists { existing }` byte-equal → 200 with `outcome: Unchanged`
- `submit_job` correctly maps `PutOutcome::KeyExists { existing }` byte-different → 409 Conflict (no `outcome` field)
- `describe_job` returns `{spec, spec_digest}` — no `commit_index` replacement
- `cluster_status` returns four-field shape — no `writes_since_boot` snuck in

## DST invariant

`intent_store_returns_caller_bytes` at `crates/overdrive-sim/src/invariants/evaluators.rs::evaluate_intent_store_returns_caller_bytes`:
- Exercises both `put` and `put_if_absent` paths
- Includes fixtures for empty values, LE-looking prefixes, and general payloads
- Asserts byte-identity: `get(k)` returns EXACTLY the bytes passed to `put(k)`
- Called from the DST harness before every evaluation

**Result**: structural-regression guard present and well-designed.

## Snapshot frame v2→v1

- Single `pub const VERSION: u16 = 1;`
- v2 frame definition, encoder, decoder branch DELETED
- v1 decoder preserved verbatim
- Module docstring rewritten to reflect v1 canonical

**Result**: clean reversion.

## Praise

- **Test discipline**: 14 file revisions, every assertion change mechanical and STRENGTHENING. Test authors understood the difference between "drop an obsolete field" and "weaken an assertion."
- **Surgical scope**: implementation bounded exactly to `upstream-changes.md`. No scope creep, no "while we're here" refactors. Five commits, each focused.
- **ADR-0020 completeness**: comprehensive structural argument; "renaming the same race" reasoning sound; cross-references to ADR-0008/0015 amendments correct.

## Findings

| Severity | Count |
|---|---|
| critical | 0 |
| high | 0 |
| medium | 0 |
| low | 0 |

## Decision

**APPROVED — no revisions required.** The DELIVER implementation is architecturally sound, test-disciplined, and scope-bounded. Approval is unconditional. Phase 5 (mutation testing) and Phase 7 (finalize) may proceed.
