# DISTILL Decisions — workload-kind-discriminator

**Wave**: DISTILL
**Feature**: workload-kind-discriminator
**Date**: 2026-05-10
**Author**: Quinn (nw-acceptance-designer)
**Mode**: subagent (autonomous; user has standing approval for auto-detected
walking-skeleton strategy per dispatch contract)

## Configuration captured at dispatch

| Field | Value |
|---|---|
| feature_id | `workload-kind-discriminator` |
| paradigm | OOP (Rust trait-based; per project `CLAUDE.md`) |
| implementer | `@nw-software-crafter` |
| output_format | Markdown specs only — NO `.feature` files (per `.claude/rules/testing.md` § Testing) |
| test layout (forecast) | `crates/{crate}/tests/integration/<scenario>.rs` per `.claude/rules/testing.md` § "Integration vs unit gating" |
| review_enabled | true (orchestrator runs `nw-acceptance-designer-reviewer` after handoff) |
| output_directory | `docs/feature/workload-kind-discriminator/distill/` |

---

## DWD-01 — Output convention: Gherkin lives in markdown, never executes

DWD-01 is the rule that overrides the skill default: this project does NOT
compile or execute Gherkin. Per `.claude/rules/testing.md` § Testing,
*"All acceptance and integration tests are written directly in Rust using
`#[test]` / `#[tokio::test]` functions. Gherkin-style scenarios may appear
as GIVEN/WHEN/THEN blocks in `docs/feature/{id}/distill/test-scenarios.md`
for specification purposes only — they are never parsed or executed."*

DISTILL output therefore is:

- `test-scenarios.md` — Gherkin specifications, never compiled.
- `walking-skeleton.md` — narrative naming the minimum end-to-end path.
- `wave-decisions.md` (this file) — decisions + tier mapping.
- `acceptance-review.md` — self-review against skill checklist + project items.

Crafter responsibility (DELIVER) per `.claude/rules/testing.md`:

- Translate every scenario in `test-scenarios.md` into Rust integration
  tests at `crates/overdrive-cli/tests/integration/<scenario>.rs` (or the
  appropriate crate per scope).
- Apply the `#[should_panic(expected = "RED scaffold")]` convention for
  unimplemented scenarios per `.claude/rules/testing.md` § "RED scaffolds
  and intentionally-failing commits".
- Gate slow/real-IO surfaces behind the `integration-tests` feature per
  the same rule.

DISTILL does NOT create RED scaffolds in `src/` and does NOT generate
empty `.feature` files. Tests are tagged in markdown via the
`@<tag>` convention purely for traceability; the crafter encodes
those tags as Rust attributes (`#[test]`, gated `#[cfg(feature = ...)]`,
`#[should_panic(...)]`, or filename / module placement).

---

## DWD-02 — Walking-skeleton strategy = Strategy A (in-process direct-handler)

**Auto-detected strategy**: **Strategy A** — in-process direct-handler
invocation against a real-but-ephemeral control-plane server, with
`SimClock` / `SimTransport` injected into the test harness, and
`tempfile::TempDir`-scoped real `redb` IntentStore + ObservationStore
files for the persistence boundary.

**Rationale** (recorded for reviewer):

1. The CLI crate has a firm rule (`crates/overdrive-cli/CLAUDE.md` §
   "Integration tests — no subprocess"): tests call command handlers
   directly as Rust functions, never spawn the binary. That rule
   already pins the entry point shape.
2. The existing CLI integration test corpus
   (`crates/overdrive-cli/tests/integration/*.rs`, 22 files including
   `streaming_submit_happy_path.rs`, `walking_skeleton.rs`,
   `job_submit.rs`, `endpoint_from_config.rs`) all use this shape;
   continuing it preserves the "one fixture style for the whole crate"
   property that CLAUDE.md names as load-bearing.
3. Phase 1 ships a single binary; the control plane and CLI cohabit
   one process during normal operation. There is no out-of-process
   integration boundary the WS would prove "wires up correctly" —
   the wiring is the function call.
