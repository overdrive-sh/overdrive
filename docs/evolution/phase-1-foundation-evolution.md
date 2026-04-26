# phase-1-foundation — Feature Evolution

**Feature ID**: phase-1-foundation
**Branch**: marcus-sa/phase-1-foundation
**Duration**: 2026-04-21 — 2026-04-22 (DISCUSS opened 2026-04-21; DELIVER closed 2026-04-22)
**Status**: Delivered
**Walking-skeleton gate**: step 06-03 (`cargo xtask dst` green + canary red-run with usable reproduction command)

> **Nomenclature note (2026-04-23).** This archived evolution doc describes
> crates with `crate_class = "adapter-real"`. That class value was renamed
> to `"adapter-host"` on 2026-04-23 (see ADR-0003 Amendment 2026-04-23 and
> ADR-0016). The prose below preserves the name as it stood at delivery.
> Current crate declarations use `adapter-host`; the taxonomy shape is
> unchanged.

## What shipped

The project's walking skeleton: the type universe (11 identifier newtypes),
the state-store split (`IntentStore` + `ObservationStore` traits), the six
nondeterminism ports (`Clock`, `Transport`, `Entropy`, `Dataplane`,
`Driver`, `Llm`) plus `ObservationStore` as the seventh, every Sim adapter
backing those ports, a turmoil-based DST harness, the `cargo xtask dst`
and `cargo xtask dst-lint` CLIs, and the CI lint gate that forbids banned
APIs in `crate_class = "core"` crates. This scaffold is the foundation
every subsequent Overdrive feature will build against: typed identifiers,
a deterministic simulation environment, a bit-identical state-store
round-trip, and a mechanical barrier against nondeterminism leaking into
core logic.

## Wave journey

- **DISCUSS** → bootstrapped `docs/product/` SSOT (`vision.md`,
  `jobs.yaml`, `journeys/trust-the-sim.yaml`), six LeanUX stories
  (US-01 – US-06), six carpaccio slices, six outcome KPIs (K1–K6), DoR
  PASS on all six stories. See
  [`discuss/wave-decisions.md`](../feature/phase-1-foundation/discuss/wave-decisions.md).
- **DESIGN** → six ADRs (0001–0006), bootstrap of
  `docs/product/architecture/`, C4 L1/L2/L3 diagrams in `brief.md`, K4
  reframed to Phase 2+ guardrail via back-propagation. Paradigm: OOP
  (Rust trait-based). See
  [`design/wave-decisions.md`](../feature/phase-1-foundation/design/wave-decisions.md).
- **DISTILL** → ~65 acceptance scenarios as Gherkin-in-markdown (no
  `.feature` files per project rule), Walking Skeleton Strategy C (real
  local redb against `tempfile::TempDir`), 10-row adapter coverage
  table, KPI-to-scenario tagging (K1/K2/K3/K5/K6 covered; K4
  deliberately absent). See
  [`distill/wave-decisions.md`](../feature/phase-1-foundation/distill/wave-decisions.md).
- **DEVOPS** → `.github/workflows/ci.yml` (five required PR checks) +
  `.github/workflows/nightly.yml` (full-workspace mutants with trend
  tracking), per-feature mutation testing strategy, GitHub Flow branch
  model. See
  [`devops/wave-decisions.md`](../feature/phase-1-foundation/devops/wave-decisions.md).
- **DELIVER** → 16 steps across 6 phases (one phase per slice) following
  the Outside-In TDD rhythm (RED-acceptance → RED-unit → GREEN →
  REFACTOR → PROPTEST → MUTATION → COMMIT). 80 DES phase events (16 × 5
  lifecycle events) recorded green; one step (04-03) justified a
  RED_UNIT skip inline. See
  [`deliver/roadmap.json`](../feature/phase-1-foundation/deliver/roadmap.json)
  and
  [`deliver/execution-log.json`](../feature/phase-1-foundation/deliver/execution-log.json).

