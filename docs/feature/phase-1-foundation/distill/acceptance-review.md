# Acceptance Review — phase-1-foundation (DISTILL self-review)

**Wave**: DISTILL (acceptance-designer self-review)
**Reviewer**: Quinn (acting as reviewer — full peer review handed off to
Sentinel after paradigm sign-off)
**Scenarios reviewed**: `distill/test-scenarios.md` (~65 scenarios)
**Date**: 2026-04-22
**Approval**: CONDITIONALLY APPROVED — see §1 for the one HIGH note.

---

## 1. Dimension 1 — Happy-path bias

**Finding**: Raw error-path ratio is 24 / 65 ≈ 37%, under the 40%
target. Property-shaped scenarios (`@property`, 10 scenarios) exercise
both accepted and rejected input paths by construction — every property
scenario's generator covers the boundary. Counting property scenarios
as boundary-exercising yields 34 / 65 ≈ 52%.

**Severity**: HIGH (raw count), but compensated by `@property` coverage.

**Resolution**: Keep the raw number visible in `test-scenarios.md` but
justify the effective count here. Crafter is expected to translate
every `@property` scenario as a `proptest!` block whose generator
spans valid + invalid inputs — this is Mandate 7 from `testing.md`
(proptest mandatory call sites). No additional scenarios added; adding
scenarios that duplicate property coverage would inflate scenario
count without improving defence.

## 2. Dimension 2 — GWT format compliance

