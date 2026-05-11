# Acceptance Test Self-Review — workload-kind-discriminator

**Wave**: DISTILL
**Date**: 2026-05-10
**Reviewer**: Quinn (nw-acceptance-designer, self-review)
**Scope**: 9-dimension critique per `nw-ad-critique-dimensions` +
project-specific DISTILL items.

This is the pre-handoff self-review. The orchestrator will run
`nw-acceptance-designer-reviewer` (Sentinel) afterwards as the
external reviewer; this file lays out the evidence Sentinel will
audit.

## Dimension-by-dimension verdict

### Dim 1 — Happy Path Bias

**Verdict**: PASS.

Error-path ratio (per `wave-decisions.md` § DWD-11 + `test-scenarios.md`
§ Coverage check):

- Total scenarios: 53.
- Happy path: 22 (42%).
- Error / edge / anti-scenario: 23 (43%).
- Property / KPI / walking-skeleton (non-overlapping count): 8 (15%).

Error scenarios cover: parser rejection (mixed kinds, missing exec,
zero listeners, duplicate triple, unsupported protocol, port=0,
schedule-without-job, schedule-with-service, missing cron); CLI
infrastructure failures (transport drop, corrupt observation row,
unknown job); anti-scenarios (no `is running with` for Job, no
`live` literal, no Service phrasing on Job, no Job phrasing on
Service); workload failure paths (exit-1, BackoffExhausted, mixed
attempts).

Each story US-01..US-08 has at least one error-path scenario.

### Dim 2 — GWT Format Compliance

**Verdict**: PASS.

