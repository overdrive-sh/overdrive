# ADR-0001 — Complete existing trait scaffolding in place

## Status

Accepted. 2026-04-21.

## Context

`crates/overdrive-core/src/traits/` and `crates/overdrive-core/src/id.rs`
already contain partial implementations of the eight ports and eleven
newtypes required by Phase 1. The DISCUSS wave identified this as a design
decision point: is the scaffolding canonical, or does it need to be
refactored?

Reading each file against the user-story AC:

- **`id.rs`** — 11 newtypes present (`JobId`, `AllocationId`, `NodeId`,
  `PolicyId`, `InvestigationId`, `Region`, `SpiffeId`, `ContentHash`,
  `SchematicId`, `CertSerial`, `CorrelationKey`). `FromStr`, `Display`,
  `serde try_from/into` round-trip. `SpiffeId` has structured
  `trust_domain()` / `path()` accessors. A `validate_label` helper is
  module-private inside the `define_label_newtype!` macro — it is *not* a
  pub helper, so the "no `normalize_*` helpers" rule is satisfied.
- **`error.rs`** — `Error` with `#[from] IdParseError`, plus the Result
  alias. Matches `CLAUDE.md` convention.
- **`traits/*.rs`** — all eight traits present, with error enums, helper
  types (e.g. `StateSnapshot`, `Rows`, `Verdict`, `AllocationSpec`,
  `Prompt`), and documentation that cross-references the whitepaper. Each
  trait matches whitepaper §21's signature shape.

The scaffolding is well-structured, documented, clippy-clean, and already
wires the Result-alias and thiserror discipline. What is *missing* against
the Phase 1 AC:

1. Proptest coverage for newtype round-trip (AC: US-01, US-02).
2. Proptest for `LocalStore` snapshot round-trip (AC: US-03) — no
   `LocalStore` exists yet.
3. Sim adapters and harness (AC: US-04, US-06) — no `overdrive-sim` crate.
4. `xtask dst-lint` + banned-API scan (AC: US-05).
5. Compile-fail tests for IntentStore / ObservationStore non-substitutability
   (AC: US-04).

These are *additions*, not refactors. The scaffolding as it stands is
structurally correct and aligned with the architecture.

## Decision

**Complete the scaffolding in place.** Do not refactor or rename anything
in `overdrive-core`. Add:

- Proptest strategies and round-trip tests for every newtype, in
  `crates/overdrive-core/tests/newtype_roundtrip.rs`.
- `overdrive-store-local` crate with `LocalStore` implementing the existing
  `IntentStore` trait against `redb`.
- `overdrive-sim` crate with Sim* adapters, the turmoil harness, and the
  invariant catalogue.
- `xtask dst` and `xtask dst-lint` subcommand bodies (currently stubbed).
- Compile-fail tests under `crates/overdrive-core/tests/compile_fail/`.

The one small addition **inside** `overdrive-core` is a compile-fail test
harness (`trybuild` dev-dep) for the IntentStore/ObservationStore
non-substitutability invariant.

## Alternatives considered

### Option A — Refactor to new module layout

Reorganise `overdrive-core` to separate ports into their own crate
(`overdrive-ports`) and keep only newtypes and errors in `overdrive-core`.
**Rejected.** The separation has no practical benefit: every consumer of a
newtype also consumes at least one trait, and every adapter crate already
has a clean `overdrive-core = ...` workspace dependency. Splitting would
add a crate with zero internal cohesion and would require every consumer
to import both. Refactoring a greenfield scaffolding without evidence that
the current shape causes friction is precisely the "refactor for
refactor's sake" the team wants to avoid.

### Option B — Rewrite trait signatures for ergonomics

Replace `async_trait` with native async-in-trait (rustc 1.75+) and drop
the `Box<dyn Connection>` / `Box<dyn Stream<...>>` return shapes for
generic associated types. **Rejected for Phase 1.** `dyn`-compatible async
trait support is still evolving; `async_trait` is the stable choice the
whitepaper and testing rules already assume. Native async can be adopted
later with a mechanical find/replace once the ecosystem consolidates.

### Option C — Complete in place (chosen)

See Decision above.

## Consequences

### Positive

- Zero thrash on the greenfield scaffolding.
- All subsequent work is additive — new crates, new tests, new adapters —
  which review and merge independently.
- The trait signatures, error variants, and newtype names become stable
  reference points acceptance-designer and software-crafter can rely on.
- ADR-1 through ADR-6 reference modules that actually exist.

### Negative

- Any future port signature change will break all adapters; this is the
  normal cost of a stable port surface and is why port surfaces are small.
- `async_trait` adds a minor allocation per method call. Acceptable — the
  Phase 1 trait surface is not in any throughput-sensitive path.

### Neutral

- The decision does not block future adoption of native async-in-trait; a
  later ADR supersedes this one if the ergonomic win materialises.

## References

- `docs/feature/phase-1-foundation/discuss/user-stories.md` (US-01, US-02,
  US-03, US-04)
- `crates/overdrive-core/src/{id.rs, error.rs, traits/*.rs}`
- `.claude/rules/development.md` (newtype rules, Result alias)
