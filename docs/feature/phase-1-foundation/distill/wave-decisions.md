# DISTILL Wave Decisions — phase-1-foundation

**Wave**: DISTILL (acceptance-designer)
**Owner**: Quinn
**Date**: 2026-04-22
**Status**: COMPLETE — handoff-ready for DELIVER (software-crafter)

---

## DWD-01 — Walking Skeleton Strategy: C (Real local)

**Decision**: the walking skeleton uses **real local adapters** for every
local resource; no paid externals; no mocks.

**Concrete bindings:**

- `LocalStore` wraps **real redb** against `tempfile::TempDir` in every
  `@walking_skeleton` scenario that exercises intent storage. This is the
  actual production adapter, rebound to a per-test scratch directory. A
  deleted `redb` dep would fail the walking-skeleton scenarios — which is
  the Strategy C litmus test per the critique-dimensions skill (9d).
- `Sim*` adapters (`SimClock`, `SimTransport`, `SimEntropy`,
  `SimDataplane`, `SimDriver`, `SimLlm`, `SimObservationStore`) ARE the
  production adapters for simulation use per the architecture brief
  §1 ports table. Replacing them with anything else would corrupt
  determinism. They are not mocks — they are the intended implementation
  for the DST execution environment. No additional faking layer exists
  above them.
- No paid external services touch the walking skeleton. Nothing requires
  an `@requires_external` marker in Phase 1.
- **Tagging convention:**
  - `@walking_skeleton @real-io @adapter-integration` — end-to-end
    subprocess scenarios that exercise `LocalStore` on real redb.
  - `@walking_skeleton` (no `@real-io`) — pure-sim walking skeletons that
    prove `SimClock` / `SimTransport` / `SimEntropy` wire together
    deterministically. These do not claim real I/O because there is none;
    Strategy C requires real I/O for *local resource adapters*, which the
    sim traits are not.

**Rationale**: Strategy C is the only posture compatible with the project
rule "**DST tests compose `LocalStore` with sim traits, but the store
itself is real.**" (US-03 Technical Notes; whitepaper §21; testing.md Tier
1). Any other posture replaces the production redb adapter with a fake,
which would let wiring bugs and path-resolution errors into DELIVER
unchecked.

## DWD-02 — K4 scenarios are out of scope in Phase 1

**Decision**: no scenarios tagged `@kpi K4` are written.

**Rationale**: `docs/feature/phase-1-foundation/design/upstream-changes.md`
reframes K4 from a Phase 1 acceptance gate to a Phase 2+ commercial
guardrail. The `<50ms cold start` / `<30MB RSS` figures apply to a
control-plane process that does not exist in Phase 1; benchmarking
`LocalStore` in isolation measures the wrong thing.

**Residual note**: US-03 still lists "LocalStore cold start stays within
the commercial envelope" as a UAT scenario and carries two AC bullets
(`cold start < 50ms`, `RSS < 30MB`). These are retained in US-03 as-of-
DISCUSS truth but are NOT translated into Phase 1 acceptance scenarios
per the back-propagation record. DELIVER crafter should likewise skip
them until Phase 2 DISCUSS reopens the question.

## DWD-03 — No `.feature` files; scenarios are Gherkin-in-markdown

**Decision**: every scenario in `test-scenarios.md` is a fenced markdown
block (```gherkin ... ```). The crafter translates each to a Rust
`#[test]` / `#[tokio::test]` function in `tests/acceptance/` (per
ADR-0005).

**Rationale**: `.claude/rules/testing.md` bans `.feature` files
project-wide. DISCUSS and DESIGN both confirm this. DELIVER does not
introduce cucumber-rs, pytest-bdd, or any `.feature` consumer.

## DWD-04 — Driving ports identified for this feature

Per the architecture brief §7 (DST harness) and §12 (Integration
patterns — *none* external), the driving ports of phase-1-foundation are:

1. **`cargo xtask dst` subprocess** — the primary engineer entry point.
   Scenarios tagged `@driving_port` for this port invoke the subprocess,
   assert on exit code, stdout format (seed on first line per ADR-0006),
   and artifact file presence. They do NOT call the Rust `dst()` function
   directly.
2. **`cargo xtask dst-lint` subprocess** — the lint gate.
   `@driving_port`-tagged scenarios invoke the subprocess and assert on
   exit code plus error-message content.