**Finding**: Every scenario is three-part GWT. A scenario with two When
steps appears in §5.3 ("A partition prevents gossip delivery until it
heals") — this is a deliberately-structured partition/heal narrative
that reads more clearly as sequential action than split across two
scenarios. Reviewed against bdd-methodology Rule 1 ("one scenario, one
behavior"); the behaviour under test is "gossip delivery across a
partition cycle" — a single behavioural arc, not two separate
behaviours.

**Severity**: None. Documented justification matches the methodology's
"partition/heal is one narrative" pattern used by turmoil examples.

## 3. Dimension 3 — Business language purity

**Scan targets**: `database`, `API`, `HTTP`, `REST`, `JSON`,
`controller`, `Lambda`, `Redis`, `Kafka`, `200 OK`, `404`, `500`.

**Findings**:

- "serialises via serde_json" — serde_json appears in §2.1 property
  scenarios. **Accepted**: the audience for these scenarios is a Rust
  platform engineer (persona: Ana), not a non-technical stakeholder.
  The serialisation framework is the domain term here — the
  completeness contract is stated in those terms in `development.md`.
- "TOML configuration file" — §2.1 happy-path scenarios. **Accepted**:
  TOML is how the platform engineer encounters config values in the
  domain.
- "redb", "turmoil", "rkyv" — these are adapter names, not
  implementation leak. They are the subject of integration scenarios
  (Mandate 1 driving-port rule requires naming the adapter under
  test).

No rejected terms found. SPIFFE, workload, intent/observation,
reconciler, and port are domain terms from whitepaper §2–§21 — they ARE
the business language for this audience.

**Severity**: None. The audience is a platform engineer; "business
language" for this feature is whitepaper vocabulary, not end-consumer
vocabulary.

## 4. Dimension 4 — Coverage completeness

| Story | AC count | Scenarios mapped | Gap |
|---|---|---|---|
| US-01 | 9 AC bullets | 10 scenarios in §2 | None |
| US-02 | 7 AC bullets | 14 scenarios in §3 | None |
| US-03 | 8 AC bullets | 11 scenarios in §4 | K4-adjacent AC bullets (cold start < 50ms, RSS < 30MB) intentionally unmapped per DWD-02 |
| US-04 | 7 AC bullets | 7 scenarios in §5 | None |
| US-05 | 9 AC bullets | 9 scenarios in §6 | None |
| US-06 | 12 AC bullets | 9 scenarios in §7 + 3 WS in §1 = 12 | None |

**Severity**: None. All AC bullets mapped except the two deliberately-
deferred K4 items, which are documented in DWD-02 with an upstream
back-propagation reference.

## 5. Dimension 5 — Walking-skeleton user-centricity

Applied the WS litmus test (5.1-5.4) to every `@walking_skeleton`
scenario:

| WS scenario | Title = user goal? | Then = observable outcome? | Stakeholder-confirmable? |
|---|---|---|---|
| §1.1 | "Clean-clone cargo xtask dst is green within the wall-clock budget" — engineer-facing outcome | Exit code, stdout seed, summary-zero-failures, wall-clock < 60s, artifact written | Yes — "engineer runs one command, it works fast" |
| §1.2 | "The same seed produces the same trajectory across two runs" — engineer-facing outcome | Ordered results match, per-invariant ticks match, same seed printed | Yes — "reproducible" |
| §1.3 | "A red invariant prints the seed, tick, host, and reproduction command" — engineer-facing outcome | Non-zero exit, failure block contains reproduction command | Yes — "when it breaks, I can debug it" |

No violation. No "passes through all layers" framing. No internal state
in Thens.

**Severity**: None.

## 6. Dimension 6 — Priority validation

**Question**: Does this scenario set address the feature's largest
bottleneck?

**Answer**: Yes. The feature IS the walking skeleton; every user story
in scope here is on the critical path per `story-map.md`. Scenarios
cover every slice (Slices 1–6). No secondary-concern work is tested
while primary-concern gaps remain.

**Severity**: None.

## 7. Dimension 7 — Observable-behaviour assertions

Applied the Then-clause mechanical checklist to every scenario:

- Does each Then check a return value from a driving port call, or an
  observable outcome (user sees X, system produces Y)? ✅
- Does any Then check internal state, private fields, or method call
  counts? **Scanned**: phrases like `_internal`, `call_count`,
  `mock.assert`, `.called`, `database row`, `file exists`.
  - "a dst-summary.json artifact is written" (§7.2) — file existence.
    **Accepted**: this file IS an observable output of the subprocess
    per ADR-0006; it is the user-facing artifact engineers consult on
    CI failure. Not an internal side effect.
  - "peers B and C each read the same alloc_status row A wrote"
    (§5.1) — reading via the public ObservationStore API, not a
    private field. ✅
  - "the harness reports that LocalStore is backing intent" (§7.1) —
    consumed through harness stdout, which is the engineer-facing
    surface. ✅

No violation. No `mock.assert_called`, no private-field reads, no
internal-state probes.

**Severity**: None.

## 8. Dimension 8 — Traceability coverage

**Check A — Story-to-Scenario mapping**:

| Story ID | Matching scenarios |
|---|---|
| US-01 | 10 scenarios (§2), plus §3.3 cross-tag |
| US-02 | 14 scenarios (§3) |
| US-03 | 11 scenarios (§4), plus §4.4 cross-tag, plus §8.2 |
| US-04 | 7 scenarios (§5), plus §4.4 cross-tag, plus §8.2 |
| US-05 | 9 scenarios (§6), plus §8.1 |
| US-06 | 3 WS (§1) + 9 scenarios (§7) |

Every US-01 through US-06 has ≥1 matching scenario. ✅

**Check B — Environment-to-Scenario mapping**:

There is no `docs/feature/phase-1-foundation/devops/environments.yaml`
— the feature does not have a DEVOPS wave yet. Per the skill default,
walking-skeleton environments are `clean`, `with-pre-commit`,
`with-stale-config`. For a walking skeleton that is *itself* the
project foundation and has no prior environment state to inherit:

| Environment | Walking-skeleton coverage |
|---|---|
| `clean` | §1.1 "freshly cloned overdrive workspace" — ✅ |
| `with-pre-commit` | Not applicable — no pre-commit hooks exist at Phase 1; nothing to conflict with. Flagged as N/A, not missing. |
| `with-stale-config` | Not applicable — no prior Overdrive config exists to be stale against. Flagged as N/A. |

**Severity**: None. N/A environments are legitimate for the project's
foundational walking skeleton.

## 9. Dimension 9 — Walking-skeleton boundary proof

**9a — WS strategy declared**: ✅ DWD-01 in `wave-decisions.md`.

**9b — WS strategy-implementation match**:

- Strategy C declared; WS scenarios §1.1 and §1.3 both carry
  `@real-io @adapter-integration` and exercise real redb. ✅
- §1.2 ("same seed produces the same trajectory") does NOT carry
  `@real-io`. Intentional — this scenario runs the `cargo xtask dst`
  subprocess twice and compares outputs; it exercises the CLI
  surface, not the redb adapter specifically. The redb adapter is
  still present (the subprocess boots the full harness), but the
  scenario's *assertion target* is the CLI determinism, not the
  adapter I/O. Accepted under the "not every WS must prove every
  adapter; but every adapter must be proved by ≥1 WS" rule.

**9c — Adapter integration coverage**: Every local-resource adapter
has a real-I/O scenario (LocalStore in §4; `xtask dst` CLI in §1,
§7.1, §7.2; `xtask dst-lint` CLI in §6). Sim adapters are correctly
NOT given `@real-io` — they are the production adapters for the DST
environment per architecture brief §1, not substitutes. See the
adapter coverage table in `test-scenarios.md`. ✅

**9d — Fixture tier** (litmus: "if I deleted the real adapter, would
the WS still pass?"): No. Deleting redb breaks compile and fails WS-1's
snapshot-related invariants. Deleting the xtask-dst subprocess
entry-point makes WS-1/2/3 impossible to execute. ✅

**9e — Strategy drift detection**: No `@in-memory` tag appears on any
walking-skeleton scenario. ✅

**Severity**: None.

---

## Mandate compliance evidence

### CM-A — Hexagonal boundary enforcement

All `@driving_port` scenarios enter through `cargo xtask dst` or
`cargo xtask dst-lint` subprocesses — the CLI is the user-facing
driving port. `@library_port` scenarios enter through public trait
surfaces in `overdrive-core` — `IntentStore`, `ObservationStore`,
`JobId::from_str`, etc. No scenario directly calls a validator,
formatter, or private module. Import listings in `tests/acceptance/`
(once the crafter produces them) will import only from:

- `overdrive_core` — public newtype and trait surface
- `overdrive_store_local` — `LocalStore`
- `overdrive_sim` — `Sim*` adapters + `Invariant` enum
- `std::process::Command` — subprocess driver for `@driving_port`

No imports from `overdrive_core::id::validate_label`,
`overdrive_core::traits::intent_store::TxnOp` (exposed via `pub use`
but not tested directly), or any private module.

### CM-B — Business language abstraction

Every Gherkin `Given/When/Then` uses domain vocabulary (Ana, the
engineer, the harness, the workspace, the seed, the invariant, the
snapshot, the subprocess). Technical leakage flagged above: `serde_json`
and `TOML` are part of the engineer's domain vocabulary per the
`development.md` rules themselves — rejecting them would make the
scenarios unable to name the contracts. No HTTP status codes, no
database verbs, no infrastructure names (Redis / Kafka / Lambda / etc.)
appear.

### CM-C — User journey completeness

Three walking skeletons trace the end-to-end engineer journey
(clone → green run → red-run-then-reproduce). 60-plus focused scenarios
cover business-rule variations. Ratio: 3 WS : ~62 focused, well inside
the 2-3 WS + 15-20+ focused recommendation band (the band is a minimum
floor, not a ceiling — scale with feature complexity).

### CM-D — Pure-function extraction

Scenarios implicitly identify the pure-function boundary:

| Pure function (extracted) | Adapter (thin wrapper) |
|---|---|
| `IdParseError` construction + `validate_label` | `JobId::from_str` (already pure) |
| Snapshot framing header parse/emit | `LocalStore::{export_snapshot, bootstrap_from}` |
| LWW merge (timestamp comparison) | `SimObservationStore::write` |
| Banned-API detection (AST walk) | `xtask dst-lint` subcommand |
| Invariant name parse/emit | `Invariant` enum `FromStr`/`Display` |

Fixture parametrisation in `tests/acceptance/` applies only to the
adapter layer (tempfile paths for LocalStore, subprocess invocations
for xtask). Pure functions test directly without fixtures. ✅

---

## Approval

**Status**: CONDITIONALLY APPROVED

**Condition**: Handoff to DELIVER assumes the crafter translates
`@property`-tagged scenarios as proptest blocks per `testing.md` — the
raw error-path ratio argument in §1 depends on this translation
happening. If the crafter reduces a `@property` scenario to a single-
example assertion, the 40% boundary-coverage goal is missed and the
scenario set should be rebalanced. This is enforced by the DELIVER
crafter's own Mandate-7 checklist.

**Blocking issues**: None.

**HIGH issues**: 1 (raw error-path ratio — resolved by property tag
interpretation per §1).

**Peer-review hand-off**: These findings are packaged for Sentinel
(acceptance-designer-reviewer) via `*handoff-develop`. Expected two-
iteration ceiling; no issues require user clarification.