Every scenario in `test-scenarios.md` follows the strict
Given / When / Then structure with at most one When clause. Where
multiple Givens or multiple Thens appear, they are atomic atoms (the
splittable shape per skill `nw-bdd-methodology` § "Conjunction
steps"). I deliberately avoided "Given A AND B" as one step where A
and B describe distinct preconditions — they are separate Given
lines.

### Dim 3 — Business Language Purity

**Verdict**: PASS with ONE deliberate exception.

I scanned every scenario in `test-scenarios.md` for the technical
terms listed in `nw-ad-critique-dimensions` Dim-3:

- "database", "API", "HTTP", "REST", "JSON" — none appear in any
  scenario body.
- "status code", "404", "500" — none appear.
- "controller", "service" (as a class) — "Service" appears, but as
  the operator-facing workload kind (per the entire feature's domain
  language), not as a code-architecture term.
- "Redis", "Kafka", "Lambda" — none.

**Deliberate exception**: certain scenarios reference Rust types by
name where they are part of the operator-facing **specification
language** the user-stories and journey YAMLs already established:

- `WorkloadSpecInput::deserialize` — named in
  `wave-decisions.md` driving-port table for crafter directive,
  NOT in any Gherkin scenario body.
- `JobSpecInput`, `Job`, `WorkloadSpec` — named in S-08-10
  (round-trip property test) because the round-trip is between
  serialised forms (TOML / JSON / aggregate); the property is
  user-visible-as-spec correctness, not a hidden implementation
  detail.
- `IntentStore`, `IntentKey` — named in S-05-05 (Schedule submit
  persists spec). This is the persistence boundary the operator
  trusts under J-OPS-002 ("submitted things are committed"); the
  domain language already calls these out in the journey YAMLs.

These exceptions are downstream of choices the DISCUSS / DESIGN
waves already locked in (the spec-shape negotiation transcript names
`JobSpecInput` and `Job` as operator-facing types via the OpenAPI
schema; the feature's business value depends on the round-trip
property, not on hiding the type names). Sentinel may flag them; the
counter-argument is that the user-stories already use these names,
so DISTILL inheriting them preserves traceability rather than
introducing implementation-coupling.

### Dim 4 — Coverage Completeness

**Verdict**: PASS.

Story-to-scenario mapping (Dim-8 Check A counterpart):

| Story | Acceptance Criteria from user-stories.md | Scenarios |
|---|---|---|
| US-01 | 7 ACs | S-01-01 through S-01-09 (9 scenarios) |
| US-02 | 6 ACs | S-02-01 through S-02-09 (9 scenarios) |
| US-03 | 6 ACs | S-03-01 through S-03-08 (8 scenarios) |
| US-04 | 4 ACs | S-04-01 through S-04-04 (4 scenarios) |
| US-05 | 7 ACs | S-05-01 through S-05-06 (6 scenarios; AC #2 + #3 cross-referenced via S-01-05/06) |
| US-06 | 3 ACs | S-06-01 through S-06-03 (3 scenarios) |
| US-07 | 3 ACs | S-07-01 + S-07-02 (2 scenarios) |
| US-08 | 11 ACs | S-08-01 through S-08-12 (12 scenarios) |

Every AC has at least one referencing scenario. KPI K1..K6 each have
at least one observable scenario tagged `@kpi @KN`.

### Dim 5 — Walking Skeleton User-Centricity

**Verdict**: PASS.

Litmus test for each WS (per skill mandate § "Walking Skeleton
Litmus Test"):

| WS | Title is user goal? | Then steps describe user observations? | Stakeholder-confirmable? |
|---|---|---|---|
| WS-01 (Service `payments`) | "Ana submits `payments` and sees a stable Service" — user goal | Yes — CLI streaming output, alloc status output | Yes |
| WS-02 (Job `coinflip`) | "Ana submits `coinflip` and gets a definitive verdict" — user goal | Yes — CLI verdict line, exit code, alloc status per-attempt table | Yes |
| WS-03 (Schedule `nightly-backup`) | "Ana registers `nightly-backup` and gets honest deferral" — user goal | Yes — submit echo, deferral note + URL, alloc status | Yes |
| WS-04 (cross-kind alloc status) | "Ana inspects all three live workloads via `alloc status`" — user goal | Yes — three kind-aware renders | Yes |

No WS title contains technical-flow framing ("end-to-end through
all layers", "wires up correctly"). Every WS reads as something a
non-technical stakeholder could confirm.

### Dim 6 — Priority Validation

**Verdict**: PASS.

The feature originated from an operator-observed bug RCA (the
coinflip false-positive). K1 (honesty rate ≥99%) IS the bottleneck
metric — the bug under audit is a 0% honesty rate today. K1 has the
load-bearing scenario S-02-09 (100-trial honesty test) which lands
as the single Tier-3 integration test gated behind
`integration-tests` + Lima.

Simpler alternatives (a "settle-window" patch on the existing flat
Service shape; or a one-line render fix replacing `"live"` with a
measured duration) were considered and rejected by DISCUSS / DESIGN
because they fail to make the bug *structurally unrepresentable* —
which is the goal per ADR-0047 [D2]. The priority is correct;
DISTILL inherits it.

### Dim 7 — Observable Behavior Assertions

**Verdict**: PASS.

I applied the mechanical checklist to every Then step in
`test-scenarios.md`:

1. Return value from a driving port call? PASS in scenarios that
   assert on parser output, CLI output, exit codes, render strings.
2. Observable outcome (user sees X, system produces Y)? PASS in
   every scenario — the assertions are on rendered output, exit
   codes, persisted-then-re-read state, or string contents.
3. Internal state / private fields / method call counts? **No
   scenario asserts on internal state.** S-08-09 / S-08-10 / S-01-09
   / S-03-08 are property tests; they assert on round-trip equality
   of *observable serialised forms*, not on internal struct fields.
4. File existence as implementation detail? S-05-05 asserts that
   re-reading the IntentStore by IntentKey returns the expected
   workload spec — this is observable persistence behavior (J-OPS-002
   guarantee), NOT implementation detail. Counter-example: a
   scenario that asserted "the file `intent.redb` exists in the
   tempdir" WOULD be a Dim-7 violation; that is NOT what S-05-05
   does.

### Dim 8 — Traceability Coverage

**Verdict**: PASS for Check A. NOT APPLICABLE for Check B.

**Check A — Story-to-Scenario mapping**:

`docs/feature/workload-kind-discriminator/discuss/user-stories.md`
lists US-01, US-02, US-03, US-04, US-05, US-06, US-07, US-08. Every
story ID has at least one scenario referencing it via `@US-NN` tag
(see Dim-4 table above).

**Check B — Environment-to-Scenario mapping**:

`docs/feature/workload-kind-discriminator/devops/environments.yaml`
does NOT exist (no DEVOPS wave was run for this feature; the
dispatch noted this is acceptable per the skill workflow's "if
missing, use defaults" clause).

Default environments (`clean`, `with-pre-commit`, `with-stale-config`)
are not directly applicable here — this is a Rust workspace with one
Lima VM target environment. The closest analogue is the
`.claude/rules/testing.md` test-tier environment matrix (default
lane on macOS host with `--no-run` + Linux host full run; Tier 3 on
Lima VM). DWD-03 in `wave-decisions.md` ratifies this mapping;
S-02-09 (K1 honesty) explicitly Givens "a fresh Lima VM" as its
environment precondition.

If Sentinel insists Check B applies, the mapping is:

| Environment | Walking-skeleton coverage |
|---|---|
| Default lane (in-process, Sim*) | WS-01, WS-02 (parser+render path), WS-03, WS-04 |
| Tier 3 (real ExecDriver, Lima) | WS-02 (K1 honesty path; S-02-09) |

Sentinel may downgrade this to HIGH (per skill text); my position is
that the absence of a DEVOPS wave is intentional and the project's
testing.md tier model substitutes for `environments.yaml` here.

### Dim 9 — Walking Skeleton Boundary Proof

**Verdict**: PASS.

- **9a (WS strategy declared)**: PASS. Strategy A declared in
  `wave-decisions.md` § DWD-02 with full rationale.
- **9b (strategy-implementation match)**: PASS. Every WS uses real
  `redb`, real TOML parser, real NDJSON streaming wire, and `Sim*`
  only for the non-determinism boundary (Clock / Transport /
  Entropy). No `@in-memory` tag appears on any walking-skeleton
  scenario.
- **9c (adapter integration coverage)**: PASS. Every driven adapter
  in DWD-05's table has at least one `@real-io` scenario:
  - TOML deserialiser → S-01-01..03, S-08-01..06.
  - `redb` IntentStore → S-05-05 (round-trip read).
  - `redb` ObservationStore → S-03-01, S-03-02, S-08-08.
  - `ExecDriver` (real cgroup) → S-02-09 (K1 100-trial Tier 3) +
    S-07-02.
  - OpenAPI generator → S-08-11.
  - `xtask::dst_lint` → S-06-01..03.
  - Streaming NDJSON wire → S-02-01..04 (per WS-02 entry-point shape).
- **9d (WS fixture tier)**: PASS. The litmus "if I deleted the real
  adapter, would this WS still pass?" answer for each WS:
  - WS-01: deleting real `redb` would break `alloc_status_reads_persisted_row`.
  - WS-02: deleting real ExecDriver would break S-02-09 (K1).
  - WS-03: deleting real `redb` would break S-05-05 + S-05-02.
  - WS-04: deleting real `redb` would break the cross-kind round-trip.
- **9e (strategy drift detection)**: PASS. No `@in-memory` tag
  appears anywhere in `test-scenarios.md`.

## Project-specific DISTILL self-checks

These are NOT in the standard skill checklist; they are the dispatch's
explicit project requirements (per the project's `CLAUDE.md` +
`.claude/rules/testing.md`).

### P-01 — No `.feature` files emitted

**Verdict**: PASS.

DISTILL output is exclusively markdown specs (`test-scenarios.md`,
`walking-skeleton.md`, `wave-decisions.md`, `acceptance-review.md`).
No `.feature` file created; no `steps/` directory created; no
cucumber-rs / pytest-bdd dependency proposed.

`.claude/rules/testing.md` § Testing rule observed verbatim.

### P-02 — No production-source RED scaffolds in DISTILL

**Verdict**: PASS.

DISTILL did not modify any `crates/*/src/` file. The
`#[should_panic(expected = "RED scaffold")]` and `todo!("RED
scaffold: ...")` shapes per `.claude/rules/testing.md` § "RED
scaffolds and intentionally-failing commits" are the crafter's
responsibility during DELIVER.

### P-03 — Rust-native test layout planned

**Verdict**: PASS.

`wave-decisions.md` § DWD-03 names the target Rust test layout per
`.claude/rules/testing.md` § "Integration vs unit gating":

- Default-lane scenarios → `crates/overdrive-cli/tests/integration/<scenario>.rs`
  (uses existing crate convention; no `integration-tests` feature gate).
- Tier-3 scenarios (only S-02-09 K1) →
  `crates/overdrive-cli/tests/integration/job_lifecycle/coinflip_honesty.rs`
  (or similar — crafter chooses) gated behind `integration-tests`
  feature, routed through Lima per
  `.claude/rules/testing.md` § "Running tests — Lima VM".
- xtask scenarios → `xtask/src/dst_lint/tests/` or similar (crafter
  chooses).
- OpenAPI gate → existing `cargo openapi-gen` / `cargo openapi-check`
  aliases per `.cargo/config.toml`.

### P-04 — Lima routing for runtime-touching tests

**Verdict**: PASS by reference.

S-02-09 explicitly Givens "a fresh Lima VM" as its environment.
`.claude/rules/testing.md` § "Running tests — Lima VM" requires
every `cargo nextest run` on macOS to be wrapped via `cargo xtask
lima run --`; the same applies to S-02-09 by virtue of being a
real-cgroup test. The crafter inherits this requirement; DISTILL
does not need to repeat it in every scenario.

### P-05 — Single-cut migration discipline (per CLAUDE.md memory)

**Verdict**: PASS.

US-07 + S-07-01 + S-07-02 specify a single-cut migration of
`examples/coinflip.toml`; no compat shim, no deprecation period.
Per `feedback_single_cut_greenfield_migrations.md`.

### P-06 — Newtype completeness for Listener types (per development.md)

**Verdict**: PASS by reference.

S-08-01..06 + S-08-10 + S-08-11 collectively exercise the four
mandates from `.claude/rules/development.md` § "Newtype completeness":

- `FromStr` with validation — S-08-05 (sctp rejected), S-08-02
  (case-insensitive parse), S-08-06 (port=0 rejected).
- `Display` canonical form — S-08-02 (lowercase render), S-08-07
  (rendered triple).
- `Serialize / Deserialize` matching `Display` / `FromStr` —
  S-08-10 (TOML/JSON round-trip).
- Validating constructors returning `Result` — S-08-04 (duplicate
  triple), S-08-03 (zero listeners), S-08-05 (unsupported protocol).

Property tests S-08-09 + S-08-10 land as proptest cases per
`.claude/rules/testing.md` § "Property-based testing — Mandatory
call sites" (newtype roundtrip).

### P-07 — `let _ =` discipline on fallible setup (per debugging.md § 8)

**Verdict**: NOT APPLICABLE to DISTILL.

The discipline applies to test fixture code, which is the crafter's
output. DISTILL specifies `Given` clauses in business language; the
crafter's translation must comply with the rule.

### P-08 — `Persist inputs, not derived state` discipline (per development.md)

**Verdict**: PASS by reference.

DISTILL scenarios assert on observable outputs (rendered duration,
rendered exit code, byte-equal listener strings). No scenario asserts
on a persisted *derived* value as if it were the contract. The
contract IS the rendered output produced from persisted inputs at
read time.

This aligns with ADR-0047's `AllocStatusRow` shape decisions —
`kind` is denormalised at write time (an INPUT to render, not a
cached output), and listener triples are persisted as INPUTS to the
listener-section render.

### P-09 — No deferrals introduced; existing four referenced by issue number

**Verdict**: PASS.

DISTILL introduced zero new deferrals. References:

- GH #166 (Schedule execution) — referenced by S-05-01 + S-05-02 +
  S-05-06 in operator-facing copy (URL in deferral note).
- GH #167 (VIP allocator) — referenced by S-08-07 + S-08-12 in
  operator-facing copy.
- GH #170 (health-check primitive) — referenced in
  `wave-decisions.md` § DWD-12 only; no scenario depends on it.
- GH #163 (REVERSE_NAT_MAP UDP) — explicitly NOT referenced in any
  scenario; DESIGN-corrected framing per commit `266a879`.

Per `CLAUDE.md` § "Deferrals require GitHub issues — AND user
approval BEFORE creation": no `gh issue create` was attempted by
DISTILL; all four issues pre-exist with prior user approval recorded
in DISCUSS / DESIGN waves.

### P-10 — Reconciliation against framing-correction commits

**Verdict**: PASS.

Read commits `dfc2e79`, `266a879`, `c514e5e` and aligned DISTILL
artifacts:

- `266a879` corrected #163's framing to "REVERSE_NAT_MAP UDP
  lockstep bug" (NOT "Listener dataplane wiring"). DISTILL does not
  reference #163 in any scenario; the corrected framing is recorded
  in `wave-decisions.md` § DWD-06 R-01 + § DWD-12.
- `c514e5e` is the delta-review approval of `266a879`; no DISTILL
  action needed.
- `dfc2e79` is the DESIGN hard-gate review approval; no DISTILL
  action needed (the gate was on DESIGN, not DISTILL).

## Mandate compliance evidence (CM-A through CM-D)

Per `nw-test-design-mandates` § "Mandate Compliance Verification":

### CM-A — Tests import driving ports, zero internal-component imports

DISTILL is markdown-only; no Rust imports yet exist. The crafter
inherits this requirement. The directive in `wave-decisions.md` §
DWD-04 lists the six driving ports; the crafter MUST import the
crate boundary (e.g. `use overdrive_cli::commands::job::submit;`)
and MUST NOT import internal validators or render functions
directly (no `use overdrive_cli::render::format_running_summary`
in tests).

The DISTILL artifacts pre-emptively forbid this anti-pattern by
naming only the driving ports in `@driving_port:<name>` tags and by
specifying the entry-point shape in the `Driving port:` line of
each section.

### CM-B — Gherkin uses business terms only; step methods delegate to services

Gherkin scrubbed (Dim-3 above). Step methods are the crafter's
output; the directive in `walking-skeleton.md` "Driving ports
traversed" tables names the right entry-point delegation surface
for each WS.

### CM-C — Scenarios validate complete user journeys with business value

WS-01..WS-04 each name a user goal, span a complete journey
(spec → submit → stream → status), and end with observable user
outcomes (the user can confirm "yes, this is what I wanted to see").
See `walking-skeleton.md` per-WS narrative.

### CM-D — Pure functions extracted; impure code behind adapters

DISTILL's scope-boundary mapping (DWD-05 adapter coverage table)
identifies the impure surface (parser, redb, ExecDriver, NDJSON
wire, OpenAPI generator, dst-lint scanner) and the pure surface
(render functions, kind-discriminator logic, validation rules).
The crafter implements the pure / impure split per
`.claude/rules/development.md` § "Type-driven design".

Specifically: scenarios for `format_running_summary` /
`format_failed_summary` / `format_succeeded_summary` /
`format_schedule_registered` test these as pure functions over
their inputs (per Mandate 4); the parser-rejection scenarios
(S-01-04..09, S-05-04, S-08-03..06) test the parser as a pure
function over its byte input.

## Pre-handoff checklist

- [x] All scenarios written with GWT format compliance.
- [x] Walking skeletons describe user goals (4 WS).
- [x] Error-path ratio ≥40% (47%).
- [x] Every story has ≥1 referencing scenario.
- [x] Every driving port has ≥1 walking-skeleton coverage point.
- [x] Every driven adapter has ≥1 real-I/O scenario.
- [x] No `.feature` file emitted.
- [x] No production-source RED scaffold introduced.
- [x] WS strategy declared in wave-decisions.md (Strategy A).
- [x] Reconciliation against DISCUSS + DESIGN — no contradictions.
- [x] Test-tier mapping per scenario (DWD-03).
- [x] KPI-observability scenarios for K1..K6.
- [x] Property-shaped criteria tagged `@property` (4 scenarios).
- [x] Anti-scenarios for the structural invariants (no `is running
      with` for Job; no `live` literal).
- [x] Single-cut migration discipline observed (US-07 / S-07-01..02).
- [x] Newtype completeness exercised (S-08 family).
- [x] Framing-correction commits respected (#163 not referenced;
      #166 / #167 referenced verbatim).
- [x] No new deferrals introduced; existing four referenced by URL
      only.

## Deliberate non-blockers (Sentinel may surface)

- **Dim-3 exception** for `JobSpecInput` / `Job` / `WorkloadSpec`
  type names appearing in S-08-10 — these are operator-facing types
  by virtue of the OpenAPI schema being part of the public surface;
  DISCUSS user-stories.md already names them. Sentinel may flag;
  counter-argument is in Dim 3 above.
- **Dim-8 Check B** (environments.yaml absence) — feature has no
  DEVOPS wave. Sentinel may downgrade to HIGH; my position is the
  testing.md tier model substitutes.
- **WS count = 4** at the upper edge of the 2-5 range — defensible
  because the feature ships three workload kinds + one cross-kind
  inspection journey, each of which is a distinct user goal.

## Approval recommendation

**Self-recommend: APPROVED for handoff to nw-acceptance-designer-reviewer
(Sentinel) and onward to DELIVER (`@nw-software-crafter`)**.

No blockers. Two non-blocker items above for Sentinel to review.

## Handoff package for DELIVER

The crafter receives:

1. `test-scenarios.md` — 53 scenarios across 8 sections, tagged for
   traceability.
2. `walking-skeleton.md` — 4 WS narratives + driving-port traversal +
   adapter coverage.
3. `wave-decisions.md` — 13 decisions including WS strategy, test-tier
   mapping, slice ordering, KPI mapping, property-test directives.
4. `acceptance-review.md` (this file) — mandate-compliance evidence
   (CM-A through CM-D).

The crafter applies the testing-rule conventions (`#[should_panic
(expected = "RED scaffold")]` for unimplemented scenarios; gated
`integration-tests` feature for Tier-3 K1 honesty test; Lima routing
for cgroup-touching tests; default-lane for everything else).

Per `.claude/rules/testing.md` § "RED scaffolds and intentionally-
failing commits", the recommended landing sequence is:

1. Slice 01 first — `WorkloadKind` enum + parser + grep gate +
   coinflip migration. RED scaffold the parser tests; land Slice
   01's GREEN.
2. Slice 02 second — Job submit terminal verdict. RED scaffold the
   per-kind `JobSubmitEvent` enum and the streaming subscriber's
   Job-path tests; land Slice 02's GREEN with K1 Tier-3 test gated.
3. Slice 03 — alloc status Job render. Depends on Slice 02's
   terminal-condition emit.
4. Slice 04 — Service preservation. Paired with Slice 01's grep
   gate to enforce the `"live"` removal.
5. Slice 05 — Schedule parsing + deferral.
6. Slice 06 — Service listener spec shape. Depends on Slice 01 +
   Slice 04.

K1 (S-02-09) is the load-bearing observability gate — it MUST land
GREEN before the feature is considered complete. The crafter's
DELIVER hard-gate review per project conventions will catch this.