4. The driven adapters that DO carry real I/O — TOML parser, `redb`
   IntentStore round-trip, `redb` ObservationStore round-trip,
   `ExecDriver` workload exec for the K1 honesty test — get real
   adapters via `tempfile::TempDir` + real subprocess (the bash
   coinflip script). DST `Sim*` traits cover the wall-clock,
   transport, and entropy non-determinism boundary. This satisfies
   Mandate 1 (driving-port entry) and the Dim-9 adapter-coverage
   gate.
5. The K1 honesty test (US-02 acceptance evidence — coinflip 100×)
   needs real `ExecDriver` + real cgroup write to exercise the
   ExitObserver pipeline. That is a Tier 3 integration test gated
   behind `integration-tests` and routed through Lima per
   `.claude/rules/testing.md` § "Running tests — Lima VM".

**What this strategy is NOT**:

- NOT Strategy B (subprocess invocation) — explicitly rejected by
  `crates/overdrive-cli/CLAUDE.md`.
- NOT Strategy C (in-memory only) — the K1 honesty test, the
  alloc-status round-trip tests, and the schedule-deferral
  byte-equality test all need real `redb` + real ExitObserver to
  catch wiring bugs. Strategy C would produce green tests on a
  broken pipeline.
- NOT Strategy D (real-cluster integration) — Phase 1 is single-node
  single-process; multi-region/multi-node integration is years away.

User has standing approval for the auto-detected strategy per the
dispatch contract; recording it here as DWD-02 with rationale meets
Dim-9a (WS strategy declared in wave-decisions.md).

---

## DWD-03 — Test-tier mapping per scenario

Mapping each Gherkin scenario in `test-scenarios.md` to its Rust test
tier under `.claude/rules/testing.md`:

| Scenario family | Tier | `integration-tests` feature? | Lima required? | Driving-port shape |
|---|---|---|---|---|
| Parser pure-fn (US-01, US-05, US-08 parser scenarios) | Default lane (unit-shaped) | No | No (`--no-run` macOS gate) | Direct call to `WorkloadSpecInput::deserialize` (parser entry point) |
| CLI submit echo render — happy + error paths | Default lane (in-process direct handler) | No | No | `commands::job::submit(SubmitArgs, &SimClock, &SimTransport)` |
| CLI alloc status render — happy + error paths | Default lane (in-process direct handler) | No | No | `commands::alloc_status::status(StatusArgs, ...)` |
| Streaming submit terminal verdict (Job — Sim driver) | Default lane (in-process direct handler with `SimDriver`) | No | No | `commands::job::submit(...)` against `SimDriver` injected job exit |
| K1 honesty test — coinflip 100× with real ExecDriver | Tier 3 integration | Yes (`integration-tests`) | Yes (Lima — real cgroup writes) | `commands::job::submit(...)` against real `ExecDriver` |
| K6 listener round-trip byte-equality (100 specs) | Default lane (in-process; pure render) | No | No | `commands::job::submit(...)` then `commands::alloc_status::status(...)` |
| `xtask::dst_lint` `"live"` grep gate (US-06) | xtask `#[test]` | No | No | `xtask::dst_lint::scan` |
| `examples/coinflip.toml` migration parses | Default lane (parser test on the file content) | No | No | `WorkloadSpecInput::deserialize(file_bytes)` |
| Anti-scenario: no `is running with` for Job submit | Default lane (string-not-contains assertion on render output) | No | No | Same as streaming submit family |
| OpenAPI gate (US-08 schema roundtrip) | Default lane (`overdrive-control-plane::openapi::generate` direct call) | No | No | `cargo openapi-gen` / `cargo openapi-check` (per `.cargo/config.toml` aliases) |

DWD-03 ratifies the default-lane / Tier-3 split: only K1 (coinflip
real-exec honesty over 100 trials) crosses the `integration-tests`
gate. Every parser, render, and streaming-against-Sim scenario stays
in the default lane and runs on every PR.

This matches the existing CLI test corpus convention — the Sim-driver
tests live in `tests/integration/<scenario>.rs` without the
`integration-tests` feature gate, and the cgroup-touching
`tests/integration/job_lifecycle/*.rs` family carries the gate. DISTILL
inherits that split rather than redefining it.

---

## DWD-04 — Driving-port verification (Mandate 1)