## Artifacts produced

### Platform crates

- `crates/overdrive-core` — 11 identifier newtypes (`JobId`, `NodeId`,
  `AllocationId`, `SpiffeId`, `PolicyId`, `InvestigationId`, `Region`,
  `ContentHash`, `SchematicId`, `CertSerial`, `CorrelationKey`),
  `IntentStore` + `ObservationStore` traits, the six nondeterminism
  traits (`Clock`, `Transport`, `Entropy`, `Dataplane`, `Driver`,
  `Llm`), state-layer types (`StateSnapshot`, `TxnOp`, `ObservationRow`,
  `Verdict`, etc.), `IdParseError`, crate-level `Error` + `Result`
  alias. `crate_class = "core"`.
- `crates/overdrive-store-local` — redb-backed `LocalStore` with
  deterministic rkyv-archived snapshot framing (`OSNP` magic + u16
  version + rkyv payload sorted by key). `crate_class = "adapter-real"`.
- `crates/overdrive-sim` — seven Sim adapters (`SimClock`,
  `SimTransport`, `SimEntropy`, `SimDataplane`, `SimDriver`, `SimLlm`,
  `SimObservationStore`), turmoil-based `Harness`, `Invariant` enum
  with six variants, invariant evaluators. `crate_class = "adapter-sim"`.
- `xtask` — `cargo xtask dst` (seed printing, text-log + JSON summary
  artifacts per ADR-0006), `cargo xtask dst-lint` (syn-based banned-API
  walker across core-class crates). Pre-existing `ci` / `mcp` / `lima`
  / `hooks` subcommands untouched. `crate_class = "binary"`.

### CI and tooling

- `.github/workflows/ci.yml` — five required PR checks (fmt+clippy,
  test, dst, dst-lint, mutants-diff) per ADR-0006.
- `.github/workflows/nightly.yml` — full-workspace mutants + trend
  tracking against `mutants-baseline/main/`.
- `.cargo/mutants.toml` — exclusion rules (unsafe, aya-rs /
  overdrive-bpf, generated code, async scheduling, tests/benches,
  xtask). Matches `.claude/rules/testing.md` §Mutation testing.
- `mutants-baseline/main/.gitkeep` — baseline directory seeded so the
  first nightly run has somewhere to write `kill_rate.txt`.
- `lefthook.yml` kept coherent with the new CI workflow (pre-commit
  runs fmt/toml/yaml checks; pre-push runs `cargo xtask ci`).

### Documentation

- Six ADRs under `docs/product/architecture/`: 0001 complete-scaffolding-
  in-place, 0002 schematic-id-canonicalisation (rkyv), 0003 core-crate
  labelling, 0004 overdrive-sim-single-crate, 0005 test-distribution,
  0006 ci-wiring-dst-gates.
- `docs/product/architecture/brief.md` — Application Architecture section
  (C4 L1 + L2 Mermaid + L3 for `overdrive-sim`).
- `crates/overdrive-core/tests/compile_fail/intent_vs_observation.rs` —
  trybuild compile-fail test for `IntentStore` ↔ `ObservationStore`
  non-substitutability.
- `crates/overdrive-core/tests/compile_fail/observation_write_rejects_intent.rs`
  — trybuild compile-fail test proving the observation write surface is
  parametric on `ObservationRow`, not raw bytes.
- 80 DES lifecycle events recorded in
  [`deliver/execution-log.json`](../feature/phase-1-foundation/deliver/execution-log.json).

## Key decisions

