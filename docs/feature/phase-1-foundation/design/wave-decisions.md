# DESIGN Wave Decisions — phase-1-foundation

**Wave**: DESIGN (solution-architect)
**Owner**: Morgan
**Mode**: Propose (analysis + options + recommendation)
**Paradigm**: OOP (Rust trait-based) — confirmed against `.claude/rules/development.md`
**Date**: 2026-04-21
**Status**: COMPLETE — handoff-ready for DISTILL (acceptance-designer)

---

## Handoff package index

**Feature delta** (this directory):

- `docs/feature/phase-1-foundation/design/wave-decisions.md` (this file)
  — options, trade-offs, and the six chosen decisions.
- `docs/feature/phase-1-foundation/design/upstream-changes.md`
  — back-propagation record for assumptions changed since DISCUSS (K4
  reframe).

**SSOT** (product architecture):

- `docs/product/architecture/brief.md` — Application Architecture section
  owned by Morgan. Includes C4 L1 + L2 (Mermaid) and a L3 component
  diagram for `overdrive-sim`.
- `docs/product/architecture/adr-0001-complete-scaffolding-in-place.md`
- `docs/product/architecture/adr-0002-schematic-id-canonicalisation.md`
- `docs/product/architecture/adr-0003-core-crate-labelling.md`
- `docs/product/architecture/adr-0004-overdrive-sim-single-crate.md`
- `docs/product/architecture/adr-0005-test-distribution.md`
- `docs/product/architecture/adr-0006-ci-wiring-dst-gates.md`

---

## Reuse Analysis (MANDATORY gate)

Scanned existing codebase for every proposed component. Row count: 19
(11 newtypes, 8 trait ports, `overdrive-store-local`, `overdrive-sim`,
`xtask dst`, `xtask dst-lint`, `overdrive-cli`, compile-fail harness,
snapshot framing, invariant enum — the last three are sub-components
and grouped below the crate rows).