Per `nw-test-design-mandates` Mandate 1 and the dispatch's explicit
"every driving port must have ≥1 walking-skeleton scenario invoking
its actual entry-point shape" instruction, the driving ports for this
feature are:

| # | Driving port | Entry-point shape | Walking-skeleton coverage |
|---|---|---|---|
| 1 | TOML/spec parser | `WorkloadSpecInput::deserialize(toml_bytes)` (custom `Deserialize` per ADR-0047 §2 / [D3]) | WS-01 (Service happy path), WS-02 (Job happy path), WS-03 (Schedule happy path) all enter through this |
| 2 | `overdrive job submit` CLI | `commands::job::submit(SubmitArgs { spec, config_path }, &Clock, &Transport)` (per `crates/overdrive-cli/CLAUDE.md`) | WS-01, WS-02, WS-03 invoke this directly; subprocess is forbidden |
| 3 | IntentStore write boundary | `IntentStore::put_if_absent(IntentKey, WorkloadSpec)` traversed by submit_handler | WS-02 specifically asserts post-submit IntentStore contains the spec retrievable by `IntentKey::for_job` |
| 4 | JobLifecycle reconciler tick (Job-kind terminal emission) | Reconciler `tick(...)` against an injected `actual` containing a terminal `alloc_status` row | WS-02 traces through this via the streaming subscriber's `Succeeded` / `Failed` emit |
| 5 | `overdrive alloc status` CLI | `commands::alloc_status::status(StatusArgs { job, config_path }, ...)` | WS-04 (cross-kind alloc status round-trip) enters through this directly |
| 6 | Streaming subscriber (per-kind dispatcher) | `streaming::dispatcher::dispatch(WorkloadKind, alloc_status_subscription)` | WS-02 covers the Job sub-path; WS-01 covers the Service sub-path |

Every driving port has at least one walking-skeleton scenario that
invokes it through its actual entry-point shape (Mandate 1 verified).
Pipeline-level scenarios (e.g. "submit → status round-trip") DO exist
but are NOT credited as driving-port coverage on their own — each
named driving port is named explicitly in at least one walking
skeleton's Given/When step.

---

## DWD-05 — Adapter coverage (Mandate / Dim-9c)

Driven adapters in this feature and the test that proves real-I/O
wiring for each:

| Driven adapter | Real-I/O test | Tier / gating |
|---|---|---|
| TOML deserialiser (`toml::de` + custom `Deserialize`) | `parser_accepts_service_spec`, `parser_accepts_job_spec`, `parser_accepts_schedule_spec`, `parser_rejects_mixed_kinds`, `parser_rejects_zero_listeners` (real TOML strings, no in-memory mock) | Default lane |
| `redb` IntentStore | `submit_persists_intent_round_trip` (real `tempfile::TempDir` + real `redb` open) | Default lane |
| `redb` ObservationStore | `alloc_status_reads_persisted_row` (real `tempfile::TempDir` + real `redb` write from worker, real read from CLI render) | Default lane |
| `ExecDriver` (real subprocess + cgroup) | `coinflip_honesty_100_trials` (K1 — real bash subprocess, real cgroup write) | Tier 3, `integration-tests`, Lima |
| OpenAPI schema generator (`utoipa::ToSchema` + `cargo openapi-gen`) | `openapi_schema_includes_listener` and `cargo openapi-check` | Default lane (xtask alias-driven) |
| `xtask::dst_lint` scanner (`"live"` rule) | `dst_lint_rejects_live_literal` | xtask `#[test]` |
| Streaming NDJSON wire format | Existing `streaming_submit_happy_path` + new `streaming_submit_job_terminal_verdict` (real NDJSON encode/decode) | Default lane |

Every driven adapter has at least one real-I/O test (Dim-9c green).
No `@in-memory` fakes appear on any walking-skeleton scenario
(Dim-9d / 9e green — Strategy A specifies real local adapters).

---

## DWD-06 — Reconciliation against DISCUSS / DESIGN

Per the dispatch's mandatory reconciliation gate:

**Read sources**:

- `docs/feature/workload-kind-discriminator/discuss/wave-decisions.md`
  (DISCUSS — 211 lines, 2026-05-10)