| Decision | Source | Rationale |
|---|---|---|
| Intent vs observation is a compile-time boundary (distinct traits, distinct stores) | `brief.md` §6, whitepaper §4 | Stops the wrong-store class of bug at the type level; the trybuild harness makes substitution non-compiling. |
| `SchematicId` canonicalisation uses rkyv-archived bytes | [ADR-0002](../product/architecture/adr-0002-schematic-id-canonicalisation.md) | Matches `development.md` "internal data → rkyv"; no new dep; deterministic by construction. |
| Every workspace crate declares `package.metadata.overdrive.crate_class` | [ADR-0003](../product/architecture/adr-0003-core-crate-labelling.md) | Locality + "every crate must declare" assertion closes the silent-skip blind spot in the lint gate. |
| Single `overdrive-sim` crate hosts every Sim adapter, the harness, and the invariant catalogue | [ADR-0004](../product/architecture/adr-0004-overdrive-sim-single-crate.md) | Minimal compile cost for turmoil; one-sentence dep-boundary rule; no consumer wants one adapter in isolation. |
| Plumbing tests in `tests/*.rs`; DISTILL acceptance scenarios in `tests/acceptance/*.rs` | [ADR-0005](../product/architecture/adr-0005-test-distribution.md) | Clear DISTILL-scenario → test-file mapping; plumbing stays visibly separate. |
| `cargo xtask dst` is the canonical DST entry point; seed on line 1; text + JSON artifacts always written | [ADR-0006](../product/architecture/adr-0006-ci-wiring-dst-gates.md) | Dev/CI path unity; seed preserved on killed runs; dashboard-parseable JSON. |
| Complete existing scaffolding in place; do not refactor | [ADR-0001](../product/architecture/adr-0001-complete-scaffolding-in-place.md) | Structural shape was correct; all work is additive (proptests, adapter crates, DST harness). |
| K4 (LocalStore cold start / RSS) deferred from Phase 1 acceptance gate to Phase 2+ guardrail | [`design/upstream-changes.md`](../feature/phase-1-foundation/design/upstream-changes.md) | Benchmarking LocalStore in isolation measures the wrong thing; commercial claim applies to a control-plane process Phase 1 does not ship. |
| Walking Skeleton Strategy C — real redb against `tempfile::TempDir` | DWD-01 in [`distill/wave-decisions.md`](../feature/phase-1-foundation/distill/wave-decisions.md) | Only posture compatible with "DST tests compose LocalStore with sim traits, but the store itself is real." |
| trybuild is the compile-fail gate; dev-dependency of `overdrive-core` only | ADR-0001 row 24 + `.claude/rules/testing.md` | IntentStore ↔ ObservationStore non-substitutability is type-level by nature; no alternative mechanism exists. |
| `xtask` excluded from `cargo-mutants` runs | `.cargo/mutants.toml` rule 6 | xtask is binary glue / CLI dispatch; mutations against it produce noise rather than actionable kill-rate signal. |
| Per-feature mutation testing, diff-scoped per PR, full-workspace nightly | DEVOPS Decision 9 + `.claude/rules/testing.md` | Matches testing-rules mandate exactly; per-PR budget stays tight, trend drift caught nightly. |
| GitHub Flow branching; PR-gated with five required checks | DEVOPS Decision 8 | Matches the repo's single-main shape and the per-feature branch already in use. |

## KPIs (outcome)

From
[`discuss/outcome-kpis.md`](../feature/phase-1-foundation/discuss/outcome-kpis.md):

- **K1** — `cargo xtask dst` green in <60s wall-clock on a clean clone.
  ✅ Step 06-02 enforces; CI `test`/`dst` jobs gate on this.
- **K2** — CI blocks banned-API smuggling in core crates with 0% false
  positives on wiring crates. ✅ Step 05-02 ships; `dst-lint` job gates
  on this.
- **K3** — Red runs reproduce bit-for-bit from the printed seed on the
  same git SHA + toolchain. ✅ Step 06-03 (twin-run identity proptest +
  canary red run) enforces.
- **K4** — LocalStore cold start <50ms; RSS <30MB. ⏸ **Deferred** to
  Phase 2+ guardrail per
  [`design/upstream-changes.md`](../feature/phase-1-foundation/design/upstream-changes.md).
  Re-examined when the control-plane process the claim applies to lands.