| # | Existing Component | File | Overlap | Decision | Justification |
|---|---|---|---|---|---|
| 1 | `JobId`, `NodeId`, `AllocationId`, `PolicyId`, `InvestigationId`, `Region` label newtypes | `crates/overdrive-core/src/id.rs` via `define_label_newtype!` | Exact match — FromStr/Display/serde via `try_from`/`into`, validating constructor, DNS-1123-ish label | **EXTEND** | Add proptest coverage for round-trip and negative cases; no code refactor. (AC: US-01 AC list.) |
| 2 | `SpiffeId` | `crates/overdrive-core/src/id.rs` | Exact match — structured `trust_domain()` / `path()` accessors, case-insensitive parse, canonical lowercase, serde | **EXTEND** | Add proptest + whitepaper-canonical-example test; no code refactor. |
| 3 | `ContentHash` (SHA-256, 32 bytes, hex `Display`/`FromStr`) | `crates/overdrive-core/src/id.rs` | Exact match — `from_bytes`, `of(data)`, `from_hex`, serde | **EXTEND** | Add proptest + length-edge-case tests. |
| 4 | `SchematicId` (newtype wrapping `ContentHash`) | `crates/overdrive-core/src/id.rs` | Present but lacks canonicalisation rule — no constructor that takes a `Schematic` value | **EXTEND** | Ships the `Schematic` struct + `SchematicId::from_schematic(&schematic)` rkyv-based constructor in Phase 1 as a follow-up. Phase 1 commits to rkyv (ADR-0002); impl is additive. |
| 5 | `CertSerial` | `crates/overdrive-core/src/id.rs` | Exact match — lowercase hex only, RFC-5280 20-byte ceiling, serde | **EXTEND** | Add proptest for upper/lower/odd-length/too-long cases. |
| 6 | `CorrelationKey` (derived from target + spec_hash + purpose) | `crates/overdrive-core/src/id.rs` | Exact match — deterministic SHA-256-based derive, serde | **EXTEND** | Add proptest asserting determinism across inputs. |
| 7 | `IdParseError` | `crates/overdrive-core/src/id.rs` | Exact match — structured variants (Empty / TooLong / InvalidChar / InvalidFormat / SpiffeMissingScheme / SpiffeEmptyTrustDomain / SpiffeEmptyPath / ContentHashWrongLength) | **EXTEND** | Add missing variants as needed (none currently known); no refactor. |
| 8 | `Clock` trait | `crates/overdrive-core/src/traits/clock.rs` | Exact match — `now()`, `unix_now()`, `sleep()` | **EXTEND** | No change to trait; Real (`SystemClock`) + Sim (`SimClock`) adapters land in new crates. |
| 9 | `Transport` trait + `Connection` blanket impl + `TransportError` | `crates/overdrive-core/src/traits/transport.rs` | Exact match — `connect` returning `Box<dyn Connection>`, `send_datagram`, error variants with `#[source]` | **EXTEND** | No change to trait; adapters land in new crates. |
| 10 | `Entropy` trait | `crates/overdrive-core/src/traits/entropy.rs` | Exact match — `u64`, `fill` | **EXTEND** | No change; adapters land in new crates. |
| 11 | `Dataplane` trait + `Verdict`, `Backend`, `PolicyKey`, `FlowEvent`, `DataplaneError` | `crates/overdrive-core/src/traits/dataplane.rs` | Exact match — `update_policy`, `update_service`, `drain_flow_events` | **EXTEND** | No change; Sim adapter is in-memory HashMap (US-05 scope). Real adapter is Phase 2+. |
| 12 | `Driver` trait + `DriverType` + `AllocationSpec` + `AllocationHandle` + `AllocationState` + `Resources` + `DriverError` | `crates/overdrive-core/src/traits/driver.rs` | Exact match — start/stop/status/resize, typed driver variant enum | **EXTEND** | No change; Sim adapter is in-memory allocation table. Real adapters (CloudHypervisor, Process, Wasm) are Phase 2+. |
| 13 | `IntentStore` trait + `TxnOp`, `TxnOutcome`, `StateSnapshot`, `IntentStoreError` | `crates/overdrive-core/src/traits/intent_store.rs` | Exact match — get/put/delete/txn/watch/export_snapshot/bootstrap_from | **EXTEND** | Implement (not mock) `LocalStore` against redb in new `overdrive-store-local` crate. (AC: US-03.) |
| 14 | `ObservationStore` trait + `Value`, `Rows`, `ObservationStoreError` | `crates/overdrive-core/src/traits/observation_store.rs` | Exact match — read/write/subscribe against SQL-shaped API | **EXTEND** | Sim impl in `overdrive-sim::adapters::observation_store::SimObservationStore`. Real `CorrosionStore` is Phase 2+. |
| 15 | `Llm` trait + `Prompt`, `Completion`, `ToolDef`, `ToolCall`, `Usage`, `Message`, `Role`, `LlmError` | `crates/overdrive-core/src/traits/llm.rs` | Exact match — `complete(prompt, tools)` | **EXTEND** | Sim impl (`SimLlm` with transcript replay) in overdrive-sim. Real `RigLlm` is Phase 3+. |
| 16 | `Error` + `Result` alias (top-level) | `crates/overdrive-core/src/error.rs` | Exact match — `#[from] IdParseError`, Result alias | **EXTEND** | Add new variants only if needed by future ports; none required in Phase 1. |
| 17 | `xtask dst` subcommand stub | `xtask/src/main.rs::dst` | Present; currently shells out to `cargo test --workspace --features dst` with `OVERDRIVE_DST_SEED` env passthrough | **EXTEND** | Fill in per ADR-0006 (seed generation, first-line seed print, summary formatter, JSON artifact). |
| 18 | `xtask dst-lint` subcommand | — (not present) | No overlap | **CREATE NEW** | No alternative. The banned-API scan requires a `syn`-based walker over `src/**/*.rs` filtered by `crate_class = "core"`. No existing Rust tool enforces this (cargo-deny checks dependencies, not call sites; clippy has no custom lint registration without a compiler plugin). Custom tooling is the only path; see ADR-0003, ADR-0006. |
| 19 | `overdrive-cli` (`overdrive` binary) | `crates/overdrive-cli/src/main.rs` | Placeholder binary that parses args and logs; no control-plane wiring in Phase 1 | **EXTEND** (no-op) | Acknowledged — no Phase 1 work on CLI. Phase 2 adds real handlers. |

**Sub-components below the crate line**:

| # | Existing Component | File | Overlap | Decision | Justification |
|---|---|---|---|---|---|
| 20 | `overdrive-store-local` (`LocalStore` on redb) | — | No overlap | **CREATE NEW** | No alternative. redb is the whitepaper-committed IntentStore backend (§4, §17). The `IntentStore` trait exists in overdrive-core; an adapter crate is required to host the redb dependency and `LocalStore` type. Cannot live in `overdrive-core` because core is class `core` (no redb dep allowed on the core-compile path). |
| 21 | `overdrive-sim` crate (all Sim* adapters + harness + invariants) | — | No overlap | **CREATE NEW** | No alternative. Hosts `turmoil` dependency and the `Sim*` adapters; cannot live in overdrive-core (turmoil would leak onto the core-compile path) and cannot live in overdrive-store-local (separation of concerns, and turmoil is not the IntentStore's dependency). Single crate vs split rationale: ADR-0004. |
| 22 | `Schematic` struct + rkyv derive | — | `SchematicId` exists but the input type that feeds it does not | **CREATE NEW** | No alternative. The whitepaper §23 schematic TOML requires a typed Rust struct to deserialize into for canonical archival. Adding a struct in `overdrive-core` (or a dedicated `overdrive-schematic` crate later) is additive; the SchematicId canonicalisation rule (ADR-0002) requires it. |
| 23 | `Invariant` enum + evaluator traits | — | No overlap | **CREATE NEW** | No alternative. `overdrive-sim::invariants::Invariant` is the canonical name-source referenced by `shared-artifacts-registry` with HIGH integration risk. An enum with `FromStr`/`Display` is the exact shape needed. |
| 24 | `compile_fail/` test harness (trybuild-based) | — | No overlap | **CREATE NEW** | No alternative. `IntentStore` / `ObservationStore` non-substitutability is explicitly stated by US-04 as a compile-time property. `trybuild` is the only ecosystem-standard way to assert "this code does not compile." Added as a `dev-dependency` to `overdrive-core` only. |
| 25 | Snapshot framing header (version + payload) | — | `StateSnapshot { version, entries }` exists but is in-memory only; on-disk framing is not yet defined | **CREATE NEW** | No alternative. US-03 AC requires bit-identical round-trip of `export_snapshot → bootstrap_from → export_snapshot`. The framing byte layout is a new artifact (magic bytes + version + rkyv payload). Not a new trait or crate — it is a helper module under `overdrive-store-local::snapshot_frame` implementing the contract via rkyv. |
| 26 | `BANNED_APIS` constant | — | No overlap | **CREATE NEW** | No alternative. Referenced by `shared-artifacts-registry` as the single source of truth for the banned-API list, consumed by `xtask dst-lint` and by `.claude/rules/development.md` documentation. |

**Summary counts**:

- **EXTEND**: 17 (rows 1–17)
- **CREATE NEW**: 7 (rows 18–26: `xtask dst-lint`, `overdrive-store-local`,
  `overdrive-sim`, `Schematic` struct, `Invariant` enum, compile-fail
  harness, snapshot framing header, `BANNED_APIS` constant — note the
  eighth entry is subsumed into the snapshot framing item)
- Every CREATE NEW entry carries explicit "no existing alternative"
  justification.

---

## Paradigm confirmation

**OOP (Rust trait-based)** — confirmed against:

- `.claude/rules/development.md` (thiserror, Result alias, newtypes STRICT,
  `Send + Sync`, state-layer hygiene — all OOP-shaped rules).
- `crates/overdrive-core/src/traits/*.rs` (every existing port is `trait
  Foo: Send + Sync + 'static`).
- `crates/overdrive-core/src/id.rs` (every newtype is a `struct` with
  validating constructors).
- `crates/overdrive-core/src/error.rs` (error is an `enum` under thiserror).

No functional-first pull. The platform has substitution semantics via
trait objects, not algebraic effects; composition via `Arc<dyn Trait>`,
not monad stacks.

The CLAUDE.md `## Development Paradigm` section declaring OOP and
pointing at `@nw-software-crafter` is applied. No further user action
required on paradigm confirmation.

---

## The six DESIGN decisions

### Decision 1 — Reconcile partial scaffolding

**Question**: complete the existing `crates/overdrive-core/src/traits/*` and
`id.rs` in place, or refactor to a new layout?

**Options considered**:

- **A. Complete in place.** Add proptest coverage and new adapter crates;
  leave existing trait and newtype code untouched.
- **B. Refactor to separate `overdrive-ports` crate.** Split port traits
  out of `overdrive-core` into their own crate; `overdrive-core` becomes
  newtypes + errors only.
- **C. Rewrite traits with native async-in-trait.** Replace `async_trait`
  with stable async trait functions; replace `Box<dyn Connection>` with
  generic associated types.

**Trade-offs**:

| Option | Work | Future flexibility | Risk |
|---|---|---|---|
| A | ~zero | medium — ports and newtypes share the same crate (fine; they are both "stable vocabulary for everyone else") | low |
| B | ~1 day | marginal — consumers would import both crates anyway | medium — greenfield refactor with no evidence the current shape causes friction |
| C | ~2 days | high — ergonomic — but turmoil and async-trait ecosystem still interop with `Box<dyn>` by convention | medium — native async-in-trait is still maturing; risk of churn |

**Chosen**: **A. Complete in place**. See ADR-0001.

**One-liner**: Scaffolding is structurally correct; all Phase 1 work is
additive (proptests, new adapter crates, DST harness).

### Decision 2 — `SchematicId` canonicalisation

**Question**: rkyv-archived bytes or RFC 8785 JCS?

**Options considered**:

- **A. rkyv-archived bytes.** Hash the archived bytes of the `Schematic`
  struct.
- **B. RFC 8785 JCS over JSON.** Serialise the schematic to JSON,
  canonicalise via a JCS implementation, hash the canonical bytes.
- **C. Hand-rolled canonical TOML.** Reformat the schematic TOML via a
  bespoke canonicaliser and hash the result.

**Trade-offs**:

| Option | Deterministic | External interop | New dep | Matches `development.md` guidance |
|---|---|---|---|---|
| A | yes (by construction) | Rust only | no — rkyv already in workspace | "internal data → rkyv" (exact) |
| B | yes (by spec) | any language | yes (`serde_jcs` or equivalent) | "external / JSON data → RFC 8785" — this is internal, so guidance disrecommends JCS |
| C | partially — requires new canonicalisation spec | any language with canonicaliser | writing it from scratch | no ecosystem spec for canonical TOML exists |

**Chosen**: **A. rkyv-archived bytes**. See ADR-0002.

**One-liner**: Matches `development.md` exactly; no new dep; composes with
existing snapshot framing.

### Decision 3 — Core-crate labelling mechanism

**Question**: how is a crate identified as "core" for the banned-API lint?

**Options considered**:

- **A. `package.metadata.overdrive.crate_class = "core"` per crate.**
- **B. `workspace.metadata.overdrive.core_crates = [...]` central list.**
- **C. Filesystem convention (name-based).**
- **D. Build-script sentinel files.**
- **E. Dedicated manifest file at workspace root.**

**Trade-offs**:

| Option | Locality | Drift risk | Toolchain cost | Extensibility (more classes later) |
|---|---|---|---|---|
| A | high — lives next to the crate | low — enforced by "every crate must declare" assertion | `cargo metadata` (already used) | excellent — enum of classes |
| B | low — distant from the crate | high — drift on every new crate | `cargo metadata` | medium |
| C | implicit | N/A | none | poor — forces naming to match class |
| D | medium | medium — build script must be maintained | slows all builds slightly | medium |
| E | low | medium | file parser | medium |

**Chosen**: **A. `package.metadata.overdrive.crate_class` with required
declaration on every workspace crate.** See ADR-0003.

**One-liner**: Locality + an enforced "every crate declares" assertion
closes the "new core crate added, not labelled, lint silently skipped"
blind spot.

### Decision 4 — `overdrive-sim` crate layout

**Question**: one crate for Sim* + harness + invariants, or split?

**Options considered**:

- **A. Single `overdrive-sim` crate.** All Sim adapters, harness, invariants
  in one crate.
- **B. Three crates (`overdrive-sim-traits`, `overdrive-sim-impls`,
  `overdrive-sim-harness`).**
- **C. Per-adapter crate (seven + harness + invariants).**

**Trade-offs**:

| Option | Compile time | Dep-boundary clarity | Semver surface | Reuse potential |
|---|---|---|---|---|
| A | best — turmoil compiles once | excellent — "one crate holds turmoil + StdRng" | minimal | adequate — consumers get the whole crate (acceptable at scope) |
| B | mediocre — impls + harness both need turmoil, no real separation | poor — no natural boundary | 3x | none — impls alone cannot be used without harness |
| C | worst — N crates × turmoil dep declarations | medium | Nx | hypothetical — no known consumer wants one adapter only |

**Chosen**: **A. Single `overdrive-sim` crate.** See ADR-0004.

**One-liner**: Minimum compile + coordination cost; turmoil dep boundary
is a one-sentence rule.

### Decision 5 — Test distribution

**Question**: per-crate `tests/*.rs` vs top-level `tests/acceptance/*.rs`?

**Options considered**:

- **A. `tests/acceptance/` is canonical for all integration tests.**
- **B. `tests/*.rs` plumbing; `tests/acceptance/` reserved for DISTILL
  scenarios only.**
- **C. Everything inline (`#[cfg(test)] mod tests`).**

**Trade-offs**:

| Option | Scenario traceability | Plumbing test home | Fits testing.md pattern |
|---|---|---|---|
| A | mixed — acceptance + plumbing in one directory | no obvious home | partial — testing.md mentions both locations |
| B | clean — acceptance = user-scenario, plumbing = integration | `tests/*.rs` top-level | exact |
| C | poor — cannot separate public-API integration | N/A | no |

**Chosen**: **B. Per-crate `tests/*.rs` for plumbing; `tests/acceptance/`
is for DISTILL scenarios only (empty in Phase 1 until DISTILL runs).**
See ADR-0005.

**One-liner**: Clear mapping DISTILL scenario → `tests/acceptance/` file;
plumbing tests stay visibly separate.

### Decision 6 — CI wiring for `xtask dst` + `xtask dst-lint`

**Question**: what is the xtask command surface, where do failure
artifacts go, and how is the seed surfaced?

**Options considered**:

- **A. Run `cargo test` directly from CI; no xtask wrapper.**
- **B. Deterministic seed (from git SHA).**
- **C. xtask wrapper without JSON artifact.**
- **D. xtask wrapper with seed-as-first-line, JSON + text artifacts.**

**Trade-offs**:

| Option | Seed visibility on partial runs | CI dashboard parse cost | Dev/CI path unity |
|---|---|---|---|
| A | depends on `cargo test` output | high — regex over text | poor — CI diverges from `cargo xtask` |
| B | always same on same SHA — misses flaky discovery space | free | good |
| C | yes | high | good |
| D | yes (first line) | low (JSON) | excellent |

**Chosen**: **D. xtask wrapper; seed on first line; text log + JSON
summary both uploaded as CI artifacts.** See ADR-0006.

**One-liner**: One canonical invocation (`cargo xtask dst`); seed
preserved even on killed runs; dashboard-parseable.

---

## Architecture enforcement (annotation for crafter)

**Style**: Hexagonal (ports + adapters), single-process Rust workspace
**Language**: Rust 2024 edition, rustc ≥ 1.85
**Tools**:

- `cargo xtask dst-lint` (custom, `syn`-based) — primary enforcement
- `cargo clippy` workspace pedantic+nursery+cargo
- `trybuild` compile-fail tests for type-level contracts
- `proptest` for newtype / snapshot / LWW-convergence round-trip

**Rules to enforce** (referenced in the brief §9, repeated here for the
crafter as an explicit checklist):

1. Core crates declare `crate_class = "core"` and do NOT use
   `Instant::now`, `SystemTime::now`, `rand::random`, `rand::thread_rng`,
   `tokio::time::sleep`, `std::thread::sleep`,
   `tokio::net::{TcpStream, TcpListener, UdpSocket}`.
2. Every workspace crate declares a `crate_class` (enforced by `dst-lint`
   pre-scan assertion).
3. `IntentStore` and `ObservationStore` are not type-substitutable
   (trybuild `tests/compile_fail/intent_vs_observation.rs`).
4. Every newtype: Display/FromStr/serde/rkyv round-trip lossless
   (`crates/overdrive-core/tests/newtype_roundtrip.rs`).
5. Every newtype: `serde_json::to_string(&x)` equals
   `format!("\"{}\"", x.to_string())` for identifiers that serialize as
   strings.
6. `LocalStore::export_snapshot → LocalStore::bootstrap_from →
   LocalStore::export_snapshot` is bit-identical
   (`crates/overdrive-store-local/tests/snapshot_roundtrip.rs`).
7. `SimObservationStore` LWW converges deterministically under seeded
   reordering (`crates/overdrive-sim/tests/dst/sim_observation_lww_converges.rs`).
8. DST twin-run under the same seed is bit-identical
   (`crates/overdrive-sim/tests/dst/twin_run_identity.rs`).

---

## Quality gates

- [x] Requirements traced to components (reuse table + US-01..US-06 AC)
- [x] Component boundaries with clear responsibilities (crate topology,
      class labelling, C4 L2)
- [x] Technology choices in ADRs with alternatives (6 ADRs)
- [x] Quality attributes addressed (brief §11; K1–K6 measurable in CI)
- [x] Dependency-inversion compliance (hexagonal: ports in
      `overdrive-core`, adapters in sibling crates)
- [x] C4 diagrams (L1 + L2 in brief; L3 for `overdrive-sim`)
- [x] Integration patterns specified (there are none external; handoff
      annotation explicit about zero contract tests needed Phase 1)
- [x] OSS preference validated (every dep MIT / Apache-2; list in
      brief §10)
- [x] AC behavioural, not implementation-coupled (design speaks of
      traits + observable properties; crafter owns internal structure)
- [x] External integrations annotated (none — annotated explicitly in
      brief §12)
- [x] Architectural enforcement tooling recommended (brief §9 +
      annotation above)
- [ ] Peer review completed and approved — **pending**; see below

---

## Peer review

Peer review (solution-architect-reviewer) will run after this file is
written and the user confirms paradigm + CLAUDE.md amendment. Per wave
protocol: max 2 iterations, all critical/high issues addressed, handoff
to DISTILL only after approval.

---

## Open questions surfaced for user

1. **`Schematic` struct home**: should the struct land in `overdrive-core`
   alongside `SchematicId`, or in a dedicated future `overdrive-schematic`
   crate? Phase 1 can defer the struct entirely if the image factory is
   out of scope; ADR-0002 commits to the canonicalisation rule but not
   the struct's location. Recommend: defer the struct to Phase 2 when the
   image factory surface actually lands — Phase 1 only needs the
   SchematicId type (already exists) and the committed canonicalisation
   rule.

Does not block handoff.

---

## Changed Assumptions (back-propagation to DISCUSS)

- **K4 reframed from Phase 1 acceptance gate to Phase 2+ commercial
  guardrail.** Phase 1 is a walking skeleton; it does not run the
  control-plane process the `<30MB RSS` / `<50ms cold start` commercial
  claim applies to. Benchmarking `LocalStore` in isolation measures the
  wrong thing and invites flaky CI bypass culture. Full rationale and
  original-text preservation in
  `docs/feature/phase-1-foundation/design/upstream-changes.md`. The
  DISCUSS `outcome-kpis.md` is NOT edited directly — the
  back-propagation rule requires the DISCUSS artifact stays the
  as-of-DISCUSS truth.

---

## What is NOT being decided here (deferred to DISTILL / DELIVER)

- Concrete trait-method signatures beyond what the current scaffolding
  already exposes — the crafter owns any additive signatures.
- Exact `redb` table layout inside `LocalStore` — crafter's call during
  GREEN.
- Exact rkyv derive attributes on `StateSnapshot` — crafter's call.
- Invariant-evaluation implementation (how exactly `single_leader` is
  computed against the stubbed topology) — crafter's call during GREEN.
- Test-scenarios.md — that is DISTILL's deliverable, drawn from
  `user-stories.md`.
- **Concrete "current scaffolding → Phase 1 target" gap list.** The
  crafter produces this as part of the DELIVER roadmap. Inputs: the
  mandatory call sites enumerated in `.claude/rules/testing.md`
  (Tier 1 DST, proptest mandatory call sites, mutation-test mandatory
  targets) and the AC checklists in
  `docs/feature/phase-1-foundation/discuss/user-stories.md` (US-01
  through US-06). DESIGN owns component boundaries and trait surfaces;
  DELIVER owns the ordered task list that closes the gap.
- **`SimObservationStore` row schema versioning mechanism.** Phase 1
  row shapes (`alloc_status`, `node_health`) are locked (brief §6);
  how those rows evolve across Phase 1 → Phase 2 without breaking the
  `CorrosionStore` wire compatibility is a crafter decision at
  implementation time. Phase 2 feature scope will lock the mechanism
  as part of its own DESIGN wave.

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-21 | Initial DESIGN wave decisions for phase-1-foundation. |
| 2026-04-22 | Review revisions: added upstream-changes.md index entry and K4 reframe (Changed Assumptions); removed "paradigm pending" language (CLAUDE.md already amended); added deferral entries for scaffolding-gap list (#3) and SimObservationStore row schema versioning (#4). |