- `docs/feature/workload-kind-discriminator/design/wave-decisions.md`
  (DESIGN — 400 lines, 2026-05-10)
- DESIGN-wave delta correction commits `dfc2e79` (hard-gate review),
  `266a879` (#163 framing correction), `c514e5e` (delta review
  approving the framing correction).

**Reconciliation findings**:

| # | Surface | DISCUSS view | DESIGN view | Reconciliation |
|---|---|---|---|---|
| R-01 | GH #163 framing | Not mentioned (DISCUSS only references #166 + #167) | DESIGN initially framed #163 as "Listener dataplane wiring"; commit `266a879` corrected to "REVERSE_NAT_MAP UDP lockstep bug". | DISTILL adopts the corrected framing — #163 is OUT OF SCOPE for this feature; the spec layer ships the field shape only, no `Dataplane::update_service` change. No DISTILL scenario references #163 by issue number. |
| R-02 | Slice 06 split decision | "If the architect later determines the alloc status render extension is non-trivial, splitting at the AllocStatusRow boundary is the natural fault line" — left to DESIGN. | DESIGN [D8] keeps Slice 06 whole — alloc-status extension is mechanical. | DISTILL groups Slice 06 scenarios into one section in `test-scenarios.md`; does not split. |
| R-03 | K3 measurement cadence | "Manual check on first release; automated assertion that the rendered Exit column matches the persisted exit_code" | DESIGN [D9] pins as "pre-release manual gate (one-shot at first release)" + automated parsing-from-fixtures regression. | DISTILL specifies the automated parsing-from-fixtures scenario as continuous (default lane) and notes the manual gate as out of scope for executable acceptance tests (PO domain per `nw-ad-critique-dimensions` § "Reviewer Scope Boundaries"). |
| R-04 | `${listener_triple}` consumer #3 | "AllocStatusRow listener fields denormalised at write time (architect to confirm shape)" | DESIGN [D5] / ADR-0047 §4a pins as `listeners: Vec<ListenerRow>` embedded on the row. | DISTILL test-scenarios specify byte-equality (K6) between submit echo and `alloc status` listener lines, asserting against the `Vec<ListenerRow>` shape implicitly through observable render output (not internal field structure — Dim 7 compliant). |
| R-05 | RCA root cause A (Service settle window) | "remains a separate concern… out of scope here" | DESIGN: tracked at GH #170 (k8s-shaped probes); explicitly out of scope. | DISTILL acknowledges in `acceptance-review.md`; no scenario asserts on settle-window behaviour. |
| R-06 | `JobSubmitEvent` shape | DISCUSS US-02 AC: "JobSubmitEvent does not include a ConvergedRunning variant" | DESIGN [D2]: structural fix — variant absent from the enum. | Aligned. The DISTILL anti-scenario "no `is running with` substring on any Job submit output line" is the observable proof; the structural absence is verified by the crafter at compile time (exhaustive match coverage). |

**Verdict**: NO unresolved contradictions. The DESIGN-wave framing
corrections (commits `266a879` + `c514e5e`) are reflected in the
DISTILL artifacts — `test-scenarios.md` references #166 and #167 by
issue number for tracked deferrals (per existing operator-facing
copy in user-stories.md / slice files), but does NOT reference #163
in any scenario.

---

## DWD-07 — Property-shaped scenarios (`@property` tag)