- **K5** — Every identifier type in the public API is a complete
  newtype; zero `String`-as-identifier on the public surface. ✅ Step
  02-03 (extended newtype completeness) + step 01-02 (`public_api_shape`
  test) enforce.
- **K6** — `export_snapshot → bootstrap_from → export_snapshot` is
  byte-identical. ✅ Step 03-02 (snapshot round-trip proptest) enforces.

North Star (K1 ∧ K3) green. Guardrails (DST wall-clock, lint-gate
false-positive rate, snapshot round-trip) monitored in CI.

## Known follow-ups

From
[`deliver/upstream-issues.md`](../feature/phase-1-foundation/deliver/upstream-issues.md):

1. **`SimObservationStore::PeerState::dominates_for_merge` tiebreak — 4
   missed mutants** (lines 200-204). The equal-timestamp branch is only
   hit by a single 04-02 scenario; the 04-03 proptest generator rarely
   synthesises exact timestamp collisions. **Phase 2 home**: convergence-
   engine work will add reconciler-driven tests exercising equal-
   timestamp scenarios under real load.
2. **`evaluate_sim_observation_lww` internal write/compare — 3 missed
   mutants** (`invariants/evaluators.rs:302,329,330`). The invariant
   *result* is what CI gates on, not the setup bytes. **Phase 2 home**:
   real reconcilers writing through the observation store make the
   evaluator's setup code a test subject on its own.
3. **`overdrive-host::entropy::CountingOsEntropy` — 5 unwired mutants**
   (moved from `overdrive-sim/src/real/mod.rs:132,138,143` when the
   host adapters were extracted into their own `adapter-real` crate).
   No production call site depends on `overdrive-host` yet, so the
   mutation run has no test that could kill these. Not a Phase 1 gap.
   **Phase 2+ home**: node-agent and control-plane wiring land
   `SystemClock` / `OsEntropy` / `TcpTransport` on real call paths.

Platform-code kill rate excluding `xtask/**` and the unwired
`overdrive-host` code: ≈ 95.5% (149 / 156). No Phase 1 rework
required.

## What this unblocks

Every Phase 2 feature now has: typed identifiers, a working state-store
split with byte-identical snapshot round-trip, a deterministic
simulation environment with seven Sim adapters, a turmoil-based DST
harness with an extensible invariant catalogue, and a CI lint gate that
mechanically blocks banned-API leakage into core-class crates.

Remaining Phase 2 issues unblocked by this scaffold:

- **#9** — Convergence engine (reconciler runtime): consumes
  `IntentStore`, `ObservationStore`, `Clock`, `Entropy`, adds new
  invariants to the catalogue.
- **#14** — Execution layer (workload drivers): consumes `Driver`,
  `Dataplane`, `Transport`; ships real `CloudHypervisorDriver`,
  `ProcessDriver`, etc. against the existing trait surface.
- **#15** — CLI (`overdrive` binary): consumes every newtype and the
  `IntentStore` trait directly; no more placeholder main.
- **#17** — Object store adapter (Garage): content-addressed by
  `ContentHash` (already shipped).
- **#18** — SRE investigation agent: consumes `Llm` (Sim already
  ships transcript-replay; real `RigLlm` is the new work).
- **#20** — Operator auth (operator SPIFFE IDs): consumes `SpiffeId`
  + `CertSerial` + the `IntentStore` trust bundle surface.
- **#21** — Corrosion-backed `ObservationStore`: real adapter against
  the trait Sim already satisfies; LWW semantics already validated.
- **#22** — Workflow runtime: adds new invariants
  (`ReplayEquivalentEmptyWorkflow` already scaffolded as a Phase 1
  stub) and consumes `Clock` + `Entropy` through `WorkflowCtx`.

## Commit range

`17d0c27..ba6884e` (35 commits — workspace bootstrap through feature-
level WS acceptance gate).