3. **Library trait surface** (`IntentStore`, `ObservationStore`, `Clock`,
   etc.) — consumed by in-process DST harness code, not by end users.
   Trait-level scenarios (e.g. "LocalStore snapshot round-trip is bit-
   identical") execute in a `#[test]` function in `tests/acceptance/`
   inside the owning crate. They are tagged `@library_port` rather than
   `@driving_port` — they test the port surface through the public API,
   not through a subprocess.

Every AC in `user-stories.md` maps to at least one scenario; at least one
scenario per CLI entry point exercises the subprocess path.

## DWD-05 — Property-based scenarios marked `@property`

Per `testing.md`'s proptest mandatory call sites, certain scenarios
express universal invariants ("for any valid X, Y holds"). These carry
the `@property` tag so the DELIVER crafter translates them via
`proptest!` blocks, not single-example assertions. The mandatory sites
covered:

- Newtype `Display` / `FromStr` / serde roundtrip for every identifier.
- `IntentStore::export_snapshot → bootstrap_from → export_snapshot`
  byte-identical.
- rkyv archive / access / deserialise equality (where archival is used
  for hashing per development.md).
- Hash determinism (`ContentHash::of` is stable across invocations).

## DWD-06 — Mandate 7 RED scaffolding posture

Extensive scaffolding already exists per ADR-0001 ("complete in place").
All 11 newtype types and all 8 trait ports live in
`crates/overdrive-core/src/{id,error,traits}.rs`; `xtask dst` has a stub.
Therefore:

- DISTILL does NOT overwrite existing scaffolding. `Grep` confirms
  existing symbols before any `// SCAFFOLD: true` stub is produced.
- What genuinely does not exist and must be scaffolded for tests to
  compile:
  - `crates/overdrive-sim/` crate (all Sim* adapters + invariants +
    harness — per ADR-0004, single crate).
  - `crates/overdrive-store-local/` crate (real redb LocalStore).
  - `xtask/src/dst.rs` body (per ADR-0006 — the stub exists in main.rs
    but the full seed-printing / artifact-writing logic is new).
  - `xtask/src/dst_lint.rs` (entirely new per ADR-0006).
- Existing scaffolding for `IntentStore`, `ObservationStore`, `Clock`,
  etc. is left untouched. The acceptance harness references those
  symbols directly.

## DWD-07 — Scenario title discipline

Every scenario title describes a business-framed outcome: what the
engineer observes or what cannot happen. Bad shapes rejected:

- "`test_export_snapshot_returns_bytes`" — function-name framing.
- "`IntentStore.put succeeds on valid key`" — method-name framing.
- "End-to-end DST flow through all layers" — technical-flow framing.

Good shapes accepted:

- "Clean-clone `cargo xtask dst` is green within wall-clock budget".
- "Lint gate blocks a core crate that uses `Instant::now()`".
- "Snapshot round-trip is bit-identical across LocalStore instances".

## DWD-08 — Story-to-scenario traceability tagging

Every scenario carries a `@us-XX` tag naming the originating user story.
A single scenario that validates across multiple stories carries each tag
(e.g. `@us-03 @us-06`). Scenarios derived from the journey rather than
a single story carry `@journey:trust-the-sim` alongside the covered
`@us-XX` tags.

## DWD-09 — KPI tag shape

K1, K2, K3, K5, K6 each tag at least one scenario. K4 is deliberately
absent (DWD-02). Tag form: `@kpi K1`. Multiple KPIs on one scenario:
`@kpi K1 @kpi K3`.

## DWD-10 — Error-path ratio target

Target: ≥40% of scenarios are error/boundary/invariant-red. Counted by
reviewing the `@error-path` and invariant-red tag groups in
`test-scenarios.md`. If the ratio drops below 40%, a handoff block is
triggered.

---

## Cross-wave reconciliation record

| Delta | Source | Action |
|---|---|---|
| K4 reframed to Phase 2+ guardrail | `design/upstream-changes.md` | No K4 scenarios written (DWD-02). US-03 AC bullets kept as-of-DISCUSS truth, not translated. |
| DISCUSS already promised "no `.feature` files" | `discuss/wave-decisions.md` §2 | Honoured (DWD-03). |
| DESIGN locked `overdrive-sim` as single crate | ADR-0004 | Scaffold path: `crates/overdrive-sim/` only (DWD-06). |
| DESIGN set `overdrive-store-local` as new crate | Reuse table row 20 | Scaffold path: `crates/overdrive-store-local/` (DWD-06). |
| DESIGN committed to rkyv canonicalisation for `SchematicId` | ADR-0002 | `SchematicId`-related scenarios assume rkyv-archived-bytes hashing (see "Extended identifier newtypes" scenarios). |
| DESIGN locked the observation-row minimal set to `alloc_status` + `node_health` | brief §6 | ObservationStore scenarios reference only these rows. |

No contradictions surfaced. `CLARIFICATION_NEEDED` not required.

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-22 | Initial DISTILL wave decisions for phase-1-foundation. |