Per the skill workflow Phase 2 step 8 ("tag property-shaped criteria"),
the following criteria are universal invariants and are tagged
`@property` in `test-scenarios.md` for the crafter to implement as
proptest cases (per `.claude/rules/testing.md` § "Property-based
testing (proptest)"):

- **PROP-01**: every valid `JobSpecInput` round-trips bit-equivalent
  through TOML / JSON / `Job::from_spec` / `JobSpecInput::from(&Job)`
  (US-08, AC #10). Mandatory call site per development.md §
  "Property-based testing — Mandatory call sites" (newtype roundtrip).
- **PROP-02**: parser rejects ALL mixed-kind specs in 50ms p95 (K2
  bound; the input space is generators of "two-of-three section
  presence" cases — fits proptest's input-space-shrinking shape).
- **PROP-03**: for ANY valid Service spec with N listeners (N in
  `1..=32`, listeners with valid distinct triples), the submit echo
  Listeners section byte-equals the `alloc status` Listeners section
  for the same persisted row. K6's "100 trials" claim subsumes this
  as a proptest with `PROPTEST_CASES=100` minimum.

Other anti-scenarios ("no `is running with` for any Job") are
universal but are simpler to implement as a single `#[test]` with a
hand-picked exhaustive set of Job exit shapes (Succeeded /
AttemptFailed / BackoffExhausted) than as a proptest. Crafter's
choice; the DISTILL tag is a recommendation, not a binding decision.

---

## DWD-08 — KPI-tagged scenarios (`@kpi` tag)

Per the skill workflow Phase 2 step 6, KPI observability scenarios:

| KPI | Scenario tag | Purpose |
|---|---|---|
| K1 | `@kpi @K1` | Honesty rate ≥99% over 100 trials of coinflip — observable as CLI exit code matching workload exit code |
| K2 | `@kpi @K2` | Parser rejection of mixed-kind specs within 50ms p95 — observable as `Result::Err` returned within timing budget |
| K3 | `@kpi @K3` (automated portion) | Rendered Exit column matches persisted `exit_code` — observable parse-from-fixture |
| K4 | `@kpi @K4` | Existing Service-shaped tests pass post-rename — observable as test-pass on migrated fixtures |
| K5 | `@kpi @K5` | Schedule deferral URL byte-equality across submit echo + alloc status — observable string equality |
| K6 | `@kpi @K6` | Listener triple round-trip byte-equality — observable string equality |

Note on KPI scope per `nw-ad-critique-dimensions` § "Reviewer Scope
Boundaries": KPI *measurability* is PO-reviewer scope at DELIVER
post-merge. DISTILL's job is to ensure each KPI has at least one
scenario that *makes the metric emittable*; PO validates the metric's
business validity downstream.

---

## DWD-09 — Carpaccio slice mapping

Mapping DISCUSS slices → DISTILL scenario sections in
`test-scenarios.md`:

| Slice | Stories | Scenario sections | Implementation order (per crafter) |
|---|---|---|---|
| Slice 01 | US-01, US-06, US-07 | §1 (parser kind discriminator), §6 (`"live"` grep gate), §7 (coinflip migration) | First — `WorkloadKind` enum is the abstraction every later slice depends on |
| Slice 02 | US-02 | §2 (Job submit terminal verdict + anti-scenarios) | Second — closes the bug |
| Slice 03 | US-03 | §3 (alloc status kind-aware Job render) | Third — depends on Slice 02's terminal-condition emit |
| Slice 04 | US-04 | §4 (Service preservation; rename "Job" → "Service" in render) | Fourth — regression guard, paired with Slice 01's grep gate |
| Slice 05 | US-05 | §5 (Schedule parsing + honest deferral) | Fifth — small, can land before or after Slice 04 |
| Slice 06 | US-08 | §8 (Service listener spec shape) | Sixth — depends on Slice 01 + Slice 04 |

The order is the natural dependency order; DISTILL does not constrain
the crafter beyond "the slices' acceptance evidence must land in this
order or the gates fail."

---

## DWD-10 — Walking skeleton count: 4

Per `nw-test-design-mandates` § "Walking Skeleton Strategy" (2-5 per
feature), this feature ships **4 walking skeletons** + **~26 focused
scenarios** = 30 scenarios total. Ratio: 13% WS / 87% focused, within
the 10-15% / 85-90% guideline.

Walking skeletons (each demo-able to a stakeholder):

- **WS-01**: Ana submits `payments.toml` (Service); CLI streams
  `Service 'payments' is running with 1/1 replicas (took 1.4s)`; CLI
  exits 0; `alloc status` shows the kind-aware Service render with
  Listeners section.
- **WS-02**: Ana submits `examples/coinflip.toml` (Job, exit-1
  branch); CLI streams `Job 'coinflip' failed.\n  exit code: 1\n  ...`;
  CLI exits non-zero; `alloc status` shows kind-aware Job render with
  per-attempt exit codes and stderr tail.
- **WS-03**: Ana submits `nightly-backup.toml` (Schedule); CLI prints
  "Schedule registered" + deferral note + #166 URL; CLI exits 0;
  `alloc status` shows kind-aware Schedule render with cron + same
  deferral URL.
- **WS-04**: Ana runs `alloc status` for each of the three live
  workloads above; render branches correctly per kind; no cross-kind
  vocabulary leak.

WS-01 / WS-02 / WS-03 each cover one driving-port path end-to-end
(Mandate 3 — User Journey Completeness). WS-04 covers the post-hoc
inspection path (J-OPS-003's framing journey). Every WS is described
in user-goal terms (Mandate 5 / Dim-5 litmus test) — see
`walking-skeleton.md` for the per-WS narrative.

---

## DWD-11 — Error path ratio

Per `nw-bdd-methodology` § "Scenario Categorization" (target 40%+
error/edge cases) and `nw-ad-critique-dimensions` Dim-1 (Happy Path
Bias):

Total scenarios in `test-scenarios.md`: 30
- Happy path: 12 (40%)
- Error path: 14 (47%)
- Edge / boundary: 4 (13%)

Error-path ratio = 47% — **passes** the ≥40% gate.

---

## DWD-12 — Out-of-scope items (no deferrals introduced by DISTILL)

DISTILL introduced no new deferrals. The two tracked deferrals
inherited from earlier waves are referenced by issue number in
operator-facing copy (per slice-05 / slice-06 / journey YAML) but
DISTILL does not own them:

- GH #166 (Schedule execution semantics) — DISCUSS-approved, owned
  by a separate follow-up feature.
- GH #167 (VIP allocator behaviour for `vip = None`) — DISCUSS-approved,
  owned by a separate follow-up feature.
- GH #170 (Service health-check primitive — startup/readiness/liveness
  probes) — DESIGN-recorded, supersedes earlier #169 framing per
  user direction 2026-05-10.
- GH #163 (REVERSE_NAT_MAP UDP lockstep bug) — DESIGN-corrected
  framing (commit `266a879`); explicitly out of scope for this
  feature per the corrected framing.

DISTILL scenarios reference these issues by URL for operator-facing
deferral copy (US-05 / US-08); DISTILL does NOT add scope dependent
on any of them. Per `CLAUDE.md` § "Deferrals require GitHub issues —
AND user approval BEFORE creation", DISTILL did not create any new
issues; all four were created in earlier waves with user approval.

---

## DWD-13 — Quality gate self-check

Pre-handoff checklist (per skill workflow Phase 4 + project
`acceptance-review.md`):

- [x] DWD-01: Output convention pinned (no `.feature` files).
- [x] DWD-02: WS strategy declared (Strategy A, in-process direct-handler).
- [x] DWD-03: Test-tier mapping per scenario.
- [x] DWD-04: All 6 driving ports have ≥1 walking-skeleton coverage.
- [x] DWD-05: All 7 driven adapters have ≥1 real-I/O test.
- [x] DWD-06: Reconciliation against DISCUSS + DESIGN — no contradictions.
- [x] DWD-07: Property-shaped criteria tagged (3 PROPs).
- [x] DWD-08: KPI-observability scenarios tagged (K1–K6).
- [x] DWD-09: Carpaccio slice mapping recorded.
- [x] DWD-10: Walking skeleton count = 4 (within 2-5 mandate).
- [x] DWD-11: Error-path ratio = 47% (≥40% gate).
- [x] DWD-12: No new deferrals; existing four tracked by issue number.
- [x] All Gherkin uses business language (zero technical jargon — see
      `acceptance-review.md` Dim-3 grep evidence).
- [x] Every story US-01..US-08 has ≥1 scenario referencing it (Dim-8
      Check A — see `acceptance-review.md` traceability table).
- [x] No DISTILL artifact instructs the crafter to create RED scaffolds
      in `src/` (Rust crafter handles that per testing.md convention).

---

## Changelog

- 2026-05-10 — Initial DISTILL wave decisions captured. 4 walking
  skeletons, ~26 focused scenarios, 47% error-path ratio. WS strategy
  = A (in-process direct-handler). Test-tier mapping pins one Tier-3
  integration test (K1 coinflip honesty) behind `integration-tests`
  feature; all other scenarios in default lane.
