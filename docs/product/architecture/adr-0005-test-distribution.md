# ADR-0005 — Test distribution: per-crate `tests/`, `tests/acceptance/` reserved for DISTILL scenarios

## Status

Accepted. 2026-04-21.

## Context

`.claude/rules/testing.md` is explicit about the project-wide rule: no
`.feature` files, all test code is Rust under `#[test]` / `#[tokio::test]`
in `crates/{crate}/tests/` (or in module-local `#[cfg(test)] mod tests`).
It also says the crafter translates DISTILL scenarios into Rust
integration tests "in `crates/{crate}/tests/acceptance/*.rs` (or
`tests/*.rs`)".

Two interpretations are operationally different:

- **A. `tests/acceptance/` is the canonical layout** — every integration
  test that maps to a user-story scenario lives under `tests/acceptance/`,
  named for the scenario.
- **B. Plain `tests/*.rs` is the canonical layout** — integration tests
  live at `tests/` top level; the `tests/acceptance/` subdirectory is
  reserved for *acceptance tests specifically derived from DISTILL
  scenarios*.

Considerations:

- Cargo treats `tests/*.rs` as independent integration-test binaries;
  subdirectories are non-standard but honoured if modules are wired
  through `tests/acceptance/mod.rs` or via `[[test]]` in `Cargo.toml`.
  (Cargo automatically compiles top-level `tests/*.rs` as independent
  binaries; any subdirectory file is included as a module of whichever
  top-level file references it. See `cargo test` docs.)
- A clean split prevents engineering drift: unit test (inside the module),
  integration test (one crate's public surface, `tests/*.rs`), acceptance
  test (user-observable scenario, `tests/acceptance/`).
- Acceptance tests are the artifact a reviewer inspects when asking "did
  we actually build US-03's AC?" They benefit from being visibly
  separate from plumbing-level integration tests.
- Phase 1 has no DISTILL artifacts yet. The acceptance scenarios in
  `user-stories.md` will become DISTILL `test-scenarios.md` entries; the
  crafter will translate them into `tests/acceptance/*.rs`.

## Decision

**Option B (refined)**. Layering:

1. **Unit tests** — inline `#[cfg(test)] mod tests` in the module under
   test. Use for: pure-function behaviour, constructor validation,
   error-variant coverage, macro expansion.
2. **Integration tests (plumbing)** — `crates/{crate}/tests/*.rs`. Use
   for: cross-module behaviour within one crate, trait contract checks,
   proptest round-trips, compile-fail (`trybuild`) tests.
3. **Acceptance tests** — `crates/{crate}/tests/acceptance/*.rs`,
   compiled as a single test binary via a `crates/{crate}/tests/acceptance.rs`
   entrypoint that `mod`-declares each scenario file. One `*.rs` file per
   DISTILL scenario. Filename and top-level function name mirror the
   scenario title (snake-case).
4. **DST tests** — `crates/overdrive-sim/tests/dst/*.rs`, compiled via
   `tests/dst.rs`. Each file is one DST scenario run under the harness.

Phase 1 concrete layout:

```
crates/overdrive-core/
  tests/
    newtype_roundtrip.rs          # proptest for all 11 newtypes
    newtype_static_api.rs         # static scan: no String-as-identifier in public API
    trait_non_substitutability.rs # trybuild compile-fail harness
    compile_fail/
      intent_vs_observation.rs    # the compile-fail case
  # no acceptance/ in Phase 1 — DISTILL has not landed yet

crates/overdrive-store-local/
  tests/
    snapshot_roundtrip.rs         # proptest, US-03 AC
    cold_start.rs                 # criterion-style bench-as-test, US-03 AC
    watch.rs                      # watch stream delivers events, US-03 AC

crates/overdrive-sim/
  tests/
    dst.rs                         # mod harness_loader; mod scenarios;
    dst/
      single_leader.rs
      intent_never_crosses_into_observation.rs
      snapshot_roundtrip_bit_identical.rs
      sim_observation_lww_converges.rs
      replay_equivalent_empty_workflow.rs
      entropy_determinism_under_reseed.rs
      twin_run_identity.rs         # US-06 self-test (replay-equivalence)
```

The `tests/acceptance/` directory is reserved for the crafter to populate
once DISTILL lands test-scenarios.md for Phase 1. In Phase 1 itself the
user-stories `UAT Scenarios (BDD)` sections double as acceptance specs;
they are translated into plumbing-level integration tests in the layout
above, one test per AC line.

## Alternatives considered

### Option A — `tests/acceptance/` is the canonical layout

**Rejected.** Treating every integration test as an acceptance test
dilutes the distinction that makes acceptance tests valuable — the
direct trace from test name to user-story scenario. Some proptests
(rkyv-archived-bytes determinism) are plumbing, not a user-visible
outcome; conflating them with acceptance tests hides the scenario list
inside a longer undifferentiated list.

### Option B — Plain `tests/*.rs` for all integration tests (chosen, with `tests/acceptance/` subdirectory for DISTILL output)

See Decision above.

### Option C — `#[cfg(test)] mod tests` for everything

**Rejected.** Unit-level only. Cannot exercise public API + private
impl separation, cannot host `trybuild` compile-fail cases, and would
grow core-crate compile units unboundedly.

## Consequences

### Positive

- Clear mapping from DISTILL `test-scenarios.md` entries to
  `tests/acceptance/` files.
- Plumbing tests coexist with acceptance tests without collision.
- Cargo's standard integration-test layout works out of the box
  (sub-directory harness via one wrapper file per sub-directory).
- Reviewer inspecting "did the crafter ship US-03's AC?" goes straight to
  `tests/acceptance/` once DISTILL lands.

### Negative

- Requires one wrapper `tests/dst.rs` (and eventually `tests/acceptance.rs`)
  per crate with sub-directory tests. Minor boilerplate.
- Developers unfamiliar with the layout need to see ADR-5 on first
  encounter. Mitigated by one-line comments at the top of each wrapper
  file referencing this ADR.

### Neutral

- Future adoption of `cargo-nextest` (which natively supports
  sub-directory test organisation) will work without change.

## References

- `.claude/rules/testing.md` (no `.feature` files)
- `docs/feature/phase-1-foundation/discuss/user-stories.md` (UAT Scenarios)
- Cargo book: [Integration
  tests](https://doc.rust-lang.org/cargo/guide/tests.html)

## Changelog

- 2026-04-22 — Removed `ScenarioBuilder` references (residual terminology from another project).
